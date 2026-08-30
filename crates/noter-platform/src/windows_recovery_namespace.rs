//! Windows recovery-directory namespace binding foundation.
//!
//! This module binds the state and recovery directories to retained handles.
//! It does not yet expose handle-relative record operations, so completing the
//! recovery namespace protocol remains separate work.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Component, Path, PathBuf, Prefix};

use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_LIST_DIRECTORY,
    FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK, FileBasicInfo,
    FileIdInfo, GetDriveTypeW, GetFileInformationByHandleEx, GetFileType,
    GetVolumeInformationByHandleW, READ_CONTROL, WRITE_DAC,
};
use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;

use crate::combine_disjoint_flag_bits;
use crate::imp::{
    windows_create_private_directory, windows_tighten_private_directory_security,
    windows_verify_owner_controlled_state_directory, windows_verify_private_directory_security,
};

const RECORDS_DIRECTORY_NAME: &str = "records";
const QUARANTINE_DIRECTORY_NAME: &str = "quarantine";
const FILE_SYSTEM_NAME_CAPACITY: usize = 32;

/// Stable preferred Windows identity of one retained directory handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowsDirectoryIdentity {
    volume_serial: u64,
    file_id: [u8; 16],
}

/// Validated single-component name for a recovery-directory entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WindowsRecoveryEntryName(OsString);

impl WindowsRecoveryEntryName {
    /// Validates one unambiguous Windows pathname component.
    ///
    /// Names with separators, streams, device aliases, trailing spaces or
    /// periods, invalid UTF-16, or Windows-forbidden characters are rejected.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] when `name` is not one ordinary
    /// pathname component with an exact Windows spelling.
    pub fn new(name: &OsStr) -> io::Result<Self> {
        validate_windows_component(name)?;
        Ok(Self(name.to_os_string()))
    }

    /// Returns the validated component spelling.
    #[must_use]
    pub fn as_os_str(&self) -> &OsStr {
        &self.0
    }
}

#[derive(Debug)]
struct BoundWindowsDirectory {
    // This handle intentionally has no public accessor. Holding it without
    // FILE_SHARE_DELETE prevents pathname retirement while the namespace lives.
    handle: File,
    identity: WindowsDirectoryIdentity,
}

impl BoundWindowsDirectory {
    const fn handle(&self) -> &File {
        &self.handle
    }
}

/// Retained Windows handles for the recovery directory hierarchy.
///
/// Construction fails before recovery content is written unless the state path
/// is an absolute drive path on a fixed-drive NTFS volume, every traversed
/// directory is a non-reparse disk directory on the same volume, the state
/// directory belongs to the current user, and principals other than that user,
/// SYSTEM, and Administrators have read-only access. The recovery directories
/// must have Noter's exact private owner and inheritable DACL policy. Newly
/// created directories receive that policy at creation time. Fixed-drive
/// classification does not prove that the profile is unsynchronized or local.
///
/// This type is a namespace foundation. Record creation, enumeration, rename,
/// quarantine, synchronization, and retirement are not yet routed through
/// these handles.
pub struct WindowsRecoveryNamespace {
    state: BoundWindowsDirectory,
    recovery: BoundWindowsDirectory,
    records: BoundWindowsDirectory,
    quarantine: BoundWindowsDirectory,
    _traversal_guards: Vec<BoundWindowsDirectory>,
}

impl WindowsRecoveryNamespace {
    /// Opens or creates and binds the state and recovery directory hierarchy.
    ///
    /// `state_root` must name at least one component below a drive root. Its
    /// parent hierarchy must already exist. The state directory itself and the
    /// three recovery directories may be created. Requiring an existing parent
    /// keeps this API from changing security on unrelated profile directories.
    ///
    /// # Errors
    ///
    /// Returns an error without writing recovery content when the pathname,
    /// filesystem, directory identity, reparse policy, or private security
    /// contract cannot be established. A newly created empty private directory
    /// can remain if a later native verification call fails.
    pub fn open_or_create(state_root: &Path, recovery_name: &OsStr) -> io::Result<Self> {
        let parsed = ParsedStatePath::new(state_root)?;
        let recovery_name = WindowsRecoveryEntryName::new(recovery_name)?;
        let records_name = WindowsRecoveryEntryName::new(OsStr::new(RECORDS_DIRECTORY_NAME))?;
        let quarantine_name = WindowsRecoveryEntryName::new(OsStr::new(QUARANTINE_DIRECTORY_NAME))?;

        verify_fixed_drive(&parsed.drive_root)?;
        let root = bind_existing_directory(&parsed.drive_root, None)?;
        verify_ntfs(root.handle())?;
        let expected_volume = root.identity.volume_serial;

        let mut traversal_guards = vec![root];
        let mut current = parsed.drive_root;
        let (state_name, parent_names) = parsed
            .components
            .split_last()
            .ok_or_else(invalid_state_root_error)?;
        for component in parent_names {
            current.push(component);
            traversal_guards.push(bind_existing_directory(&current, Some(expected_volume))?);
        }

        current.push(state_name);
        let state = bind_state_directory(&current, expected_volume)?;
        current.push(recovery_name.as_os_str());
        let recovery = bind_private_directory(&current, expected_volume)?;
        let mut records_path = current.clone();
        records_path.push(records_name.as_os_str());
        let records = bind_private_directory(&records_path, expected_volume)?;
        current.push(quarantine_name.as_os_str());
        let quarantine = bind_private_directory(&current, expected_volume)?;

        Ok(Self {
            state,
            recovery,
            records,
            quarantine,
            _traversal_guards: traversal_guards,
        })
    }

    /// Returns the identity bound to the state directory.
    #[must_use]
    pub const fn state_identity(&self) -> WindowsDirectoryIdentity {
        self.state.identity
    }

    /// Returns the identity bound to the recovery directory.
    #[must_use]
    pub const fn recovery_identity(&self) -> WindowsDirectoryIdentity {
        self.recovery.identity
    }

    /// Returns the identity bound to the records directory.
    #[must_use]
    pub const fn records_identity(&self) -> WindowsDirectoryIdentity {
        self.records.identity
    }

    /// Returns the identity bound to the quarantine directory.
    #[must_use]
    pub const fn quarantine_identity(&self) -> WindowsDirectoryIdentity {
        self.quarantine.identity
    }
}

struct ParsedStatePath {
    drive_root: PathBuf,
    components: Vec<OsString>,
}

impl ParsedStatePath {
    fn new(path: &Path) -> io::Result<Self> {
        let mut path_components = path.components();
        let drive_letter = match path_components.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(letter) if letter.is_ascii_alphabetic() => letter,
                _ => return Err(invalid_state_root_error()),
            },
            _ => return Err(invalid_state_root_error()),
        };
        if !matches!(path_components.next(), Some(Component::RootDir)) {
            return Err(invalid_state_root_error());
        }

        let mut components = Vec::new();
        for component in path_components {
            let Component::Normal(name) = component else {
                return Err(invalid_state_root_error());
            };
            validate_windows_component(name)?;
            components.push(name.to_os_string());
        }
        if components.is_empty() {
            return Err(invalid_state_root_error());
        }

        let drive_root = PathBuf::from(format!("{}:\\", char::from(drive_letter)));
        Ok(Self {
            drive_root,
            components,
        })
    }
}

fn validate_windows_component(name: &OsStr) -> io::Result<()> {
    let units: Vec<u16> = name.encode_wide().collect();
    if units.is_empty() || units.len() > 255 || units.contains(&0) {
        return Err(invalid_entry_name_error());
    }
    let name = String::from_utf16(&units).map_err(|_| invalid_entry_name_error())?;
    if name.ends_with([' ', '.'])
        || name.chars().any(|character| {
            character <= '\u{1f}'
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
    {
        return Err(invalid_entry_name_error());
    }

    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let numbered_device = stem
        .strip_prefix("COM")
        .or_else(|| stem.strip_prefix("LPT"))
        .is_some_and(|suffix| {
            matches!(
                suffix,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
            )
        });
    if matches!(
        stem.as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
    ) || numbered_device
    {
        return Err(invalid_entry_name_error());
    }
    Ok(())
}

fn verify_fixed_drive(drive_root: &Path) -> io::Result<()> {
    let wide = nul_terminated_path(drive_root)?;
    // SAFETY: `wide` is a live NUL-terminated UTF-16 drive-root path and the
    // function reads no output pointers.
    #[allow(unsafe_code)]
    let drive_type = unsafe { GetDriveTypeW(wide.as_ptr()) };
    if drive_type != DRIVE_FIXED {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "recovery state requires a fixed drive",
        ));
    }
    Ok(())
}

fn verify_ntfs(handle: &File) -> io::Result<()> {
    let mut file_system_name = [0_u16; FILE_SYSTEM_NAME_CAPACITY];
    let capacity = u32::try_from(file_system_name.len()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "filesystem-name buffer does not fit the Windows API parameter",
        )
    })?;
    // SAFETY: the directory handle is live, unused output fields are null, and
    // the filesystem-name output points to a writable buffer of `capacity`
    // UTF-16 units.
    #[allow(unsafe_code)]
    if unsafe {
        GetVolumeInformationByHandleW(
            handle.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            file_system_name.as_mut_ptr(),
            capacity,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let name_length = file_system_name
        .iter()
        .position(|unit| *unit == 0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "Windows returned an unterminated filesystem name",
            )
        })?;
    let name = String::from_utf16(&file_system_name[..name_length]).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Windows returned an invalid filesystem name: {error}"),
        )
    })?;
    if !name.eq_ignore_ascii_case("NTFS") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "recovery state requires NTFS",
        ));
    }
    Ok(())
}

fn bind_existing_directory(
    path: &Path,
    expected_volume: Option<u64>,
) -> io::Result<BoundWindowsDirectory> {
    let handle = open_directory_no_follow(path, DirectorySharePolicy::Traversal)?;
    let identity = verify_directory_handle(&handle)?;
    if expected_volume.is_some_and(|volume| volume != identity.volume_serial) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery state directory crossed a volume boundary",
        ));
    }
    Ok(BoundWindowsDirectory { handle, identity })
}

fn bind_state_directory(path: &Path, expected_volume: u64) -> io::Result<BoundWindowsDirectory> {
    let handle = open_or_create_private_directory(path)?;
    let identity = verify_directory_handle(&handle)?;
    if identity.volume_serial != expected_volume {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery state directory crossed a volume boundary",
        ));
    }
    windows_verify_owner_controlled_state_directory(&handle)?;
    Ok(BoundWindowsDirectory { handle, identity })
}

fn bind_private_directory(path: &Path, expected_volume: u64) -> io::Result<BoundWindowsDirectory> {
    let handle = open_or_create_private_directory(path)?;
    let identity = verify_directory_handle(&handle)?;
    if identity.volume_serial != expected_volume {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery directory crossed a volume boundary",
        ));
    }
    if windows_verify_private_directory_security(&handle).is_err() {
        windows_verify_owner_controlled_state_directory(&handle)?;
        windows_tighten_private_directory_security(&handle)?;
    }
    Ok(BoundWindowsDirectory { handle, identity })
}

fn open_or_create_private_directory(path: &Path) -> io::Result<File> {
    let handle = match open_directory_no_follow(path, DirectorySharePolicy::BoundPrivate) {
        Ok(handle) => handle,
        Err(error) => match classify_directory_open_error(&error) {
            DirectoryOpenError::Missing => {
                if let Err(error) = windows_create_private_directory(path) {
                    match classify_directory_creation_error(&error) {
                        DirectoryCreationError::Raced => {}
                        DirectoryCreationError::Fatal => return Err(error),
                    }
                }
                open_directory_no_follow(path, DirectorySharePolicy::BoundPrivate)?
            }
            DirectoryOpenError::Fatal => return Err(error),
        },
    };
    Ok(handle)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectoryOpenError {
    Missing,
    Fatal,
}

fn classify_directory_open_error(error: &io::Error) -> DirectoryOpenError {
    match error.kind() {
        io::ErrorKind::NotFound => DirectoryOpenError::Missing,
        _ => DirectoryOpenError::Fatal,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirectoryCreationError {
    Raced,
    Fatal,
}

fn classify_directory_creation_error(error: &io::Error) -> DirectoryCreationError {
    match error.kind() {
        io::ErrorKind::AlreadyExists => DirectoryCreationError::Raced,
        _ => DirectoryCreationError::Fatal,
    }
}

#[derive(Clone, Copy)]
enum DirectorySharePolicy {
    Traversal,
    BoundPrivate,
}

fn open_directory_no_follow(path: &Path, share_policy: DirectorySharePolicy) -> io::Result<File> {
    let (security_access, share_mode) = match share_policy {
        DirectorySharePolicy::Traversal => (
            0,
            combine_disjoint_flag_bits(FILE_SHARE_READ, FILE_SHARE_WRITE),
        ),
        DirectorySharePolicy::BoundPrivate => (
            WRITE_DAC,
            combine_disjoint_flag_bits(FILE_SHARE_READ, FILE_SHARE_WRITE),
        ),
    };
    let read_access = combine_disjoint_flag_bits(FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES);
    let read_and_control_access = combine_disjoint_flag_bits(read_access, READ_CONTROL);
    let access_mode = combine_disjoint_flag_bits(read_and_control_access, security_access);
    let custom_flags =
        combine_disjoint_flag_bits(FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT);
    OpenOptions::new()
        .access_mode(access_mode)
        .share_mode(share_mode)
        .custom_flags(custom_flags)
        .open(path)
}

const fn directory_attributes_are_safe(attributes: u32) -> bool {
    attributes & FILE_ATTRIBUTE_DIRECTORY != 0 && attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
}

fn verify_directory_handle(handle: &File) -> io::Result<WindowsDirectoryIdentity> {
    // SAFETY: the live handle value is passed by value and no buffers are used.
    #[allow(unsafe_code)]
    if unsafe { GetFileType(handle.as_raw_handle()) } != FILE_TYPE_DISK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery path did not resolve to a disk object",
        ));
    }

    let mut basic = FILE_BASIC_INFO::default();
    let basic_size = u32::try_from(size_of::<FILE_BASIC_INFO>()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "FILE_BASIC_INFO size does not fit the Windows API parameter",
        )
    })?;
    // SAFETY: the live directory handle remains valid, `basic` is writable,
    // and the byte count is the exact size of its initialized structure.
    #[allow(unsafe_code)]
    if unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileBasicInfo,
            (&raw mut basic).cast(),
            basic_size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if !directory_attributes_are_safe(basic.FileAttributes) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery path must be a non-reparse directory",
        ));
    }

    let first = query_preferred_identity(handle)?;
    let second = query_preferred_identity(handle)?;
    if first != second {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery directory identity changed during ratification",
        ));
    }
    Ok(first)
}

fn query_preferred_identity(handle: &File) -> io::Result<WindowsDirectoryIdentity> {
    let mut information = FILE_ID_INFO::default();
    let information_size = u32::try_from(size_of::<FILE_ID_INFO>()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "FILE_ID_INFO size does not fit the Windows API parameter",
        )
    })?;
    // SAFETY: the live directory handle remains valid, `information` is
    // writable, and the byte count exactly matches `FILE_ID_INFO`.
    #[allow(unsafe_code)]
    if unsafe {
        GetFileInformationByHandleEx(
            handle.as_raw_handle(),
            FileIdInfo,
            (&raw mut information).cast(),
            information_size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    if information.FileId.Identifier == [0; 16] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows returned an empty preferred directory identifier",
        ));
    }
    Ok(WindowsDirectoryIdentity {
        volume_serial: information.VolumeSerialNumber,
        file_id: information.FileId.Identifier,
    })
}

fn nul_terminated_path(path: &Path) -> io::Result<Vec<u16>> {
    let mut units = Vec::new();
    for unit in path.as_os_str().encode_wide() {
        if unit == 0 {
            return Err(invalid_state_root_error());
        }
        units.push(unit);
    }
    units.push(0);
    Ok(units)
}

fn invalid_state_root_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "recovery state path must be an absolute drive path below its root",
    )
}

fn invalid_entry_name_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "recovery entry name must be one unambiguous Windows pathname component",
    )
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::fs::{self, File};
    use std::io;
    use std::os::windows::fs::symlink_dir;
    use std::path::Path;

    use tempfile::tempdir;

    use super::{
        DirectoryCreationError, DirectoryOpenError, DirectorySharePolicy, ParsedStatePath,
        WindowsRecoveryEntryName, WindowsRecoveryNamespace, classify_directory_creation_error,
        classify_directory_open_error, directory_attributes_are_safe, open_directory_no_follow,
        verify_fixed_drive, verify_ntfs,
    };
    use crate::imp::{
        windows_create_owner_controlled_readable_directory_for_test,
        windows_verify_private_directory_security,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT,
    };

    #[test]
    fn entry_names_accept_only_unambiguous_single_components() {
        for accepted in ["recovery", "record-0123.json", "name with spaces"] {
            assert_eq!(
                WindowsRecoveryEntryName::new(OsStr::new(accepted))
                    .expect("ordinary name should be accepted")
                    .as_os_str(),
                OsStr::new(accepted)
            );
        }

        for rejected in [
            "",
            ".",
            "..",
            "child\\entry",
            "child/entry",
            "stream:name",
            "trailing.",
            "trailing ",
            "NUL",
            "con.txt",
            "COM1.log",
            "COM¹",
            "COM².log",
            "COM³",
            "LPT¹",
            "LPT².log",
            "LPT³",
            "bad*name",
        ] {
            assert!(
                WindowsRecoveryEntryName::new(OsStr::new(rejected)).is_err(),
                "unexpectedly accepted {rejected:?}"
            );
        }
        assert!(WindowsRecoveryEntryName::new(OsStr::new(&"x".repeat(255))).is_ok());
        assert!(WindowsRecoveryEntryName::new(OsStr::new(&"x".repeat(256))).is_err());
    }

    #[test]
    fn directory_error_classification_is_exact() {
        assert_eq!(
            classify_directory_open_error(&io::Error::from(io::ErrorKind::NotFound)),
            DirectoryOpenError::Missing
        );
        assert_eq!(
            classify_directory_open_error(&io::Error::from(io::ErrorKind::PermissionDenied)),
            DirectoryOpenError::Fatal
        );
        assert_eq!(
            classify_directory_creation_error(&io::Error::from(io::ErrorKind::AlreadyExists)),
            DirectoryCreationError::Raced
        );
        assert_eq!(
            classify_directory_creation_error(&io::Error::from(io::ErrorKind::NotFound)),
            DirectoryCreationError::Fatal
        );
    }

    #[test]
    fn directory_attribute_policy_requires_an_ordinary_directory() {
        assert!(directory_attributes_are_safe(FILE_ATTRIBUTE_DIRECTORY));
        assert!(!directory_attributes_are_safe(0));
        assert!(!directory_attributes_are_safe(
            FILE_ATTRIBUTE_DIRECTORY | FILE_ATTRIBUTE_REPARSE_POINT
        ));
        assert!(!directory_attributes_are_safe(FILE_ATTRIBUTE_REPARSE_POINT));
    }

    #[test]
    fn native_volume_checks_reject_non_volume_inputs() -> io::Result<()> {
        assert_eq!(
            verify_fixed_drive(Path::new(r"?:\"))
                .expect_err("an invalid drive root must not be classified as fixed")
                .kind(),
            io::ErrorKind::Unsupported
        );
        let null_device = File::open("NUL")?;
        assert!(verify_ntfs(&null_device).is_err());
        Ok(())
    }

    #[test]
    fn state_path_requires_a_drive_root_and_normal_components() {
        assert!(ParsedStatePath::new(Path::new(r"C:\Users\owner\Noter")).is_ok());
        for rejected in [
            r"Noter",
            r"C:Noter",
            r"C:\",
            r"C:\Users\..\Noter",
            r"\\server\share\Noter",
            r"\\?\C:\Users\owner\Noter",
            r"\\.\C:\Users\owner\Noter",
        ] {
            assert!(
                ParsedStatePath::new(Path::new(rejected)).is_err(),
                "unexpectedly accepted {rejected:?}"
            );
        }
    }

    #[test]
    fn native_reparse_traversal_is_rejected_without_following_target() -> io::Result<()> {
        let parent = tempdir()?;
        let target = parent.path().join("target");
        let link = parent.path().join("link");
        fs::create_dir(&target)?;
        symlink_dir(&target, &link)?;

        let result =
            WindowsRecoveryNamespace::open_or_create(&link.join("state"), OsStr::new("recovery"));
        let Err(error) = result else {
            panic!("a reparse traversal component must be rejected");
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(!target.join("state").exists());
        Ok(())
    }

    #[test]
    fn namespace_binds_distinct_private_directories_on_one_volume() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;

        let identities = [
            namespace.state_identity(),
            namespace.recovery_identity(),
            namespace.records_identity(),
            namespace.quarantine_identity(),
        ];
        assert!(identities.iter().all(|identity| {
            identity.volume_serial == identities[0].volume_serial && identity.file_id != [0; 16]
        }));
        for (index, identity) in identities.iter().enumerate() {
            assert!(identities[..index].iter().all(|other| other != identity));
        }
        assert!(state.join("recovery").join("records").is_dir());
        assert!(state.join("recovery").join("quarantine").is_dir());
        fs::write(
            state.join("recovery").join("records").join("entry"),
            b"bound child operation",
        )?;
        Ok(())
    }

    #[test]
    fn owner_controlled_existing_recovery_directories_are_tightened() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let recovery = state.join("recovery");
        let initial = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;
        drop(initial);
        fs::remove_dir_all(&recovery)?;
        let records = recovery.join("records");
        let quarantine = recovery.join("quarantine");
        for directory in [&recovery, &records, &quarantine] {
            windows_create_owner_controlled_readable_directory_for_test(directory)?;
        }

        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;
        drop(namespace);
        for directory in [&recovery, &records, &quarantine] {
            let handle = open_directory_no_follow(directory, DirectorySharePolicy::BoundPrivate)?;
            windows_verify_private_directory_security(&handle)?;
        }
        Ok(())
    }

    #[test]
    fn retained_handles_prevent_state_path_swap() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let moved = parent.path().join("moved");
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;

        assert!(fs::rename(&state, &moved).is_err());
        assert!(state.is_dir());
        assert!(!moved.exists());

        drop(namespace);
        fs::rename(&state, &moved)?;
        assert!(!state.exists());
        assert!(moved.is_dir());
        Ok(())
    }

    #[test]
    fn retained_traversal_handles_prevent_ancestor_path_swap() -> io::Result<()> {
        let parent = tempdir()?;
        let ancestor = parent.path().join("ancestor");
        let state = ancestor.join("state");
        let moved = parent.path().join("moved-ancestor");
        fs::create_dir(&ancestor)?;
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;

        assert!(fs::rename(&ancestor, &moved).is_err());
        assert!(ancestor.is_dir());
        assert!(!moved.exists());

        drop(namespace);
        fs::rename(&ancestor, &moved)?;
        assert!(!ancestor.exists());
        assert!(moved.is_dir());
        Ok(())
    }

    #[test]
    fn namespace_is_send_and_sync() {
        const fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<WindowsRecoveryNamespace>();
    }
}
