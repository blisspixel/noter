//! Revision-tagged, fault-injectable save protocol.

use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use super::revision::Revision;

/// Strength of the platform-provided file identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IdentityQuality {
    /// The preferred native identifier for the current platform.
    Preferred,
    /// A reduced identifier used when the preferred query is unavailable.
    Reduced,
}

/// A stable platform file identity represented as volume and file components.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileIdentity {
    quality: IdentityQuality,
    volume: u128,
    file: u128,
}

impl FileIdentity {
    /// Creates a platform-neutral identity from adapter-provided components.
    pub const fn new(volume: u128, file: u128) -> Self {
        Self {
            quality: IdentityQuality::Preferred,
            volume,
            file,
        }
    }

    /// Creates a reduced-strength identity from fallback platform components.
    pub const fn reduced(volume: u128, file: u128) -> Self {
        Self {
            quality: IdentityQuality::Reduced,
            volume,
            file,
        }
    }

    /// Returns the platform identity strength.
    pub const fn quality(self) -> IdentityQuality {
        self.quality
    }

    /// Returns the volume or device component.
    pub const fn volume(self) -> u128 {
        self.volume
    }

    /// Returns the file or inode component.
    pub const fn file(self) -> u128 {
        self.file
    }
}

/// A BLAKE3-256 digest of serialized file content.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ContentFingerprint([u8; 32]);

impl ContentFingerprint {
    /// Creates a fingerprint from an already validated BLAKE3-256 digest.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Computes the fingerprint of a complete byte slice.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(*blake3::hash(bytes).as_bytes())
    }

    /// Computes the fingerprint of all bytes read from a stream.
    ///
    /// # Errors
    ///
    /// Returns the reader's error if the complete stream cannot be consumed.
    pub fn from_reader(reader: impl Read) -> io::Result<Self> {
        let mut hasher = blake3::Hasher::new();
        hasher.update_reader(reader)?;
        Ok(Self(*hasher.finalize().as_bytes()))
    }

    /// Returns the digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Opaque platform timestamp components for the last content or metadata change.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileChangeToken {
    primary: i64,
    secondary: i64,
}

impl FileChangeToken {
    /// Creates a token from platform-provided timestamp components.
    pub const fn new(primary: i64, secondary: i64) -> Self {
        Self { primary, secondary }
    }

    /// Returns the primary platform timestamp component.
    pub const fn primary(self) -> i64 {
        self.primary
    }

    /// Returns the subsecond or reserved platform timestamp component.
    pub const fn secondary(self) -> i64 {
        self.secondary
    }
}

/// Identity and content facts captured for an existing regular file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileObservation {
    identity: FileIdentity,
    fingerprint: ContentFingerprint,
    length: u64,
    link_count: u64,
    change_token: FileChangeToken,
}

impl FileObservation {
    /// Creates a complete regular-file observation.
    pub const fn new(
        identity: FileIdentity,
        fingerprint: ContentFingerprint,
        length: u64,
        link_count: u64,
        change_token: FileChangeToken,
    ) -> Self {
        Self {
            identity,
            fingerprint,
            length,
            link_count,
            change_token,
        }
    }

    /// Returns the platform file identity.
    pub const fn identity(self) -> FileIdentity {
        self.identity
    }

    /// Returns the serialized-content fingerprint.
    pub const fn fingerprint(self) -> ContentFingerprint {
        self.fingerprint
    }

    /// Returns the observed byte length.
    pub const fn length(self) -> u64 {
        self.length
    }

    /// Returns the observed hard-link count.
    pub const fn link_count(self) -> u64 {
        self.link_count
    }

    /// Returns the platform content-or-metadata change timestamp.
    pub const fn change_token(self) -> FileChangeToken {
        self.change_token
    }
}

/// A non-regular final path entry that cannot be overwritten implicitly.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpecialFileKind {
    /// A symbolic link or Windows reparse point.
    SymbolicLink,
    /// A directory.
    Directory,
    /// A device, socket, pipe, or other unsupported object.
    Other,
}

/// The observed state of a save destination.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetState {
    /// No final path entry exists.
    Missing,
    /// A supported regular file exists.
    Regular(FileObservation),
    /// A final path entry exists but is not safe for implicit replacement.
    Special(SpecialFileKind),
}

/// The destination state against which a snapshot may commit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TargetExpectation {
    /// The destination must remain absent through commit.
    Missing,
    /// The destination must remain this exact file version through commit.
    Existing(FileObservation),
}

impl TargetExpectation {
    fn matches(self, state: TargetState) -> bool {
        matches!(
            (self, state),
            (Self::Missing, TargetState::Missing) | (Self::Existing(_), TargetState::Regular(_))
        ) && match (self, state) {
            (Self::Existing(expected), TargetState::Regular(actual)) => expected == actual,
            _ => true,
        }
    }
}

/// Immutable bytes and destination facts captured for one document revision.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SaveSnapshot {
    revision: Revision,
    target: PathBuf,
    expected: TargetExpectation,
    bytes: Arc<[u8]>,
}

impl SaveSnapshot {
    /// Creates an immutable save snapshot.
    pub fn new(
        revision: Revision,
        target: PathBuf,
        expected: TargetExpectation,
        bytes: impl Into<Arc<[u8]>>,
    ) -> Self {
        Self {
            revision,
            target,
            expected,
            bytes: bytes.into(),
        }
    }

    /// Returns the document revision represented by this snapshot.
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the final destination path.
    pub fn target(&self) -> &Path {
        &self.target
    }

    /// Returns the destination state required for commit.
    pub const fn expected(&self) -> TargetExpectation {
        self.expected
    }

    /// Returns the exact serialized bytes to commit.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// A fault boundary in the save protocol.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SaveStage {
    /// Initial destination inspection.
    InspectInitial,
    /// Unique sibling creation.
    CreateTemporary,
    /// Temporary-file content write.
    Write,
    /// User-space buffer flush.
    Flush,
    /// Destination metadata application.
    ApplyMetadata,
    /// Temporary-file data and metadata synchronization.
    SyncFile,
    /// Immediate pre-commit destination revalidation.
    Revalidate,
    /// Atomic replacement or exclusive creation.
    Replace,
    /// Post-commit destination-state reconciliation.
    Reconcile,
    /// Parent-directory or platform-equivalent synchronization.
    SyncParent,
    /// Temporary or backup artifact cleanup.
    Cleanup,
}

/// A storage failure tagged with the exact protocol boundary.
#[derive(Clone, PartialEq, Eq, Error, Debug)]
#[error("{stage:?} failed: {message}")]
pub struct StorageError {
    stage: SaveStage,
    message: String,
}

impl StorageError {
    /// Creates a tagged storage failure.
    pub fn new(stage: SaveStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }

    /// Returns the failed protocol boundary.
    pub const fn stage(&self) -> SaveStage {
        self.stage
    }

    /// Returns the non-sensitive diagnostic message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Verified destination facts and commit cleanup status after replacement.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ReplaceReceipt {
    observation: FileObservation,
    cleanup_warnings: Vec<StorageError>,
}

impl ReplaceReceipt {
    /// Creates a verified replacement receipt without a cleanup warning.
    pub const fn new(observation: FileObservation) -> Self {
        Self {
            observation,
            cleanup_warnings: Vec::new(),
        }
    }

    /// Creates a verified replacement receipt with a post-commit cleanup warning.
    pub fn with_cleanup_warning(
        observation: FileObservation,
        cleanup_warning: StorageError,
    ) -> Self {
        Self {
            observation,
            cleanup_warnings: vec![cleanup_warning],
        }
    }

    /// Creates a verified replacement receipt with all artifact cleanup warnings.
    pub const fn with_cleanup_warnings(
        observation: FileObservation,
        cleanup_warnings: Vec<StorageError>,
    ) -> Self {
        Self {
            observation,
            cleanup_warnings,
        }
    }

    /// Returns the committed destination observation.
    pub const fn observation(&self) -> FileObservation {
        self.observation
    }

    /// Returns a post-commit backup or temporary cleanup warning, if any.
    pub fn cleanup_warnings(&self) -> &[StorageError] {
        &self.cleanup_warnings
    }

    fn into_parts(self) -> (FileObservation, Vec<StorageError>) {
        (self.observation, self.cleanup_warnings)
    }
}

/// Result of the commit-point platform operation.
#[derive(Debug)]
pub enum ReplaceOutcome<T> {
    /// The new bytes are the destination and were verified.
    Committed(ReplaceReceipt),
    /// The destination changed before the commit point.
    Conflict {
        /// Still-private temporary file.
        temporary: T,
        /// Destination state observed at the conflicting commit point.
        actual: TargetState,
    },
    /// The adapter proved that the destination did not commit.
    NotCommitted {
        /// Still-private temporary file.
        temporary: T,
        /// Commit operation failure.
        error: StorageError,
    },
    /// The platform operation may have changed one or more path entries.
    CommitStateUnknown {
        /// Failure requiring path reconciliation or user intervention.
        error: StorageError,
    },
}

/// Strength of the completed persistence barrier.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Durability {
    /// File data, file metadata, and the destination directory entry were synced.
    FileAndDirectorySynced,
    /// File data and metadata were synced, but name durability is unproven.
    FileSynced,
    /// The filesystem or transport exposes only weaker persistence semantics.
    BestEffort,
}

/// Result of the post-commit directory or platform persistence barrier.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DurabilityOutcome {
    /// The adapter reached the stated durability level without an error.
    Achieved(Durability),
    /// Commit succeeded, but a later barrier failed or was only partly supported.
    Warning {
        /// Durability level still known to have been reached.
        achieved: Durability,
        /// Post-commit persistence failure.
        error: StorageError,
    },
}

/// Independent warnings that can occur after the destination has committed.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct SaveWarnings {
    cleanup: Vec<StorageError>,
    durability: Option<StorageError>,
}

impl SaveWarnings {
    const fn new(cleanup: Vec<StorageError>, durability: Option<StorageError>) -> Self {
        Self {
            cleanup,
            durability,
        }
    }

    /// Returns a backup or temporary cleanup warning, if any.
    pub fn cleanup(&self) -> &[StorageError] {
        &self.cleanup
    }

    /// Returns a post-commit persistence-barrier warning, if any.
    pub const fn durability(&self) -> Option<&StorageError> {
        self.durability.as_ref()
    }

    /// Returns whether neither post-commit warning occurred.
    pub const fn is_empty(&self) -> bool {
        self.cleanup.is_empty() && self.durability.is_none()
    }
}

/// Exact result of attempting to save one immutable revision.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SaveOutcome {
    /// The snapshot committed to the destination.
    Committed {
        /// Revision contained in the committed snapshot.
        revision: Revision,
        /// Strongest persistence level reached.
        durability: Durability,
        /// Verified destination facts after commit.
        observation: FileObservation,
        /// Independent cleanup and persistence warnings after commit.
        warnings: SaveWarnings,
    },
    /// A different destination version was observed before commit.
    Conflict {
        /// Revision that was not committed.
        revision: Revision,
        /// Destination state required by the snapshot.
        expected: TargetExpectation,
        /// Destination state that prevented commit.
        actual: TargetState,
        /// Cleanup failure for the private temporary file, if any.
        cleanup_error: Option<StorageError>,
    },
    /// The adapter proved that the snapshot did not commit.
    NotCommitted {
        /// Revision that was not committed.
        revision: Revision,
        /// Failure before or at the commit point.
        error: StorageError,
        /// Cleanup failure for the private temporary file, if any.
        cleanup_error: Option<StorageError>,
    },
    /// The platform may have changed the destination despite reporting failure.
    CommitStateUnknown {
        /// Revision whose commit state is unknown.
        revision: Revision,
        /// Failure requiring reconciliation and recovery retention.
        error: StorageError,
    },
}

/// Effectful storage operations used by the pure save protocol.
pub trait Storage {
    /// Adapter-owned private temporary file.
    type Temporary;

    /// Inspects the final path without following an unsafe final link.
    ///
    /// # Errors
    ///
    /// Returns a stage-tagged error when destination facts cannot be read safely.
    fn inspect(&mut self, path: &Path, stage: SaveStage) -> Result<TargetState, StorageError>;

    /// Creates an unpredictable same-directory sibling with create-new semantics.
    ///
    /// # Errors
    ///
    /// Returns an error when the sibling cannot be created exclusively.
    fn create_unique_sibling(
        &mut self,
        destination: &Path,
    ) -> Result<Self::Temporary, StorageError>;

    /// Writes all serialized bytes to the private temporary file.
    ///
    /// # Errors
    ///
    /// Returns an error when any byte cannot be written completely.
    fn write_all(
        &mut self,
        temporary: &mut Self::Temporary,
        bytes: &[u8],
    ) -> Result<(), StorageError>;

    /// Flushes user-space buffers associated with the temporary file.
    ///
    /// # Errors
    ///
    /// Returns an error when buffered bytes cannot be delivered to the file.
    fn flush(&mut self, temporary: &mut Self::Temporary) -> Result<(), StorageError>;

    /// Applies the ratified destination metadata policy before commit.
    ///
    /// # Errors
    ///
    /// Returns an error rather than silently dropping required metadata.
    fn apply_metadata(
        &mut self,
        temporary: &mut Self::Temporary,
        destination: &Path,
        source: Option<&FileObservation>,
    ) -> Result<(), StorageError>;

    /// Synchronizes temporary-file data and metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the platform cannot complete the file barrier.
    fn sync_file(&mut self, temporary: &mut Self::Temporary) -> Result<(), StorageError>;

    /// Commits by atomic replacement or exclusive creation.
    fn replace(
        &mut self,
        temporary: Self::Temporary,
        destination: &Path,
        expected: TargetState,
    ) -> ReplaceOutcome<Self::Temporary>;

    /// Performs the post-commit directory or platform persistence barrier.
    fn sync_parent(&mut self, destination: &Path) -> DurabilityOutcome;

    /// Explicitly removes an uncommitted temporary file.
    ///
    /// # Errors
    ///
    /// Returns an error when the private artifact could not be removed.
    fn discard(&mut self, temporary: Self::Temporary) -> Result<(), StorageError>;
}

/// Executes the save protocol for one immutable snapshot.
pub fn save_snapshot<S: Storage>(storage: &mut S, snapshot: &SaveSnapshot) -> SaveOutcome {
    let initial = match storage.inspect(snapshot.target(), SaveStage::InspectInitial) {
        Ok(state) => state,
        Err(error) => {
            return SaveOutcome::NotCommitted {
                revision: snapshot.revision(),
                error,
                cleanup_error: None,
            };
        }
    };

    if !snapshot.expected().matches(initial) {
        return SaveOutcome::Conflict {
            revision: snapshot.revision(),
            expected: snapshot.expected(),
            actual: initial,
            cleanup_error: None,
        };
    }

    let mut temporary = match storage.create_unique_sibling(snapshot.target()) {
        Ok(temporary) => temporary,
        Err(error) => {
            return SaveOutcome::NotCommitted {
                revision: snapshot.revision(),
                error,
                cleanup_error: None,
            };
        }
    };

    if let Err(error) = storage.write_all(&mut temporary, snapshot.bytes()) {
        return not_committed_with_cleanup(storage, snapshot.revision(), temporary, error);
    }
    if let Err(error) = storage.flush(&mut temporary) {
        return not_committed_with_cleanup(storage, snapshot.revision(), temporary, error);
    }

    let source = match initial {
        TargetState::Regular(observation) => Some(observation),
        TargetState::Missing | TargetState::Special(_) => None,
    };
    if let Err(error) = storage.apply_metadata(&mut temporary, snapshot.target(), source.as_ref()) {
        return not_committed_with_cleanup(storage, snapshot.revision(), temporary, error);
    }
    if let Err(error) = storage.sync_file(&mut temporary) {
        return not_committed_with_cleanup(storage, snapshot.revision(), temporary, error);
    }

    let revalidated = match storage.inspect(snapshot.target(), SaveStage::Revalidate) {
        Ok(state) => state,
        Err(error) => {
            return not_committed_with_cleanup(storage, snapshot.revision(), temporary, error);
        }
    };
    if revalidated != initial {
        return conflict_with_cleanup(storage, snapshot, temporary, revalidated);
    }

    match storage.replace(temporary, snapshot.target(), revalidated) {
        ReplaceOutcome::Committed(receipt) => {
            let (observation, cleanup_warnings) = receipt.into_parts();
            let (durability, durability_warning) = match storage.sync_parent(snapshot.target()) {
                DurabilityOutcome::Achieved(durability) => (durability, None),
                DurabilityOutcome::Warning { achieved, error } => (achieved, Some(error)),
            };
            SaveOutcome::Committed {
                revision: snapshot.revision(),
                durability,
                observation,
                warnings: SaveWarnings::new(cleanup_warnings, durability_warning),
            }
        }
        ReplaceOutcome::Conflict { temporary, actual } => {
            conflict_with_cleanup(storage, snapshot, temporary, actual)
        }
        ReplaceOutcome::NotCommitted { temporary, error } => {
            not_committed_with_cleanup(storage, snapshot.revision(), temporary, error)
        }
        ReplaceOutcome::CommitStateUnknown { error } => SaveOutcome::CommitStateUnknown {
            revision: snapshot.revision(),
            error,
        },
    }
}

fn not_committed_with_cleanup<S: Storage>(
    storage: &mut S,
    revision: Revision,
    temporary: S::Temporary,
    error: StorageError,
) -> SaveOutcome {
    SaveOutcome::NotCommitted {
        revision,
        error,
        cleanup_error: storage.discard(temporary).err(),
    }
}

fn conflict_with_cleanup<S: Storage>(
    storage: &mut S,
    snapshot: &SaveSnapshot,
    temporary: S::Temporary,
    actual: TargetState,
) -> SaveOutcome {
    SaveOutcome::Conflict {
        revision: snapshot.revision(),
        expected: snapshot.expected(),
        actual,
        cleanup_error: storage.discard(temporary).err(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct FakeTemporary {
        bytes: Vec<u8>,
    }

    struct FakeStorage {
        destination: Option<(Vec<u8>, u128)>,
        fail_at: Option<SaveStage>,
        cleanup_fails: bool,
        race_on_revalidate: Option<Vec<u8>>,
        race_on_replace: Option<Vec<u8>>,
        unknown_after_replace: bool,
        replacement_cleanup_warning: bool,
        durability: DurabilityOutcome,
        calls: Vec<SaveStage>,
        next_identity: u128,
    }

    impl FakeStorage {
        fn existing(bytes: &[u8]) -> Self {
            Self {
                destination: Some((bytes.to_vec(), 1)),
                fail_at: None,
                cleanup_fails: false,
                race_on_revalidate: None,
                race_on_replace: None,
                unknown_after_replace: false,
                replacement_cleanup_warning: false,
                durability: DurabilityOutcome::Achieved(Durability::FileAndDirectorySynced),
                calls: Vec::new(),
                next_identity: 2,
            }
        }

        fn missing() -> Self {
            let mut storage = Self::existing(b"");
            storage.destination = None;
            storage
        }

        fn state(&self) -> TargetState {
            self.destination
                .as_ref()
                .map_or(TargetState::Missing, |(bytes, id)| {
                    TargetState::Regular(observation(bytes, *id))
                })
        }

        fn expectation(&self) -> TargetExpectation {
            match self.state() {
                TargetState::Missing => TargetExpectation::Missing,
                TargetState::Regular(observation) => TargetExpectation::Existing(observation),
                TargetState::Special(_) => unreachable!("the fake stores only regular files"),
            }
        }

        fn destination_bytes(&self) -> Option<&[u8]> {
            self.destination.as_ref().map(|(bytes, _)| bytes.as_slice())
        }

        fn enter(&mut self, stage: SaveStage) -> Result<(), StorageError> {
            self.calls.push(stage);
            if self.fail_at == Some(stage) {
                Err(StorageError::new(stage, "injected failure"))
            } else {
                Ok(())
            }
        }

        fn replace_destination(&mut self, bytes: Vec<u8>) -> FileObservation {
            let identity = self.next_identity;
            self.next_identity += 1;
            let observation = observation(&bytes, identity);
            self.destination = Some((bytes, identity));
            observation
        }
    }

    impl Storage for FakeStorage {
        type Temporary = FakeTemporary;

        fn inspect(&mut self, _path: &Path, stage: SaveStage) -> Result<TargetState, StorageError> {
            self.enter(stage)?;
            if stage == SaveStage::Revalidate
                && let Some(bytes) = self.race_on_revalidate.take()
            {
                self.replace_destination(bytes);
            }
            Ok(self.state())
        }

        fn create_unique_sibling(
            &mut self,
            _destination: &Path,
        ) -> Result<Self::Temporary, StorageError> {
            self.enter(SaveStage::CreateTemporary)?;
            Ok(FakeTemporary { bytes: Vec::new() })
        }

        fn write_all(
            &mut self,
            temporary: &mut Self::Temporary,
            bytes: &[u8],
        ) -> Result<(), StorageError> {
            if let Err(error) = self.enter(SaveStage::Write) {
                temporary.bytes.extend_from_slice(&bytes[..bytes.len() / 2]);
                return Err(error);
            }
            temporary.bytes.extend_from_slice(bytes);
            Ok(())
        }

        fn flush(&mut self, _temporary: &mut Self::Temporary) -> Result<(), StorageError> {
            self.enter(SaveStage::Flush)
        }

        fn apply_metadata(
            &mut self,
            _temporary: &mut Self::Temporary,
            _destination: &Path,
            _source: Option<&FileObservation>,
        ) -> Result<(), StorageError> {
            self.enter(SaveStage::ApplyMetadata)
        }

        fn sync_file(&mut self, _temporary: &mut Self::Temporary) -> Result<(), StorageError> {
            self.enter(SaveStage::SyncFile)
        }

        fn replace(
            &mut self,
            temporary: Self::Temporary,
            _destination: &Path,
            expected: TargetState,
        ) -> ReplaceOutcome<Self::Temporary> {
            if let Err(error) = self.enter(SaveStage::Replace) {
                return ReplaceOutcome::NotCommitted { temporary, error };
            }

            if let Some(bytes) = self.race_on_replace.take() {
                self.replace_destination(bytes);
                return ReplaceOutcome::Conflict {
                    temporary,
                    actual: self.state(),
                };
            }

            if self.unknown_after_replace {
                self.replace_destination(temporary.bytes);
                return ReplaceOutcome::CommitStateUnknown {
                    error: StorageError::new(SaveStage::Reconcile, "injected indeterminate commit"),
                };
            }

            if self.state() != expected {
                return ReplaceOutcome::Conflict {
                    temporary,
                    actual: self.state(),
                };
            }

            let observation = self.replace_destination(temporary.bytes);
            if self.replacement_cleanup_warning {
                ReplaceOutcome::Committed(ReplaceReceipt::with_cleanup_warning(
                    observation,
                    StorageError::new(SaveStage::Cleanup, "injected post-commit cleanup failure"),
                ))
            } else {
                ReplaceOutcome::Committed(ReplaceReceipt::new(observation))
            }
        }

        fn sync_parent(&mut self, _destination: &Path) -> DurabilityOutcome {
            if let Err(error) = self.enter(SaveStage::SyncParent) {
                return DurabilityOutcome::Warning {
                    achieved: Durability::FileSynced,
                    error,
                };
            }
            self.durability.clone()
        }

        fn discard(&mut self, _temporary: Self::Temporary) -> Result<(), StorageError> {
            self.calls.push(SaveStage::Cleanup);
            if self.cleanup_fails {
                Err(StorageError::new(
                    SaveStage::Cleanup,
                    "injected cleanup failure",
                ))
            } else {
                Ok(())
            }
        }
    }

    fn observation(bytes: &[u8], identity: u128) -> FileObservation {
        FileObservation::new(
            FileIdentity::new(7, identity),
            ContentFingerprint::from_bytes(bytes),
            bytes.len() as u64,
            1,
            FileChangeToken::new(identity as i64, 0),
        )
    }

    fn snapshot(storage: &FakeStorage, revision: u64, bytes: &[u8]) -> SaveSnapshot {
        SaveSnapshot::new(
            Revision::new(revision),
            PathBuf::from("note.txt"),
            storage.expectation(),
            bytes.to_vec(),
        )
    }

    #[test]
    fn successful_save_commits_exact_bytes_in_protocol_order() {
        let mut storage = FakeStorage::existing(b"old");
        let snapshot = snapshot(&storage, 9, b"complete new content");

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(
            outcome,
            SaveOutcome::Committed {
                revision,
                durability: Durability::FileAndDirectorySynced,
                ref warnings,
                ..
            } if revision == Revision::new(9) && warnings.is_empty()
        ));
        assert_eq!(
            storage.destination_bytes(),
            Some(b"complete new content".as_slice())
        );
        assert_eq!(
            storage.calls,
            [
                SaveStage::InspectInitial,
                SaveStage::CreateTemporary,
                SaveStage::Write,
                SaveStage::Flush,
                SaveStage::ApplyMetadata,
                SaveStage::SyncFile,
                SaveStage::Revalidate,
                SaveStage::Replace,
                SaveStage::SyncParent,
            ]
        );
    }

    #[test]
    fn every_proven_precommit_failure_preserves_original_bytes() {
        for failed_stage in [
            SaveStage::InspectInitial,
            SaveStage::CreateTemporary,
            SaveStage::Write,
            SaveStage::Flush,
            SaveStage::ApplyMetadata,
            SaveStage::SyncFile,
            SaveStage::Revalidate,
            SaveStage::Replace,
        ] {
            let mut storage = FakeStorage::existing(b"irreplaceable original");
            storage.fail_at = Some(failed_stage);
            let snapshot = snapshot(&storage, 3, b"new bytes");

            let outcome = save_snapshot(&mut storage, &snapshot);

            assert!(
                matches!(outcome, SaveOutcome::NotCommitted { ref error, .. } if error.stage() == failed_stage),
                "wrong outcome at {failed_stage:?}: {outcome:?}"
            );
            assert_eq!(
                storage.destination_bytes(),
                Some(b"irreplaceable original".as_slice()),
                "destination changed at {failed_stage:?}"
            );
        }
    }

    #[test]
    fn initial_version_conflict_does_not_create_temporary_file() {
        let mut storage = FakeStorage::existing(b"version one");
        let snapshot = snapshot(&storage, 4, b"mine");
        storage.replace_destination(b"external version".to_vec());

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(
            outcome,
            SaveOutcome::Conflict {
                cleanup_error: None,
                ..
            }
        ));
        assert_eq!(storage.calls, [SaveStage::InspectInitial]);
        assert_eq!(
            storage.destination_bytes(),
            Some(b"external version".as_slice())
        );
    }

    #[test]
    fn revalidation_conflict_discards_temporary_without_overwrite() {
        let mut storage = FakeStorage::existing(b"version one");
        let snapshot = snapshot(&storage, 5, b"mine");
        storage.race_on_revalidate = Some(b"external version".to_vec());

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(
            outcome,
            SaveOutcome::Conflict {
                cleanup_error: None,
                ..
            }
        ));
        assert_eq!(
            storage.destination_bytes(),
            Some(b"external version".as_slice())
        );
        assert_eq!(storage.calls.last(), Some(&SaveStage::Cleanup));
        assert!(!storage.calls.contains(&SaveStage::Replace));
    }

    #[test]
    fn commit_point_conflict_does_not_overwrite_external_version() {
        let mut storage = FakeStorage::existing(b"version one");
        let snapshot = snapshot(&storage, 6, b"mine");
        storage.race_on_replace = Some(b"last instant external version".to_vec());

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(
            outcome,
            SaveOutcome::Conflict {
                cleanup_error: None,
                ..
            }
        ));
        assert_eq!(
            storage.destination_bytes(),
            Some(b"last instant external version".as_slice())
        );
        assert_eq!(storage.calls.last(), Some(&SaveStage::Cleanup));
    }

    #[test]
    fn indeterminate_platform_result_is_never_reported_as_not_committed() {
        let mut storage = FakeStorage::existing(b"old");
        storage.unknown_after_replace = true;
        let snapshot = snapshot(&storage, 7, b"possibly committed");

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(
            outcome,
            SaveOutcome::CommitStateUnknown { revision, ref error }
                if revision == Revision::new(7) && error.stage() == SaveStage::Reconcile
        ));
        assert_eq!(
            storage.destination_bytes(),
            Some(b"possibly committed".as_slice())
        );
        assert!(!storage.calls.contains(&SaveStage::Cleanup));
    }

    #[test]
    fn parent_sync_failure_reports_committed_with_warning() {
        let mut storage = FakeStorage::existing(b"old");
        storage.fail_at = Some(SaveStage::SyncParent);
        let snapshot = snapshot(&storage, 8, b"committed before warning");

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(
            outcome,
            SaveOutcome::Committed {
                durability: Durability::FileSynced,
                ref warnings,
                ..
            } if warnings.durability().is_some_and(|error| error.stage() == SaveStage::SyncParent)
                && warnings.cleanup().is_empty()
        ));
        assert_eq!(
            storage.destination_bytes(),
            Some(b"committed before warning".as_slice())
        );
    }

    #[test]
    fn committed_cleanup_and_durability_warnings_are_both_preserved() {
        let mut storage = FakeStorage::existing(b"old");
        storage.replacement_cleanup_warning = true;
        storage.fail_at = Some(SaveStage::SyncParent);
        let snapshot = snapshot(&storage, 12, b"committed with two warnings");

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(
            outcome,
            SaveOutcome::Committed {
                durability: Durability::FileSynced,
                ref warnings,
                ..
            } if warnings.cleanup().iter().any(|error| error.stage() == SaveStage::Cleanup)
                && warnings.durability().is_some_and(|error| error.stage() == SaveStage::SyncParent)
        ));
        assert_eq!(
            storage.destination_bytes(),
            Some(b"committed with two warnings".as_slice())
        );
    }

    #[test]
    fn cleanup_failure_is_preserved_beside_primary_failure() {
        let mut storage = FakeStorage::existing(b"old");
        storage.fail_at = Some(SaveStage::Write);
        storage.cleanup_fails = true;
        let snapshot = snapshot(&storage, 10, b"new");

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(
            outcome,
            SaveOutcome::NotCommitted {
                ref error,
                cleanup_error: Some(ref cleanup),
                ..
            } if error.stage() == SaveStage::Write && cleanup.stage() == SaveStage::Cleanup
        ));
        assert_eq!(storage.destination_bytes(), Some(b"old".as_slice()));
    }

    #[test]
    fn absent_destination_uses_exclusive_creation_contract() {
        let mut storage = FakeStorage::missing();
        let snapshot = snapshot(&storage, 11, b"brand new");

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(outcome, SaveOutcome::Committed { .. }));
        assert_eq!(storage.destination_bytes(), Some(b"brand new".as_slice()));
    }

    #[test]
    fn domain_values_retain_exact_adapter_data() {
        let identity = FileIdentity::new(12, 34);
        let fingerprint = ContentFingerprint::new([5; 32]);
        let change_token = FileChangeToken::new(67, 89);
        let observation = FileObservation::new(identity, fingerprint, 56, 2, change_token);
        let snapshot = SaveSnapshot::new(
            Revision::new(78),
            PathBuf::from("note.txt"),
            TargetExpectation::Existing(observation),
            vec![1, 2, 3],
        );
        let error = StorageError::new(SaveStage::Write, "safe diagnostic");

        assert_eq!(identity.volume(), 12);
        assert_eq!(identity.file(), 34);
        assert_eq!(identity.quality(), IdentityQuality::Preferred);
        assert_eq!(
            FileIdentity::reduced(12, 34).quality(),
            IdentityQuality::Reduced
        );
        assert_eq!(fingerprint.as_bytes(), &[5; 32]);
        assert_eq!(observation.identity(), identity);
        assert_eq!(observation.fingerprint(), fingerprint);
        assert_eq!(observation.length(), 56);
        assert_eq!(observation.link_count(), 2);
        assert_eq!(observation.change_token(), change_token);
        assert_eq!(change_token.primary(), 67);
        assert_eq!(change_token.secondary(), 89);
        assert_eq!(snapshot.revision(), Revision::new(78));
        assert_eq!(snapshot.target(), Path::new("note.txt"));
        assert_eq!(snapshot.bytes(), &[1, 2, 3]);
        assert_eq!(error.message(), "safe diagnostic");
    }

    #[test]
    fn content_fingerprints_match_official_blake3_vectors() {
        assert_eq!(
            ContentFingerprint::from_bytes(b"").as_bytes(),
            &[
                0xaf, 0x13, 0x49, 0xb9, 0xf5, 0xf9, 0xa1, 0xa6, 0xa0, 0x40, 0x4d, 0xea, 0x36, 0xdc,
                0xc9, 0x49, 0x9b, 0xcb, 0x25, 0xc9, 0xad, 0xc1, 0x12, 0xb7, 0xcc, 0x9a, 0x93, 0xca,
                0xe4, 0x1f, 0x32, 0x62,
            ]
        );
        assert_eq!(
            ContentFingerprint::from_bytes(&[0]).as_bytes(),
            &[
                0x2d, 0x3a, 0xde, 0xdf, 0xf1, 0x1b, 0x61, 0xf1, 0x4c, 0x88, 0x6e, 0x35, 0xaf, 0xa0,
                0x36, 0x73, 0x6d, 0xcd, 0x87, 0xa7, 0x4d, 0x27, 0xb5, 0xc1, 0x51, 0x02, 0x25, 0xd0,
                0xf5, 0x92, 0xe2, 0x13,
            ]
        );
    }

    #[test]
    fn reader_fingerprint_matches_in_memory_fingerprint() -> io::Result<()> {
        let bytes = b"content supplied through a reader";

        assert_eq!(
            ContentFingerprint::from_reader(bytes.as_slice())?,
            ContentFingerprint::from_bytes(bytes)
        );
        Ok(())
    }

    #[test]
    fn reader_fingerprint_propagates_read_failure() {
        struct FailedReader;

        impl Read for FailedReader {
            fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "injected read failure",
                ))
            }
        }

        let error = ContentFingerprint::from_reader(FailedReader)
            .expect_err("an incomplete read must not produce a fingerprint");

        assert_eq!(error.kind(), io::ErrorKind::UnexpectedEof);
    }
}
