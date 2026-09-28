//! Windows recovery-directory namespace binding.
//!
//! This module binds the state and recovery directories to retained handles.
//! Its entry creation, open, classification, enumeration, installation, and
//! directory synchronization use those handles. Complete root classification
//! remains M4-H1 work.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::path::{Component, Path, PathBuf, Prefix};
use std::sync::Mutex;

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_REPARSE_POINT, FILE_RENAME_INFORMATION,
    FILE_SYNCHRONOUS_IO_NONALERT, FileRenameInformation, NtCreateFile, NtSetInformationFile,
};
use windows_sys::Win32::Foundation::{
    ERROR_CLOUD_FILE_NOT_UNDER_SYNC_ROOT, ERROR_NO_MORE_FILES, FreeLibrary, GENERIC_READ, HANDLE,
    HMODULE, INVALID_HANDLE_VALUE, OBJ_CASE_INSENSITIVE, RtlNtStatusToDosError, UNICODE_STRING,
};
use windows_sys::Win32::Storage::CloudFilters::{
    CF_SYNC_ROOT_BASIC_INFO, CF_SYNC_ROOT_INFO_BASIC, CF_SYNC_ROOT_INFO_CLASS,
};
use windows_sys::Win32::Storage::FileSystem::{
    DELETE, FILE_ADD_FILE, FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_BASIC_INFO, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_BOTH_DIR_INFO, FILE_ID_INFO, FILE_LIST_DIRECTORY,
    FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_DISK,
    FileBasicInfo, FileIdBothDirectoryInfo, FileIdBothDirectoryRestartInfo, FileIdInfo,
    GetDriveTypeW, GetFileInformationByHandleEx, GetFileType, GetVolumeInformationByHandleW,
    READ_CONTROL, SYNCHRONIZE, WRITE_DAC,
};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;
use windows_sys::Win32::System::LibraryLoader::{
    GetModuleFileNameW, GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;
use windows_sys::Win32::UI::Shell::{
    FOLDERID_LocalAppData, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath,
};

use crate::imp::{
    windows_create_private_directory, windows_create_private_new_at,
    windows_tighten_private_directory_security, windows_verify_owner_controlled_state_directory,
    windows_verify_private_directory_security,
};
use crate::{
    CommitReceipt, InstallNewOutcome, ParentSyncOutcome, ParentSyncReceipt,
    combine_disjoint_flag_bits,
};

const RECORDS_DIRECTORY_NAME: &str = "records";
const QUARANTINE_DIRECTORY_NAME: &str = "quarantine";
const FILE_SYSTEM_NAME_CAPACITY: usize = 32;
const DIRECTORY_ENUMERATION_BUFFER_BYTES: usize = 65_536;
const KNOWN_FOLDER_PATH_LIMIT: usize = 32_767;

struct KnownFolderAllocation(*mut u16);

impl Drop for KnownFolderAllocation {
    fn drop(&mut self) {
        // SAFETY: SHGetKnownFolderPath allocates this pointer with the COM task
        // allocator; it stays owned here and is freed exactly once.
        #[allow(unsafe_code)]
        unsafe {
            CoTaskMemFree(self.0.cast());
        }
    }
}

/// Returns the current user's `LocalAppData` known-folder path.
///
/// # Errors
///
/// Returns an error when Windows cannot resolve a bounded, nonempty path.
pub fn windows_local_appdata_directory() -> io::Result<PathBuf> {
    let mut raw = std::ptr::null_mut();
    // SAFETY: Windows writes one task-allocated null-terminated path pointer
    // into `raw`. A null token requests the current user.
    #[allow(unsafe_code)]
    let status = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_LocalAppData,
            KF_FLAG_DONT_VERIFY as u32,
            std::ptr::null_mut(),
            &raw mut raw,
        )
    };
    let allocation = KnownFolderAllocation(raw);
    if status != 0 {
        return Err(io::Error::other(format!(
            "Windows LocalAppData lookup failed with HRESULT {status:#010x}"
        )));
    }
    let path = std::ptr::NonNull::new(allocation.0)
        .ok_or_else(|| io::Error::other("Windows LocalAppData lookup returned no path"))?;
    let mut units = Vec::new();
    for offset in 0..KNOWN_FOLDER_PATH_LIMIT {
        // SAFETY: successful SHGetKnownFolderPath returns a null-terminated
        // allocation. The loop reads only through that terminator.
        #[allow(unsafe_code)]
        let unit = unsafe { *path.as_ptr().add(offset) };
        if unit == 0 {
            if units.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Windows LocalAppData path is empty",
                ));
            }
            return Ok(PathBuf::from(OsString::from_wide(&units)));
        }
        units.push(unit);
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "Windows LocalAppData path exceeds the supported length",
    ))
}

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

/// A retained, verified Windows directory used for recovery entry operations.
#[derive(Debug)]
pub struct WindowsRecoveryDirectory {
    // This handle intentionally has no public accessor. Holding it without
    // FILE_SHARE_DELETE prevents pathname retirement while the namespace lives.
    handle: File,
    enumeration_lock: Mutex<()>,
    identity: WindowsDirectoryIdentity,
    path: PathBuf,
}

impl WindowsRecoveryDirectory {
    const fn handle(&self) -> &File {
        &self.handle
    }

    /// Returns the spelling of this directory when it was bound.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Flushes metadata for this exact retained directory after an entry change.
    ///
    /// # Errors
    ///
    /// Returns an operating-system error if the directory barrier fails.
    pub fn sync(&self) -> io::Result<ParentSyncOutcome> {
        self.handle.sync_all()?;
        Ok(ParentSyncOutcome::Synced)
    }

    /// Opens one entry relative to this retained directory handle.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry name is invalid, missing, or cannot be
    /// opened without following its final reparse point.
    pub fn open_existing(&self, name: &OsStr) -> io::Result<File> {
        open_entry_relative(&self.handle, name, false)
    }

    /// Opens one entry for exact handle-bound cleanup.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry name is invalid, missing, or cannot be
    /// opened with deletion access without following its final reparse point.
    pub fn open_for_cleanup(&self, name: &OsStr) -> io::Result<File> {
        open_entry_relative(&self.handle, name, true)
    }

    /// Opens one entry through the held directory while denying competing
    /// writes and renames during replacement reconciliation.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is invalid, the entry is missing, or a
    /// competing handle prevents the exclusive observation share mode.
    pub fn open_for_reconciliation(&self, name: &OsStr) -> io::Result<File> {
        let options = combine_disjoint_flag_bits(
            combine_disjoint_flag_bits(FILE_NON_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT),
            FILE_SYNCHRONOUS_IO_NONALERT,
        );
        let file = open_entry_relative_with(
            &self.handle,
            name,
            combine_disjoint_flag_bits(GENERIC_READ, SYNCHRONIZE),
            FILE_SHARE_READ,
            options,
        )?;
        verify_regular_entry_handle(&file)?;
        Ok(file)
    }

    /// Opens one entry for a handle-relative rename while denying competing
    /// writes and renames until the caller finishes its directory barriers.
    ///
    /// # Errors
    ///
    /// Returns an error when the entry is invalid, missing, not regular, or
    /// cannot be opened with both delete access and exclusive rename sharing.
    pub fn open_for_bound_replacement(&self, name: &OsStr) -> io::Result<File> {
        let options = combine_disjoint_flag_bits(
            combine_disjoint_flag_bits(FILE_NON_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT),
            FILE_SYNCHRONOUS_IO_NONALERT,
        );
        let access = combine_disjoint_flag_bits(
            combine_disjoint_flag_bits(GENERIC_READ, DELETE),
            SYNCHRONIZE,
        );
        let file = open_entry_relative_with(&self.handle, name, access, FILE_SHARE_READ, options)?;
        verify_regular_entry_handle(&file)?;
        Ok(file)
    }

    /// Classifies an entry through the retained directory without following a
    /// final reparse point. Directories and reparse points are not files.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is invalid, the entry is missing, or
    /// its attributes cannot be inspected.
    pub fn is_regular_file(&self, name: &OsStr) -> io::Result<bool> {
        let share = combine_disjoint_flag_bits(
            combine_disjoint_flag_bits(FILE_SHARE_READ, FILE_SHARE_WRITE),
            FILE_SHARE_DELETE,
        );
        let options =
            combine_disjoint_flag_bits(FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT);
        let file = open_entry_relative_with(
            &self.handle,
            name,
            combine_disjoint_flag_bits(FILE_READ_ATTRIBUTES, SYNCHRONIZE),
            share,
            options,
        )?;
        regular_entry_handle(&file)
    }

    /// Exclusively creates one owner-restricted entry relative to this handle.
    ///
    /// # Errors
    ///
    /// Returns an error without writing recovery bytes when the name is
    /// invalid, already exists, or private security cannot be established.
    pub fn create_private_new(&self, name: &OsStr) -> io::Result<File> {
        windows_create_private_new_at(&self.handle, name)
    }

    /// Lists at most `limit` entry names through the held directory handle.
    /// The enumeration cursor is shared by calls on one handle.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be enumerated or Windows
    /// returns an invalid directory-entry buffer.
    pub fn entry_names(&self, limit: usize) -> io::Result<Vec<OsString>> {
        let _guard = self
            .enumeration_lock
            .lock()
            .map_err(|_| io::Error::other("recovery directory enumeration lock is unavailable"))?;
        entry_names_from_handle(&self.handle, limit)
    }

    /// Installs the exact opened stage under a new name in this directory.
    /// An existing destination is never replaced.
    ///
    /// # Errors
    ///
    /// Returns an error without consuming the stage when the name is invalid,
    /// the destination exists, or the rename cannot complete.
    pub fn install_new_from_open(
        &self,
        stage: &File,
        destination: &OsStr,
    ) -> io::Result<CommitReceipt<InstallNewOutcome>> {
        let name = WindowsRecoveryEntryName::new(destination)?;
        let units: Vec<u16> = name.as_os_str().encode_wide().collect();
        let name_bytes = units
            .len()
            .checked_mul(size_of::<u16>())
            .ok_or_else(invalid_entry_name_error)?;
        let file_name_length = u32::try_from(name_bytes).map_err(|_| invalid_entry_name_error())?;
        let required_bytes = offset_of!(FILE_RENAME_INFORMATION, FileName)
            .checked_add(name_bytes)
            .and_then(|length| length.checked_add(size_of::<u16>()))
            .map(|length| length.max(size_of::<FILE_RENAME_INFORMATION>()))
            .ok_or_else(invalid_entry_name_error)?;
        let word_count = required_bytes.div_ceil(size_of::<usize>());
        let mut buffer = vec![0_usize; word_count];
        let buffer_bytes = u32::try_from(
            buffer
                .len()
                .checked_mul(size_of::<usize>())
                .ok_or_else(invalid_entry_name_error)?,
        )
        .map_err(|_| invalid_entry_name_error())?;
        let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFORMATION>();
        let mut status_block = IO_STATUS_BLOCK::default();
        // SAFETY: the usize allocation is at least as aligned as
        // FILE_RENAME_INFORMATION and large enough for its header and validated
        // UTF-16 component. The source file, destination directory, buffer,
        // and writable status block remain live for this synchronous call.
        // The kernel does not retain pointers.
        #[allow(unsafe_code)]
        let status = unsafe {
            (*info).Anonymous.ReplaceIfExists = false;
            (*info).RootDirectory = self.handle.as_raw_handle();
            (*info).FileNameLength = file_name_length;
            std::ptr::copy_nonoverlapping(
                units.as_ptr(),
                std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
                units.len(),
            );
            NtSetInformationFile(
                stage.as_raw_handle(),
                &raw mut status_block,
                info.cast(),
                buffer_bytes,
                FileRenameInformation,
            )
        };
        if status != 0 {
            // SAFETY: this status came directly from the preceding native call.
            #[allow(unsafe_code)]
            let code = unsafe { RtlNtStatusToDosError(status) };
            return Err(io::Error::from_raw_os_error(code.cast_signed()));
        }
        Ok(CommitReceipt::new(
            InstallNewOutcome::Clean,
            ParentSyncReceipt::windows_unsupported(),
        ))
    }
}

fn malformed_directory_entries() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Windows returned malformed recovery directory entries",
    )
}

fn directory_entry_u32(bytes: &[u8], offset: usize) -> io::Result<u32> {
    let field = bytes
        .get(offset..offset + size_of::<u32>())
        .ok_or_else(malformed_directory_entries)?;
    let mut value = [0_u8; size_of::<u32>()];
    value.copy_from_slice(field);
    Ok(u32::from_ne_bytes(value))
}

fn parse_directory_entry_batch(bytes: &[u8], limit: usize) -> io::Result<Vec<OsString>> {
    let mut names = Vec::new();
    let name_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
    let next_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, NextEntryOffset);
    let name_length_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength);
    let mut offset = 0_usize;
    loop {
        let entry = bytes
            .get(offset..)
            .ok_or_else(malformed_directory_entries)?;
        let next = usize::try_from(directory_entry_u32(entry, next_offset)?)
            .map_err(|_| malformed_directory_entries())?;
        let name_length = usize::try_from(directory_entry_u32(entry, name_length_offset)?)
            .map_err(|_| malformed_directory_entries())?;
        if next != 0 && next >= entry.len() {
            return Err(malformed_directory_entries());
        }
        if next != 0 && !next.is_multiple_of(8) {
            return Err(malformed_directory_entries());
        }
        let record_length = if next == 0 { entry.len() } else { next };
        let name_end = name_offset
            .checked_add(name_length)
            .ok_or_else(malformed_directory_entries)?;
        if name_length == 0
            || !name_length.is_multiple_of(size_of::<u16>())
            || name_end > record_length
        {
            return Err(malformed_directory_entries());
        }
        let name_bytes = &entry[name_offset..name_end];
        let units: Vec<u16> = name_bytes
            .chunks_exact(size_of::<u16>())
            .map(|unit| u16::from_ne_bytes([unit[0], unit[1]]))
            .collect();
        let name = OsString::from_wide(&units);
        if name != "." && name != ".." {
            names.push(name);
            if names.len() == limit {
                return Ok(names);
            }
        }
        if next == 0 {
            return Ok(names);
        }
        offset = offset
            .checked_add(next)
            .ok_or_else(malformed_directory_entries)?;
    }
}

fn entry_names_from_handle(directory: &File, limit: usize) -> io::Result<Vec<OsString>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    let mut buffer = vec![0_u64; DIRECTORY_ENUMERATION_BUFFER_BYTES / size_of::<u64>()];
    let mut information_class = FileIdBothDirectoryRestartInfo;
    let buffer_size = u32::try_from(DIRECTORY_ENUMERATION_BUFFER_BYTES)
        .map_err(|_| malformed_directory_entries())?;
    loop {
        buffer.fill(0);
        // SAFETY: the aligned, initialized buffer is writable for its exact
        // byte length. The directory handle and buffer remain live for this
        // synchronous query, and Windows retains neither pointer.
        #[allow(unsafe_code)]
        let success = unsafe {
            GetFileInformationByHandleEx(
                directory.as_raw_handle(),
                information_class,
                buffer.as_mut_ptr().cast(),
                buffer_size,
            )
        };
        if success == 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_NO_MORE_FILES.cast_signed()) {
                return Ok(names);
            }
            return Err(error);
        }
        // SAFETY: the allocation contains exactly this many initialized bytes;
        // the parser bounds-checks every record and name before decoding them.
        #[allow(unsafe_code)]
        let bytes = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr().cast::<u8>(),
                DIRECTORY_ENUMERATION_BUFFER_BYTES,
            )
        };
        names.extend(parse_directory_entry_batch(bytes, limit - names.len())?);
        if names.len() == limit {
            return Ok(names);
        }
        information_class = FileIdBothDirectoryInfo;
    }
}

fn open_entry_relative(directory: &File, name: &OsStr, for_cleanup: bool) -> io::Result<File> {
    let access = if for_cleanup {
        combine_disjoint_flag_bits(
            combine_disjoint_flag_bits(GENERIC_READ, DELETE),
            SYNCHRONIZE,
        )
    } else {
        combine_disjoint_flag_bits(GENERIC_READ, SYNCHRONIZE)
    };
    let share = if for_cleanup {
        combine_disjoint_flag_bits(FILE_SHARE_READ, FILE_SHARE_DELETE)
    } else {
        combine_disjoint_flag_bits(
            combine_disjoint_flag_bits(FILE_SHARE_READ, FILE_SHARE_WRITE),
            FILE_SHARE_DELETE,
        )
    };
    let options = combine_disjoint_flag_bits(
        combine_disjoint_flag_bits(FILE_NON_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT),
        FILE_SYNCHRONOUS_IO_NONALERT,
    );
    let file = open_entry_relative_with(directory, name, access, share, options)?;
    verify_regular_entry_handle(&file)?;
    Ok(file)
}

fn open_entry_relative_with(
    directory: &File,
    name: &OsStr,
    access: u32,
    share: u32,
    options: u32,
) -> io::Result<File> {
    let name = WindowsRecoveryEntryName::new(name)?;
    let mut units: Vec<u16> = name.as_os_str().encode_wide().collect();
    let byte_length = units
        .len()
        .checked_mul(size_of::<u16>())
        .and_then(|length| u16::try_from(length).ok())
        .ok_or_else(invalid_entry_name_error)?;
    let unicode_name = UNICODE_STRING {
        Length: byte_length,
        MaximumLength: byte_length,
        Buffer: units.as_mut_ptr(),
    };
    let object_attributes = OBJECT_ATTRIBUTES {
        Length: u32::try_from(size_of::<OBJECT_ATTRIBUTES>()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows object attributes are too large",
            )
        })?,
        RootDirectory: directory.as_raw_handle(),
        ObjectName: &raw const unicode_name,
        Attributes: OBJ_CASE_INSENSITIVE,
        SecurityDescriptor: std::ptr::null(),
        SecurityQualityOfService: std::ptr::null(),
    };
    let mut handle: HANDLE = std::ptr::null_mut();
    let mut status_block = IO_STATUS_BLOCK::default();
    // SAFETY: `directory` stays open throughout the call. `units` is a live
    // validated UTF-16 component with an exact byte length; the object name,
    // attributes, output handle, and status block all point to initialized
    // writable or readable storage for the duration of this synchronous call.
    #[allow(unsafe_code)]
    let status = unsafe {
        NtCreateFile(
            &raw mut handle,
            access,
            &raw const object_attributes,
            &raw mut status_block,
            std::ptr::null(),
            FILE_ATTRIBUTE_NORMAL,
            share,
            FILE_OPEN,
            options,
            std::ptr::null(),
            0,
        )
    };
    if status != 0 {
        // SAFETY: the status value is returned by the preceding native call;
        // this conversion has no pointer or lifetime obligations.
        #[allow(unsafe_code)]
        let code = unsafe { RtlNtStatusToDosError(status) };
        return Err(io::Error::from_raw_os_error(code.cast_signed()));
    }
    if !nt_open_handle_usable(handle) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Windows returned an unusable recovery entry handle",
        ));
    }
    // SAFETY: a successful NtCreateFile returns one owned file handle, which
    // has not been wrapped or closed. File takes over its sole ownership.
    #[allow(unsafe_code)]
    let file = unsafe { File::from_raw_handle(handle) };
    Ok(file)
}

fn nt_open_handle_usable(handle: HANDLE) -> bool {
    !handle.is_null() && handle != INVALID_HANDLE_VALUE
}

fn verify_regular_entry_handle(file: &File) -> io::Result<()> {
    if !regular_entry_handle(file)? {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery entry must be an ordinary file",
        ));
    }
    Ok(())
}

fn regular_entry_handle(file: &File) -> io::Result<bool> {
    let mut basic = FILE_BASIC_INFO::default();
    let size = u32::try_from(size_of::<FILE_BASIC_INFO>()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "FILE_BASIC_INFO size does not fit the Windows API parameter",
        )
    })?;
    // SAFETY: `file` remains open and `basic` is writable for exactly `size`
    // bytes. The native function does not retain either pointer.
    #[allow(unsafe_code)]
    if unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileBasicInfo,
            (&raw mut basic).cast(),
            size,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let rejected =
        combine_disjoint_flag_bits(FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT);
    Ok(basic.FileAttributes & rejected == 0)
}

/// Retained Windows handles for the recovery directory hierarchy.
///
/// Construction fails before recovery content is written unless the state path
/// is an absolute drive path on a fixed-drive NTFS volume, every traversed
/// directory is a non-reparse disk directory on the same volume, the state
/// directory belongs to the current user, and principals other than that user,
/// SYSTEM, and Administrators have read-only access. The recovery directories
/// must have Noter's exact private owner and inheritable DACL policy. Newly
/// created directories receive that policy at creation time. Registered Cloud
/// Files sync roots are refused. Other synchronization and redirection models
/// remain outside the supported recovery boundary.
///
/// Entry creation, open, classification, enumeration, new-record installation,
/// existing-record backup and installation, and directory synchronization use
/// these handles.
pub struct WindowsRecoveryNamespace {
    state: WindowsRecoveryDirectory,
    recovery: WindowsRecoveryDirectory,
    records: WindowsRecoveryDirectory,
    quarantine: WindowsRecoveryDirectory,
    _traversal_guards: Vec<WindowsRecoveryDirectory>,
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
        Self::open_or_create_with_cloud_check(state_root, recovery_name, reject_cloud_sync_root)
    }

    fn open_or_create_with_cloud_check(
        state_root: &Path,
        recovery_name: &OsStr,
        mut check_cloud_root: impl FnMut(&File) -> io::Result<()>,
    ) -> io::Result<Self> {
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
        check_cloud_root(
            traversal_guards
                .last()
                .ok_or_else(invalid_state_root_error)?
                .handle(),
        )?;

        current.push(state_name);
        let state = bind_state_directory(&current, expected_volume)?;
        check_cloud_root(state.handle())?;
        current.push(recovery_name.as_os_str());
        let recovery = bind_private_directory(&current, expected_volume)?;
        check_cloud_root(recovery.handle())?;
        let mut records_path = current.clone();
        records_path.push(records_name.as_os_str());
        let records = bind_private_directory(&records_path, expected_volume)?;
        check_cloud_root(records.handle())?;
        current.push(quarantine_name.as_os_str());
        let quarantine = bind_private_directory(&current, expected_volume)?;
        check_cloud_root(quarantine.handle())?;

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

    /// The held recovery directory.
    #[must_use]
    pub const fn recovery(&self) -> &WindowsRecoveryDirectory {
        &self.recovery
    }

    /// The held records directory.
    #[must_use]
    pub const fn records(&self) -> &WindowsRecoveryDirectory {
        &self.records
    }

    /// The held quarantine directory.
    #[must_use]
    pub const fn quarantine(&self) -> &WindowsRecoveryDirectory {
        &self.quarantine
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

fn reject_cloud_sync_root(handle: &File) -> io::Result<()> {
    type QuerySyncRoot = unsafe extern "system" fn(
        HANDLE,
        CF_SYNC_ROOT_INFO_CLASS,
        *mut core::ffi::c_void,
        u32,
        *mut u32,
    ) -> i32;
    let info_size = u32::try_from(size_of::<CF_SYNC_ROOT_BASIC_INFO>()).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "cloud information size overflow",
        )
    })?;

    let library_path = system_cloud_library_path()?;
    let library_name = nul_terminated_path(&library_path)?;
    // SAFETY: the fully qualified System32 path stays live for the call. The
    // search flag restricts dependency resolution to System32 as well.
    #[allow(unsafe_code)]
    let library = unsafe {
        LoadLibraryExW(
            library_name.as_ptr(),
            std::ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if library.is_null() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows Cloud Files classification is unavailable",
        ));
    }
    let result = verify_loaded_cloud_library(library, &library_path).and_then(|()| {
        let symbol = b"CfGetSyncRootInfoByHandle\0";
        // SAFETY: the verified library handle is live, and `symbol` is
        // NUL-terminated.
        #[allow(unsafe_code)]
        let query = unsafe { GetProcAddress(library, symbol.as_ptr()) }.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Windows Cloud Files classification is unavailable",
            )
        })?;
        // SAFETY: the named export has the documented CfGetSyncRootInfoByHandle
        // signature. The library, directory handle, output structure, and
        // writable length remain live until the synchronous call returns.
        #[allow(unsafe_code)]
        let query: QuerySyncRoot = unsafe { std::mem::transmute(query) };
        let mut info = CF_SYNC_ROOT_BASIC_INFO::default();
        let mut returned = 0_u32;
        // SAFETY: the directory handle is live and the output pointers refer to
        // writable values of the advertised sizes for this synchronous call.
        #[allow(unsafe_code)]
        let status = unsafe {
            query(
                handle.as_raw_handle(),
                CF_SYNC_ROOT_INFO_BASIC,
                (&raw mut info).cast(),
                info_size,
                &raw mut returned,
            )
        };
        classify_cloud_sync_root_result(status)
    });
    // SAFETY: `library` is a live handle returned by LoadLibraryExW. The query
    // has completed, so no code or pointer from the library is used afterward.
    #[allow(unsafe_code)]
    unsafe {
        FreeLibrary(library)
    };
    result
}

fn system_cloud_library_path() -> io::Result<PathBuf> {
    let mut buffer = [0_u16; 512];
    let capacity = u32::try_from(buffer.len()).map_err(|_| invalid_state_root_error())?;
    // SAFETY: the output buffer is writable for `capacity` UTF-16 units.
    #[allow(unsafe_code)]
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), capacity) };
    let length = bounded_windows_path_length(length, capacity).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows system directory cannot be established",
        )
    })?;
    let mut path = PathBuf::from(OsString::from_wide(&buffer[..length]));
    path.push("cldapi.dll");
    Ok(path)
}

fn verify_loaded_cloud_library(library: HMODULE, expected: &Path) -> io::Result<()> {
    let mut buffer = [0_u16; 512];
    let capacity = u32::try_from(buffer.len()).map_err(|_| invalid_state_root_error())?;
    // SAFETY: `library` is a live module handle and the output buffer is
    // writable for `capacity` UTF-16 units.
    #[allow(unsafe_code)]
    let length = unsafe { GetModuleFileNameW(library, buffer.as_mut_ptr(), capacity) };
    let length = bounded_windows_path_length(length, capacity).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows Cloud Files library origin cannot be established",
        )
    })?;
    let actual = OsString::from_wide(&buffer[..length]);
    if !actual
        .to_string_lossy()
        .eq_ignore_ascii_case(expected.as_os_str().to_string_lossy().as_ref())
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows Cloud Files library did not come from System32",
        ));
    }
    Ok(())
}

fn bounded_windows_path_length(length: u32, capacity: u32) -> Option<usize> {
    if length == 0 || length >= capacity {
        return None;
    }
    usize::try_from(length).ok()
}

fn classify_cloud_sync_root_result(status: i32) -> io::Result<()> {
    if status == 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "recovery state cannot use a Windows Cloud Files sync root",
        ));
    }
    // HRESULT_FROM_WIN32 places this Win32 error in the low 16 bits.
    let not_under_sync_root =
        (0x8007_0000_u32 + ERROR_CLOUD_FILE_NOT_UNDER_SYNC_ROOT).cast_signed();
    if status == not_under_sync_root {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("Windows Cloud Files classification failed with HRESULT {status:#010x}"),
    ))
}

fn bind_existing_directory(
    path: &Path,
    expected_volume: Option<u64>,
) -> io::Result<WindowsRecoveryDirectory> {
    let handle = open_directory_no_follow(path, DirectorySharePolicy::Traversal)?;
    let identity = verify_directory_handle(&handle)?;
    if expected_volume.is_some_and(|volume| volume != identity.volume_serial) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery state directory crossed a volume boundary",
        ));
    }
    Ok(WindowsRecoveryDirectory {
        handle,
        enumeration_lock: Mutex::new(()),
        identity,
        path: path.to_path_buf(),
    })
}

fn bind_state_directory(path: &Path, expected_volume: u64) -> io::Result<WindowsRecoveryDirectory> {
    let handle = open_or_create_private_directory(path)?;
    let identity = verify_directory_handle(&handle)?;
    if identity.volume_serial != expected_volume {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery state directory crossed a volume boundary",
        ));
    }
    windows_verify_owner_controlled_state_directory(&handle)?;
    Ok(WindowsRecoveryDirectory {
        handle,
        enumeration_lock: Mutex::new(()),
        identity,
        path: path.to_path_buf(),
    })
}

fn bind_private_directory(
    path: &Path,
    expected_volume: u64,
) -> io::Result<WindowsRecoveryDirectory> {
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
    Ok(WindowsRecoveryDirectory {
        handle,
        enumeration_lock: Mutex::new(()),
        identity,
        path: path.to_path_buf(),
    })
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
            combine_disjoint_flag_bits(WRITE_DAC, FILE_ADD_FILE),
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
    use std::ffi::{OsStr, OsString};
    use std::fs::{self, File};
    use std::io::{self, Read, Write};
    use std::mem::{offset_of, size_of};
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::os::windows::fs::{OpenOptionsExt, symlink_dir, symlink_file};
    use std::path::Path;
    use std::sync::Mutex;

    use tempfile::tempdir;

    use super::{
        DirectoryCreationError, DirectoryOpenError, DirectorySharePolicy, ParsedStatePath,
        WindowsRecoveryDirectory, WindowsRecoveryEntryName, WindowsRecoveryNamespace,
        bounded_windows_path_length, classify_cloud_sync_root_result,
        classify_directory_creation_error, classify_directory_open_error,
        directory_attributes_are_safe, entry_names_from_handle, nt_open_handle_usable,
        open_directory_no_follow, parse_directory_entry_batch, query_preferred_identity,
        reject_cloud_sync_root, system_cloud_library_path, verify_fixed_drive,
        verify_loaded_cloud_library, verify_ntfs, windows_local_appdata_directory,
    };
    use crate::imp::{
        windows_create_owner_controlled_readable_directory_for_test,
        windows_verify_private_directory_security,
    };
    use crate::{InstallNewOutcome, ParentSyncOutcome};
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows_sys::Win32::Storage::CloudFilters::{
        CF_HYDRATION_POLICY_ALWAYS_FULL, CF_POPULATION_POLICY_ALWAYS_FULL, CF_REGISTER_FLAG_NONE,
        CF_SYNC_POLICIES, CF_SYNC_REGISTRATION, CfRegisterSyncRoot, CfUnregisterSyncRoot,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_ID_BOTH_DIR_INFO, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    #[test]
    fn native_open_handle_check_rejects_both_failure_sentinels() {
        assert!(!nt_open_handle_usable(std::ptr::null_mut()));
        assert!(!nt_open_handle_usable(INVALID_HANDLE_VALUE));
        assert!(nt_open_handle_usable(std::ptr::dangling_mut()));
    }

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
    fn local_appdata_known_folder_is_an_absolute_directory() -> io::Result<()> {
        let path = windows_local_appdata_directory()?;
        assert!(path.is_absolute());
        assert!(path.is_dir());
        Ok(())
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
    fn cloud_sync_root_result_accepts_only_the_specific_non_root_status() {
        let not_under_sync_root = 0x8007_0186_u32.cast_signed();
        assert!(classify_cloud_sync_root_result(not_under_sync_root).is_ok());
        assert_eq!(
            classify_cloud_sync_root_result(0).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            classify_cloud_sync_root_result(-1).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }

    #[test]
    fn windows_path_length_requires_a_nonempty_terminated_buffer() {
        assert_eq!(bounded_windows_path_length(0, 512), None);
        assert_eq!(bounded_windows_path_length(511, 512), Some(511));
        assert_eq!(bounded_windows_path_length(512, 512), None);
        assert_eq!(bounded_windows_path_length(513, 512), None);
    }

    #[test]
    fn native_cloud_sync_root_query_accepts_an_ordinary_local_directory() -> io::Result<()> {
        let local = tempdir()?;
        let open_directory = |path: &Path| {
            fs::OpenOptions::new()
                .access_mode(FILE_READ_ATTRIBUTES)
                .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(path)
        };
        reject_cloud_sync_root(&open_directory(local.path())?)?;
        Ok(())
    }

    #[test]
    fn native_cloud_sync_root_query_refuses_a_registered_subtree() -> io::Result<()> {
        struct RegisteredSyncRoot {
            path: Vec<u16>,
            registered: bool,
        }

        impl RegisteredSyncRoot {
            fn unregister(&mut self) -> io::Result<()> {
                if !self.registered {
                    return Ok(());
                }
                // SAFETY: the registered path remains NUL-terminated and live.
                #[allow(unsafe_code)]
                let status = unsafe { CfUnregisterSyncRoot(self.path.as_ptr()) };
                if status != 0 {
                    return Err(io::Error::other(format!(
                        "Cloud Files fixture unregistration failed: {status:#x}"
                    )));
                }
                self.registered = false;
                Ok(())
            }
        }

        impl Drop for RegisteredSyncRoot {
            fn drop(&mut self) {
                let _ = self.unregister();
            }
        }

        let local = tempdir()?;
        let child = local.path().join("child");
        fs::create_dir(&child)?;
        let path: Vec<u16> = local
            .path()
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let provider_name: Vec<u16> = OsStr::new("Noter test fixture")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let provider_version: Vec<u16> = OsStr::new("1")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let registration = CF_SYNC_REGISTRATION {
            StructSize: u32::try_from(size_of::<CF_SYNC_REGISTRATION>())
                .expect("Cloud Files registration structure fits in u32"),
            ProviderName: provider_name.as_ptr(),
            ProviderVersion: provider_version.as_ptr(),
            ProviderId: windows_sys::core::GUID::from_u128(
                0x9f95_c9cd_3494_4c57_9e8b_70a4_2e35_9bde,
            ),
            ..Default::default()
        };
        let mut policies = CF_SYNC_POLICIES {
            StructSize: u32::try_from(size_of::<CF_SYNC_POLICIES>())
                .expect("Cloud Files policy structure fits in u32"),
            ..Default::default()
        };
        policies.Hydration.Primary = CF_HYDRATION_POLICY_ALWAYS_FULL;
        policies.Population.Primary = CF_POPULATION_POLICY_ALWAYS_FULL;
        // SAFETY: all registration fields and pointed-to strings remain live
        // through the synchronous call. The temporary directory is writable.
        #[allow(unsafe_code)]
        let status = unsafe {
            CfRegisterSyncRoot(
                path.as_ptr(),
                &raw const registration,
                &raw const policies,
                CF_REGISTER_FLAG_NONE,
            )
        };
        assert_eq!(
            status, 0,
            "Cloud Files fixture registration failed: {status:#x}"
        );
        let mut registration_guard = RegisteredSyncRoot {
            path,
            registered: true,
        };
        let opened = fs::OpenOptions::new()
            .access_mode(FILE_READ_ATTRIBUTES)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&child)?;
        assert_eq!(
            reject_cloud_sync_root(&opened).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        drop(opened);
        registration_guard.unregister()?;
        Ok(())
    }

    #[test]
    fn native_cloud_sync_root_query_refuses_a_non_filesystem_handle() -> io::Result<()> {
        let device = File::open("NUL")?;
        assert_eq!(
            reject_cloud_sync_root(&device).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        Ok(())
    }

    #[test]
    fn bound_records_directory_accepts_a_native_metadata_barrier() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;
        assert_eq!(namespace.records().sync()?, ParentSyncOutcome::Synced);
        Ok(())
    }

    #[test]
    fn directory_barrier_reports_a_missing_write_right() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;
        let path = namespace.records().path().to_path_buf();
        let handle = open_directory_no_follow(&path, DirectorySharePolicy::Traversal)?;
        let identity = query_preferred_identity(&handle)?;
        let read_only = WindowsRecoveryDirectory {
            handle,
            enumeration_lock: Mutex::new(()),
            identity,
            path,
        };
        assert_eq!(
            read_only.sync().unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        Ok(())
    }

    #[test]
    fn cloud_library_origin_rejects_the_process_module() -> io::Result<()> {
        let expected = system_cloud_library_path()?;
        assert!(expected.is_absolute());
        assert_eq!(expected.file_name(), Some(OsStr::new("cldapi.dll")));
        // A null HMODULE identifies the process image for GetModuleFileNameW,
        // which cannot satisfy the exact System32 Cloud Files library path.
        assert_eq!(
            verify_loaded_cloud_library(std::ptr::null_mut(), &expected)
                .unwrap_err()
                .kind(),
            io::ErrorKind::Unsupported
        );
        Ok(())
    }

    #[test]
    fn cloud_root_checks_cover_every_bound_directory_before_content() -> io::Result<()> {
        for rejected_check in 1..=5 {
            let parent = tempdir()?;
            let state = parent.path().join("state");
            let recovery = state.join("recovery");
            let expected_paths = [
                parent.path().to_path_buf(),
                state.clone(),
                recovery.clone(),
                recovery.join("records"),
                recovery.join("quarantine"),
            ];
            let mut checks = 0;
            let error = WindowsRecoveryNamespace::open_or_create_with_cloud_check(
                &state,
                OsStr::new("recovery"),
                |handle| {
                    let expected = fs::OpenOptions::new()
                        .access_mode(FILE_READ_ATTRIBUTES)
                        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
                        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                        .open(&expected_paths[checks])?;
                    assert_eq!(
                        query_preferred_identity(handle)?,
                        query_preferred_identity(&expected)?
                    );
                    checks += 1;
                    if checks == rejected_check {
                        Err(io::Error::new(
                            io::ErrorKind::Unsupported,
                            "injected cloud sync root",
                        ))
                    } else {
                        Ok(())
                    }
                },
            )
            .err()
            .expect("every bound directory must be checked");
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert_eq!(checks, rejected_check);
        }
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
    fn entries_open_relative_to_held_directories_and_reject_final_links() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;
        let record = namespace.records().path().join("entry.rec");
        let quarantined = namespace.quarantine().path().join("entry.rec");
        let disposable = namespace.quarantine().path().join("delete.rec");
        fs::write(&record, b"record")?;
        fs::write(&quarantined, b"quarantine")?;
        fs::write(&disposable, b"delete")?;
        assert!(
            namespace
                .records()
                .is_regular_file(OsStr::new("entry.rec"))?
        );
        let directory = namespace.records().path().join("folder.rec");
        fs::create_dir(&directory)?;
        assert!(
            !namespace
                .records()
                .is_regular_file(OsStr::new("folder.rec"))?
        );

        let mut opened = namespace.records().open_existing(OsStr::new("entry.rec"))?;
        let mut bytes = Vec::new();
        opened.read_to_end(&mut bytes)?;
        assert_eq!(bytes, b"record");
        let raced_path = namespace.records().path().join("raced.rec");
        let moved_path = namespace.records().path().join("moved.rec");
        fs::write(&raced_path, b"original")?;
        let mut raced = namespace.records().open_existing(OsStr::new("raced.rec"))?;
        fs::rename(&raced_path, &moved_path)?;
        fs::write(&raced_path, b"replacement")?;
        bytes.clear();
        raced.read_to_end(&mut bytes)?;
        assert_eq!(bytes, b"original");
        assert_eq!(fs::read(&raced_path)?, b"replacement");
        let mut opened = namespace
            .quarantine()
            .open_for_cleanup(OsStr::new("entry.rec"))?;
        bytes.clear();
        opened.read_to_end(&mut bytes)?;
        assert_eq!(bytes, b"quarantine");
        let delete_handle = namespace
            .quarantine()
            .open_for_cleanup(OsStr::new("delete.rec"))?;
        crate::delete_open_file(&delete_handle)?;
        drop(delete_handle);
        assert!(!disposable.exists());

        let link = namespace.records().path().join("link.rec");
        symlink_file(&quarantined, &link)?;
        assert!(
            !namespace
                .records()
                .is_regular_file(OsStr::new("link.rec"))?
        );
        assert_eq!(
            namespace
                .records()
                .open_existing(OsStr::new("link.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            namespace
                .records()
                .open_for_cleanup(OsStr::new("link.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            namespace
                .records()
                .open_existing(OsStr::new("..\\entry.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            namespace
                .records()
                .open_existing(OsStr::new("missing.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            namespace
                .records()
                .is_regular_file(OsStr::new("missing.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(fs::read(&quarantined)?, b"quarantine");
        assert!(super::verify_regular_entry_handle(&File::open("NUL")?).is_err());
        Ok(())
    }

    #[test]
    fn reconciliation_open_blocks_mutation_and_rejects_final_links() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;
        let record = namespace.records().path().join("entry.rec");
        let moved = namespace.records().path().join("moved.rec");
        fs::write(&record, b"record")?;

        let mut opened = namespace
            .records()
            .open_for_reconciliation(OsStr::new("entry.rec"))?;
        let mut bytes = Vec::new();
        opened.read_to_end(&mut bytes)?;
        assert_eq!(bytes, b"record");
        assert!(fs::OpenOptions::new().write(true).open(&record).is_err());
        assert!(fs::rename(&record, &moved).is_err());
        assert!(fs::remove_file(&record).is_err());
        drop(opened);

        assert_eq!(
            namespace
                .records()
                .open_for_reconciliation(OsStr::new("..\\entry.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        let link = namespace.records().path().join("link.rec");
        symlink_file(&record, &link)?;
        assert_eq!(
            namespace
                .records()
                .open_for_reconciliation(OsStr::new("link.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
        fs::rename(&record, &moved)?;
        Ok(())
    }

    #[test]
    fn reconciliation_open_uses_held_directory_after_path_rebind() -> io::Result<()> {
        let parent = tempdir()?;
        let original = parent.path().join("original");
        let moved = parent.path().join("moved");
        fs::create_dir(&original)?;
        fs::write(original.join("entry.rec"), b"original")?;
        let handle = fs::OpenOptions::new()
            .access_mode(FILE_LIST_DIRECTORY)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&original)?;
        let identity = query_preferred_identity(&handle)?;
        let directory = WindowsRecoveryDirectory {
            handle,
            enumeration_lock: std::sync::Mutex::new(()),
            identity,
            path: original.clone(),
        };
        fs::rename(&original, &moved)?;
        fs::create_dir(&original)?;
        fs::write(original.join("entry.rec"), b"decoy")?;

        let mut opened = directory.open_for_reconciliation(OsStr::new("entry.rec"))?;
        let mut bytes = Vec::new();
        opened.read_to_end(&mut bytes)?;
        assert_eq!(bytes, b"original");
        assert_eq!(fs::read(original.join("entry.rec"))?, b"decoy");
        Ok(())
    }

    #[test]
    fn private_entry_creation_is_exclusive_and_relative_to_the_held_directory() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;
        let name = OsStr::new("created.rec");
        let path = namespace.records().path().join(name);
        let mut created = namespace.records().create_private_new(name)?;
        created.write_all(b"private record")?;
        assert_eq!(
            namespace
                .records()
                .create_private_new(name)
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&path)?, b"private record");
        assert!(!namespace.quarantine().path().join(name).exists());
        assert_eq!(
            namespace
                .records()
                .create_private_new(OsStr::new("..\\escape.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        Ok(())
    }

    #[test]
    fn opened_stage_installs_exclusively_relative_to_the_held_directory() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;
        let records = namespace.records();
        let stage_path = records.path().join("stage.rec");
        let moved_path = records.path().join("moved.rec");
        let destination = records.path().join("installed.rec");
        let mut opened_stage = records.create_private_new(OsStr::new("stage.rec"))?;
        opened_stage.write_all(b"verified stage")?;
        opened_stage.sync_all()?;

        fs::rename(&stage_path, &moved_path)?;
        fs::write(&stage_path, b"rebound stage name")?;
        let receipt = records.install_new_from_open(&opened_stage, OsStr::new("installed.rec"))?;
        let (outcome, parent_sync) = receipt.into_parts();
        assert!(matches!(outcome, InstallNewOutcome::Clean));
        assert!(matches!(
            parent_sync.sync()?,
            crate::ParentSyncOutcome::Unsupported
        ));
        assert_eq!(fs::read(&destination)?, b"verified stage");
        assert_eq!(fs::read(&stage_path)?, b"rebound stage name");
        assert!(!moved_path.exists());

        let collision_path = records.path().join("collision.rec");
        let mut collision = records.create_private_new(OsStr::new("collision.rec"))?;
        collision.write_all(b"preserved stage")?;
        assert_eq!(
            records
                .install_new_from_open(&collision, OsStr::new("installed.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&destination)?, b"verified stage");
        assert_eq!(fs::read(&collision_path)?, b"preserved stage");
        assert_eq!(
            records
                .install_new_from_open(&collision, OsStr::new("stream.rec:alt"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        Ok(())
    }

    #[test]
    fn held_predecessor_can_move_to_backup_before_exclusive_stage_install() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;
        let records = namespace.records();
        let stage_path = records.path().join("stage.rec");
        let destination_path = records.path().join("current.rec");
        let backup_path = records.path().join("backup.rec");
        let competitor_path = records.path().join("competitor.rec");
        fs::write(&stage_path, b"new snapshot")?;
        fs::write(&destination_path, b"old snapshot")?;
        fs::write(&competitor_path, b"raced snapshot")?;

        let opened_stage = records.open_for_bound_replacement(OsStr::new("stage.rec"))?;
        let predecessor = records.open_for_bound_replacement(OsStr::new("current.rec"))?;

        assert!(fs::rename(&competitor_path, &destination_path).is_err());
        fs::write(&backup_path, b"retained backup")?;
        assert_eq!(
            records
                .install_new_from_open(&predecessor, OsStr::new("backup.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&destination_path)?, b"old snapshot");
        assert_eq!(fs::read(&backup_path)?, b"retained backup");
        fs::remove_file(&backup_path)?;
        let (outcome, _) = records
            .install_new_from_open(&predecessor, OsStr::new("backup.rec"))?
            .into_parts();
        assert!(matches!(outcome, InstallNewOutcome::Clean));
        assert!(matches!(records.sync()?, ParentSyncOutcome::Synced));
        assert!(!destination_path.exists());
        assert_eq!(fs::read(&backup_path)?, b"old snapshot");

        fs::rename(&competitor_path, &destination_path)?;
        assert_eq!(
            records
                .install_new_from_open(&opened_stage, OsStr::new("current.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(fs::read(&destination_path)?, b"raced snapshot");
        assert_eq!(fs::read(&stage_path)?, b"new snapshot");
        assert_eq!(fs::read(&backup_path)?, b"old snapshot");

        fs::remove_file(&destination_path)?;
        let (outcome, _) = records
            .install_new_from_open(&opened_stage, OsStr::new("current.rec"))?
            .into_parts();
        assert!(matches!(outcome, InstallNewOutcome::Clean));
        assert!(matches!(records.sync()?, ParentSyncOutcome::Synced));
        assert_eq!(fs::read(&destination_path)?, b"new snapshot");
        assert_eq!(fs::read(&backup_path)?, b"old snapshot");
        crate::delete_open_file(&predecessor)?;
        drop(predecessor);
        assert!(!backup_path.exists());
        assert!(matches!(records.sync()?, ParentSyncOutcome::Synced));
        Ok(())
    }

    #[test]
    fn held_directory_enumeration_is_bounded_and_restarts_after_a_partial_read() -> io::Result<()> {
        let parent = tempdir()?;
        let state = parent.path().join("state");
        let namespace = WindowsRecoveryNamespace::open_or_create(&state, OsStr::new("recovery"))?;
        let records = namespace.records();
        assert!(records.entry_names(1)?.is_empty());
        let expected: Vec<_> = (0..200)
            .map(|index| format!("{index:03}-{}", "x".repeat(140)))
            .collect();
        for name in &expected {
            fs::create_dir(records.path().join(name))?;
        }
        assert!(records.entry_names(0)?.is_empty());
        assert_eq!(records.entry_names(199)?.len(), 199);
        let mut actual: Vec<_> = records
            .entry_names(201)?
            .into_iter()
            .map(|name| name.to_string_lossy().into_owned())
            .collect();
        actual.sort();
        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn directory_entry_parser_rejects_invalid_record_lengths() -> io::Result<()> {
        let mut bytes = vec![0_u8; 256.max(size_of::<FILE_ID_BOTH_DIR_INFO>())];
        let name_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
        let name_length_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength);
        bytes[name_offset..name_offset + 2].copy_from_slice(&u16::from(b'a').to_ne_bytes());
        bytes[name_length_offset..name_length_offset + 4].copy_from_slice(&2_u32.to_ne_bytes());
        assert_eq!(
            parse_directory_entry_batch(&bytes, 1)?,
            vec![OsString::from("a")]
        );

        for invalid_name_length in [0_u32, 3, 1024] {
            bytes[name_length_offset..name_length_offset + 4]
                .copy_from_slice(&invalid_name_length.to_ne_bytes());
            assert_eq!(
                parse_directory_entry_batch(&bytes, 1).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        bytes[name_length_offset..name_length_offset + 4].copy_from_slice(&2_u32.to_ne_bytes());
        for invalid_next_offset in [1_u32, 8, 110, 256, 264, u32::MAX] {
            bytes[..4].copy_from_slice(&invalid_next_offset.to_ne_bytes());
            assert_eq!(
                parse_directory_entry_batch(&bytes, 1).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        Ok(())
    }

    #[test]
    fn directory_entry_parser_preserves_wide_names_and_rejects_unsafe_components() -> io::Result<()>
    {
        let names = [
            vec![0x6f22],
            vec![0xd800, 0xdf48],
            vec![u16::from(b'a'), 0x0001],
            vec![0xd800],
        ];
        let name_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
        let name_length_offset = offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength);
        let mut bytes = Vec::new();
        for (index, units) in names.iter().enumerate() {
            let record_length = (name_offset + units.len() * size_of::<u16>()).next_multiple_of(8);
            let start = bytes.len();
            bytes.resize(start + record_length, 0);
            if index + 1 < names.len() {
                bytes[start..start + 4].copy_from_slice(
                    &u32::try_from(record_length)
                        .map_err(io::Error::other)?
                        .to_ne_bytes(),
                );
            }
            bytes[start + name_length_offset..start + name_length_offset + 4].copy_from_slice(
                &u32::try_from(units.len() * size_of::<u16>())
                    .map_err(io::Error::other)?
                    .to_ne_bytes(),
            );
            for (unit_index, unit) in units.iter().enumerate() {
                let position = start + name_offset + unit_index * size_of::<u16>();
                bytes[position..position + 2].copy_from_slice(&unit.to_ne_bytes());
            }
        }

        let expected: Vec<_> = names
            .iter()
            .map(|units| OsString::from_wide(units))
            .collect();
        assert_eq!(parse_directory_entry_batch(&bytes, names.len())?, expected);
        for name in &expected[..2] {
            assert!(WindowsRecoveryEntryName::new(name).is_ok());
        }
        for name in &expected[2..] {
            assert_eq!(
                WindowsRecoveryEntryName::new(name).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
        }
        Ok(())
    }

    #[test]
    fn enumeration_uses_the_opened_directory_after_path_rebind() -> io::Result<()> {
        let parent = tempdir()?;
        let original = parent.path().join("original");
        let moved = parent.path().join("moved");
        fs::create_dir(&original)?;
        fs::write(original.join("original.rec"), b"original")?;
        let directory = fs::OpenOptions::new()
            .access_mode(FILE_LIST_DIRECTORY)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&original)?;
        fs::rename(&original, &moved)?;
        fs::create_dir(&original)?;
        fs::write(original.join("decoy.rec"), b"decoy")?;

        assert_eq!(
            entry_names_from_handle(&directory, 10)?,
            vec![OsStr::new("original.rec")]
        );
        assert_eq!(fs::read(original.join("decoy.rec"))?, b"decoy");
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
