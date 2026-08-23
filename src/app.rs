use std::fmt::{self, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui;
use noter::core::conflict::{
    ConflictCommand, ConflictDecision, ConflictEffect, ConflictState, classify_external_change,
};
use noter::core::document::{Document, PreparedSaveAs};
use noter::core::edit::{
    AppliedTransaction, EditError, EditOrigin, EditTimestamp, EditTransaction, Selection, TextRange,
};
use noter::core::file_observation::inspect_target;
use noter::core::lifecycle::{
    DestructiveIntent as PendingAbandonAction, DirtyDecision, LifecycleCommand, LifecycleEffect,
    LifecycleState, SaveContinuation,
};
use noter::core::markdown::count_markdown_diagnostics;
use noter::core::revision::Revision;
use noter::core::save::{SaveOutcome, SaveStage};
use noter::core::search::SearchDirection;
use noter::core::undo::{HistoryApplyOutcome, HistoryRecordOutcome, UndoHistory};
use noter::error::NoterError;

use crate::bounded_text_input::{
    BoundedTextBuffer, ImeFrameState, focused_ime_frame_state, isolate_active_ime_commit,
    retain_active_ime_commit_focus, sanitize_bounded_text_events, take_events_after_ime_terminal,
};
use crate::crash_recovery::{
    CrashRecoverySession, RECOVERY_CLEANUP_FAILURE_MESSAGE, RECOVERY_PERSIST_FAILURE_MESSAGE,
    RECOVERY_UNAVAILABLE_MESSAGE,
};
use crate::editor_settings::{
    EditorZoom, PointerZoomAccumulator, TextWrap, WORD_WRAP_STORAGE_KEY, ZOOM_STORAGE_KEY,
    apply_editor_zoom,
};
use crate::find_ui::{FindBar, FindBarAction, ReplaceScope};
use crate::go_to_line_ui::{GoToLineAction, GoToLineDialog};
use crate::idle_screen::IdleScreen;
use crate::keyboard_nav::{
    KeyboardPlatform, consume_navigation_gestures, editor_event_orders_input,
    resolve_navigation_gesture,
};
use crate::markdown_ui::{MarkdownEditor, MarkdownProjectionLimit, markdown_projection_limit};
use crate::theme::{self, AppTheme, THEME_STORAGE_KEY};

const EDITOR_ID_SALT: &str = "noter-document-editor";
const ABOUT_SUMMARY: &str = "A focused editor for plain text and Markdown files.";
const ABOUT_MARKDOWN_STATUS: &str = "Markdown Mode provides a formatted, direct editing surface while keeping ordinary Markdown source authoritative on disk.";
const ABOUT_PRIVACY: &str = "Noter has no accounts, telemetry, or background network activity.";
const ABOUT_LINK_BEHAVIOR: &str = "The project link opens in your default browser.";
const UPDATE_STATUS: &str = "Noter does not check for updates in the background. Open the releases page to compare this version with published builds.";
const RELEASES_URL: &str = "https://github.com/blisspixel/noter/releases";
/// Names the window opened by `noter update` while its status is still shown.
const UPDATE_WINDOW_TITLE: &str = "Update status";
const UNCERTAIN_SAVE_ABANDON_GUIDANCE: &str = "Cancel this dialog and reconcile every uncertain save outcome before attempting another save. Your current text remains editable.";
const MENU_BAR_HEIGHT: f32 = 30.0;
/// Horizontal gap between top-level menu names.
///
/// Menu names are separate targets, not one run of words, so they need visible
/// air between them at every supported width.
const MENU_ITEM_SPACING: f32 = 8.0;
/// Horizontal gap between the Mode and Theme controls in the top bar.
const TOP_CONTROL_SPACING: f32 = 4.0;
const EDITOR_TOOLBAR_HEIGHT: f32 = 40.0;
const STATUS_BAR_HEIGHT: f32 = 26.0;
const EXPANDED_TOP_CONTROLS_MIN_WIDTH: f32 = 600.0;
const EXPANDED_TOP_CONTROLS_WIDTH: f32 = 372.0;
const COMPACT_TOP_CONTROLS_WIDTH: f32 = 280.0;
const MARKDOWN_READING_TOP_PADDING: f32 = 16.0;
const MARKDOWN_READING_BOTTOM_PADDING: f32 = 48.0;
const INLINE_ZOOM_MIN_WIDTH: f32 = 180.0;
const INTERACTIVE_TEXT_MAX_BYTES: usize = 8 << 20;
const INTERACTIVE_TEXT_MAX_LABEL: &str = "8 MiB";
// Focus regain always inspects. While focused, re-check at a bounded interval
// so concurrent writers surface without thrashing large-file fingerprint work.
const EXTERNAL_INSPECT_INTERVAL_SECS: f64 = 15.0;
const EXTERNAL_CHANGE_SAVE_BLOCK_MESSAGE: &str = "Ordinary Save is paused while an external file change needs a decision. Choose Reload Disk Version, Keep Editing, or Save As first.";
const MAX_SAVE_RECOVERY_RECORDS: usize = 16;
const MAX_SAVE_RECOVERY_MESSAGE_BYTES: usize = 4 << 10;
const MAX_SAVE_RECOVERY_DESTINATION_BYTES: usize = 128 << 10;
const MAX_SAVE_RECOVERY_LABEL_BYTES: usize = 1 << 10;
const SAVE_RECOVERY_BLOCK_MESSAGE: &str = "Another save cannot start while an uncertain save outcome remains. Inspect the destination and retained recovery artifact, preserve the version you need, and explicitly reconcile the listed outcome first.";
const SAVE_RECOVERY_RESERVATION_FAILURE_MESSAGE: &str = "Save stopped before writing because Noter could not safely retain the recovery evidence required if the commit outcome became uncertain. Preserve and reconcile any listed recovery artifacts before retrying.";
const SAVE_RECOVERY_PATH_LIMIT_MESSAGE: &str = "Save stopped before writing because the selected destination path is too large to retain safely if the commit outcome becomes uncertain.";
const SAVE_RECOVERY_TRUNCATION_SUFFIX: &str = "... Recovery detail was shortened to bound memory. Do not save again. Inspect the destination and every retained `.noter-save-*.tmp` sibling before explicit reconciliation.";
const TEXT_INPUT_LIMIT_PREFIX: &str =
    "Input was limited to keep this document within its supported";
const MARKDOWN_INPUT_LIMIT_MESSAGE: &str = "Markdown Mode limited this input to keep the source within its 1 MiB safety budget. Text within the remaining budget was preserved.";

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum DocumentView {
    #[default]
    Text,
    Markdown,
}

impl DocumentView {
    const fn label(self) -> &'static str {
        match self {
            Self::Text => "Text",
            Self::Markdown => "Markdown",
        }
    }
}

/// One width for every Mode button, sized for the longest label.
///
/// Two segments of the same control must not change size with their text, or
/// the pair reads as a layout accident rather than one switch.
const DOCUMENT_VIEW_BUTTON_WIDTH: f32 = 96.0;

#[derive(Default, Debug)]
pub struct LaunchOptions {
    pub initial_path: Option<PathBuf>,
    pub theme: Option<AppTheme>,
    pub view: Option<DocumentView>,
    pub show_updates: bool,
    pub screenshot_path: Option<PathBuf>,
    pub screenshot_idle: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FileCommand {
    New,
    Open,
    Reload,
    Save,
    SaveAs,
    Quit,
}

impl FileCommand {
    const SHORTCUTS_IN_PRECEDENCE_ORDER: [(Self, egui::KeyboardShortcut); 4] = [
        (
            Self::SaveAs,
            egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                egui::Key::S,
            ),
        ),
        (
            Self::New,
            egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::N),
        ),
        (
            Self::Open,
            egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O),
        ),
        (
            Self::Save,
            egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S),
        ),
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::New => "New",
            Self::Open => "Open...",
            Self::Reload => "Reload from Disk",
            Self::Save => "Save",
            Self::SaveAs => "Save As...",
            Self::Quit => "Quit",
        }
    }

    fn shortcut(self) -> Option<egui::KeyboardShortcut> {
        Self::SHORTCUTS_IN_PRECEDENCE_ORDER
            .iter()
            .find_map(|(command, shortcut)| (*command == self).then_some(*shortcut))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditCommand {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Find,
    FindNext,
    FindPrevious,
    Replace,
    GoToLine,
}

impl EditCommand {
    const INPUT_PRECEDENCE: [Self; 11] = [
        Self::FindPrevious,
        Self::Redo,
        Self::Undo,
        Self::Cut,
        Self::Copy,
        Self::Paste,
        Self::SelectAll,
        Self::Find,
        Self::Replace,
        Self::GoToLine,
        Self::FindNext,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Undo => "Undo",
            Self::Redo => "Redo",
            Self::Cut => "Cut",
            Self::Copy => "Copy",
            Self::Paste => "Paste",
            Self::SelectAll => "Select All",
            Self::Find => "Find...",
            Self::FindNext => "Find Next",
            Self::FindPrevious => "Find Previous",
            Self::Replace => "Replace...",
            Self::GoToLine => "Go To Line...",
        }
    }

    const fn shortcut(self, operating_system: egui::os::OperatingSystem) -> egui::KeyboardShortcut {
        match (self, operating_system) {
            (Self::Redo, egui::os::OperatingSystem::Mac) => egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                egui::Key::Z,
            ),
            (Self::Redo, _) => egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y),
            (Self::Undo, _) => egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z),
            (Self::Cut, _) => egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::X),
            (Self::Copy, _) => egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::C),
            (Self::Paste, _) => egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::V),
            (Self::SelectAll, _) => {
                egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::A)
            }
            (Self::Find, _) => egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::F),
            (Self::FindNext, egui::os::OperatingSystem::Mac) => {
                egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::G)
            }
            (Self::FindNext, _) => {
                egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F3)
            }
            (Self::FindPrevious, egui::os::OperatingSystem::Mac) => egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                egui::Key::G,
            ),
            (Self::FindPrevious, _) => {
                egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, egui::Key::F3)
            }
            (Self::Replace, egui::os::OperatingSystem::Mac) => egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::ALT),
                egui::Key::F,
            ),
            (Self::Replace, _) => {
                egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::H)
            }
            (Self::GoToLine, _) => egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::G),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ViewCommand {
    ToggleWordWrap,
    ToggleDocumentView,
    ToggleFullscreen,
    ZoomIn,
    ZoomOut,
    ResetZoom,
}

fn zoom_command_from_wheel_delta(delta: egui::Vec2) -> Option<ViewCommand> {
    if !delta.y.is_finite() || delta.y == 0.0 {
        return None;
    }
    Some(if delta.y > 0.0 {
        ViewCommand::ZoomIn
    } else {
        ViewCommand::ZoomOut
    })
}

fn shortcut_matches_event(event: &egui::Event, shortcut: egui::KeyboardShortcut) -> bool {
    matches!(
        event,
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } if *key == shortcut.logical_key && modifiers.matches_logically(shortcut.modifiers)
    )
}

fn file_command_for_shortcut_event(event: &egui::Event) -> Option<FileCommand> {
    FileCommand::SHORTCUTS_IN_PRECEDENCE_ORDER
        .into_iter()
        .find_map(|(command, shortcut)| shortcut_matches_event(event, shortcut).then_some(command))
}

fn edit_command_for_shortcut_event(
    event: &egui::Event,
    operating_system: egui::os::OperatingSystem,
    document_shortcuts_enabled: bool,
    go_to_line_shortcut_enabled: bool,
) -> Option<EditCommand> {
    let alternate_redo = egui::KeyboardShortcut::new(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::Z,
    );
    if document_shortcuts_enabled && shortcut_matches_event(event, alternate_redo) {
        return Some(EditCommand::Redo);
    }
    EditCommand::INPUT_PRECEDENCE.into_iter().find(|candidate| {
        if !document_shortcuts_enabled
            && matches!(
                candidate,
                EditCommand::Undo
                    | EditCommand::Redo
                    | EditCommand::Cut
                    | EditCommand::Copy
                    | EditCommand::Paste
                    | EditCommand::SelectAll
            )
        {
            return false;
        }
        if !go_to_line_shortcut_enabled && *candidate == EditCommand::GoToLine {
            return false;
        }
        shortcut_matches_event(event, candidate.shortcut(operating_system))
    })
}

fn view_command_for_shortcut_event(
    event: &egui::Event,
    document_shortcuts_enabled: bool,
) -> Option<ViewCommand> {
    if document_shortcuts_enabled {
        for command in [ViewCommand::ToggleDocumentView, ViewCommand::ToggleWordWrap] {
            if shortcut_matches_event(event, command.shortcut()) {
                return Some(command);
            }
        }
    }
    for command in [
        ViewCommand::ToggleFullscreen,
        ViewCommand::ResetZoom,
        ViewCommand::ZoomIn,
        ViewCommand::ZoomOut,
    ] {
        if shortcut_matches_event(event, command.shortcut()) {
            return Some(command);
        }
    }
    shortcut_matches_event(event, egui::gui_zoom::kb_shortcuts::ZOOM_IN_SECONDARY)
        .then_some(ViewCommand::ZoomIn)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ViewCommandFocus {
    RestoreDocument,
    PreserveControl,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ViewCommandRequest {
    command: ViewCommand,
    focus: ViewCommandFocus,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum InputShortcut {
    File(FileCommand),
    Edit(EditCommand),
    View(ViewCommandRequest),
}

fn input_shortcut_for_event(
    event: &egui::Event,
    operating_system: egui::os::OperatingSystem,
    document_shortcuts_enabled: bool,
    go_to_line_shortcut_enabled: bool,
) -> Option<InputShortcut> {
    if let Some(command) = file_command_for_shortcut_event(event) {
        return Some(InputShortcut::File(command));
    }
    if let Some(command) = edit_command_for_shortcut_event(
        event,
        operating_system,
        document_shortcuts_enabled,
        go_to_line_shortcut_enabled,
    ) {
        return Some(InputShortcut::Edit(command));
    }
    view_command_for_shortcut_event(event, document_shortcuts_enabled)
        .map(ViewCommandRequest::preserve_control)
        .map(InputShortcut::View)
}

impl ViewCommandRequest {
    const fn restore_document(command: ViewCommand) -> Self {
        Self {
            command,
            focus: ViewCommandFocus::RestoreDocument,
        }
    }

    const fn preserve_control(command: ViewCommand) -> Self {
        Self {
            command,
            focus: ViewCommandFocus::PreserveControl,
        }
    }
}

impl ViewCommand {
    const fn label(self) -> &'static str {
        match self {
            Self::ToggleWordWrap => "Word Wrap",
            Self::ToggleDocumentView => "Switch Mode",
            Self::ToggleFullscreen => "Full Screen",
            Self::ZoomIn => "Zoom In",
            Self::ZoomOut => "Zoom Out",
            Self::ResetZoom => "Reset Zoom",
        }
    }

    const fn shortcut(self) -> egui::KeyboardShortcut {
        match self {
            Self::ToggleWordWrap => egui::KeyboardShortcut::new(egui::Modifiers::ALT, egui::Key::Z),
            Self::ToggleDocumentView => egui::KeyboardShortcut::new(
                egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                egui::Key::M,
            ),
            Self::ToggleFullscreen => {
                egui::KeyboardShortcut::new(egui::Modifiers::NONE, egui::Key::F11)
            }
            Self::ZoomIn => egui::gui_zoom::kb_shortcuts::ZOOM_IN,
            Self::ZoomOut => egui::gui_zoom::kb_shortcuts::ZOOM_OUT,
            Self::ResetZoom => egui::gui_zoom::kb_shortcuts::ZOOM_RESET,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct MarkdownIssueCache {
    document_serial: u64,
    revision: Revision,
    issue_count: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct StatusSnapshot {
    line: usize,
    column: usize,
    selected_characters: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct StatusCache {
    document_serial: u64,
    revision: Revision,
    selection: Selection,
    snapshot: StatusSnapshot,
}

#[derive(Clone, Copy, Debug)]
struct EditorFrameOutcome {
    changed: bool,
    selection: Selection,
    origin: EditOrigin,
    observed_at: EditTimestamp,
}

#[derive(Debug)]
struct TextImeComposition {
    draft: String,
    base_selection: Selection,
}

#[derive(Debug)]
enum PendingHardLinkSave {
    Current {
        target: PathBuf,
        link_count: u64,
    },
    SaveAs {
        prepared: PreparedSaveAs,
        target: PathBuf,
        link_count: u64,
    },
}

impl PendingHardLinkSave {
    const fn link_count(&self) -> u64 {
        match self {
            Self::Current { link_count, .. } | Self::SaveAs { link_count, .. } => *link_count,
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct SaveRecovery {
    destination: PathBuf,
    destination_label: String,
    message: String,
    notice_pending: bool,
}

#[derive(Debug)]
struct SaveRecoveryReservation {
    record_count: usize,
    attempt: SaveAttempt,
    destination_label: String,
    message: String,
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum SaveAttempt {
    Current(PathBuf),
    SaveAs(PathBuf),
}

impl SaveAttempt {
    fn destination(&self) -> &Path {
        match self {
            Self::Current(destination) | Self::SaveAs(destination) => destination,
        }
    }

    fn into_destination(self) -> PathBuf {
        match self {
            Self::Current(destination) | Self::SaveAs(destination) => destination,
        }
    }
}

/// Whether the local update status is showing, and what opened it.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum UpdateStatusState {
    #[default]
    Closed,
    /// Opened from Help > Check for Updates during an editing session.
    Open,
    /// Opened by `noter update`, which also names the window while it shows.
    OpenedByLaunch,
}

impl UpdateStatusState {
    const fn is_open(self) -> bool {
        !matches!(self, Self::Closed)
    }

    const fn names_the_window(self) -> bool {
        matches!(self, Self::OpenedByLaunch)
    }
}

pub struct NoterApp {
    text: String,
    text_ime_composition: Option<TextImeComposition>,
    document: Document,
    history: UndoHistory,
    selection: Selection,
    pending_selection_restore: Option<Selection>,
    preserve_focus_on_selection_restore: bool,
    pending_document_view: Option<DocumentView>,
    deferred_input_events: Vec<egui::Event>,
    view: DocumentView,
    theme: AppTheme,
    text_wrap: TextWrap,
    editor_zoom: EditorZoom,
    pointer_zoom: PointerZoomAccumulator,
    find_bar: FindBar,
    go_to_line: GoToLineDialog,
    idle_screen: IdleScreen,
    markdown_editor: MarkdownEditor,
    document_editor_serial: u64,
    markdown_issue_cache: Option<MarkdownIssueCache>,
    status_cache: Option<StatusCache>,
    error_msg: Option<String>,
    save_recoveries: Vec<SaveRecovery>,
    pending_recovery_reconciliation: Option<usize>,
    about_open: bool,
    updates: UpdateStatusState,
    /// The last title handed to the window manager, so repeats are not resent.
    sent_window_title: Option<String>,
    pending_hard_link_save: Option<PendingHardLinkSave>,
    lifecycle: LifecycleState,
    conflict: ConflictState,
    external_memory_at_risk: bool,
    last_external_inspect_at: Option<f64>,
    crash_recovery: CrashRecoverySession,
    #[cfg(test)]
    test_recovery_root: Option<tempfile::TempDir>,
    #[cfg(feature = "screenshot-qa")]
    screenshot: Option<ScreenshotCapture>,
}

impl Default for NoterApp {
    fn default() -> Self {
        #[cfg(test)]
        {
            let test_recovery_root =
                tempfile::tempdir().expect("app tests require an isolated recovery directory");
            let crash_recovery = CrashRecoverySession::open_at(test_recovery_root.path());
            let mut app = Self::with_crash_recovery(crash_recovery);
            app.test_recovery_root = Some(test_recovery_root);
            app
        }
        #[cfg(not(test))]
        {
            Self::with_crash_recovery(CrashRecoverySession::open_default())
        }
    }
}

impl NoterApp {
    fn with_crash_recovery(crash_recovery: CrashRecoverySession) -> Self {
        Self {
            text: String::new(),
            text_ime_composition: None,
            document: Document::new(),
            history: UndoHistory::default(),
            selection: Selection::caret(0),
            pending_selection_restore: None,
            preserve_focus_on_selection_restore: false,
            pending_document_view: None,
            deferred_input_events: Vec::new(),
            view: DocumentView::Text,
            theme: AppTheme::System,
            text_wrap: TextWrap::default(),
            editor_zoom: EditorZoom::default(),
            pointer_zoom: PointerZoomAccumulator::default(),
            find_bar: FindBar::default(),
            go_to_line: GoToLineDialog::default(),
            idle_screen: IdleScreen::default(),
            markdown_editor: MarkdownEditor::default(),
            document_editor_serial: 0,
            markdown_issue_cache: None,
            status_cache: None,
            error_msg: None,
            save_recoveries: Vec::new(),
            pending_recovery_reconciliation: None,
            about_open: false,
            updates: UpdateStatusState::Closed,
            sent_window_title: None,
            pending_hard_link_save: None,
            lifecycle: LifecycleState::default(),
            conflict: ConflictState::default(),
            external_memory_at_risk: false,
            last_external_inspect_at: None,
            crash_recovery,
            #[cfg(test)]
            test_recovery_root: None,
            #[cfg(feature = "screenshot-qa")]
            screenshot: None,
        }
    }

    fn interactive_text_maximum_for(document: &Document) -> usize {
        document
            .maximum_text_bytes()
            .min(INTERACTIVE_TEXT_MAX_BYTES)
    }

    fn interactive_text_maximum(&self) -> usize {
        Self::interactive_text_maximum_for(&self.document)
    }

    pub fn new(cc: &eframe::CreationContext<'_>, options: LaunchOptions) -> Self {
        theme::configure_styles(&cc.egui_ctx);
        cc.egui_ctx
            .options_mut(|options| options.zoom_with_keyboard = false);
        let selected_theme = options
            .theme
            .unwrap_or_else(|| AppTheme::from_storage(cc.storage));
        selected_theme.apply(&cc.egui_ctx);

        let mut app = Self::with_crash_recovery(CrashRecoverySession::open_default());
        app.theme = selected_theme;
        app.text_wrap = TextWrap::from_storage(cc.storage);
        app.editor_zoom = EditorZoom::from_storage(cc.storage);
        app.updates = if options.show_updates {
            UpdateStatusState::OpenedByLaunch
        } else {
            UpdateStatusState::Closed
        };
        if app.crash_recovery.is_unavailable() {
            app.error_msg = Some(RECOVERY_UNAVAILABLE_MESSAGE.to_owned());
        }
        // Explicit file opens and screenshot automation skip interactive recovery
        // offers so double-click open and capture remain deterministic. Records
        // stay on disk for a later untitled launch; Discard is a user choice.
        let skip_recovery_offers =
            options.initial_path.is_some() || options.screenshot_path.is_some();
        if skip_recovery_offers {
            app.crash_recovery.defer_startup_offers();
        }
        if let Some(path) = options.initial_path.clone() {
            app.open_path(&path, options.view);
        } else if let Some(view) = options.view {
            app.select_document_view(view);
        }
        if options.initial_path.is_none() && options.screenshot_path.is_none() {
            app.request_untitled_editor_focus();
        }

        #[cfg(feature = "screenshot-qa")]
        if let Some(path) = options.screenshot_path {
            if app.view == DocumentView::Markdown {
                app.markdown_editor.activate_first_block(&app.text);
            }
            if options.screenshot_idle {
                app.idle_screen.force_active_for_capture();
            }
            app.screenshot = Some(ScreenshotCapture::new(path));
        }
        #[cfg(not(feature = "screenshot-qa"))]
        if options.screenshot_path.is_some() {
            app.error_msg = Some(
                "Screenshot capture requires a build with the `screenshot-qa` feature.".to_owned(),
            );
        }

        app
    }

    fn open_path(&mut self, path: &std::path::Path, requested_view: Option<DocumentView>) {
        let document = match Self::prepare_open_path(path) {
            Ok(document) => document,
            Err(message) => {
                self.error_msg = Some(message);
                return;
            }
        };
        self.install_prepared_document(document);
        self.begin_fresh_recovery_identity();
        self.select_document_view(requested_view.unwrap_or_else(|| preferred_view_for_path(path)));
    }

    fn prepare_open_path(path: &std::path::Path) -> Result<Document, String> {
        let document =
            Document::from_path(path).map_err(|error| format!("Failed to open file: {error}"))?;
        let source_bytes = document.rope().len_bytes();
        let maximum = Self::interactive_text_maximum_for(&document);
        if source_bytes > maximum {
            return Err(format!(
                "This file contains {source_bytes} UTF-8 bytes. The current editor safely supports files up to {INTERACTIVE_TEXT_MAX_LABEL}, so the file was not opened. Larger-file editing requires the planned virtualized editor."
            ));
        }
        Ok(document)
    }

    fn install_prepared_document(&mut self, document: Document) {
        self.text = String::from(document.rope());
        self.document = document;
        self.history.reset(self.document.revision());
        self.selection = Selection::caret(0);
        self.pending_selection_restore = Some(self.selection);
        self.advance_document_editor();
        self.find_bar.reset();
        self.go_to_line.reset();
        self.markdown_editor.reset();
        self.markdown_issue_cache = None;
        self.error_msg = None;
        self.reset_external_conflict_state();
        self.view = DocumentView::Text;
    }

    fn do_open_unchecked(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            self.open_path(&path, None);
        }
    }

    fn reload_current_path_unchecked(&mut self) {
        let Some(path) = self.document.path().map(std::path::Path::to_path_buf) else {
            self.error_msg = Some(
                "Reload is unavailable because this document has not been saved yet.".to_owned(),
            );
            return;
        };
        let view = self.view;
        self.open_path(&path, Some(view));
    }

    fn discard_retained_clean_copy_and_reload(&mut self) {
        debug_assert!(!self.document.is_dirty());
        debug_assert!(self.external_memory_at_risk);
        let Some(path) = self.document.path().map(std::path::Path::to_path_buf) else {
            self.error_msg = Some(
                "Reload is unavailable because this document has not been saved yet.".to_owned(),
            );
            return;
        };
        let view = self.view;
        let document = match Self::prepare_open_path(&path) {
            Ok(document) => document,
            Err(message) => {
                self.error_msg = Some(message);
                return;
            }
        };

        // Preparation above has fully loaded and bounded the replacement. Only
        // now commit the user's explicit discard of the retained clean copy.
        self.crash_recovery.on_discarded();
        self.install_prepared_document(document);
        self.surface_recovery_unavailable();
        self.select_document_view(view);
    }

    fn request_open(&mut self, ctx: &egui::Context) {
        self.request_destructive_action(PendingAbandonAction::Open, ctx);
    }

    fn request_reload(&mut self, ctx: &egui::Context) {
        self.request_destructive_action(PendingAbandonAction::Reload, ctx);
    }

    fn do_save(&mut self) {
        self.do_save_with_retained_conflict(false);
    }

    fn do_save_for_pending_abandon(&mut self) {
        self.do_save_with_retained_conflict(self.pending_reload_can_attempt_durable_save());
    }

    fn do_save_with_retained_conflict(&mut self, allow_retained_conflict: bool) {
        if self.conflict.blocks_ordinary_save() && !allow_retained_conflict {
            self.error_msg = Some(EXTERNAL_CHANGE_SAVE_BLOCK_MESSAGE.to_owned());
            return;
        }
        if self.save_is_blocked() {
            self.show_active_save_recovery_messages();
            self.error_msg = Some(SAVE_RECOVERY_BLOCK_MESSAGE.to_owned());
            return;
        }
        let Some(path) = self.document.path().map(std::path::Path::to_path_buf) else {
            self.do_save_as();
            return;
        };
        let Some(reservation) = self.reserve_save_recovery_slot(SaveAttempt::Current(path.clone()))
        else {
            return;
        };
        let result = self.document.save();
        if let Err(NoterError::HardLinkedTarget(link_count)) = result {
            self.pending_hard_link_save = Some(PendingHardLinkSave::Current {
                target: path,
                link_count,
            });
            self.error_msg = None;
            return;
        }
        self.handle_save_result(result, reservation);
    }

    fn do_save_as(&mut self) {
        if self.save_is_blocked() {
            self.show_active_save_recovery_messages();
            self.error_msg = Some(SAVE_RECOVERY_BLOCK_MESSAGE.to_owned());
            return;
        }
        if let Some(path) = rfd::FileDialog::new().save_file() {
            self.do_save_as_to(path);
        }
    }

    fn do_save_as_to(&mut self, path: PathBuf) {
        if self.save_is_blocked() {
            self.show_active_save_recovery_messages();
            self.error_msg = Some(SAVE_RECOVERY_BLOCK_MESSAGE.to_owned());
            return;
        }
        let prepared = match self.document.prepare_save_as(&path) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.error_msg = Some(format!("Failed to save file: {error}"));
                return;
            }
        };
        if let Some(link_count) = prepared
            .hard_link_count()
            .filter(|link_count| *link_count > 1)
        {
            self.pending_hard_link_save = Some(PendingHardLinkSave::SaveAs {
                prepared,
                target: path,
                link_count,
            });
            self.error_msg = None;
            return;
        }
        let Some(reservation) = self.reserve_save_recovery_slot(SaveAttempt::SaveAs(path)) else {
            return;
        };
        let result = self.document.save_prepared_as(prepared);
        self.handle_save_result(result, reservation);
    }

    fn confirm_pending_hard_link_save(&mut self) {
        if self.save_is_blocked() {
            self.show_active_save_recovery_messages();
            self.error_msg = Some(SAVE_RECOVERY_BLOCK_MESSAGE.to_owned());
            return;
        }
        let Some(attempt) = self
            .pending_hard_link_save
            .as_ref()
            .map(|pending| match pending {
                PendingHardLinkSave::Current { target, .. } => SaveAttempt::Current(target.clone()),
                PendingHardLinkSave::SaveAs { target, .. } => SaveAttempt::SaveAs(target.clone()),
            })
        else {
            return;
        };
        let Some(reservation) = self.reserve_save_recovery_slot(attempt) else {
            return;
        };
        let Some(pending) = self.pending_hard_link_save.take() else {
            return;
        };
        let result = match pending {
            PendingHardLinkSave::Current { .. } => {
                self.document.save_confirming_hard_link_replacement()
            }
            PendingHardLinkSave::SaveAs { prepared, .. } => self
                .document
                .save_prepared_as_confirming_hard_link_replacement(prepared),
        };
        self.handle_save_result(result, reservation);
    }

    fn handle_save_result(
        &mut self,
        result: Result<SaveOutcome, NoterError>,
        reservation: SaveRecoveryReservation,
    ) {
        let SaveRecoveryReservation {
            record_count,
            attempt,
            destination_label,
            mut message,
        } = reservation;
        self.error_msg = match result {
            Ok(SaveOutcome::Committed { ref warnings, .. }) if warnings.is_empty() => {
                self.reset_external_conflict_state();
                self.crash_recovery.on_saved_clean(self.document.revision());
                None
            }
            Ok(SaveOutcome::Committed { warnings, .. }) => {
                self.reset_external_conflict_state();
                self.crash_recovery.on_saved_clean(self.document.revision());
                let mut details: Vec<String> =
                    warnings.cleanup().iter().map(ToString::to_string).collect();
                details.extend(warnings.durability().iter().map(ToString::to_string));
                Some(format!(
                    "Saved, but follow-up is required: {}",
                    details.join("; ")
                ))
            }
            Ok(SaveOutcome::Conflict { cleanup_error, .. }) => {
                let mut message =
                    "Save stopped because the destination changed. Your edits remain unsaved."
                        .to_owned();
                append_cleanup_error(&mut message, cleanup_error.as_ref());
                Some(message)
            }
            Ok(SaveOutcome::NotCommitted {
                error,
                cleanup_error,
                ..
            }) => {
                let mut message = format!("Save did not commit: {error}");
                append_cleanup_error(&mut message, cleanup_error.as_ref());
                Some(message)
            }
            Ok(SaveOutcome::CommitStateUnknown {
                error,
                recovery_artifact,
                ..
            }) => {
                message = write_save_recovery_message(message, &recovery_artifact, &error);
                debug_assert_eq!(self.save_recoveries.len(), record_count);
                self.save_recoveries.push(SaveRecovery {
                    destination: attempt.into_destination(),
                    destination_label,
                    message,
                    notice_pending: true,
                });
                None
            }
            Err(error) => Some(format!("Failed to save file: {error}")),
        };
    }

    const fn ordinary_save_is_blocked(&self) -> bool {
        self.save_is_blocked() || self.conflict.blocks_ordinary_save()
    }

    fn pending_reload_can_attempt_durable_save(&self) -> bool {
        self.lifecycle.pending_intent() == Some(PendingAbandonAction::Reload)
            && self.document.is_dirty()
            && self.conflict.is_prompting()
            && !self.conflict.is_confirming_overwrite()
    }

    fn pending_abandon_save_is_blocked(&self) -> bool {
        self.save_is_blocked()
            || (self.conflict.blocks_ordinary_save()
                && !self.pending_reload_can_attempt_durable_save())
    }

    const fn save_is_blocked(&self) -> bool {
        !self.save_recoveries.is_empty()
    }

    fn reset_external_conflict_state(&mut self) {
        let _ = self.conflict.reduce(ConflictCommand::Reset);
        self.external_memory_at_risk = false;
        self.last_external_inspect_at = None;
    }

    fn has_unsaved_state(&self) -> bool {
        self.document.is_dirty() || self.external_memory_at_risk
    }

    fn synchronize_crash_recovery(&mut self) {
        if self.has_unsaved_state() {
            self.crash_recovery
                .on_retained(&self.document, self.selection);
        } else {
            self.crash_recovery.on_saved_clean(self.document.revision());
        }
    }

    fn show_active_save_recovery_messages(&mut self) {
        for recovery in &mut self.save_recoveries {
            recovery.notice_pending = true;
        }
    }

    fn reserve_save_recovery_slot(
        &mut self,
        attempt: SaveAttempt,
    ) -> Option<SaveRecoveryReservation> {
        if self.save_recoveries.len() >= MAX_SAVE_RECOVERY_RECORDS {
            self.show_active_save_recovery_messages();
            self.error_msg = Some(SAVE_RECOVERY_RESERVATION_FAILURE_MESSAGE.to_owned());
            return None;
        }
        if self.save_is_blocked() {
            self.show_active_save_recovery_messages();
            self.error_msg = Some(SAVE_RECOVERY_BLOCK_MESSAGE.to_owned());
            return None;
        }
        if attempt.destination().as_os_str().as_encoded_bytes().len()
            > MAX_SAVE_RECOVERY_DESTINATION_BYTES
        {
            self.error_msg = Some(SAVE_RECOVERY_PATH_LIMIT_MESSAGE.to_owned());
            return None;
        }
        if self.save_recoveries.try_reserve(1).is_err() {
            self.show_active_save_recovery_messages();
            self.error_msg = Some(SAVE_RECOVERY_RESERVATION_FAILURE_MESSAGE.to_owned());
            return None;
        }
        let mut message = String::new();
        if message
            .try_reserve_exact(MAX_SAVE_RECOVERY_MESSAGE_BYTES)
            .is_err()
        {
            self.show_active_save_recovery_messages();
            self.error_msg = Some(SAVE_RECOVERY_RESERVATION_FAILURE_MESSAGE.to_owned());
            return None;
        }
        let Some(destination_label) = bounded_destination_label(attempt.destination()) else {
            self.error_msg = Some(SAVE_RECOVERY_RESERVATION_FAILURE_MESSAGE.to_owned());
            return None;
        };
        Some(SaveRecoveryReservation {
            record_count: self.save_recoveries.len(),
            attempt,
            destination_label,
            message,
        })
    }

    fn request_untitled_editor_focus(&mut self) {
        if self.crash_recovery.active_offer().is_none() {
            self.pending_selection_restore = Some(self.selection);
        }
    }

    fn start_new_document_unchecked(&mut self) {
        self.text.clear();
        self.document = Document::new();
        self.history.reset(self.document.revision());
        self.selection = Selection::caret(0);
        self.pending_selection_restore = Some(self.selection);
        self.view = DocumentView::Text;
        self.advance_document_editor();
        self.find_bar.reset();
        self.go_to_line.reset();
        self.markdown_editor.reset();
        self.markdown_issue_cache = None;
        self.error_msg = None;
        self.reset_external_conflict_state();
        // Clean New skips the discard prompt, so rotate recovery identity here
        // the same way Open and dirty-Discard do. Otherwise later dirty
        // snapshots would reuse the previous session's instance file.
        self.begin_fresh_recovery_identity();
    }

    fn request_new_document(&mut self, ctx: &egui::Context) {
        self.request_destructive_action(PendingAbandonAction::New, ctx);
    }

    fn request_close(&mut self, ctx: &egui::Context) {
        let revision = self.document.revision();
        let has_unsaved_state = self.has_unsaved_state();
        let dirty_close_is_blocked =
            has_unsaved_state && !self.lifecycle.close_authorized(revision);
        let effect = self.lifecycle.reduce(LifecycleCommand::Request {
            intent: PendingAbandonAction::Quit,
            document_dirty: has_unsaved_state,
            revision,
        });
        if dirty_close_is_blocked {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        self.apply_lifecycle_effect(effect, ctx);
    }

    fn protect_native_close(&mut self, ctx: &egui::Context) {
        let revision = self.document.revision();
        if ctx.input(|input| input.viewport().close_requested())
            && self.has_unsaved_state()
            && !self.lifecycle.close_authorized(revision)
        {
            let effect = self.lifecycle.reduce(LifecycleCommand::Request {
                intent: PendingAbandonAction::Quit,
                document_dirty: true,
                revision,
            });
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.apply_lifecycle_effect(effect, ctx);
        }
    }

    fn request_destructive_action(&mut self, intent: PendingAbandonAction, ctx: &egui::Context) {
        let effect = self.lifecycle.reduce(LifecycleCommand::Request {
            intent,
            document_dirty: self.has_unsaved_state(),
            revision: self.document.revision(),
        });
        self.apply_lifecycle_effect(effect, ctx);
    }

    fn cancel_pending_abandon(&mut self) {
        let _ = self
            .lifecycle
            .reduce(LifecycleCommand::Decide(DirtyDecision::Cancel));
        self.show_active_save_recovery_messages();
    }

    fn discard_pending_abandon(&mut self, ctx: &egui::Context) {
        let effect = self
            .lifecycle
            .reduce(LifecycleCommand::Decide(DirtyDecision::Discard));
        self.crash_recovery.on_discarded();
        self.surface_recovery_unavailable();
        match effect {
            LifecycleEffect::Continue(action) => {
                self.execute_abandon_action(action, ctx);
                self.rearm_recovery_after_incomplete_abandon(action, ctx);
            }
            other => self.apply_lifecycle_effect(other, ctx),
        }
    }

    fn rearm_recovery_after_incomplete_abandon(
        &mut self,
        action: PendingAbandonAction,
        ctx: &egui::Context,
    ) {
        if !matches!(
            action,
            PendingAbandonAction::Open | PendingAbandonAction::Reload
        ) || !self.has_unsaved_state()
        {
            return;
        }
        self.synchronize_crash_recovery();
        if let Some(delay) = self.crash_recovery.next_persist_delay() {
            ctx.request_repaint_after(delay);
        }
        self.surface_recovery_unavailable();
    }

    fn begin_fresh_recovery_identity(&mut self) {
        self.crash_recovery.begin_fresh_identity();
        self.surface_recovery_unavailable();
    }

    fn surface_recovery_unavailable(&mut self) {
        if self.crash_recovery.is_unavailable() && self.error_msg.is_none() {
            self.error_msg = Some(RECOVERY_UNAVAILABLE_MESSAGE.to_owned());
        }
    }

    fn save_pending_abandon(&mut self, ctx: &egui::Context) {
        let effect = self
            .lifecycle
            .reduce(LifecycleCommand::Decide(DirtyDecision::Save));
        self.apply_lifecycle_effect(effect, ctx);
    }

    fn continue_pending_abandon_if_clean(&mut self, ctx: &egui::Context) {
        let effect = self
            .lifecycle
            .reduce(LifecycleCommand::SaveSettled(SaveContinuation::new(
                self.document.revision(),
                self.has_unsaved_state(),
                self.pending_hard_link_save.is_some(),
                self.error_msg.is_some(),
            )));
        self.apply_lifecycle_effect(effect, ctx);
    }

    fn apply_lifecycle_effect(&mut self, effect: LifecycleEffect, ctx: &egui::Context) {
        match effect {
            LifecycleEffect::None => {}
            LifecycleEffect::PromptDirty(_) => {
                self.show_active_save_recovery_messages();
            }
            LifecycleEffect::StartSave => {
                self.do_save_for_pending_abandon();
                self.continue_pending_abandon_if_clean(ctx);
            }
            LifecycleEffect::Continue(action) => self.execute_abandon_action(action, ctx),
        }
    }

    fn maybe_inspect_external_change(&mut self, ctx: &egui::Context) {
        if self.lifecycle.pending_intent().is_some()
            || self.pending_hard_link_save.is_some()
            || self.pending_recovery_reconciliation.is_some()
            || self.crash_recovery.active_offer().is_some()
            || self.about_open
            || self.updates.is_open()
        {
            return;
        }
        let Some(path) = self.document.path().map(Path::to_path_buf) else {
            return;
        };
        let Some(expected) = self.document.saved_target() else {
            return;
        };

        let (now, focus_regained, window_focused) = ctx.input(|input| {
            (
                input.time,
                input
                    .events
                    .iter()
                    .any(|event| matches!(event, egui::Event::WindowFocused(true))),
                // Unknown focus is treated as unfocused so background or
                // minimized sessions do not schedule periodic disk inspection.
                input.viewport().focused.unwrap_or(false),
            )
        });
        let interval_elapsed = self
            .last_external_inspect_at
            .is_none_or(|last| now - last >= EXTERNAL_INSPECT_INTERVAL_SECS);
        if !(focus_regained || (window_focused && interval_elapsed)) {
            if window_focused {
                ctx.request_repaint_after(Duration::from_secs_f64(EXTERNAL_INSPECT_INTERVAL_SECS));
            }
            return;
        }

        self.last_external_inspect_at = Some(now);
        if window_focused {
            ctx.request_repaint_after(Duration::from_secs_f64(EXTERNAL_INSPECT_INTERVAL_SECS));
        }
        let observed = inspect_target(&path, SaveStage::InspectInitial).map_err(|_| ());
        let kind = classify_external_change(Some(expected), Some(observed));
        let _ = self.conflict.reduce(ConflictCommand::ObservedExact {
            kind,
            evidence: observed,
            revision: self.document.revision(),
        });
        if kind.requires_prompt() {
            if !self.external_memory_at_risk {
                self.external_memory_at_risk = true;
                self.revoke_stale_close_authorization(ctx);
                self.synchronize_crash_recovery();
                if let Some(delay) = self.crash_recovery.next_persist_delay() {
                    ctx.request_repaint_after(delay);
                }
                self.surface_recovery_unavailable();
            }
        } else if self.external_memory_at_risk {
            self.external_memory_at_risk = false;
            self.synchronize_crash_recovery();
        }
    }

    fn revoke_stale_close_authorization(&mut self, ctx: &egui::Context) {
        let revision = self.document.revision();
        if !self.lifecycle.close_authorized(revision) {
            return;
        }
        let effect = self.lifecycle.reduce(LifecycleCommand::Request {
            intent: PendingAbandonAction::Quit,
            document_dirty: true,
            revision,
        });
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.apply_lifecycle_effect(effect, ctx);
    }

    fn apply_conflict_effect(&mut self, effect: ConflictEffect, ctx: &egui::Context) {
        match effect {
            ConflictEffect::None
            | ConflictEffect::Prompt(_)
            | ConflictEffect::PromptOverwriteConfirm(_) => {}
            ConflictEffect::RequestReload if self.document.is_dirty() => self.request_reload(ctx),
            ConflictEffect::RequestReload => self.discard_retained_clean_copy_and_reload(),
            ConflictEffect::RequestSaveAs => self.do_save_as(),
            ConflictEffect::AuthorizeOverwrite => self.perform_authorized_overwrite(),
        }
    }

    fn perform_authorized_overwrite(&mut self) {
        let Some(path) = self.document.path().map(Path::to_path_buf) else {
            self.error_msg = Some(
                "Overwrite is unavailable because this document has not been saved yet.".to_owned(),
            );
            return;
        };
        match inspect_target(&path, SaveStage::InspectInitial) {
            Ok(noter::core::save::TargetState::Regular(observation)) => {
                self.document.rebaseline_to_observed_disk(observation);
                self.do_save();
            }
            Ok(noter::core::save::TargetState::Missing) => {
                self.error_msg = Some(
                    "Overwrite stopped because the file is now missing. Use Save As to keep your work.".to_owned(),
                );
            }
            Ok(noter::core::save::TargetState::Special(_)) => {
                self.error_msg = Some(
                    "Overwrite stopped because the path is no longer an ordinary file. Use Save As.".to_owned(),
                );
            }
            Err(error) => {
                self.error_msg = Some(format!(
                    "Overwrite stopped because the path could not be inspected safely: {error}"
                ));
            }
        }
    }

    fn show_external_change_confirmation(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.conflict.prompt_kind() else {
            return;
        };
        if self.conflict.is_confirming_overwrite() {
            self.show_overwrite_second_confirmation(ctx, kind);
            return;
        }
        let mut reload = false;
        let mut keep = false;
        let mut save_as = false;
        let mut overwrite = false;
        let recovery_blocked = self.save_is_blocked();
        let allow_overwrite = matches!(
            kind,
            noter::core::conflict::ExternalChangeKind::ContentOrIdentityChanged
        ) && !recovery_blocked
            && self.document.path().is_some();

        let response =
            egui::Modal::new(egui::Id::new("external-change-confirmation")).show(ctx, |ui| {
                ui.set_min_width(440.0);
                ui.set_max_width(560.0);
                ui.heading("File changed on disk");
                ui.label(kind.description());
                ui.label(
                    "Noter has not overwritten the disk version. Choose how to continue with the current in-memory document.",
                );
                if recovery_blocked {
                    ui.separator();
                    ui.label(SAVE_RECOVERY_BLOCK_MESSAGE);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    reload = ui.button("Reload Disk Version").clicked();
                    keep = ui.button("Keep Editing").clicked();
                    save_as = ui
                        .add_enabled(!recovery_blocked, egui::Button::new("Save As…"))
                        .on_disabled_hover_text(
                            "Save As is blocked until the uncertain save state is reconciled",
                        )
                        .clicked();
                    overwrite = ui
                        .add_enabled(allow_overwrite, egui::Button::new("Overwrite Disk Version…"))
                        .on_disabled_hover_text(
                            "Overwrite is available only when the path still points at a regular file and no uncertain save is open",
                        )
                        .clicked();
                });
            });

        if reload {
            let effect = self
                .conflict
                .reduce(ConflictCommand::Decide(ConflictDecision::ReloadDisk));
            self.apply_conflict_effect(effect, ctx);
        } else if keep || response.should_close() {
            let effect = self
                .conflict
                .reduce(ConflictCommand::Decide(ConflictDecision::KeepEditing));
            self.apply_conflict_effect(effect, ctx);
        } else if save_as {
            let effect = self
                .conflict
                .reduce(ConflictCommand::Decide(ConflictDecision::SaveAs));
            self.apply_conflict_effect(effect, ctx);
        } else if overwrite {
            let effect = self
                .conflict
                .reduce(ConflictCommand::Decide(ConflictDecision::RequestOverwrite));
            self.apply_conflict_effect(effect, ctx);
        }
    }

    fn show_overwrite_second_confirmation(
        &mut self,
        ctx: &egui::Context,
        kind: noter::core::conflict::ExternalChangeKind,
    ) {
        let mut confirm = false;
        let mut cancel = false;
        let response =
            egui::Modal::new(egui::Id::new("external-change-overwrite-confirm")).show(ctx, |ui| {
                ui.set_min_width(440.0);
                ui.set_max_width(560.0);
                ui.heading("Replace the disk version?");
                ui.label(kind.description());
                ui.label(
                    "This replaces the file on disk with the text currently in this window. The disk version will not be recoverable from Noter after this save.",
                );
                ui.separator();
                ui.horizontal(|ui| {
                    confirm = ui
                        .add(egui::Button::new("Replace Disk Version").min_size(egui::vec2(160.0, 28.0)))
                        .clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });
        if confirm {
            let effect = self
                .conflict
                .reduce(ConflictCommand::Decide(ConflictDecision::ConfirmOverwrite));
            self.apply_conflict_effect(effect, ctx);
        } else if cancel || response.should_close() {
            let effect = self
                .conflict
                .reduce(ConflictCommand::Decide(ConflictDecision::CancelOverwrite));
            self.apply_conflict_effect(effect, ctx);
        }
    }

    fn execute_abandon_action(&mut self, action: PendingAbandonAction, ctx: &egui::Context) {
        match action {
            PendingAbandonAction::New => self.start_new_document_unchecked(),
            PendingAbandonAction::Open => self.do_open_unchecked(),
            PendingAbandonAction::Reload => self.reload_current_path_unchecked(),
            PendingAbandonAction::Quit => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn restore_deferred_input(&mut self, ui: &egui::Ui) {
        if self.deferred_input_events.is_empty() {
            return;
        }
        ui.input_mut(|input| {
            self.deferred_input_events.append(&mut input.events);
            std::mem::swap(&mut self.deferred_input_events, &mut input.events);
        });
    }

    fn defer_input_events(&mut self, mut deferred: Vec<egui::Event>) {
        deferred.append(&mut self.deferred_input_events);
        self.deferred_input_events = deferred;
    }

    fn blocking_modal_open(&self) -> bool {
        self.crash_recovery.active_offer().is_some()
            || self.pending_recovery_reconciliation.is_some()
            || self.pending_hard_link_save.is_some()
            || self.lifecycle.pending_intent().is_some()
            || self.conflict.is_prompting()
    }

    fn discard_deferred_input(&mut self) {
        self.deferred_input_events.clear();
        self.find_bar.discard_deferred_input();
        self.markdown_editor.discard_deferred_input();
    }

    fn serialize_next_text_navigation(&mut self, ui: &egui::Ui) {
        let mut deferred = Vec::new();
        let platform = KeyboardPlatform::from_egui(ui.ctx().os());
        ui.input_mut(|input| {
            let Some(position) = input.events.iter().position(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } if resolve_navigation_gesture(*key, *modifiers, platform).is_some()
                )
            }) else {
                return;
            };
            if input.events[..position]
                .iter()
                .any(editor_event_orders_input)
            {
                deferred = input.events.split_off(position);
            } else if position + 1 < input.events.len() {
                deferred = input.events.split_off(position + 1);
            }
        });
        if !deferred.is_empty() {
            self.defer_input_events(deferred);
            ui.ctx().request_repaint();
        }
    }

    fn collect_input_shortcut(
        &mut self,
        ui: &egui::Ui,
        document_shortcuts_enabled: bool,
        go_to_line_shortcut_enabled: bool,
    ) -> Option<InputShortcut> {
        let mut deferred = Vec::new();
        let operating_system = ui.ctx().os();
        let command = ui.input_mut(|input| {
            let (position, command) =
                input
                    .events
                    .iter()
                    .enumerate()
                    .find_map(|(position, event)| {
                        input_shortcut_for_event(
                            event,
                            operating_system,
                            document_shortcuts_enabled,
                            go_to_line_shortcut_enabled,
                        )
                        .map(|command| (position, command))
                    })?;
            if input.events[..position]
                .iter()
                .any(editor_event_orders_input)
            {
                deferred = input.events.split_off(position);
                return None;
            }
            input.events.remove(position);
            deferred = input.events.split_off(position);
            Some(command)
        });
        if !deferred.is_empty() {
            self.defer_input_events(deferred);
            ui.ctx().request_repaint();
        }
        command
    }

    fn execute_input_shortcut(
        &mut self,
        shortcut: Option<InputShortcut>,
        commands_enabled: bool,
        context: &egui::Context,
    ) -> bool {
        if !commands_enabled {
            return false;
        }
        match shortcut {
            Some(InputShortcut::File(command)) => self.execute_file_command(command, context),
            Some(InputShortcut::Edit(command)) => {
                self.execute_edit_command(command, context);
                return true;
            }
            Some(InputShortcut::View(command)) => {
                self.execute_view_command(command, context);
                self.apply_pending_document_view();
            }
            None => {}
        }
        false
    }

    fn execute_file_command(&mut self, command: FileCommand, ctx: &egui::Context) {
        match command {
            FileCommand::New => self.request_new_document(ctx),
            FileCommand::Open => self.request_open(ctx),
            FileCommand::Reload => self.request_reload(ctx),
            FileCommand::Save => self.do_save(),
            FileCommand::SaveAs => self.do_save_as(),
            FileCommand::Quit => self.request_close(ctx),
        }
    }

    fn execute_edit_command(&mut self, command: EditCommand, ctx: &egui::Context) {
        match command {
            EditCommand::Find => {
                self.find_bar.open(false, &self.text, self.selection);
                return;
            }
            EditCommand::FindNext => {
                if !self.find_bar.has_query() {
                    self.find_bar.open(false, &self.text, self.selection);
                    return;
                }
                self.execute_find_navigation(SearchDirection::Next);
                return;
            }
            EditCommand::FindPrevious => {
                if !self.find_bar.has_query() {
                    self.find_bar.open(false, &self.text, self.selection);
                    return;
                }
                self.execute_find_navigation(SearchDirection::Previous);
                return;
            }
            EditCommand::Replace => {
                self.find_bar.open(true, &self.text, self.selection);
                return;
            }
            EditCommand::GoToLine => {
                let (current_line, _) = caret_line_column(&self.text, self.selection.active());
                self.go_to_line.open(current_line);
                return;
            }
            EditCommand::SelectAll => {
                self.selection = Selection::new(0, self.text.len());
                self.pending_selection_restore = Some(self.selection);
                self.preserve_focus_on_selection_restore = false;
                return;
            }
            EditCommand::Copy => {
                self.clipboard_copy(ctx);
                return;
            }
            EditCommand::Cut => {
                self.clipboard_cut(ctx);
                return;
            }
            EditCommand::Paste => {
                self.clipboard_paste(ctx);
                return;
            }
            EditCommand::Undo | EditCommand::Redo => {}
        }
        let result = match command {
            EditCommand::Undo => self.history.undo(&mut self.document),
            EditCommand::Redo => self.history.redo(&mut self.document),
            EditCommand::Find
            | EditCommand::FindNext
            | EditCommand::FindPrevious
            | EditCommand::Replace
            | EditCommand::GoToLine
            | EditCommand::SelectAll
            | EditCommand::Cut
            | EditCommand::Copy
            | EditCommand::Paste => return,
        };
        match result {
            Ok(Some(outcome)) => self.synchronize_after_history(outcome),
            Ok(None) => {}
            Err(error) => {
                self.restore_editor_after_failed_change(&error);
                self.history.reset(self.document.revision());
            }
        }
    }

    fn execute_view_command(&mut self, request: ViewCommandRequest, ctx: &egui::Context) {
        let ViewCommandRequest { command, focus } = request;
        match command {
            ViewCommand::ToggleWordWrap
                if self.pending_document_view.unwrap_or(self.view) == DocumentView::Text =>
            {
                self.text_wrap.toggle();
            }
            ViewCommand::ToggleWordWrap => return,
            ViewCommand::ToggleDocumentView => {
                let effective_view = self.pending_document_view.unwrap_or(self.view);
                let next = match effective_view {
                    DocumentView::Text => DocumentView::Markdown,
                    DocumentView::Markdown => DocumentView::Text,
                };
                self.request_document_view(next);
                return;
            }
            ViewCommand::ToggleFullscreen => {
                let is_fullscreen = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!is_fullscreen));
            }
            ViewCommand::ZoomIn => {
                let _ = self.editor_zoom.zoom_in();
            }
            ViewCommand::ZoomOut => {
                let _ = self.editor_zoom.zoom_out();
            }
            ViewCommand::ResetZoom => {
                let _ = self.editor_zoom.reset();
            }
        }
        if focus == ViewCommandFocus::RestoreDocument {
            self.pending_selection_restore = Some(self.selection);
            self.preserve_focus_on_selection_restore = false;
        }
    }

    fn synchronize_after_history(&mut self, outcome: HistoryApplyOutcome) {
        debug_assert_eq!(outcome.revision(), self.document.revision());
        self.text = String::from(self.document.rope());
        self.selection = outcome.selection();
        self.synchronize_crash_recovery();
        self.markdown_editor.reset();
        self.pending_selection_restore = Some(self.selection);
        self.markdown_issue_cache = None;
        if self.view == DocumentView::Markdown
            && let Some(limit) = markdown_projection_limit(&self.text)
        {
            self.view = DocumentView::Text;
            self.error_msg = Some(markdown_limit_message(self.text.len(), limit));
        }
    }

    fn execute_find_navigation(&mut self, direction: SearchDirection) {
        let search = match self
            .find_bar
            .prepared_search(self.document.revision(), &self.text)
        {
            Ok(search) => search,
            Err(error) => {
                self.error_msg = Some(format!(
                    "Find could not run: {error}. The document was not changed."
                ));
                return;
            }
        };
        let selected = self.selection.ordered_range();
        let position = match (direction, search.matches_range(&self.text, selected)) {
            (SearchDirection::Next, true) => selected.end(),
            (SearchDirection::Previous, true) => selected.start(),
            (SearchDirection::Next | SearchDirection::Previous, false) => self.selection.active(),
        };
        let Some(navigation) = search.navigate(&self.text, position, direction) else {
            return;
        };
        let range = navigation.range();
        self.selection = Selection::new(range.start(), range.end());
        self.pending_selection_restore = Some(self.selection);
        self.preserve_focus_on_selection_restore = true;
        self.find_bar.record_navigation(navigation);
    }

    fn execute_find_bar_action(&mut self, action: FindBarAction, observed_at: EditTimestamp) {
        match action {
            FindBarAction::Close => {
                self.pending_selection_restore = Some(self.selection);
                self.preserve_focus_on_selection_restore = false;
            }
            FindBarAction::Next => self.execute_find_navigation(SearchDirection::Next),
            FindBarAction::Previous => self.execute_find_navigation(SearchDirection::Previous),
            FindBarAction::Replace => self.replace_selected_match(observed_at),
            FindBarAction::ReplaceAll => self.replace_all_matches(observed_at),
        }
    }

    fn replace_selected_match(&mut self, observed_at: EditTimestamp) {
        let search = match self
            .find_bar
            .prepared_search(self.document.revision(), &self.text)
        {
            Ok(search) => search,
            Err(error) => {
                self.error_msg = Some(format!(
                    "Replace could not run: {error}. The document was not changed."
                ));
                return;
            }
        };
        let range = self.selection.ordered_range();
        if !search.matches_range(&self.text, range) {
            return;
        }
        let replacement = self.find_bar.replacement().to_owned();
        if self.text.get(range.start()..range.end()) == Some(replacement.as_str()) {
            self.find_bar
                .record_replacements(0, self.document.revision());
            return;
        }
        let maximum = self.interactive_text_maximum();
        let Some(projected) =
            projected_replacement_length(self.text.len(), range, replacement.len())
        else {
            self.error_msg = Some(
                "Replace could not calculate a bounded result. The document was not changed."
                    .to_owned(),
            );
            return;
        };
        if projected > maximum {
            self.error_msg = Some(format!(
                "Replace would create {projected} bytes; the maximum is {maximum} bytes. The document was not changed."
            ));
            return;
        }
        let Some(selection_end) = range.start().checked_add(replacement.len()) else {
            self.error_msg = Some(
                "Replace could not calculate a valid selection. The document was not changed."
                    .to_owned(),
            );
            return;
        };
        self.text
            .replace_range(range.start()..range.end(), &replacement);
        let revision_before = self.document.revision();
        self.record_editor_change(EditorFrameOutcome {
            changed: true,
            selection: Selection::caret(selection_end),
            origin: EditOrigin::Replace,
            observed_at,
        });
        if self.document.revision() != revision_before {
            self.synchronize_after_external_edit();
            self.find_bar
                .record_replacements(1, self.document.revision());
        }
    }

    fn replace_all_matches(&mut self, observed_at: EditTimestamp) {
        let search = match self
            .find_bar
            .prepared_search(self.document.revision(), &self.text)
        {
            Ok(search) => search,
            Err(error) => {
                self.error_msg = Some(format!(
                    "Replace All could not run: {error}. The document was not changed."
                ));
                return;
            }
        };
        let replace_scope = self.find_bar.replace_scope();
        let scope = match replace_scope {
            ReplaceScope::Selection => self.selection.ordered_range(),
            ReplaceScope::Document => TextRange::new(0, self.text.len()),
        };
        if scope.start() == scope.end() {
            return;
        }
        let replacement = self.find_bar.replacement().to_owned();
        let replacement_result = match search.replace_all(
            &self.text,
            scope,
            &replacement,
            self.interactive_text_maximum(),
        ) {
            Ok(Some(result)) => result,
            Ok(None) => return,
            Err(error) => {
                self.error_msg = Some(format!(
                    "Replace All could not run: {error}. The document was not changed."
                ));
                return;
            }
        };
        let replacement_count = replacement_result.replacement_count();
        let replacement_text = replacement_result.into_text();
        if self.text.get(scope.start()..scope.end()) == Some(replacement_text.as_str()) {
            self.find_bar
                .record_replacements(0, self.document.revision());
            return;
        }
        let Some(scope_end) = scope.start().checked_add(replacement_text.len()) else {
            self.error_msg = Some(
                "Replace All could not calculate a valid selection. The document was not changed."
                    .to_owned(),
            );
            return;
        };
        let selection_was_forward = self.selection.anchor() <= self.selection.active();
        let preserved_active = self.selection.active();
        self.text
            .replace_range(scope.start()..scope.end(), &replacement_text);
        let selection_after = match replace_scope {
            ReplaceScope::Selection if selection_was_forward => {
                Selection::new(scope.start(), scope_end)
            }
            ReplaceScope::Selection => Selection::new(scope_end, scope.start()),
            ReplaceScope::Document => {
                Selection::caret(byte_at_or_before(&self.text, preserved_active))
            }
        };
        let revision_before = self.document.revision();
        self.record_editor_change(EditorFrameOutcome {
            changed: true,
            selection: selection_after,
            origin: EditOrigin::Replace,
            observed_at,
        });
        if self.document.revision() != revision_before {
            self.synchronize_after_external_edit();
            self.find_bar
                .record_replacements(replacement_count, self.document.revision());
        }
    }

    fn synchronize_after_external_edit(&mut self) {
        self.markdown_editor.reset();
        self.pending_selection_restore = Some(self.selection);
        self.preserve_focus_on_selection_restore = true;
    }

    /// Returns the window title for the current session and document state.
    ///
    /// A session started by `noter update` names the update status it opened
    /// with, so an outside observer can tell that window apart from a blank
    /// editor. Dismissing the status returns the window to document titles.
    fn window_title(&self) -> String {
        if self.updates.names_the_window() {
            return format!("{UPDATE_WINDOW_TITLE} - Noter");
        }
        let dirty = if self.has_unsaved_state() { "*" } else { "" };
        self.document.path().map_or_else(
            || format!("Untitled{dirty} - Noter"),
            |path| {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                format!("{file_name}{dirty} - Noter")
            },
        )
    }

    /// Sends the window title only when it actually changes.
    ///
    /// Every viewport command requests a repaint, so sending an unchanged title
    /// each frame would hold the event loop awake and burn a core while the
    /// window sits idle.
    fn update_title(&mut self, ctx: &egui::Context) {
        let title = self.window_title();
        if self.sent_window_title.as_deref() == Some(title.as_str()) {
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
        self.sent_window_title = Some(title);
    }

    fn show_menu(
        &mut self,
        ui: &mut egui::Ui,
        file_command: &mut Option<FileCommand>,
        edit_command: &mut Option<EditCommand>,
        view_command: &mut Option<ViewCommandRequest>,
    ) {
        egui::Panel::top("menu_bar")
            .exact_size(MENU_BAR_HEIGHT)
            .show(ui, |ui| {
                let available = ui.available_rect_before_wrap();
                let expanded = available.width() >= EXPANDED_TOP_CONTROLS_MIN_WIDTH;
                let controls_width = if expanded {
                    EXPANDED_TOP_CONTROLS_WIDTH
                } else {
                    COMPACT_TOP_CONTROLS_WIDTH
                }
                .min(available.width());
                let split_x = available.max.x - controls_width;
                let menu_rect =
                    egui::Rect::from_min_max(available.min, egui::pos2(split_x, available.max.y));
                let controls_rect =
                    egui::Rect::from_min_max(egui::pos2(split_x, available.min.y), available.max);

                let mut menu_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(menu_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                menu_ui.spacing_mut().item_spacing.x = MENU_ITEM_SPACING;
                egui::MenuBar::new().ui(&mut menu_ui, |ui| {
                    ui.menu_button("File", |ui| self.show_file_menu(ui, file_command));
                    if expanded {
                        ui.menu_button("Edit", |ui| self.show_edit_menu(ui, edit_command));
                        ui.menu_button("View", |ui| {
                            self.show_view_menu(ui, view_command);
                        });
                        ui.menu_button("Help", |ui| self.show_help_menu(ui));
                    } else {
                        ui.menu_button("More", |ui| {
                            ui.menu_button("Edit", |ui| self.show_edit_menu(ui, edit_command));
                            ui.menu_button("View", |ui| {
                                self.show_view_preferences(ui, view_command);
                            });
                            ui.menu_button("Help", |ui| self.show_help_menu(ui));
                        });
                    }
                });

                let mut controls_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(controls_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                self.show_top_controls(&mut controls_ui, expanded);
            });
    }

    fn show_top_controls(&mut self, ui: &mut egui::Ui, expanded: bool) {
        ui.spacing_mut().item_spacing.x = TOP_CONTROL_SPACING;
        if !expanded {
            self.show_document_mode_menu_button(ui);
            let theme_label = format!("Theme: {}", self.theme.compact_label());
            self.show_theme_menu_button(ui, &theme_label);
            return;
        }

        ui.label(
            egui::RichText::new("Mode")
                .text_style(egui::TextStyle::Button)
                .weak(),
        );
        for view in [DocumentView::Text, DocumentView::Markdown] {
            let is_selected = self.view == view;
            let response = ui.add(
                egui::Button::selectable(is_selected, view.label())
                    .min_size(egui::vec2(DOCUMENT_VIEW_BUTTON_WIDTH, 28.0)),
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::RadioButton,
                    true,
                    is_selected,
                    format!("{} Mode", view.label()),
                )
            });
            if response.clicked() {
                self.request_document_view(view);
            }
        }
        ui.separator();
        ui.label(
            egui::RichText::new("Theme")
                .text_style(egui::TextStyle::Button)
                .weak(),
        );
        self.show_theme_menu_button(ui, self.theme.compact_label());
    }

    fn show_document_mode_menu_button(&mut self, ui: &mut egui::Ui) {
        let label = format!("Mode: {}", self.view.label());
        let response = ui
            .menu_button(&label, |ui| {
                ui.set_min_width(132.0);
                self.show_document_mode_choices(ui);
            })
            .response;
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::ComboBox, true, &label));
    }

    fn show_theme_menu_button(&mut self, ui: &mut egui::Ui, label: &str) {
        let response = ui
            .menu_button(label, |ui| {
                ui.set_min_width(120.0);
                self.show_theme_choices(ui);
            })
            .response;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::WidgetType::ComboBox,
                true,
                format!("Theme: {}", self.theme.label()),
            )
        });
        response.on_hover_text("Choose the application theme");
    }

    fn show_file_menu(&self, ui: &mut egui::Ui, command: &mut Option<FileCommand>) {
        for (index, candidate) in [
            FileCommand::New,
            FileCommand::Open,
            FileCommand::Reload,
            FileCommand::Save,
            FileCommand::SaveAs,
        ]
        .into_iter()
        .enumerate()
        {
            if index == 2 {
                ui.separator();
            }
            let mut button = egui::Button::new(candidate.label());
            if let Some(shortcut) = candidate.shortcut() {
                button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
            }
            let enabled = self.file_command_enabled(candidate);
            let response = ui.add_enabled(enabled, button);
            if !enabled && candidate == FileCommand::Reload {
                response
                    .clone()
                    .on_disabled_hover_text("Available after this document has been saved");
            } else if !enabled
                && candidate == FileCommand::Save
                && self.conflict.blocks_ordinary_save()
            {
                response.clone().on_disabled_hover_text(
                    "Choose how to handle the external file change before ordinary Save",
                );
            } else if !enabled && matches!(candidate, FileCommand::Save | FileCommand::SaveAs) {
                response.clone().on_disabled_hover_text(
                    "Reconcile every uncertain save outcome before saving again",
                );
            }
            if response.clicked() {
                command.get_or_insert(candidate);
                ui.close();
            }
        }
        ui.separator();
        if ui.button(FileCommand::Quit.label()).clicked() {
            command.get_or_insert(FileCommand::Quit);
            ui.close();
        }
    }

    fn file_command_enabled(&self, command: FileCommand) -> bool {
        match command {
            FileCommand::Reload => self.document.path().is_some(),
            FileCommand::Save => !self.ordinary_save_is_blocked(),
            FileCommand::SaveAs => !self.save_is_blocked(),
            FileCommand::New | FileCommand::Open | FileCommand::Quit => true,
        }
    }

    fn show_edit_menu(&self, ui: &mut egui::Ui, command: &mut Option<EditCommand>) {
        for (index, candidate) in [
            EditCommand::Undo,
            EditCommand::Redo,
            EditCommand::Cut,
            EditCommand::Copy,
            EditCommand::Paste,
            EditCommand::SelectAll,
            EditCommand::Find,
            EditCommand::FindNext,
            EditCommand::FindPrevious,
            EditCommand::Replace,
            EditCommand::GoToLine,
        ]
        .into_iter()
        .enumerate()
        {
            if matches!(index, 2 | 5 | 6 | 10) {
                ui.separator();
            }
            let enabled = self.edit_command_enabled(candidate);
            let shortcut = candidate.shortcut(ui.ctx().os());
            let button = egui::Button::new(candidate.label())
                .shortcut_text(ui.ctx().format_shortcut(&shortcut));
            let response = ui.add_enabled(enabled, button);
            if !enabled && candidate == EditCommand::GoToLine {
                response
                    .clone()
                    .on_disabled_hover_text("Available in Text Mode");
            }
            if response.clicked() {
                command.get_or_insert(candidate);
                ui.close();
            }
        }
    }

    fn edit_command_enabled(&self, command: EditCommand) -> bool {
        match command {
            EditCommand::Undo => self.history.can_undo(),
            EditCommand::Redo => self.history.can_redo(),
            EditCommand::Cut | EditCommand::Copy => {
                self.selection.anchor() != self.selection.active()
            }
            EditCommand::GoToLine => self.view == DocumentView::Text,
            EditCommand::Paste
            | EditCommand::SelectAll
            | EditCommand::Find
            | EditCommand::FindNext
            | EditCommand::FindPrevious
            | EditCommand::Replace => true,
        }
    }

    fn selected_source_text(&self) -> String {
        let range = self.selection.ordered_range();
        let start = range.start().min(self.text.len());
        let end = range.end().min(self.text.len());
        if start > end || !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
            return String::new();
        }
        self.text[start..end].to_owned()
    }

    fn clipboard_copy(&self, ctx: &egui::Context) {
        let selected = self.selected_source_text();
        if selected.is_empty() {
            return;
        }
        ctx.copy_text(selected);
    }

    fn clipboard_cut(&mut self, ctx: &egui::Context) {
        let selected = self.selected_source_text();
        if selected.is_empty() {
            return;
        }
        ctx.copy_text(selected);
        self.replace_selection_with("", EditOrigin::Programmatic);
    }

    fn clipboard_paste(&mut self, ctx: &egui::Context) {
        // System paste arrives as Event::Paste in the same input frame as Ctrl+V.
        // Menu Paste without an OS event cannot invent clipboard bytes; request
        // a platform paste so the next frame can deliver Event::Paste when the
        // integration supports it, and apply any paste event already present.
        let payload = ctx.input(|input| {
            input.events.iter().rev().find_map(|event| {
                if let egui::Event::Paste(text) = event {
                    Some(text.clone())
                } else {
                    None
                }
            })
        });
        if let Some(text) = payload.filter(|text| !text.is_empty()) {
            self.replace_selection_with(&text, EditOrigin::Paste);
            return;
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    }

    fn replace_selection_with(&mut self, replacement: &str, origin: EditOrigin) {
        let range = self.selection.ordered_range();
        let start = range.start().min(self.text.len());
        let end = range.end().min(self.text.len());
        if start > end || !self.text.is_char_boundary(start) || !self.text.is_char_boundary(end) {
            return;
        }
        let mut next = String::with_capacity(self.text.len() - (end - start) + replacement.len());
        next.push_str(&self.text[..start]);
        next.push_str(replacement);
        next.push_str(&self.text[end..]);
        let caret = start + replacement.len();
        if !next.is_char_boundary(caret) {
            return;
        }
        let after = Selection::caret(caret);
        let observed_at = EditTimestamp::new(std::time::Duration::from_secs(0));
        self.text = next;
        self.record_editor_change(EditorFrameOutcome {
            changed: true,
            selection: after,
            origin,
            observed_at,
        });
    }

    fn show_view_menu(&mut self, ui: &mut egui::Ui, command: &mut Option<ViewCommandRequest>) {
        ui.label("Mode");
        self.show_document_mode_choices(ui);
        ui.separator();
        ui.label("Theme");
        self.show_theme_choices(ui);
        ui.separator();
        self.show_view_preferences(ui, command);
    }

    fn show_view_preferences(&self, ui: &mut egui::Ui, command: &mut Option<ViewCommandRequest>) {
        let mut wrap_button = egui::Button::selectable(
            self.text_wrap.is_wrapped(),
            ViewCommand::ToggleWordWrap.label(),
        );
        wrap_button = wrap_button.shortcut_text(
            ui.ctx()
                .format_shortcut(&ViewCommand::ToggleWordWrap.shortcut()),
        );
        let wrap = ui.add_enabled(self.view == DocumentView::Text, wrap_button);
        if self.view != DocumentView::Text {
            wrap.clone()
                .on_disabled_hover_text("Markdown Mode wraps formatted content by design");
        }
        if wrap.clicked() {
            command.get_or_insert(ViewCommandRequest::restore_document(
                ViewCommand::ToggleWordWrap,
            ));
            ui.close();
        }
        let is_fullscreen = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
        let mut fs_button =
            egui::Button::selectable(is_fullscreen, ViewCommand::ToggleFullscreen.label());
        fs_button = fs_button.shortcut_text(
            ui.ctx()
                .format_shortcut(&ViewCommand::ToggleFullscreen.shortcut()),
        );
        if ui.add(fs_button).clicked() {
            command.get_or_insert(ViewCommandRequest::restore_document(
                ViewCommand::ToggleFullscreen,
            ));
            ui.close();
        }
        ui.menu_button(format!("Zoom: {}%", self.editor_zoom.percent()), |ui| {
            for candidate in [
                ViewCommand::ZoomIn,
                ViewCommand::ZoomOut,
                ViewCommand::ResetZoom,
            ] {
                let mut button = egui::Button::new(candidate.label());
                button = button.shortcut_text(ui.ctx().format_shortcut(&candidate.shortcut()));
                let enabled = self.view_command_enabled(candidate);
                let response = ui.add_enabled(enabled, button);
                if !enabled {
                    response.clone().on_disabled_hover_text(match candidate {
                        ViewCommand::ZoomIn => "Already at maximum zoom",
                        ViewCommand::ZoomOut => "Already at minimum zoom",
                        _ => "The current zoom is already 100%",
                    });
                }
                if response.clicked() {
                    command.get_or_insert(ViewCommandRequest::restore_document(candidate));
                    ui.close();
                }
            }
        });
    }

    const fn view_command_enabled(&self, command: ViewCommand) -> bool {
        match command {
            ViewCommand::ZoomIn => self.editor_zoom.can_zoom_in(),
            ViewCommand::ZoomOut => self.editor_zoom.can_zoom_out(),
            ViewCommand::ResetZoom => self.editor_zoom.percent() != 100,
            ViewCommand::ToggleWordWrap
            | ViewCommand::ToggleDocumentView
            | ViewCommand::ToggleFullscreen => true,
        }
    }

    fn show_document_mode_choices(&mut self, ui: &mut egui::Ui) {
        for view in [DocumentView::Text, DocumentView::Markdown] {
            if ui
                .selectable_label(self.view == view, view.label())
                .clicked()
            {
                self.request_document_view(view);
                ui.close();
            }
        }
    }

    fn show_theme_choices(&mut self, ui: &mut egui::Ui) {
        for theme in AppTheme::ALL {
            if ui
                .selectable_label(self.theme == theme, theme.label())
                .clicked()
            {
                self.select_theme(theme, ui.ctx());
                ui.close();
            }
        }
    }

    fn select_document_view(&mut self, view: DocumentView) {
        self.pending_document_view = None;
        if view == DocumentView::Markdown
            && let Some(limit) = markdown_projection_limit(&self.text)
        {
            self.error_msg = Some(markdown_limit_message(self.text.len(), limit));
            return;
        }
        if view == DocumentView::Markdown
            && !MarkdownEditor::can_restore_source_selection(&self.text, self.selection)
        {
            self.pending_selection_restore = Some(self.selection);
            self.error_msg = Some(
                "Markdown Mode could not map the current selection to exact UTF-8 source boundaries, so Noter kept Text Mode and preserved the selection."
                    .to_owned(),
            );
            return;
        }
        if self.view != view {
            self.view = view;
            if view != DocumentView::Text {
                self.go_to_line.reset();
            }
            self.markdown_editor.reset();
            self.pending_selection_restore = Some(self.selection);
        }
    }

    fn request_document_view(&mut self, view: DocumentView) {
        self.pending_document_view = (self.view != view).then_some(view);
    }

    fn apply_pending_document_view(&mut self) {
        if let Some(view) = self.pending_document_view.take() {
            self.select_document_view(view);
        }
    }

    fn select_theme(&mut self, theme: AppTheme, context: &egui::Context) {
        self.theme = theme;
        theme.apply(context);
    }

    fn show_help_menu(&mut self, ui: &mut egui::Ui) {
        if ui.button("Check for Updates...").clicked() {
            self.open_updates();
            ui.close();
        }
        ui.separator();
        if ui.button("About Noter").clicked() {
            self.open_about();
            ui.close();
        }
    }

    const fn open_about(&mut self) {
        self.about_open = true;
    }

    const fn open_updates(&mut self) {
        self.updates = UpdateStatusState::Open;
    }

    fn show_about(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }

        let mut open = self.about_open;
        let mut close = false;
        egui::Window::new("About Noter")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Noter");
                ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
                ui.label(ABOUT_SUMMARY);
                ui.separator();
                ui.label(ABOUT_MARKDOWN_STATUS);
                ui.label(ABOUT_PRIVACY);
                ui.label(ABOUT_LINK_BEHAVIOR);
                ui.hyperlink_to("Project repository", env!("CARGO_PKG_REPOSITORY"));
                ui.separator();
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        self.about_open = dialog_remains_open(open, close);
    }

    fn show_updates(&mut self, ctx: &egui::Context) {
        if !self.updates.is_open() {
            return;
        }

        let mut open = true;
        let mut close = false;
        egui::Window::new("Noter Updates")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Update status");
                ui.label(UPDATE_STATUS);
                ui.separator();
                ui.label("The releases page opens only when you select this link.");
                ui.hyperlink_to("Open Noter releases", RELEASES_URL);
                ui.separator();
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        if !dialog_remains_open(open, close) {
            self.updates = UpdateStatusState::Closed;
        }
    }

    fn show_unsaved_changes_confirmation(&mut self, ctx: &egui::Context) {
        let Some(action) = self.lifecycle.pending_intent() else {
            return;
        };
        if self.pending_hard_link_save.is_some() {
            return;
        }

        let document_name = self.document.path().map_or_else(
            || "Untitled".to_owned(),
            |path| {
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            },
        );
        let mut save = false;
        let mut discard = false;
        let mut cancel = false;
        let mut reconcile = None;
        let save_is_blocked = self.pending_abandon_save_is_blocked();
        let save_recoveries = &self.save_recoveries;

        let response =
            egui::Modal::new(egui::Id::new("unsaved-changes-confirmation")).show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.set_max_width(560.0);
                ui.heading("Save changes?");
                ui.label(format!("{document_name} has unsaved changes."));
                ui.label(pending_abandon_prompt(action));
                if !save_recoveries.is_empty() {
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(144.0)
                        .show(ui, |ui| {
                            reconcile = show_save_recovery_records(ui, save_recoveries, false);
                        });
                    ui.label(UNCERTAIN_SAVE_ABANDON_GUIDANCE);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    save = ui
                        .add_enabled(
                            !save_is_blocked,
                            egui::Button::new(egui::RichText::new("Save").strong()),
                        )
                        .on_disabled_hover_text(
                            "Ordinary Save is blocked until the uncertain save state is reconciled",
                        )
                        .clicked();
                    discard = ui.button("Discard Changes").clicked();
                    cancel = ui.button("Cancel").clicked();
                });
            });

        if let Some(index) = reconcile {
            self.pending_recovery_reconciliation = Some(index);
        } else if save {
            self.save_pending_abandon(ctx);
        } else if discard {
            self.discard_pending_abandon(ctx);
        } else if cancel || response.should_close() {
            self.cancel_pending_abandon();
        }
    }

    fn show_hard_link_confirmation(&mut self, ctx: &egui::Context) {
        let Some(pending) = self.pending_hard_link_save.as_ref() else {
            return;
        };
        let link_count = pending.link_count();
        let mut confirm = false;
        let mut cancel = false;

        let response = egui::Modal::new(egui::Id::new("hard-link-save-confirmation")).show(
            ctx,
            |ui| {
                ui.set_min_width(440.0);
                ui.heading("Confirm hard-link replacement");
                ui.label(format!(
                    "This file has {link_count} directory entries that currently share the same bytes."
                ));
                ui.label(
                    "Replacing this entry will save here only. The other hard links will keep the previous revision.",
                );
                ui.label("Your edits remain unsaved unless you confirm.");
                ui.separator();
                ui.horizontal(|ui| {
                    cancel = ui.button("Cancel").clicked();
                    confirm = ui.button("Replace This Entry").clicked();
                });
            },
        );

        if confirm {
            self.confirm_pending_hard_link_save();
            self.continue_pending_abandon_if_clean(ctx);
        } else if cancel || response.should_close() {
            self.pending_hard_link_save = None;
            self.continue_pending_abandon_if_clean(ctx);
        }
    }

    fn show_error(&mut self, ui: &mut egui::Ui) {
        let mut dismiss = false;
        if let Some(error) = self.error_msg.as_deref() {
            egui::Panel::top("error_bar").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(ui.visuals().error_fg_color, format!("Error: {error}"));
                    dismiss = ui.button("Dismiss").clicked();
                });
            });
        }
        if dismiss {
            self.error_msg = None;
        }
    }

    fn show_save_recovery_notice(&mut self, ui: &mut egui::Ui) {
        if !self
            .save_recoveries
            .iter()
            .any(|recovery| recovery.notice_pending)
        {
            return;
        }

        let mut dismiss = false;
        let mut reconcile = None;
        egui::Panel::top("save_recovery_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.strong("Recovery follow-up required");
                dismiss = ui.button("Dismiss notice").clicked();
            });
            egui::ScrollArea::vertical()
                .max_height(144.0)
                .show(ui, |ui| {
                    reconcile = show_save_recovery_records(ui, &self.save_recoveries, true);
                });
        });
        if let Some(index) = reconcile {
            self.pending_recovery_reconciliation = Some(index);
        }
        if dismiss {
            for recovery in &mut self.save_recoveries {
                recovery.notice_pending = false;
            }
        }
    }

    fn show_save_recovery_reconciliation(&mut self, ctx: &egui::Context) {
        let Some(index) = self.pending_recovery_reconciliation else {
            return;
        };
        let Some(recovery) = self.save_recoveries.get(index) else {
            self.pending_recovery_reconciliation = None;
            return;
        };
        let mut confirm = false;
        let mut cancel = false;
        let response =
            egui::Modal::new(egui::Id::new("save-recovery-reconciliation")).show(ctx, |ui| {
                (confirm, cancel) = show_save_recovery_reconciliation_contents(ui, recovery);
            });

        if confirm {
            self.reconcile_save_recovery(index);
        } else if cancel || response.should_close() {
            self.pending_recovery_reconciliation = None;
        }
    }

    fn reconcile_save_recovery(&mut self, index: usize) -> bool {
        if index >= self.save_recoveries.len() {
            self.pending_recovery_reconciliation = None;
            return false;
        }
        self.save_recoveries.remove(index);
        self.pending_recovery_reconciliation = None;
        if self.save_recoveries.is_empty()
            && self.error_msg.as_deref() == Some(SAVE_RECOVERY_BLOCK_MESSAGE)
        {
            self.error_msg = None;
        }
        true
    }

    fn markdown_issue_count(&mut self) -> usize {
        self.markdown_issue_count_with(count_markdown_diagnostics)
    }

    fn markdown_issue_count_with(&mut self, analyze: impl FnOnce(&str) -> usize) -> usize {
        let revision = self.document.revision();
        if let Some(cache) = self.markdown_issue_cache
            && cache.document_serial == self.document_editor_serial
            && cache.revision == revision
        {
            return cache.issue_count;
        }

        let issue_count = analyze(&self.text);
        self.markdown_issue_cache = Some(MarkdownIssueCache {
            document_serial: self.document_editor_serial,
            revision,
            issue_count,
        });
        issue_count
    }

    fn show_status(&mut self, ui: &mut egui::Ui) {
        let available_width = ui.available_width();
        let show_extended = available_width >= 680.0;
        let show_markdown_checks = available_width >= 900.0;
        let markdown_issue_count = (self.view == DocumentView::Markdown && show_markdown_checks)
            .then(|| self.markdown_issue_count());
        let StatusSnapshot {
            line,
            column,
            selected_characters,
        } = self.status_snapshot();
        let modified_label = persistence_status_label(&self.document, self.external_memory_at_risk);
        egui::Panel::bottom("status_bar")
            .exact_size(STATUS_BAR_HEIGHT)
            .show(ui, |ui| {
                ui.style_mut().override_text_style = Some(egui::TextStyle::Small);
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let document_label = self.document.path().map_or_else(
                        || "Untitled".to_owned(),
                        |path| {
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .into_owned()
                        },
                    );
                    let document_response = ui.label(document_label);
                    if let Some(path) = self.document.path() {
                        document_response.on_hover_text(path.display().to_string());
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("Ln {line}, Col {column}"));
                        ui.separator();
                        if selected_characters > 0 {
                            ui.label(format!("Selected: {selected_characters}"));
                            ui.separator();
                        }
                        ui.label(modified_label);
                        ui.separator();
                        if let Some(issue_count) = markdown_issue_count {
                            let label = if issue_count == 0 {
                                "Markdown checks: clean".to_owned()
                            } else {
                                format!("Markdown checks: {issue_count}")
                            };
                            ui.label(label);
                            ui.separator();
                        }
                        if show_extended {
                            ui.label(format!("Zoom {}%", self.editor_zoom.percent()));
                            ui.separator();
                            if self.view == DocumentView::Text {
                                ui.label(if self.text_wrap.is_wrapped() {
                                    "Wrap"
                                } else {
                                    "No Wrap"
                                });
                                ui.separator();
                            }
                            ui.label(format!("{} Mode", self.view.label()));
                            ui.separator();
                        }
                        ui.label(self.document.line_endings().status_label());
                        ui.separator();
                        ui.label(self.document.encoding().status_label());
                        if self.document.bom().is_present() {
                            ui.separator();
                            ui.label("BOM");
                        }
                    });
                });
            });
    }

    fn status_snapshot(&mut self) -> StatusSnapshot {
        self.status_snapshot_with(|source, selection| {
            let (line, column) = caret_line_column(source, selection.active());
            StatusSnapshot {
                line,
                column,
                selected_characters: selected_character_count(source, selection),
            }
        })
    }

    fn status_snapshot_with(
        &mut self,
        analyze: impl FnOnce(&str, Selection) -> StatusSnapshot,
    ) -> StatusSnapshot {
        let revision = self.document.revision();
        if let Some(cache) = self.status_cache
            && cache.document_serial == self.document_editor_serial
            && cache.revision == revision
            && cache.selection == self.selection
        {
            return cache.snapshot;
        }

        let snapshot = analyze(&self.text, self.selection);
        self.status_cache = Some(StatusCache {
            document_serial: self.document_editor_serial,
            revision,
            selection: self.selection,
            snapshot,
        });
        snapshot
    }

    fn show_format_toolbar(
        &mut self,
        ui: &mut egui::Ui,
        view_command: &mut Option<ViewCommandRequest>,
    ) {
        if self.view != DocumentView::Markdown {
            return;
        }

        egui::Panel::top("editor_toolbar")
            .exact_size(EDITOR_TOOLBAR_HEIGHT)
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    self.markdown_editor.toolbar(ui);
                    let editor_is_active = self.markdown_editor.is_editing();
                    let zoom = self.editor_zoom;
                    if !editor_is_active && ui.available_width() >= INLINE_ZOOM_MIN_WIDTH + 300.0 {
                        ui.weak("Click or drag in formatted text to edit or format");
                    }
                    if ui.available_width() >= INLINE_ZOOM_MIN_WIDTH {
                        ui.add_space(ui.available_width() - INLINE_ZOOM_MIN_WIDTH);
                        Self::show_inline_zoom_controls(ui, zoom, view_command);
                    }
                });
            });
    }

    fn show_inline_zoom_controls(
        ui: &mut egui::Ui,
        zoom: EditorZoom,
        command: &mut Option<ViewCommandRequest>,
    ) {
        if ui.available_width() < INLINE_ZOOM_MIN_WIDTH {
            return;
        }

        ui.spacing_mut().item_spacing.x = 4.0;
        ui.weak("Zoom");
        if Self::inline_zoom_button(
            ui,
            "-",
            ViewCommand::ZoomOut,
            zoom.can_zoom_out(),
            "Decrease document zoom",
        )
        .clicked()
        {
            command.get_or_insert(ViewCommandRequest::preserve_control(ViewCommand::ZoomOut));
        }
        let reset = Self::inline_zoom_button(
            ui,
            &format!("{}%", zoom.percent()),
            ViewCommand::ResetZoom,
            true,
            "Click to reset document zoom to 100%. Scroll to zoom in or out.",
        );
        if reset.clicked() {
            command.get_or_insert(ViewCommandRequest::preserve_control(ViewCommand::ResetZoom));
        } else if reset.hovered() {
            let wheel_delta = ui.input(|input| {
                input
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        egui::Event::MouseWheel { delta, .. } => Some(*delta),
                        _ => None,
                    })
                    .fold(egui::Vec2::ZERO, |total, delta| total + delta)
            });
            if let Some(wheel_command) = zoom_command_from_wheel_delta(wheel_delta) {
                command.get_or_insert(ViewCommandRequest::preserve_control(wheel_command));
            }
        }
        if Self::inline_zoom_button(
            ui,
            "+",
            ViewCommand::ZoomIn,
            zoom.can_zoom_in(),
            "Increase document zoom",
        )
        .clicked()
        {
            command.get_or_insert(ViewCommandRequest::preserve_control(ViewCommand::ZoomIn));
        }
    }

    fn inline_zoom_button(
        ui: &mut egui::Ui,
        visible_label: &str,
        command: ViewCommand,
        enabled: bool,
        hover_text: &str,
    ) -> egui::Response {
        let width = if command == ViewCommand::ResetZoom {
            54.0
        } else {
            30.0
        };
        let response = ui.add_enabled(
            enabled,
            egui::Button::new(visible_label).min_size(egui::vec2(width, 28.0)),
        );
        let accessible_label = if command == ViewCommand::ResetZoom {
            format!("{visible_label}, {}", command.label())
        } else {
            command.label().to_owned()
        };
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &accessible_label)
        });
        if command == ViewCommand::ResetZoom {
            ui.ctx().accesskit_node_builder(response.id, |node| {
                node.set_value(visible_label);
            });
        }
        response.on_hover_text(hover_text)
    }

    fn show_find_bar(&mut self, ui: &mut egui::Ui) {
        let action = self
            .find_bar
            .show(ui, self.document.revision(), &self.text, self.selection);
        if let Some(action) = action {
            self.execute_find_bar_action(action, edit_timestamp(ui));
        }
    }

    fn show_go_to_line(&mut self, context: &egui::Context) {
        let Some(action) = self.go_to_line.show(context, &self.text) else {
            return;
        };
        match action {
            GoToLineAction::Navigate(offset) => {
                self.selection = Selection::caret(offset);
                self.pending_selection_restore = Some(self.selection);
                self.preserve_focus_on_selection_restore = false;
            }
            GoToLineAction::Close => {
                self.pending_selection_restore = Some(self.selection);
                self.preserve_focus_on_selection_restore = false;
            }
        }
    }

    fn show_editor(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(ui.visuals().extreme_bg_color)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| {
                let pointer_over_document = ui.rect_contains_pointer(ui.max_rect());
                if pointer_over_document {
                    let delta = ui.input(egui::InputState::zoom_delta);
                    if self.pointer_zoom.apply(delta, &mut self.editor_zoom) {
                        self.pending_selection_restore = Some(self.selection);
                        self.preserve_focus_on_selection_restore = false;
                    }
                } else {
                    self.pointer_zoom.reset();
                }
                apply_editor_zoom(ui.style_mut(), self.editor_zoom);
                let previous_selection = self.selection;
                let outcome = match self.view {
                    DocumentView::Text => self.show_text_editor(ui),
                    DocumentView::Markdown => self.show_markdown_editor(ui),
                };
                if outcome.changed {
                    self.record_editor_change(outcome);
                } else {
                    self.selection = valid_selection_or_end(&self.text, outcome.selection);
                }
                if self.selection != previous_selection {
                    // Status is painted before the editor so Ln/Col can lag one
                    // frame. Book a follow-up paint after caret-only movement,
                    // including after the window has been sleeping.
                    ui.ctx().request_repaint();
                }
            });
    }

    fn record_editor_change(&mut self, outcome: EditorFrameOutcome) {
        let maximum = self.interactive_text_maximum();
        if self.text.len() > maximum {
            let error =
                format!("the result exceeded the current {maximum}-byte interactive safety limit");
            self.restore_editor_after_failed_change(&error);
            return;
        }
        let before = String::from(self.document.rope());
        let transaction = EditTransaction::between(
            self.document.revision(),
            &before,
            &self.text,
            self.selection,
            outcome.selection,
            outcome.origin,
            outcome.observed_at,
        );
        match transaction {
            Ok(Some(transaction)) => {
                self.record_editor_change_with(&transaction, Document::apply_transaction);
            }
            Ok(None) => self.selection = outcome.selection,
            Err(error) => self.restore_editor_after_failed_change(&error),
        }
    }

    fn record_editor_change_with(
        &mut self,
        transaction: &EditTransaction,
        record: impl FnOnce(&mut Document, &EditTransaction) -> Result<AppliedTransaction, EditError>,
    ) {
        match record(&mut self.document, transaction) {
            Ok(applied) => {
                self.selection = applied.selection();
                let history_outcome = self.history.record(applied);
                self.text = String::from(self.document.rope());
                self.markdown_issue_cache = None;
                self.crash_recovery
                    .on_edited(&self.document, self.selection);
                if history_outcome == HistoryRecordOutcome::ClearedForOversizedTransaction {
                    self.error_msg = Some(format!(
                        "The edit was applied, but its {}-byte inverse exceeded the bounded undo history and older history was cleared.",
                        transaction.retained_bytes()
                    ));
                }
            }
            Err(error) => {
                self.restore_editor_after_failed_change(&error);
                return;
            }
        }
        if self.view == DocumentView::Markdown {
            let Some(limit) = markdown_projection_limit(&self.text) else {
                return;
            };
            self.view = DocumentView::Text;
            self.markdown_editor.reset();
            self.pending_selection_restore = Some(self.selection);
            self.error_msg = Some(markdown_limit_message(self.text.len(), limit));
        }
    }

    fn restore_editor_after_failed_change(&mut self, error: &dyn std::fmt::Display) {
        self.text = String::from(self.document.rope());
        self.advance_document_editor();
        self.markdown_editor.reset();
        self.pending_selection_restore = Some(self.selection);
        self.error_msg = Some(format!(
            "Failed to record edit. The editor was restored to the last authoritative text: {error}"
        ));
    }

    fn show_text_editor(&mut self, ui: &mut egui::Ui) -> EditorFrameOutcome {
        self.show_text_editor_with_limit(ui, self.interactive_text_maximum())
    }

    fn show_text_editor_with_limit(
        &mut self,
        ui: &mut egui::Ui,
        maximum_text_bytes: usize,
    ) -> EditorFrameOutcome {
        let observed_at = edit_timestamp(ui);
        let origin = direct_input_origin(ui, EditOrigin::TextInput);
        let editor_id = self.editor_id();
        let ime_focus_restore =
            retain_active_ime_commit_focus(ui, editor_id, self.text_ime_composition.is_some());
        // Apply pure word/line-home/document policy before TextEdit so Ctrl/Cmd
        // and Home/End share one path with unit tests. Plain arrows stay with
        // egui for platform grapheme movement.
        let editor_focused = ui.memory(|memory| memory.has_focus(editor_id));
        let deferred =
            take_events_after_ime_terminal(ui, editor_focused, self.text_ime_composition.is_some());
        if !deferred.is_empty() {
            self.defer_input_events(deferred);
            ui.ctx().request_repaint();
        }
        if editor_focused && self.text_ime_composition.is_none() {
            self.serialize_next_text_navigation(ui);
            let gestures =
                consume_navigation_gestures(ui, KeyboardPlatform::from_egui(ui.ctx().os()));
            if !gestures.is_empty() {
                let mut next = self.selection;
                for gesture in gestures {
                    next = gesture.apply(&self.text, next);
                }
                self.selection = valid_selection_or_end(&self.text, next);
                self.pending_selection_restore = Some(self.selection);
                self.preserve_focus_on_selection_restore = true;
            }
        }
        let (restored_selection, restore_focus) = self.restore_text_editor_selection(ui, editor_id);
        let (ime_state, event_was_limited) =
            self.prepare_text_ime_input(ui, editor_id, maximum_text_bytes);

        let viewport_size = ui.available_size();
        let desired_width = if self.text_wrap.is_wrapped() {
            viewport_size.x
        } else {
            f32::INFINITY
        };
        let (response, buffer_was_limited) = {
            let editable_text = self
                .text_ime_composition
                .as_mut()
                .map_or(&mut self.text, |composition| &mut composition.draft);
            let mut buffer = BoundedTextBuffer::new(editable_text, maximum_text_bytes);
            let editor = egui::TextEdit::multiline(&mut buffer)
                .id(editor_id)
                .font(egui::TextStyle::Monospace)
                .code_editor()
                .desired_width(desired_width)
                .min_size(viewport_size)
                .frame(egui::Frame::NONE)
                .lock_focus(true);
            let response = if self.text_wrap.is_wrapped() {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| ui.add(editor))
                    .inner
            } else {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| ui.add(editor))
                    .inner
            };
            (response, buffer.was_limited())
        };
        let input_was_limited = event_was_limited || buffer_was_limited;
        if input_was_limited {
            self.error_msg = Some(format!(
                "{TEXT_INPUT_LIMIT_PREFIX} {maximum_text_bytes}-byte safety limit. Text within the remaining budget was preserved."
            ));
        }
        if restore_focus {
            // A mode/menu pointer click can surrender focus during TextEdit's
            // interaction pass. Reassert it after the widget as well.
            response.request_focus();
        }
        let mut state = egui::TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
        let displayed_text = self.text_editor_displayed_text();
        let widget_selection = state.cursor.char_range().map_or_else(
            || {
                restored_selection
                    .unwrap_or_else(|| valid_selection_or_end(displayed_text, self.selection))
            },
            |range| selection_from_cursor_range(displayed_text, range),
        );
        let (changed, selection) = self.resolve_text_ime_input(
            ui,
            ime_state,
            response.changed(),
            widget_selection,
            &mut state,
        );
        // Shared document history owns Undo and Redo. Discard egui's whole-string
        // snapshots so they cannot retain a second, separately bounded history.
        state.clear_undoer();
        egui::TextEdit::store_state(ui.ctx(), editor_id, state);
        if let Some(restore) = ime_focus_restore {
            restore.restore(ui, editor_id);
        }
        EditorFrameOutcome {
            changed,
            selection,
            origin,
            observed_at,
        }
    }

    fn text_editor_displayed_text(&self) -> &str {
        self.text_ime_composition
            .as_ref()
            .map_or(self.text.as_str(), |composition| composition.draft.as_str())
    }

    fn restore_text_editor_selection(
        &mut self,
        ui: &egui::Ui,
        editor_id: egui::Id,
    ) -> (Option<Selection>, bool) {
        if self.pending_selection_restore.is_some() {
            self.text_ime_composition = None;
        }
        let restored_selection = self
            .pending_selection_restore
            .take()
            .map(|pending| valid_selection_or_end(&self.text, pending));
        let restore_focus = restored_selection.is_some()
            && !std::mem::take(&mut self.preserve_focus_on_selection_restore);
        let mut state = egui::TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
        if let Some(selection) = restored_selection {
            if let Some(cursor_range) = cursor_range_from_selection(&self.text, selection) {
                state.cursor.set_char_range(Some(cursor_range));
            }
            if restore_focus {
                ui.memory_mut(|memory| memory.request_focus(editor_id));
            }
        }
        state.clear_undoer();
        egui::TextEdit::store_state(ui.ctx(), editor_id, state);
        (restored_selection, restore_focus)
    }

    fn prepare_text_ime_input(
        &mut self,
        ui: &egui::Ui,
        editor_id: egui::Id,
        maximum_text_bytes: usize,
    ) -> (ImeFrameState, bool) {
        let composition_was_active = self.text_ime_composition.is_some();
        let state_before_sanitizing =
            focused_ime_frame_state(ui, editor_id, composition_was_active);
        if self.text_ime_composition.is_none() && state_before_sanitizing != ImeFrameState::None {
            self.text_ime_composition = Some(TextImeComposition {
                draft: self.text.clone(),
                base_selection: self.selection,
            });
        }
        let event_was_limited = sanitize_bounded_text_events(
            ui,
            editor_id,
            self.text_editor_displayed_text(),
            maximum_text_bytes,
        );
        (
            focused_ime_frame_state(ui, editor_id, composition_was_active),
            event_was_limited,
        )
    }

    fn resolve_text_ime_input(
        &mut self,
        ui: &egui::Ui,
        ime_state: ImeFrameState,
        widget_changed: bool,
        widget_selection: Selection,
        state: &mut egui::text_edit::TextEditState,
    ) -> (bool, Selection) {
        match ime_state {
            ImeFrameState::Composing => {
                let base_selection = self
                    .text_ime_composition
                    .as_ref()
                    .map_or(self.selection, |composition| composition.base_selection);
                (false, base_selection)
            }
            ImeFrameState::Committed => {
                if let Some(composition) = self.text_ime_composition.take() {
                    let changed = composition.draft != self.text;
                    self.text = composition.draft;
                    (changed, widget_selection)
                } else {
                    (widget_changed, widget_selection)
                }
            }
            ImeFrameState::Cancelled => {
                let base_selection = self
                    .text_ime_composition
                    .take()
                    .map_or(self.selection, |composition| composition.base_selection);
                if let Some(cursor_range) = cursor_range_from_selection(&self.text, base_selection)
                {
                    state.cursor.set_char_range(Some(cursor_range));
                }
                self.pending_selection_restore = Some(base_selection);
                self.preserve_focus_on_selection_restore = true;
                ui.ctx().request_repaint();
                (false, base_selection)
            }
            ImeFrameState::None => self
                .text_ime_composition
                .take()
                .map_or((widget_changed, widget_selection), |composition| {
                    (false, composition.base_selection)
                }),
        }
    }

    fn show_markdown_editor(&mut self, ui: &mut egui::Ui) -> EditorFrameOutcome {
        let observed_at = edit_timestamp(ui);
        // Commit any dirty Markdown draft before selection restore. Restore
        // replaces the active block and must not discard uncommitted formatting
        // or typing from the same frame's toolbar or prior unfinished edit.
        let committed_origin = self.markdown_editor.commit_pending_source(&mut self.text);
        if let Some(pending) = self.pending_selection_restore.take() {
            let restore_focus = !std::mem::take(&mut self.preserve_focus_on_selection_restore);
            let _ = self.markdown_editor.restore_source_selection_with_focus(
                &self.text,
                pending,
                restore_focus,
            );
        }
        let outcome = egui::ScrollArea::vertical()
            .show(ui, |ui| {
                ui.add_space(MARKDOWN_READING_TOP_PADDING);
                let outcome = self.markdown_editor.show(ui, &mut self.text);
                ui.add_space(MARKDOWN_READING_BOTTOM_PADDING);
                outcome
            })
            .inner;
        let input_was_limited = self.markdown_editor.take_input_was_limited();
        let selection = self
            .markdown_editor
            .source_selection()
            .unwrap_or_else(|| valid_selection_or_end(&self.text, self.selection));
        if let Some(limit) = outcome.projection_limit() {
            self.view = DocumentView::Text;
            self.markdown_editor.reset();
            self.pending_selection_restore = Some(selection);
            self.error_msg = Some(markdown_limit_message(self.text.len(), limit));
        } else if input_was_limited {
            self.error_msg = Some(MARKDOWN_INPUT_LIMIT_MESSAGE.to_owned());
        }
        EditorFrameOutcome {
            changed: outcome.changed() || committed_origin.is_some(),
            selection,
            origin: outcome
                .origin()
                .or(committed_origin)
                .unwrap_or(EditOrigin::MarkdownInput),
            observed_at,
        }
    }

    fn render_frame(&mut self, ui: &mut egui::Ui) {
        // Inspect before dispatching commands so a focus-regain observation can
        // protect the retained in-memory revision in this same input frame.
        let blocking_modal_before_inspection = self.blocking_modal_open();
        self.maybe_inspect_external_change(ui.ctx());
        let blocking_modal_at_start = self.blocking_modal_open();
        let modal_opened_during_inspection =
            !blocking_modal_before_inspection && blocking_modal_at_start;
        let mut blocking_modal_input = if blocking_modal_at_start {
            self.take_blocking_modal_input(ui, modal_opened_during_inspection)
        } else {
            self.restore_deferred_input(ui);
            self.find_bar.restore_deferred_input(ui);
            self.markdown_editor.restore_deferred_input(ui);
            Vec::new()
        };
        let isolated_ime_commit = if blocking_modal_at_start {
            None
        } else {
            self.isolate_document_ime_commit(ui)
        };
        let document_shortcuts_enabled =
            !self.find_bar.owns_text_focus(ui.ctx()) && !self.go_to_line.owns_text_focus(ui.ctx());
        let go_to_line_shortcut_enabled =
            document_shortcuts_enabled && self.view == DocumentView::Text;
        let input_shortcut = self.collect_input_shortcut(
            ui,
            document_shortcuts_enabled,
            go_to_line_shortcut_enabled,
        );
        let mut file_command = None;
        let mut edit_command = None;
        let mut view_command = None;
        self.show_menu(ui, &mut file_command, &mut edit_command, &mut view_command);
        self.show_error(ui);
        self.show_crash_recovery_quarantine_notices(ui);
        self.show_crash_recovery_persist_failure(ui);
        self.show_crash_recovery_cleanup_failure(ui);
        self.show_save_recovery_notice(ui);
        let recovery_offer_open = self.crash_recovery.active_offer().is_some();
        let commands_enabled = !blocking_modal_at_start;
        let input_edit_executed =
            self.execute_input_shortcut(input_shortcut, commands_enabled, ui.ctx());
        if !blocking_modal_at_start && self.blocking_modal_open() {
            self.discard_deferred_input();
        }
        let menu_edit_executed = if commands_enabled {
            edit_command.take().is_some_and(|command| {
                self.execute_edit_command(command, ui.ctx());
                true
            })
        } else {
            false
        };
        self.show_find_bar(ui);
        self.show_format_toolbar(ui, &mut view_command);
        if commands_enabled && let Some(command) = view_command {
            self.execute_view_command(command, ui.ctx());
            self.apply_pending_document_view();
        }
        self.show_status(ui);
        self.show_editor_with_isolated_ime_commit(ui, isolated_ime_commit);
        self.markdown_editor.finish_input_frame();
        self.apply_pending_document_view();
        if commands_enabled
            && !input_edit_executed
            && !menu_edit_executed
            && let Some(command) = file_command
        {
            self.execute_file_command(command, ui.ctx());
        }
        if !recovery_offer_open {
            self.crash_recovery.on_tick(&self.document, self.selection);
            if let Some(delay) = self.crash_recovery.next_persist_delay() {
                // The window sleeps when nothing is happening, so a dirty
                // document has to book its own wake-up for the next persist.
                ui.ctx().request_repaint_after(delay);
            }
        }
        self.surface_crash_recovery_failures();
        self.protect_native_close(ui.ctx());
        self.update_title(ui.ctx());
        self.show_about(ui.ctx());
        self.show_updates(ui.ctx());
        self.show_go_to_line(ui.ctx());
        if !blocking_modal_input.is_empty() {
            ui.input_mut(|input| {
                blocking_modal_input.append(&mut input.events);
                std::mem::swap(&mut blocking_modal_input, &mut input.events);
            });
        }
        if !blocking_modal_at_start && self.blocking_modal_open() {
            self.discard_deferred_input();
        }
        if recovery_offer_open {
            self.show_startup_recovery_offer(ui.ctx());
        } else if self.pending_recovery_reconciliation.is_some() {
            self.show_save_recovery_reconciliation(ui.ctx());
        } else if self.pending_hard_link_save.is_some() {
            self.show_hard_link_confirmation(ui.ctx());
        } else if self.lifecycle.pending_intent().is_some() {
            self.show_unsaved_changes_confirmation(ui.ctx());
        } else if self.conflict.is_prompting() {
            self.show_external_change_confirmation(ui.ctx());
        }
    }

    fn take_blocking_modal_input(
        &mut self,
        ui: &egui::Ui,
        modal_opened_during_inspection: bool,
    ) -> Vec<egui::Event> {
        let events = ui.input_mut(|input| std::mem::take(&mut input.events));
        if !modal_opened_during_inspection {
            return events;
        }
        let (deferred, modal): (Vec<_>, Vec<_>) = events
            .into_iter()
            .partition(document_input_event_survives_modal_transition);
        if !deferred.is_empty() {
            self.defer_input_events(deferred);
            ui.ctx().request_repaint();
        }
        modal
    }

    fn surface_crash_recovery_failures(&mut self) {
        if self.error_msg.is_some() {
            return;
        }
        if self.crash_recovery.has_persist_failure() {
            self.error_msg = Some(RECOVERY_PERSIST_FAILURE_MESSAGE.to_owned());
        } else if self.crash_recovery.has_cleanup_failure() {
            self.error_msg = Some(RECOVERY_CLEANUP_FAILURE_MESSAGE.to_owned());
        }
    }

    fn show_crash_recovery_quarantine_notices(&mut self, ui: &mut egui::Ui) {
        if self.crash_recovery.quarantine_notices().is_empty() {
            return;
        }
        egui::Panel::top("crash_recovery_quarantine").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "Noter found an issue while reviewing private recovery records.",
                );
                if ui.button("Dismiss").clicked() {
                    self.crash_recovery.clear_quarantine_notices();
                }
            });
            for notice in self.crash_recovery.quarantine_notices() {
                ui.label(notice.as_str());
            }
        });
    }

    fn show_crash_recovery_persist_failure(&mut self, ui: &mut egui::Ui) {
        if !self.crash_recovery.has_persist_failure() {
            return;
        }
        egui::Panel::top("crash_recovery_persist_failure").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, RECOVERY_PERSIST_FAILURE_MESSAGE);
                if ui.button("Dismiss notice").clicked() {
                    self.crash_recovery.dismiss_persist_failure();
                    if self.error_msg.as_deref() == Some(RECOVERY_PERSIST_FAILURE_MESSAGE) {
                        self.error_msg = None;
                    }
                }
            });
        });
    }

    fn show_crash_recovery_cleanup_failure(&mut self, ui: &mut egui::Ui) {
        if !self.crash_recovery.has_cleanup_failure() {
            return;
        }
        egui::Panel::top("crash_recovery_cleanup_failure").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.colored_label(ui.visuals().warn_fg_color, RECOVERY_CLEANUP_FAILURE_MESSAGE);
                if ui.button("Dismiss notice").clicked() {
                    self.crash_recovery.dismiss_cleanup_failure();
                    if self.error_msg.as_deref() == Some(RECOVERY_CLEANUP_FAILURE_MESSAGE) {
                        self.error_msg = None;
                    }
                }
            });
        });
    }

    fn show_startup_recovery_offer(&mut self, ctx: &egui::Context) {
        let Some(offer) = self.crash_recovery.active_offer() else {
            return;
        };
        let label = offer.original_path_label();
        let content_preview_len = offer.metadata().content_len();
        let mut restore = false;
        let mut discard = false;
        let mut later = false;
        let response = egui::Modal::new(egui::Id::new("startup-crash-recovery")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.heading("Recover unsaved work?");
            ui.add_space(8.0);
            ui.label(format!(
                "Noter found a private recovery copy for \"{label}\" ({content_preview_len} bytes)."
            ));
            ui.label(
                "Restore opens it as an unsaved document in this window. Later hides this offer for now and keeps the private copy. Discard deletes only that private recovery copy. Your original file on disk is not changed until you Save.",
            );
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                restore = ui
                    .add(egui::Button::new("Restore").min_size(egui::vec2(100.0, 28.0)))
                    .clicked();
                later = ui
                    .add(egui::Button::new("Later").min_size(egui::vec2(100.0, 28.0)))
                    .clicked();
                discard = ui
                    .add(egui::Button::new("Discard").min_size(egui::vec2(100.0, 28.0)))
                    .clicked();
            });
        });
        if restore {
            if let Some(offer) = self.crash_recovery.active_offer()
                && offer.metadata().content_len() > INTERACTIVE_TEXT_MAX_BYTES
            {
                self.error_msg = Some(format!(
                    "The recovered document is larger than the current {INTERACTIVE_TEXT_MAX_LABEL} interactive limit and was not opened. Use Later to keep the private copy, Discard to remove only that copy, or restore it with a future virtualized editor."
                ));
                return;
            }
            let preferred_view = self.crash_recovery.active_offer().and_then(|offer| {
                std::str::from_utf8(offer.metadata().original_path())
                    .ok()
                    .filter(|path| !path.is_empty())
                    .map(std::path::Path::new)
                    .map(preferred_view_for_path)
            });
            match self.crash_recovery.restore_active_offer() {
                Ok((document, selection)) => {
                    self.text = String::from(document.rope());
                    self.document = document;
                    self.history.reset(self.document.revision());
                    self.selection = valid_selection_or_end(&self.text, selection);
                    self.pending_selection_restore = Some(self.selection);
                    self.advance_document_editor();
                    self.find_bar.reset();
                    self.go_to_line.reset();
                    self.markdown_editor.reset();
                    self.markdown_issue_cache = None;
                    self.error_msg = None;
                    self.reset_external_conflict_state();
                    self.view = DocumentView::Text;
                    if let Some(view) = preferred_view {
                        self.select_document_view(view);
                    }
                    self.crash_recovery
                        .on_edited(&self.document, self.selection);
                    // Remaining offers stay on disk for a later untitled launch.
                    // Presenting the next one now would replace this restored document.
                    self.crash_recovery.defer_startup_offers();
                }
                Err(message) => {
                    self.error_msg = Some(message);
                }
            }
        } else if later || response.should_close() {
            self.crash_recovery.defer_startup_offers();
        } else if discard {
            self.crash_recovery.discard_active_offer();
        }
    }

    fn editor_id(&self) -> egui::Id {
        egui::Id::new((EDITOR_ID_SALT, self.document_editor_serial))
    }

    fn document_ime_composition_active(&self) -> bool {
        match self.view {
            DocumentView::Text => self.text_ime_composition.is_some(),
            DocumentView::Markdown => self.markdown_editor.has_active_ime_composition(),
        }
    }

    fn isolate_document_ime_commit(&mut self, ui: &egui::Ui) -> Option<egui::Event> {
        let (commit, deferred) =
            isolate_active_ime_commit(ui, self.document_ime_composition_active())?;
        if !deferred.is_empty() {
            self.defer_input_events(deferred);
            ui.ctx().request_repaint();
        }
        Some(commit)
    }

    fn show_editor_with_isolated_ime_commit(
        &mut self,
        ui: &mut egui::Ui,
        commit: Option<egui::Event>,
    ) {
        let commit_was_isolated = commit.is_some();
        if let Some(commit) = commit {
            ui.input_mut(|input| input.events.push(commit));
        }
        self.show_editor(ui);
        if commit_was_isolated {
            let removed = ui.input_mut(|input| input.events.pop());
            debug_assert!(matches!(
                removed,
                Some(egui::Event::Ime(egui::ImeEvent::Commit(_)))
            ));
        }
    }

    fn advance_document_editor(&mut self) {
        self.document_editor_serial = self.document_editor_serial.wrapping_add(1);
        self.text_ime_composition = None;
        self.discard_deferred_input();
    }

    #[cfg(feature = "screenshot-qa")]
    fn advance_screenshot_capture(&mut self, ctx: &egui::Context) {
        self.markdown_editor.suppress_capture_focus(ctx);
        let completed = ctx.input(|input| {
            input.events.iter().find_map(|event| {
                let egui::Event::Screenshot {
                    user_data, image, ..
                } = event
                else {
                    return None;
                };
                user_data
                    .data
                    .as_ref()
                    .and_then(|data| data.downcast_ref::<String>())
                    .filter(|marker| marker.as_str() == ScreenshotCapture::MARKER)
                    .map(|_| image.clone())
            })
        });

        if let Some(image) = completed {
            let Some(capture) = self.screenshot.take() else {
                return;
            };
            if let Err(error) = write_screenshot_png(&capture.path, &image) {
                self.error_msg = Some(format!("Failed to write screenshot: {error}"));
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let Some(capture) = self.screenshot.as_mut() else {
            return;
        };
        if capture.requested {
            return;
        }
        if capture.frames_remaining > 0 {
            capture.frames_remaining -= 1;
            ctx.request_repaint();
            return;
        }
        capture.requested = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(
            ScreenshotCapture::MARKER.to_owned(),
        )));
    }
}

struct BoundedTextWriter {
    output: String,
    maximum_bytes: usize,
    truncation_suffix: &'static str,
    truncated: bool,
}

impl BoundedTextWriter {
    fn new(output: String, maximum_bytes: usize, truncation_suffix: &'static str) -> Self {
        debug_assert!(output.capacity() >= maximum_bytes);
        debug_assert!(truncation_suffix.len() <= maximum_bytes);
        Self {
            output,
            maximum_bytes,
            truncation_suffix,
            truncated: false,
        }
    }

    fn finish(mut self) -> String {
        if self.truncated {
            self.output.push_str(self.truncation_suffix);
        }
        debug_assert!(self.output.len() <= self.maximum_bytes);
        self.output
    }
}

impl fmt::Write for BoundedTextWriter {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        if self.truncated {
            return Ok(());
        }
        if self.output.len().saturating_add(value.len()) <= self.maximum_bytes {
            self.output.push_str(value);
            return Ok(());
        }

        let prefix_limit = self
            .maximum_bytes
            .saturating_sub(self.truncation_suffix.len());
        if self.output.len() > prefix_limit {
            let mut boundary = prefix_limit;
            while !self.output.is_char_boundary(boundary) {
                boundary -= 1;
            }
            self.output.truncate(boundary);
        }
        let available = prefix_limit.saturating_sub(self.output.len());
        let mut boundary = available.min(value.len());
        while !value.is_char_boundary(boundary) {
            boundary -= 1;
        }
        self.output.push_str(&value[..boundary]);
        self.truncated = true;
        Ok(())
    }
}

fn bounded_destination_label(path: &Path) -> Option<String> {
    let mut output = String::new();
    output
        .try_reserve_exact(MAX_SAVE_RECOVERY_LABEL_BYTES)
        .ok()?;
    let mut writer = BoundedTextWriter::new(output, MAX_SAVE_RECOVERY_LABEL_BYTES, "...");
    match (path.parent().and_then(Path::file_name), path.file_name()) {
        (Some(parent), Some(name)) => {
            let _ = write!(
                writer,
                "{}{}{}",
                parent.to_string_lossy(),
                std::path::MAIN_SEPARATOR,
                name.to_string_lossy()
            );
        }
        (_, Some(name)) => {
            let _ = write!(writer, "{}", name.to_string_lossy());
        }
        _ => {
            let _ = write!(writer, "{}", path.display());
        }
    }
    Some(writer.finish())
}

fn write_save_recovery_message(
    output: String,
    recovery_artifact: &noter::core::save::StorageError,
    error: &noter::core::save::StorageError,
) -> String {
    let mut writer = BoundedTextWriter::new(
        output,
        MAX_SAVE_RECOVERY_MESSAGE_BYTES,
        SAVE_RECOVERY_TRUNCATION_SUFFIX,
    );
    let _ = write!(
        writer,
        "Save state is uncertain. Noter has stopped every save until you explicitly reconcile this outcome. Recovery follow-up: {recovery_artifact}. Commit detail: {error}"
    );
    writer.finish()
}

fn show_save_recovery_records(
    ui: &mut egui::Ui,
    recoveries: &[SaveRecovery],
    pending_only: bool,
) -> Option<usize> {
    let mut reconcile = None;
    let mut displayed_record = false;
    for (index, recovery) in recoveries.iter().enumerate() {
        if pending_only && !recovery.notice_pending {
            continue;
        }
        if displayed_record {
            ui.separator();
        }
        displayed_record = true;
        ui.strong(format!("Destination: {}", recovery.destination_label));
        ui.colored_label(ui.visuals().error_fg_color, &recovery.message);
        show_save_recovery_copy_action(ui, recovery);
        ui.horizontal(|ui| {
            if ui.button("Reconcile...").clicked() {
                reconcile = Some(index);
            }
        });
    }
    reconcile
}

fn show_save_recovery_reconciliation_contents(
    ui: &mut egui::Ui,
    recovery: &SaveRecovery,
) -> (bool, bool) {
    ui.set_min_width(460.0);
    ui.set_max_width(600.0);
    ui.heading("Reconcile uncertain save");
    ui.strong(format!("Destination: {}", recovery.destination_label));
    show_save_recovery_copy_action(ui, recovery);
    egui::ScrollArea::vertical()
        .max_height(120.0)
        .show(ui, |ui| {
            ui.colored_label(ui.visuals().error_fg_color, &recovery.message);
        });
    ui.label(
        "Continue only after you inspected the destination and retained private sibling, preserved the version you need, and determined whether the earlier save committed.",
    );
    ui.label(
        "Confirming removes this safety record. It does not write, retry, or change the document.",
    );
    ui.separator();
    let mut cancel = false;
    let mut confirm = false;
    ui.horizontal(|ui| {
        cancel = ui.button("Cancel").clicked();
        confirm = ui.button("I Have Reconciled This Outcome").clicked();
    });
    (confirm, cancel)
}

fn show_save_recovery_copy_action(ui: &mut egui::Ui, recovery: &SaveRecovery) {
    let path_is_unicode = recovery.destination.to_str().is_some();
    if !path_is_unicode {
        ui.label(
            "This operating-system path is not valid Unicode. Copy uses a reversible hexadecimal representation instead of changing the path.",
        );
    }
    let button_label = if path_is_unicode {
        "Copy Destination Path"
    } else {
        "Copy Exact Path Encoding"
    };
    if ui.button(button_label).clicked() {
        ui.ctx()
            .copy_text(recovery_path_clipboard_text(&recovery.destination));
    }
}

fn recovery_path_clipboard_text(path: &Path) -> String {
    if let Some(path) = path.to_str() {
        return path.to_owned();
    }

    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        hex_encoded_path("unix-path-bytes:", path.as_os_str().as_bytes())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        const HEX: &[u8; 16] = b"0123456789abcdef";
        let units = path.as_os_str().encode_wide();
        let unit_count = units.clone().count();
        let mut output = String::new();
        output
            .try_reserve_exact(
                "windows-path-utf16:"
                    .len()
                    .saturating_add(unit_count.saturating_mul(4)),
            )
            .expect("bounded recovery paths fit the clipboard representation");
        output.push_str("windows-path-utf16:");
        for unit in units {
            for shift in [12, 8, 4, 0] {
                output.push(char::from(HEX[usize::from((unit >> shift) & 0x0f)]));
            }
        }
        output
    }
    #[cfg(not(any(unix, windows)))]
    {
        hex_encoded_path(
            "platform-path-encoding:",
            path.as_os_str().as_encoded_bytes(),
        )
    }
}

const fn document_input_event_survives_modal_transition(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Cut
            | egui::Event::Paste(_)
            | egui::Event::Text(_)
            | egui::Event::Key { pressed: true, .. }
            | egui::Event::Ime(_)
    )
}

#[cfg(not(windows))]
fn hex_encoded_path(prefix: &str, bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::new();
    output
        .try_reserve_exact(prefix.len().saturating_add(bytes.len().saturating_mul(2)))
        .expect("bounded recovery paths fit the clipboard representation");
    output.push_str(prefix);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn persistence_status_label(document: &Document, external_memory_at_risk: bool) -> &'static str {
    if document.is_dirty() || external_memory_at_risk {
        "Modified"
    } else if document.path().is_none() {
        "Unsaved"
    } else {
        "Saved"
    }
}

fn preferred_view_for_path(path: &std::path::Path) -> DocumentView {
    let is_markdown = path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("md") || extension.eq_ignore_ascii_case("markdown")
    });
    if is_markdown {
        DocumentView::Markdown
    } else {
        DocumentView::Text
    }
}

const fn pending_abandon_prompt(action: PendingAbandonAction) -> &'static str {
    match action {
        PendingAbandonAction::New => "Save your changes before creating a new document?",
        PendingAbandonAction::Open => "Save your changes before opening another file?",
        PendingAbandonAction::Reload => "Save your changes before reloading from disk?",
        PendingAbandonAction::Quit => "Save your changes before closing Noter?",
    }
}

fn edit_timestamp(ui: &egui::Ui) -> EditTimestamp {
    let seconds = ui.input(|input| input.time);
    let elapsed = Duration::try_from_secs_f64(seconds).unwrap_or_default();
    EditTimestamp::new(elapsed)
}

fn direct_input_origin(ui: &egui::Ui, fallback: EditOrigin) -> EditOrigin {
    ui.input(|input| {
        if input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Paste(_)))
        {
            EditOrigin::Paste
        } else {
            fallback
        }
    })
}

fn char_index_to_byte(source: &str, character: usize) -> usize {
    source
        .char_indices()
        .nth(character)
        .map_or(source.len(), |(offset, _)| offset)
}

fn byte_at_or_before(source: &str, position: usize) -> usize {
    let mut position = position.min(source.len());
    while !source.is_char_boundary(position) {
        position = position.saturating_sub(1);
    }
    position
}

fn caret_line_column(source: &str, position: usize) -> (usize, usize) {
    let position = byte_at_or_before(source, position);
    let mut line = 1_usize;
    let mut column = 1_usize;
    let mut characters = source[..position].chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                line = line.saturating_add(1);
                column = 1;
            }
            '\n' => {
                line = line.saturating_add(1);
                column = 1;
            }
            _ => column = column.saturating_add(1),
        }
    }
    (line, column)
}

fn selected_character_count(source: &str, selection: Selection) -> usize {
    let range = selection.ordered_range();
    source
        .get(range.start()..range.end())
        .map_or(0, |selected| selected.chars().count())
}

fn projected_replacement_length(
    source_len: usize,
    range: TextRange,
    replacement_len: usize,
) -> Option<usize> {
    let removed = range.end().checked_sub(range.start())?;
    source_len
        .checked_sub(removed)?
        .checked_add(replacement_len)
}

fn selection_from_cursor_range(source: &str, range: egui::text::CCursorRange) -> Selection {
    Selection::new(
        char_index_to_byte(source, range.secondary.index.into()),
        char_index_to_byte(source, range.primary.index.into()),
    )
}

fn cursor_range_from_selection(
    source: &str,
    selection: Selection,
) -> Option<egui::text::CCursorRange> {
    if !selection_is_valid(source, selection) {
        return None;
    }
    let anchor = source[..selection.anchor()].chars().count();
    let active = source[..selection.active()].chars().count();
    Some(egui::text::CCursorRange::two(
        egui::text::CCursor::new(anchor),
        egui::text::CCursor::new(active),
    ))
}

const fn valid_selection_or_end(source: &str, selection: Selection) -> Selection {
    if selection_is_valid(source, selection) {
        selection
    } else {
        Selection::caret(source.len())
    }
}

const fn selection_is_valid(source: &str, selection: Selection) -> bool {
    selection.anchor() <= source.len()
        && selection.active() <= source.len()
        && source.is_char_boundary(selection.anchor())
        && source.is_char_boundary(selection.active())
}

fn markdown_limit_message(byte_len: usize, limit: MarkdownProjectionLimit) -> String {
    format!(
        "Markdown Mode is unavailable because the current bounded renderer would exceed its {}. This {byte_len}-byte source remains fully available in Text Mode and can be edited there.",
        limit.description()
    )
}

#[cfg(feature = "screenshot-qa")]
struct ScreenshotCapture {
    path: PathBuf,
    frames_remaining: u8,
    requested: bool,
}

#[cfg(feature = "screenshot-qa")]
impl ScreenshotCapture {
    const MARKER: &str = "noter-readme-screenshot";

    const fn new(path: PathBuf) -> Self {
        Self {
            path,
            frames_remaining: 5,
            requested: false,
        }
    }
}

#[cfg(feature = "screenshot-qa")]
fn write_screenshot_png(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    const EXPORT_WIDTH: u32 = 1200;
    const EXPORT_HEIGHT: u32 = 760;

    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let width = u32::try_from(image.size[0])
        .map_err(|_| std::io::Error::other("screenshot width exceeds PNG limits"))?;
    let height = u32::try_from(image.size[1])
        .map_err(|_| std::io::Error::other("screenshot height exceeds PNG limits"))?;
    let bytes = image
        .pixels
        .iter()
        .flat_map(egui::Color32::to_srgba_unmultiplied)
        .collect::<Vec<_>>();
    let captured = image::RgbaImage::from_raw(width, height, bytes)
        .ok_or_else(|| std::io::Error::other("screenshot pixel count is inconsistent"))?;
    let exported = image::imageops::resize(
        &captured,
        EXPORT_WIDTH,
        EXPORT_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );
    exported
        .save_with_format(path, image::ImageFormat::Png)
        .map_err(std::io::Error::other)?;
    Ok(())
}

fn append_cleanup_error(
    message: &mut String,
    cleanup_error: Option<&noter::core::save::StorageError>,
) {
    if let Some(cleanup_error) = cleanup_error {
        message.push_str(" Cleanup also failed and a private artifact may remain: ");
        message.push_str(&cleanup_error.to_string());
    }
}

const fn dialog_remains_open(window_open: bool, close_clicked: bool) -> bool {
    window_open && !close_clicked
}

impl eframe::App for NoterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.idle_screen.show(ui.ctx(), self.theme.idle_effect()) {
            self.render_frame(ui);
        }
        theme::paint_crt_overlay(ui.ctx(), self.theme);
        #[cfg(feature = "screenshot-qa")]
        self.advance_screenshot_capture(ui.ctx());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(THEME_STORAGE_KEY, self.theme.storage_value().to_owned());
        storage.set_string(
            WORD_WRAP_STORAGE_KEY,
            self.text_wrap.storage_value().to_owned(),
        );
        storage.set_string(ZOOM_STORAGE_KEY, self.editor_zoom.storage_value());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;

    use super::*;
    use noter::core::conflict::ExternalChangeKind;
    use tempfile::tempdir;

    #[test]
    fn default_test_apps_use_independent_recovery_storage() {
        let first = NoterApp::default();
        let second = NoterApp::default();

        assert_eq!(
            first.crash_recovery.recovery_root_for_test(),
            first
                .test_recovery_root
                .as_ref()
                .map(tempfile::TempDir::path)
        );
        assert_eq!(
            second.crash_recovery.recovery_root_for_test(),
            second
                .test_recovery_root
                .as_ref()
                .map(tempfile::TempDir::path)
        );
        assert_ne!(
            first
                .test_recovery_root
                .as_ref()
                .map(tempfile::TempDir::path),
            second
                .test_recovery_root
                .as_ref()
                .map(tempfile::TempDir::path)
        );
        assert!(!first.crash_recovery.is_unavailable());
        assert!(!second.crash_recovery.is_unavailable());
    }

    fn collect_text_shapes(shape: &egui::Shape, text: &mut Vec<(String, egui::Pos2)>) {
        match shape {
            egui::Shape::Text(text_shape) => {
                text.push((text_shape.galley.job.text.clone(), text_shape.pos));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_text_shapes(shape, text);
                }
            }
            _ => {}
        }
    }

    fn rendered_text(output: &egui::FullOutput) -> Vec<(String, egui::Pos2)> {
        let mut text = Vec::new();
        for shape in &output.shapes {
            collect_text_shapes(&shape.shape, &mut text);
        }
        text
    }

    fn text_shape_font_size(shape: &egui::Shape, label: &str) -> Option<f32> {
        match shape {
            egui::Shape::Text(text_shape) if text_shape.galley.job.text == label => text_shape
                .galley
                .job
                .sections
                .first()
                .map(|section| section.format.font_id.size),
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|shape| text_shape_font_size(shape, label)),
            _ => None,
        }
    }

    fn rendered_font_size(output: &egui::FullOutput, label: &str) -> f32 {
        output
            .shapes
            .iter()
            .find_map(|shape| text_shape_font_size(&shape.shape, label))
            .unwrap_or_else(|| panic!("expected rendered text `{label}` with a font section"))
    }

    fn text_shape_color(shape: &egui::Shape, label: &str) -> Option<egui::Color32> {
        match shape {
            egui::Shape::Text(text_shape) if text_shape.galley.job.text == label => text_shape
                .galley
                .job
                .sections
                .first()
                .map(|section| section.format.color),
            egui::Shape::Vec(shapes) => shapes
                .iter()
                .find_map(|shape| text_shape_color(shape, label)),
            _ => None,
        }
    }

    fn accesskit_labels(output: &egui::FullOutput) -> Vec<String> {
        fn visit(
            id: egui::accesskit::NodeId,
            nodes: &HashMap<egui::accesskit::NodeId, &egui::accesskit::Node>,
            labels: &mut Vec<String>,
        ) {
            let node = nodes
                .get(&id)
                .unwrap_or_else(|| panic!("missing AccessKit node {id:?}"));
            if let Some(label) = node.label() {
                labels.push(label.to_owned());
            }
            for child in node.children() {
                visit(*child, nodes, labels);
            }
        }

        let update = output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled");
        let root = update
            .tree
            .as_ref()
            .expect("the first AccessKit update must include a tree")
            .root;
        let nodes = update
            .nodes
            .iter()
            .map(|(id, node)| (*id, node))
            .collect::<HashMap<_, _>>();
        let mut labels = Vec::new();
        visit(root, &nodes, &mut labels);
        labels
    }

    fn accesskit_bounds(output: &egui::FullOutput, label: &str) -> egui::accesskit::Rect {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled")
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
            .unwrap_or_else(|| panic!("expected an AccessKit node labeled `{label}` with bounds"))
    }

    fn accesskit_value(output: &egui::FullOutput, label: &str) -> String {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled")
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.value())
                    .flatten()
            })
            .unwrap_or_else(|| panic!("expected an AccessKit node labeled `{label}` with a value"))
            .to_owned()
    }

    fn accesskit_node_id(output: &egui::FullOutput, label: &str) -> egui::accesskit::NodeId {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled")
            .nodes
            .iter()
            .find_map(|(id, node)| (node.label() == Some(label)).then_some(*id))
            .unwrap_or_else(|| panic!("expected an AccessKit node labeled `{label}`"))
    }

    fn accesskit_disabled(output: &egui::FullOutput, label: &str) -> bool {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled")
            .nodes
            .iter()
            .find_map(|(_, node)| (node.label() == Some(label)).then(|| node.is_disabled()))
            .unwrap_or_else(|| panic!("expected an AccessKit node labeled `{label}`"))
    }

    fn text_position(text: &[(String, egui::Pos2)], label: &str) -> egui::Pos2 {
        text.iter()
            .find_map(|(candidate, position)| (candidate == label).then_some(*position))
            .unwrap_or_else(|| {
                let rendered = text
                    .iter()
                    .map(|(candidate, _)| candidate.as_str())
                    .collect::<Vec<_>>();
                panic!("expected the UI to render `{label}` among {rendered:?}")
            })
    }

    fn ui_input(width: f32, height: f32, time: f64) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, height),
            )),
            time: Some(time),
            ..Default::default()
        }
    }

    fn ime_preedit_input(time: f64, text: &str) -> egui::RawInput {
        let mut input = ui_input(800.0, 600.0, time);
        input.events.push(egui::Event::Ime(egui::ImeEvent::Preedit {
            text: text.to_owned(),
            active_range_chars: None,
        }));
        input
    }

    fn ime_commit_input(time: f64, text: &str) -> egui::RawInput {
        let mut input = ui_input(800.0, 600.0, time);
        input
            .events
            .push(egui::Event::Ime(egui::ImeEvent::Commit(text.to_owned())));
        input
    }

    fn click_input(width: f32, height: f32, time: f64, position: egui::Pos2) -> egui::RawInput {
        let mut input = ui_input(width, height, time);
        input.events = vec![
            egui::Event::PointerMoved(position),
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        input
    }

    fn key_press(modifiers: egui::Modifiers, key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn shortcut_input(modifiers: egui::Modifiers, key: egui::Key) -> egui::RawInput {
        egui::RawInput {
            events: vec![key_press(modifiers, key)],
            ..Default::default()
        }
    }

    fn repeated_shortcut_input(modifiers: egui::Modifiers, key: egui::Key) -> egui::RawInput {
        let event = key_press(modifiers, key);
        egui::RawInput {
            events: vec![event.clone(), event],
            ..Default::default()
        }
    }

    fn collect_shortcut_from_input(input: egui::RawInput) -> Option<FileCommand> {
        match collect_input_shortcut_from_input(input, egui::os::OperatingSystem::Nix, true, true) {
            Some(InputShortcut::File(command)) => Some(command),
            _ => None,
        }
    }

    fn collect_edit_shortcut_from_input(input: egui::RawInput) -> Option<EditCommand> {
        collect_edit_shortcut_from_input_with_availability(input, true, true)
    }

    fn collect_edit_shortcut_from_input_for_os(
        input: egui::RawInput,
        operating_system: egui::os::OperatingSystem,
    ) -> Option<EditCommand> {
        match collect_input_shortcut_from_input(input, operating_system, true, true) {
            Some(InputShortcut::Edit(command)) => Some(command),
            _ => None,
        }
    }

    fn collect_edit_shortcut_from_input_with_document_focus(
        input: egui::RawInput,
        document_shortcuts_enabled: bool,
    ) -> Option<EditCommand> {
        collect_edit_shortcut_from_input_with_availability(
            input,
            document_shortcuts_enabled,
            document_shortcuts_enabled,
        )
    }

    fn collect_edit_shortcut_from_input_with_availability(
        input: egui::RawInput,
        document_shortcuts_enabled: bool,
        go_to_line_shortcut_enabled: bool,
    ) -> Option<EditCommand> {
        match collect_input_shortcut_from_input(
            input,
            egui::os::OperatingSystem::Nix,
            document_shortcuts_enabled,
            go_to_line_shortcut_enabled,
        ) {
            Some(InputShortcut::Edit(command)) => Some(command),
            _ => None,
        }
    }

    fn collect_view_shortcut_from_input(input: egui::RawInput) -> Option<ViewCommandRequest> {
        collect_view_shortcut_from_input_with_availability(input, true)
    }

    fn collect_view_shortcut_from_input_with_availability(
        input: egui::RawInput,
        document_shortcuts_enabled: bool,
    ) -> Option<ViewCommandRequest> {
        match collect_input_shortcut_from_input(
            input,
            egui::os::OperatingSystem::Nix,
            document_shortcuts_enabled,
            document_shortcuts_enabled,
        ) {
            Some(InputShortcut::View(command)) => Some(command),
            _ => None,
        }
    }

    fn collect_input_shortcut_from_input(
        input: egui::RawInput,
        operating_system: egui::os::OperatingSystem,
        document_shortcuts_enabled: bool,
        go_to_line_shortcut_enabled: bool,
    ) -> Option<InputShortcut> {
        let context = egui::Context::default();
        context.set_os(operating_system);
        let mut app = NoterApp::default();
        let mut command = None;
        let _ = context.run_ui(input, |ui| {
            command = app.collect_input_shortcut(
                ui,
                document_shortcuts_enabled,
                go_to_line_shortcut_enabled,
            );
        });
        command
    }

    fn direct_origin_from_input(input: egui::RawInput, fallback: EditOrigin) -> EditOrigin {
        let context = egui::Context::default();
        let mut origin = fallback;
        let _ = context.run_ui(input, |ui| origin = direct_input_origin(ui, fallback));
        origin
    }

    fn focus_empty_replacement_field(app: &mut NoterApp, context: &egui::Context, time: f64) {
        app.find_bar.open(true, &app.text, app.selection);
        app.find_bar.set_replacement_for_test(String::new());
        let _ = context.run_ui(ui_input(1_200.0, 760.0, time), |ui| {
            app.render_frame(ui);
        });
        let replacement_id = egui::Id::new("noter-find-replacement");
        context.memory_mut(|memory| memory.request_focus(replacement_id));
        let _ = context.run_ui(ui_input(1_200.0, 760.0, time + 0.01), |ui| {
            app.render_frame(ui);
        });
        assert!(context.memory(|memory| memory.has_focus(replacement_id)));
    }

    fn show_menu_frame(
        app: &mut NoterApp,
        context: &egui::Context,
        input: egui::RawInput,
    ) -> egui::FullOutput {
        context.run_ui(input, |ui| {
            let mut file_command = None;
            let mut edit_command = None;
            let mut view_command = None;
            app.show_menu(ui, &mut file_command, &mut edit_command, &mut view_command);
            if let Some(command) = view_command {
                app.execute_view_command(command, context);
            }
            app.apply_pending_document_view();
        })
    }

    fn click_menu_label(
        app: &mut NoterApp,
        context: &egui::Context,
        output: &egui::FullOutput,
        label: &str,
        viewport: egui::Vec2,
        time: f64,
    ) -> egui::FullOutput {
        let position = text_position(&rendered_text(output), label) + egui::vec2(4.0, 4.0);
        show_menu_frame(
            app,
            context,
            click_input(viewport.x, viewport.y, time, position),
        )
    }

    fn hover_menu_label(
        app: &mut NoterApp,
        context: &egui::Context,
        output: &egui::FullOutput,
        label: &str,
        viewport: egui::Vec2,
        time: f64,
    ) -> egui::FullOutput {
        let position = text_position(&rendered_text(output), label) + egui::vec2(4.0, 4.0);
        let mut input = ui_input(viewport.x, viewport.y, time);
        input.events.push(egui::Event::PointerMoved(position));
        show_menu_frame(app, context, input)
    }

    fn record_test_editor_change(app: &mut NoterApp, origin: EditOrigin) {
        let selection = Selection::caret(app.text.len());
        app.record_editor_change(EditorFrameOutcome {
            changed: true,
            selection,
            origin,
            observed_at: EditTimestamp::default(),
        });
    }

    fn active_test_recovery(path: PathBuf, message: &str) -> SaveRecovery {
        let destination_label = bounded_destination_label(&path)
            .expect("the bounded test destination should be representable");
        SaveRecovery {
            destination: path,
            destination_label,
            message: message.to_owned(),
            notice_pending: true,
        }
    }

    fn test_save_recovery_reservation(
        app: &mut NoterApp,
        attempt: SaveAttempt,
    ) -> SaveRecoveryReservation {
        app.reserve_save_recovery_slot(attempt)
            .expect("the test save should reserve a recovery slot")
    }

    fn record_test_unknown_save(app: &mut NoterApp, attempt: SaveAttempt, label: &str) {
        use noter::core::revision::Revision;
        use noter::core::save::{SaveStage, StorageError};

        let reservation = test_save_recovery_reservation(app, attempt);
        app.handle_save_result(
            Ok(SaveOutcome::CommitStateUnknown {
                revision: Revision::INITIAL,
                error: StorageError::new(
                    SaveStage::Reconcile,
                    format!("{label} destination state differs"),
                ),
                recovery_artifact: StorageError::new(
                    SaveStage::Cleanup,
                    format!("inspect `{label}.noter-save-recovery.tmp` before retrying"),
                ),
            }),
            reservation,
        );
    }

    fn arrange_pending_intent(app: &mut NoterApp, intent: PendingAbandonAction) {
        assert_eq!(
            app.lifecycle.reduce(LifecycleCommand::Request {
                intent,
                document_dirty: true,
                revision: app.document.revision(),
            }),
            LifecycleEffect::PromptDirty(intent)
        );
    }

    fn inspect_external_change_for_test(app: &mut NoterApp, context: &egui::Context) {
        let mut focused = egui::RawInput::default();
        focused.events.push(egui::Event::WindowFocused(true));
        let _ = context.run_ui(focused, |ui| {
            app.maybe_inspect_external_change(ui.ctx());
        });
    }

    fn request_external_reload_for_test(app: &mut NoterApp, context: &egui::Context) {
        let effect = app
            .conflict
            .reduce(ConflictCommand::Decide(ConflictDecision::ReloadDisk));
        app.apply_conflict_effect(effect, context);
    }

    fn authorize_dirty_close(app: &mut NoterApp) {
        arrange_pending_intent(app, PendingAbandonAction::Quit);
        assert_eq!(
            app.lifecycle
                .reduce(LifecycleCommand::Decide(DirtyDecision::Discard)),
            LifecycleEffect::Continue(PendingAbandonAction::Quit)
        );
    }

    fn arrange_saving_intent(app: &mut NoterApp, intent: PendingAbandonAction) {
        arrange_pending_intent(app, intent);
        assert_eq!(
            app.lifecycle
                .reduce(LifecycleCommand::Decide(DirtyDecision::Save)),
            LifecycleEffect::StartSave
        );
    }

    fn app_with_dismissed_uncertain_save() -> NoterApp {
        use noter::core::revision::Revision;
        use noter::core::save::{SaveStage, StorageError};

        let mut app = NoterApp::default();
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        let reservation = test_save_recovery_reservation(
            &mut app,
            SaveAttempt::SaveAs(PathBuf::from("uncertain-save.txt")),
        );
        app.handle_save_result(
            Ok(SaveOutcome::CommitStateUnknown {
                revision: Revision::INITIAL,
                error: StorageError::new(SaveStage::Reconcile, "destination state differs"),
                recovery_artifact: StorageError::new(
                    SaveStage::Cleanup,
                    "inspect `.noter-save-recovery.tmp` before retrying",
                ),
            }),
            reservation,
        );
        app.error_msg = None;
        for recovery in &mut app.save_recoveries {
            recovery.notice_pending = false;
        }
        app
    }

    fn viewport_titles(output: &egui::FullOutput) -> Vec<String> {
        output
            .viewport_output
            .values()
            .flat_map(|viewport| viewport.commands.iter())
            .filter_map(|command| match command {
                egui::ViewportCommand::Title(title) => Some(title.clone()),
                _ => None,
            })
            .collect()
    }

    fn text_bounds(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == label => {
                Some(text.visual_bounding_rect())
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_bounds(shape, label)),
            _ => None,
        }
    }

    fn rendered_bounds(output: &egui::FullOutput, label: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|shape| text_bounds(&shape.shape, label))
            .unwrap_or_else(|| panic!("expected the UI to render `{label}`"))
    }

    #[test]
    fn menu_names_stay_separated_and_inside_the_narrowest_expanded_bar() {
        // The expanded bar is tightest at its own threshold width, where the
        // reserved Mode and Theme controls leave the least room for names.
        let context = egui::Context::default();
        crate::theme::configure_styles(&context);
        let names = ["File", "Edit", "View", "Help"];
        let mut expanded_widths = 0_usize;

        for width in [
            EXPANDED_TOP_CONTROLS_MIN_WIDTH,
            EXPANDED_TOP_CONTROLS_MIN_WIDTH + 40.0,
            900.0,
            1_400.0,
        ] {
            let mut app = NoterApp::default();
            let output = context.run_ui(ui_input(width, 300.0, 0.0), |ui| {
                let (mut file, mut edit, mut view) = (None, None, None);
                app.show_menu(ui, &mut file, &mut edit, &mut view);
            });
            if output
                .shapes
                .iter()
                .all(|shape| text_bounds(&shape.shape, "Edit").is_none())
            {
                // Still the compact bar at this width; it shows File and More.
                continue;
            }
            expanded_widths += 1;

            let bounds = names.map(|name| rendered_bounds(&output, name));
            let controls_start = rendered_bounds(&output, "Mode").left();
            for (name, rect) in names.iter().zip(bounds) {
                assert!(
                    rect.right() < controls_start,
                    "`{name}` reaches {} and collides with the controls at {controls_start}",
                    rect.right()
                );
            }
            for pair in bounds.windows(2) {
                assert!(
                    pair[1].left() - pair[0].right() >= MENU_ITEM_SPACING,
                    "menu names must keep at least {MENU_ITEM_SPACING} points of air apart"
                );
            }
        }

        assert!(
            expanded_widths > 0,
            "at least one sampled width must show the expanded menu bar"
        );
    }

    #[test]
    fn both_document_mode_segments_share_one_width() {
        let context = egui::Context::default();
        crate::theme::configure_styles(&context);
        let mut widths = Vec::new();
        for view in [DocumentView::Text, DocumentView::Markdown] {
            let mut app = NoterApp {
                view,
                ..NoterApp::default()
            };
            let output = context.run_ui(ui_input(900.0, 300.0, 0.0), |ui| {
                let (mut file, mut edit, mut view_command) = (None, None, None);
                app.show_menu(ui, &mut file, &mut edit, &mut view_command);
            });
            widths.push((
                rendered_bounds(&output, "Text"),
                rendered_bounds(&output, "Markdown"),
            ));
        }

        // The selected segment is the one that paints a background, so equal
        // label centers in both modes prove the pair does not resize.
        let (text_in_text_mode, markdown_in_text_mode) = widths[0];
        let (text_in_markdown_mode, markdown_in_markdown_mode) = widths[1];
        assert_eq!(text_in_text_mode.center(), text_in_markdown_mode.center());
        assert_eq!(
            markdown_in_text_mode.center(),
            markdown_in_markdown_mode.center()
        );
        // Equal centers alone would also hold if both segments grew with their
        // labels, so pin the step to the shared width. Glyph bearings move the
        // measured centers by a fraction of a point.
        let step = markdown_in_text_mode.center().x - text_in_text_mode.center().x;
        let expected_step = DOCUMENT_VIEW_BUTTON_WIDTH + TOP_CONTROL_SPACING;
        assert!(
            (step - expected_step).abs() < 1.0,
            "Mode segments stepped {step} apart, expected about {expected_step}; \
             a label wider than DOCUMENT_VIEW_BUTTON_WIDTH would resize its segment"
        );
    }

    #[test]
    fn an_unchanged_window_title_is_never_resent() {
        // Every viewport command requests a repaint, so a title resent each
        // frame would keep an idle window painting at full speed.
        let mut app = NoterApp::default();
        let context = egui::Context::default();

        let first = context.run_ui(ui_input(600.0, 400.0, 0.0), |ui| app.update_title(ui.ctx()));
        let repeat = context.run_ui(ui_input(600.0, 400.0, 0.1), |ui| app.update_title(ui.ctx()));
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        let edited = context.run_ui(ui_input(600.0, 400.0, 0.2), |ui| app.update_title(ui.ctx()));

        assert_eq!(viewport_titles(&first), ["Untitled - Noter"]);
        assert!(viewport_titles(&repeat).is_empty());
        assert_eq!(viewport_titles(&edited), ["Untitled* - Noter"]);
    }

    #[test]
    fn untitled_first_frame_focuses_the_editor() {
        let mut app = NoterApp::default();
        app.request_untitled_editor_focus();
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.render_frame(ui));

        assert!(context.memory(|memory| memory.has_focus(app.editor_id())));
        assert_eq!(app.pending_selection_restore, None);
    }

    #[test]
    fn untitled_first_keystroke_does_not_require_a_pointer_click() {
        let mut app = NoterApp::default();
        app.request_untitled_editor_focus();
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.render_frame(ui));
        let mut typed = ui_input(800.0, 600.0, 0.05);
        typed.events.push(egui::Event::Text("hello".to_owned()));
        let _ = context.run_ui(typed, |ui| app.render_frame(ui));

        assert_eq!(app.text, "hello");
        assert!(app.document.is_dirty());
        assert_eq!(persistence_status_label(&app.document, false), "Modified");
    }

    #[test]
    fn text_input_and_custom_navigation_share_the_ordered_queue() {
        let run = |events: Vec<egui::Event>| {
            let source = "ab cd";
            let selection = Selection::caret(2);
            let mut app = NoterApp {
                text: source.to_owned(),
                document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
                selection,
                pending_selection_restore: Some(selection),
                ..NoterApp::default()
            };
            let context = egui::Context::default();
            context.set_os(egui::os::OperatingSystem::Windows);
            theme::configure_styles(&context);
            let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| {
                app.render_frame(ui);
            });
            let mut input = ui_input(800.0, 600.0, 0.1);
            input.events = events;
            let _ = context.run_ui(input, |ui| app.render_frame(ui));
            let _ = context.run_ui(ui_input(800.0, 600.0, 0.2), |ui| {
                app.render_frame(ui);
            });
            (app.text, app.selection)
        };
        let word_right = || key_press(egui::Modifiers::CTRL, egui::Key::ArrowRight);

        assert_eq!(
            run(vec![egui::Event::Text(" ".to_owned()), word_right()]),
            ("ab  cd".to_owned(), Selection::caret(4))
        );
        assert_eq!(
            run(vec![word_right(), egui::Event::Text("x".to_owned())]),
            ("ab xcd".to_owned(), Selection::caret(4))
        );
    }

    #[test]
    fn navigation_text_and_save_keep_their_original_order() -> std::io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("ordered-save.txt");
        fs::write(&path, "ab cd")?;
        let selection = Selection::caret(2);
        let mut app = NoterApp {
            text: "ab cd".to_owned(),
            document: Document::from_path(&path).expect("fixture should load"),
            selection,
            pending_selection_restore: Some(selection),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        context.set_os(egui::os::OperatingSystem::Windows);
        theme::configure_styles(&context);
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.render_frame(ui));

        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };
        let mut input = ui_input(800.0, 600.0, 0.1);
        input.events = vec![
            key_press(egui::Modifiers::CTRL, egui::Key::ArrowRight),
            egui::Event::Text("x".to_owned()),
            key_press(command, egui::Key::S),
        ];
        let _ = context.run_ui(input, |ui| app.render_frame(ui));
        assert_eq!(app.text, "ab cd");
        assert_eq!(fs::read_to_string(&path)?, "ab cd");

        let _ = context.run_ui(ui_input(800.0, 600.0, 0.2), |ui| app.render_frame(ui));
        assert_eq!(app.text, "ab xcd");
        assert_eq!(fs::read_to_string(&path)?, "ab cd");

        let _ = context.run_ui(ui_input(800.0, 600.0, 0.3), |ui| app.render_frame(ui));
        assert_eq!(fs::read_to_string(path)?, "ab xcd");
        assert!(!app.document.is_dirty());
        Ok(())
    }

    #[test]
    fn navigation_text_and_mode_keep_their_original_order() {
        let selection = Selection::caret(2);
        let mut app = NoterApp {
            text: "ab cd".to_owned(),
            document: Document::from_bytes(b"ab cd").expect("fixture should load"),
            selection,
            pending_selection_restore: Some(selection),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        context.set_os(egui::os::OperatingSystem::Windows);
        theme::configure_styles(&context);
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.render_frame(ui));

        let mode = egui::Modifiers {
            ctrl: true,
            command: true,
            shift: true,
            ..egui::Modifiers::NONE
        };
        let mut input = ui_input(800.0, 600.0, 0.1);
        input.events = vec![
            key_press(egui::Modifiers::CTRL, egui::Key::ArrowRight),
            egui::Event::Text("x".to_owned()),
            key_press(mode, egui::Key::M),
        ];
        let _ = context.run_ui(input, |ui| app.render_frame(ui));
        for time in [0.2, 0.3] {
            let _ = context.run_ui(ui_input(800.0, 600.0, time), |ui| app.render_frame(ui));
        }

        assert_eq!(app.text, "ab xcd");
        assert_eq!(app.view, DocumentView::Markdown);
        assert!(app.deferred_input_events.is_empty());
    }

    #[test]
    fn persistence_status_distinguishes_untitled_from_saved() {
        assert_eq!(persistence_status_label(&Document::new(), false), "Unsaved");

        let mut dirty = Document::new();
        dirty
            .replace_text("draft")
            .expect("fixture edit should advance the revision");
        assert_eq!(persistence_status_label(&dirty, false), "Modified");

        let directory = tempdir().expect("tempdir");
        let path = directory.path().join("notes.txt");
        fs::write(&path, "hello").expect("fixture file");
        let saved = Document::from_path(&path).expect("open fixture");
        assert_eq!(persistence_status_label(&saved, false), "Saved");
    }

    #[test]
    fn launching_with_a_path_defers_recovery_offers_instead_of_discarding_them() {
        use crate::crash_recovery::CrashRecoverySession;
        use noter::core::recovery::{
            RecoveryDocumentId, RecoveryInstanceId, RecoverySnapshot, RecoverySnapshotParts,
            RecoveryWallTime,
        };
        use noter::core::recovery_store::{RecoveryScanDisposition, RecoveryStore};
        use noter::core::revision::Revision;
        use noter::core::text_format::{Bom, Encoding};

        let directory = tempdir().expect("tempdir");
        let store = RecoveryStore::open(directory.path()).expect("store");
        let snapshot = RecoverySnapshot::try_new(RecoverySnapshotParts {
            document_id: RecoveryDocumentId::new([1; 16]),
            instance_id: RecoveryInstanceId::new([7; 16]),
            revision: Revision::new(1),
            created_at: RecoveryWallTime::from_unix_millis(1),
            updated_at: RecoveryWallTime::from_unix_millis(2),
            original_path: b"notes.md".to_vec(),
            bom: Bom::Absent,
            encoding: Encoding::Utf8,
            selection: Selection::caret(0),
            content: b"keep me".to_vec(),
        })
        .expect("snapshot");
        store.persist(&snapshot).expect("persist");

        let mut app = NoterApp {
            crash_recovery: CrashRecoverySession::open_at(directory.path()),
            ..NoterApp::default()
        };
        assert!(app.crash_recovery.active_offer().is_some());
        app.crash_recovery.defer_startup_offers();

        assert!(app.crash_recovery.active_offer().is_none());
        let entries = store.scan_startup().expect("rescan");
        assert_eq!(entries.len(), 1);
        match entries[0].disposition() {
            RecoveryScanDisposition::Offer(offer) => {
                let record = store.load_record(offer.primary()).expect("load offer");
                assert_eq!(record.content(), b"keep me");
            }
            RecoveryScanDisposition::Quarantine(_) => {
                panic!("an explicit open must not delete a valid recovery record")
            }
        }
    }

    #[test]
    fn an_update_session_names_its_window_until_the_status_closes() {
        let mut app = NoterApp {
            updates: UpdateStatusState::OpenedByLaunch,
            ..NoterApp::default()
        };
        assert_eq!(app.window_title(), "Update status - Noter");

        let context = egui::Context::default();
        // The floating status window settles its layout on the second pass.
        let _ = context.run_ui(ui_input(600.0, 400.0, 0.0), |ui| app.show_updates(ui.ctx()));
        let opened = context.run_ui(ui_input(600.0, 400.0, 0.05), |ui| {
            app.show_updates(ui.ctx());
        });
        let close = text_position(&rendered_text(&opened), "Close") + egui::vec2(4.0, 4.0);
        let _ = context.run_ui(click_input(600.0, 400.0, 0.1, close), |ui| {
            app.show_updates(ui.ctx());
        });

        assert_eq!(app.updates, UpdateStatusState::Closed);
        assert_eq!(app.window_title(), "Untitled - Noter");
    }

    #[test]
    fn about_action_opens_and_renders_the_window() {
        let mut app = NoterApp::default();
        app.open_about();

        assert!(app.about_open);
        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| app.show_about(ui.ctx()));
        assert!(!output.shapes.is_empty());
        assert_eq!(
            ABOUT_SUMMARY,
            "A focused editor for plain text and Markdown files."
        );
        assert_eq!(
            ABOUT_MARKDOWN_STATUS,
            "Markdown Mode provides a formatted, direct editing surface while keeping ordinary Markdown source authoritative on disk."
        );
        assert_eq!(
            ABOUT_PRIVACY,
            "Noter has no accounts, telemetry, or background network activity."
        );
        assert_eq!(
            ABOUT_LINK_BEHAVIOR,
            "The project link opens in your default browser."
        );
        assert_eq!(
            env!("CARGO_PKG_REPOSITORY"),
            "https://github.com/blisspixel/noter"
        );
        assert!(app.about_open);
    }

    #[test]
    fn update_action_opens_and_renders_the_release_status() {
        let mut app = NoterApp::default();
        app.open_updates();

        assert!(app.updates.is_open());
        let context = egui::Context::default();
        context.enable_accesskit();
        let output = context.run_ui(egui::RawInput::default(), |ui| app.show_updates(ui.ctx()));
        let rendered = accesskit_labels(&output);

        for expected in ["Noter Updates", "Open Noter releases", "Close"] {
            assert!(
                rendered.iter().any(|text| text == expected),
                "expected the update dialog to render {expected:?} among {rendered:?}"
            );
        }
        assert_eq!(
            UPDATE_STATUS,
            "Noter does not check for updates in the background. Open the releases page to compare this version with published builds."
        );
        assert!(app.updates.is_open());
    }

    #[test]
    fn identity_rotation_failure_surfaces_recovery_unavailability() {
        let directory = tempdir().expect("tempdir");
        let blocked_root = directory.path().join("blocked-recovery-root");
        std::fs::write(&blocked_root, b"not a directory").expect("blocking file");
        let mut app = NoterApp {
            crash_recovery: CrashRecoverySession::open_at(blocked_root),
            ..NoterApp::default()
        };
        assert!(app.crash_recovery.is_unavailable());
        app.error_msg = None;

        app.begin_fresh_recovery_identity();

        assert_eq!(app.error_msg.as_deref(), Some(RECOVERY_UNAVAILABLE_MESSAGE));
    }

    #[test]
    fn help_menu_actions_open_their_dialogs_from_pointer_clicks() {
        let mut app = NoterApp::default();
        let context = egui::Context::default();
        let initial = context.run_ui(ui_input(600.0, 300.0, 0.0), |ui| app.show_help_menu(ui));
        let text = rendered_text(&initial);
        let updates = text_position(&text, "Check for Updates...") + egui::vec2(4.0, 4.0);
        let about = text_position(&text, "About Noter") + egui::vec2(4.0, 4.0);

        let _ = context.run_ui(click_input(600.0, 300.0, 0.1, updates), |ui| {
            app.show_help_menu(ui);
        });
        assert!(app.updates.is_open());
        assert!(!app.about_open);

        let _ = context.run_ui(click_input(600.0, 300.0, 0.2, about), |ui| {
            app.show_help_menu(ui);
        });
        assert!(app.updates.is_open());
        assert!(app.about_open);
    }

    #[test]
    fn dialog_state_closes_for_either_native_or_button_request() {
        assert!(dialog_remains_open(true, false));
        assert!(!dialog_remains_open(false, false));
        assert!(!dialog_remains_open(true, true));
        assert!(!dialog_remains_open(false, true));
    }

    #[test]
    fn paste_events_are_explicit_and_other_direct_input_keeps_its_surface() {
        let mut paste = egui::RawInput::default();
        paste.events.push(egui::Event::Paste("content".to_owned()));
        assert_eq!(
            direct_origin_from_input(paste, EditOrigin::TextInput),
            EditOrigin::Paste
        );

        let mut text = egui::RawInput::default();
        text.events.push(egui::Event::Text("x".to_owned()));
        assert_eq!(
            direct_origin_from_input(text, EditOrigin::MarkdownInput),
            EditOrigin::MarkdownInput
        );
    }

    #[test]
    fn status_position_handles_unicode_and_every_supported_line_ending() {
        let source = "a\r\né\rb\nc";
        for (position, expected) in [
            (0, (1, 1)),
            (1, (1, 2)),
            (3, (2, 1)),
            (4, (2, 1)),
            (5, (2, 2)),
            (6, (3, 1)),
            (8, (4, 1)),
            (9, (4, 2)),
            (usize::MAX, (4, 2)),
        ] {
            assert_eq!(caret_line_column(source, position), expected);
        }
        assert_eq!(selected_character_count(source, Selection::new(8, 3)), 4);
        assert_eq!(selected_character_count(source, Selection::new(4, 5)), 0);
    }

    #[test]
    fn status_analysis_is_cached_by_document_revision_and_selection() {
        use std::cell::Cell;

        let mut app = NoterApp {
            text: "one\ntwo".to_owned(),
            selection: Selection::caret(3),
            ..NoterApp::default()
        };
        app.document
            .replace_text(&app.text)
            .expect("fixture text should become authoritative");
        let calls = Cell::new(0_usize);
        let analyze = |_: &str, _: Selection| {
            calls.set(calls.get() + 1);
            StatusSnapshot {
                line: calls.get(),
                column: 1,
                selected_characters: 0,
            }
        };

        assert_eq!(app.status_snapshot_with(analyze).line, 1);
        assert_eq!(app.status_snapshot_with(analyze).line, 1);
        assert_eq!(calls.get(), 1);

        app.selection = Selection::new(0, 3);
        assert_eq!(app.status_snapshot_with(analyze).line, 2);
        app.text.push('!');
        app.document
            .replace_text(&app.text)
            .expect("fixture edit should advance its revision");
        assert_eq!(app.status_snapshot_with(analyze).line, 3);
        app.advance_document_editor();
        assert_eq!(app.status_snapshot_with(analyze).line, 4);
        assert_eq!(calls.get(), 4);
    }

    #[test]
    fn save_as_shortcut_is_checked_before_the_less_specific_save_shortcut() {
        let modifiers = egui::Modifiers {
            ctrl: true,
            shift: true,
            command: true,
            ..egui::Modifiers::NONE
        };

        assert!(matches!(
            collect_shortcut_from_input(shortcut_input(modifiers, egui::Key::S)),
            Some(FileCommand::SaveAs)
        ));
    }

    #[test]
    fn file_shortcuts_accept_the_platform_command_modifier() {
        let modifiers = egui::Modifiers {
            mac_cmd: true,
            command: true,
            ..egui::Modifiers::NONE
        };

        assert!(matches!(
            collect_shortcut_from_input(shortcut_input(modifiers, egui::Key::S)),
            Some(FileCommand::Save)
        ));
    }

    #[test]
    fn file_shortcut_labels_are_platform_correct() {
        let shortcuts = [
            FileCommand::New,
            FileCommand::Open,
            FileCommand::Save,
            FileCommand::SaveAs,
        ]
        .map(|command| {
            command
                .shortcut()
                .expect("each tested file command should have a shortcut")
        });

        assert_eq!(
            shortcuts.map(|shortcut| shortcut.format(&egui::ModifierNames::NAMES, false)),
            ["Ctrl+N", "Ctrl+O", "Ctrl+S", "Ctrl+Shift+S"]
        );
        assert_eq!(
            shortcuts.map(|shortcut| shortcut.format(&egui::ModifierNames::NAMES, true)),
            ["Cmd+N", "Cmd+O", "Cmd+S", "Shift+Cmd+S"]
        );
    }

    #[test]
    fn rendered_file_menu_contains_every_platform_shortcut() {
        for operating_system in [
            egui::os::OperatingSystem::Windows,
            egui::os::OperatingSystem::Mac,
        ] {
            let context = egui::Context::default();
            context.set_os(operating_system);
            let mut command = None;
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(320.0);
                NoterApp::default().show_file_menu(ui, &mut command);
            });
            let labels = rendered_text(&output)
                .into_iter()
                .map(|(label, _)| label)
                .collect::<Vec<_>>();
            assert!(labels.contains(&"Reload from Disk".to_owned()));

            for file_command in [
                FileCommand::New,
                FileCommand::Open,
                FileCommand::Save,
                FileCommand::SaveAs,
            ] {
                let shortcut = file_command
                    .shortcut()
                    .expect("each rendered file command should have a shortcut");
                let expected = context.format_shortcut(&shortcut);
                assert!(
                    labels.contains(&expected),
                    "missing {expected:?} from {operating_system:?} menu labels {labels:?}"
                );
            }
            assert!(command.is_none());
        }
    }

    #[test]
    fn reload_menu_state_matches_document_path_semantics() -> Result<(), Box<dyn std::error::Error>>
    {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut command = None;
        let untitled = NoterApp::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            untitled.show_file_menu(ui, &mut command);
        });

        assert!(!untitled.file_command_enabled(FileCommand::Reload));
        assert!(accesskit_disabled(&output, "Reload from Disk"));
        assert!(command.is_none());

        let directory = tempdir()?;
        let path = directory.path().join("saved.txt");
        fs::write(&path, b"saved")?;
        let saved = NoterApp {
            text: "saved".to_owned(),
            document: Document::from_path(&path)?,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        context.enable_accesskit();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            saved.show_file_menu(ui, &mut command);
        });

        assert!(saved.file_command_enabled(FileCommand::Reload));
        assert!(!accesskit_disabled(&output, "Reload from Disk"));
        Ok(())
    }

    #[test]
    fn error_banner_uses_each_standard_themes_contrast_safe_error_color() {
        for app_theme in [AppTheme::Light, AppTheme::Dark] {
            let context = egui::Context::default();
            theme::configure_styles(&context);
            app_theme.apply(&context);
            let egui_theme = match app_theme {
                AppTheme::Light => egui::Theme::Light,
                AppTheme::Dark => egui::Theme::Dark,
                _ => unreachable!("the test covers standard themes only"),
            };
            let expected = context.style_of(egui_theme).visuals.error_fg_color;
            let mut app = NoterApp {
                error_msg: Some("visible failure".to_owned()),
                ..NoterApp::default()
            };

            let output = context.run_ui(egui::RawInput::default(), |ui| app.show_error(ui));
            let actual = output
                .shapes
                .iter()
                .find_map(|shape| text_shape_color(&shape.shape, "Error: visible failure"));

            assert_eq!(actual, Some(expected));
        }
    }

    #[test]
    fn undo_and_redo_shortcuts_accept_platform_command_conventions() {
        let windows_command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };
        let mac_command = egui::Modifiers {
            mac_cmd: true,
            command: true,
            ..egui::Modifiers::NONE
        };

        assert_eq!(
            collect_edit_shortcut_from_input_for_os(
                shortcut_input(windows_command, egui::Key::Z),
                egui::os::OperatingSystem::Windows,
            ),
            Some(EditCommand::Undo)
        );
        assert_eq!(
            collect_edit_shortcut_from_input_for_os(
                shortcut_input(windows_command, egui::Key::Y),
                egui::os::OperatingSystem::Windows,
            ),
            Some(EditCommand::Redo)
        );
        assert_eq!(
            collect_edit_shortcut_from_input_for_os(
                shortcut_input(windows_command.plus(egui::Modifiers::SHIFT), egui::Key::Z,),
                egui::os::OperatingSystem::Windows,
            ),
            Some(EditCommand::Redo)
        );
        assert_eq!(
            collect_edit_shortcut_from_input_for_os(
                shortcut_input(mac_command, egui::Key::Z),
                egui::os::OperatingSystem::Mac,
            ),
            Some(EditCommand::Undo)
        );
        assert_eq!(
            collect_edit_shortcut_from_input_for_os(
                shortcut_input(mac_command.plus(egui::Modifiers::SHIFT), egui::Key::Z),
                egui::os::OperatingSystem::Mac,
            ),
            Some(EditCommand::Redo)
        );
    }

    #[test]
    fn select_all_remains_global_when_go_to_line_is_unavailable() {
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };

        assert_eq!(
            collect_edit_shortcut_from_input(shortcut_input(command, egui::Key::A)),
            Some(EditCommand::SelectAll)
        );
        assert_eq!(
            collect_edit_shortcut_from_input(shortcut_input(egui::Modifiers::CTRL, egui::Key::G,)),
            Some(EditCommand::GoToLine)
        );
        assert_eq!(
            collect_edit_shortcut_from_input_with_availability(
                shortcut_input(command, egui::Key::A),
                true,
                false,
            ),
            Some(EditCommand::SelectAll)
        );
        assert_eq!(
            collect_edit_shortcut_from_input_with_availability(
                shortcut_input(egui::Modifiers::CTRL, egui::Key::G),
                true,
                false,
            ),
            None
        );
    }

    #[test]
    fn wrap_and_mode_shortcuts_are_document_gated() {
        let wrap = egui::Modifiers {
            alt: true,
            ..egui::Modifiers::NONE
        };
        let mode = egui::Modifiers {
            ctrl: true,
            command: true,
            shift: true,
            ..egui::Modifiers::NONE
        };

        assert_eq!(
            collect_view_shortcut_from_input(shortcut_input(wrap, egui::Key::Z)),
            Some(ViewCommandRequest::preserve_control(
                ViewCommand::ToggleWordWrap
            ))
        );
        assert_eq!(
            collect_view_shortcut_from_input(shortcut_input(mode, egui::Key::M)),
            Some(ViewCommandRequest::preserve_control(
                ViewCommand::ToggleDocumentView
            ))
        );
        assert_eq!(
            collect_view_shortcut_from_input_with_availability(
                shortcut_input(wrap, egui::Key::Z),
                false
            ),
            None
        );
        assert_eq!(
            collect_view_shortcut_from_input_with_availability(
                shortcut_input(mode, egui::Key::M),
                false
            ),
            None
        );
    }

    #[test]
    fn repeated_view_shortcuts_execute_once_per_input_event() {
        let wrap = egui::Modifiers {
            alt: true,
            ..egui::Modifiers::NONE
        };
        let mode = egui::Modifiers {
            ctrl: true,
            command: true,
            shift: true,
            ..egui::Modifiers::NONE
        };
        let context = egui::Context::default();
        let mut app = NoterApp::default();
        let initially_wrapped = app.text_wrap.is_wrapped();
        let collect_next = |app: &mut NoterApp, input: egui::RawInput| {
            let mut command = None;
            let _ = context.run_ui(input, |ui| {
                app.restore_deferred_input(ui);
                command = app.collect_input_shortcut(ui, true, true);
            });
            command
        };

        for input in [
            repeated_shortcut_input(wrap, egui::Key::Z),
            egui::RawInput::default(),
        ] {
            let Some(InputShortcut::View(command)) = collect_next(&mut app, input) else {
                panic!("each Wrap event should produce one command");
            };
            app.execute_view_command(command, &context);
        }
        assert_eq!(app.text_wrap.is_wrapped(), initially_wrapped);

        for input in [
            repeated_shortcut_input(mode, egui::Key::M),
            egui::RawInput::default(),
        ] {
            let Some(InputShortcut::View(command)) = collect_next(&mut app, input) else {
                panic!("each Mode event should produce one command");
            };
            app.execute_view_command(command, &context);
            app.apply_pending_document_view();
        }
        assert_eq!(app.view, DocumentView::Text);
    }

    #[test]
    fn mode_shortcuts_serialize_with_enter_in_both_editor_directions() {
        let mode_modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            shift: true,
            ..egui::Modifiers::NONE
        };
        let run = |start_view: DocumentView, events: Vec<egui::Event>| {
            let source = "- item";
            let selection = Selection::caret(source.len());
            let mut app = NoterApp {
                text: source.to_owned(),
                document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
                selection,
                pending_selection_restore: Some(selection),
                view: start_view,
                ..NoterApp::default()
            };
            let context = egui::Context::default();
            theme::configure_styles(&context);
            let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| {
                app.render_frame(ui);
            });
            let mut input = ui_input(1_200.0, 760.0, 0.1);
            input.events = events;
            let _ = context.run_ui(input, |ui| app.render_frame(ui));
            for (step, time) in [0.2, 0.3, 0.4].into_iter().enumerate() {
                let _ = context.run_ui(ui_input(1_200.0, 760.0, time), |ui| {
                    app.render_frame(ui);
                });
                assert!(
                    step < 2
                        || (app.deferred_input_events.is_empty()
                            && !app.markdown_editor.has_deferred_input()),
                    "serialized input should drain within the bounded follow-up frames"
                );
            }
            (app.view, app.text)
        };
        let mode = || key_press(mode_modifiers, egui::Key::M);
        let enter = || key_press(egui::Modifiers::NONE, egui::Key::Enter);

        assert_eq!(
            run(DocumentView::Markdown, vec![mode(), enter()]),
            (DocumentView::Text, "- item\n".to_owned())
        );
        assert_eq!(
            run(DocumentView::Markdown, vec![enter(), mode()]),
            (DocumentView::Text, "- item\n- ".to_owned())
        );
        assert_eq!(
            run(DocumentView::Text, vec![mode(), enter()]),
            (DocumentView::Markdown, "- item\n- ".to_owned())
        );
        assert_eq!(
            run(DocumentView::Text, vec![enter(), mode()]),
            (DocumentView::Markdown, "- item\n".to_owned())
        );
    }

    #[test]
    fn markdown_text_format_and_mode_shortcuts_share_one_ordered_queue() {
        let source = "ab";
        let selection = Selection::caret(1);
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection,
            pending_selection_restore: Some(selection),
            view: DocumentView::Markdown,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        theme::configure_styles(&context);
        let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| {
            app.render_frame(ui);
        });
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };
        let mode = command.plus(egui::Modifiers::SHIFT);
        let mut input = ui_input(1_200.0, 760.0, 0.1);
        input.events = vec![
            egui::Event::Text("x".to_owned()),
            key_press(command, egui::Key::B),
            key_press(mode, egui::Key::M),
        ];
        let _ = context.run_ui(input, |ui| app.render_frame(ui));
        for time in [0.2, 0.3, 0.4] {
            let _ = context.run_ui(ui_input(1_200.0, 760.0, time), |ui| {
                app.render_frame(ui);
            });
        }

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.text, "ax****b");
        assert!(app.deferred_input_events.is_empty());
        assert!(!app.markdown_editor.has_deferred_input());
    }

    #[test]
    fn destructive_prompt_discards_trailing_input_and_blocks_editor_mutation() {
        let mut document = Document::new();
        document
            .replace_text("dirty")
            .expect("fixture edit should make the document dirty");
        let selection = Selection::caret(5);
        let mut app = NoterApp {
            text: "dirty".to_owned(),
            document,
            selection,
            pending_selection_restore: Some(selection),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        theme::configure_styles(&context);
        let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| {
            app.render_frame(ui);
        });
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };
        let mut request_new = ui_input(1_200.0, 760.0, 0.1);
        request_new.events = vec![
            key_press(command, egui::Key::N),
            egui::Event::Text("x".to_owned()),
        ];
        let _ = context.run_ui(request_new, |ui| app.render_frame(ui));

        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::New)
        );
        assert_eq!(app.text, "dirty");
        assert_eq!(String::from(app.document.rope()), "dirty");
        assert!(app.deferred_input_events.is_empty());

        let mut typed_behind_modal = ui_input(1_200.0, 760.0, 0.2);
        typed_behind_modal.events = vec![egui::Event::Text("y".to_owned())];
        let _ = context.run_ui(typed_behind_modal, |ui| app.render_frame(ui));
        assert_eq!(app.text, "dirty");
        assert_eq!(String::from(app.document.rope()), "dirty");

        app.cancel_pending_abandon();
        let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.3), |ui| {
            app.render_frame(ui);
        });
        assert_eq!(app.text, "dirty");
        assert!(app.lifecycle.pending_intent().is_none());
    }

    #[test]
    fn switching_mode_by_command_does_not_change_bytes() {
        let source = "# Heading\n\nParagraph";
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            view: DocumentView::Text,
            ..NoterApp::default()
        };
        let ctx = egui::Context::default();
        app.execute_view_command(
            ViewCommandRequest::restore_document(ViewCommand::ToggleDocumentView),
            &ctx,
        );
        app.apply_pending_document_view();

        assert_eq!(app.view, DocumentView::Markdown);
        assert_eq!(String::from(app.document.rope()), source);
        assert!(!app.document.is_dirty());
    }

    #[test]
    fn zoom_shortcuts_share_one_bounded_view_command_path() {
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };

        for (key, expected) in [
            (egui::Key::Plus, ViewCommand::ZoomIn),
            (egui::Key::Equals, ViewCommand::ZoomIn),
            (egui::Key::Minus, ViewCommand::ZoomOut),
            (egui::Key::Num0, ViewCommand::ResetZoom),
        ] {
            assert_eq!(
                collect_view_shortcut_from_input(shortcut_input(command, key)),
                Some(ViewCommandRequest::preserve_control(expected))
            );
        }
    }

    #[test]
    fn zoom_menu_commands_are_disabled_at_their_bounds() {
        let mut app = NoterApp::default();
        assert!(!app.view_command_enabled(ViewCommand::ResetZoom));
        while app.editor_zoom.zoom_in() {}
        assert!(!app.view_command_enabled(ViewCommand::ZoomIn));
        assert!(app.view_command_enabled(ViewCommand::ZoomOut));
        assert!(app.view_command_enabled(ViewCommand::ResetZoom));

        while app.editor_zoom.zoom_out() {}
        assert!(app.view_command_enabled(ViewCommand::ZoomIn));
        assert!(!app.view_command_enabled(ViewCommand::ZoomOut));
    }

    #[test]
    fn text_and_markdown_canvases_share_the_same_editor_gutter() {
        let source = "Full-width paragraph";
        let mut markdown_app = NoterApp {
            view: DocumentView::Markdown,
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            ..NoterApp::default()
        };
        let mut text_app = NoterApp {
            view: DocumentView::Text,
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        theme::configure_styles(&context);

        let markdown_output = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| {
            let _ = markdown_app.show_markdown_editor(ui);
        });
        let text_output = context.run_ui(ui_input(1_200.0, 760.0, 0.1), |ui| {
            let _ = text_app.show_text_editor(ui);
        });
        let markdown_x = text_position(&rendered_text(&markdown_output), source).x;
        let text_x = text_position(&rendered_text(&text_output), source).x;

        assert!(
            (markdown_x - text_x).abs() <= 4.0,
            "Text began at x={text_x}, but Markdown began at x={markdown_x}"
        );
    }

    #[test]
    fn zoom_wheel_direction_uses_vertical_motion_and_rejects_invalid_input() {
        assert_eq!(
            zoom_command_from_wheel_delta(egui::vec2(0.0, 12.0)),
            Some(ViewCommand::ZoomIn)
        );
        assert_eq!(
            zoom_command_from_wheel_delta(egui::vec2(0.0, -12.0)),
            Some(ViewCommand::ZoomOut)
        );
        assert_eq!(
            zoom_command_from_wheel_delta(egui::vec2(2.0, 12.0)),
            Some(ViewCommand::ZoomIn)
        );
        for delta in [
            egui::Vec2::ZERO,
            egui::vec2(12.0, 0.0),
            egui::vec2(0.0, f32::NAN),
            egui::vec2(0.0, f32::INFINITY),
        ] {
            assert_eq!(zoom_command_from_wheel_delta(delta), None);
        }
    }

    #[test]
    fn markdown_toolbar_zoom_uses_the_shared_view_command_without_editing_source() {
        let source = "# Zoom sample";
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            ..NoterApp::default()
        };
        let revision = app.document.revision();
        let context = egui::Context::default();
        context.enable_accesskit();
        theme::configure_styles(&context);
        let initial = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| app.render_frame(ui));
        let zoom_out = text_position(&rendered_text(&initial), "-");
        let reset = text_position(&rendered_text(&initial), "100%");
        let zoom_in = text_position(&rendered_text(&initial), "+");
        assert!(zoom_out.x < reset.x);
        assert!(reset.x < zoom_in.x);

        let _ = context.run_ui(
            click_input(1_200.0, 760.0, 0.1, zoom_in + egui::vec2(4.0, 4.0)),
            |ui| app.render_frame(ui),
        );

        assert_eq!(app.editor_zoom.percent(), 110);
        assert_eq!(app.document.revision(), revision);
        assert_eq!(String::from(app.document.rope()), source);
        assert!(!app.document.is_dirty());
    }

    #[test]
    fn scrolling_over_the_zoom_percentage_uses_the_shared_document_zoom_path() {
        let source = "# Zoom sample";
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            ..NoterApp::default()
        };
        let revision = app.document.revision();
        let context = egui::Context::default();
        theme::configure_styles(&context);
        let initial = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| app.render_frame(ui));
        let reset = text_position(&rendered_text(&initial), "100%") + egui::vec2(4.0, 4.0);
        let mut wheel = ui_input(1_200.0, 760.0, 0.1);
        wheel.events.extend([
            egui::Event::PointerMoved(reset),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, 12.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
        ]);

        let _ = context.run_ui(wheel, |ui| app.render_frame(ui));

        assert_eq!(app.editor_zoom.percent(), 110);
        assert_eq!(app.document.revision(), revision);
        assert_eq!(String::from(app.document.rope()), source);
        assert!(!app.document.is_dirty());
    }

    #[test]
    fn wheel_and_keyboard_zoom_events_execute_in_queue_order_at_the_bound() {
        let source = "# Zoom sample";
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            ..NoterApp::default()
        };
        while app.editor_zoom.zoom_in() {}
        assert_eq!(app.editor_zoom.percent(), 300);
        let revision = app.document.revision();
        let context = egui::Context::default();
        theme::configure_styles(&context);
        let initial = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| app.render_frame(ui));
        let reset = text_position(&rendered_text(&initial), "300%") + egui::vec2(4.0, 4.0);
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };
        let mut input = ui_input(1_200.0, 760.0, 0.1);
        input.events = vec![
            egui::Event::PointerMoved(reset),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -12.0),
                modifiers: egui::Modifiers::NONE,
                phase: egui::TouchPhase::Move,
            },
            key_press(command, egui::Key::Plus),
        ];
        let _ = context.run_ui(input, |ui| app.render_frame(ui));
        assert_eq!(app.editor_zoom.percent(), 290);
        let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.2), |ui| {
            app.render_frame(ui);
        });

        assert_eq!(app.editor_zoom.percent(), 300);
        assert_eq!(app.document.revision(), revision);
        assert_eq!(String::from(app.document.rope()), source);
    }

    #[test]
    fn pointer_zoom_applies_only_over_the_document_and_preserves_control_type() {
        let source = "zoom sample";
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            ..NoterApp::default()
        };
        let revision = app.document.revision();
        let context = egui::Context::default();
        theme::configure_styles(&context);

        let initial = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.render_frame(ui));
        let initial_menu_size = rendered_font_size(&initial, "File");
        let initial_document_size = rendered_font_size(&initial, source);

        let mut menu_zoom = ui_input(800.0, 600.0, 0.1);
        menu_zoom.events.extend([
            egui::Event::PointerMoved(egui::pos2(20.0, 15.0)),
            egui::Event::Zoom(1.1),
        ]);
        let _ = context.run_ui(menu_zoom, |ui| app.render_frame(ui));
        assert_eq!(app.editor_zoom.percent(), 100);

        let mut document_zoom = ui_input(800.0, 600.0, 0.2);
        document_zoom.events.extend([
            egui::Event::PointerMoved(egui::pos2(400.0, 300.0)),
            egui::Event::Zoom(1.1),
        ]);
        let zoomed = context.run_ui(document_zoom, |ui| app.render_frame(ui));

        assert_eq!(app.editor_zoom.percent(), 110);
        assert_eq!(
            rendered_font_size(&zoomed, "File").to_bits(),
            initial_menu_size.to_bits()
        );
        assert!(rendered_font_size(&zoomed, source) > initial_document_size);
        assert_eq!(app.document.revision(), revision);
        assert_eq!(String::from(app.document.rope()), source);
    }

    #[test]
    fn focused_find_input_retains_local_edit_shortcuts() {
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };
        let shifted_command = command.plus(egui::Modifiers::SHIFT);

        for input in [
            shortcut_input(command, egui::Key::Z),
            shortcut_input(command, egui::Key::Y),
            shortcut_input(shifted_command, egui::Key::Z),
            shortcut_input(command, egui::Key::A),
        ] {
            assert_eq!(
                collect_edit_shortcut_from_input_with_document_focus(input, false),
                None
            );
        }
        assert_eq!(
            collect_edit_shortcut_from_input_with_document_focus(
                shortcut_input(command, egui::Key::F),
                false,
            ),
            Some(EditCommand::Find)
        );
    }

    #[test]
    fn keyboard_zoom_preserves_find_focus_and_routes_follow_up_text_to_the_query() {
        let source = "document body";
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: Selection::caret(source.len()),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        theme::configure_styles(&context);
        app.find_bar.open(false, &app.text, app.selection);
        let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| {
            app.render_frame(ui);
        });
        assert!(app.find_bar.owns_text_focus(&context));

        let shortcut = ViewCommand::ZoomIn.shortcut();
        let mut zoom = ui_input(1_200.0, 760.0, 0.1);
        zoom.events
            .push(key_press(shortcut.modifiers, shortcut.logical_key));
        let _ = context.run_ui(zoom, |ui| app.render_frame(ui));
        assert_eq!(app.editor_zoom.percent(), 110);
        assert!(app.find_bar.owns_text_focus(&context));

        let mut text = ui_input(1_200.0, 760.0, 0.2);
        text.events.push(egui::Event::Text("x".to_owned()));
        let _ = context.run_ui(text, |ui| app.render_frame(ui));
        assert_eq!(app.text, source);
        assert_eq!(String::from(app.document.rope()), source);
        assert!(!app.document.is_dirty());
        assert!(app.find_bar.has_query());
    }

    #[test]
    fn find_navigation_without_a_query_opens_find_instead_of_doing_nothing() {
        let mut app = NoterApp::default();
        app.execute_edit_command(EditCommand::FindNext, &egui::Context::default());
        assert!(app.find_bar.is_open());
        assert!(!app.find_bar.has_query());
    }

    #[test]
    fn closing_find_restores_document_focus_for_immediate_typing() {
        let mut app = NoterApp::default();
        app.find_bar.open(false, &app.text, app.selection);
        app.execute_find_bar_action(FindBarAction::Close, EditTimestamp::default());
        let context = egui::Context::default();

        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.render_frame(ui));

        assert!(context.memory(|memory| memory.has_focus(app.editor_id())));
    }

    #[test]
    fn find_replace_and_app_shortcuts_share_one_ordered_queue() -> std::io::Result<()> {
        let directory = tempdir()?;
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };
        let enter = || key_press(egui::Modifiers::NONE, egui::Key::Enter);

        let save_first = directory.path().join("save-first.txt");
        fs::write(&save_first, "one")?;
        let mut app = NoterApp {
            text: "one".to_owned(),
            document: Document::from_path(&save_first).expect("fixture should load"),
            selection: Selection::new(0, 3),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        theme::configure_styles(&context);
        focus_empty_replacement_field(&mut app, &context, 0.0);
        let mut input = ui_input(1_200.0, 760.0, 0.1);
        input.events = vec![key_press(command, egui::Key::S), enter()];
        let _ = context.run_ui(input, |ui| app.render_frame(ui));
        assert_eq!(app.text, "one");
        assert_eq!(fs::read_to_string(&save_first)?, "one");
        assert!(!app.document.is_dirty());
        let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.2), |ui| {
            app.render_frame(ui);
        });
        assert_eq!(app.text, "");
        assert_eq!(fs::read_to_string(&save_first)?, "one");
        assert!(app.document.is_dirty());

        let enter_first = directory.path().join("enter-first.txt");
        fs::write(&enter_first, "one")?;
        let mut app = NoterApp {
            text: "one".to_owned(),
            document: Document::from_path(&enter_first).expect("fixture should load"),
            selection: Selection::new(0, 3),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        theme::configure_styles(&context);
        focus_empty_replacement_field(&mut app, &context, 1.0);
        let mut input = ui_input(1_200.0, 760.0, 1.1);
        input.events = vec![enter(), key_press(command, egui::Key::S)];
        let _ = context.run_ui(input, |ui| app.render_frame(ui));
        assert_eq!(app.text, "");
        assert_eq!(fs::read_to_string(&enter_first)?, "one");
        assert!(app.document.is_dirty());
        let _ = context.run_ui(ui_input(1_200.0, 760.0, 1.2), |ui| {
            app.render_frame(ui);
        });
        assert_eq!(fs::read_to_string(&enter_first)?, "");
        assert!(!app.document.is_dirty());

        for (find_first, expected_text, expected_selection) in [
            (false, " one", Selection::new(1, 4)),
            (true, "one ", Selection::caret(4)),
        ] {
            let source = "one one";
            let mut app = NoterApp {
                text: source.to_owned(),
                document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
                selection: Selection::new(0, 3),
                ..NoterApp::default()
            };
            let context = egui::Context::default();
            theme::configure_styles(&context);
            focus_empty_replacement_field(&mut app, &context, 2.0);
            let shortcut = EditCommand::FindNext.shortcut(context.os());
            let find_next = key_press(shortcut.modifiers, shortcut.logical_key);
            let mut input = ui_input(1_200.0, 760.0, 2.1);
            input.events = if find_first {
                vec![find_next, enter()]
            } else {
                vec![enter(), find_next]
            };
            let _ = context.run_ui(input, |ui| app.render_frame(ui));
            let _ = context.run_ui(ui_input(1_200.0, 760.0, 2.2), |ui| {
                app.render_frame(ui);
            });
            assert_eq!(app.text, expected_text);
            assert_eq!(app.selection, expected_selection);
        }
        Ok(())
    }

    #[test]
    fn find_escape_never_routes_query_text_into_the_document() {
        let escape = || key_press(egui::Modifiers::NONE, egui::Key::Escape);
        for events in [
            vec![egui::Event::Text("x".to_owned()), escape()],
            vec![escape(), egui::Event::Text("x".to_owned())],
        ] {
            let source = "body";
            let mut app = NoterApp {
                text: source.to_owned(),
                document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
                selection: Selection::caret(source.len()),
                ..NoterApp::default()
            };
            let context = egui::Context::default();
            theme::configure_styles(&context);
            app.find_bar.open(false, &app.text, app.selection);
            let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| {
                app.render_frame(ui);
            });
            let mut input = ui_input(1_200.0, 760.0, 0.1);
            input.events = events;
            let _ = context.run_ui(input, |ui| app.render_frame(ui));
            let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.2), |ui| {
                app.render_frame(ui);
            });
            assert_eq!(app.text, source);
            assert_eq!(String::from(app.document.rope()), source);
            assert!(!app.document.is_dirty());
        }
    }

    #[test]
    fn replacement_projection_is_checked_before_mutating_ui_text() {
        assert_eq!(
            projected_replacement_length(8, TextRange::new(2, 6), 4),
            Some(8)
        );
        assert_eq!(
            projected_replacement_length(8, TextRange::new(2, 6), 5),
            Some(9)
        );
        assert_eq!(
            projected_replacement_length(8, TextRange::new(6, 2), 1),
            None
        );
        assert_eq!(
            projected_replacement_length(usize::MAX, TextRange::new(0, 0), 1),
            None
        );
    }

    #[test]
    fn every_interactive_path_uses_the_measured_editor_ceiling() {
        let plain = NoterApp::default();
        assert_eq!(plain.interactive_text_maximum(), INTERACTIVE_TEXT_MAX_BYTES);

        let bom_document =
            Document::from_bytes(b"\xEF\xBB\xBFtext").expect("the UTF-8 BOM fixture should load");
        assert_eq!(
            NoterApp::interactive_text_maximum_for(&bom_document),
            INTERACTIVE_TEXT_MAX_BYTES
        );
    }

    #[test]
    fn replace_cannot_grow_an_interactive_document_past_the_ceiling() {
        let source = "x".repeat(INTERACTIVE_TEXT_MAX_BYTES);
        let selection = Selection::new(source.len() - 1, source.len());
        let mut app = NoterApp {
            document: Document::from_bytes(source.as_bytes())
                .expect("the exact-boundary fixture should load"),
            text: source.clone(),
            selection,
            ..NoterApp::default()
        };
        app.execute_edit_command(EditCommand::Replace, &egui::Context::default());
        app.find_bar.set_replacement_for_test("yy".to_owned());

        app.replace_selected_match(EditTimestamp::default());

        assert_eq!(app.text, source);
        assert_eq!(app.document.rope().len_bytes(), INTERACTIVE_TEXT_MAX_BYTES);
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("Replace would create")
                && message.contains(&INTERACTIVE_TEXT_MAX_BYTES.to_string())
                && message.contains("document was not changed")
        }));

        app.find_bar.set_replacement_for_test(String::new());
        app.replace_selected_match(EditTimestamp::default());
        assert_eq!(app.text.len(), INTERACTIVE_TEXT_MAX_BYTES - 1);
        assert_eq!(
            app.document.rope().len_bytes(),
            INTERACTIVE_TEXT_MAX_BYTES - 1
        );
    }

    #[test]
    fn replace_all_accepts_the_ceiling_and_rejects_one_byte_beyond_it() {
        let source = format!("{}z", "x".repeat(INTERACTIVE_TEXT_MAX_BYTES - 2));
        let selection = Selection::new(source.len() - 1, source.len());
        let mut app = NoterApp {
            document: Document::from_bytes(source.as_bytes())
                .expect("the near-boundary fixture should load"),
            text: source.clone(),
            selection,
            ..NoterApp::default()
        };
        app.execute_edit_command(EditCommand::Replace, &egui::Context::default());
        app.find_bar.set_replacement_for_test("zzz".to_owned());

        app.replace_all_matches(EditTimestamp::default());

        assert_eq!(app.text, source);
        assert_eq!(
            app.document.rope().len_bytes(),
            INTERACTIVE_TEXT_MAX_BYTES - 1
        );
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("Replace All could not run")
                && message.contains(&INTERACTIVE_TEXT_MAX_BYTES.to_string())
                && message.contains("document was not changed")
        }));

        app.find_bar.set_replacement_for_test("zz".to_owned());
        app.replace_all_matches(EditTimestamp::default());
        assert_eq!(app.text.len(), INTERACTIVE_TEXT_MAX_BYTES);
        assert_eq!(app.document.rope().len_bytes(), INTERACTIVE_TEXT_MAX_BYTES);
    }

    #[test]
    fn edit_menu_uses_platform_labels_and_history_enabled_state() {
        for (operating_system, undo_label, redo_label) in [
            (egui::os::OperatingSystem::Windows, "Ctrl+Z", "Ctrl+Y"),
            (egui::os::OperatingSystem::Mac, "Cmd+Z", "Shift+Cmd+Z"),
        ] {
            let context = egui::Context::default();
            context.set_os(operating_system);
            let mut app = NoterApp::default();
            let mut command = None;
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(240.0);
                app.show_edit_menu(ui, &mut command);
            });
            let labels = rendered_text(&output)
                .into_iter()
                .map(|(label, _)| label)
                .collect::<Vec<_>>();
            assert!(labels.contains(&"Undo".to_owned()));
            assert!(labels.contains(&"Redo".to_owned()));
            assert!(labels.contains(&"Select All".to_owned()));
            assert!(labels.contains(&"Find...".to_owned()));
            assert!(labels.contains(&"Find Next".to_owned()));
            assert!(labels.contains(&"Find Previous".to_owned()));
            assert!(labels.contains(&"Replace...".to_owned()));
            assert!(labels.contains(&"Go To Line...".to_owned()));
            assert!(labels.contains(&undo_label.to_owned()));
            assert!(labels.contains(&redo_label.to_owned()));
            assert!(command.is_none());

            app.text = "x".to_owned();
            record_test_editor_change(&mut app, EditOrigin::TextInput);
            assert!(app.history.can_undo());
            assert!(!app.history.can_redo());
        }
    }

    #[test]
    fn markdown_edit_menu_enables_select_all_but_not_go_to_line() {
        let app = NoterApp {
            view: DocumentView::Markdown,
            ..NoterApp::default()
        };

        assert!(app.edit_command_enabled(EditCommand::SelectAll));
        assert!(!app.edit_command_enabled(EditCommand::GoToLine));
    }

    #[test]
    fn select_all_restores_the_exact_text_mode_source_selection() {
        let source = "one\r\n三";
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: Selection::caret(2),
            ..NoterApp::default()
        };

        app.execute_edit_command(EditCommand::SelectAll, &egui::Context::default());

        assert_eq!(app.selection, Selection::new(0, source.len()));
        assert_eq!(app.pending_selection_restore, Some(app.selection));
        assert_eq!(String::from(app.document.rope()), source);
        assert!(!app.document.is_dirty());
    }

    #[test]
    fn cut_command_removes_selection_through_the_shared_edit_path() {
        let source = "abcdef";
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: Selection::new(1, 4),
            ..NoterApp::default()
        };
        assert!(app.edit_command_enabled(EditCommand::Cut));
        assert!(app.edit_command_enabled(EditCommand::Copy));
        app.execute_edit_command(EditCommand::Cut, &egui::Context::default());
        assert_eq!(String::from(app.document.rope()), "aef");
        assert!(app.document.is_dirty());
        assert_eq!(app.selection, Selection::caret(1));
    }

    #[test]
    fn wrap_and_zoom_commands_never_change_document_bytes_or_revision() {
        let source = "a long line that remains authoritative";
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            ..NoterApp::default()
        };
        let revision = app.document.revision();

        let ctx = egui::Context::default();
        app.execute_view_command(
            ViewCommandRequest::restore_document(ViewCommand::ToggleWordWrap),
            &ctx,
        );
        assert_eq!(app.text_wrap, TextWrap::Unwrapped);
        app.execute_view_command(
            ViewCommandRequest::restore_document(ViewCommand::ZoomIn),
            &ctx,
        );
        assert_eq!(app.editor_zoom.percent(), 110);
        app.execute_view_command(
            ViewCommandRequest::restore_document(ViewCommand::ResetZoom),
            &ctx,
        );

        assert_eq!(app.editor_zoom.percent(), 100);
        assert_eq!(app.document.revision(), revision);
        assert_eq!(String::from(app.document.rope()), source);
        assert!(!app.document.is_dirty());
    }

    #[test]
    fn markdown_mode_keeps_formatted_wrapping_and_supports_select_all() {
        let source = "# One\n\nTwo";
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            text_wrap: TextWrap::Wrapped,
            selection: Selection::caret(0),
            ..NoterApp::default()
        };

        let ctx = egui::Context::default();
        app.execute_view_command(
            ViewCommandRequest::restore_document(ViewCommand::ToggleWordWrap),
            &ctx,
        );
        app.execute_edit_command(EditCommand::SelectAll, &egui::Context::default());

        assert_eq!(app.text_wrap, TextWrap::Wrapped);
        assert_eq!(app.selection, Selection::new(0, source.len()));
        assert_eq!(app.pending_selection_restore, Some(app.selection));
        assert_eq!(String::from(app.document.rope()), source);
        assert!(!app.document.is_dirty());
    }

    #[test]
    fn markdown_select_all_shortcut_activates_the_exact_document_selection() {
        let source = "# One\r\n\r\nTwo";
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: Selection::caret(2),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };

        let _ = context.run_ui(shortcut_input(command, egui::Key::A), |ui| {
            ui.set_width(800.0);
            app.render_frame(ui);
        });

        let selection = Selection::new(0, source.len());
        assert_eq!(app.selection, selection);
        assert_eq!(app.markdown_editor.source_selection(), Some(selection));
        assert_eq!(app.text, source);
        assert_eq!(String::from(app.document.rope()), source);
        assert!(!app.document.is_dirty());
    }

    #[test]
    fn markdown_mode_restores_a_directional_cross_block_text_selection() {
        let source = "# First\r\n\r\nSecond\rThird";
        let selection = Selection::new(source.len(), 2);
        let mut app = NoterApp {
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            text: source.to_owned(),
            selection,
            ..NoterApp::default()
        };
        let revision = app.document.revision();

        app.select_document_view(DocumentView::Markdown);
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| {
            let _ = app.show_markdown_editor(ui);
        });

        assert_eq!(app.view, DocumentView::Markdown);
        assert_eq!(app.selection, selection);
        assert_eq!(app.pending_selection_restore, None);
        assert_eq!(app.markdown_editor.source_selection(), Some(selection));
        assert!(app.markdown_editor.is_editing());
        assert!(app.error_msg.is_none());
        assert_eq!(app.document.revision(), revision);
        assert_eq!(String::from(app.document.rope()), source);
        assert!(!app.document.is_dirty());
    }

    #[test]
    fn markdown_mode_rejects_a_selection_outside_exact_utf8_boundaries() {
        let source = "é";
        let selection = Selection::caret(1);
        let mut app = NoterApp {
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            text: source.to_owned(),
            selection,
            ..NoterApp::default()
        };

        app.select_document_view(DocumentView::Markdown);

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.selection, selection);
        assert_eq!(app.pending_selection_restore, Some(selection));
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("exact UTF-8 source boundaries")
                && message.contains("kept Text Mode")
                && message.contains("preserved")
        }));
    }

    #[test]
    fn markdown_projection_budget_precedes_cross_block_selection_mapping() {
        let source = "x\n\n".repeat(513);
        let selection = Selection::new(0, source.len());
        let mut app = NoterApp {
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            text: source,
            selection,
            ..NoterApp::default()
        };

        app.select_document_view(DocumentView::Markdown);

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.selection, selection);
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("512-block layout budget")
                && message.contains("remains fully available in Text Mode")
                && !message.contains("one formatted block at a time")
        }));
    }

    #[test]
    fn markdown_mode_restores_a_same_block_text_selection_for_formatting() {
        let source = "# First\n\nSecond";
        let selection = Selection::new(2, 7);
        let mut app = NoterApp {
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            text: source.to_owned(),
            selection,
            ..NoterApp::default()
        };

        app.select_document_view(DocumentView::Markdown);
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| {
            let _ = app.show_markdown_editor(ui);
        });

        assert_eq!(app.view, DocumentView::Markdown);
        assert_eq!(app.markdown_editor.source_selection(), Some(selection));
        assert!(app.markdown_editor.is_editing());
        assert!(app.error_msg.is_none());
    }

    #[test]
    fn markdown_selection_restore_commits_dirty_input_first() {
        let source = "plain";
        let mut app = NoterApp {
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            text: source.to_owned(),
            selection: Selection::caret(source.len()),
            pending_selection_restore: Some(Selection::caret(source.len())),
            view: DocumentView::Markdown,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| {
            let _ = app.show_markdown_editor(ui);
        });
        assert!(app.markdown_editor.is_editing());

        let mut typed = ui_input(800.0, 600.0, 0.1);
        typed.events.push(egui::Event::Text("!".to_owned()));
        let _ = context.run_ui(typed, |ui| {
            let _ = app.show_markdown_editor(ui);
        });
        assert_eq!(app.text, "plain!");

        // Queue a restore while the next frame will also receive more input so
        // commit-before-restore is exercised for pending selection work.
        app.pending_selection_restore = Some(Selection::caret(0));
        app.preserve_focus_on_selection_restore = true;
        let mut more = ui_input(800.0, 600.0, 0.2);
        more.events.push(egui::Event::Text("?".to_owned()));
        let _ = context.run_ui(more, |ui| {
            let outcome = app.show_markdown_editor(ui);
            assert!(
                outcome.changed || app.text.contains('!'),
                "prior committed input must remain after selection restore"
            );
        });
        assert!(
            app.text.contains('!'),
            "selection restore must not discard committed Markdown input; got {:?}",
            app.text
        );
    }

    #[test]
    fn markdown_escape_records_the_final_caret_for_undo_and_redo() {
        let source = "abcXYZ";
        let initial_selection = Selection::new(0, 3);
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: initial_selection,
            pending_selection_restore: Some(initial_selection),
            view: DocumentView::Markdown,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.show_editor(ui));
        let mut input = ui_input(800.0, 600.0, 0.1);
        input.events.push(egui::Event::Text("q".to_owned()));
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });

        let _ = context.run_ui(input, |ui| app.show_editor(ui));

        assert_eq!(app.text, "qXYZ");
        assert_eq!(String::from(app.document.rope()), "qXYZ");
        assert_eq!(app.selection, Selection::caret(1));
        assert!(!app.markdown_editor.is_editing());

        app.execute_edit_command(EditCommand::Undo, &egui::Context::default());
        assert_eq!(app.text, source);
        assert_eq!(app.selection, initial_selection);
        app.execute_edit_command(EditCommand::Redo, &egui::Context::default());
        assert_eq!(app.text, "qXYZ");
        assert_eq!(app.selection, Selection::caret(1));
    }

    #[test]
    fn find_navigation_uses_the_selected_literal_and_reports_wrap_selection() {
        let source = "one two one";
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: Selection::new(0, 3),
            ..NoterApp::default()
        };

        app.execute_edit_command(EditCommand::Find, &egui::Context::default());
        app.execute_edit_command(EditCommand::FindNext, &egui::Context::default());
        assert_eq!(app.selection, Selection::new(8, 11));
        assert_eq!(app.pending_selection_restore, Some(app.selection));
        assert!(app.preserve_focus_on_selection_restore);

        app.execute_edit_command(EditCommand::FindNext, &egui::Context::default());
        assert_eq!(app.selection, Selection::new(0, 3));
        app.execute_edit_command(EditCommand::FindPrevious, &egui::Context::default());
        assert_eq!(app.selection, Selection::new(8, 11));
        assert_eq!(app.document.rope().to_string(), source);
        assert!(!app.document.is_dirty());
    }

    #[test]
    fn replace_and_replace_all_are_distinct_revision_checked_undo_steps() {
        let source = "cat cat";
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: Selection::new(0, 3),
            ..NoterApp::default()
        };

        app.execute_edit_command(EditCommand::Replace, &egui::Context::default());
        app.replace_selected_match(EditTimestamp::default());
        assert_eq!(app.text, " cat");
        assert_eq!(app.document.rope().to_string(), " cat");
        assert_eq!(app.history.len(), 1);
        app.execute_edit_command(EditCommand::Undo, &egui::Context::default());
        assert_eq!(app.text, source);

        app.selection = Selection::new(0, source.len());
        app.replace_all_matches(EditTimestamp::default());
        assert_eq!(app.text, " ");
        assert_eq!(app.document.rope().to_string(), " ");
        assert_eq!(app.history.len(), 1);
        assert_eq!(app.selection, Selection::new(0, 1));
        app.execute_edit_command(EditCommand::Undo, &egui::Context::default());
        assert_eq!(app.text, source);
        assert_eq!(app.selection, Selection::new(0, source.len()));
    }

    #[test]
    fn app_typing_coalesces_but_paste_remains_a_separate_undo_step() {
        let mut app = NoterApp::default();
        for (text, selection, origin, time) in [
            (
                "a",
                Selection::caret(1),
                EditOrigin::TextInput,
                Duration::from_millis(0),
            ),
            (
                "ab",
                Selection::caret(2),
                EditOrigin::TextInput,
                Duration::from_millis(100),
            ),
            (
                "abc",
                Selection::caret(3),
                EditOrigin::Paste,
                Duration::from_millis(200),
            ),
        ] {
            app.text = text.to_owned();
            app.record_editor_change(EditorFrameOutcome {
                changed: true,
                selection,
                origin,
                observed_at: EditTimestamp::new(time),
            });
        }

        assert_eq!(app.history.len(), 2);
        app.execute_edit_command(EditCommand::Undo, &egui::Context::default());
        assert_eq!(app.text, "ab");
        app.execute_edit_command(EditCommand::Undo, &egui::Context::default());
        assert_eq!(app.text, "");
    }

    #[test]
    fn app_undo_and_redo_restore_authority_selection_and_dirty_state() {
        let document = Document::from_bytes(b"abc").expect("fixture should load");
        let mut app = NoterApp {
            text: "abc".to_owned(),
            document,
            selection: Selection::new(2, 1),
            ..NoterApp::default()
        };
        app.text = "aBc".to_owned();
        app.record_editor_change(EditorFrameOutcome {
            changed: true,
            selection: Selection::new(1, 2),
            origin: EditOrigin::TextInput,
            observed_at: EditTimestamp::default(),
        });

        assert_eq!(String::from(app.document.rope()), "aBc");
        assert!(app.document.is_dirty());
        assert!(app.history.can_undo());

        app.execute_edit_command(EditCommand::Undo, &egui::Context::default());
        assert_eq!(app.text, "abc");
        assert_eq!(String::from(app.document.rope()), "abc");
        assert_eq!(app.selection, Selection::new(2, 1));
        assert!(!app.document.is_dirty());
        assert!(app.history.can_redo());

        app.execute_edit_command(EditCommand::Redo, &egui::Context::default());
        assert_eq!(app.text, "aBc");
        assert_eq!(String::from(app.document.rope()), "aBc");
        assert_eq!(app.selection, Selection::new(1, 2));
        assert!(app.document.is_dirty());
        assert!(app.history.can_undo());
    }

    #[test]
    fn undo_persists_the_latest_dirty_recovery_revision() {
        use crate::crash_recovery::CrashRecoverySession;

        let directory = tempdir().expect("tempdir");
        let mut app = NoterApp {
            crash_recovery: CrashRecoverySession::open_at(directory.path()),
            ..NoterApp::default()
        };
        app.text = "alpha".to_owned();
        app.record_editor_change(EditorFrameOutcome {
            changed: true,
            selection: Selection::caret(5),
            origin: EditOrigin::TextInput,
            observed_at: EditTimestamp::default(),
        });
        app.text = "alpha beta".to_owned();
        app.record_editor_change(EditorFrameOutcome {
            changed: true,
            selection: Selection::caret(10),
            origin: EditOrigin::Paste,
            observed_at: EditTimestamp::default(),
        });
        app.crash_recovery
            .force_due_persist_for_test(&app.document, app.selection);

        app.execute_edit_command(EditCommand::Undo, &egui::Context::default());
        assert_eq!(app.text, "alpha");
        assert_eq!(app.selection, Selection::caret(5));
        assert!(app.document.is_dirty());
        app.crash_recovery
            .force_due_persist_for_test(&app.document, app.selection);
        drop(app);

        let recovered = CrashRecoverySession::open_at(directory.path());
        let offer = recovered
            .active_offer()
            .expect("the latest dirty history state should be recoverable");
        assert_eq!(offer.metadata().content_len(), b"alpha".len());
        assert_eq!(offer.metadata().selection(), Selection::caret(5));
    }

    #[test]
    fn rendered_undo_shortcut_uses_shared_history_instead_of_widget_history() {
        let mut app = NoterApp {
            text: "recorded".to_owned(),
            ..NoterApp::default()
        };
        record_test_editor_change(&mut app, EditOrigin::TextInput);
        assert!(app.history.can_undo());
        let context = egui::Context::default();
        let modifiers = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };

        let _ = context.run_ui(shortcut_input(modifiers, egui::Key::Z), |ui| {
            app.render_frame(ui);
        });

        assert!(app.text.is_empty());
        assert_eq!(app.document.rope().len_bytes(), 0);
        assert!(!app.document.is_dirty());
        assert!(app.history.can_redo());
    }

    #[test]
    fn text_ime_preedit_is_transient_until_one_committed_transaction() {
        use crate::crash_recovery::CrashRecoverySession;

        let directory = tempdir().expect("tempdir");
        let source = "abc";
        let base_selection = Selection::new(1, 2);
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: base_selection,
            pending_selection_restore: Some(base_selection),
            crash_recovery: CrashRecoverySession::open_at(directory.path()),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.show_editor(ui));

        let first = context.run_ui(ime_preedit_input(0.1, "仮"), |ui| app.show_editor(ui));
        assert!(
            rendered_text(&first)
                .iter()
                .any(|(text, _)| text.contains("a仮c"))
        );
        assert_eq!(app.text, source);
        assert_eq!(String::from(app.document.rope()), source);
        assert_eq!(app.selection, base_selection);
        assert!(!app.document.is_dirty());
        assert!(!app.history.can_undo());
        assert!(app.crash_recovery.next_persist_delay().is_none());

        let second = context.run_ui(ime_preedit_input(0.2, "仮名"), |ui| app.show_editor(ui));
        assert!(
            rendered_text(&second)
                .iter()
                .any(|(text, _)| text.contains("a仮名c"))
        );
        assert_eq!(app.text, source);
        assert_eq!(String::from(app.document.rope()), source);
        assert_eq!(app.history.len(), 0);
        assert!(app.crash_recovery.next_persist_delay().is_none());

        let _ = context.run_ui(ime_commit_input(0.3, "漢"), |ui| app.show_editor(ui));
        assert_eq!(app.text, "a漢c");
        assert_eq!(String::from(app.document.rope()), "a漢c");
        assert_eq!(app.selection, Selection::caret(4));
        assert_eq!(app.history.len(), 1);
        assert!(app.document.is_dirty());
        assert!(app.crash_recovery.next_persist_delay().is_some());
        app.execute_edit_command(EditCommand::Undo, &context);
        assert_eq!(app.text, source);
        assert_eq!(app.selection, base_selection);
        assert!(!app.history.can_undo());
        app.execute_edit_command(EditCommand::Redo, &context);
        assert_eq!(app.text, "a漢c");
        app.crash_recovery
            .force_due_persist_for_test(&app.document, app.selection);
        drop(app);

        let mut recovered = CrashRecoverySession::open_at(directory.path());
        let (document, selection) = recovered
            .restore_active_offer()
            .expect("the committed IME transaction should be recoverable");
        assert_eq!(String::from(document.rope()), "a漢c");
        assert_eq!(selection, Selection::caret(4));
    }

    #[test]
    fn text_ime_cancellation_restores_authority_without_history_or_recovery() {
        use crate::crash_recovery::CrashRecoverySession;

        let directory = tempdir().expect("tempdir");
        let source = "abc";
        let base_selection = Selection::new(1, 2);
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: base_selection,
            pending_selection_restore: Some(base_selection),
            crash_recovery: CrashRecoverySession::open_at(directory.path()),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.show_editor(ui));
        let _ = context.run_ui(ime_preedit_input(0.1, "仮"), |ui| app.show_editor(ui));
        let _ = context.run_ui(ime_preedit_input(0.2, ""), |ui| app.show_editor(ui));
        let restored = context.run_ui(ui_input(800.0, 600.0, 0.3), |ui| app.show_editor(ui));

        assert!(
            rendered_text(&restored)
                .iter()
                .any(|(text, _)| text.contains(source))
        );
        assert_eq!(app.text, source);
        assert_eq!(String::from(app.document.rope()), source);
        assert_eq!(app.selection, base_selection);
        assert!(!app.document.is_dirty());
        assert!(!app.history.can_undo());
        assert!(app.crash_recovery.next_persist_delay().is_none());
        drop(app);

        assert!(
            CrashRecoverySession::open_at(directory.path())
                .active_offer()
                .is_none()
        );
    }

    #[test]
    fn text_ime_publishes_a_commit_before_the_next_composition() {
        let source = "abc";
        let base_selection = Selection::new(1, 2);
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: base_selection,
            pending_selection_restore: Some(base_selection),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.show_editor(ui));
        let mut combined = ui_input(800.0, 600.0, 0.1);
        combined.events.extend([
            egui::Event::Ime(egui::ImeEvent::Commit("漢".to_owned())),
            egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "次".to_owned(),
                active_range_chars: None,
            }),
        ]);

        let _ = context.run_ui(combined, |ui| app.show_editor(ui));
        assert_eq!(app.text, "a漢c");
        assert_eq!(String::from(app.document.rope()), "a漢c");
        assert_eq!(app.history.len(), 1);

        let preedit = context.run_ui(ui_input(800.0, 600.0, 0.2), |ui| {
            app.restore_deferred_input(ui);
            app.show_editor(ui);
        });
        assert!(
            rendered_text(&preedit)
                .iter()
                .any(|(text, _)| text.contains("a漢次c"))
        );
        assert_eq!(app.text, "a漢c");
        assert_eq!(String::from(app.document.rope()), "a漢c");
        assert_eq!(app.history.len(), 1);

        let _ = context.run_ui(ime_commit_input(0.3, ""), |ui| app.show_editor(ui));
        assert_eq!(app.text, "a漢c");
        assert_eq!(String::from(app.document.rope()), "a漢c");
        assert_eq!(app.history.len(), 1);
    }

    #[test]
    fn active_ime_commit_survives_same_frame_focus_transfer_in_both_modes() {
        for view in [DocumentView::Text, DocumentView::Markdown] {
            let source = "abc";
            let base_selection = Selection::new(1, 2);
            let mut app = NoterApp {
                text: source.to_owned(),
                document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
                selection: base_selection,
                pending_selection_restore: Some(base_selection),
                view,
                ..NoterApp::default()
            };
            let context = egui::Context::default();
            let other_control = egui::Id::new(("ime-focus-transfer", view.label()));
            let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.show_editor(ui));
            let _ = context.run_ui(ime_preedit_input(0.1, "仮"), |ui| app.show_editor(ui));

            let _ = context.run_ui(ime_commit_input(0.2, "漢"), |ui| {
                ui.memory_mut(|memory| memory.request_focus(other_control));
                app.show_editor(ui);
            });

            assert_eq!(app.text, "a漢c", "{view:?}");
            assert_eq!(String::from(app.document.rope()), "a漢c", "{view:?}");
            assert_eq!(app.selection, Selection::caret(4), "{view:?}");
            assert_eq!(app.history.len(), 1, "{view:?}");
            assert_eq!(context.memory(egui::Memory::focused), Some(other_control));
        }
    }

    #[test]
    fn full_render_routes_active_ime_commit_only_to_the_document() {
        use crate::crash_recovery::CrashRecoverySession;

        for view in [DocumentView::Text, DocumentView::Markdown] {
            let recovery = tempdir().expect("tempdir");
            let source = "abc";
            let base_selection = Selection::new(1, 2);
            let mut app = NoterApp {
                text: source.to_owned(),
                document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
                selection: base_selection,
                pending_selection_restore: Some(base_selection),
                view,
                crash_recovery: CrashRecoverySession::open_at(recovery.path()),
                ..NoterApp::default()
            };
            let context = egui::Context::default();
            theme::configure_styles(&context);
            let _ = context.run_ui(ui_input(1_000.0, 700.0, 0.0), |ui| app.render_frame(ui));
            let _ = context.run_ui(ime_preedit_input(0.1, "仮"), |ui| app.render_frame(ui));
            app.find_bar.open(false, &app.text, Selection::caret(0));

            let _ = context.run_ui(ime_commit_input(0.2, "漢"), |ui| app.render_frame(ui));

            assert_eq!(app.text, "a漢c", "{view:?}");
            assert_eq!(String::from(app.document.rope()), "a漢c", "{view:?}");
            assert_eq!(app.selection, Selection::caret(4), "{view:?}");
            assert_eq!(app.history.len(), 1, "{view:?}");
            assert!(!app.find_bar.has_query(), "{view:?}");
            assert!(app.find_bar.owns_text_focus(&context), "{view:?}");
        }
    }

    #[test]
    fn full_render_removes_ime_commit_before_later_text_controls() {
        use crate::crash_recovery::CrashRecoverySession;

        let recovery = tempdir().expect("tempdir");
        let source = "abc";
        let base_selection = Selection::new(1, 2);
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: base_selection,
            pending_selection_restore: Some(base_selection),
            crash_recovery: CrashRecoverySession::open_at(recovery.path()),
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        theme::configure_styles(&context);
        let _ = context.run_ui(ui_input(1_000.0, 700.0, 0.0), |ui| app.render_frame(ui));
        let _ = context.run_ui(ime_preedit_input(0.1, "仮"), |ui| app.render_frame(ui));
        app.go_to_line.open(1);

        let _ = context.run_ui(ime_commit_input(0.2, "漢"), |ui| app.render_frame(ui));

        assert_eq!(app.text, "a漢c");
        assert_eq!(String::from(app.document.rope()), "a漢c");
        assert_eq!(app.history.len(), 1);
        assert_eq!(app.go_to_line.input_for_test(), "1");
        assert!(app.go_to_line.owns_text_focus(&context));
    }

    #[test]
    fn markdown_ime_preedit_uses_the_same_commit_boundary() {
        let source = "abc";
        let base_selection = Selection::new(1, 2);
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection: base_selection,
            pending_selection_restore: Some(base_selection),
            view: DocumentView::Markdown,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.show_editor(ui));

        let first = context.run_ui(ime_preedit_input(0.1, "仮"), |ui| app.show_editor(ui));
        assert!(
            rendered_text(&first)
                .iter()
                .any(|(text, _)| text.contains("a仮c"))
        );
        assert_eq!(app.text, source);
        assert_eq!(String::from(app.document.rope()), source);
        assert_eq!(app.selection, base_selection);
        assert!(!app.document.is_dirty());
        assert!(!app.history.can_undo());
        assert!(app.crash_recovery.next_persist_delay().is_none());

        let second = context.run_ui(ime_preedit_input(0.2, "仮名"), |ui| app.show_editor(ui));
        assert!(
            rendered_text(&second)
                .iter()
                .any(|(text, _)| text.contains("a仮名c"))
        );
        assert_eq!(app.text, source);
        assert_eq!(app.history.len(), 0);

        let _ = context.run_ui(ime_commit_input(0.3, "漢"), |ui| app.show_editor(ui));
        assert_eq!(app.text, "a漢c");
        assert_eq!(String::from(app.document.rope()), "a漢c");
        assert_eq!(app.selection, Selection::caret(4));
        assert_eq!(app.history.len(), 1);
        app.execute_edit_command(EditCommand::Undo, &context);
        assert_eq!(app.text, source);
        assert_eq!(app.selection, base_selection);

        let _ = context.run_ui(ui_input(800.0, 600.0, 0.4), |ui| app.show_editor(ui));
        let _ = context.run_ui(ime_preedit_input(0.5, "仮"), |ui| app.show_editor(ui));
        let _ = context.run_ui(ime_commit_input(0.6, ""), |ui| app.show_editor(ui));
        let restored = context.run_ui(ui_input(800.0, 600.0, 0.7), |ui| app.show_editor(ui));
        assert!(
            rendered_text(&restored)
                .iter()
                .any(|(text, _)| text.contains(source))
        );
        assert_eq!(app.text, source);
        assert_eq!(String::from(app.document.rope()), source);
        assert_eq!(app.selection, base_selection);
        assert!(!app.history.can_undo());
        assert!(app.history.can_redo());
        assert!(!app.document.is_dirty());
        assert!(app.crash_recovery.next_persist_delay().is_none());
    }

    #[test]
    fn text_editor_discards_widget_local_undo_snapshots() {
        let context = egui::Context::default();
        let mut app = NoterApp {
            text: "bounded history".to_owned(),
            pending_selection_restore: Some(Selection::caret("bounded history".len())),
            ..NoterApp::default()
        };
        let editor_id = app.editor_id();

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(640.0);
            let _ = app.show_text_editor(ui);
        });
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Text("!".to_owned()));
        let _ = context.run_ui(input, |ui| {
            ui.set_width(640.0);
            let outcome = app.show_text_editor(ui);
            assert!(outcome.changed);
        });
        assert_eq!(app.text, "bounded history!");

        let state = egui::TextEdit::load_state(&context, editor_id)
            .expect("the rendered editor should persist its state");
        let cursor = state
            .cursor
            .char_range()
            .unwrap_or_else(|| egui::text::CCursorRange::one(egui::text::CCursor::new(0)));
        let current = (cursor, app.text);
        assert!(!state.undoer().has_undo(&current));
        assert!(!state.undoer().has_redo(&current));
    }

    #[test]
    fn text_editor_bounds_paste_before_widget_layout_and_reports_the_limit() {
        let context = egui::Context::default();
        let mut app = NoterApp {
            text: "text".to_owned(),
            pending_selection_restore: Some(Selection::caret(4)),
            ..NoterApp::default()
        };

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(640.0);
            assert!(!app.show_text_editor_with_limit(ui, 8).changed);
        });
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Paste("12345é".to_owned()));
        let _ = context.run_ui(input, |ui| {
            ui.set_width(640.0);
            assert!(app.show_text_editor_with_limit(ui, 8).changed);
        });

        assert_eq!(app.text, "text1234");
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("8-byte safety limit")
                && message.contains("remaining budget was preserved")
        }));
    }

    #[test]
    fn first_keystroke_after_text_undo_replaces_the_restored_selection() {
        let document = Document::from_bytes(b"abc").expect("fixture should load");
        let mut app = NoterApp {
            text: "abc".to_owned(),
            document,
            selection: Selection::new(1, 2),
            ..NoterApp::default()
        };
        app.text = "aBc".to_owned();
        app.record_editor_change(EditorFrameOutcome {
            changed: true,
            selection: Selection::caret(2),
            origin: EditOrigin::TextInput,
            observed_at: EditTimestamp::default(),
        });
        let editor_id = app.editor_id();
        let context = egui::Context::default();
        context.memory_mut(|memory| memory.request_focus(editor_id));

        app.execute_edit_command(EditCommand::Undo, &egui::Context::default());
        assert_eq!(app.editor_id(), editor_id);
        assert_eq!(app.pending_selection_restore, Some(Selection::new(1, 2)));

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Text("X".to_owned()));
        let _ = context.run_ui(input, |ui| {
            ui.set_width(900.0);
            app.render_frame(ui);
        });

        assert_eq!(app.text, "aXc");
        assert_eq!(app.document.rope().to_string(), "aXc");
        assert!(app.history.can_undo());
        assert!(!app.history.can_redo());
    }

    #[test]
    fn failed_editor_change_restores_the_authoritative_document() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("authoritative text")
            .expect("the authoritative fixture should advance the revision");
        app.text = "unrecorded editor text".to_owned();
        app.markdown_editor.activate_first_block(&app.text);
        let previous_editor_serial = app.document_editor_serial;

        let before = String::from(app.document.rope());
        let transaction = EditTransaction::between(
            app.document.revision(),
            &before,
            &app.text,
            Selection::caret(0),
            Selection::caret(app.text.len()),
            EditOrigin::TextInput,
            EditTimestamp::default(),
        )
        .expect("fixture selections should be valid")
        .expect("fixture text should differ");
        app.record_editor_change_with(&transaction, |_, _| Err(EditError::RevisionExhausted));

        assert_eq!(app.text, "authoritative text");
        assert_eq!(
            app.document_editor_serial,
            previous_editor_serial.wrapping_add(1)
        );
        assert!(!app.markdown_editor.is_editing());
        assert!(
            app.error_msg
                .as_deref()
                .is_some_and(|message| message.contains("restored to the last authoritative text"))
        );

        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            app.request_close(ui.ctx());
        });
        assert!(app.document.is_dirty());
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::Quit)
        );
        assert!(
            output.viewport_output[&egui::ViewportId::ROOT]
                .commands
                .contains(&egui::ViewportCommand::CancelClose)
        );
    }

    #[test]
    fn top_menu_aligns_mode_and_theme_controls_opposite_the_application_menus() {
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            theme: AppTheme::System,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        context.enable_accesskit();
        theme::configure_styles(&context);
        let output = show_menu_frame(&mut app, &context, ui_input(1_200.0, 760.0, 0.0));
        let text = rendered_text(&output);
        let file = text_position(&text, "File");
        let edit = text_position(&text, "Edit");
        let view = text_position(&text, "View");
        let help = text_position(&text, "Help");
        let mode = text_position(&text, "Mode");
        let plain_text = text_position(&text, "Text");
        let markdown = text_position(&text, "Markdown");
        let theme = text_position(&text, "Theme");
        let system = text_position(&text, "System");
        let theme_bounds = accesskit_bounds(&output, "Theme: System");

        assert!(file.x < edit.x);
        assert!(edit.x < view.x);
        assert!(view.x < help.x);
        assert!(help.x < mode.x);
        assert!(mode.x < plain_text.x);
        assert!(plain_text.x < markdown.x);
        assert!(markdown.x < theme.x);
        assert!(theme.x < system.x);
        assert!(system.x > 1_130.0);
        assert!(
            theme_bounds.x1 <= 1_200.0,
            "Theme extends beyond the viewport: {theme_bounds:?}"
        );
        for position in [edit, view, help, mode, plain_text, markdown, theme, system] {
            assert!((file.y - position.y).abs() <= 2.0);
        }
    }

    #[test]
    fn narrow_top_controls_use_labeled_menus_without_losing_mode_or_theme() {
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        context.enable_accesskit();
        theme::configure_styles(&context);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(420.0, 300.0),
            )),
            ..Default::default()
        };
        let output = context.run_ui(input, |ui| {
            let mut file_command = None;
            let mut edit_command = None;
            let mut view_command = None;
            app.show_menu(ui, &mut file_command, &mut edit_command, &mut view_command);
        });
        let text = rendered_text(&output);
        let file = text_position(&text, "File");
        let more_menu_position = text_position(&text, "More");
        let mode_control_position = text_position(&text, "Mode: Markdown");
        let theme = text_position(&text, "Theme: System");
        let more_menu_bounds = accesskit_bounds(&output, "More");
        let mode_control_bounds = accesskit_bounds(&output, "Mode: Markdown");
        let theme_bounds = accesskit_bounds(&output, "Theme: System");

        assert!(file.x < more_menu_position.x);
        assert!(more_menu_position.x < mode_control_position.x);
        assert!(mode_control_position.x < theme.x);
        assert!(
            more_menu_bounds.x1 <= mode_control_bounds.x0,
            "More and Mode overlap: {more_menu_bounds:?}, {mode_control_bounds:?}"
        );
        assert!(
            mode_control_bounds.x1 <= theme_bounds.x0,
            "Mode and Theme overlap: {mode_control_bounds:?}, {theme_bounds:?}"
        );
        assert!(
            theme_bounds.x1 <= 420.0,
            "Theme extends beyond the minimum viewport: {theme_bounds:?}"
        );
        for position in [more_menu_position, mode_control_position, theme] {
            assert!((file.y - position.y).abs() <= 2.0);
        }
        assert!(!text.iter().any(|(label, _)| label == "View"));
        assert!(!text.iter().any(|(label, _)| label == "Text"));
    }

    #[test]
    fn compact_more_menu_keeps_wrap_and_zoom_pointer_reachable() {
        let mut app = NoterApp::default();
        let context = egui::Context::default();
        theme::configure_styles(&context);
        let viewport = egui::vec2(420.0, 300.0);
        let mut time = 0.0;

        let mut output =
            show_menu_frame(&mut app, &context, ui_input(viewport.x, viewport.y, time));
        time += 0.1;
        let _ = click_menu_label(&mut app, &context, &output, "More", viewport, time);
        time += 0.05;
        output = show_menu_frame(&mut app, &context, ui_input(viewport.x, viewport.y, time));
        time += 0.1;
        let _ = hover_menu_label(&mut app, &context, &output, "View", viewport, time);
        time += 0.5;
        output = show_menu_frame(&mut app, &context, ui_input(viewport.x, viewport.y, time));
        let labels = rendered_text(&output)
            .into_iter()
            .map(|(label, _)| label)
            .collect::<Vec<_>>();
        assert!(
            labels.iter().any(|label| label == "Word Wrap"),
            "{labels:?}"
        );
        time += 0.1;
        let _ = click_menu_label(&mut app, &context, &output, "Word Wrap", viewport, time);
        assert_eq!(app.text_wrap, TextWrap::Unwrapped);

        time += 0.05;
        output = show_menu_frame(&mut app, &context, ui_input(viewport.x, viewport.y, time));
        time += 0.1;
        let _ = click_menu_label(&mut app, &context, &output, "More", viewport, time);
        time += 0.05;
        output = show_menu_frame(&mut app, &context, ui_input(viewport.x, viewport.y, time));
        time += 0.1;
        let _ = hover_menu_label(&mut app, &context, &output, "View", viewport, time);
        time += 0.5;
        output = show_menu_frame(&mut app, &context, ui_input(viewport.x, viewport.y, time));
        time += 0.1;
        let _ = hover_menu_label(&mut app, &context, &output, "Zoom: 100%", viewport, time);
        time += 0.5;
        output = show_menu_frame(&mut app, &context, ui_input(viewport.x, viewport.y, time));
        time += 0.1;
        let _ = click_menu_label(&mut app, &context, &output, "Zoom In", viewport, time);

        assert_eq!(app.editor_zoom.percent(), 110);
    }

    #[test]
    fn narrow_top_controls_keep_specialty_theme_labels_inside_the_viewport() {
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            theme: AppTheme::AmberScreen,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        context.enable_accesskit();
        theme::configure_styles(&context);
        AppTheme::AmberScreen.apply(&context);

        let output = show_menu_frame(&mut app, &context, ui_input(420.0, 300.0, 0.0));
        let text = rendered_text(&output);
        let mode_bounds = accesskit_bounds(&output, "Mode: Markdown");
        let theme_bounds = accesskit_bounds(&output, "Theme: Amber Screen");

        assert!(text.iter().any(|(label, _)| label == "Theme: Amber"));
        assert!(mode_bounds.x1 <= theme_bounds.x0);
        assert!(theme_bounds.x1 <= 420.0);
    }

    #[test]
    fn switching_modes_carries_the_directional_source_selection() {
        let source = "# heading\n\nParagraph";
        let selection = Selection::new(9, 2);
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection,
            ..NoterApp::default()
        };

        app.select_document_view(DocumentView::Markdown);
        assert_eq!(app.view, DocumentView::Markdown);
        assert_eq!(app.pending_selection_restore, Some(selection));

        app.pending_selection_restore = None;
        app.select_document_view(DocumentView::Text);
        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.pending_selection_restore, Some(selection));
    }

    #[test]
    fn requested_mode_change_commits_same_frame_markdown_input_before_reset() {
        let source = "# A";
        let selection = Selection::new(0, source.len());
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection,
            pending_selection_restore: Some(selection),
            view: DocumentView::Markdown,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        theme::configure_styles(&context);

        let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| app.render_frame(ui));
        app.request_document_view(DocumentView::Text);
        let mut input = ui_input(1_200.0, 760.0, 0.1);
        input.events.push(egui::Event::Text("x".to_owned()));

        let _ = context.run_ui(input, |ui| app.render_frame(ui));

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.text, "x");
        assert_eq!(String::from(app.document.rope()), "x");
        assert!(app.document.is_dirty());
        assert!(app.history.can_undo());
    }

    #[test]
    fn top_mode_switch_and_theme_menu_execute_from_normal_pointer_clicks() {
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            theme: AppTheme::Light,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        theme::configure_styles(&context);
        AppTheme::Light.apply(&context);

        let initial = show_menu_frame(&mut app, &context, ui_input(1_200.0, 760.0, 0.0));
        let initial_text = rendered_text(&initial);
        let text_button = text_position(&initial_text, "Text") + egui::vec2(4.0, 4.0);

        let mode_switched = show_menu_frame(
            &mut app,
            &context,
            click_input(1_200.0, 760.0, 0.1, text_button),
        );
        assert_eq!(app.view, DocumentView::Text);
        let light_button =
            text_position(&rendered_text(&mode_switched), "Light") + egui::vec2(4.0, 4.0);

        show_menu_frame(
            &mut app,
            &context,
            click_input(1_200.0, 760.0, 0.2, light_button),
        );
        let theme_menu = show_menu_frame(&mut app, &context, ui_input(1_200.0, 760.0, 0.25));
        let theme_choices = rendered_text(&theme_menu);
        assert!(
            theme_choices
                .iter()
                .any(|(label, _)| label == "Green Screen")
        );
        assert!(
            theme_choices
                .iter()
                .any(|(label, _)| label == "Amber Screen")
        );
        let dark_choice = text_position(&theme_choices, "Dark") + egui::vec2(4.0, 4.0);
        show_menu_frame(
            &mut app,
            &context,
            click_input(1_200.0, 760.0, 0.3, dark_choice),
        );

        assert_eq!(app.theme, AppTheme::Dark);
        assert_eq!(context.theme(), egui::Theme::Dark);
    }

    #[test]
    fn top_menu_accessibility_order_matches_visual_order() {
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            theme: AppTheme::Light,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        context.enable_accesskit();
        theme::configure_styles(&context);

        let output = show_menu_frame(&mut app, &context, ui_input(1_200.0, 760.0, 0.0));
        let expected = [
            "File",
            "Edit",
            "View",
            "Help",
            "Text Mode",
            "Markdown Mode",
            "Theme: Light",
        ];
        let labels = accesskit_labels(&output);
        let relevant = labels
            .iter()
            .map(String::as_str)
            .filter(|label| expected.contains(label))
            .collect::<Vec<_>>();

        assert_eq!(relevant, expected);
    }

    #[test]
    fn format_toolbar_is_present_only_in_markdown_mode() {
        let mut app = NoterApp::default();
        let context = egui::Context::default();
        context.enable_accesskit();
        theme::configure_styles(&context);
        let mut view_command = None;

        let text_output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(1_200.0);
            app.show_format_toolbar(ui, &mut view_command);
        });
        assert!(rendered_text(&text_output).is_empty());
        assert_eq!(view_command, None);

        app.select_document_view(DocumentView::Markdown);
        let markdown_output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(1_200.0);
            app.show_format_toolbar(ui, &mut view_command);
        });
        let text = rendered_text(&markdown_output);
        let graphic_labels = ["Style", "B", "</>"];
        let visible_graphic_labels = text
            .iter()
            .map(|(label, _)| label.as_str())
            .filter(|label| graphic_labels.contains(label))
            .collect::<Vec<_>>();
        assert_eq!(visible_graphic_labels, graphic_labels);
        assert!(!text.iter().any(|(label, _)| label == "Format"));
        assert!(!text.iter().any(|(label, _)| label == "Bold"));

        let expected_accessible_labels = [
            "Paragraph style",
            "Bold",
            "Italic",
            "Link",
            "Inline code",
            "Bulleted list",
            "Quote",
            "Zoom Out",
            "100%, Reset Zoom",
            "Zoom In",
        ];
        let labels = accesskit_labels(&markdown_output);
        let relevant = labels
            .iter()
            .map(String::as_str)
            .filter(|label| expected_accessible_labels.contains(label))
            .collect::<Vec<_>>();
        assert_eq!(relevant, expected_accessible_labels);
        assert_eq!(
            accesskit_value(&markdown_output, "Paragraph style"),
            "Style"
        );
        assert!(!labels.iter().any(|label| label == "Mode"));
    }

    #[test]
    fn compact_markdown_document_bar_keeps_format_and_zoom_inside_the_viewport() {
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            ..NoterApp::default()
        };
        assert!(app.editor_zoom.zoom_in());
        let context = egui::Context::default();
        context.enable_accesskit();
        theme::configure_styles(&context);
        let mut command = None;

        let output = context.run_ui(ui_input(420.0, 100.0, 0.0), |ui| {
            app.show_format_toolbar(ui, &mut command);
        });
        let format = accesskit_bounds(&output, "Format");
        let zoom_out = accesskit_bounds(&output, "Zoom Out");
        let reset_label = "110%, Reset Zoom";
        let reset = accesskit_bounds(&output, reset_label);
        let zoom_in = accesskit_bounds(&output, "Zoom In");

        assert_eq!(accesskit_value(&output, reset_label), "110%");
        assert!(format.x1 <= zoom_out.x0);
        assert!(zoom_out.x1 <= reset.x0);
        assert!(reset.x1 <= zoom_in.x0);
        assert!(zoom_in.x1 <= 420.0);
        assert_eq!(command, None);
    }

    #[test]
    fn markdown_document_bar_zoom_preserves_control_focus_without_activating_content() {
        let source = "# Heading";
        let mut app = NoterApp {
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            text: source.to_owned(),
            selection: Selection::caret(2),
            view: DocumentView::Markdown,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        context.enable_accesskit();
        theme::configure_styles(&context);
        let viewport = egui::vec2(420.0, 300.0);

        let initial = context.run_ui(ui_input(viewport.x, viewport.y, 0.0), |ui| {
            app.render_frame(ui);
        });
        let zoom_in = accesskit_node_id(&initial, "Zoom In");
        let mut input = ui_input(viewport.x, viewport.y, 0.1);
        for action in [
            egui::accesskit::Action::Focus,
            egui::accesskit::Action::Click,
        ] {
            input.events.push(egui::Event::AccessKitActionRequest(
                egui::accesskit::ActionRequest {
                    action,
                    target_tree: egui::accesskit::TreeId::ROOT,
                    target_node: zoom_in,
                    data: None,
                },
            ));
        }
        let activated = context.run_ui(input, |ui| app.render_frame(ui));
        let update = activated
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled");

        assert_eq!(app.editor_zoom.percent(), 110);
        assert_eq!(update.focus, zoom_in);
        assert!(!app.markdown_editor.is_editing());
        assert_eq!(app.pending_selection_restore, None);
        assert_eq!(app.text, source);
        assert!(!app.document.is_dirty());
    }

    #[test]
    fn oversized_markdown_files_open_safely_in_text_mode() -> std::io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("large.md");
        fs::write(
            &path,
            vec![b'x'; crate::markdown_ui::PROTOTYPE_MARKDOWN_MAX_BYTES + 1],
        )?;
        let mut app = NoterApp::default();

        app.open_path(&path, None);

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(
            app.text.len(),
            crate::markdown_ui::PROTOTYPE_MARKDOWN_MAX_BYTES + 1
        );
        assert_eq!(app.document.to_bytes(), app.text.as_bytes());
        assert!(
            app.error_msg
                .as_deref()
                .is_some_and(|message| message.contains("fully available in Text Mode"))
        );
        Ok(())
    }

    #[test]
    fn file_above_interactive_limit_does_not_replace_the_open_document() -> std::io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("too-large.txt");
        fs::write(&path, vec![b'x'; INTERACTIVE_TEXT_MAX_BYTES + 1])?;
        let mut app = NoterApp {
            document: Document::from_bytes(b"keep this document")
                .expect("the existing document fixture should load"),
            text: "keep this document".to_owned(),
            selection: Selection::caret(4),
            ..NoterApp::default()
        };

        app.open_path(&path, None);

        assert_eq!(app.text, "keep this document");
        assert_eq!(app.document.to_bytes(), b"keep this document");
        assert_eq!(app.selection, Selection::caret(4));
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("file was not opened") && message.contains(INTERACTIVE_TEXT_MAX_LABEL)
        }));
        Ok(())
    }

    #[test]
    fn file_at_interactive_limit_opens_normally() -> std::io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("interactive-limit.txt");
        fs::write(&path, vec![b'x'; INTERACTIVE_TEXT_MAX_BYTES])?;
        let mut app = NoterApp::default();

        app.open_path(&path, None);

        assert_eq!(app.text.len(), INTERACTIVE_TEXT_MAX_BYTES);
        assert_eq!(app.document.rope().len_bytes(), INTERACTIVE_TEXT_MAX_BYTES);
        assert!(app.error_msg.is_none());
        Ok(())
    }

    #[test]
    fn exact_limit_unicode_markdown_preserves_its_utf8_bom() -> std::io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("unicode.md");
        let source = format!("{}\n\n", "é".repeat(2_047)).repeat(256);
        let mut bytes = Vec::with_capacity(source.len() + 3);
        bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        bytes.extend_from_slice(source.as_bytes());
        fs::write(&path, &bytes)?;
        let mut app = NoterApp::default();

        app.open_path(&path, None);

        assert_eq!(
            source.len(),
            crate::markdown_ui::PROTOTYPE_MARKDOWN_MAX_BYTES
        );
        assert_eq!(app.view, DocumentView::Markdown);
        assert_eq!(app.text, source);
        assert_eq!(app.document.to_bytes(), bytes);
        assert!(app.error_msg.is_none());
        Ok(())
    }

    #[test]
    fn structurally_adversarial_markdown_opens_safely_in_text_mode() -> std::io::Result<()> {
        let directory = tempdir()?;
        let path = directory.path().join("many-blocks.md");
        let source = "# x\n\n".repeat(513);
        fs::write(&path, &source)?;
        let mut app = NoterApp::default();

        app.open_path(&path, None);

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.text, source);
        assert_eq!(app.document.to_bytes(), app.text.as_bytes());
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("512-block layout budget")
                && message.contains("fully available in Text Mode")
        }));
        Ok(())
    }

    #[test]
    fn markdown_edits_crossing_the_prototype_limit_fall_back_to_text_mode() {
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            text: "x".repeat(crate::markdown_ui::PROTOTYPE_MARKDOWN_MAX_BYTES + 1),
            ..NoterApp::default()
        };

        record_test_editor_change(&mut app, EditOrigin::MarkdownInput);

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.document.to_bytes(), app.text.as_bytes());
        assert!(!app.markdown_editor.is_editing());
        assert_eq!(app.pending_selection_restore, Some(app.selection));
        assert!(app.error_msg.is_some());
    }

    #[test]
    fn markdown_edits_crossing_a_structural_budget_fall_back_to_text_mode() {
        let original = "# x\n\n".repeat(512);
        let mut app = NoterApp {
            view: DocumentView::Markdown,
            text: original.clone(),
            ..NoterApp::default()
        };
        app.document
            .replace_text(&original)
            .expect("the in-budget fixture should advance the revision");
        app.text.push_str("# x\n\n");

        record_test_editor_change(&mut app, EditOrigin::MarkdownInput);

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.document.to_bytes(), app.text.as_bytes());
        assert!(!app.markdown_editor.is_editing());
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("512-block layout budget")
                && message.contains("fully available in Text Mode")
        }));
    }

    #[test]
    fn same_frame_event_heavy_markdown_paste_falls_back_after_bounded_layout() {
        let source = "text";
        let selection = Selection::caret(source.len());
        let mut app = NoterApp {
            text: source.to_owned(),
            document: Document::from_bytes(source.as_bytes()).expect("fixture should load"),
            selection,
            pending_selection_restore: Some(selection),
            view: DocumentView::Markdown,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.0), |ui| app.show_editor(ui));
        let paste = "*x* ".repeat(3_000);
        let mut input = ui_input(800.0, 600.0, 0.1);
        input.events.push(egui::Event::Paste(paste.clone()));

        let _ = context.run_ui(input, |ui| app.show_editor(ui));

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.text, format!("text{paste}"));
        assert_eq!(String::from(app.document.rope()), app.text);
        assert!(!app.markdown_editor.is_editing());
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("8,192-event parser budget")
                && message.contains("fully available in Text Mode")
        }));
    }

    #[test]
    fn markdown_diagnostic_cache_is_revision_and_document_scoped() {
        use std::cell::Cell;

        let mut app = NoterApp {
            text: "# First\n\n### Third\n".to_owned(),
            ..NoterApp::default()
        };
        app.document
            .replace_text(&app.text)
            .expect("the first diagnostic fixture should advance the revision");

        let analysis_calls = Cell::new(0_usize);
        assert_eq!(
            app.markdown_issue_count_with(|_| {
                analysis_calls.set(analysis_calls.get() + 1);
                1
            }),
            1
        );
        let first_cache = app
            .markdown_issue_cache
            .expect("the analysis result should be cached");
        assert_eq!(
            app.markdown_issue_count_with(|_| {
                analysis_calls.set(analysis_calls.get() + 1);
                99
            }),
            1
        );
        assert_eq!(analysis_calls.get(), 1);
        assert_eq!(app.markdown_issue_cache, Some(first_cache));

        app.text = "# First\n".to_owned();
        app.document
            .replace_text(&app.text)
            .expect("the same document edit should advance its revision");
        assert_eq!(app.document_editor_serial, first_cache.document_serial);
        assert_ne!(app.document.revision(), first_cache.revision);
        assert_eq!(
            app.markdown_issue_count_with(|_| {
                analysis_calls.set(analysis_calls.get() + 1);
                0
            }),
            0
        );
        assert_eq!(analysis_calls.get(), 2);

        app.document = Document::new();
        app.text = "# First\n\n### Third\n".to_owned();
        app.document
            .replace_text(&app.text)
            .expect("the second diagnostic fixture should advance the revision");
        app.advance_document_editor();

        assert_eq!(app.document.revision(), first_cache.revision);
        assert_eq!(
            app.markdown_issue_count_with(|_| {
                analysis_calls.set(analysis_calls.get() + 1);
                1
            }),
            1
        );
        assert_eq!(analysis_calls.get(), 3);
        assert_ne!(
            app.markdown_issue_cache
                .expect("the second analysis result should be cached")
                .document_serial,
            first_cache.document_serial
        );
    }

    #[test]
    fn new_document_requests_an_unsaved_changes_decision() {
        let mut app = NoterApp {
            text: "unsaved text".to_owned(),
            ..NoterApp::default()
        };
        app.document
            .replace_text(&app.text)
            .expect("the test edit should advance the document revision");

        let context = egui::Context::default();
        app.request_new_document(&context);
        assert_eq!(app.text, "unsaved text");
        assert_eq!(String::from(app.document.rope()), "unsaved text");
        assert!(app.document.is_dirty());
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::New)
        );
        assert!(app.error_msg.is_none());
    }

    #[test]
    fn open_requests_an_unsaved_changes_decision() {
        let mut app = NoterApp {
            text: "unsaved text".to_owned(),
            ..NoterApp::default()
        };
        app.document
            .replace_text(&app.text)
            .expect("the test edit should advance the document revision");

        let context = egui::Context::default();
        app.request_open(&context);

        assert_eq!(app.text, "unsaved text");
        assert!(app.document.is_dirty());
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::Open)
        );
        assert!(app.error_msg.is_none());
    }

    #[test]
    fn reload_reads_a_clean_path_and_routes_dirty_work_through_the_same_decision()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("reload.txt");
        fs::write(&path, b"first")?;
        let document = Document::from_path(&path)?;
        let mut app = NoterApp {
            text: "first".to_owned(),
            document,
            ..NoterApp::default()
        };
        let context = egui::Context::default();

        fs::write(&path, b"second")?;
        app.request_reload(&context);
        assert_eq!(app.text, "second");
        assert!(!app.document.is_dirty());

        app.text = "unsaved".to_owned();
        app.document.replace_text(&app.text)?;
        fs::write(&path, b"third")?;
        app.request_reload(&context);
        assert_eq!(app.text, "unsaved");
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::Reload)
        );

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            app.discard_pending_abandon(ui.ctx());
        });
        assert_eq!(app.text, "third");
        assert!(!app.document.is_dirty());
        assert!(app.lifecycle.pending_intent().is_none());
        Ok(())
    }

    #[test]
    fn new_document_replaces_a_clean_document() {
        let mut app = NoterApp {
            text: "stale view text".to_owned(),
            save_recoveries: vec![active_test_recovery(
                PathBuf::from("stale.txt"),
                "Retain recovery guidance.",
            )],
            ..NoterApp::default()
        };
        app.go_to_line.open(1);
        let previous_editor_id = app.editor_id();

        let context = egui::Context::default();
        app.request_new_document(&context);
        assert!(app.text.is_empty());
        assert_eq!(app.document.rope().len_bytes(), 0);
        assert!(!app.document.is_dirty());
        assert!(app.error_msg.is_none());
        assert_eq!(app.save_recoveries.len(), 1);
        assert!(app.save_recoveries[0].notice_pending);
        assert!(app.save_is_blocked());
        assert!(!app.go_to_line.is_open());
        assert_ne!(app.editor_id(), previous_editor_id);
    }

    #[test]
    fn leaving_text_mode_closes_go_to_line() {
        let mut app = NoterApp::default();
        app.go_to_line.open(1);
        assert!(app.go_to_line.is_open());

        app.select_document_view(DocumentView::Markdown);

        assert_eq!(app.view, DocumentView::Markdown);
        assert!(!app.go_to_line.is_open());
    }

    #[test]
    fn successful_open_preserves_destination_block_and_resets_go_to_line_state()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("opened.txt");
        fs::write(&path, b"opened")?;
        let mut app = NoterApp {
            save_recoveries: vec![active_test_recovery(
                PathBuf::from("stale.txt"),
                "Retain recovery guidance.",
            )],
            ..NoterApp::default()
        };
        app.go_to_line.open(7);

        app.open_path(&path, Some(DocumentView::Text));

        assert_eq!(app.document.path(), Some(path.as_path()));
        assert_eq!(app.save_recoveries.len(), 1);
        assert!(app.save_recoveries[0].notice_pending);
        assert!(app.save_is_blocked());
        assert!(!app.go_to_line.is_open());
        Ok(())
    }

    #[test]
    fn dirty_close_opens_a_decision_instead_of_an_error() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        let context = egui::Context::default();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            app.request_close(ui.ctx());
        });
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert!(commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(!commands.contains(&egui::ViewportCommand::Close));
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::Quit)
        );
        assert!(app.error_msg.is_none());
    }

    #[test]
    fn native_dirty_close_event_is_cancelled() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        let context = egui::Context::default();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);

        let output = context.run_ui(input, |ui| app.protect_native_close(ui.ctx()));
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert!(commands.contains(&egui::ViewportCommand::CancelClose));
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::Quit)
        );
        assert!(app.error_msg.is_none());
    }

    #[test]
    fn native_close_guard_is_inert_without_a_close_request() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        let context = egui::Context::default();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            app.protect_native_close(ui.ctx());
        });
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert!(!commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(app.lifecycle.pending_intent().is_none());
    }

    #[test]
    fn native_close_guard_allows_a_clean_close_request() {
        let mut app = NoterApp::default();
        let context = egui::Context::default();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);

        let output = context.run_ui(input, |ui| app.protect_native_close(ui.ctx()));
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert!(!commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(app.lifecycle.pending_intent().is_none());
    }

    #[test]
    fn native_close_guard_allows_a_confirmed_dirty_close() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("discarded text")
            .expect("the test edit should advance the document revision");
        authorize_dirty_close(&mut app);
        let context = egui::Context::default();
        let mut input = egui::RawInput::default();
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);

        let output = context.run_ui(input, |ui| app.protect_native_close(ui.ctx()));
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert!(!commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(app.lifecycle.pending_intent().is_none());
    }

    #[test]
    fn discarding_a_dirty_close_allows_the_viewport_to_close() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        arrange_pending_intent(&mut app, PendingAbandonAction::Quit);
        let context = egui::Context::default();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            app.discard_pending_abandon(ui.ctx());
        });
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert!(commands.contains(&egui::ViewportCommand::Close));
        assert!(app.lifecycle.close_authorized(app.document.revision()));
        assert!(app.lifecycle.pending_intent().is_none());
    }

    #[test]
    fn cancelling_a_dirty_close_keeps_the_document_and_window() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        arrange_pending_intent(&mut app, PendingAbandonAction::Quit);

        app.cancel_pending_abandon();

        assert!(app.document.is_dirty());
        assert!(app.lifecycle.pending_intent().is_none());
        assert!(!app.lifecycle.close_authorized(app.document.revision()));
    }

    #[test]
    fn uncertain_save_guidance_survives_every_dirty_action_cancel() {
        #[derive(Clone, Copy)]
        enum Trigger {
            New,
            Open,
            Quit,
            NativeClose,
        }

        assert_eq!(
            UNCERTAIN_SAVE_ABANDON_GUIDANCE,
            "Cancel this dialog and reconcile every uncertain save outcome before attempting another save. Your current text remains editable."
        );

        for trigger in [
            Trigger::New,
            Trigger::Open,
            Trigger::Quit,
            Trigger::NativeClose,
        ] {
            let mut app = app_with_dismissed_uncertain_save();
            let context = egui::Context::default();
            match trigger {
                Trigger::New => app.request_new_document(&context),
                Trigger::Open => app.request_open(&context),
                Trigger::Quit => {
                    let _ = context.run_ui(egui::RawInput::default(), |ui| {
                        app.request_close(ui.ctx());
                    });
                }
                Trigger::NativeClose => {
                    let mut input = egui::RawInput::default();
                    input
                        .viewports
                        .entry(egui::ViewportId::ROOT)
                        .or_default()
                        .events
                        .push(egui::ViewportEvent::Close);
                    let _ = context.run_ui(input, |ui| app.protect_native_close(ui.ctx()));
                }
            }

            let recovery = app
                .save_recoveries
                .first()
                .expect("the uncertain save must retain recovery guidance");
            assert!(recovery.notice_pending);
            assert!(recovery.message.contains(".noter-save-recovery.tmp"));

            app.cancel_pending_abandon();

            assert!(app.lifecycle.pending_intent().is_none());
            assert!(app.save_recoveries[0].notice_pending);
        }
    }

    #[test]
    fn uncertain_save_state_blocks_an_ordinary_retry() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        fs::write(&path, b"original")?;
        let mut document = Document::from_path(&path)?;
        document.replace_text("unsaved replacement")?;
        let mut app = NoterApp {
            text: "unsaved replacement".to_owned(),
            document,
            save_recoveries: vec![active_test_recovery(
                path.clone(),
                "Reconcile the uncertain save before retrying.",
            )],
            ..NoterApp::default()
        };

        app.do_save();

        assert_eq!(fs::read(&path)?, b"original");
        assert!(app.document.is_dirty());
        assert_eq!(app.error_msg.as_deref(), Some(SAVE_RECOVERY_BLOCK_MESSAGE));
        assert!(app.save_recoveries[0].notice_pending);
        Ok(())
    }

    #[test]
    fn ordinary_save_preserves_the_loaded_baseline_for_a_dotted_path()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let ordinary_path = directory.path().join("note.txt");
        let dotted_path = directory.path().join(".").join("note.txt");
        fs::write(&ordinary_path, b"loaded bytes")?;
        let mut document = Document::from_path(&dotted_path)?;
        document.replace_text("unsaved replacement")?;
        let mut app = NoterApp {
            text: "unsaved replacement".to_owned(),
            document,
            ..NoterApp::default()
        };
        fs::write(&ordinary_path, b"external replacement")?;

        app.do_save();

        assert_eq!(fs::read(&ordinary_path)?, b"external replacement");
        assert_eq!(app.document.path(), Some(dotted_path.as_path()));
        assert!(app.document.is_dirty());
        assert!(
            app.error_msg
                .as_deref()
                .is_some_and(|message| message.contains("destination changed"))
        );
        Ok(())
    }

    #[test]
    fn save_as_refuses_an_unresolved_destination_before_preparation()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        fs::write(&path, b"original")?;
        let mut document = Document::from_path(&path)?;
        document.replace_text("unsaved replacement")?;
        let mut app = NoterApp {
            text: "unsaved replacement".to_owned(),
            document,
            save_recoveries: vec![active_test_recovery(
                path.clone(),
                "Reconcile this destination before retrying.",
            )],
            ..NoterApp::default()
        };

        app.do_save_as_to(path.clone());

        assert_eq!(fs::read(&path)?, b"original");
        assert!(app.document.is_dirty());
        assert!(app.pending_hard_link_save.is_none());
        assert!(app.save_recoveries[0].notice_pending);
        Ok(())
    }

    #[test]
    fn save_as_refuses_an_alias_of_an_unresolved_destination()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        let alias = directory.path().join(".").join("note.txt");
        fs::write(&path, b"original")?;
        let mut document = Document::from_path(&path)?;
        document.replace_text("unsaved replacement")?;
        let mut app = NoterApp {
            text: "unsaved replacement".to_owned(),
            document,
            save_recoveries: vec![active_test_recovery(
                path.clone(),
                "Reconcile this destination before retrying.",
            )],
            ..NoterApp::default()
        };

        app.do_save_as_to(alias);

        assert_eq!(fs::read(&path)?, b"original");
        assert!(app.document.is_dirty());
        assert!(app.pending_hard_link_save.is_none());
        Ok(())
    }

    #[test]
    fn save_as_refuses_a_blocked_hard_link_entry_before_confirmation()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let selected = directory.path().join("selected.txt");
        let other_link = directory.path().join("other.txt");
        fs::write(&selected, b"shared")?;
        fs::hard_link(&selected, &other_link)?;
        let mut document = Document::from_path(&selected)?;
        document.replace_text("unsaved replacement")?;
        let mut app = NoterApp {
            text: "unsaved replacement".to_owned(),
            document,
            save_recoveries: vec![active_test_recovery(
                selected.clone(),
                "Reconcile this directory entry before retrying.",
            )],
            ..NoterApp::default()
        };

        app.do_save_as_to(selected.clone());

        assert!(app.pending_hard_link_save.is_none());
        assert_eq!(fs::read(&selected)?, b"shared");
        assert_eq!(fs::read(&other_link)?, b"shared");
        assert!(app.document.is_dirty());
        Ok(())
    }

    #[test]
    fn one_unknown_save_blocks_every_later_destination_before_work_begins()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let current = directory.path().join("current.txt");
        let alternate = directory.path().join("alternate.txt");
        fs::write(&current, b"original")?;
        let mut document = Document::from_path(&current)?;
        document.replace_text("unsaved replacement")?;
        let mut app = NoterApp {
            text: "unsaved replacement".to_owned(),
            document,
            ..NoterApp::default()
        };

        record_test_unknown_save(&mut app, SaveAttempt::Current(current.clone()), "current");
        app.do_save_as_to(alternate.clone());

        assert_eq!(app.save_recoveries.len(), 1);
        assert!(app.save_recoveries[0].notice_pending);
        assert!(app.save_recoveries[0].message.contains("current"));
        assert!(app.ordinary_save_is_blocked());
        assert_eq!(app.error_msg.as_deref(), Some(SAVE_RECOVERY_BLOCK_MESSAGE));
        assert!(!alternate.exists());

        app.do_save();
        assert_eq!(fs::read(&current)?, b"original");
        assert!(app.document.is_dirty());
        Ok(())
    }

    #[test]
    fn recovery_ledger_bounds_records_messages_and_future_save_work()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let current = directory.path().join("current.txt");
        fs::write(&current, b"original")?;
        let mut document = Document::from_path(&current)?;
        document.replace_text("unsaved replacement")?;
        let mut app = NoterApp {
            text: "unsaved replacement".to_owned(),
            document,
            ..NoterApp::default()
        };

        for index in 0..MAX_SAVE_RECOVERY_RECORDS {
            let parent = directory.path().join(format!("recovery-{index}"));
            fs::create_dir(&parent)?;
            let path = parent.join("note.txt");
            fs::write(&path, b"unresolved")?;
            app.save_recoveries.push(active_test_recovery(
                path,
                &"x".repeat(MAX_SAVE_RECOVERY_MESSAGE_BYTES),
            ));
        }

        app.do_save();

        assert_eq!(fs::read(&current)?, b"original");
        assert!(app.document.is_dirty());
        assert_eq!(app.save_recoveries.len(), MAX_SAVE_RECOVERY_RECORDS);
        assert!(
            app.save_recoveries
                .iter()
                .all(|recovery| recovery.message.len() <= MAX_SAVE_RECOVERY_MESSAGE_BYTES)
        );
        assert_eq!(app.error_msg.as_deref(), Some(SAVE_RECOVERY_BLOCK_MESSAGE));
        assert!(
            app.reserve_save_recovery_slot(SaveAttempt::SaveAs(
                directory.path().join("another.txt")
            ))
            .is_none()
        );
        assert_eq!(
            app.error_msg.as_deref(),
            Some(SAVE_RECOVERY_RESERVATION_FAILURE_MESSAGE)
        );
        Ok(())
    }

    #[test]
    fn recovery_message_truncation_retains_fail_closed_guidance() {
        use noter::core::save::{SaveStage, StorageError};

        let mut output = String::new();
        output
            .try_reserve_exact(MAX_SAVE_RECOVERY_MESSAGE_BYTES)
            .expect("the bounded test buffer should reserve");
        let detail = "detail".repeat(2_000);
        let message = write_save_recovery_message(
            output,
            &StorageError::new(SaveStage::Cleanup, detail),
            &StorageError::new(SaveStage::Reconcile, "state differs"),
        );

        assert_eq!(message.len(), MAX_SAVE_RECOVERY_MESSAGE_BYTES);
        assert!(message.is_char_boundary(message.len()));
        assert!(message.contains("Do not save again"));
        assert!(message.contains(".noter-save-*.tmp"));
    }

    #[test]
    fn recovery_reservation_preallocates_every_bounded_record_field() {
        let destination = PathBuf::from("parent").join("note.txt");
        let mut app = NoterApp::default();

        let reservation = app
            .reserve_save_recovery_slot(SaveAttempt::SaveAs(destination.clone()))
            .expect("a bounded destination should reserve before save work");

        assert_eq!(reservation.attempt.destination(), destination);
        assert!(reservation.message.capacity() >= MAX_SAVE_RECOVERY_MESSAGE_BYTES);
        assert!(reservation.destination_label.capacity() >= MAX_SAVE_RECOVERY_LABEL_BYTES);
        assert_eq!(
            reservation.destination_label,
            destination.display().to_string()
        );
    }

    #[test]
    fn oversized_recovery_destination_is_rejected_before_save_work() {
        let oversized = PathBuf::from("x".repeat(MAX_SAVE_RECOVERY_DESTINATION_BYTES + 1));
        let mut app = NoterApp::default();

        assert!(
            app.reserve_save_recovery_slot(SaveAttempt::SaveAs(oversized))
                .is_none()
        );
        assert!(app.save_recoveries.is_empty());
        assert_eq!(
            app.error_msg.as_deref(),
            Some(SAVE_RECOVERY_PATH_LIMIT_MESSAGE)
        );
    }

    #[test]
    fn recovery_destination_label_is_bounded_on_a_utf8_boundary() {
        let path = PathBuf::from("directory").join("leaf".repeat(1_000));
        let label = bounded_destination_label(&path)
            .expect("the bounded label buffer should reserve successfully");

        assert_eq!(label.len(), MAX_SAVE_RECOVERY_LABEL_BYTES);
        assert!(label.is_char_boundary(label.len()));
        assert!(label.ends_with("..."));
    }

    #[test]
    fn reconciliation_removes_only_the_confirmed_record_without_writing()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let first = directory.path().join("first.txt");
        let second = directory.path().join("second.txt");
        fs::write(&first, b"first bytes")?;
        fs::write(&second, b"second bytes")?;
        let mut app = NoterApp {
            save_recoveries: vec![
                active_test_recovery(first.clone(), "Inspect first."),
                active_test_recovery(second.clone(), "Inspect second."),
            ],
            pending_recovery_reconciliation: Some(0),
            error_msg: Some(SAVE_RECOVERY_BLOCK_MESSAGE.to_owned()),
            ..NoterApp::default()
        };

        assert!(app.reconcile_save_recovery(0));

        assert_eq!(app.save_recoveries.len(), 1);
        assert_eq!(app.save_recoveries[0].destination, second);
        assert!(app.pending_recovery_reconciliation.is_none());
        assert_eq!(app.error_msg.as_deref(), Some(SAVE_RECOVERY_BLOCK_MESSAGE));
        assert_eq!(fs::read(first)?, b"first bytes");
        assert_eq!(
            fs::read(&app.save_recoveries[0].destination)?,
            b"second bytes"
        );
        assert!(app.reconcile_save_recovery(0));
        assert!(app.save_recoveries.is_empty());
        assert!(app.error_msg.is_none());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_unix_recovery_path_copy_is_exact_and_reversible() {
        use std::os::unix::ffi::OsStringExt as _;

        let path = PathBuf::from(std::ffi::OsString::from_vec(vec![
            b'n', b'o', b't', b'e', b'-', 0xff,
        ]));
        let copied = recovery_path_clipboard_text(&path);

        assert_eq!(copied, "unix-path-bytes:6e6f74652dff");
        assert!(!copied.contains('\u{fffd}'));
    }

    #[cfg(windows)]
    #[test]
    fn non_unicode_windows_recovery_path_copy_is_exact_and_reversible() {
        use std::os::windows::ffi::OsStringExt as _;

        let path = PathBuf::from(std::ffi::OsString::from_wide(&[
            0x006e, 0x006f, 0x0074, 0x0065, 0xd800,
        ]));
        let copied = recovery_path_clipboard_text(&path);

        assert_eq!(copied, "windows-path-utf16:006e006f00740065d800");
        assert!(!copied.contains('\u{fffd}'));
    }

    #[test]
    fn recovery_notice_stays_visible_until_explicit_dismissal() {
        let mut app = NoterApp {
            save_recoveries: vec![active_test_recovery(
                PathBuf::from("uncertain.txt"),
                "Inspect the retained recovery artifact.",
            )],
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let initial = context.run_ui(ui_input(800.0, 240.0, 0.0), |ui| {
            app.show_save_recovery_notice(ui);
        });
        let dismiss_position =
            text_position(&rendered_text(&initial), "Dismiss notice") + egui::vec2(4.0, 4.0);

        let _ = context.run_ui(click_input(800.0, 240.0, 1.0, dismiss_position), |ui| {
            app.show_save_recovery_notice(ui);
        });

        assert!(!app.save_recoveries[0].notice_pending);
        assert!(app.save_is_blocked());
    }

    #[test]
    fn recovery_notice_exposes_destination_and_explicit_reconciliation() {
        let path = PathBuf::from("private-notes").join("uncertain.txt");
        let mut app = NoterApp {
            save_recoveries: vec![active_test_recovery(
                path,
                "Inspect the retained recovery artifact.",
            )],
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let initial = context.run_ui(ui_input(900.0, 320.0, 0.0), |ui| {
            let _ = show_save_recovery_records(ui, &app.save_recoveries, false);
        });
        let text = rendered_text(&initial);

        assert!(
            text.iter()
                .any(|(value, _)| value.contains("private-notes"))
        );
        assert!(
            text.iter()
                .any(|(value, _)| value == "Copy Destination Path")
        );
        let reconcile_position = text_position(&text, "Reconcile...") + egui::vec2(4.0, 4.0);

        let mut requested = None;
        let _ = context.run_ui(click_input(900.0, 320.0, 1.0, reconcile_position), |ui| {
            requested = show_save_recovery_records(ui, &app.save_recoveries, false);
        });
        app.pending_recovery_reconciliation = requested;

        assert_eq!(app.pending_recovery_reconciliation, Some(0));
        assert_eq!(app.save_recoveries.len(), 1);

        let confirmation_context = egui::Context::default();
        let confirmation = confirmation_context.run_ui(ui_input(900.0, 420.0, 0.0), |ui| {
            let _ = show_save_recovery_reconciliation_contents(ui, &app.save_recoveries[0]);
        });
        let modal_text = rendered_text(&confirmation);
        assert!(
            modal_text
                .iter()
                .any(|(value, _)| value == "Copy Destination Path"),
            "{modal_text:?}"
        );
        assert!(
            modal_text
                .iter()
                .any(|(value, _)| { value.contains("Inspect the retained recovery artifact.") })
        );
        assert!(
            modal_text
                .iter()
                .any(|(value, _)| { value == "I Have Reconciled This Outcome" })
        );
    }

    #[test]
    fn discarding_before_new_replaces_the_dirty_document() {
        let mut app = NoterApp {
            text: "unsaved text".to_owned(),
            ..NoterApp::default()
        };
        app.document
            .replace_text(&app.text)
            .expect("the test edit should advance the document revision");
        arrange_pending_intent(&mut app, PendingAbandonAction::New);
        let context = egui::Context::default();

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            app.discard_pending_abandon(ui.ctx());
        });

        assert!(app.text.is_empty());
        assert_eq!(app.document.rope().len_bytes(), 0);
        assert!(!app.document.is_dirty());
        assert!(app.lifecycle.pending_intent().is_none());
        assert!(!app.lifecycle.close_authorized(app.document.revision()));
    }

    #[test]
    fn incomplete_open_after_discard_rearms_dirty_recovery() {
        use crate::crash_recovery::CrashRecoverySession;

        let directory = tempdir().expect("tempdir");
        let mut app = NoterApp {
            text: "unsaved text".to_owned(),
            crash_recovery: CrashRecoverySession::open_at(directory.path()),
            ..NoterApp::default()
        };
        app.document
            .replace_text(&app.text)
            .expect("the test edit should advance the document revision");
        app.crash_recovery.on_discarded();
        let context = egui::Context::default();

        app.rearm_recovery_after_incomplete_abandon(PendingAbandonAction::Open, &context);

        assert!(app.document.is_dirty());
        assert!(app.crash_recovery.next_persist_delay().is_some());
    }

    #[test]
    fn saving_before_close_respects_platform_follow_up() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("note.txt");
        fs::write(&path, b"saved text")?;
        let mut document = Document::from_path(&path)?;
        document.replace_text("new text")?;
        let mut app = NoterApp {
            text: "new text".to_owned(),
            document,
            ..NoterApp::default()
        };
        arrange_pending_intent(&mut app, PendingAbandonAction::Quit);
        let context = egui::Context::default();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            app.save_pending_abandon(ui.ctx());
        });
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert_eq!(fs::read(&path)?, b"new text");
        assert!(!app.document.is_dirty());
        assert!(app.lifecycle.pending_intent().is_none());
        #[cfg(windows)]
        {
            assert!(app.error_msg.is_none());
            assert!(app.lifecycle.close_authorized(app.document.revision()));
            assert!(commands.contains(&egui::ViewportCommand::Close));
        }
        #[cfg(unix)]
        {
            assert!(!app.lifecycle.close_authorized(app.document.revision()));
            assert!(!commands.contains(&egui::ViewportCommand::Close));
            assert!(
                app.error_msg
                    .as_deref()
                    .is_some_and(|message| message.contains("displaced recovery artifact"))
            );
        }
        Ok(())
    }

    #[test]
    fn pending_action_waits_while_the_document_is_dirty() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        arrange_pending_intent(&mut app, PendingAbandonAction::New);
        let context = egui::Context::default();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            app.continue_pending_abandon_if_clean(ui.ctx());
        });
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert!(commands.is_empty());
        assert!(app.document.is_dirty());
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::New)
        );
    }

    #[test]
    fn external_rewrite_prompts_and_keep_editing_preserves_save_conflict()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("shared.txt");
        fs::write(&path, b"original")?;
        let document = Document::from_path(&path)?;
        let mut app = NoterApp {
            text: String::from(document.rope()),
            document,
            crash_recovery: CrashRecoverySession::open_at(directory.path().join("recovery")),
            ..NoterApp::default()
        };
        fs::write(&path, b"external revision")?;

        let context = egui::Context::default();
        let mut focused = egui::RawInput::default();
        focused.events.push(egui::Event::WindowFocused(true));
        let _ = context.run_ui(focused, |ui| {
            app.maybe_inspect_external_change(ui.ctx());
        });
        assert!(app.conflict.is_prompting());
        assert_eq!(
            app.conflict.prompt_kind(),
            Some(ExternalChangeKind::ContentOrIdentityChanged)
        );
        assert!(app.ordinary_save_is_blocked());
        assert!(!app.save_is_blocked());
        assert!(app.external_memory_at_risk);
        assert!(app.has_unsaved_state());
        assert_eq!(
            persistence_status_label(&app.document, app.external_memory_at_risk),
            "Modified"
        );
        assert!(app.window_title().contains("shared.txt*"));

        let close_output = context.run_ui(egui::RawInput::default(), |ui| {
            app.request_close(ui.ctx());
        });
        let close_commands = &close_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;
        assert!(close_commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(!close_commands.contains(&egui::ViewportCommand::Close));
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::Quit)
        );
        app.cancel_pending_abandon();

        app.crash_recovery
            .force_due_persist_for_test(&app.document, app.selection);

        let effect = app
            .conflict
            .reduce(ConflictCommand::Decide(ConflictDecision::KeepEditing));
        assert_eq!(effect, ConflictEffect::None);
        assert!(!app.conflict.is_prompting());
        assert!(!app.ordinary_save_is_blocked());

        let outcome = app.document.save()?;
        assert!(matches!(outcome, SaveOutcome::Conflict { .. }));
        assert_eq!(fs::read(&path)?, b"external revision");
        assert_eq!(app.text, "original");
        drop(app);

        let recovered = CrashRecoverySession::open_at(directory.path().join("recovery"));
        let offer = recovered
            .active_offer()
            .expect("the retained in-memory disk version should be recoverable");
        assert_eq!(offer.metadata().content_len(), b"original".len());
        Ok(())
    }

    #[test]
    fn clean_external_reload_accepts_disk_without_dirty_lifecycle()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let recovery_path = directory.path().join("recovery");
        let path = directory.path().join("clean-reload.txt");
        fs::write(&path, b"original")?;
        let document = Document::from_path(&path)?;
        let mut app = NoterApp {
            text: String::from(document.rope()),
            document,
            crash_recovery: CrashRecoverySession::open_at(&recovery_path),
            ..NoterApp::default()
        };
        fs::write(&path, b"external revision")?;
        let context = egui::Context::default();

        inspect_external_change_for_test(&mut app, &context);
        assert!(!app.document.is_dirty());
        assert!(app.external_memory_at_risk);
        app.crash_recovery
            .force_due_persist_for_test(&app.document, app.selection);
        app.defer_input_events(vec![egui::Event::Text("stale editor input".to_owned())]);
        assert!(!app.deferred_input_events.is_empty());

        request_external_reload_for_test(&mut app, &context);

        assert_eq!(app.text, "external revision");
        assert!(app.deferred_input_events.is_empty());
        assert!(!app.markdown_editor.has_deferred_input());
        assert!(!app.document.is_dirty());
        assert!(app.lifecycle.pending_intent().is_none());
        assert!(!app.conflict.is_prompting());
        assert!(!app.external_memory_at_risk);
        let _ = context.run_ui(ui_input(800.0, 600.0, 0.1), |ui| {
            app.restore_deferred_input(ui);
            app.show_editor(ui);
        });
        assert_eq!(app.text, "external revision");
        assert_eq!(String::from(app.document.rope()), "external revision");
        drop(app);

        let recovered = CrashRecoverySession::open_at(&recovery_path);
        assert!(
            recovered.active_offer().is_none(),
            "successful Reload must retire the explicitly discarded retained copy"
        );
        Ok(())
    }

    #[test]
    fn focus_regain_inspection_precedes_same_frame_destructive_shortcut()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("focus-race.txt");
        fs::write(&path, b"retained memory")?;
        let document = Document::from_path(&path)?;
        let mut app = NoterApp {
            text: String::from(document.rope()),
            document,
            ..NoterApp::default()
        };
        fs::write(&path, b"external revision")?;
        let context = egui::Context::default();
        theme::configure_styles(&context);
        let command = egui::Modifiers {
            ctrl: true,
            command: true,
            ..egui::Modifiers::NONE
        };
        let mut input = ui_input(1_200.0, 760.0, 0.0);
        input.events = vec![
            egui::Event::WindowFocused(true),
            key_press(command, egui::Key::N),
        ];

        let _ = context.run_ui(input, |ui| app.render_frame(ui));

        assert_eq!(app.text, "retained memory");
        assert_eq!(app.document.path(), Some(path.as_path()));
        assert!(app.external_memory_at_risk);
        assert!(app.conflict.is_prompting());
        assert!(app.lifecycle.pending_intent().is_none());
        Ok(())
    }

    #[test]
    fn newly_opened_external_change_modal_defers_same_frame_document_input()
    -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            ("text", egui::Event::Text("T".to_owned()), "originalT"),
            ("paste", egui::Event::Paste("P".to_owned()), "originalP"),
            (
                "IME commit",
                egui::Event::Ime(egui::ImeEvent::Commit("漢".to_owned())),
                "original漢",
            ),
        ];

        for (label, event, expected) in cases {
            let directory = tempdir()?;
            let path = directory.path().join(format!("{label}.txt"));
            fs::write(&path, b"original")?;
            let document = Document::from_path(&path)?;
            let selection = Selection::caret("original".len());
            let mut app = NoterApp {
                text: String::from(document.rope()),
                document,
                selection,
                pending_selection_restore: Some(selection),
                crash_recovery: CrashRecoverySession::open_at(directory.path().join("recovery")),
                ..NoterApp::default()
            };
            let context = egui::Context::default();
            theme::configure_styles(&context);
            let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.0), |ui| {
                app.render_frame(ui);
            });
            fs::write(&path, b"external revision")?;
            let mut input = ui_input(1_200.0, 760.0, 0.1);
            input.events = vec![egui::Event::WindowFocused(true), event];

            let _ = context.run_ui(input, |ui| app.render_frame(ui));

            assert!(app.conflict.is_prompting(), "{label}");
            assert_eq!(app.text, "original", "{label}");
            assert!(!app.deferred_input_events.is_empty(), "{label}");
            let effect = app
                .conflict
                .reduce(ConflictCommand::Decide(ConflictDecision::KeepEditing));
            app.apply_conflict_effect(effect, &context);
            let _ = context.run_ui(ui_input(1_200.0, 760.0, 0.2), |ui| {
                app.render_frame(ui);
            });

            assert_eq!(app.text, expected, "{label}");
            assert_eq!(String::from(app.document.rope()), expected, "{label}");
            assert!(app.document.is_dirty(), "{label}");
            assert!(app.deferred_input_events.is_empty(), "{label}");
        }
        Ok(())
    }

    #[test]
    fn external_retention_revokes_same_revision_close_authorization()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("closing-race.txt");
        fs::write(&path, b"retained memory")?;
        let document = Document::from_path(&path)?;
        let revision = document.revision();
        let mut app = NoterApp {
            text: String::from(document.rope()),
            document,
            ..NoterApp::default()
        };
        let context = egui::Context::default();
        let first_close = context.run_ui(egui::RawInput::default(), |ui| {
            app.request_close(ui.ctx());
        });
        assert!(app.lifecycle.close_authorized(revision));
        assert!(
            first_close
                .viewport_output
                .get(&egui::ViewportId::ROOT)
                .expect("the root viewport should have output")
                .commands
                .contains(&egui::ViewportCommand::Close)
        );

        fs::write(&path, b"external revision")?;
        let mut focused = egui::RawInput::default();
        focused.events.push(egui::Event::WindowFocused(true));
        let inspected = context.run_ui(focused, |ui| {
            app.maybe_inspect_external_change(ui.ctx());
        });
        let commands = &inspected
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert!(commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(!app.lifecycle.close_authorized(revision));
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::Quit)
        );
        assert!(app.external_memory_at_risk);
        Ok(())
    }

    #[test]
    fn dirty_external_reload_requires_lifecycle_and_cancel_retains_conflict()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("dirty-reload.txt");
        fs::write(&path, b"original")?;
        let mut document = Document::from_path(&path)?;
        document.replace_text("local edit")?;
        let mut app = NoterApp {
            text: "local edit".to_owned(),
            document,
            ..NoterApp::default()
        };
        fs::write(&path, b"external revision")?;
        let context = egui::Context::default();

        inspect_external_change_for_test(&mut app, &context);
        request_external_reload_for_test(&mut app, &context);

        assert_eq!(app.text, "local edit");
        assert_eq!(fs::read(&path)?, b"external revision");
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::Reload)
        );
        assert_eq!(
            app.conflict.prompt_kind(),
            Some(ExternalChangeKind::ContentOrIdentityChanged)
        );

        app.cancel_pending_abandon();

        assert!(app.lifecycle.pending_intent().is_none());
        assert!(app.document.is_dirty());
        assert_eq!(app.text, "local edit");
        assert_eq!(
            app.conflict.prompt_kind(),
            Some(ExternalChangeKind::ContentOrIdentityChanged)
        );
        assert!(app.external_memory_at_risk);
        Ok(())
    }

    #[test]
    fn save_before_dirty_external_reload_uses_durable_conflict_check()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("save-before-reload.txt");
        fs::write(&path, b"original")?;
        let mut document = Document::from_path(&path)?;
        document.replace_text("local edit")?;
        let mut app = NoterApp {
            text: "local edit".to_owned(),
            document,
            ..NoterApp::default()
        };
        fs::write(&path, b"external revision")?;
        let context = egui::Context::default();

        inspect_external_change_for_test(&mut app, &context);
        request_external_reload_for_test(&mut app, &context);
        assert!(!app.pending_abandon_save_is_blocked());

        app.save_pending_abandon(&context);

        assert_eq!(fs::read(&path)?, b"external revision");
        assert_eq!(app.text, "local edit");
        assert!(app.document.is_dirty());
        assert!(app.lifecycle.pending_intent().is_none());
        assert_eq!(
            app.conflict.prompt_kind(),
            Some(ExternalChangeKind::ContentOrIdentityChanged)
        );
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("Save stopped because the destination changed")
        }));
        Ok(())
    }

    #[test]
    fn failed_dirty_external_reload_retains_conflict_text_and_recovery()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let recovery_path = directory.path().join("recovery");
        let path = directory.path().join("failed-dirty-reload.txt");
        fs::write(&path, b"original")?;
        let mut document = Document::from_path(&path)?;
        document.replace_text("local edit")?;
        let mut app = NoterApp {
            text: "local edit".to_owned(),
            document,
            crash_recovery: CrashRecoverySession::open_at(&recovery_path),
            ..NoterApp::default()
        };
        fs::remove_file(&path)?;
        let context = egui::Context::default();

        inspect_external_change_for_test(&mut app, &context);
        request_external_reload_for_test(&mut app, &context);
        app.discard_pending_abandon(&context);

        assert_eq!(app.text, "local edit");
        assert!(app.document.is_dirty());
        assert!(app.lifecycle.pending_intent().is_none());
        assert_eq!(
            app.conflict.prompt_kind(),
            Some(ExternalChangeKind::Deleted)
        );
        assert!(app.external_memory_at_risk);
        assert!(
            app.error_msg
                .as_deref()
                .is_some_and(|message| { message.contains("Failed to open file") })
        );
        app.crash_recovery
            .force_due_persist_for_test(&app.document, app.selection);
        drop(app);

        let recovered = CrashRecoverySession::open_at(&recovery_path);
        let offer = recovered
            .active_offer()
            .expect("the failed reload should keep the local edit recoverable");
        assert_eq!(offer.metadata().content_len(), b"local edit".len());
        Ok(())
    }

    #[test]
    fn failed_external_reload_keeps_the_retained_copy_protected()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let recovery_path = directory.path().join("recovery");
        let path = directory.path().join("vanished.txt");
        fs::write(&path, b"present")?;
        let document = Document::from_path(&path)?;
        let mut app = NoterApp {
            text: String::from(document.rope()),
            document,
            crash_recovery: CrashRecoverySession::open_at(&recovery_path),
            ..NoterApp::default()
        };
        fs::remove_file(&path)?;

        let context = egui::Context::default();
        inspect_external_change_for_test(&mut app, &context);
        assert_eq!(
            app.conflict.prompt_kind(),
            Some(ExternalChangeKind::Deleted)
        );
        app.crash_recovery
            .force_due_persist_for_test(&app.document, app.selection);
        let records_dir = recovery_path.join("records");
        let record_path = fs::read_dir(&records_dir)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|candidate| {
                candidate
                    .extension()
                    .is_some_and(|extension| extension == "rec")
            })
            .expect("the forced persist must create one recovery record");
        let encoded_record = fs::read(&record_path)?;

        request_external_reload_for_test(&mut app, &context);
        // Reload of a missing path surfaces an open failure and leaves recovery
        // free of a silent overwrite.
        assert_eq!(app.text, "present");
        assert_eq!(
            app.conflict.prompt_kind(),
            Some(ExternalChangeKind::Deleted)
        );
        assert!(app.external_memory_at_risk);
        assert!(app.has_unsaved_state());
        assert!(
            app.error_msg
                .as_deref()
                .is_some_and(|message| message.contains("Failed to open file"))
        );
        assert_eq!(
            fs::read(&record_path)?,
            encoded_record,
            "failed Reload must leave the already-durable exact record untouched"
        );
        drop(app);

        let recovered = CrashRecoverySession::open_at(&recovery_path);
        let offer = recovered
            .active_offer()
            .expect("failed Reload must preserve the already-durable retained clean copy");
        assert_eq!(offer.metadata().content_len(), b"present".len());
        assert_eq!(
            offer.metadata().content_checksum(),
            noter::core::save::ContentFingerprint::from_bytes(b"present")
        );
        assert_eq!(offer.metadata().selection(), Selection::caret(0));
        Ok(())
    }

    #[test]
    fn failed_reload_error_survives_recovery_unavailability()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let path = directory.path().join("missing-on-reload.txt");
        fs::write(&path, b"saved")?;
        let mut document = Document::from_path(&path)?;
        document.replace_text("local edit")?;
        let blocked_recovery = directory.path().join("blocked-recovery-root");
        fs::write(&blocked_recovery, b"not a directory")?;
        let mut app = NoterApp {
            text: "local edit".to_owned(),
            document,
            crash_recovery: CrashRecoverySession::open_at(blocked_recovery),
            ..NoterApp::default()
        };
        assert!(app.crash_recovery.is_unavailable());
        fs::remove_file(&path)?;
        let context = egui::Context::default();

        app.request_reload(&context);
        app.discard_pending_abandon(&context);

        assert_eq!(app.text, "local edit");
        assert!(app.document.is_dirty());
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("Failed to open file") && message != RECOVERY_UNAVAILABLE_MESSAGE
        }));
        Ok(())
    }

    #[test]
    fn untitled_documents_skip_external_inspection() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("scratch")
            .expect("fixture edit should advance the revision");
        app.text = "scratch".to_owned();
        let context = egui::Context::default();
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            app.maybe_inspect_external_change(ui.ctx());
        });
        assert!(!app.conflict.is_prompting());
        assert!(!app.ordinary_save_is_blocked());
    }

    #[test]
    fn post_save_warning_stops_the_pending_action_for_review() {
        let mut app = NoterApp {
            error_msg: Some(
                "Saved, but follow-up is required: inspect retained artifact".to_owned(),
            ),
            ..NoterApp::default()
        };
        arrange_saving_intent(&mut app, PendingAbandonAction::Quit);
        let context = egui::Context::default();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            app.continue_pending_abandon_if_clean(ui.ctx());
        });
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert!(commands.is_empty());
        assert!(app.lifecycle.pending_intent().is_none());
        assert!(!app.lifecycle.close_authorized(app.document.revision()));
        assert!(app.error_msg.is_some());
    }

    #[test]
    fn same_frame_editor_input_is_recorded_before_native_close_decision() {
        let mut app = NoterApp::default();
        let context = egui::Context::default();
        context.memory_mut(|memory| memory.request_focus(app.editor_id()));
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Text("unsaved".to_owned()));
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);

        let output = context.run_ui(input, |ui| app.render_frame(ui));
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert_eq!(app.text, "unsaved");
        assert_eq!(String::from(app.document.rope()), "unsaved");
        assert!(app.document.is_dirty());
        assert!(commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(!commands.contains(&egui::ViewportCommand::Close));
        assert_eq!(
            app.lifecycle.pending_intent(),
            Some(PendingAbandonAction::Quit)
        );
    }

    #[test]
    fn noncommitted_cleanup_failure_is_visible() {
        use noter::core::revision::Revision;
        use noter::core::save::{SaveStage, StorageError};

        let mut app = NoterApp::default();
        let reservation =
            test_save_recovery_reservation(&mut app, SaveAttempt::SaveAs(PathBuf::from("new.txt")));
        app.handle_save_result(
            Ok(SaveOutcome::NotCommitted {
                revision: Revision::INITIAL,
                error: StorageError::new(SaveStage::Write, "primary failure"),
                cleanup_error: Some(StorageError::new(
                    SaveStage::Cleanup,
                    "private artifact was preserved",
                )),
            }),
            reservation,
        );

        let message = app.error_msg.expect("the failure must be visible");
        assert!(message.contains("primary failure"));
        assert!(message.contains("Cleanup also failed"));
        assert!(message.contains("private artifact was preserved"));
        assert!(app.save_recoveries.is_empty());
    }

    #[test]
    fn unknown_commit_recovery_artifact_is_visible() {
        use noter::core::revision::Revision;
        use noter::core::save::{SaveStage, StorageError};

        let mut app = NoterApp::default();
        let reservation = test_save_recovery_reservation(
            &mut app,
            SaveAttempt::Current(PathBuf::from("note.txt")),
        );
        app.handle_save_result(
            Ok(SaveOutcome::CommitStateUnknown {
                revision: Revision::INITIAL,
                error: StorageError::new(SaveStage::Reconcile, "destination state differs"),
                recovery_artifact: StorageError::new(
                    SaveStage::Cleanup,
                    "inspect `.noter-save-recovery.tmp` before retrying",
                ),
            }),
            reservation,
        );

        assert!(app.error_msg.is_none());
        let recovery = app
            .save_recoveries
            .first()
            .expect("the recovery action must be retained");
        let message = &recovery.message;
        assert!(message.contains("destination state differs"));
        assert!(message.contains(".noter-save-recovery.tmp"));
        assert!(message.contains("before retrying"));
        assert!(recovery.notice_pending);
        assert_eq!(recovery.destination, PathBuf::from("note.txt"));
        assert_eq!(recovery.destination_label, "note.txt");
    }

    #[test]
    fn committed_save_warnings_remain_visible() {
        use noter::core::revision::Revision;
        use noter::core::save::{
            ContentFingerprint, Durability, FileChangeToken, FileIdentity, FileObservation,
            SaveStage, SaveWarnings, StorageError,
        };

        let warning = StorageError::new(SaveStage::SyncParent, "directory sync failed");
        let observation = FileObservation::new(
            FileIdentity::new(1, 2),
            ContentFingerprint::from_bytes(b"saved"),
            5,
            1,
            FileChangeToken::new(3, 4),
        );
        let mut app = NoterApp::default();
        let reservation = test_save_recovery_reservation(
            &mut app,
            SaveAttempt::Current(PathBuf::from("note.txt")),
        );

        app.handle_save_result(
            Ok(SaveOutcome::Committed {
                revision: Revision::INITIAL,
                durability: Durability::FileSynced,
                observation,
                warnings: SaveWarnings::new(Vec::new(), vec![warning]),
            }),
            reservation,
        );

        assert_eq!(
            app.error_msg.as_deref(),
            Some("Saved, but follow-up is required: SyncParent failed: directory sync failed")
        );
    }

    #[test]
    fn clean_close_is_forwarded_to_the_viewport() {
        let mut app = NoterApp::default();
        let context = egui::Context::default();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            app.request_close(ui.ctx());
        });
        let commands = &output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .expect("the root viewport should have output")
            .commands;

        assert!(commands.contains(&egui::ViewportCommand::Close));
        assert!(!commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(app.error_msg.is_none());
    }

    #[test]
    fn hard_link_save_is_available_only_after_gui_confirmation()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let selected = directory.path().join("selected.txt");
        let other_link = directory.path().join("other.txt");
        fs::write(&selected, b"shared original")?;
        fs::hard_link(&selected, &other_link)?;

        let mut document = Document::from_path(&selected)?;
        document.replace_text("selected replacement")?;
        let mut app = NoterApp {
            text: "selected replacement".to_owned(),
            document,
            ..NoterApp::default()
        };

        app.do_save();

        assert!(matches!(
            app.pending_hard_link_save,
            Some(PendingHardLinkSave::Current { link_count, .. }) if link_count >= 2
        ));
        assert_eq!(fs::read(&selected)?, b"shared original");
        assert_eq!(fs::read(&other_link)?, b"shared original");

        let context = egui::Context::default();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            app.show_hard_link_confirmation(ui.ctx());
        });
        assert!(!output.shapes.is_empty());

        app.confirm_pending_hard_link_save();

        assert!(app.pending_hard_link_save.is_none());
        assert_eq!(fs::read(&selected)?, b"selected replacement");
        assert_eq!(fs::read(&other_link)?, b"shared original");
        assert!(!app.document.is_dirty());
        Ok(())
    }

    #[test]
    fn hard_link_save_as_confirmation_adopts_only_the_selected_entry()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let selected = directory.path().join("selected.txt");
        let other_link = directory.path().join("other.txt");
        fs::write(&selected, b"shared original")?;
        fs::hard_link(&selected, &other_link)?;

        let mut app = NoterApp {
            text: "new document".to_owned(),
            ..NoterApp::default()
        };
        app.document.replace_text(&app.text)?;

        app.do_save_as_to(selected.clone());

        assert!(matches!(
            app.pending_hard_link_save,
            Some(PendingHardLinkSave::SaveAs { link_count, .. }) if link_count >= 2
        ));
        assert!(app.document.path().is_none());

        app.confirm_pending_hard_link_save();

        assert!(app.pending_hard_link_save.is_none());
        assert_eq!(app.document.path(), Some(selected.as_path()));
        assert_eq!(fs::read(&selected)?, b"new document");
        assert_eq!(fs::read(&other_link)?, b"shared original");
        assert!(!app.document.is_dirty());
        Ok(())
    }

    #[test]
    fn hard_link_save_as_confirmation_rejects_a_rebound_selected_entry()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let selected = directory.path().join("selected.txt");
        let other_link = directory.path().join("other.txt");
        fs::write(&selected, b"shared original")?;
        fs::hard_link(&selected, &other_link)?;

        let mut app = NoterApp {
            text: "new document".to_owned(),
            ..NoterApp::default()
        };
        app.document.replace_text(&app.text)?;
        app.do_save_as_to(selected.clone());
        assert!(app.pending_hard_link_save.is_some());

        fs::remove_file(&selected)?;
        fs::write(&selected, b"external replacement")?;
        app.confirm_pending_hard_link_save();

        assert!(app.pending_hard_link_save.is_none());
        assert_eq!(fs::read(&selected)?, b"external replacement");
        assert_eq!(fs::read(&other_link)?, b"shared original");
        assert!(app.document.path().is_none());
        assert!(app.document.is_dirty());
        assert!(
            app.error_msg
                .as_deref()
                .is_some_and(|message| message.contains("destination changed"))
        );
        Ok(())
    }

    #[test]
    fn save_as_to_single_link_destination_commits_without_confirmation()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let selected = directory.path().join("selected.txt");
        fs::write(&selected, b"previous document")?;

        let mut app = NoterApp {
            text: "replacement document".to_owned(),
            ..NoterApp::default()
        };
        app.document.replace_text(&app.text)?;

        app.do_save_as_to(selected.clone());

        assert!(app.pending_hard_link_save.is_none());
        assert_eq!(app.document.path(), Some(selected.as_path()));
        assert_eq!(fs::read(&selected)?, b"replacement document");
        assert!(!app.document.is_dirty());
        #[cfg(windows)]
        assert!(app.error_msg.is_none());
        #[cfg(unix)]
        assert!(
            app.error_msg
                .as_deref()
                .is_some_and(|message| message.contains("displaced recovery artifact"))
        );
        Ok(())
    }

    #[test]
    fn explicit_reconciliation_is_required_before_saving_to_any_destination()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let original = directory.path().join("original.txt");
        let preserved = directory.path().join("preserved.txt");
        fs::write(&original, b"original")?;
        let mut document = Document::from_path(&original)?;
        document.replace_text("first preserved revision")?;
        let mut app = NoterApp {
            text: "first preserved revision".to_owned(),
            document,
            save_recoveries: vec![active_test_recovery(
                original.clone(),
                "Inspect the retained recovery artifact.",
            )],
            ..NoterApp::default()
        };

        app.do_save_as_to(preserved.clone());

        assert_eq!(app.document.path(), Some(original.as_path()));
        assert_eq!(app.save_recoveries.len(), 1);
        assert!(app.save_recoveries[0].notice_pending);
        assert!(!preserved.exists());
        assert_eq!(fs::read(&original)?, b"original");
        assert!(app.document.is_dirty());
        assert_eq!(app.error_msg.as_deref(), Some(SAVE_RECOVERY_BLOCK_MESSAGE));

        assert!(app.reconcile_save_recovery(0));
        app.do_save_as_to(preserved.clone());

        assert_eq!(app.document.path(), Some(preserved.as_path()));
        assert_eq!(fs::read(&preserved)?, b"first preserved revision");
        assert_eq!(fs::read(&original)?, b"original");
        assert!(!app.document.is_dirty());
        Ok(())
    }

    #[test]
    fn unresolved_recovery_blocks_hard_link_save_before_confirmation()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempdir()?;
        let original = directory.path().join("original.txt");
        let selected = directory.path().join("selected.txt");
        let other_link = directory.path().join("other.txt");
        fs::write(&original, b"original")?;
        fs::write(&selected, b"shared")?;
        fs::hard_link(&selected, &other_link)?;
        let mut document = Document::from_path(&original)?;
        document.replace_text("first preserved revision")?;
        let mut app = NoterApp {
            text: "first preserved revision".to_owned(),
            document,
            save_recoveries: vec![active_test_recovery(
                original.clone(),
                "Inspect the retained recovery artifact.",
            )],
            ..NoterApp::default()
        };

        app.do_save_as_to(selected.clone());
        assert!(app.pending_hard_link_save.is_none());
        assert_eq!(app.document.path(), Some(original.as_path()));
        assert_eq!(fs::read(&selected)?, b"shared");
        assert_eq!(fs::read(&other_link)?, b"shared");
        assert!(app.document.is_dirty());

        assert!(app.reconcile_save_recovery(0));
        app.do_save_as_to(selected.clone());
        assert!(matches!(
            app.pending_hard_link_save,
            Some(PendingHardLinkSave::SaveAs { .. })
        ));
        app.confirm_pending_hard_link_save();

        assert_eq!(app.document.path(), Some(selected.as_path()));
        assert_eq!(fs::read(&selected)?, b"first preserved revision");
        assert_eq!(fs::read(&other_link)?, b"shared");
        assert!(app.save_recoveries.is_empty());
        assert!(!app.document.is_dirty());
        Ok(())
    }
}
