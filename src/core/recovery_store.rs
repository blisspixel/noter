//! Durable private storage for restart-spanning recovery records.
//!
//! Adapters own the recovery root directory. This module never writes a user
//! document path. Records are staged with private exclusive creation, synced,
//! then installed or replaced atomically. Corrupt records are moved into a
//! quarantine directory instead of being deleted silently. Quarantine failures
//! are reported on the scan entry rather than swallowed.

use std::fs::{self, File, TryLockError};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use getrandom::fill as fill_random;
use noter_platform::{InstallNewOutcome, ReplaceExistingOutcome};

use super::recovery::{
    RECOVERY_MAGIC, RECOVERY_SCHEMA_VERSION, RecoveryInstanceId, RecoveryQuarantineReason,
    RecoverySnapshot, RecoveryStartupDisposition, ValidatedRecoveryMetadata,
    ValidatedRecoveryRecord, validate_recovery_metadata, validate_recovery_record,
};

/// Subdirectory of the recovery root that holds active records.
pub const RECOVERY_RECORDS_DIR: &str = "records";

/// Subdirectory that holds quarantined corrupt or unsupported records.
pub const RECOVERY_QUARANTINE_DIR: &str = "quarantine";

/// Maximum number of restore offers presented from one startup scan.
///
/// A bounded scan reports omissions explicitly so callers never interpret the
/// returned set as globally complete.
pub const MAX_STARTUP_RECOVERY_OFFERS: usize = 32;

/// Maximum eligible recovery candidates inspected by one startup scan.
pub const MAX_STARTUP_RECOVERY_FILES: usize = 256;

/// Maximum raw directory entries enumerated by one startup scan.
///
/// Non-files, live markers, and records proven to belong to a living instance
/// do not consume the smaller eligible-candidate budget above. Stale live
/// markers are removed under an exclusive claim so bounded later scans make
/// progress through crash residue instead of revisiting it forever.
pub const MAX_STARTUP_RECOVERY_DIRECTORY_ENTRIES: usize = 1024;

/// Maximum aggregate encoded bytes read by one startup scan.
pub const MAX_STARTUP_RECOVERY_BYTES: u64 = 128 * 1024 * 1024;

/// Maximum quarantine results retained for the startup UI.
pub const MAX_STARTUP_QUARANTINE_RESULTS: usize = 32;

/// Maximum exact superseded artifacts retained behind one primary offer.
pub const MAX_SUPERSEDED_RECOVERY_HANDLES: usize = 16;

/// Maximum directory entries inspected by one owned-artifact cleanup.
const MAX_OWNED_RECOVERY_CLEANUP_FILES: usize = 256;

/// Maximum individual recovery file size accepted during a startup scan.
pub const MAX_RECOVERY_FILE_BYTES: u64 = 64 * 1024 * 1024 + 256 * 1024;

/// Opaque exact reference to one validated on-disk recovery artifact.
#[derive(Debug)]
pub struct RecoveryRecordHandle {
    path: PathBuf,
    metadata: ValidatedRecoveryMetadata,
    file: File,
    facts: noter_platform::FileFacts,
    encoded_len: u64,
}

impl RecoveryRecordHandle {
    /// Returns the validated metadata bound to this exact artifact.
    pub const fn metadata(&self) -> &ValidatedRecoveryMetadata {
        &self.metadata
    }
}

/// One bounded restore offer and exact causally superseded artifacts.
#[derive(Debug)]
pub struct RecoveryOffer {
    primary: Box<RecoveryRecordHandle>,
    superseded: Vec<RecoveryRecordHandle>,
    superseded_omitted: bool,
}

impl RecoveryOffer {
    /// Returns the primary exact artifact selected for restore.
    pub const fn primary(&self) -> &RecoveryRecordHandle {
        &self.primary
    }

    /// Returns validated metadata for the primary artifact.
    pub const fn metadata(&self) -> &ValidatedRecoveryMetadata {
        self.primary.metadata()
    }

    /// Returns exact older artifacts in deletion-safe order.
    ///
    /// Callers should delete these before deleting [`Self::primary`].
    pub fn superseded(&self) -> &[RecoveryRecordHandle] {
        &self.superseded
    }

    /// Returns whether additional superseded artifacts were intentionally left.
    pub const fn superseded_omitted(&self) -> bool {
        self.superseded_omitted
    }

    /// Consumes the offer into exact cleanup handles, primary last.
    pub fn into_cleanup_handles(mut self) -> Vec<RecoveryRecordHandle> {
        self.superseded.push(*self.primary);
        self.superseded
    }
}

/// Startup classification that never retains recovery document content.
#[derive(Debug)]
pub enum RecoveryScanDisposition {
    /// Offer one bounded metadata-only recovery lineage branch.
    Offer(RecoveryOffer),
    /// A corrupt or unsupported artifact was quarantined or reported.
    Quarantine(RecoveryQuarantineReason),
}

/// One bounded startup scan result paired with its best-known on-disk path.
#[derive(Debug)]
pub struct RecoveryScanEntry {
    path: PathBuf,
    disposition: RecoveryScanDisposition,
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

    /// Returns the metadata-only startup disposition for this path.
    pub const fn disposition(&self) -> &RecoveryScanDisposition {
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
    pub fn into_parts(self) -> (PathBuf, RecoveryScanDisposition) {
        (self.path, self.disposition)
    }
}

/// Bounded result of one recovery startup scan.
#[derive(Debug, Default)]
pub struct RecoveryStartupScan {
    entries: Vec<RecoveryScanEntry>,
    omission_flags: u8,
    quarantine_results_omitted: usize,
}

const FILE_LIMIT_REACHED: u8 = 0b0_0001;
const BYTE_LIMIT_REACHED: u8 = 0b0_0010;
const OFFERS_OMITTED: u8 = 0b0_0100;
const SUPERSEDED_HANDLES_OMITTED: u8 = 0b0_1000;
const DIRECTORY_LIMIT_REACHED: u8 = 0b1_0000;

impl RecoveryStartupScan {
    const fn note_omission(&mut self, flag: u8) {
        self.omission_flags |= flag;
    }

    /// Returns bounded metadata offers and quarantine results.
    pub fn entries(&self) -> &[RecoveryScanEntry] {
        &self.entries
    }

    /// Consumes the scan into its bounded entries.
    pub fn into_entries(self) -> Vec<RecoveryScanEntry> {
        self.entries
    }

    /// Returns whether eligible recovery candidates exceeded their scan bound.
    pub const fn limit_reached(&self) -> bool {
        self.omission_flags & FILE_LIMIT_REACHED != 0
    }

    /// Returns whether raw directory entries exceeded their hard scan bound.
    pub const fn directory_limit_reached(&self) -> bool {
        self.omission_flags & DIRECTORY_LIMIT_REACHED != 0
    }

    /// Returns whether the aggregate encoded-byte scan budget was exhausted.
    pub const fn byte_limit_reached(&self) -> bool {
        self.omission_flags & BYTE_LIMIT_REACHED != 0
    }

    /// Returns whether valid incomparable offers exceeded the offer bound.
    pub const fn offers_omitted(&self) -> bool {
        self.omission_flags & OFFERS_OMITTED != 0
    }

    /// Returns whether exact superseded cleanup handles exceeded their bound.
    pub const fn superseded_handles_omitted(&self) -> bool {
        self.omission_flags & SUPERSEDED_HANDLES_OMITTED != 0
    }

    /// Returns the number of processed quarantine results omitted from the UI.
    pub const fn quarantine_results_omitted(&self) -> usize {
        self.quarantine_results_omitted
    }

    /// Returns whether any bounded scan result was intentionally omitted.
    pub const fn has_omissions(&self) -> bool {
        self.omission_flags != 0 || self.quarantine_results_omitted != 0
    }
}

/// Exclusive instance lease proving that one offered recovery instance is dead.
#[derive(Debug)]
pub struct RecoveryInstanceClaim {
    lease: RecoveryLiveLease,
}

#[derive(Debug)]
struct LockedLiveFile {
    file: File,
    facts: noter_platform::FileFacts,
}

/// Process-lifetime lease for one recovery instance.
///
/// Two independently locked paths keep one pathname rebind from hiding a live
/// owner. Both objects remain locked until facts-bound release completes.
#[derive(Debug)]
pub struct RecoveryLiveLease {
    instance_id: RecoveryInstanceId,
    primary: LockedLiveFile,
    guard: LockedLiveFile,
}

impl RecoveryLiveLease {
    /// Returns the logical recovery instance protected by this lease.
    pub const fn instance_id(&self) -> RecoveryInstanceId {
        self.instance_id
    }
}

impl std::ops::Deref for RecoveryStartupScan {
    type Target = [RecoveryScanEntry];

    fn deref(&self) -> &Self::Target {
        self.entries()
    }
}

impl IntoIterator for RecoveryStartupScan {
    type Item = RecoveryScanEntry;
    type IntoIter = std::vec::IntoIter<RecoveryScanEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
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

    /// Returns the independent live-lease guard for one editor instance.
    pub fn live_guard_path(&self, instance_id: RecoveryInstanceId) -> PathBuf {
        self.records_dir()
            .join(format!("{}.guard", hex16(&instance_id.as_bytes())))
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
    pub fn try_hold_live_lease(
        &self,
        instance_id: RecoveryInstanceId,
    ) -> io::Result<RecoveryLiveLease> {
        let attempt = (|| {
            let primary_path = self.live_path(instance_id);
            let primary = acquire_live_file(&primary_path)?;
            let guard_path = self.live_guard_path(instance_id);
            let guard = match acquire_live_file(&guard_path) {
                Ok(guard) => guard,
                Err(error) => {
                    let _ = delete_locked_live_path(&primary_path, &primary);
                    return Err(error);
                }
            };
            Ok(RecoveryLiveLease {
                instance_id,
                primary,
                guard,
            })
        })();
        match attempt {
            Ok(lease) => Ok(lease),
            Err(error) => Err(classify_live_lease_acquisition_error(error, || {
                self.instance_is_live(instance_id).unwrap_or(false)
            })),
        }
    }

    /// Returns whether another living Noter window still holds this instance.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when ownership cannot be determined. Callers must
    /// fail closed rather than expose a potentially live record for restore or
    /// discard.
    pub fn instance_is_live(&self, instance_id: RecoveryInstanceId) -> io::Result<bool> {
        let primary = probe_live_path(&self.live_path(instance_id))?;
        let guard = probe_live_path(&self.live_guard_path(instance_id))?;
        Ok(primary == Some(true) || guard == Some(true))
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
        write_atomic_private(&destination, snapshot.instance_id(), &encoded)
    }

    /// Removes only the active record for an owned instance after save or
    /// worker invalidation. The process-lifetime live lease remains intact.
    ///
    /// Missing files are treated as success so cleanup is idempotent.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when an existing record cannot be removed.
    pub fn delete_record(&self, instance_id: RecoveryInstanceId) -> io::Result<()> {
        remove_file_if_present(&self.record_path(instance_id))
    }

    /// Removes the canonical record and only keyed temporary artifacts owned by
    /// one instance, without opening or parsing unrelated recovery content.
    ///
    /// The process-lifetime live lease remains intact.
    ///
    /// # Errors
    ///
    /// Returns the first directory-listing or removal error.
    pub fn delete_owned_artifacts(&self, instance_id: RecoveryInstanceId) -> io::Result<()> {
        let mut first_error = self.delete_record(instance_id).err();
        let records = self.records_dir();
        let dir = match fs::read_dir(&records) {
            Ok(dir) => dir,
            Err(error) => match error.kind() {
                io::ErrorKind::NotFound => return first_error.map_or(Ok(()), Err),
                _ => return Err(first_error.unwrap_or(error)),
            },
        };
        for (index, next) in dir.enumerate() {
            if index == MAX_OWNED_RECOVERY_CLEANUP_FILES {
                if first_error.is_none() {
                    first_error = Some(io::Error::other(
                        "owned recovery cleanup reached its directory-entry bound",
                    ));
                }
                break;
            }
            let entry = match next {
                Ok(entry) => entry,
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    continue;
                }
            };
            let candidate = entry.path();
            if keyed_temporary_instance(&candidate) == Some(instance_id)
                && let Err(error) = remove_file_if_present(&candidate)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Reloads one exact startup handle and returns fully validated content.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the path cannot be read, no longer validates,
    /// or no longer matches every item of metadata bound into the handle.
    pub fn load_record(
        &self,
        handle: &RecoveryRecordHandle,
    ) -> io::Result<ValidatedRecoveryRecord> {
        let claim = self.claim_offered_record(handle)?;
        let result = self.load_claimed_record(handle, &claim);
        let release = self.release_claim(claim);
        match (result, release) {
            (Ok(record), Ok(())) => Ok(record),
            (Err(error), _) | (Ok(_), Err(error)) => Err(error),
        }
    }

    /// Claims one offered instance and holds its lease until explicitly released.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::ResourceBusy`] when another window owns the
    /// instance, or another I/O error when the lease cannot be acquired.
    pub fn claim_offered_record(
        &self,
        handle: &RecoveryRecordHandle,
    ) -> io::Result<RecoveryInstanceClaim> {
        self.claim_instance(handle.metadata.instance_id())
    }

    fn claim_instance(&self, instance_id: RecoveryInstanceId) -> io::Result<RecoveryInstanceClaim> {
        self.try_hold_live_lease(instance_id)
            .map(|lease| RecoveryInstanceClaim { lease })
    }

    /// Reloads an exact handle while its instance claim remains held.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for a mismatched claim, changed open object,
    /// pathname replacement, length change, or failed record validation.
    pub fn load_claimed_record(
        &self,
        handle: &RecoveryRecordHandle,
        claim: &RecoveryInstanceClaim,
    ) -> io::Result<ValidatedRecoveryRecord> {
        require_matching_claim(handle, claim)?;
        load_bound_record(handle)
    }

    /// Releases an offered-instance claim and removes its stale lease path.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the stale lease path cannot be removed.
    pub fn release_claim(&self, claim: RecoveryInstanceClaim) -> io::Result<()> {
        self.release_live_lease(claim.lease)
    }

    /// Releases a process-lifetime lease by deleting both exact locked paths
    /// before either lock is dropped.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when either path no longer identifies its locked
    /// lease object or facts-bound deletion fails.
    pub fn release_live_lease(&self, lease: RecoveryLiveLease) -> io::Result<()> {
        let primary = delete_locked_live_path(&self.live_path(lease.instance_id), &lease.primary);
        let guard = delete_locked_live_path(&self.live_guard_path(lease.instance_id), &lease.guard);
        drop(lease);
        primary.and(guard)
    }

    /// Revalidates and removes one exact startup artifact only while its
    /// instance can be exclusively claimed as dead.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::ResourceBusy`] for a live foreign instance and
    /// fails closed when the exact artifact changed or cannot be removed.
    pub fn delete_offered_record(&self, handle: RecoveryRecordHandle) -> io::Result<()> {
        let claim = self.claim_offered_record(&handle)?;
        let result = self.delete_claimed_record(handle, &claim);
        let release = self.release_claim(claim);
        result.and(release)
    }

    /// Deletes one exact open artifact while a matching dead-instance claim is
    /// held. The handle is consumed so Windows deletion can complete.
    ///
    /// # Errors
    ///
    /// Returns an I/O error for a mismatched claim, any identity or content
    /// change, or a failed exact-object deletion.
    pub fn delete_claimed_record(
        &self,
        handle: RecoveryRecordHandle,
        claim: &RecoveryInstanceClaim,
    ) -> io::Result<()> {
        require_matching_claim(&handle, claim)?;
        let _ = load_bound_record(&handle)?;
        delete_bound_record(handle)
    }

    /// Scans a bounded number of active recovery artifacts into metadata-only
    /// restore offers and quarantine results.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the records directory cannot be listed.
    #[allow(clippy::too_many_lines)]
    pub fn scan_startup(&self) -> io::Result<RecoveryStartupScan> {
        let mut scan = RecoveryStartupScan::default();
        let mut paths = Vec::with_capacity(MAX_STARTUP_RECOVERY_FILES);
        let records = self.records_dir();
        let dir = match fs::read_dir(&records) {
            Ok(dir) => dir,
            Err(error) => match error.kind() {
                io::ErrorKind::NotFound => return Ok(scan),
                _ => return Err(error),
            },
        };
        for (raw_index, next) in dir.enumerate() {
            if raw_index == MAX_STARTUP_RECOVERY_DIRECTORY_ENTRIES {
                scan.note_omission(DIRECTORY_LIMIT_REACHED);
                break;
            }
            let path = next?.path();
            let Ok(path_metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !path_metadata.file_type().is_file() {
                continue;
            }
            if let Some(instance_id) = live_instance_from_path(&path) {
                self.cleanup_stale_live_marker(instance_id);
                continue;
            }
            if let Some(instance_id) = recovery_artifact_instance(&path)
                && self.instance_is_live(instance_id)?
            {
                continue;
            }
            if paths.len() == MAX_STARTUP_RECOVERY_FILES {
                scan.note_omission(FILE_LIMIT_REACHED);
                break;
            }
            paths.push(path);
        }
        paths.sort();

        let mut offers = Vec::with_capacity(MAX_STARTUP_RECOVERY_OFFERS);
        let mut scanned_bytes = 0_u64;
        for path in paths {
            let opened = match open_recovery_candidate(&path) {
                Ok(opened) => opened,
                Err(reason) => {
                    retain_quarantine_result(
                        &mut scan,
                        retained_quarantine_entry(
                            path,
                            reason,
                            "Noter left this recovery pathname unchanged because it could not bind the exact file for safe review.",
                        ),
                    );
                    continue;
                }
            };
            let path_instance = recovery_artifact_instance(&path);
            let Ok(header_instance) = peek_recovery_instance(&opened.file, opened.encoded_len)
            else {
                retain_quarantine_result(
                    &mut scan,
                    retained_quarantine_entry(
                        path,
                        RecoveryQuarantineReason::Truncated,
                        "Noter retained this recovery file because its exact header could not be read safely.",
                    ),
                );
                continue;
            };
            let mut belongs_to_live_instance = false;
            for instance_id in path_instance.into_iter().chain(header_instance) {
                if self.instance_is_live(instance_id)? {
                    belongs_to_live_instance = true;
                    break;
                }
            }
            if belongs_to_live_instance {
                continue;
            }
            let Ok(instance_hint) = reconcile_recovery_instance_ids(path_instance, header_instance)
            else {
                retain_quarantine_result(
                    &mut scan,
                    retained_quarantine_entry(
                        path,
                        RecoveryQuarantineReason::InstanceMismatch,
                        "Noter retained the file because neither named instance can authorize movement.",
                    ),
                );
                continue;
            };
            let Some(next_bytes) = advance_scan_byte_budget(scanned_bytes, opened.encoded_len)
            else {
                scan.note_omission(BYTE_LIMIT_REACHED);
                break;
            };
            scanned_bytes = next_bytes;
            let Ok(bytes) = read_bound_open_file(&opened.file, opened.facts, opened.encoded_len)
            else {
                retain_quarantine_result(
                    &mut scan,
                    retained_quarantine_entry(
                        path,
                        RecoveryQuarantineReason::Truncated,
                        "Noter retained this recovery file because the exact open artifact changed while it was being read.",
                    ),
                );
                continue;
            };
            if revalidate_path_identity(&path, opened.facts, opened.encoded_len).is_err() {
                retain_quarantine_result(
                    &mut scan,
                    retained_quarantine_entry(
                        path,
                        RecoveryQuarantineReason::Truncated,
                        "Noter retained this recovery pathname because it changed after the exact file was opened.",
                    ),
                );
                continue;
            }
            match validate_recovery_metadata(&bytes) {
                Ok(metadata) => {
                    if self.instance_is_live(metadata.instance_id())? {
                        continue;
                    }
                    consider_offer(
                        &mut offers,
                        RecoveryRecordHandle {
                            path,
                            metadata,
                            file: opened.file,
                            facts: opened.facts,
                            encoded_len: opened.encoded_len,
                        },
                        &mut scan.omission_flags,
                    );
                }
                Err(reason) => {
                    retain_quarantine_result(
                        &mut scan,
                        self.quarantine_opened_scan_entry(
                            path,
                            &opened,
                            &bytes,
                            reason,
                            instance_hint,
                        ),
                    );
                }
            }
        }
        offers.sort_by(|left, right| offer_sort_key(left).cmp(&offer_sort_key(right)));
        if offers.iter().any(RecoveryOffer::superseded_omitted) {
            scan.note_omission(SUPERSEDED_HANDLES_OMITTED);
        }
        scan.entries.extend(offers.into_iter().map(|offer| {
            let path = offer.primary.path.clone();
            RecoveryScanEntry {
                path,
                disposition: RecoveryScanDisposition::Offer(offer),
                quarantine_error: None,
            }
        }));
        Ok(scan)
    }

    /// Moves one exact bound recovery file into the quarantine directory.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the source cannot be removed after an exact
    /// verified quarantine copy is created. The verified copy remains available
    /// for recovery review. A missing source is reported as
    /// [`io::ErrorKind::NotFound`] rather than success.
    pub fn quarantine_file(&self, path: &Path) -> io::Result<PathBuf> {
        fs::symlink_metadata(path)?;
        let opened = open_recovery_candidate(path)
            .map_err(|reason| io::Error::new(io::ErrorKind::InvalidData, reason.description()))?;
        let path_instance = recovery_artifact_instance(path);
        let header_instance = peek_recovery_instance(&opened.file, opened.encoded_len)?;
        let instance_hint = reconcile_recovery_instance_ids(path_instance, header_instance)
            .map_err(|()| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "recovery pathname and encoded instance identities disagree",
                )
            })?;
        let bytes = read_bound_open_file(&opened.file, opened.facts, opened.encoded_len)?;
        let claim = instance_hint
            .map(|instance_id| self.claim_instance(instance_id))
            .transpose()?;
        let result = quarantine_bound_file(self, path, &opened, &bytes);
        let release = claim.map_or(Ok(()), |claim| self.release_claim(claim));
        finish_public_quarantine(result, release)
    }

    fn cleanup_stale_live_marker(&self, instance_id: RecoveryInstanceId) {
        let Ok(claim) = self.claim_instance(instance_id) else {
            return;
        };
        let _ = self.release_claim(claim);
    }

    fn quarantine_opened_scan_entry(
        &self,
        path: PathBuf,
        opened: &OpenedRecoveryCandidate,
        bytes: &[u8],
        reason: RecoveryQuarantineReason,
        instance_hint: Option<RecoveryInstanceId>,
    ) -> RecoveryScanEntry {
        let claim = match instance_hint.map(|instance_id| self.claim_instance(instance_id)) {
            Some(Ok(claim)) => Some(claim),
            Some(Err(error)) => {
                return retained_quarantine_entry(
                    path,
                    reason,
                    &format!(
                        "Noter retained this damaged recovery file because its instance could not be exclusively claimed ({error})."
                    ),
                );
            }
            None => None,
        };
        let quarantine = quarantine_bound_file(self, &path, opened, bytes);
        let release = claim.map_or(Ok(()), |claim| self.release_claim(claim));
        match (quarantine, release) {
            (Ok((quarantined, None)), Ok(())) => RecoveryScanEntry {
                path: quarantined,
                disposition: RecoveryScanDisposition::Quarantine(reason),
                quarantine_error: None,
            },
            (Ok((quarantined, cleanup_error)), release) => RecoveryScanEntry {
                path: quarantined,
                disposition: RecoveryScanDisposition::Quarantine(reason),
                quarantine_error: Some(format!(
                    "Noter preserved the exact damaged recovery bytes in quarantine, but source or instance-claim cleanup was incomplete ({}).",
                    cleanup_error
                        .map(|error| error.to_string())
                        .or_else(|| release.err().map(|error| error.to_string()))
                        .unwrap_or_else(|| "unknown cleanup failure".to_owned())
                )),
            },
            (Err(error), _) => retained_quarantine_entry(
                path,
                reason,
                &format!(
                    "Noter retained this damaged recovery file because exact quarantine could not be completed ({error})."
                ),
            ),
        }
    }
}

fn finish_public_quarantine(
    quarantine: io::Result<(PathBuf, Option<io::Error>)>,
    release: io::Result<()>,
) -> io::Result<PathBuf> {
    match (quarantine, release) {
        (Ok((destination, None)), Ok(())) => Ok(destination),
        (Ok((_, Some(error))) | Err(error), _) | (Ok(_), Err(error)) => Err(error),
    }
}

fn retained_quarantine_entry(
    path: PathBuf,
    reason: RecoveryQuarantineReason,
    message: &str,
) -> RecoveryScanEntry {
    RecoveryScanEntry {
        path,
        disposition: RecoveryScanDisposition::Quarantine(reason),
        quarantine_error: Some(message.to_owned()),
    }
}

fn quarantine_bound_file(
    store: &RecoveryStore,
    path: &Path,
    opened: &OpenedRecoveryCandidate,
    bytes: &[u8],
) -> io::Result<(PathBuf, Option<io::Error>)> {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != opened.encoded_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bound recovery bytes do not match the opened artifact length",
        ));
    }
    revalidate_path_identity(path, opened.facts, opened.encoded_len)?;
    let (destination, mut quarantine_file) = create_quarantine_copy(store, bytes)?;
    if let Err(error) = verify_exact_bytes(&mut quarantine_file, bytes) {
        let _ = noter_platform::delete_open_file(&quarantine_file);
        drop(quarantine_file);
        let _ = fs::remove_file(&destination);
        return Err(error);
    }

    let cleanup_result = delete_bound_candidate(path, opened);
    drop(quarantine_file);
    let _ = noter_platform::sync_parent(&destination);
    Ok((destination, cleanup_result.err()))
}

fn create_quarantine_copy(store: &RecoveryStore, bytes: &[u8]) -> io::Result<(PathBuf, File)> {
    for _ in 0..32 {
        let mut random = [0_u8; 16];
        fill_random(&mut random).map_err(|error| {
            io::Error::other(format!("recovery quarantine random name failed: {error}"))
        })?;
        let destination = store
            .quarantine_dir()
            .join(format!("noter-quarantine-{}.rec", hex16(&random)));
        let mut file = match noter_platform::create_private_new_file(&destination) {
            Ok(file) => file,
            Err(error) => {
                if is_quarantine_name_collision(error.kind()) {
                    continue;
                }
                return Err(error);
            }
        };
        let write_result = (|| {
            file.write_all(bytes)?;
            file.flush()?;
            noter_platform::sync_file(&file)
        })();
        if let Err(error) = write_result {
            let _ = noter_platform::delete_open_file(&file);
            drop(file);
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
        return Ok((destination, file));
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a private recovery quarantine name",
    ))
}

fn verify_exact_bytes(file: &mut File, expected: &[u8]) -> io::Result<()> {
    file.seek(SeekFrom::Start(0))?;
    let mut buffer = vec![0_u8; 64 * 1024].into_boxed_slice();
    for expected_chunk in expected.chunks(buffer.len()) {
        file.read_exact(&mut buffer[..expected_chunk.len()])?;
        if buffer[..expected_chunk.len()] != *expected_chunk {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "quarantined recovery bytes did not verify",
            ));
        }
    }
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing)? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quarantined recovery copy has trailing bytes",
        ));
    }
    Ok(())
}

fn delete_bound_candidate(path: &Path, opened: &OpenedRecoveryCandidate) -> io::Result<()> {
    revalidate_path_identity(path, opened.facts, opened.encoded_len)?;
    let cleanup_file = noter_platform::open_for_cleanup(path)?;
    if !file_binding_matches(
        noter_platform::file_facts(&cleanup_file)? == opened.facts,
        cleanup_file.metadata()?.len() == opened.encoded_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery pathname changed before quarantine cleanup",
        ));
    }
    match noter_platform::delete_open_file(&cleanup_file) {
        Ok(()) => Ok(()),
        Err(error) => {
            if requires_path_delete_fallback(error.kind()) {
                delete_bound_candidate_unix_fallback(path, opened, cleanup_file)
            } else {
                Err(error)
            }
        }
    }
}

fn delete_bound_candidate_unix_fallback(
    path: &Path,
    opened: &OpenedRecoveryCandidate,
    cleanup_file: File,
) -> io::Result<()> {
    revalidate_path_identity(path, opened.facts, opened.encoded_len)?;
    drop(cleanup_file);
    fs::remove_file(path)?;
    if noter_platform::file_facts(&opened.file)?.link_count() >= opened.facts.link_count() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "quarantine cleanup did not unlink the bound recovery object",
        ));
    }
    Ok(())
}

fn live_instance_from_path(path: &Path) -> Option<RecoveryInstanceId> {
    let name = path.file_name()?.to_str()?;
    let instance = name
        .strip_suffix(".live")
        .or_else(|| name.strip_suffix(".guard"))?;
    decode_hex16(instance).map(RecoveryInstanceId::new)
}

fn reconcile_recovery_instance_ids(
    path_instance: Option<RecoveryInstanceId>,
    header_instance: Option<RecoveryInstanceId>,
) -> Result<Option<RecoveryInstanceId>, ()> {
    match (path_instance, header_instance) {
        (Some(path_id), Some(header_id)) => {
            if path_id == header_id {
                Ok(Some(path_id))
            } else {
                Err(())
            }
        }
        (Some(instance_id), None) | (None, Some(instance_id)) => Ok(Some(instance_id)),
        (None, None) => Ok(None),
    }
}

fn recovery_artifact_instance(path: &Path) -> Option<RecoveryInstanceId> {
    keyed_temporary_instance(path).or_else(|| {
        let name = path.file_name()?.to_str()?;
        let instance = name.strip_suffix(".rec")?;
        decode_hex16(instance).map(RecoveryInstanceId::new)
    })
}

fn peek_recovery_instance(file: &File, encoded_len: u64) -> io::Result<Option<RecoveryInstanceId>> {
    const IDENTITY_HEADER_LEN: usize = 44;
    if encoded_len < u64::try_from(IDENTITY_HEADER_LEN).unwrap_or(u64::MAX) {
        return Ok(None);
    }
    let mut reader = file;
    reader.seek(SeekFrom::Start(0))?;
    let mut header = [0_u8; IDENTITY_HEADER_LEN];
    reader.read_exact(&mut header)?;
    if &header[..RECOVERY_MAGIC.len()] != RECOVERY_MAGIC {
        return Ok(None);
    }
    let schema = u32::from_le_bytes(
        header[8..12]
            .try_into()
            .expect("the recovery schema slice has four bytes"),
    );
    if !matches!(schema, 1 | RECOVERY_SCHEMA_VERSION) {
        return Ok(None);
    }
    let instance = header[28..44]
        .try_into()
        .expect("the recovery instance slice has sixteen bytes");
    Ok(Some(RecoveryInstanceId::new(instance)))
}

fn consider_offer(
    offers: &mut Vec<RecoveryOffer>,
    handle: RecoveryRecordHandle,
    omission_flags: &mut u8,
) {
    let mut candidate = RecoveryOffer {
        primary: Box::new(handle),
        superseded: Vec::new(),
        superseded_omitted: false,
    };

    if let Some(index) = offers
        .iter()
        .position(|offer| offer.primary.metadata == candidate.primary.metadata)
    {
        absorb_superseded(&mut offers[index], candidate);
        return;
    }

    while let Some(index) = offers.iter().position(|offer| {
        authenticated_same_instance_document(&candidate.primary.metadata, &offer.primary.metadata)
            && candidate.primary.metadata.revision() > offer.primary.metadata.revision()
    }) {
        let older = offers.remove(index);
        absorb_superseded(&mut candidate, older);
    }
    if let Some(index) = offers.iter().position(|offer| {
        authenticated_same_instance_document(&candidate.primary.metadata, &offer.primary.metadata)
            && offer.primary.metadata.revision() > candidate.primary.metadata.revision()
    }) {
        absorb_superseded(&mut offers[index], candidate);
        return;
    }

    while let Some(index) = offers
        .iter()
        .position(|offer| directly_supersedes(&candidate.primary.metadata, &offer.primary.metadata))
    {
        let older = offers.remove(index);
        absorb_superseded(&mut candidate, older);
    }
    if let Some(index) = offers
        .iter()
        .position(|offer| directly_supersedes(&offer.primary.metadata, &candidate.primary.metadata))
    {
        absorb_superseded(&mut offers[index], candidate);
        return;
    }
    if offers.len() < MAX_STARTUP_RECOVERY_OFFERS {
        offers.push(candidate);
    } else {
        *omission_flags |= OFFERS_OMITTED;
    }
}

fn same_instance_document(
    left: &ValidatedRecoveryMetadata,
    right: &ValidatedRecoveryMetadata,
) -> bool {
    left.instance_id() == right.instance_id() && left.document_id() == right.document_id()
}

fn authenticated_same_instance_document(
    left: &ValidatedRecoveryMetadata,
    right: &ValidatedRecoveryMetadata,
) -> bool {
    left.schema_version() == RECOVERY_SCHEMA_VERSION
        && right.schema_version() == RECOVERY_SCHEMA_VERSION
        && same_instance_document(left, right)
}

fn push_superseded(offer: &mut RecoveryOffer, handle: RecoveryRecordHandle) {
    if offer.superseded.len() < MAX_SUPERSEDED_RECOVERY_HANDLES {
        offer.superseded.push(handle);
    } else {
        offer.superseded_omitted = true;
    }
}

fn absorb_superseded(target: &mut RecoveryOffer, older: RecoveryOffer) {
    for handle in older.superseded {
        push_superseded(target, handle);
    }
    push_superseded(target, *older.primary);
    target.superseded_omitted |= older.superseded_omitted;
}

fn directly_supersedes(
    candidate: &ValidatedRecoveryMetadata,
    current: &ValidatedRecoveryMetadata,
) -> bool {
    candidate.schema_version() == RECOVERY_SCHEMA_VERSION
        && current.schema_version() == RECOVERY_SCHEMA_VERSION
        && candidate.document_id() == current.document_id()
        && candidate.predecessor_instance() == Some(current.instance_id())
        && current
            .lineage_generation()
            .and_then(super::recovery::RecoveryLineageGeneration::checked_next)
            == candidate.lineage_generation()
}

fn offer_sort_key(offer: &RecoveryOffer) -> ([u8; 16], [u8; 16], &Path) {
    (
        offer.metadata().document_id().as_bytes(),
        offer.metadata().instance_id().as_bytes(),
        &offer.primary.path,
    )
}

fn validate_and_lock_live_file(file: File) -> io::Result<(File, noter_platform::FileFacts)> {
    let metadata = file.metadata()?;
    let facts = noter_platform::file_facts(&file)?;
    if !valid_live_file_shape(metadata.is_file(), facts.link_count()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery live lease is not a private regular file",
        ));
    }
    file.try_lock().map_err(|error| match error {
        TryLockError::WouldBlock => live_lease_busy_error(),
        TryLockError::Error(error) => error,
    })?;
    Ok((file, facts))
}

fn acquire_live_file(path: &Path) -> io::Result<LockedLiveFile> {
    let file = match noter_platform::create_private_new_file(path) {
        Ok(file) => file,
        Err(error) => match error.kind() {
            io::ErrorKind::AlreadyExists => noter_platform::open_for_cleanup(path)?,
            _ => return Err(error),
        },
    };
    let (file, facts) = validate_and_lock_live_file(file)?;
    Ok(LockedLiveFile { file, facts })
}

fn probe_live_path(path: &Path) -> io::Result<Option<bool>> {
    let file = match noter_platform::open_existing_no_follow(path) {
        Ok(file) => file,
        Err(error) => match error.kind() {
            io::ErrorKind::NotFound => return Ok(None),
            _ => return Err(error),
        },
    };
    let metadata = file.metadata()?;
    let facts = noter_platform::file_facts(&file)?;
    if !valid_live_file_shape(metadata.is_file(), facts.link_count()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery live lease is not a private regular file",
        ));
    }
    match file.try_lock() {
        Ok(()) => Ok(Some(false)),
        Err(TryLockError::WouldBlock) => Ok(Some(true)),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

const fn valid_live_file_shape(is_file: bool, link_count: u64) -> bool {
    is_file && link_count == 1
}

fn live_lease_busy_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::ResourceBusy,
        "recovery live lease is held by another Noter window",
    )
}

fn classify_nonbusy_lease_error(error: io::Error, observed_live: bool) -> io::Error {
    if observed_live {
        live_lease_busy_error()
    } else {
        error
    }
}

fn classify_live_lease_acquisition_error(
    error: io::Error,
    observe_live: impl FnOnce() -> bool,
) -> io::Error {
    match error.kind() {
        io::ErrorKind::ResourceBusy => error,
        _ => classify_nonbusy_lease_error(error, observe_live()),
    }
}

const fn is_quarantine_name_collision(kind: io::ErrorKind) -> bool {
    matches!(kind, io::ErrorKind::AlreadyExists)
}

const fn requires_path_delete_fallback(kind: io::ErrorKind) -> bool {
    matches!(kind, io::ErrorKind::Unsupported)
}

fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn retain_quarantine_result(scan: &mut RecoveryStartupScan, entry: RecoveryScanEntry) {
    if scan.entries.len() < MAX_STARTUP_QUARANTINE_RESULTS {
        scan.entries.push(entry);
    } else {
        scan.quarantine_results_omitted = scan.quarantine_results_omitted.saturating_add(1);
    }
}

fn advance_scan_byte_budget(consumed: u64, next: u64) -> Option<u64> {
    consumed
        .checked_add(next)
        .filter(|total| *total <= MAX_STARTUP_RECOVERY_BYTES)
}

struct OpenedRecoveryCandidate {
    file: File,
    facts: noter_platform::FileFacts,
    encoded_len: u64,
}

fn open_recovery_candidate(
    path: &Path,
) -> Result<OpenedRecoveryCandidate, RecoveryQuarantineReason> {
    let file = noter_platform::open_existing_no_follow(path)
        .map_err(|_| RecoveryQuarantineReason::Truncated)?;
    let metadata = file
        .metadata()
        .map_err(|_| RecoveryQuarantineReason::Truncated)?;
    if !metadata.is_file() {
        return Err(RecoveryQuarantineReason::Truncated);
    }
    if exceeds_recovery_file_bound(metadata.len()) {
        return Err(RecoveryQuarantineReason::ContentTooLarge);
    }
    let facts =
        noter_platform::file_facts(&file).map_err(|_| RecoveryQuarantineReason::Truncated)?;
    if facts.link_count() != 1 {
        return Err(RecoveryQuarantineReason::Truncated);
    }
    Ok(OpenedRecoveryCandidate {
        file,
        facts,
        encoded_len: metadata.len(),
    })
}

fn read_bound_open_file(
    file: &File,
    expected_facts: noter_platform::FileFacts,
    expected_len: u64,
) -> io::Result<Vec<u8>> {
    if !file_binding_matches(
        noter_platform::file_facts(file)? == expected_facts,
        file.metadata()?.len() == expected_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery artifact changed before validation",
        ));
    }
    let mut reader = file;
    reader.seek(SeekFrom::Start(0))?;
    let mut bytes = Vec::with_capacity(
        usize::try_from(expected_len)
            .unwrap_or(0)
            .min(usize::try_from(MAX_RECOVERY_FILE_BYTES).unwrap_or(usize::MAX)),
    );
    reader
        .take(MAX_RECOVERY_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if exceeds_recovery_file_bound(u64::try_from(bytes.len()).unwrap_or(u64::MAX)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery artifact exceeded the read bound",
        ));
    }
    if !complete_bound_read_matches(
        u64::try_from(bytes.len()).unwrap_or(u64::MAX) == expected_len,
        noter_platform::file_facts(file)? == expected_facts,
        file.metadata()?.len() == expected_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery artifact changed during validation",
        ));
    }
    Ok(bytes)
}

fn revalidate_path_identity(
    path: &Path,
    expected_facts: noter_platform::FileFacts,
    expected_len: u64,
) -> io::Result<()> {
    let path_file = noter_platform::open_existing_no_follow(path)?;
    if !file_binding_matches(
        noter_platform::file_facts(&path_file)? == expected_facts,
        path_file.metadata()?.len() == expected_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery pathname no longer identifies the validated artifact",
        ));
    }
    Ok(())
}

const fn exceeds_recovery_file_bound(encoded_len: u64) -> bool {
    encoded_len > MAX_RECOVERY_FILE_BYTES
}

const fn file_binding_matches(facts_match: bool, length_matches: bool) -> bool {
    facts_match && length_matches
}

const fn complete_bound_read_matches(
    bytes_read_match: bool,
    facts_match: bool,
    length_matches: bool,
) -> bool {
    bytes_read_match && facts_match && length_matches
}

fn require_matching_claim(
    handle: &RecoveryRecordHandle,
    claim: &RecoveryInstanceClaim,
) -> io::Result<()> {
    if claim.lease.instance_id != handle.metadata.instance_id() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "recovery instance claim does not match the artifact",
        ));
    }
    Ok(())
}

fn load_bound_record(handle: &RecoveryRecordHandle) -> io::Result<ValidatedRecoveryRecord> {
    let bytes = read_bound_open_file(&handle.file, handle.facts, handle.encoded_len)?;
    revalidate_path_identity(&handle.path, handle.facts, handle.encoded_len)?;
    let RecoveryStartupDisposition::Offer(record) = validate_recovery_record(&bytes) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery offer no longer validates",
        ));
    };
    if record.metadata() != &handle.metadata {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery offer changed after startup validation",
        ));
    }
    Ok(record)
}

fn delete_bound_record(handle: RecoveryRecordHandle) -> io::Result<()> {
    let cleanup_file = noter_platform::open_for_cleanup(&handle.path)?;
    if !file_binding_matches(
        noter_platform::file_facts(&cleanup_file)? == handle.facts,
        cleanup_file.metadata()?.len() == handle.encoded_len,
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery pathname changed before exact cleanup",
        ));
    }
    match noter_platform::delete_open_file(&cleanup_file) {
        Ok(()) => {
            drop(cleanup_file);
            drop(handle);
            Ok(())
        }
        Err(error) => {
            if requires_path_delete_fallback(error.kind()) {
                delete_bound_record_unix_fallback(&handle, cleanup_file)
            } else {
                Err(error)
            }
        }
    }
}

fn delete_bound_record_unix_fallback(
    handle: &RecoveryRecordHandle,
    cleanup_file: File,
) -> io::Result<()> {
    revalidate_path_identity(&handle.path, handle.facts, handle.encoded_len)?;
    drop(cleanup_file);
    fs::remove_file(&handle.path)?;
    if noter_platform::file_facts(&handle.file)?.link_count() >= handle.facts.link_count() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery pathname cleanup did not unlink the validated object",
        ));
    }
    Ok(())
}

fn delete_claimed_live_path(
    path: &Path,
    lease: &File,
    expected_facts: noter_platform::FileFacts,
) -> io::Result<()> {
    if noter_platform::file_facts(lease)? != expected_facts {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery live lease changed while its claim was held",
        ));
    }
    match noter_platform::delete_open_file(lease) {
        Ok(()) => Ok(()),
        Err(error) => {
            if requires_path_delete_fallback(error.kind()) {
                delete_claimed_live_path_unix_fallback(path, lease, expected_facts)
            } else {
                Err(error)
            }
        }
    }
}

fn delete_claimed_live_path_unix_fallback(
    path: &Path,
    lease: &File,
    expected_facts: noter_platform::FileFacts,
) -> io::Result<()> {
    let path_file = match noter_platform::open_existing_no_follow(path) {
        Ok(file) => file,
        Err(error) => match error.kind() {
            io::ErrorKind::NotFound => return Ok(()),
            _ => return Err(error),
        },
    };
    if noter_platform::file_facts(&path_file)? != expected_facts {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery live pathname changed before claim cleanup",
        ));
    }
    drop(path_file);
    fs::remove_file(path)?;
    if noter_platform::file_facts(lease)?.link_count() >= expected_facts.link_count() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "recovery live claim cleanup did not unlink the claimed object",
        ));
    }
    Ok(())
}

fn delete_locked_live_path(path: &Path, locked: &LockedLiveFile) -> io::Result<()> {
    delete_claimed_live_path(path, &locked.file, locked.facts)
}

#[derive(Clone, Copy)]
enum TemporaryArtifactKind {
    Stage,
    Backup,
}

impl TemporaryArtifactKind {
    const fn extension(self) -> &'static str {
        match self {
            Self::Stage => "stage",
            Self::Backup => "backup",
        }
    }
}

fn write_atomic_private(
    destination: &Path,
    instance_id: RecoveryInstanceId,
    bytes: &[u8],
) -> io::Result<()> {
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let stage = exclusive_stage_path(parent, instance_id, TemporaryArtifactKind::Stage)?;
    let backup = exclusive_stage_path(parent, instance_id, TemporaryArtifactKind::Backup)?;
    let write_result = commit_staged_record(&stage, destination, &backup, bytes);

    if let Err(error) = write_result {
        let _ = fs::remove_file(&stage);
        // A failed replace may leave a backup sibling; remove only empty or
        // newly created backup names that are not the committed destination.
        let _ = fs::remove_file(&backup);
        return Err(error);
    }
    // Successful replace on Windows keeps the previous destination in the
    // backup path. That is superseded recovery content and must not linger.
    remove_file_if_present(&backup)?;
    noter_platform::sync_parent(destination).map(|_| ())
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

fn exclusive_stage_path(
    parent: &Path,
    instance_id: RecoveryInstanceId,
    kind: TemporaryArtifactKind,
) -> io::Result<PathBuf> {
    for _ in 0..32 {
        let mut random = [0_u8; 16];
        fill_random(&mut random).map_err(|error| {
            io::Error::other(format!("recovery stage random name failed: {error}"))
        })?;
        let path = parent.join(format!(
            ".noter-recovery-{}-{}.{}",
            hex16(&instance_id.as_bytes()),
            hex16(&random),
            kind.extension()
        ));
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a private recovery stage name",
    ))
}

fn keyed_temporary_instance(path: &Path) -> Option<RecoveryInstanceId> {
    let name = path.file_name()?.to_str()?;
    let remainder = name.strip_prefix(".noter-recovery-")?;
    let (instance_hex, random_and_extension) = remainder.split_once('-')?;
    let (random_hex, extension) = random_and_extension.rsplit_once('.')?;
    if !matches!(extension, "stage" | "backup") {
        return None;
    }
    let _ = decode_hex16(random_hex)?;
    decode_hex16(instance_hex).map(RecoveryInstanceId::new)
}

fn decode_hex16(value: &str) -> Option<[u8; 16]> {
    if value.len() != 32 {
        return None;
    }
    let mut bytes = [0_u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let offset = index.saturating_mul(2);
        *byte = u8::from_str_radix(value.get(offset..offset.saturating_add(2))?, 16).ok()?;
    }
    Some(bytes)
}

fn hex16(bytes: &[u8; 16]) -> String {
    let mut out = String::with_capacity(32);
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
        RECOVERY_MAGIC, RECOVERY_SCHEMA_VERSION, RecoveryDocumentId, RecoveryLineageGeneration,
        RecoverySnapshotParts, RecoveryWallTime,
    };
    use crate::core::revision::Revision;
    use crate::core::save::ContentFingerprint;
    use crate::core::text_format::{Bom, Encoding};
    use tempfile::tempdir;

    fn sample_snapshot(instance: u8, content: &[u8]) -> RecoverySnapshot {
        snapshot_with_document(instance, instance, u64::from(instance), 2, content)
    }

    fn indexed_instance(index: usize) -> RecoveryInstanceId {
        let mut bytes = [0_u8; 16];
        bytes[..std::mem::size_of::<usize>()].copy_from_slice(&index.to_le_bytes());
        RecoveryInstanceId::new(bytes)
    }

    fn snapshot_at(
        instance: u8,
        revision: u64,
        updated_at: u64,
        content: &[u8],
    ) -> RecoverySnapshot {
        snapshot_with_document(9, instance, revision, updated_at, content)
    }

    fn snapshot_with_document(
        document: u8,
        instance: u8,
        revision: u64,
        updated_at: u64,
        content: &[u8],
    ) -> RecoverySnapshot {
        RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([document; 16]),
            instance_id: RecoveryInstanceId::new([instance; 16]),
            revision: Revision::new(revision),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(updated_at),
            original_path: b"memo.txt".to_vec(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(content.len().min(1)),
            content: content.to_vec(),
        })
        .expect("snapshot")
    }

    fn offer(entry: &RecoveryScanEntry) -> &RecoveryOffer {
        let RecoveryScanDisposition::Offer(offer) = entry.disposition() else {
            panic!("expected recovery offer, got {:?}", entry.disposition());
        };
        offer
    }

    fn load_offer(
        store: &RecoveryStore,
        entry: &RecoveryScanEntry,
    ) -> io::Result<ValidatedRecoveryRecord> {
        store.load_record(offer(entry).primary())
    }

    fn encode_v1(snapshot: &RecoverySnapshot) -> Vec<u8> {
        const V1_HEADER_LEN: usize = 130;
        let path_len = u32::try_from(snapshot.original_path().len()).expect("fixture path length");
        let content_len = u64::try_from(snapshot.content().len()).expect("fixture content length");
        let checksum = ContentFingerprint::from_bytes(snapshot.content());
        let mut bytes = Vec::with_capacity(
            V1_HEADER_LEN + snapshot.original_path().len() + snapshot.content().len(),
        );
        bytes.extend_from_slice(RECOVERY_MAGIC);
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&snapshot.document_id().as_bytes());
        bytes.extend_from_slice(&snapshot.instance_id().as_bytes());
        bytes.extend_from_slice(&snapshot.revision().get().to_le_bytes());
        bytes.extend_from_slice(&snapshot.created_at().unix_millis().to_le_bytes());
        bytes.extend_from_slice(&snapshot.updated_at().unix_millis().to_le_bytes());
        bytes.extend_from_slice(&path_len.to_le_bytes());
        bytes.push(u8::from(snapshot.bom() == Bom::Utf8));
        bytes.push(0);
        bytes.extend_from_slice(
            &u64::try_from(snapshot.selection().anchor())
                .expect("fixture selection")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &u64::try_from(snapshot.selection().active())
                .expect("fixture selection")
                .to_le_bytes(),
        );
        bytes.extend_from_slice(&content_len.to_le_bytes());
        bytes.extend_from_slice(checksum.as_bytes());
        bytes.extend_from_slice(snapshot.original_path());
        bytes.extend_from_slice(snapshot.content());
        bytes
    }

    #[test]
    fn omission_flags_have_independent_exact_semantics() {
        let empty = RecoveryStartupScan::default();
        assert!(!empty.limit_reached());
        assert!(!empty.directory_limit_reached());
        assert!(!empty.byte_limit_reached());
        assert!(!empty.offers_omitted());
        assert!(!empty.superseded_handles_omitted());
        assert!(!empty.has_omissions());

        let cases = [
            (FILE_LIMIT_REACHED, [true, false, false, false, false]),
            (DIRECTORY_LIMIT_REACHED, [false, true, false, false, false]),
            (BYTE_LIMIT_REACHED, [false, false, true, false, false]),
            (OFFERS_OMITTED, [false, false, false, true, false]),
            (
                SUPERSEDED_HANDLES_OMITTED,
                [false, false, false, false, true],
            ),
        ];
        for (flag, expected) in cases {
            let mut scan = RecoveryStartupScan::default();
            scan.note_omission(flag);
            assert_eq!(
                [
                    scan.limit_reached(),
                    scan.directory_limit_reached(),
                    scan.byte_limit_reached(),
                    scan.offers_omitted(),
                    scan.superseded_handles_omitted(),
                ],
                expected
            );
            assert!(scan.has_omissions());
        }

        let quarantine_only = RecoveryStartupScan {
            quarantine_results_omitted: 1,
            ..RecoveryStartupScan::default()
        };
        assert!(quarantine_only.has_omissions());
    }

    #[test]
    fn recovery_store_decision_predicates_cover_each_independent_input() {
        assert!(valid_live_file_shape(true, 1));
        assert!(!valid_live_file_shape(false, 1));
        assert!(!valid_live_file_shape(true, 2));

        assert!(file_binding_matches(true, true));
        assert!(!file_binding_matches(false, true));
        assert!(!file_binding_matches(true, false));

        assert!(complete_bound_read_matches(true, true, true));
        assert!(!complete_bound_read_matches(false, true, true));
        assert!(!complete_bound_read_matches(true, false, true));
        assert!(!complete_bound_read_matches(true, true, false));

        assert!(!exceeds_recovery_file_bound(MAX_RECOVERY_FILE_BYTES));
        assert!(exceeds_recovery_file_bound(MAX_RECOVERY_FILE_BYTES + 1));

        let original = classify_nonbusy_lease_error(
            io::Error::new(io::ErrorKind::PermissionDenied, "original lease error"),
            false,
        );
        assert_eq!(original.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(original.to_string(), "original lease error");
        let reclassified = classify_nonbusy_lease_error(
            io::Error::new(io::ErrorKind::PermissionDenied, "original lease error"),
            true,
        );
        assert_eq!(reclassified.kind(), io::ErrorKind::ResourceBusy);
        assert_eq!(
            reclassified.to_string(),
            "recovery live lease is held by another Noter window"
        );

        let busy = classify_live_lease_acquisition_error(
            io::Error::new(io::ErrorKind::ResourceBusy, "exact contention evidence"),
            || panic!("an exact contention result must not be probed again"),
        );
        assert_eq!(busy.kind(), io::ErrorKind::ResourceBusy);
        assert_eq!(busy.to_string(), "exact contention evidence");

        let mut probed = false;
        let inferred_busy = classify_live_lease_acquisition_error(
            io::Error::new(io::ErrorKind::PermissionDenied, "ambiguous lease failure"),
            || {
                probed = true;
                true
            },
        );
        assert!(probed);
        assert_eq!(inferred_busy.kind(), io::ErrorKind::ResourceBusy);

        assert!(is_quarantine_name_collision(io::ErrorKind::AlreadyExists));
        assert!(!is_quarantine_name_collision(
            io::ErrorKind::PermissionDenied
        ));
        assert!(!is_quarantine_name_collision(io::ErrorKind::NotADirectory));

        assert!(requires_path_delete_fallback(io::ErrorKind::Unsupported));
        assert!(!requires_path_delete_fallback(
            io::ErrorKind::PermissionDenied
        ));
        assert!(!requires_path_delete_fallback(io::ErrorKind::ResourceBusy));
    }

    #[test]
    fn recovery_instance_sources_reconcile_only_when_they_agree() {
        let first = RecoveryInstanceId::new([1; 16]);
        let second = RecoveryInstanceId::new([2; 16]);
        assert_eq!(
            reconcile_recovery_instance_ids(Some(first), Some(first)),
            Ok(Some(first))
        );
        assert_eq!(
            reconcile_recovery_instance_ids(Some(first), None),
            Ok(Some(first))
        );
        assert_eq!(
            reconcile_recovery_instance_ids(None, Some(first)),
            Ok(Some(first))
        );
        assert_eq!(reconcile_recovery_instance_ids(None, None), Ok(None));
        assert_eq!(
            reconcile_recovery_instance_ids(Some(first), Some(second)),
            Err(())
        );
    }

    #[test]
    fn recovery_artifact_names_and_minimal_headers_are_exact() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(22, b"header");
        let instance_id = snapshot.instance_id();
        assert_eq!(
            recovery_artifact_instance(&store.record_path(instance_id)),
            Some(instance_id)
        );
        let stage = exclusive_stage_path(
            &store.records_dir(),
            instance_id,
            TemporaryArtifactKind::Stage,
        )?;
        let backup = exclusive_stage_path(
            &store.records_dir(),
            instance_id,
            TemporaryArtifactKind::Backup,
        )?;
        assert_eq!(recovery_artifact_instance(&stage), Some(instance_id));
        assert_eq!(recovery_artifact_instance(&backup), Some(instance_id));
        assert_eq!(
            recovery_artifact_instance(&store.records_dir().join("not-a-record.txt")),
            None
        );

        let path = store.records_dir().join("minimal-header.bin");
        let mut header = snapshot.encode();
        header.truncate(44);
        fs::write(&path, &header)?;
        let file = File::open(&path)?;
        assert_eq!(peek_recovery_instance(&file, 43)?, None);
        assert_eq!(peek_recovery_instance(&file, 44)?, Some(instance_id));

        let mut legacy = header.clone();
        legacy[8..12].copy_from_slice(&1_u32.to_le_bytes());
        fs::write(&path, &legacy)?;
        let file = File::open(&path)?;
        assert_eq!(peek_recovery_instance(&file, 44)?, Some(instance_id));

        let mut bad_magic = header.clone();
        bad_magic[0] ^= 1;
        fs::write(&path, bad_magic)?;
        let file = File::open(&path)?;
        assert_eq!(peek_recovery_instance(&file, 44)?, None);

        let mut unknown_schema = header;
        unknown_schema[8..12].copy_from_slice(&99_u32.to_le_bytes());
        fs::write(&path, unknown_schema)?;
        let file = File::open(&path)?;
        assert_eq!(peek_recovery_instance(&file, 44)?, None);
        Ok(())
    }

    #[test]
    fn quarantine_copy_verification_rejects_mismatch_and_trailing_bytes() -> io::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("quarantine-copy.bin");
        fs::write(&path, b"exact bytes")?;

        let mut file = File::open(&path)?;
        verify_exact_bytes(&mut file, b"exact bytes")?;

        let mut file = File::open(&path)?;
        let mismatch = verify_exact_bytes(&mut file, b"exact bytez")
            .expect_err("a byte mismatch must fail verification");
        assert_eq!(mismatch.kind(), io::ErrorKind::InvalidData);

        let mut file = File::open(&path)?;
        let trailing = verify_exact_bytes(&mut file, b"exact")
            .expect_err("trailing bytes must fail verification");
        assert_eq!(trailing.kind(), io::ErrorKind::InvalidData);
        Ok(())
    }

    #[test]
    fn persist_scan_and_delete_round_trip() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(1, b"recovered text");
        store.persist(&snapshot)?;

        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert!(!offer(&entries[0]).superseded_omitted());
        let record = load_offer(&store, &entries[0])?;
        assert_eq!(record.content(), b"recovered text");
        assert_eq!(record.instance_id(), snapshot.instance_id());
        assert_eq!(record.original_path(), b"memo.txt");
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
        assert_eq!(load_offer(&store, &entries[0])?.content(), b"updated");

        store.delete_owned_artifacts(snapshot.instance_id())?;
        assert!(
            store
                .scan_startup()?
                .iter()
                .all(|entry| !matches!(entry.disposition(), RecoveryScanDisposition::Offer(_)))
        );
        store.delete_owned_artifacts(snapshot.instance_id())?;
        Ok(())
    }

    #[test]
    fn startup_coalesces_interrupted_replacement_to_newest_revision() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let old = snapshot_at(7, 1, 10, b"old recovery");
        let newest = snapshot_at(7, 2, 11, b"new recovery");
        store.persist(&newest)?;
        let retained_stage = store.records_dir().join(".noter-recovery-interrupted.tmp");
        fs::write(&retained_stage, old.encode())?;

        let entries = store.scan_startup()?;
        let offers: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) => Some(offer),
                RecoveryScanDisposition::Quarantine(_) => None,
            })
            .collect();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].metadata().revision(), Revision::new(2));
        assert_eq!(
            store.load_record(offers[0].primary())?.content(),
            b"new recovery"
        );
        assert_eq!(offers[0].superseded().len(), 1);
        let entry = entries
            .into_entries()
            .pop()
            .expect("one coalesced recovery offer");
        let (_, RecoveryScanDisposition::Offer(offer)) = entry.into_parts() else {
            panic!("expected coalesced recovery offer");
        };
        for handle in offer.into_cleanup_handles() {
            store.delete_offered_record(handle)?;
        }
        assert!(!retained_stage.exists());
        Ok(())
    }

    #[test]
    fn same_instance_with_different_documents_remains_incomparable() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let first = snapshot_with_document(1, 7, 1, 1, b"first document");
        let second = snapshot_with_document(2, 7, 2, 2, b"second document");
        fs::write(store.record_path(first.instance_id()), first.encode())?;
        fs::write(
            store.records_dir().join("same-instance-other-document.rec"),
            second.encode(),
        )?;

        let scan = store.scan_startup()?;

        assert_eq!(
            scan.iter()
                .filter(|entry| matches!(entry.disposition(), RecoveryScanDisposition::Offer(_)))
                .count(),
            2
        );
        Ok(())
    }

    #[test]
    fn same_instance_equal_revision_with_different_content_remains_incomparable() -> io::Result<()>
    {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let first = snapshot_at(7, 3, 3, b"first branch");
        let second = snapshot_at(7, 3, 3, b"second branch");
        fs::write(store.record_path(first.instance_id()), first.encode())?;
        fs::write(
            store.records_dir().join("same-revision-other-content.rec"),
            second.encode(),
        )?;

        let scan = store.scan_startup()?;

        assert_eq!(
            scan.iter()
                .filter(|entry| matches!(entry.disposition(), RecoveryScanDisposition::Offer(_)))
                .count(),
            2
        );
        Ok(())
    }

    #[test]
    fn startup_coalesces_restore_successors_by_document_lineage() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let predecessor = snapshot_at(3, 20, 9_999, b"same recovered text");
        let successor = RecoverySnapshot::try_new_with_lineage(
            RecoverySnapshotParts {
                document_id: predecessor.document_id(),
                instance_id: RecoveryInstanceId::new([4; 16]),
                revision: Revision::new(0),
                created_at: RecoveryWallTime::from_unix_millis(2),
                updated_at: RecoveryWallTime::from_unix_millis(1),
                original_path: b"memo.txt".to_vec(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::caret(1),
                content: b"same recovered text".to_vec(),
            },
            RecoveryLineageGeneration::new(1),
            Some(predecessor.instance_id()),
        )
        .expect("causal successor");
        store.persist(&predecessor)?;
        store.persist(&successor)?;

        let entries = store.scan_startup()?;
        let offers: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) => Some(offer),
                RecoveryScanDisposition::Quarantine(_) => None,
            })
            .collect();
        assert_eq!(offers.len(), 1);
        assert_eq!(offers[0].metadata().instance_id(), successor.instance_id());
        assert_eq!(offers[0].superseded().len(), 1);
        Ok(())
    }

    #[test]
    fn incomparable_v2_and_legacy_branches_remain_separate_offers() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let first = snapshot_with_document(12, 1, 5, 9_999, b"first branch");
        let second = snapshot_with_document(12, 2, 1, 1, b"second branch");
        store.persist(&first)?;
        store.persist(&second)?;

        let scan = store.scan_startup()?;
        let v2_offers: Vec<_> = scan
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) => Some(offer),
                RecoveryScanDisposition::Quarantine(_) => None,
            })
            .collect();
        assert_eq!(v2_offers.len(), 2);
        assert!(v2_offers.iter().all(|offer| offer.superseded().is_empty()));
        drop(scan);

        let legacy_one = snapshot_with_document(13, 3, 5, 9_999, b"legacy one");
        let legacy_two = snapshot_with_document(13, 4, 1, 1, b"legacy two");
        fs::write(
            store.records_dir().join("legacy-one.rec"),
            encode_v1(&legacy_one),
        )?;
        fs::write(
            store.records_dir().join("legacy-two.rec"),
            encode_v1(&legacy_two),
        )?;
        let scan = store.scan_startup()?;
        let legacy_offers = scan
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) if offer.metadata().schema_version() == 1 => {
                    Some(offer)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(legacy_offers.len(), 2);
        assert!(
            legacy_offers
                .iter()
                .all(|offer| offer.metadata().lineage_generation().is_none())
        );
        Ok(())
    }

    #[test]
    fn legacy_metadata_cannot_suppress_a_current_recovery_record() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let genuine = snapshot_with_document(15, 15, 5, 5, b"genuine newest bytes");
        store.persist(&genuine)?;
        let legacy = snapshot_with_document(15, 15, 6, 6, b"legacy alternate bytes");
        fs::write(
            store.records_dir().join("legacy-alternate.rec"),
            encode_v1(&legacy),
        )?;

        let scan = store.scan_startup()?;
        let offers = scan
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) => Some(offer),
                RecoveryScanDisposition::Quarantine(_) => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(offers.len(), 2);
        assert!(offers.iter().all(|offer| offer.superseded().is_empty()));
        let mut schemas = offers
            .iter()
            .map(|offer| offer.metadata().schema_version())
            .collect::<Vec<_>>();
        schemas.sort_unstable();
        assert_eq!(schemas, [1, RECOVERY_SCHEMA_VERSION]);
        Ok(())
    }

    #[test]
    fn exact_handle_rejects_path_replacement_for_load_and_delete() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let original = sample_snapshot(14, b"original bytes");
        store.persist(&original)?;
        let scan = store.scan_startup()?;
        assert_eq!(load_offer(&store, &scan[0])?.content(), b"original bytes");

        let replacement = snapshot_with_document(14, 14, 99, 99, b"replacement bytes");
        store.persist(&replacement)?;
        let error = load_offer(&store, &scan[0]).expect_err("replacement must invalidate handle");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);

        let entry = scan.into_entries().pop().expect("one offer");
        let (_, RecoveryScanDisposition::Offer(offer)) = entry.into_parts() else {
            panic!("expected offer");
        };
        let handle = offer.into_cleanup_handles().pop().expect("primary handle");
        assert!(store.delete_offered_record(handle).is_err());
        let encoded = fs::read(store.record_path(replacement.instance_id()))?;
        let RecoveryStartupDisposition::Offer(record) = validate_recovery_record(&encoded) else {
            panic!("replacement must remain valid");
        };
        assert_eq!(record.content(), b"replacement bytes");
        Ok(())
    }

    #[test]
    fn exact_quarantine_refuses_a_replaced_pathname() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let path = store.records_dir().join("damaged.rec");
        let damaged = b"not a recovery record";
        fs::write(&path, damaged)?;
        let opened = open_recovery_candidate(&path).expect("bind damaged fixture");

        fs::remove_file(&path)?;
        let replacement = sample_snapshot(42, b"valid replacement").encode();
        fs::write(&path, &replacement)?;

        assert!(quarantine_bound_file(&store, &path, &opened, damaged).is_err());
        assert_eq!(fs::read(&path)?, replacement);
        assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());
        Ok(())
    }

    #[test]
    fn pathname_and_header_instance_mismatch_is_retained_without_offer() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let encoded = sample_snapshot(45, b"identity mismatch").encode();
        let mismatched_path = store.record_path(RecoveryInstanceId::new([46; 16]));
        fs::write(&mismatched_path, &encoded)?;

        let scan = store.scan_startup()?;

        assert_eq!(scan.len(), 1);
        assert!(matches!(
            scan[0].disposition(),
            RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::InstanceMismatch)
        ));
        assert!(scan[0].remains_in_records());
        assert!(scan[0].quarantine_error().is_some());
        assert_eq!(fs::read(&mismatched_path)?, encoded);
        assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());

        let error = store
            .quarantine_file(&mismatched_path)
            .expect_err("public quarantine must reject disagreeing identities");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&mismatched_path)?, encoded);
        assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());
        Ok(())
    }

    #[test]
    fn live_identity_defers_mismatch_notice_without_moving_the_artifact() -> io::Result<()> {
        for live_path_identity in [true, false] {
            let dir = tempdir()?;
            let store = RecoveryStore::open(dir.path())?;
            let path_instance = indexed_instance(47);
            let snapshot = sample_snapshot(48, b"deferred identity mismatch");
            let mismatched_path = store.record_path(path_instance);
            let encoded = snapshot.encode();
            fs::write(&mismatched_path, &encoded)?;
            let live_instance = if live_path_identity {
                path_instance
            } else {
                snapshot.instance_id()
            };
            let lease = store.try_hold_live_lease(live_instance)?;

            assert!(store.scan_startup()?.is_empty());
            assert_eq!(fs::read(&mismatched_path)?, encoded);
            assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());

            store.release_live_lease(lease)?;
            let scan = store.scan_startup()?;
            assert_eq!(scan.len(), 1);
            assert!(matches!(
                scan[0].disposition(),
                RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::InstanceMismatch)
            ));
            assert_eq!(fs::read(&mismatched_path)?, encoded);
            assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());
        }
        Ok(())
    }

    #[test]
    fn startup_never_quarantines_a_live_instances_damaged_record() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(43, b"live damaged record");
        store.persist(&snapshot)?;
        let path = store.record_path(snapshot.instance_id());
        let mut damaged = fs::read(&path)?;
        let last = damaged.len().saturating_sub(1);
        damaged[last] ^= 0x55;
        fs::write(&path, &damaged)?;
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;

        assert!(store.scan_startup()?.is_empty());
        assert_eq!(fs::read(&path)?, damaged);
        assert!(fs::read_dir(store.quarantine_dir())?.next().is_none());
        drop(lease);
        Ok(())
    }

    #[test]
    fn exact_offered_delete_refuses_a_live_foreign_instance() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(15, b"live foreign work");
        store.persist(&snapshot)?;
        let scan = store.scan_startup()?;
        let entry = scan.into_entries().pop().expect("one offer");
        let (_, RecoveryScanDisposition::Offer(offer)) = entry.into_parts() else {
            panic!("expected offer");
        };
        let handle = offer.into_cleanup_handles().pop().expect("primary handle");
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;

        let error = store
            .delete_offered_record(handle)
            .expect_err("live foreign record must not be deleted");
        assert_eq!(error.kind(), io::ErrorKind::ResourceBusy);
        assert!(store.record_path(snapshot.instance_id()).exists());
        drop(lease);
        remove_file_if_present(&store.live_path(snapshot.instance_id()))?;
        Ok(())
    }

    #[test]
    fn owned_cleanup_uses_only_canonical_and_keyed_names() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(16, b"owned");
        store.persist(&snapshot)?;
        let stage = exclusive_stage_path(
            &store.records_dir(),
            snapshot.instance_id(),
            TemporaryArtifactKind::Stage,
        )?;
        let backup = exclusive_stage_path(
            &store.records_dir(),
            snapshot.instance_id(),
            TemporaryArtifactKind::Backup,
        )?;
        fs::write(&stage, b"not parsed")?;
        fs::write(&backup, b"also not parsed")?;
        let legacy_random = store.records_dir().join(".noter-recovery-legacy.tmp");
        fs::write(&legacy_random, b"unrelated invalid content")?;
        let other = exclusive_stage_path(
            &store.records_dir(),
            RecoveryInstanceId::new([17; 16]),
            TemporaryArtifactKind::Stage,
        )?;
        fs::write(&other, b"foreign invalid content")?;

        store.delete_owned_artifacts(snapshot.instance_id())?;

        assert!(!store.record_path(snapshot.instance_id()).exists());
        assert!(!stage.exists());
        assert!(!backup.exists());
        assert!(legacy_random.exists());
        assert!(other.exists());
        Ok(())
    }

    #[test]
    fn owned_cleanup_bounds_unrelated_directory_work() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(18, b"owned bounded cleanup");
        store.persist(&snapshot)?;
        for index in 0..=MAX_OWNED_RECOVERY_CLEANUP_FILES {
            fs::write(
                store
                    .records_dir()
                    .join(format!("unrelated-{index:04}.tmp")),
                b"not recovery content",
            )?;
        }

        let error = store
            .delete_owned_artifacts(snapshot.instance_id())
            .expect_err("surplus entries must surface incomplete cleanup");

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert!(!store.record_path(snapshot.instance_id()).exists());
        assert!(store.records_dir().join("unrelated-0256.tmp").exists());
        Ok(())
    }

    #[test]
    fn a_held_live_lease_hides_the_instance_from_startup_scan() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(9, b"still running");
        store.persist(&snapshot)?;
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;

        assert!(store.instance_is_live(snapshot.instance_id())?);
        assert!(store.scan_startup()?.is_empty());

        drop(lease);
        assert!(!store.instance_is_live(snapshot.instance_id())?);
        let entries = store.scan_startup()?;
        assert_eq!(entries.len(), 1);
        assert_eq!(load_offer(&store, &entries[0])?.content(), b"still running");
        Ok(())
    }

    #[test]
    fn hard_linked_live_markers_fail_closed() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let instance_id = indexed_instance(26);
        let live_path = store.live_path(instance_id);
        fs::write(&live_path, b"")?;
        let alias = store.root().join("live-marker-alias");
        fs::hard_link(&live_path, &alias)?;

        let acquire = store
            .try_hold_live_lease(instance_id)
            .expect_err("a multiply linked live marker is not private");
        assert_eq!(acquire.kind(), io::ErrorKind::InvalidData);
        let probe = store
            .instance_is_live(instance_id)
            .expect_err("a multiply linked live marker is indeterminate");
        assert_eq!(probe.kind(), io::ErrorKind::InvalidData);
        assert!(live_path.exists());
        assert!(alias.exists());
        Ok(())
    }

    #[test]
    fn releasing_a_live_lease_removes_both_fact_bound_paths() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let instance_id = indexed_instance(20);
        let lease = store.try_hold_live_lease(instance_id)?;

        assert!(store.live_path(instance_id).exists());
        assert!(store.live_guard_path(instance_id).exists());
        store.release_live_lease(lease)?;

        assert!(!store.live_path(instance_id).exists());
        assert!(!store.live_guard_path(instance_id).exists());
        Ok(())
    }

    #[test]
    fn stale_marker_cleanup_claims_and_removes_both_paths() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let instance_id = indexed_instance(21);
        fs::write(store.live_path(instance_id), b"")?;
        fs::write(store.live_guard_path(instance_id), b"")?;

        store.cleanup_stale_live_marker(instance_id);

        assert!(!store.live_path(instance_id).exists());
        assert!(!store.live_guard_path(instance_id).exists());
        Ok(())
    }

    #[test]
    fn releasing_an_offered_claim_removes_its_exclusive_lease() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(23, b"claim release");
        store.persist(&snapshot)?;
        let scan = store.scan_startup()?;
        let handle = offer(&scan[0]).primary();
        let claim = store.claim_offered_record(handle)?;
        assert!(store.live_path(snapshot.instance_id()).exists());
        assert!(store.live_guard_path(snapshot.instance_id()).exists());

        store.release_claim(claim)?;

        assert!(!store.live_path(snapshot.instance_id()).exists());
        assert!(!store.live_guard_path(snapshot.instance_id()).exists());
        Ok(())
    }

    #[test]
    fn claimed_load_rejects_a_different_instance_handle() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let first = sample_snapshot(24, b"first claim");
        let second = sample_snapshot(25, b"second claim");
        store.persist(&first)?;
        store.persist(&second)?;
        let scan = store.scan_startup()?;
        let first_handle = scan
            .iter()
            .find_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(candidate)
                    if candidate.metadata().instance_id() == first.instance_id() =>
                {
                    Some(candidate.primary())
                }
                _ => None,
            })
            .expect("first handle");
        let second_handle = scan
            .iter()
            .find_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(candidate)
                    if candidate.metadata().instance_id() == second.instance_id() =>
                {
                    Some(candidate.primary())
                }
                _ => None,
            })
            .expect("second handle");
        let claim = store.claim_offered_record(second_handle)?;

        let error = store
            .load_claimed_record(first_handle, &claim)
            .expect_err("a claim must be bound to one recovery instance");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        store.release_claim(claim)?;
        Ok(())
    }

    #[test]
    fn one_rebound_live_path_cannot_hide_the_independent_guard() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(19, b"active guarded recovery");
        store.persist(&snapshot)?;
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;
        let displaced = store.root().join("displaced-live-object");
        fs::rename(store.live_path(snapshot.instance_id()), &displaced)?;

        assert!(store.instance_is_live(snapshot.instance_id())?);
        assert_eq!(
            store
                .try_hold_live_lease(snapshot.instance_id())
                .expect_err("the locked guard must prevent a competing lease")
                .kind(),
            io::ErrorKind::ResourceBusy
        );
        assert!(
            store
                .scan_startup()?
                .iter()
                .all(|entry| !matches!(entry.disposition(), RecoveryScanDisposition::Offer(_)))
        );

        let release_error = store
            .release_live_lease(lease)
            .expect_err("a rebound lease pathname must fail exact release");
        assert_eq!(release_error.kind(), io::ErrorKind::InvalidData);
        assert!(!store.live_guard_path(snapshot.instance_id()).exists());
        remove_file_if_present(&displaced)?;
        Ok(())
    }

    #[test]
    fn unix_live_cleanup_accepts_an_already_unlinked_path() -> io::Result<()> {
        let dir = tempdir()?;
        let path = dir.path().join("claimed.live");
        let locked = acquire_live_file(&path)?;
        fs::remove_file(&path)?;

        delete_claimed_live_path_unix_fallback(&path, &locked.file, locked.facts)?;

        assert!(!path.exists());
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
            RecoveryScanDisposition::Quarantine(
                RecoveryQuarantineReason::Truncated | RecoveryQuarantineReason::InvalidMagic
            )
        ));
        assert!(!path.exists());
        assert!(!entries[0].remains_in_records());
        assert!(entries[0].quarantine_error().is_none());
        let quarantined = fs::read_dir(store.quarantine_dir())?
            .next()
            .transpose()?
            .expect("quarantined exact bytes");
        assert_eq!(fs::read(quarantined.path())?, b"not a recovery record");
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
    fn public_quarantine_reports_a_retained_source_as_failure() {
        let destination = PathBuf::from("verified-quarantine-copy.rec");
        let cleanup_error = io::Error::new(
            io::ErrorKind::PermissionDenied,
            "bound source could not be removed",
        );
        let error = finish_public_quarantine(Ok((destination, Some(cleanup_error))), Ok(()))
            .expect_err("a retained source is not a completed move");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
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
            RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::ChecksumMismatch)
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
            RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::UnknownSchema)
        ));
        Ok(())
    }

    #[test]
    fn resource_ceilings_are_exact() {
        assert_eq!(MAX_STARTUP_RECOVERY_OFFERS, 32);
        assert_eq!(MAX_STARTUP_RECOVERY_FILES, 256);
        assert_eq!(MAX_STARTUP_RECOVERY_DIRECTORY_ENTRIES, 1024);
        assert_eq!(MAX_STARTUP_QUARANTINE_RESULTS, 32);
        assert_eq!(MAX_SUPERSEDED_RECOVERY_HANDLES, 16);
        assert_eq!(MAX_STARTUP_RECOVERY_BYTES, 128 * 1024 * 1024);
        assert_eq!(MAX_RECOVERY_FILE_BYTES, 64 * 1024 * 1024 + 256 * 1024);
        assert_eq!(MAX_RECOVERY_FILE_BYTES, 67_371_008);
        assert_eq!(
            advance_scan_byte_budget(MAX_STARTUP_RECOVERY_BYTES - 1, 1),
            Some(MAX_STARTUP_RECOVERY_BYTES)
        );
        assert_eq!(
            advance_scan_byte_budget(MAX_STARTUP_RECOVERY_BYTES, 1),
            None
        );
    }

    #[test]
    fn live_records_do_not_consume_the_eligible_candidate_budget() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let mut leases = Vec::with_capacity(MAX_STARTUP_RECOVERY_FILES);
        for index in 1..=MAX_STARTUP_RECOVERY_FILES {
            let instance_id = indexed_instance(index);
            let snapshot = RecoverySnapshot::try_new(RecoverySnapshotParts {
                document_id: RecoveryDocumentId::new(instance_id.as_bytes()),
                instance_id,
                revision: Revision::new(1),
                created_at: RecoveryWallTime::from_unix_millis(1),
                updated_at: RecoveryWallTime::from_unix_millis(2),
                original_path: Vec::new(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::caret(0),
                content: b"live noise".to_vec(),
            })
            .expect("live snapshot");
            store.persist(&snapshot)?;
            leases.push(store.try_hold_live_lease(instance_id)?);
        }
        let dead = sample_snapshot(0, b"dead canonical recovery");
        store.persist(&dead)?;

        let scan = store.scan_startup()?;
        assert_eq!(scan.len(), 1);
        assert_eq!(offer(&scan[0]).metadata().instance_id(), dead.instance_id());
        drop(leases);
        Ok(())
    }

    #[test]
    fn repeated_bounded_scans_progress_past_stale_live_noise() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        for index in 1..=MAX_STARTUP_RECOVERY_DIRECTORY_ENTRIES + 1 {
            fs::write(store.live_path(indexed_instance(index)), b"")?;
        }
        let dead = sample_snapshot(0, b"recovery behind stale live noise");
        store.persist(&dead)?;

        let first = store.scan_startup()?;
        let found_first = first.iter().any(|entry| {
            matches!(entry.disposition(), RecoveryScanDisposition::Offer(candidate) if candidate.metadata().instance_id() == dead.instance_id())
        });
        if !found_first {
            assert!(first.directory_limit_reached());
            let second = store.scan_startup()?;
            assert!(second.iter().any(|entry| {
                matches!(entry.disposition(), RecoveryScanDisposition::Offer(candidate) if candidate.metadata().instance_id() == dead.instance_id())
            }));
        }
        Ok(())
    }

    #[test]
    fn startup_file_and_quarantine_result_bounds_are_surfaced() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        for index in 0..=MAX_STARTUP_RECOVERY_FILES {
            fs::write(
                store.records_dir().join(format!("broken-{index:04}.rec")),
                b"broken",
            )?;
        }

        let scan = store.scan_startup()?;

        assert!(scan.limit_reached());
        assert!(scan.has_omissions());
        assert_eq!(scan.len(), MAX_STARTUP_QUARANTINE_RESULTS);
        assert_eq!(
            scan.quarantine_results_omitted(),
            MAX_STARTUP_RECOVERY_FILES - MAX_STARTUP_QUARANTINE_RESULTS
        );
        assert_eq!(fs::read_dir(store.records_dir())?.count(), 1);
        Ok(())
    }

    #[test]
    fn superseded_handle_bound_is_surfaced_and_primary_is_cleanup_last() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let instance = 18;
        for revision in 0..=MAX_SUPERSEDED_RECOVERY_HANDLES + 1 {
            let snapshot = snapshot_at(
                instance,
                u64::try_from(revision).expect("fixture revision"),
                1,
                format!("revision-{revision}").as_bytes(),
            );
            let path = if revision == MAX_SUPERSEDED_RECOVERY_HANDLES + 1 {
                store.record_path(snapshot.instance_id())
            } else {
                store
                    .records_dir()
                    .join(format!("duplicate-{revision:02}.tmp"))
            };
            fs::write(path, snapshot.encode())?;
        }

        let scan = store.scan_startup()?;
        assert!(scan.superseded_handles_omitted());
        assert!(scan.has_omissions());
        let entry = scan.into_entries().pop().expect("one offer");
        let (_, RecoveryScanDisposition::Offer(offer)) = entry.into_parts() else {
            panic!("expected offer");
        };
        assert_eq!(offer.superseded().len(), MAX_SUPERSEDED_RECOVERY_HANDLES);
        let handles = offer.into_cleanup_handles();
        assert_eq!(
            handles.last().expect("primary last").metadata().revision(),
            Revision::new(
                u64::try_from(MAX_SUPERSEDED_RECOVERY_HANDLES + 1).expect("fixture revision")
            )
        );
        Ok(())
    }

    #[test]
    fn delete_record_rejects_non_missing_failures() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let id = RecoveryInstanceId::new([42; 16]);
        let path = store.record_path(id);
        fs::create_dir(&path)?;
        let error = store
            .delete_record(id)
            .expect_err("directory at record path is not a successful missing delete");
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert!(path.exists());
        Ok(())
    }

    #[test]
    fn live_probe_errors_fail_closed() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(43, b"protected recovery");
        store.persist(&snapshot)?;
        fs::create_dir(store.live_path(snapshot.instance_id()))?;

        assert!(store.instance_is_live(snapshot.instance_id()).is_err());
        assert!(store.scan_startup().is_err());
        assert!(store.record_path(snapshot.instance_id()).exists());
        Ok(())
    }

    #[test]
    fn record_cleanup_preserves_the_live_lease() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let snapshot = sample_snapshot(44, b"clean now");
        store.persist(&snapshot)?;
        let lease = store.try_hold_live_lease(snapshot.instance_id())?;

        store.delete_record(snapshot.instance_id())?;

        assert!(!store.record_path(snapshot.instance_id()).exists());
        assert!(store.live_path(snapshot.instance_id()).exists());
        assert!(store.instance_is_live(snapshot.instance_id())?);
        drop(lease);
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
            RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::ContentTooLarge)
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
                RecoveryScanDisposition::Quarantine(RecoveryQuarantineReason::ContentTooLarge)
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
        store.delete_owned_artifacts(indexed_instance(27))?;
        Ok(())
    }

    #[test]
    fn owned_cleanup_rejects_a_non_directory_records_path() -> io::Result<()> {
        let dir = tempdir()?;
        let store = RecoveryStore::open(dir.path())?;
        let records = store.records_dir();
        fs::remove_dir_all(&records)?;
        fs::write(&records, b"not a directory")?;

        let error = store
            .delete_owned_artifacts(indexed_instance(28))
            .expect_err("a non-directory records path is not missing");
        assert_ne!(error.kind(), io::ErrorKind::NotFound);
        assert!(records.is_file());
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
                document_id: RecoveryDocumentId::new(id),
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
            .filter(|entry| matches!(entry.disposition(), RecoveryScanDisposition::Offer(_)))
            .count();
        assert_eq!(offers, MAX_STARTUP_RECOVERY_OFFERS);
        assert!(entries.offers_omitted());

        let remaining = fs::read_dir(store.records_dir())?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_file())
            .count();
        assert_eq!(remaining, MAX_STARTUP_RECOVERY_OFFERS + 1);
        Ok(())
    }
}
