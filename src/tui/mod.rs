//! Native Terminal User Interface (TUI) mode for Noter.
//!
//! Provides a fast, focused terminal editor ("like Nano... but better") powered
//! by Noter's authoritative core document engine and platform primitives.
//!
//! Features:
//! - Dual shortcuts: Nano (^O Save As with a prefilled name, ^X Exit, ^W `WhereIs`, ^K Cut,
//!   ^U Paste, ^J Jump, ^G Help) and Modern (Ctrl+S, Ctrl+Q, Ctrl+F, Ctrl+Z, Ctrl+Y, Ctrl+E,
//!   Ctrl+A).
//! - Saving never exits or discards text unless the write committed. Replacing an existing
//!   file or splitting a hard link asks first. A save that finds a disk change offers
//!   reload, keep editing, Save As, and overwrite after a second confirmation. Until
//!   reload, a committed save, or an explicit discard, that open text stays recoverable.
//! - The host terminal supplies the colors. A non-empty `NO_COLOR` turns color off.
//!   `--theme` selects the window theme and does not change this interface.
//! - Fluid formatted Markdown viewing toggle (^E).
//! - Terminal mouse support (click to set caret, scroll wheel to scroll lines, clickable legend).
//! - Authoritative revision tracking, atomic safe save, and Undo/Redo history.

use std::fmt::Write as FmtWrite;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use noter::core::conflict::{
    ConflictCommand, ConflictDecision, ConflictEffect, ConflictState, ExternalChangeKind,
    classify_external_change,
};
use noter::core::document::{Document, PreparedSaveAs};
use noter::core::edit::{
    EditOrigin, EditTimestamp, EditTransaction, Selection, TextEdit, TextRange,
};
use noter::core::file_observation::inspect_target;
use noter::core::fs_storage::{
    remove_verified_retained_siblings, retained_sibling_removal_message,
};
use noter::core::limits::MAX_DOCUMENT_BYTES;
use noter::core::line_endings::{logical_lines, normalize_inserted_text};
use noter::core::navigation::{
    LineNavigationError, MoveDirection, MoveUnit, line_start_offset, move_caret,
};
use noter::core::revision::Revision;
use noter::core::save::{
    FileObservation, SaveOutcome, SaveStage, SaveWarnings, TargetExpectation, TargetState,
};
use noter::core::save_recovery::{
    MAX_SAVE_RECOVERY_DESTINATION_BYTES, MAX_SAVE_RECOVERY_MESSAGE_BYTES,
    MAX_SAVE_RECOVERY_RECORDS, bounded_destination_label, recovery_path_clipboard_text,
    write_save_recovery_message,
};
use noter::core::search::{LiteralSearch, MatchCase, SearchDirection};
use noter::core::terminal_text::{
    cell_width, column_of, display_width, escaped_cli_path, fit_line, offset_at_column,
    push_display,
};
use noter::core::undo::{
    HistoryApplyOutcome, HistoryError, HistoryLimits, HistoryRecordOutcome, UndoHistory,
};
use noter::error::NoterError;

use crate::app::{DocumentView, LaunchOptions};
use crate::crash_recovery::{
    CrashRecoverySession, RECOVERY_CLEANUP_FAILURE_MESSAGE, RECOVERY_PERSIST_FAILURE_MESSAGE,
    SaveDurabilityRisk,
};

mod disk_conflict;
mod input;
mod paint;
mod save_warnings;

use disk_conflict::ReviewedDisk;
use paint::{Paint, Role, no_color_from_env, push_bar};

pub use input::{InputDecoder, TuiEvent, TuiKey, TuiMouseEvent};

/// Active modal prompt in the status area.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PromptMode {
    None,
    SaveAs,
    /// The Save As destination exists; replacing it needs a yes.
    ConfirmReplace,
    /// The destination has other hard links that keep the previous text.
    ConfirmHardLink(u64),
    Find,
    GoToLine,
    ExitConfirm,
    /// Unsaved text from an earlier session is waiting in private recovery.
    RecoveryOffer,
    /// An uncertain save requires explicit inspection and confirmation.
    ReconcileSave(usize),
    /// Scrollable cleanup and durability diagnostics from the latest save.
    SaveWarnings,
    /// A save found a disk change and is waiting for reload, keep, Save As, or overwrite.
    DiskChange,
    /// Overwrite was requested and still needs a second confirmation.
    ConfirmDiskOverwrite,
    /// Reload would discard unsaved text and still needs a second confirmation.
    ConfirmDiskReload,
}

/// The answer to a startup recovery offer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RecoveryDecision {
    /// Replace the empty document with the recovered text.
    Restore,
    /// Delete the recovered text.
    Discard,
    /// Keep the recovered text for a later launch.
    Later,
}

/// What happens once the save in progress commits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AfterSave {
    Stay,
    Exit,
}

/// A save that is waiting for the user to answer a confirmation.
#[derive(Clone, Debug)]
pub enum PendingSave {
    /// Save in place, splitting the document's hard link.
    SplitHardLinkInPlace(SaveRecoveryReservation),
    /// Save As over an existing file.
    Replace(PreparedSaveAs, SaveRecoveryReservation),
    /// Save As over an existing file, splitting its hard link.
    SplitHardLink(PreparedSaveAs, SaveRecoveryReservation),
}

/// One unresolved save outcome retained independently until reconciliation.
#[derive(Clone, Debug)]
pub struct SaveRecoveryRecord {
    destination: PathBuf,
    destination_label: String,
    message: String,
}

/// Storage reserved before a save can inspect or mutate its destination.
#[derive(Clone, Debug)]
pub struct SaveRecoveryReservation {
    destination: PathBuf,
    destination_label: String,
    message: String,
}

/// The result of one save step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SaveStep {
    Committed,
    /// Nothing was written, or the outcome is uncertain; the text stays open.
    NotSaved,
    /// A prompt now asks for a file name or a confirmation.
    AwaitingInput,
}

/// How a temporary status line is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum StatusTone {
    /// Reverse video, the ordinary status bar.
    #[default]
    Plain,
    /// Green. A committed save with no durability warning.
    Success,
    /// Yellow. Durability or storage still needs attention.
    Warning,
    /// Red. The save did not commit.
    Error,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RetainedSiblingRemoval {
    Idle,
    Armed,
}

/// The open text is the last copy of a version the disk no longer matches.
///
/// Content can still match the last save, so this is separate from dirty
/// state. Keep Editing leaves it set. Reload, a committed save, and an
/// explicit discard clear it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ExternalRetention {
    Clear,
    AtRisk,
}

/// Whether the terminal may use color.
///
/// `Off` is a non-empty `NO_COLOR`, or a test that forces the plain frame.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TerminalColor {
    Host,
    Off,
}

impl TerminalColor {
    fn from_env() -> Self {
        if no_color_from_env() {
            Self::Off
        } else {
            Self::Host
        }
    }

    const fn paint(self) -> Paint {
        Paint::new(matches!(self, Self::Off))
    }
}

/// State for the terminal editor session.
pub struct TuiSession {
    pub document: Document,
    pub view: DocumentView,
    pub caret_byte: usize,
    pub scroll_row: usize,
    pub scroll_col: usize,
    /// Whether the next frame scrolls to keep the caret visible. The mouse
    /// wheel clears it so the view can move away from the caret.
    pub follow_caret: bool,
    pub clipboard: String,
    pub prompt: PromptMode,
    pub prompt_input: String,
    pub status_message: Option<(String, std::time::Instant)>,
    status_tone: StatusTone,
    /// Set once from `NO_COLOR` when the session starts. Tests set it directly.
    terminal_color: TerminalColor,
    pub search_query: Option<String>,
    pub show_help: bool,
    /// Bounded undo and redo, shared with the window through the core.
    pub history: UndoHistory,
    /// Private crash-recovery records, shared with the window's store.
    pub recovery: CrashRecoverySession,
    save_durability: SaveDurabilityRisk,
    /// Monotonic origin for edit timestamps, so typing coalesces into Undo
    /// steps by the same rules as the window.
    pub started: Instant,
    pub should_exit: bool,
    pub after_save: AfterSave,
    pub pending_save: Option<PendingSave>,
    /// Independent unresolved save outcomes that block all later saves.
    pub save_recoveries: Vec<SaveRecoveryRecord>,
    /// Next terminal clipboard request, emitted only after an explicit copy.
    pending_clipboard: Option<String>,
    reconcile_scroll: usize,
    last_save_warnings: Option<SaveWarnings>,
    retained_sibling_removal: RetainedSiblingRemoval,
    /// Decision state shared with the window. The terminal does not watch while idle.
    conflict: ConflictState,
    /// Destination reviewed for the visible disk decision, if one is open.
    reviewed_disk: Option<ReviewedDisk>,
    /// Set when a save finds a disk change, and cleared only by reload, a
    /// committed save, a matching observation, or an explicit discard.
    external_retention: ExternalRetention,
    /// Durable note for a path launch or a restore. Escape clears it. It
    /// remains after the temporary status expires.
    recovery_path_hint: Option<String>,
    /// Session-long note that private recovery is off. Escape hides the line.
    /// Ctrl+R still shows the full reason.
    recovery_unavailable_hint: Option<String>,
}

impl TuiSession {
    /// Initializes a new TUI session from launch options.
    ///
    /// An explicit file keeps any startup recovery offers for a later
    /// untitled launch, as the window does, and leaves a durable note when a
    /// copy stores that exact path. An untitled launch presents the first
    /// offer.
    ///
    /// # Errors
    ///
    /// Returns an error if an initial file path was supplied but cannot be loaded.
    pub fn new(options: &LaunchOptions, recovery: CrashRecoverySession) -> Result<Self, String> {
        let document = if let Some(path) = &options.initial_path {
            Document::from_path(path)
                .map_err(|e| format!("cannot load `{}`: {e}", escaped_cli_path(path)))?
        } else {
            Document::new()
        };

        // `--theme` selects the window palette. The terminal follows the host.
        let _ = options.theme;
        let view = options.view.unwrap_or(DocumentView::Text);
        let history = UndoHistory::new(HistoryLimits::default(), document.revision());
        // Read the hint before the path launch hides the in-memory offers.
        // Hiding them does not delete the records.
        let recovery_path_hint = options.initial_path.as_ref().and_then(|path| {
            recovery
                .path_recovery_hint(path)
                .map(|hint| hint.terminal_message().to_owned())
        });

        let mut session = Self {
            document,
            view,
            caret_byte: 0,
            scroll_row: 0,
            scroll_col: 0,
            follow_caret: true,
            clipboard: String::new(),
            prompt: PromptMode::None,
            prompt_input: String::new(),
            status_message: None,
            status_tone: StatusTone::Plain,
            terminal_color: TerminalColor::from_env(),
            search_query: None,
            show_help: false,
            history,
            recovery,
            save_durability: SaveDurabilityRisk::Clear,
            started: Instant::now(),
            should_exit: false,
            after_save: AfterSave::Stay,
            pending_save: None,
            save_recoveries: Vec::new(),
            pending_clipboard: None,
            reconcile_scroll: 0,
            last_save_warnings: None,
            retained_sibling_removal: RetainedSiblingRemoval::Idle,
            conflict: ConflictState::default(),
            reviewed_disk: None,
            external_retention: ExternalRetention::Clear,
            recovery_path_hint,
            recovery_unavailable_hint: None,
        };
        if options.initial_path.is_some() {
            session.recovery.defer_startup_offers();
            session.recovery.begin_fresh_identity();
        } else if session.recovery.active_offer().is_some() {
            session.prompt = PromptMode::RecoveryOffer;
        }
        if session.recovery.is_unavailable() {
            session.recovery_unavailable_hint = Some(session.recovery.unavailable_message());
        }
        Ok(session)
    }

    /// Tells crash recovery whether the text now needs a private copy, after
    /// Undo or Redo, as the window's `synchronize_crash_recovery`
    /// does. A direct edit uses `on_edited` instead, also as the window does.
    fn sync_recovery(&mut self) {
        let selection = Selection::caret(self.caret_byte);
        if self.has_unsaved_state() {
            self.recovery.on_retained(&self.document, selection);
        } else {
            self.recovery.on_saved_clean(self.document.revision());
        }
    }

    fn has_unsaved_state(&self) -> bool {
        self.document.is_dirty()
            || self.external_retention == ExternalRetention::AtRisk
            || self.save_durability.is_at_risk()
    }

    /// Remembers that the open text must survive a crash and an exit.
    fn note_external_retention(&mut self) {
        if self.external_retention == ExternalRetention::AtRisk {
            return;
        }
        self.external_retention = ExternalRetention::AtRisk;
        self.sync_recovery();
    }

    /// Drops crash retention once the open text is no longer the last copy.
    fn clear_external_retention(&mut self) {
        if self.external_retention == ExternalRetention::Clear {
            return;
        }
        self.external_retention = ExternalRetention::Clear;
        self.sync_recovery();
    }

    /// Runs recovery work that has come due, reports a failed write once,
    /// and returns whether that changed the status line.
    pub fn tick_recovery(&mut self) -> bool {
        self.recovery
            .on_tick(&self.document, Selection::caret(self.caret_byte));
        if !self.recovery.has_persist_failure() {
            return false;
        }
        self.recovery.dismiss_persist_failure();
        self.set_status(RECOVERY_PERSIST_FAILURE_MESSAGE);
        true
    }

    /// How long the loop may wait before recovery work comes due.
    pub fn recovery_delay(&self) -> Option<Duration> {
        self.recovery.next_persist_delay()
    }

    /// Keeps unsaved text in a recovery record before an exit the user did
    /// not choose, such as a terminal hangup.
    pub fn persist_before_exit(&mut self) -> bool {
        let selection = Selection::caret(self.caret_byte);
        self.recovery.persist_before_exit(
            &self.document,
            selection,
            EXIT_PERSIST_LIMIT,
            self.save_durability.is_at_risk(),
        )
    }

    /// Answers the startup recovery offer that is showing.
    pub fn answer_recovery_offer(&mut self, decision: RecoveryDecision) {
        match decision {
            RecoveryDecision::Restore => self.restore_offered_document(),
            RecoveryDecision::Discard => {
                if self.recovery.discard_active_offer() {
                    self.set_status("Discarded the recovered text");
                } else {
                    self.set_status(RECOVERY_CLEANUP_FAILURE_MESSAGE);
                }
            }
            RecoveryDecision::Later => {
                self.recovery.defer_startup_offers();
                self.set_status("Kept the recovered text for a later launch");
            }
        }
        self.prompt = if self.recovery.active_offer().is_some() {
            PromptMode::RecoveryOffer
        } else {
            PromptMode::None
        };
    }

    /// Restores the open offer and reconnects it to a regular file when it can.
    ///
    /// The explanation stays on the status line after the temporary status
    /// expires. Escape clears that note and does not discard the text.
    fn restore_offered_document(&mut self) {
        let path_bytes = self
            .recovery
            .active_offer()
            .map(|offer| offer.metadata().original_path().to_vec())
            .unwrap_or_default();
        match self.recovery.restore_active_offer() {
            Ok((mut document, selection)) => {
                let notice =
                    crate::crash_recovery::bind_recovered_destination(&mut document, &path_bytes);
                self.document = document;
                self.save_durability = SaveDurabilityRisk::Clear;
                self.caret_byte = selection.active();
                self.history.reset(self.document.revision());
                self.recovery.on_edited(&self.document, selection);
                // Remaining offers stay on disk for a later untitled launch.
                // Presenting the next one now would replace this text.
                self.recovery.defer_startup_offers();
                self.recovery_path_hint = Some(format!("{notice} Esc hides this."));
                self.set_status(notice);
            }
            Err(message) => self.set_status(message),
        }
    }

    /// Sets a temporary status message visible for 3 seconds.
    pub fn set_status(&mut self, message: impl Into<String>) {
        self.set_toned_status(message, StatusTone::Plain);
    }

    fn set_toned_status(&mut self, message: impl Into<String>, tone: StatusTone) {
        self.status_message = Some((message.into(), std::time::Instant::now()));
        self.status_tone = tone;
    }

    fn set_save_error(&mut self, message: impl Into<String>) {
        self.set_toned_status(message, StatusTone::Error);
    }

    fn set_uncertain_guidance(&mut self) {
        self.set_toned_status(UNCERTAIN_SAVE_GUIDANCE, StatusTone::Warning);
    }

    /// Clears a status message that has been visible for 3 seconds and
    /// reports whether the frame changed.
    pub fn expire_status(&mut self) -> bool {
        let expired = self
            .status_message
            .as_ref()
            .is_some_and(|(_, created)| created.elapsed() >= STATUS_LIFETIME);
        if expired {
            self.status_message = None;
        }
        expired
    }

    /// Inserts pasted text as one edit, with line endings converted to the
    /// document's convention and within the document size limit.
    pub fn paste_text(&mut self, pasted: &str) {
        let available = MAX_DOCUMENT_BYTES.saturating_sub(self.document.rope().len_bytes());
        let ending = self.document.line_endings().fallback_insertion();
        let normalized = normalize_inserted_text(pasted, ending, available);
        if normalized.was_limited() {
            self.set_status("Paste shortened to fit the document size limit");
        }
        self.insert_with_origin(normalized.text(), EditOrigin::Paste);
    }

    /// Returns the current document text as a String.
    #[must_use]
    pub fn text(&self) -> String {
        self.document.rope().to_string()
    }

    /// Moves the caret using pure navigation logic.
    pub fn move_caret(&mut self, direction: MoveDirection, unit: MoveUnit) {
        let text = self.text();
        self.caret_byte = move_caret(&text, self.caret_byte, direction, unit);
    }

    /// Replaces `range` with `inserted` as one transaction and moves the
    /// caret to `caret_after`.
    ///
    /// Only the replaced range is read, so an edit costs the size of the
    /// change, not the size of the document. The core validates boundaries,
    /// the removed text, and the size limit before anything changes.
    pub fn replace_range(
        &mut self,
        range: std::ops::Range<usize>,
        inserted: &str,
        caret_after: usize,
        origin: EditOrigin,
    ) {
        let removed = self.document.rope().byte_slice(range.clone()).to_string();
        if removed.is_empty() && inserted.is_empty() {
            return;
        }
        let transaction = EditTransaction::new(
            self.document.revision(),
            vec![TextEdit::replace(
                TextRange::new(range.start, range.end),
                inserted,
                removed,
            )],
            Selection::caret(self.caret_byte),
            Selection::caret(caret_after),
            origin,
            EditTimestamp::new(self.started.elapsed()),
        );
        match self.document.apply_transaction(&transaction) {
            Ok(applied) => {
                self.caret_byte = caret_after;
                if self.history.record(applied)
                    == HistoryRecordOutcome::ClearedForOversizedTransaction
                {
                    self.set_status("Undo history cleared: that edit was too large to keep");
                }
                self.recovery
                    .on_edited(&self.document, Selection::caret(self.caret_byte));
            }
            Err(error) => self.set_status(format!("Edit refused: {error}")),
        }
    }

    /// Returns the caret one character away, deciding from a few characters
    /// of context instead of copying the document.
    ///
    /// A character step moves over one scalar or one whole line ending, so
    /// four characters on each side always hold the answer.
    fn character_step(&self, direction: MoveDirection) -> usize {
        let rope = self.document.rope();
        let caret = self.caret_byte.min(rope.len_bytes());
        let caret_char = rope.byte_to_char(caret);
        let start = rope.char_to_byte(caret_char.saturating_sub(4));
        let end = rope.char_to_byte((caret_char + 4).min(rope.len_chars()));
        let window = rope.byte_slice(start..end).to_string();
        start + move_caret(&window, caret - start, direction, MoveUnit::Character)
    }

    /// Inserts a string at the current caret.
    pub fn insert_str(&mut self, insert: &str) {
        self.insert_with_origin(insert, EditOrigin::TextInput);
    }

    fn insert_with_origin(&mut self, insert: &str, origin: EditOrigin) {
        let caret = self.caret_byte.min(self.document.rope().len_bytes());
        self.replace_range(caret..caret, insert, caret + insert.len(), origin);
    }

    /// Inserts a newline matching the document's line-ending profile.
    pub fn insert_newline(&mut self) {
        let ending = self.document.line_endings().fallback_insertion().as_str();
        self.insert_str(ending);
    }

    /// Deletes the character before the caret (Backspace).
    pub fn delete_backwards(&mut self) {
        let previous = self.character_step(MoveDirection::Backward);
        if previous < self.caret_byte {
            self.replace_range(
                previous..self.caret_byte,
                "",
                previous,
                EditOrigin::TextInput,
            );
        }
    }

    /// Deletes the character at the caret (Delete).
    pub fn delete_forwards(&mut self) {
        let next = self.character_step(MoveDirection::Forward);
        if next > self.caret_byte {
            let caret = self.caret_byte;
            self.replace_range(caret..next, "", caret, EditOrigin::TextInput);
        }
    }

    /// Cuts the current line to the internal clipboard (^K).
    pub fn cut_line(&mut self) {
        let text = self.text();
        if text.is_empty() {
            return;
        }
        let table = LineTable::new(&text);
        let span = table.line(table.line_of(self.caret_byte.min(text.len())));
        let (start, next_start) = (span.start, span.next);

        text[start..next_start].clone_into(&mut self.clipboard);
        self.replace_range(start..next_start, "", start, EditOrigin::Programmatic);
        self.set_status("Cut 1 line to clipboard");
    }

    /// Pastes the clipboard at the current caret (^U).
    pub fn paste(&mut self) {
        if self.clipboard.is_empty() {
            self.set_status("Clipboard is empty");
            return;
        }
        let clip = self.clipboard.clone();
        self.insert_with_origin(&clip, EditOrigin::Paste);
        self.set_status("Pasted from clipboard");
    }

    /// Undoes the last edit transaction (^Z).
    pub fn undo(&mut self) {
        let result = self.history.undo(&mut self.document);
        self.finish_history_step(result, "Undid 1 change", "Already at oldest change");
    }

    /// Redoes the last undone edit transaction (^Y).
    pub fn redo(&mut self) {
        let result = self.history.redo(&mut self.document);
        self.finish_history_step(result, "Redid 1 change", "Already at newest change");
    }

    fn finish_history_step(
        &mut self,
        result: Result<Option<HistoryApplyOutcome>, HistoryError>,
        done: &str,
        nothing: &str,
    ) {
        match result {
            Ok(Some(outcome)) => {
                self.caret_byte = outcome.selection().active();
                self.sync_recovery();
                self.set_status(done);
            }
            Ok(None) => self.set_status(nothing),
            Err(error) => self.set_status(format!("History unavailable: {error}")),
        }
    }

    /// Saves the document in place (^S), or asks for a name when untitled.
    pub fn save(&mut self) -> SaveStep {
        if self.disk_prompt_is_open() {
            return SaveStep::AwaitingInput;
        }
        if self.save_is_blocked() {
            self.set_uncertain_guidance();
            return SaveStep::NotSaved;
        }
        let Some(target) = self.document.path().map(std::path::Path::to_path_buf) else {
            return self.open_save_as();
        };
        let Some(reservation) = self.reserve_save_recovery(target) else {
            return SaveStep::NotSaved;
        };
        match self.document.save() {
            Err(NoterError::HardLinkedTarget(link_count)) => {
                self.pending_save = Some(PendingSave::SplitHardLinkInPlace(reservation));
                self.prompt = PromptMode::ConfirmHardLink(link_count);
                SaveStep::AwaitingInput
            }
            result => self.report_save(reservation, result),
        }
    }

    /// Opens the Save As prompt (^O), prefilled with the current path.
    pub fn open_save_as(&mut self) -> SaveStep {
        if self.save_is_blocked() {
            self.set_uncertain_guidance();
            return SaveStep::NotSaved;
        }
        self.prompt_input = self
            .document
            .path()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        self.prompt = PromptMode::SaveAs;
        SaveStep::AwaitingInput
    }

    /// Resolves the name typed into the Save As prompt.
    pub fn submit_save_as(&mut self) -> SaveStep {
        if self.save_is_blocked() {
            self.prompt = PromptMode::None;
            self.set_uncertain_guidance();
            return SaveStep::NotSaved;
        }
        if self.prompt_input.trim().is_empty() {
            self.set_status("Enter a file name, or press Esc to cancel");
            return SaveStep::AwaitingInput;
        }
        self.prompt = PromptMode::None;
        // The prompt shows the current path lossily; unchanged input means
        // that exact path, even when its name is not valid UTF-8.
        let path = match self.document.path() {
            Some(current) if current.to_string_lossy() == self.prompt_input.as_str() => {
                current.to_path_buf()
            }
            _ => PathBuf::from(&self.prompt_input),
        };
        let Some(reservation) = self.reserve_save_recovery(path.clone()) else {
            return SaveStep::NotSaved;
        };
        let prepared = match self.document.prepare_save_as(&path) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.set_save_error(format!("Not saved: {error}"));
                if self.reviewed_disk.is_some() && self.conflict.is_prompting() {
                    self.show_disk_conflict();
                }
                return SaveStep::NotSaved;
            }
        };
        // Saving to the document's own path is an ordinary save: the core
        // compares against the saved baseline and reports an external change
        // as a conflict, so there is nothing new to confirm.
        let is_current_path = self.document.path() == Some(path.as_path());
        if prepared.replaces_existing() && !is_current_path {
            self.pending_save = Some(PendingSave::Replace(prepared, reservation));
            self.prompt = PromptMode::ConfirmReplace;
            return SaveStep::AwaitingInput;
        }
        self.save_prepared(prepared, reservation, false)
    }

    const fn save_is_blocked(&self) -> bool {
        !self.save_recoveries.is_empty()
    }

    fn reserve_save_recovery(&mut self, destination: PathBuf) -> Option<SaveRecoveryReservation> {
        if self.save_recoveries.len() >= MAX_SAVE_RECOVERY_RECORDS
            || self.save_recoveries.try_reserve(1).is_err()
        {
            self.set_save_error("Save stopped: uncertain-save evidence cannot be retained");
            return None;
        }
        if destination.as_os_str().as_encoded_bytes().len() > MAX_SAVE_RECOVERY_DESTINATION_BYTES {
            self.set_save_error("Save stopped: destination path is too large to retain safely");
            return None;
        }
        let Some(destination_label) = bounded_destination_label(&destination) else {
            self.set_save_error("Save stopped: destination label cannot be retained");
            return None;
        };
        let mut message = String::new();
        if message
            .try_reserve_exact(MAX_SAVE_RECOVERY_MESSAGE_BYTES)
            .is_err()
        {
            self.set_save_error("Save stopped: uncertain-save diagnostic cannot be retained");
            return None;
        }
        Some(SaveRecoveryReservation {
            destination,
            destination_label,
            message,
        })
    }

    fn open_save_reconciliation(&mut self) {
        if self.save_recoveries.is_empty() {
            if self.last_save_warnings.is_some() || self.recovery.is_unavailable() {
                self.prompt = PromptMode::SaveWarnings;
                self.reconcile_scroll = 0;
                self.retained_sibling_removal = RetainedSiblingRemoval::Idle;
            } else {
                self.set_status("No save warnings or uncertain outcomes to review");
            }
            return;
        }
        self.prompt = PromptMode::ReconcileSave(0);
        self.reconcile_scroll = 0;
    }

    pub(super) const fn retained_removal_is_armed(&self) -> bool {
        matches!(self.retained_sibling_removal, RetainedSiblingRemoval::Armed)
    }

    fn remove_confirmed_retained_siblings(&mut self) {
        let Some(warnings) = self.last_save_warnings.clone() else {
            self.set_status("No verified single-link retained sibling is available to remove");
            return;
        };
        if warnings.retained().is_empty() {
            self.set_status("No verified single-link retained sibling is available to remove");
            return;
        }
        let (count, remaining) = remove_verified_retained_siblings(warnings.retained());
        self.last_save_warnings = Some(warnings.with_retained(remaining));
        self.set_status(retained_sibling_removal_message(count));
    }

    fn scroll_save_details(&mut self, direction: MoveDirection, cols: u16, rows: u16) {
        let columns = usize::from(cols);
        let lines = match self.prompt {
            PromptMode::SaveWarnings => save_warnings::lines(self, columns),
            PromptMode::ReconcileSave(index) => save_reconciliation_rows(self, index, columns)
                .into_iter()
                .map(|(text, _)| text)
                .collect(),
            _ => return,
        };
        let maximum = lines
            .len()
            .saturating_sub(usize::from(rows).saturating_sub(2));
        let current = self.reconcile_scroll.min(maximum);
        self.reconcile_scroll = match direction {
            MoveDirection::Forward => current.saturating_add(1).min(maximum),
            MoveDirection::Backward => current.saturating_sub(1),
        };
    }

    fn reconcile_save(&mut self, index: usize) {
        if index >= self.save_recoveries.len() {
            self.prompt = PromptMode::None;
            return;
        }
        self.save_recoveries.remove(index);
        self.reconcile_scroll = 0;
        if self.save_recoveries.is_empty() {
            self.prompt = PromptMode::None;
            if self
                .status_message
                .as_ref()
                .is_some_and(|(message, _)| message == UNCERTAIN_SAVE_GUIDANCE)
            {
                self.status_message = None;
            }
        } else {
            self.prompt = PromptMode::ReconcileSave(index.min(self.save_recoveries.len() - 1));
        }
    }

    fn copy_recovery_destination(&mut self, index: usize) {
        if let Some(recovery) = self.save_recoveries.get(index) {
            self.pending_clipboard = Some(recovery_path_clipboard_text(&recovery.destination));
            self.set_status("Exact path copy requested; terminal clipboard support varies");
        }
    }

    /// Answers the replace or hard-link confirmation that is showing.
    pub fn confirm_pending_save(&mut self, confirmed: bool) -> SaveStep {
        self.prompt = PromptMode::None;
        if self.save_is_blocked() {
            self.pending_save = None;
            self.set_uncertain_guidance();
            return SaveStep::NotSaved;
        }
        let Some(pending) = self.pending_save.take() else {
            return SaveStep::NotSaved;
        };
        if !confirmed {
            if self.reviewed_disk.is_some() && self.conflict.is_prompting() {
                self.show_disk_conflict();
            } else {
                self.set_save_error("Not saved");
            }
            return SaveStep::NotSaved;
        }
        match pending {
            PendingSave::SplitHardLinkInPlace(reservation) => {
                let result = self.document.save_confirming_hard_link_replacement();
                self.report_save(reservation, result)
            }
            PendingSave::Replace(prepared, reservation) => {
                self.save_prepared(prepared, reservation, false)
            }
            PendingSave::SplitHardLink(prepared, reservation) => {
                self.save_prepared(prepared, reservation, true)
            }
        }
    }

    fn save_prepared(
        &mut self,
        prepared: PreparedSaveAs,
        reservation: SaveRecoveryReservation,
        hard_link_confirmed: bool,
    ) -> SaveStep {
        if self.save_is_blocked() {
            self.set_uncertain_guidance();
            return SaveStep::NotSaved;
        }
        let result = if hard_link_confirmed {
            self.document
                .save_prepared_as_confirming_hard_link_replacement(prepared.clone())
        } else {
            self.document.save_prepared_as(prepared.clone())
        };
        if let Err(NoterError::HardLinkedTarget(link_count)) = result {
            self.pending_save = Some(PendingSave::SplitHardLink(prepared, reservation));
            self.prompt = PromptMode::ConfirmHardLink(link_count);
            return SaveStep::AwaitingInput;
        }
        self.report_save(reservation, result)
    }

    /// Records a conflict and shows reload, keep, Save As, and maybe overwrite.
    ///
    /// Keep Editing does not rebaseline. The next Save conflicts again and asks
    /// for a decision. An explicit Save resets a previous Keep so the terminal,
    /// which does not watch the file while idle, is not a dead end.
    fn begin_disk_conflict(
        &mut self,
        revision: Revision,
        expected: TargetExpectation,
        actual: TargetState,
        cleanup_failed: bool,
    ) {
        let kind = classify_external_change(Some(expected), Some(Ok(actual)));
        if !kind.requires_prompt() {
            self.clear_external_retention();
            self.dismiss_disk_conflict();
            self.set_status(disk_conflict::MATCHES_AGAIN);
            return;
        }
        self.reviewed_disk = Some(ReviewedDisk {
            expected,
            actual,
            cleanup_failed,
        });
        self.conflict.reduce(ConflictCommand::Reset);
        self.conflict.reduce(ConflictCommand::ObservedExact {
            kind,
            evidence: Ok(actual),
            revision,
        });
        // The bytes on disk are no longer the trusted baseline. Keep the
        // open text even when it still matches the last save.
        self.note_external_retention();
        self.show_disk_conflict();
    }

    fn show_disk_conflict(&mut self) {
        if self.reviewed_disk.is_none() || self.conflict.prompt_kind().is_none() {
            return;
        }
        let notice = self.disk_prompt_notice();
        self.prompt = PromptMode::DiskChange;
        self.prompt_input.clear();
        self.set_status(notice);
    }

    fn disk_prompt_notice(&self) -> &'static str {
        let Some(reviewed) = self.reviewed_disk else {
            return disk_conflict::notice(disk_conflict::NoticeFacts {
                kind: ExternalChangeKind::Unchanged,
                metadata_only: false,
                cleanup_failed: false,
                overwrite_allowed: false,
            });
        };
        let kind = self
            .conflict
            .prompt_kind()
            .unwrap_or(ExternalChangeKind::Unchanged);
        disk_conflict::notice(disk_conflict::NoticeFacts {
            kind,
            metadata_only: disk_conflict::metadata_only(&reviewed),
            cleanup_failed: reviewed.cleanup_failed,
            overwrite_allowed: self.overwrite_allowed(),
        })
    }

    fn overwrite_allowed(&self) -> bool {
        let Some(reviewed) = self.reviewed_disk else {
            return false;
        };
        !self.save_is_blocked()
            && self.document.path().is_some()
            && matches!(reviewed.actual, TargetState::Regular(_))
            && matches!(
                self.conflict.prompt_kind(),
                Some(ExternalChangeKind::ContentOrIdentityChanged)
            )
    }

    const fn disk_prompt_is_open(&self) -> bool {
        matches!(
            self.prompt,
            PromptMode::DiskChange
                | PromptMode::ConfirmDiskOverwrite
                | PromptMode::ConfirmDiskReload
        )
    }

    fn dismiss_disk_conflict(&mut self) {
        self.reviewed_disk = None;
        self.conflict.reduce(ConflictCommand::Reset);
        if self.disk_prompt_is_open() {
            self.prompt = PromptMode::None;
        }
    }

    fn keep_disk_text(&mut self) {
        self.conflict
            .reduce(ConflictCommand::Decide(ConflictDecision::KeepEditing));
        // A stale keep leaves a visible confirm in place. An accepted keep
        // does not rebaseline, and the open text stays retained.
        if self.conflict.is_prompting() {
            return;
        }
        self.reviewed_disk = None;
        self.prompt = PromptMode::None;
        self.prompt_input.clear();
        self.after_save = AfterSave::Stay;
        self.should_exit = false;
        self.set_status(disk_conflict::KEPT);
    }

    fn request_disk_reload(&mut self) -> SaveStep {
        let effect = self
            .conflict
            .reduce(ConflictCommand::Decide(ConflictDecision::ReloadDisk));
        if !matches!(effect, ConflictEffect::RequestReload) {
            self.show_disk_conflict();
            return SaveStep::AwaitingInput;
        }
        // A clean document reloads immediately, as the window does. Typed
        // edits get a second confirmation. External retention is not a
        // second copy of text the user typed.
        if self.document.is_dirty() {
            self.prompt = PromptMode::ConfirmDiskReload;
            self.set_status(disk_conflict::RELOAD_CONFIRM);
            SaveStep::AwaitingInput
        } else {
            self.reload_disk_version()
        }
    }

    fn request_disk_overwrite(&mut self) {
        if !self.overwrite_allowed() {
            return;
        }
        let effect = self
            .conflict
            .reduce(ConflictCommand::Decide(ConflictDecision::RequestOverwrite));
        if matches!(effect, ConflictEffect::PromptOverwriteConfirm(_)) {
            self.prompt = PromptMode::ConfirmDiskOverwrite;
            self.set_status(disk_conflict::OVERWRITE_CONFIRM);
        }
    }

    fn cancel_disk_overwrite(&mut self) {
        self.conflict
            .reduce(ConflictCommand::Decide(ConflictDecision::CancelOverwrite));
        self.show_disk_conflict();
    }

    /// Replaces the disk file only when the second confirmation's observation is still current.
    fn confirm_disk_overwrite(&mut self) -> SaveStep {
        if self.save_is_blocked() {
            self.show_disk_conflict();
            self.set_uncertain_guidance();
            return SaveStep::NotSaved;
        }
        let ConflictEffect::AuthorizeOverwrite(reviewed) = self
            .conflict
            .reduce(ConflictCommand::Decide(ConflictDecision::ConfirmOverwrite))
        else {
            self.show_disk_conflict();
            return SaveStep::AwaitingInput;
        };
        let Some(path) = self.document.path().map(std::path::Path::to_path_buf) else {
            self.dismiss_disk_conflict();
            self.set_status(disk_conflict::RELOAD_FAILED);
            return SaveStep::NotSaved;
        };
        match inspect_target(&path, SaveStage::InspectInitial) {
            Ok(TargetState::Regular(observation)) if observation == reviewed => {
                self.write_over_reviewed_disk(observation)
            }
            Ok(observed) => self.note_disk_changed_again(observed),
            Err(_) => self.note_disk_unreadable(),
        }
    }

    fn write_over_reviewed_disk(&mut self, observation: FileObservation) -> SaveStep {
        // Rebaseline only after the second confirmation and a matching re-read.
        // A declined hard-link prompt leaves this baseline, as the window does.
        self.document.rebaseline_to_observed_disk(observation);
        self.reviewed_disk = None;
        self.prompt = PromptMode::None;
        self.save()
    }

    fn note_disk_changed_again(&mut self, observed: TargetState) -> SaveStep {
        let Some(mut reviewed) = self.reviewed_disk else {
            self.dismiss_disk_conflict();
            self.set_status(disk_conflict::CHANGED_AGAIN);
            return SaveStep::AwaitingInput;
        };
        let kind = classify_external_change(Some(reviewed.expected), Some(Ok(observed)));
        reviewed.actual = observed;
        self.reviewed_disk = Some(reviewed);
        if !kind.requires_prompt() {
            self.clear_external_retention();
            self.dismiss_disk_conflict();
            self.set_status(disk_conflict::MATCHES_AGAIN);
            return SaveStep::NotSaved;
        }
        self.note_external_retention();
        self.conflict.reduce(ConflictCommand::ObservedExact {
            kind,
            evidence: Ok(observed),
            revision: self.document.revision(),
        });
        self.show_disk_conflict();
        self.set_status(disk_conflict::CHANGED_AGAIN);
        SaveStep::AwaitingInput
    }

    fn note_disk_unreadable(&mut self) -> SaveStep {
        self.conflict.reduce(ConflictCommand::ObservedExact {
            kind: ExternalChangeKind::Unreadable,
            evidence: Err(()),
            revision: self.document.revision(),
        });
        self.show_disk_conflict();
        self.set_status(disk_conflict::UNREADABLE_OVERWRITE);
        SaveStep::AwaitingInput
    }

    fn reload_disk_version(&mut self) -> SaveStep {
        let Some(path) = self.document.path().map(std::path::Path::to_path_buf) else {
            self.fail_disk_reload();
            return SaveStep::AwaitingInput;
        };
        // A load error can name the path. Keep the fixed sentence instead.
        let Ok(loaded) = Document::from_path(&path) else {
            self.fail_disk_reload();
            return SaveStep::AwaitingInput;
        };
        if self.has_unsaved_state() {
            self.recovery.on_discarded();
        }
        self.install_loaded_document(loaded);
        SaveStep::NotSaved
    }

    fn fail_disk_reload(&mut self) {
        self.show_disk_conflict();
        self.set_status(disk_conflict::RELOAD_FAILED);
    }

    fn install_loaded_document(&mut self, document: Document) {
        let revision = document.revision();
        self.document = document;
        self.caret_byte = 0;
        self.scroll_row = 0;
        self.scroll_col = 0;
        self.follow_caret = true;
        self.history.reset(revision);
        self.save_durability = SaveDurabilityRisk::Clear;
        self.pending_save = None;
        self.reviewed_disk = None;
        self.external_retention = ExternalRetention::Clear;
        self.conflict.reduce(ConflictCommand::Reset);
        self.prompt = PromptMode::None;
        self.prompt_input.clear();
        self.after_save = AfterSave::Stay;
        self.should_exit = false;
        self.sync_recovery();
        self.set_status(disk_conflict::LOADED);
    }

    /// Turns a save result into status text and the next step.
    fn report_save(
        &mut self,
        reservation: SaveRecoveryReservation,
        result: Result<SaveOutcome, NoterError>,
    ) -> SaveStep {
        match result {
            Ok(SaveOutcome::Committed {
                observation,
                warnings,
                ..
            }) => {
                self.external_retention = ExternalRetention::Clear;
                self.dismiss_disk_conflict();
                let warning_count = warnings.cleanup().len() + warnings.durability().len();
                self.save_durability = SaveDurabilityRisk::from_warnings(&warnings);
                if self.save_durability.is_at_risk() {
                    self.recovery
                        .on_retained(&self.document, Selection::caret(self.caret_byte));
                } else {
                    self.recovery.on_committed_save(self.document.revision());
                    if self.recovery.active_offer().is_some() {
                        self.prompt = PromptMode::RecoveryOffer;
                    }
                }
                if self.save_durability.is_at_risk() {
                    self.set_toned_status(
                        "Saved with uncertain durability. Keep open and Save again. ^R details.",
                        StatusTone::Warning,
                    );
                } else if warning_count == 0 {
                    self.set_toned_status(
                        format!("Wrote {} bytes", observation.length()),
                        StatusTone::Success,
                    );
                } else {
                    self.set_toned_status(
                        format!(
                            "Wrote {} bytes with {warning_count} storage warning(s). ^R details.",
                            observation.length()
                        ),
                        StatusTone::Warning,
                    );
                }
                self.last_save_warnings = (!warnings.is_empty()).then_some(warnings);
                SaveStep::Committed
            }
            Ok(SaveOutcome::Conflict {
                revision,
                expected,
                actual,
                cleanup_error,
            }) => {
                self.begin_disk_conflict(revision, expected, actual, cleanup_error.is_some());
                SaveStep::NotSaved
            }
            Ok(SaveOutcome::NotCommitted { cleanup_error, .. }) => {
                // The engine diagnostic stays on the outcome for recovery.
                // The status names the folder instead of that diagnostic.
                self.dismiss_disk_conflict();
                self.set_save_error(crate::save_status::not_committed_status(
                    &reservation.destination,
                    cleanup_error.as_ref(),
                ));
                SaveStep::NotSaved
            }
            Ok(SaveOutcome::CommitStateUnknown {
                error,
                recovery_artifact,
                ..
            }) => {
                self.dismiss_disk_conflict();
                let SaveRecoveryReservation {
                    destination,
                    destination_label,
                    message,
                } = reservation;
                let message = write_save_recovery_message(message, &recovery_artifact, &error);
                self.save_recoveries.push(SaveRecoveryRecord {
                    destination,
                    destination_label,
                    message,
                });
                self.set_uncertain_guidance();
                SaveStep::NotSaved
            }
            Err(error) => {
                self.dismiss_disk_conflict();
                self.set_save_error(format!("Not saved: {error}"));
                SaveStep::NotSaved
            }
        }
    }

    /// Applies a save step to a pending exit.
    ///
    /// Exit happens only after a commit. Any other result cancels the pending
    /// exit so the text stays open.
    pub fn finish_save(&mut self, step: SaveStep) {
        match step {
            SaveStep::Committed => {
                if self.after_save == AfterSave::Exit
                    && !self.save_durability.is_at_risk()
                    && self.recovery.active_offer().is_none()
                {
                    self.should_exit = true;
                } else if self.save_durability.is_at_risk()
                    || self.recovery.active_offer().is_some()
                {
                    self.after_save = AfterSave::Stay;
                }
            }
            SaveStep::NotSaved => self.after_save = AfterSave::Stay,
            SaveStep::AwaitingInput => {}
        }
    }

    /// Toggles between plain text and formatted Markdown mode (^E).
    pub fn toggle_view(&mut self) {
        self.view = match self.view {
            DocumentView::Text => DocumentView::Markdown,
            DocumentView::Markdown => DocumentView::Text,
        };
        self.set_status(format!("Mode: {}", self.view.label()));
    }

    /// Moves the caret to the next match of the search query (^W or ^F).
    ///
    /// Matching uses the core literal search with Unicode case folding, and
    /// starts just after the caret so repeating the search advances.
    pub fn find_next(&mut self) {
        let Some(query) = self.search_query.clone() else {
            return;
        };
        let search = match LiteralSearch::new(&query, MatchCase::Insensitive) {
            Ok(search) => search,
            Err(error) => {
                self.set_status(format!("Cannot search: {error}"));
                return;
            }
        };
        let text = self.text();
        match search.navigate(&text, self.caret_byte + 1, SearchDirection::Next) {
            Some(found) => {
                self.caret_byte = found.range().start();
                self.follow_caret = true;
                let wrapped = if found.wrapped() { " (wrapped)" } else { "" };
                self.set_status(format!(
                    "Match {} of {}{wrapped}",
                    found.ordinal(),
                    found.match_count()
                ));
            }
            None => self.set_status(format!("Not found: {query}")),
        }
    }

    /// Scrolls so the caret is visible, unless the wheel moved the view.
    pub fn scroll_to_caret(&mut self, cols: u16, rows: u16) {
        if !self.follow_caret {
            return;
        }
        let text = self.text();
        let table = LineTable::new(&text);
        let (line, column) = table.caret(&text, self.caret_byte.min(text.len()));
        let height = viewport_height(rows);
        let width = (cols as usize)
            .saturating_sub(text_offset(table.len()))
            .max(1);
        if line < self.scroll_row {
            self.scroll_row = line;
        } else if line >= self.scroll_row + height {
            self.scroll_row = line + 1 - height;
        }
        if column < self.scroll_col {
            self.scroll_col = column;
        } else if column >= self.scroll_col + width {
            self.scroll_col = column + 1 - width;
        }
    }

    /// Jumps to a 1-based line number (^J).
    pub fn jump_to_line(&mut self, line_1based: usize) {
        let text = self.text();
        match line_start_offset(&text, line_1based) {
            Ok(offset) => {
                self.caret_byte = offset;
                self.set_status(format!("Jumped to line {line_1based}"));
            }
            Err(LineNavigationError::OutOfRange { maximum, .. }) => {
                self.set_status(format!("Line out of range (max {maximum})"));
            }
            Err(LineNavigationError::Zero) => {
                self.set_status("Line numbers start at 1");
            }
        }
    }
}

/// Longest wait for a recovery write when the terminal goes away.
const EXIT_PERSIST_LIMIT: Duration = Duration::from_secs(2);

/// How long a status message stays in place of the position summary.
const STATUS_LIFETIME: Duration = Duration::from_secs(3);

/// Shown when an earlier save may have reached disk.
const UNCERTAIN_SAVE_GUIDANCE: &str =
    "Every save is paused. Press ^R to inspect and reconcile the uncertain outcome.";

/// One logical line: its content bytes and where the next line starts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct LineSpan {
    start: usize,
    end: usize,
    next: usize,
}

/// The document's logical lines, split exactly as the core splits them.
///
/// Rope line indexing also breaks at Unicode separators such as U+2028, which
/// the core navigation does not, so the terminal interface uses this table
/// for every line and column decision.
struct LineTable {
    spans: Vec<LineSpan>,
}

impl LineTable {
    fn new(text: &str) -> Self {
        let mut spans = Vec::new();
        let mut start = 0;
        // Empty text, or text ending in a terminator, has a final empty line.
        let mut open_final_line = true;
        for segment in logical_lines(text) {
            let end = start + segment.content().len();
            let next = end + segment.ending().map_or(0, |ending| ending.as_str().len());
            spans.push(LineSpan { start, end, next });
            open_final_line = segment.ending().is_some();
            start = next;
        }
        if open_final_line {
            spans.push(LineSpan {
                start: text.len(),
                end: text.len(),
                next: text.len(),
            });
        }
        Self { spans }
    }

    const fn len(&self) -> usize {
        self.spans.len()
    }

    /// Returns the line at `index`, or the last line when past the end.
    fn line(&self, index: usize) -> LineSpan {
        self.spans[index.min(self.spans.len() - 1)]
    }

    /// Returns the index of the line containing `offset`.
    fn line_of(&self, offset: usize) -> usize {
        self.spans
            .partition_point(|span| span.start <= offset)
            .saturating_sub(1)
    }

    /// Returns the caret's line index and display column.
    fn caret(&self, text: &str, offset: usize) -> (usize, usize) {
        let index = self.line_of(offset);
        let span = self.spans[index];
        let column = column_of(
            &text[span.start..span.end],
            offset.min(span.end) - span.start,
        );
        (index, column)
    }
}

/// Screen rows used by the header, status line, and two legend rows.
const CHROME_ROWS: usize = 4;

/// Narrowest terminal that fits the line-number gutter and some text.
const MIN_COLUMNS: usize = 20;

/// Cells before the text: a space, the line number, " │", and a space.
fn text_offset(line_count: usize) -> usize {
    let digits = line_count.max(1).ilog10() as usize + 1;
    digits.max(4) + 4
}

fn viewport_height(rows: u16) -> usize {
    (rows as usize).saturating_sub(CHROME_ROWS).max(1)
}

/// How a run of document text is drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Style {
    Text,
    Match,
    Heading,
    Marker,
    Quote,
    Muted,
}

impl Style {
    const fn role(self) -> Role {
        match self {
            Self::Text => Role::Text,
            Self::Match => Role::Match,
            Self::Heading => Role::AccentStrong,
            Self::Marker => Role::Accent,
            Self::Quote => Role::Quote,
            Self::Muted => Role::Muted,
        }
    }
}

fn push_style(out: &mut String, paint: Paint, style: Style) {
    paint.open(out, style.role());
}

/// Styles Markdown structure on one line without changing a single cell, so
/// columns, the caret, and mouse positions match the source exactly.
fn markdown_spans(line: &str) -> Vec<(std::ops::Range<usize>, Style)> {
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    let body = &line[indent..];
    let hashes = body.bytes().take_while(|byte| *byte == b'#').count();
    if (1..=6).contains(&hashes) && body[hashes..].starts_with(' ') {
        return vec![(0..line.len(), Style::Heading)];
    }
    if body.starts_with("```") || body.starts_with("~~~") {
        return vec![(0..line.len(), Style::Muted)];
    }
    if body.starts_with('>') {
        return vec![
            (indent..indent + 1, Style::Muted),
            (indent + 1..line.len(), Style::Quote),
        ];
    }
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    let marker = if body.starts_with("- ") || body.starts_with("* ") || body.starts_with("+ ") {
        1
    } else if (1..=9).contains(&digits)
        && (body[digits..].starts_with(". ") || body[digits..].starts_with(") "))
    {
        digits + 1
    } else {
        0
    };
    if marker > 0 {
        return vec![(indent..indent + marker, Style::Marker)];
    }
    Vec::new()
}

/// Draws the cells of `line` from `first_column` for `width` cells.
///
/// Every character goes through the terminal-safe display rules. A wide
/// character or tab cut by either edge is drawn as spaces, so each row is
/// exactly `width` cells.
fn push_line_window(
    out: &mut String,
    line: &str,
    first_column: usize,
    width: usize,
    spans: &[(std::ops::Range<usize>, Style)],
    paint: Paint,
) {
    let last_column = first_column + width;
    let mut column = 0;
    let mut drawn = 0;
    let mut current = None;
    for (offset, character) in line.char_indices() {
        if column >= last_column {
            break;
        }
        let cells = cell_width(character, column);
        if cells == 0 {
            // A combining mark, joiner, or variation selector attaches to the
            // character before it and is drawn only when that one was.
            if column > first_column && drawn > 0 {
                push_display(out, character, column);
            }
            continue;
        }
        let visible_start = column.max(first_column);
        let visible_end = (column + cells).min(last_column);
        if visible_end > visible_start {
            let style = spans
                .iter()
                .rev()
                .find(|(range, _)| range.contains(&offset))
                .map_or(Style::Text, |(_, style)| *style);
            if current != Some(style) {
                push_style(out, paint, style);
                current = Some(style);
            }
            if visible_end - visible_start == cells {
                push_display(out, character, column);
            } else {
                out.extend(std::iter::repeat_n(' ', visible_end - visible_start));
            }
            drawn += visible_end - visible_start;
        }
        column += cells;
    }
    if current != Some(Style::Text) {
        push_style(out, paint, Style::Text);
    }
    out.extend(std::iter::repeat_n(' ', width - drawn));
}

/// Returns the status-line prompt as a label, the typed input, and a hint.
fn prompt_parts(session: &TuiSession) -> Option<(String, &str, &'static str)> {
    let (label, input, hint) = match &session.prompt {
        PromptMode::SaveAs => (
            "Save As: ",
            session.prompt_input.as_str(),
            " (Enter to save, Esc to cancel)",
        ),
        PromptMode::Find => (
            "Find: ",
            session.prompt_input.as_str(),
            " (Enter next, Esc cancel)",
        ),
        PromptMode::GoToLine => (
            "Go to line: ",
            session.prompt_input.as_str(),
            " (Enter jump, Esc cancel)",
        ),
        PromptMode::ConfirmReplace => ("That file exists. Replace it? (y)es, (n)o", "", ""),
        PromptMode::ConfirmHardLink(link_count) => {
            return Some((
                format!(
                    "This file has {link_count} hard links. Save here only; the others keep the old text? (y)es, (n)o"
                ),
                "",
                "",
            ));
        }
        PromptMode::ExitConfirm => (
            "Save modified buffer before exit? (y)es, (n)o, (c)ancel",
            "",
            "",
        ),
        PromptMode::RecoveryOffer => {
            let label = session.recovery.active_offer().map_or_else(
                || "Untitled".to_owned(),
                crate::crash_recovery::StartupRecoveryOffer::original_path_label,
            );
            // The decision keys come first so an 80-column bar cannot cut
            // them off when the folder and time make the label long.
            return Some((
                format!("(r)estore, (d)iscard, (l)ater. Unsaved text from {label}."),
                "",
                "",
            ));
        }
        PromptMode::ReconcileSave(index) => {
            return Some((
                format!(
                    "Uncertain save {}/{}. Inspect, (c)opy path, (y)es reconciled, (n)ext, (p)revious, Esc cancel",
                    index + 1,
                    session.save_recoveries.len()
                ),
                "",
                "",
            ));
        }
        PromptMode::DiskChange => {
            return Some((session.disk_prompt_notice().to_owned(), "", ""));
        }
        PromptMode::ConfirmDiskOverwrite => {
            return Some((disk_conflict::OVERWRITE_CONFIRM.to_owned(), "", ""));
        }
        PromptMode::ConfirmDiskReload => {
            return Some((disk_conflict::RELOAD_CONFIRM.to_owned(), "", ""));
        }
        PromptMode::None | PromptMode::SaveWarnings => return None,
    };
    Some((label.to_owned(), input, hint))
}

/// The text one frame draws, its lines, and where the caret is.
struct Frame<'a> {
    session: &'a TuiSession,
    text: &'a str,
    table: LineTable,
    caret_line: usize,
    caret_column: usize,
}

impl<'a> Frame<'a> {
    fn new(session: &'a TuiSession, text: &'a str) -> Self {
        let table = LineTable::new(text);
        let (caret_line, caret_column) = table.caret(text, session.caret_byte.min(text.len()));
        Self {
            session,
            text,
            table,
            caret_line,
            caret_column,
        }
    }
}

/// Draws the document rows, with line numbers, Markdown styling, and
/// search highlights, from the session's scroll position.
fn push_viewport(out: &mut String, frame: &Frame<'_>, columns: usize, height: usize, paint: Paint) {
    let &Frame {
        session,
        text,
        ref table,
        ..
    } = frame;
    let offset = text_offset(table.len());
    let digits = offset - 4;
    let text_width = columns.saturating_sub(offset);
    let search = session
        .search_query
        .as_deref()
        .and_then(|query| LiteralSearch::new(query, MatchCase::Insensitive).ok());
    for row in 0..height {
        let index = session.scroll_row + row;
        if index < table.len() {
            let span = table.line(index);
            let line = &text[span.start..span.end];
            paint.open(out, Role::Muted);
            let _ = write!(out, " {:>digits$} │ ", index + 1);
            let mut spans = if session.view == DocumentView::Markdown {
                markdown_spans(line)
            } else {
                Vec::new()
            };
            if let Some(search) = &search {
                spans.extend(
                    search
                        .ranges(line)
                        .map(|range| (range.start()..range.end(), Style::Match)),
                );
            }
            push_line_window(out, line, session.scroll_col, text_width, &spans, paint);
        } else {
            paint.open(out, Role::Muted);
            out.push_str(&fit_line(&format!(" {:>digits$} ", "~"), columns));
        }
        out.push_str("\x1b[0m\r\n");
    }
}

const fn tone_role(tone: StatusTone) -> Role {
    match tone {
        StatusTone::Plain => Role::BarStrong,
        StatusTone::Success => Role::Success,
        StatusTone::Warning => Role::Warning,
        StatusTone::Error => Role::Error,
    }
}

/// Returns the status line and the role that draws it.
fn status_line(frame: &Frame<'_>) -> (String, Role) {
    let &Frame {
        session,
        text,
        ref table,
        caret_line,
        caret_column,
    } = frame;
    if let Some((label, input, hint)) = prompt_parts(session) {
        return (format!("{label}{input}{hint}"), Role::BarStrong);
    }
    if let Some((message, created)) = &session.status_message
        && created.elapsed() < STATUS_LIFETIME
    {
        return (format!(" {message}"), tone_role(session.status_tone));
    }
    if let Some(hint) = &session.recovery_unavailable_hint {
        return (format!(" {hint}"), Role::Warning);
    }
    if let Some(hint) = &session.recovery_path_hint {
        return (format!(" {hint}"), Role::BarStrong);
    }
    (
        format!(
            " Ln {}, Col {}  │  {} B  │  {} L  │  {}  │  {} ",
            caret_line + 1,
            caret_column + 1,
            text.len(),
            table.len(),
            session.document.encoding().status_label(),
            session.document.line_endings().status_label()
        ),
        Role::BarStrong,
    )
}

/// Renders the complete TUI frame into an output buffer.
///
/// The frame uses the scroll position stored in the session; call
/// [`TuiSession::scroll_to_caret`] first to keep the caret visible.
pub fn render_frame(session: &TuiSession, cols: u16, rows: u16) -> String {
    let columns = cols as usize;
    let mut out = String::with_capacity(columns * rows as usize * 4);
    let paint = session.terminal_color.paint();
    out.push_str("\x1b[?25l\x1b[H");
    if usize::from(rows) <= CHROME_ROWS || columns < MIN_COLUMNS {
        // Too small for the layout: say so in what fits instead of drawing
        // rows that wrap or scroll the terminal.
        out.push_str("\x1b[0m\x1b[2J\x1b[H");
        out.push_str(&fit_line("Enlarge the terminal", columns));
        return out;
    }

    if let PromptMode::ReconcileSave(index) = &session.prompt {
        return render_save_reconciliation(session, *index, cols, rows, paint);
    }
    if session.prompt == PromptMode::SaveWarnings {
        return save_warnings::render(session, cols, rows, paint);
    }

    let text = session.text();
    let frame = Frame::new(session, &text);
    let (caret_line, caret_column) = (frame.caret_line, frame.caret_column);
    let offset = text_offset(frame.table.len());
    let text_width = columns.saturating_sub(offset);
    let height = viewport_height(rows);

    let doc_name = session.document.path().map_or_else(
        || "Untitled".to_owned(),
        |path| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        },
    );
    // Dirty edits, uncertain durability, and a divergent disk version all
    // mark the title. Exit uses the same signal.
    let dirty = if session.has_unsaved_state() {
        " *"
    } else {
        ""
    };
    let mode = format!("[{}]", session.view.label());
    let header = format!(
        " Noter {} │ {doc_name}{dirty} │ {mode} ",
        env!("CARGO_PKG_VERSION")
    );
    push_mode_header(&mut out, &header, &mode, columns, paint);
    out.push_str("\r\n");

    push_viewport(&mut out, &frame, columns, height, paint);

    let (status, status_role) = status_line(&frame);
    push_bar(&mut out, &status, columns, paint, status_role);
    out.push_str("\r\n");
    push_legends(&mut out, columns, paint);

    // The cursor stays hidden unless it has a place: over help, or with the
    // caret scrolled out of view, a visible cursor would mark a wrong cell.
    if session.show_help {
        render_help_overlay(&mut out, cols, rows, paint);
    } else if let Some((label, input, _)) = prompt_parts(session) {
        let column = 1 + display_width(&label) + display_width(input);
        let _ = write!(
            out,
            "\x1b[{};{}H\x1b[?25h",
            (rows as usize).saturating_sub(2),
            column.min(columns)
        );
    } else if (session.scroll_row..session.scroll_row + height).contains(&caret_line)
        && (session.scroll_col..session.scroll_col + text_width).contains(&caret_column)
    {
        let _ = write!(
            out,
            "\x1b[{};{}H\x1b[?25h",
            2 + caret_line - session.scroll_row,
            1 + offset + caret_column - session.scroll_col
        );
    }
    out
}

/// Draws the title bar and paints the current mode in the accent role.
///
/// The mode stays accent only when the fitted line still ends with it. A
/// terminal too narrow to keep that label draws the whole title as a bar.
fn push_mode_header(out: &mut String, header: &str, mode: &str, columns: usize, paint: Paint) {
    let fitted = fit_line(header, columns);
    let visible = fitted.trim_end();
    if let Some(head) = visible.strip_suffix(mode) {
        let tail = &fitted[head.len() + mode.len()..];
        paint.open(out, Role::BarStrong);
        out.push_str(head);
        paint.open(out, Role::AccentStrong);
        out.push_str(mode);
        paint.open(out, Role::BarStrong);
        out.push_str(tail);
        out.push_str("\x1b[0m");
    } else {
        push_bar(out, header, columns, paint, Role::BarStrong);
    }
}

fn render_save_reconciliation(
    session: &TuiSession,
    index: usize,
    cols: u16,
    rows: u16,
    paint: Paint,
) -> String {
    let columns = usize::from(cols);
    let height = usize::from(rows);
    let mut out = String::with_capacity(columns * height * 4);
    out.push_str("\x1b[?25l\x1b[H");
    if index >= session.save_recoveries.len() {
        return out;
    }
    let header = format!(
        " Uncertain save {}/{} | All saves paused ",
        index + 1,
        session.save_recoveries.len()
    );
    push_bar(&mut out, &header, columns, paint, Role::BarStrong);
    out.push_str("\r\n");

    let lines = save_reconciliation_rows(session, index, columns);
    let viewport = height.saturating_sub(2);
    let start = session
        .reconcile_scroll
        .min(lines.len().saturating_sub(viewport));
    for row in 0..viewport {
        let (text, role) = lines
            .get(start + row)
            .map_or(("", Role::Text), |(text, role)| (text.as_str(), *role));
        push_bar(&mut out, text, columns, paint, role);
        out.push_str("\r\n");
    }
    let default_footer =
        " Up/Down scroll | C copy exact path | Y reconciled | N/P record | Esc cancel";
    let (footer, role) = session.status_message.as_ref().map_or(
        (default_footer, Role::BarStrong),
        |(message, created)| {
            if created.elapsed() < STATUS_LIFETIME {
                (message.as_str(), tone_role(session.status_tone))
            } else {
                (default_footer, Role::BarStrong)
            }
        },
    );
    push_bar(&mut out, footer, columns, paint, role);
    out
}

fn save_reconciliation_rows(
    session: &TuiSession,
    index: usize,
    columns: usize,
) -> Vec<(String, Role)> {
    let Some(recovery) = session.save_recoveries.get(index) else {
        return Vec::new();
    };
    let mut rows = vec![
        (
            format!(" Destination: {}", recovery.destination_label),
            Role::Text,
        ),
        (
            " Inspect that destination and the retained siblings named below.".to_owned(),
            Role::Text,
        ),
        (
            " Preserve the version you need; decide whether the earlier save committed.".to_owned(),
            Role::Text,
        ),
        (
            " Confirming removes only this in-memory safety record. It does not write.".to_owned(),
            Role::Text,
        ),
        (String::new(), Role::Text),
        (" Diagnostic:".to_owned(), Role::Text),
    ];
    let mut diagnostic = Vec::new();
    save_warnings::append_diagnostic(&mut diagnostic, &recovery.message, columns);
    rows.extend(diagnostic.into_iter().map(|text| (text, Role::Warning)));
    if recovery.destination.to_str().is_none() {
        rows.push((
            " Copy uses reversible operating-system path encoding.".to_owned(),
            Role::Text,
        ));
    }
    rows
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LegendCommand {
    Help,
    SaveAs,
    Find,
    CutLine,
    Paste,
    Jump,
    Exit,
    Save,
    Undo,
    Redo,
    Markdown,
}

struct LegendEntry {
    label: &'static str,
    command: LegendCommand,
}

struct LegendSpan {
    command: LegendCommand,
    /// First terminal column of this label, 1-based and inclusive.
    first: u16,
    /// Last terminal column of this label, 1-based and inclusive.
    last: u16,
}

const LEGEND_TOP: &[LegendEntry] = &[
    LegendEntry {
        label: "^G Help",
        command: LegendCommand::Help,
    },
    LegendEntry {
        label: "^O Save As",
        command: LegendCommand::SaveAs,
    },
    LegendEntry {
        label: "^W Where Is",
        command: LegendCommand::Find,
    },
    LegendEntry {
        label: "^K Cut Line",
        command: LegendCommand::CutLine,
    },
    LegendEntry {
        label: "^U Paste",
        command: LegendCommand::Paste,
    },
    LegendEntry {
        label: "^J Jump Line",
        command: LegendCommand::Jump,
    },
];

const LEGEND_BOTTOM: &[LegendEntry] = &[
    LegendEntry {
        label: "^X Exit",
        command: LegendCommand::Exit,
    },
    LegendEntry {
        label: "^S Save",
        command: LegendCommand::Save,
    },
    LegendEntry {
        label: "^F Find",
        command: LegendCommand::Find,
    },
    LegendEntry {
        label: "^Z Undo",
        command: LegendCommand::Undo,
    },
    LegendEntry {
        label: "^Y Redo",
        command: LegendCommand::Redo,
    },
    LegendEntry {
        label: "^E Markdown",
        command: LegendCommand::Markdown,
    },
];

fn push_legends(out: &mut String, columns: usize, paint: Paint) {
    let (legend_top, _) = legend_line(LEGEND_TOP, columns);
    push_bar(out, &legend_top, columns, paint, Role::BarMuted);
    out.push_str("\r\n");
    let (legend_bottom, _) = legend_line(LEGEND_BOTTOM, columns);
    push_bar(out, &legend_bottom, columns, paint, Role::BarMuted);
}

/// Lays `entries` into `columns` cells.
///
/// Extra space is shared across the labels that fit. A label that does not
/// fit is left off whole, so a narrow terminal never cuts a word in half.
/// The span for each label includes the gap after it, and those spans are
/// the click targets.
fn legend_line(entries: &[LegendEntry], columns: usize) -> (String, Vec<LegendSpan>) {
    if columns == 0 {
        return (String::new(), Vec::new());
    }
    let mut chosen: Vec<&LegendEntry> = Vec::new();
    let mut used = 0usize;
    for entry in entries {
        let width = display_width(entry.label);
        let gap = usize::from(!chosen.is_empty());
        if used + gap + width > columns {
            break;
        }
        chosen.push(entry);
        used += gap + width;
    }
    if chosen.is_empty() {
        return (" ".repeat(columns), Vec::new());
    }
    let gaps = chosen.len() - 1;
    let label_cells: usize = chosen.iter().map(|entry| display_width(entry.label)).sum();
    let extra = columns - label_cells - gaps;
    let base = extra / chosen.len();
    let mut remainder = extra % chosen.len();
    let mut line = String::new();
    let mut spans = Vec::with_capacity(chosen.len());
    let mut column = 0usize;
    for (index, entry) in chosen.iter().enumerate() {
        let mut pad = base;
        if remainder > 0 {
            pad += 1;
            remainder -= 1;
        }
        let start = column;
        line.push_str(entry.label);
        line.extend(std::iter::repeat_n(' ', pad));
        column += display_width(entry.label) + pad;
        if index + 1 < chosen.len() {
            line.push(' ');
            column += 1;
        }
        spans.push(LegendSpan {
            command: entry.command,
            first: (start + 1) as u16,
            last: column as u16,
        });
    }
    (line, spans)
}

fn legend_command(row: u16, col: u16, rows: u16, columns: usize) -> Option<LegendCommand> {
    let entries = if row + 1 == rows {
        LEGEND_TOP
    } else if row == rows {
        LEGEND_BOTTOM
    } else {
        return None;
    };
    legend_line(entries, columns)
        .1
        .into_iter()
        .find(|span| col >= span.first && col <= span.last)
        .map(|span| span.command)
}

fn apply_legend_command(session: &mut TuiSession, command: LegendCommand) {
    match command {
        LegendCommand::Help => session.show_help = true,
        LegendCommand::SaveAs => {
            session.open_save_as();
        }
        LegendCommand::Find => {
            session.prompt = PromptMode::Find;
            session.prompt_input.clear();
        }
        LegendCommand::CutLine => session.cut_line(),
        LegendCommand::Paste => session.paste(),
        LegendCommand::Jump => {
            session.prompt = PromptMode::GoToLine;
            session.prompt_input.clear();
        }
        LegendCommand::Exit => trigger_exit(session),
        LegendCommand::Save => {
            session.save();
        }
        LegendCommand::Undo => session.undo(),
        LegendCommand::Redo => session.redo(),
        LegendCommand::Markdown => session.toggle_view(),
    }
}

/// Help text. The first line says how to close the overlay, so a short
/// terminal still shows that even when later rows do not fit.
const HELP_PARAGRAPHS: &[&str] = &[
    "^G or Esc closes this help.",
    "Arrows and mouse move. Home and End move within the line.",
    "PgUp and PgDn scroll a page. The wheel scrolls lines.",
    "^S Save. ^O Save As. ^X, ^C, or ^Q Exit.",
    "^W or ^F Find. ^K Cut line. ^U Paste.",
    "^Z Undo. ^Y Redo. ^J Jump line. ^E Markdown.",
    "^R Review warnings.",
    "Disk change: r reloads, k keeps, o overwrites after confirm.",
    "A disk change keeps this text until reload, save, or discard.",
];

/// Breaks `text` on spaces so every returned line is at most `width` cells.
fn wrap_plain(text: &str, width: usize) -> Vec<String> {
    if width == 0 || text.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut cells = 0usize;
    for word in text.split(' ').filter(|word| !word.is_empty()) {
        let word_cells = display_width(word);
        if word_cells > width {
            if !current.is_empty() {
                lines.push(std::mem::take(&mut current));
                cells = 0;
            }
            let mut chunk = String::new();
            let mut chunk_cells = 0usize;
            for character in word.chars() {
                let mut step = cell_width(character, chunk_cells);
                if chunk_cells + step > width && !chunk.is_empty() {
                    lines.push(std::mem::take(&mut chunk));
                    chunk_cells = 0;
                    step = cell_width(character, chunk_cells);
                }
                if step > width {
                    continue;
                }
                chunk.push(character);
                chunk_cells += step;
            }
            if !chunk.is_empty() {
                current = chunk;
                cells = chunk_cells;
            }
            continue;
        }
        let gap = usize::from(!current.is_empty());
        if cells + gap + word_cells > width {
            lines.push(std::mem::take(&mut current));
            cells = 0;
        } else if gap == 1 {
            current.push(' ');
            cells += 1;
        }
        current.push_str(word);
        cells += word_cells;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn box_rule(left: char, fill: char, right: char, width: usize) -> String {
    let mut line = String::with_capacity(width);
    line.push(left);
    line.extend(std::iter::repeat_n(fill, width.saturating_sub(2)));
    if width >= 2 {
        line.push(right);
    }
    line
}

fn box_content(text: &str, width: usize) -> String {
    let inner = width.saturating_sub(4);
    let mut line = String::from("║ ");
    line.push_str(&fit_line(text, inner));
    line.push_str(" ║");
    line
}

fn render_help_overlay(out: &mut String, cols: u16, rows: u16, paint: Paint) {
    let columns = usize::from(cols);
    let screen_rows = usize::from(rows);
    if columns < 8 || screen_rows < 3 {
        return;
    }
    let box_width = columns.min(68);
    let inner = box_width.saturating_sub(4);
    let mut body = Vec::new();
    for paragraph in HELP_PARAGRAPHS {
        body.extend(wrap_plain(paragraph, inner));
    }
    let body_room = screen_rows.saturating_sub(2);
    if body.len() > body_room {
        body.truncate(body_room);
    }
    if body.is_empty() {
        return;
    }
    let box_height = body.len() + 2;
    let start_col = columns.saturating_sub(box_width) / 2 + 1;
    let start_row = screen_rows.saturating_sub(box_height) / 2 + 1;
    let mut lines = Vec::with_capacity(box_height);
    lines.push(box_rule('╔', '═', '╗', box_width));
    for text in &body {
        lines.push(box_content(text, box_width));
    }
    lines.push(box_rule('╚', '═', '╝', box_width));

    for (index, line) in lines.iter().enumerate() {
        let _ = write!(out, "\x1b[{};{}H", start_row + index, start_col);
        paint.open(out, Role::AccentStrong);
        out.push_str(line);
        out.push_str("\x1b[0m");
    }
}

/// Terminal modes the interface turns on: the alternate screen, mouse
/// reporting in SGR form, and bracketed paste.
const ENTER_SCREEN: &[u8] = b"\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1006h\x1b[?2004h";

/// Turns every mode in [`ENTER_SCREEN`] off again and shows the cursor.
const LEAVE_SCREEN: &[u8] = b"\x1b[?2004l\x1b[?1006l\x1b[?1002l\x1b[?1000l\x1b[?1049l\x1b[?25h";

/// How long the loop waits for input before checking the terminal size and
/// expiring status messages without a keypress.
const IDLE_TICK: Duration = Duration::from_millis(250);

/// At least the capacity of the standard library's stdin buffer (8 KiB).
/// `BufReader` skips its own buffer when that buffer is empty and a read asks
/// for at least its capacity, so with every read this size no byte ever
/// waits in a buffer that `wait_for_terminal_input` cannot see.
const READ_BUFFER_BYTES: usize = 16 * 1024;

fn leave_screen() -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(LEAVE_SCREEN)?;
    stdout.flush()
}

/// Holds the terminal modes in [`ENTER_SCREEN`] and turns them off when
/// dropped, on every return path.
struct ScreenModes;

impl ScreenModes {
    fn enter() -> io::Result<Self> {
        let mut stdout = io::stdout().lock();
        let entered = stdout.write_all(ENTER_SCREEN).and_then(|()| stdout.flush());
        drop(stdout);
        if let Err(error) = entered {
            let _ = leave_screen();
            return Err(error);
        }
        Ok(Self)
    }
}

impl Drop for ScreenModes {
    fn drop(&mut self) {
        let _ = leave_screen();
    }
}

/// Runs the interactive terminal user interface.
///
/// The document loads before the terminal changes mode, so a load failure
/// leaves the terminal untouched. Release builds abort on panic, which skips
/// destructors, so a panic hook restores the screen modes and terminal
/// settings before the previous hook reports the panic. The previous hook
/// is reinstated on return.
///
/// # Errors
///
/// Returns an [`io::Error`] if the document cannot load, terminal raw mode
/// cannot be enabled, or standard I/O fails.
pub fn run(options: &LaunchOptions) -> io::Result<()> {
    let mut session =
        TuiSession::new(options, CrashRecoverySession::open_default()).map_err(io::Error::other)?;
    #[cfg(unix)]
    noter_platform::ignore_terminal_hangup();
    let raw_terminal = noter_platform::enable_raw_terminal()?;
    let restorer = raw_terminal.restorer();
    // SIGTERM does not unwind, so the guard and the panic hook would not run.
    #[cfg(unix)]
    noter_platform::restore_terminal_on_terminate(restorer, LEAVE_SCREEN);
    let previous_hook = Arc::new(std::panic::take_hook());
    let chained_hook = Arc::clone(&previous_hook);
    std::panic::set_hook(Box::new(move |info| {
        // Settings first: they matter most and need no stream. The screen
        // modes go out through unbuffered stderr, which reaches the same
        // terminal and cannot be mid-write when a render to stdout panics.
        restorer.restore();
        let _ = io::stderr().write_all(LEAVE_SCREEN);
        chained_hook(info);
    }));

    let result = ScreenModes::enter().and_then(|_screen| event_loop(&mut session));
    // The loop ends without the user choosing to exit only when the terminal
    // went away or failed; keep the unsaved text for the next launch.
    if !session.should_exit {
        session.persist_before_exit();
    }

    drop(raw_terminal);
    drop(std::panic::take_hook());
    if let Ok(previous_hook) = Arc::try_unwrap(previous_hook) {
        std::panic::set_hook(previous_hook);
    }
    result
}

/// Draws, waits for input, and dispatches events until the session exits or
/// the terminal hangs up.
fn event_loop(session: &mut TuiSession) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    let mut stdin = io::stdin();
    let mut decoder = InputDecoder::default();
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
    let mut drawn_size = None;
    let mut needs_frame = true;

    while !session.should_exit {
        let (cols, rows) = noter_platform::terminal_size();
        if needs_frame || drawn_size != Some((cols, rows)) {
            session.scroll_to_caret(cols, rows);
            stdout.write_all(render_frame(session, cols, rows).as_bytes())?;
            stdout.flush()?;
            drawn_size = Some((cols, rows));
            needs_frame = false;
        }

        let wait = session
            .recovery_delay()
            .map_or(IDLE_TICK, |delay| delay.min(IDLE_TICK));
        if !noter_platform::wait_for_terminal_input(wait)? {
            let reported = session.tick_recovery();
            if session.expire_status() || reported {
                needs_frame = true;
            }
            continue;
        }
        let read = stdin.read(&mut buffer)?;
        if read == 0 {
            // The terminal hung up.
            break;
        }
        for event in decoder.feed(&buffer[..read]) {
            handle_event(session, event, cols, rows);
            if session.should_exit {
                break;
            }
        }
        if let Some(exact_path) = session.pending_clipboard.take() {
            stdout.write_all(osc52_copy(&exact_path).as_bytes())?;
            stdout.flush()?;
        }
        needs_frame = true;
    }
    Ok(())
}

/// Encodes an explicit path-copy request without putting path bytes into a
/// terminal control sequence. The terminal decides whether OSC 52 is allowed.
fn osc52_copy(text: &str) -> String {
    const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = text.as_bytes();
    let mut output = String::with_capacity(8 + bytes.len().div_ceil(3) * 4);
    output.push_str("\x1b]52;c;");
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        for index in [
            usize::from(first >> 2),
            usize::from(((first & 0x03) << 4) | (second >> 4)),
            usize::from(((second & 0x0f) << 2) | (third >> 6)),
            usize::from(third & 0x3f),
        ] {
            output.push(char::from(BASE64[index]));
        }
        if chunk.len() < 3 {
            output.pop();
            output.push('=');
        }
        if chunk.len() < 2 {
            let last = output.len() - 2;
            output.replace_range(last..=last, "=");
        }
    }
    output.push('\x07');
    output
}

#[allow(clippy::too_many_lines)]
fn handle_event(session: &mut TuiSession, event: TuiEvent, cols: u16, rows: u16) {
    if session.show_help {
        if let TuiEvent::Key(TuiKey::Escape | TuiKey::Ctrl('g') | TuiKey::Enter) = event {
            session.show_help = false;
        }
        return;
    }

    match &session.prompt {
        PromptMode::SaveWarnings => {
            if let TuiEvent::Key(key) = event {
                match key {
                    TuiKey::Down => {
                        session.scroll_save_details(MoveDirection::Forward, cols, rows);
                    }
                    TuiKey::Up => {
                        session.scroll_save_details(MoveDirection::Backward, cols, rows);
                    }
                    TuiKey::Char('x') => {
                        if session.retained_removal_is_armed() {
                            session.remove_confirmed_retained_siblings();
                            session.retained_sibling_removal = RetainedSiblingRemoval::Idle;
                        } else {
                            session.retained_sibling_removal = RetainedSiblingRemoval::Armed;
                            session.set_status(
                                "Press x again to remove the verified retained sibling, or Esc to cancel",
                            );
                        }
                    }
                    TuiKey::Escape => {
                        if session.retained_removal_is_armed() {
                            session.retained_sibling_removal = RetainedSiblingRemoval::Idle;
                            session.set_status("Retained sibling removal canceled");
                        } else {
                            session.prompt = PromptMode::None;
                        }
                    }
                    TuiKey::Enter | TuiKey::Ctrl('r') => {
                        session.retained_sibling_removal = RetainedSiblingRemoval::Idle;
                        session.prompt = PromptMode::None;
                    }
                    _ => {}
                }
            }
            return;
        }
        PromptMode::RecoveryOffer => {
            if let TuiEvent::Key(key) = event {
                let decision = match key {
                    TuiKey::Char('r' | 'R') => Some(RecoveryDecision::Restore),
                    TuiKey::Char('d' | 'D') => Some(RecoveryDecision::Discard),
                    TuiKey::Char('l' | 'L') | TuiKey::Escape => Some(RecoveryDecision::Later),
                    _ => None,
                };
                if let Some(decision) = decision {
                    session.answer_recovery_offer(decision);
                }
            }
            return;
        }
        PromptMode::ReconcileSave(index) => {
            if let TuiEvent::Key(key) = event {
                match key {
                    TuiKey::Char('y' | 'Y') => session.reconcile_save(*index),
                    TuiKey::Char('c' | 'C') => session.copy_recovery_destination(*index),
                    TuiKey::Char('n' | 'N') if *index + 1 < session.save_recoveries.len() => {
                        session.prompt = PromptMode::ReconcileSave(*index + 1);
                        session.reconcile_scroll = 0;
                    }
                    TuiKey::Char('p' | 'P') if *index > 0 => {
                        session.prompt = PromptMode::ReconcileSave(*index - 1);
                        session.reconcile_scroll = 0;
                    }
                    TuiKey::Down => {
                        session.scroll_save_details(MoveDirection::Forward, cols, rows);
                    }
                    TuiKey::Up => {
                        session.scroll_save_details(MoveDirection::Backward, cols, rows);
                    }
                    TuiKey::Escape => session.prompt = PromptMode::None,
                    _ => {}
                }
            }
            return;
        }
        PromptMode::ExitConfirm => {
            if let TuiEvent::Key(key) = event {
                match key {
                    TuiKey::Char('y' | 'Y') => {
                        session.prompt = PromptMode::None;
                        session.after_save = AfterSave::Exit;
                        let step = session.save();
                        session.finish_save(step);
                    }
                    TuiKey::Char('n' | 'N') => {
                        session.recovery.on_discarded();
                        session.should_exit = true;
                    }
                    TuiKey::Escape | TuiKey::Char('c' | 'C') => {
                        session.prompt = PromptMode::None;
                        session.set_status("Exit cancelled");
                    }
                    _ => {}
                }
            }
            return;
        }
        PromptMode::ConfirmReplace | PromptMode::ConfirmHardLink(_) => {
            if let TuiEvent::Key(key) = event {
                let answer = match key {
                    TuiKey::Char('y' | 'Y') => Some(true),
                    TuiKey::Char('n' | 'N') | TuiKey::Escape => Some(false),
                    _ => None,
                };
                if let Some(confirmed) = answer {
                    let step = session.confirm_pending_save(confirmed);
                    session.finish_save(step);
                }
            }
            return;
        }
        PromptMode::DiskChange
        | PromptMode::ConfirmDiskOverwrite
        | PromptMode::ConfirmDiskReload => {
            if let TuiEvent::Key(key) = event {
                handle_disk_prompt_key(session, key);
            }
            return;
        }
        PromptMode::SaveAs => {
            match event {
                TuiEvent::Key(TuiKey::Enter) => {
                    let step = session.submit_save_as();
                    session.finish_save(step);
                }
                TuiEvent::Key(TuiKey::Escape) => {
                    if session.reviewed_disk.is_some() && session.conflict.is_prompting() {
                        session.show_disk_conflict();
                        session.finish_save(SaveStep::NotSaved);
                    } else {
                        session.prompt = PromptMode::None;
                        session.finish_save(SaveStep::NotSaved);
                        session.set_status("Save As cancelled");
                    }
                }
                TuiEvent::Key(TuiKey::Backspace) => {
                    session.prompt_input.pop();
                }
                TuiEvent::Key(TuiKey::Char(c)) => {
                    session.prompt_input.push(c);
                }
                TuiEvent::Paste(text) => handle_paste(session, &text),
                _ => {}
            }
            return;
        }
        PromptMode::Find => {
            match event {
                TuiEvent::Key(TuiKey::Enter) => {
                    session.search_query = Some(session.prompt_input.clone());
                    session.find_next();
                    session.prompt = PromptMode::None;
                }
                TuiEvent::Key(TuiKey::Escape) => {
                    session.prompt = PromptMode::None;
                    session.set_status("Search cancelled");
                }
                TuiEvent::Key(TuiKey::Backspace) => {
                    session.prompt_input.pop();
                }
                TuiEvent::Key(TuiKey::Char(c)) => {
                    session.prompt_input.push(c);
                }
                TuiEvent::Paste(text) => handle_paste(session, &text),
                _ => {}
            }
            return;
        }
        PromptMode::GoToLine => {
            match event {
                TuiEvent::Key(TuiKey::Enter) => {
                    if let Ok(line) = session.prompt_input.trim().parse::<usize>() {
                        session.jump_to_line(line);
                    } else {
                        session.set_status("Invalid line number");
                    }
                    session.prompt = PromptMode::None;
                }
                TuiEvent::Key(TuiKey::Escape) => {
                    session.prompt = PromptMode::None;
                    session.set_status("Jump cancelled");
                }
                TuiEvent::Key(TuiKey::Backspace) => {
                    session.prompt_input.pop();
                }
                TuiEvent::Key(TuiKey::Char(c)) if c.is_ascii_digit() => {
                    session.prompt_input.push(c);
                }
                TuiEvent::Paste(text) => handle_paste(session, &text),
                _ => {}
            }
            return;
        }
        PromptMode::None => {}
    }

    if let TuiEvent::Key(_) = event {
        session.follow_caret = true;
    }
    match event {
        TuiEvent::Mouse(mouse) => match mouse {
            TuiMouseEvent::ScrollUp { .. } => {
                session.follow_caret = false;
                session.scroll_row = session.scroll_row.saturating_sub(3);
            }
            TuiMouseEvent::ScrollDown { .. } => {
                session.follow_caret = false;
                let last_line = LineTable::new(&session.text()).len() - 1;
                session.scroll_row = (session.scroll_row + 3).min(last_line);
            }
            TuiMouseEvent::Press { col, row } => {
                let first_text_row = 2;
                if (first_text_row..first_text_row + viewport_height(rows))
                    .contains(&usize::from(row))
                {
                    let text = session.text();
                    let table = LineTable::new(&text);
                    let span = table.line(session.scroll_row + usize::from(row) - first_text_row);
                    let column = session.scroll_col
                        + usize::from(col).saturating_sub(text_offset(table.len()) + 1);
                    session.caret_byte =
                        span.start + offset_at_column(&text[span.start..span.end], column);
                    session.follow_caret = true;
                } else if let Some(command) = legend_command(row, col, rows, usize::from(cols)) {
                    apply_legend_command(session, command);
                }
            }
        },
        TuiEvent::Paste(text) => handle_paste(session, &text),
        TuiEvent::Key(key) => match key {
            TuiKey::Ctrl('c' | 'x' | 'q') => {
                trigger_exit(session);
            }
            TuiKey::Ctrl('o') => {
                session.open_save_as();
            }
            TuiKey::Ctrl('s') => {
                session.save();
            }
            TuiKey::Ctrl('r') => {
                session.open_save_reconciliation();
            }
            TuiKey::Ctrl('w' | 'f') => {
                session.prompt = PromptMode::Find;
                session.prompt_input.clear();
            }
            TuiKey::Ctrl('k') => {
                session.cut_line();
            }
            TuiKey::Ctrl('u') => {
                session.paste();
            }
            TuiKey::Ctrl('z') => {
                session.undo();
            }
            TuiKey::Ctrl('y') => {
                session.redo();
            }
            TuiKey::Ctrl('e') | TuiKey::F(2) => {
                session.toggle_view();
            }
            TuiKey::Ctrl('g') | TuiKey::F(1) => {
                session.show_help = !session.show_help;
            }
            TuiKey::Ctrl('j' | '_') => {
                session.prompt = PromptMode::GoToLine;
                session.prompt_input.clear();
            }
            TuiKey::Ctrl('a') | TuiKey::Home => {
                session.move_caret(MoveDirection::Backward, MoveUnit::LineHome);
            }
            TuiKey::End => {
                session.move_caret(MoveDirection::Forward, MoveUnit::LineEnd);
            }
            TuiKey::Left => {
                session.move_caret(MoveDirection::Backward, MoveUnit::Character);
            }
            TuiKey::Right => {
                session.move_caret(MoveDirection::Forward, MoveUnit::Character);
            }
            TuiKey::Up => {
                session.move_caret(MoveDirection::Backward, MoveUnit::Line);
            }
            TuiKey::Down => {
                session.move_caret(MoveDirection::Forward, MoveUnit::Line);
            }
            TuiKey::CtrlLeft => {
                session.move_caret(MoveDirection::Backward, MoveUnit::Word);
            }
            TuiKey::CtrlRight => {
                session.move_caret(MoveDirection::Forward, MoveUnit::Word);
            }
            TuiKey::PageUp => {
                let page_size = (rows as usize).saturating_sub(4).max(1);
                for _ in 0..page_size {
                    session.move_caret(MoveDirection::Backward, MoveUnit::Line);
                }
            }
            TuiKey::PageDown => {
                let page_size = (rows as usize).saturating_sub(4).max(1);
                for _ in 0..page_size {
                    session.move_caret(MoveDirection::Forward, MoveUnit::Line);
                }
            }
            TuiKey::Enter => {
                session.insert_newline();
            }
            TuiKey::Backspace => {
                session.delete_backwards();
            }
            TuiKey::Delete => {
                session.delete_forwards();
            }
            TuiKey::Tab => {
                session.insert_str("    ");
            }
            TuiKey::Char(c) => {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                session.insert_str(s);
            }
            TuiKey::Escape if session.recovery_unavailable_hint.is_some() => {
                session.recovery_unavailable_hint = None;
            }
            TuiKey::Escape if session.recovery_path_hint.is_some() => {
                session.recovery_path_hint = None;
            }
            _ => {}
        },
    }
}

/// Delivers a bracketed paste to the prompt that is open, or to the text.
fn handle_paste(session: &mut TuiSession, pasted: &str) {
    if session.show_help {
        return;
    }
    // A prompt takes one line; its Enter key must stay a deliberate press.
    // Terminals send pasted line breaks as CR, LF, or both.
    let first_line = pasted.split(['\r', '\n']).next().unwrap_or_default();
    match session.prompt {
        PromptMode::None => {
            session.follow_caret = true;
            session.paste_text(pasted);
        }
        PromptMode::SaveAs | PromptMode::Find => session.prompt_input.extend(
            first_line
                .chars()
                .filter(|character| !character.is_control()),
        ),
        PromptMode::GoToLine => session
            .prompt_input
            .extend(first_line.chars().filter(char::is_ascii_digit)),
        PromptMode::ConfirmReplace
        | PromptMode::ConfirmHardLink(_)
        | PromptMode::ExitConfirm
        | PromptMode::RecoveryOffer
        | PromptMode::ReconcileSave(_)
        | PromptMode::SaveWarnings
        | PromptMode::DiskChange
        | PromptMode::ConfirmDiskOverwrite
        | PromptMode::ConfirmDiskReload => {}
    }
}

fn handle_disk_prompt_key(session: &mut TuiSession, key: TuiKey) {
    if session.prompt == PromptMode::DiskChange {
        handle_disk_change_key(session, key);
    } else if session.prompt == PromptMode::ConfirmDiskOverwrite {
        handle_disk_overwrite_key(session, key);
    } else if session.prompt == PromptMode::ConfirmDiskReload {
        handle_disk_reload_key(session, key);
    }
}

fn handle_disk_change_key(session: &mut TuiSession, key: TuiKey) {
    match key {
        TuiKey::Char('r' | 'R') => {
            let step = session.request_disk_reload();
            session.finish_save(step);
        }
        TuiKey::Char('k' | 'K') | TuiKey::Escape => session.keep_disk_text(),
        TuiKey::Char('o' | 'O') => session.request_disk_overwrite(),
        TuiKey::Ctrl('o') => {
            let effect = session
                .conflict
                .reduce(ConflictCommand::Decide(ConflictDecision::SaveAs));
            if matches!(effect, ConflictEffect::RequestSaveAs) {
                let step = session.open_save_as();
                session.finish_save(step);
            }
        }
        _ => {}
    }
}

fn handle_disk_overwrite_key(session: &mut TuiSession, key: TuiKey) {
    match key {
        TuiKey::Char('y' | 'Y') => {
            let step = session.confirm_disk_overwrite();
            session.finish_save(step);
        }
        TuiKey::Char('n' | 'N') | TuiKey::Escape => session.cancel_disk_overwrite(),
        _ => {}
    }
}

fn handle_disk_reload_key(session: &mut TuiSession, key: TuiKey) {
    match key {
        TuiKey::Char('y' | 'Y') => {
            let step = session.reload_disk_version();
            session.finish_save(step);
        }
        TuiKey::Char('n' | 'N') | TuiKey::Escape => session.show_disk_conflict(),
        _ => {}
    }
}

fn trigger_exit(session: &mut TuiSession) {
    if session.save_is_blocked() {
        session.set_uncertain_guidance();
        return;
    }
    if session.has_unsaved_state() {
        session.prompt = PromptMode::ExitConfirm;
    } else {
        session.should_exit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noter::core::recovery::{
        RecoveryDocumentId, RecoveryInstanceId, RecoveryLineageGeneration, RecoverySnapshot,
        RecoverySnapshotParts, RecoveryWallTime,
    };
    use noter::core::recovery_store::RecoveryStore;
    use noter::core::revision::Revision;
    use noter::core::text_format::{Bom, Encoding};

    #[cfg(unix)]
    #[test]
    fn failed_load_escapes_control_characters_in_filename() {
        use noter::core::terminal_text::is_terminal_unsafe;

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notes\u{1b}]52;c;YQ==\u{7}.md");
        std::fs::write(&path, [0xff]).unwrap();
        let result = TuiSession::new(
            &LaunchOptions {
                initial_path: Some(path),
                ..LaunchOptions::default()
            },
            CrashRecoverySession::disabled_for_test(),
        );
        let Err(error) = result else {
            panic!("invalid UTF-8 must fail to load");
        };

        assert!(error.starts_with("cannot load `"));
        assert!(error.contains("\\u{1b}]52;c;YQ==\\u{7}.md"));
        assert!(!error.chars().any(is_terminal_unsafe));
    }

    #[test]
    fn tui_session_typing_undo_and_redo() {
        let mut session = untitled();
        assert_eq!(session.text(), "");

        // Adjacent typing within the coalescing window is one Undo step, as
        // in the window.
        session.insert_str("Hello");
        session.insert_str(" World");
        assert_eq!(session.caret_byte, 11);
        session.undo();
        assert_eq!(session.text(), "");
        session.redo();
        assert_eq!(session.text(), "Hello World");
        assert_eq!(session.caret_byte, 11);
    }

    #[test]
    fn moving_the_caret_starts_a_new_undo_step() {
        let mut session = untitled();
        session.insert_str("ab");
        key(&mut session, TuiKey::Left);
        key(&mut session, TuiKey::Char('X'));
        assert_eq!(session.text(), "aXb");

        session.undo();
        assert_eq!(session.text(), "ab");
        assert_eq!(session.caret_byte, 1);
        session.undo();
        assert_eq!(session.text(), "");
    }

    #[test]
    fn backspace_and_delete_remove_a_whole_line_ending() {
        let mut session = untitled();
        session.insert_str("a\r\nb");
        session.caret_byte = 3;
        key(&mut session, TuiKey::Backspace);
        assert_eq!(session.text(), "ab");
        assert_eq!(session.caret_byte, 1);

        session.insert_str("\r\n");
        session.caret_byte = 1;
        key(&mut session, TuiKey::Delete);
        assert_eq!(session.text(), "ab");
    }

    #[test]
    fn tui_session_cut_and_paste_line() {
        let mut session = untitled();
        session.insert_str("Line 1\nLine 2\nLine 3\n");
        session.caret_byte = 8; // On Line 2

        session.cut_line();
        assert_eq!(session.clipboard, "Line 2\n");
        assert_eq!(session.text(), "Line 1\nLine 3\n");

        session.paste();
        assert_eq!(session.text(), "Line 1\nLine 2\nLine 3\n");
    }

    #[test]
    fn ctrl_t_does_not_change_the_terminal() {
        let mut session = untitled();
        session.insert_str("keep");
        session.view = DocumentView::Markdown;
        key(&mut session, TuiKey::Ctrl('t'));
        assert_eq!(session.text(), "keep");
        assert_eq!(session.view, DocumentView::Markdown);
        assert!(session.status_message.is_none());
    }

    #[test]
    fn render_frame_produces_valid_terminal_strings() {
        let mut session = untitled();
        session.insert_str("# Markdown Header\nSome regular text here.");

        let frame = render_frame(&session, 80, 24);
        assert!(frame.contains("Noter"));
        assert!(frame.contains("Markdown Header"));
        assert!(frame.contains("^O Save"));
        assert!(frame.contains("^X Exit"));
    }

    #[test]
    fn shortcut_bar_keeps_every_label_whole_at_80_columns() {
        let session = untitled();
        let visible = strip_own_sequences(&render_frame(&session, 80, 24));
        let rows: Vec<&str> = visible.split("\r\n").collect();
        let top = rows[rows.len() - 2];
        let bottom = rows[rows.len() - 1];
        assert_eq!(display_width(top), 80, "top legend {top:?}");
        assert_eq!(display_width(bottom), 80, "bottom legend {bottom:?}");
        assert!(top.contains("^J Jump Line"), "clipped top legend: {top:?}");
        assert!(
            bottom.contains("^E Markdown"),
            "clipped bottom legend: {bottom:?}"
        );
        assert!(!top.contains("Jump L "));
        assert!(!bottom.contains("Markdo "));
    }

    #[test]
    fn shortcut_bar_drops_a_whole_label_when_the_terminal_is_narrow() {
        let (line, spans) = legend_line(LEGEND_TOP, 63);
        assert_eq!(display_width(&line), 63);
        assert!(line.contains("^U Paste"), "{line:?}");
        assert!(!line.contains("Jump"), "{line:?}");
        assert_eq!(spans.len(), 5);
    }

    #[test]
    fn legend_clicks_run_the_label_under_the_pointer() {
        for width in [80u16, 100, 140] {
            let (_, top) = legend_line(LEGEND_TOP, usize::from(width));
            let (_, bottom) = legend_line(LEGEND_BOTTOM, usize::from(width));
            assert_eq!(top.len(), 6, "width {width}");
            assert_eq!(bottom.len(), 6, "width {width}");
            click_legend(&top, 23, width);
            click_legend(&bottom, 24, width);
        }
    }

    fn click_legend(spans: &[LegendSpan], row: u16, width: u16) {
        for span in spans {
            let mut session = untitled();
            let col = span.first + (span.last - span.first) / 2;
            match span.command {
                LegendCommand::CutLine | LegendCommand::Undo => {
                    session.insert_str("one");
                }
                LegendCommand::Redo => {
                    session.insert_str("one");
                    handle_event(&mut session, TuiEvent::Key(TuiKey::Ctrl('z')), width, 24);
                }
                LegendCommand::Paste => session.clipboard = "more".to_owned(),
                _ => {}
            }
            handle_event(
                &mut session,
                TuiEvent::Mouse(TuiMouseEvent::Press { col, row }),
                width,
                24,
            );
            match span.command {
                LegendCommand::Help => assert!(session.show_help),
                LegendCommand::SaveAs | LegendCommand::Save => {
                    assert_eq!(session.prompt, PromptMode::SaveAs);
                }
                LegendCommand::Find => assert_eq!(session.prompt, PromptMode::Find),
                LegendCommand::CutLine => {
                    assert_eq!(session.text(), "");
                    assert_eq!(session.clipboard, "one");
                }
                LegendCommand::Paste => {
                    assert!(session.text().contains("more"), "{}", session.text());
                }
                LegendCommand::Jump => assert_eq!(session.prompt, PromptMode::GoToLine),
                LegendCommand::Exit => assert!(session.should_exit),
                LegendCommand::Undo => assert_eq!(session.text(), ""),
                LegendCommand::Redo => assert_eq!(session.text(), "one"),
                LegendCommand::Markdown => assert_eq!(session.view, DocumentView::Markdown),
            }
        }
    }

    #[test]
    fn help_rows_stay_inside_the_border() {
        for width in [20usize, 80, 140] {
            let box_width = width.min(68);
            let inner = box_width - 4;
            for paragraph in HELP_PARAGRAPHS {
                for wrapped in wrap_plain(paragraph, inner) {
                    let row = box_content(&wrapped, box_width);
                    assert_eq!(display_width(&row), box_width, "{row:?}");
                    assert!(row.starts_with('║') && row.ends_with('║'), "{row:?}");
                }
            }
            assert_eq!(
                display_width(&box_rule('╔', '═', '╗', box_width)),
                box_width
            );
            assert_eq!(
                display_width(&box_rule('╚', '═', '╝', box_width)),
                box_width
            );
        }
    }

    #[test]
    fn help_overlay_fits_80_by_24_and_names_every_binding() {
        let mut session = untitled();
        session.show_help = true;
        let frame = render_frame(&session, 80, 24);
        let visible = strip_own_sequences(&frame);
        for needle in [
            "closes this help",
            "^S Save",
            "^O Save As",
            "^X, ^C, or ^Q Exit",
            "^W or ^F Find",
            "^K Cut line",
            "^U Paste",
            "^Z Undo",
            "^Y Redo",
            "^J Jump line",
            "^E Markdown",
            "^R Review warnings",
            "o overwrites",
            "until reload",
            "Home and End",
            "PgUp and PgDn",
            "╔",
            "╚",
        ] {
            assert!(visible.contains(needle), "help missing {needle}");
        }
    }

    #[test]
    fn handle_event_navigation_and_editing() {
        let mut session = untitled();

        // Type "abc"
        handle_event(&mut session, TuiEvent::Key(TuiKey::Char('a')), 80, 24);
        handle_event(&mut session, TuiEvent::Key(TuiKey::Char('b')), 80, 24);
        handle_event(&mut session, TuiEvent::Key(TuiKey::Char('c')), 80, 24);
        assert_eq!(session.text(), "abc");

        // Backspace
        handle_event(&mut session, TuiEvent::Key(TuiKey::Backspace), 80, 24);
        assert_eq!(session.text(), "ab");

        // Left arrow
        handle_event(&mut session, TuiEvent::Key(TuiKey::Left), 80, 24);
        assert_eq!(session.caret_byte, 1);

        // Delete
        handle_event(&mut session, TuiEvent::Key(TuiKey::Delete), 80, 24);
        assert_eq!(session.text(), "a");

        // Enter
        handle_event(&mut session, TuiEvent::Key(TuiKey::Enter), 80, 24);
        assert!(session.text().starts_with("a\n") || session.text().starts_with("a\r\n"));

        // Home and End
        handle_event(&mut session, TuiEvent::Key(TuiKey::Home), 80, 24);
        handle_event(&mut session, TuiEvent::Key(TuiKey::End), 80, 24);
        handle_event(&mut session, TuiEvent::Key(TuiKey::PageUp), 80, 24);
        handle_event(&mut session, TuiEvent::Key(TuiKey::PageDown), 80, 24);
    }

    #[test]
    fn handle_event_search_and_jump() {
        let mut session = untitled();
        session.insert_str("First line\nSecond line\nThird line\n");

        // Trigger Find
        handle_event(&mut session, TuiEvent::Key(TuiKey::Ctrl('w')), 80, 24);
        assert_eq!(session.prompt, PromptMode::Find);

        // Type query "Second"
        for ch in "Second".chars() {
            handle_event(&mut session, TuiEvent::Key(TuiKey::Char(ch)), 80, 24);
        }
        assert_eq!(session.prompt_input, "Second");

        // Press Enter to search
        handle_event(&mut session, TuiEvent::Key(TuiKey::Enter), 80, 24);
        assert_eq!(session.prompt, PromptMode::None);
        assert_eq!(session.caret_byte, 11);

        // Trigger GoToLine
        handle_event(&mut session, TuiEvent::Key(TuiKey::Ctrl('j')), 80, 24);
        assert_eq!(session.prompt, PromptMode::GoToLine);

        // Type line 3
        handle_event(&mut session, TuiEvent::Key(TuiKey::Char('3')), 80, 24);
        handle_event(&mut session, TuiEvent::Key(TuiKey::Enter), 80, 24);
        assert_eq!(session.prompt, PromptMode::None);
        assert_eq!(session.caret_byte, 23);
    }

    #[test]
    fn handle_event_mouse_clicks_and_scroll() {
        let mut session = untitled();
        session.insert_str("Hello World\nLine 2 here\n");

        // Click on row 2, col 12 (in editor text)
        handle_event(
            &mut session,
            TuiEvent::Mouse(TuiMouseEvent::Press { col: 12, row: 2 }),
            80,
            24,
        );
        assert_eq!(session.caret_byte, 3);

        // Scroll down and up
        handle_event(
            &mut session,
            TuiEvent::Mouse(TuiMouseEvent::ScrollDown { col: 10, row: 5 }),
            80,
            24,
        );
        handle_event(
            &mut session,
            TuiEvent::Mouse(TuiMouseEvent::ScrollUp { col: 10, row: 5 }),
            80,
            24,
        );

        // Click bottom legend to toggle mode (col 75, row 24)
        handle_event(
            &mut session,
            TuiEvent::Mouse(TuiMouseEvent::Press { col: 75, row: 24 }),
            80,
            24,
        );
        assert_eq!(session.view, DocumentView::Markdown);
    }

    #[test]
    fn handle_event_help_and_exit_prompts() {
        let mut session = untitled();

        // Toggle help with Ctrl+G
        handle_event(&mut session, TuiEvent::Key(TuiKey::Ctrl('g')), 80, 24);
        assert!(session.show_help);

        // Dismiss help with Escape
        handle_event(&mut session, TuiEvent::Key(TuiKey::Escape), 80, 24);
        assert!(!session.show_help);

        // Exit clean document: should exit immediately
        handle_event(&mut session, TuiEvent::Key(TuiKey::Ctrl('q')), 80, 24);
        assert!(session.should_exit);
    }

    fn key(session: &mut TuiSession, key: TuiKey) {
        handle_event(session, TuiEvent::Key(key), 80, 24);
    }

    fn type_text(session: &mut TuiSession, text: &str) {
        for character in text.chars() {
            key(session, TuiKey::Char(character));
        }
    }

    fn session_for(path: &std::path::Path) -> TuiSession {
        let session = TuiSession::new(
            &LaunchOptions {
                initial_path: Some(path.to_path_buf()),
                ..LaunchOptions::default()
            },
            CrashRecoverySession::disabled_for_test(),
        )
        .expect("fixture document should load");
        without_startup_status(session)
    }

    fn session_for_with_store(path: &std::path::Path, store: &std::path::Path) -> TuiSession {
        TuiSession::new(
            &LaunchOptions {
                initial_path: Some(path.to_path_buf()),
                ..LaunchOptions::default()
            },
            CrashRecoverySession::open_at(store),
        )
        .expect("fixture document should load")
    }

    fn recovery_record_paths(store: &std::path::Path) -> Vec<std::path::PathBuf> {
        let records = crate::crash_recovery::recovery_store_root_for_test(store).join("records");
        std::fs::read_dir(records)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|path| path.extension().is_some_and(|ext| ext == "rec"))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Changes the platform change token without changing bytes.
    ///
    /// Unix records ctime. A timestamp update and a mode toggle both move it,
    /// and the original mode is restored exactly. Windows records file
    /// attributes in the token. A timestamp update does not move it, so the
    /// fixture sets not-content-indexed and leaves the file writable.
    fn touch_metadata(path: &std::path::Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let file = std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .expect("metadata fixture");
            file.set_modified(
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000),
            )
            .expect("metadata timestamp");
            let mode = file.metadata().expect("metadata").permissions().mode();
            let mut toggled = file.metadata().expect("metadata").permissions();
            toggled.set_mode(mode ^ 0o200);
            file.set_permissions(toggled).expect("mode toggle");
            let mut restored = file.metadata().expect("metadata").permissions();
            restored.set_mode(mode);
            file.set_permissions(restored).expect("mode restore");
            let restored_mode = file.metadata().expect("metadata").permissions().mode();
            assert_eq!(restored_mode & 0o777, mode & 0o777);
        }
        #[cfg(windows)]
        {
            noter_platform::mark_file_not_content_indexed(path).expect("metadata fixture");
        }
    }

    fn untitled() -> TuiSession {
        without_startup_status(
            TuiSession::new(
                &LaunchOptions::default(),
                CrashRecoverySession::disabled_for_test(),
            )
            .expect("an untitled session always starts"),
        )
    }

    /// A disabled recovery store truthfully reports itself unavailable at
    /// startup; tests that read the status line start from a clean one.
    fn without_startup_status(mut session: TuiSession) -> TuiSession {
        session.status_message = None;
        session.recovery_unavailable_hint = None;
        session
    }

    #[test]
    fn exit_with_save_leaves_only_after_the_write_commits() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "first").unwrap();
        let mut session = session_for(&path);
        session.caret_byte = 5;
        type_text(&mut session, " edit");

        key(&mut session, TuiKey::Ctrl('x'));
        assert_eq!(session.prompt, PromptMode::ExitConfirm);
        key(&mut session, TuiKey::Char('y'));

        assert!(session.should_exit);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first edit");
    }

    #[test]
    fn exit_with_save_stays_open_when_the_file_changed_on_disk() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "first").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "mine ");
        std::fs::write(&path, "someone else").unwrap();

        key(&mut session, TuiKey::Ctrl('x'));
        key(&mut session, TuiKey::Char('y'));

        assert!(
            !session.should_exit,
            "a conflict must never discard the text"
        );
        assert_eq!(session.after_save, AfterSave::Stay);
        assert!(session.document.is_dirty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "someone else");
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert!(session.status_message.as_ref().is_some_and(|(message, _)| {
            message.contains("Text changed")
                && message.contains("Save As")
                && !message.contains("Metadata")
        }));
    }

    #[test]
    fn untitled_exit_asks_for_a_name_then_exits_after_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("new.txt");
        let mut session = untitled();
        type_text(&mut session, "draft");

        key(&mut session, TuiKey::Ctrl('x'));
        key(&mut session, TuiKey::Char('y'));
        assert!(!session.should_exit);
        assert_eq!(session.prompt, PromptMode::SaveAs);

        type_text(&mut session, &path.to_string_lossy());
        key(&mut session, TuiKey::Enter);

        assert!(session.should_exit);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "draft");
    }

    #[test]
    fn cancelling_the_save_as_name_cancels_a_pending_exit() {
        let mut session = untitled();
        type_text(&mut session, "draft");
        key(&mut session, TuiKey::Ctrl('x'));
        key(&mut session, TuiKey::Char('y'));

        key(&mut session, TuiKey::Escape);

        assert!(!session.should_exit);
        assert_eq!(session.after_save, AfterSave::Stay);
        assert_eq!(session.text(), "draft");
    }

    #[test]
    fn save_as_asks_before_replacing_an_existing_file() {
        let directory = tempfile::tempdir().unwrap();
        let existing = directory.path().join("existing.txt");
        std::fs::write(&existing, "keep me").unwrap();
        let mut session = untitled();
        type_text(&mut session, "replacement");

        key(&mut session, TuiKey::Ctrl('o'));
        type_text(&mut session, &existing.to_string_lossy());
        key(&mut session, TuiKey::Enter);
        assert_eq!(session.prompt, PromptMode::ConfirmReplace);
        key(&mut session, TuiKey::Char('n'));
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "keep me");
        assert_eq!(session.document.path(), None);

        key(&mut session, TuiKey::Ctrl('o'));
        type_text(&mut session, &existing.to_string_lossy());
        key(&mut session, TuiKey::Enter);
        key(&mut session, TuiKey::Char('y'));
        assert_eq!(std::fs::read_to_string(&existing).unwrap(), "replacement");
        assert_eq!(session.document.path(), Some(existing.as_path()));
        assert!(!session.document.is_dirty());
    }

    #[test]
    fn save_as_is_prefilled_for_titled_documents_and_rejects_an_empty_name() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "text").unwrap();
        let mut session = session_for(&path);

        key(&mut session, TuiKey::Ctrl('o'));
        assert_eq!(session.prompt, PromptMode::SaveAs);
        assert_eq!(session.prompt_input, path.to_string_lossy());

        session.prompt_input.clear();
        key(&mut session, TuiKey::Enter);
        assert_eq!(session.prompt, PromptMode::SaveAs);
    }

    fn storage_error(message: &str) -> noter::core::save::StorageError {
        noter::core::save::StorageError::new(noter::core::save::SaveStage::Replace, message)
    }

    #[test]
    fn a_failed_save_names_the_folder_and_keeps_the_text() {
        use noter::core::revision::Revision;
        use noter::core::save::{SaveOutcome, SaveStage, StorageError};

        let mut session = untitled();
        type_text(&mut session, "kept");
        let destination = std::path::PathBuf::from("readonly").join("note.txt");
        let reservation = session
            .reserve_save_recovery(destination)
            .expect("save reservation");
        let step = session.report_save(
            reservation,
            Ok(SaveOutcome::NotCommitted {
                revision: Revision::INITIAL,
                error: StorageError::new(
                    SaveStage::CreateTemporary,
                    "create private sibling failed with PermissionDenied, OS code 13",
                ),
                cleanup_error: None,
            }),
        );
        session.finish_save(step);

        assert_eq!(step, SaveStep::NotSaved);
        assert!(!session.should_exit);
        assert!(session.save_recoveries.is_empty());
        assert_eq!(session.text(), "kept");
        assert!(session.has_unsaved_state());
        let status = session
            .status_message
            .as_ref()
            .expect("the failure must be visible")
            .0
            .as_str();
        assert!(status.contains("could not write in `readonly`"));
        assert!(status.contains("This text is still open"));
        assert!(status.contains("the file on disk was not changed"));
        assert!(status.contains("Choose another location"));
        assert!(!status.contains("PermissionDenied"));
        assert!(!status.contains("OS code"));
        assert!(!status.contains("create private sibling"));
    }

    #[cfg(unix)]
    #[test]
    fn save_as_into_an_unwritable_directory_names_that_directory() {
        use std::os::unix::fs::PermissionsExt;

        struct RestoreWrite(std::path::PathBuf);
        impl Drop for RestoreWrite {
            fn drop(&mut self) {
                if let Ok(metadata) = std::fs::metadata(&self.0) {
                    let mut permissions = metadata.permissions();
                    permissions.set_mode(0o755);
                    let _ = std::fs::set_permissions(&self.0, permissions);
                }
            }
        }

        let root = tempfile::tempdir().expect("temporary directory");
        let folder = root.path().join("readonly");
        std::fs::create_dir(&folder).expect("folder");
        let _restore = RestoreWrite(folder.clone());
        let mut permissions = std::fs::metadata(&folder).expect("metadata").permissions();
        permissions.set_mode(0o555);
        std::fs::set_permissions(&folder, permissions).expect("revoke write permission");

        let destination = folder.join("note.txt");
        let mut session = untitled();
        type_text(&mut session, "kept");
        session.prompt_input = destination.to_string_lossy().into_owned();
        let step = session.submit_save_as();
        session.finish_save(step);

        assert_eq!(step, SaveStep::NotSaved);
        assert!(!destination.exists());
        assert_eq!(session.text(), "kept");
        assert!(session.has_unsaved_state());
        assert!(session.save_recoveries.is_empty());
        let status = session
            .status_message
            .as_ref()
            .expect("the failure must be visible")
            .0
            .as_str();
        assert!(status.contains("could not write in `"));
        assert!(status.contains("readonly"));
        assert!(status.contains("This text is still open"));
        assert!(status.contains("Choose another location"));
        assert!(!status.contains("PermissionDenied"));
        assert!(!status.contains("OS code"));
        assert!(!status.contains("create private sibling"));
    }

    #[test]
    fn committed_save_warnings_remain_reviewable_after_status_expiry() {
        use noter::core::save::{
            ContentFingerprint, Durability, FileChangeToken, FileIdentity, FileObservation,
        };

        let mut session = untitled();
        let reservation = session
            .reserve_save_recovery(PathBuf::from("note.txt"))
            .unwrap();
        let observation = FileObservation::new(
            FileIdentity::new(1, 2),
            ContentFingerprint::from_bytes(b""),
            0,
            1,
            FileChangeToken::new(3, 4),
        );
        let warning = "A displaced recovery artifact .noter-save-a123.tmp was kept. Inspect its contents before removal.";
        let outcome = |warnings| {
            Ok(SaveOutcome::Committed {
                revision: Revision::INITIAL,
                observation,
                durability: Durability::FileAndDirectorySynced,
                warnings,
            })
        };
        assert_eq!(
            session.report_save(
                reservation,
                outcome(SaveWarnings::new(vec![storage_error(warning)], Vec::new(),))
            ),
            SaveStep::Committed
        );
        assert!(
            session
                .status_message
                .as_ref()
                .unwrap()
                .0
                .contains("^R details")
        );
        session.status_message = None;
        key(&mut session, TuiKey::Ctrl('r'));
        assert_eq!(session.prompt, PromptMode::SaveWarnings);
        let frame = render_frame(&session, 240, 12);
        assert!(frame.contains(warning));
        assert!(frame.contains("Closing does not remove files"));
        assert!(
            render_frame(&session, 80, 12).contains("Closing does not remove files"),
            "an 80-column warning pane must still draw the closing promise"
        );
        key(&mut session, TuiKey::Enter);
        assert_eq!(session.prompt, PromptMode::None);
        key(&mut session, TuiKey::Ctrl('r'));
        assert_eq!(session.prompt, PromptMode::SaveWarnings);
        key(&mut session, TuiKey::Escape);
        let reservation = session
            .reserve_save_recovery(PathBuf::from("note.txt"))
            .unwrap();
        session.report_save(
            reservation,
            outcome(SaveWarnings::new(Vec::new(), Vec::new())),
        );
        key(&mut session, TuiKey::Ctrl('r'));
        // The test store is unavailable, so review stays open for that reason.
        // The cleared save warning must not remain.
        assert_eq!(session.prompt, PromptMode::SaveWarnings);
        assert!(session.last_save_warnings.is_none());
        let review = render_frame(&session, 80, 12);
        assert!(!review.contains(warning));
        assert!(review.contains("Saves still work"));
    }

    #[test]
    fn warning_review_scrolls_and_keeps_untrusted_text_inert() {
        let mut session = untitled();
        let prefix = "padding ".repeat(80);
        session.last_save_warnings = Some(SaveWarnings::new(
            vec![storage_error(&format!("{prefix}.noter-save-exact.tmp"))],
            vec![storage_error(
                "barrier \u{1b}]52;c;attack\u{7} \u{85} \u{2028} \u{2067}\u{6771}\u{301}\u{1f642}",
            )],
        ));
        key(&mut session, TuiKey::Ctrl('r'));
        let first = render_frame(&session, 80, 6);
        assert!(!first.contains(".noter-save-exact.tmp"));
        let revision = session.document.revision();
        handle_paste(&mut session, "must not edit");
        key(&mut session, TuiKey::Char('x'));
        assert_eq!(session.document.revision(), revision);
        handle_event(&mut session, TuiEvent::Key(TuiKey::Up), 80, 6);
        assert_eq!(session.reconcile_scroll, 0);
        for _ in 0..20 {
            handle_event(&mut session, TuiEvent::Key(TuiKey::Down), 80, 6);
        }
        let last = render_frame(&session, 80, 6);
        assert!(last.contains(".noter-save-exact.tmp"));
        assert!(last.contains("barrier"));
        for control in ['\u{7}', '\u{85}', '\u{2028}', '\u{2067}'] {
            assert!(!last.contains(control));
        }
        assert!(!last.contains("\u{1b}]52"));
        assert!(last.contains('\u{fffd}'));
        assert!(render_frame(&session, 8, 3).contains("Enlarge"));
        key(&mut session, TuiKey::Ctrl('r'));
        assert_eq!(session.prompt, PromptMode::None);
    }

    #[test]
    fn wrapped_diagnostics_preserve_all_sanitized_cells_including_tabs() {
        let message = format!(
            "{}\u{6771}\u{301}\u{1f642}\u{1b}\u{85}\u{2067} final-name.tmp",
            "prefix\t\t".repeat(20)
        );
        let mut lines = Vec::new();
        save_warnings::append_diagnostic(&mut lines, &message, 18);
        let joined: String = lines
            .iter()
            .map(|line| line.strip_prefix(' ').unwrap())
            .collect();
        assert_eq!(joined, noter::core::terminal_text::sanitize_line(&message));
        for line in lines {
            assert!(display_width(&line) <= 18);
            assert!(fit_line(&line, 18).starts_with(&line));
        }
        assert!(joined.ends_with("final-name.tmp"));
    }

    #[test]
    fn diagnostics_wrap_on_word_boundaries() {
        let mut lines = Vec::new();
        save_warnings::append_diagnostic(&mut lines, "alpha beta gamma delta", 12);
        let bodies: Vec<&str> = lines.iter().map(|line| line.trim()).collect();
        assert_eq!(bodies, ["alpha beta", "gamma delta"]);
        for line in &lines {
            assert!(display_width(line) <= 12, "{line}");
        }
    }

    #[test]
    fn recovery_refusal_stays_until_escape_and_ctrl_r_shows_the_reason() {
        let directory = tempfile::tempdir().unwrap();
        let blocked = directory.path().join("blocked-recovery-root");
        std::fs::write(&blocked, b"not a directory").unwrap();
        let mut session = TuiSession::new(
            &LaunchOptions::default(),
            CrashRecoverySession::open_at(&blocked),
        )
        .expect("a refused store still opens a session");
        assert!(session.recovery.is_unavailable());

        let frame = strip_own_sequences(&render_frame(&session, 80, 24));
        let message = session.recovery.unavailable_message();
        let reason = message
            .strip_prefix("Recovery is off for this session: ")
            .expect("the reason leads");
        // The status bar is one 80-column line. A long platform sentence is
        // cut after the reason starts; the review below holds the rest.
        let visible_reason: String = reason.chars().take(24).collect();
        assert!(
            frame.contains("Recovery is off for this session:"),
            "{frame}"
        );
        assert!(
            frame.contains(&visible_reason),
            "the 80-column bar must show the start of the reason:\n{frame}\n{message}"
        );
        session.set_status("temporary");
        assert!(strip_own_sequences(&render_frame(&session, 80, 24)).contains("temporary"));
        session.status_message = None;
        assert!(
            strip_own_sequences(&render_frame(&session, 80, 24))
                .contains("Recovery is off for this session:")
        );

        key(&mut session, TuiKey::Escape);
        assert!(!strip_own_sequences(&render_frame(&session, 80, 24)).contains("Recovery is off"));
        key(&mut session, TuiKey::Ctrl('r'));
        assert_eq!(session.prompt, PromptMode::SaveWarnings);
        let lines = save_warnings::lines(&session, 40);
        let review = lines.join("\n");
        let flat = review.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains("Recovery is off for this session:"),
            "{review}"
        );
        assert!(flat.contains("Saves still work"), "{review}");
        let reason_at = flat.find("os error").expect("the refusal reason");
        let step_at = flat.find("Choose a private").expect("the next step");
        assert!(reason_at < step_at, "{review}");
        assert!(
            lines.iter().any(|line| line.contains("filesystem")),
            "filesystem must stay whole: {lines:?}"
        );
        for line in &lines {
            assert!(display_width(line) <= 40, "{line}");
        }
    }

    #[test]
    fn both_save_detail_views_reverse_immediately_after_scrolling_past_bottom() {
        let mut session = untitled();
        let message = "diagnostic ".repeat(80);
        session.last_save_warnings =
            Some(SaveWarnings::new(vec![storage_error(&message)], Vec::new()));
        let reservation = session
            .reserve_save_recovery(PathBuf::from("note.txt"))
            .unwrap();
        session.report_save(
            reservation,
            Ok(SaveOutcome::CommitStateUnknown {
                revision: Revision::INITIAL,
                error: storage_error(&message),
                recovery_artifact: storage_error("retained .noter-save-exact.tmp"),
            }),
        );
        for prompt in [PromptMode::SaveWarnings, PromptMode::ReconcileSave(0)] {
            session.prompt = prompt;
            session.reconcile_scroll = 0;
            for _ in 0..200 {
                handle_event(&mut session, TuiEvent::Key(TuiKey::Down), 40, 6);
            }
            let bottom = session.reconcile_scroll;
            assert!(bottom > 0);
            handle_event(&mut session, TuiEvent::Key(TuiKey::Down), 40, 6);
            assert_eq!(session.reconcile_scroll, bottom);
            handle_event(&mut session, TuiEvent::Key(TuiKey::Up), 40, 6);
            assert_eq!(session.reconcile_scroll, bottom - 1);
            // Resizing also clamps an earlier smaller viewport's offset.
            handle_event(&mut session, TuiEvent::Key(TuiKey::Down), 240, 100);
            assert_eq!(session.reconcile_scroll, 0);
        }
    }

    #[test]
    fn confirmed_retained_sibling_removal_keeps_a_replaced_name() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join(".noter-save-11111111111111111111111111111111.tmp");
        std::fs::write(&path, b"artifact").unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let facts = noter_platform::file_facts(&file).unwrap();
        let identity = match facts.identity().quality() {
            noter_platform::IdentityQuality::Preferred => noter::core::save::FileIdentity::new(
                facts.identity().volume(),
                facts.identity().file(),
            ),
            noter_platform::IdentityQuality::Reduced => noter::core::save::FileIdentity::reduced(
                facts.identity().volume(),
                facts.identity().file(),
            ),
        };
        drop(file);
        let mut session = untitled();
        session.last_save_warnings = Some(
            SaveWarnings::new(vec![storage_error("kept")], Vec::new()).with_retained(vec![
                noter::core::save::RetainedSaveSibling::new(path.clone(), identity),
            ]),
        );
        session.prompt = PromptMode::SaveWarnings;
        handle_event(&mut session, TuiEvent::Key(TuiKey::Char('x')), 80, 24);
        assert!(session.retained_removal_is_armed());
        handle_event(&mut session, TuiEvent::Key(TuiKey::Escape), 80, 24);
        assert!(!session.retained_removal_is_armed());
        assert_eq!(std::fs::read(&path).unwrap(), b"artifact");

        let held = directory.path().join("held-artifact");
        std::fs::rename(&path, &held).unwrap();
        std::fs::write(&path, b"replacement").unwrap();
        handle_event(&mut session, TuiEvent::Key(TuiKey::Char('x')), 80, 24);
        handle_event(&mut session, TuiEvent::Key(TuiKey::Char('x')), 80, 24);

        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(std::fs::read(&held).unwrap(), b"artifact");
    }

    #[test]
    fn uncertain_saves_take_priority_over_latest_save_warnings() {
        let mut session = untitled();
        session.last_save_warnings = Some(SaveWarnings::new(
            vec![storage_error("retained .noter-save-earlier.tmp")],
            Vec::new(),
        ));
        let reservation = session
            .reserve_save_recovery(PathBuf::from("note.txt"))
            .unwrap();
        session.report_save(
            reservation,
            Ok(SaveOutcome::CommitStateUnknown {
                revision: Revision::INITIAL,
                error: storage_error("unknown"),
                recovery_artifact: storage_error("retained .noter-save-new.tmp"),
            }),
        );
        key(&mut session, TuiKey::Ctrl('r'));
        assert_eq!(session.prompt, PromptMode::ReconcileSave(0));
        assert!(session.save_is_blocked());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_retained_save_warning_names_the_actual_sibling() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "prior").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "new ");
        assert_eq!(session.save(), SaveStep::Committed);
        let siblings = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with(".noter-save-")
            })
            .collect::<Vec<_>>();
        assert_eq!(siblings.len(), 1);
        let sibling = &siblings[0];
        key(&mut session, TuiKey::Ctrl('r'));
        assert_eq!(session.prompt, PromptMode::SaveWarnings);
        let frame = render_frame(&session, 1024, 12);
        assert!(frame.contains(sibling.file_name().unwrap().to_str().unwrap()));
        assert!(frame.contains("concurrent actor"));
        key(&mut session, TuiKey::Enter);
        assert_eq!(std::fs::read(sibling).unwrap(), b"prior");
        assert_eq!(std::fs::read(path).unwrap(), b"new prior");
    }

    #[test]
    fn durability_warning_keeps_terminal_work_and_recovery_until_a_clean_save() {
        use noter::core::save::{
            ContentFingerprint, Durability, FileChangeToken, FileIdentity, FileObservation,
            SaveStage, SaveWarnings, StorageError,
        };

        let root = tempfile::tempdir().expect("recovery root");
        let path = root.path().join("note.txt");
        std::fs::write(&path, "text").expect("fixture file");
        let recovery = CrashRecoverySession::open_at(root.path().join("private"));
        let mut session = TuiSession::new(
            &LaunchOptions {
                initial_path: Some(path.clone()),
                ..LaunchOptions::default()
            },
            recovery,
        )
        .expect("titled session");
        let observation = FileObservation::new(
            FileIdentity::new(1, 2),
            ContentFingerprint::from_bytes(b"text"),
            4,
            1,
            FileChangeToken::new(3, 4),
        );
        session.after_save = AfterSave::Exit;
        let reservation = session
            .reserve_save_recovery(path)
            .expect("save reservation");
        let step = session.report_save(
            reservation,
            Ok(SaveOutcome::Committed {
                revision: session.document.revision(),
                durability: Durability::FileSynced,
                observation,
                warnings: SaveWarnings::new(
                    Vec::new(),
                    vec![StorageError::new(SaveStage::SyncParent, "barrier failed")],
                ),
            }),
        );
        session.finish_save(step);
        assert!(session.save_durability.is_at_risk());
        assert!(session.has_unsaved_state());
        assert!(!session.should_exit);
        assert_eq!(session.after_save, AfterSave::Stay);
        assert!(session.recovery.next_persist_delay().is_some());
        session
            .recovery
            .force_due_persist_for_test(&session.document, Selection::caret(session.caret_byte));
        let records_dir =
            crate::crash_recovery::recovery_store_root_for_test(&root.path().join("private"))
                .join("records");
        assert!(
            std::fs::read_dir(records_dir)
                .expect("recovery records")
                .filter_map(Result::ok)
                .any(|entry| entry.path().extension().is_some_and(|ext| ext == "rec"))
        );
        trigger_exit(&mut session);
        assert_eq!(session.prompt, PromptMode::ExitConfirm);
        key(&mut session, TuiKey::Escape);
        assert!(!session.should_exit);

        assert_eq!(session.save(), SaveStep::Committed);
        assert!(!session.save_durability.is_at_risk());
        assert!(!session.has_unsaved_state());
    }

    #[test]
    fn an_uncertain_save_pauses_all_saves_until_explicit_reconciliation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        let fresh = directory.path().join("fresh.txt");
        std::fs::write(&path, "text").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "edit ");
        session.after_save = AfterSave::Exit;

        let reservation = session.reserve_save_recovery(path.clone()).unwrap();
        let step = session.report_save(
            reservation,
            Ok(SaveOutcome::CommitStateUnknown {
                revision: session.document.revision(),
                error: storage_error("rename reported failure"),
                recovery_artifact: storage_error("a private copy was kept"),
            }),
        );
        session.finish_save(step);

        assert_eq!(step, SaveStep::NotSaved);
        assert!(!session.should_exit);
        assert_eq!(session.save_recoveries.len(), 1);
        assert_eq!(session.save_recoveries[0].destination, path);
        assert_eq!(session.save(), SaveStep::NotSaved);
        session.prompt_input = path.to_string_lossy().into_owned();
        assert_eq!(session.submit_save_as(), SaveStep::NotSaved);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "text");

        session.prompt_input = fresh.to_string_lossy().into_owned();
        assert_eq!(session.submit_save_as(), SaveStep::NotSaved);
        assert!(!fresh.exists());
        assert_eq!(session.open_save_as(), SaveStep::NotSaved);
        session.open_save_reconciliation();
        assert_eq!(session.prompt, PromptMode::ReconcileSave(0));
        key(&mut session, TuiKey::Char('c'));
        assert_eq!(
            session.pending_clipboard.take(),
            Some(path.to_string_lossy().into_owned())
        );
        let frame = render_frame(&session, 80, 24);
        assert!(frame.contains("All saves paused"));
        assert!(frame.contains("Diagnostic:"));
        assert!(
            session.save_recoveries[0]
                .message
                .contains("a private copy was kept")
        );
        key(&mut session, TuiKey::Escape);
        assert_eq!(session.save_recoveries.len(), 1);
        key(&mut session, TuiKey::Ctrl('r'));
        key(&mut session, TuiKey::Char('y'));
        assert!(session.save_recoveries.is_empty());
        assert_eq!(session.prompt, PromptMode::None);
        session.prompt_input = fresh.to_string_lossy().into_owned();
        assert_eq!(session.submit_save_as(), SaveStep::Committed);
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "edit text");
    }

    #[test]
    fn uncertain_save_records_remain_independent_and_bound_exit() {
        let mut session = untitled();
        let first = PathBuf::from("first.txt");
        let second = PathBuf::from("second.txt");
        for path in [&first, &second] {
            let reservation = session.reserve_save_recovery(path.clone()).unwrap();
            session.report_save(
                reservation,
                Ok(SaveOutcome::CommitStateUnknown {
                    revision: session.document.revision(),
                    error: storage_error("unknown"),
                    recovery_artifact: storage_error("retained"),
                }),
            );
        }
        assert_eq!(session.save_recoveries.len(), 2);
        trigger_exit(&mut session);
        assert!(!session.should_exit);
        session.open_save_reconciliation();
        key(&mut session, TuiKey::Char('n'));
        assert_eq!(session.prompt, PromptMode::ReconcileSave(1));
        key(&mut session, TuiKey::Char('y'));
        assert_eq!(session.save_recoveries.len(), 1);
        assert_eq!(session.save_recoveries[0].destination, first);
        assert_eq!(session.prompt, PromptMode::ReconcileSave(0));
        assert_eq!(session.save(), SaveStep::NotSaved);
        key(&mut session, TuiKey::Char('y'));
        assert!(session.save_recoveries.is_empty());
        assert_eq!(session.prompt, PromptMode::None);
    }

    #[test]
    fn uncertain_save_reservation_refuses_oversized_destinations_and_full_ledger() {
        let mut session = untitled();
        assert!(
            session
                .reserve_save_recovery(PathBuf::from(
                    "x".repeat(MAX_SAVE_RECOVERY_DESTINATION_BYTES + 1)
                ))
                .is_none()
        );
        assert!(session.save_recoveries.is_empty());
        for index in 0..MAX_SAVE_RECOVERY_RECORDS {
            session.save_recoveries.push(SaveRecoveryRecord {
                destination: PathBuf::from(format!("{index}.txt")),
                destination_label: format!("{index}.txt"),
                message: "unknown".to_owned(),
            });
        }
        assert!(
            session
                .reserve_save_recovery(PathBuf::from("last.txt"))
                .is_none()
        );
        assert_eq!(session.save_recoveries.len(), MAX_SAVE_RECOVERY_RECORDS);
    }

    #[test]
    fn terminal_path_copy_encodes_payload_without_raw_controls() {
        assert_eq!(osc52_copy(""), "\x1b]52;c;\x07");
        assert_eq!(osc52_copy("f"), "\x1b]52;c;Zg==\x07");
        assert_eq!(osc52_copy("fo"), "\x1b]52;c;Zm8=\x07");
        assert_eq!(osc52_copy("foo"), "\x1b]52;c;Zm9v\x07");
        assert_eq!(
            osc52_copy("a\x1b]52;c;bad"),
            "\x1b]52;c;YRtdNTI7YztiYWQ=\x07"
        );
    }

    #[test]
    fn uncertain_save_frame_draws_untrusted_diagnostic_inert() {
        let mut session = untitled();
        session.save_recoveries.push(SaveRecoveryRecord {
            destination: PathBuf::from("note.txt"),
            destination_label: "evil\x1b]0;owned\x07.txt".to_owned(),
            message: "state\x1b]52;c;ZWNobw==\x07\u{202e}unknown".to_owned(),
        });
        session.prompt = PromptMode::ReconcileSave(0);
        for (cols, rows) in [(32, 10), (80, 24), (120, 12)] {
            let frame = render_frame(&session, cols, rows);
            assert!(!frame.contains("\x1b]"));
            assert!(!frame.contains('\x07'));
            assert_frame_is_terminal_safe(&frame, usize::from(cols));
            assert!(strip_own_sequences(&frame).split("\r\n").count() <= usize::from(rows));
        }
    }

    #[test]
    fn a_save_that_did_not_commit_cancels_a_pending_exit() {
        let mut session = untitled();
        session.after_save = AfterSave::Exit;

        let reservation = session
            .reserve_save_recovery(PathBuf::from("note.txt"))
            .unwrap();
        let step = session.report_save(
            reservation,
            Ok(SaveOutcome::NotCommitted {
                revision: session.document.revision(),
                error: storage_error("disk full"),
                cleanup_error: None,
            }),
        );
        session.finish_save(step);

        assert_eq!(step, SaveStep::NotSaved);
        assert!(!session.should_exit);
        assert_eq!(session.after_save, AfterSave::Stay);
        assert!(session.status_message.as_ref().is_some_and(|(message, _)| {
            message.contains("could not write in the current directory")
                && message.contains("This text is still open")
                && !message.contains("disk full")
        }));
        assert!(session.save_recoveries.is_empty());
    }

    #[test]
    fn save_as_to_the_current_path_still_detects_an_external_change() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "first").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "mine ");
        std::fs::write(&path, "someone else").unwrap();
        key(&mut session, TuiKey::Ctrl('s'));

        // Same-path Save As is still an ordinary save, so the conflict remains.
        key(&mut session, TuiKey::Ctrl('o'));
        key(&mut session, TuiKey::Enter);

        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "someone else");
        assert_eq!(session.text(), "mine first");
        assert!(session.document.is_dirty());
    }

    #[test]
    fn metadata_only_conflict_keeps_text_until_overwrite_is_confirmed() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        let original = "café\u{1b}\n";
        std::fs::write(&path, original).unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "α");
        let edited = session.text();
        let baseline = session.document.saved_target();
        let TargetExpectation::Existing(before) = baseline.unwrap() else {
            panic!("a loaded file has a baseline");
        };
        touch_metadata(&path);
        let TargetState::Regular(after) = inspect_target(&path, SaveStage::InspectInitial).unwrap()
        else {
            panic!("the fixture stays a regular file");
        };
        assert_eq!(before.identity(), after.identity());
        assert_eq!(before.fingerprint(), after.fingerprint());
        assert_eq!(before.length(), after.length());
        assert_ne!(before, after, "the change token did not move");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

        key(&mut session, TuiKey::Ctrl('s'));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert!(session.has_unsaved_state());
        let frame = render_frame(&session, 80, 24);
        assert_frame_is_terminal_safe(&frame, 80);
        let visible = strip_own_sequences(&frame);
        assert!(
            visible.contains("Metadata") && visible.contains("matches"),
            "{visible}"
        );
        assert!(visible.contains("(o)verwrite"), "{visible}");
        assert!(!visible.contains("Text changed"), "{visible}");

        let caret = session.caret_byte;
        key(&mut session, TuiKey::Char('y'));
        key(&mut session, TuiKey::Char('z'));
        handle_event(
            &mut session,
            TuiEvent::Paste("pasted\u{1b}".to_owned()),
            80,
            24,
        );
        handle_event(
            &mut session,
            TuiEvent::Mouse(TuiMouseEvent::Press { col: 12, row: 2 }),
            80,
            24,
        );
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_eq!(session.text(), edited);
        assert_eq!(session.caret_byte, caret);
        assert_eq!(session.save(), SaveStep::AwaitingInput);
        key(&mut session, TuiKey::Ctrl('s'));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

        key(&mut session, TuiKey::Ctrl('o'));
        assert_eq!(session.prompt, PromptMode::SaveAs);
        key(&mut session, TuiKey::Escape);
        assert_eq!(session.prompt, PromptMode::DiskChange);

        key(&mut session, TuiKey::Char('o'));
        assert_eq!(session.prompt, PromptMode::ConfirmDiskOverwrite);
        assert!(strip_own_sequences(&render_frame(&session, 80, 24)).contains("Replace the disk"));
        key(&mut session, TuiKey::Char('n'));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

        key(&mut session, TuiKey::Char('k'));
        assert_eq!(session.prompt, PromptMode::None);
        assert_eq!(session.text(), edited);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(session.document.saved_target(), baseline);
        assert!(session.document.is_dirty());
        assert!(session.has_unsaved_state());
        assert!(status_text(&session).contains("not written"));

        key(&mut session, TuiKey::Ctrl('s'));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        key(&mut session, TuiKey::Char('o'));
        key(&mut session, TuiKey::Char('y'));
        assert_eq!(session.prompt, PromptMode::None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
        assert!(!session.document.is_dirty());
        assert!(!session.has_unsaved_state());
        assert_ne!(session.document.saved_target(), baseline);
    }

    #[test]
    fn reload_asks_before_replacing_unsaved_text() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "first").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "mine ");
        assert!(session.history.can_undo());
        std::fs::write(&path, "雪").unwrap();

        key(&mut session, TuiKey::Ctrl('s'));
        let visible = strip_own_sequences(&render_frame(&session, 80, 24));
        assert!(visible.contains("Text changed"), "{visible}");
        assert!(!visible.contains("Metadata"), "{visible}");
        key(&mut session, TuiKey::Char('r'));
        assert_eq!(session.prompt, PromptMode::ConfirmDiskReload);
        key(&mut session, TuiKey::Char('n'));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_eq!(session.text(), "mine first");
        assert!(session.document.is_dirty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "雪");

        key(&mut session, TuiKey::Char('r'));
        key(&mut session, TuiKey::Char('y'));
        assert_eq!(session.text(), "雪");
        assert!(!session.document.is_dirty());
        assert!(!session.has_unsaved_state());
        assert_eq!(session.prompt, PromptMode::None);
        assert_eq!(session.caret_byte, 0);
        assert!(!session.history.can_undo());
        assert!(status_text(&session).contains("Loaded"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "雪");
    }

    #[test]
    fn unedited_text_reloads_from_disk_without_a_second_question() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "original").unwrap();
        let mut session = session_for(&path);
        std::fs::write(&path, "雪").unwrap();

        key(&mut session, TuiKey::Ctrl('s'));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert!(!session.document.is_dirty());
        assert!(session.has_unsaved_state());
        assert!(strip_own_sequences(&render_frame(&session, 80, 24)).contains("note.txt *"));

        key(&mut session, TuiKey::Char('r'));
        assert_eq!(session.prompt, PromptMode::None);
        assert_ne!(session.prompt, PromptMode::ConfirmDiskReload);
        assert_eq!(session.text(), "雪");
        assert!(!session.document.is_dirty());
        assert!(!session.has_unsaved_state());
        assert_eq!(session.caret_byte, 0);
        let visible = strip_own_sequences(&render_frame(&session, 80, 24));
        assert!(visible.contains("note.txt │"), "{visible}");
        assert!(!visible.contains("note.txt *"), "{visible}");
    }

    #[test]
    fn keeping_an_external_change_does_not_replace_or_rebaseline() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "original").unwrap();
        let mut session = session_for(&path);
        let baseline = session.document.saved_target();
        std::fs::write(&path, "external").unwrap();

        key(&mut session, TuiKey::Ctrl('s'));
        let visible = strip_own_sequences(&render_frame(&session, 80, 24));
        assert!(
            visible.contains("Text changed") && !visible.contains("Metadata"),
            "{visible}"
        );
        key(&mut session, TuiKey::Escape);
        assert_eq!(session.prompt, PromptMode::None);
        assert_eq!(session.text(), "original");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert_eq!(session.document.saved_target(), baseline);
        assert!(!session.document.is_dirty());
        assert!(session.has_unsaved_state());

        key(&mut session, TuiKey::Ctrl('x'));
        assert_eq!(session.prompt, PromptMode::ExitConfirm);
        assert!(!session.should_exit);
        key(&mut session, TuiKey::Char('y'));
        assert!(!session.should_exit);
        assert_eq!(session.after_save, AfterSave::Stay);
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_eq!(session.text(), "original");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");

        key(&mut session, TuiKey::Char('k'));
        key(&mut session, TuiKey::Ctrl('x'));
        assert_eq!(session.prompt, PromptMode::ExitConfirm);
        key(&mut session, TuiKey::Char('n'));
        assert!(session.should_exit);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert_eq!(session.text(), "original");
    }

    #[test]
    fn a_missing_file_cannot_be_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "first").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "mine ");
        std::fs::remove_file(&path).unwrap();

        key(&mut session, TuiKey::Ctrl('s'));
        let visible = strip_own_sequences(&render_frame(&session, 80, 24));
        assert!(visible.contains("missing"), "{visible}");
        assert!(!visible.contains("(o)"), "{visible}");
        key(&mut session, TuiKey::Char('o'));
        key(&mut session, TuiKey::Char('z'));
        assert!(!path.exists());
        assert_eq!(session.text(), "mine first");
        assert_eq!(session.prompt, PromptMode::DiskChange);

        key(&mut session, TuiKey::Char('r'));
        assert_eq!(session.prompt, PromptMode::ConfirmDiskReload);
        key(&mut session, TuiKey::Char('n'));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_eq!(session.text(), "mine first");
        key(&mut session, TuiKey::Char('r'));
        key(&mut session, TuiKey::Char('y'));
        assert_eq!(session.text(), "mine first");
        assert!(session.history.can_undo());
        assert!(status_text(&session).contains("still open"));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert!(!path.exists());
    }

    #[test]
    fn overwrite_stops_when_the_disk_changes_after_review() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "first").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "mine ");
        let baseline = session.document.saved_target();
        std::fs::write(&path, "external A").unwrap();

        key(&mut session, TuiKey::Ctrl('s'));
        key(&mut session, TuiKey::Char('o'));
        assert_eq!(session.prompt, PromptMode::ConfirmDiskOverwrite);
        std::fs::write(&path, "external B").unwrap();
        key(&mut session, TuiKey::Char('y'));

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external B");
        assert_eq!(session.text(), "mine first");
        assert!(session.document.is_dirty());
        assert!(session.has_unsaved_state());
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_ne!(session.prompt, PromptMode::ConfirmDiskOverwrite);
        assert!(status_text(&session).contains("changed again"));
        assert_eq!(session.document.saved_target(), baseline);
    }

    #[test]
    fn overwrite_stops_when_the_path_is_no_longer_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "first").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "mine ");
        let baseline = session.document.saved_target();
        std::fs::write(&path, "external").unwrap();

        key(&mut session, TuiKey::Ctrl('s'));
        key(&mut session, TuiKey::Char('o'));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        key(&mut session, TuiKey::Char('y'));

        assert!(path.is_dir());
        assert_eq!(session.text(), "mine first");
        assert_eq!(session.document.saved_target(), baseline);
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert!(status_text(&session).contains("changed again"));
        let visible = strip_own_sequences(&render_frame(&session, 80, 24));
        assert!(visible.contains("directory"), "{visible}");
        assert!(!visible.contains("(o)"), "{visible}");
    }

    #[test]
    fn a_disk_that_matches_again_is_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "original").unwrap();
        let mut session = session_for(&path);
        let TargetExpectation::Existing(expected) = session.document.saved_target().unwrap() else {
            panic!("a loaded file has a baseline");
        };
        std::fs::write(&path, "external").unwrap();
        key(&mut session, TuiKey::Ctrl('s'));
        key(&mut session, TuiKey::Char('o'));
        assert_eq!(session.prompt, PromptMode::ConfirmDiskOverwrite);

        let step = session.note_disk_changed_again(TargetState::Regular(expected));
        assert_eq!(step, SaveStep::NotSaved);
        assert_eq!(session.prompt, PromptMode::None);
        assert!(status_text(&session).contains("matches again"));
        assert_eq!(session.text(), "original");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert!(!session.document.is_dirty());
        assert!(!session.has_unsaved_state());

        key(&mut session, TuiKey::Ctrl('s'));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert!(session.has_unsaved_state());
    }

    #[test]
    fn matching_disk_again_drops_external_retention() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("note.txt");
        let store = root.path().join("private");
        std::fs::write(&path, "original").unwrap();
        let mut session = session_for_with_store(&path, &store);
        let TargetExpectation::Existing(expected) = session.document.saved_target().unwrap() else {
            panic!("a loaded file has a baseline");
        };
        std::fs::write(&path, "external").unwrap();
        key(&mut session, TuiKey::Ctrl('s'));
        session
            .recovery
            .force_due_persist_for_test(&session.document, Selection::caret(session.caret_byte));
        assert!(!recovery_record_paths(&store).is_empty());

        let step = session.note_disk_changed_again(TargetState::Regular(expected));

        assert_eq!(step, SaveStep::NotSaved);
        assert!(!session.has_unsaved_state());
        assert!(recovery_record_paths(&store).is_empty());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
    }

    #[test]
    fn save_as_to_another_path_leaves_the_conflicting_file_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        let other = directory.path().join("copy.txt");
        std::fs::write(&path, "first").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "mine ");
        std::fs::write(&path, "someone else").unwrap();
        key(&mut session, TuiKey::Ctrl('s'));

        key(&mut session, TuiKey::Ctrl('o'));
        assert_eq!(session.prompt, PromptMode::SaveAs);
        key(&mut session, TuiKey::Escape);
        assert_eq!(session.prompt, PromptMode::DiskChange);

        key(&mut session, TuiKey::Ctrl('o'));
        session.prompt_input.clear();
        type_text(&mut session, &other.to_string_lossy());
        key(&mut session, TuiKey::Enter);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "someone else");
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "mine first");
        assert_eq!(session.prompt, PromptMode::None);
        assert!(!session.document.is_dirty());
        assert!(!session.has_unsaved_state());
        assert_eq!(session.document.path(), Some(other.as_path()));
        key(&mut session, TuiKey::Ctrl('s'));
        assert_eq!(session.prompt, PromptMode::None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "someone else");
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "mine first");
    }

    #[test]
    fn declining_save_as_replace_returns_to_the_disk_decision() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        let occupied = directory.path().join("occupied.txt");
        std::fs::write(&path, "first").unwrap();
        std::fs::write(&occupied, "keep").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "mine ");
        std::fs::write(&path, "someone else").unwrap();
        key(&mut session, TuiKey::Ctrl('s'));

        key(&mut session, TuiKey::Ctrl('o'));
        session.prompt_input.clear();
        type_text(&mut session, &occupied.to_string_lossy());
        key(&mut session, TuiKey::Enter);
        assert_eq!(session.prompt, PromptMode::ConfirmReplace);
        key(&mut session, TuiKey::Char('n'));

        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_eq!(std::fs::read_to_string(&occupied).unwrap(), "keep");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "someone else");
        assert_eq!(session.text(), "mine first");
        assert!(session.document.is_dirty());
    }

    #[test]
    fn a_conflict_cleanup_failure_is_named_without_the_engine_diagnostic() {
        use noter::core::save::{FileChangeToken, FileObservation, StorageError};

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "text").unwrap();
        let mut session = session_for(&path);
        let TargetExpectation::Existing(expected) = session.document.saved_target().unwrap() else {
            panic!("a loaded file has a baseline");
        };
        let actual = FileObservation::new(
            expected.identity(),
            expected.fingerprint(),
            expected.length(),
            expected.link_count(),
            FileChangeToken::new(expected.change_token().primary().wrapping_add(9), 1),
        );
        let reservation = session.reserve_save_recovery(path.clone()).unwrap();
        let step = session.report_save(
            reservation,
            Ok(SaveOutcome::Conflict {
                revision: session.document.revision(),
                expected: TargetExpectation::Existing(expected),
                actual: TargetState::Regular(actual),
                cleanup_error: Some(StorageError::new(SaveStage::Cleanup, "secret\u{1b}path")),
            }),
        );

        assert_eq!(step, SaveStep::NotSaved);
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "text");
        let frame = render_frame(&session, 80, 24);
        assert_frame_is_terminal_safe(&frame, 80);
        let visible = strip_own_sequences(&frame);
        assert!(visible.contains("Metadata"), "{visible}");
        assert!(visible.contains("Private file remains"), "{visible}");
        assert!(!visible.contains("secret"), "{visible}");
        assert!(!visible.contains("Cleanup"), "{visible}");
    }

    #[test]
    fn reload_of_dirty_text_drops_the_recovery_record() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("note.txt");
        let store = root.path().join("private");
        std::fs::write(&path, "saved").unwrap();
        {
            let mut session = session_for_with_store(&path, &store);
            type_text(&mut session, "edited ");
            session.recovery.force_due_persist_for_test(
                &session.document,
                Selection::caret(session.caret_byte),
            );
            assert!(!recovery_record_paths(&store).is_empty());
            std::fs::write(&path, "雪").unwrap();
            key(&mut session, TuiKey::Ctrl('s'));
            key(&mut session, TuiKey::Char('r'));
            assert_eq!(session.prompt, PromptMode::ConfirmDiskReload);
            key(&mut session, TuiKey::Char('y'));
            assert_eq!(session.text(), "雪");
            assert!(!session.has_unsaved_state());
            assert!(recovery_record_paths(&store).is_empty());
        }
        let next = untitled_with_store(&store);
        assert_eq!(next.prompt, PromptMode::None);
        assert!(next.recovery.active_offer().is_none());
    }

    #[test]
    fn clean_external_text_is_retained_until_reload() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("note.txt");
        let store = root.path().join("private");
        std::fs::write(&path, "original").unwrap();
        {
            let mut session = session_for_with_store(&path, &store);
            std::fs::write(&path, "external").unwrap();
            key(&mut session, TuiKey::Ctrl('s'));
            assert!(!session.document.is_dirty());
            assert!(session.has_unsaved_state());
            session.recovery.force_due_persist_for_test(
                &session.document,
                Selection::caret(session.caret_byte),
            );
            assert!(!recovery_record_paths(&store).is_empty());
            key(&mut session, TuiKey::Char('r'));
            assert_eq!(session.prompt, PromptMode::None);
            assert_eq!(session.text(), "external");
            assert!(!session.has_unsaved_state());
            assert!(recovery_record_paths(&store).is_empty());
        }
        let next = untitled_with_store(&store);
        assert_eq!(next.prompt, PromptMode::None);
        assert!(next.recovery.active_offer().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_cannot_be_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        let other = directory.path().join("other.txt");
        std::fs::write(&path, "first").unwrap();
        std::fs::write(&other, "elsewhere").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "mine ");
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&other, &path).unwrap();

        key(&mut session, TuiKey::Ctrl('s'));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        let visible = strip_own_sequences(&render_frame(&session, 80, 24));
        assert!(visible.contains("symbolic link"), "{visible}");
        assert!(!visible.contains("(o)"), "{visible}");
        key(&mut session, TuiKey::Char('o'));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "elsewhere");
        assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(session.text(), "mine first");
    }

    #[cfg(unix)]
    #[test]
    fn declined_hard_link_after_overwrite_keeps_the_new_baseline() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        let other_name = directory.path().join("other-name.txt");
        std::fs::write(&path, "shared").unwrap();
        let mut session = session_for(&path);
        std::fs::hard_link(&path, &other_name).unwrap();
        type_text(&mut session, "new ");
        let baseline = session.document.saved_target();
        std::fs::write(&path, "external").unwrap();

        key(&mut session, TuiKey::Ctrl('s'));
        assert_eq!(session.prompt, PromptMode::DiskChange);
        let visible = strip_own_sequences(&render_frame(&session, 80, 24));
        assert!(
            visible.contains("Text changed") && !visible.contains("Metadata"),
            "{visible}"
        );
        key(&mut session, TuiKey::Char('o'));
        key(&mut session, TuiKey::Char('y'));

        assert_eq!(session.prompt, PromptMode::ConfirmHardLink(2));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert_eq!(std::fs::read_to_string(&other_name).unwrap(), "external");
        assert_eq!(session.text(), "new shared");
        assert_ne!(session.document.saved_target(), baseline);

        key(&mut session, TuiKey::Char('n'));
        assert_eq!(session.prompt, PromptMode::None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        key(&mut session, TuiKey::Ctrl('s'));
        assert_eq!(session.prompt, PromptMode::ConfirmHardLink(2));
        assert_ne!(session.prompt, PromptMode::DiskChange);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "external");
        assert_eq!(std::fs::read_to_string(&other_name).unwrap(), "external");
    }

    fn status_text(session: &TuiSession) -> String {
        session
            .status_message
            .as_ref()
            .map(|(message, _)| message.clone())
            .unwrap_or_default()
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_saves_ask_before_splitting_the_link() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        let other_name = directory.path().join("other-name.txt");
        std::fs::write(&path, "shared").unwrap();
        std::fs::hard_link(&path, &other_name).unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "new ");

        assert_eq!(session.save(), SaveStep::AwaitingInput);
        assert_eq!(session.prompt, PromptMode::ConfirmHardLink(2));
        key(&mut session, TuiKey::Char('n'));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "shared");

        session.save();
        key(&mut session, TuiKey::Char('y'));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new shared");
        assert_eq!(std::fs::read_to_string(&other_name).unwrap(), "shared");
    }

    #[cfg(unix)]
    #[test]
    fn save_as_over_a_hard_linked_file_confirms_replace_then_the_split() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target.txt");
        let other_name = directory.path().join("other-name.txt");
        std::fs::write(&target, "shared").unwrap();
        std::fs::hard_link(&target, &other_name).unwrap();
        let mut session = untitled();
        type_text(&mut session, "mine");

        key(&mut session, TuiKey::Ctrl('o'));
        type_text(&mut session, &target.to_string_lossy());
        key(&mut session, TuiKey::Enter);
        assert_eq!(session.prompt, PromptMode::ConfirmReplace);
        key(&mut session, TuiKey::Char('y'));
        assert_eq!(session.prompt, PromptMode::ConfirmHardLink(2));
        key(&mut session, TuiKey::Char('y'));

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "mine");
        assert_eq!(std::fs::read_to_string(&other_name).unwrap(), "shared");
        assert!(session.pending_save.is_none());
    }

    /// Removes the SGR and cursor sequences the renderer itself emits.
    fn strip_own_sequences(frame: &str) -> String {
        let mut out = String::new();
        let mut characters = frame.chars();
        while let Some(character) = characters.next() {
            if character == '\u{1B}' {
                assert_eq!(
                    characters.next(),
                    Some('['),
                    "only CSI sequences are emitted"
                );
                for next in characters.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            } else {
                out.push(character);
            }
        }
        out
    }

    fn assert_frame_is_terminal_safe(frame: &str, cols: usize) {
        let visible = strip_own_sequences(frame);
        for row in visible.split("\r\n") {
            assert!(
                !row.chars()
                    .any(noter::core::terminal_text::is_terminal_unsafe),
                "unsafe character in row {row:?}"
            );
            assert!(
                display_width(row) <= cols,
                "row wider than the terminal: {row:?}"
            );
        }
    }

    #[test]
    fn document_escape_sequences_are_drawn_inert() {
        let mut session = untitled();
        session.insert_str(
            "title\u{1B}]0;owned\u{7}\nclip\u{1B}]52;c;ZWNobw==\u{7}\n\u{9B}2J\u{202E}txt\u{206A}\u{206B}\u{206C}\u{206D}\u{206E}\u{206F}",
        );
        for view in [DocumentView::Text, DocumentView::Markdown] {
            session.view = view;
            let frame = render_frame(&session, 80, 24);
            assert!(!frame.contains("\u{1B}]"));
            assert!(!frame.contains('\u{7}'));
            assert!(!frame.contains([
                '\u{206A}', '\u{206B}', '\u{206C}', '\u{206D}', '\u{206E}', '\u{206F}'
            ]));
            assert_frame_is_terminal_safe(&frame, 80);
        }
    }

    #[cfg(unix)]
    #[test]
    fn file_names_with_control_characters_are_drawn_inert() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("evil\u{1B}]0;x\u{7}.txt");
        std::fs::write(&path, "text").unwrap();
        let session = session_for(&path);

        let frame = render_frame(&session, 80, 24);

        assert!(!frame.contains("\u{1B}]0"));
        assert_frame_is_terminal_safe(&frame, 80);
    }

    #[test]
    fn search_uses_exact_ranges_when_case_folding_changes_length() {
        let mut session = untitled();
        session.insert_str("\u{212A}elvin and kelvin");
        session.caret_byte = 0;
        session.search_query = Some("KELVIN".to_owned());

        session.find_next();
        assert_eq!(session.caret_byte, 13);
        session.find_next();
        assert_eq!(session.caret_byte, 0);
        assert!(
            session
                .status_message
                .as_ref()
                .is_some_and(|(message, _)| message == "Match 1 of 2 (wrapped)")
        );
        assert_frame_is_terminal_safe(&render_frame(&session, 80, 24), 80);
    }

    #[test]
    fn clicks_land_on_character_boundaries_in_multibyte_and_wide_text() {
        let mut session = untitled();
        session.insert_str("héllo\n世界ab");
        let text_column = |column: u16| 9 + column;

        // Column 2 is the l after the two-byte é.
        handle_event(
            &mut session,
            TuiEvent::Mouse(TuiMouseEvent::Press {
                col: text_column(2),
                row: 2,
            }),
            80,
            24,
        );
        assert_eq!(session.caret_byte, 3);
        key(&mut session, TuiKey::Char('x'));
        assert_eq!(session.text(), "héxllo\n世界ab");

        // Column 3 is the second cell of 界, which starts at column 2.
        handle_event(
            &mut session,
            TuiEvent::Mouse(TuiMouseEvent::Press {
                col: text_column(3),
                row: 3,
            }),
            80,
            24,
        );
        assert_eq!(session.caret_byte, "héxllo\n世".len());
        // A click in the gutter or past the end stays on the line.
        handle_event(
            &mut session,
            TuiEvent::Mouse(TuiMouseEvent::Press { col: 1, row: 3 }),
            80,
            24,
        );
        assert_eq!(session.caret_byte, "héxllo\n".len());
        handle_event(
            &mut session,
            TuiEvent::Mouse(TuiMouseEvent::Press { col: 79, row: 3 }),
            80,
            24,
        );
        assert_eq!(session.caret_byte, session.text().len());
    }

    #[test]
    fn long_lines_scroll_horizontally_and_rows_never_overflow() {
        let mut session = untitled();
        session.insert_str(&"世".repeat(100));

        session.scroll_to_caret(40, 10);
        let frame = render_frame(&session, 40, 10);

        assert!(session.scroll_col > 0);
        assert_frame_is_terminal_safe(&frame, 40);
        assert!(frame.contains("\x1b[?25h"), "the caret is visible");
    }

    #[test]
    fn the_wheel_scrolls_away_from_the_caret_until_the_next_key() {
        let mut session = untitled();
        session.insert_str(&"line\n".repeat(100));
        session.scroll_to_caret(80, 24);
        let at_caret = session.scroll_row;

        handle_event(
            &mut session,
            TuiEvent::Mouse(TuiMouseEvent::ScrollUp { col: 10, row: 5 }),
            80,
            24,
        );
        session.scroll_to_caret(80, 24);
        assert_eq!(session.scroll_row, at_caret - 3);
        assert!(!render_frame(&session, 80, 24).contains("\x1b[?25h"));

        key(&mut session, TuiKey::Right);
        session.scroll_to_caret(80, 24);
        assert_eq!(session.scroll_row, at_caret);
    }

    #[test]
    fn combining_marks_are_drawn_with_their_base_character() {
        let mut session = untitled();
        session.insert_str("cafe\u{301} \u{1F44D}\u{1F3FD}");
        let frame = strip_own_sequences(&render_frame(&session, 80, 24));
        assert!(frame.contains("cafe\u{301}"), "{frame}");
    }

    #[test]
    fn terminals_too_small_for_the_layout_get_a_fitted_notice() {
        let session = untitled();
        for (cols, rows) in [(10, 24), (80, 4), (5, 2)] {
            let frame = strip_own_sequences(&render_frame(&session, cols, rows));
            assert_eq!(frame.lines().count(), 1);
            assert_eq!(display_width(&frame), usize::from(cols));
        }
    }

    #[test]
    fn a_paste_is_one_edit_with_the_documents_line_endings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("crlf.txt");
        std::fs::write(&path, "top\r\n").unwrap();
        let mut session = session_for(&path);
        session.caret_byte = 5;

        handle_event(
            &mut session,
            TuiEvent::Paste("one\ntwo\rthree".to_owned()),
            80,
            24,
        );
        assert_eq!(session.text(), "top\r\none\r\ntwo\r\nthree");

        key(&mut session, TuiKey::Ctrl('z'));
        assert_eq!(session.text(), "top\r\n");
    }

    #[test]
    fn a_paste_into_a_prompt_takes_the_first_line_only() {
        let mut session = untitled();
        key(&mut session, TuiKey::Ctrl('f'));
        handle_event(
            &mut session,
            TuiEvent::Paste("needle\u{7}\nsecond".to_owned()),
            80,
            24,
        );
        assert_eq!(session.prompt_input, "needle");
        assert_eq!(session.prompt, PromptMode::Find);

        key(&mut session, TuiKey::Escape);
        key(&mut session, TuiKey::Ctrl('j'));
        handle_event(&mut session, TuiEvent::Paste("12a3".to_owned()), 80, 24);
        assert_eq!(session.prompt_input, "123");

        key(&mut session, TuiKey::Escape);
        key(&mut session, TuiKey::Ctrl('f'));
        handle_event(&mut session, TuiEvent::Paste("one\rtwo".to_owned()), 80, 24);
        assert_eq!(session.prompt_input, "one");
    }

    #[test]
    fn expired_status_messages_are_cleared() {
        let mut session = untitled();
        assert!(!session.expire_status());
        session.set_status("recent");
        assert!(!session.expire_status());
        session.status_message = Some((
            "old".to_owned(),
            std::time::Instant::now()
                .checked_sub(STATUS_LIFETIME)
                .expect("the clock is past the status lifetime"),
        ));
        assert!(session.expire_status());
        assert!(session.status_message.is_none());
    }

    /// Bytes stored for a recovery path. Must match `path_to_bytes`.
    fn stored_recovery_path(path: &std::path::Path) -> Vec<u8> {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            path.as_os_str().as_bytes().to_vec()
        }
        #[cfg(not(unix))]
        {
            path.to_string_lossy().into_owned().into_bytes()
        }
    }

    fn untitled_with_store(root: &std::path::Path) -> TuiSession {
        TuiSession::new(
            &LaunchOptions::default(),
            CrashRecoverySession::open_at(root),
        )
        .expect("an untitled session always starts")
    }

    #[test]
    fn unsaved_text_survives_a_lost_terminal_and_is_offered_on_the_next_launch() {
        let store = tempfile::tempdir().unwrap();
        {
            let mut session = untitled_with_store(store.path());
            assert_eq!(session.prompt, PromptMode::None);
            type_text(&mut session, "draft that must not be lost");
            assert!(session.persist_before_exit());
        }

        let mut next = untitled_with_store(store.path());
        assert_eq!(next.prompt, PromptMode::RecoveryOffer);
        let offer = strip_own_sequences(&render_frame(&next, 120, 24));
        assert!(offer.contains("Unsaved text from Untitled"), "{offer}");

        key(&mut next, TuiKey::Char('r'));
        assert_eq!(next.prompt, PromptMode::None);
        assert_eq!(next.text(), "draft that must not be lost");
        assert!(next.document.is_dirty());
        // The restored text is one fresh history, not an undo of the restore.
        next.undo();
        assert_eq!(next.text(), "draft that must not be lost");
    }

    #[test]
    fn restoring_one_offer_keeps_the_others_for_a_later_launch() {
        let store = tempfile::tempdir().unwrap();
        for text in ["first lost draft", "second lost draft"] {
            let mut session = untitled_with_store(store.path());
            key(&mut session, TuiKey::Char('l'));
            type_text(&mut session, text);
            assert!(session.persist_before_exit());
        }

        let mut session = untitled_with_store(store.path());
        assert_eq!(session.prompt, PromptMode::RecoveryOffer);
        key(&mut session, TuiKey::Char('r'));
        let restored = session.text();
        assert_eq!(session.prompt, PromptMode::None);
        // Another r is ordinary typing, never a second restore.
        key(&mut session, TuiKey::Char('r'));
        assert_eq!(session.text(), format!("{restored}r"));
        drop(session);

        assert_eq!(
            untitled_with_store(store.path()).prompt,
            PromptMode::RecoveryOffer
        );
    }

    #[test]
    fn saving_a_restored_local_copy_offers_a_separate_roaming_copy() {
        let root = tempfile::tempdir().expect("state root");
        let local_state = root.path().join("local");
        let legacy_state = root.path().join("legacy");
        let legacy_store = RecoveryStore::open_in_state(&legacy_state).expect("legacy store");
        let local_store = RecoveryStore::open_in_state(&local_state).expect("local store");
        // A relative `notes.txt` would bind to whatever file has that name
        // in the working directory. This path is absent, so restore stays
        // untitled and the later Save As is the path under test.
        let missing = root.path().join("missing-notes.txt");
        let original_path = stored_recovery_path(&missing);
        let parts = |instance, created_at, updated_at, content: &[u8]| RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([91; 16]),
            instance_id: RecoveryInstanceId::new([instance; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(created_at),
            updated_at: RecoveryWallTime::from_unix_millis(updated_at),
            original_path: original_path.clone(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: content.to_vec(),
        };
        let old =
            RecoverySnapshot::try_new(parts(91, 1, 2, b"roaming work")).expect("legacy snapshot");
        legacy_store.persist(&old).expect("legacy record");
        let local = RecoverySnapshot::try_new_with_lineage(
            parts(92, 3, 4, b"local work"),
            RecoveryLineageGeneration::new(2),
            Some(old.instance_id()),
        )
        .expect("generation-gap successor");
        local_store.persist(&local).expect("local record");

        let recovery =
            CrashRecoverySession::open_with_legacy_state(&local_state, Some(&legacy_state));
        let mut session = TuiSession::new(&LaunchOptions::default(), recovery).expect("session");
        assert_eq!(session.prompt, PromptMode::RecoveryOffer);
        key(&mut session, TuiKey::Char('r'));
        assert_eq!(session.text(), "local work");
        assert_eq!(session.prompt, PromptMode::None);
        assert!(session.recovery.has_pending_legacy_review());

        trigger_exit(&mut session);
        assert_eq!(session.prompt, PromptMode::ExitConfirm);
        key(&mut session, TuiKey::Char('y'));
        assert_eq!(session.prompt, PromptMode::SaveAs);
        let destination = root.path().join("saved.txt");
        type_text(&mut session, &destination.to_string_lossy());
        key(&mut session, TuiKey::Enter);
        assert_eq!(session.prompt, PromptMode::RecoveryOffer);
        assert!(!session.should_exit);
        assert_eq!(session.after_save, AfterSave::Stay);
        assert!(!session.document.is_dirty());
        assert_eq!(
            std::fs::read_to_string(&destination).expect("saved document"),
            "local work"
        );
        assert_eq!(
            session
                .recovery
                .active_offer()
                .expect("Roaming review")
                .metadata()
                .instance_id(),
            old.instance_id()
        );
        key(&mut session, TuiKey::Char('l'));
        assert_eq!(session.prompt, PromptMode::None);
        assert!(legacy_store.record_path(old.instance_id()).exists());
    }

    #[test]
    fn saving_or_discarding_leaves_nothing_to_recover() {
        let store = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir().unwrap();
        {
            let mut session = untitled_with_store(store.path());
            type_text(&mut session, "saved text");
            assert!(session.persist_before_exit());
            key(&mut session, TuiKey::Ctrl('s'));
            type_text(
                &mut session,
                &directory.path().join("saved.txt").to_string_lossy(),
            );
            key(&mut session, TuiKey::Enter);
            assert!(!session.document.is_dirty());
        }
        assert_eq!(untitled_with_store(store.path()).prompt, PromptMode::None);

        {
            let mut session = untitled_with_store(store.path());
            type_text(&mut session, "thrown away");
            assert!(session.persist_before_exit());
            key(&mut session, TuiKey::Ctrl('x'));
            key(&mut session, TuiKey::Char('n'));
            assert!(session.should_exit);
        }
        assert_eq!(untitled_with_store(store.path()).prompt, PromptMode::None);
    }

    #[test]
    fn a_later_answer_keeps_the_offer_and_an_explicit_file_skips_it() {
        let store = tempfile::tempdir().unwrap();
        {
            let mut session = untitled_with_store(store.path());
            type_text(&mut session, "keep for later");
            assert!(session.persist_before_exit());
        }
        let mut later = untitled_with_store(store.path());
        key(&mut later, TuiKey::Char('l'));
        assert_eq!(later.prompt, PromptMode::None);
        assert_eq!(later.text(), "");
        drop(later);

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        std::fs::write(&path, "file").unwrap();
        let explicit = TuiSession::new(
            &LaunchOptions {
                initial_path: Some(path),
                ..LaunchOptions::default()
            },
            CrashRecoverySession::open_at(store.path()),
        )
        .unwrap();
        assert_eq!(explicit.prompt, PromptMode::None);
        drop(explicit);

        let mut restored = untitled_with_store(store.path());
        assert_eq!(restored.prompt, PromptMode::RecoveryOffer);
        key(&mut restored, TuiKey::Char('d'));
        assert_eq!(restored.prompt, PromptMode::None);
        drop(restored);
        assert_eq!(untitled_with_store(store.path()).prompt, PromptMode::None);
    }

    #[test]
    fn opening_the_same_path_shows_a_durable_hint_and_keeps_the_copy() {
        let root = tempfile::tempdir().unwrap();
        let notes = root.path().join("notes");
        std::fs::create_dir(&notes).unwrap();
        let draft = notes.join("draft notes.md");
        let other = root.path().join("draft notes.md");
        std::fs::write(&draft, "disk text").unwrap();
        std::fs::write(&other, "other disk").unwrap();
        let store = tempfile::tempdir().unwrap();
        {
            let mut session = session_for_with_store(&draft, store.path());
            assert!(session.recovery_path_hint.is_none());
            session.caret_byte = session.text().len();
            type_text(&mut session, " unsaved");
            assert!(session.persist_before_exit());
        }

        let mut reopened = session_for_with_store(&draft, store.path());
        assert_eq!(reopened.prompt, PromptMode::None);
        assert_eq!(reopened.text(), "disk text");
        let hint = reopened
            .recovery_path_hint
            .clone()
            .expect("a matching copy leaves a note");
        assert!(hint.contains("no file"));
        assert!(hint.contains("Esc hides this."));
        reopened.status_message = Some((
            "old".to_owned(),
            std::time::Instant::now()
                .checked_sub(STATUS_LIFETIME)
                .expect("clock"),
        ));
        assert!(reopened.expire_status());
        let frame = strip_own_sequences(&render_frame(&reopened, 80, 24));
        assert!(frame.contains(&hint), "{frame}");
        assert_eq!(recovery_record_paths(store.path()).len(), 1);
        key(&mut reopened, TuiKey::Escape);
        assert!(reopened.recovery_path_hint.is_none());
        let cleared = strip_own_sequences(&render_frame(&reopened, 80, 24));
        assert!(cleared.contains("Ln 1, Col 1"), "{cleared}");
        assert!(!cleared.contains("no file"), "{cleared}");
        drop(reopened);
        assert_eq!(recovery_record_paths(store.path()).len(), 1);

        let other_open = session_for_with_store(&other, store.path());
        assert!(other_open.recovery_path_hint.is_none());
        assert_eq!(other_open.text(), "other disk");
        drop(other_open);

        let untitled = untitled_with_store(store.path());
        assert_eq!(untitled.prompt, PromptMode::RecoveryOffer);
        let wide = strip_own_sequences(&render_frame(&untitled, 120, 24));
        assert!(wide.contains("notes/draft notes.md"), "{wide}");
        assert!(wide.contains("UTC"), "{wide}");
        let narrow = strip_own_sequences(&render_frame(&untitled, 80, 24));
        assert!(narrow.contains("(r)estore"), "{narrow}");
        assert!(narrow.contains("(d)iscard"), "{narrow}");
        assert!(narrow.contains("(l)ater"), "{narrow}");
        assert!(narrow.contains("notes/draft notes.md"), "{narrow}");
    }

    #[test]
    fn restore_reconnects_an_existing_file_and_save_writes_it_back() {
        let root = tempfile::tempdir().unwrap();
        let draft = root.path().join("draft notes.md");
        std::fs::write(&draft, "disk text").unwrap();
        let store = tempfile::tempdir().unwrap();
        {
            let mut session = session_for_with_store(&draft, store.path());
            session.caret_byte = session.text().len();
            type_text(&mut session, " unsaved");
            assert!(session.persist_before_exit());
        }

        let mut session = untitled_with_store(store.path());
        assert_eq!(session.prompt, PromptMode::RecoveryOffer);
        key(&mut session, TuiKey::Char('r'));
        assert_eq!(session.prompt, PromptMode::None);
        assert_eq!(session.text(), "disk text unsaved");
        assert_eq!(session.document.path(), Some(draft.as_path()));
        assert!(session.document.is_dirty());
        let note = session.recovery_path_hint.as_deref().expect("restore note");
        assert!(
            note.starts_with(crate::crash_recovery::RESTORED_BOUND),
            "{note}"
        );
        key(&mut session, TuiKey::Ctrl('s'));
        assert!(!session.document.is_dirty());
        assert_eq!(
            std::fs::read_to_string(&draft).expect("written"),
            "disk text unsaved"
        );
    }

    #[test]
    fn restore_of_a_missing_file_stays_untitled_and_says_why() {
        let root = tempfile::tempdir().unwrap();
        let draft = root.path().join("gone.md");
        std::fs::write(&draft, "disk text").unwrap();
        let store = tempfile::tempdir().unwrap();
        {
            let mut session = session_for_with_store(&draft, store.path());
            session.caret_byte = session.text().len();
            type_text(&mut session, " kept");
            assert!(session.persist_before_exit());
        }
        std::fs::remove_file(&draft).unwrap();

        let mut session = untitled_with_store(store.path());
        key(&mut session, TuiKey::Char('r'));
        assert_eq!(session.text(), "disk text kept");
        assert!(session.document.path().is_none());
        assert!(session.document.is_dirty());
        let note = session.recovery_path_hint.as_deref().expect("restore note");
        assert!(
            note.starts_with(crate::crash_recovery::RESTORED_MISSING),
            "{note}"
        );
        assert!(!note.contains("gone.md"), "{note}");
    }

    #[test]
    fn status_reports_one_based_display_columns() {
        let mut session = untitled();
        session.insert_str("世界");
        let frame = strip_own_sequences(&render_frame(&session, 80, 24));
        assert!(frame.contains("Ln 1, Col 5"), "{frame}");
    }

    #[test]
    fn line_table_matches_core_line_splitting() {
        let text = "a\r\nb\rc\u{2028}d\n";
        let table = LineTable::new(text);
        assert_eq!(table.len(), 4);
        assert_eq!(
            table.line(1),
            LineSpan {
                start: 3,
                end: 4,
                next: 5
            }
        );
        assert_eq!(&text[table.line(2).start..table.line(2).end], "c\u{2028}d");
        assert_eq!(
            table.line(3),
            LineSpan {
                start: text.len(),
                end: text.len(),
                next: text.len()
            }
        );
        assert_eq!(table.line_of(4), 1);
        assert_eq!(LineTable::new("").len(), 1);
        assert_eq!(LineTable::new("x").len(), 1);
    }

    proptest::proptest! {
        #[test]
        fn character_steps_match_core_navigation_on_the_whole_text(
            text in "[a\\r\\n\\u{301}é世😀\\u{200d}\\u{2028}\\u{85}\\u{feff}]{0,24}",
            position in 0_usize..64,
        ) {
            let mut session = untitled();
            session.insert_str(&text);
            let text = session.text();
            let mut caret = position.min(text.len());
            while !text.is_char_boundary(caret) {
                caret -= 1;
            }
            session.caret_byte = caret;
            for direction in [MoveDirection::Backward, MoveDirection::Forward] {
                proptest::prop_assert_eq!(
                    session.character_step(direction),
                    move_caret(&text, caret, direction, MoveUnit::Character)
                );
            }
        }

        #[test]
        fn frames_never_emit_unsafe_characters(text in proptest::prelude::any::<String>(), cols in 1_u16..120, rows in 1_u16..16) {
            let mut session = untitled();
            session.insert_str(&text);
            session.search_query = Some(text.chars().take(2).collect());
            for view in [DocumentView::Text, DocumentView::Markdown] {
                session.view = view;
                session.scroll_to_caret(cols, rows);
                let frame = render_frame(&session, cols, rows);
                assert_frame_is_terminal_safe(&frame, usize::from(cols));
                proptest::prop_assert!(!paint::contains_truecolor(&frame));
                proptest::prop_assert!(strip_own_sequences(&frame).split("\r\n").count() <= usize::from(rows));
            }
        }
    }

    #[test]
    fn terminal_frames_follow_the_host_and_honor_no_color() {
        use crate::theme::AppTheme;

        for theme in [
            AppTheme::Light,
            AppTheme::Dark,
            AppTheme::GreenScreen,
            AppTheme::AmberScreen,
            AppTheme::System,
        ] {
            let mut session = without_startup_status(
                TuiSession::new(
                    &LaunchOptions {
                        theme: Some(theme),
                        view: Some(DocumentView::Markdown),
                        ..LaunchOptions::default()
                    },
                    CrashRecoverySession::disabled_for_test(),
                )
                .expect("theme flag does not affect terminal startup"),
            );
            session.terminal_color = TerminalColor::Host;
            session.insert_str("# Heading\nbeta line\n");
            session.search_query = Some("beta".to_owned());
            session.set_toned_status("Wrote 4 bytes", StatusTone::Success);
            let frame = render_frame(&session, 80, 24);
            let visible = strip_own_sequences(&frame);
            assert!(!paint::contains_truecolor(&frame), "{theme:?}");
            assert!(visible.contains("[Markdown]"), "{theme:?}");
            assert!(!visible.contains(theme.label()), "{theme:?}");
            assert!(frame.contains("\x1b[1;36m"), "{theme:?}");
            assert!(frame.contains("\x1b[32m"), "{theme:?}");
            assert!(frame.contains("\x1b[0m\x1b[7m"), "{theme:?}");
            assert!(frame.contains("\x1b[2m"), "{theme:?}");

            session.terminal_color = TerminalColor::Off;
            let plain = render_frame(&session, 80, 24);
            assert!(!paint::contains_truecolor(&plain));
            assert!(!paint::contains_color_sgr(&plain));
            assert!(plain.contains("\x1b[7m"));
            assert!(plain.contains("\x1b[1m"));
            assert!(plain.contains("\x1b[2m"));
            assert!(!plain.contains("\x1b[32m"));
            assert!(!plain.contains("\x1b[36m"));
        }
    }

    #[test]
    fn save_status_roles_match_the_outcome() {
        let mut session = untitled();
        session.terminal_color = TerminalColor::Host;
        session.set_toned_status("Wrote 3 bytes", StatusTone::Success);
        let frame = render_frame(&session, 80, 24);
        assert!(frame.contains("\x1b[32m"));
        assert!(strip_own_sequences(&frame).contains("Wrote 3 bytes"));

        session.set_toned_status(
            "Saved with uncertain durability. Keep open and Save again. ^R details.",
            StatusTone::Warning,
        );
        assert!(render_frame(&session, 80, 24).contains("\x1b[33m"));

        session.set_save_error("Not saved: the folder refused the write");
        assert!(render_frame(&session, 80, 24).contains("\x1b[31m"));

        session.status_message = None;
        session.recovery_unavailable_hint =
            Some("Recovery is off for this session: disk full".to_owned());
        let hint = render_frame(&session, 80, 24);
        assert!(hint.contains("\x1b[33m"));
        assert!(strip_own_sequences(&hint).contains("Recovery is off for this session"));

        session.terminal_color = TerminalColor::Off;
        let plain = render_frame(&session, 80, 24);
        assert!(!paint::contains_color_sgr(&plain));
        assert!(strip_own_sequences(&plain).contains("Recovery is off for this session"));
    }

    #[test]
    fn save_warning_screen_is_yellow_and_not_truecolor() {
        use noter::core::save::{SaveStage, SaveWarnings, StorageError};

        let mut session = untitled();
        session.terminal_color = TerminalColor::Host;
        session.last_save_warnings = Some(SaveWarnings::new(
            vec![StorageError::new(SaveStage::Cleanup, "kept a sibling")],
            Vec::new(),
        ));
        session.prompt = PromptMode::SaveWarnings;
        let frame = render_frame(&session, 80, 24);
        let visible = strip_own_sequences(&frame);
        assert!(!paint::contains_truecolor(&frame));
        assert!(frame.contains("\x1b[33m"));
        assert!(visible.contains("Latest save warnings"));
        assert!(visible.contains("kept a sibling"));
        assert!(!visible.contains("38;2"));
    }

    #[test]
    fn markdown_structure_spans_stay_on_the_source_cells() {
        assert_eq!(
            markdown_spans("# Heading One"),
            vec![(0..13, Style::Heading)]
        );
        assert_eq!(markdown_spans("  - item"), vec![(2..3, Style::Marker)]);
        assert_eq!(markdown_spans("12. item"), vec![(0..3, Style::Marker)]);
        assert_eq!(
            markdown_spans("> quote"),
            vec![(0..1, Style::Muted), (1..7, Style::Quote)]
        );
        assert_eq!(markdown_spans("```rust"), vec![(0..7, Style::Muted)]);
        assert!(markdown_spans("#hashtag").is_empty());
        assert!(markdown_spans("plain").is_empty());
    }
}
