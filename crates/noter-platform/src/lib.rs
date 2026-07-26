//! Narrow, audited operating-system primitives for Noter's storage adapter.
//!
//! The product crate forbids unsafe code. Calls that cannot yet be expressed
//! through stable standard-library APIs live here behind safe, tested types.

use std::fs::File;
use std::io;
use std::path::Path;

#[cfg(unix)]
use imp::{
    unix_apply_required_metadata as platform_apply_required_metadata,
    unix_capture_required_metadata as platform_capture_required_metadata,
    unix_create_private_new_file as platform_create_private_new_file,
    unix_delete_open_file as platform_delete_open_file, unix_file_facts as platform_file_facts,
    unix_install_new as platform_install_new, unix_open_for_cleanup as platform_open_for_cleanup,
    unix_replace_existing as platform_replace_existing,
    unix_required_metadata_matches_source as platform_required_metadata_matches_source,
    unix_sync_file as platform_sync_file, unix_sync_parent as platform_sync_parent,
};
#[cfg(not(any(unix, windows)))]
use imp::{
    unsupported_create_private_new_file as platform_create_private_new_file,
    unsupported_delete_open_file as platform_delete_open_file,
    unsupported_file_facts as platform_file_facts, unsupported_install_new as platform_install_new,
    unsupported_open_for_cleanup as platform_open_for_cleanup,
    unsupported_replace_existing as platform_replace_existing,
    unsupported_sync_file as platform_sync_file, unsupported_sync_parent as platform_sync_parent,
};
#[cfg(windows)]
use imp::{
    windows_create_private_new_file as platform_create_private_new_file,
    windows_delete_open_file as platform_delete_open_file,
    windows_file_facts as platform_file_facts, windows_install_new as platform_install_new,
    windows_open_for_cleanup as platform_open_for_cleanup,
    windows_replace_existing as platform_replace_existing, windows_sync_file as platform_sync_file,
    windows_sync_parent as platform_sync_parent,
};

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

    /// Returns the platform's secondary change component.
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

/// Immutable platform metadata ratified from one stable source file.
///
/// The representation is private so callers can apply only a snapshot captured
/// by this crate rather than constructing incomplete security metadata.
#[cfg(unix)]
#[derive(Debug)]
pub struct RequiredMetadata {
    inner: imp::RequiredMetadata,
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
    platform_file_facts(file)
}

/// Exclusively creates a private read-write file at a new path.
///
/// Unix uses owner-only mode at creation. Windows supplies a protected DACL at
/// creation so permissive parent entries are never inherited by the new file.
///
/// # Errors
///
/// Returns an operating-system error when the path already exists, the private
/// security descriptor cannot be constructed, or the file cannot be created.
pub fn create_private_new_file(path: &Path) -> io::Result<File> {
    platform_create_private_new_file(path)
}

/// Opens an existing final entry for stable observation and handle-bound cleanup.
///
/// The final entry is opened without following a link or reparse point. Windows
/// requests delete access while preserving ordinary sharing. Unix can observe
/// the open file but cannot portably unlink a directory entry by handle.
///
/// # Errors
///
/// Returns an operating-system error when the path cannot be opened with the
/// access required for verified cleanup.
pub fn open_for_cleanup(path: &Path) -> io::Result<File> {
    platform_open_for_cleanup(path)
}

/// Requests deletion of the exact object represented by an open file handle.
///
/// Windows marks the handle's file for deletion when the last handle closes.
/// Unix returns [`io::ErrorKind::Unsupported`] because portable `unlink` remains
/// pathname-based and cannot be tied atomically to a verified open object.
///
/// # Errors
///
/// Returns an operating-system error if handle-bound deletion is unsupported or
/// the deletion request fails.
pub fn delete_open_file(file: &File) -> io::Result<()> {
    platform_delete_open_file(file)
}

/// Captures required existing-file metadata from an open regular file.
///
/// Unix snapshots attainable ownership, mode, ACLs, and visible extended
/// attributes without copying content or modification time. `expected_source`
/// must be the facts ratified from the same handle. Windows has no version of
/// this API because its native replacement primitive owns metadata merging.
/// Unix capture refuses more than 4,096 extended attributes or more than 64 MiB
/// of aggregate attribute names and values before allocating any value buffer.
///
/// # Errors
///
/// Returns an operating-system error if the source changes while required
/// metadata is captured or that metadata cannot be read exactly.
#[cfg(unix)]
pub fn capture_required_metadata(
    source: &File,
    expected_source: FileFacts,
) -> io::Result<RequiredMetadata> {
    platform_capture_required_metadata(source, expected_source)
        .map(|inner| RequiredMetadata { inner })
}

/// Compares a stable source file with a previously ratified metadata snapshot.
///
/// The source's change facts must equal `expected_source` before and after the
/// comparison. The platform-induced post-exchange change time is intentionally
/// not compared with the pre-commit snapshot, but ownership, mode, ACLs, and
/// visible extended attributes are.
///
/// # Errors
///
/// Returns an operating-system error if required metadata cannot be read or if
/// the source changes during comparison.
#[cfg(unix)]
pub fn required_metadata_matches_source(
    metadata: &RequiredMetadata,
    source: &File,
    expected_source: FileFacts,
) -> io::Result<bool> {
    platform_required_metadata_matches_source(&metadata.inner, source, expected_source)
}

/// Applies a previously ratified metadata snapshot to an open regular file.
///
/// Applies ownership, security metadata, and mode from the immutable Unix
/// snapshot.
///
/// # Errors
///
/// Returns an operating-system error if required metadata cannot be applied or
/// verified.
#[cfg(unix)]
pub fn apply_required_metadata(metadata: &RequiredMetadata, destination: &File) -> io::Result<()> {
    platform_apply_required_metadata(&metadata.inner, destination)
}

/// Result of an exclusive new-file commit operation.
#[derive(Debug)]
pub enum InstallNewOutcome {
    /// The temporary name was removed as part of the commit.
    Clean,
    /// The destination committed through a hard link, but the temporary name remains.
    CommittedWithRetainedTemporary,
}

/// Result of atomically replacing an existing destination.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ReplaceExistingOutcome {
    /// The temporary name was consumed by the platform replacement operation.
    Clean,
    /// The previous destination now occupies the temporary path and must be
    /// retained unless the caller has a pathname-independent cleanup primitive.
    DisplacedDestination,
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
/// reconciled. Unix ignores `backup` and uses a same-directory atomic exchange.
///
/// # Errors
///
/// Returns the raw operating-system failure. Callers must apply platform-aware
/// reconciliation before classifying commit state.
pub fn replace_existing(
    temporary: &Path,
    destination: &Path,
    backup: Option<&Path>,
) -> io::Result<ReplaceExistingOutcome> {
    platform_replace_existing(temporary, destination, backup)
}

/// Installs a private sibling only if the destination remains absent.
///
/// # Errors
///
/// Returns the raw operating-system failure. An `AlreadyExists` error is an
/// exclusive-create conflict, while other failures require reconciliation.
pub fn install_new(temporary: &Path, destination: &Path) -> io::Result<InstallNewOutcome> {
    platform_install_new(temporary, destination)
}

/// Requests the strongest supported temporary-file persistence barrier.
///
/// # Errors
///
/// Returns an operating-system error when no supported file barrier succeeds.
pub fn sync_file(file: &File) -> io::Result<()> {
    platform_sync_file(file)
}

/// Synchronizes the destination's containing directory when supported.
///
/// # Errors
///
/// Returns an operating-system error when a supported directory barrier fails.
pub fn sync_parent(destination: &Path) -> io::Result<ParentSyncOutcome> {
    platform_sync_parent(destination)
}

#[cfg(unix)]
mod imp {
    use std::ffi::OsStr;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::ffi::{CString, OsString};
    use std::fs::{File, OpenOptions};
    use std::io;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::os::fd::AsRawFd;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::Path;

    use rustix::fs::{
        AtFlags, Gid, Mode, RawMode, RenameFlags, Uid, fchmod, fchown, linkat, renameat_with,
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use xattr::FileExt;

    use super::{
        FileChangeToken, FileFacts, FileIdentity, IdentityQuality, InstallNewOutcome,
        ParentSyncOutcome, ReplaceExistingOutcome,
    };

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const MAX_SUPPORTED_METADATA_BYTES: usize = 67_108_864;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const MAX_SUPPORTED_XATTR_COUNT: usize = 4096;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const MAX_XATTR_READ_ATTEMPTS: usize = 3;

    #[cfg(target_os = "linux")]
    use self::{
        apply_linux_metadata as apply_native_metadata,
        capture_linux_metadata as capture_native_metadata,
        linux_metadata_payload_matches as native_metadata_payload_matches,
    };
    #[cfg(target_os = "macos")]
    use self::{
        apply_macos_metadata as apply_native_metadata,
        capture_macos_metadata as capture_native_metadata,
        macos_metadata_payload_matches as native_metadata_payload_matches,
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    use self::{
        apply_unsupported_unix_metadata as apply_native_metadata,
        capture_unsupported_unix_metadata as capture_native_metadata,
        unsupported_unix_metadata_payload_matches as native_metadata_payload_matches,
    };

    pub fn unix_file_facts(file: &File) -> io::Result<FileFacts> {
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

    pub fn unix_create_private_new_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }

    pub fn unix_open_for_cleanup(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    pub fn unix_delete_open_file(_file: &File) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "portable Unix APIs cannot delete an exact open file object",
        ))
    }

    #[derive(Debug)]
    pub struct RequiredMetadata {
        stamp: MetadataStamp,
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        attributes: Vec<ExtendedAttribute>,
        #[cfg(target_os = "macos")]
        acl_carrier: File,
        #[cfg(target_os = "macos")]
        acl_text: Vec<u8>,
    }

    pub fn unix_capture_required_metadata(
        source: &File,
        expected_source: FileFacts,
    ) -> io::Result<RequiredMetadata> {
        capture_native_metadata(source, expected_source)
    }

    #[cfg(target_os = "linux")]
    fn capture_linux_metadata(
        source: &File,
        expected_source: FileFacts,
    ) -> io::Result<RequiredMetadata> {
        let stamp = unix_metadata_stamp(source)?;
        unix_ensure_metadata_source_matches(source, stamp, expected_source, "before capture")?;
        let attributes = unix_read_native_xattrs(source)?;
        unix_ensure_metadata_source_matches(source, stamp, expected_source, "during capture")?;
        Ok(RequiredMetadata { stamp, attributes })
    }

    #[cfg(target_os = "macos")]
    fn capture_macos_metadata(
        source: &File,
        expected_source: FileFacts,
    ) -> io::Result<RequiredMetadata> {
        let stamp = unix_metadata_stamp(source)?;
        unix_ensure_metadata_source_matches(source, stamp, expected_source, "before capture")?;
        let attributes = unix_read_native_xattrs(source)?;
        let acl_text = read_macos_acl_text(source)?;
        let acl_carrier = tempfile::tempfile()?;
        copy_macos_acl(source, &acl_carrier)?;
        verify_macos_acl(&acl_text, &acl_carrier)?;
        unix_ensure_metadata_source_matches(source, stamp, expected_source, "during capture")?;
        Ok(RequiredMetadata {
            stamp,
            attributes,
            acl_carrier,
            acl_text,
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn capture_unsupported_unix_metadata(
        _source: &File,
        _expected_source: FileFacts,
    ) -> io::Result<RequiredMetadata> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "metadata snapshots are unsupported on this Unix platform",
        ))
    }

    pub fn unix_apply_required_metadata(
        metadata: &RequiredMetadata,
        destination: &File,
    ) -> io::Result<()> {
        apply_native_metadata(metadata, destination)
    }

    pub fn unix_required_metadata_matches_source(
        metadata: &RequiredMetadata,
        source: &File,
        expected_source: FileFacts,
    ) -> io::Result<bool> {
        if unix_file_facts(source)? != expected_source {
            return Err(unix_metadata_comparison_race());
        }
        let matches = native_metadata_payload_matches(metadata, source)?;
        if unix_file_facts(source)? != expected_source {
            return Err(unix_metadata_comparison_race());
        }
        Ok(matches)
    }

    #[cfg(target_os = "linux")]
    fn apply_linux_metadata(metadata: &RequiredMetadata, destination: &File) -> io::Result<()> {
        unix_apply_ownership(metadata.stamp, destination)?;
        unix_apply_native_xattrs(&metadata.attributes, destination)?;
        unix_apply_mode(metadata.stamp.mode, destination)?;
        unix_verify_native_xattrs(&metadata.attributes, destination)?;
        unix_verify_destination_stamp(metadata.stamp, destination)
    }

    #[cfg(target_os = "macos")]
    fn apply_macos_metadata(metadata: &RequiredMetadata, destination: &File) -> io::Result<()> {
        unix_apply_ownership(metadata.stamp, destination)?;
        copy_macos_acl(&metadata.acl_carrier, destination)?;
        unix_apply_native_xattrs(&metadata.attributes, destination)?;
        unix_apply_mode(metadata.stamp.mode, destination)?;
        unix_verify_native_xattrs(&metadata.attributes, destination)?;
        verify_macos_acl(&metadata.acl_text, destination)?;
        unix_verify_destination_stamp(metadata.stamp, destination)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn apply_unsupported_unix_metadata(
        _metadata: &RequiredMetadata,
        _destination: &File,
    ) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "metadata snapshots are unsupported on this Unix platform",
        ))
    }

    #[cfg(target_os = "linux")]
    fn linux_metadata_payload_matches(
        metadata: &RequiredMetadata,
        source: &File,
    ) -> io::Result<bool> {
        Ok(
            unix_metadata_payload_stamp_matches(metadata.stamp, unix_metadata_stamp(source)?)
                && metadata.attributes == unix_read_native_xattrs(source)?,
        )
    }

    #[cfg(target_os = "macos")]
    fn macos_metadata_payload_matches(
        metadata: &RequiredMetadata,
        source: &File,
    ) -> io::Result<bool> {
        Ok(
            unix_metadata_payload_stamp_matches(metadata.stamp, unix_metadata_stamp(source)?)
                && metadata.attributes == unix_read_native_xattrs(source)?
                && metadata.acl_text == read_macos_acl_text(source)?,
        )
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn unsupported_unix_metadata_payload_matches(
        _metadata: &RequiredMetadata,
        _source: &File,
    ) -> io::Result<bool> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "metadata snapshot comparison is unsupported on this Unix platform",
        ))
    }

    pub fn unix_replace_existing(
        temporary: &Path,
        destination: &Path,
        _backup: Option<&Path>,
    ) -> io::Result<ReplaceExistingOutcome> {
        unix_with_sibling_parent(
            temporary,
            destination,
            |parent, temporary_name, destination_name| {
                renameat_with(
                    parent,
                    temporary_name,
                    parent,
                    destination_name,
                    RenameFlags::EXCHANGE,
                )
                .map(|()| ReplaceExistingOutcome::DisplacedDestination)
                .map_err(Into::into)
            },
        )
    }

    pub fn unix_install_new(temporary: &Path, destination: &Path) -> io::Result<InstallNewOutcome> {
        unix_with_sibling_parent(
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
                Err(error) if unix_no_replace_is_unavailable(error) => {
                    unix_install_new_with_link(parent, temporary_name, destination_name)
                }
                Err(error) => Err(error.into()),
            },
        )
    }

    fn unix_install_new_with_link(
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

        Ok(InstallNewOutcome::CommittedWithRetainedTemporary)
    }

    const fn unix_no_replace_is_unavailable(error: rustix::io::Errno) -> bool {
        matches!(
            error,
            rustix::io::Errno::NOSYS | rustix::io::Errno::INVAL | rustix::io::Errno::NOTSUP
        )
    }

    fn unix_with_sibling_parent<T>(
        temporary: &Path,
        destination: &Path,
        operation: impl FnOnce(&File, &OsStr, &OsStr) -> io::Result<T>,
    ) -> io::Result<T> {
        let temporary_parent = unix_normalized_parent(temporary);
        let destination_parent = unix_normalized_parent(destination);
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

    fn unix_normalized_parent(path: &Path) -> &Path {
        path.parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
    }

    pub fn unix_sync_file(file: &File) -> io::Result<()> {
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

    pub fn unix_sync_parent(destination: &Path) -> io::Result<ParentSyncOutcome> {
        File::open(unix_normalized_parent(destination))?.sync_all()?;
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

    fn unix_metadata_stamp(file: &File) -> io::Result<MetadataStamp> {
        let metadata = file.metadata()?;
        Ok(MetadataStamp {
            uid: metadata.uid(),
            gid: metadata.gid(),
            mode: metadata.mode(),
            ctime: metadata.ctime(),
            ctime_nsec: metadata.ctime_nsec(),
        })
    }

    fn unix_apply_ownership(metadata: MetadataStamp, destination: &File) -> io::Result<()> {
        let destination_metadata = destination.metadata()?;
        let owner =
            (metadata.uid != destination_metadata.uid()).then(|| Uid::from_raw(metadata.uid));
        let group =
            (metadata.gid != destination_metadata.gid()).then(|| Gid::from_raw(metadata.gid));
        if owner.is_some() || group.is_some() {
            fchown(destination, owner, group)?;
        }
        Ok(())
    }

    fn unix_apply_mode(mode: u32, destination: &File) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        let raw_mode: RawMode = mode.try_into().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "snapshot mode does not fit the platform mode type",
            )
        })?;
        #[cfg(not(target_os = "macos"))]
        let raw_mode: RawMode = mode;
        fchmod(destination, Mode::from_raw_mode(raw_mode)).map_err(Into::into)
    }

    fn unix_verify_destination_stamp(
        expected: MetadataStamp,
        destination: &File,
    ) -> io::Result<()> {
        const MODE_BITS: u32 = 0o7777;
        let actual = unix_metadata_stamp(destination)?;
        if actual.uid == expected.uid
            && actual.gid == expected.gid
            && actual.mode & MODE_BITS == expected.mode & MODE_BITS
        {
            return Ok(());
        }

        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "destination ownership or mode differs after metadata application",
        ))
    }

    const fn unix_metadata_payload_stamp_matches(
        expected: MetadataStamp,
        actual: MetadataStamp,
    ) -> bool {
        const MODE_BITS: u32 = 0o7777;
        expected.uid == actual.uid
            && expected.gid == actual.gid
            && expected.mode & MODE_BITS == actual.mode & MODE_BITS
    }

    fn unix_metadata_comparison_race() -> io::Error {
        io::Error::new(
            io::ErrorKind::Interrupted,
            "source metadata changed during snapshot comparison",
        )
    }

    fn unix_ensure_metadata_source_matches(
        source: &File,
        expected_metadata: MetadataStamp,
        expected_facts: FileFacts,
        boundary: &str,
    ) -> io::Result<()> {
        let actual_metadata = unix_metadata_stamp(source)?;
        let actual_facts = unix_file_facts(source)?;
        if unix_metadata_source_matches(
            expected_metadata,
            actual_metadata,
            expected_facts,
            actual_facts,
        ) {
            return Ok(());
        }

        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            format!("source metadata changed {boundary} transfer"),
        ))
    }

    fn unix_metadata_source_matches(
        expected_metadata: MetadataStamp,
        actual_metadata: MetadataStamp,
        expected_facts: FileFacts,
        actual_facts: FileFacts,
    ) -> bool {
        let token = expected_facts.change_token();
        let expected_token_matches = expected_metadata.ctime == token.primary()
            && expected_metadata.ctime_nsec == token.secondary();
        expected_token_matches
            && actual_metadata == expected_metadata
            && actual_facts == expected_facts
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_apply_native_xattrs(
        expected_attributes: &[ExtendedAttribute],
        destination: &File,
    ) -> io::Result<()> {
        let destination_attributes = unix_read_native_xattrs(destination)?;

        for attribute in &destination_attributes {
            if !expected_attributes
                .iter()
                .any(|expected| expected.name == attribute.name)
            {
                destination.remove_xattr(&attribute.name)?;
            }
        }

        for attribute in expected_attributes {
            let already_matches = destination_attributes
                .iter()
                .any(|current| current == attribute);
            if !already_matches {
                destination.set_xattr(&attribute.name, &attribute.value)?;
            }
        }

        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_verify_native_xattrs(
        expected_attributes: &[ExtendedAttribute],
        destination: &File,
    ) -> io::Result<()> {
        if expected_attributes != unix_read_native_xattrs(destination)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "extended attributes differ after metadata transfer",
            ));
        }
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[derive(PartialEq, Eq, Debug)]
    struct ExtendedAttribute {
        name: OsString,
        value: Vec<u8>,
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_read_native_xattrs(file: &File) -> io::Result<Vec<ExtendedAttribute>> {
        unix_read_native_xattrs_bounded(file, MAX_SUPPORTED_METADATA_BYTES)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_read_native_xattrs_bounded(
        file: &File,
        byte_limit: usize,
    ) -> io::Result<Vec<ExtendedAttribute>> {
        #[cfg(target_os = "linux")]
        const CRITICAL_NAMES: [&str; 3] = [
            "security.capability",
            "security.selinux",
            "system.posix_acl_access",
        ];

        let (mut names, mut used_bytes) = unix_list_native_xattrs_bounded(file, byte_limit)?;
        let mut attributes = Vec::with_capacity(names.len());

        names.sort_unstable();
        names.dedup();
        for name in &names {
            let remaining = byte_limit.saturating_sub(used_bytes);
            let value =
                unix_read_native_xattr_bounded(file, name, remaining)?.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::Interrupted,
                        "extended attribute changed while it was read",
                    )
                })?;
            unix_reserve_metadata_bytes(&mut used_bytes, value.len(), byte_limit)?;
            attributes.push(ExtendedAttribute {
                name: name.clone(),
                value,
            });
        }

        #[cfg(target_os = "linux")]
        {
            for name in CRITICAL_NAMES {
                if names.iter().any(|current| current == OsStr::new(name)) {
                    continue;
                }
                let name_bytes = name
                    .len()
                    .checked_add(1)
                    .ok_or_else(unix_metadata_too_large)?;
                let remaining = byte_limit
                    .saturating_sub(used_bytes)
                    .saturating_sub(name_bytes);
                if let Some(value) =
                    unix_read_native_xattr_bounded(file, OsStr::new(name), remaining)?
                {
                    if attributes.len() == MAX_SUPPORTED_XATTR_COUNT {
                        return Err(unix_metadata_too_large());
                    }
                    unix_reserve_metadata_bytes(&mut used_bytes, name_bytes, byte_limit)?;
                    unix_reserve_metadata_bytes(&mut used_bytes, value.len(), byte_limit)?;
                    attributes.push(ExtendedAttribute {
                        name: OsString::from(name),
                        value,
                    });
                }
            }
        }

        attributes.sort_unstable_by(|left, right| left.name.cmp(&right.name));
        Ok(attributes)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_list_native_xattrs_bounded(
        file: &File,
        byte_limit: usize,
    ) -> io::Result<(Vec<OsString>, usize)> {
        for _ in 0..MAX_XATTR_READ_ATTEMPTS {
            let announced = unix_flistxattr(file, None);
            if announced < 0 {
                return Err(io::Error::last_os_error());
            }
            let announced = usize::try_from(announced).map_err(|_| unix_metadata_too_large())?;
            if announced > byte_limit {
                return Err(unix_metadata_too_large());
            }

            let mut buffer = Vec::new();
            buffer
                .try_reserve_exact(announced)
                .map_err(|_| unix_metadata_allocation_failed())?;
            buffer.resize(announced, 0_u8);
            let read = unix_flistxattr(file, Some(&mut buffer));
            if read < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ERANGE) {
                    continue;
                }
                return Err(error);
            }
            let read = usize::try_from(read).map_err(|_| unix_metadata_too_large())?;
            if read > buffer.len() {
                continue;
            }
            buffer.truncate(read);
            return unix_parse_native_xattr_names(&buffer).map(|names| (names, read));
        }

        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "extended attribute names did not become stable",
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_parse_native_xattr_names(buffer: &[u8]) -> io::Result<Vec<OsString>> {
        if buffer.is_empty() {
            return Ok(Vec::new());
        }
        if buffer.last() != Some(&0) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "extended attribute name list is not NUL-terminated",
            ));
        }

        let mut names = Vec::new();
        for name in buffer[..buffer.len() - 1].split(|byte| *byte == 0) {
            if name.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "extended attribute name list contains an empty name",
                ));
            }
            if names.len() == MAX_SUPPORTED_XATTR_COUNT {
                return Err(unix_metadata_too_large());
            }
            names
                .try_reserve(1)
                .map_err(|_| unix_metadata_allocation_failed())?;
            names.push(OsString::from_vec(name.to_vec()));
        }
        Ok(names)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_read_native_xattr_bounded(
        file: &File,
        name: &OsStr,
        byte_limit: usize,
    ) -> io::Result<Option<Vec<u8>>> {
        let name = CString::new(name.as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "extended attribute name contains an interior NUL",
            )
        })?;

        for _ in 0..MAX_XATTR_READ_ATTEMPTS {
            let announced = unix_fgetxattr(file, &name, None);
            if announced < 0 {
                let error = io::Error::last_os_error();
                if unix_xattr_is_missing(&error) {
                    return Ok(None);
                }
                return Err(error);
            }
            let announced = usize::try_from(announced).map_err(|_| unix_metadata_too_large())?;
            if announced > byte_limit {
                return Err(unix_metadata_too_large());
            }

            let mut value = Vec::new();
            value
                .try_reserve_exact(announced)
                .map_err(|_| unix_metadata_allocation_failed())?;
            value.resize(announced, 0_u8);
            let read = unix_fgetxattr(file, &name, Some(&mut value));
            if read < 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(libc::ERANGE) {
                    continue;
                }
                if unix_xattr_is_missing(&error) {
                    return Ok(None);
                }
                return Err(error);
            }
            let read = usize::try_from(read).map_err(|_| unix_metadata_too_large())?;
            if read != value.len() {
                continue;
            }
            return Ok(Some(value));
        }

        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "extended attribute value did not become stable",
        ))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_reserve_metadata_bytes(
        used_bytes: &mut usize,
        additional_bytes: usize,
        byte_limit: usize,
    ) -> io::Result<()> {
        let total = used_bytes
            .checked_add(additional_bytes)
            .filter(|total| *total <= byte_limit)
            .ok_or_else(unix_metadata_too_large)?;
        *used_bytes = total;
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_metadata_too_large() -> io::Error {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "required file metadata exceeds the supported safety limits",
        )
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_metadata_allocation_failed() -> io::Error {
        io::Error::other("memory allocation for required file metadata failed")
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_xattr_is_missing(error: &io::Error) -> bool {
        #[cfg(target_os = "linux")]
        {
            error.raw_os_error() == Some(libc::ENODATA)
        }
        #[cfg(target_os = "macos")]
        {
            error.raw_os_error() == Some(libc::ENOATTR)
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[allow(unsafe_code)]
    fn unix_flistxattr(file: &File, buffer: Option<&mut [u8]>) -> libc::ssize_t {
        let (pointer, size) = buffer.map_or((std::ptr::null_mut(), 0), |buffer| {
            (buffer.as_mut_ptr().cast(), buffer.len())
        });
        #[cfg(target_os = "linux")]
        // SAFETY: `file` owns a live descriptor. `pointer` is either null with
        // size zero or comes from the exclusive slice borrowed for this call.
        unsafe {
            libc::flistxattr(file.as_raw_fd(), pointer, size)
        }
        #[cfg(target_os = "macos")]
        // SAFETY: the same descriptor and buffer contract applies on macOS; the
        // zero options argument requests the ordinary attribute namespace.
        unsafe {
            libc::flistxattr(file.as_raw_fd(), pointer, size, 0)
        }
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[allow(unsafe_code)]
    fn unix_fgetxattr(file: &File, name: &CString, buffer: Option<&mut [u8]>) -> libc::ssize_t {
        let (pointer, size) = buffer.map_or((std::ptr::null_mut(), 0), |buffer| {
            (buffer.as_mut_ptr().cast(), buffer.len())
        });
        #[cfg(target_os = "linux")]
        // SAFETY: `file` and `name` remain live for the call. The buffer follows
        // the same null-or-writable-storage contract described above.
        unsafe {
            libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), pointer, size)
        }
        #[cfg(target_os = "macos")]
        // SAFETY: the descriptor, C string, and output buffer are live. Position
        // zero reads the complete value and options zero uses ordinary semantics.
        unsafe {
            libc::fgetxattr(file.as_raw_fd(), name.as_ptr(), pointer, size, 0, 0)
        }
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn copy_macos_acl(source: &File, destination: &File) -> io::Result<()> {
        let flags = libc::COPYFILE_ACL;
        // SAFETY: both descriptors come from live borrowed `File` values, the
        // state pointer is explicitly allowed to be null, and the flag requests
        // ACL metadata only, so file offsets, content, and xattrs are not copied.
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

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn read_macos_acl_text(file: &File) -> io::Result<Vec<u8>> {
        use std::ffi::CStr;
        use std::os::fd::AsRawFd;

        type Acl = *mut libc::c_void;

        unsafe extern "C" {
            fn acl_get_fd(fd: libc::c_int) -> Acl;
            fn acl_to_text(acl: Acl, length: *mut libc::ssize_t) -> *mut libc::c_char;
            fn acl_free(object: *mut libc::c_void) -> libc::c_int;
        }

        // SAFETY: the descriptor belongs to a live borrowed file. The returned
        // ACL is either null with an OS error or an allocation owned by the
        // caller and released below with `acl_free`.
        let acl = unsafe { acl_get_fd(file.as_raw_fd()) };
        if acl.is_null() {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: `acl` is live and owned until the explicit frees below. A null
        // length pointer is permitted and the returned text is NUL-terminated.
        let text = unsafe { acl_to_text(acl, std::ptr::null_mut()) };
        if text.is_null() {
            let error = io::Error::last_os_error();
            // SAFETY: `acl` is the live allocation returned above and is freed
            // exactly once on this error path.
            let _ = unsafe { acl_free(acl.cast()) };
            return Err(error);
        }

        // SAFETY: `acl_to_text` returned a live NUL-terminated byte string.
        let result = unsafe { CStr::from_ptr(text) }.to_bytes().to_vec();
        // SAFETY: both pointers are distinct live allocations returned by the
        // ACL APIs and are each freed exactly once after their bytes are copied.
        let text_free = unsafe { acl_free(text.cast()) };
        let acl_free_result = unsafe { acl_free(acl.cast()) };
        if text_free != 0 || acl_free_result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(result)
    }

    #[cfg(target_os = "macos")]
    fn verify_macos_acl(expected: &[u8], destination: &File) -> io::Result<()> {
        if expected != read_macos_acl_text(destination)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "access control list differs after metadata transfer",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        use std::io;

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        use tempfile::tempfile;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        use xattr::FileExt;

        use super::{
            MetadataStamp, unix_metadata_payload_stamp_matches, unix_metadata_source_matches,
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        use super::{unix_read_native_xattr_bounded, unix_reserve_metadata_bytes};
        use crate::{FileChangeToken, FileFacts, FileIdentity, IdentityQuality};

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        #[test]
        fn native_xattr_value_read_honors_the_preallocation_limit() -> io::Result<()> {
            #[cfg(target_os = "linux")]
            const NAME: &str = "user.noter.metadata-budget";
            #[cfg(target_os = "macos")]
            const NAME: &str = "com.noter.metadata-budget";
            const VALUE: &[u8] = b"bounded metadata";

            let file = tempfile()?;
            file.set_xattr(NAME, VALUE)?;

            let error =
                unix_read_native_xattr_bounded(&file, std::ffi::OsStr::new(NAME), VALUE.len() - 1)
                    .expect_err("an oversized xattr must be refused before allocation");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(
                unix_read_native_xattr_bounded(&file, std::ffi::OsStr::new(NAME), VALUE.len())?,
                Some(VALUE.to_vec())
            );
            Ok(())
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        #[test]
        fn metadata_budget_accepts_the_boundary_without_advancing_on_failure() {
            let mut used = 7;
            unix_reserve_metadata_bytes(&mut used, 5, 12).expect("the exact limit should be valid");
            assert_eq!(used, 12);

            let error = unix_reserve_metadata_bytes(&mut used, 1, 12)
                .expect_err("one byte past the limit must be rejected");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert_eq!(used, 12);
        }

        #[test]
        fn metadata_source_match_requires_every_ratified_fact() {
            let identity = FileIdentity::new(IdentityQuality::Preferred, 7, 11);
            let metadata = MetadataStamp {
                uid: 13,
                gid: 17,
                mode: 0o640,
                ctime: 19,
                ctime_nsec: 23,
            };
            let facts = FileFacts::new(identity, 1, FileChangeToken::new(19, 23));
            let changed_metadata = MetadataStamp {
                mode: 0o600,
                ..metadata
            };
            let changed_facts = FileFacts::new(identity, 2, FileChangeToken::new(29, 31));
            let inconsistent_facts = FileFacts::new(identity, 1, FileChangeToken::new(19, 29));

            assert!(unix_metadata_source_matches(
                metadata, metadata, facts, facts
            ));
            assert!(!unix_metadata_source_matches(
                metadata,
                changed_metadata,
                facts,
                facts
            ));
            assert!(!unix_metadata_source_matches(
                metadata,
                metadata,
                facts,
                changed_facts
            ));
            assert!(!unix_metadata_source_matches(
                metadata,
                metadata,
                inconsistent_facts,
                inconsistent_facts
            ));
        }

        #[test]
        fn metadata_payload_comparison_ignores_only_change_time() {
            let expected = MetadataStamp {
                uid: 7,
                gid: 11,
                mode: 0o100_640,
                ctime: 13,
                ctime_nsec: 17,
            };

            assert!(unix_metadata_payload_stamp_matches(
                expected,
                MetadataStamp {
                    ctime: 19,
                    ctime_nsec: 23,
                    ..expected
                }
            ));
            assert!(!unix_metadata_payload_stamp_matches(
                expected,
                MetadataStamp {
                    mode: 0o100_600,
                    ..expected
                }
            ));
            assert!(!unix_metadata_payload_stamp_matches(
                expected,
                MetadataStamp {
                    uid: 29,
                    ..expected
                }
            ));
            assert!(!unix_metadata_payload_stamp_matches(
                expected,
                MetadataStamp {
                    gid: 31,
                    ..expected
                }
            ));
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::mem::size_of;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateFileW, DELETE, FILE_ATTRIBUTE_NORMAL,
        FILE_BASIC_INFO, FILE_DISPOSITION_INFO, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FileBasicInfo, FileDispositionInfo, FileIdInfo,
        GetFileInformationByHandle, GetFileInformationByHandleEx, MOVEFILE_WRITE_THROUGH,
        MoveFileExW, ReplaceFileW, SetFileInformationByHandle,
    };

    use super::{
        FileChangeToken, FileFacts, FileIdentity, IdentityQuality, InstallNewOutcome,
        ParentSyncOutcome, ReplaceExistingOutcome,
    };

    const PRIVATE_FILE_SDDL: &str = "D:P(A;;FA;;;SY)(A;;FA;;;OW)";

    type LocalDeallocator = unsafe extern "system" fn(
        windows_sys::Win32::Foundation::HLOCAL,
    )
        -> windows_sys::Win32::Foundation::HLOCAL;

    struct WindowsSecurityDescriptor {
        raw: PSECURITY_DESCRIPTOR,
        deallocate: LocalDeallocator,
    }

    impl Drop for WindowsSecurityDescriptor {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: the descriptor is returned by LocalAlloc through the SDDL
            // conversion API, remains owned by this guard, and is freed once.
            let _ = unsafe { (self.deallocate)(self.raw.cast()) };
        }
    }

    pub fn windows_file_facts(file: &File) -> io::Result<FileFacts> {
        let basic = windows_basic_information(file)?;
        let timestamps = windows_timestamp_information(file)?;
        let extended = windows_extended_information(file)?;
        let identity = windows_identity_from_information(&basic, extended.as_ref());

        Ok(FileFacts::new(
            identity,
            u64::from(basic.nNumberOfLinks),
            FileChangeToken::new(timestamps.ChangeTime, i64::from(timestamps.FileAttributes)),
        ))
    }

    #[allow(unsafe_code)]
    pub fn windows_create_private_new_file(path: &Path) -> io::Result<File> {
        let path = windows_wide_path(path)?;
        let descriptor = windows_security_descriptor_from_sddl(PRIVATE_FILE_SDDL)?;
        let attributes_length = u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "SECURITY_ATTRIBUTES size does not fit the Windows API parameter",
            )
        })?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: attributes_length,
            lpSecurityDescriptor: descriptor.raw,
            bInheritHandle: 0,
        };

        // SAFETY: the path and security descriptor remain live for the call.
        // CREATE_NEW supplies exclusive creation. Omitting FILE_SHARE_WRITE
        // prevents another handle from modifying staged bytes while this handle
        // owns the file, and a null template handle is permitted.
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE | DELETE,
                FILE_SHARE_READ | FILE_SHARE_DELETE,
                &raw const attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: CreateFileW returned a unique, valid owned handle. `File`
        // assumes ownership and closes it exactly once.
        Ok(unsafe { File::from_raw_handle(handle) })
    }

    #[allow(unsafe_code)]
    fn windows_security_descriptor_from_sddl(sddl: &str) -> io::Result<WindowsSecurityDescriptor> {
        let mut wide: Vec<u16> = sddl.encode_utf16().collect();
        wide.push(0);
        let mut raw_descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();

        // SAFETY: `wide` is a live, NUL-terminated UTF-16 string and the output
        // pointer refers to writable storage. The returned allocation is owned
        // immediately by `WindowsSecurityDescriptor`.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &raw mut raw_descriptor,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(WindowsSecurityDescriptor {
            raw: raw_descriptor,
            deallocate: LocalFree,
        })
    }

    pub fn windows_open_for_cleanup(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .access_mode(GENERIC_READ | DELETE)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    #[allow(unsafe_code)]
    pub fn windows_delete_open_file(file: &File) -> io::Result<()> {
        let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
        let disposition_size = u32::try_from(size_of::<FILE_DISPOSITION_INFO>()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "FILE_DISPOSITION_INFO size does not fit the Windows API parameter",
            )
        })?;
        // SAFETY: the file handle is live and was opened with delete access.
        // `disposition` is initialized for the exact structure and remains live
        // for the call; the byte count matches that structure.
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfo,
                (&raw const disposition).cast(),
                disposition_size,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    pub fn windows_replace_existing(
        temporary: &Path,
        destination: &Path,
        backup: Option<&Path>,
    ) -> io::Result<ReplaceExistingOutcome> {
        let temporary = windows_wide_path(temporary)?;
        let destination = windows_wide_path(destination)?;
        let backup = backup.map(windows_wide_path).transpose()?;
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

        Ok(ReplaceExistingOutcome::Clean)
    }

    #[allow(unsafe_code)]
    pub fn windows_install_new(
        temporary: &Path,
        destination: &Path,
    ) -> io::Result<InstallNewOutcome> {
        let temporary = windows_wide_path(temporary)?;
        let destination = windows_wide_path(destination)?;

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

    pub fn windows_sync_file(file: &File) -> io::Result<()> {
        file.sync_all()
    }

    #[allow(clippy::missing_const_for_fn, clippy::unnecessary_wraps)]
    pub fn windows_sync_parent(_destination: &Path) -> io::Result<ParentSyncOutcome> {
        Ok(ParentSyncOutcome::Unsupported)
    }

    fn windows_wide_path(path: &Path) -> io::Result<Vec<u16>> {
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
    fn windows_basic_information(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
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
    fn windows_timestamp_information(file: &File) -> io::Result<FILE_BASIC_INFO> {
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
    fn windows_extended_information(file: &File) -> io::Result<Option<FILE_ID_INFO>> {
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

    fn windows_identity_from_information(
        basic: &BY_HANDLE_FILE_INFORMATION,
        extended: Option<&FILE_ID_INFO>,
    ) -> FileIdentity {
        extended
            .filter(|value| value.FileId.Identifier != [0; 16])
            .map_or_else(
                || windows_reduced_identity(basic),
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
    fn windows_reduced_identity(basic: &BY_HANDLE_FILE_INFORMATION) -> FileIdentity {
        let file_index = (u128::from(basic.nFileIndexHigh) << 32) | u128::from(basic.nFileIndexLow);
        FileIdentity::new(
            IdentityQuality::Reduced,
            u128::from(basic.dwVolumeSerialNumber),
            file_index,
        )
    }

    #[cfg(test)]
    mod tests {
        use std::fs::{self, File, OpenOptions};
        use std::io::{self, Write};
        use std::os::windows::io::AsRawHandle;
        use std::path::Path;

        use tempfile::tempdir;
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::Security::Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
            SE_FILE_OBJECT,
        };
        use windows_sys::Win32::Security::{
            DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, PSECURITY_DESCRIPTOR,
            SE_DACL_PROTECTED,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ID_128, FILE_ID_INFO,
        };

        use super::{
            IdentityQuality, LocalFree, PRIVATE_FILE_SDDL, WindowsSecurityDescriptor,
            windows_create_private_new_file, windows_delete_open_file,
            windows_extended_information, windows_identity_from_information,
            windows_open_for_cleanup, windows_security_descriptor_from_sddl,
        };

        struct LocalWideString(*mut u16);

        impl Drop for LocalWideString {
            #[allow(unsafe_code)]
            fn drop(&mut self) {
                // SAFETY: the conversion API returned this LocalAlloc-owned
                // string to this guard, which frees it exactly once.
                let _ = unsafe { LocalFree(self.0.cast()) };
            }
        }

        #[allow(unsafe_code)]
        fn descriptor_dacl_sddl(descriptor: PSECURITY_DESCRIPTOR) -> io::Result<String> {
            let mut raw = std::ptr::null_mut();
            let mut length = 0_u32;
            // SAFETY: the descriptor is live for the call and both output
            // pointers refer to writable storage. The returned string is
            // transferred immediately into `LocalWideString`.
            if unsafe {
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor,
                    SDDL_REVISION_1,
                    DACL_SECURITY_INFORMATION,
                    &raw mut raw,
                    &raw mut length,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }
            let guard = LocalWideString(raw);
            let length = usize::try_from(length).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "security descriptor string length does not fit memory",
                )
            })?;
            // SAFETY: the conversion API returned `length` live UTF-16 code
            // units, including a trailing NUL, owned by `guard`.
            let units = unsafe { std::slice::from_raw_parts(guard.0, length) };
            let content_length = units
                .iter()
                .position(|unit| *unit == 0)
                .unwrap_or(units.len());
            let units = &units[..content_length];
            String::from_utf16(units).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("security descriptor returned invalid UTF-16: {error}"),
                )
            })
        }

        #[test]
        #[allow(unsafe_code)]
        fn private_file_is_exclusive_writable_and_dacl_protected() -> io::Result<()> {
            let directory = tempdir()?;
            let path = directory.path().join("private.txt");
            let mut file = windows_create_private_new_file(&path)?;
            file.write_all(b"private")?;

            let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            // SAFETY: the file handle is valid and the descriptor output points
            // to writable storage. Unrequested SID and ACL outputs may be null.
            let status = unsafe {
                GetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &raw mut descriptor,
                )
            };
            if status != ERROR_SUCCESS {
                return Err(io::Error::from_raw_os_error(status.cast_signed()));
            }
            let descriptor_guard = WindowsSecurityDescriptor {
                raw: descriptor,
                deallocate: LocalFree,
            };
            let mut control = 0_u16;
            let mut revision = 0_u32;
            // SAFETY: the descriptor guard owns a valid self-relative security
            // descriptor and both output pointers refer to writable values.
            if unsafe {
                GetSecurityDescriptorControl(
                    descriptor_guard.raw,
                    &raw mut control,
                    &raw mut revision,
                )
            } == 0
            {
                return Err(io::Error::last_os_error());
            }

            assert_ne!(control & SE_DACL_PROTECTED, 0);
            assert_eq!(
                descriptor_dacl_sddl(descriptor_guard.raw)?,
                PRIVATE_FILE_SDDL
            );
            assert_eq!(fs::read(&path)?, b"private");
            assert_eq!(
                windows_create_private_new_file(&path)
                    .expect_err("exclusive creation must reject an existing path")
                    .kind(),
                io::ErrorKind::AlreadyExists
            );
            assert!(
                OpenOptions::new().write(true).open(&path).is_err(),
                "the private staging handle must deny competing writers"
            );
            Ok(())
        }

        #[test]
        #[allow(unsafe_code)]
        fn security_descriptor_guard_dispatches_its_deallocator() {
            use std::sync::atomic::{AtomicPtr, Ordering};

            static RELEASED: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

            unsafe extern "system" fn record_release(
                allocation: windows_sys::Win32::Foundation::HLOCAL,
            ) -> windows_sys::Win32::Foundation::HLOCAL {
                RELEASED.store(allocation, Ordering::SeqCst);
                std::ptr::null_mut()
            }

            let allocation = std::ptr::dangling_mut::<std::ffi::c_void>();
            let descriptor = WindowsSecurityDescriptor {
                raw: allocation,
                deallocate: record_release,
            };

            drop(descriptor);

            assert_eq!(
                RELEASED.swap(std::ptr::null_mut(), Ordering::SeqCst),
                allocation
            );
        }

        #[test]
        fn cleanup_verification_denies_competing_writers() -> io::Result<()> {
            let directory = tempdir()?;
            let path = directory.path().join("cleanup.txt");
            fs::write(&path, b"verified revision")?;

            let cleanup = windows_open_for_cleanup(&path)?;
            assert!(
                OpenOptions::new().write(true).open(&path).is_err(),
                "cleanup verification must preserve an artifact when another writer is active"
            );
            drop(cleanup);

            OpenOptions::new().write(true).open(&path)?;
            Ok(())
        }

        #[test]
        fn windows_failure_boundaries_are_explicit() -> io::Result<()> {
            let directory = tempdir()?;
            let source_path = directory.path().join("source.txt");
            let destination_path = directory.path().join("destination.txt");
            let missing_path = directory.path().join("missing.txt");
            fs::write(&source_path, b"source")?;
            fs::write(&destination_path, b"destination")?;
            let source = File::open(&source_path)?;
            assert_eq!(
                windows_delete_open_file(&source)
                    .expect_err("a read-only handle lacks delete access")
                    .kind(),
                io::ErrorKind::PermissionDenied
            );
            assert!(
                crate::replace_existing(&missing_path, &destination_path, None).is_err(),
                "replacement must not invent a missing sibling"
            );
            assert!(
                crate::install_new(&missing_path, &directory.path().join("new.txt")).is_err(),
                "installation must not invent a missing sibling"
            );
            assert_eq!(
                crate::replace_existing(
                    Path::new("invalid\0replacement"),
                    &destination_path,
                    None,
                )
                .expect_err("interior NUL paths must fail before native calls")
                .kind(),
                io::ErrorKind::InvalidInput
            );
            assert!(
                windows_security_descriptor_from_sddl("not valid SDDL").is_err(),
                "invalid SDDL must fail without creating a file"
            );
            assert_eq!(fs::read(source_path)?, b"source");
            assert_eq!(fs::read(destination_path)?, b"destination");
            Ok(())
        }

        #[test]
        fn native_file_sync_propagates_device_failures() -> io::Result<()> {
            let device = File::open("NUL")?;

            assert!(
                crate::sync_file(&device).is_err(),
                "a failed native persistence barrier must not be reported as durable"
            );
            Ok(())
        }

        #[test]
        fn native_regular_file_exposes_a_nonzero_extended_identity() -> io::Result<()> {
            let directory = tempdir()?;
            let path = directory.path().join("identity.txt");
            fs::write(&path, b"identity")?;
            let file = File::open(path)?;

            let extended = windows_extended_information(&file)?
                .expect("the supported Windows test volume must expose FileIdInfo");

            assert_ne!(extended.FileId.Identifier, [0; 16]);
            assert_eq!(
                crate::file_facts(&file)?.identity().quality(),
                IdentityQuality::Preferred
            );
            Ok(())
        }

        #[test]
        fn handle_bound_deletion_does_not_remove_a_rebound_path() -> io::Result<()> {
            let directory = tempdir()?;
            let original_path = directory.path().join("owned.txt");
            let moved_path = directory.path().join("moved-owned.txt");
            let external_content = b"external replacement";
            let mut owned = windows_create_private_new_file(&original_path)?;
            owned.write_all(b"owned temporary")?;
            owned.sync_all()?;

            fs::rename(&original_path, &moved_path)?;
            fs::write(&original_path, external_content)?;

            windows_delete_open_file(&owned)?;
            drop(owned);

            assert!(!moved_path.exists());
            assert_eq!(fs::read(&original_path)?, external_content);
            Ok(())
        }

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

            let identity = windows_identity_from_information(&basic, Some(&extended));

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

            let identity = windows_identity_from_information(&basic, None);

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

            let identity = windows_identity_from_information(&basic, Some(&extended));

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

    use super::{FileFacts, InstallNewOutcome, ParentSyncOutcome, ReplaceExistingOutcome};

    pub fn unsupported_file_facts(_file: &File) -> io::Result<FileFacts> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "file identity is unsupported on this operating system",
        ))
    }

    pub fn unsupported_create_private_new_file(_path: &Path) -> io::Result<File> {
        unsupported_error("private file creation")
    }

    pub fn unsupported_open_for_cleanup(_path: &Path) -> io::Result<File> {
        unsupported_error("verified cleanup open")
    }

    pub fn unsupported_delete_open_file(_file: &File) -> io::Result<()> {
        unsupported_error("handle-bound file deletion")
    }

    pub fn unsupported_replace_existing(
        _temporary: &Path,
        _destination: &Path,
        _backup: Option<&Path>,
    ) -> io::Result<ReplaceExistingOutcome> {
        unsupported_error("file replacement")
    }

    pub fn unsupported_install_new(
        _temporary: &Path,
        _destination: &Path,
    ) -> io::Result<InstallNewOutcome> {
        unsupported_error("exclusive file installation")
    }

    pub fn unsupported_sync_file(_file: &File) -> io::Result<()> {
        unsupported_error("file synchronization")
    }

    pub fn unsupported_sync_parent(_destination: &Path) -> io::Result<ParentSyncOutcome> {
        unsupported_error("parent synchronization")
    }

    fn unsupported_error<T>(operation: &str) -> io::Result<T> {
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

    use super::{FileChangeToken, IdentityQuality, file_facts};
    #[cfg(unix)]
    use super::{ReplaceExistingOutcome, replace_existing};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use super::{
        apply_required_metadata, capture_required_metadata, required_metadata_matches_source,
    };

    #[test]
    fn change_token_components_are_exposed_exactly() {
        let token = FileChangeToken::new(17, 29);

        assert_eq!(token.primary(), 17);
        assert_eq!(token.secondary(), 29);
    }

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

    #[cfg(unix)]
    #[test]
    fn existing_replacement_retains_the_displaced_destination() -> io::Result<()> {
        let directory = tempdir()?;
        let temporary = directory.path().join("temporary.txt");
        let destination = directory.path().join("destination.txt");
        fs::write(&temporary, b"new bytes")?;
        fs::write(&destination, b"old bytes")?;

        let outcome = replace_existing(&temporary, &destination, None)?;

        assert_eq!(outcome, ReplaceExistingOutcome::DisplacedDestination);
        assert_eq!(fs::read(&destination)?, b"new bytes");
        assert_eq!(fs::read(&temporary)?, b"old bytes");
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn metadata_snapshot_preserves_ratified_mode_and_visible_xattrs() -> io::Result<()> {
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

        let expected_source = file_facts(&source)?;
        let metadata = capture_required_metadata(&source, expected_source)?;
        assert!(required_metadata_matches_source(
            &metadata,
            &source,
            expected_source
        )?);
        source.set_xattr(source_attribute_name(), b"changed after capture")?;
        let mut changed_permissions = source.metadata()?.permissions();
        changed_permissions.set_mode(0o600);
        source.set_permissions(changed_permissions)?;
        assert!(!required_metadata_matches_source(
            &metadata,
            &source,
            file_facts(&source)?
        )?);
        apply_required_metadata(&metadata, &destination)?;

        assert_eq!(
            destination.get_xattr(source_attribute_name())?,
            Some(b"source attribute".to_vec())
        );
        assert_eq!(
            source.get_xattr(source_attribute_name())?,
            Some(b"changed after capture".to_vec())
        );
        assert_eq!(destination.get_xattr(extra_attribute_name())?, None);
        assert_eq!(destination.metadata()?.permissions().mode() & 0o7777, 0o640);
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn metadata_capture_rejects_a_source_changed_after_ratification() -> io::Result<()> {
        let directory = tempdir()?;
        let source_path = directory.path().join("source.txt");
        let alias_path = directory.path().join("source-alias.txt");
        let destination_path = directory.path().join("destination.txt");
        File::create(&source_path)?.write_all(b"source")?;
        File::create(&destination_path)?.write_all(b"destination")?;

        let source = File::open(&source_path)?;
        let expected_source = file_facts(&source)?;
        fs::hard_link(&source_path, &alias_path)?;

        let error = capture_required_metadata(&source, expected_source)
            .expect_err("changed source facts must fail before metadata capture");

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert!(error.to_string().contains("before capture"));
        assert_eq!(fs::read(&destination_path)?, b"destination");
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_acl_snapshot_detects_change_and_restores_only_captured_acl() -> io::Result<()> {
        use std::process::Command;

        let directory = tempdir()?;
        let source_path = directory.path().join("source.txt");
        let destination_path = directory.path().join("destination.txt");
        fs::write(&source_path, b"source")?;
        fs::write(&destination_path, b"destination")?;

        let add_status = Command::new("/bin/chmod")
            .args(["+a", "everyone deny write"])
            .arg(&source_path)
            .status()?;
        if !add_status.success() {
            return Err(io::Error::other(format!(
                "chmod failed to create the macOS ACL fixture: {add_status}"
            )));
        }

        let source = File::open(&source_path)?;
        let destination = File::options()
            .read(true)
            .write(true)
            .open(&destination_path)?;
        let source_facts = file_facts(&source)?;
        let metadata = capture_required_metadata(&source, source_facts)?;
        assert!(required_metadata_matches_source(
            &metadata,
            &source,
            source_facts
        )?);

        let remove_status = Command::new("/bin/chmod")
            .args(["-a#", "0"])
            .arg(&source_path)
            .status()?;
        if !remove_status.success() {
            return Err(io::Error::other(format!(
                "chmod failed to mutate the macOS ACL fixture: {remove_status}"
            )));
        }
        assert!(!required_metadata_matches_source(
            &metadata,
            &source,
            file_facts(&source)?
        )?);

        apply_required_metadata(&metadata, &destination)?;
        assert!(required_metadata_matches_source(
            &metadata,
            &destination,
            file_facts(&destination)?
        )?);
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
