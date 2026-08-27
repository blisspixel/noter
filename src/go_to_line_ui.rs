use eframe::egui;
use noter::core::navigation::line_start_offset;

use crate::bounded_text_input::{
    BoundedTextBuffer, sanitize_bounded_text_events, truncate_to_utf8_byte_limit,
};
use crate::keyboard_nav::modal_event_may_complete_action;

const LINE_INPUT_ID: &str = "noter-go-to-line-input";
const MAX_LINE_NUMBER_BYTES: usize = 20;

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
    deferred_input_events: Vec<egui::Event>,
}

impl GoToLineDialog {
    pub fn open(&mut self, current_line: usize) {
        self.open = true;
        self.input = current_line.max(1).to_string();
        self.validation = None;
        self.request_focus = true;
        self.deferred_input_events.clear();
    }

    pub fn owns_text_focus(&self, context: &egui::Context) -> bool {
        self.open && context.memory(|memory| memory.has_focus(egui::Id::new(LINE_INPUT_ID)))
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    #[cfg(test)]
    pub const fn is_open(&self) -> bool {
        self.open
    }

    #[cfg(test)]
    pub fn input_for_test(&self) -> &str {
        &self.input
    }

    pub fn show(&mut self, context: &egui::Context, source: &str) -> Option<GoToLineAction> {
        if !self.open {
            return None;
        }
        self.restore_deferred_input(context);
        if self.take_ordered_escape(context) {
            self.capture_remaining_input(context);
            self.open = false;
            return Some(GoToLineAction::Close);
        }
        self.defer_completion_suffix(context);

        let mut window_open = self.open;
        let mut close_clicked = false;
        let mut submit = false;
        egui::Window::new("Go To Line")
            .open(&mut window_open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                let label = ui.label("Line number");
                let id = egui::Id::new(LINE_INPUT_ID);
                let stored_input_was_clamped =
                    truncate_to_utf8_byte_limit(&mut self.input, MAX_LINE_NUMBER_BYTES);
                let accepts_input = self.request_focus || ui.memory(|memory| memory.has_focus(id));
                let event_was_clamped = accepts_input
                    && sanitize_bounded_text_events(ui, id, &self.input, MAX_LINE_NUMBER_BYTES);
                let (response, buffer_was_clamped) = {
                    let mut buffer = BoundedTextBuffer::new(&mut self.input, MAX_LINE_NUMBER_BYTES);
                    let response = ui
                        .add(
                            egui::TextEdit::singleline(&mut buffer)
                                .id(id)
                                .char_limit(MAX_LINE_NUMBER_BYTES)
                                .desired_width(220.0),
                        )
                        .labelled_by(label.id);
                    (response, buffer.was_limited())
                };
                let event_was_clamped = event_was_clamped || buffer_was_clamped;
                if self.request_focus {
                    response.request_focus();
                    self.request_focus = false;
                }
                let result_was_clamped =
                    truncate_to_utf8_byte_limit(&mut self.input, MAX_LINE_NUMBER_BYTES);
                if response.changed() || stored_input_was_clamped {
                    self.validation = None;
                }
                if event_was_clamped || result_was_clamped {
                    self.validation = Some("Line number input was limited to 20 bytes.".to_owned());
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

        context.input_mut(|input| input.events.clear());

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

    fn restore_deferred_input(&mut self, context: &egui::Context) {
        if self.deferred_input_events.is_empty() {
            return;
        }
        context.input_mut(|input| {
            self.deferred_input_events.append(&mut input.events);
            std::mem::swap(&mut self.deferred_input_events, &mut input.events);
        });
    }

    pub fn take_deferred_input_events(&mut self) -> Vec<egui::Event> {
        std::mem::take(&mut self.deferred_input_events)
    }

    fn capture_remaining_input(&mut self, context: &egui::Context) {
        let mut remaining = context.input_mut(|input| std::mem::take(&mut input.events));
        remaining.append(&mut self.deferred_input_events);
        self.deferred_input_events = remaining;
    }

    fn defer_completion_suffix(&mut self, context: &egui::Context) {
        let mut suffix = context.input_mut(|input| {
            let Some(position) = input
                .events
                .iter()
                .position(modal_event_may_complete_action)
            else {
                return Vec::new();
            };
            if position + 1 == input.events.len() {
                Vec::new()
            } else {
                input.events.split_off(position + 1)
            }
        });
        if suffix.is_empty() {
            return;
        }
        suffix.append(&mut self.deferred_input_events);
        self.deferred_input_events = suffix;
        context.request_repaint();
    }

    fn take_ordered_escape(&mut self, context: &egui::Context) -> bool {
        let mut deferred = Vec::new();
        let close = context.input_mut(|input| {
            let Some(position) = input.events.iter().position(|event| {
                matches!(
                    event,
                    egui::Event::Key {
                        key: egui::Key::Escape,
                        pressed: true,
                        modifiers,
                        ..
                    } if modifiers.matches_logically(egui::Modifiers::NONE)
                )
            }) else {
                return false;
            };
            if input.events[..position]
                .iter()
                .any(crate::keyboard_nav::editor_event_orders_input)
            {
                deferred = input.events.split_off(position);
                return false;
            }
            input.events.remove(position);
            true
        });
        if !deferred.is_empty() {
            deferred.append(&mut self.deferred_input_events);
            self.deferred_input_events = deferred;
            context.request_repaint();
        }
        close
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
    fn line_number_input_references_its_visible_label() {
        let mut dialog = GoToLineDialog::default();
        dialog.open(1);
        let context = egui::Context::default();
        context.enable_accesskit();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            let _ = dialog.show(ui.ctx(), "one");
        });
        let update = output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled");
        let line_label = update
            .nodes
            .iter()
            .find_map(|(id, node)| {
                (node.role() == egui::accesskit::Role::Label && node.value() == Some("Line number"))
                    .then_some(*id)
            })
            .expect("expected the visible line-number label");
        let input = update
            .nodes
            .iter()
            .find_map(|(id, node)| {
                (*id == egui::Id::new(LINE_INPUT_ID).accesskit_id()).then_some(node)
            })
            .expect("expected the line-number input node");

        assert_eq!(input.labelled_by(), &[line_label]);
    }

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
    fn reset_closes_the_dialog_and_discards_document_specific_state() {
        let mut dialog = GoToLineDialog {
            open: true,
            input: "99".to_owned(),
            validation: Some("old document error".to_owned()),
            request_focus: true,
            deferred_input_events: vec![egui::Event::Text("stale".to_owned())],
        };

        dialog.reset();

        assert!(!dialog.is_open());
        assert!(dialog.input.is_empty());
        assert!(dialog.validation.is_none());
        assert!(!dialog.request_focus);
        assert!(dialog.deferred_input_events.is_empty());
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

    #[test]
    fn oversized_paste_is_bounded_before_single_line_normalization() {
        let context = egui::Context::default();
        let mut dialog = GoToLineDialog::default();
        dialog.open(1);
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            let _ = dialog.show(ui.ctx(), "one");
        });
        let mut input = egui::RawInput::default();
        input
            .events
            .push(egui::Event::Paste("9".repeat(1024 * 1024)));

        let _ = context.run_ui(input, |ui| {
            let _ = dialog.show(ui.ctx(), "one");
        });

        assert_eq!(dialog.input.len(), MAX_LINE_NUMBER_BYTES);
        assert_eq!(
            dialog.validation.as_deref(),
            Some("Line number input was limited to 20 bytes.")
        );
    }
}
