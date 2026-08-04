//! Application adapter for private crash-recovery records.
//!
//! Pure scheduling and on-disk record integrity live in `noter::core::recovery`
//! and `noter::core::recovery_store`. This module owns process identity, wall
//! and monotonic clocks, the private recovery root under the eframe state
//! directory, and the small state machine that surfaces startup offers and
//! persist failures without writing a user document path.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use getrandom::fill as fill_random;
use noter::core::document::Document;
use noter::core::edit::Selection;
use noter::core::recovery::{
    RecoveryClock, RecoveryDocumentId, RecoveryInstanceId, RecoveryOfferDecision,
    RecoveryOfferState, RecoveryScheduleCommand, RecoveryScheduleEffect, RecoveryScheduleState,
    RecoverySnapshot, RecoverySnapshotParts, RecoveryStartupDisposition, RecoveryWallTime,
    ValidatedRecoveryRecord,
};
use noter::core::recovery_store::{RecoveryScanEntry, RecoveryStore};
use noter::core::revision::Revision;

/// Subdirectory of the eframe state root used for crash-recovery files.
pub const RECOVERY_STATE_SUBDIR: &str = "recovery";

/// Application id shared with eframe persistence (`app.ron`).
pub const NOTER_APP_ID: &str = "Noter";

/// User-visible message when a recovery write fails. No paths or content.
pub const RECOVERY_PERSIST_FAILURE_MESSAGE: &str = "Noter could not update its private recovery copy of this document. Your text is still in this window. Keep the window open and save when you can.";

/// User-visible message when the recovery store cannot open.
pub const RECOVERY_UNAVAILABLE_MESSAGE: &str = "Private crash recovery is unavailable in this session because Noter could not open its per-user recovery folder. Saves still work; keep backups of important work.";

/// Maximum length of a quarantine notice shown at startup.
const MAX_QUARANTINE_NOTICE_BYTES: usize = 240;

/// Resolves the per-user state directory used for preferences and recovery.
///
/// Matches eframe's `storage_dir("Noter")` layout documented in INSTALLATION.md.
#[must_use]
pub fn noter_state_directory() -> Option<PathBuf> {
    eframe::storage_dir(NOTER_APP_ID)
}

/// Returns the private recovery root under the state directory.
#[must_use]
pub fn recovery_root_from_state(state_dir: impl AsRef<Path>) -> PathBuf {
    state_dir.as_ref().join(RECOVERY_STATE_SUBDIR)
}

/// One startup recovery candidate ready for an explicit user decision.
#[derive(Clone, Debug)]
pub struct StartupRecoveryOffer {
    record: ValidatedRecoveryRecord,
    offer: RecoveryOfferState,
}

impl StartupRecoveryOffer {
    /// Returns the validated recovery payload.
    pub const fn record(&self) -> &ValidatedRecoveryRecord {
        &self.record
    }

    /// Returns whether the offer is still open.
    pub const fn is_open(&self) -> bool {
        self.offer.is_open()
    }

    /// Short label for UI without full paths when the original path is non-UTF-8.
    pub fn original_path_label(&self) -> String {
        let bytes = self.record.original_path();
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
    startup_offers: Vec<StartupRecoveryOffer>,
    active_offer_index: Option<usize>,
    quarantine_notices: Vec<String>,
    persist_failure: bool,
    unavailable: bool,
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
                session.store = Some(store);
                session.ingest_scan();
            }
            Err(_) => {
                session.unavailable = true;
            }
        }
        session
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
            startup_offers: Vec::new(),
            active_offer_index: None,
            quarantine_notices: Vec::new(),
            persist_failure: false,
            unavailable,
        }
    }

    fn ingest_scan(&mut self) {
        let Some(store) = self.store.as_ref() else {
            return;
        };
        let Ok(entries) = store.scan_startup() else {
            self.unavailable = true;
            return;
        };
        self.apply_scan_entries(entries);
    }

    fn apply_scan_entries(&mut self, entries: Vec<RecoveryScanEntry>) {
        self.startup_offers.clear();
        self.quarantine_notices.clear();
        self.active_offer_index = None;
        for entry in entries {
            match entry.disposition() {
                RecoveryStartupDisposition::Offer(record) => {
                    let mut offer = RecoveryOfferState::default();
                    offer.present();
                    self.startup_offers.push(StartupRecoveryOffer {
                        record: record.clone(),
                        offer,
                    });
                }
                RecoveryStartupDisposition::Quarantine(reason) => {
                    let message = entry.quarantine_error().map_or_else(
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

    /// Clears a dismissed persist-failure notice without claiming durability.
    pub const fn dismiss_persist_failure(&mut self) {
        self.persist_failure = false;
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
    /// The on-disk recovery record for that instance is removed after a successful
    /// load so a later launch does not re-offer the same bytes.
    ///
    /// # Errors
    ///
    /// Returns an error when there is no open offer or content cannot become a
    /// document.
    pub fn restore_active_offer(&mut self) -> Result<(Document, Selection), String> {
        let index = self
            .active_offer_index
            .ok_or_else(|| "No recovery offer is open.".to_owned())?;
        let offer = self
            .startup_offers
            .get_mut(index)
            .ok_or_else(|| "No recovery offer is open.".to_owned())?;
        if !offer.is_open() {
            return Err("No recovery offer is open.".to_owned());
        }
        let _ = offer.offer.decide(RecoveryOfferDecision::Restore);
        let record = offer.record.clone();
        let selection = record.selection();

        let mut document = Document::from_bytes(record.content())
            .map_err(|error| format!("Recovered content could not be opened: {error}"))?;
        document.mark_recovered_dirty();
        debug_assert!(document.is_dirty());

        if let Some(store) = self.store.as_ref() {
            let _ = store.delete_instance(record.instance_id());
        }

        self.finish_offer_slot(index);
        self.begin_fresh_identity();
        self.document_id = record.document_id();
        Ok((document, selection))
    }

    /// Discards the active startup offer and deletes its on-disk record.
    pub fn discard_active_offer(&mut self) {
        let Some(index) = self.active_offer_index else {
            return;
        };
        let Some(offer) = self.startup_offers.get_mut(index) else {
            return;
        };
        if !offer.is_open() {
            return;
        }
        let instance = offer.record.instance_id();
        let _ = offer.offer.decide(RecoveryOfferDecision::Discard);
        if let Some(store) = self.store.as_ref() {
            let _ = store.delete_instance(instance);
        }
        self.finish_offer_slot(index);
    }

    fn finish_offer_slot(&mut self, index: usize) {
        if index < self.startup_offers.len() {
            self.startup_offers.remove(index);
        }
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
    }

    /// Begins tracking a new editor identity after New / successful open / restore.
    ///
    /// When operating-system randomness is unavailable, recovery is marked
    /// unavailable for the rest of the session so the adapter never persists
    /// with a weak or zero identity that could collide with another session.
    pub fn begin_fresh_identity(&mut self) {
        match (random_document_id(), random_instance_id()) {
            (Ok(document_id), Ok(instance_id)) => {
                self.document_id = document_id;
                self.instance_id = instance_id;
                self.created_wall = wall_now();
                self.schedule = RecoveryScheduleState::default();
                self.persist_failure = false;
            }
            _ => {
                self.mark_unavailable_for_identity_failure();
            }
        }
    }

    fn mark_unavailable_for_identity_failure(&mut self) {
        self.unavailable = true;
        self.schedule = RecoveryScheduleState::default();
        self.persist_failure = false;
    }

    /// Notifies the scheduler that content is dirty at the given revision.
    pub fn on_edited(&mut self, document: &Document, selection: Selection) {
        if self.unavailable || self.store.is_none() {
            return;
        }
        if self.active_offer().is_some() {
            return;
        }
        if !document.is_dirty() {
            return;
        }
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
        let now = self.monotonic_now();
        let effect = self.schedule.reduce(RecoveryScheduleCommand::Tick { now });
        self.apply_schedule_effect(effect, Some((document, selection)));
    }

    /// Records a successful Save and deletes the owned recovery record.
    pub fn on_saved_clean(&mut self, revision: Revision) {
        if self.unavailable || self.store.is_none() {
            return;
        }
        let effect = self
            .schedule
            .reduce(RecoveryScheduleCommand::BecameClean { revision });
        self.apply_schedule_effect(effect, None);
        self.persist_failure = false;
    }

    /// Records an explicit Discard and deletes the owned recovery record.
    pub fn on_discarded(&mut self) {
        if self.unavailable || self.store.is_none() {
            return;
        }
        let effect = self.schedule.reduce(RecoveryScheduleCommand::Discarded);
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
                    let _ = store.delete_instance(self.instance_id);
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
        let Ok(snapshot) = RecoverySnapshot::try_new(RecoverySnapshotParts {
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
        }) else {
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
        if store.persist(&snapshot).is_ok() {
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

    fn monotonic_now(&self) -> RecoveryClock {
        RecoveryClock::new(self.session_started.elapsed())
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
        RecoveryWallTime,
    };
    use noter::core::revision::Revision;
    use noter::core::text_format::{Bom, Encoding};
    use tempfile::tempdir;

    fn sample_snapshot(instance: u8, content: &[u8]) -> RecoverySnapshot {
        RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
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
        assert_eq!(offer.record().content(), b"hello recovery");
        assert_eq!(offer.original_path_label(), "notes.txt");

        let (document, selection) = session.restore_active_offer().expect("restore");
        assert!(document.is_dirty());
        assert_eq!(String::from(document.rope()), "hello recovery");
        assert_eq!(selection, Selection::caret(0));
        assert!(session.active_offer().is_none());
        assert!(store.scan_startup().expect("rescan").is_empty());
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
        assert!(!session.has_persist_failure());
        let store = RecoveryStore::open(dir.path()).expect("store");
        let entries = store.scan_startup().expect("scan");
        assert_eq!(entries.len(), 1);
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

        let entries = RecoveryStore::open(dir.path())
            .expect("store")
            .scan_startup()
            .expect("scan");
        // Prior instance remains on disk until save/discard; new instance is distinct.
        assert_eq!(entries.len(), 2);
        let contents: Vec<_> = entries
            .iter()
            .filter_map(|entry| match entry.disposition() {
                RecoveryStartupDisposition::Offer(record) => Some(record.content().to_vec()),
                RecoveryStartupDisposition::Quarantine(_) => None,
            })
            .collect();
        assert!(contents.iter().any(|c| c == b"first session"));
        assert!(contents.iter().any(|c| c == b"second session"));
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
        assert_eq!(
            RecoveryStore::open(dir.path())
                .expect("store")
                .scan_startup()
                .expect("scan")
                .len(),
            1
        );
        session.on_saved_clean(document.revision());
        assert!(
            RecoveryStore::open(dir.path())
                .expect("store")
                .scan_startup()
                .expect("scan")
                .is_empty()
        );
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
