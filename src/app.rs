use eframe::egui;
use noter::core::document::Document;

#[derive(Default)]
pub struct NoterApp {
    text: String,
    document: Document,
    error_msg: Option<String>,
}

impl NoterApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.global_style()).clone();

        // Sleek charcoal/slate dark mode instead of generic #000000
        style.visuals.panel_fill = egui::Color32::from_rgb(24, 26, 31);
        style.visuals.faint_bg_color = egui::Color32::from_rgb(32, 35, 42);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(18, 20, 24); // Text editor background

        // Improve typography legibility
        for (text_style, font_id) in &mut style.text_styles {
            if *text_style == egui::TextStyle::Body || *text_style == egui::TextStyle::Button {
                font_id.size = 14.0;
            } else if *text_style == egui::TextStyle::Monospace {
                font_id.size = 15.0; // Slightly larger for comfortable coding/writing
            } else if *text_style == egui::TextStyle::Heading {
                font_id.size = 20.0;
            }
        }

        // Spacing polish
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(6.0, 4.0);

        cc.egui_ctx.set_global_style(style);

        Self::default()
    }

    fn do_open(&mut self) {
        if let Some(path) = rfd::FileDialog::new().pick_file() {
            match Document::from_path(&path) {
                Ok(doc) => {
                    self.text = String::from(&doc.rope);
                    self.document = doc;
                    self.error_msg = None;
                }
                Err(e) => {
                    self.error_msg = Some(format!("Failed to open file: {e}"));
                }
            }
        }
    }

    fn do_save(&mut self) {
        if self.document.path.is_none() {
            self.do_save_as();
            return;
        }

        self.document.rope = ropey::Rope::from_str(&self.text);
        if let Err(e) = self.document.save_atomic() {
            self.error_msg = Some(format!("Failed to save file: {e}"));
        } else {
            self.error_msg = None;
        }
    }

    fn do_save_as(&mut self) {
        if let Some(path) = rfd::FileDialog::new().save_file() {
            self.document.path = Some(path);
            self.do_save();
        }
    }

    fn handle_shortcuts(&mut self, ui: &egui::Ui) {
        let mut open = false;
        let mut save = false;
        let mut save_as = false;
        let mut new_file = false;

        ui.input_mut(|i| {
            if i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::CTRL,
                egui::Key::N,
            )) {
                new_file = true;
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::CTRL,
                egui::Key::O,
            )) {
                open = true;
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::CTRL,
                egui::Key::S,
            )) {
                save = true;
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(
                egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
                egui::Key::S,
            )) {
                save_as = true;
            }
        });

        if new_file {
            self.text.clear();
            self.document = Document::new();
        }
        if open {
            self.do_open();
        }
        if save {
            self.do_save();
        }
        if save_as {
            self.do_save_as();
        }
    }

    fn update_title(&self, ctx: &egui::Context) {
        let dirty = if self.document.is_dirty { "*" } else { "" };
        let title = self.document.path.as_ref().map_or_else(
            || format!("Untitled{dirty} - Noter"),
            |path| {
                let file_name = path.file_name().unwrap_or_default().to_string_lossy();
                format!("{file_name}{dirty} - Noter")
            },
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }

    fn show_menu(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu_bar").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| self.show_file_menu(ui));
                ui.menu_button("Edit", Self::show_edit_menu);
                ui.menu_button("View", Self::show_view_menu);
                ui.menu_button("Help", Self::show_help_menu);
            });
        });
    }

    fn show_file_menu(&mut self, ui: &mut egui::Ui) {
        if ui
            .add(egui::Button::new("New").shortcut_text("Ctrl+N"))
            .clicked()
        {
            self.text.clear();
            self.document = Document::new();
            ui.close();
        }
        if ui
            .add(egui::Button::new("Open...").shortcut_text("Ctrl+O"))
            .clicked()
        {
            self.do_open();
            ui.close();
        }
        if ui
            .add(egui::Button::new("Save").shortcut_text("Ctrl+S"))
            .clicked()
        {
            self.do_save();
            ui.close();
        }
        if ui
            .add(egui::Button::new("Save As...").shortcut_text("Ctrl+Shift+S"))
            .clicked()
        {
            self.do_save_as();
            ui.close();
        }
        ui.separator();
        if ui.button("Quit").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn show_edit_menu(ui: &mut egui::Ui) {
        if ui
            .add(egui::Button::new("Undo").shortcut_text("Ctrl+Z"))
            .clicked()
        {
            ui.close();
        }
        if ui
            .add(egui::Button::new("Redo").shortcut_text("Ctrl+Y"))
            .clicked()
        {
            ui.close();
        }
        ui.separator();
        for (label, shortcut) in [("Cut", "Ctrl+X"), ("Copy", "Ctrl+C"), ("Paste", "Ctrl+V")] {
            if ui
                .add(egui::Button::new(label).shortcut_text(shortcut))
                .clicked()
            {
                ui.close();
            }
        }
        ui.separator();
        for (label, shortcut) in [("Find", "Ctrl+F"), ("Replace", "Ctrl+H")] {
            if ui
                .add(egui::Button::new(label).shortcut_text(shortcut))
                .clicked()
            {
                ui.close();
            }
        }
    }

    fn show_view_menu(ui: &mut egui::Ui) {
        if ui.button("Word Wrap").clicked() {
            ui.close();
        }
        ui.separator();
        for (label, shortcut) in [
            ("Zoom In", "Ctrl++"),
            ("Zoom Out", "Ctrl+-"),
            ("Restore Default Zoom", "Ctrl+0"),
        ] {
            if ui
                .add(egui::Button::new(label).shortcut_text(shortcut))
                .clicked()
            {
                ui.close();
            }
        }
    }

    fn show_help_menu(ui: &mut egui::Ui) {
        if ui.button("About Noter").clicked() {
            ui.close();
        }
    }

    fn show_error(&mut self, ui: &mut egui::Ui) {
        let mut dismiss = false;
        if let Some(error) = self.error_msg.as_deref() {
            egui::Panel::top("error_bar").show_inside(ui, |ui| {
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

    fn show_status(&self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                let document_label = self
                    .document
                    .path
                    .as_ref()
                    .map_or_else(|| "Untitled".to_owned(), |path| path.display().to_string());
                ui.label(document_label);

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(self.document.line_endings.status_label());
                    ui.separator();
                    ui.label(self.document.encoding.status_label());
                    if self.document.bom.is_present() {
                        ui.separator();
                        ui.label("BOM");
                    }
                });
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
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    let response = ui.add_sized(
                        ui.available_size(),
                        egui::TextEdit::multiline(&mut self.text)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .frame(egui::Frame::NONE)
                            .lock_focus(true),
                    );

                    if response.changed() {
                        self.document.is_dirty = true;
                    }
                });
            });
    }
}

impl eframe::App for NoterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ui);
        self.update_title(ui.ctx());
        self.show_menu(ui);
        self.show_error(ui);
        self.show_status(ui);
        self.show_editor(ui);
    }
}
