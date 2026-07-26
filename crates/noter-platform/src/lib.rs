//! Narrow, audited operating-system primitives for Noter's storage adapter.
//!
//! The product crate forbids unsafe code. Calls that cannot yet be expressed
//! through stable standard-library APIs live here behind safe, tested types.

use std::fs::File;
use std::io;
use std::path::Path;

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

/// Opaque timestamp components for the last content or metadata change.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FileChangeToken {
    primary: i64,
    secondary: i64,
}

impl FileChangeToken {
    const fn new(primary: i64, secondary: i64) -> Self {
        Self { primary, secondary }
    }

    /// Returns the platform timestamp's primary component.
    #[must_use]
    pub const fn primary(self) -> i64 {
        self.primary
    }

    /// Returns the platform timestamp's subsecond or reserved component.
    #[must_use]
    pub const fn secondary(self) -> i64 {
        self.secondary
    }
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
    change_token: FileChangeToken,
}

impl FileFacts {
    const fn new(identity: FileIdentity, link_count: u64, change_token: FileChangeToken) -> Self {
        Self {
            identity,
            link_count,
            change_token,
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

    /// Returns the platform content-or-metadata change timestamp.
    #[must_use]
    pub const fn change_token(self) -> FileChangeToken {
        self.change_token
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

/// Copies required existing-file metadata between already open regular files.
///
/// Unix implementations preserve attainable ownership, mode, ACLs, and visible
/// extended attributes without copying the source modification time. Windows
/// defers its native metadata merge to the replacement operation.
///
/// # Errors
///
/// Returns an operating-system error if required metadata cannot be read,
/// applied, or verified before commit.
#[allow(clippy::missing_const_for_fn)]
pub fn copy_required_metadata(source: &File, destination: &File) -> io::Result<()> {
    imp::copy_required_metadata(source, destination)
}

/// Result of an exclusive new-file commit operation.
#[derive(Debug)]
pub enum InstallNewOutcome {
    /// The temporary name was removed as part of the commit.
    Clean,
    /// The destination committed through a hard link, but the temporary name remains.
    CommittedWithCleanupError(io::Error),
}

/// Result of requesting a containing-directory persistence barrier.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParentSyncOutcome {
    /// The containing directory was synchronized successfully.
    Synced,
    /// The platform does not expose a supported directory barrier.
    Unsupported,
}

/// Atomically replaces an existing destination with its private sibling.
///
/// On Windows, `backup` is required so documented partial failures can be
/// reconciled. Unix ignores `backup` and uses a same-directory rename.
///
/// # Errors
///
/// Returns the raw operating-system failure. Callers must apply platform-aware
/// reconciliation before classifying commit state.
pub fn replace_existing(
    temporary: &Path,
    destination: &Path,
    backup: Option<&Path>,
) -> io::Result<()> {
    imp::replace_existing(temporary, destination, backup)
}

/// Installs a private sibling only if the destination remains absent.
///
/// # Errors
///
/// Returns the raw operating-system failure. An `AlreadyExists` error is an
/// exclusive-create conflict, while other failures require reconciliation.
pub fn install_new(temporary: &Path, destination: &Path) -> io::Result<InstallNewOutcome> {
    imp::install_new(temporary, destination)
}

/// Requests the strongest supported temporary-file persistence barrier.
///
/// # Errors
///
/// Returns an operating-system error when no supported file barrier succeeds.
pub fn sync_file(file: &File) -> io::Result<()> {
    imp::sync_file(file)
}

/// Synchronizes the destination's containing directory when supported.
///
/// # Errors
///
/// Returns an operating-system error when a supported directory barrier fails.
pub fn sync_parent(destination: &Path) -> io::Result<ParentSyncOutcome> {
    imp::sync_parent(destination)
}

#[cfg(unix)]
mod imp {
    use std::ffi::OsStr;
    #[cfg(target_os = "linux")]
    use std::ffi::OsString;
    use std::fs::File;
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    use rustix::fs::{
        AtFlags, Gid, Mode, RawMode, RenameFlags, Uid, fchmod, fchown, linkat, renameat,
        renameat_with, unlinkat,
    };
    #[cfg(target_os = "linux")]
    use xattr::FileExt;

    use super::{
        FileChangeToken, FileFacts, FileIdentity, IdentityQuality, InstallNewOutcome,
        ParentSyncOutcome,
    };

    pub fn file_facts(file: &File) -> io::Result<FileFacts> {
        let metadata = file.metadata()?;
        Ok(FileFacts::new(
            FileIdentity::new(
                IdentityQuality::Preferred,
                u128::from(metadata.dev()),
                u128::from(metadata.ino()),
            ),
            metadata.nlink(),
            FileChangeToken::new(metadata.ctime(), metadata.ctime_nsec()),
        ))
    }

    pub fn copy_required_metadata(source: &File, destination: &File) -> io::Result<()> {
        let before = metadata_stamp(source)?;
        let destination_metadata = destination.metadata()?;
        let owner = (before.uid != destination_metadata.uid()).then(|| Uid::from_raw(before.uid));
        let group = (before.gid != destination_metadata.gid()).then(|| Gid::from_raw(before.gid));

        if owner.is_some() || group.is_some() {
            fchown(destination, owner, group)?;
        }

        copy_security_metadata(source, destination)?;
        #[cfg(target_os = "macos")]
        let raw_mode: RawMode = before.mode.try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "source mode does not fit the platform mode type",
            )
        })?;
        #[cfg(not(target_os = "macos"))]
        let raw_mode: RawMode = before.mode;
        fchmod(destination, Mode::from_raw_mode(raw_mode))?;

        #[cfg(target_os = "linux")]
        verify_linux_xattrs(source, destination)?;

        if metadata_stamp(source)? != before {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "source metadata changed during transfer",
            ));
        }

        Ok(())
    }

    pub fn replace_existing(
        temporary: &Path,
        destination: &Path,
        _backup: Option<&Path>,
    ) -> io::Result<()> {
        with_sibling_parent(
            temporary,
            destination,
            |parent, temporary_name, destination_name| {
                renameat(parent, temporary_name, parent, destination_name).map_err(Into::into)
            },
        )
    }

    pub fn install_new(temporary: &Path, destination: &Path) -> io::Result<InstallNewOutcome> {
        with_sibling_parent(
            temporary,
            destination,
            |parent, temporary_name, destination_name| match renameat_with(
                parent,
                temporary_name,
                parent,
                destination_name,
                RenameFlags::NOREPLACE,
            ) {
                Ok(()) => Ok(InstallNewOutcome::Clean),
                Err(error) if no_replace_is_unavailable(error) => {
                    install_new_with_link(parent, temporary_name, destination_name)
                }
                Err(error) => Err(error.into()),
            },
        )
    }

    fn install_new_with_link(
        parent: &File,
        temporary_name: &OsStr,
        destination_name: &OsStr,
    ) -> io::Result<InstallNewOutcome> {
        linkat(
            parent,
            temporary_name,
            parent,
            destination_name,
            AtFlags::empty(),
        )
        .map_err(io::Error::from)?;

        match unlinkat(parent, temporary_name, AtFlags::empty()) {
            Ok(()) => Ok(InstallNewOutcome::Clean),
            Err(error) => Ok(InstallNewOutcome::CommittedWithCleanupError(error.into())),
        }
    }

    const fn no_replace_is_unavailable(error: rustix::io::Errno) -> bool {
        matches!(
            error,
            rustix::io::Errno::NOSYS | rustix::io::Errno::INVAL | rustix::io::Errno::NOTSUP
        )
    }

    fn with_sibling_parent<T>(
        temporary: &Path,
        destination: &Path,
        operation: impl FnOnce(&File, &OsStr, &OsStr) -> io::Result<T>,
    ) -> io::Result<T> {
        let temporary_parent = normalized_parent(temporary);
        let destination_parent = normalized_parent(destination);
        if temporary_parent != destination_parent {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "temporary and destination paths are not siblings",
            ));
        }
        let temporary_name = temporary.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "temporary path has no filename",
            )
        })?;
        let destination_name = destination.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination path has no filename",
            )
        })?;
        let parent = File::open(temporary_parent)?;
        operation(&parent, temporary_name, destination_name)
    }

    fn normalized_parent(path: &Path) -> &Path {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    pub fn sync_file(file: &File) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        {
            match rustix::fs::fcntl_fullfsync(file) {
                Ok(()) => return Ok(()),
                Err(rustix::io::Errno::NOTSUP | rustix::io::Errno::INVAL) => {}
                Err(error) => return Err(error.into()),
            }
        }

        file.sync_all()
    }

    pub fn sync_parent(destination: &Path) -> io::Result<ParentSyncOutcome> {
        File::open(normalized_parent(destination))?.sync_all()?;
        Ok(ParentSyncOutcome::Synced)
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    struct MetadataStamp {
        uid: u32,
        gid: u32,
        mode: u32,
        ctime: i64,
        ctime_nsec: i64,
    }

    fn metadata_stamp(file: &File) -> io::Result<MetadataStamp> {
        let metadata = file.metadata()?;
        Ok(MetadataStamp {
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode(),
            ctime: metadata.ctime(),
            ctime_nsec: metadata.ctime_nsec(),
        })
    }

    #[cfg(target_os = "linux")]
    fn copy_security_metadata(source: &File, destination: &File) -> io::Result<()> {
        let source_attributes = read_linux_xattrs(source)?;
        let destination_attributes = read_linux_xattrs(destination)?;

        for attribute in &destination_attributes {
            if !source_attributes
                .iter()
                .any(|source| source.name == attribute.name)
            {
                destination.remove_xattr(&attribute.name)?;
            }
        }

        for attribute in &source_attributes {
            let already_matches = destination_attributes
                .iter()
                .any(|current| current == attribute);
            if !already_matches {
                destination.set_xattr(&attribute.name, &attribute.value)?;
            }
        }

        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn verify_linux_xattrs(source: &File, destination: &File) -> io::Result<()> {
        if read_linux_xattrs(source)? != read_linux_xattrs(destination)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "extended attributes differ after metadata transfer",
            ));
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[derive(PartialEq, Eq, Debug)]
    struct ExtendedAttribute {
        name: OsString,
        value: Vec<u8>,
    }

    #[cfg(target_os = "linux")]
    fn read_linux_xattrs(file: &File) -> io::Result<Vec<ExtendedAttribute>> {
        const CRITICAL_NAMES: [&str; 3] = [
            "security.capability",
            "security.selinux",
            "system.posix_acl_access",
        ];

        let mut names: Vec<OsString> = file.list_xattr()?.collect();
        for name in CRITICAL_NAMES {
            if names.iter().any(|current| current == OsStr::new(name)) {
                continue;
            }
            if file.get_xattr(name)?.is_some() {
                names.push(OsString::from(name));
            }
        }
        names.sort_unstable();
        names.dedup();

        names
            .into_iter()
            .map(|name| {
                file.get_xattr(&name)?
                    .map(|value| ExtendedAttribute { name, value })
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::Interrupted,
                            "extended attribute changed while it was read",
                        )
                    })
            })
            .collect()
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn copy_security_metadata(source: &File, destination: &File) -> io::Result<()> {
        use std::os::fd::AsRawFd;

        let flags = libc::COPYFILE_ACL | libc::COPYFILE_XATTR;
        // SAFETY: both descriptors come from live borrowed `File` values, the
        // state pointer is explicitly allowed to be null, and the flags request
        // metadata only, so file offsets and content are not modified.
        if unsafe {
            libc::fcopyfile(
                source.as_raw_fd(),
                destination.as_raw_fd(),
                std::ptr::null_mut(),
                flags,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn copy_security_metadata(_source: &File, _destination: &File) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "security metadata transfer is unsupported on this Unix platform",
        ))
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::File;
    use std::io;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, FILE_BASIC_INFO, FILE_ID_INFO, FileBasicInfo, FileIdInfo,
        GetFileInformationByHandle, GetFileInformationByHandleEx, MOVEFILE_WRITE_THROUGH,
        MoveFileExW, ReplaceFileW,
    };

    use super::{
        FileChangeToken, FileFacts, FileIdentity, IdentityQuality, InstallNewOutcome,
        ParentSyncOutcome,
    };

    pub fn file_facts(file: &File) -> io::Result<FileFacts> {
        let basic = basic_information(file)?;
        let timestamps = timestamp_information(file)?;
        let extended = extended_information(file)?;
        let identity = identity_from_information(&basic, extended.as_ref());

        Ok(FileFacts::new(
            identity,
            u64::from(basic.nNumberOfLinks),
            FileChangeToken::new(timestamps.ChangeTime, 0),
        ))
    }

    #[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
    pub fn copy_required_metadata(_source: &File, _destination: &File) -> io::Result<()> {
        Ok(())
    }

    #[allow(unsafe_code)]
    pub fn replace_existing(
        temporary: &Path,
        destination: &Path,
        backup: Option<&Path>,
    ) -> io::Result<()> {
        let temporary = wide_path(temporary)?;
        let destination = wide_path(destination)?;
        let backup = backup.map(wide_path).transpose()?;
        let backup_pointer = backup.as_ref().map_or(std::ptr::null(), Vec::as_ptr);

        // SAFETY: all path buffers are NUL-terminated and remain live for the
        // call, optional backup uses a null pointer when absent, reserved
        // pointers are null as required, and zero flags preserve merge errors.
        if unsafe {
            ReplaceFileW(
                destination.as_ptr(),
                temporary.as_ptr(),
                backup_pointer,
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }

        Ok(())
    }

    #[allow(unsafe_code)]
    pub fn install_new(temporary: &Path, destination: &Path) -> io::Result<InstallNewOutcome> {
        let temporary = wide_path(temporary)?;
        let destination = wide_path(destination)?;

        // SAFETY: both path buffers are NUL-terminated and live for the call.
        // The only flag requests write-through; replacement and cross-volume
        // copy flags are intentionally absent.
        if unsafe {
            MoveFileExW(
                temporary.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }

        Ok(InstallNewOutcome::Clean)
    }

    pub fn sync_file(file: &File) -> io::Result<()> {
        file.sync_all()
    }

    #[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
    pub fn sync_parent(_destination: &Path) -> io::Result<ParentSyncOutcome> {
        Ok(ParentSyncOutcome::Unsupported)
    }

    fn wide_path(path: &Path) -> io::Result<Vec<u16>> {
        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        if wide.contains(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows path contains an interior NUL",
            ));
        }
        wide.push(0);
        Ok(wide)
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
    fn timestamp_information(file: &File) -> io::Result<FILE_BASIC_INFO> {
        let mut timestamps = FILE_BASIC_INFO::default();
        let timestamps_size = u32::try_from(size_of::<FILE_BASIC_INFO>()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "FILE_BASIC_INFO size does not fit the Windows API parameter",
            )
        })?;

        // SAFETY: the raw handle remains valid, the output pointer refers to a live
        // writable `FILE_BASIC_INFO`, and the passed byte count is its exact size.
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileBasicInfo,
                (&raw mut timestamps).cast(),
                timestamps_size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }

        Ok(timestamps)
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
    use std::path::Path;

    use super::{FileFacts, InstallNewOutcome, ParentSyncOutcome};

    pub fn file_facts(_file: &File) -> io::Result<FileFacts> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "file identity is unsupported on this operating system",
        ))
    }

    pub fn copy_required_metadata(_source: &File, _destination: &File) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "metadata transfer is unsupported on this operating system",
        ))
    }

    pub fn replace_existing(
        _temporary: &Path,
        _destination: &Path,
        _backup: Option<&Path>,
    ) -> io::Result<()> {
        unsupported("file replacement")
    }

    pub fn install_new(_temporary: &Path, _destination: &Path) -> io::Result<InstallNewOutcome> {
        unsupported("exclusive file installation")
    }

    pub fn sync_file(_file: &File) -> io::Result<()> {
        unsupported("file synchronization")
    }

    pub fn sync_parent(_destination: &Path) -> io::Result<ParentSyncOutcome> {
        unsupported("parent synchronization")
    }

    fn unsupported<T>(operation: &str) -> io::Result<T> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{operation} is unsupported on this operating system"),
        ))
    }
}

#[cfg(test)]
mod tests {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::fs;
    use std::fs::{File, hard_link};
    use std::io::{self, Write};

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::os::unix::fs::PermissionsExt;

    use tempfile::tempdir;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use xattr::FileExt;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use super::copy_required_metadata;
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn metadata_copy_preserves_mode_and_exact_visible_xattrs() -> io::Result<()> {
        let directory = tempdir()?;
        let source_path = directory.path().join("source.txt");
        let destination_path = directory.path().join("destination.txt");
        File::create(&source_path)?.write_all(b"source")?;
        File::create(&destination_path)?.write_all(b"destination")?;

        let mut source_permissions = fs::metadata(&source_path)?.permissions();
        source_permissions.set_mode(0o640);
        fs::set_permissions(&source_path, source_permissions)?;

        let source = File::options().read(true).write(true).open(&source_path)?;
        let destination = File::options()
            .read(true)
            .write(true)
            .open(&destination_path)?;
        source.set_xattr(source_attribute_name(), b"source attribute")?;
        destination.set_xattr(extra_attribute_name(), b"remove me")?;

        copy_required_metadata(&source, &destination)?;

        assert_eq!(
            destination.get_xattr(source_attribute_name())?,
            Some(b"source attribute".to_vec())
        );
        assert_eq!(destination.get_xattr(extra_attribute_name())?, None);
        assert_eq!(destination.metadata()?.permissions().mode() & 0o7777, 0o640);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    const fn source_attribute_name() -> &'static str {
        "user.noter.source"
    }

    #[cfg(target_os = "macos")]
    const fn source_attribute_name() -> &'static str {
        "com.noter.source"
    }

    #[cfg(target_os = "linux")]
    const fn extra_attribute_name() -> &'static str {
        "user.noter.extra"
    }

    #[cfg(target_os = "macos")]
    const fn extra_attribute_name() -> &'static str {
        "com.noter.extra"
    }
}
