//! Production filesystem storage primitives.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::file_observation::is_final_link;

const RANDOM_NAME_BYTES: usize = 16;
const MAX_CREATE_ATTEMPTS: usize = 16;
const TEMPORARY_PREFIX: &str = ".noter-save-";
const TEMPORARY_SUFFIX: &str = ".tmp";

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
    cleanup_armed: bool,
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
    /// delivered.
    pub fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.file_mut()?.write_all(bytes)
    }

    /// Flushes user-space buffers for the sibling handle.
    ///
    /// # Errors
    ///
    /// Returns the underlying flush error.
    pub fn flush(&mut self) -> io::Result<()> {
        self.file_mut()?.flush()
    }

    /// Synchronizes sibling data and metadata through the standard file barrier.
    ///
    /// # Errors
    ///
    /// Returns the underlying synchronization error.
    pub fn sync_all(&self) -> io::Result<()> {
        self.file()?.sync_all()
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
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut name = String::with_capacity(
        TEMPORARY_PREFIX.len() + (RANDOM_NAME_BYTES * 2) + TEMPORARY_SUFFIX.len(),
    );
    name.push_str(TEMPORARY_PREFIX);
    for byte in random {
        name.push(char::from(HEX[usize::from(byte >> 4)]));
        name.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    name.push_str(TEMPORARY_SUFFIX);
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::fs;
    use std::io::{self, Read};

    use tempfile::tempdir;

    use super::*;

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
