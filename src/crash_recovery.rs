//! Application adapter for private crash-recovery records.
//!
//! Pure scheduling and on-disk record integrity live in `noter::core::recovery`
//! and `noter::core::recovery_store`. This module owns process identity, wall
//! and monotonic clocks, the private recovery root under the eframe state
//! directory, one dedicated persist worker thread, and the small state machine
//! that surfaces startup offers and persist failures without writing a user
//! document path. Snapshot capture stays on the UI thread; durable write and
//! `fsync` run on the worker so typing is not stalled by disk.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use getrandom::fill as fill_random;
use noter::core::document::Document;
use noter::core::edit::Selection;
use noter::core::recovery::{
    RecoveryClock, RecoveryDocumentId, RecoveryInstanceId, RecoveryLineageGeneration,
    RecoveryOfferDecision, RecoveryOfferState, RecoveryScheduleCommand, RecoveryScheduleEffect,
    RecoveryScheduleState, RecoverySnapshot, RecoverySnapshotParts, RecoveryWallTime,
    ValidatedRecoveryMetadata,
};
use noter::core::recovery_store::{
    RecoveryInstanceClaim, RecoveryOffer, RecoveryScanDisposition, RecoveryStartupScan,
    RecoveryStore,
};
use noter::core::revision::Revision;

/// Subdirectory of the eframe state root used for crash-recovery files.
pub const RECOVERY_STATE_SUBDIR: &str = "recovery";

/// Application id shared with eframe persistence (`app.ron`).
pub const NOTER_APP_ID: &str = "Noter";

#[cfg(feature = "screenshot-qa")]
const SCREENSHOT_QA_STATE_DIRECTORY_ENV: &str = "NOTER_SCREENSHOT_QA_STATE_DIRECTORY";

/// User-visible message when a recovery write fails. No paths or content.
pub const RECOVERY_PERSIST_FAILURE_MESSAGE: &str = "Noter could not update its private recovery copy of this document. Your text is still in this window. Keep the window open and save when you can.";

/// User-visible message when an obsolete private recovery copy cannot be removed.
pub const RECOVERY_CLEANUP_FAILURE_MESSAGE: &str = "Noter could not remove an obsolete private recovery copy. Your document was not changed by this cleanup failure. Keep backups and review any recovery offer after restarting.";

/// User-visible message when the recovery store cannot open.
pub const RECOVERY_UNAVAILABLE_MESSAGE: &str = "Private crash recovery is unavailable in this session because Noter could not safely open or own its private recovery storage. Saves still work; keep backups of important work.";

/// User-visible message when bounded startup review leaves records untouched.
pub const RECOVERY_SCAN_INCOMPLETE_MESSAGE: &str = "Noter stopped its bounded startup recovery review before checking every private record. Unreviewed records remain unchanged for a later launch.";

const RECOVERY_RESTORE_RETAINED_MESSAGE: &str = "Noter could not safely transfer this recovery copy to the current window. The private recovery copy was kept. Choose Later, keep backups, and retry after checking the recovery storage.";

/// Maximum length of a quarantine notice shown at startup.
const MAX_QUARANTINE_NOTICE_BYTES: usize = 240;

struct PersistJob {
    store: RecoveryStore,
    snapshot: RecoverySnapshot,
    revision: Revision,
    epoch: u64,
}

struct PersistOutcome {
    revision: Revision,
    epoch: u64,
    succeeded: bool,
    cleanup_failed: bool,
}

struct PreparedRecoveryIdentity {
    document_id: RecoveryDocumentId,
    instance_id: RecoveryInstanceId,
    created_wall: RecoveryWallTime,
    live_lease: File,
}

/// Resolves the per-user state directory used for preferences and recovery.
///
/// Matches eframe's `storage_dir("Noter")` layout documented in INSTALLATION.md.
#[must_use]
pub fn noter_state_directory() -> Option<PathBuf> {
    #[cfg(feature = "screenshot-qa")]
    if let Some(path) = std::env::var_os(SCREENSHOT_QA_STATE_DIRECTORY_ENV) {
        return Some(PathBuf::from(path));
    }
    eframe::storage_dir(NOTER_APP_ID)
}

/// Returns the private recovery root under the state directory.
#[must_use]
pub fn recovery_root_from_state(state_dir: impl AsRef<Path>) -> PathBuf {
    state_dir.as_ref().join(RECOVERY_STATE_SUBDIR)
}

/// One startup recovery candidate ready for an explicit user decision.
#[derive(Debug)]
pub struct StartupRecoveryOffer {
    artifact: RecoveryOffer,
    offer: RecoveryOfferState,
}

impl StartupRecoveryOffer {
    /// Returns validated metadata without loading document content.
    pub const fn metadata(&self) -> &ValidatedRecoveryMetadata {
        self.artifact.metadata()
    }

    /// Returns whether the offer is still open.
    pub const fn is_open(&self) -> bool {
        self.offer.is_open()
    }

    /// Short label for UI without full paths when the original path is non-UTF-8.
    pub fn original_path_label(&self) -> String {
        let bytes = self.metadata().original_path();
        if bytes.is_empty() {
            return "Untitled".to_owned();
        }
        std::str::from_utf8(bytes).map_or_else(
            |_| "Recovered document (non-UTF-8 path)".to_owned(),
            |text| {
                let name = Path::new(text)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(text);
                truncate_for_ui(name, 96)
            },
        )
    }
}

/// Process-owned recovery session for one editor window.
pub struct CrashRecoverySession {
    store: Option<RecoveryStore>,
    schedule: RecoveryScheduleState,
    document_id: RecoveryDocumentId,
    instance_id: RecoveryInstanceId,
    session_started: Instant,
    created_wall: RecoveryWallTime,
    lineage_generation: RecoveryLineageGeneration,
    predecessor_instance: Option<RecoveryInstanceId>,
    startup_offers: Vec<StartupRecoveryOffer>,
    active_offer_index: Option<usize>,
    quarantine_notices: Vec<String>,
    persist_failure: bool,
    cleanup_failure: bool,
    unavailable: bool,
    persist_jobs: Option<Sender<PersistJob>>,
    persist_outcomes: Option<Receiver<PersistOutcome>>,
    persist_worker: Option<JoinHandle<()>>,
    persist_epoch_gate: Arc<AtomicU64>,
    live_lease: Option<File>,
}

impl CrashRecoverySession {
    /// Opens the private recovery store under the platform state directory and
    /// scans startup records.
    ///
    /// When the state directory or store cannot open, the session stays usable
    /// but does not persist recovery files (`unavailable` is true).
    pub fn open_default() -> Self {
        noter_state_directory().map_or_else(Self::unavailable, |state| {
            Self::open_at(recovery_root_from_state(state))
        })
    }

    /// Opens a recovery session under an explicit recovery root (tests).
    pub fn open_at(recovery_root: impl Into<PathBuf>) -> Self {
        let mut session = Self::blank();
        if session.unavailable {
            // Identity construction failed; never attach a store with weak IDs.
            return session;
        }
        match RecoveryStore::open(recovery_root) {
            Ok(store) => {
                session.attach_persist_worker();
                session.store = Some(store);
                session.acquire_live_lease();
                if !session.unavailable {
                    session.ingest_scan();
                }
            }
            Err(_) => {
                session.unavailable = true;
            }
        }
        session
    }

    fn attach_persist_worker(&mut self) {
        let (job_tx, job_rx) = mpsc::channel::<PersistJob>();
        let (outcome_tx, outcome_rx) = mpsc::channel::<PersistOutcome>();
        let epoch_gate = Arc::clone(&self.persist_epoch_gate);
        if let Ok(handle) = thread::Builder::new()
            .name("noter-recovery".to_owned())
            .spawn(move || persist_worker_loop(job_rx, outcome_tx, epoch_gate))
        {
            self.persist_jobs = Some(job_tx);
            self.persist_outcomes = Some(outcome_rx);
            self.persist_worker = Some(handle);
        } else {
            // Persist stays inline if the process cannot create a worker.
            self.persist_jobs = None;
            self.persist_outcomes = None;
            self.persist_worker = None;
        }
    }

    fn unavailable() -> Self {
        let mut session = Self::blank();
        session.unavailable = true;
        session
    }

    fn blank() -> Self {
        let (document_id, instance_id, unavailable) =
            match (random_document_id(), random_instance_id()) {
                (Ok(document_id), Ok(instance_id)) => (document_id, instance_id, false),
                _ => (
                    RecoveryDocumentId::new([0; 16]),
                    RecoveryInstanceId::new([0; 16]),
                    true,
                ),
            };
        Self {
            store: None,
            schedule: RecoveryScheduleState::default(),
            document_id,
            instance_id,
            session_started: Instant::now(),
            created_wall: wall_now(),
            lineage_generation: RecoveryLineageGeneration::ROOT,
            predecessor_instance: None,
            startup_offers: Vec::new(),
            active_offer_index: None,
            quarantine_notices: Vec::new(),
            persist_failure: false,
            cleanup_failure: false,
            unavailable,
            persist_jobs: None,
            persist_outcomes: None,
            persist_worker: None,
            persist_epoch_gate: Arc::new(AtomicU64::new(0)),
            live_lease: None,
        }
    }

    fn ingest_scan(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let Ok(scan) = store.scan_startup() else {
            self.unavailable = true;
            return;
        };
        self.apply_scan_entries(scan);
    }

    fn apply_scan_entries(&mut self, scan: RecoveryStartupScan) {
        self.startup_offers.clear();
        self.quarantine_notices.clear();
        self.active_offer_index = None;
        let scan_incomplete = scan.has_omissions();
        let quarantine_results_omitted = scan.quarantine_results_omitted();
        for entry in scan {
            let quarantine_error = entry.quarantine_error().map(str::to_owned);
            let (_, disposition) = entry.into_parts();
            match disposition {
                RecoveryScanDisposition::Offer(artifact) => {
                    let mut offer = RecoveryOfferState::default();
                    offer.present();
                    self.startup_offers
                        .push(StartupRecoveryOffer { artifact, offer });
                }
                RecoveryScanDisposition::Quarantine(reason) => {
                    let message = quarantine_error.as_deref().map_or_else(
                        || reason.description().to_owned(),
                        |error| {
                            truncate_for_ui(
                                &format!("{} {}", reason.description(), truncate_for_ui(error, 80)),
                                MAX_QUARANTINE_NOTICE_BYTES,
                            )
                        },
                    );
                    self.quarantine_notices.push(message);
                }
            }
        }
        if scan_incomplete {
            self.quarantine_notices
                .push(RECOVERY_SCAN_INCOMPLETE_MESSAGE.to_owned());
        }
        if quarantine_results_omitted != 0 {
            self.quarantine_notices.push(format!(
                "Noter reviewed {quarantine_results_omitted} additional damaged recovery record(s) without showing individual details."
            ));
        }
        if !self.startup_offers.is_empty() {
            self.active_offer_index = Some(0);
        }
    }

    /// Returns whether private recovery storage is unavailable this session.
    pub const fn is_unavailable(&self) -> bool {
        self.unavailable
    }

    /// Returns whether the last recovery persist failed and still needs review.
    pub const fn has_persist_failure(&self) -> bool {
        self.persist_failure
    }

    /// Returns whether an authorized recovery cleanup could not complete.
    pub const fn has_cleanup_failure(&self) -> bool {
        self.cleanup_failure
    }

    /// Clears a dismissed persist-failure notice without claiming durability.
    pub const fn dismiss_persist_failure(&mut self) {
        self.persist_failure = false;
    }

    /// Hides the cleanup warning without claiming that deletion succeeded.
    pub const fn dismiss_cleanup_failure(&mut self) {
        self.cleanup_failure = false;
    }

    /// Returns quarantine notices collected at the last scan.
    pub fn quarantine_notices(&self) -> &[String] {
        &self.quarantine_notices
    }

    /// Clears all quarantine notices after the user acknowledges them.
    pub fn clear_quarantine_notices(&mut self) {
        self.quarantine_notices.clear();
    }

    /// Returns the currently open startup offer, if any.
    pub fn active_offer(&self) -> Option<&StartupRecoveryOffer> {
        self.active_offer_index
            .and_then(|index| self.startup_offers.get(index))
            .filter(|offer| offer.is_open())
    }

    /// Restores the active offer into an in-memory dirty document and selection.
    ///
    /// The offered instance record is removed only after the same bytes have
    /// been durably written under a newly leased successor identity.
    ///
    /// # Errors
    ///
    /// Returns an error when there is no open offer, content cannot become a
    /// document, or the durable successor transfer cannot complete. Transfer
    /// failure keeps the offered record open.
    pub fn restore_active_offer(&mut self) -> Result<(Document, Selection), String> {
        let index = self
            .active_offer_index
            .ok_or_else(|| "No recovery offer is open.".to_owned())?;
        let Some(store) = self.store.clone() else {
            self.mark_unavailable_for_identity_failure();
            return Err(RECOVERY_RESTORE_RETAINED_MESSAGE.to_owned());
        };
        let (record, claim) = {
            let offer = self
                .startup_offers
                .get(index)
                .filter(|offer| offer.is_open())
                .ok_or_else(|| "No recovery offer is open.".to_owned())?;
            let claim = store
                .claim_offered_record(offer.artifact.primary())
                .map_err(|_| RECOVERY_RESTORE_RETAINED_MESSAGE.to_owned())?;
            let Ok(record) = store.load_claimed_record(offer.artifact.primary(), &claim) else {
                self.cleanup_failure |= store.release_claim(claim).is_err();
                return Err(RECOVERY_RESTORE_RETAINED_MESSAGE.to_owned());
            };
            (record, claim)
        };
        let selection = record.selection();

        let mut document = match Document::from_bytes(record.content()) {
            Ok(document) => document,
            Err(error) => {
                self.cleanup_failure |= store.release_claim(claim).is_err();
                return Err(format!("Recovered content could not be opened: {error}"));
            }
        };
        document.mark_recovered_dirty();
        debug_assert!(document.is_dirty());

        let Ok(prepared) = self.prepare_fresh_identity(record.document_id()) else {
            self.cleanup_failure |= store.release_claim(claim).is_err();
            self.mark_unavailable_for_identity_failure();
            return Err(RECOVERY_RESTORE_RETAINED_MESSAGE.to_owned());
        };
        let Ok(replacement_snapshot) = RecoverySnapshot::try_new_successor(
            RecoverySnapshotParts {
                document_id: prepared.document_id,
                instance_id: prepared.instance_id,
                revision: document.revision(),
                created_at: prepared.created_wall,
                updated_at: wall_now(),
                original_path: record.original_path().to_vec(),
                bom: record.bom(),
                encoding: record.encoding(),
                selection,
                content: record.content().to_vec(),
            },
            record.metadata(),
        ) else {
            self.release_prepared_identity(prepared);
            self.cleanup_failure |= store.release_claim(claim).is_err();
            return Err(RECOVERY_RESTORE_RETAINED_MESSAGE.to_owned());
        };
        if store.persist(&replacement_snapshot).is_err() {
            self.release_prepared_identity(prepared);
            self.cleanup_failure |= store.release_claim(claim).is_err();
            return Err(RECOVERY_RESTORE_RETAINED_MESSAGE.to_owned());
        }

        if let Some(offer) = self.startup_offers.get_mut(index) {
            let _ = offer.offer.decide(RecoveryOfferDecision::Restore);
        }
        let lineage_generation = replacement_snapshot.lineage_generation();
        let predecessor_instance = replacement_snapshot.predecessor_instance();
        let restored_offer = self
            .take_offer_slot(index)
            .expect("the active recovery offer must remain present");
        self.commit_fresh_identity(prepared, lineage_generation, predecessor_instance);
        let cleanup = cleanup_offer_artifacts(&store, restored_offer.artifact, &claim);
        let release = store.release_claim(claim);
        self.cleanup_failure |= cleanup.is_err() || release.is_err();
        Ok((document, selection))
    }

    /// Discards the active startup offer and deletes its on-disk record.
    pub fn discard_active_offer(&mut self) -> bool {
        let Some(index) = self.active_offer_index else {
            return false;
        };
        let Some(offer) = self.startup_offers.get(index) else {
            return false;
        };
        if !offer.is_open() {
            return false;
        }
        let Some(store) = self.store.clone() else {
            self.cleanup_failure = true;
            return false;
        };
        let Ok(claim) = store.claim_offered_record(offer.artifact.primary()) else {
            self.cleanup_failure = true;
            return false;
        };
        if store
            .load_claimed_record(offer.artifact.primary(), &claim)
            .is_err()
        {
            self.cleanup_failure = true;
            let _ = store.release_claim(claim);
            return false;
        }
        let discarded_offer = self
            .take_offer_slot(index)
            .expect("the active recovery offer must remain present");
        let cleanup = cleanup_offer_artifacts(&store, discarded_offer.artifact, &claim);
        let release = store.release_claim(claim);
        if cleanup.is_err() || release.is_err() {
            self.cleanup_failure = true;
            self.ingest_scan();
            return false;
        }
        true
    }

    /// Hides startup recovery offers for this session without deleting records.
    ///
    /// Explicit file opens and screenshot automation skip the interactive review
    /// so those launches stay deterministic. Discard remains an explicit choice.
    /// A later untitled launch scans the same private store and can offer restore.
    pub fn defer_startup_offers(&mut self) {
        self.startup_offers.clear();
        self.active_offer_index = None;
    }

    fn take_offer_slot(&mut self, index: usize) -> Option<StartupRecoveryOffer> {
        let removed =
            (index < self.startup_offers.len()).then(|| self.startup_offers.remove(index));
        self.active_offer_index = if self.startup_offers.is_empty() {
            None
        } else {
            Some(0)
        };
        // Present next offer if any remaining.
        if let Some(next) = self.active_offer_index
            && let Some(offer) = self.startup_offers.get_mut(next)
            && !offer.is_open()
        {
            offer.offer.present();
        }
        removed
    }

    /// Begins tracking a new editor identity after New / successful open / restore.
    ///
    /// When operating-system randomness is unavailable, recovery is marked
    /// unavailable for the rest of the session so the adapter never persists
    /// with a weak or zero identity that could collide with another session.
    pub fn begin_fresh_identity(&mut self) {
        let prepared =
            random_document_id().and_then(|document_id| self.prepare_fresh_identity(document_id));
        match prepared {
            Ok(prepared) => {
                self.commit_fresh_identity(prepared, RecoveryLineageGeneration::ROOT, None);
            }
            Err(()) => self.mark_unavailable_for_identity_failure(),
        }
    }

    fn prepare_fresh_identity(
        &self,
        document_id: RecoveryDocumentId,
    ) -> Result<PreparedRecoveryIdentity, ()> {
        if self.unavailable {
            return Err(());
        }
        let store = self.store.as_ref().ok_or(())?;
        let instance_id = random_instance_id()?;
        let live_lease = store.try_hold_live_lease(instance_id).map_err(|_| ())?;
        Ok(PreparedRecoveryIdentity {
            document_id,
            instance_id,
            created_wall: wall_now(),
            live_lease,
        })
    }

    fn commit_fresh_identity(
        &mut self,
        prepared: PreparedRecoveryIdentity,
        lineage_generation: RecoveryLineageGeneration,
        predecessor_instance: Option<RecoveryInstanceId>,
    ) {
        let previous_instance = self.instance_id;
        let previous_lease = self.live_lease.take();
        self.document_id = prepared.document_id;
        self.instance_id = prepared.instance_id;
        self.created_wall = prepared.created_wall;
        self.lineage_generation = lineage_generation;
        self.predecessor_instance = predecessor_instance;
        self.schedule.reset_for_new_identity();
        self.persist_epoch_gate
            .store(self.schedule.epoch(), Ordering::Release);
        self.persist_failure = false;
        self.unavailable = false;
        self.live_lease = Some(prepared.live_lease);
        drop(previous_lease);
        if let Some(store) = self.store.as_ref() {
            let _ = fs::remove_file(store.live_path(previous_instance));
        }
    }

    fn release_prepared_identity(&self, prepared: PreparedRecoveryIdentity) {
        let instance_id = prepared.instance_id;
        drop(prepared.live_lease);
        if let Some(store) = self.store.as_ref() {
            let _ = fs::remove_file(store.live_path(instance_id));
        }
    }

    fn acquire_live_lease(&mut self) {
        self.live_lease = None;
        let Some(store) = self.store.as_ref() else {
            return;
        };
        if let Ok(file) = store.try_hold_live_lease(self.instance_id) {
            self.live_lease = Some(file);
        } else {
            self.unavailable = true;
            self.persist_failure = false;
        }
    }

    fn release_live_lease_for(&mut self, instance_id: RecoveryInstanceId) {
        self.live_lease = None;
        if let Some(store) = self.store.as_ref() {
            let _ = fs::remove_file(store.live_path(instance_id));
        }
    }

    fn mark_unavailable_for_identity_failure(&mut self) {
        self.unavailable = true;
        self.schedule.reset_for_new_identity();
        self.persist_epoch_gate
            .store(self.schedule.epoch(), Ordering::Release);
        self.persist_failure = false;
    }

    /// Notifies the scheduler that content is dirty at the given revision.
    pub fn on_edited(&mut self, document: &Document, selection: Selection) {
        if !document.is_dirty() {
            return;
        }
        self.on_retained(document, selection);
    }

    /// Notifies the scheduler that the in-memory content must be retained.
    ///
    /// This includes ordinary dirty edits and a clean loaded revision whose
    /// trusted disk version was replaced externally. The latter must remain
    /// recoverable without changing the document's save-conflict baseline.
    pub fn on_retained(&mut self, document: &Document, selection: Selection) {
        if self.unavailable || self.store.is_none() {
            return;
        }
        if self.active_offer().is_some() {
            return;
        }
        self.poll_persist_outcomes();
        let revision = document.revision();
        let now = self.monotonic_now();
        let effect = self
            .schedule
            .reduce(RecoveryScheduleCommand::Edited { revision, now });
        self.apply_schedule_effect(effect, Some((document, selection)));
    }

    /// Polls the scheduler on the UI timer and performs due work.
    pub fn on_tick(&mut self, document: &Document, selection: Selection) {
        if self.unavailable || self.store.is_none() {
            return;
        }
        if self.active_offer().is_some() {
            return;
        }
        self.poll_persist_outcomes();
        let now = self.monotonic_now();
        let effect = self.schedule.reduce(RecoveryScheduleCommand::Tick { now });
        self.apply_schedule_effect(effect, Some((document, selection)));
    }

    /// Returns how long the interface may sleep before the next [`Self::on_tick`].
    ///
    /// The interface only draws when something asks it to, so an idle window
    /// would otherwise never reach a due persist.
    pub fn next_persist_delay(&self) -> Option<Duration> {
        if self.unavailable || self.store.is_none() || self.active_offer().is_some() {
            return None;
        }
        self.schedule.next_persist_delay(self.monotonic_now())
    }

    /// Records a successful Save and deletes the owned recovery record.
    pub fn on_saved_clean(&mut self, revision: Revision) {
        if self.unavailable || self.store.is_none() {
            return;
        }
        self.poll_persist_outcomes();
        let effect = self
            .schedule
            .reduce(RecoveryScheduleCommand::BecameClean { revision });
        self.persist_epoch_gate
            .store(self.schedule.epoch(), Ordering::Release);
        self.apply_schedule_effect(effect, None);
        self.persist_failure = false;
    }

    /// Records an explicit Discard and deletes the owned recovery record.
    pub fn on_discarded(&mut self) {
        if self.unavailable || self.store.is_none() {
            return;
        }
        self.poll_persist_outcomes();
        let effect = self.schedule.reduce(RecoveryScheduleCommand::Discarded);
        self.persist_epoch_gate
            .store(self.schedule.epoch(), Ordering::Release);
        self.apply_schedule_effect(effect, None);
        self.persist_failure = false;
        self.begin_fresh_identity();
    }

    fn apply_schedule_effect(
        &mut self,
        effect: RecoveryScheduleEffect,
        document: Option<(&Document, Selection)>,
    ) {
        match effect {
            RecoveryScheduleEffect::None => {}
            RecoveryScheduleEffect::Persist { revision, epoch } => {
                let Some((document, selection)) = document else {
                    // Tick without document context: re-queue by treating as failure.
                    let now = self.monotonic_now();
                    let _ = self
                        .schedule
                        .reduce(RecoveryScheduleCommand::PersistFailed {
                            revision,
                            epoch,
                            now,
                        });
                    return;
                };
                self.persist_snapshot(document, selection, revision, epoch);
            }
            RecoveryScheduleEffect::DeleteOwned { epoch: _ } => {
                if let Some(store) = self.store.as_ref() {
                    self.cleanup_failure |= store.delete_owned_artifacts(self.instance_id).is_err();
                }
            }
        }
    }

    fn persist_snapshot(
        &mut self,
        document: &Document,
        selection: Selection,
        revision: Revision,
        epoch: u64,
    ) {
        let now = self.monotonic_now();
        let content = document.to_bytes();
        let original_path = document.path().map(path_to_bytes).unwrap_or_default();
        let Ok(snapshot) = RecoverySnapshot::try_new_with_lineage(
            RecoverySnapshotParts {
                document_id: self.document_id,
                instance_id: self.instance_id,
                revision,
                created_at: self.created_wall,
                updated_at: wall_now(),
                original_path,
                bom: document.bom(),
                encoding: document.encoding(),
                selection,
                content,
            },
            self.lineage_generation,
            self.predecessor_instance,
        ) else {
            let _ = self
                .schedule
                .reduce(RecoveryScheduleCommand::PersistFailed {
                    revision,
                    epoch,
                    now,
                });
            self.persist_failure = true;
            return;
        };

        let Some(store) = self.store.as_ref() else {
            return;
        };
        let job = PersistJob {
            store: store.clone(),
            snapshot,
            revision,
            epoch,
        };
        if let Some(jobs) = self.persist_jobs.as_ref() {
            match jobs.send(job) {
                Ok(()) => return,
                Err(error) => {
                    self.persist_snapshot_inline(&error.0.snapshot, revision, epoch);
                    return;
                }
            }
        }
        self.persist_snapshot_inline(&job.snapshot, revision, epoch);
    }

    fn persist_snapshot_inline(
        &mut self,
        snapshot: &RecoverySnapshot,
        revision: Revision,
        epoch: u64,
    ) {
        let now = self.monotonic_now();
        let Some(store) = self.store.as_ref() else {
            return;
        };
        if store.persist(snapshot).is_ok() {
            let _ = self
                .schedule
                .reduce(RecoveryScheduleCommand::PersistAcknowledged { revision, epoch });
            self.persist_failure = false;
        } else {
            let _ = self
                .schedule
                .reduce(RecoveryScheduleCommand::PersistFailed {
                    revision,
                    epoch,
                    now,
                });
            self.persist_failure = true;
        }
    }

    fn poll_persist_outcomes(&mut self) {
        let batch = {
            let Some(outcomes) = self.persist_outcomes.as_mut() else {
                return;
            };
            let mut batch = Vec::new();
            while let Ok(outcome) = outcomes.try_recv() {
                batch.push(outcome);
            }
            batch
        };
        for outcome in batch {
            self.apply_persist_outcome(&outcome);
        }
    }

    fn apply_persist_outcome(&mut self, outcome: &PersistOutcome) {
        let now = self.monotonic_now();
        if outcome.cleanup_failed {
            self.cleanup_failure = true;
        }
        if outcome.epoch != self.schedule.epoch() {
            return;
        }
        if outcome.succeeded {
            let _ = self
                .schedule
                .reduce(RecoveryScheduleCommand::PersistAcknowledged {
                    revision: outcome.revision,
                    epoch: outcome.epoch,
                });
            self.persist_failure = false;
        } else {
            let _ = self
                .schedule
                .reduce(RecoveryScheduleCommand::PersistFailed {
                    revision: outcome.revision,
                    epoch: outcome.epoch,
                    now,
                });
            self.persist_failure = true;
        }
    }

    fn monotonic_now(&self) -> RecoveryClock {
        RecoveryClock::new(self.session_started.elapsed())
    }

    #[cfg(test)]
    fn wait_for_persist(&mut self, document: &Document, selection: Selection) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let expected = document.revision();
        loop {
            self.poll_persist_outcomes();
            if self.schedule.in_flight_revision().is_none()
                && self.schedule.last_persisted_revision() == Some(expected)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "recovery persist worker did not finish"
            );
            thread::sleep(Duration::from_millis(1));
            self.on_tick(document, selection);
        }
    }

    #[cfg(test)]
    pub(crate) fn force_due_persist_for_test(&mut self, document: &Document, selection: Selection) {
        self.session_started = self
            .session_started
            .checked_sub(Duration::from_secs(3))
            .expect("the recovery test clock must move backward");
        self.on_tick(document, selection);
        self.wait_for_persist(document, selection);
    }
}

impl Drop for CrashRecoverySession {
    fn drop(&mut self) {
        self.persist_jobs.take();
        if let Some(handle) = self.persist_worker.take() {
            let _ = handle.join();
        }
        let instance_id = self.instance_id;
        self.release_live_lease_for(instance_id);
    }
}

fn cleanup_offer_artifacts(
    store: &RecoveryStore,
    offer: RecoveryOffer,
    primary_claim: &RecoveryInstanceClaim,
) -> std::io::Result<()> {
    let primary_instance = offer.metadata().instance_id();
    for handle in offer.into_cleanup_handles() {
        if handle.metadata().instance_id() == primary_instance {
            store.delete_claimed_record(handle, primary_claim)?;
        } else {
            store.delete_offered_record(handle)?;
        }
    }
    Ok(())
}

// Channels and the epoch gate are moved onto the worker even though the loop
// only borrows them; they must not stay on the UI thread.
#[allow(clippy::needless_pass_by_value)]
fn persist_worker_loop(
    jobs: Receiver<PersistJob>,
    outcomes: Sender<PersistOutcome>,
    epoch_gate: Arc<AtomicU64>,
) {
    while let Ok(job) = jobs.recv() {
        let instance_id = job.snapshot.instance_id();
        let current_epoch = epoch_gate.load(Ordering::Acquire);
        if job.epoch != current_epoch {
            let cleanup_failed = job.store.delete_record(instance_id).is_err();
            if outcomes
                .send(PersistOutcome {
                    revision: job.revision,
                    epoch: job.epoch,
                    succeeded: false,
                    cleanup_failed,
                })
                .is_err()
            {
                break;
            }
            continue;
        }
        let succeeded = job.store.persist(&job.snapshot).is_ok();
        if epoch_gate.load(Ordering::Acquire) != job.epoch {
            let cleanup_failed = job.store.delete_record(instance_id).is_err();
            let _ = outcomes.send(PersistOutcome {
                revision: job.revision,
                epoch: job.epoch,
                succeeded: false,
                cleanup_failed,
            });
            continue;
        }
        if outcomes
            .send(PersistOutcome {
                revision: job.revision,
                epoch: job.epoch,
                succeeded,
                cleanup_failed: false,
            })
            .is_err()
        {
            break;
        }
    }
}

fn path_to_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    }
    #[cfg(windows)]
    {
        path.to_string_lossy().into_owned().into_bytes()
    }
    #[cfg(not(any(unix, windows)))]
    {
        path.to_string_lossy().into_owned().into_bytes()
    }
}

fn wall_now() -> RecoveryWallTime {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis();
    let millis = u64::try_from(millis).unwrap_or(u64::MAX);
    RecoveryWallTime::from_unix_millis(millis)
}

fn random_document_id() -> Result<RecoveryDocumentId, ()> {
    Ok(RecoveryDocumentId::new(random_id_bytes()?))
}

fn random_instance_id() -> Result<RecoveryInstanceId, ()> {
    Ok(RecoveryInstanceId::new(random_id_bytes()?))
}

fn random_id_bytes() -> Result<[u8; 16], ()> {
    let mut bytes = [0_u8; 16];
    fill_random(&mut bytes).map_err(|_| ())?;
    // All-zero should not occur after a successful CSPRNG fill; treat it as
    // failure so instance files never share a fixed weak identity.
    if bytes.iter().all(|&byte| byte == 0) {
        return Err(());
    }
    Ok(bytes)
}

fn truncate_for_ui(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let mut end = max_bytes.saturating_sub(1);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

#[cfg(test)]
mod tests {
    use super::*;
    use noter::core::recovery::{
        RecoveryDocumentId, RecoveryInstanceId, RecoverySnapshot, RecoverySnapshotParts,
        RecoveryStartupDisposition, RecoveryWallTime, validate_recovery_record,
    };
    use noter::core::revision::Revision;
    use noter::core::text_format::{Bom, Encoding};
    use tempfile::tempdir;

    fn recovery_record_count(store: &RecoveryStore) -> usize {
        std::fs::read_dir(store.records_dir())
            .expect("records dir")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "rec")
            })
            .count()
    }

    fn sample_snapshot(instance: u8, content: &[u8]) -> RecoverySnapshot {
        RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([instance; 16]),
            instance_id: RecoveryInstanceId::new([instance; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(2),
            original_path: b"notes.txt".to_vec(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: content.to_vec(),
        })
        .expect("snapshot")
    }

    fn snapshot_for(
        instance_id: RecoveryInstanceId,
        revision: Revision,
        content: &[u8],
    ) -> RecoverySnapshot {
        RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([2; 16]),
            instance_id,
            revision,
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(2),
            original_path: b"recovery-test.txt".to_vec(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: content.to_vec(),
        })
        .expect("snapshot")
    }

    #[test]
    fn recovery_root_is_state_subdir() {
        let root = recovery_root_from_state(Path::new("/tmp/state"));
        assert_eq!(
            root,
            PathBuf::from("/tmp/state").join(RECOVERY_STATE_SUBDIR)
        );
    }

    #[test]
    fn startup_scan_offers_valid_record() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        store
            .persist(&sample_snapshot(7, b"hello recovery"))
            .expect("persist");

        let mut session = CrashRecoverySession::open_at(dir.path());
        assert!(!session.is_unavailable());
        let offer = session.active_offer().expect("offer");
        assert_eq!(offer.metadata().content_len(), b"hello recovery".len());
        assert_eq!(offer.original_path_label(), "notes.txt");

        let (document, selection) = session.restore_active_offer().expect("restore");
        assert!(document.is_dirty());
        assert_eq!(String::from(document.rope()), "hello recovery");
        assert_eq!(selection, Selection::caret(0));
        assert!(session.active_offer().is_none());
        assert!(store.scan_startup().expect("rescan").is_empty());
    }

    #[test]
    fn stale_startup_discard_does_not_delete_the_replacement() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        let snapshot = sample_snapshot(6, b"keep after failed discard");
        let record_path = store.record_path(snapshot.instance_id());
        store.persist(&snapshot).expect("persist");
        let mut session = CrashRecoverySession::open_at(dir.path());
        assert!(session.active_offer().is_some());

        fs::remove_file(&record_path).expect("replace record fixture");
        fs::create_dir(&record_path).expect("block record deletion");
        assert!(!session.discard_active_offer());
        assert!(session.has_cleanup_failure());
        assert!(record_path.is_dir());

        fs::remove_dir(record_path).expect("clean fixture");
    }

    #[test]
    fn failed_save_cleanup_surfaces_a_durable_warning() {
        let dir = tempdir().expect("tempdir");
        let mut session = CrashRecoverySession::open_at(dir.path());
        let mut document = Document::new();
        document.replace_text("dirty").expect("edit");
        session.on_edited(&document, Selection::caret(5));
        session.force_due_persist_for_test(&document, Selection::caret(5));

        let store = RecoveryStore::open(dir.path()).expect("store");
        let record_path = store.record_path(session.instance_id);
        fs::remove_file(&record_path).expect("replace record fixture");
        fs::create_dir(&record_path).expect("block record deletion");

        session.on_saved_clean(document.revision());
        assert!(session.has_cleanup_failure());

        fs::remove_dir(record_path).expect("clean fixture");
    }

    #[test]
    fn restore_keeps_the_durable_offer_when_successor_storage_is_unavailable() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        let snapshot = sample_snapshot(8, b"only durable recovery");
        let original_record = store.record_path(snapshot.instance_id());
        store.persist(&snapshot).expect("persist");
        let mut session = CrashRecoverySession::open_at(dir.path());
        assert!(session.active_offer().is_some());

        session.store = None;
        let Err(error) = session.restore_active_offer() else {
            panic!("restore must stop before deleting the only record");
        };

        assert_eq!(error, RECOVERY_RESTORE_RETAINED_MESSAGE);
        assert!(session.is_unavailable());
        assert!(session.active_offer().is_some());
        assert!(original_record.exists());
        let encoded = fs::read(original_record).expect("retained record");
        let RecoveryStartupDisposition::Offer(record) = validate_recovery_record(&encoded) else {
            panic!("retained record must remain valid");
        };
        assert_eq!(record.content(), b"only durable recovery");
    }

    #[test]
    fn restore_remains_successful_when_later_ancestor_cleanup_is_busy() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        let predecessor = sample_snapshot(1, b"causal recovery");
        store.persist(&predecessor).expect("persist predecessor");
        let predecessor_bytes =
            fs::read(store.record_path(predecessor.instance_id())).expect("read predecessor");
        let RecoveryStartupDisposition::Offer(predecessor_record) =
            validate_recovery_record(&predecessor_bytes)
        else {
            panic!("predecessor record");
        };
        let successor = RecoverySnapshot::try_new_successor(
            RecoverySnapshotParts {
                document_id: predecessor.document_id(),
                instance_id: RecoveryInstanceId::new([2; 16]),
                revision: Revision::new(1),
                created_at: RecoveryWallTime::from_unix_millis(3),
                updated_at: RecoveryWallTime::from_unix_millis(1),
                original_path: b"notes.txt".to_vec(),
                bom: Bom::Absent,
                encoding: Encoding::Utf8,
                selection: Selection::caret(0),
                content: b"causal recovery".to_vec(),
            },
            predecessor_record.metadata(),
        )
        .expect("successor");
        store.persist(&successor).expect("persist successor");

        let mut session = CrashRecoverySession::open_at(dir.path());
        let offer = session.active_offer().expect("coalesced successor offer");
        assert_eq!(offer.metadata().instance_id(), successor.instance_id());
        assert_eq!(offer.artifact.superseded().len(), 1);
        let busy_ancestor = store
            .try_hold_live_lease(predecessor.instance_id())
            .expect("hold ancestor busy after startup scan");

        let (document, _) = session
            .restore_active_offer()
            .expect("durable successor makes cleanup non-fatal");
        assert_eq!(String::from(document.rope()), "causal recovery");
        assert!(session.has_cleanup_failure());
        assert!(session.active_offer().is_none());
        let durable_successor = fs::read(store.record_path(session.instance_id))
            .expect("replacement successor remains durable");
        let RecoveryStartupDisposition::Offer(record) =
            validate_recovery_record(&durable_successor)
        else {
            panic!("replacement successor record");
        };
        assert_eq!(
            record.lineage_generation(),
            Some(RecoveryLineageGeneration::new(2))
        );
        assert_eq!(record.predecessor_instance(), Some(successor.instance_id()));

        session.on_saved_clean(document.revision());
        assert!(
            session.has_cleanup_failure(),
            "cleaning the successor must not hide the unresolved ancestor cleanup"
        );

        drop(busy_ancestor);
        fs::remove_file(store.live_path(predecessor.instance_id())).expect("remove test lease");
    }

    #[test]
    fn restore_then_later_keeps_remaining_records_on_disk() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        store
            .persist(&sample_snapshot(1, b"first"))
            .expect("persist first");
        store
            .persist(&sample_snapshot(2, b"second"))
            .expect("persist second");
        let mut session = CrashRecoverySession::open_at(dir.path());
        assert!(session.active_offer().is_some());

        let (document, _) = session.restore_active_offer().expect("restore");
        let restored = String::from(document.rope());
        assert!(restored == "first" || restored == "second");
        session.defer_startup_offers();

        assert!(session.active_offer().is_none());
        let remaining = store
            .scan_startup()
            .expect("rescan")
            .into_iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) => Some(
                    store
                        .load_record(offer.primary())
                        .expect("load remaining offer")
                        .content()
                        .to_vec(),
                ),
                RecoveryScanDisposition::Quarantine(_) => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(remaining.len(), 1);
        assert_ne!(remaining[0], restored.as_bytes());
    }

    #[test]
    fn discard_offer_deletes_record() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        store
            .persist(&sample_snapshot(3, b"discard me"))
            .expect("persist");
        let mut session = CrashRecoverySession::open_at(dir.path());
        assert!(session.active_offer().is_some());
        session.discard_active_offer();
        assert!(session.active_offer().is_none());
        assert!(store.scan_startup().expect("rescan").is_empty());
    }

    #[test]
    fn defer_startup_offers_hides_review_without_deleting_records() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        store
            .persist(&sample_snapshot(4, b"keep this recovery"))
            .expect("persist");
        let mut session = CrashRecoverySession::open_at(dir.path());
        assert!(session.active_offer().is_some());

        session.defer_startup_offers();

        assert!(session.active_offer().is_none());
        let entries = store.scan_startup().expect("rescan");
        assert_eq!(entries.len(), 1);
        match entries[0].disposition() {
            RecoveryScanDisposition::Offer(offer) => {
                let record = store
                    .load_record(offer.primary())
                    .expect("load deferred offer");
                assert_eq!(record.content(), b"keep this recovery");
            }
            RecoveryScanDisposition::Quarantine(_) => {
                panic!("deferring offers must not quarantine a valid record")
            }
        }
    }

    #[test]
    fn a_living_window_does_not_offer_another_windows_recovery_record() {
        let dir = tempdir().expect("tempdir");
        let mut owner = CrashRecoverySession::open_at(dir.path());
        let mut document = Document::new();
        document.replace_text("owned draft").expect("edit");
        owner.on_edited(&document, Selection::caret(0));
        owner.session_started = Instant::now()
            .checked_sub(Duration::from_secs(3))
            .expect("clock");
        owner.on_tick(&document, Selection::caret(0));
        owner.wait_for_persist(&document, Selection::caret(0));

        let other = CrashRecoverySession::open_at(dir.path());
        assert!(other.active_offer().is_none());

        drop(owner);
        let after = CrashRecoverySession::open_at(dir.path());
        let offer = after
            .active_offer()
            .expect("dead window should offer restore");
        assert_eq!(offer.metadata().content_len(), b"owned draft".len());
    }

    #[test]
    fn stale_worker_outcome_cannot_delete_a_newer_record() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        let mut session = CrashRecoverySession::open_at(dir.path());
        let instance_id = session.instance_id;
        let stale_revision = Revision::new(1);
        let current_revision = Revision::new(2);

        let _ = session
            .schedule
            .reduce(RecoveryScheduleCommand::BecameClean {
                revision: stale_revision,
            });
        let current_epoch = session.schedule.epoch();
        let _ = session.schedule.reduce(RecoveryScheduleCommand::Edited {
            revision: current_revision,
            now: RecoveryClock::new(Duration::ZERO),
        });
        let _ = session.schedule.reduce(RecoveryScheduleCommand::Tick {
            now: RecoveryClock::new(Duration::from_secs(2)),
        });

        let (jobs_tx, jobs_rx) = mpsc::channel();
        let (outcomes_tx, outcomes_rx) = mpsc::channel();
        let gate = Arc::new(AtomicU64::new(current_epoch));
        let worker_gate = Arc::clone(&gate);
        let worker = thread::spawn(move || {
            persist_worker_loop(jobs_rx, outcomes_tx, worker_gate);
        });
        jobs_tx
            .send(PersistJob {
                store: store.clone(),
                snapshot: snapshot_for(instance_id, stale_revision, b"stale"),
                revision: stale_revision,
                epoch: current_epoch - 1,
            })
            .expect("stale job");
        jobs_tx
            .send(PersistJob {
                store: store.clone(),
                snapshot: snapshot_for(instance_id, current_revision, b"newer recovery"),
                revision: current_revision,
                epoch: current_epoch,
            })
            .expect("current job");
        drop(jobs_tx);
        worker.join().expect("worker");

        session.persist_outcomes = Some(outcomes_rx);
        session.poll_persist_outcomes();

        assert_eq!(
            session.schedule.last_persisted_revision(),
            Some(current_revision)
        );
        let encoded = fs::read(store.record_path(instance_id)).expect("current record");
        let RecoveryStartupDisposition::Offer(record) = validate_recovery_record(&encoded) else {
            panic!("current record must remain valid");
        };
        assert_eq!(record.content(), b"newer recovery");
    }

    #[test]
    fn lease_errors_disable_recovery_and_fail_startup_scan_closed() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        let mut owner = CrashRecoverySession::open_at(dir.path());
        let owner_instance = owner.instance_id;
        let mut document = Document::new();
        document.replace_text("live unsaved work").expect("edit");
        owner.on_edited(&document, Selection::caret(0));
        owner.session_started = Instant::now()
            .checked_sub(Duration::from_secs(3))
            .expect("clock");
        owner.on_tick(&document, Selection::caret(0));
        owner.wait_for_persist(&document, Selection::caret(0));

        owner.release_live_lease_for(owner_instance);
        fs::create_dir(store.live_path(owner_instance)).expect("blocking lease path");
        owner.acquire_live_lease();

        assert!(owner.is_unavailable());
        assert!(store.record_path(owner_instance).exists());
        let other = CrashRecoverySession::open_at(dir.path());
        assert!(other.is_unavailable());
        assert!(other.active_offer().is_none());
        assert!(store.record_path(owner_instance).exists());
    }

    #[test]
    fn dirty_edit_persists_after_idle_debounce() {
        let dir = tempdir().expect("tempdir");
        let mut session = CrashRecoverySession::open_at(dir.path());
        let mut document = Document::new();
        document.replace_text("draft").expect("edit");
        session.on_edited(&document, Selection::caret(0));
        // Advance monotonic clock by rewriting session_started into the past.
        session.session_started = Instant::now()
            .checked_sub(Duration::from_secs(3))
            .expect("clock");
        session.on_tick(&document, Selection::caret(0));
        session.wait_for_persist(&document, Selection::caret(0));
        assert!(!session.has_persist_failure());
        let store = RecoveryStore::open(dir.path()).expect("store");
        assert_eq!(recovery_record_count(&store), 1);
        assert!(
            store.scan_startup().expect("scan").is_empty(),
            "a living window must not offer its own in-flight recovery record"
        );
    }

    #[test]
    fn fresh_identity_keeps_distinct_recovery_instances() {
        let dir = tempdir().expect("tempdir");
        let mut session = CrashRecoverySession::open_at(dir.path());
        assert!(!session.is_unavailable());
        let first_instance = session.instance_id;

        let mut first = Document::new();
        first.replace_text("first session").expect("edit");
        session.on_edited(&first, Selection::caret(0));
        session.session_started = Instant::now()
            .checked_sub(Duration::from_secs(3))
            .expect("clock");
        session.on_tick(&first, Selection::caret(0));
        session.wait_for_persist(&first, Selection::caret(0));

        session.begin_fresh_identity();
        assert!(!session.is_unavailable());
        assert_ne!(session.instance_id, first_instance);

        let mut second = Document::new();
        second.replace_text("second session").expect("edit");
        session.on_edited(&second, Selection::caret(0));
        session.session_started = Instant::now()
            .checked_sub(Duration::from_secs(3))
            .expect("clock");
        session.on_tick(&second, Selection::caret(0));
        session.wait_for_persist(&second, Selection::caret(0));

        let store = RecoveryStore::open(dir.path()).expect("store");
        assert_eq!(recovery_record_count(&store), 2);
        let entries = store.scan_startup().expect("scan");
        // The abandoned first instance is offerable; the living second is not.
        let contents: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryScanDisposition::Offer(offer) => Some(
                    store
                        .load_record(offer.primary())
                        .expect("load abandoned offer")
                        .content()
                        .to_vec(),
                ),
                RecoveryScanDisposition::Quarantine(_) => None,
            })
            .collect();
        assert_eq!(contents, vec![b"first session".to_vec()]);
    }

    #[test]
    fn random_id_bytes_rejects_all_zero_buffer() {
        // Operating-system fill is expected to succeed on CI hosts; the
        // zero-rejection branch is structural insurance, not a forced fault.
        let bytes = random_id_bytes().expect("csprng");
        assert!(bytes.iter().any(|&byte| byte != 0));
    }

    #[test]
    fn save_clean_deletes_owned_record() {
        let dir = tempdir().expect("tempdir");
        let mut session = CrashRecoverySession::open_at(dir.path());
        let mut document = Document::new();
        document.replace_text("keep").expect("edit");
        session.on_edited(&document, Selection::caret(0));
        session.session_started = Instant::now()
            .checked_sub(Duration::from_secs(3))
            .expect("clock");
        session.on_tick(&document, Selection::caret(0));
        session.wait_for_persist(&document, Selection::caret(0));
        let store = RecoveryStore::open(dir.path()).expect("store");
        assert_eq!(recovery_record_count(&store), 1);
        session.on_saved_clean(document.revision());
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            session.poll_persist_outcomes();
            if RecoveryStore::open(dir.path())
                .expect("store")
                .scan_startup()
                .expect("scan")
                .is_empty()
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "save cleanup must delete the recovery record, including a late worker write"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn save_keeps_the_live_lease_for_later_unsaved_edits() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        let mut owner = CrashRecoverySession::open_at(dir.path());
        let owner_instance = owner.instance_id;
        let mut document = Document::new();
        document.replace_text("before save").expect("edit");
        owner.on_edited(&document, Selection::caret(0));
        owner.session_started = Instant::now()
            .checked_sub(Duration::from_secs(3))
            .expect("clock");
        owner.on_tick(&document, Selection::caret(0));
        owner.wait_for_persist(&document, Selection::caret(0));

        owner.on_saved_clean(document.revision());

        assert!(store.live_path(owner_instance).exists());
        assert!(store.instance_is_live(owner_instance).expect("probe"));
        document.replace_text("after save").expect("second edit");
        owner.on_edited(&document, Selection::caret(0));
        owner.session_started = Instant::now()
            .checked_sub(Duration::from_secs(6))
            .expect("clock");
        owner.on_tick(&document, Selection::caret(0));
        owner.wait_for_persist(&document, Selection::caret(0));

        let other = CrashRecoverySession::open_at(dir.path());
        assert!(other.active_offer().is_none());
        assert!(store.record_path(owner_instance).exists());
    }

    #[test]
    fn fresh_identity_keeps_worker_epoch_in_sync() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        let mut session = CrashRecoverySession::open_at(dir.path());
        let mut first = Document::new();
        first.replace_text("first document").expect("edit");
        session.on_edited(&first, Selection::caret(0));
        session.session_started = Instant::now()
            .checked_sub(Duration::from_secs(3))
            .expect("clock");
        session.on_tick(&first, Selection::caret(0));
        session.wait_for_persist(&first, Selection::caret(0));
        session.on_saved_clean(first.revision());

        let previous_epoch = session.schedule.epoch();
        session.begin_fresh_identity();
        let second_instance = session.instance_id;
        assert_eq!(session.schedule.epoch(), previous_epoch.wrapping_add(1));
        assert_eq!(
            session.persist_epoch_gate.load(Ordering::Acquire),
            session.schedule.epoch()
        );

        let mut second = Document::new();
        second.replace_text("second document").expect("edit");
        session.on_edited(&second, Selection::caret(0));
        session.session_started = Instant::now()
            .checked_sub(Duration::from_secs(6))
            .expect("clock");
        session.on_tick(&second, Selection::caret(0));
        session.wait_for_persist(&second, Selection::caret(0));

        assert!(!session.has_persist_failure());
        assert!(store.record_path(second_instance).exists());
    }

    #[test]
    fn quarantine_notice_is_recorded() {
        let dir = tempdir().expect("tempdir");
        let store = RecoveryStore::open(dir.path()).expect("store");
        std::fs::write(store.records_dir().join("broken.rec"), b"not-a-record")
            .expect("write broken");
        let session = CrashRecoverySession::open_at(dir.path());
        assert!(!session.quarantine_notices().is_empty());
        assert!(session.active_offer().is_none());
    }
}
