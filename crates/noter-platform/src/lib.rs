//! Narrow, audited operating-system primitives for Noter's storage adapter.
//!
//! The product crate forbids unsafe code. Calls that cannot yet be expressed
//! through stable standard-library APIs live here behind safe, tested types.

use std::fs::File;
use std::io;

/// Strength of the platform-provided file identifier.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum IdentityQuality {
    /// The preferred native identifier, including the Windows 128-bit file ID.
    Preferred,
    /// A reduced identifier used only when the preferred platform query is unavailable.
    Reduced,
}

/// Stable identity components for one open file handle.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileIdentity {
    quality: IdentityQuality,
    volume: u128,
    file: u128,
}

impl FileIdentity {
    const fn new(quality: IdentityQuality, volume: u128, file: u128) -> Self {
        Self {
            quality,
            volume,
            file,
        }
    }

    /// Returns the identifier strength.
    #[must_use]
    pub const fn quality(self) -> IdentityQuality {
        self.quality
    }

    /// Returns the volume or device component.
    #[must_use]
    pub const fn volume(self) -> u128 {
        self.volume
    }

    /// Returns the file or inode component.
    #[must_use]
    pub const fn file(self) -> u128 {
        self.file
    }
}

/// Identity and hard-link facts obtained from one open file handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FileFacts {
    identity: FileIdentity,
    link_count: u64,
}

impl FileFacts {
    const fn new(identity: FileIdentity, link_count: u64) -> Self {
        Self {
            identity,
            link_count,
        }
    }

    /// Returns the native file identity.
    #[must_use]
    pub const fn identity(self) -> FileIdentity {
        self.identity
    }

    /// Returns the number of hard links reported for the open file.
    #[must_use]
    pub const fn link_count(self) -> u64 {
        self.link_count
    }
}

/// Obtains identity and hard-link facts from an already open file.
///
/// Keeping the file open while the facts are used prevents normal pathname
/// replacement races from changing the object represented by the handle.
///
/// # Errors
///
/// Returns an operating-system error if the handle metadata cannot be queried.
pub fn file_facts(file: &File) -> io::Result<FileFacts> {
    imp::file_facts(file)
}

#[cfg(unix)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::os::unix::fs::MetadataExt;

    use super::{FileFacts, FileIdentity, IdentityQuality};

    pub fn file_facts(file: &File) -> io::Result<FileFacts> {
        let metadata = file.metadata()?;
        Ok(FileFacts::new(
            FileIdentity::new(
                IdentityQuality::Preferred,
                u128::from(metadata.dev()),
                u128::from(metadata.ino()),
            ),
            metadata.nlink(),
        ))
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::mem::size_of;
    use std::os::windows::io::AsRawHandle;

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_ID_INFO, FileIdInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx,
    };

    use super::{FileFacts, FileIdentity, IdentityQuality};

    pub fn file_facts(file: &File) -> io::Result<FileFacts> {
        let basic = basic_information(file)?;
        let extended = extended_information(file)?;
        let identity = identity_from_information(&basic, extended.as_ref());

        Ok(FileFacts::new(identity, u64::from(basic.nNumberOfLinks)))
    }

    #[allow(unsafe_code)]
    fn basic_information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
        let mut basic = BY_HANDLE_FILE_INFORMATION::default();

        // SAFETY: the raw handle remains valid for the duration of the borrowed
        // `File`, and `basic` is a live, writable value of the exact structure
        // required by the API.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &raw mut basic) } == 0 {
            return Err(io::Error::last_os_error());
        }

        Ok(basic)
    }

    #[allow(unsafe_code)]
    fn extended_information(file: &File) -> io::Result<Option<FILE_ID_INFO>> {
        let mut extended = FILE_ID_INFO::default();
        let extended_size = u32::try_from(size_of::<FILE_ID_INFO>()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "FILE_ID_INFO size does not fit the Windows API parameter",
            )
        })?;
        // SAFETY: the raw handle remains valid, the output pointer refers to a live
        // writable `FILE_ID_INFO`, and the passed byte count is its exact size.
        let has_extended = unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileIdInfo,
                (&raw mut extended).cast(),
                extended_size,
            )
        } != 0;

        Ok(has_extended.then_some(extended))
    }

    fn identity_from_information(
        basic: &BY_HANDLE_FILE_INFORMATION,
        extended: Option<&FILE_ID_INFO>,
    ) -> FileIdentity {
        extended
            .filter(|value| value.FileId.Identifier != [0; 16])
            .map_or_else(
                || reduced_identity(basic),
                |extended| {
                    FileIdentity::new(
                        IdentityQuality::Preferred,
                        u128::from(extended.VolumeSerialNumber),
                        u128::from_le_bytes(extended.FileId.Identifier),
                    )
                },
            )
    }

    // `u128::from` is not const on the pinned toolchain.
    #[allow(clippy::missing_const_for_fn)]
    fn reduced_identity(basic: &BY_HANDLE_FILE_INFORMATION) -> FileIdentity {
        let file_index = (u128::from(basic.nFileIndexHigh) << 32) | u128::from(basic.nFileIndexLow);
        FileIdentity::new(
            IdentityQuality::Reduced,
            u128::from(basic.dwVolumeSerialNumber),
            file_index,
        )
    }

    #[cfg(test)]
    mod tests {
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ID_128, FILE_ID_INFO,
        };

        use super::{IdentityQuality, identity_from_information};

        #[test]
        fn nonzero_extended_identity_is_preferred() {
            let basic = BY_HANDLE_FILE_INFORMATION {
                dwVolumeSerialNumber: 7,
                nFileIndexHigh: 8,
                nFileIndexLow: 9,
                ..BY_HANDLE_FILE_INFORMATION::default()
            };
            let extended = FILE_ID_INFO {
                VolumeSerialNumber: 10,
                FileId: FILE_ID_128 {
                    Identifier: [11; 16],
                },
            };

            let identity = identity_from_information(&basic, Some(&extended));

            assert_eq!(identity.quality(), IdentityQuality::Preferred);
            assert_eq!(identity.volume(), 10);
            assert_eq!(identity.file(), u128::from_le_bytes([11; 16]));
        }

        #[test]
        fn unavailable_extended_identity_uses_labeled_reduced_fallback() {
            let basic = BY_HANDLE_FILE_INFORMATION {
                dwVolumeSerialNumber: 12,
                nFileIndexHigh: 0x0102_0304,
                nFileIndexLow: 0x0506_0708,
                ..BY_HANDLE_FILE_INFORMATION::default()
            };

            let identity = identity_from_information(&basic, None);

            assert_eq!(identity.quality(), IdentityQuality::Reduced);
            assert_eq!(identity.volume(), 12);
            assert_eq!(identity.file(), 0x0102_0304_0506_0708);
        }

        #[test]
        fn all_zero_extended_identity_uses_labeled_reduced_fallback() {
            let basic = BY_HANDLE_FILE_INFORMATION {
                dwVolumeSerialNumber: 13,
                nFileIndexLow: 14,
                ..BY_HANDLE_FILE_INFORMATION::default()
            };
            let extended = FILE_ID_INFO::default();

            let identity = identity_from_information(&basic, Some(&extended));

            assert_eq!(identity.quality(), IdentityQuality::Reduced);
            assert_eq!(identity.volume(), 13);
            assert_eq!(identity.file(), 14);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    use std::fs::File;
    use std::io;

    use super::FileFacts;

    pub fn file_facts(_file: &File) -> io::Result<FileFacts> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "file identity is unsupported on this operating system",
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{File, hard_link};
    use std::io::{self, Write};

    use tempfile::tempdir;

    use super::{IdentityQuality, file_facts};

    #[test]
    fn facts_are_stable_for_one_open_file() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        File::create(&path)?.write_all(b"identity")?;
        let file = File::open(path)?;

        let first = file_facts(&file)?;
        let second = file_facts(&file)?;

        assert_eq!(first, second);
        assert!(first.link_count() >= 1);
        assert!(matches!(
            first.identity().quality(),
            IdentityQuality::Preferred | IdentityQuality::Reduced
        ));
        Ok(())
    }

    #[test]
    fn hard_links_share_identity_and_increase_link_count() -> io::Result<()> {
        let directory = tempdir()?;
        let first_path = directory.path().join("first.txt");
        let second_path = directory.path().join("second.txt");
        File::create(&first_path)?.write_all(b"linked")?;
        hard_link(&first_path, &second_path)?;

        let first = file_facts(&File::open(first_path)?)?;
        let second = file_facts(&File::open(second_path)?)?;

        assert_eq!(first.identity(), second.identity());
        assert!(first.link_count() >= 2);
        assert!(second.link_count() >= 2);
        Ok(())
    }

    #[test]
    fn separate_files_have_distinct_identity() -> io::Result<()> {
        let directory = tempdir()?;
        let first_path = directory.path().join("first.txt");
        let second_path = directory.path().join("second.txt");
        File::create(&first_path)?.write_all(b"same bytes")?;
        File::create(&second_path)?.write_all(b"same bytes")?;

        let first = file_facts(&File::open(first_path)?)?;
        let second = file_facts(&File::open(second_path)?)?;

        assert_ne!(first.identity(), second.identity());
        Ok(())
    }
}
