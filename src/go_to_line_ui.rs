use eframe::egui;
use noter::core::navigation::line_start_offset;

const LINE_INPUT_ID: &str = "noter-go-to-line-input";
const MAX_LINE_NUMBER_CHARACTERS: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoToLineAction {
    Navigate(usize),
    Close,
}

#[derive(Default, Debug)]
pub struct GoToLineDialog {
    open: bool,
    input: String,
    validation: Option<String>,
    request_focus: bool,
}

impl GoToLineDialog {
    pub fn open(&mut self, current_line: usize) {
        self.open = true;
        self.input = current_line.max(1).to_string();
        self.validation = None;
        self.request_focus = true;
    }

    pub fn owns_text_focus(&self, context: &egui::Context) -> bool {
        self.open && context.memory(|memory| memory.has_focus(egui::Id::new(LINE_INPUT_ID)))
    }

    pub fn show(&mut self, context: &egui::Context, source: &str) -> Option<GoToLineAction> {
        if !self.open {
            return None;
        }
        if context.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
            self.open = false;
            return Some(GoToLineAction::Close);
        }

        let mut window_open = self.open;
        let mut close_clicked = false;
        let mut submit = false;
        egui::Window::new("Go To Line")
            .open(&mut window_open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label("Line number");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.input)
                        .id(egui::Id::new(LINE_INPUT_ID))
                        .char_limit(MAX_LINE_NUMBER_CHARACTERS)
                        .desired_width(220.0),
                );
                if self.request_focus {
                    response.request_focus();
                    self.request_focus = false;
                }
                if response.changed() {
                    self.validation = None;
                }
                submit =
                    response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter));

                if let Some(validation) = self.validation.as_deref() {
                    ui.colored_label(ui.visuals().error_fg_color, validation);
                }
                ui.horizontal(|ui| {
                    if ui.button("Go To").clicked() {
                        submit = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close_clicked = true;
                    }
                });
            });

        if close_clicked || !window_open {
            self.open = false;
            return Some(GoToLineAction::Close);
        }
        if !submit {
            return None;
        }

        match resolve_line_offset(source, &self.input) {
            Ok(offset) => {
                self.open = false;
                self.validation = None;
                Some(GoToLineAction::Navigate(offset))
            }
            Err(validation) => {
                self.validation = Some(validation);
                self.request_focus = true;
                None
            }
        }
    }
}

fn resolve_line_offset(source: &str, input: &str) -> Result<usize, String> {
    let value = input.trim();
    if value.is_empty() {
        return Err("Enter a one-based line number.".to_owned());
    }
    let requested = value
        .parse::<usize>()
        .map_err(|_| "Enter a whole line number using digits only.".to_owned())?;
    line_start_offset(source, requested).map_err(|error| format!("{error}."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolver_accepts_exact_mixed_ending_lines() {
        let source = "one\r\ntwo\rthree\nfour";

        assert_eq!(resolve_line_offset(source, " 3 "), Ok("one\r\ntwo\r".len()));
        assert_eq!(
            resolve_line_offset(source, "4"),
            Ok("one\r\ntwo\rthree\n".len())
        );
    }

    #[test]
    fn resolver_explains_empty_non_numeric_zero_and_out_of_range_values() {
        assert_eq!(
            resolve_line_offset("one", ""),
            Err("Enter a one-based line number.".to_owned())
        );
        assert_eq!(
            resolve_line_offset("one", "1.5"),
            Err("Enter a whole line number using digits only.".to_owned())
        );
        assert_eq!(
            resolve_line_offset("one", "0"),
            Err("line numbers begin at 1.".to_owned())
        );
        assert_eq!(
            resolve_line_offset("one", "2"),
            Err("line 2 is outside this 1-line document.".to_owned())
        );
    }

    #[test]
    fn opening_prefills_the_current_line_and_resets_prior_validation() {
        let mut dialog = GoToLineDialog {
            validation: Some("old error".to_owned()),
            ..GoToLineDialog::default()
        };

        dialog.open(7);

        assert!(dialog.open);
        assert_eq!(dialog.input, "7");
        assert_eq!(dialog.validation, None);
        assert!(dialog.request_focus);
    }

    #[test]
    fn focus_ownership_requires_both_an_open_dialog_and_its_input_focus() {
        let context = egui::Context::default();
        let mut dialog = GoToLineDialog::default();
        context.memory_mut(|memory| memory.request_focus(egui::Id::new(LINE_INPUT_ID)));
        assert!(!dialog.owns_text_focus(&context));

        dialog.open(1);
        assert!(dialog.owns_text_focus(&context));
        context.memory_mut(|memory| memory.surrender_focus(egui::Id::new(LINE_INPUT_ID)));
        assert!(!dialog.owns_text_focus(&context));
    }
}
