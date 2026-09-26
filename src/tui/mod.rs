//! Native Terminal User Interface (TUI) mode for Noter.
//!
//! Provides a fast, focused terminal editor ("like Nano... but better") powered
//! by Noter's authoritative core document engine and platform primitives.
//!
//! Features:
//! - Dual shortcuts: Nano (^O Save As with a prefilled name, ^X Exit, ^W `WhereIs`, ^K Cut,
//!   ^U Paste, ^J Jump, ^G Help) and Modern (Ctrl+S, Ctrl+Q, Ctrl+F, Ctrl+Z, Ctrl+Y, Ctrl+E,
//!   Ctrl+T, Ctrl+A).
//! - Saving never exits or discards text unless the write committed. Replacing an existing
//!   file or splitting a hard link asks first.
//! - Phosphor CRT themes (Green Screen and Amber Screen) matching Noter's desktop visuals.
//! - Fluid formatted Markdown viewing toggle (^E).
//! - Terminal mouse support (click to set caret, scroll wheel to scroll lines, clickable legend).
//! - Authoritative revision tracking, atomic safe save, and Undo/Redo history.

use std::fmt::Write as FmtWrite;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use noter::core::document::{Document, PreparedSaveAs};
use noter::core::edit::{
    EditOrigin, EditTimestamp, EditTransaction, Selection, TextEdit, TextRange,
};
use noter::core::limits::MAX_DOCUMENT_BYTES;
use noter::core::line_endings::{logical_lines, normalize_inserted_text};
use noter::core::navigation::{
    LineNavigationError, MoveDirection, MoveUnit, line_start_offset, move_caret,
};
use noter::core::save::SaveOutcome;
use noter::core::search::{LiteralSearch, MatchCase, SearchDirection};
use noter::core::terminal_text::{
    cell_width, column_of, display_width, fit_line, offset_at_column, push_display,
};
use noter::core::undo::{
    HistoryApplyOutcome, HistoryError, HistoryLimits, HistoryRecordOutcome, UndoHistory,
};
use noter::error::NoterError;

use crate::app::{DocumentView, LaunchOptions};
use crate::crash_recovery::{
    CrashRecoverySession, RECOVERY_CLEANUP_FAILURE_MESSAGE, RECOVERY_PERSIST_FAILURE_MESSAGE,
    RECOVERY_UNAVAILABLE_MESSAGE,
};

mod input;

use crate::theme::AppTheme;
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
    SplitHardLinkInPlace,
    /// Save As over an existing file.
    Replace(PreparedSaveAs),
    /// Save As over an existing file, splitting its hard link.
    SplitHardLink(PreparedSaveAs),
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

/// ANSI color definition for TUI rendering.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AnsiColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl AnsiColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// Palette colors for TUI rendering matching Noter desktop themes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct TuiPalette {
    pub bg: AnsiColor,
    pub fg: AnsiColor,
    pub bar_bg: AnsiColor,
    pub bar_fg: AnsiColor,
    pub gutter_fg: AnsiColor,
    pub accent: AnsiColor,
    pub match_bg: AnsiColor,
    pub match_fg: AnsiColor,
}

impl TuiPalette {
    #[must_use]
    pub const fn for_theme(theme: AppTheme) -> Self {
        match theme {
            AppTheme::GreenScreen => Self {
                bg: AnsiColor::new(3, 10, 5),
                fg: AnsiColor::new(174, 255, 177),
                bar_bg: AnsiColor::new(14, 40, 18),
                bar_fg: AnsiColor::new(174, 255, 177),
                gutter_fg: AnsiColor::new(70, 130, 80),
                accent: AnsiColor::new(66, 255, 78),
                match_bg: AnsiColor::new(66, 255, 78),
                match_fg: AnsiColor::new(3, 10, 5),
            },
            AppTheme::AmberScreen => Self {
                bg: AnsiColor::new(14, 9, 2),
                fg: AnsiColor::new(255, 215, 137),
                bar_bg: AnsiColor::new(45, 30, 10),
                bar_fg: AnsiColor::new(255, 215, 137),
                gutter_fg: AnsiColor::new(150, 115, 60),
                accent: AnsiColor::new(255, 180, 40),
                match_bg: AnsiColor::new(255, 180, 40),
                match_fg: AnsiColor::new(14, 9, 2),
            },
            AppTheme::Light => Self {
                bg: AnsiColor::new(250, 251, 251),
                fg: AnsiColor::new(31, 33, 36),
                bar_bg: AnsiColor::new(225, 228, 233),
                bar_fg: AnsiColor::new(31, 33, 36),
                gutter_fg: AnsiColor::new(130, 135, 145),
                accent: AnsiColor::new(13, 110, 253),
                match_bg: AnsiColor::new(255, 235, 59),
                match_fg: AnsiColor::new(31, 33, 36),
            },
            AppTheme::Dark | AppTheme::System => Self {
                bg: AnsiColor::new(20, 22, 25),
                fg: AnsiColor::new(230, 232, 236),
                bar_bg: AnsiColor::new(35, 39, 45),
                bar_fg: AnsiColor::new(230, 232, 236),
                gutter_fg: AnsiColor::new(100, 105, 115),
                accent: AnsiColor::new(77, 159, 255),
                match_bg: AnsiColor::new(180, 130, 20),
                match_fg: AnsiColor::new(255, 255, 255),
            },
        }
    }
}

/// State for the terminal editor session.
pub struct TuiSession {
    pub document: Document,
    pub theme: AppTheme,
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
    pub search_query: Option<String>,
    pub show_help: bool,
    /// Bounded undo and redo, shared with the window through the core.
    pub history: UndoHistory,
    /// Private crash-recovery records, shared with the window's store.
    pub recovery: CrashRecoverySession,
    /// Monotonic origin for edit timestamps, so typing coalesces into Undo
    /// steps by the same rules as the window.
    pub started: Instant,
    pub should_exit: bool,
    pub after_save: AfterSave,
    pub pending_save: Option<PendingSave>,
    /// Paths where an earlier save may or may not have reached disk.
    ///
    /// Writing there again would hide which version is on disk, so saves to
    /// these paths are refused for the rest of the session. Other destinations
    /// stay available so the text is never trapped.
    pub uncertain_paths: Vec<PathBuf>,
}

impl TuiSession {
    /// Initializes a new TUI session from launch options.
    ///
    /// An explicit file keeps any startup recovery offers for a later
    /// untitled launch, as the window does; an untitled launch presents the
    /// first offer.
    ///
    /// # Errors
    ///
    /// Returns an error if an initial file path was supplied but cannot be loaded.
    pub fn new(options: &LaunchOptions, recovery: CrashRecoverySession) -> Result<Self, String> {
        let document = if let Some(path) = &options.initial_path {
            Document::from_path(path)
                .map_err(|e| format!("cannot load `{}`: {e}", path.display()))?
        } else {
            Document::new()
        };

        let theme = options.theme.unwrap_or(AppTheme::Dark);
        let view = options.view.unwrap_or(DocumentView::Text);
        let history = UndoHistory::new(HistoryLimits::default(), document.revision());

        let mut session = Self {
            document,
            theme,
            view,
            caret_byte: 0,
            scroll_row: 0,
            scroll_col: 0,
            follow_caret: true,
            clipboard: String::new(),
            prompt: PromptMode::None,
            prompt_input: String::new(),
            status_message: None,
            search_query: None,
            show_help: false,
            history,
            recovery,
            started: Instant::now(),
            should_exit: false,
            after_save: AfterSave::Stay,
            pending_save: None,
            uncertain_paths: Vec::new(),
        };
        if options.initial_path.is_some() {
            session.recovery.defer_startup_offers();
            session.recovery.begin_fresh_identity();
        } else if session.recovery.active_offer().is_some() {
            session.prompt = PromptMode::RecoveryOffer;
        }
        if session.recovery.is_unavailable() {
            session.set_status(RECOVERY_UNAVAILABLE_MESSAGE);
        }
        Ok(session)
    }

    /// Tells crash recovery whether the text now needs a private copy, after
    /// Undo, Redo, or a save, as the window's `synchronize_crash_recovery`
    /// does. A direct edit uses `on_edited` instead, also as the window does.
    fn sync_recovery(&mut self) {
        let selection = Selection::caret(self.caret_byte);
        if self.document.is_dirty() {
            self.recovery.on_retained(&self.document, selection);
        } else {
            self.recovery.on_saved_clean(self.document.revision());
        }
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
        self.recovery
            .persist_before_exit(&self.document, selection, EXIT_PERSIST_LIMIT)
    }

    /// Answers the startup recovery offer that is showing.
    pub fn answer_recovery_offer(&mut self, decision: RecoveryDecision) {
        match decision {
            RecoveryDecision::Restore => match self.recovery.restore_active_offer() {
                Ok((document, selection)) => {
                    self.document = document;
                    self.caret_byte = selection.active();
                    self.history.reset(self.document.revision());
                    self.recovery.on_edited(&self.document, selection);
                    // Remaining offers stay on disk for a later untitled
                    // launch; presenting the next one now would replace the
                    // text just restored.
                    self.recovery.defer_startup_offers();
                    self.set_status("Restored unsaved text. Save it to keep it.");
                }
                Err(message) => self.set_status(message),
            },
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

    /// Sets a temporary status message visible for 3 seconds.
    pub fn set_status(&mut self, message: impl Into<String>) {
        self.status_message = Some((message.into(), std::time::Instant::now()));
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
        let Some(target) = self.document.path().map(std::path::Path::to_path_buf) else {
            return self.open_save_as();
        };
        if self.is_uncertain(Some(&target)) {
            self.set_status(UNCERTAIN_SAVE_GUIDANCE);
            return SaveStep::NotSaved;
        }
        match self.document.save() {
            Err(NoterError::HardLinkedTarget(link_count)) => {
                self.pending_save = Some(PendingSave::SplitHardLinkInPlace);
                self.prompt = PromptMode::ConfirmHardLink(link_count);
                SaveStep::AwaitingInput
            }
            result => self.report_save(target, result),
        }
    }

    /// Opens the Save As prompt (^O), prefilled with the current path.
    pub fn open_save_as(&mut self) -> SaveStep {
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
        let prepared = match self.document.prepare_save_as(&path) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.set_status(format!("Not saved: {error}"));
                return SaveStep::NotSaved;
            }
        };
        if self.is_uncertain(Some(&path)) {
            self.set_status(UNCERTAIN_SAVE_GUIDANCE);
            return SaveStep::NotSaved;
        }
        // Saving to the document's own path is an ordinary save: the core
        // compares against the saved baseline and reports an external change
        // as a conflict, so there is nothing new to confirm.
        let is_current_path = self.document.path() == Some(path.as_path());
        if prepared.replaces_existing() && !is_current_path {
            self.pending_save = Some(PendingSave::Replace(prepared));
            self.prompt = PromptMode::ConfirmReplace;
            return SaveStep::AwaitingInput;
        }
        self.save_prepared(prepared, false)
    }

    fn is_uncertain(&self, path: Option<&std::path::Path>) -> bool {
        path.is_some_and(|path| {
            self.uncertain_paths
                .iter()
                .any(|uncertain| uncertain == path)
        })
    }

    /// Answers the replace or hard-link confirmation that is showing.
    pub fn confirm_pending_save(&mut self, confirmed: bool) -> SaveStep {
        self.prompt = PromptMode::None;
        let Some(pending) = self.pending_save.take() else {
            return SaveStep::NotSaved;
        };
        if !confirmed {
            self.set_status("Not saved");
            return SaveStep::NotSaved;
        }
        match pending {
            PendingSave::SplitHardLinkInPlace => {
                let Some(target) = self.document.path().map(std::path::Path::to_path_buf) else {
                    return SaveStep::NotSaved;
                };
                let result = self.document.save_confirming_hard_link_replacement();
                self.report_save(target, result)
            }
            PendingSave::Replace(prepared) => self.save_prepared(prepared, false),
            PendingSave::SplitHardLink(prepared) => self.save_prepared(prepared, true),
        }
    }

    fn save_prepared(&mut self, prepared: PreparedSaveAs, hard_link_confirmed: bool) -> SaveStep {
        let target = prepared.path().to_path_buf();
        let result = if hard_link_confirmed {
            self.document
                .save_prepared_as_confirming_hard_link_replacement(prepared.clone())
        } else {
            self.document.save_prepared_as(prepared.clone())
        };
        if let Err(NoterError::HardLinkedTarget(link_count)) = result {
            self.pending_save = Some(PendingSave::SplitHardLink(prepared));
            self.prompt = PromptMode::ConfirmHardLink(link_count);
            return SaveStep::AwaitingInput;
        }
        self.report_save(target, result)
    }

    /// Turns a save result into status text and the next step.
    fn report_save(
        &mut self,
        target: PathBuf,
        result: Result<SaveOutcome, NoterError>,
    ) -> SaveStep {
        match result {
            Ok(SaveOutcome::Committed {
                observation,
                warnings,
                ..
            }) => {
                let warning_count = warnings.cleanup().len() + warnings.durability().len();
                self.sync_recovery();
                if warning_count == 0 {
                    self.set_status(format!("Wrote {} bytes", observation.length()));
                } else {
                    self.set_status(format!(
                        "Wrote {} bytes with {warning_count} storage warning(s)",
                        observation.length()
                    ));
                }
                SaveStep::Committed
            }
            Ok(SaveOutcome::Conflict { .. }) => {
                self.set_status(
                    "Not saved: the file changed on disk. Use ^O Save As to keep this text under a new name.",
                );
                SaveStep::NotSaved
            }
            Ok(SaveOutcome::NotCommitted { error, .. }) => {
                self.set_status(format!("Not saved: {}", error.message()));
                SaveStep::NotSaved
            }
            Ok(SaveOutcome::CommitStateUnknown {
                recovery_artifact, ..
            }) => {
                if !self.is_uncertain(Some(&target)) {
                    self.uncertain_paths.push(target);
                }
                self.set_status(format!(
                    "Save outcome unknown: {}. {UNCERTAIN_SAVE_GUIDANCE}",
                    recovery_artifact.message()
                ));
                SaveStep::NotSaved
            }
            Err(error) => {
                self.set_status(format!("Not saved: {error}"));
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
                if self.after_save == AfterSave::Exit {
                    self.should_exit = true;
                }
            }
            SaveStep::NotSaved => self.after_save = AfterSave::Stay,
            SaveStep::AwaitingInput => {}
        }
    }

    /// Cycles the current visual theme (^T).
    pub fn cycle_theme(&mut self) {
        self.theme = match self.theme {
            AppTheme::Dark | AppTheme::System => AppTheme::GreenScreen,
            AppTheme::GreenScreen => AppTheme::AmberScreen,
            AppTheme::AmberScreen => AppTheme::Light,
            AppTheme::Light => AppTheme::Dark,
        };
        self.set_status(format!("Theme: {}", self.theme.label()));
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
    "Check that file on disk. Saving to it is paused; ^O Save As can write another file.";

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

fn push_sgr(out: &mut String, style: Style, palette: &TuiPalette) {
    let (background, foreground, attributes) = match style {
        Style::Text => (palette.bg, palette.fg, ""),
        Style::Match => (palette.match_bg, palette.match_fg, "\x1b[1m"),
        Style::Heading => (palette.bg, palette.accent, "\x1b[1m"),
        Style::Marker => (palette.bg, palette.accent, ""),
        Style::Quote => (palette.bg, palette.fg, "\x1b[3m"),
        Style::Muted => (palette.bg, palette.gutter_fg, ""),
    };
    let _ = write!(
        out,
        "\x1b[0m\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m{attributes}",
        background.r, background.g, background.b, foreground.r, foreground.g, foreground.b,
    );
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
    palette: &TuiPalette,
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
                push_sgr(out, style, palette);
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
        push_sgr(out, Style::Text, palette);
    }
    out.extend(std::iter::repeat_n(' ', width - drawn));
}

fn push_bar(
    out: &mut String,
    text: &str,
    cols: usize,
    background: AnsiColor,
    foreground: AnsiColor,
    bold: bool,
) {
    let _ = write!(
        out,
        "\x1b[0m\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m{}{}\x1b[0m",
        background.r,
        background.g,
        background.b,
        foreground.r,
        foreground.g,
        foreground.b,
        if bold { "\x1b[1m" } else { "" },
        fit_line(text, cols),
    );
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
            return Some((
                format!(
                    "Unsaved text from {label} was kept after Noter closed. (r)estore, (d)iscard, (l)ater"
                ),
                "",
                "",
            ));
        }
        PromptMode::None => return None,
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
fn push_viewport(
    out: &mut String,
    frame: &Frame<'_>,
    columns: usize,
    height: usize,
    palette: &TuiPalette,
) {
    let &Frame {
        session,
        text,
        ref table,
        caret_line,
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
            let gutter = if index == caret_line {
                palette.accent
            } else {
                palette.gutter_fg
            };
            let _ = write!(
                out,
                "\x1b[0m\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m {:>digits$} │ ",
                palette.bg.r,
                palette.bg.g,
                palette.bg.b,
                gutter.r,
                gutter.g,
                gutter.b,
                index + 1,
            );
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
            push_line_window(out, line, session.scroll_col, text_width, &spans, palette);
        } else {
            let _ = write!(
                out,
                "\x1b[0m\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m{}",
                palette.bg.r,
                palette.bg.g,
                palette.bg.b,
                palette.gutter_fg.r,
                palette.gutter_fg.g,
                palette.gutter_fg.b,
                fit_line(&format!(" {:>digits$} ", "~"), columns),
            );
        }
        out.push_str("\x1b[0m\r\n");
    }
}

/// Returns the status line: a prompt, a recent message, or the position.
fn status_line(frame: &Frame<'_>) -> String {
    let &Frame {
        session,
        text,
        ref table,
        caret_line,
        caret_column,
    } = frame;
    prompt_parts(session).map_or_else(
        || match &session.status_message {
            Some((message, created)) if created.elapsed() < STATUS_LIFETIME => {
                format!(" {message}")
            }
            _ => format!(
                " Ln {}, Col {}  │  {} B  │  {} L  │  {}  │  {} ",
                caret_line + 1,
                caret_column + 1,
                text.len(),
                table.len(),
                session.document.encoding().status_label(),
                session.document.line_endings().status_label()
            ),
        },
        |(label, input, hint)| format!("{label}{input}{hint}"),
    )
}

/// Renders the complete TUI frame into an output buffer.
///
/// The frame uses the scroll position stored in the session; call
/// [`TuiSession::scroll_to_caret`] first to keep the caret visible.
pub fn render_frame(session: &TuiSession, cols: u16, rows: u16) -> String {
    let columns = cols as usize;
    let mut out = String::with_capacity(columns * rows as usize * 4);
    let palette = TuiPalette::for_theme(session.theme);
    out.push_str("\x1b[?25l\x1b[H");
    if usize::from(rows) <= CHROME_ROWS || columns < MIN_COLUMNS {
        // Too small for the layout: say so in what fits instead of drawing
        // rows that wrap or scroll the terminal.
        out.push_str("\x1b[0m\x1b[2J\x1b[H");
        out.push_str(&fit_line("Enlarge the terminal", columns));
        return out;
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
    let dirty = if session.document.is_dirty() {
        " *"
    } else {
        ""
    };
    let header = format!(
        " Noter {} │ {doc_name}{dirty} │ [{}] │ {} ",
        env!("CARGO_PKG_VERSION"),
        session.view.label(),
        session.theme.label()
    );
    push_bar(
        &mut out,
        &header,
        columns,
        palette.bar_bg,
        palette.bar_fg,
        true,
    );
    out.push_str("\r\n");

    push_viewport(&mut out, &frame, columns, height, &palette);

    let status = status_line(&frame);
    push_bar(
        &mut out,
        &status,
        columns,
        palette.bar_bg,
        palette.bar_fg,
        true,
    );
    out.push_str("\r\n");
    push_bar(&mut out, LEGEND_TOP, columns, palette.bg, palette.fg, false);
    out.push_str("\r\n");
    push_bar(
        &mut out,
        LEGEND_BOTTOM,
        columns,
        palette.bg,
        palette.fg,
        false,
    );

    // The cursor stays hidden unless it has a place: over help, or with the
    // caret scrolled out of view, a visible cursor would mark a wrong cell.
    if session.show_help {
        render_help_overlay(&mut out, cols, rows, &palette);
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

const LEGEND_TOP: &str =
    " ^G Help       ^O Save As    ^W Where Is   ^K Cut Line   ^U Paste      ^J Jump Line ";
const LEGEND_BOTTOM: &str =
    " ^X Exit       ^S Save       ^F Find       ^Z Undo       ^Y Redo       ^E Markdown  ";

fn render_help_overlay(out: &mut String, cols: u16, rows: u16, palette: &TuiPalette) {
    let width = 64.min((cols as usize).saturating_sub(4));
    let height = 18.min((rows as usize).saturating_sub(4));
    let start_col = (cols as usize).saturating_sub(width) / 2 + 1;
    let start_row = (rows as usize).saturating_sub(height) / 2 + 1;

    let lines = [
        "╔══════════════════════════════════════════════════════════════╗",
        "║                     NOTER HELP & SHORTCUTS                   ║",
        "╠══════════════════════════════════════════════════════════════╣",
        "║  Navigation:                                                 ║",
        "║    Arrows / Mouse Click : Move cursor                        ║",
        "║    Home / End           : Line Start / End                   ║",
        "║    PgUp / PgDown        : Page Scroll                        ║",
        "║    Scroll Wheel         : Scroll lines up/down               ║",
        "║                                                              ║",
        "║  Editing & Actions:                                          ║",
        "║    ^S                   : Save file                          ║",
        "║    ^O                   : Save As (asks before replacing)    ║",
        "║    ^X or ^Q             : Exit editor (prompts if modified)  ║",
        "║    ^W or ^F             : Find text in document              ║",
        "║    ^K                   : Cut current line                   ║",
        "║    ^U                   : Paste line                         ║",
        "║    ^Z / ^Y              : Undo / Redo                        ║",
        "║    ^E                   : Toggle Markdown formatted view     ║",
        "║    ^T                   : Cycle Theme (Green/Amber/Dark/Lite)║",
        "║    ^J                   : Jump to Line Number                ║",
        "║    ^G or Esc            : Close this help dialog             ║",
        "╚══════════════════════════════════════════════════════════════╝",
    ];

    for (i, line) in lines.iter().enumerate().take(height) {
        let _ = write!(
            out,
            "\x1b[{};{}H\x1b[48;2;{};{};{}m\x1b[38;2;{};{};{}m\x1b[1m{line:<width$}\x1b[0m",
            start_row + i,
            start_col,
            palette.bar_bg.r,
            palette.bar_bg.g,
            palette.bar_bg.b,
            palette.accent.r,
            palette.accent.g,
            palette.accent.b,
            width = width
        );
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
        needs_frame = true;
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn handle_event(session: &mut TuiSession, event: TuiEvent, _cols: u16, rows: u16) {
    if session.show_help {
        if let TuiEvent::Key(TuiKey::Escape | TuiKey::Ctrl('g') | TuiKey::Enter) = event {
            session.show_help = false;
        }
        return;
    }

    match &session.prompt {
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
        PromptMode::SaveAs => {
            match event {
                TuiEvent::Key(TuiKey::Enter) => {
                    let step = session.submit_save_as();
                    session.finish_save(step);
                }
                TuiEvent::Key(TuiKey::Escape) => {
                    session.prompt = PromptMode::None;
                    session.finish_save(SaveStep::NotSaved);
                    session.set_status("Save As cancelled");
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
                } else if row == rows - 1 {
                    // Clicked top legend: ^G Help, ^O Save, ^W Where Is, ^K Cut Line, ^U Paste, ^J Jump Line
                    if col < 15 {
                        session.show_help = true;
                    } else if col < 29 {
                        session.open_save_as();
                    } else if col < 43 {
                        session.prompt = PromptMode::Find;
                        session.prompt_input.clear();
                    } else if col < 57 {
                        session.cut_line();
                    } else if col < 71 {
                        session.paste();
                    } else {
                        session.prompt = PromptMode::GoToLine;
                        session.prompt_input.clear();
                    }
                } else if row == rows {
                    // Clicked bottom legend: ^X Exit, ^S Save, ^F Find, ^Z Undo, ^Y Redo, ^E Markdown
                    if col < 15 {
                        trigger_exit(session);
                    } else if col < 29 {
                        session.save();
                    } else if col < 43 {
                        session.prompt = PromptMode::Find;
                        session.prompt_input.clear();
                    } else if col < 57 {
                        session.undo();
                    } else if col < 71 {
                        session.redo();
                    } else {
                        session.toggle_view();
                    }
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
            TuiKey::Ctrl('t') => {
                session.cycle_theme();
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
        | PromptMode::RecoveryOffer => {}
    }
}

fn trigger_exit(session: &mut TuiSession) {
    if session.document.is_dirty() {
        session.prompt = PromptMode::ExitConfirm;
    } else {
        session.should_exit = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn tui_session_theme_cycling() {
        let mut session = untitled();
        assert_eq!(session.theme, AppTheme::Dark);

        session.cycle_theme();
        assert_eq!(session.theme, AppTheme::GreenScreen);

        session.cycle_theme();
        assert_eq!(session.theme, AppTheme::AmberScreen);

        session.cycle_theme();
        assert_eq!(session.theme, AppTheme::Light);

        session.cycle_theme();
        assert_eq!(session.theme, AppTheme::Dark);
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
        assert!(
            session
                .status_message
                .as_ref()
                .is_some_and(|(message, _)| message.contains("Save As"))
        );
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
    fn an_uncertain_save_pauses_that_path_but_leaves_others_available() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("note.txt");
        let fresh = directory.path().join("fresh.txt");
        std::fs::write(&path, "text").unwrap();
        let mut session = session_for(&path);
        type_text(&mut session, "edit ");
        session.after_save = AfterSave::Exit;

        let step = session.report_save(
            path.clone(),
            Ok(SaveOutcome::CommitStateUnknown {
                revision: session.document.revision(),
                error: storage_error("rename reported failure"),
                recovery_artifact: storage_error("a private copy was kept"),
            }),
        );
        session.finish_save(step);

        assert_eq!(step, SaveStep::NotSaved);
        assert!(!session.should_exit);
        assert_eq!(session.uncertain_paths, vec![path.clone()]);
        assert_eq!(session.save(), SaveStep::NotSaved);
        session.prompt_input = path.to_string_lossy().into_owned();
        assert_eq!(session.submit_save_as(), SaveStep::NotSaved);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "text");

        session.prompt_input = fresh.to_string_lossy().into_owned();
        assert_eq!(session.submit_save_as(), SaveStep::Committed);
        assert_eq!(std::fs::read_to_string(&fresh).unwrap(), "edit text");
        // The document now lives at the new path, which is not uncertain.
        type_text(&mut session, "more ");
        assert_eq!(session.save(), SaveStep::Committed);
    }

    #[test]
    fn a_save_that_did_not_commit_cancels_a_pending_exit() {
        let mut session = untitled();
        session.after_save = AfterSave::Exit;

        let step = session.report_save(
            PathBuf::from("note.txt"),
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
        assert!(
            session
                .status_message
                .as_ref()
                .is_some_and(|(message, _)| message.contains("disk full"))
        );
        assert!(session.uncertain_paths.is_empty());
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

        // The conflict status points at ^O, which is prefilled with this path.
        key(&mut session, TuiKey::Ctrl('o'));
        key(&mut session, TuiKey::Enter);

        assert_eq!(session.prompt, PromptMode::None);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "someone else");
        assert!(session.document.is_dirty());
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
            "title\u{1B}]0;owned\u{7}\nclip\u{1B}]52;c;ZWNobw==\u{7}\n\u{9B}2J\u{202E}txt",
        );
        for view in [DocumentView::Text, DocumentView::Markdown] {
            session.view = view;
            let frame = render_frame(&session, 80, 24);
            assert!(!frame.contains("\u{1B}]"));
            assert!(!frame.contains('\u{7}'));
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
    fn saving_or_discarding_leaves_nothing_to_recover() {
        let store = tempfile::tempdir().unwrap();
        let directory = tempfile::tempdir().unwrap();
        {
            let mut session = untitled_with_store(store.path());
            type_text(&mut session, "saved text");
            session.recovery.force_due_persist_for_test(
                &session.document,
                Selection::caret(session.caret_byte),
            );
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
                proptest::prop_assert!(strip_own_sequences(&frame).split("\r\n").count() <= usize::from(rows));
            }
        }
    }

    #[test]
    fn tui_palette_and_markdown_rendering() {
        for theme in [
            AppTheme::Light,
            AppTheme::Dark,
            AppTheme::GreenScreen,
            AppTheme::AmberScreen,
            AppTheme::System,
        ] {
            let palette = TuiPalette::for_theme(theme);
            assert!(palette.fg.r != 0 || palette.fg.g != 0 || palette.fg.b != 0);
        }

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
