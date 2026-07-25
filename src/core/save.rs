//! Revision-tagged, fault-injectable save protocol.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

use super::revision::Revision;

/// A stable platform file identity represented as volume and file components.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileIdentity {
    volume: u128,
    file: u128,
}

impl FileIdentity {
    /// Creates a platform-neutral identity from adapter-provided components.
    pub const fn new(volume: u128, file: u128) -> Self {
        Self { volume, file }
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

/// A collision-resistant digest of serialized file content.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct ContentFingerprint([u8; 32]);

impl ContentFingerprint {
    /// Creates a fingerprint from adapter-computed digest bytes.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the digest bytes.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Identity and content facts captured for an existing regular file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileObservation {
    identity: FileIdentity,
    fingerprint: ContentFingerprint,
    length: u64,
    link_count: u64,
}

impl FileObservation {
    /// Creates a complete regular-file observation.
    pub const fn new(
        identity: FileIdentity,
        fingerprint: ContentFingerprint,
        length: u64,
        link_count: u64,
    ) -> Self {
        Self {
            identity,
            fingerprint,
            length,
            link_count,
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

/// Verified destination facts after a successful replacement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReplaceReceipt {
    observation: FileObservation,
}

impl ReplaceReceipt {
    /// Creates a verified replacement receipt.
    pub const fn new(observation: FileObservation) -> Self {
        Self { observation }
    }

    /// Returns the committed destination observation.
    pub const fn observation(self) -> FileObservation {
        self.observation
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
        /// Post-commit warning, if a stronger barrier failed.
        warning: Option<StorageError>,
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
            let (durability, warning) = match storage.sync_parent(snapshot.target()) {
                DurabilityOutcome::Achieved(durability) => (durability, None),
                DurabilityOutcome::Warning { achieved, error } => (achieved, Some(error)),
            };
            SaveOutcome::Committed {
                revision: snapshot.revision(),
                durability,
                observation: receipt.observation(),
                warning,
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
            ReplaceOutcome::Committed(ReplaceReceipt::new(observation))
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

    fn fingerprint(bytes: &[u8]) -> ContentFingerprint {
        let mut digest = [0_u8; 32];
        for (index, byte) in bytes.iter().copied().enumerate() {
            let slot = index % digest.len();
            digest[slot] = digest[slot].wrapping_add(byte).wrapping_add(index as u8);
        }
        ContentFingerprint::new(digest)
    }

    fn observation(bytes: &[u8], identity: u128) -> FileObservation {
        FileObservation::new(
            FileIdentity::new(7, identity),
            fingerprint(bytes),
            bytes.len() as u64,
            1,
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
                warning: None,
                ..
            } if revision == Revision::new(9)
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
                warning: Some(ref error),
                ..
            } if error.stage() == SaveStage::SyncParent
        ));
        assert_eq!(
            storage.destination_bytes(),
            Some(b"committed before warning".as_slice())
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
        let observation = FileObservation::new(identity, fingerprint, 56, 2);
        let snapshot = SaveSnapshot::new(
            Revision::new(78),
            PathBuf::from("note.txt"),
            TargetExpectation::Existing(observation),
            vec![1, 2, 3],
        );
        let error = StorageError::new(SaveStage::Write, "safe diagnostic");

        assert_eq!(identity.volume(), 12);
        assert_eq!(identity.file(), 34);
        assert_eq!(fingerprint.as_bytes(), &[5; 32]);
        assert_eq!(observation.identity(), identity);
        assert_eq!(observation.fingerprint(), fingerprint);
        assert_eq!(observation.length(), 56);
        assert_eq!(observation.link_count(), 2);
        assert_eq!(snapshot.revision(), Revision::new(78));
        assert_eq!(snapshot.target(), Path::new("note.txt"));
        assert_eq!(snapshot.bytes(), &[1, 2, 3]);
        assert_eq!(error.message(), "safe diagnostic");
    }
}
