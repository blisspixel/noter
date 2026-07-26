//! Production filesystem storage primitives.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::file_observation::{inspect_target, is_final_link};
use super::save::{
    ContentFingerprint, Durability, DurabilityOutcome, FileIdentity, FileObservation,
    IdentityQuality, ReplaceOutcome, ReplaceReceipt, SaveStage, Storage, StorageError, TargetState,
};

const RANDOM_NAME_BYTES: usize = 16;
const MAX_CREATE_ATTEMPTS: usize = 16;
const TEMPORARY_PREFIX: &str = ".noter-save-";
const TEMPORARY_SUFFIX: &str = ".tmp";
#[cfg(windows)]
const BACKUP_PREFIX: &str = ".noter-backup-";
#[cfg(windows)]
const BACKUP_SUFFIX: &str = ".bak";

/// An exclusively created sibling owned by one save attempt.
///
/// Dropping this value makes a best-effort cleanup attempt. The storage
/// protocol uses [`TemporaryFile::discard`] when it must observe cleanup
/// failures explicitly.
#[derive(Debug)]
pub struct TemporaryFile {
    file: Option<File>,
    path: PathBuf,
    identity: noter_platform::FileIdentity,
    intended: Option<IntendedContent>,
    cleanup_armed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct IntendedContent {
    fingerprint: ContentFingerprint,
    length: u64,
}

impl IntendedContent {
    fn matches(self, observation: FileObservation) -> bool {
        self.fingerprint == observation.fingerprint() && self.length == observation.length()
    }
}

impl TemporaryFile {
    /// Returns the private sibling path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the identity captured from the original open handle.
    pub const fn identity(&self) -> noter_platform::FileIdentity {
        self.identity
    }

    /// Writes every byte to the open sibling handle.
    ///
    /// # Errors
    ///
    /// Returns the underlying write error if the complete slice cannot be
    /// delivered. A sibling accepts exactly one complete serialized snapshot.
    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.intended.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "temporary file already contains a serialized snapshot",
            ));
        }
        let length = u64::try_from(bytes.len()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "serialized snapshot length exceeds the supported file size",
            )
        })?;
        self.file_mut()?.write_all(bytes)?;
        self.intended = Some(IntendedContent {
            fingerprint: ContentFingerprint::from_bytes(bytes),
            length,
        });
        Ok(())
    }

    /// Flushes user-space buffers for the sibling handle.
    ///
    /// # Errors
    ///
    /// Returns the underlying flush error.
    pub fn flush(&mut self) -> io::Result<()> {
        self.file_mut()?.flush()
    }

    /// Synchronizes sibling data and metadata through the strongest supported barrier.
    ///
    /// # Errors
    ///
    /// Returns the underlying synchronization error.
    pub fn sync_all(&self) -> io::Result<()> {
        noter_platform::sync_file(self.file()?)
    }

    /// Verifies that the random pathname still identifies the original handle.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the path or its identity cannot be inspected.
    pub fn path_still_identifies_file(&self) -> io::Result<bool> {
        let metadata = fs::symlink_metadata(&self.path)?;
        if !metadata.is_file() || is_final_link(&metadata) {
            return Ok(false);
        }

        let reopened = File::open(&self.path)?;
        Ok(noter_platform::file_facts(&reopened)?.identity() == self.identity)
    }

    /// Closes the handle and removes the private sibling explicitly.
    ///
    /// A missing path already satisfies cleanup and is treated as success.
    ///
    /// # Errors
    ///
    /// Returns an error when an existing sibling cannot be removed or the
    /// random path no longer names the file owned by this save attempt.
    pub fn discard(mut self) -> io::Result<()> {
        match self.path_still_identifies_file() {
            Ok(true) => {}
            Ok(false) => {
                self.cleanup_armed = false;
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "temporary path no longer identifies the owned file",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.cleanup_armed = false;
                return Ok(());
            }
            Err(error) => return Err(error),
        }

        self.file.take();
        match fs::remove_file(&self.path) {
            Ok(()) => {
                self.cleanup_armed = false;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                self.cleanup_armed = false;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    fn file(&self) -> io::Result<&File> {
        self.file
            .as_ref()
            .ok_or_else(|| io::Error::other("temporary file handle is closed"))
    }

    fn file_mut(&mut self) -> io::Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| io::Error::other("temporary file handle is closed"))
    }

    fn intended(&self) -> io::Result<IntendedContent> {
        self.intended.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "temporary file has no complete serialized snapshot",
            )
        })
    }

    fn committed_observation_matches(&self, observation: FileObservation) -> io::Result<bool> {
        Ok(self.intended()?.matches(observation)
            && platform_identity_matches(self.identity, observation.identity()))
    }

    fn close_handle(&mut self) {
        self.file.take();
    }

    const fn preserve_artifact(&mut self) {
        self.cleanup_armed = false;
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if self.cleanup_armed && matches!(self.path_still_identifies_file(), Ok(true)) {
            self.file.take();
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Creates an unpredictable, exclusive sibling in the destination directory.
///
/// The sibling starts owner-only on Unix and is opened for both reading and
/// writing. Later metadata policy may widen its final mode immediately before
/// commit. Its name contains 128 bits from the operating-system random source.
///
/// # Errors
///
/// Returns an error when the destination has no filename, the operating-system
/// random source fails, or an exclusive sibling cannot be created after a
/// bounded number of collisions.
pub fn create_unique_sibling(destination: &Path) -> io::Result<TemporaryFile> {
    create_unique_sibling_with(destination, &mut OsRandom)
}

fn create_unique_sibling_with(
    destination: &Path,
    random: &mut impl RandomSource,
) -> io::Result<TemporaryFile> {
    if destination.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "save destination must have a filename",
        ));
    }

    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));

    for _ in 0..MAX_CREATE_ATTEMPTS {
        let mut random_bytes = [0_u8; RANDOM_NAME_BYTES];
        random.fill(&mut random_bytes)?;
        let path = parent.join(candidate_name(&random_bytes));

        match open_exclusive(&path) {
            Ok(file) => {
                let identity = match noter_platform::file_facts(&file) {
                    Ok(facts) => facts.identity(),
                    Err(error) => {
                        drop(file);
                        let _ = fs::remove_file(&path);
                        return Err(error);
                    }
                };
                return Ok(TemporaryFile {
                    file: Some(file),
                    path,
                    identity,
                    intended: None,
                    cleanup_armed: true,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create an exclusive random sibling after 16 attempts",
    ))
}

fn open_exclusive(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);

    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }

    options.open(path)
}

fn candidate_name(random: &[u8; RANDOM_NAME_BYTES]) -> String {
    artifact_name(TEMPORARY_PREFIX, random, TEMPORARY_SUFFIX)
}

fn artifact_name(prefix: &str, random: &[u8; RANDOM_NAME_BYTES], suffix: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut name = String::with_capacity(prefix.len() + (RANDOM_NAME_BYTES * 2) + suffix.len());
    name.push_str(prefix);
    for byte in random {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    name.push_str(suffix);
    name
}

trait RandomSource {
    fn fill(&mut self, bytes: &mut [u8]) -> io::Result<()>;
}

struct OsRandom;

impl RandomSource for OsRandom {
    fn fill(&mut self, bytes: &mut [u8]) -> io::Result<()> {
        getrandom::fill(bytes).map_err(|error| {
            io::Error::other(format!("operating-system randomness failed: {error}"))
        })
    }
}

/// Production adapter for the fault-injected save protocol.
#[derive(Default, Debug)]
pub struct FilesystemStorage;

impl Storage for FilesystemStorage {
    type Temporary = TemporaryFile;

    fn inspect(&mut self, path: &Path, stage: SaveStage) -> Result<TargetState, StorageError> {
        inspect_target(path, stage)
    }

    fn create_unique_sibling(
        &mut self,
        destination: &Path,
    ) -> Result<Self::Temporary, StorageError> {
        create_unique_sibling(destination).map_err(|error| {
            redacted_io_error(SaveStage::CreateTemporary, "create private sibling", &error)
        })
    }

    fn write_all(
        &mut self,
        temporary: &mut Self::Temporary,
        bytes: &[u8],
    ) -> Result<(), StorageError> {
        temporary
            .write_all(bytes)
            .map_err(|error| redacted_io_error(SaveStage::Write, "write complete snapshot", &error))
    }

    fn flush(&mut self, temporary: &mut Self::Temporary) -> Result<(), StorageError> {
        temporary
            .flush()
            .map_err(|error| redacted_io_error(SaveStage::Flush, "flush sibling", &error))
    }

    fn apply_metadata(
        &mut self,
        temporary: &mut Self::Temporary,
        destination: &Path,
        source: Option<&FileObservation>,
    ) -> Result<(), StorageError> {
        let Some(expected) = source.copied() else {
            return Ok(());
        };

        let observed = inspect_target(destination, SaveStage::ApplyMetadata)?;
        if observed != TargetState::Regular(expected) {
            return Err(StorageError::new(
                SaveStage::ApplyMetadata,
                "destination changed before metadata transfer",
            ));
        }

        let source_file = File::open(destination).map_err(|error| {
            redacted_io_error(SaveStage::ApplyMetadata, "open metadata source", &error)
        })?;
        let source_facts = noter_platform::file_facts(&source_file).map_err(|error| {
            redacted_io_error(
                SaveStage::ApplyMetadata,
                "read metadata-source identity",
                &error,
            )
        })?;
        if !platform_facts_match(source_facts, expected) {
            return Err(StorageError::new(
                SaveStage::ApplyMetadata,
                "destination changed while opening metadata source",
            ));
        }
        if source_file
            .metadata()
            .map_err(|error| {
                redacted_io_error(
                    SaveStage::ApplyMetadata,
                    "read metadata-source permissions",
                    &error,
                )
            })?
            .permissions()
            .readonly()
        {
            return Err(StorageError::new(
                SaveStage::ApplyMetadata,
                "destination is read-only and requires an explicit permission change or Save As",
            ));
        }

        let temporary_file = temporary.file().map_err(|error| {
            redacted_io_error(
                SaveStage::ApplyMetadata,
                "access private sibling handle",
                &error,
            )
        })?;
        noter_platform::copy_required_metadata(&source_file, temporary_file).map_err(|error| {
            redacted_io_error(
                SaveStage::ApplyMetadata,
                "preserve required destination metadata",
                &error,
            )
        })?;

        if inspect_target(destination, SaveStage::ApplyMetadata)? != TargetState::Regular(expected)
        {
            return Err(StorageError::new(
                SaveStage::ApplyMetadata,
                "destination changed during metadata transfer",
            ));
        }
        Ok(())
    }

    fn sync_file(&mut self, temporary: &mut Self::Temporary) -> Result<(), StorageError> {
        temporary
            .sync_all()
            .map_err(|error| redacted_io_error(SaveStage::SyncFile, "synchronize sibling", &error))
    }

    fn replace(
        &mut self,
        mut temporary: Self::Temporary,
        destination: &Path,
        expected: TargetState,
    ) -> ReplaceOutcome<Self::Temporary> {
        match temporary.path_still_identifies_file() {
            Ok(true) => {}
            Ok(false) => {
                return ReplaceOutcome::NotCommitted {
                    temporary,
                    error: StorageError::new(
                        SaveStage::Replace,
                        "private sibling identity changed before commit",
                    ),
                };
            }
            Err(error) => {
                return ReplaceOutcome::NotCommitted {
                    temporary,
                    error: redacted_io_error(
                        SaveStage::Replace,
                        "revalidate private sibling",
                        &error,
                    ),
                };
            }
        }
        if let Err(error) = temporary.intended() {
            return ReplaceOutcome::NotCommitted {
                temporary,
                error: redacted_io_error(
                    SaveStage::Replace,
                    "validate complete sibling content",
                    &error,
                ),
            };
        }

        temporary.close_handle();
        match expected {
            TargetState::Regular(expected) => {
                replace_existing_file(temporary, destination, expected)
            }
            TargetState::Missing => install_new_file(temporary, destination),
            TargetState::Special(kind) => ReplaceOutcome::NotCommitted {
                temporary,
                error: StorageError::new(
                    SaveStage::Replace,
                    format!("refused special destination at commit point: {kind:?}"),
                ),
            },
        }
    }

    fn sync_parent(&mut self, destination: &Path) -> DurabilityOutcome {
        match noter_platform::sync_parent(destination) {
            Ok(noter_platform::ParentSyncOutcome::Synced) => {
                DurabilityOutcome::Achieved(Durability::FileAndDirectorySynced)
            }
            Ok(noter_platform::ParentSyncOutcome::Unsupported) => {
                DurabilityOutcome::Achieved(Durability::FileSynced)
            }
            Err(error) => DurabilityOutcome::Warning {
                achieved: Durability::FileSynced,
                error: redacted_io_error(
                    SaveStage::SyncParent,
                    "synchronize destination directory",
                    &error,
                ),
            },
        }
    }

    fn discard(&mut self, temporary: Self::Temporary) -> Result<(), StorageError> {
        temporary.discard().map_err(|error| {
            redacted_io_error(SaveStage::Cleanup, "remove private sibling", &error)
        })
    }
}

fn replace_existing_file(
    temporary: TemporaryFile,
    destination: &Path,
    expected: FileObservation,
) -> ReplaceOutcome<TemporaryFile> {
    let backup = match replacement_backup_path(destination) {
        Ok(backup) => backup,
        Err(error) => {
            return ReplaceOutcome::NotCommitted {
                temporary,
                error: redacted_io_error(
                    SaveStage::Replace,
                    "reserve replacement backup name",
                    &error,
                ),
            };
        }
    };

    match noter_platform::replace_existing(temporary.path(), destination, backup.as_deref()) {
        Ok(()) => finalize_commit(
            temporary,
            destination,
            backup.map(|path| (path, expected)),
            Vec::new(),
        ),
        Err(error) => reconcile_existing_failure(temporary, destination, expected, backup, &error),
    }
}

fn install_new_file(temporary: TemporaryFile, destination: &Path) -> ReplaceOutcome<TemporaryFile> {
    match noter_platform::install_new(temporary.path(), destination) {
        Ok(_outcome) => finalize_commit(temporary, destination, None, Vec::new()),
        Err(error) => reconcile_new_failure(temporary, destination, &error),
    }
}

fn reconcile_new_failure(
    temporary: TemporaryFile,
    destination: &Path,
    platform_error: &io::Error,
) -> ReplaceOutcome<TemporaryFile> {
    let actual = match inspect_target(destination, SaveStage::Reconcile) {
        Ok(actual) => actual,
        Err(error) => return unknown_with_preserved_temporary(temporary, error),
    };
    if let TargetState::Regular(observation) = actual
        && matches!(
            temporary.committed_observation_matches(observation),
            Ok(true)
        )
    {
        return finalize_commit(temporary, destination, None, Vec::new());
    }

    match temporary.path_still_identifies_file() {
        Ok(true) if actual == TargetState::Missing => ReplaceOutcome::NotCommitted {
            temporary,
            error: redacted_io_error(
                SaveStage::Replace,
                "install absent destination",
                platform_error,
            ),
        },
        Ok(true) => ReplaceOutcome::Conflict { temporary, actual },
        Ok(false) => unknown_with_preserved_temporary(
            temporary,
            StorageError::new(
                SaveStage::Reconcile,
                "new-file commit failed and private sibling identity changed",
            ),
        ),
        Err(error) => unknown_with_preserved_temporary(
            temporary,
            redacted_io_error(
                SaveStage::Reconcile,
                "reconcile private sibling after new-file failure",
                &error,
            ),
        ),
    }
}

fn reconcile_existing_failure(
    temporary: TemporaryFile,
    destination: &Path,
    expected: FileObservation,
    backup: Option<PathBuf>,
    platform_error: &io::Error,
) -> ReplaceOutcome<TemporaryFile> {
    let actual = match inspect_target(destination, SaveStage::Reconcile) {
        Ok(actual) => actual,
        Err(error) => return unknown_with_preserved_temporary(temporary, error),
    };
    if let TargetState::Regular(observation) = actual
        && matches!(
            temporary.committed_observation_matches(observation),
            Ok(true)
        )
    {
        return finalize_commit(
            temporary,
            destination,
            backup.map(|path| (path, expected)),
            Vec::new(),
        );
    }

    let partial_move = is_documented_partial_replacement(platform_error);
    if partial_move
        && actual == TargetState::Missing
        && matches!(temporary.path_still_identifies_file(), Ok(true))
        && backup
            .as_deref()
            .is_some_and(|path| backup_matches_expected(path, expected))
    {
        match noter_platform::install_new(temporary.path(), destination) {
            Ok(_outcome) => {
                return finalize_commit(
                    temporary,
                    destination,
                    backup.map(|path| (path, expected)),
                    Vec::new(),
                );
            }
            Err(_finish_error) => {
                return unknown_with_preserved_temporary(
                    temporary,
                    StorageError::new(
                        SaveStage::Reconcile,
                        "documented partial replacement could not be completed safely",
                    ),
                );
            }
        }
    }

    match temporary.path_still_identifies_file() {
        Ok(true) if !partial_move && actual == TargetState::Regular(expected) => {
            ReplaceOutcome::NotCommitted {
                temporary,
                error: redacted_io_error(
                    SaveStage::Replace,
                    "replace existing destination",
                    platform_error,
                ),
            }
        }
        Ok(true) if !partial_move => ReplaceOutcome::Conflict { temporary, actual },
        Ok(true | false) => unknown_with_preserved_temporary(
            temporary,
            StorageError::new(
                SaveStage::Reconcile,
                "replacement failure left a documented partial or unexplained path state",
            ),
        ),
        Err(error) => unknown_with_preserved_temporary(
            temporary,
            redacted_io_error(
                SaveStage::Reconcile,
                "reconcile private sibling after replacement failure",
                &error,
            ),
        ),
    }
}

fn finalize_commit(
    temporary: TemporaryFile,
    destination: &Path,
    backup: Option<(PathBuf, FileObservation)>,
    mut cleanup_warnings: Vec<StorageError>,
) -> ReplaceOutcome<TemporaryFile> {
    let observation = match inspect_target(destination, SaveStage::Reconcile) {
        Ok(TargetState::Regular(observation))
            if matches!(
                temporary.committed_observation_matches(observation),
                Ok(true)
            ) =>
        {
            observation
        }
        Ok(_) => {
            return unknown_with_preserved_temporary(
                temporary,
                StorageError::new(
                    SaveStage::Reconcile,
                    "commit operation returned success but destination verification differed",
                ),
            );
        }
        Err(error) => return unknown_with_preserved_temporary(temporary, error),
    };

    if let Err(error) = temporary.discard() {
        cleanup_warnings.push(redacted_io_error(
            SaveStage::Cleanup,
            "remove committed temporary name",
            &error,
        ));
    }
    if let Some((backup, expected)) = backup
        && let Err(error) = remove_verified_backup(&backup, expected)
    {
        cleanup_warnings.push(error);
    }

    ReplaceOutcome::Committed(ReplaceReceipt::with_cleanup_warnings(
        observation,
        cleanup_warnings,
    ))
}

fn unknown_with_preserved_temporary(
    mut temporary: TemporaryFile,
    error: StorageError,
) -> ReplaceOutcome<TemporaryFile> {
    temporary.preserve_artifact();
    ReplaceOutcome::CommitStateUnknown { error }
}

fn remove_verified_backup(path: &Path, expected: FileObservation) -> Result<(), StorageError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(redacted_io_error(
                SaveStage::Cleanup,
                "inspect replacement backup",
                &error,
            ));
        }
    };
    if !metadata.is_file() || is_final_link(&metadata) {
        return Err(StorageError::new(
            SaveStage::Cleanup,
            "replacement backup path no longer names the expected regular file",
        ));
    }
    let file = File::open(path).map_err(|error| {
        redacted_io_error(SaveStage::Cleanup, "open replacement backup", &error)
    })?;
    let facts = noter_platform::file_facts(&file).map_err(|error| {
        redacted_io_error(
            SaveStage::Cleanup,
            "read replacement backup identity",
            &error,
        )
    })?;
    if !platform_identity_matches(facts.identity(), expected.identity()) {
        return Err(StorageError::new(
            SaveStage::Cleanup,
            "replacement backup identity differs from the replaced file",
        ));
    }
    drop(file);
    fs::remove_file(path)
        .map_err(|error| redacted_io_error(SaveStage::Cleanup, "remove replacement backup", &error))
}

fn backup_matches_expected(path: &Path, expected: FileObservation) -> bool {
    matches!(
        inspect_target(path, SaveStage::Reconcile),
        Ok(TargetState::Regular(actual))
            if actual.identity() == expected.identity()
                && actual.fingerprint() == expected.fingerprint()
                && actual.length() == expected.length()
    )
}

const fn platform_facts_match(facts: noter_platform::FileFacts, expected: FileObservation) -> bool {
    let change = facts.change_token();
    platform_identity_matches(facts.identity(), expected.identity())
        && change.primary() == expected.change_token().primary()
        && change.secondary() == expected.change_token().secondary()
}

const fn platform_identity_matches(
    actual: noter_platform::FileIdentity,
    expected: FileIdentity,
) -> bool {
    actual.volume() == expected.volume()
        && actual.file() == expected.file()
        && matches!(
            (actual.quality(), expected.quality()),
            (
                noter_platform::IdentityQuality::Preferred,
                IdentityQuality::Preferred
            ) | (
                noter_platform::IdentityQuality::Reduced,
                IdentityQuality::Reduced
            )
        )
}

#[cfg(windows)]
fn replacement_backup_path(destination: &Path) -> io::Result<Option<PathBuf>> {
    let parent = normalized_parent(destination);
    for _ in 0..MAX_CREATE_ATTEMPTS {
        let mut random = [0_u8; RANDOM_NAME_BYTES];
        OsRandom.fill(&mut random)?;
        let candidate = parent.join(artifact_name(BACKUP_PREFIX, &random, BACKUP_SUFFIX));
        match fs::symlink_metadata(&candidate) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(Some(candidate));
            }
            Ok(_) => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not reserve a random backup name after 16 attempts",
    ))
}

#[cfg(not(windows))]
#[allow(clippy::unnecessary_wraps)]
const fn replacement_backup_path(_destination: &Path) -> io::Result<Option<PathBuf>> {
    Ok(None)
}

#[cfg(windows)]
fn is_documented_partial_replacement(error: &io::Error) -> bool {
    const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1_177;
    error.raw_os_error() == Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2)
}

#[cfg(not(windows))]
const fn is_documented_partial_replacement(_error: &io::Error) -> bool {
    false
}

#[cfg(windows)]
fn normalized_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

fn redacted_io_error(stage: SaveStage, operation: &str, error: &io::Error) -> StorageError {
    let os_code = error
        .raw_os_error()
        .map_or_else(String::new, |code| format!(", OS code {code}"));
    StorageError::new(
        stage,
        format!("{operation} failed with {:?}{os_code}", error.kind()),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::io::{self, Read};

    use tempfile::tempdir;

    use super::*;
    use crate::core::revision::Revision;
    use crate::core::save::{
        SaveOutcome, SaveSnapshot, SpecialFileKind, TargetExpectation, save_snapshot,
    };

    struct SequenceRandom {
        values: VecDeque<[u8; RANDOM_NAME_BYTES]>,
        fallback: [u8; RANDOM_NAME_BYTES],
        failure: Option<io::ErrorKind>,
        calls: usize,
    }

    impl SequenceRandom {
        fn new(values: impl IntoIterator<Item = [u8; RANDOM_NAME_BYTES]>) -> Self {
            Self {
                values: values.into_iter().collect(),
                fallback: [0; RANDOM_NAME_BYTES],
                failure: None,
                calls: 0,
            }
        }
    }

    impl RandomSource for SequenceRandom {
        fn fill(&mut self, bytes: &mut [u8]) -> io::Result<()> {
            self.calls += 1;
            if let Some(kind) = self.failure {
                return Err(io::Error::new(kind, "injected random-source failure"));
            }

            let value = self.values.pop_front().unwrap_or(self.fallback);
            bytes.copy_from_slice(&value);
            Ok(())
        }
    }

    #[test]
    fn random_name_has_fixed_private_shape() {
        assert_eq!(
            candidate_name(&[0xab; RANDOM_NAME_BYTES]),
            ".noter-save-abababababababababababababababab.tmp"
        );
    }

    #[test]
    fn exclusive_creation_retries_collision_without_reusing_file() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let collision = directory
            .path()
            .join(candidate_name(&[0; RANDOM_NAME_BYTES]));
        fs::write(&collision, b"belongs to someone else")?;
        let mut random = SequenceRandom::new([[0; RANDOM_NAME_BYTES], [1; RANDOM_NAME_BYTES]]);

        let temporary = create_unique_sibling_with(&destination, &mut random)?;

        assert_eq!(random.calls, 2);
        assert_eq!(fs::read(&collision)?, b"belongs to someone else");
        assert_eq!(
            temporary.path().file_name(),
            Some(candidate_name(&[1; RANDOM_NAME_BYTES]).as_ref())
        );
        Ok(())
    }

    #[test]
    fn repeated_collisions_stop_at_the_bounded_limit() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let collision = directory
            .path()
            .join(candidate_name(&[0; RANDOM_NAME_BYTES]));
        fs::write(collision, b"untouched")?;
        let mut random = SequenceRandom::new([]);

        let error = create_unique_sibling_with(&destination, &mut random)
            .expect_err("repeated collisions must stop");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(random.calls, MAX_CREATE_ATTEMPTS);
        Ok(())
    }

    #[test]
    fn random_source_failure_creates_nothing() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let mut random = SequenceRandom::new([]);
        random.failure = Some(io::ErrorKind::Other);

        let error = create_unique_sibling_with(&destination, &mut random)
            .expect_err("random-source failure must abort creation");

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(fs::read_dir(directory.path())?.count(), 0);
        Ok(())
    }

    #[test]
    fn invalid_destination_is_rejected_before_randomness() {
        let mut random = SequenceRandom::new([]);

        let error = create_unique_sibling_with(Path::new("/"), &mut random)
            .expect_err("a destination without a filename is invalid");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(random.calls, 0);
    }

    #[test]
    fn writes_syncs_verifies_and_explicitly_discards() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let mut random = SequenceRandom::new([[2; RANDOM_NAME_BYTES]]);
        let mut temporary = create_unique_sibling_with(&destination, &mut random)?;
        let path = temporary.path().to_path_buf();

        temporary.write_all(b"complete bytes")?;
        temporary.flush()?;
        temporary.sync_all()?;

        let mut actual = Vec::new();
        File::open(&path)?.read_to_end(&mut actual)?;
        assert_eq!(actual, b"complete bytes");
        assert!(temporary.path_still_identifies_file()?);

        temporary.discard()?;
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn drop_removes_an_uncommitted_sibling() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let mut random = SequenceRandom::new([[3; RANDOM_NAME_BYTES]]);
        let temporary = create_unique_sibling_with(&destination, &mut random)?;
        let path = temporary.path().to_path_buf();

        drop(temporary);

        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn replaced_random_path_fails_identity_verification() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let mut random = SequenceRandom::new([[4; RANDOM_NAME_BYTES]]);
        let temporary = create_unique_sibling_with(&destination, &mut random)?;
        let path = temporary.path().to_path_buf();
        fs::remove_file(&path)?;
        fs::write(&path, b"attacker replacement")?;

        assert!(!temporary.path_still_identifies_file()?);
        drop(temporary);
        assert_eq!(fs::read(path)?, b"attacker replacement");
        Ok(())
    }

    #[test]
    fn explicit_discard_never_deletes_a_replaced_path() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let mut random = SequenceRandom::new([[6; RANDOM_NAME_BYTES]]);
        let temporary = create_unique_sibling_with(&destination, &mut random)?;
        let path = temporary.path().to_path_buf();
        fs::remove_file(&path)?;
        fs::write(&path, b"replacement to preserve")?;

        let error = temporary
            .discard()
            .expect_err("cleanup must refuse a path owned by another file");

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(path)?, b"replacement to preserve");
        Ok(())
    }

    #[test]
    fn explicit_discard_accepts_an_already_missing_path() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let mut random = SequenceRandom::new([[7; RANDOM_NAME_BYTES]]);
        let temporary = create_unique_sibling_with(&destination, &mut random)?;
        fs::remove_file(temporary.path())?;

        temporary.discard()?;
        Ok(())
    }

    #[test]
    fn production_adapter_replaces_existing_file_exactly() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"irreplaceable original")?;
        let expected = expectation_for(&destination);
        let snapshot = SaveSnapshot::new(
            Revision::new(41),
            destination.clone(),
            expected,
            b"complete replacement\r\nwith exact bytes\n".to_vec(),
        );
        let mut storage = FilesystemStorage;

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(
            outcome,
            SaveOutcome::Committed {
                revision,
                ref warnings,
                ..
            } if revision == Revision::new(41) && warnings.is_empty()
        ));
        assert_eq!(
            fs::read(&destination)?,
            b"complete replacement\r\nwith exact bytes\n"
        );
        assert_no_private_artifacts(directory.path())?;
        Ok(())
    }

    #[test]
    fn production_adapter_installs_absent_file_exclusively() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("new-note.txt");
        let snapshot = SaveSnapshot::new(
            Revision::new(42),
            destination.clone(),
            TargetExpectation::Missing,
            b"new private note".to_vec(),
        );
        let mut storage = FilesystemStorage;

        let outcome = save_snapshot(&mut storage, &snapshot);

        assert!(matches!(
            outcome,
            SaveOutcome::Committed { ref warnings, .. } if warnings.is_empty()
        ));
        assert_eq!(fs::read(&destination)?, b"new private note");
        assert_no_private_artifacts(directory.path())?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&destination)?.permissions().mode() & 0o777,
                0o600
            );
        }
        Ok(())
    }

    #[test]
    fn read_only_destination_is_not_replaced_implicitly() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("read-only.txt");
        fs::write(&destination, b"protected original")?;
        make_read_only(&destination)?;
        let snapshot = SaveSnapshot::new(
            Revision::new(43),
            destination.clone(),
            expectation_for(&destination),
            b"must not replace".to_vec(),
        );
        let mut storage = FilesystemStorage;

        let outcome = save_snapshot(&mut storage, &snapshot);

        make_writable(&destination)?;
        assert!(matches!(
            outcome,
            SaveOutcome::NotCommitted {
                ref error,
                cleanup_error: None,
                ..
            } if error.stage() == SaveStage::ApplyMetadata
        ));
        assert_eq!(fs::read(&destination)?, b"protected original");
        assert_no_private_artifacts(directory.path())?;
        Ok(())
    }

    #[test]
    fn metadata_transfer_refuses_a_stale_destination_observation() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"observed version")?;
        let expected = regular_observation(&destination);
        fs::write(&destination, b"new external version")?;
        let mut temporary = prepared_temporary(&destination, b"my replacement")?;
        let mut storage = FilesystemStorage;

        let error = storage
            .apply_metadata(&mut temporary, &destination, Some(&expected))
            .expect_err("metadata transfer must not use a stale source");

        assert_eq!(error.stage(), SaveStage::ApplyMetadata);
        assert_eq!(fs::read(&destination)?, b"new external version");
        temporary.discard()?;
        Ok(())
    }

    #[test]
    fn sibling_rejects_a_second_snapshot_write() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let mut temporary = create_unique_sibling(&destination)?;
        temporary.write_all(b"first complete snapshot")?;

        let error = temporary
            .write_all(b"second snapshot")
            .expect_err("a private sibling must contain exactly one snapshot");

        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        temporary.discard()?;
        Ok(())
    }

    #[test]
    fn adapter_maps_private_file_failures_to_exact_stages() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"metadata source")?;
        let expected = regular_observation(&destination);
        let mut storage = FilesystemStorage;

        let create_error = storage
            .create_unique_sibling(Path::new("/"))
            .expect_err("a destination without a filename must fail");
        assert_eq!(create_error.stage(), SaveStage::CreateTemporary);

        let mut written = create_unique_sibling(&destination)?;
        storage
            .write_all(&mut written, b"first snapshot")
            .expect("first storage write should succeed");
        let write_error = storage
            .write_all(&mut written, b"second snapshot")
            .expect_err("a second storage write must fail");
        assert_eq!(write_error.stage(), SaveStage::Write);
        written.discard()?;

        let mut closed = create_unique_sibling(&destination)?;
        closed.write_all(b"complete")?;
        closed.close_handle();
        assert_eq!(
            storage
                .flush(&mut closed)
                .expect_err("a closed handle cannot flush")
                .stage(),
            SaveStage::Flush
        );
        assert_eq!(
            storage
                .sync_file(&mut closed)
                .expect_err("a closed handle cannot sync")
                .stage(),
            SaveStage::SyncFile
        );
        assert_eq!(
            storage
                .apply_metadata(&mut closed, &destination, Some(&expected))
                .expect_err("a closed handle cannot receive metadata")
                .stage(),
            SaveStage::ApplyMetadata
        );
        closed.discard()?;

        let stolen = create_unique_sibling(&destination)?;
        let stolen_path = stolen.path().to_path_buf();
        fs::remove_file(&stolen_path)?;
        fs::write(&stolen_path, b"different file")?;
        let cleanup_error = storage
            .discard(stolen)
            .expect_err("storage cleanup must preserve a replacement");
        assert_eq!(cleanup_error.stage(), SaveStage::Cleanup);
        fs::remove_file(stolen_path)?;
        Ok(())
    }

    #[test]
    fn replace_refuses_invalid_private_and_special_states() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let mut storage = FilesystemStorage;

        let unwritten = create_unique_sibling(&destination)?;
        let outcome = storage.replace(unwritten, &destination, TargetState::Missing);
        discard_not_committed(outcome)?;

        let mut stolen = create_unique_sibling(&destination)?;
        stolen.write_all(b"complete")?;
        let stolen_path = stolen.path().to_path_buf();
        fs::remove_file(&stolen_path)?;
        fs::write(&stolen_path, b"replacement")?;
        let outcome = storage.replace(stolen, &destination, TargetState::Missing);
        assert!(matches!(outcome, ReplaceOutcome::NotCommitted { .. }));
        fs::remove_file(&stolen_path)?;

        let mut missing = create_unique_sibling(&destination)?;
        missing.write_all(b"complete")?;
        fs::remove_file(missing.path())?;
        let outcome = storage.replace(missing, &destination, TargetState::Missing);
        assert!(matches!(outcome, ReplaceOutcome::NotCommitted { .. }));

        let mut special = create_unique_sibling(&destination)?;
        special.write_all(b"complete")?;
        let outcome = storage.replace(
            special,
            &destination,
            TargetState::Special(SpecialFileKind::Directory),
        );
        discard_not_committed(outcome)?;
        Ok(())
    }

    #[test]
    fn new_file_failure_reconciliation_covers_all_provable_states() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let platform_error = io::Error::new(io::ErrorKind::PermissionDenied, "injected");

        let intact = prepared_temporary(&destination, b"mine")?;
        let outcome = reconcile_new_failure(intact, &destination, &platform_error);
        discard_not_committed(outcome)?;

        fs::write(&destination, b"external")?;
        let conflicting = prepared_temporary(&destination, b"mine")?;
        let outcome = reconcile_new_failure(conflicting, &destination, &platform_error);
        discard_conflict(outcome)?;
        fs::remove_file(&destination)?;

        let committed = prepared_temporary(&destination, b"mine")?;
        fs::hard_link(committed.path(), &destination)?;
        let outcome = reconcile_new_failure(committed, &destination, &platform_error);
        assert!(matches!(outcome, ReplaceOutcome::Committed(_)));
        assert_eq!(fs::read(&destination)?, b"mine");
        fs::remove_file(&destination)?;

        let replaced = prepared_temporary(&destination, b"mine")?;
        let replaced_path = replaced.path().to_path_buf();
        fs::remove_file(&replaced_path)?;
        fs::write(&replaced_path, b"not ours")?;
        let outcome = reconcile_new_failure(replaced, &destination, &platform_error);
        assert!(matches!(outcome, ReplaceOutcome::CommitStateUnknown { .. }));
        assert_eq!(fs::read(&replaced_path)?, b"not ours");
        fs::remove_file(replaced_path)?;

        let vanished = prepared_temporary(&destination, b"mine")?;
        fs::remove_file(vanished.path())?;
        let outcome = reconcile_new_failure(vanished, &destination, &platform_error);
        assert!(matches!(outcome, ReplaceOutcome::CommitStateUnknown { .. }));

        let invalid_destination = Path::new("invalid\0destination");
        let preserved = prepared_temporary(&destination, b"mine")?;
        let preserved_path = preserved.path().to_path_buf();
        let outcome = reconcile_new_failure(preserved, invalid_destination, &platform_error);
        assert!(matches!(outcome, ReplaceOutcome::CommitStateUnknown { .. }));
        assert_eq!(fs::read(&preserved_path)?, b"mine");
        fs::remove_file(preserved_path)?;
        Ok(())
    }

    #[test]
    fn existing_file_failure_reconciliation_preserves_or_conflicts() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"original")?;
        let expected = regular_observation(&destination);
        let platform_error = io::Error::new(io::ErrorKind::PermissionDenied, "injected");

        let intact = prepared_temporary(&destination, b"mine")?;
        let outcome =
            reconcile_existing_failure(intact, &destination, expected, None, &platform_error);
        discard_not_committed(outcome)?;

        let conflicting = prepared_temporary(&destination, b"mine")?;
        fs::write(&destination, b"external")?;
        let outcome =
            reconcile_existing_failure(conflicting, &destination, expected, None, &platform_error);
        discard_conflict(outcome)?;

        fs::remove_file(&destination)?;
        let committed = prepared_temporary(&destination, b"mine")?;
        fs::hard_link(committed.path(), &destination)?;
        let outcome =
            reconcile_existing_failure(committed, &destination, expected, None, &platform_error);
        assert!(matches!(outcome, ReplaceOutcome::Committed(_)));
        assert_eq!(fs::read(&destination)?, b"mine");
        Ok(())
    }

    #[test]
    fn existing_failure_with_uninspectable_destination_remains_unknown() -> io::Result<()> {
        let directory = tempdir()?;
        let observed_path = directory.path().join("observed.txt");
        let sibling_target = directory.path().join("target.txt");
        fs::write(&observed_path, b"original")?;
        let expected = regular_observation(&observed_path);
        let temporary = prepared_temporary(&sibling_target, b"mine")?;
        let temporary_path = temporary.path().to_path_buf();
        let platform_error = io::Error::new(io::ErrorKind::PermissionDenied, "injected");

        let outcome = reconcile_existing_failure(
            temporary,
            Path::new("invalid\0destination"),
            expected,
            None,
            &platform_error,
        );

        assert!(matches!(outcome, ReplaceOutcome::CommitStateUnknown { .. }));
        assert_eq!(fs::read(&temporary_path)?, b"mine");
        fs::remove_file(temporary_path)?;
        Ok(())
    }

    #[test]
    fn claimed_success_without_matching_destination_remains_unknown() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let temporary = prepared_temporary(&destination, b"mine")?;
        let temporary_path = temporary.path().to_path_buf();

        let outcome = finalize_commit(temporary, &destination, None, Vec::new());

        assert!(matches!(outcome, ReplaceOutcome::CommitStateUnknown { .. }));
        assert_eq!(fs::read(&temporary_path)?, b"mine");
        fs::remove_file(temporary_path)?;
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn documented_windows_partial_replacement_is_completed_and_reconciled() -> io::Result<()> {
        const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1_177;

        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let backup = directory.path().join("backup.txt");
        fs::write(&destination, b"original")?;
        let expected = regular_observation(&destination);
        let mut temporary = prepared_temporary(&destination, b"mine")?;
        temporary.close_handle();
        fs::rename(&destination, &backup)?;
        let platform_error = io::Error::from_raw_os_error(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2);

        let outcome = reconcile_existing_failure(
            temporary,
            &destination,
            expected,
            Some(backup.clone()),
            &platform_error,
        );

        assert!(matches!(outcome, ReplaceOutcome::Committed(_)));
        assert_eq!(fs::read(&destination)?, b"mine");
        assert!(!backup.exists());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn unexplained_windows_partial_state_remains_unknown() -> io::Result<()> {
        const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1_177;

        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"original")?;
        let expected = regular_observation(&destination);
        let temporary = prepared_temporary(&destination, b"mine")?;
        let temporary_path = temporary.path().to_path_buf();
        let platform_error = io::Error::from_raw_os_error(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2);

        let outcome =
            reconcile_existing_failure(temporary, &destination, expected, None, &platform_error);

        assert!(matches!(outcome, ReplaceOutcome::CommitStateUnknown { .. }));
        assert_eq!(fs::read(&temporary_path)?, b"mine");
        fs::remove_file(temporary_path)?;
        Ok(())
    }

    #[test]
    fn committed_cleanup_warnings_preserve_replaced_artifacts() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let temporary = prepared_temporary(&destination, b"mine")?;
        let temporary_path = temporary.path().to_path_buf();
        fs::hard_link(&temporary_path, &destination)?;
        fs::remove_file(&temporary_path)?;
        fs::write(&temporary_path, b"replacement to preserve")?;

        let outcome = finalize_commit(temporary, &destination, None, Vec::new());

        let ReplaceOutcome::Committed(receipt) = outcome else {
            panic!("matching committed destination must reconcile as committed");
        };
        assert_eq!(receipt.cleanup_warnings().len(), 1);
        assert_eq!(fs::read(&temporary_path)?, b"replacement to preserve");
        fs::remove_file(temporary_path)?;
        Ok(())
    }

    #[test]
    fn backup_cleanup_is_identity_safe_and_explicit() -> io::Result<()> {
        let directory = tempdir()?;
        let expected_path = directory.path().join("expected.txt");
        let matching_path = directory.path().join("matching.txt");
        let other_path = directory.path().join("other.txt");
        let directory_path = directory.path().join("not-a-file");
        fs::write(&expected_path, b"old")?;
        fs::hard_link(&expected_path, &matching_path)?;
        fs::write(&other_path, b"old")?;
        fs::create_dir(&directory_path)?;
        let expected = regular_observation(&expected_path);

        remove_verified_backup(&directory.path().join("missing.txt"), expected)
            .expect("an absent backup is already clean");
        assert!(remove_verified_backup(&directory_path, expected).is_err());
        assert!(remove_verified_backup(&other_path, expected).is_err());
        assert!(backup_matches_expected(&matching_path, expected));
        assert!(!backup_matches_expected(&other_path, expected));
        remove_verified_backup(&matching_path, expected)
            .expect("the matching backup should be removed");
        assert!(!matching_path.exists());
        Ok(())
    }

    #[test]
    fn error_redaction_is_exact() {
        let raw = io::Error::from_raw_os_error(5);
        let redacted = redacted_io_error(SaveStage::Replace, "replace", &raw);
        assert!(redacted.message().contains("OS code 5"));
    }

    #[test]
    fn platform_identity_quality_is_part_of_equality() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("identity.txt");
        fs::write(&path, b"identity")?;
        let facts = noter_platform::file_facts(&File::open(path)?)?;
        let actual = facts.identity();
        let matching = match actual.quality() {
            noter_platform::IdentityQuality::Preferred => {
                FileIdentity::new(actual.volume(), actual.file())
            }
            noter_platform::IdentityQuality::Reduced => {
                FileIdentity::reduced(actual.volume(), actual.file())
            }
        };
        let wrong_quality = match actual.quality() {
            noter_platform::IdentityQuality::Preferred => {
                FileIdentity::reduced(actual.volume(), actual.file())
            }
            noter_platform::IdentityQuality::Reduced => {
                FileIdentity::new(actual.volume(), actual.file())
            }
        };

        assert!(platform_identity_matches(actual, matching));
        assert!(!platform_identity_matches(actual, wrong_quality));
        Ok(())
    }

    fn prepared_temporary(destination: &Path, bytes: &[u8]) -> io::Result<TemporaryFile> {
        let mut temporary = create_unique_sibling(destination)?;
        temporary.write_all(bytes)?;
        temporary.flush()?;
        Ok(temporary)
    }

    fn regular_observation(path: &Path) -> FileObservation {
        let TargetState::Regular(observation) =
            inspect_target(path, SaveStage::InspectInitial).expect("test file should inspect")
        else {
            panic!("test path was not a regular file");
        };
        observation
    }

    fn discard_not_committed(outcome: ReplaceOutcome<TemporaryFile>) -> io::Result<()> {
        let ReplaceOutcome::NotCommitted { temporary, .. } = outcome else {
            panic!("expected a proven not-committed outcome");
        };
        temporary.discard()
    }

    fn discard_conflict(outcome: ReplaceOutcome<TemporaryFile>) -> io::Result<()> {
        let ReplaceOutcome::Conflict { temporary, .. } = outcome else {
            panic!("expected a conflict outcome");
        };
        temporary.discard()
    }

    fn expectation_for(path: &Path) -> TargetExpectation {
        match inspect_target(path, SaveStage::InspectInitial)
            .expect("test destination should be observable")
        {
            TargetState::Regular(observation) => TargetExpectation::Existing(observation),
            state => panic!("expected a regular test destination, found {state:?}"),
        }
    }

    fn assert_no_private_artifacts(directory: &Path) -> io::Result<()> {
        for entry in fs::read_dir(directory)? {
            let name = entry?.file_name();
            assert!(
                !name.to_string_lossy().starts_with(".noter-"),
                "private save artifact remained after a resolved outcome"
            );
        }
        Ok(())
    }

    #[cfg(unix)]
    fn make_read_only(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o400);
        fs::set_permissions(path, permissions)
    }

    #[cfg(unix)]
    fn make_writable(path: &Path) -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
    }

    #[cfg(windows)]
    fn make_read_only(path: &Path) -> io::Result<()> {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions)
    }

    #[cfg(windows)]
    #[allow(clippy::permissions_set_readonly_false)]
    fn make_writable(path: &Path) -> io::Result<()> {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(false);
        fs::set_permissions(path, permissions)
    }

    #[cfg(unix)]
    #[test]
    fn sibling_starts_owner_only_on_unix() -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let mut random = SequenceRandom::new([[5; RANDOM_NAME_BYTES]]);
        let temporary = create_unique_sibling_with(&destination, &mut random)?;
        let mode = fs::metadata(temporary.path())?.permissions().mode() & 0o777;

        assert_eq!(mode, 0o600);
        Ok(())
    }
}
