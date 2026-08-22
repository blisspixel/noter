//! Durable private storage for restart-spanning recovery records.
//!
//! Adapters own the recovery root directory. This module never writes a user
//! document path. Records are staged with private exclusive creation, synced,
//! then installed or replaced atomically. Corrupt records are moved into a
//! quarantine directory instead of being deleted silently. Quarantine failures
//! are reported on the scan entry rather than swallowed.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use getrandom::fill as fill_random;
use noter_platform::{InstallNewOutcome, ReplaceExistingOutcome};

use super::recovery::{
    RecoveryInstanceId, RecoveryQuarantineReason, RecoverySnapshot, RecoveryStartupDisposition,
    validate_recovery_record,
};

/// Subdirectory of the recovery root that holds active records.
pub const RECOVERY_RECORDS_DIR: &str = "records";

/// Subdirectory that holds quarantined corrupt or unsupported records.
pub const RECOVERY_QUARANTINE_DIR: &str = "quarantine";

/// Maximum number of restore offers presented from one startup scan.
///
/// The directory is still fully walked so corrupt files beyond this limit are
/// quarantined instead of left indefinitely in the active records folder.
pub const MAX_STARTUP_RECOVERY_OFFERS: usize = 32;

/// Maximum individual recovery file size accepted during a startup scan.
pub const MAX_RECOVERY_FILE_BYTES: u64 = 64 * 1024 * 1024 + 256 * 1024;

/// One startup scan result paired with its on-disk path.
#[derive(Clone, Debug)]
pub struct RecoveryScanEntry {
    path: PathBuf,
    disposition: RecoveryStartupDisposition,
    /// Present when quarantine was required but the move failed.
    quarantine_error: Option<String>,
}

impl RecoveryScanEntry {
    /// Returns the best-known path of the artifact after the scan.
    ///
    /// For successfully quarantined records this is the quarantine path. When
    /// quarantine fails it remains the original records path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the pure validation disposition for this path.
    pub const fn disposition(&self) -> &RecoveryStartupDisposition {
        &self.disposition
    }

    /// Returns a short quarantine failure message when relocation failed.
    pub fn quarantine_error(&self) -> Option<&str> {
        self.quarantine_error.as_deref()
    }

    /// Returns whether the artifact still lives under the active records dir.
    pub fn remains_in_records(&self) -> bool {
        self.path
            .parent()
            .and_then(|parent| parent.file_name())
            .is_some_and(|name| name == RECOVERY_RECORDS_DIR)
    }

    /// Consumes the entry into path and disposition.
    pub fn into_parts(self) -> (PathBuf, RecoveryStartupDisposition) {
        (self.path, self.disposition)
    }
}

/// Private recovery directory layout under a caller-supplied root.
#[derive(Clone, Debug)]
pub struct RecoveryStore {
    root: PathBuf,
}

impl RecoveryStore {
    /// Opens or creates the recovery layout under `root`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the root or required subdirectories cannot be
    /// created.
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        fs::create_dir_all(root.join(RECOVERY_RECORDS_DIR))?;
        fs::create_dir_all(root.join(RECOVERY_QUARANTINE_DIR))?;
        Ok(Self { root })
    }

    /// Returns the recovery root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Returns the active records directory.
    pub fn records_dir(&self) -> PathBuf {
        self.root.join(RECOVERY_RECORDS_DIR)
    }

    /// Returns the quarantine directory.
    pub fn quarantine_dir(&self) -> PathBuf {
        self.root.join(RECOVERY_QUARANTINE_DIR)
    }

    /// Returns the stable active path for one editor instance.
    pub fn record_path(&self, instance_id: RecoveryInstanceId) -> PathBuf {
        self.records_dir()
            .join(format!("{}.rec", hex16(&instance_id.as_bytes())))
    }

    /// Returns the live-lease sibling that marks an instance as still running.
    pub fn live_path(&self, instance_id: RecoveryInstanceId) -> PathBuf {
        self.records_dir()
            .join(format!("{}.live", hex16(&instance_id.as_bytes())))
    }

    /// Holds an exclusive lock that another Noter window can probe without
    /// deleting this instance's recovery record.
    ///
    /// The returned file must stay open for the session. Dropping it releases
    /// the lock so a later launch can offer restore after a crash.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the live file cannot be created or locked.
    pub fn try_hold_live_lease(&self, instance_id: RecoveryInstanceId) -> io::Result<File> {
        let path = self.live_path(instance_id);
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)?;
        file.try_lock().map_err(|error| match error {
            TryLockError::WouldBlock => io::Error::new(
                io::ErrorKind::ResourceBusy,
                "recovery live lease is held by another Noter window",
            ),
            TryLockError::Error(error) => error,
        })?;
        Ok(file)
    }

    /// Returns whether another living Noter window still holds this instance.
    pub fn instance_is_live(&self, instance_id: RecoveryInstanceId) -> bool {
        let path = self.live_path(instance_id);
        let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
            return false;
        };
        matches!(file.try_lock(), Err(TryLockError::WouldBlock))
    }

    /// Persists one recovery snapshot under its instance identity.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when private staging, write, sync, install, or
    /// replace fails.
    pub fn persist(&self, snapshot: &RecoverySnapshot) -> io::Result<()> {
        let destination = self.record_path(snapshot.instance_id());
        let encoded = snapshot.encode();
        write_atomic_private(&destination, &encoded)
    }

    /// Removes the active record for an owned instance after save or discard.
    ///
    /// Missing files are treated as success so cleanup is idempotent.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when an existing record cannot be removed.
    pub fn delete_instance(&self, instance_id: RecoveryInstanceId) -> io::Result<()> {
        let path = self.record_path(instance_id);
        let record_result = match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
        let _ = fs::remove_file(self.live_path(instance_id));
        record_result
    }

    /// Scans active recovery records and validates each complete file.
    ///
    /// The records directory is fully walked. Corrupt files are quarantined.
    /// At most [`MAX_STARTUP_RECOVERY_OFFERS`] valid restore offers are returned;
    /// additional valid records remain in place for a later session.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the records directory cannot be listed.
    pub fn scan_startup(&self) -> io::Result<Vec<RecoveryScanEntry>> {
        let mut entries = Vec::new();
        let records = self.records_dir();
        let dir = match fs::read_dir(&records) {
            Ok(dir) => dir,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(entries),
            Err(error) => return Err(error),
        };

        let mut offer_count = 0_usize;
        for next in dir {
            let entry = next?;
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if path
                .extension()
                .is_some_and(|extension| extension == "live")
            {
                continue;
            }
            let disposition = match classify_recovery_file(&path) {
                Ok(disposition) => disposition,
                Err(reason) => RecoveryStartupDisposition::Quarantine(reason),
            };

            match &disposition {
                RecoveryStartupDisposition::Offer(record)
                    if self.instance_is_live(record.instance_id()) =>
                {
                    // Another living window owns this record. Leave it in place.
                }
                RecoveryStartupDisposition::Offer(_) => {
                    if offer_count >= MAX_STARTUP_RECOVERY_OFFERS {
                        // Leave surplus valid records for a later launch.
                        continue;
                    }
                    offer_count = offer_count.saturating_add(1);
                    entries.push(RecoveryScanEntry {
                        path,
                        disposition,
                        quarantine_error: None,
                    });
                }
                RecoveryStartupDisposition::Quarantine(_) => {
                    entries.push(self.quarantine_scan_entry(path, disposition));
                }
            }
        }
        Ok(entries)
    }

    /// Moves a recovery file into the quarantine directory.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the file cannot be relocated. A missing source
    /// is reported as [`io::ErrorKind::NotFound`] rather than success.
    pub fn quarantine_file(&self, path: &Path) -> io::Result<PathBuf> {
        let file_name = path.file_name().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "recovery quarantine requires a file name",
            )
        })?;
        if !path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "recovery quarantine source is missing",
            ));
        }
        let mut random = [0_u8; 8];
        fill_random(&mut random).map_err(|error| {
            io::Error::other(format!("recovery quarantine random name failed: {error}"))
        })?;
        let dest = self.quarantine_dir().join(format!(
            "{}-{}.rec",
            file_name.to_string_lossy(),
            hex8(random)
        ));
        match fs::rename(path, &dest) {
            Ok(()) => Ok(dest),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Err(error),
            Err(_) => {
                // Cross-volume rename is uncommon for private state; fall back
                // to copy-then-remove and fail closed if the original remains.
                fs::copy(path, &dest)?;
                match fs::remove_file(path) {
                    Ok(()) => Ok(dest),
                    Err(remove_error) => {
                        let _ = fs::remove_file(&dest);
                        Err(remove_error)
                    }
                }
            }
        }
    }

    fn quarantine_scan_entry(
        &self,
        path: PathBuf,
        disposition: RecoveryStartupDisposition,
    ) -> RecoveryScanEntry {
        match self.quarantine_file(&path) {
            Ok(quarantined) => RecoveryScanEntry {
                path: quarantined,
                disposition,
                quarantine_error: None,
            },
            Err(error) => RecoveryScanEntry {
                path,
                disposition,
                quarantine_error: Some(format!(
                    "Noter could not quarantine a damaged recovery file ({error}). The file remains in the recovery records folder."
                )),
            },
        }
    }
}

fn classify_recovery_file(
    path: &Path,
) -> Result<RecoveryStartupDisposition, RecoveryQuarantineReason> {
    let metadata = fs::metadata(path).map_err(|_| RecoveryQuarantineReason::Truncated)?;
    if !metadata.is_file() {
        return Err(RecoveryQuarantineReason::Truncated);
    }
    if metadata.len() > MAX_RECOVERY_FILE_BYTES {
        return Err(RecoveryQuarantineReason::ContentTooLarge);
    }
    let mut file = File::open(path).map_err(|_| RecoveryQuarantineReason::Truncated)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| RecoveryQuarantineReason::Truncated)?;
    Ok(validate_recovery_record(&bytes))
}

fn write_atomic_private(destination: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let stage = exclusive_stage_path(parent)?;
    let backup = exclusive_stage_path(parent)?;
    let write_result = commit_staged_record(&stage, destination, &backup, bytes);

    if write_result.is_err() {
        let _ = fs::remove_file(&stage);
        // A failed replace may leave a backup sibling; remove only empty or
        // newly created backup names that are not the committed destination.
        let _ = fs::remove_file(&backup);
    } else {
        // Successful replace on Windows keeps the previous destination in the
        // backup path. That is superseded recovery content and must not linger.
        let _ = fs::remove_file(&backup);
        let _ = noter_platform::sync_parent(destination);
    }
    write_result
}

fn commit_staged_record(
    stage: &Path,
    destination: &Path,
    backup: &Path,
    bytes: &[u8],
) -> io::Result<()> {
    let mut file = noter_platform::create_private_new_file(stage)?;
    file.write_all(bytes)?;
    file.flush()?;
    noter_platform::sync_file(&file)?;
    drop(file);

    if destination.exists() {
        finish_replace(stage, destination, backup)
    } else {
        match noter_platform::install_new(stage, destination) {
            Ok(InstallNewOutcome::Clean) => Ok(()),
            Ok(InstallNewOutcome::CommittedWithRetainedTemporary) => {
                // Destination is committed; remove the retained stage name.
                fs::remove_file(stage)?;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // A concurrent install won the destination. Replace that file
                // with this staged snapshot instead of reporting success without
                // committing these bytes.
                finish_replace(stage, destination, backup)
            }
            Err(error) => Err(error),
        }
    }
}

fn finish_replace(stage: &Path, destination: &Path, backup: &Path) -> io::Result<()> {
    match noter_platform::replace_existing(stage, destination, Some(backup))? {
        ReplaceExistingOutcome::Clean => Ok(()),
        ReplaceExistingOutcome::DisplacedDestination => {
            // Unix exchange leaves the previous destination at the stage path.
            fs::remove_file(stage)?;
            Ok(())
        }
    }
}

fn exclusive_stage_path(parent: &Path) -> io::Result<PathBuf> {
    for _ in 0..32 {
        let mut random = [0_u8; 16];
        fill_random(&mut random).map_err(|error| {
            io::Error::other(format!("recovery stage random name failed: {error}"))
        })?;
        let path = parent.join(format!(".noter-recovery-{}.tmp", hex16(&random)));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a private recovery stage name",
    ))
}

fn hex16(bytes: &[u8; 16]) -> String {
    let mut out = String::with_capacity(32);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn hex8(bytes: [u8; 8]) -> String {
    let mut out = String::with_capacity(16);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::edit::Selection;
    use crate::core::recovery::{
        RECOVERY_SCHEMA_VERSION, RecoveryDocumentId, RecoverySnapshotParts, RecoveryWallTime,
    };
    use crate::core::revision::Revision;
    use crate::core::text_format::{Bom, Encoding};
    use tempfile::tempdir;

    fn sample_snapshot(instance: u8, content: &[u8]) -> RecoverySnapshot {
        RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([9; 16]),
            instance_id: RecoveryInstanceId::new([instance; 16]),
            revision: Revision::new(u64::from(instance)),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(2),
            original_path: b"memo.txt".to_vec(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(content.len().min(1)),
            content: content.to_vec(),
        })
        .expect("snapshot")
    }

    #[test]
    fn persist_scan_and_delete_round_trip() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(1, b"recovered text");
        store.persist(&snapshot)?;

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        match entries[0].disposition() {
            RecoveryStartupDisposition::Offer(record) => {
                assert_eq!(record.content(), b"recovered text");
                assert_eq!(record.instance_id(), snapshot.instance_id());
                assert_eq!(record.original_path(), b"memo.txt");
            }
            RecoveryStartupDisposition::Quarantine(reason) => {
                panic!("expected offer, quarantined: {reason:?}")
            }
        }
        assert!(entries[0].remains_in_records());
        assert!(entries[0].quarantine_error().is_none());

        // No stage or backup siblings should remain after a clean persist.
        let leftovers: Vec<_> = fs::read_dir(store.records_dir())?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name())
            .filter(|name| {
                name.to_string_lossy().starts_with(".noter-recovery-")
                    || name.to_string_lossy().ends_with(".tmp")
            })
            .collect();
        assert!(
            leftovers.is_empty(),
            "unexpected recovery stage leftovers: {leftovers:?}"
        );

        store.persist(&sample_snapshot(1, b"updated"))?;
        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        match entries[0].disposition() {
            RecoveryStartupDisposition::Offer(record) => {
                assert_eq!(record.content(), b"updated");
            }
            RecoveryStartupDisposition::Quarantine(reason) => {
                panic!("expected updated offer, quarantined: {reason:?}")
            }
        }

        store.delete_instance(snapshot.instance_id())?;
        assert!(store.scan_startup()?.is_empty());
        store.delete_instance(snapshot.instance_id())?;
        Ok(())
    }

    #[test]
    fn a_held_live_lease_hides_the_instance_from_startup_scan() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(9, b"still running");
        store.persist(&snapshot)?;
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;

        assert!(store.instance_is_live(snapshot.instance_id()));
        assert!(store.scan_startup()?.is_empty());

        drop(lease);
        assert!(!store.instance_is_live(snapshot.instance_id()));
        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        match entries[0].disposition() {
            RecoveryStartupDisposition::Offer(record) => {
                assert_eq!(record.content(), b"still running");
            }
            RecoveryStartupDisposition::Quarantine(reason) => {
                panic!("expected offer after the lease dropped, quarantined: {reason:?}")
            }
        }
        Ok(())
    }

    #[test]
    fn corrupt_records_are_quarantined_on_scan() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("broken.rec");
        fs::write(&path, b"not a recovery record")?;

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].disposition(),
            RecoveryStartupDisposition::Quarantine(
                RecoveryQuarantineReason::Truncated | RecoveryQuarantineReason::InvalidMagic
            )
        ));
        assert!(!path.exists());
        assert!(!entries[0].remains_in_records());
        assert!(entries[0].quarantine_error().is_none());
        assert!(fs::read_dir(store.quarantine_dir())?.next().is_some());
        Ok(())
    }

    #[test]
    fn missing_quarantine_source_is_not_success() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let missing = store.records_dir().join("gone.rec");
        let error = store
            .quarantine_file(&missing)
            .expect_err("missing source must fail");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        Ok(())
    }

    #[test]
    fn checksum_tamper_is_quarantined() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(2, b"safe");
        store.persist(&snapshot)?;
        let path = store.record_path(snapshot.instance_id());
        let mut bytes = fs::read(&path)?;
        let last = bytes.len() - 1;
        bytes[last] ^= 0x5A;
        fs::write(&path, bytes)?;

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].disposition(),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::ChecksumMismatch)
        ));
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn unknown_schema_is_quarantined() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(3, b"schema");
        store.persist(&snapshot)?;
        let path = store.record_path(snapshot.instance_id());
        let mut bytes = fs::read(&path)?;
        bytes[8..12].copy_from_slice(&(RECOVERY_SCHEMA_VERSION + 1).to_le_bytes());
        fs::write(&path, bytes)?;

        let entries = store.scan_startup()?;
        assert!(matches!(
            entries[0].disposition(),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::UnknownSchema)
        ));
        Ok(())
    }

    #[test]
    fn resource_ceilings_are_exact() {
        assert_eq!(MAX_STARTUP_RECOVERY_OFFERS, 32);
        assert_eq!(MAX_RECOVERY_FILE_BYTES, 64 * 1024 * 1024 + 256 * 1024);
        assert_eq!(MAX_RECOVERY_FILE_BYTES, 67_371_008);
    }

    #[test]
    fn delete_instance_rejects_non_missing_failures() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let id = RecoveryInstanceId::new([42; 16]);
        let path = store.record_path(id);
        fs::create_dir(&path)?;
        let error = store
            .delete_instance(id)
            .expect_err("directory at record path is not a successful missing delete");
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert!(path.exists());
        Ok(())
    }

    #[test]
    fn oversized_recovery_file_is_quarantined() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("huge.rec");
        // Metadata length check happens before a full read; use set_len so the
        // ceiling is exercised without writing 64 MiB of payload bytes.
        let file = File::create(&path)?;
        file.set_len(MAX_RECOVERY_FILE_BYTES + 1)?;
        drop(file);

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(matches!(
            entries[0].disposition(),
            RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::ContentTooLarge)
        ));
        Ok(())
    }

    #[test]
    fn exact_recovery_file_ceiling_is_not_size_rejected() -> io::Result<()> {
        // metadata.len() > MAX must stay strict greater-than: exact size proceeds
        // to content validation (truncated/magic) instead of ContentTooLarge.
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("exact-ceiling.rec");
        let file = File::create(&path)?;
        file.set_len(MAX_RECOVERY_FILE_BYTES)?;
        drop(file);

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(
            !matches!(
                entries[0].disposition(),
                RecoveryStartupDisposition::Quarantine(RecoveryQuarantineReason::ContentTooLarge)
            ),
            "exact file ceiling must not be size-rejected, got {:?}",
            entries[0].disposition()
        );
        Ok(())
    }

    #[test]
    fn missing_records_directory_scans_as_empty() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        fs::remove_dir_all(store.records_dir())?;
        let entries = store.scan_startup()?;
        assert!(
            entries.is_empty(),
            "a missing records directory is an empty startup scan, not an error"
        );
        Ok(())
    }

    #[test]
    fn quarantine_error_is_reported_when_move_fails() -> io::Result<()> {
        // A valid quarantine failure leaves remains_in_records true and a message.
        // Use a non-empty records file, then remove write access to quarantine by
        // replacing the quarantine directory with a file so rename fails.
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("broken.rec");
        fs::write(&path, b"not a recovery record")?;
        let quarantine = store.quarantine_dir();
        fs::remove_dir_all(&quarantine)?;
        fs::write(&quarantine, b"block")?;

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(entries[0].remains_in_records());
        assert!(entries[0].quarantine_error().is_some());
        assert!(path.exists());
        Ok(())
    }

    #[test]
    fn surplus_valid_records_remain_for_later_session() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        for index in 0..=MAX_STARTUP_RECOVERY_OFFERS {
            let instance = u8::try_from(index).unwrap_or(u8::MAX);
            // Distinct instance ids: use patterned bytes.
            let mut id = [0_u8; 16];
            id[0] = instance;
            id[1] = (index / 256) as u8;
            let snapshot = RecoverySnapshot::try_new(RecoverySnapshotParts {
                document_id: RecoveryDocumentId::new([1; 16]),
                instance_id: RecoveryInstanceId::new(id),
                revision: Revision::new(1),
                created_at: RecoveryWallTime::from_unix_millis(1),
                updated_at: RecoveryWallTime::from_unix_millis(2),
                original_path: Vec::new(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::caret(0),
                content: format!("doc-{index}").into_bytes(),
            })
            .expect("snapshot");
            store.persist(&snapshot)?;
        }

        let entries = store.scan_startup()?;
        let offers = entries
            .iter()
            .filter(|entry| matches!(entry.disposition(), RecoveryStartupDisposition::Offer(_)))
            .count();
        assert_eq!(offers, MAX_STARTUP_RECOVERY_OFFERS);

        let remaining = fs::read_dir(store.records_dir())?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .count();
        assert_eq!(remaining, MAX_STARTUP_RECOVERY_OFFERS + 1);
        Ok(())
    }
}
