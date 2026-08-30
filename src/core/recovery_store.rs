//! Durable private storage for restart-spanning recovery records.
//!
//! Adapters own the recovery root directory. This module never writes a user
//! document path. Records are staged with private exclusive creation, synced,
//! then installed or replaced atomically. Corrupt records are moved into a
//! quarantine directory instead of being deleted silently. Quarantine failures
//! are reported on the scan entry rather than swallowed.

use std::fs::{self, File, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::Arc;

use getrandom::fill as fill_random;
#[cfg(windows)]
use noter_platform::WindowsRecoveryNamespace;
#[cfg(any(windows, test))]
use noter_platform::{CommitReceipt, ReplaceExistingOutcome};

use super::recovery::{
    RECOVERY_MAGIC, RECOVERY_SCHEMA_VERSION, RecoveryInstanceId, RecoveryQuarantineReason,
    RecoverySnapshot, RecoveryStartupDisposition, ValidatedRecoveryMetadata,
    ValidatedRecoveryRecord, validate_recovery_metadata, validate_recovery_record,
};
#[cfg(windows)]
use super::save::ContentFingerprint;

/// Subdirectory of the recovery root that holds active records.
pub const RECOVERY_RECORDS_DIR: &str = "records";

/// Subdirectory that holds quarantined corrupt or unsupported records.
pub const RECOVERY_QUARANTINE_DIR: &str = "quarantine";

/// Private recovery root beneath the platform-selected application state root.
pub const RECOVERY_STATE_SUBDIR: &str = "recovery";

/// Maximum number of restore offers presented from one startup scan.
///
/// A bounded scan reports omissions explicitly so callers never interpret the
/// returned set as globally complete.
pub const MAX_STARTUP_RECOVERY_OFFERS: usize = 32;

/// Maximum eligible recovery candidates inspected by one startup scan.
pub const MAX_STARTUP_RECOVERY_FILES: usize = 256;

/// Maximum raw directory entries enumerated by one startup scan.
///
/// Non-files, live markers, and records proven to belong to a living instance
/// do not consume the smaller eligible-candidate budget above. Stale live
/// markers are removed under an exclusive claim so bounded later scans make
/// progress through crash residue instead of revisiting it forever.
pub const MAX_STARTUP_RECOVERY_DIRECTORY_ENTRIES: usize = 1024;

/// Maximum aggregate encoded bytes read by one startup scan.
pub const MAX_STARTUP_RECOVERY_BYTES: u64 = 128 * 1024 * 1024;

/// Maximum quarantine results retained for the startup UI.
pub const MAX_STARTUP_QUARANTINE_RESULTS: usize = 32;

/// Maximum exact superseded artifacts retained behind one primary offer.
pub const MAX_SUPERSEDED_RECOVERY_HANDLES: usize = 16;

/// Maximum directory entries inspected by one owned-artifact cleanup.
const MAX_OWNED_RECOVERY_CLEANUP_FILES: usize = 256;

/// Maximum individual recovery file size accepted during a startup scan.
pub const MAX_RECOVERY_FILE_BYTES: u64 = 64 * 1024 * 1024 + 256 * 1024;

/// Opaque exact reference to one validated on-disk recovery artifact.
#[derive(Debug)]
pub struct RecoveryRecordHandle {
    path: PathBuf,
    metadata: ValidatedRecoveryMetadata,
    file: File,
    facts: noter_platform::FileFacts,
    encoded_len: u64,
}

impl RecoveryRecordHandle {
    /// Returns the validated metadata bound to this exact artifact.
    pub const fn metadata(&self) -> &ValidatedRecoveryMetadata {
        &self.metadata
    }
}

/// One bounded restore offer and exact causally superseded artifacts.
#[derive(Debug)]
pub struct RecoveryOffer {
    primary: Box<RecoveryRecordHandle>,
    superseded: Vec<RecoveryRecordHandle>,
    superseded_omitted: bool,
}

impl RecoveryOffer {
    /// Returns the primary exact artifact selected for restore.
    pub const fn primary(&self) -> &RecoveryRecordHandle {
        &self.primary
    }

    /// Returns validated metadata for the primary artifact.
    pub const fn metadata(&self) -> &ValidatedRecoveryMetadata {
        self.primary.metadata()
    }

    /// Returns exact older artifacts in deletion-safe order.
    ///
    /// Callers should delete these before deleting [`Self::primary`].
    pub fn superseded(&self) -> &[RecoveryRecordHandle] {
        &self.superseded
    }

    /// Returns whether additional superseded artifacts were intentionally left.
    pub const fn superseded_omitted(&self) -> bool {
        self.superseded_omitted
    }

    /// Consumes the offer into exact cleanup handles, primary last.
    pub fn into_cleanup_handles(mut self) -> Vec<RecoveryRecordHandle> {
        self.superseded.push(*self.primary);
        self.superseded
    }
}

/// Startup classification that never retains recovery document content.
#[derive(Debug)]
pub enum RecoveryScanDisposition {
    /// Offer one bounded metadata-only recovery lineage branch.
    Offer(RecoveryOffer),
    /// A corrupt or unsupported artifact was quarantined or reported.
    Quarantine(RecoveryQuarantineReason),
}

/// One bounded startup scan result paired with its best-known on-disk path.
#[derive(Debug)]
pub struct RecoveryScanEntry {
    path: PathBuf,
    disposition: RecoveryScanDisposition,
    /// Present when quarantine was required but the move failed.
    quarantine_error: Option<String>,
}

impl RecoveryScanEntry {
    /// Returns the best-known path of the artifact after the scan.
    ///
    /// For successfully quarantined records this is the quarantine path. When
    /// quarantine fails it remains the original records path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the metadata-only startup disposition for this path.
    pub const fn disposition(&self) -> &RecoveryScanDisposition {
        &self.disposition
    }

    /// Returns a short quarantine failure message when relocation failed.
    pub fn quarantine_error(&self) -> Option<&str> {
        self.quarantine_error.as_deref()
    }

    /// Returns whether the artifact still lives under the active records dir.
    pub fn remains_in_records(&self) -> bool {
        self.path
            .parent()
            .and_then(|parent| parent.file_name())
            .is_some_and(|name| name == RECOVERY_RECORDS_DIR)
    }

    /// Consumes the entry into path and disposition.
    pub fn into_parts(self) -> (PathBuf, RecoveryScanDisposition) {
        (self.path, self.disposition)
    }
}

/// Bounded result of one recovery startup scan.
#[derive(Debug, Default)]
pub struct RecoveryStartupScan {
    entries: Vec<RecoveryScanEntry>,
    omission_flags: u8,
    quarantine_results_omitted: usize,
}

const FILE_LIMIT_REACHED: u8 = 0b0_0001;
const BYTE_LIMIT_REACHED: u8 = 0b0_0010;
const OFFERS_OMITTED: u8 = 0b0_0100;
const SUPERSEDED_HANDLES_OMITTED: u8 = 0b0_1000;
const DIRECTORY_LIMIT_REACHED: u8 = 0b1_0000;

impl RecoveryStartupScan {
    const fn note_omission(&mut self, flag: u8) {
        self.omission_flags |= flag;
    }

    /// Returns bounded metadata offers and quarantine results.
    pub fn entries(&self) -> &[RecoveryScanEntry] {
        &self.entries
    }

    /// Consumes the scan into its bounded entries.
    pub fn into_entries(self) -> Vec<RecoveryScanEntry> {
        self.entries
    }

    /// Returns whether eligible recovery candidates exceeded their scan bound.
    pub const fn limit_reached(&self) -> bool {
        self.omission_flags & FILE_LIMIT_REACHED != 0
    }

    /// Returns whether raw directory entries exceeded their hard scan bound.
    pub const fn directory_limit_reached(&self) -> bool {
        self.omission_flags & DIRECTORY_LIMIT_REACHED != 0
    }

    /// Returns whether the aggregate encoded-byte scan budget was exhausted.
    pub const fn byte_limit_reached(&self) -> bool {
        self.omission_flags & BYTE_LIMIT_REACHED != 0
    }

    /// Returns whether valid incomparable offers exceeded the offer bound.
    pub const fn offers_omitted(&self) -> bool {
        self.omission_flags & OFFERS_OMITTED != 0
    }

    /// Returns whether exact superseded cleanup handles exceeded their bound.
    pub const fn superseded_handles_omitted(&self) -> bool {
        self.omission_flags & SUPERSEDED_HANDLES_OMITTED != 0
    }

    /// Returns the number of processed quarantine results omitted from the UI.
    pub const fn quarantine_results_omitted(&self) -> usize {
        self.quarantine_results_omitted
    }

    /// Returns whether any bounded scan result was intentionally omitted.
    pub const fn has_omissions(&self) -> bool {
        self.omission_flags != 0 || self.quarantine_results_omitted != 0
    }
}

/// Exclusive instance lease proving that one offered recovery instance is dead.
#[derive(Debug)]
pub struct RecoveryInstanceClaim {
    lease: RecoveryLiveLease,
}

#[derive(Debug)]
struct LockedLiveFile {
    file: File,
    facts: noter_platform::FileFacts,
}

/// Process-lifetime lease for one recovery instance.
///
/// Two independently locked paths keep one pathname rebind from hiding a live
/// owner. Both objects remain locked until facts-bound release completes.
#[derive(Debug)]
pub struct RecoveryLiveLease {
    instance_id: RecoveryInstanceId,
    primary: LockedLiveFile,
    guard: LockedLiveFile,
}

impl RecoveryLiveLease {
    /// Returns the logical recovery instance protected by this lease.
    pub const fn instance_id(&self) -> RecoveryInstanceId {
        self.instance_id
    }
}

impl std::ops::Deref for RecoveryStartupScan {
    type Target = [RecoveryScanEntry];

    fn deref(&self) -> &Self::Target {
        self.entries()
    }
}

impl IntoIterator for RecoveryStartupScan {
    type Item = RecoveryScanEntry;
    type IntoIter = std::vec::IntoIter<RecoveryScanEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}

/// Private recovery directory layout beneath a platform state root.
#[derive(Clone)]
pub struct RecoveryStore {
    root: PathBuf,
    #[cfg(windows)]
    _namespace_guard: Arc<WindowsRecoveryNamespace>,
}

fn recovery_directory_error_is_missing(kind: io::ErrorKind) -> bool {
    kind == io::ErrorKind::NotFound
}

impl RecoveryStore {
    #[cfg(test)]
    fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        #[cfg(windows)]
        {
            Self::open_in_state(root.join("state"))
        }
        #[cfg(not(windows))]
        {
            Self::open_unbound_on_unix(root)
        }
    }

    #[cfg(not(windows))]
    fn open_unbound_on_unix(root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(root.join(RECOVERY_RECORDS_DIR))?;
        fs::create_dir_all(root.join(RECOVERY_QUARANTINE_DIR))?;
        Ok(Self { root })
    }

    /// Opens the production recovery layout beneath a platform state root.
    ///
    /// Windows validates and retains the state, recovery, records, and
    /// quarantine directory namespace before recovery content can be written.
    /// Other platforms retain the existing path-based implementation until
    /// their M4-H1 namespace adapters are complete.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the state path is unsafe or unsupported, or
    /// when the recovery layout cannot be created.
    pub fn open_in_state(state_root: impl Into<PathBuf>) -> io::Result<Self> {
        let state_root = state_root.into();
        #[cfg(windows)]
        {
            let namespace = WindowsRecoveryNamespace::open_or_create(
                &state_root,
                std::ffi::OsStr::new(RECOVERY_STATE_SUBDIR),
            )?;
            Ok(Self {
                root: state_root.join(RECOVERY_STATE_SUBDIR),
                _namespace_guard: Arc::new(namespace),
            })
        }
        #[cfg(not(windows))]
        {
            Self::open_unbound_on_unix(state_root.join(RECOVERY_STATE_SUBDIR))
        }
    }

    /// Returns the recovery root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the active records directory.
    pub fn records_dir(&self) -> PathBuf {
        self.root.join(RECOVERY_RECORDS_DIR)
    }

    /// Returns the quarantine directory.
    pub fn quarantine_dir(&self) -> PathBuf {
        self.root.join(RECOVERY_QUARANTINE_DIR)
    }

    /// Returns the stable active path for one editor instance.
    pub fn record_path(&self, instance_id: RecoveryInstanceId) -> PathBuf {
        self.records_dir()
            .join(format!("{}.rec", hex16(&instance_id.as_bytes())))
    }

    /// Returns the live-lease sibling that marks an instance as still running.
    pub fn live_path(&self, instance_id: RecoveryInstanceId) -> PathBuf {
        self.records_dir()
            .join(format!("{}.live", hex16(&instance_id.as_bytes())))
    }

    /// Returns the independent live-lease guard for one editor instance.
    pub fn live_guard_path(&self, instance_id: RecoveryInstanceId) -> PathBuf {
        self.records_dir()
            .join(format!("{}.guard", hex16(&instance_id.as_bytes())))
    }

    /// Holds an exclusive lock that another Noter window can probe without
    /// deleting this instance's recovery record.
    ///
    /// The returned file must stay open for the session. Dropping it releases
    /// the lock so a later launch can offer restore after a crash.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the live file cannot be created or locked.
    pub fn try_hold_live_lease(
        &self,
        instance_id: RecoveryInstanceId,
    ) -> io::Result<RecoveryLiveLease> {
        let attempt = (|| {
            let primary_path = self.live_path(instance_id);
            let primary = acquire_live_file(&primary_path)?;
            let guard_path = self.live_guard_path(instance_id);
            let guard = match acquire_live_file(&guard_path) {
                Ok(guard) => guard,
                Err(error) => {
                    let _ = delete_locked_live_path(&primary_path, &primary);
                    return Err(error);
                }
            };
            Ok(RecoveryLiveLease {
                instance_id,
                primary,
                guard,
            })
        })();
        match attempt {
            Ok(lease) => Ok(lease),
            Err(error) => Err(classify_live_lease_acquisition_error(error, || {
                self.instance_is_live(instance_id).unwrap_or(false)
            })),
        }
    }

    /// Returns whether another living Noter window still holds this instance.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when ownership cannot be determined. Callers must
    /// fail closed rather than expose a potentially live record for restore or
    /// discard.
    pub fn instance_is_live(&self, instance_id: RecoveryInstanceId) -> io::Result<bool> {
        let primary = probe_live_path(&self.live_path(instance_id))?;
        let guard = probe_live_path(&self.live_guard_path(instance_id))?;
        Ok(primary == Some(true) || guard == Some(true))
    }

    /// Persists one recovery snapshot under its instance identity.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when private staging, write, sync, install, or
    /// replace fails.
    pub fn persist(&self, snapshot: &RecoverySnapshot) -> io::Result<()> {
        let destination = self.record_path(snapshot.instance_id());
        let encoded = snapshot.encode();
        #[cfg(unix)]
        {
            write_atomic_private_unix(&destination, snapshot.instance_id(), &encoded)
        }
        #[cfg(windows)]
        {
            write_atomic_private_windows(&destination, snapshot.instance_id(), &encoded)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (destination, encoded);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "recovery persistence is unsupported on this operating system",
            ))
        }
    }

    /// Removes only the active record for an owned instance after save or
    /// worker invalidation. The process-lifetime live lease remains intact.
    ///
    /// Missing files are treated as success so cleanup is idempotent.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when an existing record cannot be removed.
    pub fn delete_record(&self, instance_id: RecoveryInstanceId) -> io::Result<()> {
        remove_file_if_present(&self.record_path(instance_id))
    }

    /// Removes the canonical record and only keyed temporary artifacts owned by
    /// one instance, without opening or parsing unrelated recovery content.
    ///
    /// The process-lifetime live lease remains intact.
    ///
    /// # Errors
    ///
    /// Returns the first directory-listing or removal error.
    pub fn delete_owned_artifacts(&self, instance_id: RecoveryInstanceId) -> io::Result<()> {
        let mut first_error = self.delete_record(instance_id).err();
        let records = self.records_dir();
        let dir = match fs::read_dir(&records) {
            Ok(dir) => dir,
            Err(error) => {
                if recovery_directory_error_is_missing(error.kind()) {
                    return first_error.map_or(Ok(()), Err);
                }
                return Err(first_error.unwrap_or(error));
            }
        };
        for (index, next) in dir.enumerate() {
            if index == MAX_OWNED_RECOVERY_CLEANUP_FILES {
                if first_error.is_none() {
                    first_error = Some(io::Error::other(
                        "owned recovery cleanup reached its directory-entry bound",
                    ));
                }
                break;
            }
            let entry = match next {
                Ok(entry) => entry,
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            };
            let candidate = entry.path();
            if keyed_temporary_instance(&candidate) == Some(instance_id)
                && let Err(error) = remove_file_if_present(&candidate)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Reloads one exact startup handle and returns fully validated content.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the path cannot be read, no longer validates,
    /// or no longer matches every item of metadata bound into the handle.
    pub fn load_record(
        &self,
        handle: &RecoveryRecordHandle,
    ) -> io::Result<ValidatedRecoveryRecord> {
        let claim = self.claim_offered_record(handle)?;
        let result = self.load_claimed_record(handle, &claim);
        let release = self.release_claim(claim);
        match (result, release) {
            (Ok(record), Ok(())) => Ok(record),
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        }
    }

    /// Claims one offered instance and holds its lease until explicitly released.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::ResourceBusy`] when another window owns the
    /// instance, or another I/O error when the lease cannot be acquired.
    pub fn claim_offered_record(
        &self,
        handle: &RecoveryRecordHandle,
    ) -> io::Result<RecoveryInstanceClaim> {
        self.claim_instance(handle.metadata.instance_id())
    }

    fn claim_instance(&self, instance_id: RecoveryInstanceId) -> io::Result<RecoveryInstanceClaim> {
        self.try_hold_live_lease(instance_id)
            .map(|lease| RecoveryInstanceClaim { lease })
    }

    /// Reloads an exact handle while its instance claim remains held.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for a mismatched claim, changed open object,
    /// pathname replacement, length change, or failed record validation.
    pub fn load_claimed_record(
        &self,
        handle: &RecoveryRecordHandle,
        claim: &RecoveryInstanceClaim,
    ) -> io::Result<ValidatedRecoveryRecord> {
        require_matching_claim(handle, claim)?;
        load_bound_record(handle)
    }

    /// Releases an offered-instance claim and removes its stale lease path.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the stale lease path cannot be removed.
    pub fn release_claim(&self, claim: RecoveryInstanceClaim) -> io::Result<()> {
        self.release_live_lease(claim.lease)
    }

    /// Releases a process-lifetime lease by deleting both exact locked objects
    /// by handle where supported, or accepting an already-absent exact path,
    /// before either lock is dropped.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when a present path no longer identifies its locked
    /// lease object or facts-bound deletion fails.
    pub fn release_live_lease(&self, lease: RecoveryLiveLease) -> io::Result<()> {
        let primary = delete_locked_live_path(&self.live_path(lease.instance_id), &lease.primary);
        let guard = delete_locked_live_path(&self.live_guard_path(lease.instance_id), &lease.guard);
        drop(lease);
        primary.and(guard)
    }

    /// Revalidates and removes one exact startup artifact only while its
    /// instance can be exclusively claimed as dead.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::ResourceBusy`] for a live foreign instance and
    /// fails closed when the exact artifact changed or cannot be removed.
    pub fn delete_offered_record(&self, handle: RecoveryRecordHandle) -> io::Result<()> {
        let claim = self.claim_offered_record(&handle)?;
        let result = self.delete_claimed_record(handle, &claim);
        let release = self.release_claim(claim);
        result.and(release)
    }

    /// Deletes one exact open artifact while a matching dead-instance claim is
    /// held. The handle is consumed so Windows deletion can complete.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for a mismatched claim, any identity or content
    /// change, or a failed exact-object deletion.
    pub fn delete_claimed_record(
        &self,
        handle: RecoveryRecordHandle,
        claim: &RecoveryInstanceClaim,
    ) -> io::Result<()> {
        require_matching_claim(&handle, claim)?;
        let _ = load_bound_record(&handle)?;
        delete_bound_record(handle)
    }

    /// Scans a bounded number of active recovery artifacts into metadata-only
    /// restore offers and quarantine results.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the records directory or an enumerated entry
    /// cannot be inspected, except for an entry that disappeared concurrently.
    #[allow(clippy::too_many_lines)]
    pub fn scan_startup(&self) -> io::Result<RecoveryStartupScan> {
        let mut scan = RecoveryStartupScan::default();
        let mut paths = Vec::with_capacity(MAX_STARTUP_RECOVERY_FILES);
        let records = self.records_dir();
        let dir = match fs::read_dir(&records) {
            Ok(dir) => dir,
            Err(error) => {
                if recovery_directory_error_is_missing(error.kind()) {
                    return Ok(scan);
                }
                return Err(error);
            }
        };
        for (raw_index, next) in dir.enumerate() {
            if raw_index == MAX_STARTUP_RECOVERY_DIRECTORY_ENTRIES {
                scan.note_omission(DIRECTORY_LIMIT_REACHED);
                break;
            }
            let path = next?.path();
            let Some(path_metadata) = startup_path_metadata(&path)? else {
                continue;
            };
            if !path_metadata.file_type().is_file() {
                continue;
            }
            if let Some(instance_id) = live_instance_from_path(&path) {
                self.cleanup_stale_live_marker(instance_id);
                continue;
            }
            if let Some(instance_id) = recovery_artifact_instance(&path)
                && self.instance_is_live(instance_id)?
            {
                continue;
            }
            if paths.len() == MAX_STARTUP_RECOVERY_FILES {
                scan.note_omission(FILE_LIMIT_REACHED);
                break;
            }
            paths.push(path);
        }
        paths.sort();

        let mut offers = Vec::with_capacity(MAX_STARTUP_RECOVERY_OFFERS);
        let mut scanned_bytes = 0_u64;
        for path in paths {
            let opened = match open_recovery_candidate(&path) {
                Ok(opened) => opened,
                Err(OpenRecoveryCandidateFailure::Missing) => continue,
                Err(OpenRecoveryCandidateFailure::Inaccessible(error)) => return Err(error),
                Err(OpenRecoveryCandidateFailure::Invalid(reason)) => {
                    retain_quarantine_result(
                        &mut scan,
                        retained_quarantine_entry(
                            path,
                            reason,
                            "Noter left this recovery pathname unchanged because it could not bind the exact file for safe review.",
                        ),
                    );
                    continue;
                }
            };
            let path_instance = recovery_artifact_instance(&path);
            let Ok(header_instance) = peek_recovery_instance(&opened.file, opened.encoded_len)
            else {
                retain_quarantine_result(
                    &mut scan,
                    retained_quarantine_entry(
                        path,
                        RecoveryQuarantineReason::Truncated,
                        "Noter retained this recovery file because its exact header could not be read safely.",
                    ),
                );
                continue;
            };
            let mut belongs_to_live_instance = false;
            for instance_id in path_instance.into_iter().chain(header_instance) {
                if self.instance_is_live(instance_id)? {
                    belongs_to_live_instance = true;
                    break;
                }
            }
            if belongs_to_live_instance {
                continue;
            }
            let Ok(instance_hint) = reconcile_recovery_instance_ids(path_instance, header_instance)
            else {
                retain_quarantine_result(
                    &mut scan,
                    retained_quarantine_entry(
                        path,
                        RecoveryQuarantineReason::InstanceMismatch,
                        "Noter retained the file because neither named instance can authorize movement.",
                    ),
                );
                continue;
            };
            let Some(next_bytes) = advance_scan_byte_budget(scanned_bytes, opened.encoded_len)
            else {
                scan.note_omission(BYTE_LIMIT_REACHED);
                break;
            };
            scanned_bytes = next_bytes;
            let Ok(bytes) = read_bound_open_file(&opened.file, opened.facts, opened.encoded_len)
            else {
                retain_quarantine_result(
                    &mut scan,
                    retained_quarantine_entry(
                        path,
                        RecoveryQuarantineReason::Truncated,
                        "Noter retained this recovery file because the exact open artifact changed while it was being read.",
                    ),
                );
                continue;
            };
            if revalidate_path_identity(&path, opened.facts, opened.encoded_len).is_err() {
                retain_quarantine_result(
                    &mut scan,
                    retained_quarantine_entry(
                        path,
                        RecoveryQuarantineReason::Truncated,
                        "Noter retained this recovery pathname because it changed after the exact file was opened.",
                    ),
                );
                continue;
            }
            match validate_recovery_metadata(&bytes) {
                Ok(metadata) => {
                    if self.instance_is_live(metadata.instance_id())? {
                        continue;
                    }
                    consider_offer(
                        &mut offers,
                        RecoveryRecordHandle {
                            path,
                            metadata,
                            file: opened.file,
                            facts: opened.facts,
                            encoded_len: opened.encoded_len,
                        },
                        &mut scan.omission_flags,
                    );
                }
                Err(reason) => {
                    retain_quarantine_result(
                        &mut scan,
                        self.quarantine_opened_scan_entry(
                            path,
                            &opened,
                            &bytes,
                            reason,
                            instance_hint,
                        ),
                    );
                }
            }
        }
        offers.sort_by(|left, right| offer_sort_key(left).cmp(&offer_sort_key(right)));
        if offers.iter().any(RecoveryOffer::superseded_omitted) {
            scan.note_omission(SUPERSEDED_HANDLES_OMITTED);
        }
        scan.entries.extend(offers.into_iter().map(|offer| {
            let path = offer.primary.path.clone();
            RecoveryScanEntry {
                path,
                disposition: RecoveryScanDisposition::Offer(offer),
                quarantine_error: None,
            }
        }));
        Ok(scan)
    }

    /// Moves one exact bound recovery file into the quarantine directory.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the durable quarantine copy cannot be completed,
    /// or when the source cannot be removed or its parent cannot be synchronized
    /// afterward. A completed copy remains available for recovery review. A
    /// missing source is reported as [`io::ErrorKind::NotFound`] rather than
    /// success.
    pub fn quarantine_file(&self, path: &Path) -> io::Result<PathBuf> {
        fs::symlink_metadata(path)?;
        let opened = open_recovery_candidate(path).map_err(|failure| match failure {
            OpenRecoveryCandidateFailure::Missing => io::Error::new(
                io::ErrorKind::NotFound,
                "recovery source disappeared before it could be bound",
            ),
            OpenRecoveryCandidateFailure::Inaccessible(error) => error,
            OpenRecoveryCandidateFailure::Invalid(reason) => {
                io::Error::new(io::ErrorKind::InvalidData, reason.description())
            }
        })?;
        let path_instance = recovery_artifact_instance(path);
        let header_instance = peek_recovery_instance(&opened.file, opened.encoded_len)?;
        let instance_hint = reconcile_recovery_instance_ids(path_instance, header_instance)
            .map_err(|()| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "recovery pathname and encoded instance identities disagree",
                )
            })?;
        let bytes = read_bound_open_file(&opened.file, opened.facts, opened.encoded_len)?;
        let claim = instance_hint
            .map(|instance_id| self.claim_instance(instance_id))
            .transpose()?;
        let result = quarantine_bound_file(self, path, &opened, &bytes);
        let release = claim.map_or(Ok(()), |claim| self.release_claim(claim));
        finish_public_quarantine(result, release)
    }

    fn cleanup_stale_live_marker(&self, instance_id: RecoveryInstanceId) {
        let Ok(claim) = self.claim_instance(instance_id) else {
            return;
        };
        let _ = self.release_claim(claim);
    }

    fn quarantine_opened_scan_entry(
        &self,
        path: PathBuf,
        opened: &OpenedRecoveryCandidate,
        bytes: &[u8],
        reason: RecoveryQuarantineReason,
        instance_hint: Option<RecoveryInstanceId>,
    ) -> RecoveryScanEntry {
        let claim = match instance_hint.map(|instance_id| self.claim_instance(instance_id)) {
            Some(Ok(claim)) => Some(claim),
            Some(Err(error)) => {
                return retained_quarantine_entry(
                    path,
                    reason,
                    &format!(
                        "Noter retained this damaged recovery file because its instance could not be exclusively claimed ({error})."
                    ),
                );
            }
            None => None,
        };
        let quarantine = quarantine_bound_file(self, &path, opened, bytes);
        let release = claim.map_or(Ok(()), |claim| self.release_claim(claim));
        match (quarantine, release) {
            (Ok((quarantined, None)), Ok(())) => RecoveryScanEntry {
                path: quarantined,
                disposition: RecoveryScanDisposition::Quarantine(reason),
                quarantine_error: None,
            },
            (Ok((quarantined, cleanup_error)), release) => RecoveryScanEntry {
                path: quarantined,
                disposition: RecoveryScanDisposition::Quarantine(reason),
                quarantine_error: Some(format!(
                    "Noter preserved the exact damaged recovery bytes in quarantine, but source or instance-claim cleanup was incomplete ({}).",
                    cleanup_error
                        .map(|error| error.to_string())
                        .or_else(|| release.err().map(|error| error.to_string()))
                        .unwrap_or_else(|| "unknown cleanup failure".to_owned())
                )),
            },
            (Err(error), _) => retained_quarantine_entry(
                path,
                reason,
                &format!(
                    "Noter retained this damaged recovery file because exact quarantine could not be completed ({error})."
                ),
            ),
        }
    }
}

fn finish_public_quarantine(
    quarantine: io::Result<(PathBuf, Option<io::Error>)>,
    release: io::Result<()>,
) -> io::Result<PathBuf> {
    match (quarantine, release) {
        (Ok((destination, None)), Ok(())) => Ok(destination),
        (Ok((_, Some(error))) | Err(error), _) | (Ok(_), Err(error)) => Err(error),
    }
}

fn retained_quarantine_entry(
    path: PathBuf,
    reason: RecoveryQuarantineReason,
    message: &str,
) -> RecoveryScanEntry {
    RecoveryScanEntry {
        path,
        disposition: RecoveryScanDisposition::Quarantine(reason),
        quarantine_error: Some(message.to_owned()),
    }
}

fn quarantine_bound_file(
    store: &RecoveryStore,
    path: &Path,
    opened: &OpenedRecoveryCandidate,
    bytes: &[u8],
) -> io::Result<(PathBuf, Option<io::Error>)> {
    quarantine_bound_file_with(store, path, opened, bytes, noter_platform::sync_parent)
}

fn quarantine_bound_file_with(
    store: &RecoveryStore,
    path: &Path,
    opened: &OpenedRecoveryCandidate,
    bytes: &[u8],
    mut sync_parent: impl FnMut(&Path) -> io::Result<noter_platform::ParentSyncOutcome>,
) -> io::Result<(PathBuf, Option<io::Error>)> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != opened.encoded_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bound recovery bytes do not match the opened artifact length",
        ));
    }
    revalidate_path_identity(path, opened.facts, opened.encoded_len)?;
    let (destination, mut quarantine_file) = create_quarantine_copy(store, bytes)?;
    if let Err(error) = verify_exact_bytes(&mut quarantine_file, bytes) {
        let _ = noter_platform::delete_open_file(&quarantine_file);
        drop(quarantine_file);
        let _ = fs::remove_file(&destination);
        return Err(error);
    }

    noter_platform::sync_file(&quarantine_file)?;
    let _ = sync_parent(&destination)?;
    let cleanup_error = match delete_bound_candidate(path, opened) {
        Ok(()) => sync_parent(path).map(|_| ()).err(),
        Err(error) => Some(error),
    };
    drop(quarantine_file);
    Ok((destination, cleanup_error))
}

fn create_quarantine_copy(store: &RecoveryStore, bytes: &[u8]) -> io::Result<(PathBuf, File)> {
    for _ in 0..32 {
        let mut random = [0_u8; 16];
        fill_random(&mut random).map_err(|error| {
            io::Error::other(format!("recovery quarantine random name failed: {error}"))
        })?;
        let destination = store
            .quarantine_dir()
            .join(format!("noter-quarantine-{}.rec", hex16(&random)));
        let mut file = match noter_platform::create_private_new_file(&destination) {
            Ok(file) => file,
            Err(error) => {
                if is_quarantine_name_collision(error.kind()) {
                    continue;
                }
                return Err(error);
            }
        };
        let write_result = (|| {
            file.write_all(bytes)?;
            file.flush()?;
            noter_platform::sync_file(&file)
        })();
        if let Err(error) = write_result {
            let _ = noter_platform::delete_open_file(&file);
            drop(file);
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
        return Ok((destination, file));
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a private recovery quarantine name",
    ))
}

fn verify_exact_bytes(file: &mut File, expected: &[u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    for expected_chunk in expected.chunks(buffer.len()) {
        file.read_exact(&mut buffer[..expected_chunk.len()])?;
        if buffer[..expected_chunk.len()] != *expected_chunk {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "quarantined recovery bytes did not verify",
            ));
        }
    }
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing)? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quarantined recovery copy has trailing bytes",
        ));
    }
    Ok(())
}

fn delete_bound_candidate(path: &Path, opened: &OpenedRecoveryCandidate) -> io::Result<()> {
    revalidate_path_identity(path, opened.facts, opened.encoded_len)?;
    let cleanup_file = noter_platform::open_for_cleanup(path)?;
    if !file_binding_matches(
        noter_platform::file_facts(&cleanup_file)? == opened.facts,
        cleanup_file.metadata()?.len() == opened.encoded_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery pathname changed before quarantine cleanup",
        ));
    }
    match noter_platform::delete_open_file(&cleanup_file) {
        Ok(()) => Ok(()),
        Err(error) => {
            if requires_path_delete_fallback(error.kind()) {
                delete_bound_candidate_unix_fallback(path, opened, cleanup_file)
            } else {
                Err(error)
            }
        }
    }
}

fn delete_bound_candidate_unix_fallback(
    path: &Path,
    opened: &OpenedRecoveryCandidate,
    cleanup_file: File,
) -> io::Result<()> {
    revalidate_path_identity(path, opened.facts, opened.encoded_len)?;
    drop(cleanup_file);
    fs::remove_file(path)?;
    if noter_platform::file_facts(&opened.file)?.link_count() >= opened.facts.link_count() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quarantine cleanup did not unlink the bound recovery object",
        ));
    }
    Ok(())
}

fn live_instance_from_path(path: &Path) -> Option<RecoveryInstanceId> {
    let name = path.file_name()?.to_str()?;
    let instance = name
        .strip_suffix(".live")
        .or_else(|| name.strip_suffix(".guard"))?;
    decode_hex16(instance).map(RecoveryInstanceId::new)
}

fn reconcile_recovery_instance_ids(
    path_instance: Option<RecoveryInstanceId>,
    header_instance: Option<RecoveryInstanceId>,
) -> Result<Option<RecoveryInstanceId>, ()> {
    match (path_instance, header_instance) {
        (Some(path_id), Some(header_id)) => {
            if path_id == header_id {
                Ok(Some(path_id))
            } else {
                Err(())
            }
        }
        (Some(instance_id), None) | (None, Some(instance_id)) => Ok(Some(instance_id)),
        (None, None) => Ok(None),
    }
}

fn recovery_artifact_instance(path: &Path) -> Option<RecoveryInstanceId> {
    keyed_temporary_instance(path).or_else(|| {
        let name = path.file_name()?.to_str()?;
        let instance = name.strip_suffix(".rec")?;
        decode_hex16(instance).map(RecoveryInstanceId::new)
    })
}

fn peek_recovery_instance(file: &File, encoded_len: u64) -> io::Result<Option<RecoveryInstanceId>> {
    const IDENTITY_HEADER_LEN: usize = 44;
    if encoded_len < u64::try_from(IDENTITY_HEADER_LEN).unwrap_or(u64::MAX) {
        return Ok(None);
    }
    let mut reader = file;
    reader.seek(SeekFrom::Start(0))?;
    let mut header = [0_u8; IDENTITY_HEADER_LEN];
    reader.read_exact(&mut header)?;
    if &header[..RECOVERY_MAGIC.len()] != RECOVERY_MAGIC {
        return Ok(None);
    }
    let schema = u32::from_le_bytes(
        header[8..12]
            .try_into()
            .expect("the recovery schema slice has four bytes"),
    );
    if !matches!(schema, 1 | RECOVERY_SCHEMA_VERSION) {
        return Ok(None);
    }
    let instance = header[28..44]
        .try_into()
        .expect("the recovery instance slice has sixteen bytes");
    Ok(Some(RecoveryInstanceId::new(instance)))
}

fn consider_offer(
    offers: &mut Vec<RecoveryOffer>,
    handle: RecoveryRecordHandle,
    omission_flags: &mut u8,
) {
    let mut candidate = RecoveryOffer {
        primary: Box::new(handle),
        superseded: Vec::new(),
        superseded_omitted: false,
    };

    if let Some(index) = offers
        .iter()
        .position(|offer| offer.primary.metadata == candidate.primary.metadata)
    {
        absorb_superseded(&mut offers[index], candidate);
        return;
    }

    while let Some(index) = offers.iter().position(|offer| {
        authenticated_same_instance_document(&candidate.primary.metadata, &offer.primary.metadata)
            && candidate.primary.metadata.revision() > offer.primary.metadata.revision()
    }) {
        let older = offers.remove(index);
        absorb_superseded(&mut candidate, older);
    }
    if let Some(index) = offers.iter().position(|offer| {
        authenticated_same_instance_document(&candidate.primary.metadata, &offer.primary.metadata)
            && offer.primary.metadata.revision() > candidate.primary.metadata.revision()
    }) {
        absorb_superseded(&mut offers[index], candidate);
        return;
    }

    while let Some(index) = offers
        .iter()
        .position(|offer| directly_supersedes(&candidate.primary.metadata, &offer.primary.metadata))
    {
        let older = offers.remove(index);
        absorb_superseded(&mut candidate, older);
    }
    if let Some(index) = offers
        .iter()
        .position(|offer| directly_supersedes(&offer.primary.metadata, &candidate.primary.metadata))
    {
        absorb_superseded(&mut offers[index], candidate);
        return;
    }
    if offers.len() < MAX_STARTUP_RECOVERY_OFFERS {
        offers.push(candidate);
    } else {
        *omission_flags |= OFFERS_OMITTED;
    }
}

fn same_instance_document(
    left: &ValidatedRecoveryMetadata,
    right: &ValidatedRecoveryMetadata,
) -> bool {
    left.instance_id() == right.instance_id() && left.document_id() == right.document_id()
}

fn authenticated_same_instance_document(
    left: &ValidatedRecoveryMetadata,
    right: &ValidatedRecoveryMetadata,
) -> bool {
    left.schema_version() == RECOVERY_SCHEMA_VERSION
        && right.schema_version() == RECOVERY_SCHEMA_VERSION
        && same_instance_document(left, right)
}

fn push_superseded(offer: &mut RecoveryOffer, handle: RecoveryRecordHandle) {
    if offer.superseded.len() < MAX_SUPERSEDED_RECOVERY_HANDLES {
        offer.superseded.push(handle);
    } else {
        offer.superseded_omitted = true;
    }
}

fn absorb_superseded(target: &mut RecoveryOffer, older: RecoveryOffer) {
    for handle in older.superseded {
        push_superseded(target, handle);
    }
    push_superseded(target, *older.primary);
    target.superseded_omitted |= older.superseded_omitted;
}

fn directly_supersedes(
    candidate: &ValidatedRecoveryMetadata,
    current: &ValidatedRecoveryMetadata,
) -> bool {
    candidate.schema_version() == RECOVERY_SCHEMA_VERSION
        && current.schema_version() == RECOVERY_SCHEMA_VERSION
        && candidate.document_id() == current.document_id()
        && candidate.predecessor_instance() == Some(current.instance_id())
        && current
            .lineage_generation()
            .and_then(super::recovery::RecoveryLineageGeneration::checked_next)
            == candidate.lineage_generation()
}

fn offer_sort_key(offer: &RecoveryOffer) -> ([u8; 16], [u8; 16], &Path) {
    (
        offer.metadata().document_id().as_bytes(),
        offer.metadata().instance_id().as_bytes(),
        &offer.primary.path,
    )
}

fn validate_and_lock_live_file(file: File) -> io::Result<(File, noter_platform::FileFacts)> {
    let metadata = file.metadata()?;
    let facts = noter_platform::file_facts(&file)?;
    if !valid_live_file_shape(metadata.is_file(), facts.link_count()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery live lease is not a private regular file",
        ));
    }
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => live_lease_busy_error(),
        TryLockError::Error(error) => error,
    })?;
    Ok((file, facts))
}

fn acquire_live_file(path: &Path) -> io::Result<LockedLiveFile> {
    let file = match noter_platform::create_private_new_file(path) {
        Ok(file) => file,
        Err(error) => match error.kind() {
            io::ErrorKind::AlreadyExists => noter_platform::open_for_cleanup(path)?,
            _ => return Err(error),
        },
    };
    let (file, facts) = validate_and_lock_live_file(file)?;
    Ok(LockedLiveFile { file, facts })
}

fn probe_live_path(path: &Path) -> io::Result<Option<bool>> {
    let file = match noter_platform::open_existing_no_follow(path) {
        Ok(file) => file,
        Err(error) => match error.kind() {
            io::ErrorKind::NotFound => return Ok(None),
            _ => return Err(error),
        },
    };
    let metadata = file.metadata()?;
    let facts = noter_platform::file_facts(&file)?;
    if !valid_live_file_shape(metadata.is_file(), facts.link_count()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery live lease is not a private regular file",
        ));
    }
    match file.try_lock() {
        Ok(()) => Ok(Some(false)),
        Err(TryLockError::WouldBlock) => Ok(Some(true)),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

const fn valid_live_file_shape(is_file: bool, link_count: u64) -> bool {
    is_file && link_count == 1
}

fn live_lease_busy_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::ResourceBusy,
        "recovery live lease is held by another Noter window",
    )
}

fn classify_nonbusy_lease_error(error: io::Error, observed_live: bool) -> io::Error {
    if observed_live {
        live_lease_busy_error()
    } else {
        error
    }
}

fn classify_live_lease_acquisition_error(
    error: io::Error,
    observe_live: impl FnOnce() -> bool,
) -> io::Error {
    match error.kind() {
        io::ErrorKind::ResourceBusy => error,
        _ => classify_nonbusy_lease_error(error, observe_live()),
    }
}

const fn is_quarantine_name_collision(kind: io::ErrorKind) -> bool {
    matches!(kind, io::ErrorKind::AlreadyExists)
}

const fn requires_path_delete_fallback(kind: io::ErrorKind) -> bool {
    matches!(kind, io::ErrorKind::Unsupported)
}

fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn startup_path_metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    classify_startup_path_metadata(fs::symlink_metadata(path))
}

fn classify_startup_path_metadata(
    result: io::Result<fs::Metadata>,
) -> io::Result<Option<fs::Metadata>> {
    match result {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn retain_quarantine_result(scan: &mut RecoveryStartupScan, entry: RecoveryScanEntry) {
    if scan.entries.len() < MAX_STARTUP_QUARANTINE_RESULTS {
        scan.entries.push(entry);
    } else {
        scan.quarantine_results_omitted = scan.quarantine_results_omitted.saturating_add(1);
    }
}

fn advance_scan_byte_budget(consumed: u64, next: u64) -> Option<u64> {
    consumed
        .checked_add(next)
        .filter(|total| *total <= MAX_STARTUP_RECOVERY_BYTES)
}

struct OpenedRecoveryCandidate {
    file: File,
    facts: noter_platform::FileFacts,
    encoded_len: u64,
}

#[derive(Debug)]
enum OpenRecoveryCandidateFailure {
    Missing,
    Inaccessible(io::Error),
    Invalid(RecoveryQuarantineReason),
}

fn classify_recovery_candidate_io(error: io::Error) -> OpenRecoveryCandidateFailure {
    if error.kind() == io::ErrorKind::NotFound {
        OpenRecoveryCandidateFailure::Missing
    } else {
        OpenRecoveryCandidateFailure::Inaccessible(error)
    }
}

fn open_recovery_candidate(
    path: &Path,
) -> Result<OpenedRecoveryCandidate, OpenRecoveryCandidateFailure> {
    let file =
        noter_platform::open_existing_no_follow(path).map_err(classify_recovery_candidate_io)?;
    let metadata = file.metadata().map_err(classify_recovery_candidate_io)?;
    if !metadata.is_file() {
        return Err(OpenRecoveryCandidateFailure::Invalid(
            RecoveryQuarantineReason::Truncated,
        ));
    }
    if exceeds_recovery_file_bound(metadata.len()) {
        return Err(OpenRecoveryCandidateFailure::Invalid(
            RecoveryQuarantineReason::ContentTooLarge,
        ));
    }
    let facts = noter_platform::file_facts(&file).map_err(classify_recovery_candidate_io)?;
    if facts.link_count() != 1 {
        return Err(OpenRecoveryCandidateFailure::Invalid(
            RecoveryQuarantineReason::Truncated,
        ));
    }
    Ok(OpenedRecoveryCandidate {
        file,
        facts,
        encoded_len: metadata.len(),
    })
}

fn read_bound_open_file(
    file: &File,
    expected_facts: noter_platform::FileFacts,
    expected_len: u64,
) -> io::Result<Vec<u8>> {
    if !file_binding_matches(
        noter_platform::file_facts(file)? == expected_facts,
        file.metadata()?.len() == expected_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery artifact changed before validation",
        ));
    }
    let mut reader = file;
    reader.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::with_capacity(
        usize::try_from(expected_len)
            .unwrap_or(0)
            .min(usize::try_from(MAX_RECOVERY_FILE_BYTES).unwrap_or(usize::MAX)),
    );
    reader
        .take(MAX_RECOVERY_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if exceeds_recovery_file_bound(u64::try_from(bytes.len()).unwrap_or(u64::MAX)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery artifact exceeded the read bound",
        ));
    }
    if !complete_bound_read_matches(
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) == expected_len,
        noter_platform::file_facts(file)? == expected_facts,
        file.metadata()?.len() == expected_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery artifact changed during validation",
        ));
    }
    Ok(bytes)
}

fn revalidate_path_identity(
    path: &Path,
    expected_facts: noter_platform::FileFacts,
    expected_len: u64,
) -> io::Result<()> {
    let path_file = noter_platform::open_existing_no_follow(path)?;
    if !file_binding_matches(
        noter_platform::file_facts(&path_file)? == expected_facts,
        path_file.metadata()?.len() == expected_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery pathname no longer identifies the validated artifact",
        ));
    }
    Ok(())
}

const fn exceeds_recovery_file_bound(encoded_len: u64) -> bool {
    encoded_len > MAX_RECOVERY_FILE_BYTES
}

const fn file_binding_matches(facts_match: bool, length_matches: bool) -> bool {
    facts_match && length_matches
}

const fn complete_bound_read_matches(
    bytes_read_match: bool,
    facts_match: bool,
    length_matches: bool,
) -> bool {
    bytes_read_match && facts_match && length_matches
}

#[cfg(any(windows, test))]
const fn reconciled_cleanup_state_is_exact(
    destination_matches: bool,
    stage_matches_or_is_absent: bool,
    backup_matches_or_is_absent: bool,
) -> bool {
    destination_matches && stage_matches_or_is_absent && backup_matches_or_is_absent
}

fn require_matching_claim(
    handle: &RecoveryRecordHandle,
    claim: &RecoveryInstanceClaim,
) -> io::Result<()> {
    if claim.lease.instance_id != handle.metadata.instance_id() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "recovery instance claim does not match the artifact",
        ));
    }
    Ok(())
}

fn load_bound_record(handle: &RecoveryRecordHandle) -> io::Result<ValidatedRecoveryRecord> {
    let bytes = read_bound_open_file(&handle.file, handle.facts, handle.encoded_len)?;
    revalidate_path_identity(&handle.path, handle.facts, handle.encoded_len)?;
    let RecoveryStartupDisposition::Offer(record) = validate_recovery_record(&bytes) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery offer no longer validates",
        ));
    };
    if record.metadata() != &handle.metadata {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery offer changed after startup validation",
        ));
    }
    Ok(record)
}

fn delete_bound_record(handle: RecoveryRecordHandle) -> io::Result<()> {
    let cleanup_file = noter_platform::open_for_cleanup(&handle.path)?;
    if !file_binding_matches(
        noter_platform::file_facts(&cleanup_file)? == handle.facts,
        cleanup_file.metadata()?.len() == handle.encoded_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery pathname changed before exact cleanup",
        ));
    }
    match noter_platform::delete_open_file(&cleanup_file) {
        Ok(()) => {
            drop(cleanup_file);
            drop(handle);
            Ok(())
        }
        Err(error) => {
            if requires_path_delete_fallback(error.kind()) {
                delete_bound_record_unix_fallback(&handle, cleanup_file)
            } else {
                Err(error)
            }
        }
    }
}

fn delete_bound_record_unix_fallback(
    handle: &RecoveryRecordHandle,
    cleanup_file: File,
) -> io::Result<()> {
    revalidate_path_identity(&handle.path, handle.facts, handle.encoded_len)?;
    drop(cleanup_file);
    fs::remove_file(&handle.path)?;
    if noter_platform::file_facts(&handle.file)?.link_count() >= handle.facts.link_count() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery pathname cleanup did not unlink the validated object",
        ));
    }
    Ok(())
}

fn delete_claimed_live_path(
    path: &Path,
    lease: &File,
    expected_facts: noter_platform::FileFacts,
) -> io::Result<()> {
    match noter_platform::delete_open_file(lease) {
        Ok(()) => Ok(()),
        Err(error) => {
            if requires_path_delete_fallback(error.kind()) {
                if !live_lease_facts_match(noter_platform::file_facts(lease)?, expected_facts) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "recovery live lease changed while its claim was held",
                    ));
                }
                delete_claimed_live_path_unix_fallback(path, lease, expected_facts)
            } else {
                Err(error)
            }
        }
    }
}

fn delete_claimed_live_path_unix_fallback(
    path: &Path,
    lease: &File,
    expected_facts: noter_platform::FileFacts,
) -> io::Result<()> {
    let path_file = match noter_platform::open_existing_no_follow(path) {
        Ok(file) => file,
        Err(error) => match error.kind() {
            io::ErrorKind::NotFound => return Ok(()),
            _ => return Err(error),
        },
    };
    if !live_lease_facts_match(noter_platform::file_facts(&path_file)?, expected_facts) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery live pathname changed before claim cleanup",
        ));
    }
    drop(path_file);
    fs::remove_file(path)?;
    if noter_platform::file_facts(lease)?.link_count() >= expected_facts.link_count() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery live claim cleanup did not unlink the claimed object",
        ));
    }
    Ok(())
}

fn delete_locked_live_path(path: &Path, locked: &LockedLiveFile) -> io::Result<()> {
    delete_claimed_live_path(path, &locked.file, locked.facts)
}

fn live_lease_facts_match(
    actual: noter_platform::FileFacts,
    expected: noter_platform::FileFacts,
) -> bool {
    stable_live_lease_binding_matches(
        actual.identity() == expected.identity(),
        actual.link_count() == expected.link_count(),
    )
}

const fn stable_live_lease_binding_matches(
    identity_matches: bool,
    link_count_matches: bool,
) -> bool {
    identity_matches && link_count_matches
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy)]
enum TemporaryArtifactKind {
    Stage,
    Backup,
}

#[cfg(any(windows, test))]
impl TemporaryArtifactKind {
    const fn extension(self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Backup => "backup",
        }
    }
}

#[cfg(unix)]
fn write_atomic_private_unix(
    destination: &Path,
    instance_id: RecoveryInstanceId,
    bytes: &[u8],
) -> io::Result<()> {
    write_atomic_private_unix_with(destination, instance_id, bytes, |_| Ok(()))
}

#[cfg(unix)]
fn write_atomic_private_unix_with(
    destination: &Path,
    instance_id: RecoveryInstanceId,
    bytes: &[u8],
    before_commit: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;
    let commit_parent = noter_platform::UnixRecoveryCommitParent::bind(destination)?;
    let stage = unix_recovery_stage_path(parent, instance_id);
    let mut file = commit_parent.create_private_new(&stage).map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            io::Error::new(
                io::ErrorKind::ResourceBusy,
                "a retained recovery stage requires startup review before persistence can retry",
            )
        } else {
            error
        }
    })?;
    file.write_all(bytes)?;
    file.flush()?;
    noter_platform::sync_file(&file)?;
    before_commit(&stage)?;

    // Keep the exact stage object open until the consuming descriptor-relative
    // rename completes. Any failure after exclusive creation deliberately
    // retains the bounded keyed artifact for startup review; pathname cleanup
    // cannot be made exact on Unix after a concurrent basename rebind.
    let receipt = commit_parent.replace_existing_consuming(&stage, &file)?;
    drop(file);
    let (_outcome, parent_sync) = receipt.into_parts();
    parent_sync.sync().map(|_| ())
}

#[cfg(unix)]
fn unix_recovery_stage_path(parent: &Path, instance_id: RecoveryInstanceId) -> PathBuf {
    // One reserved slot bounds every post-create failure to one retained stage
    // per instance. Reusing or deleting an existing pathname cannot be made
    // exact after a rebind, so a later retry reports ResourceBusy until bounded
    // startup review or explicit owned-artifact cleanup handles the artifact.
    parent.join(format!(
        ".noter-recovery-{}-00000000000000000000000000000000.stage",
        hex16(&instance_id.as_bytes())
    ))
}

#[cfg(windows)]
fn write_atomic_private_windows(
    destination: &Path,
    instance_id: RecoveryInstanceId,
    bytes: &[u8],
) -> io::Result<()> {
    write_atomic_private_windows_with(
        destination,
        instance_id,
        bytes,
        noter_platform::replace_existing,
    )
}

#[cfg(windows)]
fn write_atomic_private_windows_with(
    destination: &Path,
    instance_id: RecoveryInstanceId,
    bytes: &[u8],
    replace: impl FnOnce(
        &Path,
        &Path,
        Option<&Path>,
    ) -> io::Result<CommitReceipt<ReplaceExistingOutcome>>,
) -> io::Result<()> {
    write_atomic_private_with_sync(
        destination,
        instance_id,
        bytes,
        replace,
        RecoveryParentSync::sync,
    )
}

#[cfg(any(windows, test))]
fn write_atomic_private_with_sync(
    destination: &Path,
    instance_id: RecoveryInstanceId,
    bytes: &[u8],
    replace: impl FnOnce(
        &Path,
        &Path,
        Option<&Path>,
    ) -> io::Result<CommitReceipt<ReplaceExistingOutcome>>,
    sync_parent: impl FnOnce(RecoveryParentSync) -> io::Result<noter_platform::ParentSyncOutcome>,
) -> io::Result<()> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let stage = exclusive_stage_path(parent, instance_id, TemporaryArtifactKind::Stage)?;
    let backup = exclusive_stage_path(parent, instance_id, TemporaryArtifactKind::Backup)?;
    let write_result = commit_staged_record_with(&stage, destination, &backup, bytes, replace);

    // Every post-create failure retains only the deterministic per-instance
    // slots. Pathname cleanup could remove a rebound object, while the next
    // attempt will fail with ResourceBusy before creating another artifact.
    let success = match write_result {
        Ok(success) => success,
        Err(failure) => return Err(failure.error),
    };
    sync_parent(success.parent_sync).map(|_| ())
}

#[cfg(any(windows, test))]
#[derive(Debug)]
enum RecoveryParentSync {
    Bound(noter_platform::ParentSyncReceipt),
    #[cfg(windows)]
    UnsupportedAfterReconciliation,
}

#[cfg(any(windows, test))]
#[derive(Debug)]
struct RecoveryCommitSuccess {
    parent_sync: RecoveryParentSync,
}

#[cfg(any(windows, test))]
impl RecoveryCommitSuccess {
    const fn clean(parent_sync: RecoveryParentSync) -> Self {
        Self { parent_sync }
    }
}

#[cfg(any(windows, test))]
impl RecoveryParentSync {
    const fn bound(receipt: noter_platform::ParentSyncReceipt) -> Self {
        Self::Bound(receipt)
    }

    fn sync(self) -> io::Result<noter_platform::ParentSyncOutcome> {
        match self {
            Self::Bound(receipt) => receipt.sync(),
            #[cfg(windows)]
            Self::UnsupportedAfterReconciliation => {
                Ok(noter_platform::ParentSyncOutcome::Unsupported)
            }
        }
    }
}

#[cfg(any(windows, test))]
#[derive(Debug)]
struct RecoveryCommitFailure {
    error: io::Error,
}

#[cfg(any(windows, test))]
impl RecoveryCommitFailure {
    #[cfg(windows)]
    const fn preserve_windows_artifacts(error: io::Error) -> Self {
        Self { error }
    }
}

#[cfg(any(windows, test))]
impl From<io::Error> for RecoveryCommitFailure {
    fn from(error: io::Error) -> Self {
        Self { error }
    }
}

#[cfg(any(windows, test))]
fn commit_staged_record_with(
    stage: &Path,
    destination: &Path,
    backup: &Path,
    bytes: &[u8],
    replace: impl FnOnce(
        &Path,
        &Path,
        Option<&Path>,
    ) -> io::Result<CommitReceipt<ReplaceExistingOutcome>>,
) -> Result<RecoveryCommitSuccess, RecoveryCommitFailure> {
    let mut file = noter_platform::create_private_new_file(stage)?;
    file.write_all(bytes)?;
    file.flush()?;
    noter_platform::sync_file(&file)?;
    drop(file);

    if destination.exists() {
        finish_replace_with(stage, destination, backup, replace)
    } else {
        match noter_platform::install_new(stage, destination) {
            Ok(receipt) => {
                let (_outcome, parent_sync) = receipt.into_parts();
                // A platform fallback may retain one keyed hard-link stage.
                // Retaining it is safer than unlinking a basename that could
                // have rebound after the commit; bounded recovery review and
                // owned-artifact cleanup already recognize this exact name.
                Ok(RecoveryCommitSuccess::clean(RecoveryParentSync::bound(
                    parent_sync,
                )))
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // A concurrent install won the destination. Replace that file
                // with this staged snapshot instead of reporting success without
                // committing these bytes.
                finish_replace_with(stage, destination, backup, replace)
            }
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(any(windows, test))]
fn finish_replace_with(
    stage: &Path,
    destination: &Path,
    backup: &Path,
    replace: impl FnOnce(
        &Path,
        &Path,
        Option<&Path>,
    ) -> io::Result<CommitReceipt<ReplaceExistingOutcome>>,
) -> Result<RecoveryCommitSuccess, RecoveryCommitFailure> {
    #[cfg(windows)]
    let intended = inspect_windows_recovery_artifact(stage)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "recovery stage disappeared before replacement",
        )
    })?;
    #[cfg(windows)]
    let expected = inspect_windows_recovery_artifact(destination)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "recovery destination disappeared before replacement",
        )
    })?;

    match replace(stage, destination, Some(backup)) {
        Ok(receipt) => {
            let (outcome, parent_sync) = receipt.into_parts();
            match outcome {
                ReplaceExistingOutcome::Clean => {
                    #[cfg(windows)]
                    {
                        let success = io::Error::other("recovery replacement reported success");
                        finalize_reconciled_windows_recovery(
                            stage,
                            destination,
                            backup,
                            IntendedWindowsRecoveryContent::from_observation(intended),
                            expected,
                            &success,
                            true,
                        )?;
                        Ok(RecoveryCommitSuccess::clean(RecoveryParentSync::bound(
                            parent_sync,
                        )))
                    }
                    #[cfg(not(windows))]
                    {
                        Ok(RecoveryCommitSuccess::clean(RecoveryParentSync::bound(
                            parent_sync,
                        )))
                    }
                }
                ReplaceExistingOutcome::DisplacedDestination => {
                    // Injected or non-Unix exchange implementations may retain
                    // a predecessor. Never unlink that basename after returning
                    // from the atomic operation because it may have rebound.
                    Ok(RecoveryCommitSuccess::clean(RecoveryParentSync::bound(
                        parent_sync,
                    )))
                }
            }
        }
        Err(error) => {
            #[cfg(windows)]
            return reconcile_windows_recovery_replace(
                stage,
                destination,
                backup,
                IntendedWindowsRecoveryContent::from_observation(intended),
                expected,
                error,
            )
            .map(RecoveryCommitSuccess::clean);
            #[cfg(not(windows))]
            Err(error.into())
        }
    }
}

#[cfg(windows)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RecoveryArtifactObservation {
    identity: noter_platform::FileIdentity,
    fingerprint: ContentFingerprint,
    length: u64,
}

#[cfg(windows)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct IntendedWindowsRecoveryContent {
    observation: RecoveryArtifactObservation,
}

#[cfg(windows)]
impl IntendedWindowsRecoveryContent {
    const fn from_observation(observation: RecoveryArtifactObservation) -> Self {
        Self { observation }
    }

    fn matches(self, actual: RecoveryArtifactObservation) -> bool {
        self.observation == actual
    }
}

#[cfg(windows)]
fn inspect_windows_recovery_artifact(
    path: &Path,
) -> io::Result<Option<RecoveryArtifactObservation>> {
    let file = match noter_platform::open_existing_no_follow(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    observe_windows_recovery_artifact(path, &file).map(Some)
}

#[cfg(windows)]
#[derive(Debug)]
struct OpenRecoveryArtifact {
    file: File,
    observation: RecoveryArtifactObservation,
}

#[cfg(windows)]
fn open_windows_recovery_artifact_for_cleanup(
    path: &Path,
) -> io::Result<Option<OpenRecoveryArtifact>> {
    let file = match noter_platform::open_for_cleanup(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let observation = observe_windows_recovery_artifact(path, &file)?;
    Ok(Some(OpenRecoveryArtifact { file, observation }))
}

#[cfg(windows)]
fn open_windows_recovery_artifact_for_ratification(
    path: &Path,
) -> io::Result<Option<OpenRecoveryArtifact>> {
    let file = match noter_platform::open_for_reconciliation(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let observation = observe_windows_recovery_artifact(path, &file)?;
    Ok(Some(OpenRecoveryArtifact { file, observation }))
}

#[cfg(windows)]
fn delete_verified_windows_recovery_artifact(artifact: OpenRecoveryArtifact) -> io::Result<()> {
    noter_platform::delete_open_file(&artifact.file)?;
    drop(artifact.file);
    Ok(())
}

#[cfg(windows)]
fn observe_windows_recovery_artifact(
    path: &Path,
    file: &File,
) -> io::Result<RecoveryArtifactObservation> {
    let metadata = file.metadata()?;
    let facts = noter_platform::file_facts(file)?;
    if !metadata.is_file() || facts.link_count() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery replacement candidate is not a private regular file",
        ));
    }
    if exceeds_recovery_file_bound(metadata.len()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery replacement candidate exceeds the supported size",
        ));
    }

    let fingerprint = fingerprint_bound_open_windows_file(file, facts, metadata.len())?;
    revalidate_path_identity(path, facts, metadata.len())?;
    Ok(RecoveryArtifactObservation {
        identity: facts.identity(),
        fingerprint,
        length: metadata.len(),
    })
}

#[cfg(windows)]
fn fingerprint_bound_open_windows_file(
    file: &File,
    expected_facts: noter_platform::FileFacts,
    expected_len: u64,
) -> io::Result<ContentFingerprint> {
    if !file_binding_matches(
        noter_platform::file_facts(file)? == expected_facts,
        file.metadata()?.len() == expected_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery replacement candidate changed before validation",
        ));
    }

    let read_limit = MAX_RECOVERY_FILE_BYTES.checked_add(1).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "recovery replacement read bound cannot be represented",
        )
    })?;
    let mut reader = file;
    reader.seek(SeekFrom::Start(0))?;
    let mut bounded = reader.take(read_limit);
    let fingerprint = ContentFingerprint::from_reader(&mut bounded)?;
    let bytes_read = read_limit.saturating_sub(bounded.limit());
    if !complete_bound_read_matches(
        bytes_read == expected_len,
        noter_platform::file_facts(file)? == expected_facts,
        file.metadata()?.len() == expected_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery replacement candidate changed during validation",
        ));
    }
    Ok(fingerprint)
}

#[cfg(windows)]
fn reconcile_windows_recovery_replace(
    stage: &Path,
    destination: &Path,
    backup: &Path,
    intended: IntendedWindowsRecoveryContent,
    expected: RecoveryArtifactObservation,
    platform_error: io::Error,
) -> Result<RecoveryParentSync, RecoveryCommitFailure> {
    let destination_ratification = open_windows_recovery_artifact_for_ratification(destination)
        .map_err(|error| {
            uncertain_windows_recovery_failure(
                stage,
                backup,
                &error,
                "the destination could not be inspected after replacement failure",
            )
        })?;
    let destination_state = destination_ratification
        .as_ref()
        .map(|artifact| artifact.observation);
    let stage_artifact = open_windows_recovery_artifact_for_cleanup(stage).map_err(|error| {
        uncertain_windows_recovery_failure(
            stage,
            backup,
            &error,
            "the staged recovery snapshot could not be verified after replacement failure",
        )
    })?;
    let backup_artifact = open_windows_recovery_artifact_for_cleanup(backup).map_err(|error| {
        uncertain_windows_recovery_failure(
            stage,
            backup,
            &error,
            "the predecessor recovery backup could not be verified after replacement failure",
        )
    })?;
    let stage_state = stage_artifact.as_ref().map(|artifact| artifact.observation);
    let backup_state = backup_artifact
        .as_ref()
        .map(|artifact| artifact.observation);
    if destination_state.is_some_and(|actual| intended.matches(actual)) {
        drop(stage_artifact);
        drop(backup_artifact);
        return finalize_reconciled_windows_recovery(
            stage,
            destination,
            backup,
            intended,
            expected,
            &platform_error,
            false,
        )
        .map(|()| RecoveryParentSync::UnsupportedAfterReconciliation);
    }

    let stage_is_intended = stage_state.is_some_and(|actual| intended.matches(actual));
    if destination_state == Some(expected) && stage_is_intended && backup_state.is_none() {
        let artifact = stage_artifact.expect("the verified intended stage must be open");
        delete_verified_windows_recovery_artifact(artifact).map_err(|error| {
            uncertain_windows_recovery_failure(
                stage,
                backup,
                &error,
                "the proven non-committing recovery stage could not be cleaned safely",
            )
        })?;
        return Err(RecoveryCommitFailure::preserve_windows_artifacts(
            platform_error,
        ));
    }

    if destination_state.is_none() && stage_is_intended && backup_state == Some(expected) {
        drop(stage_artifact);
        drop(backup_artifact);
        let completion = noter_platform::install_new(stage, destination);
        return match completion {
            Ok(receipt) => {
                let (_outcome, parent_sync) = receipt.into_parts();
                finalize_reconciled_windows_recovery(
                    stage,
                    destination,
                    backup,
                    intended,
                    expected,
                    &platform_error,
                    false,
                )
                .map(|()| RecoveryParentSync::bound(parent_sync))
            }
            Err(completion_error) => finalize_reconciled_windows_recovery(
                stage,
                destination,
                backup,
                intended,
                expected,
                &completion_error,
                false,
            )
            .map(|()| RecoveryParentSync::UnsupportedAfterReconciliation),
        };
    }

    Err(uncertain_windows_recovery_failure(
        stage,
        backup,
        &platform_error,
        "the replacement failure left an unexplained path state",
    ))
}

#[cfg(windows)]
fn finalize_reconciled_windows_recovery(
    stage: &Path,
    destination: &Path,
    backup: &Path,
    intended: IntendedWindowsRecoveryContent,
    expected: RecoveryArtifactObservation,
    cause: &io::Error,
    backup_required: bool,
) -> Result<(), RecoveryCommitFailure> {
    finalize_reconciled_windows_recovery_with_cleanup_hook(
        stage,
        destination,
        backup,
        intended,
        expected,
        cause,
        backup_required,
        || Ok(()),
    )
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn finalize_reconciled_windows_recovery_with_cleanup_hook(
    stage: &Path,
    destination: &Path,
    backup: &Path,
    intended: IntendedWindowsRecoveryContent,
    expected: RecoveryArtifactObservation,
    cause: &io::Error,
    backup_required: bool,
    before_cleanup: impl FnOnce() -> io::Result<()>,
) -> Result<(), RecoveryCommitFailure> {
    let destination_ratification = open_windows_recovery_artifact_for_ratification(destination)
        .map_err(|error| {
            uncertain_windows_recovery_failure(
                stage,
                backup,
                &error,
                "the recovery destination could not be verified after reconciliation",
            )
        })?;
    let destination_state = destination_ratification
        .as_ref()
        .map(|artifact| artifact.observation);
    let stage_artifact = open_windows_recovery_artifact_for_cleanup(stage).map_err(|error| {
        uncertain_windows_recovery_failure(
            stage,
            backup,
            &error,
            "the recovery stage could not be verified after reconciliation",
        )
    })?;
    let backup_artifact = open_windows_recovery_artifact_for_cleanup(backup).map_err(|error| {
        uncertain_windows_recovery_failure(
            stage,
            backup,
            &error,
            "the recovery backup could not be verified after reconciliation",
        )
    })?;
    let stage_state = stage_artifact.as_ref().map(|artifact| artifact.observation);
    let backup_state = backup_artifact
        .as_ref()
        .map(|artifact| artifact.observation);
    let destination_matches = destination_state.is_some_and(|actual| intended.matches(actual));
    let stage_matches_or_is_absent = stage_state.is_none_or(|actual| intended.matches(actual));
    let backup_matches_or_is_absent = if backup_required {
        backup_state == Some(expected)
    } else {
        backup_state.is_none_or(|actual| actual == expected)
    };
    if !reconciled_cleanup_state_is_exact(
        destination_matches,
        stage_matches_or_is_absent,
        backup_matches_or_is_absent,
    ) {
        return Err(uncertain_windows_recovery_failure(
            stage,
            backup,
            cause,
            "the replacement result could not be reconciled exactly",
        ));
    }
    before_cleanup().map_err(|error| {
        uncertain_windows_recovery_failure(
            stage,
            backup,
            &error,
            "recovery cleanup could not proceed after exact verification",
        )
    })?;
    if let Some(artifact) = stage_artifact {
        delete_verified_windows_recovery_artifact(artifact).map_err(|error| {
            uncertain_windows_recovery_failure(
                stage,
                backup,
                &error,
                "the duplicate recovery stage could not be cleaned safely",
            )
        })?;
    }
    if let Some(artifact) = backup_artifact {
        delete_verified_windows_recovery_artifact(artifact).map_err(|error| {
            uncertain_windows_recovery_failure(
                stage,
                backup,
                &error,
                "the predecessor recovery backup could not be cleaned safely",
            )
        })?;
    }
    drop(destination_ratification);
    Ok(())
}

#[cfg(windows)]
fn uncertain_windows_recovery_failure(
    stage: &Path,
    backup: &Path,
    cause: &io::Error,
    detail: &str,
) -> RecoveryCommitFailure {
    let stage = windows_recovery_artifact_label(stage, "a private recovery stage");
    let backup = windows_recovery_artifact_label(backup, "a private recovery backup");
    let os_code = cause
        .raw_os_error()
        .map_or_else(String::new, |code| format!(", OS code {code}"));
    RecoveryCommitFailure::preserve_windows_artifacts(io::Error::new(
        cause.kind(),
        format!(
            "{detail} after {:?}{os_code}. Recovery candidates {stage} and {backup} were preserved when present. Inspect the canonical recovery record and every existing candidate before retrying or removing either candidate.",
            cause.kind()
        ),
    ))
}

#[cfg(windows)]
fn windows_recovery_artifact_label(path: &Path, fallback: &str) -> String {
    path.file_name().map_or_else(
        || fallback.to_owned(),
        |name| format!("`{}`", name.to_string_lossy()),
    )
}

#[cfg(any(windows, test))]
fn exclusive_stage_path(
    parent: &Path,
    instance_id: RecoveryInstanceId,
    kind: TemporaryArtifactKind,
) -> io::Result<PathBuf> {
    let path = parent.join(format!(
        ".noter-recovery-{}-00000000000000000000000000000000.{}",
        hex16(&instance_id.as_bytes()),
        kind.extension()
    ));
    match fs::symlink_metadata(&path) {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::ResourceBusy,
            format!(
                "a retained recovery {} requires startup review before persistence can retry",
                kind.extension()
            ),
        )),
        Err(error) => classify_available_recovery_slot(path, error),
    }
}

#[cfg(any(windows, test))]
fn classify_available_recovery_slot(path: PathBuf, error: io::Error) -> io::Result<PathBuf> {
    if error.kind() == io::ErrorKind::NotFound {
        Ok(path)
    } else {
        Err(error)
    }
}

fn keyed_temporary_instance(path: &Path) -> Option<RecoveryInstanceId> {
    let name = path.file_name()?.to_str()?;
    let remainder = name.strip_prefix(".noter-recovery-")?;
    let (instance_hex, random_and_extension) = remainder.split_once('-')?;
    let (random_hex, extension) = random_and_extension.rsplit_once('.')?;
    if !matches!(extension, "stage" | "backup") {
        return None;
    }
    let _ = decode_hex16(random_hex)?;
    decode_hex16(instance_hex).map(RecoveryInstanceId::new)
}

fn decode_hex16(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32 {
        return None;
    }
    let mut bytes = [0_u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let offset = index.saturating_mul(2);
        *byte = u8::from_str_radix(value.get(offset..offset.saturating_add(2))?, 16).ok()?;
    }
    Some(bytes)
}

fn hex16(bytes: &[u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::edit::Selection;
    use crate::core::recovery::{
        RECOVERY_MAGIC, RECOVERY_SCHEMA_VERSION, RecoveryDocumentId, RecoveryLineageGeneration,
        RecoverySnapshotParts, RecoveryWallTime,
    };
    use crate::core::revision::Revision;
    use crate::core::save::ContentFingerprint;
    use crate::core::text_format::{Bom, Encoding};
    use tempfile::tempdir;

    fn sample_snapshot(instance: u8, content: &[u8]) -> RecoverySnapshot {
        snapshot_with_document(instance, instance, u64::from(instance), 2, content)
    }

    fn indexed_instance(index: usize) -> RecoveryInstanceId {
        let mut bytes = [0_u8; 16];
        bytes[..std::mem::size_of::<usize>()].copy_from_slice(&index.to_le_bytes());
        RecoveryInstanceId::new(bytes)
    }

    fn snapshot_at(
        instance: u8,
        revision: u64,
        updated_at: u64,
        content: &[u8],
    ) -> RecoverySnapshot {
        snapshot_with_document(9, instance, revision, updated_at, content)
    }

    fn snapshot_with_document(
        document: u8,
        instance: u8,
        revision: u64,
        updated_at: u64,
        content: &[u8],
    ) -> RecoverySnapshot {
        RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([document; 16]),
            instance_id: RecoveryInstanceId::new([instance; 16]),
            revision: Revision::new(revision),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(updated_at),
            original_path: b"memo.txt".to_vec(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(content.len().min(1)),
            content: content.to_vec(),
        })
        .expect("snapshot")
    }

    fn offer(entry: &RecoveryScanEntry) -> &RecoveryOffer {
        let RecoveryScanDisposition::Offer(offer) = entry.disposition() else {
            panic!("expected recovery offer, got {:?}", entry.disposition());
        };
        offer
    }

    #[cfg(windows)]
    #[test]
    fn state_bound_store_clones_and_peers_retain_the_namespace() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let moved = parent.path().join("moved-state");
        let first = RecoveryStore::open_in_state(&state)?;
        let cloned = first.clone();
        let peer = RecoveryStore::open_in_state(&state)?;

        assert!(fs::rename(&state, &moved).is_err());
        drop(peer);
        drop(first);
        assert!(
            fs::rename(&state, &moved).is_err(),
            "the final clone must retain every namespace handle"
        );

        drop(cloned);
        fs::rename(&state, &moved)?;
        assert!(!state.exists());
        assert!(moved.is_dir());
        Ok(())
    }

    fn load_offer(
        store: &RecoveryStore,
        entry: &RecoveryScanEntry,
    ) -> io::Result<ValidatedRecoveryRecord> {
        store.load_record(offer(entry).primary())
    }

    fn encode_v1(snapshot: &RecoverySnapshot) -> Vec<u8> {
        const V1_HEADER_LEN: usize = 130;
        let path_len = u32::try_from(snapshot.original_path().len()).expect("fixture path length");
        let content_len = u64::try_from(snapshot.content().len()).expect("fixture content length");
        let checksum = ContentFingerprint::from_bytes(snapshot.content());
        let mut bytes = Vec::with_capacity(
            V1_HEADER_LEN + snapshot.original_path().len() + snapshot.content().len(),
        );
        bytes.extend_from_slice(RECOVERY_MAGIC);
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&snapshot.document_id().as_bytes());
        bytes.extend_from_slice(&snapshot.instance_id().as_bytes());
        bytes.extend_from_slice(&snapshot.revision().get().to_le_bytes());
        bytes.extend_from_slice(&snapshot.created_at().unix_millis().to_le_bytes());
        bytes.extend_from_slice(&snapshot.updated_at().unix_millis().to_le_bytes());
        bytes.extend_from_slice(&path_len.to_le_bytes());
        bytes.push(u8::from(snapshot.bom() == Bom::Utf8));
        bytes.push(0);
        bytes.extend_from_slice(
            &u64::try_from(snapshot.selection().anchor())
                .expect("fixture selection")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &u64::try_from(snapshot.selection().active())
                .expect("fixture selection")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&content_len.to_le_bytes());
        bytes.extend_from_slice(checksum.as_bytes());
        bytes.extend_from_slice(snapshot.original_path());
        bytes.extend_from_slice(snapshot.content());
        bytes
    }

    #[test]
    fn omission_flags_have_independent_exact_semantics() {
        let empty = RecoveryStartupScan::default();
        assert!(!empty.limit_reached());
        assert!(!empty.directory_limit_reached());
        assert!(!empty.byte_limit_reached());
        assert!(!empty.offers_omitted());
        assert!(!empty.superseded_handles_omitted());
        assert!(!empty.has_omissions());

        let cases = [
            (FILE_LIMIT_REACHED, [true, false, false, false, false]),
            (DIRECTORY_LIMIT_REACHED, [false, true, false, false, false]),
            (BYTE_LIMIT_REACHED, [false, false, true, false, false]),
            (OFFERS_OMITTED, [false, false, false, true, false]),
            (
                SUPERSEDED_HANDLES_OMITTED,
                [false, false, false, false, true],
            ),
        ];
        for (flag, expected) in cases {
            let mut scan = RecoveryStartupScan::default();
            scan.note_omission(flag);
            assert_eq!(
                [
                    scan.limit_reached(),
                    scan.directory_limit_reached(),
                    scan.byte_limit_reached(),
                    scan.offers_omitted(),
                    scan.superseded_handles_omitted(),
                ],
                expected
            );
            assert!(scan.has_omissions());
        }

        let quarantine_only = RecoveryStartupScan {
            quarantine_results_omitted: 1,
            ..RecoveryStartupScan::default()
        };
        assert!(quarantine_only.has_omissions());
    }

    #[test]
    fn recovery_directory_missing_classification_is_exact() {
        assert!(recovery_directory_error_is_missing(io::ErrorKind::NotFound));
        assert!(!recovery_directory_error_is_missing(
            io::ErrorKind::PermissionDenied
        ));
        assert!(!recovery_directory_error_is_missing(io::ErrorKind::Other));
    }

    #[test]
    fn recovery_store_decision_predicates_cover_each_independent_input() {
        assert!(valid_live_file_shape(true, 1));
        assert!(!valid_live_file_shape(false, 1));
        assert!(!valid_live_file_shape(true, 2));

        assert!(file_binding_matches(true, true));
        assert!(!file_binding_matches(false, true));
        assert!(!file_binding_matches(true, false));

        assert!(complete_bound_read_matches(true, true, true));
        assert!(!complete_bound_read_matches(false, true, true));
        assert!(!complete_bound_read_matches(true, false, true));
        assert!(!complete_bound_read_matches(true, true, false));

        assert!(stable_live_lease_binding_matches(true, true));
        assert!(!stable_live_lease_binding_matches(false, true));
        assert!(!stable_live_lease_binding_matches(true, false));

        assert!(reconciled_cleanup_state_is_exact(true, true, true));
        assert!(!reconciled_cleanup_state_is_exact(false, true, true));
        assert!(!reconciled_cleanup_state_is_exact(true, false, true));
        assert!(!reconciled_cleanup_state_is_exact(true, true, false));

        assert!(!exceeds_recovery_file_bound(MAX_RECOVERY_FILE_BYTES));
        assert!(exceeds_recovery_file_bound(MAX_RECOVERY_FILE_BYTES + 1));

        let original = classify_nonbusy_lease_error(
            io::Error::new(io::ErrorKind::PermissionDenied, "original lease error"),
            false,
        );
        assert_eq!(original.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(original.to_string(), "original lease error");
        let reclassified = classify_nonbusy_lease_error(
            io::Error::new(io::ErrorKind::PermissionDenied, "original lease error"),
            true,
        );
        assert_eq!(reclassified.kind(), io::ErrorKind::ResourceBusy);
        assert_eq!(
            reclassified.to_string(),
            "recovery live lease is held by another Noter window"
        );

        let busy = classify_live_lease_acquisition_error(
            io::Error::new(io::ErrorKind::ResourceBusy, "exact contention evidence"),
            || panic!("an exact contention result must not be probed again"),
        );
        assert_eq!(busy.kind(), io::ErrorKind::ResourceBusy);
        assert_eq!(busy.to_string(), "exact contention evidence");

        let mut probed = false;
        let inferred_busy = classify_live_lease_acquisition_error(
            io::Error::new(io::ErrorKind::PermissionDenied, "ambiguous lease failure"),
            || {
                probed = true;
                true
            },
        );
        assert!(probed);
        assert_eq!(inferred_busy.kind(), io::ErrorKind::ResourceBusy);

        assert!(is_quarantine_name_collision(io::ErrorKind::AlreadyExists));
        assert!(!is_quarantine_name_collision(
            io::ErrorKind::PermissionDenied
        ));
        assert!(!is_quarantine_name_collision(io::ErrorKind::NotADirectory));

        assert!(requires_path_delete_fallback(io::ErrorKind::Unsupported));
        assert!(!requires_path_delete_fallback(
            io::ErrorKind::PermissionDenied
        ));
        assert!(!requires_path_delete_fallback(io::ErrorKind::ResourceBusy));

        let available = PathBuf::from("available-recovery-slot.stage");
        assert_eq!(
            classify_available_recovery_slot(
                available.clone(),
                io::Error::new(io::ErrorKind::NotFound, "injected absent slot"),
            )
            .expect("only an absent slot is available"),
            available
        );
        let code = if cfg!(windows) { 5 } else { 13 };
        let inaccessible = classify_available_recovery_slot(
            PathBuf::from("inaccessible-recovery-slot.stage"),
            io::Error::from_raw_os_error(code),
        )
        .expect_err("an inaccessible slot must not be treated as absent");
        assert_eq!(inaccessible.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(inaccessible.raw_os_error(), Some(code));
    }

    #[test]
    fn live_lease_fact_comparison_requires_real_identity_and_link_count() -> io::Result<()> {
        let directory = tempdir()?;
        let first_path = directory.path().join("first.live");
        let alias_path = directory.path().join("first-alias.live");
        let second_path = directory.path().join("second.live");
        fs::write(&first_path, b"first lease")?;
        fs::write(&second_path, b"second lease")?;

        let initial = noter_platform::file_facts(&File::open(&first_path)?)?;
        let same = noter_platform::file_facts(&File::open(&first_path)?)?;
        let different = noter_platform::file_facts(&File::open(&second_path)?)?;
        assert!(live_lease_facts_match(same, initial));
        assert!(!live_lease_facts_match(different, initial));

        fs::hard_link(&first_path, &alias_path)?;
        let linked = noter_platform::file_facts(&File::open(&first_path)?)?;
        let alias = noter_platform::file_facts(&File::open(&alias_path)?)?;
        assert_eq!(linked.identity(), initial.identity());
        assert_ne!(linked.link_count(), initial.link_count());
        assert!(!live_lease_facts_match(linked, initial));
        assert!(live_lease_facts_match(linked, alias));
        Ok(())
    }

    #[test]
    fn recovery_instance_sources_reconcile_only_when_they_agree() {
        let first = RecoveryInstanceId::new([1; 16]);
        let second = RecoveryInstanceId::new([2; 16]);
        assert_eq!(
            reconcile_recovery_instance_ids(Some(first), Some(first)),
            Ok(Some(first))
        );
        assert_eq!(
            reconcile_recovery_instance_ids(Some(first), None),
            Ok(Some(first))
        );
        assert_eq!(
            reconcile_recovery_instance_ids(None, Some(first)),
            Ok(Some(first))
        );
        assert_eq!(reconcile_recovery_instance_ids(None, None), Ok(None));
        assert_eq!(
            reconcile_recovery_instance_ids(Some(first), Some(second)),
            Err(())
        );
    }

    #[test]
    fn recovery_artifact_names_and_minimal_headers_are_exact() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(22, b"header");
        let instance_id = snapshot.instance_id();
        assert_eq!(
            recovery_artifact_instance(&store.record_path(instance_id)),
            Some(instance_id)
        );
        let stage = exclusive_stage_path(
            &store.records_dir(),
            instance_id,
            TemporaryArtifactKind::Stage,
        )?;
        let backup = exclusive_stage_path(
            &store.records_dir(),
            instance_id,
            TemporaryArtifactKind::Backup,
        )?;
        assert_eq!(recovery_artifact_instance(&stage), Some(instance_id));
        assert_eq!(recovery_artifact_instance(&backup), Some(instance_id));
        assert_eq!(
            recovery_artifact_instance(&store.records_dir().join("not-a-record.txt")),
            None
        );

        let path = store.records_dir().join("minimal-header.bin");
        let mut header = snapshot.encode();
        header.truncate(44);
        fs::write(&path, &header)?;
        let file = File::open(&path)?;
        assert_eq!(peek_recovery_instance(&file, 43)?, None);
        assert_eq!(peek_recovery_instance(&file, 44)?, Some(instance_id));

        let mut legacy = header.clone();
        legacy[8..12].copy_from_slice(&1_u32.to_le_bytes());
        fs::write(&path, &legacy)?;
        let file = File::open(&path)?;
        assert_eq!(peek_recovery_instance(&file, 44)?, Some(instance_id));

        let mut bad_magic = header.clone();
        bad_magic[0] ^= 1;
        fs::write(&path, bad_magic)?;
        let file = File::open(&path)?;
        assert_eq!(peek_recovery_instance(&file, 44)?, None);

        let mut unknown_schema = header;
        unknown_schema[8..12].copy_from_slice(&99_u32.to_le_bytes());
        fs::write(&path, unknown_schema)?;
        let file = File::open(&path)?;
        assert_eq!(peek_recovery_instance(&file, 44)?, None);
        Ok(())
    }

    #[test]
    fn quarantine_copy_verification_rejects_mismatch_and_trailing_bytes() -> io::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("quarantine-copy.bin");
        fs::write(&path, b"exact bytes")?;

        let mut file = File::open(&path)?;
        verify_exact_bytes(&mut file, b"exact bytes")?;

        let mut file = File::open(&path)?;
        let mismatch = verify_exact_bytes(&mut file, b"exact bytez")
            .expect_err("a byte mismatch must fail verification");
        assert_eq!(mismatch.kind(), io::ErrorKind::InvalidData);

        let mut file = File::open(&path)?;
        let trailing = verify_exact_bytes(&mut file, b"exact")
            .expect_err("trailing bytes must fail verification");
        assert_eq!(trailing.kind(), io::ErrorKind::InvalidData);
        Ok(())
    }

    #[test]
    fn persist_scan_and_delete_round_trip() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(1, b"recovered text");
        store.persist(&snapshot)?;

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(!offer(&entries[0]).superseded_omitted());
        let record = load_offer(&store, &entries[0])?;
        assert_eq!(record.content(), b"recovered text");
        assert_eq!(record.instance_id(), snapshot.instance_id());
        assert_eq!(record.original_path(), b"memo.txt");
        assert!(entries[0].remains_in_records());
        assert!(entries[0].quarantine_error().is_none());

        // No stage or backup siblings should remain after a clean persist.
        let leftovers: Vec<_> = fs::read_dir(store.records_dir())?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| {
                name.to_string_lossy().starts_with(".noter-recovery-")
                    || name.to_string_lossy().ends_with(".tmp")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "unexpected recovery stage leftovers: {leftovers:?}"
        );

        store.persist(&sample_snapshot(1, b"updated"))?;
        let entries = store.scan_startup()?;
        assert_eq!(
            entries.len(),
            1,
            "unexpected recovery entries: {entries:#?}"
        );
        assert_eq!(load_offer(&store, &entries[0])?.content(), b"updated");

        store.delete_owned_artifacts(snapshot.instance_id())?;
        assert!(
            store
                .scan_startup()?
                .iter()
                .all(|entry| !matches!(entry.disposition(), RecoveryScanDisposition::Offer(_)))
        );
        store.delete_owned_artifacts(snapshot.instance_id())?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn atomic_recovery_install_syncs_bound_parent_after_path_rebind() -> io::Result<()> {
        let directory = tempdir()?;
        let active = directory.path().join("active");
        let moved = directory.path().join("moved");
        fs::create_dir(&active)?;
        let destination = active.join("record.bin");
        let snapshot = snapshot_at(60, 1, 10, b"bound recovery bytes");
        let bytes = snapshot.encode();
        let sync_called = std::cell::Cell::new(false);

        write_atomic_private_with_sync(
            &destination,
            snapshot.instance_id(),
            &bytes,
            |_stage, _destination, _backup| {
                panic!("an absent recovery destination must use exclusive installation")
            },
            |parent_sync| {
                sync_called.set(true);
                fs::rename(&active, &moved)?;
                fs::create_dir(&active)?;
                parent_sync.sync()
            },
        )?;

        assert!(sync_called.get());
        assert_eq!(fs::read(moved.join("record.bin"))?, bytes);
        assert!(!active.join("record.bin").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unix_recovery_commit_consumes_the_reserved_stage() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("record.bin");
        let first = snapshot_at(60, 6, 15, b"first recovery");
        let second = snapshot_at(60, 7, 16, b"second recovery");
        let stage = unix_recovery_stage_path(directory.path(), first.instance_id());

        write_atomic_private_unix(&destination, first.instance_id(), &first.encode())?;
        assert_eq!(fs::read(&destination)?, first.encode());
        assert!(!stage.exists());

        write_atomic_private_unix(&destination, second.instance_id(), &second.encode())?;
        assert_eq!(fs::read(&destination)?, second.encode());
        assert!(!stage.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unix_recovery_commit_stays_in_original_parent_during_rebind() -> io::Result<()> {
        let directory = tempdir()?;
        let active = directory.path().join("active");
        let moved = directory.path().join("moved");
        fs::create_dir(&active)?;
        let destination = active.join("record.bin");
        let snapshot = snapshot_at(60, 8, 17, b"descriptor-bound recovery");
        let encoded = snapshot.encode();

        write_atomic_private_unix_with(&destination, snapshot.instance_id(), &encoded, |stage| {
            let stage_name = stage
                .file_name()
                .expect("the reserved stage has a basename")
                .to_owned();
            fs::rename(&active, &moved)?;
            fs::create_dir(&active)?;
            fs::write(active.join(&stage_name), b"rebound stage")?;
            fs::write(active.join("record.bin"), b"rebound destination")
        })?;

        assert_eq!(fs::read(moved.join("record.bin"))?, encoded);
        assert_eq!(fs::read(active.join("record.bin"))?, b"rebound destination");
        assert_eq!(
            fs::read(unix_recovery_stage_path(&active, snapshot.instance_id()))?,
            b"rebound stage"
        );
        assert!(!unix_recovery_stage_path(&moved, snapshot.instance_id()).exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn injected_exchange_retains_displaced_sibling_after_parent_rebind() -> io::Result<()> {
        let directory = tempdir()?;
        let active = directory.path().join("active");
        let moved = directory.path().join("moved");
        fs::create_dir(&active)?;
        let destination = active.join("record.bin");
        let predecessor = snapshot_at(60, 2, 11, b"predecessor recovery");
        let predecessor_bytes = predecessor.encode();
        let intended = snapshot_at(60, 3, 12, b"intended recovery");
        let intended_bytes = intended.encode();
        fs::write(&destination, &predecessor_bytes)?;
        let rebound_stage = std::cell::RefCell::new(None);
        let rebound_backup = std::cell::RefCell::new(None);

        write_atomic_private_with_sync(
            &destination,
            intended.instance_id(),
            &intended_bytes,
            |stage, destination, backup| {
                let receipt = noter_platform::replace_existing(stage, destination, backup)?;
                fs::rename(&active, &moved)?;
                fs::create_dir(&active)?;
                fs::write(stage, b"rebound stage")?;
                let backup = backup.expect("recovery replacement reserves a backup name");
                fs::write(backup, b"rebound backup")?;
                rebound_stage.replace(Some(stage.to_path_buf()));
                rebound_backup.replace(Some(backup.to_path_buf()));
                Ok(receipt)
            },
            RecoveryParentSync::sync,
        )?;

        let rebound_stage = rebound_stage
            .into_inner()
            .expect("replacement should record the rebound stage");
        let rebound_backup = rebound_backup
            .into_inner()
            .expect("replacement should record the rebound backup");
        let stage_name = rebound_stage
            .file_name()
            .expect("stage path should have a filename");
        assert_eq!(fs::read(moved.join("record.bin"))?, intended_bytes);
        assert_eq!(fs::read(moved.join(stage_name))?, predecessor_bytes);
        assert_eq!(fs::read(rebound_stage)?, b"rebound stage");
        assert_eq!(fs::read(rebound_backup)?, b"rebound backup");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unix_recovery_retry_refuses_a_second_retained_stage() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("record.bin");
        let snapshot = snapshot_at(60, 4, 13, b"bounded retained recovery");
        let stage = unix_recovery_stage_path(directory.path(), snapshot.instance_id());
        fs::write(&stage, b"retained partial recovery")?;

        let error =
            write_atomic_private_unix(&destination, snapshot.instance_id(), &snapshot.encode())
                .expect_err("a retained stage must stop retries from accumulating artifacts");

        assert_eq!(error.kind(), io::ErrorKind::ResourceBusy);
        assert_eq!(fs::read(&stage)?, b"retained partial recovery");
        assert!(!destination.exists());
        let keyed: Vec<_> = fs::read_dir(directory.path())?
            .filter_map(Result::ok)
            .filter(|entry| keyed_temporary_instance(&entry.path()) == Some(snapshot.instance_id()))
            .collect();
        assert_eq!(keyed.len(), 1);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn atomic_recovery_parent_sync_failure_preserves_committed_record() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("record.bin");
        let snapshot = snapshot_at(60, 2, 11, b"committed before barrier failure");
        let bytes = snapshot.encode();

        let error = write_atomic_private_with_sync(
            &destination,
            snapshot.instance_id(),
            &bytes,
            |_stage, _destination, _backup| {
                panic!("an absent recovery destination must use exclusive installation")
            },
            |_parent_sync| Err(io::Error::other("injected parent barrier failure")),
        )
        .expect_err("a failed post-commit parent barrier must be reported");

        assert!(
            error
                .to_string()
                .contains("injected parent barrier failure")
        );
        assert_eq!(fs::read(&destination)?, bytes);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn successful_windows_replace_with_mismatched_destination_preserves_backup() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("record.bin");
        let predecessor = snapshot_at(60, 3, 12, b"predecessor recovery");
        let intended = snapshot_at(60, 4, 13, b"intended recovery");
        let unexpected = snapshot_at(60, 5, 14, b"unexpected recovery");
        let predecessor_bytes = predecessor.encode();
        let intended_bytes = intended.encode();
        let unexpected_bytes = unexpected.encode();
        fs::write(&destination, &predecessor_bytes)?;
        let artifact_paths = std::cell::RefCell::new(None);

        let error = write_atomic_private_windows_with(
            &destination,
            intended.instance_id(),
            &intended_bytes,
            |stage, destination, backup| {
                let backup = backup.expect("Windows replacement reserves a backup");
                artifact_paths.replace(Some((stage.to_path_buf(), backup.to_path_buf())));
                let receipt = noter_platform::replace_existing(stage, destination, Some(backup))?;
                fs::write(destination, &unexpected_bytes)?;
                Ok(receipt)
            },
        )
        .expect_err("post-success destination mismatch must fail closed");

        let (stage, backup) = artifact_paths
            .into_inner()
            .expect("the injected replacement should record its artifacts");
        assert!(
            error
                .to_string()
                .contains("could not be reconciled exactly")
        );
        assert_eq!(fs::read(&destination)?, unexpected_bytes);
        assert!(!stage.exists());
        assert_eq!(fs::read(&backup)?, predecessor_bytes);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn successful_windows_replace_rejects_an_identical_rebound_destination() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("record.bin");
        let displaced_destination = directory.path().join("displaced-record.bin");
        let predecessor = snapshot_at(60, 6, 15, b"predecessor recovery");
        let intended = snapshot_at(60, 7, 16, b"intended recovery");
        let predecessor_bytes = predecessor.encode();
        let intended_bytes = intended.encode();
        fs::write(&destination, &predecessor_bytes)?;
        let artifact_paths = std::cell::RefCell::new(None);

        let error = write_atomic_private_windows_with(
            &destination,
            intended.instance_id(),
            &intended_bytes,
            |stage, destination, backup| {
                let backup = backup.expect("Windows replacement reserves a backup");
                artifact_paths.replace(Some((stage.to_path_buf(), backup.to_path_buf())));
                let receipt = noter_platform::replace_existing(stage, destination, Some(backup))?;
                fs::rename(destination, &displaced_destination)?;
                fs::write(destination, &intended_bytes)?;
                Ok(receipt)
            },
        )
        .expect_err("an identical pathname rebound must fail exact reconciliation");

        let (stage, backup) = artifact_paths
            .into_inner()
            .expect("the injected replacement should record its artifacts");
        assert!(
            error
                .to_string()
                .contains("could not be reconciled exactly")
        );
        assert_eq!(fs::read(&destination)?, intended_bytes);
        assert_eq!(fs::read(&displaced_destination)?, intended_bytes);
        assert!(!stage.exists());
        assert_eq!(fs::read(&backup)?, predecessor_bytes);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn reconciled_windows_cleanup_deletes_only_the_verified_backup_handle() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("record.bin");
        let stage = directory.path().join("record.stage");
        let backup = directory.path().join("record.backup");
        let displaced_backup = directory.path().join("displaced.backup");
        let predecessor = snapshot_at(60, 6, 15, b"predecessor recovery").encode();
        let intended = snapshot_at(60, 7, 16, b"intended recovery").encode();
        fs::write(&destination, &intended)?;
        fs::write(&backup, &predecessor)?;
        let intended = IntendedWindowsRecoveryContent::from_observation(
            inspect_windows_recovery_artifact(&destination)?
                .expect("the intended destination should be inspectable"),
        );
        let expected = inspect_windows_recovery_artifact(&backup)?
            .expect("the predecessor backup should be inspectable");
        let cause = io::Error::other("injected successful replacement");

        finalize_reconciled_windows_recovery_with_cleanup_hook(
            &stage,
            &destination,
            &backup,
            intended,
            expected,
            &cause,
            true,
            || {
                fs::rename(&backup, &displaced_backup)?;
                fs::write(&backup, b"rebound backup")
            },
        )
        .map_err(|failure| failure.error)?;

        assert_eq!(
            fs::read(&destination)?,
            snapshot_at(60, 7, 16, b"intended recovery").encode()
        );
        assert!(!displaced_backup.exists());
        assert_eq!(fs::read(&backup)?, b"rebound backup");
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn reconciled_windows_cleanup_blocks_destination_rewrite() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("record.bin");
        let stage = directory.path().join("record.stage");
        let backup = directory.path().join("record.backup");
        let predecessor = snapshot_at(60, 9, 18, b"predecessor recovery").encode();
        let intended_bytes = snapshot_at(60, 10, 19, b"intended recovery").encode();
        let unexpected = snapshot_at(60, 11, 20, b"unexpected recovery").encode();
        fs::write(&destination, &intended_bytes)?;
        fs::write(&backup, &predecessor)?;
        let intended = IntendedWindowsRecoveryContent::from_observation(
            inspect_windows_recovery_artifact(&destination)?
                .expect("the intended destination should be inspectable"),
        );
        let expected = inspect_windows_recovery_artifact(&backup)?
            .expect("the predecessor backup should be inspectable");
        let cause = io::Error::other("injected successful replacement");

        finalize_reconciled_windows_recovery_with_cleanup_hook(
            &stage,
            &destination,
            &backup,
            intended,
            expected,
            &cause,
            true,
            || {
                assert!(
                    fs::write(&destination, &unexpected).is_err(),
                    "ratification must deny an in-place destination rewrite"
                );
                Ok(())
            },
        )
        .map_err(|failure| failure.error)?;

        assert_eq!(fs::read(&destination)?, intended_bytes);
        assert!(!backup.exists());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_retry_refuses_to_accumulate_retained_recovery_artifacts() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("record.bin");
        let predecessor = snapshot_at(60, 14, 23, b"predecessor recovery");
        let intended = snapshot_at(60, 15, 24, b"intended recovery");
        fs::write(&destination, predecessor.encode())?;
        let stage = exclusive_stage_path(
            directory.path(),
            intended.instance_id(),
            TemporaryArtifactKind::Stage,
        )?;
        let backup = exclusive_stage_path(
            directory.path(),
            intended.instance_id(),
            TemporaryArtifactKind::Backup,
        )?;
        let cleanup_blocker = std::cell::RefCell::new(None);

        let first = write_atomic_private_with_sync(
            &destination,
            intended.instance_id(),
            &intended.encode(),
            |stage, destination, backup| {
                fs::rename(destination, backup.expect("replacement reserves a backup"))?;
                fs::rename(stage, destination)?;
                cleanup_blocker.replace(Some(noter_platform::open_for_reconciliation(
                    backup.expect("replacement reserves a backup"),
                )?));
                Err(io::Error::other("injected post-commit cleanup failure"))
            },
            |_| panic!("an uncertain replacement must not reach parent sync"),
        )
        .expect_err("the blocked exact backup cleanup must fail closed");
        assert_ne!(first.kind(), io::ErrorKind::ResourceBusy);
        assert!(!stage.exists());
        assert!(backup.exists());

        let second = write_atomic_private_with_sync(
            &destination,
            intended.instance_id(),
            &intended.encode(),
            |_stage, _destination, _backup| {
                panic!("a retained deterministic slot must stop replacement")
            },
            |_| panic!("a retained deterministic slot must stop parent sync"),
        )
        .expect_err("a retained backup must gate repeated scheduled retries");
        assert_eq!(second.kind(), io::ErrorKind::ResourceBusy);
        let retained: Vec<_> = fs::read_dir(directory.path())?
            .filter_map(Result::ok)
            .filter(|entry| keyed_temporary_instance(&entry.path()) == Some(intended.instance_id()))
            .collect();
        assert_eq!(retained.len(), 1);
        assert_eq!(fs::read(&destination)?, intended.encode());

        drop(cleanup_blocker.into_inner());
        fs::remove_file(backup)?;
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn reconciled_windows_cleanup_blocks_destination_path_rebind() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("record.bin");
        let displaced = directory.path().join("displaced.bin");
        let stage = directory.path().join("record.stage");
        let backup = directory.path().join("record.backup");
        let predecessor = snapshot_at(60, 12, 21, b"predecessor recovery").encode();
        let intended_bytes = snapshot_at(60, 13, 22, b"intended recovery").encode();
        fs::write(&destination, &intended_bytes)?;
        fs::write(&backup, &predecessor)?;
        let intended = IntendedWindowsRecoveryContent::from_observation(
            inspect_windows_recovery_artifact(&destination)?
                .expect("the intended destination should be inspectable"),
        );
        let expected = inspect_windows_recovery_artifact(&backup)?
            .expect("the predecessor backup should be inspectable");
        let cause = io::Error::other("injected successful replacement");

        finalize_reconciled_windows_recovery_with_cleanup_hook(
            &stage,
            &destination,
            &backup,
            intended,
            expected,
            &cause,
            true,
            || {
                assert!(
                    fs::rename(&destination, &displaced).is_err(),
                    "ratification must deny a destination pathname rebind"
                );
                Ok(())
            },
        )
        .map_err(|failure| failure.error)?;

        assert_eq!(fs::read(&destination)?, intended_bytes);
        assert!(!displaced.exists());
        assert!(!backup.exists());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_stage_cleanup_deletes_only_the_verified_open_object() -> io::Result<()> {
        let directory = tempdir()?;
        let stage = directory.path().join("record.stage");
        let displaced_stage = directory.path().join("displaced.stage");
        fs::write(&stage, snapshot_at(60, 8, 17, b"verified stage").encode())?;
        let artifact = open_windows_recovery_artifact_for_cleanup(&stage)?
            .expect("the recovery stage should be inspectable");

        fs::rename(&stage, &displaced_stage)?;
        fs::write(&stage, b"rebound stage")?;
        delete_verified_windows_recovery_artifact(artifact)?;

        assert!(!displaced_stage.exists());
        assert_eq!(fs::read(&stage)?, b"rebound stage");
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_recovery_artifact_opening_classifies_each_outcome() -> io::Result<()> {
        use std::os::windows::fs::OpenOptionsExt as _;

        let directory = tempdir()?;
        let present = directory.path().join("present.rec");
        let missing = directory.path().join("missing.rec");
        let blocked = directory.path().join("blocked.rec");
        let bytes = snapshot_at(60, 20, 29, b"bound recovery content").encode();
        fs::write(&present, &bytes)?;
        fs::write(&blocked, &bytes)?;

        let cleanup = open_windows_recovery_artifact_for_cleanup(&present)?
            .expect("an existing regular recovery artifact must open for cleanup");
        assert_eq!(cleanup.observation.length, bytes.len() as u64);
        assert_eq!(
            cleanup.observation.fingerprint,
            ContentFingerprint::from_bytes(&bytes)
        );
        drop(cleanup);

        let ratification = open_windows_recovery_artifact_for_ratification(&present)?
            .expect("an existing regular recovery artifact must open for ratification");
        assert_eq!(ratification.observation.length, bytes.len() as u64);
        assert_eq!(
            ratification.observation.fingerprint,
            ContentFingerprint::from_bytes(&bytes)
        );
        drop(ratification);

        assert!(open_windows_recovery_artifact_for_cleanup(&missing)?.is_none());
        assert!(open_windows_recovery_artifact_for_ratification(&missing)?.is_none());

        let exclusive = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(&blocked)?;
        let cleanup_error = open_windows_recovery_artifact_for_cleanup(&blocked)
            .expect_err("a sharing failure must not be classified as a missing artifact");
        assert_ne!(cleanup_error.kind(), io::ErrorKind::NotFound);
        let ratification_error = open_windows_recovery_artifact_for_ratification(&blocked)
            .expect_err("a sharing failure must not be classified as a missing artifact");
        assert_ne!(ratification_error.kind(), io::ErrorKind::NotFound);
        drop(exclusive);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_recovery_observation_binds_content_length_and_single_link() -> io::Result<()> {
        let directory = tempdir()?;
        let primary = directory.path().join("primary.rec");
        let other = directory.path().join("other.rec");
        let alias = directory.path().join("primary-alias.rec");
        let bytes = snapshot_at(60, 21, 30, b"exact observed recovery").encode();
        fs::write(&primary, &bytes)?;
        fs::write(&other, b"different recovery bytes")?;

        let file = noter_platform::open_existing_no_follow(&primary)?;
        let facts = noter_platform::file_facts(&file)?;
        let observation = observe_windows_recovery_artifact(&primary, &file)?;
        assert_eq!(observation.identity, facts.identity());
        assert_eq!(observation.length, bytes.len() as u64);
        assert_eq!(
            observation.fingerprint,
            ContentFingerprint::from_bytes(&bytes)
        );

        let other_file = noter_platform::open_existing_no_follow(&other)?;
        let other_facts = noter_platform::file_facts(&other_file)?;
        let wrong_facts =
            fingerprint_bound_open_windows_file(&file, other_facts, bytes.len() as u64)
                .expect_err("a different file identity must fail bound fingerprinting");
        assert_eq!(wrong_facts.kind(), io::ErrorKind::InvalidData);
        let wrong_length = fingerprint_bound_open_windows_file(
            &file,
            facts,
            u64::try_from(bytes.len())
                .expect("fixture length")
                .saturating_add(1),
        )
        .expect_err("a different expected length must fail bound fingerprinting");
        assert_eq!(wrong_length.kind(), io::ErrorKind::InvalidData);
        drop(file);

        fs::hard_link(&primary, &alias)?;
        let linked_file = noter_platform::open_existing_no_follow(&primary)?;
        let linked = observe_windows_recovery_artifact(&primary, &linked_file)
            .expect_err("a multiply linked recovery artifact is not private");
        assert_eq!(linked.kind(), io::ErrorKind::InvalidData);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_recovery_artifact_labels_are_exact() {
        assert_eq!(
            windows_recovery_artifact_label(
                Path::new(r"C:\recovery\snapshot.stage"),
                "fallback stage"
            ),
            "`snapshot.stage`"
        );
        assert_eq!(
            windows_recovery_artifact_label(Path::new(r"C:\"), "fallback stage"),
            "fallback stage"
        );
    }

    #[cfg(windows)]
    #[test]
    fn documented_windows_partial_recovery_replace_is_completed_safely() -> io::Result<()> {
        const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1_177;

        let dir = tempdir()?;
        let destination = dir.path().join("record.bin");
        let predecessor = snapshot_at(61, 1, 10, b"predecessor recovery");
        let intended = snapshot_at(61, 2, 11, b"intended recovery");
        let predecessor_bytes = predecessor.encode();
        let intended_bytes = intended.encode();
        fs::write(&destination, &predecessor_bytes)?;
        let artifact_paths = std::cell::RefCell::new(None);

        write_atomic_private_windows_with(
            &destination,
            intended.instance_id(),
            &intended_bytes,
            |stage, destination, backup| {
                let backup = backup.expect("Windows replacement reserves a backup");
                artifact_paths.replace(Some((stage.to_path_buf(), backup.to_path_buf())));
                fs::rename(destination, backup)?;
                Err(io::Error::from_raw_os_error(
                    ERROR_UNABLE_TO_MOVE_REPLACEMENT_2,
                ))
            },
        )?;

        let (stage, backup) = artifact_paths
            .into_inner()
            .expect("the injected replacement should record its artifacts");
        assert_eq!(fs::read(&destination)?, intended_bytes);
        assert!(matches!(
            validate_recovery_record(&fs::read(&destination)?),
            RecoveryStartupDisposition::Offer(_)
        ));
        assert!(!stage.exists());
        assert!(!backup.exists());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn unexplained_windows_partial_recovery_state_preserves_valid_snapshots() -> io::Result<()> {
        const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1_177;

        let dir = tempdir()?;
        let destination = dir.path().join("record.bin");
        let predecessor = snapshot_at(62, 1, 10, b"predecessor recovery");
        let intended = snapshot_at(62, 2, 11, b"intended recovery");
        let predecessor_bytes = predecessor.encode();
        let intended_bytes = intended.encode();
        fs::write(&destination, &predecessor_bytes)?;
        let artifact_paths = std::cell::RefCell::new(None);

        let error = write_atomic_private_windows_with(
            &destination,
            intended.instance_id(),
            &intended_bytes,
            |stage, destination, backup| {
                let backup = backup.expect("Windows replacement reserves a backup");
                artifact_paths.replace(Some((stage.to_path_buf(), backup.to_path_buf())));
                fs::remove_file(destination)?;
                Err(io::Error::from_raw_os_error(
                    ERROR_UNABLE_TO_MOVE_REPLACEMENT_2,
                ))
            },
        )
        .expect_err("an unexplained partial replacement must remain an error");

        let (stage, backup) = artifact_paths
            .into_inner()
            .expect("the injected replacement should record its artifacts");
        assert!(error.to_string().contains("were preserved when present"));
        assert!(!destination.exists());
        assert_eq!(fs::read(&stage)?, intended_bytes);
        assert!(matches!(
            validate_recovery_record(&fs::read(&stage)?),
            RecoveryStartupDisposition::Offer(_)
        ));
        assert!(!backup.exists());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn unchanged_windows_replace_failure_cleans_only_the_intended_stage() -> io::Result<()> {
        const ERROR_ACCESS_DENIED: i32 = 5;

        let dir = tempdir()?;
        let destination = dir.path().join("record.bin");
        let predecessor = snapshot_at(63, 1, 10, b"predecessor recovery");
        let intended = snapshot_at(63, 2, 11, b"intended recovery");
        let predecessor_bytes = predecessor.encode();
        let intended_bytes = intended.encode();
        fs::write(&destination, &predecessor_bytes)?;
        let artifact_paths = std::cell::RefCell::new(None);

        write_atomic_private_windows_with(
            &destination,
            intended.instance_id(),
            &intended_bytes,
            |stage, _destination, backup| {
                let backup = backup.expect("Windows replacement reserves a backup");
                artifact_paths.replace(Some((stage.to_path_buf(), backup.to_path_buf())));
                Err(io::Error::from_raw_os_error(ERROR_ACCESS_DENIED))
            },
        )
        .expect_err("a proven non-commit must return the replacement error");

        let (stage, backup) = artifact_paths
            .into_inner()
            .expect("the injected replacement should record its artifacts");
        assert_eq!(fs::read(&destination)?, predecessor_bytes);
        assert!(!stage.exists());
        assert!(!backup.exists());
        Ok(())
    }

    /// A missing artifact is absence; any other failure must stay an error.
    ///
    /// Startup review calls this for every candidate path. Collapsing a real
    /// access failure into "not present" would silently drop a recovery record
    /// the user still needs, so both sides of the guard are exercised here.
    #[cfg(windows)]
    #[test]
    fn windows_artifact_inspection_separates_absence_from_failure() -> io::Result<()> {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempdir()?;

        // Absent path: reported as no artifact, not as an error.
        let missing = dir.path().join("not-created.rec");
        assert!(inspect_windows_recovery_artifact(&missing)?.is_none());

        // Present path: reported as an artifact.
        let present = dir.path().join("present.rec");
        fs::write(&present, b"recovery artifact bytes")?;
        assert!(inspect_windows_recovery_artifact(&present)?.is_some());

        // Present but unopenable: a sharing violation is not absence.
        let locked = dir.path().join("locked.rec");
        fs::write(&locked, b"recovery artifact bytes")?;
        let _exclusive = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&locked)?;
        let error = inspect_windows_recovery_artifact(&locked)
            .expect_err("a denied open must not be reported as a missing artifact");
        assert_ne!(error.kind(), io::ErrorKind::NotFound);

        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn unexplained_nonpartial_windows_replace_failure_preserves_every_snapshot() -> io::Result<()> {
        const ERROR_ACCESS_DENIED: i32 = 5;

        let dir = tempdir()?;
        let destination = dir.path().join("record.bin");
        let predecessor = snapshot_at(64, 1, 10, b"predecessor recovery");
        let intended = snapshot_at(64, 2, 11, b"intended recovery");
        let unexpected = snapshot_at(64, 3, 12, b"unexpected concurrent recovery");
        let predecessor_bytes = predecessor.encode();
        let intended_bytes = intended.encode();
        let unexpected_bytes = unexpected.encode();
        fs::write(&destination, &predecessor_bytes)?;
        let artifact_paths = std::cell::RefCell::new(None);

        let error = write_atomic_private_windows_with(
            &destination,
            intended.instance_id(),
            &intended_bytes,
            |stage, destination, backup| {
                let backup = backup.expect("Windows replacement reserves a backup");
                artifact_paths.replace(Some((stage.to_path_buf(), backup.to_path_buf())));
                fs::rename(destination, backup)?;
                fs::write(destination, &unexpected_bytes)?;
                Err(io::Error::from_raw_os_error(ERROR_ACCESS_DENIED))
            },
        )
        .expect_err("an unexplained replacement failure must remain an error");

        let (stage, backup) = artifact_paths
            .into_inner()
            .expect("the injected replacement should record its artifacts");
        assert!(error.to_string().contains("were preserved when present"));
        for bytes in [
            fs::read(&destination)?,
            fs::read(&stage)?,
            fs::read(&backup)?,
        ] {
            assert!(matches!(
                validate_recovery_record(&bytes),
                RecoveryStartupDisposition::Offer(_)
            ));
        }
        assert_eq!(fs::read(&destination)?, unexpected_bytes);
        assert_eq!(fs::read(&stage)?, intended_bytes);
        assert_eq!(fs::read(&backup)?, predecessor_bytes);
        Ok(())
    }

    #[test]
    fn startup_coalesces_interrupted_replacement_to_newest_revision() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let old = snapshot_at(7, 1, 10, b"old recovery");
        let newest = snapshot_at(7, 2, 11, b"new recovery");
        store.persist(&newest)?;
        let retained_stage = store.records_dir().join(".noter-recovery-interrupted.tmp");
        fs::write(&retained_stage, old.encode())?;

        let entries = store.scan_startup()?;
        let offers: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) => Some(offer),
                RecoveryScanDisposition::Quarantine(_) => None,
            })
            .collect();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].metadata().revision(), Revision::new(2));
        assert_eq!(
            store.load_record(offers[0].primary())?.content(),
            b"new recovery"
        );
        assert_eq!(offers[0].superseded().len(), 1);
        let entry = entries
            .into_entries()
            .pop()
            .expect("one coalesced recovery offer");
        let (_, RecoveryScanDisposition::Offer(offer)) = entry.into_parts() else {
            panic!("expected coalesced recovery offer");
        };
        for handle in offer.into_cleanup_handles() {
            store.delete_offered_record(handle)?;
        }
        assert!(!retained_stage.exists());
        Ok(())
    }

    #[test]
    fn same_instance_with_different_documents_remains_incomparable() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let first = snapshot_with_document(1, 7, 1, 1, b"first document");
        let second = snapshot_with_document(2, 7, 2, 2, b"second document");
        fs::write(store.record_path(first.instance_id()), first.encode())?;
        fs::write(
            store.records_dir().join("same-instance-other-document.rec"),
            second.encode(),
        )?;

        let scan = store.scan_startup()?;

        assert_eq!(
            scan.iter()
                .filter(|entry| matches!(entry.disposition(), RecoveryScanDisposition::Offer(_)))
                .count(),
            2
        );
        Ok(())
    }

    #[test]
    fn same_instance_equal_revision_with_different_content_remains_incomparable() -> io::Result<()>
    {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let first = snapshot_at(7, 3, 3, b"first branch");
        let second = snapshot_at(7, 3, 3, b"second branch");
        fs::write(store.record_path(first.instance_id()), first.encode())?;
        fs::write(
            store.records_dir().join("same-revision-other-content.rec"),
            second.encode(),
        )?;

        let scan = store.scan_startup()?;

        assert_eq!(
            scan.iter()
                .filter(|entry| matches!(entry.disposition(), RecoveryScanDisposition::Offer(_)))
                .count(),
            2
        );
        Ok(())
    }

    #[test]
    fn startup_coalesces_restore_successors_by_document_lineage() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let predecessor = snapshot_at(3, 20, 9_999, b"same recovered text");
        let successor = RecoverySnapshot::try_new_with_lineage(
            RecoverySnapshotParts {
                document_id: predecessor.document_id(),
                instance_id: RecoveryInstanceId::new([4; 16]),
                revision: Revision::new(0),
                created_at: RecoveryWallTime::from_unix_millis(2),
                updated_at: RecoveryWallTime::from_unix_millis(1),
                original_path: b"memo.txt".to_vec(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::caret(1),
                content: b"same recovered text".to_vec(),
            },
            RecoveryLineageGeneration::new(1),
            Some(predecessor.instance_id()),
        )
        .expect("causal successor");
        store.persist(&predecessor)?;
        store.persist(&successor)?;

        let entries = store.scan_startup()?;
        let offers: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) => Some(offer),
                RecoveryScanDisposition::Quarantine(_) => None,
            })
            .collect();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].metadata().instance_id(), successor.instance_id());
        assert_eq!(offers[0].superseded().len(), 1);
        Ok(())
    }

    #[test]
    fn incomparable_v2_and_legacy_branches_remain_separate_offers() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let first = snapshot_with_document(12, 1, 5, 9_999, b"first branch");
        let second = snapshot_with_document(12, 2, 1, 1, b"second branch");
        store.persist(&first)?;
        store.persist(&second)?;

        let scan = store.scan_startup()?;
        let v2_offers: Vec<_> = scan
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) => Some(offer),
                RecoveryScanDisposition::Quarantine(_) => None,
            })
            .collect();
        assert_eq!(v2_offers.len(), 2);
        assert!(v2_offers.iter().all(|offer| offer.superseded().is_empty()));
        drop(scan);

        let legacy_one = snapshot_with_document(13, 3, 5, 9_999, b"legacy one");
        let legacy_two = snapshot_with_document(13, 4, 1, 1, b"legacy two");
        fs::write(
            store.records_dir().join("legacy-one.rec"),
            encode_v1(&legacy_one),
        )?;
        fs::write(
            store.records_dir().join("legacy-two.rec"),
            encode_v1(&legacy_two),
        )?;
        let scan = store.scan_startup()?;
        let legacy_offers = scan
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) if offer.metadata().schema_version() == 1 => {
                    Some(offer)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(legacy_offers.len(), 2);
        assert!(
            legacy_offers
                .iter()
                .all(|offer| offer.metadata().lineage_generation().is_none())
        );
        Ok(())
    }

    #[test]
    fn legacy_metadata_cannot_suppress_a_current_recovery_record() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let genuine = snapshot_with_document(15, 15, 5, 5, b"genuine newest bytes");
        store.persist(&genuine)?;
        let legacy = snapshot_with_document(15, 15, 6, 6, b"legacy alternate bytes");
        fs::write(
            store.records_dir().join("legacy-alternate.rec"),
            encode_v1(&legacy),
        )?;

        let scan = store.scan_startup()?;
        let offers = scan
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) => Some(offer),
                RecoveryScanDisposition::Quarantine(_) => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(offers.len(), 2);
        assert!(offers.iter().all(|offer| offer.superseded().is_empty()));
        let mut schemas = offers
            .iter()
            .map(|offer| offer.metadata().schema_version())
            .collect::<Vec<_>>();
        schemas.sort_unstable();
        assert_eq!(schemas, [1, RECOVERY_SCHEMA_VERSION]);
        Ok(())
    }

    #[test]
    fn exact_handle_rejects_path_replacement_for_load_and_delete() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let original = sample_snapshot(14, b"original bytes");
        store.persist(&original)?;
        let scan = store.scan_startup()?;
        assert_eq!(load_offer(&store, &scan[0])?.content(), b"original bytes");

        let replacement = snapshot_with_document(14, 14, 99, 99, b"replacement bytes");
        store.persist(&replacement)?;
        let error = load_offer(&store, &scan[0]).expect_err("replacement must invalidate handle");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let entry = scan.into_entries().pop().expect("one offer");
        let (_, RecoveryScanDisposition::Offer(offer)) = entry.into_parts() else {
            panic!("expected offer");
        };
        let handle = offer.into_cleanup_handles().pop().expect("primary handle");
        assert!(store.delete_offered_record(handle).is_err());
        let encoded = fs::read(store.record_path(replacement.instance_id()))?;
        let RecoveryStartupDisposition::Offer(record) = validate_recovery_record(&encoded) else {
            panic!("replacement must remain valid");
        };
        assert_eq!(record.content(), b"replacement bytes");
        Ok(())
    }

    #[test]
    fn exact_quarantine_refuses_a_replaced_pathname() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("damaged.rec");
        let damaged = b"not a recovery record";
        fs::write(&path, damaged)?;
        let opened = open_recovery_candidate(&path).expect("bind damaged fixture");

        fs::remove_file(&path)?;
        let replacement = sample_snapshot(42, b"valid replacement").encode();
        fs::write(&path, &replacement)?;

        assert!(quarantine_bound_file(&store, &path, &opened, damaged).is_err());
        assert_eq!(fs::read(&path)?, replacement);
        assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());
        Ok(())
    }

    #[test]
    fn quarantine_parent_sync_failure_retains_the_bound_source() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("damaged.rec");
        let damaged = b"damaged recovery bytes";
        fs::write(&path, damaged)?;
        let opened = open_recovery_candidate(&path).expect("bind damaged fixture");
        let quarantine_parent = store.quarantine_dir();
        let sync_calls = std::cell::RefCell::new(Vec::new());

        let error = quarantine_bound_file_with(
            &store,
            &path,
            &opened,
            damaged,
            |candidate| -> io::Result<noter_platform::ParentSyncOutcome> {
                sync_calls.borrow_mut().push(
                    candidate
                        .parent()
                        .expect("quarantine candidate has a parent")
                        .to_path_buf(),
                );
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "injected quarantine parent sync failure",
                ))
            },
        )
        .expect_err("source deletion must wait for quarantine parent durability");

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(sync_calls.into_inner(), [quarantine_parent]);
        assert_eq!(fs::read(&path)?, damaged);
        let quarantined: Vec<_> =
            fs::read_dir(store.quarantine_dir())?.collect::<Result<_, _>>()?;
        assert_eq!(quarantined.len(), 1);
        assert_eq!(fs::read(quarantined[0].path())?, damaged);
        Ok(())
    }

    #[test]
    fn source_parent_sync_failure_is_a_cleanup_warning_after_quarantine() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("damaged.rec");
        let damaged = b"damaged recovery bytes";
        fs::write(&path, damaged)?;
        let opened = open_recovery_candidate(&path).expect("bind damaged fixture");
        let quarantine_parent = store.quarantine_dir();
        let source_parent = store.records_dir();
        let sync_calls = std::cell::RefCell::new(Vec::new());

        let (quarantined, cleanup_error) = quarantine_bound_file_with(
            &store,
            &path,
            &opened,
            damaged,
            |candidate| -> io::Result<noter_platform::ParentSyncOutcome> {
                let parent = candidate
                    .parent()
                    .expect("recovery candidate has a parent")
                    .to_path_buf();
                sync_calls.borrow_mut().push(parent.clone());
                if parent == source_parent {
                    Err(io::Error::other("injected source parent sync failure"))
                } else {
                    noter_platform::sync_parent(candidate)
                }
            },
        )?;
        drop(opened);

        let cleanup_error = cleanup_error.expect("source parent failure must be surfaced");
        assert_eq!(cleanup_error.kind(), io::ErrorKind::Other);
        assert_eq!(sync_calls.into_inner(), [quarantine_parent, source_parent]);
        assert!(!path.exists());
        assert_eq!(fs::read(&quarantined)?, damaged);
        Ok(())
    }

    #[test]
    fn pathname_and_header_instance_mismatch_is_retained_without_offer() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let encoded = sample_snapshot(45, b"identity mismatch").encode();
        let mismatched_path = store.record_path(RecoveryInstanceId::new([46; 16]));
        fs::write(&mismatched_path, &encoded)?;

        let scan = store.scan_startup()?;

        assert_eq!(scan.len(), 1);
        assert!(matches!(
            scan[0].disposition(),
            RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::InstanceMismatch)
        ));
        assert!(scan[0].remains_in_records());
        assert!(scan[0].quarantine_error().is_some());
        assert_eq!(fs::read(&mismatched_path)?, encoded);
        assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());

        let error = store
            .quarantine_file(&mismatched_path)
            .expect_err("public quarantine must reject disagreeing identities");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&mismatched_path)?, encoded);
        assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());
        Ok(())
    }

    #[test]
    fn live_identity_defers_mismatch_notice_without_moving_the_artifact() -> io::Result<()> {
        for live_path_identity in [true, false] {
            let dir = tempdir()?;
            let store = RecoveryStore::open(dir.path())?;
            let path_instance = indexed_instance(47);
            let snapshot = sample_snapshot(48, b"deferred identity mismatch");
            let mismatched_path = store.record_path(path_instance);
            let encoded = snapshot.encode();
            fs::write(&mismatched_path, &encoded)?;
            let live_instance = if live_path_identity {
                path_instance
            } else {
                snapshot.instance_id()
            };
            let lease = store.try_hold_live_lease(live_instance)?;

            assert!(store.scan_startup()?.is_empty());
            assert_eq!(fs::read(&mismatched_path)?, encoded);
            assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());

            store.release_live_lease(lease)?;
            let scan = store.scan_startup()?;
            assert_eq!(scan.len(), 1);
            assert!(matches!(
                scan[0].disposition(),
                RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::InstanceMismatch)
            ));
            assert_eq!(fs::read(&mismatched_path)?, encoded);
            assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());
        }
        Ok(())
    }

    #[test]
    fn startup_never_quarantines_a_live_instances_damaged_record() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(43, b"live damaged record");
        store.persist(&snapshot)?;
        let path = store.record_path(snapshot.instance_id());
        let mut damaged = fs::read(&path)?;
        let last = damaged.len().saturating_sub(1);
        damaged[last] ^= 0x55;
        fs::write(&path, &damaged)?;
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;

        assert!(store.scan_startup()?.is_empty());
        assert_eq!(fs::read(&path)?, damaged);
        assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());
        drop(lease);
        Ok(())
    }

    #[test]
    fn exact_offered_delete_refuses_a_live_foreign_instance() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(15, b"live foreign work");
        store.persist(&snapshot)?;
        let scan = store.scan_startup()?;
        let entry = scan.into_entries().pop().expect("one offer");
        let (_, RecoveryScanDisposition::Offer(offer)) = entry.into_parts() else {
            panic!("expected offer");
        };
        let handle = offer.into_cleanup_handles().pop().expect("primary handle");
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;

        let error = store
            .delete_offered_record(handle)
            .expect_err("live foreign record must not be deleted");
        assert_eq!(error.kind(), io::ErrorKind::ResourceBusy);
        assert!(store.record_path(snapshot.instance_id()).exists());
        drop(lease);
        remove_file_if_present(&store.live_path(snapshot.instance_id()))?;
        Ok(())
    }

    #[test]
    fn owned_cleanup_uses_only_canonical_and_keyed_names() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(16, b"owned");
        store.persist(&snapshot)?;
        let stage = exclusive_stage_path(
            &store.records_dir(),
            snapshot.instance_id(),
            TemporaryArtifactKind::Stage,
        )?;
        let backup = exclusive_stage_path(
            &store.records_dir(),
            snapshot.instance_id(),
            TemporaryArtifactKind::Backup,
        )?;
        fs::write(&stage, b"not parsed")?;
        fs::write(&backup, b"also not parsed")?;
        let legacy_random = store.records_dir().join(".noter-recovery-legacy.tmp");
        fs::write(&legacy_random, b"unrelated invalid content")?;
        let other = exclusive_stage_path(
            &store.records_dir(),
            RecoveryInstanceId::new([17; 16]),
            TemporaryArtifactKind::Stage,
        )?;
        fs::write(&other, b"foreign invalid content")?;

        store.delete_owned_artifacts(snapshot.instance_id())?;

        assert!(!store.record_path(snapshot.instance_id()).exists());
        assert!(!stage.exists());
        assert!(!backup.exists());
        assert!(legacy_random.exists());
        assert!(other.exists());
        Ok(())
    }

    #[test]
    fn owned_cleanup_bounds_unrelated_directory_work() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(18, b"owned bounded cleanup");
        store.persist(&snapshot)?;
        for index in 0..=MAX_OWNED_RECOVERY_CLEANUP_FILES {
            fs::write(
                store
                    .records_dir()
                    .join(format!("unrelated-{index:04}.tmp")),
                b"not recovery content",
            )?;
        }

        let error = store
            .delete_owned_artifacts(snapshot.instance_id())
            .expect_err("surplus entries must surface incomplete cleanup");

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(!store.record_path(snapshot.instance_id()).exists());
        assert!(store.records_dir().join("unrelated-0256.tmp").exists());
        Ok(())
    }

    #[test]
    fn a_held_live_lease_hides_the_instance_from_startup_scan() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(9, b"still running");
        store.persist(&snapshot)?;
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;

        assert!(store.instance_is_live(snapshot.instance_id())?);
        assert!(store.scan_startup()?.is_empty());

        drop(lease);
        assert!(!store.instance_is_live(snapshot.instance_id())?);
        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert_eq!(load_offer(&store, &entries[0])?.content(), b"still running");
        Ok(())
    }

    #[test]
    fn hard_linked_live_markers_fail_closed() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let instance_id = indexed_instance(26);
        let live_path = store.live_path(instance_id);
        fs::write(&live_path, b"")?;
        let alias = store.root().join("live-marker-alias");
        fs::hard_link(&live_path, &alias)?;

        let acquire = store
            .try_hold_live_lease(instance_id)
            .expect_err("a multiply linked live marker is not private");
        assert_eq!(acquire.kind(), io::ErrorKind::InvalidData);
        let probe = store
            .instance_is_live(instance_id)
            .expect_err("a multiply linked live marker is indeterminate");
        assert_eq!(probe.kind(), io::ErrorKind::InvalidData);
        assert!(live_path.exists());
        assert!(alias.exists());
        Ok(())
    }

    #[test]
    fn releasing_a_live_lease_removes_both_fact_bound_paths() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let instance_id = indexed_instance(20);
        let lease = store.try_hold_live_lease(instance_id)?;

        assert!(store.live_path(instance_id).exists());
        assert!(store.live_guard_path(instance_id).exists());
        store.release_live_lease(lease)?;

        assert!(!store.live_path(instance_id).exists());
        assert!(!store.live_guard_path(instance_id).exists());
        Ok(())
    }

    #[test]
    fn stale_marker_cleanup_claims_and_removes_both_paths() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let instance_id = indexed_instance(21);
        fs::write(store.live_path(instance_id), b"")?;
        fs::write(store.live_guard_path(instance_id), b"")?;

        store.cleanup_stale_live_marker(instance_id);

        assert!(!store.live_path(instance_id).exists());
        assert!(!store.live_guard_path(instance_id).exists());
        Ok(())
    }

    #[test]
    fn releasing_an_offered_claim_removes_its_exclusive_lease() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(23, b"claim release");
        store.persist(&snapshot)?;
        let scan = store.scan_startup()?;
        let handle = offer(&scan[0]).primary();
        let claim = store.claim_offered_record(handle)?;
        assert!(store.live_path(snapshot.instance_id()).exists());
        assert!(store.live_guard_path(snapshot.instance_id()).exists());

        store.release_claim(claim)?;

        assert!(!store.live_path(snapshot.instance_id()).exists());
        assert!(!store.live_guard_path(snapshot.instance_id()).exists());
        Ok(())
    }

    #[test]
    fn claimed_load_rejects_a_different_instance_handle() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let first = sample_snapshot(24, b"first claim");
        let second = sample_snapshot(25, b"second claim");
        store.persist(&first)?;
        store.persist(&second)?;
        let scan = store.scan_startup()?;
        let first_handle = scan
            .iter()
            .find_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(candidate)
                    if candidate.metadata().instance_id() == first.instance_id() =>
                {
                    Some(candidate.primary())
                }
                _ => None,
            })
            .expect("first handle");
        let second_handle = scan
            .iter()
            .find_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(candidate)
                    if candidate.metadata().instance_id() == second.instance_id() =>
                {
                    Some(candidate.primary())
                }
                _ => None,
            })
            .expect("second handle");
        let claim = store.claim_offered_record(second_handle)?;

        let error = store
            .load_claimed_record(first_handle, &claim)
            .expect_err("a claim must be bound to one recovery instance");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        store.release_claim(claim)?;
        Ok(())
    }

    #[test]
    fn one_rebound_live_path_cannot_hide_the_independent_guard() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(19, b"active guarded recovery");
        store.persist(&snapshot)?;
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;
        let displaced = store.root().join("displaced-live-object");
        fs::rename(store.live_path(snapshot.instance_id()), &displaced)?;

        assert!(store.instance_is_live(snapshot.instance_id())?);
        assert_eq!(
            store
                .try_hold_live_lease(snapshot.instance_id())
                .expect_err("the locked guard must prevent a competing lease")
                .kind(),
            io::ErrorKind::ResourceBusy
        );
        remove_file_if_present(&store.live_path(snapshot.instance_id()))?;
        assert!(
            store
                .scan_startup()?
                .iter()
                .all(|entry| !matches!(entry.disposition(), RecoveryScanDisposition::Offer(_)))
        );

        store.release_live_lease(lease)?;
        assert!(!store.live_path(snapshot.instance_id()).exists());
        assert!(!store.live_guard_path(snapshot.instance_id()).exists());
        remove_file_if_present(&displaced)?;
        Ok(())
    }

    #[test]
    fn unix_live_cleanup_accepts_an_already_unlinked_path() -> io::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("claimed.live");
        let locked = acquire_live_file(&path)?;
        fs::remove_file(&path)?;

        delete_claimed_live_path_unix_fallback(&path, &locked.file, locked.facts)?;

        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn corrupt_records_are_quarantined_on_scan() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("broken.rec");
        fs::write(&path, b"not a recovery record")?;

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].disposition(),
            RecoveryScanDisposition::Quarantine(
                RecoveryQuarantineReason::Truncated | RecoveryQuarantineReason::InvalidMagic
            )
        ));
        assert!(!path.exists());
        assert!(!entries[0].remains_in_records());
        assert!(entries[0].quarantine_error().is_none());
        let quarantined = fs::read_dir(store.quarantine_dir())?
            .next()
            .transpose()?
            .expect("quarantined exact bytes");
        assert_eq!(fs::read(quarantined.path())?, b"not a recovery record");
        Ok(())
    }

    #[test]
    fn missing_quarantine_source_is_not_success() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let missing = store.records_dir().join("gone.rec");
        let error = store
            .quarantine_file(&missing)
            .expect_err("missing source must fail");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        Ok(())
    }

    #[test]
    fn missing_candidate_is_omitted_from_scan_but_public_quarantine_fails() -> io::Result<()> {
        let directory = tempdir()?;
        let store = RecoveryStore::open(directory.path())?;
        let missing = store.records_dir().join("disappeared.rec");

        assert!(matches!(
            open_recovery_candidate(&missing),
            Err(OpenRecoveryCandidateFailure::Missing)
        ));
        assert!(store.scan_startup()?.is_empty());
        assert_eq!(
            store
                .quarantine_file(&missing)
                .expect_err("explicit quarantine must not accept a missing source")
                .kind(),
            io::ErrorKind::NotFound
        );
        Ok(())
    }

    #[test]
    fn inaccessible_candidate_preserves_the_original_io_error() {
        let code = if cfg!(windows) { 5 } else { 13 };
        let failure = classify_recovery_candidate_io(io::Error::from_raw_os_error(code));

        let OpenRecoveryCandidateFailure::Inaccessible(error) = failure else {
            panic!("a non-missing access failure must remain inaccessible");
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.raw_os_error(), Some(code));
    }

    #[test]
    fn startup_metadata_ignores_only_missing_and_preserves_other_io_errors() {
        let missing = classify_startup_path_metadata(Err(io::Error::new(
            io::ErrorKind::NotFound,
            "injected disappearance",
        )))
        .expect("a metadata disappearance is an ordinary scan race");
        assert!(missing.is_none());

        let code = if cfg!(windows) { 5 } else { 13 };
        let error = classify_startup_path_metadata(Err(io::Error::from_raw_os_error(code)))
            .expect_err("an inaccessible enumerated candidate must fail the scan closed");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(error.raw_os_error(), Some(code));
    }

    #[test]
    fn public_quarantine_reports_a_retained_source_as_failure() {
        let destination = PathBuf::from("verified-quarantine-copy.rec");
        let cleanup_error = io::Error::new(
            io::ErrorKind::PermissionDenied,
            "bound source could not be removed",
        );
        let error = finish_public_quarantine(Ok((destination, Some(cleanup_error))), Ok(()))
            .expect_err("a retained source is not a completed move");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn checksum_tamper_is_quarantined() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(2, b"safe");
        store.persist(&snapshot)?;
        let path = store.record_path(snapshot.instance_id());
        let mut bytes = fs::read(&path)?;
        let last = bytes.len() - 1;
        bytes[last] ^= 0x5A;
        fs::write(&path, bytes)?;

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].disposition(),
            RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::ChecksumMismatch)
        ));
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn unknown_schema_is_quarantined() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(3, b"schema");
        store.persist(&snapshot)?;
        let path = store.record_path(snapshot.instance_id());
        let mut bytes = fs::read(&path)?;
        bytes[8..12].copy_from_slice(&(RECOVERY_SCHEMA_VERSION + 1).to_le_bytes());
        fs::write(&path, bytes)?;

        let entries = store.scan_startup()?;
        assert!(matches!(
            entries[0].disposition(),
            RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::UnknownSchema)
        ));
        Ok(())
    }

    #[test]
    fn resource_ceilings_are_exact() {
        assert_eq!(MAX_STARTUP_RECOVERY_OFFERS, 32);
        assert_eq!(MAX_STARTUP_RECOVERY_FILES, 256);
        assert_eq!(MAX_STARTUP_RECOVERY_DIRECTORY_ENTRIES, 1024);
        assert_eq!(MAX_STARTUP_QUARANTINE_RESULTS, 32);
        assert_eq!(MAX_SUPERSEDED_RECOVERY_HANDLES, 16);
        assert_eq!(MAX_STARTUP_RECOVERY_BYTES, 128 * 1024 * 1024);
        assert_eq!(MAX_RECOVERY_FILE_BYTES, 64 * 1024 * 1024 + 256 * 1024);
        assert_eq!(MAX_RECOVERY_FILE_BYTES, 67_371_008);
        assert_eq!(
            advance_scan_byte_budget(MAX_STARTUP_RECOVERY_BYTES - 1, 1),
            Some(MAX_STARTUP_RECOVERY_BYTES)
        );
        assert_eq!(
            advance_scan_byte_budget(MAX_STARTUP_RECOVERY_BYTES, 1),
            None
        );
    }

    #[test]
    fn live_records_do_not_consume_the_eligible_candidate_budget() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let mut leases = Vec::with_capacity(MAX_STARTUP_RECOVERY_FILES);
        for index in 1..=MAX_STARTUP_RECOVERY_FILES {
            let instance_id = indexed_instance(index);
            let snapshot = RecoverySnapshot::try_new(RecoverySnapshotParts {
                document_id: RecoveryDocumentId::new(instance_id.as_bytes()),
                instance_id,
                revision: Revision::new(1),
                created_at: RecoveryWallTime::from_unix_millis(1),
                updated_at: RecoveryWallTime::from_unix_millis(2),
                original_path: Vec::new(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::caret(0),
                content: b"live noise".to_vec(),
            })
            .expect("live snapshot");
            store.persist(&snapshot)?;
            leases.push(store.try_hold_live_lease(instance_id)?);
        }
        let dead = sample_snapshot(0, b"dead canonical recovery");
        store.persist(&dead)?;

        let scan = store.scan_startup()?;
        assert_eq!(scan.len(), 1);
        assert_eq!(offer(&scan[0]).metadata().instance_id(), dead.instance_id());
        drop(leases);
        Ok(())
    }

    #[test]
    fn repeated_bounded_scans_progress_past_stale_live_noise() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        for index in 1..=MAX_STARTUP_RECOVERY_DIRECTORY_ENTRIES + 1 {
            fs::write(store.live_path(indexed_instance(index)), b"")?;
        }
        let dead = sample_snapshot(0, b"recovery behind stale live noise");
        store.persist(&dead)?;

        let first = store.scan_startup()?;
        let found_first = first.iter().any(|entry| {
            matches!(entry.disposition(), RecoveryScanDisposition::Offer(candidate) if candidate.metadata().instance_id() == dead.instance_id())
        });
        if !found_first {
            assert!(first.directory_limit_reached());
            let second = store.scan_startup()?;
            assert!(second.iter().any(|entry| {
                matches!(entry.disposition(), RecoveryScanDisposition::Offer(candidate) if candidate.metadata().instance_id() == dead.instance_id())
            }));
        }
        Ok(())
    }

    #[test]
    fn startup_file_and_quarantine_result_bounds_are_surfaced() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        for index in 0..=MAX_STARTUP_RECOVERY_FILES {
            fs::write(
                store.records_dir().join(format!("broken-{index:04}.rec")),
                b"broken",
            )?;
        }

        let scan = store.scan_startup()?;

        assert!(scan.limit_reached());
        assert!(scan.has_omissions());
        assert_eq!(scan.len(), MAX_STARTUP_QUARANTINE_RESULTS);
        assert_eq!(
            scan.quarantine_results_omitted(),
            MAX_STARTUP_RECOVERY_FILES - MAX_STARTUP_QUARANTINE_RESULTS
        );
        assert_eq!(fs::read_dir(store.records_dir())?.count(), 1);
        Ok(())
    }

    #[test]
    fn superseded_handle_bound_is_surfaced_and_primary_is_cleanup_last() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let instance = 18;
        for revision in 0..=MAX_SUPERSEDED_RECOVERY_HANDLES + 1 {
            let snapshot = snapshot_at(
                instance,
                u64::try_from(revision).expect("fixture revision"),
                1,
                format!("revision-{revision}").as_bytes(),
            );
            let path = if revision == MAX_SUPERSEDED_RECOVERY_HANDLES + 1 {
                store.record_path(snapshot.instance_id())
            } else {
                store
                    .records_dir()
                    .join(format!("duplicate-{revision:02}.tmp"))
            };
            fs::write(path, snapshot.encode())?;
        }

        let scan = store.scan_startup()?;
        assert!(scan.superseded_handles_omitted());
        assert!(scan.has_omissions());
        let entry = scan.into_entries().pop().expect("one offer");
        let (_, RecoveryScanDisposition::Offer(offer)) = entry.into_parts() else {
            panic!("expected offer");
        };
        assert_eq!(offer.superseded().len(), MAX_SUPERSEDED_RECOVERY_HANDLES);
        let handles = offer.into_cleanup_handles();
        assert_eq!(
            handles.last().expect("primary last").metadata().revision(),
            Revision::new(
                u64::try_from(MAX_SUPERSEDED_RECOVERY_HANDLES + 1).expect("fixture revision")
            )
        );
        Ok(())
    }

    #[test]
    fn delete_record_rejects_non_missing_failures() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let id = RecoveryInstanceId::new([42; 16]);
        let path = store.record_path(id);
        fs::create_dir(&path)?;
        let error = store
            .delete_record(id)
            .expect_err("directory at record path is not a successful missing delete");
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert!(path.exists());
        Ok(())
    }

    #[test]
    fn live_probe_errors_fail_closed() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(43, b"protected recovery");
        store.persist(&snapshot)?;
        fs::create_dir(store.live_path(snapshot.instance_id()))?;

        assert!(store.instance_is_live(snapshot.instance_id()).is_err());
        assert!(store.scan_startup().is_err());
        assert!(store.record_path(snapshot.instance_id()).exists());
        Ok(())
    }

    #[test]
    fn record_cleanup_preserves_the_live_lease() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(44, b"clean now");
        store.persist(&snapshot)?;
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;

        store.delete_record(snapshot.instance_id())?;

        assert!(!store.record_path(snapshot.instance_id()).exists());
        assert!(store.live_path(snapshot.instance_id()).exists());
        assert!(store.instance_is_live(snapshot.instance_id())?);
        drop(lease);
        Ok(())
    }

    #[test]
    fn oversized_recovery_file_is_quarantined() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("huge.rec");
        // Metadata length check happens before a full read; use set_len so the
        // ceiling is exercised without writing 64 MiB of payload bytes.
        let file = File::create(&path)?;
        file.set_len(MAX_RECOVERY_FILE_BYTES + 1)?;
        drop(file);

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].disposition(),
            RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::ContentTooLarge)
        ));
        Ok(())
    }

    #[test]
    fn exact_recovery_file_ceiling_is_not_size_rejected() -> io::Result<()> {
        // metadata.len() > MAX must stay strict greater-than: exact size proceeds
        // to content validation (truncated/magic) instead of ContentTooLarge.
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("exact-ceiling.rec");
        let file = File::create(&path)?;
        file.set_len(MAX_RECOVERY_FILE_BYTES)?;
        drop(file);

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(
            !matches!(
                entries[0].disposition(),
                RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::ContentTooLarge)
            ),
            "exact file ceiling must not be size-rejected, got {:?}",
            entries[0].disposition()
        );
        Ok(())
    }

    #[cfg(not(windows))]
    #[test]
    fn missing_records_directory_scans_as_empty() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        fs::remove_dir_all(store.records_dir())?;
        let entries = store.scan_startup()?;
        assert!(
            entries.is_empty(),
            "a missing records directory is an empty startup scan, not an error"
        );
        store.delete_owned_artifacts(indexed_instance(27))?;
        Ok(())
    }

    #[cfg(not(windows))]
    #[test]
    fn owned_cleanup_rejects_a_non_directory_records_path() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let records = store.records_dir();
        fs::remove_dir_all(&records)?;
        fs::write(&records, b"not a directory")?;

        let error = store
            .delete_owned_artifacts(indexed_instance(28))
            .expect_err("a non-directory records path is not missing");
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert!(records.is_file());
        Ok(())
    }

    #[cfg(not(windows))]
    #[test]
    fn quarantine_error_is_reported_when_move_fails() -> io::Result<()> {
        // A valid quarantine failure leaves remains_in_records true and a message.
        // Use a non-empty records file, then remove write access to quarantine by
        // replacing the quarantine directory with a file so rename fails.
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("broken.rec");
        fs::write(&path, b"not a recovery record")?;
        let quarantine = store.quarantine_dir();
        fs::remove_dir_all(&quarantine)?;
        fs::write(&quarantine, b"block")?;

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(entries[0].remains_in_records());
        assert!(entries[0].quarantine_error().is_some());
        assert!(path.exists());
        Ok(())
    }

    #[test]
    fn surplus_valid_records_remain_for_later_session() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        for index in 0..=MAX_STARTUP_RECOVERY_OFFERS {
            let instance = u8::try_from(index).unwrap_or(u8::MAX);
            // Distinct instance ids: use patterned bytes.
            let mut id = [0_u8; 16];
            id[0] = instance;
            id[1] = (index / 256) as u8;
            let snapshot = RecoverySnapshot::try_new(RecoverySnapshotParts {
                document_id: RecoveryDocumentId::new(id),
                instance_id: RecoveryInstanceId::new(id),
                revision: Revision::new(1),
                created_at: RecoveryWallTime::from_unix_millis(1),
                updated_at: RecoveryWallTime::from_unix_millis(2),
                original_path: Vec::new(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::caret(0),
                content: format!("doc-{index}").into_bytes(),
            })
            .expect("snapshot");
            store.persist(&snapshot)?;
        }

        let entries = store.scan_startup()?;
        let offers = entries
            .iter()
            .filter(|entry| matches!(entry.disposition(), RecoveryScanDisposition::Offer(_)))
            .count();
        assert_eq!(offers, MAX_STARTUP_RECOVERY_OFFERS);
        assert!(entries.offers_omitted());

        let remaining = fs::read_dir(store.records_dir())?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .count();
        assert_eq!(remaining, MAX_STARTUP_RECOVERY_OFFERS + 1);
        Ok(())
    }
}
