//! Pure crash-recovery decisions, record integrity, and scheduling.
//!
//! Filesystem and clocks stay in adapters. This module defines the recovery
//! record contract, validates complete records, schedules persistence against
//! the recovery-point objective, and classifies startup dispositions. Restored
//! content always remains dirty until the user saves; nothing here writes a
//! user document path.

use std::time::Duration;

use super::edit::Selection;
use super::limits::MAX_DOCUMENT_BYTES;
use super::revision::Revision;
use super::save::ContentFingerprint;
use super::text_format::{Bom, Encoding};

/// Magic bytes identifying a Noter recovery record.
pub const RECOVERY_MAGIC: &[u8; 8] = b"NOTERREC";

/// Legacy recovery schema retained for backward-compatible reads.
const LEGACY_RECOVERY_SCHEMA_VERSION: u32 = 1;

/// Current on-disk recovery schema version.
pub const RECOVERY_SCHEMA_VERSION: u32 = 2;

/// Persist after this much continuous idle time while dirty.
pub const RECOVERY_IDLE_DEBOUNCE: Duration = Duration::from_secs(2);

/// Persist at least this often while dirty and still editing.
pub const RECOVERY_MAX_DIRTY_INTERVAL: Duration = Duration::from_secs(15);

/// How often an adapter must poll while a persist is in flight off-thread.
///
/// The write itself must not run on the render thread. The scheduler still
/// needs a wake-up so a completed worker result is applied promptly.
pub const RECOVERY_IN_FLIGHT_POLL: Duration = Duration::from_millis(16);

/// Maximum encoded original-path metadata retained in a recovery record.
pub const MAX_RECOVERY_PATH_BYTES: usize = 128 * 1024;

/// Schema v1 fixed header size before the variable path and content payloads.
const V1_FIXED_HEADER_LEN: usize = 8 + 4 + 16 + 16 + 8 + 8 + 8 + 4 + 1 + 1 + 8 + 8 + 8 + 32;

/// Schema v2 fixed header size before the variable path and content payloads.
///
/// Little-endian field order: magic, schema, document id, instance id,
/// lineage generation, predecessor tag and id, revision, created and updated
/// wall times, path length, BOM and encoding tags, selection endpoints,
/// content length, and whole-record checksum.
const V2_FIXED_HEADER_LEN: usize =
    8 + 4 + 16 + 16 + 8 + 1 + 16 + 8 + 8 + 8 + 4 + 1 + 1 + 8 + 8 + 8 + 32;
const V1_CHECKSUM_OFFSET: usize = 98;
const V2_CHECKSUM_OFFSET: usize = 123;
const CHECKSUM_LEN: usize = 32;

/// Random document identity used only for recovery correlation.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RecoveryDocumentId([u8; 16]);

impl RecoveryDocumentId {
    /// Creates an identity from sixteen random or fixture bytes.
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the raw identity bytes.
    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }
}

/// Random process-instance identity for one open editor session.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct RecoveryInstanceId([u8; 16]);

impl RecoveryInstanceId {
    /// Creates an identity from sixteen random or fixture bytes.
    pub const fn new(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Returns the raw identity bytes.
    pub const fn as_bytes(self) -> [u8; 16] {
        self.0
    }
}

/// Monotonic generation within one recovery document lineage.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RecoveryLineageGeneration(u64);

impl RecoveryLineageGeneration {
    /// The first generation for a newly opened document lineage.
    pub const ROOT: Self = Self(0);

    /// Creates a generation from a persisted or fixture value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the stored generation value.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Returns the next causal generation, or `None` at the integer ceiling.
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(next) => Some(Self(next)),
            None => None,
        }
    }
}

/// Wall-clock milliseconds for recovery metadata display.
///
/// Adapters supply these values. Scheduling policy uses monotonic
/// [`RecoveryClock`] separately so tests stay deterministic. Causal recovery
/// ordering never trusts this value.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RecoveryWallTime(u64);

impl RecoveryWallTime {
    /// Creates a wall-clock timestamp from Unix epoch milliseconds.
    pub const fn from_unix_millis(millis: u64) -> Self {
        Self(millis)
    }

    /// Returns Unix epoch milliseconds.
    pub const fn unix_millis(self) -> u64 {
        self.0
    }
}

/// Monotonic elapsed time used only for recovery scheduling.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct RecoveryClock(Duration);

impl RecoveryClock {
    /// Creates a clock reading from an elapsed monotonic duration.
    pub const fn new(elapsed: Duration) -> Self {
        Self(elapsed)
    }

    /// Returns the stored elapsed duration.
    pub const fn elapsed(self) -> Duration {
        self.0
    }
}

/// Why a loaded recovery artifact cannot be offered for restore.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RecoveryQuarantineReason {
    /// The file does not begin with the recovery magic.
    InvalidMagic,
    /// The schema version is newer or unsupported.
    UnknownSchema,
    /// Fixed or variable lengths do not match the buffer.
    Truncated,
    /// Path metadata exceeds the supported bound.
    PathTooLarge,
    /// Content exceeds the shared document ceiling.
    ContentTooLarge,
    /// Content bytes are not valid UTF-8 after an optional UTF-8 BOM.
    InvalidUtf8,
    /// The content checksum does not match the payload.
    ChecksumMismatch,
    /// Selection offsets fall outside the recovered content body.
    InvalidSelection,
    /// Encoding or BOM tags are not recognized.
    InvalidFormatTags,
    /// Causal lineage metadata is malformed or self-referential.
    InvalidLineage,
    /// The pathname and encoded header name different recovery instances.
    InstanceMismatch,
}

impl RecoveryQuarantineReason {
    /// Returns a short user-facing explanation without paths or content.
    pub const fn description(self) -> &'static str {
        match self {
            Self::InvalidMagic => "The recovery file is not a Noter recovery record.",
            Self::UnknownSchema => "The recovery record uses an unsupported schema version.",
            Self::Truncated => "The recovery record is incomplete or truncated.",
            Self::PathTooLarge => "The recovery record path metadata exceeds the supported limit.",
            Self::ContentTooLarge => "The recovery record content exceeds the document size limit.",
            Self::InvalidUtf8 => "The recovery record content is not valid UTF-8.",
            Self::ChecksumMismatch => "The recovery record failed its integrity check.",
            Self::InvalidSelection => {
                "The recovery record selection is outside the recovered text."
            }
            Self::InvalidFormatTags => "The recovery record encoding metadata is not recognized.",
            Self::InvalidLineage => "The recovery record lineage metadata is not valid.",
            Self::InstanceMismatch => {
                "The recovery pathname and encoded instance identities do not agree."
            }
        }
    }
}

/// Validated recovery metadata suitable for a bounded startup offer.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValidatedRecoveryMetadata {
    schema_version: u32,
    document_id: RecoveryDocumentId,
    instance_id: RecoveryInstanceId,
    lineage_generation: Option<RecoveryLineageGeneration>,
    predecessor_instance: Option<RecoveryInstanceId>,
    revision: Revision,
    created_at: RecoveryWallTime,
    updated_at: RecoveryWallTime,
    /// Original path bytes when known; empty for untitled documents.
    original_path: Vec<u8>,
    bom: Bom,
    encoding: Encoding,
    selection: Selection,
    content_len: usize,
    content_checksum: ContentFingerprint,
    record_checksum: ContentFingerprint,
}

impl ValidatedRecoveryMetadata {
    /// Returns the validated on-disk schema version.
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Returns the document identity.
    pub const fn document_id(&self) -> RecoveryDocumentId {
        self.document_id
    }

    /// Returns the editor-instance identity.
    pub const fn instance_id(&self) -> RecoveryInstanceId {
        self.instance_id
    }

    /// Returns the causal generation, or `None` for a legacy v1 record.
    pub const fn lineage_generation(&self) -> Option<RecoveryLineageGeneration> {
        self.lineage_generation
    }

    /// Returns the immediate predecessor instance when recorded by schema v2.
    pub const fn predecessor_instance(&self) -> Option<RecoveryInstanceId> {
        self.predecessor_instance
    }

    /// Returns the content revision captured in the record.
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns when the recovery session began.
    pub const fn created_at(&self) -> RecoveryWallTime {
        self.created_at
    }

    /// Returns when this revision was persisted.
    pub const fn updated_at(&self) -> RecoveryWallTime {
        self.updated_at
    }

    /// Returns original path metadata bytes, or empty when untitled.
    pub fn original_path(&self) -> &[u8] {
        &self.original_path
    }

    /// Returns the recorded BOM state.
    pub const fn bom(&self) -> Bom {
        self.bom
    }

    /// Returns the recorded encoding.
    pub const fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Returns the directional selection to restore.
    pub const fn selection(&self) -> Selection {
        self.selection
    }

    /// Returns the exact serialized content length.
    pub const fn content_len(&self) -> usize {
        self.content_len
    }

    /// Returns the content integrity fingerprint.
    pub const fn content_checksum(&self) -> ContentFingerprint {
        self.content_checksum
    }

    /// Returns the persisted record integrity fingerprint.
    ///
    /// Schema v1 protects content only. Schema v2 protects every encoded field
    /// except the checksum bytes themselves.
    pub const fn record_checksum(&self) -> ContentFingerprint {
        self.record_checksum
    }
}

/// Validated recovery metadata and content ready for restore.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ValidatedRecoveryRecord {
    metadata: Box<ValidatedRecoveryMetadata>,
    /// Exact serialized document bytes including an optional UTF-8 BOM.
    content: Vec<u8>,
}

impl ValidatedRecoveryRecord {
    /// Returns metadata that can be retained without owning document content.
    pub const fn metadata(&self) -> &ValidatedRecoveryMetadata {
        &self.metadata
    }

    /// Returns the validated on-disk schema version.
    pub const fn schema_version(&self) -> u32 {
        self.metadata.schema_version()
    }

    /// Returns the document identity.
    pub const fn document_id(&self) -> RecoveryDocumentId {
        self.metadata.document_id()
    }

    /// Returns the editor-instance identity.
    pub const fn instance_id(&self) -> RecoveryInstanceId {
        self.metadata.instance_id()
    }

    /// Returns the causal generation, or `None` for a legacy v1 record.
    pub const fn lineage_generation(&self) -> Option<RecoveryLineageGeneration> {
        self.metadata.lineage_generation()
    }

    /// Returns the immediate predecessor instance when recorded by schema v2.
    pub const fn predecessor_instance(&self) -> Option<RecoveryInstanceId> {
        self.metadata.predecessor_instance()
    }

    /// Returns the content revision captured in the record.
    pub const fn revision(&self) -> Revision {
        self.metadata.revision()
    }

    /// Returns when the recovery session began.
    pub const fn created_at(&self) -> RecoveryWallTime {
        self.metadata.created_at()
    }

    /// Returns when this revision was persisted.
    pub const fn updated_at(&self) -> RecoveryWallTime {
        self.metadata.updated_at()
    }

    /// Returns original path metadata bytes, or empty when untitled.
    pub fn original_path(&self) -> &[u8] {
        self.metadata.original_path()
    }

    /// Returns the recorded BOM state.
    pub const fn bom(&self) -> Bom {
        self.metadata.bom()
    }

    /// Returns the recorded encoding.
    pub const fn encoding(&self) -> Encoding {
        self.metadata.encoding()
    }

    /// Returns the directional selection to restore.
    pub const fn selection(&self) -> Selection {
        self.metadata.selection()
    }

    /// Returns the exact serialized content bytes.
    pub fn content(&self) -> &[u8] {
        &self.content
    }

    /// Returns the content integrity fingerprint.
    pub const fn content_checksum(&self) -> ContentFingerprint {
        self.metadata.content_checksum()
    }

    /// Returns the persisted record integrity fingerprint.
    pub const fn record_checksum(&self) -> ContentFingerprint {
        self.metadata.record_checksum()
    }
}

/// Startup classification for one recovery artifact.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum RecoveryStartupDisposition {
    /// Offer restore of a validated dirty recovery record.
    Offer(ValidatedRecoveryRecord),
    /// Move aside and explain a corrupt or unsupported record.
    Quarantine(RecoveryQuarantineReason),
}

/// Inputs required to build a [`RecoverySnapshot`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecoverySnapshotParts {
    /// Random document identity for recovery correlation.
    pub document_id: RecoveryDocumentId,
    /// Random editor-instance identity.
    pub instance_id: RecoveryInstanceId,
    /// Content revision captured in the snapshot.
    pub revision: Revision,
    /// Wall time when the recovery session began.
    pub created_at: RecoveryWallTime,
    /// Wall time when this revision was captured.
    pub updated_at: RecoveryWallTime,
    /// Original path bytes when known; empty for untitled documents.
    pub original_path: Vec<u8>,
    /// BOM state matching the serialized content prefix.
    pub bom: Bom,
    /// Text encoding of the serialized content.
    pub encoding: Encoding,
    /// Directional selection measured in body UTF-8 bytes.
    pub selection: Selection,
    /// Exact serialized document bytes including an optional UTF-8 BOM.
    pub content: Vec<u8>,
}

/// Immutable snapshot the adapter may persist for one dirty revision.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecoverySnapshot {
    document_id: RecoveryDocumentId,
    instance_id: RecoveryInstanceId,
    lineage_generation: RecoveryLineageGeneration,
    predecessor_instance: Option<RecoveryInstanceId>,
    revision: Revision,
    created_at: RecoveryWallTime,
    updated_at: RecoveryWallTime,
    original_path: Vec<u8>,
    bom: Bom,
    encoding: Encoding,
    selection: Selection,
    content: Vec<u8>,
}

impl RecoverySnapshot {
    /// Builds a root-lineage snapshot after enforcing recovery resource ceilings.
    ///
    /// # Errors
    ///
    /// Returns a quarantine reason when path or content bounds fail, content is
    /// not valid UTF-8 with an optional BOM, or the selection is out of range.
    pub fn try_new(parts: RecoverySnapshotParts) -> Result<Self, RecoveryQuarantineReason> {
        Self::try_new_with_lineage(parts, RecoveryLineageGeneration::ROOT, None)
    }

    /// Builds a causally attributed snapshot after enforcing recovery limits.
    ///
    /// # Errors
    ///
    /// Returns a quarantine reason for invalid content metadata or when the
    /// predecessor is the snapshot's own instance.
    pub fn try_new_with_lineage(
        parts: RecoverySnapshotParts,
        lineage_generation: RecoveryLineageGeneration,
        predecessor_instance: Option<RecoveryInstanceId>,
    ) -> Result<Self, RecoveryQuarantineReason> {
        if parts.original_path.len() > MAX_RECOVERY_PATH_BYTES {
            return Err(RecoveryQuarantineReason::PathTooLarge);
        }
        if parts.content.len() > MAX_DOCUMENT_BYTES {
            return Err(RecoveryQuarantineReason::ContentTooLarge);
        }
        let body = validate_content_body(&parts.content, parts.bom)?;
        validate_selection(parts.selection, body)?;
        let is_root = lineage_generation == RecoveryLineageGeneration::ROOT;
        if is_root != predecessor_instance.is_none()
            || predecessor_instance == Some(parts.instance_id)
        {
            return Err(RecoveryQuarantineReason::InvalidLineage);
        }
        Ok(Self {
            document_id: parts.document_id,
            instance_id: parts.instance_id,
            lineage_generation,
            predecessor_instance,
            revision: parts.revision,
            created_at: parts.created_at,
            updated_at: parts.updated_at,
            original_path: parts.original_path,
            bom: parts.bom,
            encoding: parts.encoding,
            selection: parts.selection,
            content: parts.content,
        })
    }

    /// Builds the immediate schema-v2 successor of a validated recovery record.
    ///
    /// Legacy schema-v1 records are treated as generation zero. The document
    /// identity must remain unchanged, and generation overflow fails closed.
    ///
    /// # Errors
    ///
    /// Returns a quarantine reason when the document identity changes, the
    /// generation cannot advance, or the snapshot payload is invalid.
    pub fn try_new_successor(
        parts: RecoverySnapshotParts,
        predecessor: &ValidatedRecoveryMetadata,
    ) -> Result<Self, RecoveryQuarantineReason> {
        if parts.document_id != predecessor.document_id() {
            return Err(RecoveryQuarantineReason::InvalidLineage);
        }
        let generation = predecessor
            .lineage_generation()
            .unwrap_or(RecoveryLineageGeneration::ROOT)
            .checked_next()
            .ok_or(RecoveryQuarantineReason::InvalidLineage)?;
        Self::try_new_with_lineage(parts, generation, Some(predecessor.instance_id()))
    }

    /// Returns the document identity.
    pub const fn document_id(&self) -> RecoveryDocumentId {
        self.document_id
    }

    /// Returns the instance identity.
    pub const fn instance_id(&self) -> RecoveryInstanceId {
        self.instance_id
    }

    /// Returns the causal generation written to schema v2.
    pub const fn lineage_generation(&self) -> RecoveryLineageGeneration {
        self.lineage_generation
    }

    /// Returns the immediate predecessor instance, when this is a successor.
    pub const fn predecessor_instance(&self) -> Option<RecoveryInstanceId> {
        self.predecessor_instance
    }

    /// Returns the snapshot revision.
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns creation wall time.
    pub const fn created_at(&self) -> RecoveryWallTime {
        self.created_at
    }

    /// Returns update wall time.
    pub const fn updated_at(&self) -> RecoveryWallTime {
        self.updated_at
    }

    /// Returns original path metadata.
    pub fn original_path(&self) -> &[u8] {
        &self.original_path
    }

    /// Returns BOM state.
    pub const fn bom(&self) -> Bom {
        self.bom
    }

    /// Returns encoding.
    pub const fn encoding(&self) -> Encoding {
        self.encoding
    }

    /// Returns selection.
    pub const fn selection(&self) -> Selection {
        self.selection
    }

    /// Returns serialized content.
    pub fn content(&self) -> &[u8] {
        &self.content
    }

    /// Serializes this snapshot into the versioned recovery record bytes.
    ///
    /// Path and content lengths are always representable because
    /// [`RecoverySnapshot::try_new`] enforces ceilings far below the on-disk
    /// integer widths.
    pub fn encode(&self) -> Vec<u8> {
        // try_new enforces path <= MAX_RECOVERY_PATH_BYTES (128 KiB) and
        // content <= MAX_DOCUMENT_BYTES (64 MiB), both well inside these widths.
        #[allow(clippy::cast_possible_truncation)]
        let path_len = self.original_path.len() as u32;
        #[allow(clippy::cast_possible_truncation)]
        let content_len = self.content.len() as u64;
        debug_assert!(self.original_path.len() <= MAX_RECOVERY_PATH_BYTES);
        debug_assert!(self.content.len() <= MAX_DOCUMENT_BYTES);
        let mut out = Vec::with_capacity(
            V2_FIXED_HEADER_LEN
                .saturating_add(self.original_path.len())
                .saturating_add(self.content.len()),
        );
        out.extend_from_slice(RECOVERY_MAGIC);
        out.extend_from_slice(&RECOVERY_SCHEMA_VERSION.to_le_bytes());
        out.extend_from_slice(&self.document_id.as_bytes());
        out.extend_from_slice(&self.instance_id.as_bytes());
        out.extend_from_slice(&self.lineage_generation.get().to_le_bytes());
        if let Some(instance_id) = self.predecessor_instance {
            out.push(1);
            out.extend_from_slice(&instance_id.as_bytes());
        } else {
            out.push(0);
            out.extend_from_slice(&[0; 16]);
        }
        out.extend_from_slice(&self.revision.get().to_le_bytes());
        out.extend_from_slice(&self.created_at.unix_millis().to_le_bytes());
        out.extend_from_slice(&self.updated_at.unix_millis().to_le_bytes());
        out.extend_from_slice(&path_len.to_le_bytes());
        out.push(encode_bom(self.bom));
        out.push(encode_encoding(self.encoding));
        out.extend_from_slice(&usize_to_u64(self.selection.anchor()).to_le_bytes());
        out.extend_from_slice(&usize_to_u64(self.selection.active()).to_le_bytes());
        out.extend_from_slice(&content_len.to_le_bytes());
        out.extend_from_slice(&[0; CHECKSUM_LEN]);
        out.extend_from_slice(&self.original_path);
        out.extend_from_slice(&self.content);
        let checksum = recovery_record_checksum(&out, V2_CHECKSUM_OFFSET);
        out[V2_CHECKSUM_OFFSET..V2_CHECKSUM_OFFSET + CHECKSUM_LEN]
            .copy_from_slice(checksum.as_bytes());
        out
    }
}

struct ParsedRecoveryRecord<'a> {
    metadata: ValidatedRecoveryMetadata,
    content: &'a [u8],
}

struct RecoveryHeader {
    schema_version: u32,
    document_id: RecoveryDocumentId,
    instance_id: RecoveryInstanceId,
    lineage_generation: Option<RecoveryLineageGeneration>,
    predecessor_instance: Option<RecoveryInstanceId>,
    revision: Revision,
    created_at: RecoveryWallTime,
    updated_at: RecoveryWallTime,
    path_len_offset: usize,
    bom_offset: usize,
    encoding_offset: usize,
    anchor_offset: usize,
    active_offset: usize,
    content_len_offset: usize,
    checksum_offset: usize,
    fixed_header_len: usize,
}

/// Validates complete recovery record bytes without consulting the filesystem.
pub fn validate_recovery_record(bytes: &[u8]) -> RecoveryStartupDisposition {
    match parse_recovery_record(bytes) {
        Ok(parsed) => RecoveryStartupDisposition::Offer(ValidatedRecoveryRecord {
            metadata: Box::new(parsed.metadata),
            content: parsed.content.to_vec(),
        }),
        Err(reason) => RecoveryStartupDisposition::Quarantine(reason),
    }
}

/// Validates complete recovery bytes while retaining metadata only.
///
/// This performs the same integrity, UTF-8, format, and selection validation as
/// [`validate_recovery_record`] without cloning the document content.
///
/// # Errors
///
/// Returns a quarantine reason when the bytes fail format, integrity, content,
/// or selection validation.
pub fn validate_recovery_metadata(
    bytes: &[u8],
) -> Result<ValidatedRecoveryMetadata, RecoveryQuarantineReason> {
    parse_recovery_record(bytes).map(|parsed| parsed.metadata)
}

fn parse_recovery_record(
    bytes: &[u8],
) -> Result<ParsedRecoveryRecord<'_>, RecoveryQuarantineReason> {
    let header = parse_recovery_header(bytes)?;
    let path_len_u32 =
        read_u32(bytes, header.path_len_offset).ok_or(RecoveryQuarantineReason::Truncated)?;
    let path_len =
        usize::try_from(path_len_u32).map_err(|_| RecoveryQuarantineReason::PathTooLarge)?;
    if path_len > MAX_RECOVERY_PATH_BYTES {
        return Err(RecoveryQuarantineReason::PathTooLarge);
    }

    let bom = bytes
        .get(header.bom_offset)
        .copied()
        .and_then(decode_bom)
        .ok_or(RecoveryQuarantineReason::InvalidFormatTags)?;
    let encoding = bytes
        .get(header.encoding_offset)
        .copied()
        .and_then(decode_encoding)
        .ok_or(RecoveryQuarantineReason::InvalidFormatTags)?;
    let anchor = read_u64(bytes, header.anchor_offset)
        .and_then(u64_to_usize)
        .ok_or(RecoveryQuarantineReason::InvalidSelection)?;
    let active = read_u64(bytes, header.active_offset)
        .and_then(u64_to_usize)
        .ok_or(RecoveryQuarantineReason::InvalidSelection)?;
    let selection = Selection::new(anchor, active);
    let content_len_u64 =
        read_u64(bytes, header.content_len_offset).ok_or(RecoveryQuarantineReason::Truncated)?;
    let content_len =
        usize::try_from(content_len_u64).map_err(|_| RecoveryQuarantineReason::ContentTooLarge)?;
    if content_len > MAX_DOCUMENT_BYTES {
        return Err(RecoveryQuarantineReason::ContentTooLarge);
    }
    let stored_checksum = read_array32(bytes, header.checksum_offset)
        .map(ContentFingerprint::new)
        .ok_or(RecoveryQuarantineReason::Truncated)?;

    let path_end = header.fixed_header_len.saturating_add(path_len);
    let content_end = path_end.saturating_add(content_len);
    if bytes.len() != content_end {
        return Err(RecoveryQuarantineReason::Truncated);
    }
    let original_path = bytes[header.fixed_header_len..path_end].to_vec();
    let content = &bytes[path_end..content_end];
    let content_checksum = ContentFingerprint::from_bytes(content);
    let record_checksum = if header.schema_version == LEGACY_RECOVERY_SCHEMA_VERSION {
        content_checksum
    } else {
        recovery_record_checksum(bytes, header.checksum_offset)
    };
    if record_checksum != stored_checksum {
        return Err(RecoveryQuarantineReason::ChecksumMismatch);
    }

    let body = validate_content_body(content, bom)?;
    validate_selection(selection, body)?;
    Ok(ParsedRecoveryRecord {
        metadata: ValidatedRecoveryMetadata {
            schema_version: header.schema_version,
            document_id: header.document_id,
            instance_id: header.instance_id,
            lineage_generation: header.lineage_generation,
            predecessor_instance: header.predecessor_instance,
            revision: header.revision,
            created_at: header.created_at,
            updated_at: header.updated_at,
            original_path,
            bom,
            encoding,
            selection,
            content_len,
            content_checksum,
            record_checksum,
        },
        content,
    })
}

fn parse_recovery_header(bytes: &[u8]) -> Result<RecoveryHeader, RecoveryQuarantineReason> {
    if bytes.len() < V1_FIXED_HEADER_LEN {
        return Err(RecoveryQuarantineReason::Truncated);
    }
    if &bytes[0..8] != RECOVERY_MAGIC.as_slice() {
        return Err(RecoveryQuarantineReason::InvalidMagic);
    }
    let schema_version = read_u32(bytes, 8).ok_or(RecoveryQuarantineReason::Truncated)?;
    let document_id = read_array16(bytes, 12)
        .map(RecoveryDocumentId::new)
        .ok_or(RecoveryQuarantineReason::Truncated)?;
    let instance_id = read_array16(bytes, 28)
        .map(RecoveryInstanceId::new)
        .ok_or(RecoveryQuarantineReason::Truncated)?;

    match schema_version {
        LEGACY_RECOVERY_SCHEMA_VERSION => Ok(RecoveryHeader {
            schema_version,
            document_id,
            instance_id,
            lineage_generation: None,
            predecessor_instance: None,
            revision: read_u64(bytes, 44)
                .map(Revision::new)
                .ok_or(RecoveryQuarantineReason::Truncated)?,
            created_at: read_u64(bytes, 52)
                .map(RecoveryWallTime::from_unix_millis)
                .ok_or(RecoveryQuarantineReason::Truncated)?,
            updated_at: read_u64(bytes, 60)
                .map(RecoveryWallTime::from_unix_millis)
                .ok_or(RecoveryQuarantineReason::Truncated)?,
            path_len_offset: 68,
            bom_offset: 72,
            encoding_offset: 73,
            anchor_offset: 74,
            active_offset: 82,
            content_len_offset: 90,
            checksum_offset: V1_CHECKSUM_OFFSET,
            fixed_header_len: V1_FIXED_HEADER_LEN,
        }),
        RECOVERY_SCHEMA_VERSION => {
            if bytes.len() < V2_FIXED_HEADER_LEN {
                return Err(RecoveryQuarantineReason::Truncated);
            }
            let lineage_generation = read_u64(bytes, 44)
                .map(RecoveryLineageGeneration::new)
                .ok_or(RecoveryQuarantineReason::Truncated)?;
            let predecessor_bytes =
                read_array16(bytes, 53).ok_or(RecoveryQuarantineReason::Truncated)?;
            let predecessor_instance = match bytes[52] {
                0 if predecessor_bytes == [0; 16] => None,
                1 => Some(RecoveryInstanceId::new(predecessor_bytes)),
                _ => return Err(RecoveryQuarantineReason::InvalidLineage),
            };
            let is_root = lineage_generation == RecoveryLineageGeneration::ROOT;
            if is_root != predecessor_instance.is_none()
                || predecessor_instance == Some(instance_id)
            {
                return Err(RecoveryQuarantineReason::InvalidLineage);
            }
            Ok(RecoveryHeader {
                schema_version,
                document_id,
                instance_id,
                lineage_generation: Some(lineage_generation),
                predecessor_instance,
                revision: read_u64(bytes, 69)
                    .map(Revision::new)
                    .ok_or(RecoveryQuarantineReason::Truncated)?,
                created_at: read_u64(bytes, 77)
                    .map(RecoveryWallTime::from_unix_millis)
                    .ok_or(RecoveryQuarantineReason::Truncated)?,
                updated_at: read_u64(bytes, 85)
                    .map(RecoveryWallTime::from_unix_millis)
                    .ok_or(RecoveryQuarantineReason::Truncated)?,
                path_len_offset: 93,
                bom_offset: 97,
                encoding_offset: 98,
                anchor_offset: 99,
                active_offset: 107,
                content_len_offset: 115,
                checksum_offset: V2_CHECKSUM_OFFSET,
                fixed_header_len: V2_FIXED_HEADER_LEN,
            })
        }
        _ => Err(RecoveryQuarantineReason::UnknownSchema),
    }
}

fn recovery_record_checksum(bytes: &[u8], checksum_offset: usize) -> ContentFingerprint {
    let Some(checksum_end) = checksum_offset.checked_add(CHECKSUM_LEN) else {
        return ContentFingerprint::from_bytes(bytes);
    };
    let (Some(prefix), Some(suffix)) = (bytes.get(..checksum_offset), bytes.get(checksum_end..))
    else {
        return ContentFingerprint::from_bytes(bytes);
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(prefix);
    hasher.update(suffix);
    ContentFingerprint::new(*hasher.finalize().as_bytes())
}

/// Pure effect requested by the recovery scheduler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecoveryScheduleEffect {
    /// No persistence work is required.
    None,
    /// Persist a snapshot of the current dirty revision.
    Persist {
        /// Exact revision the adapter must capture.
        revision: Revision,
        /// Session epoch that must match the later acknowledgement.
        epoch: u64,
    },
    /// Remove the owned recovery record after a clean save or discard.
    DeleteOwned {
        /// Session epoch that invalidates in-flight writes from earlier epochs.
        epoch: u64,
    },
}

/// One input to the recovery scheduler.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecoveryScheduleCommand {
    /// Content became dirty or was edited again at this monotonic time.
    Edited {
        /// Current content revision after the edit.
        revision: Revision,
        /// Monotonic observation time.
        now: RecoveryClock,
    },
    /// Content matches the last committed save (or a clean untitled reset).
    BecameClean {
        /// Current clean revision.
        revision: Revision,
    },
    /// Periodic or idle poll of the scheduler.
    Tick {
        /// Monotonic observation time.
        now: RecoveryClock,
    },
    /// The adapter finished writing the named revision for the named epoch.
    PersistAcknowledged {
        /// Revision that was successfully persisted.
        revision: Revision,
        /// Epoch captured when the persist was requested.
        epoch: u64,
    },
    /// The adapter failed to persist; keep dirty scheduling active.
    PersistFailed {
        /// Revision that failed to persist.
        revision: Revision,
        /// Epoch captured when the persist was requested.
        epoch: u64,
        /// Monotonic observation time of the failure.
        now: RecoveryClock,
    },
    /// Explicit discard of recoverable content for this session.
    Discarded,
}

/// Pure recovery persistence scheduler.
///
/// Dirty content schedules a write after [`RECOVERY_IDLE_DEBOUNCE`] of idle
/// time, and no later than [`RECOVERY_MAX_DIRTY_INTERVAL`] after the first edit
/// in a dirty interval. Clean save or discard requests deletion. Persist
/// acknowledgements for stale revisions or epochs are ignored so a late write
/// cannot reintroduce recovery after save or discard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct RecoveryScheduleState {
    dirty: bool,
    current_revision: Revision,
    last_edit_at: Option<RecoveryClock>,
    dirty_since: Option<RecoveryClock>,
    last_persisted_revision: Option<Revision>,
    in_flight_revision: Option<Revision>,
    /// Increments on clean and discard so late disk completions cannot apply.
    epoch: u64,
}

impl Default for RecoveryScheduleState {
    fn default() -> Self {
        Self {
            dirty: false,
            current_revision: Revision::INITIAL,
            last_edit_at: None,
            dirty_since: None,
            last_persisted_revision: None,
            in_flight_revision: None,
            epoch: 0,
        }
    }
}

impl RecoveryScheduleState {
    /// Returns whether the scheduler currently tracks dirty content.
    pub const fn is_dirty(self) -> bool {
        self.dirty
    }

    /// Returns the current tracked content revision.
    pub const fn current_revision(self) -> Revision {
        self.current_revision
    }

    /// Returns the revision waiting for a persist acknowledgement, if any.
    pub const fn in_flight_revision(self) -> Option<Revision> {
        self.in_flight_revision
    }

    /// Returns the newest revision known to be on durable recovery storage.
    pub const fn last_persisted_revision(self) -> Option<Revision> {
        self.last_persisted_revision
    }

    /// Returns the current session epoch for adapter correlation.
    pub const fn epoch(self) -> u64 {
        self.epoch
    }

    /// Clears document-specific scheduling state while advancing the epoch.
    ///
    /// Adapters use this when a window starts tracking a new document
    /// identity. Keeping the epoch monotonic invalidates queued work from the
    /// previous identity without making a fresh worker job look stale.
    pub fn reset_for_new_identity(&mut self) {
        let epoch = self.epoch.wrapping_add(1);
        *self = Self {
            epoch,
            ..Self::default()
        };
    }

    /// Returns how long an adapter may sleep before the next `Tick` is due.
    ///
    /// `None` means no persist is outstanding, so the adapter needs no timer at
    /// all. `Some(Duration::ZERO)` means a persist is already due. An in-flight
    /// persist returns [`RECOVERY_IN_FLIGHT_POLL`] so the adapter can collect a
    /// worker completion without blocking the render thread on disk I/O.
    /// An adapter that only ticks while it happens to be drawing would silently
    /// lengthen the recovery-point objective, so it must schedule a wake-up from
    /// this value instead.
    pub fn next_persist_delay(self, now: RecoveryClock) -> Option<Duration> {
        if self.in_flight_revision.is_some() {
            return Some(RECOVERY_IN_FLIGHT_POLL);
        }
        if !self.dirty {
            return None;
        }
        if self.last_persisted_revision == Some(self.current_revision) {
            return None;
        }
        let last_edit_at = self.last_edit_at?;
        let dirty_since = self.dirty_since?;
        let (Some(idle), Some(dirty_for)) = (
            now.elapsed().checked_sub(last_edit_at.elapsed()),
            now.elapsed().checked_sub(dirty_since.elapsed()),
        ) else {
            // A clock regression is already due, exactly as `tick` decides.
            return Some(Duration::ZERO);
        };
        Some(
            RECOVERY_IDLE_DEBOUNCE
                .saturating_sub(idle)
                .min(RECOVERY_MAX_DIRTY_INTERVAL.saturating_sub(dirty_for)),
        )
    }

    /// Applies one scheduling command and returns the adapter effect.
    pub fn reduce(&mut self, command: RecoveryScheduleCommand) -> RecoveryScheduleEffect {
        match command {
            RecoveryScheduleCommand::Edited { revision, now } => self.edited(revision, now),
            RecoveryScheduleCommand::BecameClean { revision } => self.became_clean(revision),
            RecoveryScheduleCommand::Tick { now } => self.tick(now),
            RecoveryScheduleCommand::PersistAcknowledged { revision, epoch } => {
                self.persist_acknowledged(revision, epoch)
            }
            RecoveryScheduleCommand::PersistFailed {
                revision,
                epoch,
                now,
            } => self.persist_failed(revision, epoch, now),
            RecoveryScheduleCommand::Discarded => self.discarded(),
        }
    }

    fn edited(&mut self, revision: Revision, now: RecoveryClock) -> RecoveryScheduleEffect {
        self.current_revision = revision;
        self.dirty = true;
        self.last_edit_at = Some(now);
        if self.dirty_since.is_none() {
            self.dirty_since = Some(now);
        }
        self.tick(now)
    }

    const fn became_clean(&mut self, revision: Revision) -> RecoveryScheduleEffect {
        self.current_revision = revision;
        self.dirty = false;
        self.last_edit_at = None;
        self.dirty_since = None;
        self.in_flight_revision = None;
        self.last_persisted_revision = None;
        self.epoch = self.epoch.wrapping_add(1);
        // A clean document still needs any owned recovery artifact removed.
        RecoveryScheduleEffect::DeleteOwned { epoch: self.epoch }
    }

    const fn discarded(&mut self) -> RecoveryScheduleEffect {
        self.dirty = false;
        self.last_edit_at = None;
        self.dirty_since = None;
        self.in_flight_revision = None;
        self.current_revision = Revision::INITIAL;
        self.last_persisted_revision = None;
        self.epoch = self.epoch.wrapping_add(1);
        RecoveryScheduleEffect::DeleteOwned { epoch: self.epoch }
    }

    fn tick(&mut self, now: RecoveryClock) -> RecoveryScheduleEffect {
        if !self.dirty {
            return RecoveryScheduleEffect::None;
        }
        if self.in_flight_revision.is_some() {
            return RecoveryScheduleEffect::None;
        }
        if self.last_persisted_revision == Some(self.current_revision) {
            return RecoveryScheduleEffect::None;
        }

        let Some(last_edit_at) = self.last_edit_at else {
            return RecoveryScheduleEffect::None;
        };
        let Some(dirty_since) = self.dirty_since else {
            return RecoveryScheduleEffect::None;
        };

        // Clock regression must not silently disable the recovery-point objective.
        let due = match (
            now.elapsed().checked_sub(last_edit_at.elapsed()),
            now.elapsed().checked_sub(dirty_since.elapsed()),
        ) {
            (None, _) | (_, None) => true,
            (Some(idle), Some(dirty_for)) => {
                idle >= RECOVERY_IDLE_DEBOUNCE || dirty_for >= RECOVERY_MAX_DIRTY_INTERVAL
            }
        };

        if due {
            self.in_flight_revision = Some(self.current_revision);
            RecoveryScheduleEffect::Persist {
                revision: self.current_revision,
                epoch: self.epoch,
            }
        } else {
            RecoveryScheduleEffect::None
        }
    }

    fn persist_acknowledged(&mut self, revision: Revision, epoch: u64) -> RecoveryScheduleEffect {
        if epoch != self.epoch || self.in_flight_revision != Some(revision) {
            return RecoveryScheduleEffect::None;
        }
        self.in_flight_revision = None;
        self.last_persisted_revision = Some(revision);
        if self.dirty && self.current_revision == revision {
            // This dirty interval is covered until the next edit.
            self.dirty_since = None;
        }
        RecoveryScheduleEffect::None
    }

    fn persist_failed(
        &mut self,
        revision: Revision,
        epoch: u64,
        now: RecoveryClock,
    ) -> RecoveryScheduleEffect {
        if epoch != self.epoch || self.in_flight_revision != Some(revision) {
            return RecoveryScheduleEffect::None;
        }
        self.in_flight_revision = None;
        // Keep the dirty interval open so the next eligible tick retries.
        if self.dirty_since.is_none() {
            self.dirty_since = Some(now);
        }
        if self.last_edit_at.is_none() {
            self.last_edit_at = Some(now);
        }
        RecoveryScheduleEffect::None
    }
}

/// User choice for a startup recovery offer.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RecoveryOfferDecision {
    /// Open the recovered content as a dirty document without writing the path.
    Restore,
    /// Discard the recovery record and continue with a normal untitled document.
    Discard,
}

/// Pure startup offer state for at most one pending recovery decision.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct RecoveryOfferState {
    open: bool,
}

impl RecoveryOfferState {
    /// Returns whether a recovery decision is currently required.
    pub const fn is_open(self) -> bool {
        self.open
    }

    /// Opens the startup offer when a validated record exists.
    pub const fn present(&mut self) {
        self.open = true;
    }

    /// Applies an explicit user decision and closes the offer.
    pub const fn decide(&mut self, decision: RecoveryOfferDecision) -> RecoveryOfferDecision {
        self.open = false;
        decision
    }

    /// Clears a pending offer without restoring (for tests and path reset).
    pub const fn clear(&mut self) {
        self.open = false;
    }
}

fn validate_content_body(content: &[u8], bom: Bom) -> Result<&str, RecoveryQuarantineReason> {
    let (detected, body) = Bom::split_utf8(content);
    if detected != bom {
        // BOM tag must match the leading bytes so restore cannot invent a mark.
        return Err(RecoveryQuarantineReason::InvalidFormatTags);
    }
    std::str::from_utf8(body).map_err(|_| RecoveryQuarantineReason::InvalidUtf8)
}

const fn validate_selection(
    selection: Selection,
    body: &str,
) -> Result<(), RecoveryQuarantineReason> {
    let body_len = body.len();
    // Check endpoints separately so a single out-of-range side cannot hide
    // behind the other, and so range failures stay distinct from mid-codepoint
    // boundary failures (is_char_boundary is false for every index > len).
    if selection.anchor() > body_len {
        return Err(RecoveryQuarantineReason::InvalidSelection);
    }
    if selection.active() > body_len {
        return Err(RecoveryQuarantineReason::InvalidSelection);
    }
    // Mid-codepoint offsets would silently corrupt restore selection.
    if !body.is_char_boundary(selection.anchor()) {
        return Err(RecoveryQuarantineReason::InvalidSelection);
    }
    if !body.is_char_boundary(selection.active()) {
        return Err(RecoveryQuarantineReason::InvalidSelection);
    }
    Ok(())
}

fn read_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset + 4)?
        .try_into()
        .ok()
        .map(u32::from_le_bytes)
}

fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    bytes
        .get(offset..offset + 8)?
        .try_into()
        .ok()
        .map(u64::from_le_bytes)
}

fn read_array16(bytes: &[u8], offset: usize) -> Option<[u8; 16]> {
    bytes.get(offset..offset + 16)?.try_into().ok()
}

fn read_array32(bytes: &[u8], offset: usize) -> Option<[u8; 32]> {
    bytes.get(offset..offset + 32)?.try_into().ok()
}

const fn encode_bom(bom: Bom) -> u8 {
    match bom {
        Bom::Absent => 0,
        Bom::Utf8 => 1,
    }
}

const fn decode_bom(value: u8) -> Option<Bom> {
    match value {
        0 => Some(Bom::Absent),
        1 => Some(Bom::Utf8),
        _ => None,
    }
}

const fn encode_encoding(encoding: Encoding) -> u8 {
    match encoding {
        Encoding::Utf8 => 0,
    }
}

const fn decode_encoding(value: u8) -> Option<Encoding> {
    match value {
        0 => Some(Encoding::Utf8),
        _ => None,
    }
}

const fn usize_to_u64(value: usize) -> u64 {
    value as u64
}

fn u64_to_usize(value: u64) -> Option<usize> {
    usize::try_from(value).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn sample_snapshot(content: &[u8], selection: Selection) -> RecoverySnapshot {
        RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(3),
            created_at: RecoveryWallTime::from_unix_millis(100),
            updated_at: RecoveryWallTime::from_unix_millis(200),
            original_path: b"notes.md".to_vec(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection,
            content: content.to_vec(),
        })
        .expect("fixture snapshot")
    }

    fn encode_v1(snapshot: &RecoverySnapshot) -> Vec<u8> {
        let checksum = ContentFingerprint::from_bytes(snapshot.content());
        let path_len = u32::try_from(snapshot.original_path().len()).expect("fixture path length");
        let content_len = u64::try_from(snapshot.content().len()).expect("fixture content length");
        let mut out = Vec::with_capacity(
            V1_FIXED_HEADER_LEN + snapshot.original_path().len() + snapshot.content().len(),
        );
        out.extend_from_slice(RECOVERY_MAGIC);
        out.extend_from_slice(&LEGACY_RECOVERY_SCHEMA_VERSION.to_le_bytes());
        out.extend_from_slice(&snapshot.document_id().as_bytes());
        out.extend_from_slice(&snapshot.instance_id().as_bytes());
        out.extend_from_slice(&snapshot.revision().get().to_le_bytes());
        out.extend_from_slice(&snapshot.created_at().unix_millis().to_le_bytes());
        out.extend_from_slice(&snapshot.updated_at().unix_millis().to_le_bytes());
        out.extend_from_slice(&path_len.to_le_bytes());
        out.push(encode_bom(snapshot.bom()));
        out.push(encode_encoding(snapshot.encoding()));
        out.extend_from_slice(&usize_to_u64(snapshot.selection().anchor()).to_le_bytes());
        out.extend_from_slice(&usize_to_u64(snapshot.selection().active()).to_le_bytes());
        out.extend_from_slice(&content_len.to_le_bytes());
        out.extend_from_slice(checksum.as_bytes());
        out.extend_from_slice(snapshot.original_path());
        out.extend_from_slice(snapshot.content());
        out
    }

    #[test]
    fn encode_round_trip_preserves_record() {
        let snapshot = sample_snapshot(b"hello\r\nworld", Selection::new(1, 5));
        let encoded = snapshot.encode();
        match validate_recovery_record(&encoded) {
            RecoveryStartupDisposition::Offer(record) => {
                assert_eq!(record.schema_version(), RECOVERY_SCHEMA_VERSION);
                assert_eq!(record.document_id(), snapshot.document_id());
                assert_eq!(record.instance_id(), snapshot.instance_id());
                assert_eq!(
                    record.lineage_generation(),
                    Some(RecoveryLineageGeneration::ROOT)
                );
                assert_eq!(record.predecessor_instance(), None);
                assert_eq!(record.revision(), snapshot.revision());
                assert_eq!(record.created_at(), snapshot.created_at());
                assert_eq!(record.updated_at(), snapshot.updated_at());
                assert_eq!(record.original_path(), snapshot.original_path());
                assert_eq!(record.bom(), snapshot.bom());
                assert_eq!(record.encoding(), snapshot.encoding());
                assert_eq!(record.selection(), snapshot.selection());
                assert_eq!(record.content(), snapshot.content());
                assert_eq!(
                    record.content_checksum(),
                    ContentFingerprint::from_bytes(snapshot.content())
                );
                assert_eq!(record.metadata().content_len(), snapshot.content().len());
                assert_eq!(
                    validate_recovery_metadata(&encoded).expect("metadata"),
                    record.metadata().clone()
                );
            }
            RecoveryStartupDisposition::Quarantine(reason) => {
                panic!("expected offer, got quarantine {reason:?}")
            }
        }
    }

    #[test]
    fn v2_round_trip_preserves_causal_lineage_and_protects_it() {
        let predecessor = RecoveryInstanceId::new([7; 16]);
        let snapshot = RecoverySnapshot::try_new_with_lineage(
            RecoverySnapshotParts {
                document_id: RecoveryDocumentId::new([1; 16]),
                instance_id: RecoveryInstanceId::new([2; 16]),
                revision: Revision::new(3),
                created_at: RecoveryWallTime::from_unix_millis(100),
                updated_at: RecoveryWallTime::from_unix_millis(1),
                original_path: b"notes.md".to_vec(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::caret(2),
                content: b"newer".to_vec(),
            },
            RecoveryLineageGeneration::new(9),
            Some(predecessor),
        )
        .expect("lineage snapshot");
        let encoded = snapshot.encode();
        let RecoveryStartupDisposition::Offer(record) = validate_recovery_record(&encoded) else {
            panic!("schema v2 lineage must validate");
        };
        assert_eq!(
            record.lineage_generation(),
            Some(RecoveryLineageGeneration::new(9))
        );
        assert_eq!(record.predecessor_instance(), Some(predecessor));
        assert_ne!(record.record_checksum(), record.content_checksum());

        let mut generation_tamper = encoded.clone();
        generation_tamper[44] ^= 1;
        assert_eq!(
            validate_recovery_record(&generation_tamper),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::ChecksumMismatch)
        );

        let mut predecessor_tamper = encoded;
        predecessor_tamper[53] ^= 1;
        assert_eq!(
            validate_recovery_record(&predecessor_tamper),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::ChecksumMismatch)
        );
    }

    #[test]
    fn recovery_header_boundaries_and_lineage_rules_are_independently_observable() {
        let root = sample_snapshot(b"root", Selection::caret(0));
        let legacy = encode_v1(&root);
        assert!(parse_recovery_header(&legacy[..V1_FIXED_HEADER_LEN]).is_ok());
        assert!(matches!(
            parse_recovery_header(&legacy[..V1_FIXED_HEADER_LEN - 1]),
            Err(RecoveryQuarantineReason::Truncated)
        ));

        let current = root.encode();
        assert!(parse_recovery_header(&current[..V2_FIXED_HEADER_LEN]).is_ok());

        let mut nonzero_absent_predecessor = root.encode();
        nonzero_absent_predecessor[53] = 1;
        assert!(matches!(
            parse_recovery_header(&nonzero_absent_predecessor),
            Err(RecoveryQuarantineReason::InvalidLineage)
        ));

        let predecessor = RecoveryInstanceId::new([7; 16]);
        let successor = RecoverySnapshot::try_new_with_lineage(
            RecoverySnapshotParts {
                document_id: root.document_id(),
                instance_id: root.instance_id(),
                revision: Revision::new(4),
                created_at: RecoveryWallTime::from_unix_millis(1),
                updated_at: RecoveryWallTime::from_unix_millis(2),
                original_path: Vec::new(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::caret(0),
                content: b"successor".to_vec(),
            },
            RecoveryLineageGeneration::new(1),
            Some(predecessor),
        )
        .expect("valid successor fixture");
        let mut self_predecessor = successor.encode();
        self_predecessor[53..69].copy_from_slice(&successor.instance_id().as_bytes());
        assert!(matches!(
            parse_recovery_header(&self_predecessor),
            Err(RecoveryQuarantineReason::InvalidLineage)
        ));
    }

    #[test]
    fn schema_v1_remains_readable_with_its_content_only_checksum() {
        let snapshot = sample_snapshot(b"legacy recovery", Selection::caret(6));
        let encoded = encode_v1(&snapshot);
        let RecoveryStartupDisposition::Offer(record) = validate_recovery_record(&encoded) else {
            panic!("schema v1 must remain readable");
        };
        assert_eq!(record.schema_version(), LEGACY_RECOVERY_SCHEMA_VERSION);
        assert_eq!(record.lineage_generation(), None);
        assert_eq!(record.predecessor_instance(), None);
        assert_eq!(record.content(), snapshot.content());
        assert_eq!(record.record_checksum(), record.content_checksum());

        let mut legacy_header_tamper = encoded.clone();
        legacy_header_tamper[60] ^= 1;
        assert!(matches!(
            validate_recovery_record(&legacy_header_tamper),
            RecoveryStartupDisposition::Offer(_)
        ));

        let mut legacy_content_tamper = encoded;
        let last = legacy_content_tamper.len() - 1;
        legacy_content_tamper[last] ^= 1;
        assert_eq!(
            validate_recovery_record(&legacy_content_tamper),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::ChecksumMismatch)
        );
    }

    #[test]
    fn lineage_generation_and_predecessor_invariants_are_enforced() {
        assert_eq!(
            RecoveryLineageGeneration::new(8).checked_next(),
            Some(RecoveryLineageGeneration::new(9))
        );
        assert_eq!(
            RecoveryLineageGeneration::new(u64::MAX).checked_next(),
            None
        );

        let parts = RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(2),
            original_path: Vec::new(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: b"safe".to_vec(),
        };
        let instance_id = parts.instance_id;
        assert_eq!(
            RecoverySnapshot::try_new_with_lineage(
                parts.clone(),
                RecoveryLineageGeneration::new(1),
                Some(instance_id)
            ),
            Err(RecoveryQuarantineReason::InvalidLineage)
        );
        assert_eq!(
            RecoverySnapshot::try_new_with_lineage(
                parts.clone(),
                RecoveryLineageGeneration::ROOT,
                Some(RecoveryInstanceId::new([3; 16]))
            ),
            Err(RecoveryQuarantineReason::InvalidLineage)
        );
        assert_eq!(
            RecoverySnapshot::try_new_with_lineage(parts, RecoveryLineageGeneration::new(1), None),
            Err(RecoveryQuarantineReason::InvalidLineage)
        );
    }

    #[test]
    fn successor_constructor_advances_v2_and_legacy_lineage() {
        let predecessor = sample_snapshot(b"parent", Selection::caret(2));
        let RecoveryStartupDisposition::Offer(parent_record) =
            validate_recovery_record(&predecessor.encode())
        else {
            panic!("parent record");
        };
        let parts = RecoverySnapshotParts {
            document_id: parent_record.document_id(),
            instance_id: RecoveryInstanceId::new([9; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(3),
            updated_at: RecoveryWallTime::from_unix_millis(4),
            original_path: Vec::new(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: b"child".to_vec(),
        };
        let successor = RecoverySnapshot::try_new_successor(parts, parent_record.metadata())
            .expect("v2 successor");
        assert_eq!(
            successor.lineage_generation(),
            RecoveryLineageGeneration::new(1)
        );
        assert_eq!(
            successor.predecessor_instance(),
            Some(parent_record.instance_id())
        );

        let legacy_bytes = encode_v1(&predecessor);
        let RecoveryStartupDisposition::Offer(legacy_record) =
            validate_recovery_record(&legacy_bytes)
        else {
            panic!("legacy parent record");
        };
        let legacy_parts = RecoverySnapshotParts {
            document_id: legacy_record.document_id(),
            instance_id: RecoveryInstanceId::new([8; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(5),
            updated_at: RecoveryWallTime::from_unix_millis(6),
            original_path: Vec::new(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: b"legacy child".to_vec(),
        };
        let legacy_successor =
            RecoverySnapshot::try_new_successor(legacy_parts, legacy_record.metadata())
                .expect("legacy successor");
        assert_eq!(
            legacy_successor.lineage_generation(),
            RecoveryLineageGeneration::new(1)
        );
        assert_eq!(
            legacy_successor.predecessor_instance(),
            Some(legacy_record.instance_id())
        );
    }

    #[test]
    fn bom_content_round_trips_with_matching_tag() {
        let mut content = Bom::UTF8_BYTES.to_vec();
        content.extend_from_slice("café".as_bytes());
        let snapshot = RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([3; 16]),
            instance_id: RecoveryInstanceId::new([4; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(2),
            original_path: Vec::new(),
            bom: Bom::Utf8,
            encoding: Encoding::Utf8,
            // "café" is five UTF-8 body bytes; caret must sit on a char boundary.
            selection: Selection::caret(5),
            content: content.clone(),
        })
        .expect("bom snapshot");
        let encoded = snapshot.encode();
        let RecoveryStartupDisposition::Offer(record) = validate_recovery_record(&encoded) else {
            panic!("expected offer");
        };
        assert_eq!(record.bom(), Bom::Utf8);
        assert_eq!(record.content(), content);
        assert_eq!(record.selection(), Selection::caret(5));
    }

    #[test]
    fn invalid_magic_and_schema_are_quarantined() {
        let mut bytes = sample_snapshot(b"x", Selection::caret(0)).encode();
        bytes[0] = b'X';
        assert_eq!(
            validate_recovery_record(&bytes),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::InvalidMagic)
        );

        let mut bytes = sample_snapshot(b"x", Selection::caret(0)).encode();
        bytes[8..12].copy_from_slice(&99u32.to_le_bytes());
        assert_eq!(
            validate_recovery_record(&bytes),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::UnknownSchema)
        );
    }

    #[test]
    fn checksum_mismatch_and_truncation_are_quarantined() {
        let mut bytes = sample_snapshot(b"hello", Selection::caret(0)).encode();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xFF;
        assert_eq!(
            validate_recovery_record(&bytes),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::ChecksumMismatch)
        );

        let bytes = sample_snapshot(b"hello", Selection::caret(0)).encode();
        assert_eq!(
            validate_recovery_record(&bytes[..bytes.len() - 1]),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::Truncated)
        );
        assert_eq!(
            validate_recovery_record(&[]),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::Truncated)
        );
    }

    #[test]
    fn invalid_utf8_and_selection_are_quarantined() {
        let err = RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(1),
            original_path: Vec::new(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: vec![0xFF],
        });
        assert_eq!(err, Err(RecoveryQuarantineReason::InvalidUtf8));

        let err = RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(1),
            original_path: Vec::new(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(10),
            content: b"hi".to_vec(),
        });
        assert_eq!(err, Err(RecoveryQuarantineReason::InvalidSelection));
    }

    #[test]
    fn mismatched_bom_tag_is_rejected() {
        let err = RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(1),
            original_path: Vec::new(),
            bom: Bom::Utf8,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: b"no bom".to_vec(),
        });
        assert_eq!(err, Err(RecoveryQuarantineReason::InvalidFormatTags));
    }

    #[test]
    fn path_and_content_ceilings_are_enforced() {
        assert_eq!(MAX_RECOVERY_PATH_BYTES, 128 * 1024);
        assert_eq!(MAX_RECOVERY_PATH_BYTES, 131_072);

        let path = vec![b'a'; MAX_RECOVERY_PATH_BYTES + 1];
        let err = RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(1),
            original_path: path,
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: b"x".to_vec(),
        });
        assert_eq!(err, Err(RecoveryQuarantineReason::PathTooLarge));

        // Exact ceiling must still be accepted (strict greater-than, not >=).
        let exact_path = vec![b'b'; MAX_RECOVERY_PATH_BYTES];
        RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(1),
            original_path: exact_path,
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: b"x".to_vec(),
        })
        .expect("exact path ceiling is allowed");

        let content = vec![b'x'; MAX_DOCUMENT_BYTES + 1];
        let err = RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(1),
            original_path: Vec::new(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content,
        });
        assert_eq!(err, Err(RecoveryQuarantineReason::ContentTooLarge));

        let exact_content = vec![b'y'; MAX_DOCUMENT_BYTES];
        RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(1),
            original_path: Vec::new(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: exact_content,
        })
        .expect("exact document ceiling is allowed");
    }

    #[test]
    fn identities_and_descriptions_are_exact() {
        let id = RecoveryDocumentId::new([9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 1, 2, 3, 4, 5, 6]);
        assert_eq!(
            id.as_bytes(),
            [9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 1, 2, 3, 4, 5, 6]
        );
        let instance =
            RecoveryInstanceId::new([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
        assert_eq!(
            instance.as_bytes(),
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
        assert_eq!(
            RecoveryQuarantineReason::InvalidMagic.description(),
            "The recovery file is not a Noter recovery record."
        );
        assert_eq!(
            RecoveryQuarantineReason::ChecksumMismatch.description(),
            "The recovery record failed its integrity check."
        );
    }

    #[test]
    fn encoded_path_and_content_ceilings_match_try_new() {
        let exact_path = vec![b'p'; MAX_RECOVERY_PATH_BYTES];
        let snapshot = RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(2),
            original_path: exact_path.clone(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: b"ok".to_vec(),
        })
        .expect("exact path");
        let encoded = snapshot.encode();
        let RecoveryStartupDisposition::Offer(record) = validate_recovery_record(&encoded) else {
            panic!("exact path must validate");
        };
        assert_eq!(record.original_path(), exact_path.as_slice());

        // Inflate path_len beyond the ceiling while keeping a short payload.
        let mut bad = sample_snapshot(b"x", Selection::caret(0)).encode();
        bad[93..97].copy_from_slice(&(MAX_RECOVERY_PATH_BYTES as u32 + 1).to_le_bytes());
        assert_eq!(
            validate_recovery_record(&bad),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::PathTooLarge)
        );

        let mut bad = sample_snapshot(b"x", Selection::caret(0)).encode();
        bad[115..123].copy_from_slice(&(MAX_DOCUMENT_BYTES as u64 + 1).to_le_bytes());
        assert_eq!(
            validate_recovery_record(&bad),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::ContentTooLarge)
        );
    }

    #[test]
    fn the_next_persist_delay_matches_the_moment_a_tick_becomes_due() {
        fn clock(seconds: f64) -> RecoveryClock {
            RecoveryClock::new(Duration::from_secs_f64(seconds))
        }

        let mut state = RecoveryScheduleState::default();
        // A clean document needs no timer at all, so the window can sleep.
        assert_eq!(state.next_persist_delay(clock(0.0)), None);

        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(1),
            now: clock(0.0),
        });
        assert_eq!(
            state.next_persist_delay(clock(0.0)),
            Some(RECOVERY_IDLE_DEBOUNCE)
        );
        assert_eq!(
            state.next_persist_delay(clock(1.5)),
            Some(Duration::from_millis(500))
        );
        // Waking at the reported delay must find the tick due.
        assert!(matches!(
            state.reduce(RecoveryScheduleCommand::Tick { now: clock(2.0) }),
            RecoveryScheduleEffect::Persist { .. }
        ));
        // A dirty document with a persist in flight on a worker still needs a
        // short poll so the completion can be applied without blocking typing.
        assert!(state.is_dirty());
        assert_eq!(state.in_flight_revision(), Some(Revision::new(1)));
        assert_eq!(
            state.next_persist_delay(clock(2.0)),
            Some(RECOVERY_IN_FLIGHT_POLL)
        );

        // Continuous typing is capped by the maximum dirty interval, not the
        // idle debounce, so the reported delay must follow the earlier bound.
        let mut typing = RecoveryScheduleState::default();
        for second in 0..14_u32 {
            let _ = typing.reduce(RecoveryScheduleCommand::Edited {
                revision: Revision::new(u64::from(second) + 1),
                now: clock(f64::from(second)),
            });
        }
        assert_eq!(
            typing.next_persist_delay(clock(13.0)),
            Some(Duration::from_secs(2))
        );

        // A clock regression is already due rather than a long sleep.
        assert_eq!(typing.next_persist_delay(clock(0.0)), Some(Duration::ZERO));
    }

    #[test]
    fn selection_checks_range_and_char_boundaries_independently() {
        // Active out of range only.
        assert_eq!(
            RecoverySnapshot::try_new(RecoverySnapshotParts {
                document_id: RecoveryDocumentId::new([1; 16]),
                instance_id: RecoveryInstanceId::new([2; 16]),
                revision: Revision::new(1),
                created_at: RecoveryWallTime::from_unix_millis(1),
                updated_at: RecoveryWallTime::from_unix_millis(1),
                original_path: Vec::new(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::new(0, 3),
                content: b"hi".to_vec(),
            }),
            Err(RecoveryQuarantineReason::InvalidSelection)
        );
        // Anchor out of range only.
        assert_eq!(
            RecoverySnapshot::try_new(RecoverySnapshotParts {
                document_id: RecoveryDocumentId::new([1; 16]),
                instance_id: RecoveryInstanceId::new([2; 16]),
                revision: Revision::new(1),
                created_at: RecoveryWallTime::from_unix_millis(1),
                updated_at: RecoveryWallTime::from_unix_millis(1),
                original_path: Vec::new(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::new(3, 0),
                content: b"hi".to_vec(),
            }),
            Err(RecoveryQuarantineReason::InvalidSelection)
        );
        // Mid-codepoint only (in range for length but not a boundary).
        assert_eq!(
            RecoverySnapshot::try_new(RecoverySnapshotParts {
                document_id: RecoveryDocumentId::new([1; 16]),
                instance_id: RecoveryInstanceId::new([2; 16]),
                revision: Revision::new(1),
                created_at: RecoveryWallTime::from_unix_millis(1),
                updated_at: RecoveryWallTime::from_unix_millis(1),
                original_path: Vec::new(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::new(1, 0),
                content: "é".as_bytes().to_vec(),
            }),
            Err(RecoveryQuarantineReason::InvalidSelection)
        );
        // Active mid-codepoint only.
        assert_eq!(
            RecoverySnapshot::try_new(RecoverySnapshotParts {
                document_id: RecoveryDocumentId::new([1; 16]),
                instance_id: RecoveryInstanceId::new([2; 16]),
                revision: Revision::new(1),
                created_at: RecoveryWallTime::from_unix_millis(1),
                updated_at: RecoveryWallTime::from_unix_millis(1),
                original_path: Vec::new(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::new(0, 1),
                content: "é".as_bytes().to_vec(),
            }),
            Err(RecoveryQuarantineReason::InvalidSelection)
        );
    }

    #[test]
    fn encoded_exact_document_ceiling_validates() {
        // 64 MiB fixture: keeps validate_recovery_record's content_len > ceiling
        // branch distinct from >= (exact size must still Offer).
        let exact_content = vec![b'y'; MAX_DOCUMENT_BYTES];
        let snapshot = RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(2),
            original_path: Vec::new(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: exact_content,
        })
        .expect("exact document ceiling is allowed at try_new");
        let encoded = snapshot.encode();
        match validate_recovery_record(&encoded) {
            RecoveryStartupDisposition::Offer(record) => {
                assert_eq!(record.content().len(), MAX_DOCUMENT_BYTES);
            }
            RecoveryStartupDisposition::Quarantine(reason) => {
                panic!("exact content ceiling must validate, got {reason:?}")
            }
        }
    }

    #[test]
    fn persist_ack_requires_matching_epoch_and_revision() {
        let mut state = RecoveryScheduleState::default();
        let epoch = state.epoch();
        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(1),
            now: RecoveryClock::new(Duration::from_secs(0)),
        });
        assert!(state.is_dirty());
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick {
                now: RecoveryClock::new(Duration::from_secs(2))
            }),
            RecoveryScheduleEffect::Persist {
                revision: Revision::new(1),
                epoch,
            }
        );

        // Matching revision with the wrong epoch must not clear in-flight state.
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistAcknowledged {
                revision: Revision::new(1),
                epoch: epoch.wrapping_add(1),
            }),
            RecoveryScheduleEffect::None
        );
        assert_eq!(state.in_flight_revision(), Some(Revision::new(1)));
        assert!(state.is_dirty());

        // Matching epoch with the wrong revision is also ignored.
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistAcknowledged {
                revision: Revision::new(2),
                epoch,
            }),
            RecoveryScheduleEffect::None
        );
        assert_eq!(state.in_flight_revision(), Some(Revision::new(1)));

        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistAcknowledged {
                revision: Revision::new(1),
                epoch,
            }),
            RecoveryScheduleEffect::None
        );
        assert!(state.in_flight_revision().is_none());
        assert!(state.is_dirty());
        assert_eq!(state.last_persisted_revision(), Some(Revision::new(1)));
    }

    #[test]
    fn matching_persist_ack_clears_dirty_since_so_max_interval_restarts() {
        // After a successful ack of the current dirty revision, dirty_since must
        // clear. The next edit starts a fresh max-interval window. If ack failed
        // to clear dirty_since, continuous edits would hit the old window early.
        let mut state = RecoveryScheduleState::default();
        let epoch = state.epoch();
        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(1),
            now: RecoveryClock::new(Duration::from_secs(0)),
        });
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick {
                now: RecoveryClock::new(Duration::from_secs(2))
            }),
            RecoveryScheduleEffect::Persist {
                revision: Revision::new(1),
                epoch,
            }
        );
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistAcknowledged {
                revision: Revision::new(1),
                epoch,
            }),
            RecoveryScheduleEffect::None
        );

        // New dirty interval begins at t=3.
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Edited {
                revision: Revision::new(2),
                now: RecoveryClock::new(Duration::from_secs(3)),
            }),
            RecoveryScheduleEffect::None
        );

        // Continuous edits every second: from dirty_since=3 the max interval is
        // due at t=18. At t=15 (12s dirty) it must still be idle.
        for step in 4..=15 {
            assert_eq!(
                state.reduce(RecoveryScheduleCommand::Edited {
                    revision: Revision::new(step),
                    now: RecoveryClock::new(Duration::from_secs(step)),
                }),
                RecoveryScheduleEffect::None,
                "step {step} must not fire the old dirty_since window"
            );
        }

        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Edited {
                revision: Revision::new(18),
                now: RecoveryClock::new(Duration::from_secs(18)),
            }),
            RecoveryScheduleEffect::Persist {
                revision: Revision::new(18),
                epoch,
            }
        );
    }

    #[test]
    fn stale_in_flight_ack_preserves_dirty_since_for_newer_revision() {
        // Edit while a persist is in flight advances current_revision. Acking the
        // older in-flight revision must not clear dirty_since; otherwise ticks
        // would skip scheduling until another edit re-arms the interval.
        let mut state = RecoveryScheduleState::default();
        let epoch = state.epoch();
        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(1),
            now: RecoveryClock::new(Duration::from_secs(0)),
        });
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick {
                now: RecoveryClock::new(Duration::from_secs(2))
            }),
            RecoveryScheduleEffect::Persist {
                revision: Revision::new(1),
                epoch,
            }
        );
        // Newer edit while rev 1 is still in flight.
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Edited {
                revision: Revision::new(2),
                now: RecoveryClock::new(Duration::from_millis(2_500)),
            }),
            RecoveryScheduleEffect::None
        );
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistAcknowledged {
                revision: Revision::new(1),
                epoch,
            }),
            RecoveryScheduleEffect::None
        );
        assert_eq!(state.last_persisted_revision(), Some(Revision::new(1)));
        assert_eq!(state.current_revision(), Revision::new(2));

        // Idle debounce from the newer edit must still schedule rev 2.
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick {
                now: RecoveryClock::new(Duration::from_millis(4_500))
            }),
            RecoveryScheduleEffect::Persist {
                revision: Revision::new(2),
                epoch,
            }
        );
    }

    #[test]
    fn persist_failed_requires_matching_epoch_and_revision() {
        let mut state = RecoveryScheduleState::default();
        let epoch = state.epoch();
        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(3),
            now: RecoveryClock::new(Duration::from_secs(0)),
        });
        let _ = state.reduce(RecoveryScheduleCommand::Tick {
            now: RecoveryClock::new(Duration::from_secs(2)),
        });

        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistFailed {
                revision: Revision::new(3),
                epoch: epoch.wrapping_add(9),
                now: RecoveryClock::new(Duration::from_secs(3)),
            }),
            RecoveryScheduleEffect::None
        );
        assert_eq!(state.in_flight_revision(), Some(Revision::new(3)));

        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistFailed {
                revision: Revision::new(99),
                epoch,
                now: RecoveryClock::new(Duration::from_secs(3)),
            }),
            RecoveryScheduleEffect::None
        );
        assert_eq!(state.in_flight_revision(), Some(Revision::new(3)));

        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistFailed {
                revision: Revision::new(3),
                epoch,
                now: RecoveryClock::new(Duration::from_secs(3)),
            }),
            RecoveryScheduleEffect::None
        );
        assert!(state.in_flight_revision().is_none());
        assert!(state.is_dirty());
    }

    #[test]
    fn idle_debounce_and_max_interval_schedule_persist() {
        let mut state = RecoveryScheduleState::default();
        let epoch = state.epoch();
        let t0 = RecoveryClock::new(Duration::from_secs(0));
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Edited {
                revision: Revision::new(1),
                now: t0,
            }),
            RecoveryScheduleEffect::None
        );

        let almost_idle = RecoveryClock::new(Duration::from_millis(1_999));
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick { now: almost_idle }),
            RecoveryScheduleEffect::None
        );

        let idle = RecoveryClock::new(Duration::from_secs(2));
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick { now: idle }),
            RecoveryScheduleEffect::Persist {
                revision: Revision::new(1),
                epoch,
            }
        );
        assert_eq!(state.in_flight_revision(), Some(Revision::new(1)));

        // While in flight, further ticks do not double-schedule.
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick {
                now: RecoveryClock::new(Duration::from_secs(3))
            }),
            RecoveryScheduleEffect::None
        );

        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistAcknowledged {
                revision: Revision::new(1),
                epoch,
            }),
            RecoveryScheduleEffect::None
        );
        assert_eq!(state.last_persisted_revision(), Some(Revision::new(1)));
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick {
                now: RecoveryClock::new(Duration::from_secs(20))
            }),
            RecoveryScheduleEffect::None
        );
    }

    #[test]
    fn continuous_editing_hits_max_dirty_interval() {
        let mut state = RecoveryScheduleState::default();
        let epoch = state.epoch();
        let mut now = Duration::ZERO;
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Edited {
                revision: Revision::new(1),
                now: RecoveryClock::new(now),
            }),
            RecoveryScheduleEffect::None
        );

        // Keep editing every second so idle debounce never fires.
        for step in 2..=15 {
            now = Duration::from_secs(step);
            let revision = Revision::new(step);
            let effect = state.reduce(RecoveryScheduleCommand::Edited {
                revision,
                now: RecoveryClock::new(now),
            });
            if step < 15 {
                assert_eq!(effect, RecoveryScheduleEffect::None, "step {step}");
            } else {
                assert_eq!(
                    effect,
                    RecoveryScheduleEffect::Persist {
                        revision: Revision::new(15),
                        epoch,
                    }
                );
            }
        }
    }

    #[test]
    fn clean_and_discard_request_delete() {
        let mut state = RecoveryScheduleState::default();
        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(2),
            now: RecoveryClock::new(Duration::from_secs(0)),
        });
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::BecameClean {
                revision: Revision::new(2)
            }),
            RecoveryScheduleEffect::DeleteOwned { epoch: 1 }
        );
        assert!(!state.is_dirty());
        assert_eq!(state.epoch(), 1);

        let mut state = RecoveryScheduleState::default();
        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(4),
            now: RecoveryClock::new(Duration::from_secs(0)),
        });
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Discarded),
            RecoveryScheduleEffect::DeleteOwned { epoch: 1 }
        );
        assert!(!state.is_dirty());
        assert_eq!(state.current_revision(), Revision::INITIAL);
    }

    #[test]
    fn new_identity_clears_document_state_and_advances_epoch() {
        let mut state = RecoveryScheduleState::default();
        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(7),
            now: RecoveryClock::new(Duration::from_secs(0)),
        });
        let _ = state.reduce(RecoveryScheduleCommand::Tick {
            now: RecoveryClock::new(Duration::from_secs(2)),
        });
        let previous_epoch = state.epoch();

        state.reset_for_new_identity();

        assert_eq!(state.epoch(), previous_epoch.wrapping_add(1));
        assert!(!state.is_dirty());
        assert_eq!(state.current_revision(), Revision::INITIAL);
        assert!(state.in_flight_revision().is_none());
        assert!(state.last_persisted_revision().is_none());
    }

    #[test]
    fn stale_persist_ack_is_ignored_and_failure_retries() {
        let mut state = RecoveryScheduleState::default();
        let epoch = state.epoch();
        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(1),
            now: RecoveryClock::new(Duration::from_secs(0)),
        });
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick {
                now: RecoveryClock::new(Duration::from_secs(2))
            }),
            RecoveryScheduleEffect::Persist {
                revision: Revision::new(1),
                epoch,
            }
        );

        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistAcknowledged {
                revision: Revision::new(99),
                epoch,
            }),
            RecoveryScheduleEffect::None
        );
        assert_eq!(state.in_flight_revision(), Some(Revision::new(1)));

        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistFailed {
                revision: Revision::new(1),
                epoch,
                now: RecoveryClock::new(Duration::from_secs(3)),
            }),
            RecoveryScheduleEffect::None
        );
        assert!(state.in_flight_revision().is_none());

        // After failure, idle debounce from the last edit time can fire again.
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick {
                now: RecoveryClock::new(Duration::from_secs(5))
            }),
            RecoveryScheduleEffect::Persist {
                revision: Revision::new(1),
                epoch,
            }
        );
    }

    #[test]
    fn clean_invalidates_in_flight_epoch() {
        let mut state = RecoveryScheduleState::default();
        let epoch = state.epoch();
        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(1),
            now: RecoveryClock::new(Duration::from_secs(0)),
        });
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick {
                now: RecoveryClock::new(Duration::from_secs(2))
            }),
            RecoveryScheduleEffect::Persist {
                revision: Revision::new(1),
                epoch,
            }
        );
        let _ = state.reduce(RecoveryScheduleCommand::BecameClean {
            revision: Revision::new(1),
        });
        // Late ack from the pre-clean epoch must not re-arm recovery state.
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::PersistAcknowledged {
                revision: Revision::new(1),
                epoch,
            }),
            RecoveryScheduleEffect::None
        );
        assert!(state.last_persisted_revision().is_none());
        assert!(!state.is_dirty());
    }

    #[test]
    fn clock_regression_still_schedules_persist() {
        let mut state = RecoveryScheduleState::default();
        let epoch = state.epoch();
        let _ = state.reduce(RecoveryScheduleCommand::Edited {
            revision: Revision::new(1),
            now: RecoveryClock::new(Duration::from_secs(10)),
        });
        assert_eq!(
            state.reduce(RecoveryScheduleCommand::Tick {
                now: RecoveryClock::new(Duration::from_secs(1))
            }),
            RecoveryScheduleEffect::Persist {
                revision: Revision::new(1),
                epoch,
            }
        );
    }

    #[test]
    fn mid_codepoint_selection_is_rejected() {
        let content = "é".as_bytes(); // two UTF-8 bytes
        let err = RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([2; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(1),
            original_path: Vec::new(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(1),
            content: content.to_vec(),
        });
        assert_eq!(err, Err(RecoveryQuarantineReason::InvalidSelection));
    }

    #[test]
    fn recovery_offer_state_requires_explicit_decision() {
        let mut offer = RecoveryOfferState::default();
        assert!(!offer.is_open());
        offer.present();
        assert!(offer.is_open());
        assert_eq!(
            offer.decide(RecoveryOfferDecision::Restore),
            RecoveryOfferDecision::Restore
        );
        assert!(!offer.is_open());
        offer.present();
        assert_eq!(
            offer.decide(RecoveryOfferDecision::Discard),
            RecoveryOfferDecision::Discard
        );
        offer.present();
        offer.clear();
        assert!(!offer.is_open());
    }

    #[test]
    fn quarantine_reasons_have_nonempty_descriptions() {
        for reason in [
            RecoveryQuarantineReason::InvalidMagic,
            RecoveryQuarantineReason::UnknownSchema,
            RecoveryQuarantineReason::Truncated,
            RecoveryQuarantineReason::PathTooLarge,
            RecoveryQuarantineReason::ContentTooLarge,
            RecoveryQuarantineReason::InvalidUtf8,
            RecoveryQuarantineReason::ChecksumMismatch,
            RecoveryQuarantineReason::InvalidSelection,
            RecoveryQuarantineReason::InvalidFormatTags,
            RecoveryQuarantineReason::InvalidLineage,
            RecoveryQuarantineReason::InstanceMismatch,
        ] {
            assert!(!reason.description().is_empty());
        }
    }

    fn floor_char_boundary(text: &str, index: usize) -> usize {
        let index = index.min(text.len());
        if text.is_char_boundary(index) {
            index
        } else {
            (0..index)
                .rev()
                .find(|&i| text.is_char_boundary(i))
                .unwrap_or(0)
        }
    }

    proptest! {
        #[test]
        fn encoded_records_round_trip(
            content in prop::collection::vec(prop::num::u8::ANY, 0..256)
                .prop_filter("utf8", |bytes| std::str::from_utf8(bytes).is_ok()),
            anchor in 0usize..256,
            active in 0usize..256,
            path in prop::collection::vec(prop::num::u8::ANY, 0..64),
        ) {
            let text = std::str::from_utf8(&content).expect("utf8 filter");
            let selection = Selection::new(
                floor_char_boundary(text, anchor),
                floor_char_boundary(text, active),
            );
            let snapshot = RecoverySnapshot::try_new(RecoverySnapshotParts {
                document_id: RecoveryDocumentId::new([7; 16]),
                instance_id: RecoveryInstanceId::new([8; 16]),
                revision: Revision::new(9),
                created_at: RecoveryWallTime::from_unix_millis(10),
                updated_at: RecoveryWallTime::from_unix_millis(11),
                original_path: path,
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection,
                content: content.clone(),
            })
            .expect("valid snapshot");
            let encoded = snapshot.encode();
            match validate_recovery_record(&encoded) {
                RecoveryStartupDisposition::Offer(record) => {
                    assert_eq!(record.content(), content.as_slice());
                    assert_eq!(record.selection(), selection);
                    assert_eq!(record.original_path(), snapshot.original_path());
                }
                RecoveryStartupDisposition::Quarantine(reason) => {
                    panic!("unexpected quarantine: {reason:?}")
                }
            }
        }
    }
}
