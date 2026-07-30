//! Stable destination classification, identity, and content observations.

use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::Path;
use std::time::SystemTime;

use noter_platform::{FileFacts, IdentityQuality as PlatformIdentityQuality};

use super::limits::MAX_DOCUMENT_BYTES;
use super::save::{
    ContentFingerprint, FileChangeToken, FileIdentity, FileObservation, SaveStage, SpecialFileKind,
    StorageError, TargetState,
};

const MAX_STABILITY_ATTEMPTS: usize = 3;
const MAX_SUPPORTED_FILE_BYTES: u64 = MAX_DOCUMENT_BYTES as u64;
const FILE_TOO_LARGE_MESSAGE: &str = "file exceeds the supported 64 MiB document limit";

/// Inspects a final path without following a final link or accepting a torn read.
///
/// A regular file is read from an open handle. Identity, length, hard-link
/// count, and modification time are checked before and after hashing, then a
/// second handle verifies that the pathname still names the same object.
/// Transient pathname changes are retried a bounded number of times.
///
/// # Errors
///
/// Returns a stage-tagged, path-redacted error when the destination cannot be
/// inspected safely or does not become stable after three attempts.
pub fn inspect_target(path: &Path, boundary: SaveStage) -> Result<TargetState, StorageError> {
    for _ in 0..MAX_STABILITY_ATTEMPTS {
        match inspect_once(path) {
            Ok(observed) => return Ok(observed),
            Err(AttemptError::Changed) => {}
            Err(AttemptError::TooLarge) => {
                return Err(StorageError::new(boundary, FILE_TOO_LARGE_MESSAGE));
            }
            Err(AttemptError::Io { operation, error }) => {
                return Err(redacted_error(boundary, operation, &error));
            }
        }
    }

    Err(StorageError::new(
        boundary,
        "destination changed repeatedly during bounded inspection",
    ))
}

/// Reads a regular file and captures the exact observation tied to those bytes.
///
/// The bytes, identity, length, change token, hard-link count, and final path
/// entry are checked as one bounded stable-handle operation. Missing files,
/// links, directories, and other special entries are rejected.
///
/// # Errors
///
/// Returns a stage-tagged, path-redacted error if the file cannot be read as a
/// stable regular file after three attempts.
pub fn read_regular_file(
    path: &Path,
    boundary: SaveStage,
) -> Result<(Vec<u8>, FileObservation), StorageError> {
    for _ in 0..MAX_STABILITY_ATTEMPTS {
        match read_once(path) {
            Ok(result) => return Ok(result),
            Err(AttemptError::Changed) => {}
            Err(AttemptError::TooLarge) => {
                return Err(StorageError::new(boundary, FILE_TOO_LARGE_MESSAGE));
            }
            Err(AttemptError::Io { operation, error }) => {
                return Err(redacted_error(boundary, operation, &error));
            }
        }
    }

    Err(StorageError::new(
        boundary,
        "file changed repeatedly during bounded read",
    ))
}

/// Opens a cleanup candidate and returns the stable observation for that exact
/// open object. A missing path is benign; special entries are rejected.
///
/// Keeping the handle live lets Windows delete the observed object by handle
/// even if the pathname is rebound after observation. Unix callers receive the
/// same stable evidence but must preserve the object because portable deletion
/// remains pathname-based.
pub(crate) fn open_verified_cleanup_candidate(
    path: &Path,
    boundary: SaveStage,
) -> Result<Option<(File, FileObservation)>, StorageError> {
    for _ in 0..MAX_STABILITY_ATTEMPTS {
        let initial = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(redacted_error(
                    boundary,
                    "read cleanup final-entry metadata",
                    &error,
                ));
            }
        };
        if non_regular_state(&initial).is_some() {
            return Err(StorageError::new(
                boundary,
                "cleanup candidate is not a supported regular file",
            ));
        }

        let file = match noter_platform::open_for_cleanup(path) {
            Ok(file) => file,
            Err(error) => {
                return Err(redacted_error(boundary, "open cleanup candidate", &error));
            }
        };
        match observe_regular_handle(path, file) {
            Ok(observed) => return Ok(Some(observed)),
            Err(AttemptError::Changed) => {}
            Err(AttemptError::TooLarge) => {
                return Err(StorageError::new(boundary, FILE_TOO_LARGE_MESSAGE));
            }
            Err(AttemptError::Io { operation, error }) => {
                return Err(redacted_error(boundary, operation, &error));
            }
        }
    }

    Err(StorageError::new(
        boundary,
        "cleanup candidate changed repeatedly during bounded inspection",
    ))
}

fn read_once(path: &Path) -> Result<(Vec<u8>, FileObservation), AttemptError> {
    let initial = fs::symlink_metadata(path)
        .map_err(|error| changed_or_io("read final-entry metadata", error))?;
    if non_regular_state(&initial).is_some() {
        return Err(AttemptError::io(
            "validate regular file",
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "final path entry is not a supported regular file",
            ),
        ));
    }

    let (mut file, before) = open_regular_file(path, "open regular file without following links")?;

    let bytes = read_bytes_bounded(&mut file, before.length, MAX_SUPPORTED_FILE_BYTES)?;
    let length = u64::try_from(bytes.len()).map_err(|_| {
        AttemptError::io(
            "verify content length",
            io::Error::new(
                io::ErrorKind::InvalidData,
                "file length exceeds the supported in-memory size",
            ),
        )
    })?;
    let after = handle_stamp(&file)?;
    if read_window_changed(before, after, length) {
        return Err(AttemptError::Changed);
    }

    let final_entry = fs::symlink_metadata(path)
        .map_err(|error| changed_or_io("revalidate final-entry metadata", error))?;
    if non_regular_state(&final_entry).is_some() {
        return Err(AttemptError::Changed);
    }
    let (_, reopened_stamp) =
        open_regular_file(path, "reopen regular file without following links")?;
    let closing_entry = fs::symlink_metadata(path)
        .map_err(|error| changed_or_io("close final-entry race window", error))?;
    if reopened_path_changed(
        non_regular_state(&closing_entry).is_some(),
        reopened_stamp,
        after,
    ) {
        return Err(AttemptError::Changed);
    }

    let observation = FileObservation::new(
        map_identity(after.facts),
        ContentFingerprint::from_bytes(&bytes),
        after.length,
        after.facts.link_count(),
        map_change_token(after.facts),
    );
    Ok((bytes, observation))
}

fn inspect_once(path: &Path) -> Result<TargetState, AttemptError> {
    let initial = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(TargetState::Missing),
        Err(error) => return Err(AttemptError::io("read final-entry metadata", error)),
    };

    if let Some(state) = non_regular_state(&initial) {
        return Ok(state);
    }

    let (file, _) = open_regular_file(path, "open regular file without following links")?;
    let (_, observation) = observe_regular_handle(path, file)?;

    Ok(TargetState::Regular(observation))
}

fn observe_regular_handle(
    path: &Path,
    mut file: File,
) -> Result<(File, FileObservation), AttemptError> {
    let before = handle_stamp(&file)?;
    if !before.is_regular {
        return Err(AttemptError::Changed);
    }

    let (fingerprint, bytes_read) =
        fingerprint_bounded(&mut file, before.length, MAX_SUPPORTED_FILE_BYTES)?;
    let after = handle_stamp(&file)?;

    if read_window_changed(before, after, bytes_read) {
        return Err(AttemptError::Changed);
    }

    let final_entry = fs::symlink_metadata(path)
        .map_err(|error| changed_or_io("revalidate final-entry metadata", error))?;
    if non_regular_state(&final_entry).is_some() {
        return Err(AttemptError::Changed);
    }

    let (_, reopened_stamp) =
        open_regular_file(path, "reopen regular file without following links")?;
    let closing_entry = fs::symlink_metadata(path)
        .map_err(|error| changed_or_io("close final-entry race window", error))?;

    if reopened_path_changed(
        non_regular_state(&closing_entry).is_some(),
        reopened_stamp,
        after,
    ) {
        return Err(AttemptError::Changed);
    }

    Ok((
        file,
        FileObservation::new(
            map_identity(after.facts),
            fingerprint,
            after.length,
            after.facts.link_count(),
            map_change_token(after.facts),
        ),
    ))
}

fn read_bytes_bounded(
    file: &mut File,
    announced_length: u64,
    maximum: u64,
) -> Result<Vec<u8>, AttemptError> {
    let read_limit = bounded_read_limit(announced_length, maximum)?;
    let capacity = usize::try_from(announced_length).map_err(|_| {
        AttemptError::io(
            "allocate bounded file buffer",
            io::Error::new(
                io::ErrorKind::InvalidData,
                "announced file length does not fit the address space",
            ),
        )
    })?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| AttemptError::io("read bounded file content", error))?;
    let bytes_read = u64::try_from(bytes.len()).map_err(|_| {
        AttemptError::io(
            "verify bounded file length",
            io::Error::new(
                io::ErrorKind::InvalidData,
                "read length does not fit the supported file size",
            ),
        )
    })?;
    if bytes_read > maximum {
        return Err(AttemptError::TooLarge);
    }
    Ok(bytes)
}

fn fingerprint_bounded(
    file: &mut File,
    announced_length: u64,
    maximum: u64,
) -> Result<(ContentFingerprint, u64), AttemptError> {
    let read_limit = bounded_read_limit(announced_length, maximum)?;
    let mut bounded = file.take(read_limit);
    let fingerprint = ContentFingerprint::from_reader(&mut bounded)
        .map_err(|error| AttemptError::io("read bounded file content", error))?;
    let bytes_read = read_limit - bounded.limit();
    if bytes_read > maximum {
        return Err(AttemptError::TooLarge);
    }
    Ok((fingerprint, bytes_read))
}

fn bounded_read_limit(announced_length: u64, maximum: u64) -> Result<u64, AttemptError> {
    if announced_length > maximum {
        return Err(AttemptError::TooLarge);
    }
    maximum.checked_add(1).ok_or_else(|| {
        AttemptError::io(
            "calculate bounded read limit",
            io::Error::new(io::ErrorKind::InvalidInput, "file limit cannot be bounded"),
        )
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct HandleStamp {
    facts: FileFacts,
    length: u64,
    modified: Option<SystemTime>,
    is_regular: bool,
}

fn read_window_changed(before: HandleStamp, after: HandleStamp, observed_length: u64) -> bool {
    before != after || observed_length != after.length
}

fn reopened_path_changed(
    final_entry_is_special: bool,
    reopened: HandleStamp,
    observed: HandleStamp,
) -> bool {
    final_entry_is_special || reopened != observed
}

fn open_regular_file(
    path: &Path,
    operation: &'static str,
) -> Result<(File, HandleStamp), AttemptError> {
    let file = noter_platform::open_existing_no_follow(path)
        .map_err(|error| changed_or_io(operation, error))?;
    let stamp = handle_stamp(&file)?;
    if !stamp.is_regular {
        return Err(AttemptError::Changed);
    }
    Ok((file, stamp))
}

fn handle_stamp(file: &File) -> Result<HandleStamp, AttemptError> {
    let metadata = file
        .metadata()
        .map_err(|error| AttemptError::io("read open-handle metadata", error))?;
    let facts = noter_platform::file_facts(file)
        .map_err(|error| AttemptError::io("read open-handle identity", error))?;

    Ok(HandleStamp {
        facts,
        length: metadata.len(),
        modified: metadata.modified().ok(),
        is_regular: handle_metadata_is_regular(metadata.is_file(), is_final_link(&metadata)),
    })
}

const fn handle_metadata_is_regular(is_file: bool, is_final_link: bool) -> bool {
    is_file && !is_final_link
}

const fn map_identity(facts: FileFacts) -> FileIdentity {
    let identity = facts.identity();
    match identity.quality() {
        PlatformIdentityQuality::Preferred => FileIdentity::new(identity.volume(), identity.file()),
        PlatformIdentityQuality::Reduced => {
            FileIdentity::reduced(identity.volume(), identity.file())
        }
    }
}

const fn map_change_token(facts: FileFacts) -> FileChangeToken {
    let token = facts.change_token();
    FileChangeToken::new(token.primary(), token.secondary())
}

fn non_regular_state(metadata: &Metadata) -> Option<TargetState> {
    if is_final_link(metadata) {
        Some(TargetState::Special(SpecialFileKind::SymbolicLink))
    } else if metadata.is_dir() {
        Some(TargetState::Special(SpecialFileKind::Directory))
    } else if metadata.is_file() {
        None
    } else {
        Some(TargetState::Special(SpecialFileKind::Other))
    }
}

pub(crate) fn is_final_link(metadata: &Metadata) -> bool {
    let is_symlink = metadata.file_type().is_symlink();
    #[cfg(windows)]
    let is_reparse_point = {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    };
    #[cfg(not(windows))]
    let is_reparse_point = false;

    link_attributes_indicate_link(is_symlink, is_reparse_point)
}

const fn link_attributes_indicate_link(is_symlink: bool, is_reparse_point: bool) -> bool {
    is_symlink || is_reparse_point
}

fn changed_or_io(operation: &'static str, error: io::Error) -> AttemptError {
    if error.kind() == io::ErrorKind::NotFound {
        AttemptError::Changed
    } else {
        AttemptError::io(operation, error)
    }
}

fn redacted_error(stage: SaveStage, operation: &str, error: &io::Error) -> StorageError {
    StorageError::new(stage, format!("{operation} failed with {:?}", error.kind()))
}

#[derive(Debug)]
enum AttemptError {
    Changed,
    TooLarge,
    Io {
        operation: &'static str,
        error: io::Error,
    },
}

impl AttemptError {
    const fn io(operation: &'static str, error: io::Error) -> Self {
        Self::Io { operation, error }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{File, hard_link};
    use std::io::{self, Write};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn read_window_change_checks_each_observed_dimension() -> io::Result<()> {
        let directory = tempdir()?;
        let first_path = directory.path().join("first.txt");
        let second_path = directory.path().join("second.txt");
        fs::write(&first_path, b"same length")?;
        fs::write(&second_path, b"same length")?;
        let first = handle_stamp(&File::open(first_path)?)
            .expect("first test handle facts should be readable");
        let second = handle_stamp(&File::open(second_path)?)
            .expect("second test handle facts should be readable");

        assert!(!read_window_changed(first, first, first.length));
        assert!(read_window_changed(first, second, first.length));

        let mut changed_length = first;
        changed_length.length += 1;
        assert!(read_window_changed(first, changed_length, first.length));

        let mut changed_modified = first;
        changed_modified.modified = if first.modified.is_some() {
            None
        } else {
            Some(SystemTime::UNIX_EPOCH)
        };
        assert!(read_window_changed(first, changed_modified, first.length));

        let mut changed_kind = first;
        changed_kind.is_regular = !first.is_regular;
        assert!(read_window_changed(first, changed_kind, first.length));
        assert!(read_window_changed(first, first, first.length + 1));
        Ok(())
    }

    #[test]
    fn reopened_path_change_checks_entry_and_handle_independently() -> io::Result<()> {
        let directory = tempdir()?;
        let first_path = directory.path().join("first.txt");
        let second_path = directory.path().join("second.txt");
        fs::write(&first_path, b"first")?;
        fs::write(&second_path, b"second")?;
        let first = handle_stamp(&File::open(first_path)?)
            .expect("first test handle facts should be readable");
        let second = handle_stamp(&File::open(second_path)?)
            .expect("second test handle facts should be readable");

        assert!(!reopened_path_changed(false, first, first));
        assert!(reopened_path_changed(true, first, first));
        assert!(reopened_path_changed(false, second, first));
        Ok(())
    }

    #[test]
    fn link_attribute_classification_has_an_exact_truth_table() {
        assert!(!link_attributes_indicate_link(false, false));
        assert!(link_attributes_indicate_link(true, false));
        assert!(link_attributes_indicate_link(false, true));
        assert!(link_attributes_indicate_link(true, true));
    }

    #[test]
    fn open_handle_classification_has_an_exact_truth_table() {
        assert!(!handle_metadata_is_regular(false, false));
        assert!(!handle_metadata_is_regular(false, true));
        assert!(handle_metadata_is_regular(true, false));
        assert!(!handle_metadata_is_regular(true, true));
    }

    #[test]
    fn missing_path_is_classified_without_error() -> io::Result<()> {
        let directory = tempdir()?;
        let state = inspect_target(
            &directory.path().join("missing.txt"),
            SaveStage::InspectInitial,
        )
        .expect("an absent path is a valid destination state");

        assert_eq!(state, TargetState::Missing);
        Ok(())
    }

    #[test]
    fn cleanup_candidate_distinguishes_missing_from_invalid_paths() -> io::Result<()> {
        let directory = tempdir()?;
        let missing = directory.path().join("missing.txt");
        assert!(
            open_verified_cleanup_candidate(&missing, SaveStage::Cleanup)
                .expect("a missing cleanup candidate is already clean")
                .is_none()
        );

        let error = open_verified_cleanup_candidate(
            Path::new("invalid\0cleanup-candidate"),
            SaveStage::Cleanup,
        )
        .expect_err("an invalid path must not be classified as missing");
        assert_eq!(error.stage(), SaveStage::Cleanup);
        assert!(
            error
                .message()
                .contains("read cleanup final-entry metadata")
        );
        assert!(!error.message().contains("invalid\0cleanup-candidate"));
        Ok(())
    }

    #[test]
    fn directory_is_refused_as_a_special_final_entry() -> io::Result<()> {
        let directory = tempdir()?;
        let state = inspect_target(directory.path(), SaveStage::InspectInitial)
            .expect("a directory should be classified without following it");

        assert_eq!(state, TargetState::Special(SpecialFileKind::Directory));
        Ok(())
    }

    #[test]
    fn regular_file_observation_contains_exact_content_and_identity() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        let content = b"exact content\r\nwith another line\n";
        File::create(&path)?.write_all(content)?;

        let state = inspect_target(&path, SaveStage::InspectInitial)
            .expect("a stable regular file should be observed");
        let TargetState::Regular(observation) = state else {
            panic!("regular file was classified as {state:?}");
        };

        assert_eq!(
            observation.fingerprint(),
            ContentFingerprint::from_bytes(content)
        );
        assert_eq!(observation.length(), content.len() as u64);
        assert!(observation.link_count() >= 1);
        Ok(())
    }

    #[test]
    fn stable_read_returns_exact_bytes_and_matching_observation() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        let expected_bytes = b"exact bytes\r\nincluding unicode: \xF0\x9F\xA6\x80\n";
        File::create(&path)?.write_all(expected_bytes)?;

        let (bytes, loaded) = read_regular_file(&path, SaveStage::InspectInitial)
            .expect("a stable regular file should load");
        let TargetState::Regular(inspected) =
            inspect_target(&path, SaveStage::InspectInitial).expect("the same file should inspect")
        else {
            panic!("loaded file was no longer regular");
        };

        assert_eq!(bytes, expected_bytes);
        assert_eq!(loaded, inspected);
        Ok(())
    }

    #[test]
    fn bounded_read_detects_growth_past_its_announced_length() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        fs::write(&path, b"12345")?;
        let mut file = File::open(path)?;

        let result = read_bytes_bounded(&mut file, 4, 4);

        assert!(matches!(result, Err(AttemptError::TooLarge)));
        Ok(())
    }

    #[test]
    fn bounded_read_accepts_the_exact_limit_and_rejects_a_larger_announcement() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        fs::write(&path, b"1234")?;

        let exact = read_bytes_bounded(&mut File::open(&path)?, 4, 4)
            .expect("the supported limit is inclusive");
        let oversized = read_bytes_bounded(&mut File::open(path)?, 5, 4);

        assert_eq!(exact, b"1234");
        assert!(matches!(oversized, Err(AttemptError::TooLarge)));
        Ok(())
    }

    #[test]
    fn bounded_fingerprint_detects_growth_past_its_announced_length() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        fs::write(&path, b"12345")?;
        let mut file = File::open(path)?;

        let result = fingerprint_bounded(&mut file, 4, 4);

        assert!(matches!(result, Err(AttemptError::TooLarge)));
        Ok(())
    }

    #[test]
    fn bounded_fingerprint_accepts_the_exact_limit_and_rejects_a_larger_announcement()
    -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        fs::write(&path, b"1234")?;

        let exact = fingerprint_bounded(&mut File::open(&path)?, 4, 4)
            .expect("the supported limit is inclusive");
        let oversized = fingerprint_bounded(&mut File::open(path)?, 5, 4);

        assert_eq!(exact, (ContentFingerprint::from_bytes(b"1234"), 4));
        assert!(matches!(oversized, Err(AttemptError::TooLarge)));
        Ok(())
    }

    #[test]
    fn maximum_size_constant_is_exact_and_limit_arithmetic_cannot_wrap() {
        assert_eq!(MAX_SUPPORTED_FILE_BYTES, 67_108_864);
        assert!(matches!(
            bounded_read_limit(0, u64::MAX),
            Err(AttemptError::Io {
                operation: "calculate bounded read limit",
                ..
            })
        ));
    }

    #[test]
    fn oversized_sparse_file_is_rejected_without_reading_its_content() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("oversized.txt");
        let file = File::create(&path)?;
        file.set_len(MAX_SUPPORTED_FILE_BYTES + 1)?;
        drop(file);

        let load_error = read_regular_file(&path, SaveStage::InspectInitial)
            .expect_err("oversized document loads must fail before allocation");
        let inspect_error = inspect_target(&path, SaveStage::Revalidate)
            .expect_err("oversized save targets must fail before hashing");

        assert_eq!(load_error.stage(), SaveStage::InspectInitial);
        assert_eq!(load_error.message(), FILE_TOO_LARGE_MESSAGE);
        assert_eq!(inspect_error.stage(), SaveStage::Revalidate);
        assert_eq!(inspect_error.message(), FILE_TOO_LARGE_MESSAGE);
        Ok(())
    }

    #[test]
    fn stable_read_rejects_missing_and_special_entries_with_exact_stages() -> io::Result<()> {
        let directory = tempdir()?;
        let missing = directory.path().join("missing.txt");

        let missing_error = read_regular_file(&missing, SaveStage::InspectInitial)
            .expect_err("a missing path cannot be loaded as a regular file");
        let directory_error = read_regular_file(directory.path(), SaveStage::Revalidate)
            .expect_err("a directory cannot be loaded as a regular file");

        assert_eq!(missing_error.stage(), SaveStage::InspectInitial);
        assert!(missing_error.message().contains("changed repeatedly"));
        assert_eq!(directory_error.stage(), SaveStage::Revalidate);
        assert!(directory_error.message().contains("InvalidInput"));
        Ok(())
    }

    #[test]
    fn hard_links_share_identity_and_report_multiple_names() -> io::Result<()> {
        let directory = tempdir()?;
        let first_path = directory.path().join("first.txt");
        let second_path = directory.path().join("second.txt");
        File::create(&first_path)?.write_all(b"linked")?;
        hard_link(&first_path, &second_path)?;

        let TargetState::Regular(first) = inspect_target(&first_path, SaveStage::InspectInitial)
            .expect("first hard link should be observable")
        else {
            panic!("first hard link was not regular");
        };
        let TargetState::Regular(second) = inspect_target(&second_path, SaveStage::InspectInitial)
            .expect("second hard link should be observable")
        else {
            panic!("second hard link was not regular");
        };

        assert_eq!(first.identity(), second.identity());
        assert!(first.link_count() >= 2);
        assert!(second.link_count() >= 2);
        Ok(())
    }

    #[test]
    fn same_content_in_separate_files_has_distinct_identity() -> io::Result<()> {
        let directory = tempdir()?;
        let first_path = directory.path().join("first.txt");
        let second_path = directory.path().join("second.txt");
        File::create(&first_path)?.write_all(b"same content")?;
        File::create(&second_path)?.write_all(b"same content")?;

        let TargetState::Regular(first) = inspect_target(&first_path, SaveStage::InspectInitial)
            .expect("first file should be observable")
        else {
            panic!("first file was not regular");
        };
        let TargetState::Regular(second) = inspect_target(&second_path, SaveStage::InspectInitial)
            .expect("second file should be observable")
        else {
            panic!("second file was not regular");
        };

        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_ne!(first.identity(), second.identity());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unix_link_count_change_invalidates_the_observation() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        let linked_path = directory.path().join("linked-note.txt");
        File::create(&path)?.write_all(b"unchanged content")?;
        let TargetState::Regular(before) = inspect_target(&path, SaveStage::InspectInitial)
            .expect("initial file should be observable")
        else {
            panic!("initial file was not regular");
        };

        hard_link(&path, linked_path)?;

        let TargetState::Regular(after) = inspect_target(&path, SaveStage::Revalidate)
            .expect("changed file should remain observable")
        else {
            panic!("changed file was not regular");
        };

        assert_eq!(before.identity(), after.identity());
        assert_eq!(before.fingerprint(), after.fingerprint());
        assert!(after.link_count() > before.link_count());
        assert_ne!(before, after);
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_attribute_change_invalidates_the_observation() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        File::create(&path)?.write_all(b"unchanged content")?;
        let TargetState::Regular(before) = inspect_target(&path, SaveStage::InspectInitial)
            .expect("initial file should be observable")
        else {
            panic!("initial file was not regular");
        };

        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions)?;

        let TargetState::Regular(after) = inspect_target(&path, SaveStage::Revalidate)
            .expect("changed file should remain observable")
        else {
            panic!("changed file was not regular");
        };

        assert_eq!(before.identity(), after.identity());
        assert_eq!(before.fingerprint(), after.fingerprint());
        assert_ne!(before.change_token(), after.change_token());
        assert_ne!(before, after);
        Ok(())
    }

    #[test]
    fn inspection_errors_retain_stage_without_disclosing_path() {
        let private_marker = "private-note-name";
        let invalid_path = Path::new("private-note-name\0.txt");

        let error = inspect_target(invalid_path, SaveStage::Revalidate)
            .expect_err("an invalid operating-system path must fail safely");

        assert_eq!(error.stage(), SaveStage::Revalidate);
        assert!(!error.message().contains(private_marker));
        assert!(error.message().contains("InvalidInput"));
    }

    #[cfg(unix)]
    #[test]
    fn final_symlink_is_classified_without_following_it() -> io::Result<()> {
        use std::os::unix::fs::symlink;

        let directory = tempdir()?;
        let target = directory.path().join("target.txt");
        let link = directory.path().join("link.txt");
        File::create(&target)?.write_all(b"secret target")?;
        symlink(&target, &link)?;

        assert!(
            open_regular_file(&link, "test no-follow open").is_err(),
            "the observation handle must not expose the link target as a regular file"
        );

        let state = inspect_target(&link, SaveStage::InspectInitial)
            .expect("a final symlink should be classified without following it");

        assert_eq!(state, TargetState::Special(SpecialFileKind::SymbolicLink));
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn final_symlink_is_classified_without_following_it() -> io::Result<()> {
        use std::os::windows::fs::symlink_file;

        const ERROR_PRIVILEGE_NOT_HELD: i32 = 1_314;

        let directory = tempdir()?;
        let target = directory.path().join("target.txt");
        let link = directory.path().join("link.txt");
        File::create(&target)?.write_all(b"secret target")?;
        if let Err(error) = symlink_file(&target, &link) {
            if error.raw_os_error() == Some(ERROR_PRIVILEGE_NOT_HELD) {
                return Ok(());
            }
            return Err(error);
        }

        assert!(
            open_regular_file(&link, "test no-follow open").is_err(),
            "the observation handle must not expose the reparse target as a regular file"
        );

        let state = inspect_target(&link, SaveStage::InspectInitial)
            .expect("a final symlink should be classified without following it");

        assert_eq!(state, TargetState::Special(SpecialFileKind::SymbolicLink));
        Ok(())
    }
}
