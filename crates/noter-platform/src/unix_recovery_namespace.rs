//! Unix recovery-directory namespace binding.
//!
//! The state directory is reached one component at a time without following
//! links. The recovery, records, and quarantine directories are then created
//! or opened through their held parents. Before any recovery content can be
//! written, each directory is verified: every ancestor is owned by the
//! superuser or this user and cannot be changed by other users, the state
//! directory is owned by this user and closed to writes by others, the
//! recovery subtree is private to this user and on the state directory's
//! device with no extended ACL on macOS, and the file system is not a known
//! network or shared one.
//!
//! Every entry operation is relative to a held descriptor, so renaming or
//! replacing any ancestor after binding cannot redirect recovery content.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::os::fd::AsFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};

use rustix::fs::{
    AtFlags, CWD, Dir, FileType, Mode, OFlags, Stat, fchmod, fstat, fsync, mkdirat, openat, statat,
    unlinkat,
};
use rustix::process::{getegid, geteuid};

use crate::UnixRecoveryCommitParent;

const RECORDS_DIRECTORY_NAME: &str = "records";
const QUARANTINE_DIRECTORY_NAME: &str = "quarantine";
const GROUP_OR_OTHER_WRITE: u32 = 0o022;
const GROUP_WRITE: u32 = 0o020;
const OTHER_WRITE: u32 = 0o002;
const GROUP_OR_OTHER_ANY: u32 = 0o077;
const STICKY: u32 = 0o1000;

/// Held, verified recovery directories for one session.
#[derive(Debug)]
pub struct UnixRecoveryNamespace {
    state: UnixRecoveryDirectory,
    recovery: UnixRecoveryDirectory,
    records: UnixRecoveryDirectory,
    quarantine: UnixRecoveryDirectory,
}

impl UnixRecoveryNamespace {
    /// Opens or creates `state_root/recovery_name` with its records and
    /// quarantine directories, and verifies the whole chain before returning.
    ///
    /// Links in the part of `state_root` that already exists are resolved once,
    /// so a system link such as `/var` on macOS keeps working; every component
    /// is then opened again without following links and verified.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] for a relative root or an
    /// invalid directory name, [`io::ErrorKind::PermissionDenied`] when a
    /// directory is owned by another user or can be changed by one,
    /// [`io::ErrorKind::Unsupported`] for a network or shared file system or a
    /// recovery directory on another device, and the operating-system error
    /// when a component is a link, is not a directory, or cannot be opened.
    pub fn open_or_create(state_root: &Path, recovery_name: &OsStr) -> io::Result<Self> {
        if !state_root.is_absolute() {
            return Err(invalid_input("the state directory path must be absolute"));
        }
        validate_entry_name(recovery_name)?;
        let user = User::current();
        let state = bind_state_directory(state_root, user)?;
        require_local_file_system(&state.directory, &state.path)?;
        let state_status = fstat(&state.directory)?;
        let recovery = state.bind_private_child(recovery_name, user, &state_status)?;
        let records =
            recovery.bind_private_child(OsStr::new(RECORDS_DIRECTORY_NAME), user, &state_status)?;
        let quarantine = recovery.bind_private_child(
            OsStr::new(QUARANTINE_DIRECTORY_NAME),
            user,
            &state_status,
        )?;
        Ok(Self {
            state,
            recovery,
            records,
            quarantine,
        })
    }

    /// The verified state directory.
    #[must_use]
    pub const fn state(&self) -> &UnixRecoveryDirectory {
        &self.state
    }

    /// The private recovery directory.
    #[must_use]
    pub const fn recovery(&self) -> &UnixRecoveryDirectory {
        &self.recovery
    }

    /// The private directory that holds recovery records.
    #[must_use]
    pub const fn records(&self) -> &UnixRecoveryDirectory {
        &self.records
    }

    /// The private directory that holds quarantined records.
    #[must_use]
    pub const fn quarantine(&self) -> &UnixRecoveryDirectory {
        &self.quarantine
    }
}

/// One held directory and the path it had when it was bound.
///
/// The path is for messages and for comparing record paths; operations use
/// the descriptor.
#[derive(Debug)]
pub struct UnixRecoveryDirectory {
    directory: File,
    path: PathBuf,
}

impl UnixRecoveryDirectory {
    /// The path the directory had when it was bound.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Exclusively creates a private file named `name` in this directory.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] for an invalid name,
    /// [`io::ErrorKind::AlreadyExists`] when the name is taken,
    /// [`io::ErrorKind::NotFound`] when the directory has been removed, and
    /// other creation or permission failures.
    pub fn create_private_new(&self, name: &OsStr) -> io::Result<File> {
        validate_entry_name(name)?;
        // The directory is private and, on macOS, free of ACLs a new file
        // could inherit, so creation relative to it needs no path-based
        // ACL-aware primitive.
        crate::imp::unix_create_private_new_at(&self.directory, name)
    }

    /// Opens the existing entry `name` for reading without following a link.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] for an invalid name and the
    /// operating-system error when the entry is missing, is a link, or cannot
    /// be opened.
    pub fn open_existing(&self, name: &OsStr) -> io::Result<File> {
        validate_entry_name(name)?;
        crate::imp::unix_open_existing_at(&self.directory, name)
    }

    /// Lists at most `limit` entry names in this directory, excluding `.`
    /// and `..`, in the order the directory returns them.
    ///
    /// # Errors
    ///
    /// Returns the operating-system error when the directory cannot be read.
    pub fn entry_names(&self, limit: usize) -> io::Result<Vec<OsString>> {
        let mut names = Vec::new();
        for entry in Dir::read_from(&self.directory)? {
            if names.len() == limit {
                break;
            }
            let entry = entry?;
            let name = entry.file_name().to_bytes();
            if name != b"." && name != b".." {
                names.push(OsStr::from_bytes(name).to_os_string());
            }
        }
        Ok(names)
    }

    /// Reports whether the entry `name` is a regular file, without following
    /// a link.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::NotFound`] when the entry is gone and other
    /// operating-system failures.
    pub fn is_regular_file(&self, name: &OsStr) -> io::Result<bool> {
        validate_entry_name(name)?;
        let status = statat(&self.directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
        Ok(FileType::from_raw_mode(status.st_mode) == FileType::RegularFile)
    }

    /// Removes the entry `name` only if it still identifies `expected`.
    ///
    /// The directory is private to this user, so only this user's processes
    /// can change the entry between the check and the removal.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidData`] when the entry now names a
    /// different object, [`io::ErrorKind::NotFound`] when it is gone, and other
    /// operating-system failures.
    pub fn remove_if_identifies(&self, name: &OsStr, expected: &File) -> io::Result<()> {
        validate_entry_name(name)?;
        let expected = fstat(expected)?;
        let named = statat(&self.directory, name, AtFlags::SYMLINK_NOFOLLOW)?;
        if (named.st_dev, named.st_ino) != (expected.st_dev, expected.st_ino) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the recovery entry no longer identifies the opened file",
            ));
        }
        unlinkat(&self.directory, name, AtFlags::empty()).map_err(io::Error::from)
    }

    /// Removes the entry `name` from this directory.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::NotFound`] when the entry is gone and other
    /// operating-system failures.
    pub fn remove(&self, name: &OsStr) -> io::Result<()> {
        validate_entry_name(name)?;
        unlinkat(&self.directory, name, AtFlags::empty()).map_err(io::Error::from)
    }

    /// Binds a commit of a staged record to `destination_name` in this
    /// directory, through a duplicate of the held descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] for an invalid name,
    /// [`io::ErrorKind::NotFound`] when the directory has been removed, and
    /// the operating-system error when the descriptor cannot be duplicated.
    pub fn commit_parent(&self, destination_name: &OsStr) -> io::Result<UnixRecoveryCommitParent> {
        validate_entry_name(destination_name)?;
        Ok(UnixRecoveryCommitParent::from_bound_directory(
            self.directory.try_clone()?,
            self.path.clone(),
            destination_name.to_os_string(),
        ))
    }

    /// Makes entry creations, renames, and removals in this directory durable.
    ///
    /// # Errors
    ///
    /// Returns the operating-system error from `fsync`.
    pub fn sync(&self) -> io::Result<()> {
        fsync(&self.directory).map_err(io::Error::from)
    }

    fn bind_private_child(&self, name: &OsStr, user: User, state: &Stat) -> io::Result<Self> {
        let directory = open_or_create_directory(&self.directory, name, true)?;
        let path = self.path.join(name);
        let status = fstat(&directory)?;
        if status.st_uid != user.owner {
            return Err(permission_denied(format!(
                "{} is owned by another user",
                path.display()
            )));
        }
        if status.st_dev != state.st_dev {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!(
                    "{} is on a different device from the state directory",
                    path.display()
                ),
            ));
        }
        // Mode bits do not cover an extended ACL, which on macOS could still
        // grant another user access to a 0700 directory.
        #[cfg(target_os = "macos")]
        crate::imp::macos_restrict_open_file_acl_to_owner(&directory)?;
        fchmod(&directory, Mode::RWXU)?;
        if permission_bits(&fstat(&directory)?) & GROUP_OR_OTHER_ANY != 0 {
            return Err(permission_denied(format!(
                "{} could not be made private",
                path.display()
            )));
        }
        Ok(Self { directory, path })
    }
}

fn bind_state_directory(state_root: &Path, user: User) -> io::Result<UnixRecoveryDirectory> {
    let (existing, missing) = split_existing_prefix(state_root)?;
    let canonical = std::fs::canonicalize(&existing)?;
    let mut names: Vec<OsString> = Vec::new();
    for component in canonical.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(name) => names.push(name.to_os_string()),
            _ => return Err(invalid_input("the state directory path is not canonical")),
        }
    }
    let existing_count = names.len();
    names.extend(missing);
    let (state_name, ancestor_names) = names
        .split_last()
        .ok_or_else(|| invalid_input("the state directory cannot be the root directory"))?;

    let mut directory = open_directory_no_follow(CWD, OsStr::new("/"))?;
    let mut walked = PathBuf::from("/");
    verify_ancestor(&fstat(&directory)?, user, &walked)?;
    for (index, name) in ancestor_names.iter().enumerate() {
        directory = open_or_create_directory(&directory, name, index >= existing_count)?;
        walked.push(name);
        verify_ancestor(&fstat(&directory)?, user, &walked)?;
    }
    let directory = open_or_create_directory(&directory, state_name, names.len() > existing_count)?;
    walked.push(state_name);
    let status = fstat(&directory)?;
    if status.st_uid != user.owner {
        return Err(permission_denied(format!(
            "{} is not owned by you",
            walked.display()
        )));
    }
    // The state directory is Noter's own, so it is made private rather than
    // refused when others could write it.
    fchmod(&directory, Mode::RWXU)?;
    if permission_bits(&fstat(&directory)?) & GROUP_OR_OTHER_WRITE != 0 {
        return Err(permission_denied(format!(
            "{} could not be made private",
            walked.display()
        )));
    }
    Ok(UnixRecoveryDirectory {
        directory,
        path: state_root.to_path_buf(),
    })
}

/// Opens `name` below `parent` without following a link, first creating it
/// private when `create` is set.
fn open_or_create_directory(parent: &File, name: &OsStr, create: bool) -> io::Result<File> {
    validate_entry_name(name)?;
    if create {
        match mkdirat(parent, name, Mode::RWXU) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(error) => return Err(error.into()),
        }
    }
    open_directory_no_follow(parent, name)
}

/// Splits `path` into its longest existing prefix and the names below it.
fn split_existing_prefix(path: &Path) -> io::Result<(PathBuf, Vec<OsString>)> {
    let mut missing = Vec::new();
    let mut existing = path.to_path_buf();
    loop {
        match std::fs::symlink_metadata(&existing) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let name = existing
                    .file_name()
                    .ok_or_else(|| invalid_input("the state directory path has no existing root"))?
                    .to_os_string();
                missing.push(name);
                if !existing.pop() {
                    return Err(invalid_input(
                        "the state directory path has no existing root",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
    missing.reverse();
    Ok((existing, missing))
}

/// The effective user and group this process acts as.
#[derive(Clone, Copy)]
struct User {
    owner: u32,
    group: u32,
}

impl User {
    fn current() -> Self {
        Self {
            owner: geteuid().as_raw(),
            group: getegid().as_raw(),
        }
    }
}

fn verify_ancestor(status: &Stat, user: User, path: &Path) -> io::Result<()> {
    if !ancestor_is_trusted(status.st_uid, status.st_gid, permission_bits(status), user) {
        return Err(permission_denied(format!(
            "{} can be changed by another user",
            path.display()
        )));
    }
    Ok(())
}

/// Accepts a directory on the path when no other user can replace its
/// entries. It must be owned by the superuser or this user. Others may write
/// it only with the sticky bit, which stops them renaming or removing entries
/// they do not own. A group may write it only when that group is this user's
/// private group: the process's group, numbered like the user, as systems
/// that give each user their own group create it.
const fn ancestor_is_trusted(
    directory_owner: u32,
    directory_group: u32,
    mode: u32,
    user: User,
) -> bool {
    let trusted_owner = directory_owner == 0 || directory_owner == user.owner;
    let private_group = directory_group == user.group && user.group == user.owner;
    let group_safe = mode & GROUP_WRITE == 0 || private_group;
    let others_safe = mode & OTHER_WRITE == 0;
    let shielded = mode & STICKY != 0 || (group_safe && others_safe);
    trusted_owner && shielded
}

/// The permission bits of `status`; the mode field is narrower on macOS.
#[cfg(target_os = "macos")]
fn macos_permission_bits(status: &Stat) -> u32 {
    u32::from(status.st_mode)
}

#[cfg(target_os = "macos")]
use macos_permission_bits as permission_bits;

#[cfg(not(target_os = "macos"))]
const fn permission_bits(status: &Stat) -> u32 {
    status.st_mode
}

fn open_directory_no_follow(parent: impl AsFd, name: &OsStr) -> io::Result<File> {
    openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(io::Error::from)
}

fn validate_entry_name(name: &OsStr) -> io::Result<()> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes == b"."
        || bytes == b".."
        || bytes.contains(&b'/')
        || bytes.contains(&0)
    {
        return Err(invalid_input(
            "a recovery entry name must be one ordinary path component",
        ));
    }
    Ok(())
}

fn require_local_file_system(directory: &File, path: &Path) -> io::Result<()> {
    if file_system_is_local(directory)? {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("{} is on a network or shared file system", path.display()),
        ))
    }
}

#[cfg(target_os = "linux")]
fn linux_file_system_is_local(directory: &File) -> io::Result<bool> {
    // The magic numbers are 32-bit; a value that does not fit is unknown and
    // therefore not trusted.
    let kind = u32::try_from(rustix::fs::fstatfs(directory)?.f_type);
    Ok(kind.is_ok_and(|kind| !linux_file_system_is_shared(kind)))
}

#[cfg(target_os = "linux")]
use linux_file_system_is_local as file_system_is_local;

/// Network, cluster, shared-folder, and user-space file systems whose
/// locking, rename, and durability cannot be verified from here, as named in
/// Linux's `magic.h` and coreutils' file-system table.
#[cfg(target_os = "linux")]
const fn linux_file_system_is_shared(kind: u32) -> bool {
    matches!(
        kind,
        0x6969 // NFS
            | 0x517B // SMB
            | 0xFF53_4D42 // CIFS
            | 0xFE53_4D42 // SMB2
            | 0x7375_7245 // Coda
            | 0x5346_414F // AFS
            | 0x6B41_4653 // kAFS
            | 0x564C // NCP
            | 0x00C3_6400 // Ceph
            | 0x0102_1997 // 9P
            | 0x6573_5546 // FUSE
            | 0x0116_1970 // GFS2
            | 0x7461_636F // OCFS2
            | 0x0BD0_0BD0 // Lustre
            | 0x2003_0528 // OrangeFS
            | 0x1983_0326 // BeeGFS
            | 0x4750_4653 // GPFS
            | 0xAAD7_AAEA // PanFS
            | 0x0131_11A8 // IBRIX
            | 0x6163_6673 // ACFS
            | 0xBEEF_DEAD // SNFS
            | 0x786F_4256 // VirtualBox shared folders
            | 0xBACB_ACBC // VMware shared folders
            | 0x7C7C_6673 // Parallels shared folders
    )
}

#[cfg(target_os = "macos")]
fn macos_file_system_is_local(directory: &File) -> io::Result<bool> {
    Ok(macos_mount_flags_are_local(
        rustix::fs::fstatfs(directory)?.f_flags,
    ))
}

/// Whether a mount's flags include `MNT_LOCAL` from `<sys/mount.h>`.
#[cfg(target_os = "macos")]
const fn macos_mount_flags_are_local(flags: u32) -> bool {
    const MNT_LOCAL: u32 = 0x0000_1000;
    flags & MNT_LOCAL == MNT_LOCAL
}

#[cfg(target_os = "macos")]
use macos_file_system_is_local as file_system_is_local;

/// Other Unix systems have no verified classification, so recovery is
/// refused there.
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
const fn unsupported_file_system_is_local(_directory: &File) -> io::Result<bool> {
    Ok(false)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
use unsupported_file_system_is_local as file_system_is_local;

fn invalid_input(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn permission_denied(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    fn open(state_root: &Path) -> io::Result<UnixRecoveryNamespace> {
        UnixRecoveryNamespace::open_or_create(state_root, OsStr::new("recovery"))
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().mode() & 0o7777
    }

    #[test]
    fn creates_a_private_tree_under_a_new_state_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("a/b/state");
        let namespace = open(&state).unwrap();

        assert_eq!(namespace.state().path(), state);
        assert_eq!(namespace.records().path(), state.join("recovery/records"));
        assert_eq!(
            namespace.quarantine().path(),
            state.join("recovery/quarantine")
        );
        for directory in ["recovery", "recovery/records", "recovery/quarantine"] {
            assert_eq!(mode(&state.join(directory)) & 0o077, 0, "{directory}");
        }
        assert_eq!(mode(&state) & 0o022, 0);
        let again = open(&state).unwrap();
        assert_eq!(again.recovery().path(), namespace.recovery().path());
    }

    #[test]
    fn entries_are_created_listed_opened_and_removed_through_the_held_directory() {
        let temporary = tempfile::tempdir().unwrap();
        let namespace = open(&temporary.path().join("state")).unwrap();
        let records = namespace.records();

        let mut created = records.create_private_new(OsStr::new("one.rec")).unwrap();
        created.write_all(b"content").unwrap();
        assert_eq!(created.metadata().unwrap().mode() & 0o077, 0);
        assert_eq!(
            records
                .create_private_new(OsStr::new("one.rec"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::AlreadyExists
        );
        assert_eq!(records.entry_names(8).unwrap(), [OsString::from("one.rec")]);
        assert!(records.is_regular_file(OsStr::new("one.rec")).unwrap());
        symlink("one.rec", records.path().join("link")).unwrap();
        assert!(!records.is_regular_file(OsStr::new("link")).unwrap());
        records.remove(OsStr::new("link")).unwrap();
        assert_eq!(
            records
                .is_regular_file(OsStr::new("absent"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        records.create_private_new(OsStr::new("two.rec")).unwrap();
        assert_eq!(records.entry_names(1).unwrap().len(), 1);
        assert_eq!(records.entry_names(0).unwrap().len(), 0);
        records.remove(OsStr::new("two.rec")).unwrap();

        let mut text = String::new();
        records
            .open_existing(OsStr::new("one.rec"))
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "content");

        records
            .remove_if_identifies(OsStr::new("one.rec"), &created)
            .unwrap();
        assert!(records.entry_names(8).unwrap().is_empty());
        assert_eq!(
            records.remove(OsStr::new("one.rec")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        records.sync().unwrap();
        namespace.quarantine().sync().unwrap();
    }

    #[test]
    fn removal_refuses_an_entry_replaced_after_it_was_opened() {
        let temporary = tempfile::tempdir().unwrap();
        let namespace = open(&temporary.path().join("state")).unwrap();
        let records = namespace.records();
        let original = records.create_private_new(OsStr::new("entry")).unwrap();
        let replacement = records.create_private_new(OsStr::new("other")).unwrap();
        std::fs::rename(records.path().join("other"), records.path().join("entry")).unwrap();

        let error = records
            .remove_if_identifies(OsStr::new("entry"), &original)
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        records
            .remove_if_identifies(OsStr::new("entry"), &replacement)
            .unwrap();
    }

    #[test]
    fn operations_follow_the_bound_directory_after_an_ancestor_is_renamed() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        let namespace = open(&state).unwrap();
        let moved = temporary.path().join("moved");
        std::fs::rename(&state, &moved).unwrap();
        std::fs::create_dir_all(state.join("recovery/records")).unwrap();

        namespace
            .records()
            .create_private_new(OsStr::new("bound"))
            .unwrap();
        assert!(moved.join("recovery/records/bound").exists());
        assert!(!state.join("recovery/records/bound").exists());
    }

    #[test]
    fn a_removed_directory_refuses_new_content() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        let namespace = open(&state).unwrap();
        std::fs::remove_dir_all(state.join("recovery")).unwrap();

        let records = namespace.records();
        assert_eq!(
            records
                .create_private_new(OsStr::new("late"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        // The operating system refuses entries in a removed directory, so a
        // commit through its held descriptor cannot stage a record either.
        let commit = records.commit_parent(OsStr::new("late")).unwrap();
        assert_eq!(
            commit
                .create_private_new(&records.path().join("late.stage"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert!(records.entry_names(8).unwrap().is_empty());
    }

    #[test]
    fn loose_recovery_directories_are_made_private() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        std::fs::create_dir_all(state.join("recovery/records")).unwrap();
        std::fs::set_permissions(state.join("recovery"), PermissionsExt::from_mode(0o755)).unwrap();
        std::fs::set_permissions(
            state.join("recovery/records"),
            PermissionsExt::from_mode(0o777),
        )
        .unwrap();

        open(&state).unwrap();
        assert_eq!(mode(&state.join("recovery")), 0o700);
        assert_eq!(mode(&state.join("recovery/records")), 0o700);
    }

    #[test]
    fn a_state_directory_others_can_write_is_made_private() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        std::fs::create_dir(&state).unwrap();
        std::fs::set_permissions(&state, PermissionsExt::from_mode(0o777)).unwrap();

        open(&state).unwrap();
        assert_eq!(mode(&state), 0o700);
    }

    #[test]
    fn an_ancestor_others_can_write_is_rejected_unless_it_is_sticky() {
        let temporary = tempfile::tempdir().unwrap();
        let shared = temporary.path().join("shared");
        std::fs::create_dir(&shared).unwrap();
        std::fs::set_permissions(&shared, PermissionsExt::from_mode(0o777)).unwrap();
        let error = open(&shared.join("state")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let canonical = std::fs::canonicalize(&shared).unwrap();
        assert_eq!(
            error.to_string(),
            format!("{} can be changed by another user", canonical.display())
        );
        assert!(!shared.join("state").exists());

        std::fs::set_permissions(&shared, PermissionsExt::from_mode(0o1777)).unwrap();
        open(&shared.join("state")).unwrap();
    }

    #[test]
    fn ancestor_trust_requires_an_owner_and_protection_from_other_users() {
        let private = User {
            owner: 1000,
            group: 1000,
        };
        let shared = User {
            owner: 1000,
            group: 100,
        };
        assert!(ancestor_is_trusted(0, 0, 0o755, private));
        assert!(ancestor_is_trusted(1000, 1000, 0o700, private));
        assert!(ancestor_is_trusted(0, 0, 0o1777, private));
        assert!(!ancestor_is_trusted(1001, 1000, 0o755, private));
        assert!(!ancestor_is_trusted(1001, 1001, 0o1777, private));
        assert!(!ancestor_is_trusted(1000, 1000, 0o757, private));
        // A group-writable directory is safe only in the user's private group.
        assert!(ancestor_is_trusted(1000, 1000, 0o775, private));
        assert!(!ancestor_is_trusted(1000, 1001, 0o775, private));
        assert!(!ancestor_is_trusted(1000, 100, 0o775, shared));
        assert!(ancestor_is_trusted(1000, 100, 0o755, shared));
        assert!(ancestor_is_trusted(1000, 100, 0o1775, shared));
    }

    #[test]
    fn links_in_the_recovery_tree_are_refused() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        let elsewhere = temporary.path().join("elsewhere");
        std::fs::create_dir_all(&state).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        symlink(&elsewhere, state.join("recovery")).unwrap();

        assert!(open(&state).is_err());
        assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
    }

    #[test]
    fn a_link_in_the_existing_state_path_resolves_once_to_its_target() {
        let temporary = tempfile::tempdir().unwrap();
        let target = temporary.path().join("target");
        std::fs::create_dir(&target).unwrap();
        symlink(&target, temporary.path().join("link")).unwrap();

        let requested = temporary.path().join("link/state");
        let namespace = open(&requested).unwrap();
        assert_eq!(namespace.state().path(), requested);
        assert!(target.join("state/recovery/records").is_dir());
    }

    #[test]
    fn a_file_where_a_recovery_directory_belongs_is_refused() {
        let temporary = tempfile::tempdir().unwrap();
        let state = temporary.path().join("state");
        std::fs::create_dir_all(state.join("recovery")).unwrap();
        std::fs::write(state.join("recovery/records"), b"").unwrap();
        assert!(open(&state).is_err());
    }

    #[test]
    fn relative_roots_and_compound_names_are_refused() {
        assert_eq!(
            open(Path::new("relative/state")).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let temporary = tempfile::tempdir().unwrap();
        for name in ["", ".", "..", "a/b", "nul\0"] {
            assert_eq!(
                UnixRecoveryNamespace::open_or_create(temporary.path(), OsStr::new(name))
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::InvalidInput,
                "{name:?}"
            );
        }
        let namespace = open(&temporary.path().join("state")).unwrap();
        for name in ["", "..", "a/b"] {
            let name = OsStr::new(name);
            let records = namespace.records();
            assert_eq!(
                records.create_private_new(name).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(
                records.open_existing(name).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert_eq!(
                records.remove(name).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            assert!(records.commit_parent(name).is_err());
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn only_mounts_marked_local_are_local() {
        assert!(macos_mount_flags_are_local(0x0000_1000));
        assert!(macos_mount_flags_are_local(0x0000_1001));
        assert!(!macos_mount_flags_are_local(0));
        assert!(!macos_mount_flags_are_local(0x0000_0800));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn network_and_user_space_file_systems_are_classified_as_shared() {
        for kind in [0x6969, 0xFF53_4D42, 0x0102_1997, 0x6573_5546] {
            assert!(linux_file_system_is_shared(kind), "{kind:#x}");
        }
        for kind in [0xEF53, 0x5846_5342, 0x9123_683E, 0x0102_1994, 0x794C_7630] {
            assert!(!linux_file_system_is_shared(kind), "{kind:#x}");
        }
    }
}
