//! Production filesystem storage primitives.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::file_observation::{inspect_target, is_final_link, open_verified_cleanup_candidate};
use super::save::{
    ContentFingerprint, Durability, DurabilityOutcome, FileIdentity, FileObservation,
    IdentityQuality, ReplaceOutcome, ReplaceReceipt, SaveStage, Storage, StorageError, TargetState,
    TemporaryCreationFailure,
};

const RANDOM_NAME_BYTES: usize = 16;
const MAX_CREATE_ATTEMPTS: usize = 16;
const TEMPORARY_PREFIX: &str = ".noter-save-";
const TEMPORARY_SUFFIX: &str = ".tmp";
#[cfg(windows)]
const BACKUP_PREFIX: &str = ".noter-backup-";
#[cfg(windows)]
const BACKUP_SUFFIX: &str = ".bak";

#[derive(Debug)]
struct RetainedCreationArtifact {
    basename: String,
    cause: RetainedCreationCause,
}

#[derive(Debug)]
enum RetainedCreationCause {
    IdentityInspection {
        inspection_kind: io::ErrorKind,
        cleanup_kind: io::ErrorKind,
    },
    SecurityFinalization {
        failure_kind: io::ErrorKind,
        os_code: Option<i32>,
    },
}

impl RetainedCreationArtifact {
    fn primary_error(&self) -> StorageError {
        match self.cause {
            RetainedCreationCause::IdentityInspection {
                inspection_kind, ..
            } => StorageError::new(
                SaveStage::CreateTemporary,
                format!(
                    "inspect the identity of the new private sibling failed with {inspection_kind:?}"
                ),
            ),
            RetainedCreationCause::SecurityFinalization {
                failure_kind,
                os_code,
            } => {
                let os_code = os_code.map_or_else(String::new, |code| format!(", OS code {code}"));
                StorageError::new(
                    SaveStage::CreateTemporary,
                    format!(
                        "finalize private sibling security failed with {failure_kind:?}{os_code}"
                    ),
                )
            }
        }
    }

    fn cleanup_error(&self) -> StorageError {
        let message = match self.cause {
            RetainedCreationCause::IdentityInspection { cleanup_kind, .. } => format!(
                "handle-bound cleanup failed with {cleanup_kind:?}; the newly created private sibling `{}` may remain beside the destination. Noter had not written application bytes, but a same-authority process could have changed it. Inspect it before retrying or removing it.",
                self.basename
            ),
            RetainedCreationCause::SecurityFinalization { .. } => format!(
                "the zero-byte sibling created with the requested private no-inherit ACL may remain as `{}` beside the destination. A same-authority process could have changed it. Inspect it before retrying or removing it.",
                self.basename
            ),
        };
        StorageError::new(SaveStage::Cleanup, message)
    }
}

impl fmt::Display for RetainedCreationArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}; {}",
            self.primary_error().message(),
            self.cleanup_error().message()
        )
    }
}

impl std::error::Error for RetainedCreationArtifact {}

/// An exclusively created sibling owned by one save attempt.
///
/// Dropping this value requests handle-bound cleanup where the platform
/// supports it. Unix preserves the sibling because portable unlink remains
/// pathname-based. The storage protocol uses [`TemporaryFile::discard`] when
/// it must report that conservative retention explicitly.
#[derive(Debug)]
pub struct TemporaryFile {
    file: Option<File>,
    path: PathBuf,
    identity: noter_platform::FileIdentity,
    intended: Option<IntendedContent>,
    #[cfg(unix)]
    required_metadata: Option<noter_platform::RequiredMetadata>,
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

const fn is_supported_regular_entry(is_file: bool, is_final_link: bool) -> bool {
    is_file && !is_final_link
}

fn normalized_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
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
        if !is_supported_regular_entry(metadata.is_file(), is_final_link(&metadata)) {
            return Ok(false);
        }

        let reopened = File::open(&self.path)?;
        Ok(noter_platform::file_facts(&reopened)?.identity() == self.identity)
    }

    /// Removes the private sibling through its original open handle.
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
            Err(error) => match error.kind() {
                io::ErrorKind::NotFound => {
                    self.cleanup_armed = false;
                    return Ok(());
                }
                _ => return Err(error),
            },
        }

        if let Some(file) = self.file.as_ref() {
            noter_platform::delete_open_file(file)?;
            self.file.take();
        } else {
            let file = noter_platform::open_for_cleanup(&self.path)?;
            if noter_platform::file_facts(&file)?.identity() != self.identity {
                self.cleanup_armed = false;
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "temporary path no longer identifies the owned file",
                ));
            }
            noter_platform::delete_open_file(&file)?;
            drop(file);
        }
        self.cleanup_armed = false;
        Ok(())
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

    #[cfg(any(windows, test))]
    fn close_handle(&mut self) {
        self.file.take();
    }

    const fn preserve_artifact(&mut self) {
        self.cleanup_armed = false;
    }
}

impl Drop for TemporaryFile {
    fn drop(&mut self) {
        if self.cleanup_armed
            && matches!(self.path_still_identifies_file(), Ok(true))
            && let Some(file) = self.file.as_ref()
        {
            let _ = noter_platform::delete_open_file(file);
        }
    }
}

/// Creates an unpredictable, exclusive sibling in the destination directory.
///
/// The sibling starts owner-only on Unix and is opened for both reading and
/// writing. macOS suppresses inherited ACLs atomically and removes its bootstrap
/// ACL before this function can return a writable sibling. Existing-file
/// metadata is finalized only after atomic exchange, so staged bytes are never
/// exposed through a widened precommit mode. Its name contains 128 bits from the
/// operating-system random source.
///
/// # Errors
///
/// Returns an error when the destination has no filename, the operating-system
/// random source fails, or an exclusive sibling cannot be created after a
/// bounded number of collisions. On macOS, creation also fails when the private
/// mode or bootstrap ACL cannot be finalized and verified. Such a failure can
/// carry a retained-artifact warning because the random zero-byte sibling was
/// already created.
pub fn create_unique_sibling(destination: &Path) -> io::Result<TemporaryFile> {
    create_unique_sibling_with(destination, &mut OsRandom)
}

fn create_unique_sibling_with(
    destination: &Path,
    random: &mut impl RandomSource,
) -> io::Result<TemporaryFile> {
    create_unique_sibling_with_identity(destination, random, |file| {
        noter_platform::file_facts(file).map(noter_platform::FileFacts::identity)
    })
}

fn create_unique_sibling_with_identity(
    destination: &Path,
    random: &mut impl RandomSource,
    mut identity_for: impl FnMut(&File) -> io::Result<noter_platform::FileIdentity>,
) -> io::Result<TemporaryFile> {
    if destination.file_name().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "save destination must have a filename",
        ));
    }

    let parent = normalized_parent(destination);

    for _ in 0..MAX_CREATE_ATTEMPTS {
        let mut random_bytes = [0_u8; RANDOM_NAME_BYTES];
        random.fill(&mut random_bytes)?;
        let basename = candidate_name(&random_bytes);
        let path = parent.join(&basename);

        match open_exclusive(&path) {
            Ok(file) => {
                let identity = match identity_for(&file) {
                    Ok(identity) => identity,
                    Err(error) => {
                        let cleanup_error = noter_platform::delete_open_file(&file).err();
                        drop(file);
                        let Some(cleanup_error) = cleanup_error else {
                            return Err(error);
                        };
                        let error_kind = error.kind();
                        return Err(io::Error::new(
                            error_kind,
                            RetainedCreationArtifact {
                                basename,
                                cause: RetainedCreationCause::IdentityInspection {
                                    inspection_kind: error_kind,
                                    cleanup_kind: cleanup_error.kind(),
                                },
                            },
                        ));
                    }
                };
                return Ok(TemporaryFile {
                    file: Some(file),
                    path,
                    identity,
                    intended: None,
                    #[cfg(unix)]
                    required_metadata: None,
                    cleanup_armed: true,
                });
            }
            Err(error) => {
                let retained_cause = noter_platform::retained_private_file_creation_cause(&error)
                    .map(|cause| (cause.kind(), cause.raw_os_error()));
                classify_exclusive_creation_failure(basename, error, retained_cause)?;
            }
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create an exclusive random sibling after 16 attempts",
    ))
}

fn classify_exclusive_creation_failure(
    basename: String,
    error: io::Error,
    retained_cause: Option<(io::ErrorKind, Option<i32>)>,
) -> io::Result<()> {
    if let Some((failure_kind, os_code)) = retained_cause {
        return Err(io::Error::new(
            failure_kind,
            RetainedCreationArtifact {
                basename,
                cause: RetainedCreationCause::SecurityFinalization {
                    failure_kind,
                    os_code,
                },
            },
        ));
    }
    if error.kind() == io::ErrorKind::AlreadyExists {
        Ok(())
    } else {
        Err(error)
    }
}

fn open_exclusive(path: &Path) -> io::Result<File> {
    noter_platform::create_private_new_file(path)
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
    ) -> Result<Self::Temporary, TemporaryCreationFailure> {
        create_unique_sibling(destination).map_err(|error| temporary_creation_failure(&error))
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
        #[cfg(not(unix))]
        let _ = temporary;
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

        #[cfg(unix)]
        let required_metadata =
            noter_platform::capture_required_metadata(&source_file, source_facts).map_err(
                |error| {
                    redacted_io_error(
                        SaveStage::ApplyMetadata,
                        "capture required destination metadata",
                        &error,
                    )
                },
            )?;

        if inspect_target(destination, SaveStage::ApplyMetadata)? != TargetState::Regular(expected)
        {
            return Err(StorageError::new(
                SaveStage::ApplyMetadata,
                "destination changed during metadata capture",
            ));
        }
        #[cfg(unix)]
        {
            temporary.required_metadata = Some(required_metadata);
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
        temporary: Self::Temporary,
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
        let artifact = artifact_label(&temporary);
        temporary.discard().map_err(|error| {
            let failure = redacted_io_error(SaveStage::Cleanup, "remove private sibling", &error);
            StorageError::new(
                SaveStage::Cleanup,
                format!(
                    "{}. The uncommitted private sibling may remain as {artifact} beside the destination. Inspect it before retrying, recovering, or removing it.",
                    failure.message()
                ),
            )
        })
    }
}

fn replace_existing_file(
    temporary: TemporaryFile,
    destination: &Path,
    expected: FileObservation,
) -> ReplaceOutcome<TemporaryFile> {
    replace_existing_file_with(
        temporary,
        destination,
        expected,
        noter_platform::replace_existing,
    )
}

fn replace_existing_file_with(
    temporary: TemporaryFile,
    destination: &Path,
    expected: FileObservation,
    replace: impl FnOnce(
        &Path,
        &Path,
        Option<&Path>,
    ) -> io::Result<noter_platform::ReplaceExistingOutcome>,
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

    #[cfg(windows)]
    let mut temporary = temporary;
    #[cfg(not(windows))]
    let temporary = temporary;

    #[cfg(windows)]
    {
        temporary.close_handle();
        match closed_temporary_matches_intended(&temporary) {
            Ok(true) => {}
            Ok(false) => {
                return ReplaceOutcome::NotCommitted {
                    temporary,
                    error: StorageError::new(
                        SaveStage::Replace,
                        "private sibling content or identity changed during the Windows replacement handoff",
                    ),
                };
            }
            Err(error) => {
                return ReplaceOutcome::NotCommitted { temporary, error };
            }
        }
    }

    match replace(temporary.path(), destination, backup.as_deref()) {
        Ok(noter_platform::ReplaceExistingOutcome::Clean) => finalize_commit(
            temporary,
            destination,
            backup.map(|path| (path, expected)),
            Vec::new(),
        ),
        Ok(noter_platform::ReplaceExistingOutcome::DisplacedDestination) => {
            #[cfg(unix)]
            {
                finalize_unix_displaced_destination(temporary, destination, expected, backup)
            }
            #[cfg(not(unix))]
            {
                finalize_unexpected_displaced_destination(temporary, destination, expected, backup)
            }
        }
        Err(error) => reconcile_existing_failure(temporary, destination, expected, backup, &error),
    }
}

#[cfg(windows)]
fn closed_temporary_matches_intended(temporary: &TemporaryFile) -> Result<bool, StorageError> {
    let TargetState::Regular(observation) = inspect_target(temporary.path(), SaveStage::Replace)?
    else {
        return Ok(false);
    };
    temporary
        .committed_observation_matches(observation)
        .map_err(|error| {
            redacted_io_error(
                SaveStage::Replace,
                "validate private sibling after closing its staging handle",
                &error,
            )
        })
}

#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MetadataSourceStatus {
    Matches,
    ObservationChanged,
    FactsChanged,
}

#[cfg(unix)]
const fn metadata_source_status(
    observation_matches: bool,
    facts_match: bool,
) -> MetadataSourceStatus {
    match (observation_matches, facts_match) {
        (false, _) => MetadataSourceStatus::ObservationChanged,
        (true, false) => MetadataSourceStatus::FactsChanged,
        (true, true) => MetadataSourceStatus::Matches,
    }
}

#[cfg(unix)]
fn finalize_unix_displaced_destination(
    temporary: TemporaryFile,
    destination: &Path,
    expected: FileObservation,
    backup: Option<PathBuf>,
) -> ReplaceOutcome<TemporaryFile> {
    let mut cleanup_warnings = Vec::new();
    let mut durability_warnings = Vec::new();
    match open_verified_cleanup_candidate(temporary.path(), SaveStage::ApplyMetadata) {
        Ok(Some((source, observation))) => match noter_platform::file_facts(&source) {
            Ok(facts) => match metadata_source_status(
                replacement_backup_matches(observation, expected),
                post_exchange_source_facts_match(facts, observation, expected),
            ) {
                MetadataSourceStatus::Matches => match temporary.required_metadata.as_ref() {
                    Some(metadata) => match noter_platform::required_metadata_matches_source(
                        metadata, &source, facts,
                    ) {
                        Ok(true) => {
                            if let Err(error) = temporary.file().and_then(|destination_file| {
                                noter_platform::apply_required_metadata(metadata, destination_file)
                            }) {
                                cleanup_warnings.push(redacted_io_error(
                                    SaveStage::ApplyMetadata,
                                    "apply ratified committed destination metadata",
                                    &error,
                                ));
                            }
                        }
                        Ok(false) => cleanup_warnings.push(StorageError::new(
                            SaveStage::ApplyMetadata,
                            "previous destination metadata changed before committed metadata finalization",
                        )),
                        Err(error) => cleanup_warnings.push(redacted_io_error(
                            SaveStage::ApplyMetadata,
                            "compare displaced destination with ratified metadata snapshot",
                            &error,
                        )),
                    },
                    None => cleanup_warnings.push(StorageError::new(
                        SaveStage::ApplyMetadata,
                        "required destination metadata snapshot was unavailable after commit",
                    )),
                },
                MetadataSourceStatus::ObservationChanged => {
                    cleanup_warnings.push(StorageError::new(
                        SaveStage::ApplyMetadata,
                        "previous destination identity or content changed before committed metadata finalization",
                    ));
                }
                MetadataSourceStatus::FactsChanged => cleanup_warnings.push(StorageError::new(
                    SaveStage::ApplyMetadata,
                    "previous destination changed before committed metadata finalization",
                )),
            },
            Err(error) => cleanup_warnings.push(redacted_io_error(
                SaveStage::ApplyMetadata,
                "revalidate committed metadata source",
                &error,
            )),
        },
        Ok(None) => cleanup_warnings.push(StorageError::new(
            SaveStage::ApplyMetadata,
            "previous destination was unavailable during committed metadata finalization",
        )),
        Err(error) => cleanup_warnings.push(error),
    }

    if let Err(error) = temporary.sync_all() {
        durability_warnings.push(redacted_io_error(
            SaveStage::SyncFile,
            "synchronize committed metadata",
            &error,
        ));
    }

    finalize_commit_with_cleanup(
        temporary,
        destination,
        TemporaryCleanup::PreservedDisplacedDestination,
        backup.map(|path| (path, expected)),
        cleanup_warnings,
        durability_warnings,
    )
}

#[cfg(not(unix))]
fn finalize_unexpected_displaced_destination(
    temporary: TemporaryFile,
    destination: &Path,
    expected: FileObservation,
    backup: Option<PathBuf>,
) -> ReplaceOutcome<TemporaryFile> {
    finalize_commit_with_cleanup(
        temporary,
        destination,
        TemporaryCleanup::PreservedDisplacedDestination,
        backup.map(|path| (path, expected)),
        Vec::new(),
        Vec::new(),
    )
}

fn install_new_file(temporary: TemporaryFile, destination: &Path) -> ReplaceOutcome<TemporaryFile> {
    match noter_platform::install_new(temporary.path(), destination) {
        Ok(outcome) => finalize_installed_file(temporary, destination, &outcome, None),
        Err(error) => reconcile_new_failure(temporary, destination, &error),
    }
}

fn finalize_installed_file(
    temporary: TemporaryFile,
    destination: &Path,
    outcome: &noter_platform::InstallNewOutcome,
    backup: Option<(PathBuf, FileObservation)>,
) -> ReplaceOutcome<TemporaryFile> {
    match outcome {
        noter_platform::InstallNewOutcome::Clean => {
            finalize_commit(temporary, destination, backup, Vec::new())
        }
        noter_platform::InstallNewOutcome::CommittedWithRetainedTemporary => {
            finalize_commit_with_cleanup(
                temporary,
                destination,
                TemporaryCleanup::PreservedTemporaryName,
                backup,
                Vec::new(),
                Vec::new(),
            )
        }
    }
}

fn reconcile_new_failure(
    temporary: TemporaryFile,
    destination: &Path,
    platform_error: &io::Error,
) -> ReplaceOutcome<TemporaryFile> {
    let actual = match inspect_target(destination, SaveStage::Reconcile) {
        Ok(actual) => actual,
        Err(error) => return unknown_with_preserved_temporary(temporary, error, None),
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
            None,
        ),
        Err(error) => unknown_with_preserved_temporary(
            temporary,
            redacted_io_error(
                SaveStage::Reconcile,
                "reconcile private sibling after new-file failure",
                &error,
            ),
            None,
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
        Err(error) => {
            return unknown_with_preserved_temporary(temporary, error, backup.as_deref());
        }
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
    if partial_move {
        let state_is_completable = partial_state_is_completable(
            actual == TargetState::Missing,
            matches!(temporary.path_still_identifies_file(), Ok(true)),
            backup
                .as_deref()
                .is_some_and(|path| backup_matches_expected(path, expected)),
        );
        if state_is_completable {
            match noter_platform::install_new(temporary.path(), destination) {
                Ok(outcome) => {
                    return finalize_installed_file(
                        temporary,
                        destination,
                        &outcome,
                        backup.map(|path| (path, expected)),
                    );
                }
                Err(_finish_error) => {
                    return unknown_with_preserved_temporary(
                        temporary,
                        StorageError::new(
                            SaveStage::Reconcile,
                            "documented partial replacement could not be completed safely",
                        ),
                        backup.as_deref(),
                    );
                }
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
            backup.as_deref(),
        ),
        Err(error) => unknown_with_preserved_temporary(
            temporary,
            redacted_io_error(
                SaveStage::Reconcile,
                "reconcile private sibling after replacement failure",
                &error,
            ),
            backup.as_deref(),
        ),
    }
}

const fn partial_state_is_completable(
    destination_is_missing: bool,
    temporary_is_owned: bool,
    backup_matches: bool,
) -> bool {
    destination_is_missing && temporary_is_owned && backup_matches
}

fn finalize_commit(
    temporary: TemporaryFile,
    destination: &Path,
    backup: Option<(PathBuf, FileObservation)>,
    cleanup_warnings: Vec<StorageError>,
) -> ReplaceOutcome<TemporaryFile> {
    finalize_commit_with_cleanup(
        temporary,
        destination,
        TemporaryCleanup::Owned,
        backup,
        cleanup_warnings,
        Vec::new(),
    )
}

#[derive(Clone, Copy)]
enum TemporaryCleanup {
    Owned,
    PreservedTemporaryName,
    PreservedDisplacedDestination,
}

fn finalize_commit_with_cleanup(
    mut temporary: TemporaryFile,
    destination: &Path,
    temporary_cleanup: TemporaryCleanup,
    backup: Option<(PathBuf, FileObservation)>,
    mut cleanup_warnings: Vec<StorageError>,
    durability_warnings: Vec<StorageError>,
) -> ReplaceOutcome<TemporaryFile> {
    let observation = match inspect_target(destination, SaveStage::Reconcile) {
        Ok(TargetState::Regular(observation)) => {
            match temporary.committed_observation_matches(observation) {
                Ok(true) => observation,
                Ok(false) | Err(_) => {
                    return unknown_with_preserved_temporary(
                        temporary,
                        StorageError::new(
                            SaveStage::Reconcile,
                            "commit operation returned success but destination verification differed",
                        ),
                        backup.as_ref().map(|(path, _)| path.as_path()),
                    );
                }
            }
        }
        Ok(_) => {
            return unknown_with_preserved_temporary(
                temporary,
                StorageError::new(
                    SaveStage::Reconcile,
                    "commit operation returned success but destination verification differed",
                ),
                backup.as_ref().map(|(path, _)| path.as_path()),
            );
        }
        Err(error) => {
            return unknown_with_preserved_temporary(
                temporary,
                error,
                backup.as_ref().map(|(path, _)| path.as_path()),
            );
        }
    };

    match temporary_cleanup {
        TemporaryCleanup::Owned => {
            let artifact = artifact_label(&temporary);
            if let Err(error) = temporary.discard() {
                let failure = redacted_io_error(
                    SaveStage::Cleanup,
                    "remove committed temporary name",
                    &error,
                );
                cleanup_warnings.push(StorageError::new(
                    SaveStage::Cleanup,
                    format!(
                        "{}. A sibling entry may remain as {artifact} beside the destination. Inspect it before removing it.",
                        failure.message()
                    ),
                ));
            }
        }
        TemporaryCleanup::PreservedTemporaryName => {
            cleanup_warnings.push(preserved_artifact_warning(
                &temporary,
                "A hard-link sibling containing the committed bytes",
                "portable Unix cleanup cannot delete a verified object by handle",
                "It names the same saved file; remove this sibling when it is no longer needed to restore ordinary single-link saves.",
            ));
            temporary.preserve_artifact();
            drop(temporary);
        }
        TemporaryCleanup::PreservedDisplacedDestination => {
            cleanup_warnings.push(preserved_artifact_warning(
                &temporary,
                "A displaced recovery artifact",
                "portable Unix cleanup cannot delete a verified object by handle",
                "It may contain the prior destination or bytes written by a concurrent actor. Inspect its contents before recovery or removal.",
            ));
            temporary.preserve_artifact();
            drop(temporary);
        }
    }
    if let Some((backup, expected)) = backup
        && let Err(error) = remove_verified_backup(&backup, expected)
    {
        cleanup_warnings.push(error);
    }

    ReplaceOutcome::Committed(ReplaceReceipt::with_warnings(
        observation,
        cleanup_warnings,
        durability_warnings,
    ))
}

fn preserved_artifact_warning(
    temporary: &TemporaryFile,
    description: &str,
    reason: &str,
    guidance: &str,
) -> StorageError {
    let artifact = artifact_label(temporary);
    StorageError::new(
        SaveStage::Cleanup,
        format!(
            "{description} was preserved as {artifact} beside the destination because {reason}. {guidance}"
        ),
    )
}

fn artifact_label(temporary: &TemporaryFile) -> String {
    path_artifact_label(temporary.path(), "a private Noter sibling")
}

fn path_artifact_label(path: &Path, fallback: &str) -> String {
    path.file_name().map_or_else(
        || fallback.to_owned(),
        |name| format!("`{}`", name.to_string_lossy()),
    )
}

fn unknown_with_preserved_temporary(
    mut temporary: TemporaryFile,
    error: StorageError,
    backup: Option<&Path>,
) -> ReplaceOutcome<TemporaryFile> {
    let temporary_label = artifact_label(&temporary);
    let backup_label = backup.map(|path| path_artifact_label(path, "a replacement backup"));
    let candidates = backup_label.map_or_else(
        || format!("the private sibling candidate {temporary_label}"),
        |backup_label| {
            format!(
                "the private sibling candidate {temporary_label} and replacement backup candidate {backup_label}"
            )
        },
    );
    let recovery_artifact = StorageError::new(
        SaveStage::Cleanup,
        format!(
            "Commit recovery requires inspecting {candidates} beside the destination. Either candidate may be absent or may contain prior, intended, or concurrently changed bytes. Inspect the destination and every existing candidate before retrying; remove a candidate only after its recovery value is understood."
        ),
    );
    temporary.preserve_artifact();
    ReplaceOutcome::CommitStateUnknown {
        error,
        recovery_artifact,
    }
}

fn remove_verified_backup(path: &Path, expected: FileObservation) -> Result<(), StorageError> {
    let candidate = open_verified_cleanup_candidate(path, SaveStage::Cleanup)
        .map_err(|error| backup_cleanup_warning(path, error.message()))?;
    let Some((file, actual)) = candidate else {
        return Ok(());
    };
    if !replacement_backup_matches(actual, expected) {
        return Err(backup_cleanup_warning(
            path,
            "replacement backup content or identity changed during cleanup",
        ));
    }
    noter_platform::delete_open_file(&file).map_err(|error| {
        let failure = redacted_io_error(
            SaveStage::Cleanup,
            "delete verified replacement backup by handle",
            &error,
        );
        backup_cleanup_warning(path, failure.message())
    })
}

fn backup_cleanup_warning(path: &Path, detail: &str) -> StorageError {
    let artifact = path_artifact_label(path, "a replacement backup");
    StorageError::new(
        SaveStage::Cleanup,
        format!(
            "{detail}. The replacement backup may remain as {artifact} beside the destination. Inspect it before recovery or removal; remove it only after its recovery value is understood."
        ),
    )
}

fn backup_matches_expected(path: &Path, expected: FileObservation) -> bool {
    matches!(
        inspect_target(path, SaveStage::Reconcile),
        Ok(TargetState::Regular(actual))
            if replacement_backup_matches(actual, expected)
    )
}

fn replacement_backup_matches(actual: FileObservation, expected: FileObservation) -> bool {
    actual.identity() == expected.identity()
        && actual.fingerprint() == expected.fingerprint()
        && actual.length() == expected.length()
}

const fn platform_facts_match(facts: noter_platform::FileFacts, expected: FileObservation) -> bool {
    let change = facts.change_token();
    platform_identity_matches(facts.identity(), expected.identity())
        && change.primary() == expected.change_token().primary()
        && change.secondary() == expected.change_token().secondary()
}

#[cfg(unix)]
const fn post_exchange_source_facts_match(
    facts: noter_platform::FileFacts,
    observed: FileObservation,
    expected: FileObservation,
) -> bool {
    platform_facts_match(facts, observed)
        && facts.link_count() == observed.link_count()
        && observed.link_count() == expected.link_count()
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

// Keep one fallible cross-platform contract: Windows reserves a backup path,
// while other platforms intentionally return no backup path.
#[cfg_attr(
    not(windows),
    allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)
)]
fn replacement_backup_path(destination: &Path) -> io::Result<Option<PathBuf>> {
    #[cfg(windows)]
    {
        let parent = normalized_parent(destination);
        for _ in 0..MAX_CREATE_ATTEMPTS {
            let mut random = [0_u8; RANDOM_NAME_BYTES];
            OsRandom.fill(&mut random)?;
            let candidate = parent.join(artifact_name(BACKUP_PREFIX, &random, BACKUP_SUFFIX));
            match fs::symlink_metadata(&candidate) {
                Ok(_) => {}
                Err(error) => match error.kind() {
                    io::ErrorKind::NotFound => return Ok(Some(candidate)),
                    _ => return Err(error),
                },
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve a random backup name after 16 attempts",
        ))
    }
    #[cfg(not(windows))]
    {
        let _ = destination;
        Ok(None)
    }
}

// The Windows branch must inspect an OS error; the other branch preserves the
// same call site and always rejects the Windows-only partial-move condition.
#[cfg_attr(not(windows), allow(clippy::missing_const_for_fn))]
fn is_documented_partial_replacement(error: &io::Error) -> bool {
    #[cfg(windows)]
    {
        const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1_177;
        error.raw_os_error() == Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2)
    }
    #[cfg(not(windows))]
    {
        let _ = error;
        false
    }
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

fn temporary_creation_failure(error: &io::Error) -> TemporaryCreationFailure {
    let retained_artifact = error
        .get_ref()
        .and_then(|source| source.downcast_ref::<RetainedCreationArtifact>());
    let primary_error = retained_artifact.map_or_else(
        || redacted_io_error(SaveStage::CreateTemporary, "create private sibling", error),
        RetainedCreationArtifact::primary_error,
    );
    let cleanup_error = retained_artifact.map(RetainedCreationArtifact::cleanup_error);
    TemporaryCreationFailure::new(primary_error, cleanup_error)
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
        FileChangeToken, SaveOutcome, SaveSnapshot, SpecialFileKind, TargetExpectation,
        save_snapshot,
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
    fn intended_content_requires_matching_fingerprint_and_length() {
        let expected_fingerprint = ContentFingerprint::from_bytes(b"expected");
        let intended = IntendedContent {
            fingerprint: expected_fingerprint,
            length: 8,
        };
        let identity = FileIdentity::new(1, 2);
        let change_token = FileChangeToken::new(3, 4);
        let matching = FileObservation::new(identity, expected_fingerprint, 8, 1, change_token);
        let wrong_fingerprint = FileObservation::new(
            identity,
            ContentFingerprint::from_bytes(b"different"),
            8,
            1,
            change_token,
        );
        let wrong_length = FileObservation::new(identity, expected_fingerprint, 9, 1, change_token);

        assert!(intended.matches(matching));
        assert!(!intended.matches(wrong_fingerprint));
        assert!(!intended.matches(wrong_length));
    }

    #[test]
    fn regular_entry_policy_has_an_exact_truth_table() {
        assert!(!is_supported_regular_entry(false, false));
        assert!(!is_supported_regular_entry(false, true));
        assert!(!is_supported_regular_entry(true, true));
        assert!(is_supported_regular_entry(true, false));
    }

    #[test]
    fn normalized_parent_handles_relative_and_nested_destinations() {
        assert_eq!(normalized_parent(Path::new("note.txt")), Path::new("."));
        assert_eq!(
            normalized_parent(Path::new("notes/note.txt")),
            Path::new("notes")
        );
    }

    #[test]
    fn operating_system_randomness_overwrites_independent_candidates() -> io::Result<()> {
        let sentinel = [0xa5; RANDOM_NAME_BYTES];
        let mut first = sentinel;
        let mut second = sentinel;

        OsRandom.fill(&mut first)?;
        OsRandom.fill(&mut second)?;

        assert_ne!(first, sentinel);
        assert_ne!(second, sentinel);
        assert_ne!(first, second);
        Ok(())
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

    #[cfg(unix)]
    #[test]
    fn creation_identity_failure_reports_the_retained_sibling() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let random_bytes = [8; RANDOM_NAME_BYTES];
        let retained = directory.path().join(candidate_name(&random_bytes));
        let mut random = SequenceRandom::new([random_bytes]);

        let error = create_unique_sibling_with_identity(&destination, &mut random, |_file| {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "injected identity failure",
            ))
        })
        .expect_err("identity failure must abort creation");
        let failure = temporary_creation_failure(&error);
        let cleanup = failure
            .cleanup_error()
            .expect("unsupported handle deletion must report the retained sibling");

        assert_eq!(failure.error().stage(), SaveStage::CreateTemporary);
        assert_eq!(cleanup.stage(), SaveStage::Cleanup);
        assert!(
            cleanup
                .message()
                .contains(retained.file_name().unwrap().to_string_lossy().as_ref())
        );
        assert!(
            cleanup
                .message()
                .contains("had not written application bytes")
        );
        assert!(
            cleanup
                .message()
                .contains("Inspect it before retrying or removing it")
        );
        assert!(retained.exists());
        assert_eq!(fs::metadata(&retained)?.len(), 0);
        fs::remove_file(retained)?;
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

        let discard_result = temporary.discard();
        #[cfg(windows)]
        {
            discard_result?;
            assert!(!path.exists());
        }
        #[cfg(unix)]
        {
            assert_eq!(
                discard_result
                    .expect_err("portable Unix cleanup must preserve the sibling")
                    .kind(),
                io::ErrorKind::Unsupported
            );
            assert_eq!(fs::read(&path)?, b"complete bytes");
            fs::remove_file(path)?;
        }
        Ok(())
    }

    #[test]
    fn committed_observation_requires_content_and_identity_independently() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let other = directory.path().join("other.txt");
        let temporary = prepared_temporary(&destination, b"mine")?;
        fs::hard_link(temporary.path(), &destination)?;
        fs::write(&other, b"mine")?;
        let matching = regular_observation(&destination);
        let other_identity = regular_observation(&other);
        let wrong_content = FileObservation::new(
            matching.identity(),
            ContentFingerprint::from_bytes(b"not mine"),
            8,
            matching.link_count(),
            matching.change_token(),
        );

        assert!(temporary.committed_observation_matches(matching)?);
        assert!(!temporary.committed_observation_matches(other_identity)?);
        assert!(!temporary.committed_observation_matches(wrong_content)?);
        cleanup_fixture(temporary)?;
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

        #[cfg(windows)]
        assert!(!path.exists());
        #[cfg(unix)]
        {
            assert!(path.exists());
            fs::remove_file(path)?;
        }
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

    #[cfg(windows)]
    #[test]
    fn explicit_discard_reopens_a_closed_owned_sibling_for_handle_deletion() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let mut temporary = prepared_temporary(&destination, b"complete bytes")?;
        let path = temporary.path().to_path_buf();
        temporary.close_handle();

        temporary.discard()?;

        assert!(!path.exists());
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

        #[cfg(windows)]
        assert!(
            matches!(
                &outcome,
                SaveOutcome::Committed {
                    revision,
                    warnings,
                    ..
                } if *revision == Revision::new(41) && warnings.is_empty()
            ),
            "unexpected existing-file save outcome: {outcome:?}"
        );
        #[cfg(unix)]
        assert!(matches!(
            &outcome,
            SaveOutcome::Committed {
                revision,
                warnings,
                ..
            } if *revision == Revision::new(41)
                && warnings.cleanup().len() == 1
                && warnings.cleanup()[0].message().contains("preserved")
        ));
        assert_eq!(
            fs::read(&destination)?,
            b"complete replacement\r\nwith exact bytes\n"
        );
        #[cfg(windows)]
        assert_no_private_artifacts(directory.path())?;
        #[cfg(unix)]
        remove_private_artifacts(directory.path(), b"irreplaceable original")?;
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
        #[cfg(windows)]
        assert!(matches!(
            &outcome,
            SaveOutcome::NotCommitted {
                error,
                cleanup_error: None,
                ..
            } if error.stage() == SaveStage::ApplyMetadata
        ));
        #[cfg(unix)]
        assert!(matches!(
            &outcome,
            SaveOutcome::NotCommitted {
                error,
                cleanup_error: Some(cleanup_error),
                ..
            } if error.stage() == SaveStage::ApplyMetadata
                && cleanup_error.stage() == SaveStage::Cleanup
        ));
        assert_eq!(fs::read(&destination)?, b"protected original");
        #[cfg(windows)]
        assert_no_private_artifacts(directory.path())?;
        #[cfg(unix)]
        remove_private_artifacts(directory.path(), b"must not replace")?;
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
        cleanup_fixture(temporary)?;
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
        cleanup_fixture(temporary)?;
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
        assert_eq!(create_error.error().stage(), SaveStage::CreateTemporary);

        let mut written = create_unique_sibling(&destination)?;
        storage
            .write_all(&mut written, b"first snapshot")
            .expect("first storage write should succeed");
        let write_error = storage
            .write_all(&mut written, b"second snapshot")
            .expect_err("a second storage write must fail");
        assert_eq!(write_error.stage(), SaveStage::Write);
        cleanup_fixture(written)?;

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
        storage
            .apply_metadata(&mut closed, &destination, Some(&expected))
            .expect("precommit metadata validation must not widen the sibling");
        let closed_path = closed.path().to_path_buf();
        drop(closed);
        fs::remove_file(closed_path)?;

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
    fn partial_completion_requires_every_reconciled_fact() {
        assert!(partial_state_is_completable(true, true, true));
        assert!(!partial_state_is_completable(false, true, true));
        assert!(!partial_state_is_completable(true, false, true));
        assert!(!partial_state_is_completable(true, true, false));
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

    #[test]
    fn claimed_success_with_different_regular_destination_remains_unknown() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"someone else's complete file")?;
        let temporary = prepared_temporary(&destination, b"mine")?;
        let temporary_path = temporary.path().to_path_buf();

        let outcome = finalize_commit(temporary, &destination, None, Vec::new());

        let ReplaceOutcome::CommitStateUnknown {
            recovery_artifact, ..
        } = outcome
        else {
            panic!("a mismatched destination must remain indeterminate");
        };
        assert!(
            recovery_artifact.message().contains(
                temporary_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(recovery_artifact.message().contains("before retrying"));
        assert_eq!(fs::read(&destination)?, b"someone else's complete file");
        assert_eq!(fs::read(&temporary_path)?, b"mine");
        fs::remove_file(temporary_path)?;
        Ok(())
    }

    #[test]
    fn unknown_commit_names_every_known_recovery_candidate_neutrally() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let backup = directory.path().join(".noter-backup-recovery.bak");
        fs::write(&destination, b"concurrent destination")?;
        fs::write(&backup, b"previous destination")?;
        let expected = regular_observation(&backup);
        let mut temporary = prepared_temporary(&destination, b"intended bytes")?;
        temporary.close_handle();
        let temporary_path = temporary.path().to_path_buf();
        fs::remove_file(&temporary_path)?;

        let outcome = finalize_commit(
            temporary,
            &destination,
            Some((backup.clone(), expected)),
            Vec::new(),
        );
        let ReplaceOutcome::CommitStateUnknown {
            recovery_artifact, ..
        } = outcome
        else {
            panic!("a mismatched postcommit destination must remain indeterminate");
        };
        let warning = recovery_artifact.message();
        assert!(
            warning.contains(
                temporary_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(warning.contains(backup.file_name().unwrap().to_string_lossy().as_ref()));
        assert!(warning.contains("Either candidate may be absent"));
        assert!(warning.contains("before retrying"));
        assert_eq!(fs::read(&backup)?, b"previous destination");
        fs::remove_file(backup)?;
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_handoff_rejects_changed_staging_bytes_before_replace() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"previous revision")?;
        let expected = regular_observation(&destination);
        let mut temporary = prepared_temporary(&destination, b"intended revision")?;
        temporary.close_handle();
        fs::write(temporary.path(), b"changed during handoff")?;

        let outcome = replace_existing_file(temporary, &destination, expected);
        let ReplaceOutcome::NotCommitted { temporary, error } = outcome else {
            panic!("changed staging bytes must not reach ReplaceFileW");
        };
        assert!(error.message().contains("Windows replacement handoff"));
        assert_eq!(fs::read(&destination)?, b"previous revision");
        FilesystemStorage
            .discard(temporary)
            .expect("the changed owned sibling should remain removable");
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_handoff_mutation_after_validation_is_detected_postcommit() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"previous revision")?;
        let expected = regular_observation(&destination);
        let temporary = prepared_temporary(&destination, b"intended revision")?;

        let outcome = replace_existing_file_with(
            temporary,
            &destination,
            expected,
            |temporary_path, destination_path, backup_path| {
                fs::write(temporary_path, b"same-authority handoff mutation")?;
                noter_platform::replace_existing(temporary_path, destination_path, backup_path)
            },
        );
        let ReplaceOutcome::CommitStateUnknown {
            error,
            recovery_artifact,
        } = outcome
        else {
            panic!("postvalidation handoff mutation must remain indeterminate");
        };

        assert_eq!(error.stage(), SaveStage::Reconcile);
        assert!(
            error
                .message()
                .contains("destination verification differed")
        );
        assert!(recovery_artifact.message().contains("replacement backup"));
        assert_eq!(fs::read(&destination)?, b"same-authority handoff mutation");
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
        let warning = receipt.cleanup_warnings()[0].message();
        assert!(
            warning.contains(
                temporary_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(warning.contains("Inspect it before removing it"));
        assert_eq!(fs::read(&temporary_path)?, b"replacement to preserve");
        fs::remove_file(temporary_path)?;
        Ok(())
    }

    #[test]
    fn failed_precommit_cleanup_names_the_artifact_and_safe_actions() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let temporary = prepared_temporary(&destination, b"uncommitted bytes")?;
        let temporary_path = temporary.path().to_path_buf();
        fs::remove_file(&temporary_path)?;
        fs::write(&temporary_path, b"external bytes")?;

        let error = FilesystemStorage
            .discard(temporary)
            .expect_err("a rebound private-sibling path must be preserved");

        assert!(
            error.message().contains(
                temporary_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(error.message().contains("Inspect it before retrying"));
        assert_eq!(fs::read(&temporary_path)?, b"external bytes");
        fs::remove_file(temporary_path)?;
        Ok(())
    }

    #[test]
    fn retained_hard_link_warning_names_the_artifact_and_recovery_action() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let temporary = prepared_temporary(&destination, b"committed bytes")?;
        let temporary_path = temporary.path().to_path_buf();
        fs::hard_link(&temporary_path, &destination)?;

        let outcome = finalize_installed_file(
            temporary,
            &destination,
            &noter_platform::InstallNewOutcome::CommittedWithRetainedTemporary,
            None,
        );
        let ReplaceOutcome::Committed(receipt) = outcome else {
            panic!("the retained hard-link destination must remain committed");
        };

        assert_eq!(receipt.cleanup_warnings().len(), 1);
        let warning = receipt.cleanup_warnings()[0].message();
        assert!(
            warning.contains(
                temporary_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(warning.contains("restore ordinary single-link saves"));
        assert_eq!(fs::read(&destination)?, b"committed bytes");
        assert_eq!(fs::read(&temporary_path)?, b"committed bytes");
        Ok(())
    }

    #[test]
    fn displaced_destination_is_preserved_after_exact_commit() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"old bytes")?;
        let mut temporary = prepared_temporary(&destination, b"new bytes")?;
        temporary.close_handle();
        let temporary_path = temporary.path().to_path_buf();
        let swap_path = directory.path().join("swap.txt");
        fs::rename(&destination, &swap_path)?;
        fs::rename(&temporary_path, &destination)?;
        fs::rename(&swap_path, &temporary_path)?;

        let outcome = finalize_commit_with_cleanup(
            temporary,
            &destination,
            TemporaryCleanup::PreservedDisplacedDestination,
            None,
            Vec::new(),
            Vec::new(),
        );

        let ReplaceOutcome::Committed(receipt) = outcome else {
            panic!("a verified exchange should reconcile as committed");
        };
        assert_eq!(receipt.cleanup_warnings().len(), 1);
        assert!(
            receipt.cleanup_warnings()[0].message().contains(
                temporary_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(
            receipt.cleanup_warnings()[0]
                .message()
                .contains("Inspect its contents")
        );
        assert_eq!(fs::read(&destination)?, b"new bytes");
        assert_eq!(fs::read(&temporary_path)?, b"old bytes");
        fs::remove_file(temporary_path)?;
        Ok(())
    }

    #[test]
    fn changed_displaced_destination_is_preserved_with_a_warning() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"old bytes")?;
        let mut temporary = prepared_temporary(&destination, b"new bytes")?;
        temporary.close_handle();
        let temporary_path = temporary.path().to_path_buf();
        let swap_path = directory.path().join("swap.txt");
        fs::rename(&destination, &swap_path)?;
        fs::rename(&temporary_path, &destination)?;
        fs::rename(&swap_path, &temporary_path)?;
        fs::write(&temporary_path, b"external bytes")?;

        let outcome = finalize_commit_with_cleanup(
            temporary,
            &destination,
            TemporaryCleanup::PreservedDisplacedDestination,
            None,
            Vec::new(),
            Vec::new(),
        );

        let ReplaceOutcome::Committed(receipt) = outcome else {
            panic!("the new destination should remain a verified commit");
        };
        assert_eq!(receipt.cleanup_warnings().len(), 1);
        assert!(
            receipt.cleanup_warnings()[0].message().contains(
                temporary_path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(
            receipt.cleanup_warnings()[0]
                .message()
                .contains("prior destination or bytes written by a concurrent actor")
        );
        assert!(
            !receipt.cleanup_warnings()[0]
                .message()
                .contains("previous destination revision")
        );
        assert_eq!(fs::read(&destination)?, b"new bytes");
        assert_eq!(fs::read(&temporary_path)?, b"external bytes");
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
        let directory_error = remove_verified_backup(&directory_path, expected)
            .expect_err("a directory cannot be a replacement backup");
        assert!(directory_error.message().contains("not-a-file"));
        assert!(
            directory_error
                .message()
                .contains("before recovery or removal")
        );
        let mismatch_error = remove_verified_backup(&other_path, expected)
            .expect_err("a different file must be retained");
        assert!(mismatch_error.message().contains("other.txt"));
        assert!(
            mismatch_error
                .message()
                .contains("before recovery or removal")
        );
        assert!(backup_matches_expected(&matching_path, expected));
        assert!(!backup_matches_expected(&other_path, expected));
        #[cfg(windows)]
        {
            remove_verified_backup(&matching_path, expected)
                .expect("Windows should remove the verified backup by handle");
            assert!(!matching_path.exists());
        }
        #[cfg(unix)]
        {
            let cleanup_error = remove_verified_backup(&matching_path, expected)
                .expect_err("Unix must retain a backup it cannot delete by handle");
            assert!(cleanup_error.message().contains("matching.txt"));
            assert!(cleanup_error.message().contains("may remain"));
            assert_eq!(fs::read(&matching_path)?, b"old");
            fs::remove_file(&matching_path)?;
        }
        Ok(())
    }

    #[test]
    fn backup_cleanup_preserves_changed_content_with_the_same_identity() -> io::Result<()> {
        let directory = tempdir()?;
        let source_path = directory.path().join("source.txt");
        let backup_path = directory.path().join("backup.txt");
        fs::write(&source_path, b"expected")?;
        fs::hard_link(&source_path, &backup_path)?;
        let expected = regular_observation(&source_path);
        fs::write(&backup_path, b"changed after revalidation")?;

        let error = remove_verified_backup(&backup_path, expected)
            .expect_err("changed backup content must be preserved for recovery");

        assert_eq!(error.stage(), SaveStage::Cleanup);
        assert!(error.message().contains("backup.txt"));
        assert!(error.message().contains("may remain"));
        assert!(error.message().contains("before recovery or removal"));
        assert_eq!(fs::read(&backup_path)?, b"changed after revalidation");
        Ok(())
    }

    #[test]
    fn error_redaction_is_exact() {
        let raw = io::Error::from_raw_os_error(5);
        let redacted = redacted_io_error(SaveStage::Replace, "replace", &raw);
        assert!(redacted.message().contains("OS code 5"));
    }

    #[test]
    fn retained_creation_artifact_message_is_exact_and_actionable() {
        let artifact = RetainedCreationArtifact {
            basename: ".noter-save-00112233445566778899aabbccddeeff.tmp".to_owned(),
            cause: RetainedCreationCause::IdentityInspection {
                inspection_kind: io::ErrorKind::PermissionDenied,
                cleanup_kind: io::ErrorKind::Unsupported,
            },
        };

        assert_eq!(
            artifact.to_string(),
            "inspect the identity of the new private sibling failed with PermissionDenied; handle-bound cleanup failed with Unsupported; the newly created private sibling `.noter-save-00112233445566778899aabbccddeeff.tmp` may remain beside the destination. Noter had not written application bytes, but a same-authority process could have changed it. Inspect it before retrying or removing it."
        );
        assert_eq!(artifact.primary_error().stage(), SaveStage::CreateTemporary);
        assert_eq!(artifact.cleanup_error().stage(), SaveStage::Cleanup);
    }

    #[test]
    fn retained_security_finalization_message_is_exact_and_actionable() {
        let artifact = RetainedCreationArtifact {
            basename: ".noter-save-00112233445566778899aabbccddeeff.tmp".to_owned(),
            cause: RetainedCreationCause::SecurityFinalization {
                failure_kind: io::ErrorKind::InvalidData,
                os_code: Some(22),
            },
        };

        assert_eq!(
            artifact.to_string(),
            "finalize private sibling security failed with InvalidData, OS code 22; the zero-byte sibling created with the requested private no-inherit ACL may remain as `.noter-save-00112233445566778899aabbccddeeff.tmp` beside the destination. A same-authority process could have changed it. Inspect it before retrying or removing it."
        );
        assert_eq!(
            artifact.primary_error().message(),
            "finalize private sibling security failed with InvalidData, OS code 22"
        );
        assert!(!artifact.cleanup_error().message().contains("OS code"));
        assert!(!artifact.cleanup_error().message().contains('\\'));
        assert_eq!(artifact.cleanup_error().stage(), SaveStage::Cleanup);
    }

    #[test]
    fn exclusive_creation_failure_classifier_covers_every_disposition() {
        let basename = ".noter-save-00112233445566778899aabbccddeeff.tmp";
        let retained = classify_exclusive_creation_failure(
            basename.to_owned(),
            io::Error::other("outer marker"),
            Some((io::ErrorKind::InvalidData, Some(22))),
        )
        .expect_err("security finalization must preserve the random sibling");
        assert_eq!(retained.kind(), io::ErrorKind::InvalidData);
        assert!(retained.to_string().contains(basename));
        assert!(retained.to_string().contains("OS code 22"));

        assert!(
            classify_exclusive_creation_failure(
                basename.to_owned(),
                io::Error::new(io::ErrorKind::AlreadyExists, "collision"),
                None,
            )
            .is_ok(),
            "an exclusive-name collision must retry"
        );

        let ordinary = classify_exclusive_creation_failure(
            basename.to_owned(),
            io::Error::from_raw_os_error(5),
            None,
        )
        .expect_err("ordinary creation failures must remain terminal");
        assert_eq!(ordinary.raw_os_error(), Some(5));
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

    #[test]
    fn platform_fact_match_checks_identity_and_each_change_component() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("facts.txt");
        fs::write(&path, b"facts")?;
        let file = File::open(&path)?;
        let facts = noter_platform::file_facts(&file)?;
        let expected = regular_observation(&path);
        let identity = expected.identity();
        let wrong_identity = match identity.quality() {
            IdentityQuality::Preferred => {
                FileIdentity::new(identity.volume(), identity.file().wrapping_add(1))
            }
            IdentityQuality::Reduced => {
                FileIdentity::reduced(identity.volume(), identity.file().wrapping_add(1))
            }
        };
        let token = expected.change_token();
        let with_identity = |identity, change_token| {
            FileObservation::new(
                identity,
                expected.fingerprint(),
                expected.length(),
                expected.link_count(),
                change_token,
            )
        };

        assert!(platform_facts_match(facts, expected));
        assert!(!platform_facts_match(
            facts,
            with_identity(wrong_identity, token)
        ));
        assert!(!platform_facts_match(
            facts,
            with_identity(
                identity,
                FileChangeToken::new(token.primary().wrapping_add(1), token.secondary())
            )
        ));
        assert!(!platform_facts_match(
            facts,
            with_identity(
                identity,
                FileChangeToken::new(token.primary(), token.secondary().wrapping_add(1))
            )
        ));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn post_exchange_source_facts_rebase_ctime_without_weakening_stable_facts() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("facts.txt");
        fs::write(&path, b"facts")?;
        let facts = noter_platform::file_facts(&File::open(&path)?)?;
        let observed = regular_observation(&path);
        let identity = observed.identity();
        let wrong_identity = match identity.quality() {
            IdentityQuality::Preferred => {
                FileIdentity::new(identity.volume(), identity.file().wrapping_add(1))
            }
            IdentityQuality::Reduced => {
                FileIdentity::reduced(identity.volume(), identity.file().wrapping_add(1))
            }
        };
        let with_facts = |identity, link_count, change_token| {
            FileObservation::new(
                identity,
                observed.fingerprint(),
                observed.length(),
                link_count,
                change_token,
            )
        };
        let pre_exchange = with_facts(
            identity,
            observed.link_count(),
            FileChangeToken::new(i64::MAX, i64::MIN),
        );

        assert!(post_exchange_source_facts_match(
            facts,
            observed,
            pre_exchange
        ));
        assert!(!post_exchange_source_facts_match(
            facts,
            with_facts(
                identity,
                observed.link_count(),
                FileChangeToken::new(i64::MIN, i64::MAX)
            ),
            pre_exchange
        ));
        assert!(!post_exchange_source_facts_match(
            facts,
            with_facts(
                wrong_identity,
                observed.link_count(),
                observed.change_token()
            ),
            pre_exchange
        ));
        assert!(!post_exchange_source_facts_match(
            facts,
            with_facts(
                identity,
                observed.link_count().wrapping_add(1),
                observed.change_token()
            ),
            pre_exchange
        ));
        assert!(!post_exchange_source_facts_match(
            facts,
            observed,
            with_facts(
                identity,
                observed.link_count().wrapping_add(1),
                pre_exchange.change_token()
            )
        ));
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_backup_path_and_partial_error_classification_are_exact() -> io::Result<()> {
        const ERROR_UNABLE_TO_MOVE_REPLACEMENT_2: i32 = 1_177;

        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        let backup = replacement_backup_path(&destination)?
            .expect("Windows existing-file replacement requires a backup path");
        let name = backup
            .file_name()
            .expect("backup path should have a private filename")
            .to_string_lossy();

        assert_eq!(backup.parent(), Some(directory.path()));
        assert!(name.starts_with(BACKUP_PREFIX));
        assert!(name.ends_with(BACKUP_SUFFIX));
        assert!(!backup.exists());
        assert!(is_documented_partial_replacement(
            &io::Error::from_raw_os_error(ERROR_UNABLE_TO_MOVE_REPLACEMENT_2)
        ));
        assert!(!is_documented_partial_replacement(
            &io::Error::from_raw_os_error(5)
        ));
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
        cleanup_fixture(temporary)
    }

    fn discard_conflict(outcome: ReplaceOutcome<TemporaryFile>) -> io::Result<()> {
        let ReplaceOutcome::Conflict { temporary, .. } = outcome else {
            panic!("expected a conflict outcome");
        };
        cleanup_fixture(temporary)
    }

    fn cleanup_fixture(temporary: TemporaryFile) -> io::Result<()> {
        #[cfg(unix)]
        let path = temporary.path().to_path_buf();
        match temporary.discard() {
            Ok(()) => Ok(()),
            #[cfg(unix)]
            Err(error) if error.kind() == io::ErrorKind::Unsupported => fs::remove_file(path),
            Err(error) => Err(error),
        }
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
    fn remove_private_artifacts(directory: &Path, expected: &[u8]) -> io::Result<()> {
        let mut count = 0;
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            let is_private = path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".noter-"));
            if is_private {
                assert_eq!(fs::read(&path)?, expected);
                fs::remove_file(path)?;
                count += 1;
            }
        }
        assert_eq!(
            count, 1,
            "exactly one conservative recovery artifact expected"
        );
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

    #[cfg(unix)]
    #[test]
    fn metadata_source_status_has_an_exact_truth_table() {
        assert_eq!(
            metadata_source_status(false, false),
            MetadataSourceStatus::ObservationChanged
        );
        assert_eq!(
            metadata_source_status(false, true),
            MetadataSourceStatus::ObservationChanged
        );
        assert_eq!(
            metadata_source_status(true, false),
            MetadataSourceStatus::FactsChanged
        );
        assert_eq!(
            metadata_source_status(true, true),
            MetadataSourceStatus::Matches
        );
    }

    #[cfg(unix)]
    #[test]
    fn unix_metadata_is_finalized_only_after_private_content_commits() -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"previous revision")?;
        let mut permissions = fs::metadata(&destination)?.permissions();
        permissions.set_mode(0o640);
        fs::set_permissions(&destination, permissions)?;
        let expected = regular_observation(&destination);
        let mut temporary = prepared_temporary(&destination, b"committed revision")?;
        let mut storage = FilesystemStorage;

        storage
            .apply_metadata(&mut temporary, &destination, Some(&expected))
            .expect("precommit metadata validation should succeed");
        assert_eq!(
            fs::metadata(temporary.path())?.permissions().mode() & 0o777,
            0o600
        );
        storage
            .sync_file(&mut temporary)
            .map_err(|error| io::Error::other(error.message().to_owned()))?;

        let outcome = storage.replace(temporary, &destination, TargetState::Regular(expected));
        let ReplaceOutcome::Committed(receipt) = outcome else {
            panic!("Unix exchange should commit after metadata validation");
        };

        assert_eq!(fs::read(&destination)?, b"committed revision");
        assert_eq!(
            fs::metadata(&destination)?.permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(receipt.cleanup_warnings().len(), 1);
        remove_private_artifacts(directory.path(), b"previous revision")?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unix_final_window_metadata_change_is_warned_and_not_restored() -> io::Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"previous revision")?;
        let mut original_permissions = fs::metadata(&destination)?.permissions();
        original_permissions.set_mode(0o640);
        fs::set_permissions(&destination, original_permissions)?;
        let expected = regular_observation(&destination);
        let mut temporary = prepared_temporary(&destination, b"committed revision")?;
        let mut storage = FilesystemStorage;
        storage
            .apply_metadata(&mut temporary, &destination, Some(&expected))
            .expect("precommit metadata capture should succeed");
        storage
            .sync_file(&mut temporary)
            .map_err(|error| io::Error::other(error.message().to_owned()))?;

        let mut changed_permissions = fs::metadata(&destination)?.permissions();
        changed_permissions.set_mode(0o600);
        fs::set_permissions(&destination, changed_permissions)?;
        assert_eq!(
            noter_platform::replace_existing(temporary.path(), &destination, None)?,
            noter_platform::ReplaceExistingOutcome::DisplacedDestination
        );

        let outcome = finalize_unix_displaced_destination(temporary, &destination, expected, None);
        let ReplaceOutcome::Committed(receipt) = outcome else {
            panic!("the content exchange remains committed after a metadata race");
        };

        assert_eq!(fs::read(&destination)?, b"committed revision");
        assert_eq!(
            fs::metadata(&destination)?.permissions().mode() & 0o777,
            0o600,
            "stale permissive metadata must not be restored after a final-window change"
        );
        assert!(receipt.cleanup_warnings().iter().any(|warning| {
            warning.stage() == SaveStage::ApplyMetadata
                && warning.message().contains("metadata changed")
        }));
        remove_private_artifacts(directory.path(), b"previous revision")?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unix_postcommit_file_sync_failure_is_a_durability_warning() -> io::Result<()> {
        let directory = tempdir()?;
        let destination = directory.path().join("note.txt");
        fs::write(&destination, b"previous revision")?;
        let expected = regular_observation(&destination);
        let mut temporary = prepared_temporary(&destination, b"committed revision")?;
        let mut storage = FilesystemStorage;
        storage
            .apply_metadata(&mut temporary, &destination, Some(&expected))
            .expect("precommit metadata capture should succeed");

        assert_eq!(
            noter_platform::replace_existing(temporary.path(), &destination, None)?,
            noter_platform::ReplaceExistingOutcome::DisplacedDestination
        );
        temporary.close_handle();

        let outcome = finalize_unix_displaced_destination(temporary, &destination, expected, None);
        let ReplaceOutcome::Committed(receipt) = outcome else {
            panic!("the exchanged destination remains committed after a barrier failure");
        };

        assert_eq!(fs::read(&destination)?, b"committed revision");
        assert_eq!(receipt.durability_warnings().len(), 1);
        assert_eq!(
            receipt.durability_warnings()[0].stage(),
            SaveStage::SyncFile
        );
        remove_private_artifacts(directory.path(), b"previous revision")?;
        Ok(())
    }
}
