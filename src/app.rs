use eframe::egui;
use crate::core::document::{Document, LineEnding};

pub struct NoterApp {
    text: String,
    document: Document,
    error_msg: Option<String>,
}

impl Default for NoterApp {
    fn default() -> Self {
        Self {
            text: String::new(),
            document: Document::new(),
            error_msg: None,
        }
    }
}

impl NoterApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut style = (*cc.egui_ctx.global_style()).clone();
        
        // Sleek charcoal/slate dark mode instead of generic #000000
        style.visuals.panel_fill = egui::Color32::from_rgb(24, 26, 31);
        style.visuals.faint_bg_color = egui::Color32::from_rgb(32, 35, 42);
        style.visuals.extreme_bg_color = egui::Color32::from_rgb(18, 20, 24); // Text editor background
        
        // Improve typography legibility
        for (text_style, font_id) in style.text_styles.iter_mut() {
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

        Default::default()
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
                    self.error_msg = Some(format!("Failed to open file: {}", e));
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
            self.error_msg = Some(format!("Failed to save file: {}", e));
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

    fn handle_shortcuts(&mut self, ui: &mut egui::Ui) {
        let mut open = false;
        let mut save = false;
        let mut save_as = false;
        let mut new_file = false;

        ui.input_mut(|i| {
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::N)) {
                new_file = true;
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::O)) {
                open = true;
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL, egui::Key::S)) {
                save = true;
            }
            if i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::S)) {
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
        let title = if let Some(path) = &self.document.path {
            let file_name = path.file_name().unwrap_or_default().to_string_lossy();
            let dirty = if self.document.is_dirty { "*" } else { "" };
            format!("{}{dirty} - Noter", file_name)
        } else {
            let dirty = if self.document.is_dirty { "*" } else { "" };
            format!("Untitled{dirty} - Noter")
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(title));
    }
}

impl eframe::App for NoterApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_shortcuts(ui);
        self.update_title(ui.ctx());

        // Menu Bar
        egui::Panel::top("menu_bar").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.add(egui::Button::new("New").shortcut_text("Ctrl+N")).clicked() {
                        self.text.clear();
                        self.document = Document::new();
                        ui.close();
                    }
                    if ui.add(egui::Button::new("Open...").shortcut_text("Ctrl+O")).clicked() {
                        self.do_open();
                        ui.close();
                    }
                    if ui.add(egui::Button::new("Save").shortcut_text("Ctrl+S")).clicked() {
                        self.do_save();
                        ui.close();
                    }
                    if ui.add(egui::Button::new("Save As...").shortcut_text("Ctrl+Shift+S")).clicked() {
                        self.do_save_as();
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
            });
        });

        // Error message panel (if any)
        let mut dismiss_error = false;
        if let Some(err) = &self.error_msg {
            egui::Panel::top("error_bar").show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.colored_label(egui::Color32::RED, format!("Error: {}", err));
                    if ui.button("Dismiss").clicked() {
                        dismiss_error = true;
                    }
                });
            });
        }
        if dismiss_error {
            self.error_msg = None;
        }

        // Status Bar
        egui::Panel::bottom("status_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(path) = &self.document.path {
                    ui.label(path.display().to_string());
                } else {
                    ui.label("Untitled");
                }
                
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match self.document.line_ending {
                        LineEnding::Lf => ui.label("LF"),
                        LineEnding::CrLf => ui.label("CRLF"),
                        LineEnding::Cr => ui.label("CR"),
                    };
                    ui.separator();
                    ui.label("UTF-8");
                    ui.separator();
                    if self.document.had_bom {
                        ui.label("BOM");
                        ui.separator();
                    }
                });
            });
        });

        // Main Editor Area
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE
                .fill(ui.visuals().extreme_bg_color)
                .inner_margin(egui::Margin::same(12)))
            .show_inside(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                let response = ui.add_sized(
                    ui.available_size(),
                    egui::TextEdit::multiline(&mut self.text)
                        .font(egui::TextStyle::Monospace)
                        .code_editor()
                        .lock_focus(true),
                );
                
                if response.changed() {
                    self.document.is_dirty = true;
                }
            });
        });
    }
}
