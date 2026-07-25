//! Stable destination classification, identity, and content observations.

use std::fs::{self, File, Metadata};
use std::io::{self, Seek};
use std::path::Path;
use std::time::SystemTime;

use noter_platform::{FileFacts, IdentityQuality as PlatformIdentityQuality};

use super::save::{
    ContentFingerprint, FileIdentity, FileObservation, SaveStage, SpecialFileKind, StorageError,
    TargetState,
};

const MAX_STABILITY_ATTEMPTS: usize = 3;

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

fn inspect_once(path: &Path) -> Result<TargetState, AttemptError> {
    let initial = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(TargetState::Missing),
        Err(error) => return Err(AttemptError::io("read final-entry metadata", error)),
    };

    if let Some(state) = non_regular_state(&initial) {
        return Ok(state);
    }

    let mut file = File::open(path).map_err(|error| changed_or_io("open regular file", error))?;
    let before = handle_stamp(&file)?;
    if !before.is_regular {
        return Err(AttemptError::Changed);
    }

    let fingerprint = ContentFingerprint::from_reader(&mut file)
        .map_err(|error| AttemptError::io("read complete file content", error))?;
    let bytes_read = file
        .stream_position()
        .map_err(|error| AttemptError::io("verify content length", error))?;
    let after = handle_stamp(&file)?;

    if before != after || bytes_read != after.length {
        return Err(AttemptError::Changed);
    }

    let final_entry = fs::symlink_metadata(path)
        .map_err(|error| changed_or_io("revalidate final-entry metadata", error))?;
    if let Some(state) = non_regular_state(&final_entry) {
        return Ok(state);
    }

    let reopened = File::open(path).map_err(|error| changed_or_io("reopen regular file", error))?;
    let reopened_stamp = handle_stamp(&reopened)?;
    let closing_entry = fs::symlink_metadata(path)
        .map_err(|error| changed_or_io("close final-entry race window", error))?;

    if non_regular_state(&closing_entry).is_some()
        || !reopened_stamp.is_regular
        || reopened_stamp.facts != after.facts
        || reopened_stamp.length != after.length
        || reopened_stamp.modified != after.modified
    {
        return Err(AttemptError::Changed);
    }

    Ok(TargetState::Regular(FileObservation::new(
        map_identity(after.facts),
        fingerprint,
        after.length,
        after.facts.link_count(),
    )))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct HandleStamp {
    facts: FileFacts,
    length: u64,
    modified: Option<SystemTime>,
    is_regular: bool,
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
        is_regular: metadata.is_file(),
    })
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

#[cfg(windows)]
fn is_final_link(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;

    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_final_link(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
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

        let state = inspect_target(&link, SaveStage::InspectInitial)
            .expect("a final symlink should be classified without following it");

        assert_eq!(state, TargetState::Special(SpecialFileKind::SymbolicLink));
        Ok(())
    }
}
