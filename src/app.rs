use std::path::PathBuf;

use eframe::egui;
use noter::core::document::{Document, PreparedSaveAs};
use noter::core::markdown::count_markdown_diagnostics;
use noter::core::revision::Revision;
use noter::core::save::SaveOutcome;
use noter::error::NoterError;

use crate::markdown_ui::{MarkdownEditor, MarkdownProjectionLimit, markdown_projection_limit};
use crate::theme::{self, AppTheme, THEME_STORAGE_KEY};

const EDITOR_ID_SALT: &str = "noter-document-editor";
const ABOUT_SUMMARY: &str = "A focused editor for plain text and Markdown files.";
const ABOUT_MARKDOWN_STATUS: &str = "Markdown Mode provides a formatted, direct editing surface while keeping ordinary Markdown source authoritative on disk.";
const ABOUT_PRIVACY: &str = "Noter has no accounts, telemetry, or background network activity.";
const ABOUT_LINK_BEHAVIOR: &str = "The project link opens in your default browser.";
const UPDATE_STATUS: &str = "No Noter release has been published yet. This source build cannot safely self-update without verified release artifacts.";
const RELEASES_URL: &str = "https://github.com/blisspixel/noter/releases";
const UNCERTAIN_SAVE_ABANDON_GUIDANCE: &str = "Cancel this dialog, then use Save As to preserve the current text at another path or reconcile the recovery state.";
const MENU_BAR_HEIGHT: f32 = 30.0;
const EDITOR_TOOLBAR_HEIGHT: f32 = 40.0;
const STATUS_BAR_HEIGHT: f32 = 26.0;
const EXPANDED_TOP_CONTROLS_MIN_WIDTH: f32 = 600.0;
const EXPANDED_TOP_CONTROLS_WIDTH: f32 = 326.0;
const COMPACT_TOP_CONTROLS_WIDTH: f32 = 280.0;

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

const fn document_view_button_width(view: DocumentView) -> f32 {
    match view {
        DocumentView::Text => 52.0,
        DocumentView::Markdown => 86.0,
    }
}

#[derive(Default, Debug)]
pub struct LaunchOptions {
    pub initial_path: Option<PathBuf>,
    pub theme: Option<AppTheme>,
    pub view: Option<DocumentView>,
    pub show_updates: bool,
    pub screenshot_path: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FileCommand {
    New,
    Open,
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
enum PendingAbandonAction {
    New,
    Open,
    Quit,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct MarkdownIssueCache {
    document_serial: u64,
    revision: Revision,
    issue_count: usize,
}

impl PendingAbandonAction {
    const fn prompt(self) -> &'static str {
        match self {
            Self::New => "Save your changes before creating a new document?",
            Self::Open => "Save your changes before opening another file?",
            Self::Quit => "Save your changes before closing Noter?",
        }
    }
}

#[derive(Debug)]
enum PendingHardLinkSave {
    Current {
        link_count: u64,
    },
    SaveAs {
        prepared: PreparedSaveAs,
        link_count: u64,
    },
}

impl PendingHardLinkSave {
    const fn link_count(&self) -> u64 {
        match self {
            Self::Current { link_count } | Self::SaveAs { link_count, .. } => *link_count,
        }
    }
}

pub struct NoterApp {
    text: String,
    document: Document,
    view: DocumentView,
    theme: AppTheme,
    markdown_editor: MarkdownEditor,
    document_editor_serial: u64,
    markdown_issue_cache: Option<MarkdownIssueCache>,
    error_msg: Option<String>,
    save_recovery_msg: Option<String>,
    about_open: bool,
    updates_open: bool,
    pending_hard_link_save: Option<PendingHardLinkSave>,
    pending_abandon: Option<PendingAbandonAction>,
    allow_dirty_close: bool,
    #[cfg(feature = "screenshot-qa")]
    screenshot: Option<ScreenshotCapture>,
}

impl Default for NoterApp {
    fn default() -> Self {
        Self {
            text: String::new(),
            document: Document::new(),
            view: DocumentView::Text,
            theme: AppTheme::System,
            markdown_editor: MarkdownEditor::default(),
            document_editor_serial: 0,
            markdown_issue_cache: None,
            error_msg: None,
            save_recovery_msg: None,
            about_open: false,
            updates_open: false,
            pending_hard_link_save: None,
            pending_abandon: None,
            allow_dirty_close: false,
            #[cfg(feature = "screenshot-qa")]
            screenshot: None,
        }
    }
}

impl NoterApp {
    pub fn new(cc: &eframe::CreationContext<'_>, options: LaunchOptions) -> Self {
        theme::configure_styles(&cc.egui_ctx);
        let selected_theme = options
            .theme
            .unwrap_or_else(|| AppTheme::from_storage(cc.storage));
        selected_theme.apply(&cc.egui_ctx);

        let mut app = Self {
            theme: selected_theme,
            updates_open: options.show_updates,
            ..Self::default()
        };
        if let Some(path) = options.initial_path {
            app.open_path(&path, options.view);
        } else if let Some(view) = options.view {
            app.view = view;
        }

        #[cfg(feature = "screenshot-qa")]
        if let Some(path) = options.screenshot_path {
            if app.view == DocumentView::Markdown {
                app.markdown_editor.activate_first_block(&app.text);
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
        match Document::from_path(path) {
            Ok(document) => {
                self.text = String::from(document.rope());
                self.document = document;
                self.advance_document_editor();
                self.markdown_editor.reset();
                self.markdown_issue_cache = None;
                self.error_msg = None;
                self.view = DocumentView::Text;
                self.select_document_view(
                    requested_view.unwrap_or_else(|| preferred_view_for_path(path)),
                );
            }
            Err(error) => {
                self.error_msg = Some(format!("Failed to open file: {error}"));
            }
        }
    }

    fn do_open_unchecked(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            self.open_path(&path, None);
        }
    }

    fn request_open(&mut self) {
        if self.document.is_dirty() {
            self.begin_pending_abandon(PendingAbandonAction::Open);
        } else {
            self.do_open_unchecked();
        }
    }

    fn do_save(&mut self) {
        if self.document.path().is_none() {
            self.do_save_as();
            return;
        }
        if self.restore_save_recovery_message() {
            return;
        }

        match self.document.save() {
            Err(NoterError::HardLinkedTarget(link_count)) => {
                self.pending_hard_link_save = Some(PendingHardLinkSave::Current { link_count });
                self.error_msg = None;
            }
            result => self.handle_save_result(result),
        }
    }

    fn do_save_as(&mut self) {
        if let Some(path) = rfd::FileDialog::new().save_file() {
            self.do_save_as_to(path);
        }
    }

    fn do_save_as_to(&mut self, path: PathBuf) {
        let prepared = match self.document.prepare_save_as(path) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.handle_save_result(Err(error));
                return;
            }
        };
        if let Some(link_count) = prepared
            .hard_link_count()
            .filter(|link_count| *link_count > 1)
        {
            self.pending_hard_link_save = Some(PendingHardLinkSave::SaveAs {
                prepared,
                link_count,
            });
            self.error_msg = None;
            return;
        }
        let result = self.document.save_prepared_as(prepared);
        self.handle_save_result(result);
    }

    fn confirm_pending_hard_link_save(&mut self) {
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
        self.handle_save_result(result);
    }

    fn handle_save_result(&mut self, result: Result<SaveOutcome, NoterError>) {
        self.error_msg = match result {
            Ok(SaveOutcome::Committed { ref warnings, .. }) if warnings.is_empty() => None,
            Ok(SaveOutcome::Committed { warnings, .. }) => {
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
                let message = format!(
                    "Save state is uncertain and must be reconciled before retry: {error}. Recovery follow-up: {recovery_artifact}"
                );
                self.save_recovery_msg = Some(message.clone());
                Some(message)
            }
            Err(error) => Some(format!("Failed to save file: {error}")),
        };
    }

    fn restore_save_recovery_message(&mut self) -> bool {
        let Some(message) = self.save_recovery_msg.as_ref() else {
            return false;
        };
        self.error_msg = Some(message.clone());
        true
    }

    fn start_new_document_unchecked(&mut self) {
        self.text.clear();
        self.document = Document::new();
        self.view = DocumentView::Text;
        self.advance_document_editor();
        self.markdown_editor.reset();
        self.markdown_issue_cache = None;
        self.error_msg = None;
    }

    fn request_new_document(&mut self) {
        if self.document.is_dirty() {
            self.begin_pending_abandon(PendingAbandonAction::New);
        } else {
            self.start_new_document_unchecked();
        }
    }

    fn request_close(&mut self, ctx: &egui::Context) {
        if self.document.is_dirty() && !self.allow_dirty_close {
            self.begin_pending_abandon(PendingAbandonAction::Quit);
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn protect_native_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|input| input.viewport().close_requested())
            && self.document.is_dirty()
            && !self.allow_dirty_close
        {
            self.begin_pending_abandon(PendingAbandonAction::Quit);
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
    }

    fn begin_pending_abandon(&mut self, action: PendingAbandonAction) {
        self.pending_abandon = Some(action);
        let _ = self.restore_save_recovery_message();
    }

    fn cancel_pending_abandon(&mut self) {
        self.pending_abandon = None;
        self.allow_dirty_close = false;
        let _ = self.restore_save_recovery_message();
    }

    fn discard_pending_abandon(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending_abandon.take() else {
            return;
        };
        self.execute_abandon_action(action, ctx);
    }

    fn save_pending_abandon(&mut self, ctx: &egui::Context) {
        if self.pending_abandon.is_none() {
            return;
        }
        self.do_save();
        self.continue_pending_abandon_if_clean(ctx);
    }

    fn continue_pending_abandon_if_clean(&mut self, ctx: &egui::Context) {
        if self.document.is_dirty() || self.pending_hard_link_save.is_some() {
            return;
        }
        if self.error_msg.is_some() {
            self.pending_abandon = None;
            return;
        }
        if let Some(action) = self.pending_abandon.take() {
            self.execute_abandon_action(action, ctx);
        }
    }

    fn execute_abandon_action(&mut self, action: PendingAbandonAction, ctx: &egui::Context) {
        self.allow_dirty_close = false;
        match action {
            PendingAbandonAction::New => self.start_new_document_unchecked(),
            PendingAbandonAction::Open => self.do_open_unchecked(),
            PendingAbandonAction::Quit => {
                self.allow_dirty_close = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn collect_shortcut(ui: &egui::Ui) -> Option<FileCommand> {
        let mut command = None;

        ui.input_mut(|i| {
            for (candidate, shortcut) in FileCommand::SHORTCUTS_IN_PRECEDENCE_ORDER {
                if i.consume_shortcut(&shortcut) {
                    command = Some(candidate);
                    break;
                }
            }
        });

        command
    }

    fn execute_file_command(&mut self, command: FileCommand, ctx: &egui::Context) {
        match command {
            FileCommand::New => self.request_new_document(),
            FileCommand::Open => self.request_open(),
            FileCommand::Save => self.do_save(),
            FileCommand::SaveAs => self.do_save_as(),
            FileCommand::Quit => self.request_close(ctx),
        }
    }

    fn update_title(&self, ctx: &egui::Context) {
        let dirty = if self.document.is_dirty() { "*" } else { "" };
        let title = self.document.path().map_or_else(
            || format!("Untitled{dirty} - Noter"),
            |path| {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                format!("{file_name}{dirty} - Noter")
            },
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    fn show_menu(&mut self, ui: &mut egui::Ui, command: &mut Option<FileCommand>) {
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
                menu_ui.spacing_mut().item_spacing.x = 2.0;
                egui::MenuBar::new().ui(&mut menu_ui, |ui| {
                    ui.menu_button("File", |ui| Self::show_file_menu(ui, command));
                    ui.menu_button("View", |ui| self.show_view_menu(ui));
                    ui.menu_button("Help", |ui| self.show_help_menu(ui));
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
        ui.spacing_mut().item_spacing.x = 4.0;
        if !expanded {
            self.show_document_mode_menu_button(ui);
            let theme_label = format!("Theme: {}", self.theme.label());
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
                    .min_size(egui::vec2(document_view_button_width(view), 28.0)),
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
                self.select_document_view(view);
            }
        }
        ui.separator();
        ui.label(
            egui::RichText::new("Theme")
                .text_style(egui::TextStyle::Button)
                .weak(),
        );
        self.show_theme_menu_button(ui, self.theme.label());
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

    fn show_file_menu(ui: &mut egui::Ui, command: &mut Option<FileCommand>) {
        for candidate in [
            FileCommand::New,
            FileCommand::Open,
            FileCommand::Save,
            FileCommand::SaveAs,
        ] {
            let mut button = egui::Button::new(candidate.label());
            if let Some(shortcut) = candidate.shortcut() {
                button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
            }
            if ui.add(button).clicked() {
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

    fn show_view_menu(&mut self, ui: &mut egui::Ui) {
        ui.label("Mode");
        self.show_document_mode_choices(ui);
        ui.separator();
        ui.label("Theme");
        self.show_theme_choices(ui);
    }

    fn show_document_mode_choices(&mut self, ui: &mut egui::Ui) {
        for view in [DocumentView::Text, DocumentView::Markdown] {
            if ui
                .selectable_label(self.view == view, view.label())
                .clicked()
            {
                self.select_document_view(view);
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
        if view == DocumentView::Markdown
            && let Some(limit) = markdown_projection_limit(&self.text)
        {
            self.error_msg = Some(markdown_limit_message(self.text.len(), limit));
            return;
        }
        if self.view != view {
            self.view = view;
            self.markdown_editor.reset();
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
        self.updates_open = true;
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
        self.about_open = open && !close;
    }

    fn show_updates(&mut self, ctx: &egui::Context) {
        if !self.updates_open {
            return;
        }

        let mut open = self.updates_open;
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
        self.updates_open = open && !close;
    }

    fn show_unsaved_changes_confirmation(&mut self, ctx: &egui::Context) {
        let Some(action) = self.pending_abandon else {
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

        let response =
            egui::Modal::new(egui::Id::new("unsaved-changes-confirmation")).show(ctx, |ui| {
                ui.set_min_width(420.0);
                ui.set_max_width(560.0);
                ui.heading("Save changes?");
                ui.label(format!("{document_name} has unsaved changes."));
                ui.label(action.prompt());
                if let Some(message) = self.save_recovery_msg.as_deref() {
                    ui.separator();
                    ui.colored_label(ui.visuals().error_fg_color, message);
                    ui.label(UNCERTAIN_SAVE_ABANDON_GUIDANCE);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    save = ui
                        .add_enabled(
                            self.save_recovery_msg.is_none(),
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

        if save {
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
        }
    }

    fn show_error(&mut self, ui: &mut egui::Ui) {
        let mut dismiss = false;
        if let Some(error) = self.error_msg.as_deref() {
            egui::Panel::top("error_bar").show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::RED, format!("Error: {error}"));
                    dismiss = ui.button("Dismiss").clicked();
                });
            });
        }
        if dismiss {
            self.error_msg = None;
        }
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
        let markdown_issue_count =
            (self.view == DocumentView::Markdown).then(|| self.markdown_issue_count());
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
                        if let Some(issue_count) = markdown_issue_count {
                            let label = if issue_count == 0 {
                                "Markdown checks: clean".to_owned()
                            } else {
                                format!("Markdown checks: {issue_count}")
                            };
                            ui.label(label);
                            ui.separator();
                        }
                        ui.label(format!("{} Mode", self.view.label()));
                        ui.separator();
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

    fn show_format_toolbar(&mut self, ui: &mut egui::Ui) {
        if self.view != DocumentView::Markdown {
            return;
        }

        egui::Panel::top("editor_toolbar")
            .exact_size(EDITOR_TOOLBAR_HEIGHT)
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    self.markdown_editor.toolbar(ui);
                    if !self.markdown_editor.is_editing() && ui.available_width() >= 180.0 {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.weak("Select content to format");
                        });
                    }
                });
            });
    }

    fn show_editor(&mut self, ui: &mut egui::Ui) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::NONE
                    .fill(ui.visuals().extreme_bg_color)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| {
                let changed = match self.view {
                    DocumentView::Text => self.show_text_editor(ui),
                    DocumentView::Markdown => self.show_markdown_editor(ui),
                };
                if changed {
                    self.record_editor_change();
                }
            });
    }

    fn record_editor_change(&mut self) {
        self.record_editor_change_with(Document::replace_text);
    }

    fn record_editor_change_with(
        &mut self,
        record: impl FnOnce(&mut Document, &str) -> Result<bool, NoterError>,
    ) {
        let result = record(&mut self.document, &self.text);
        match result {
            Ok(_) if self.view == DocumentView::Markdown => {
                let Some(limit) = markdown_projection_limit(&self.text) else {
                    return;
                };
                self.view = DocumentView::Text;
                self.markdown_editor.reset();
                self.error_msg = Some(markdown_limit_message(self.text.len(), limit));
            }
            Ok(_) => {}
            Err(error) => self.restore_editor_after_failed_change(&error),
        }
    }

    fn restore_editor_after_failed_change(&mut self, error: &NoterError) {
        self.text = String::from(self.document.rope());
        self.advance_document_editor();
        self.markdown_editor.reset();
        self.error_msg = Some(format!(
            "Failed to record edit. The editor was restored to the last authoritative text: {error}"
        ));
    }

    fn show_text_editor(&mut self, ui: &mut egui::Ui) -> bool {
        let editor_id = self.editor_id();
        egui::ScrollArea::vertical()
            .show(ui, |ui| {
                ui.add_sized(
                    ui.available_size(),
                    egui::TextEdit::multiline(&mut self.text)
                        .id(editor_id)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .frame(egui::Frame::NONE)
                        .lock_focus(true),
                )
                .changed()
            })
            .inner
    }

    fn show_markdown_editor(&mut self, ui: &mut egui::Ui) -> bool {
        let outcome = egui::ScrollArea::vertical()
            .show(ui, |ui| {
                let content_width = ui.available_width().min(840.0);
                let left_margin = ((ui.available_width() - content_width) / 2.0).max(0.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(left_margin);
                    ui.vertical(|ui| {
                        ui.set_width(content_width);
                        self.markdown_editor.show(ui, &mut self.text)
                    })
                    .inner
                })
                .inner
            })
            .inner;
        if let Some(limit) = outcome.projection_limit() {
            self.view = DocumentView::Text;
            self.markdown_editor.reset();
            self.error_msg = Some(markdown_limit_message(self.text.len(), limit));
        }
        outcome.changed()
    }

    fn render_frame(&mut self, ui: &mut egui::Ui) {
        let mut command = Self::collect_shortcut(ui);
        self.show_menu(ui, &mut command);
        self.show_error(ui);
        self.show_format_toolbar(ui);
        self.show_status(ui);
        self.show_editor(ui);
        if let Some(command) = command
            && self.pending_hard_link_save.is_none()
            && self.pending_abandon.is_none()
        {
            self.execute_file_command(command, ui.ctx());
        }
        self.protect_native_close(ui.ctx());
        self.update_title(ui.ctx());
        self.show_about(ui.ctx());
        self.show_updates(ui.ctx());
        if self.pending_hard_link_save.is_some() {
            self.show_hard_link_confirmation(ui.ctx());
        } else {
            self.show_unsaved_changes_confirmation(ui.ctx());
        }
    }

    fn editor_id(&self) -> egui::Id {
        egui::Id::new((EDITOR_ID_SALT, self.document_editor_serial))
    }

    const fn advance_document_editor(&mut self) {
        self.document_editor_serial = self.document_editor_serial.wrapping_add(1);
    }

    #[cfg(feature = "screenshot-qa")]
    fn advance_screenshot_capture(&mut self, ctx: &egui::Context) {
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

fn markdown_limit_message(byte_len: usize, limit: MarkdownProjectionLimit) -> String {
    format!(
        "Markdown Mode is unavailable because this pre-alpha renderer would exceed its {}. This {byte_len}-byte source remains fully available in Text Mode and can be edited there.",
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

impl eframe::App for NoterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.render_frame(ui);
        #[cfg(feature = "screenshot-qa")]
        self.advance_screenshot_capture(ui.ctx());
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(THEME_STORAGE_KEY, self.theme.storage_value().to_owned());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;

    use super::*;
    use tempfile::tempdir;

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

    fn shortcut_input(modifiers: egui::Modifiers, key: egui::Key) -> egui::RawInput {
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        });
        input
    }

    fn collect_shortcut_from_input(input: egui::RawInput) -> Option<FileCommand> {
        let context = egui::Context::default();
        let mut command = None;
        let _ = context.run_ui(input, |ui| command = NoterApp::collect_shortcut(ui));
        command
    }

    fn show_menu_frame(
        app: &mut NoterApp,
        context: &egui::Context,
        input: egui::RawInput,
    ) -> egui::FullOutput {
        context.run_ui(input, |ui| {
            let mut command = None;
            app.show_menu(ui, &mut command);
        })
    }

    fn app_with_dismissed_uncertain_save() -> NoterApp {
        use noter::core::revision::Revision;
        use noter::core::save::{SaveStage, StorageError};

        let mut app = NoterApp::default();
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        app.handle_save_result(Ok(SaveOutcome::CommitStateUnknown {
            revision: Revision::INITIAL,
            error: StorageError::new(SaveStage::Reconcile, "destination state differs"),
            recovery_artifact: StorageError::new(
                SaveStage::Cleanup,
                "inspect `.noter-save-recovery.tmp` before retrying",
            ),
        }));
        app.error_msg = None;
        app
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
                NoterApp::show_file_menu(ui, &mut command);
            });
            let labels = rendered_text(&output)
                .into_iter()
                .map(|(label, _)| label)
                .collect::<Vec<_>>();

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
    fn failed_editor_change_restores_the_authoritative_document() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("authoritative text")
            .expect("the authoritative fixture should advance the revision");
        app.text = "unrecorded editor text".to_owned();
        app.markdown_editor.activate_first_block(&app.text);
        let previous_editor_serial = app.document_editor_serial;

        app.record_editor_change_with(|_, _| Err(NoterError::RevisionExhausted));

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
        assert_eq!(app.pending_abandon, Some(PendingAbandonAction::Quit));
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
        let help = text_position(&text, "Help");
        let mode = text_position(&text, "Mode");
        let plain_text = text_position(&text, "Text");
        let markdown = text_position(&text, "Markdown");
        let theme = text_position(&text, "Theme");
        let system = text_position(&text, "System");
        let theme_bounds = accesskit_bounds(&output, "Theme: System");

        assert!(file.x < help.x);
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
        for position in [help, mode, plain_text, markdown, theme, system] {
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
            let mut command = None;
            app.show_menu(ui, &mut command);
        });
        let text = rendered_text(&output);
        let file = text_position(&text, "File");
        let help = text_position(&text, "Help");
        let mode = text_position(&text, "Mode: Markdown");
        let theme = text_position(&text, "Theme: System");
        let help_bounds = accesskit_bounds(&output, "Help");
        let mode_bounds = accesskit_bounds(&output, "Mode: Markdown");
        let theme_bounds = accesskit_bounds(&output, "Theme: System");

        assert!(file.x < help.x);
        assert!(help.x < mode.x);
        assert!(mode.x < theme.x);
        assert!(
            help_bounds.x1 <= mode_bounds.x0,
            "Help and Mode overlap: {help_bounds:?}, {mode_bounds:?}"
        );
        assert!(
            mode_bounds.x1 <= theme_bounds.x0,
            "Mode and Theme overlap: {mode_bounds:?}, {theme_bounds:?}"
        );
        assert!(
            theme_bounds.x1 <= 420.0,
            "Theme extends beyond the minimum viewport: {theme_bounds:?}"
        );
        for position in [help, mode, theme] {
            assert!((file.y - position.y).abs() <= 2.0);
        }
        assert!(!text.iter().any(|(label, _)| label == "Text"));
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
        let dark_choice = text_position(&rendered_text(&theme_menu), "Dark") + egui::vec2(4.0, 4.0);
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
        theme::configure_styles(&context);

        let text_output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(1_200.0);
            app.show_format_toolbar(ui);
        });
        assert!(rendered_text(&text_output).is_empty());

        app.select_document_view(DocumentView::Markdown);
        let markdown_output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(1_200.0);
            app.show_format_toolbar(ui);
        });
        let text = rendered_text(&markdown_output);
        text_position(&text, "Format");
        text_position(&text, "Bold");
        assert!(!text.iter().any(|(label, _)| label == "Mode"));
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

        app.record_editor_change();

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.document.to_bytes(), app.text.as_bytes());
        assert!(!app.markdown_editor.is_editing());
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

        app.record_editor_change();

        assert_eq!(app.view, DocumentView::Text);
        assert_eq!(app.document.to_bytes(), app.text.as_bytes());
        assert!(!app.markdown_editor.is_editing());
        assert!(app.error_msg.as_deref().is_some_and(|message| {
            message.contains("512-block layout budget")
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

        app.request_new_document();
        assert_eq!(app.text, "unsaved text");
        assert_eq!(String::from(app.document.rope()), "unsaved text");
        assert!(app.document.is_dirty());
        assert_eq!(app.pending_abandon, Some(PendingAbandonAction::New));
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

        app.request_open();

        assert_eq!(app.text, "unsaved text");
        assert!(app.document.is_dirty());
        assert_eq!(app.pending_abandon, Some(PendingAbandonAction::Open));
        assert!(app.error_msg.is_none());
    }

    #[test]
    fn new_document_replaces_a_clean_document() {
        let mut app = NoterApp {
            text: "stale view text".to_owned(),
            ..NoterApp::default()
        };
        let previous_editor_id = app.editor_id();

        app.request_new_document();
        assert!(app.text.is_empty());
        assert_eq!(app.document.rope().len_bytes(), 0);
        assert!(!app.document.is_dirty());
        assert!(app.error_msg.is_none());
        assert_ne!(app.editor_id(), previous_editor_id);
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
        assert_eq!(app.pending_abandon, Some(PendingAbandonAction::Quit));
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
        assert_eq!(app.pending_abandon, Some(PendingAbandonAction::Quit));
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
        assert!(app.pending_abandon.is_none());
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
        assert!(app.pending_abandon.is_none());
    }

    #[test]
    fn native_close_guard_allows_a_confirmed_dirty_close() {
        let mut app = NoterApp {
            allow_dirty_close: true,
            ..NoterApp::default()
        };
        app.document
            .replace_text("discarded text")
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

        assert!(!commands.contains(&egui::ViewportCommand::CancelClose));
        assert!(app.pending_abandon.is_none());
    }

    #[test]
    fn discarding_a_dirty_close_allows_the_viewport_to_close() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        app.pending_abandon = Some(PendingAbandonAction::Quit);
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
        assert!(app.allow_dirty_close);
        assert!(app.pending_abandon.is_none());
    }

    #[test]
    fn cancelling_a_dirty_close_keeps_the_document_and_window() {
        let mut app = NoterApp::default();
        app.document
            .replace_text("unsaved text")
            .expect("the test edit should advance the document revision");
        app.pending_abandon = Some(PendingAbandonAction::Quit);

        app.cancel_pending_abandon();

        assert!(app.document.is_dirty());
        assert!(app.pending_abandon.is_none());
        assert!(!app.allow_dirty_close);
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
            "Cancel this dialog, then use Save As to preserve the current text at another path or reconcile the recovery state."
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
                Trigger::New => app.request_new_document(),
                Trigger::Open => app.request_open(),
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
                .save_recovery_msg
                .clone()
                .expect("the uncertain save must retain recovery guidance");
            assert_eq!(app.error_msg.as_deref(), Some(recovery.as_str()));
            assert!(recovery.contains(".noter-save-recovery.tmp"));

            app.cancel_pending_abandon();

            assert!(app.pending_abandon.is_none());
            assert_eq!(app.error_msg.as_deref(), Some(recovery.as_str()));
            assert_eq!(app.save_recovery_msg.as_deref(), Some(recovery.as_str()));
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
            save_recovery_msg: Some("Reconcile the uncertain save before retrying.".to_owned()),
            ..NoterApp::default()
        };

        app.do_save();

        assert_eq!(fs::read(&path)?, b"original");
        assert!(app.document.is_dirty());
        assert_eq!(
            app.error_msg.as_deref(),
            Some("Reconcile the uncertain save before retrying.")
        );
        Ok(())
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
        app.pending_abandon = Some(PendingAbandonAction::New);
        let context = egui::Context::default();

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            app.discard_pending_abandon(ui.ctx());
        });

        assert!(app.text.is_empty());
        assert_eq!(app.document.rope().len_bytes(), 0);
        assert!(!app.document.is_dirty());
        assert!(app.pending_abandon.is_none());
        assert!(!app.allow_dirty_close);
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
            pending_abandon: Some(PendingAbandonAction::Quit),
            ..NoterApp::default()
        };
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
        assert!(app.pending_abandon.is_none());
        #[cfg(windows)]
        {
            assert!(app.error_msg.is_none());
            assert!(app.allow_dirty_close);
            assert!(commands.contains(&egui::ViewportCommand::Close));
        }
        #[cfg(unix)]
        {
            assert!(!app.allow_dirty_close);
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
        app.pending_abandon = Some(PendingAbandonAction::New);
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
        assert_eq!(app.pending_abandon, Some(PendingAbandonAction::New));
    }

    #[test]
    fn post_save_warning_stops_the_pending_action_for_review() {
        let mut app = NoterApp {
            error_msg: Some(
                "Saved, but follow-up is required: inspect retained artifact".to_owned(),
            ),
            pending_abandon: Some(PendingAbandonAction::Quit),
            ..NoterApp::default()
        };
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
        assert!(app.pending_abandon.is_none());
        assert!(!app.allow_dirty_close);
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
        assert_eq!(app.pending_abandon, Some(PendingAbandonAction::Quit));
    }

    #[test]
    fn noncommitted_cleanup_failure_is_visible() {
        use noter::core::revision::Revision;
        use noter::core::save::{SaveStage, StorageError};

        let mut app = NoterApp::default();
        app.handle_save_result(Ok(SaveOutcome::NotCommitted {
            revision: Revision::INITIAL,
            error: StorageError::new(SaveStage::Write, "primary failure"),
            cleanup_error: Some(StorageError::new(
                SaveStage::Cleanup,
                "private artifact was preserved",
            )),
        }));

        let message = app.error_msg.expect("the failure must be visible");
        assert!(message.contains("primary failure"));
        assert!(message.contains("Cleanup also failed"));
        assert!(message.contains("private artifact was preserved"));
    }

    #[test]
    fn unknown_commit_recovery_artifact_is_visible() {
        use noter::core::revision::Revision;
        use noter::core::save::{SaveStage, StorageError};

        let mut app = NoterApp::default();
        app.handle_save_result(Ok(SaveOutcome::CommitStateUnknown {
            revision: Revision::INITIAL,
            error: StorageError::new(SaveStage::Reconcile, "destination state differs"),
            recovery_artifact: StorageError::new(
                SaveStage::Cleanup,
                "inspect `.noter-save-recovery.tmp` before retrying",
            ),
        }));

        let message = app.error_msg.expect("the recovery action must be visible");
        assert!(message.contains("destination state differs"));
        assert!(message.contains(".noter-save-recovery.tmp"));
        assert!(message.contains("before retrying"));
        assert_eq!(app.save_recovery_msg.as_deref(), Some(message.as_str()));
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

        app.handle_save_result(Ok(SaveOutcome::Committed {
            revision: Revision::INITIAL,
            durability: Durability::FileSynced,
            observation,
            warnings: SaveWarnings::new(Vec::new(), vec![warning]),
        }));

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
            Some(PendingHardLinkSave::Current { link_count }) if link_count >= 2
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
}
