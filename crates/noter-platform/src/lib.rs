//! Narrow, audited operating-system primitives for Noter's storage adapter.
//!
//! The product crate forbids unsafe code. Calls that cannot yet be expressed
//! through stable standard-library APIs live here behind safe, tested types.

#[cfg(unix)]
use std::ffi::OsString;
use std::fmt;
use std::fs::File;
use std::io;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

#[cfg(unix)]
use imp::{
    unix_apply_required_metadata as platform_apply_required_metadata,
    unix_capture_required_metadata as platform_capture_required_metadata,
    unix_delete_open_file as platform_delete_open_file, unix_file_facts as platform_file_facts,
    unix_install_new as platform_install_new,
    unix_open_existing_no_follow as platform_open_existing_no_follow,
    unix_open_for_cleanup as platform_open_for_cleanup,
    unix_replace_existing as platform_replace_existing,
    unix_required_metadata_matches_source as platform_required_metadata_matches_source,
    unix_restrict_open_file_to_owner as platform_restrict_open_file_to_owner,
    unix_sync_file as platform_sync_file, unix_sync_parent as platform_sync_parent,
};

#[cfg(target_os = "macos")]
use imp::macos_create_private_new_file as platform_create_private_new_file;
#[cfg(all(unix, not(target_os = "macos")))]
use imp::unix_create_private_new_file as platform_create_private_new_file;
#[cfg(not(any(unix, windows)))]
use imp::{
    unsupported_create_private_new_file as platform_create_private_new_file,
    unsupported_delete_open_file as platform_delete_open_file,
    unsupported_file_facts as platform_file_facts, unsupported_install_new as platform_install_new,
    unsupported_open_existing_no_follow as platform_open_existing_no_follow,
    unsupported_open_for_cleanup as platform_open_for_cleanup,
    unsupported_replace_existing as platform_replace_existing,
    unsupported_sync_file as platform_sync_file, unsupported_sync_parent as platform_sync_parent,
};
#[cfg(windows)]
use imp::{
    windows_create_private_new_file as platform_create_private_new_file,
    windows_delete_open_file as platform_delete_open_file,
    windows_file_facts as platform_file_facts, windows_install_new as platform_install_new,
    windows_open_existing_no_follow as platform_open_existing_no_follow,
    windows_open_for_cleanup as platform_open_for_cleanup,
    windows_replace_existing as platform_replace_existing, windows_sync_file as platform_sync_file,
    windows_sync_parent as platform_sync_parent,
};

#[cfg(any(unix, test))]
const fn combine_disjoint_flag_bits(left: u32, right: u32) -> u32 {
    assert!(
        left & right == 0,
        "overlapping flags cannot be combined as disjoint values"
    );
    left + right
}

#[derive(Debug)]
struct RetainedPrivateCreation {
    cause: io::Error,
    cleanup: Option<io::Error>,
}

impl fmt::Display for RetainedPrivateCreation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "private file security finalization failed after exclusive creation: {}",
            self.cause
        )?;
        if let Some(cleanup) = &self.cleanup {
            write!(formatter, "; handle-bound cleanup also failed: {cleanup}")?;
        }
        Ok(())
    }
}

impl std::error::Error for RetainedPrivateCreation {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

#[cfg(any(unix, test))]
fn retained_private_creation_error(cause: io::Error) -> io::Error {
    io::Error::new(
        cause.kind(),
        RetainedPrivateCreation {
            cause,
            cleanup: None,
        },
    )
}

#[cfg(any(target_os = "windows", test))]
fn retained_private_creation_error_with_cleanup(cause: io::Error, cleanup: io::Error) -> io::Error {
    io::Error::new(
        cause.kind(),
        RetainedPrivateCreation {
            cause,
            cleanup: Some(cleanup),
        },
    )
}

/// Reports whether private-file creation succeeded but security finalization failed.
///
/// A `true` result means the caller must conservatively report that the newly
/// created zero-byte pathname may remain. It must not remove that pathname
/// without independently proving that it still identifies the created object.
#[must_use]
pub fn creation_error_may_have_retained_private_file(error: &io::Error) -> bool {
    retained_private_file_creation_cause(error).is_some()
}

/// Returns the native failure that followed successful private-file creation.
///
/// Callers may use its error kind and raw operating-system code for a redacted
/// primary diagnostic. The separate retained-path warning must still avoid full
/// paths and must not imply that pathname cleanup is safe.
#[must_use]
pub fn retained_private_file_creation_cause(error: &io::Error) -> Option<&io::Error> {
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<RetainedPrivateCreation>())
        .map(|marker| &marker.cause)
}

/// Returns the handle-bound cleanup failure for a retained private creation.
///
/// `None` means either the error is not a marked post-creation failure or no
/// cleanup failure was recorded. Call
/// [`retained_private_file_creation_cause`] first to distinguish those cases.
#[must_use]
pub fn retained_private_file_cleanup_cause(error: &io::Error) -> Option<&io::Error> {
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<RetainedPrivateCreation>())
        .and_then(|marker| marker.cleanup.as_ref())
}

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
/// Linux and other supported Unix targets request owner-only mode at creation.
/// macOS atomically requests mode 0600 and a no-inherit ACL, then sets exact mode
/// 0600, applies the native remove-ACL sentinel, and verifies true ACL absence
/// before returning the still-empty file. Windows supplies a protected DACL at
/// creation with the process user's explicit SID as owner and principal, then
/// verifies that owner and the exact protected user-and-SYSTEM DACL through the
/// opened object. Filesystems that ignore or cannot report the requested policy
/// fail closed before document bytes exist.
///
/// # Errors
///
/// Returns an operating-system error when the path already exists, the private
/// security descriptor cannot be constructed, the file cannot be created, or
/// the macOS mode or ACL absence cannot be finalized and verified, or the
/// Windows owner or protected DACL cannot be verified. Call
/// [`creation_error_may_have_retained_private_file`] to distinguish the last
/// case, where an empty random sibling may require operator inspection.
pub fn create_private_new_file(path: &Path) -> io::Result<File> {
    platform_create_private_new_file(path)
}

/// Opens an existing final entry for read-only observation without following it.
///
/// The returned handle is bound to the final directory entry selected by the
/// operating system. Callers must inspect the handle metadata and reject links,
/// reparse points, directories, and other unsupported file kinds before reading.
/// Windows preserves ordinary read, write, and delete sharing so observation does
/// not impose a cleanup lock on another editor.
///
/// # Errors
///
/// Returns an operating-system error when the final entry does not exist or
/// cannot be opened without following a link or reparse point.
pub fn open_existing_no_follow(path: &Path) -> io::Result<File> {
    platform_open_existing_no_follow(path)
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

/// Opens an existing Windows entry for reconciliation and prevents mutation.
///
/// The returned handle denies new or existing write and delete access while it
/// remains open. Because Windows rename and unlink operations require delete
/// access, callers can ratify a canonical destination and keep that exact
/// pathname stable while cleaning related candidates by handle.
///
/// # Errors
///
/// Returns an operating-system error when the final entry cannot be opened
/// without following a reparse point or competing access prevents ratification.
#[cfg(windows)]
pub use imp::windows_open_for_reconciliation as open_for_reconciliation;

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

/// Restricts an exact open Unix file object to owner-only access.
///
/// This operation changes the mode through the live descriptor. On macOS it
/// also removes and verifies the absence of extended access-control entries.
/// It is used for recovery artifacts that cannot be deleted safely by path.
///
/// # Errors
///
/// Returns an operating-system error if the mode or access-control list cannot
/// be applied and verified on the open object.
#[cfg(unix)]
pub fn unix_restrict_open_file_to_owner(file: &File) -> io::Result<()> {
    platform_restrict_open_file_to_owner(file)
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

/// Successful commit result paired with its exact parent-directory barrier.
///
/// The parent receipt is deliberately one-shot and non-cloneable. After
/// splitting the result, callers may perform platform-safe verification before
/// consuming the parent token. The token intentionally exposes no basename
/// cleanup operation because a directory entry can rebind after commit.
#[must_use = "a successful commit receipt carries the exact parent-directory barrier"]
#[derive(Debug)]
pub struct CommitReceipt<T> {
    outcome: T,
    parent_sync: ParentSyncReceipt,
}

impl<T> CommitReceipt<T> {
    const fn new(outcome: T, parent_sync: ParentSyncReceipt) -> Self {
        Self {
            outcome,
            parent_sync,
        }
    }

    /// Splits the platform outcome from the one-shot parent-directory token.
    pub fn into_parts(self) -> (T, ParentSyncReceipt) {
        (self.outcome, self.parent_sync)
    }
}

/// One-shot persistence barrier for the exact parent used by a commit.
///
/// Unix receipts own the directory descriptor used by the descriptor-relative
/// commit. Windows receipts preserve the platform's explicit unsupported
/// result without pretending that a directory barrier exists.
#[must_use = "consume the receipt to complete or classify parent-directory durability"]
#[derive(Debug)]
pub struct ParentSyncReceipt {
    #[cfg(unix)]
    parent: File,
    _private: (),
}

impl ParentSyncReceipt {
    #[cfg(unix)]
    const fn from_open_parent(parent: File) -> Self {
        Self {
            parent,
            _private: (),
        }
    }

    #[cfg(windows)]
    const fn windows_unsupported() -> Self {
        Self { _private: () }
    }

    /// Synchronizes the exact directory used by the successful commit.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when the held Unix directory barrier
    /// fails. Windows returns [`ParentSyncOutcome::Unsupported`].
    #[allow(clippy::missing_const_for_fn)]
    pub fn sync(self) -> io::Result<ParentSyncOutcome> {
        #[cfg(unix)]
        {
            self.parent.sync_all()?;
            Ok(ParentSyncOutcome::Synced)
        }
        #[cfg(windows)]
        {
            Ok(ParentSyncOutcome::Unsupported)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "parent synchronization is unsupported on this operating system",
            ))
        }
    }
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
) -> io::Result<CommitReceipt<ReplaceExistingOutcome>> {
    platform_replace_existing(temporary, destination, backup)
}

/// Descriptor-bound Unix sibling directory used by recovery-only commits.
///
/// The directory is opened before the private stage is created. Creation is
/// descriptor-relative where the platform permits it; macOS instead uses its
/// atomic ACL-aware creation primitive and ratifies the result against this
/// descriptor. The consuming rename always uses the held directory, so a
/// pathname rebind cannot be acknowledged as a successful recovery commit.
#[cfg(unix)]
#[derive(Debug)]
pub struct UnixRecoveryCommitParent {
    parent: File,
    parent_path: PathBuf,
    destination_name: OsString,
}

#[cfg(unix)]
impl UnixRecoveryCommitParent {
    /// Opens and binds the destination's containing directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the destination has no basename or its containing
    /// directory cannot be opened.
    pub fn bind(destination: &Path) -> io::Result<Self> {
        let parent_path = unix_normalized_parent(destination).to_path_buf();
        let destination_name = destination.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "recovery destination has no filename",
            )
        })?;
        let parent = File::open(&parent_path)?;
        Ok(Self {
            parent,
            parent_path,
            destination_name: destination_name.to_os_string(),
        })
    }

    /// Exclusively creates one private stage in the bound directory.
    ///
    /// # Errors
    ///
    /// Returns an error when `temporary` is not a sibling of the bound
    /// destination or descriptor-relative private creation fails.
    pub fn create_private_new(&self, temporary: &Path) -> io::Result<File> {
        let temporary_name = self.require_sibling_name(temporary)?;
        #[cfg(target_os = "macos")]
        {
            // Apple's file-security creation API is path-based but prevents
            // inherited ACL access during creation. Ratify its result against
            // the already-open parent before allowing it to commit.
            let file = create_private_new_file(temporary)?;
            imp::unix_require_name_matches(&self.parent, temporary_name, &file)?;
            Ok(file)
        }
        #[cfg(not(target_os = "macos"))]
        {
            imp::unix_create_private_new_at(&self.parent, temporary_name)
        }
    }

    /// Consumes the exact staged object into the destination basename.
    ///
    /// The open stage is checked both before and after the rename. A competing
    /// basename replacement can therefore never be acknowledged as the staged
    /// recovery snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is not a sibling, the stage basename no
    /// longer identifies `staged`, or the descriptor-relative rename fails.
    pub fn replace_existing_consuming(
        self,
        temporary: &Path,
        staged: &File,
    ) -> io::Result<CommitReceipt<ReplaceExistingOutcome>> {
        let temporary_name = self.require_sibling_name(temporary)?.to_os_string();
        imp::unix_replace_existing_consuming_in_parent(
            self.parent,
            &temporary_name,
            &self.destination_name,
            staged,
        )
    }

    fn require_sibling_name<'a>(&self, path: &'a Path) -> io::Result<&'a std::ffi::OsStr> {
        if unix_normalized_parent(path) != self.parent_path {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "recovery stage and destination paths are not siblings",
            ));
        }
        path.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "recovery stage has no filename",
            )
        })
    }
}

#[cfg(unix)]
fn unix_normalized_parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}

/// Installs a private sibling only if the destination remains absent.
///
/// # Errors
///
/// Returns the raw operating-system failure. An `AlreadyExists` error is an
/// exclusive-create conflict, while other failures require reconciliation.
pub fn install_new(
    temporary: &Path,
    destination: &Path,
) -> io::Result<CommitReceipt<InstallNewOutcome>> {
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

/// Synchronizes the destination's currently resolved containing directory.
///
/// This path-based operation is for direct-create flows that have no commit
/// receipt. Callers of [`replace_existing`] and [`install_new`] must instead
/// consume the returned [`ParentSyncReceipt`] so a renamed or rebound parent
/// pathname cannot redirect the durability barrier.
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
        AtFlags, Gid, Mode, OFlags, RawMode, RenameFlags, Uid, fchmod, fchown, linkat, openat,
        renameat, renameat_with,
    };
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use xattr::FileExt;

    use super::{
        CommitReceipt, FileChangeToken, FileFacts, FileIdentity, IdentityQuality,
        InstallNewOutcome, ParentSyncOutcome, ParentSyncReceipt, ReplaceExistingOutcome,
        combine_disjoint_flag_bits,
    };

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const MAX_SUPPORTED_METADATA_BYTES: usize = 67_108_864;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const MAX_SUPPORTED_XATTR_COUNT: usize = 4096;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const MAX_XATTR_READ_ATTEMPTS: usize = 3;

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    mod macos_private_creation {
        use std::ffi::{CStr, CString};
        use std::fs::File;
        use std::io;
        use std::os::fd::FromRawFd;
        use std::os::unix::ffi::OsStrExt;
        use std::path::Path;

        type Acl = *mut libc::c_void;
        type FileSecurity = *mut libc::c_void;
        pub(super) type AclDeallocator = unsafe extern "C" fn(*mut libc::c_void) -> libc::c_int;
        pub(super) type FileSecurityDeallocator = unsafe extern "C" fn(FileSecurity);

        const FILESEC_MODE: libc::c_int = 4;
        const FILESEC_ACL: libc::c_int = 5;
        const PRIVATE_MODE: libc::mode_t = 0o600;
        const BOOTSTRAP_ACL: &CStr = c"!#acl 1 no_inherit\n";

        unsafe extern "C" {
            fn acl_from_text(buffer: *const libc::c_char) -> Acl;
            fn acl_free(object: *mut libc::c_void) -> libc::c_int;
            fn filesec_init() -> FileSecurity;
            fn filesec_free(filesec: FileSecurity);
            fn filesec_set_property(
                filesec: FileSecurity,
                property: libc::c_int,
                value: *const libc::c_void,
            ) -> libc::c_int;
            fn openx_np(
                path: *const libc::c_char,
                flags: libc::c_int,
                filesec: FileSecurity,
            ) -> libc::c_int;
        }

        pub(super) struct OwnedAcl {
            raw: Acl,
            deallocate: AclDeallocator,
        }

        impl OwnedAcl {
            fn parse(text: &CStr) -> io::Result<Self> {
                // SAFETY: `text` is a live NUL-terminated ACL representation.
                // A non-null result is owned until the guard releases it.
                let raw = unsafe { acl_from_text(text.as_ptr()) };
                if raw.is_null() {
                    return Err(io::Error::last_os_error());
                }
                Ok(Self {
                    raw,
                    deallocate: acl_free,
                })
            }

            #[cfg(test)]
            pub(super) const fn from_raw_with_deallocator(
                raw: Acl,
                deallocate: AclDeallocator,
            ) -> Self {
                Self { raw, deallocate }
            }

            pub(super) fn release(mut self) -> io::Result<()> {
                let raw = std::mem::replace(&mut self.raw, std::ptr::null_mut());
                // SAFETY: `raw` is the unique allocation returned by
                // `acl_from_text` and this consumes the guard exactly once.
                if unsafe { (self.deallocate)(raw.cast()) } != 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            }
        }

        impl Drop for OwnedAcl {
            fn drop(&mut self) {
                if !self.raw.is_null() {
                    // SAFETY: a non-null value remains the allocation uniquely
                    // owned by this guard. Drop is its final release path.
                    let _ = unsafe { (self.deallocate)(self.raw.cast()) };
                }
            }
        }

        pub(super) struct OwnedFileSecurity {
            raw: FileSecurity,
            deallocate: FileSecurityDeallocator,
        }

        impl OwnedFileSecurity {
            fn new() -> io::Result<Self> {
                // SAFETY: this allocation has no input pointers. A non-null
                // result is owned until the guard releases it.
                let raw = unsafe { filesec_init() };
                if raw.is_null() {
                    return Err(io::Error::last_os_error());
                }
                Ok(Self {
                    raw,
                    deallocate: filesec_free,
                })
            }

            #[cfg(test)]
            pub(super) const fn from_raw_with_deallocator(
                raw: FileSecurity,
                deallocate: FileSecurityDeallocator,
            ) -> Self {
                Self { raw, deallocate }
            }

            fn set_mode(&self, mode: libc::mode_t) -> io::Result<()> {
                // SAFETY: the descriptor is live and the mode pointer remains
                // valid for the complete property-copying call.
                if unsafe {
                    filesec_set_property(self.raw, FILESEC_MODE, std::ptr::from_ref(&mode).cast())
                } != 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            }

            fn set_acl(&self, acl: &OwnedAcl) -> io::Result<()> {
                let raw = acl.raw;
                // SAFETY: both objects are live. Apple's FILESEC_ACL contract
                // copies the pointed-to ACL, including its global flags.
                if unsafe {
                    filesec_set_property(self.raw, FILESEC_ACL, std::ptr::from_ref(&raw).cast())
                } != 0
                {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            }

            fn create(&self, path: &CStr) -> io::Result<File> {
                let flags = libc::O_CLOEXEC
                    | libc::O_CREAT
                    | libc::O_EXCL
                    | libc::O_NOFOLLOW
                    | libc::O_RDWR;
                // SAFETY: the path and file-security descriptor are live for
                // the call. On success the returned descriptor is uniquely
                // owned and wrapped exactly once below.
                let descriptor = unsafe { openx_np(path.as_ptr(), flags, self.raw) };
                if descriptor == -1 {
                    return Err(io::Error::last_os_error());
                }
                // SAFETY: `openx_np` returned a new owned descriptor and no
                // other Rust value owns or will close it.
                Ok(unsafe { File::from_raw_fd(descriptor) })
            }
        }

        impl Drop for OwnedFileSecurity {
            fn drop(&mut self) {
                // SAFETY: this is the unique live allocation returned by
                // `filesec_init`; the API's deallocator returns no status.
                unsafe { (self.deallocate)(self.raw) };
            }
        }

        pub fn create(path: &Path) -> io::Result<File> {
            let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "file path contains an interior NUL byte",
                )
            })?;
            let acl = OwnedAcl::parse(BOOTSTRAP_ACL)?;
            let security = OwnedFileSecurity::new()?;
            security.set_mode(PRIVATE_MODE)?;
            security.set_acl(&acl)?;
            acl.release()?;
            security.create(&path)
        }

        #[cfg(test)]
        mod tests {
            use std::ffi::CString;
            use std::io;
            use std::os::unix::{ffi::OsStrExt, fs::MetadataExt};

            use tempfile::tempdir;

            use super::OwnedFileSecurity;

            #[test]
            fn configured_mode_is_applied_during_creation() -> io::Result<()> {
                const READ_ONLY_MODE: libc::mode_t = 0o400;
                const PERMISSION_BITS: u32 = 0o7777;

                let directory = tempdir()?;
                let path = directory.path().join("mode-fixture.txt");
                let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "test file path contains an interior NUL byte",
                    )
                })?;
                let security = OwnedFileSecurity::new()?;
                security.set_mode(READ_ONLY_MODE)?;

                let file = security.create(&path)?;

                assert_eq!(
                    file.metadata()?.mode() & PERMISSION_BITS,
                    u32::from(READ_ONLY_MODE)
                );
                Ok(())
            }
        }
    }

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

    #[cfg(not(target_os = "macos"))]
    pub fn unix_create_private_new_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }

    #[cfg(not(target_os = "macos"))]
    pub fn unix_create_private_new_at(parent: &File, name: &OsStr) -> io::Result<File> {
        let file = File::from(
            openat(parent, name, unix_private_create_flags(), Mode::empty())
                .map_err(io::Error::from)?,
        );
        if let Err(error) = unix_restrict_open_file_to_owner(&file) {
            return Err(super::retained_private_creation_error(error));
        }
        Ok(file)
    }

    const fn unix_combine_disjoint_flags(left: OFlags, right: OFlags) -> OFlags {
        OFlags::from_bits_retain(combine_disjoint_flag_bits(left.bits(), right.bits()))
    }

    const fn unix_private_create_flags() -> OFlags {
        unix_combine_disjoint_flags(
            unix_combine_disjoint_flags(
                unix_combine_disjoint_flags(
                    unix_combine_disjoint_flags(OFlags::RDWR, OFlags::CREATE),
                    OFlags::EXCL,
                ),
                OFlags::NOFOLLOW,
            ),
            OFlags::CLOEXEC,
        )
    }

    const fn unix_existing_read_flags() -> OFlags {
        unix_combine_disjoint_flags(
            unix_combine_disjoint_flags(
                unix_combine_disjoint_flags(OFlags::RDONLY, OFlags::NOFOLLOW),
                OFlags::NONBLOCK,
            ),
            OFlags::CLOEXEC,
        )
    }

    #[cfg(target_os = "macos")]
    pub fn macos_create_private_new_file(path: &Path) -> io::Result<File> {
        let file = macos_private_creation::create(path)?;
        if let Err(error) = macos_finalize_private_creation(&file) {
            return Err(super::retained_private_creation_error(error));
        }
        Ok(file)
    }

    #[cfg(target_os = "macos")]
    fn macos_finalize_private_creation(file: &File) -> io::Result<()> {
        unix_restrict_open_file_to_owner(file)
    }

    pub fn unix_restrict_open_file_to_owner(file: &File) -> io::Result<()> {
        const PRIVATE_MODE: u32 = 0o600;
        const PERMISSION_BITS: u32 = 0o7777;

        unix_apply_mode(PRIVATE_MODE, file)?;
        if file.metadata()?.mode() & PERMISSION_BITS != PRIVATE_MODE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private file mode differs after owner-only restriction",
            ));
        }
        #[cfg(target_os = "macos")]
        macos_restrict_open_file_acl_to_owner(file)?;
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn macos_restrict_open_file_acl_to_owner(file: &File) -> io::Result<()> {
        remove_macos_acl(file)?;
        if read_macos_acl_snapshot(file)? != MacosAclSnapshot::Absent {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "private file retains an access control list after owner-only restriction",
            ));
        }
        Ok(())
    }

    pub fn unix_open_for_cleanup(path: &Path) -> io::Result<File> {
        unix_open_existing_no_follow(path)
    }

    pub fn unix_open_existing_no_follow(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            // O_NONBLOCK prevents a final-entry swap to a FIFO from hanging
            // between pathname classification and open. It has no effect on
            // ordinary regular-file reads after handle validation.
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
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
        acl: MacosAclSnapshot,
    }

    #[cfg(target_os = "macos")]
    #[derive(Debug, Eq, PartialEq)]
    enum MacosAclSnapshot {
        Absent,
        Present(Vec<u8>),
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
        let acl = read_macos_acl_snapshot(source)?;
        unix_ensure_metadata_source_matches(source, stamp, expected_source, "during capture")?;
        Ok(RequiredMetadata {
            stamp,
            attributes,
            acl,
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
        unix_apply_ownership(metadata.stamp, destination, |file, owner, group| {
            fchown(file, owner, group)
        })?;
        unix_apply_native_xattrs(&metadata.attributes, destination)?;
        unix_apply_mode(metadata.stamp.mode, destination)?;
        unix_verify_native_xattrs(&metadata.attributes, destination)?;
        unix_verify_destination_stamp(metadata.stamp, destination)
    }

    #[cfg(target_os = "macos")]
    fn apply_macos_metadata(metadata: &RequiredMetadata, destination: &File) -> io::Result<()> {
        unix_apply_ownership(metadata.stamp, destination, |file, owner, group| {
            fchown(file, owner, group)
        })?;
        apply_macos_acl_snapshot(&metadata.acl, destination)?;
        unix_apply_native_xattrs(&metadata.attributes, destination)?;
        unix_apply_mode(metadata.stamp.mode, destination)?;
        unix_verify_native_xattrs(&metadata.attributes, destination)?;
        verify_macos_acl(&metadata.acl, destination)?;
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
                && metadata.acl == read_macos_acl_snapshot(source)?,
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
    ) -> io::Result<CommitReceipt<ReplaceExistingOutcome>> {
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

    pub fn unix_replace_existing_consuming_in_parent(
        parent: File,
        temporary_name: &OsStr,
        destination_name: &OsStr,
        staged: &File,
    ) -> io::Result<CommitReceipt<ReplaceExistingOutcome>> {
        let staged_identity = unix_file_facts(staged)?.identity();
        unix_require_name_matches(&parent, temporary_name, staged)?;

        renameat(&parent, temporary_name, &parent, destination_name).map_err(io::Error::from)?;
        let committed = unix_open_existing_at(&parent, destination_name)?;
        if unix_file_facts(&committed)?.identity() != staged_identity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "recovery destination does not identify the staged object after rename",
            ));
        }
        drop(committed);
        Ok(CommitReceipt::new(
            ReplaceExistingOutcome::Clean,
            ParentSyncReceipt::from_open_parent(parent),
        ))
    }

    pub fn unix_require_name_matches(
        parent: &File,
        name: &OsStr,
        expected: &File,
    ) -> io::Result<()> {
        let expected_identity = unix_file_facts(expected)?.identity();
        let named = unix_open_existing_at(parent, name)?;
        if unix_file_facts(&named)?.identity() != expected_identity {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "recovery stage basename no longer identifies the staged object",
            ));
        }
        Ok(())
    }

    fn unix_open_existing_at(parent: &File, name: &OsStr) -> io::Result<File> {
        openat(parent, name, unix_existing_read_flags(), Mode::empty())
            .map(File::from)
            .map_err(io::Error::from)
    }

    pub fn unix_install_new(
        temporary: &Path,
        destination: &Path,
    ) -> io::Result<CommitReceipt<InstallNewOutcome>> {
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
                Err(error) => {
                    if unix_no_replace_is_unavailable(error) {
                        unix_install_new_with_link(parent, temporary_name, destination_name)
                    } else {
                        Err(error.into())
                    }
                }
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
    ) -> io::Result<CommitReceipt<T>> {
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
        let outcome = operation(&parent, temporary_name, destination_name)?;
        Ok(CommitReceipt::new(
            outcome,
            ParentSyncReceipt::from_open_parent(parent),
        ))
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

    fn unix_apply_ownership<E>(
        metadata: MetadataStamp,
        destination: &File,
        apply: impl FnOnce(&File, Option<Uid>, Option<Gid>) -> Result<(), E>,
    ) -> io::Result<()>
    where
        E: Into<io::Error>,
    {
        let destination_metadata = destination.metadata()?;
        let owner =
            (metadata.uid != destination_metadata.uid()).then(|| Uid::from_raw(metadata.uid));
        let group =
            (metadata.gid != destination_metadata.gid()).then(|| Gid::from_raw(metadata.gid));
        if owner.is_some() || group.is_some() {
            apply(destination, owner, group).map_err(Into::into)?;
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
                if unix_xattr_name_is_listed(&names, OsStr::new(name)) {
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
                    if unix_xattr_count_reached_limit(attributes.len(), MAX_SUPPORTED_XATTR_COUNT) {
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
            if unix_xattr_call_failed(announced) {
                return Err(io::Error::last_os_error());
            }
            let announced = usize::try_from(announced).map_err(|_| unix_metadata_too_large())?;
            if unix_xattr_size_exceeds_limit(announced, byte_limit) {
                return Err(unix_metadata_too_large());
            }

            let mut buffer = Vec::new();
            buffer
                .try_reserve_exact(announced)
                .map_err(|_| unix_metadata_allocation_failed())?;
            buffer.resize(announced, 0_u8);
            let read = unix_flistxattr(file, Some(&mut buffer));
            if unix_xattr_call_failed(read) {
                let error = io::Error::last_os_error();
                if unix_xattr_read_should_retry(&error) {
                    continue;
                }
                return Err(error);
            }
            let read = usize::try_from(read).map_err(|_| unix_metadata_too_large())?;
            if unix_xattr_size_exceeds_limit(read, buffer.len()) {
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
            if unix_xattr_call_failed(announced) {
                let error = io::Error::last_os_error();
                if unix_xattr_is_missing(&error) {
                    return Ok(None);
                }
                return Err(error);
            }
            let announced = usize::try_from(announced).map_err(|_| unix_metadata_too_large())?;
            if unix_xattr_size_exceeds_limit(announced, byte_limit) {
                return Err(unix_metadata_too_large());
            }

            let mut value = Vec::new();
            value
                .try_reserve_exact(announced)
                .map_err(|_| unix_metadata_allocation_failed())?;
            value.resize(announced, 0_u8);
            let read = unix_fgetxattr(file, &name, Some(&mut value));
            if unix_xattr_call_failed(read) {
                let error = io::Error::last_os_error();
                if unix_xattr_read_should_retry(&error) {
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
    const fn unix_xattr_call_failed(result: libc::ssize_t) -> bool {
        result < 0
    }

    #[cfg(target_os = "linux")]
    const fn unix_xattr_count_reached_limit(count: usize, limit: usize) -> bool {
        count >= limit
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    const fn unix_xattr_size_exceeds_limit(size: usize, limit: usize) -> bool {
        size > limit
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn unix_xattr_read_should_retry(error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::ERANGE)
    }

    #[cfg(target_os = "linux")]
    fn unix_xattr_name_is_listed(names: &[OsString], expected: &OsStr) -> bool {
        names.iter().any(|name| name == expected)
    }

    #[cfg(target_os = "linux")]
    use self::linux_xattr_is_missing as unix_xattr_is_missing;
    #[cfg(target_os = "macos")]
    use self::macos_xattr_is_missing as unix_xattr_is_missing;

    #[cfg(target_os = "linux")]
    fn linux_xattr_is_missing(error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::ENODATA)
    }

    #[cfg(target_os = "macos")]
    fn macos_xattr_is_missing(error: &io::Error) -> bool {
        error.raw_os_error() == Some(libc::ENOATTR)
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
    fn remove_macos_acl(destination: &File) -> io::Result<()> {
        type Acl = *mut libc::c_void;

        unsafe extern "C" {
            fn acl_set_fd(fd: libc::c_int, acl: Acl) -> libc::c_int;
        }

        // macOS exposes `_FILESEC_REMOVE_ACL` as the opaque pointer value 1.
        // It instructs `acl_set_fd` to write the kernel's distinct no-ACL
        // sentinel and is not an allocation that may be passed to `acl_free`.
        let remove_acl = std::ptr::without_provenance_mut::<libc::c_void>(1);
        // SAFETY: the descriptor is live and the sentinel is the exact public
        // macOS contract for removing its extended ACL.
        if unsafe { acl_set_fd(destination.as_raw_fd(), remove_acl) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn apply_macos_acl_snapshot(snapshot: &MacosAclSnapshot, destination: &File) -> io::Result<()> {
        type Acl = *mut libc::c_void;

        unsafe extern "C" {
            fn acl_from_text(buffer: *const libc::c_char) -> Acl;
            fn acl_set_fd(fd: libc::c_int, acl: Acl) -> libc::c_int;
            fn acl_free(object: *mut libc::c_void) -> libc::c_int;
        }

        let MacosAclSnapshot::Present(text) = snapshot else {
            return remove_macos_acl(destination);
        };
        let text = CString::new(text.as_slice()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "captured access control list contains an interior NUL byte",
            )
        })?;
        // SAFETY: `text` is a live NUL-terminated string. The returned ACL is
        // either null with an OS error or an allocation released below.
        let acl = unsafe { acl_from_text(text.as_ptr()) };
        if acl.is_null() {
            return Err(io::Error::last_os_error());
        }

        // SAFETY: the descriptor and parsed ACL remain live for the call. The
        // operation changes only the destination's descriptor-resolved ACL.
        let set_result = unsafe { acl_set_fd(destination.as_raw_fd(), acl) };
        let set_error = (set_result != 0).then(io::Error::last_os_error);
        // SAFETY: `acl` is the live allocation returned above and is freed once.
        let free_result = unsafe { acl_free(acl.cast()) };
        if let Some(error) = set_error {
            return Err(error);
        }
        if free_result != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    const fn macos_acl_release_failed(
        text_free_result: libc::c_int,
        acl_free_result: libc::c_int,
    ) -> bool {
        text_free_result != 0 || acl_free_result != 0
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn read_macos_acl_snapshot(file: &File) -> io::Result<MacosAclSnapshot> {
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
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ENOENT) {
                return Ok(MacosAclSnapshot::Absent);
            }
            return Err(error);
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
        if macos_acl_release_failed(text_free, acl_free_result) {
            return Err(io::Error::last_os_error());
        }
        Ok(MacosAclSnapshot::Present(result))
    }

    #[cfg(target_os = "macos")]
    fn verify_macos_acl(expected: &MacosAclSnapshot, destination: &File) -> io::Result<()> {
        if expected != &read_macos_acl_snapshot(destination)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "access control list differs after metadata transfer",
            ));
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        #[cfg(target_os = "macos")]
        use std::fs::File;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        use std::io;
        #[cfg(target_os = "macos")]
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        #[cfg(target_os = "macos")]
        use std::sync::atomic::{AtomicUsize, Ordering};

        #[cfg(target_os = "macos")]
        use tempfile::tempdir;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        use tempfile::tempfile;
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        use xattr::FileExt;

        #[cfg(target_os = "linux")]
        use super::linux_xattr_is_missing as native_xattr_is_missing;
        #[cfg(target_os = "macos")]
        use super::macos_xattr_is_missing as native_xattr_is_missing;
        use super::{
            ExtendedAttribute, MetadataStamp, OFlags, unix_existing_read_flags,
            unix_metadata_payload_stamp_matches, unix_metadata_source_matches, unix_metadata_stamp,
            unix_no_replace_is_unavailable, unix_private_create_flags,
            unix_verify_destination_stamp, unix_verify_native_xattrs, unix_xattr_call_failed,
            unix_xattr_read_should_retry, unix_xattr_size_exceeds_limit,
        };
        #[cfg(target_os = "linux")]
        use super::{
            Gid, Uid, unix_apply_ownership, unix_xattr_count_reached_limit,
            unix_xattr_name_is_listed,
        };
        #[cfg(target_os = "macos")]
        use super::{
            MacosAclSnapshot, apply_macos_acl_snapshot, macos_acl_release_failed,
            macos_finalize_private_creation, macos_private_creation, read_macos_acl_snapshot,
            verify_macos_acl,
        };
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        use super::{unix_read_native_xattr_bounded, unix_reserve_metadata_bytes};
        use crate::{FileChangeToken, FileFacts, FileIdentity, IdentityQuality};

        #[cfg(target_os = "macos")]
        static ACL_DROP_DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
        #[cfg(target_os = "macos")]
        static ACL_RELEASE_DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);
        #[cfg(target_os = "macos")]
        static FILESEC_DEALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

        #[cfg(target_os = "macos")]
        #[allow(unsafe_code)]
        unsafe extern "C" fn record_acl_deallocation(_raw: *mut libc::c_void) -> libc::c_int {
            ACL_DROP_DEALLOCATIONS.fetch_add(1, Ordering::SeqCst);
            0
        }

        #[cfg(target_os = "macos")]
        #[allow(unsafe_code)]
        unsafe extern "C" fn reject_acl_deallocation(_raw: *mut libc::c_void) -> libc::c_int {
            ACL_RELEASE_DEALLOCATIONS.fetch_add(1, Ordering::SeqCst);
            -1
        }

        #[cfg(target_os = "macos")]
        #[allow(unsafe_code)]
        unsafe extern "C" fn record_filesec_deallocation(_raw: *mut libc::c_void) {
            FILESEC_DEALLOCATIONS.fetch_add(1, Ordering::SeqCst);
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn macos_creation_guards_dispatch_exact_deallocators() {
            ACL_DROP_DEALLOCATIONS.store(0, Ordering::SeqCst);
            FILESEC_DEALLOCATIONS.store(0, Ordering::SeqCst);
            let sentinel = std::ptr::without_provenance_mut::<libc::c_void>(17);

            drop(macos_private_creation::OwnedAcl::from_raw_with_deallocator(
                sentinel,
                record_acl_deallocation,
            ));
            drop(
                macos_private_creation::OwnedFileSecurity::from_raw_with_deallocator(
                    sentinel,
                    record_filesec_deallocation,
                ),
            );

            assert_eq!(ACL_DROP_DEALLOCATIONS.load(Ordering::SeqCst), 1);
            assert_eq!(FILESEC_DEALLOCATIONS.load(Ordering::SeqCst), 1);
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn macos_acl_release_propagates_deallocation_failure_once() {
            ACL_RELEASE_DEALLOCATIONS.store(0, Ordering::SeqCst);
            let sentinel = std::ptr::without_provenance_mut::<libc::c_void>(19);
            let acl = macos_private_creation::OwnedAcl::from_raw_with_deallocator(
                sentinel,
                reject_acl_deallocation,
            );

            assert!(acl.release().is_err());
            assert_eq!(ACL_RELEASE_DEALLOCATIONS.load(Ordering::SeqCst), 1);
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn macos_acl_release_requires_both_deallocations_to_succeed() {
            assert!(!macos_acl_release_failed(0, 0));
            assert!(macos_acl_release_failed(-1, 0));
            assert!(macos_acl_release_failed(0, -1));
            assert!(macos_acl_release_failed(-1, -1));
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn private_creation_and_owner_restriction_remove_inherited_acl() -> io::Result<()> {
            const PRIVATE_MODE: u32 = 0o600;
            const PERMISSION_BITS: u32 = 0o7777;

            let directory = tempdir()?;
            let clear_status = std::process::Command::new("/bin/chmod")
                .arg("-N")
                .arg(directory.path())
                .status()?;
            if !clear_status.success() {
                return Err(io::Error::other(format!(
                    "chmod failed to clear the parent ACL fixture: {clear_status}"
                )));
            }
            let status = std::process::Command::new("/bin/chmod")
                .args(["+a", "everyone allow read,file_inherit"])
                .arg(directory.path())
                .status()?;
            if !status.success() {
                return Err(io::Error::other(format!(
                    "chmod failed to create the inheritable ACL fixture: {status}"
                )));
            }

            let inherited = File::create(directory.path().join("inherited.txt"))?;
            let MacosAclSnapshot::Present(inherited_acl) = read_macos_acl_snapshot(&inherited)?
            else {
                return Err(io::Error::other(
                    "the control file did not inherit the parent ACL",
                ));
            };
            assert!(!inherited_acl.is_empty());
            assert_ne!(inherited_acl, b"!#acl 1\n");

            let protected_path = directory.path().join("protected.txt");
            let protected = macos_private_creation::create(&protected_path)?;
            assert_eq!(
                read_macos_acl_snapshot(&protected)?,
                MacosAclSnapshot::Absent
            );
            let creation_mode = protected.metadata()?.mode() & PERMISSION_BITS;
            assert_eq!(creation_mode & !PRIVATE_MODE, 0);
            assert_eq!(protected.metadata()?.len(), 0);

            let mut relaxed_permissions = protected.metadata()?.permissions();
            relaxed_permissions.set_mode(0o644);
            protected.set_permissions(relaxed_permissions)?;
            assert_eq!(protected.metadata()?.mode() & PERMISSION_BITS, 0o644);

            macos_finalize_private_creation(&protected)?;
            assert_eq!(protected.metadata()?.mode() & PERMISSION_BITS, PRIVATE_MODE);
            assert_eq!(
                read_macos_acl_snapshot(&protected)?,
                MacosAclSnapshot::Absent
            );
            assert_eq!(protected.metadata()?.len(), 0);

            let mut inherited_permissions = inherited.metadata()?.permissions();
            inherited_permissions.set_mode(0o644);
            inherited.set_permissions(inherited_permissions)?;
            macos_finalize_private_creation(&inherited)?;
            assert_eq!(inherited.metadata()?.mode() & PERMISSION_BITS, PRIVATE_MODE);
            assert_eq!(
                read_macos_acl_snapshot(&inherited)?,
                MacosAclSnapshot::Absent
            );
            Ok(())
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn acl_verification_rejects_a_missing_expected_acl() -> io::Result<()> {
            let file = tempfile()?;
            let expected = MacosAclSnapshot::Present(b"expected ACL fixture".to_vec());

            let error = verify_macos_acl(&expected, &file).expect_err("ACLs should differ");

            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            Ok(())
        }

        #[cfg(target_os = "macos")]
        #[test]
        fn explicit_empty_acl_snapshot_canonicalizes_to_absence() -> io::Result<()> {
            let directory = tempdir()?;
            let file = File::create(directory.path().join("empty-acl.txt"))?;
            let snapshot = MacosAclSnapshot::Present(b"!#acl 1\n".to_vec());

            apply_macos_acl_snapshot(&snapshot, &file)?;

            assert_eq!(read_macos_acl_snapshot(&file)?, MacosAclSnapshot::Absent);
            Ok(())
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        #[test]
        fn native_missing_xattr_classifier_is_exact() {
            #[cfg(target_os = "linux")]
            const MISSING_XATTR_ERROR: i32 = libc::ENODATA;
            #[cfg(target_os = "macos")]
            const MISSING_XATTR_ERROR: i32 = libc::ENOATTR;

            assert!(native_xattr_is_missing(&io::Error::from_raw_os_error(
                MISSING_XATTR_ERROR
            )));
            assert!(!native_xattr_is_missing(&io::Error::from_raw_os_error(
                libc::ENOENT
            )));
        }

        #[test]
        fn no_replace_fallback_classifier_is_exact() {
            assert!(unix_no_replace_is_unavailable(rustix::io::Errno::NOSYS));
            assert!(unix_no_replace_is_unavailable(rustix::io::Errno::INVAL));
            assert!(unix_no_replace_is_unavailable(rustix::io::Errno::NOTSUP));
            assert!(!unix_no_replace_is_unavailable(rustix::io::Errno::EXIST));
            assert!(!unix_no_replace_is_unavailable(rustix::io::Errno::PERM));
        }

        #[test]
        fn descriptor_relative_open_flag_policies_are_exact() {
            assert_eq!(
                unix_private_create_flags(),
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC
            );
            assert_eq!(
                unix_existing_read_flags(),
                OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC
            );
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        #[test]
        fn native_xattr_result_boundaries_are_exact() {
            assert!(unix_xattr_call_failed(-1));
            assert!(!unix_xattr_call_failed(0));
            assert!(!unix_xattr_call_failed(1));

            #[cfg(target_os = "linux")]
            {
                assert!(!unix_xattr_count_reached_limit(4, 5));
                assert!(unix_xattr_count_reached_limit(5, 5));
                assert!(unix_xattr_count_reached_limit(6, 5));
            }

            assert!(!unix_xattr_size_exceeds_limit(4, 5));
            assert!(!unix_xattr_size_exceeds_limit(5, 5));
            assert!(unix_xattr_size_exceeds_limit(6, 5));

            assert!(unix_xattr_read_should_retry(&io::Error::from_raw_os_error(
                libc::ERANGE
            )));
            assert!(!unix_xattr_read_should_retry(
                &io::Error::from_raw_os_error(libc::ENOENT)
            ));
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn critical_xattr_name_membership_is_exact() {
            let names = vec![std::ffi::OsString::from("security.capability")];

            assert!(unix_xattr_name_is_listed(
                &names,
                std::ffi::OsStr::new("security.capability")
            ));
            assert!(!unix_xattr_name_is_listed(
                &names,
                std::ffi::OsStr::new("security.selinux")
            ));
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn ownership_application_requests_and_propagates_each_change() -> io::Result<()> {
            let file = tempfile()?;
            let actual = unix_metadata_stamp(&file)?;
            let replacement_owner = u32::from(actual.uid == 0);
            let replacement_group = u32::from(actual.gid == 0);
            let cases = [
                (actual, None, None),
                (
                    MetadataStamp {
                        uid: replacement_owner,
                        ..actual
                    },
                    Some(replacement_owner),
                    None,
                ),
                (
                    MetadataStamp {
                        gid: replacement_group,
                        ..actual
                    },
                    None,
                    Some(replacement_group),
                ),
                (
                    MetadataStamp {
                        uid: replacement_owner,
                        gid: replacement_group,
                        ..actual
                    },
                    Some(replacement_owner),
                    Some(replacement_group),
                ),
            ];

            for (expected, expected_owner, expected_group) in cases {
                let mut requested = None;
                let result = unix_apply_ownership(expected, &file, |_, owner, group| {
                    requested = Some((owner.map(Uid::as_raw), group.map(Gid::as_raw)));
                    Err(io::Error::other("injected ownership failure"))
                });

                if expected_owner.is_none() && expected_group.is_none() {
                    assert!(result.is_ok());
                    assert_eq!(requested, None);
                } else {
                    assert_eq!(
                        result
                            .expect_err("each requested ownership change must propagate failure")
                            .kind(),
                        io::ErrorKind::Other
                    );
                    assert_eq!(requested, Some((expected_owner, expected_group)));
                }
            }
            Ok(())
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        #[test]
        fn destination_stamp_verification_rejects_each_mismatch() -> io::Result<()> {
            let file = tempfile()?;
            let actual = unix_metadata_stamp(&file)?;
            let mismatches = [
                MetadataStamp {
                    uid: actual.uid.wrapping_add(1),
                    ..actual
                },
                MetadataStamp {
                    gid: actual.gid.wrapping_add(1),
                    ..actual
                },
                MetadataStamp {
                    mode: actual.mode ^ 0o100,
                    ..actual
                },
            ];

            for mismatch in mismatches {
                assert_eq!(
                    unix_verify_destination_stamp(mismatch, &file)
                        .expect_err("each destination-stamp mismatch must be rejected")
                        .kind(),
                    io::ErrorKind::InvalidData
                );
            }
            Ok(())
        }

        #[cfg(any(target_os = "linux", target_os = "macos"))]
        #[test]
        fn native_xattr_verification_rejects_a_missing_attribute() -> io::Result<()> {
            let file = tempfile()?;
            let expected = [ExtendedAttribute {
                name: std::ffi::OsString::from(if cfg!(target_os = "linux") {
                    "user.noter.expected"
                } else {
                    "com.noter.expected"
                }),
                value: b"required value".to_vec(),
            }];

            assert_eq!(
                unix_verify_native_xattrs(&expected, &file)
                    .expect_err("a missing expected xattr must be rejected")
                    .kind(),
                io::ErrorKind::InvalidData
            );
            Ok(())
        }

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
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::path::Path;

    use windows_sys::Win32::Foundation::{
        ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_FUNCTION, ERROR_INVALID_PARAMETER,
        ERROR_NOT_SUPPORTED, ERROR_SUCCESS, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
        LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
        ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SDDL_REVISION_1,
        SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, GetTokenInformation, OWNER_SECURITY_INFORMATION,
        PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CREATE_NEW, CreateFileW, DELETE, FILE_ATTRIBUTE_NORMAL,
        FILE_BASIC_INFO, FILE_DISPOSITION_FLAG_DELETE, FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
        FILE_DISPOSITION_INFO, FILE_DISPOSITION_INFO_EX, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_ID_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FileBasicInfo,
        FileDispositionInfo, FileDispositionInfoEx, FileIdInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx, MOVEFILE_WRITE_THROUGH, MoveFileExW, ReplaceFileW,
        SetFileInformationByHandle,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    use super::{
        CommitReceipt, FileChangeToken, FileFacts, FileIdentity, IdentityQuality,
        InstallNewOutcome, ParentSyncOutcome, ParentSyncReceipt, ReplaceExistingOutcome,
    };

    const MAX_SID_STRING_UNITS: usize = 256;
    const SYSTEM_SID: &str = "S-1-5-18";

    const fn windows_combine_disjoint_flags(left: u32, right: u32) -> u32 {
        assert!(
            left & right == 0,
            "overlapping Windows flags cannot be combined as disjoint values"
        );
        left + right
    }

    const WINDOWS_PRIVATE_FILE_ACCESS: u32 = windows_combine_disjoint_flags(
        windows_combine_disjoint_flags(GENERIC_READ, GENERIC_WRITE),
        DELETE,
    );
    const WINDOWS_PRIVATE_FILE_SHARE: u32 =
        windows_combine_disjoint_flags(FILE_SHARE_READ, FILE_SHARE_DELETE);
    const WINDOWS_PRIVATE_SECURITY_INFORMATION: u32 =
        windows_combine_disjoint_flags(OWNER_SECURITY_INFORMATION, DACL_SECURITY_INFORMATION);

    type LocalDeallocator = unsafe extern "system" fn(
        windows_sys::Win32::Foundation::HLOCAL,
    )
        -> windows_sys::Win32::Foundation::HLOCAL;

    struct WindowsSecurityDescriptor {
        raw: PSECURITY_DESCRIPTOR,
        deallocate: LocalDeallocator,
    }

    struct WindowsLocalWideString {
        raw: *mut u16,
        deallocate: LocalDeallocator,
    }

    struct WindowsPrivateSecurityPolicy {
        owner_sid: String,
        dacl_sddl: String,
        descriptor_sddl: String,
    }

    impl Drop for WindowsSecurityDescriptor {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: the SDDL conversion and GetSecurityInfo APIs return
            // LocalAlloc-owned descriptors. This guard retains sole ownership
            // of either form and frees it exactly once.
            let _ = unsafe { (self.deallocate)(self.raw.cast()) };
        }
    }

    impl Drop for WindowsLocalWideString {
        #[allow(unsafe_code)]
        fn drop(&mut self) {
            // SAFETY: the conversion API returned this LocalAlloc-owned string
            // to this guard, which frees it exactly once.
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

    pub fn windows_create_private_new_file(path: &Path) -> io::Result<File> {
        let policy = windows_private_security_policy()?;
        let file = windows_create_new_file_with_security(path, &policy.descriptor_sddl)?;
        windows_finalize_private_creation(
            file,
            |file| windows_verify_private_file_security(file, &policy.owner_sid, &policy.dacl_sddl),
            windows_delete_open_file,
        )
    }

    fn windows_private_security_policy() -> io::Result<WindowsPrivateSecurityPolicy> {
        let owner_sid = windows_current_process_user_sid()?;
        windows_private_security_policy_for_sid(owner_sid)
    }

    fn windows_private_security_policy_for_sid(
        owner_sid: String,
    ) -> io::Result<WindowsPrivateSecurityPolicy> {
        let requested_dacl = if owner_sid == SYSTEM_SID {
            "D:P(A;;FA;;;SY)".to_owned()
        } else {
            format!("D:P(A;;FA;;;SY)(A;;FA;;;{owner_sid})")
        };
        let descriptor_sddl = format!("O:{owner_sid}{requested_dacl}");
        let descriptor = windows_security_descriptor_from_sddl(&descriptor_sddl)?;
        let dacl_sddl = windows_descriptor_dacl_sddl(descriptor.raw)?;
        Ok(WindowsPrivateSecurityPolicy {
            owner_sid,
            dacl_sddl,
            descriptor_sddl,
        })
    }

    fn windows_token_user_buffer_length(required_length: u32) -> io::Result<usize> {
        let required_length = usize::try_from(required_length).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "token-user data length does not fit memory",
            )
        })?;
        if required_length < size_of::<TOKEN_USER>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows returned undersized token-user data",
            ));
        }
        Ok(required_length)
    }

    fn windows_validate_token_user_returned_length(
        returned_length: u32,
        buffer_length: u32,
    ) -> io::Result<()> {
        if returned_length > buffer_length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows returned token-user data larger than its buffer",
            ));
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn windows_current_process_user_sid() -> io::Result<String> {
        let mut raw_token = std::ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a valid pseudo-handle and the token
        // output points to writable storage owned by this function.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut raw_token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: OpenProcessToken returned a unique, valid owned handle.
        let token = unsafe { OwnedHandle::from_raw_handle(raw_token) };

        let mut required_length = 0_u32;
        // SAFETY: a zero-length query may use a null output buffer, and the
        // required-length output points to writable storage.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                std::ptr::null_mut(),
                0,
                &raw mut required_length,
            )
        } != 0
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows unexpectedly returned token-user data without a buffer",
            ));
        }
        let sizing_error = io::Error::last_os_error();
        if sizing_error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER.cast_signed()) {
            return Err(sizing_error);
        }
        let required_length = windows_token_user_buffer_length(required_length)?;
        let word_count = required_length.div_ceil(size_of::<usize>());
        let mut buffer = vec![0_usize; word_count];
        let buffer_length = u32::try_from(required_length).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "token-user buffer length does not fit the Windows API parameter",
            )
        })?;
        let mut returned_length = buffer_length;
        // SAFETY: the word buffer is suitably aligned and large enough for the
        // requested bytes, and the returned-length output is writable.
        if unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                buffer.as_mut_ptr().cast(),
                buffer_length,
                &raw mut returned_length,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        windows_validate_token_user_returned_length(returned_length, buffer_length)?;
        // SAFETY: GetTokenInformation initialized a TOKEN_USER at the aligned
        // start of the live buffer, which remains in scope for SID conversion.
        let token_user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        windows_sid_string(token_user.User.Sid)
    }

    #[allow(unsafe_code)]
    fn windows_create_new_file_with_security(path: &Path, sddl: &str) -> io::Result<File> {
        let path = windows_wide_path(path)?;
        let descriptor = windows_security_descriptor_from_sddl(sddl)?;
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
                WINDOWS_PRIVATE_FILE_ACCESS,
                WINDOWS_PRIVATE_FILE_SHARE,
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

    fn windows_finalize_private_creation(
        file: File,
        verify: impl FnOnce(&File) -> io::Result<()>,
        cleanup: impl FnOnce(&File) -> io::Result<()>,
    ) -> io::Result<File> {
        let Err(cause) = verify(&file) else {
            return Ok(file);
        };
        match cleanup(&file) {
            Ok(()) => Err(cause),
            Err(cleanup) => Err(super::retained_private_creation_error_with_cleanup(
                cause, cleanup,
            )),
        }
    }

    #[allow(unsafe_code)]
    fn windows_verify_private_file_security(
        file: &File,
        expected_owner_sid: &str,
        expected_dacl_sddl: &str,
    ) -> io::Result<()> {
        let mut raw_descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let mut raw_owner: PSID = std::ptr::null_mut();
        // SAFETY: the file handle is valid and the descriptor output points to
        // writable storage. The owner output remains live with the returned
        // descriptor, and unrequested ACL outputs may be null.
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                WINDOWS_PRIVATE_SECURITY_INFORMATION,
                &raw mut raw_owner,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &raw mut raw_descriptor,
            )
        };
        if status != ERROR_SUCCESS {
            return Err(io::Error::from_raw_os_error(status.cast_signed()));
        }
        let descriptor = WindowsSecurityDescriptor {
            raw: raw_descriptor,
            deallocate: LocalFree,
        };
        let owner_sid = windows_sid_string(raw_owner)?;
        let dacl_sddl = windows_descriptor_dacl_sddl(descriptor.raw)?;
        if owner_sid != expected_owner_sid || dacl_sddl != expected_dacl_sddl {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "created file does not expose the required explicit user owner and protected user-and-SYSTEM DACL",
            ));
        }
        Ok(())
    }

    #[allow(unsafe_code)]
    fn windows_sid_string(sid: PSID) -> io::Result<String> {
        if sid.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows returned a null security identifier",
            ));
        }
        let mut raw = std::ptr::null_mut();
        // SAFETY: `sid` points into a live token or security descriptor and the
        // output points to writable storage transferred to
        // `WindowsLocalWideString`.
        if unsafe { ConvertSidToStringSidW(sid, &raw mut raw) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let guard = WindowsLocalWideString {
            raw,
            deallocate: LocalFree,
        };
        let Some(length) = (0..MAX_SID_STRING_UNITS).find(|index| {
            // SAFETY: Windows SID strings are NUL-terminated and bounded well
            // below MAX_SID_STRING_UNITS; the guard keeps the allocation live.
            unsafe { *guard.raw.add(*index) == 0 }
        }) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows returned an unterminated security identifier string",
            ));
        };
        // SAFETY: the bounded scan found `length` initialized UTF-16 units
        // before the NUL terminator, and the guard keeps them live.
        let units = unsafe { std::slice::from_raw_parts(guard.raw, length) };
        String::from_utf16(units).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("security identifier returned invalid UTF-16: {error}"),
            )
        })
    }

    #[allow(unsafe_code)]
    fn windows_descriptor_dacl_sddl(descriptor: PSECURITY_DESCRIPTOR) -> io::Result<String> {
        let mut raw = std::ptr::null_mut();
        let mut length = 0_u32;
        // SAFETY: `descriptor` is live for the call and both output pointers
        // refer to writable storage. The returned string is transferred
        // immediately into `WindowsLocalWideString`.
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
        let guard = WindowsLocalWideString {
            raw,
            deallocate: LocalFree,
        };
        let length = usize::try_from(length).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "security descriptor string length does not fit memory",
            )
        })?;
        // SAFETY: the conversion API returned `length` live UTF-16 code units,
        // including a trailing NUL, owned by `guard`.
        let units = unsafe { std::slice::from_raw_parts(guard.raw, length) };
        let content_length = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        String::from_utf16(&units[..content_length]).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("security descriptor returned invalid UTF-16: {error}"),
            )
        })
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

    /// Opens an existing entry while denying competing Windows mutation access.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error when the final entry cannot be opened
    /// without following a reparse point or competing access prevents ratification.
    pub fn windows_open_for_reconciliation(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    pub fn windows_open_existing_no_follow(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)
    }

    #[allow(unsafe_code)]
    pub fn windows_delete_open_file(file: &File) -> io::Result<()> {
        let extended_disposition = FILE_DISPOSITION_INFO_EX {
            Flags: FILE_DISPOSITION_FLAG_DELETE | FILE_DISPOSITION_FLAG_POSIX_SEMANTICS,
        };
        let extended_size = u32::try_from(size_of::<FILE_DISPOSITION_INFO_EX>()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "FILE_DISPOSITION_INFO_EX size does not fit the Windows API parameter",
            )
        })?;
        // POSIX disposition unlinks the verified object's name immediately,
        // avoiding a delete-pending pathname that directory enumeration can
        // still observe. Older filesystems fall back to legacy delete-on-close.
        if unsafe {
            SetFileInformationByHandle(
                file.as_raw_handle(),
                FileDispositionInfoEx,
                std::ptr::from_ref(&extended_disposition).cast(),
                extended_size,
            )
        } != 0
        {
            return Ok(());
        }
        let extended_error = io::Error::last_os_error();
        let extended_delete_unsupported = extended_error
            .raw_os_error()
            .and_then(|code| u32::try_from(code).ok())
            .is_some_and(|code| {
                matches!(
                    code,
                    ERROR_INVALID_FUNCTION | ERROR_NOT_SUPPORTED | ERROR_INVALID_PARAMETER
                )
            });
        if !extended_delete_unsupported {
            return Err(extended_error);
        }

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
    ) -> io::Result<CommitReceipt<ReplaceExistingOutcome>> {
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

        Ok(CommitReceipt::new(
            ReplaceExistingOutcome::Clean,
            ParentSyncReceipt::windows_unsupported(),
        ))
    }

    #[allow(unsafe_code)]
    pub fn windows_install_new(
        temporary: &Path,
        destination: &Path,
    ) -> io::Result<CommitReceipt<InstallNewOutcome>> {
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

        Ok(CommitReceipt::new(
            InstallNewOutcome::Clean,
            ParentSyncReceipt::windows_unsupported(),
        ))
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
        use std::mem::size_of;
        use std::os::windows::io::AsRawHandle;
        use std::path::Path;

        use tempfile::tempdir;
        use windows_sys::Win32::Foundation::ERROR_SUCCESS;
        use windows_sys::Win32::Security::Authorization::{GetSecurityInfo, SE_FILE_OBJECT};
        use windows_sys::Win32::Security::{
            DACL_SECURITY_INFORMATION, GetSecurityDescriptorControl, OWNER_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED, TOKEN_USER,
        };
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, FILE_ID_128, FILE_ID_INFO,
        };

        use super::{
            DELETE, FILE_SHARE_DELETE, FILE_SHARE_READ, GENERIC_READ, GENERIC_WRITE,
            IdentityQuality, LocalFree, SYSTEM_SID, WINDOWS_PRIVATE_FILE_ACCESS,
            WINDOWS_PRIVATE_FILE_SHARE, WINDOWS_PRIVATE_SECURITY_INFORMATION,
            WindowsLocalWideString, WindowsSecurityDescriptor, windows_combine_disjoint_flags,
            windows_create_new_file_with_security, windows_create_private_new_file,
            windows_delete_open_file, windows_descriptor_dacl_sddl, windows_extended_information,
            windows_finalize_private_creation, windows_identity_from_information,
            windows_open_existing_no_follow, windows_open_for_cleanup,
            windows_open_for_reconciliation, windows_private_security_policy,
            windows_private_security_policy_for_sid, windows_security_descriptor_from_sddl,
            windows_sid_string, windows_token_user_buffer_length,
            windows_validate_token_user_returned_length, windows_verify_private_file_security,
        };

        #[test]
        fn token_user_buffer_lengths_have_exact_boundaries() -> io::Result<()> {
            let minimum = u32::try_from(size_of::<TOKEN_USER>())
                .expect("TOKEN_USER size must fit the Windows API length parameter");

            assert_eq!(
                windows_token_user_buffer_length(minimum)?,
                usize::try_from(minimum).expect("u32 must fit usize on supported Windows")
            );
            assert_eq!(
                windows_token_user_buffer_length(minimum - 1)
                    .expect_err("one byte below TOKEN_USER must fail")
                    .kind(),
                io::ErrorKind::InvalidData
            );

            windows_validate_token_user_returned_length(minimum - 1, minimum)?;
            windows_validate_token_user_returned_length(minimum, minimum)?;
            assert_eq!(
                windows_validate_token_user_returned_length(minimum + 1, minimum)
                    .expect_err("a returned length above the buffer must fail")
                    .kind(),
                io::ErrorKind::InvalidData
            );
            Ok(())
        }

        #[test]
        fn private_policy_canonicalizes_well_known_user_sids() -> io::Result<()> {
            let system = windows_private_security_policy_for_sid(SYSTEM_SID.to_owned())?;
            assert_eq!(system.owner_sid, SYSTEM_SID);
            assert_eq!(system.dacl_sddl, "D:P(A;;FA;;;SY)");

            for (sid, alias) in [("S-1-5-19", "LS"), ("S-1-5-20", "NS")] {
                let policy = windows_private_security_policy_for_sid(sid.to_owned())?;
                assert_eq!(policy.owner_sid, sid);
                assert_eq!(
                    policy.dacl_sddl,
                    format!("D:P(A;;FA;;;SY)(A;;FA;;;{alias})")
                );
            }
            Ok(())
        }

        #[test]
        #[allow(unsafe_code)]
        fn private_file_is_exclusive_writable_and_dacl_protected() -> io::Result<()> {
            let directory = tempdir()?;
            let path = directory.path().join("private.txt");
            let policy = windows_private_security_policy()?;
            let mut file = windows_create_private_new_file(&path)?;
            file.write_all(b"private")?;

            let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            let mut owner: PSID = std::ptr::null_mut();
            // SAFETY: the file handle is valid and the descriptor output points
            // to writable storage. The owner output remains live with the
            // returned descriptor, and unrequested ACL outputs may be null.
            let status = unsafe {
                GetSecurityInfo(
                    file.as_raw_handle(),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    &raw mut owner,
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
            assert_eq!(windows_sid_string(owner)?, policy.owner_sid);
            assert_eq!(
                windows_descriptor_dacl_sddl(descriptor_guard.raw)?,
                policy.dacl_sddl
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
        fn failed_private_security_verification_deletes_the_exact_open_file() -> io::Result<()> {
            let directory = tempdir()?;
            let path = directory.path().join("rejected-private.txt");
            let file = windows_create_private_new_file(&path)?;

            let error = windows_finalize_private_creation(
                file,
                |_| {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "injected private security verification failure",
                    ))
                },
                windows_delete_open_file,
            )
            .expect_err("security verification failure must reject the file");

            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(!crate::creation_error_may_have_retained_private_file(
                &error
            ));
            assert!(!path.exists());
            Ok(())
        }

        #[test]
        fn verifier_rejects_a_broader_protected_dacl() -> io::Result<()> {
            let directory = tempdir()?;
            let path = directory.path().join("broader-private.txt");
            let policy = windows_private_security_policy()?;
            let descriptor = format!(
                "O:{}D:P(A;;FA;;;SY)(A;;FA;;;{})(A;;FR;;;BA)",
                policy.owner_sid, policy.owner_sid
            );
            let file = windows_create_new_file_with_security(&path, &descriptor)?;

            let error =
                windows_verify_private_file_security(&file, &policy.owner_sid, &policy.dacl_sddl)
                    .expect_err("an additional access grant must fail private-file verification");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);

            windows_delete_open_file(&file)?;
            drop(file);
            assert!(!path.exists());
            Ok(())
        }

        #[test]
        fn verifier_rejects_an_unexpected_owner() -> io::Result<()> {
            let directory = tempdir()?;
            let path = directory.path().join("wrong-owner.txt");
            let policy = windows_private_security_policy()?;
            let file = windows_create_new_file_with_security(&path, &policy.descriptor_sddl)?;
            let unexpected_owner = if policy.owner_sid == "S-1-0-0" {
                "S-1-5-18"
            } else {
                "S-1-0-0"
            };

            let error =
                windows_verify_private_file_security(&file, unexpected_owner, &policy.dacl_sddl)
                    .expect_err("an owner other than the creating user must fail verification");
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);

            windows_delete_open_file(&file)?;
            drop(file);
            assert!(!path.exists());
            Ok(())
        }

        #[test]
        fn failed_private_security_cleanup_preserves_both_native_causes() -> io::Result<()> {
            let directory = tempdir()?;
            let path = directory.path().join("retained-private.txt");
            let file = windows_create_private_new_file(&path)?;

            let error = windows_finalize_private_creation(
                file,
                |_| Err(io::Error::from_raw_os_error(13)),
                |_| Err(io::Error::from_raw_os_error(5)),
            )
            .expect_err("failed cleanup must mark the pathname as potentially retained");

            assert_eq!(
                crate::retained_private_file_creation_cause(&error)
                    .and_then(io::Error::raw_os_error),
                Some(13)
            );
            assert_eq!(
                crate::retained_private_file_cleanup_cause(&error)
                    .and_then(io::Error::raw_os_error),
                Some(5)
            );
            assert!(path.exists());
            fs::remove_file(path)?;
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
        #[allow(unsafe_code)]
        fn local_wide_string_guard_dispatches_its_deallocator() {
            use std::sync::atomic::{AtomicPtr, Ordering};

            static RELEASED: AtomicPtr<std::ffi::c_void> = AtomicPtr::new(std::ptr::null_mut());

            unsafe extern "system" fn record_release(
                allocation: windows_sys::Win32::Foundation::HLOCAL,
            ) -> windows_sys::Win32::Foundation::HLOCAL {
                RELEASED.store(allocation, Ordering::SeqCst);
                std::ptr::null_mut()
            }

            let allocation = std::ptr::dangling_mut::<u16>();
            let wide_string = WindowsLocalWideString {
                raw: allocation,
                deallocate: record_release,
            };

            drop(wide_string);

            assert_eq!(
                RELEASED.swap(std::ptr::null_mut(), Ordering::SeqCst),
                allocation.cast()
            );
        }

        #[test]
        fn private_security_flag_sets_are_exact_and_disjoint() {
            assert_eq!(
                WINDOWS_PRIVATE_FILE_ACCESS,
                GENERIC_READ | GENERIC_WRITE | DELETE
            );
            assert_eq!(
                WINDOWS_PRIVATE_FILE_SHARE,
                FILE_SHARE_READ | FILE_SHARE_DELETE
            );
            assert_eq!(
                WINDOWS_PRIVATE_SECURITY_INFORMATION,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION
            );
            assert!(
                std::panic::catch_unwind(|| windows_combine_disjoint_flags(1, 1)).is_err(),
                "overlapping Windows flags must be rejected"
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
        fn reconciliation_ratification_denies_write_rename_and_delete() -> io::Result<()> {
            let directory = tempdir()?;
            let path = directory.path().join("ratified.txt");
            let moved = directory.path().join("moved.txt");
            fs::write(&path, b"ratified revision")?;

            let ratification = windows_open_for_reconciliation(&path)?;
            assert!(OpenOptions::new().write(true).open(&path).is_err());
            assert!(fs::rename(&path, &moved).is_err());
            assert!(fs::remove_file(&path).is_err());
            assert_eq!(fs::read(&path)?, b"ratified revision");

            drop(ratification);
            fs::rename(&path, &moved)?;
            assert_eq!(fs::read(&moved)?, b"ratified revision");
            Ok(())
        }

        #[test]
        fn observation_open_preserves_ordinary_writer_sharing() -> io::Result<()> {
            let directory = tempdir()?;
            let path = directory.path().join("observed.txt");
            fs::write(&path, b"observed revision")?;

            let observation = windows_open_existing_no_follow(&path)?;
            OpenOptions::new().write(true).open(&path)?;
            drop(observation);

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

    use super::{
        CommitReceipt, FileFacts, InstallNewOutcome, ParentSyncOutcome, ReplaceExistingOutcome,
    };

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

    pub fn unsupported_open_existing_no_follow(_path: &Path) -> io::Result<File> {
        unsupported_error("no-follow observation open")
    }

    pub fn unsupported_delete_open_file(_file: &File) -> io::Result<()> {
        unsupported_error("handle-bound file deletion")
    }

    pub fn unsupported_replace_existing(
        _temporary: &Path,
        _destination: &Path,
        _backup: Option<&Path>,
    ) -> io::Result<CommitReceipt<ReplaceExistingOutcome>> {
        unsupported_error("file replacement")
    }

    pub fn unsupported_install_new(
        _temporary: &Path,
        _destination: &Path,
    ) -> io::Result<CommitReceipt<InstallNewOutcome>> {
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
    #[cfg(unix)]
    use std::ffi::OsStr;
    use std::fs::{self, File, hard_link};
    use std::io::{self, Read, Write};

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use tempfile::tempdir;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use xattr::FileExt;

    #[cfg(unix)]
    use super::imp::unix_require_name_matches;
    #[cfg(target_os = "linux")]
    use super::sync_file;
    use super::{
        FileChangeToken, FileIdentity, IdentityQuality, combine_disjoint_flag_bits, file_facts,
    };
    #[cfg(unix)]
    use super::{
        InstallNewOutcome, ParentSyncOutcome, ParentSyncReceipt, ReplaceExistingOutcome,
        install_new, replace_existing, unix_restrict_open_file_to_owner,
    };
    #[cfg(windows)]
    use super::{InstallNewOutcome, ParentSyncOutcome, ReplaceExistingOutcome, install_new};
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    use super::{
        apply_required_metadata, capture_required_metadata, required_metadata_matches_source,
    };
    use super::{
        creation_error_may_have_retained_private_file, open_existing_no_follow,
        retained_private_creation_error, retained_private_creation_error_with_cleanup,
        retained_private_file_cleanup_cause, retained_private_file_creation_cause,
    };

    #[test]
    fn change_token_components_are_exposed_exactly() {
        let token = FileChangeToken::new(17, 29);

        assert_eq!(token.primary(), 17);
        assert_eq!(token.secondary(), 29);
    }

    #[test]
    fn file_identity_components_are_exposed_exactly() {
        let identity = FileIdentity::new(IdentityQuality::Preferred, 17, 29);

        assert_eq!(identity.quality(), IdentityQuality::Preferred);
        assert_eq!(identity.volume(), 17);
        assert_eq!(identity.file(), 29);
    }

    #[test]
    fn disjoint_flag_bits_are_combined_exactly_and_overlap_is_rejected() {
        assert_eq!(combine_disjoint_flag_bits(0b0001, 0b0100), 0b0101);
        assert!(
            std::panic::catch_unwind(|| combine_disjoint_flag_bits(0b0011, 0b0010)).is_err(),
            "overlapping flags must be rejected"
        );
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn no_follow_observation_open_reads_an_ordinary_file() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        fs::write(&path, b"exact document bytes")?;

        let mut file = open_existing_no_follow(&path)?;
        #[cfg(unix)]
        assert!(
            rustix::fs::fcntl_getfl(&file)?.contains(rustix::fs::OFlags::NONBLOCK),
            "the native open must retain nonblocking status"
        );
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        assert_eq!(bytes, b"exact document bytes");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn no_follow_observation_open_refuses_a_final_symlink() -> io::Result<()> {
        use std::os::unix::fs::symlink;

        let directory = tempdir()?;
        let target = directory.path().join("target.txt");
        let link = directory.path().join("link.txt");
        fs::write(&target, b"must not be read through the link")?;
        symlink(&target, &link)?;

        assert!(
            open_existing_no_follow(&link).is_err(),
            "the native open must not follow a final symlink"
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn descriptor_relative_name_check_requires_the_exact_open_identity() -> io::Result<()> {
        let directory = tempdir()?;
        let named_path = directory.path().join("stage.txt");
        let other_path = directory.path().join("other.txt");
        fs::write(&named_path, b"named stage")?;
        fs::write(&other_path, b"different object")?;
        let parent = File::open(directory.path())?;
        let named = File::open(&named_path)?;
        let other = File::open(&other_path)?;

        unix_require_name_matches(&parent, OsStr::new("stage.txt"), &named)?;
        let error = unix_require_name_matches(&parent, OsStr::new("stage.txt"), &other)
            .expect_err("a rebound stage name must not match another open object");

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_file_sync_propagates_device_failures() -> io::Result<()> {
        let device = File::options().write(true).open("/dev/full")?;

        assert!(sync_file(&device).is_err());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_parent_receipt_propagates_barrier_failures() -> io::Result<()> {
        let device = File::options().write(true).open("/dev/full")?;
        let receipt = super::ParentSyncReceipt::from_open_parent(device);

        assert!(receipt.sync().is_err());
        Ok(())
    }

    #[test]
    fn retained_private_creation_errors_are_distinct_and_preserve_the_cause() {
        let ordinary = io::Error::new(io::ErrorKind::PermissionDenied, "ordinary failure");
        assert!(!creation_error_may_have_retained_private_file(&ordinary));

        let permission_denied_code = if cfg!(windows) { 5 } else { 13 };
        let marked =
            retained_private_creation_error(io::Error::from_raw_os_error(permission_denied_code));
        assert_eq!(marked.kind(), io::ErrorKind::PermissionDenied);
        assert!(creation_error_may_have_retained_private_file(&marked));
        assert!(marked.to_string().contains("security finalization"));
        let source = retained_private_file_creation_cause(&marked)
            .expect("the retained marker must preserve its original cause");
        assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(source.raw_os_error(), Some(permission_denied_code));
        assert!(retained_private_file_cleanup_cause(&marked).is_none());
        let error_source = marked
            .get_ref()
            .and_then(std::error::Error::source)
            .and_then(|source| source.downcast_ref::<io::Error>())
            .expect("the marker's error chain must expose the native cause");
        assert_eq!(error_source.raw_os_error(), Some(permission_denied_code));

        let cleanup_code = if cfg!(windows) { 1 } else { 5 };
        let retained = retained_private_creation_error_with_cleanup(
            io::Error::from_raw_os_error(permission_denied_code),
            io::Error::from_raw_os_error(cleanup_code),
        );
        assert_eq!(
            retained_private_file_cleanup_cause(&retained).and_then(io::Error::raw_os_error),
            Some(cleanup_code)
        );
        assert!(retained.to_string().contains("cleanup also failed"));
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

        let (outcome, parent_sync) = replace_existing(&temporary, &destination, None)?.into_parts();

        assert_eq!(outcome, ReplaceExistingOutcome::DisplacedDestination);
        assert_eq!(parent_sync.sync()?, ParentSyncOutcome::Synced);
        assert_eq!(fs::read(&destination)?, b"new bytes");
        assert_eq!(fs::read(&temporary)?, b"old bytes");
        Ok(())
    }

    #[cfg(unix)]
    fn assert_parent_receipt_survives_rebind(
        parent_sync: ParentSyncReceipt,
        active: &std::path::Path,
        moved: &std::path::Path,
    ) -> io::Result<()> {
        let committed_parent = file_facts(&parent_sync.parent)?.identity();
        fs::rename(active, moved)?;
        fs::create_dir(active)?;
        let moved_parent = file_facts(&File::open(moved)?)?.identity();
        let rebound_parent = file_facts(&File::open(active)?)?.identity();

        assert_eq!(committed_parent, moved_parent);
        assert_ne!(committed_parent, rebound_parent);
        assert_eq!(parent_sync.sync()?, ParentSyncOutcome::Synced);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn replacement_parent_receipt_remains_bound_after_path_rebind() -> io::Result<()> {
        let directory = tempdir()?;
        let active = directory.path().join("active");
        let moved = directory.path().join("moved");
        fs::create_dir(&active)?;
        let temporary = active.join("temporary.txt");
        let destination = active.join("destination.txt");
        fs::write(&temporary, b"new bytes")?;
        fs::write(&destination, b"old bytes")?;

        let (outcome, parent_sync) = replace_existing(&temporary, &destination, None)?.into_parts();

        assert_eq!(outcome, ReplaceExistingOutcome::DisplacedDestination);
        assert_parent_receipt_survives_rebind(parent_sync, &active, &moved)?;
        assert_eq!(fs::read(moved.join("destination.txt"))?, b"new bytes");
        assert!(!active.join("destination.txt").exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn installation_parent_receipt_remains_bound_after_path_rebind() -> io::Result<()> {
        let directory = tempdir()?;
        let active = directory.path().join("active");
        let moved = directory.path().join("moved");
        fs::create_dir(&active)?;
        let temporary = active.join("temporary.txt");
        let destination = active.join("destination.txt");
        fs::write(&temporary, b"new bytes")?;

        let (outcome, parent_sync) = install_new(&temporary, &destination)?.into_parts();

        assert!(matches!(
            outcome,
            InstallNewOutcome::Clean | InstallNewOutcome::CommittedWithRetainedTemporary
        ));
        assert_parent_receipt_survives_rebind(parent_sync, &active, &moved)?;
        assert_eq!(fs::read(moved.join("destination.txt"))?, b"new bytes");
        assert!(!active.join("destination.txt").exists());
        Ok(())
    }

    #[cfg(windows)]
    #[test]
    fn windows_commit_receipts_report_parent_sync_as_unsupported() -> io::Result<()> {
        let directory = tempdir()?;
        let replacement = directory.path().join("replacement.txt");
        let destination = directory.path().join("destination.txt");
        let installation = directory.path().join("installation.txt");
        let installed = directory.path().join("installed.txt");
        fs::write(&replacement, b"replacement")?;
        fs::write(&destination, b"previous")?;
        fs::write(&installation, b"installation")?;

        let (replace_outcome, replace_parent) =
            super::replace_existing(&replacement, &destination, None)?.into_parts();
        let (install_outcome, install_parent) =
            install_new(&installation, &installed)?.into_parts();

        assert_eq!(replace_outcome, ReplaceExistingOutcome::Clean);
        assert!(matches!(install_outcome, InstallNewOutcome::Clean));
        assert_eq!(replace_parent.sync()?, ParentSyncOutcome::Unsupported);
        assert_eq!(install_parent.sync()?, ParentSyncOutcome::Unsupported);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn open_file_can_be_restricted_to_owner_only_access() -> io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("recovery.txt");
        fs::write(&path, b"private recovery bytes")?;
        let mut permissions = fs::metadata(&path)?.permissions();
        permissions.set_mode(0o664);
        fs::set_permissions(&path, permissions)?;
        let file = File::open(&path)?;

        unix_restrict_open_file_to_owner(&file)?;

        assert_eq!(file.metadata()?.permissions().mode() & 0o7777, 0o600);
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
        let directory = tempdir()?;
        let source_path = directory.path().join("source.txt");
        let destination_path = directory.path().join("destination.txt");
        fs::write(&source_path, b"source")?;
        fs::write(&destination_path, b"destination")?;

        run_macos_chmod(
            &source_path,
            &["+a", "everyone deny write"],
            "create the ACL fixture",
        )?;

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

        run_macos_chmod(&source_path, &["-a#", "0"], "mutate the ACL fixture")?;
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

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_absent_acl_snapshot_clears_a_destination_acl() -> io::Result<()> {
        let directory = tempdir()?;
        let source_path = directory.path().join("source.txt");
        let destination_path = directory.path().join("destination.txt");
        fs::write(&source_path, b"source")?;
        fs::write(&destination_path, b"destination")?;

        run_macos_chmod(&source_path, &["-N"], "remove the source ACL fixture")?;

        let source = File::open(&source_path)?;
        let destination = File::options()
            .read(true)
            .write(true)
            .open(&destination_path)?;
        run_macos_chmod(
            &destination_path,
            &["+a", "everyone deny write"],
            "create the destination ACL fixture",
        )?;
        let source_facts = file_facts(&source)?;
        let metadata = capture_required_metadata(&source, source_facts)?;
        assert!(!required_metadata_matches_source(
            &metadata,
            &destination,
            file_facts(&destination)?
        )?);

        apply_required_metadata(&metadata, &destination)?;
        assert!(required_metadata_matches_source(
            &metadata,
            &destination,
            file_facts(&destination)?
        )?);
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn run_macos_chmod(
        path: &std::path::Path,
        arguments: &[&str],
        operation: &str,
    ) -> io::Result<()> {
        let status = std::process::Command::new("/bin/chmod")
            .args(arguments)
            .arg(path)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "chmod failed to {operation}: {status}"
            )));
        }
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
