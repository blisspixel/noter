use std::{any::TypeId, cell::RefCell, ops::Range};

use eframe::egui;

/// Truncates a string to an exact UTF-8 byte ceiling without splitting a
/// scalar value.
pub fn truncate_to_utf8_byte_limit(value: &mut String, maximum: usize) -> bool {
    if value.len() <= maximum {
        return false;
    }
    let boundary = utf8_prefix(value, maximum).len();
    value.truncate(boundary);
    true
}

/// Bounds text-bearing events offered to a focused `TextEdit` against the
/// current selection budget. The buffer itself still enforces the exact
/// document ceiling because navigation can change insertion capacity mid-frame.
pub fn sanitize_bounded_text_events(
    ui: &egui::Ui,
    id: egui::Id,
    current: &str,
    maximum: usize,
) -> bool {
    if !ui.memory(|memory| memory.has_focus(id)) {
        return false;
    }
    let has_text_payload = ui.input(|input| {
        input.events.iter().any(|event| {
            matches!(event, egui::Event::Paste(_) | egui::Event::Text(_))
                || matches!(
                    event,
                    egui::Event::Ime(egui::ImeEvent::Preedit { .. } | egui::ImeEvent::Commit(_))
                )
        })
    });
    if !has_text_payload {
        return false;
    }

    let selected_bytes = egui::TextEdit::load_state(ui.ctx(), id)
        .and_then(|state| state.cursor.char_range())
        .map_or(0, |range| {
            byte_range_from_char_range(current, range.as_sorted_char_range()).len()
        });
    let retained = current.len().saturating_sub(selected_bytes);
    let mut remaining = maximum.saturating_sub(retained);
    ui.input_mut(|input| {
        let mut clamped = false;
        let pointer_drag_may_change_selection = input.pointer.primary_down()
            && input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::PointerMoved(_)));
        let pointer_button_may_change_selection = input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::PointerButton { .. }));
        let mut selection_budget_may_be_stale =
            pointer_drag_may_change_selection || pointer_button_may_change_selection;
        for event in &mut input.events {
            match event {
                egui::Event::Paste(text)
                | egui::Event::Text(text)
                | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    clamped |= truncate_to_utf8_byte_limit(text, remaining);
                    remaining = remaining.saturating_sub(text.len());
                }
                egui::Event::Ime(egui::ImeEvent::Preedit {
                    text,
                    active_range_chars,
                }) => {
                    let was_truncated = truncate_to_utf8_byte_limit(text, remaining);
                    if was_truncated || selection_budget_may_be_stale {
                        // Navigation can invalidate the precomputed selection
                        // budget earlier in the same frame. Omit the optional
                        // subrange only when truncation occurred or the exact
                        // retained composition can no longer be proven.
                        *active_range_chars = None;
                    }
                    if was_truncated {
                        clamped = true;
                    }
                }
                event => {
                    selection_budget_may_be_stale |= event_may_change_selection_or_text(event);
                }
            }
        }
        clamped
    })
}

const fn event_may_change_selection_or_text(event: &egui::Event) -> bool {
    match event {
        egui::Event::Cut | egui::Event::AccessKitActionRequest(_) => true,
        egui::Event::Key {
            key,
            pressed: true,
            modifiers,
            ..
        } => {
            matches!(
                key,
                egui::Key::ArrowDown
                    | egui::Key::ArrowLeft
                    | egui::Key::ArrowRight
                    | egui::Key::ArrowUp
                    | egui::Key::Backspace
                    | egui::Key::Delete
                    | egui::Key::End
                    | egui::Key::Enter
                    | egui::Key::Home
                    | egui::Key::Tab
            ) || modifiers.command && matches!(key, egui::Key::A | egui::Key::Y | egui::Key::Z)
                || modifiers.ctrl
                    && matches!(
                        key,
                        egui::Key::A
                            | egui::Key::B
                            | egui::Key::E
                            | egui::Key::F
                            | egui::Key::H
                            | egui::Key::K
                            | egui::Key::N
                            | egui::Key::P
                            | egui::Key::U
                            | egui::Key::W
                    )
        }
        _ => false,
    }
}

/// A `TextEdit` buffer that applies an exact UTF-8 byte ceiling at every
/// mutation. Enforcing the limit here covers text, paste, IME, Enter, Tab, and
/// replacements after earlier events have moved or collapsed the selection.
pub struct BoundedTextBuffer<'a> {
    value: &'a mut String,
    maximum: usize,
    was_limited: bool,
    recent_deletion: RefCell<Option<RecentDeletion>>,
}

impl<'a> BoundedTextBuffer<'a> {
    pub fn new(value: &'a mut String, maximum: usize) -> Self {
        let was_limited = truncate_to_utf8_byte_limit(value, maximum);
        Self {
            value,
            maximum,
            was_limited,
            recent_deletion: RefCell::new(None),
        }
    }

    pub const fn was_limited(&self) -> bool {
        self.was_limited
    }
}

struct BoundedTextBufferType;

struct RecentDeletion {
    start: egui::text::CharIndex,
    byte_start: usize,
    removed: String,
}

impl egui::TextBuffer for BoundedTextBuffer<'_> {
    fn is_mutable(&self) -> bool {
        true
    }

    fn as_str(&self) -> &str {
        // TextEdit lays out after every completed event. Reaching this method
        // means a deletion was not immediately followed by its replacement.
        self.recent_deletion.borrow_mut().take();
        self.value
    }

    fn insert_text(&mut self, text: &str, char_index: egui::text::CharIndex) -> usize {
        let remaining = self.maximum.saturating_sub(self.value.len());
        let bounded = utf8_prefix(text, remaining);
        self.was_limited |= bounded.len() != text.len();
        let recent_deletion = self.recent_deletion.borrow_mut().take();
        if !text.is_empty()
            && bounded.is_empty()
            && let Some(deletion) = recent_deletion
            && deletion.start == char_index
        {
            self.value
                .insert_str(deletion.byte_start, &deletion.removed);
            return 0;
        }
        <String as egui::TextBuffer>::insert_text(self.value, bounded, char_index)
    }

    fn delete_char_range(&mut self, char_range: Range<egui::text::CharIndex>) {
        self.recent_deletion.borrow_mut().take();
        let byte_range = byte_range_from_char_range(self.value, char_range);
        self.value.drain(byte_range);
    }

    fn insert_text_at(
        &mut self,
        ccursor: &mut egui::text::CCursor,
        text_to_insert: &str,
        char_limit: usize,
    ) {
        let cutoff = char_limit.saturating_sub(self.value.chars().count());
        let text_to_insert = text_to_insert
            .char_indices()
            .nth(cutoff)
            .map_or(text_to_insert, |(index, _)| &text_to_insert[..index]);
        ccursor.index += self.insert_text(text_to_insert, ccursor.index);
    }

    fn delete_selected(&mut self, cursor_range: &egui::text::CCursorRange) -> egui::text::CCursor {
        let [start, end] = cursor_range.sorted_cursors();
        let character_range = start.index..end.index;
        let byte_range = byte_range_from_char_range(self.value, character_range);
        let removed = &self.value[byte_range.clone()];
        *self.recent_deletion.borrow_mut() =
            (1..4).contains(&removed.len()).then(|| RecentDeletion {
                start: start.index,
                byte_start: byte_range.start,
                removed: removed.to_owned(),
            });
        self.value.drain(byte_range);
        egui::text::CCursor {
            index: start.index,
            prefer_next_row: true,
        }
    }

    fn clear(&mut self) {
        self.recent_deletion.borrow_mut().take();
        self.value.clear();
    }

    fn replace_with(&mut self, text: &str) {
        self.recent_deletion.borrow_mut().take();
        self.value.clear();
        self.insert_text(text, egui::text::CharIndex::ZERO);
    }

    fn take(&mut self) -> String {
        self.recent_deletion.borrow_mut().take();
        std::mem::take(self.value)
    }

    fn type_id(&self) -> TypeId {
        TypeId::of::<BoundedTextBufferType>()
    }
}

fn utf8_prefix(value: &str, maximum: usize) -> &str {
    if value.len() <= maximum {
        return value;
    }
    // A UTF-8 scalar occupies at most four bytes, so a boundary must exist in
    // this finite window. Avoid an open-ended decrement loop in this input
    // boundary because a malformed mutation should fail, not spin forever.
    let boundary = (maximum.saturating_sub(3)..=maximum)
        .rev()
        .find(|&candidate| value.is_char_boundary(candidate))
        .expect("a UTF-8 boundary must occur within the preceding three bytes");
    &value[..boundary]
}

fn byte_range_from_char_range(
    source: &str,
    character_range: Range<egui::text::CharIndex>,
) -> Range<usize> {
    // Defensive sort: inverted ranges must not panic the product path if the
    // widget ever reports them. Ordered byte ranges remain the contract.
    let start_character: usize = character_range.start.min(character_range.end).into();
    let end_character: usize = character_range.start.max(character_range.end).into();
    let mut byte_start = source.len();
    let mut byte_end = source.len();
    for (character, (byte, _)) in source.char_indices().enumerate() {
        if character == start_character {
            byte_start = byte;
        }
        if character == end_character {
            byte_end = byte;
            break;
        }
    }
    byte_start..byte_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::TextBuffer as _;

    fn sanitize_event(event: egui::Event) -> egui::Event {
        let context = egui::Context::default();
        let id = egui::Id::new("bounded-event-test");
        context.memory_mut(|memory| memory.request_focus(id));
        let mut sanitized = None;
        let mut input = egui::RawInput::default();
        input.events.push(event);

        let _ = context.run_ui(input, |ui| {
            assert!(sanitize_bounded_text_events(ui, id, "", 3));
            sanitized = ui.input(|input| input.events.first().cloned());
        });

        sanitized.expect("the sanitized event should remain available to TextEdit")
    }

    fn key(key: egui::Key) -> egui::Event {
        key_with_modifiers(key, egui::Modifiers::NONE)
    }

    fn key_with_modifiers(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn ime_preedit() -> egui::Event {
        egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "é".to_owned(),
            active_range_chars: Some(0..1),
        })
    }

    fn sanitized_preedit_range(events: Vec<egui::Event>) -> Option<Range<usize>> {
        let context = egui::Context::default();
        let id = egui::Id::new("ime-selection-invalidation-test");
        context.memory_mut(|memory| memory.request_focus(id));
        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut active_range = None;

        let _ = context.run_ui(input, |ui| {
            assert!(!sanitize_bounded_text_events(ui, id, "", 4));
            active_range = ui.input(|input| match input.events.last() {
                Some(egui::Event::Ime(egui::ImeEvent::Preedit {
                    active_range_chars, ..
                })) => active_range_chars.clone(),
                _ => panic!("the final event should be an IME preedit"),
            });
        });

        active_range
    }

    fn edit_with_events(
        initial: &str,
        maximum: usize,
        selection: Range<usize>,
        events: Vec<egui::Event>,
        lock_focus: bool,
    ) -> (String, bool) {
        let context = egui::Context::default();
        let id = egui::Id::new("bounded-buffer-editor-test");
        context.memory_mut(|memory| memory.request_focus(id));
        let mut state = egui::TextEdit::load_state(&context, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(selection.start),
                egui::text::CCursor::new(selection.end),
            )));
        egui::TextEdit::store_state(&context, id, state);

        let input = egui::RawInput {
            events,
            ..Default::default()
        };
        let mut value = initial.to_owned();
        let mut was_limited = false;
        let _ = context.run_ui(input, |ui| {
            let mut buffer = BoundedTextBuffer::new(&mut value, maximum);
            let editor = egui::TextEdit::multiline(&mut buffer)
                .id(id)
                .lock_focus(lock_focus);
            ui.add(editor);
            was_limited = buffer.was_limited();
        });
        (value, was_limited)
    }

    #[test]
    fn text_and_ime_payloads_are_bounded_on_utf8_boundaries() {
        for event in [
            egui::Event::Text("éé".to_owned()),
            egui::Event::Ime(egui::ImeEvent::Commit("éé".to_owned())),
        ] {
            match sanitize_event(event) {
                egui::Event::Text(text) | egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    assert_eq!(text, "é");
                }
                _ => panic!("the text-bearing event kind should be preserved"),
            }
        }
    }

    #[test]
    fn utf8_prefix_handles_every_position_inside_a_four_byte_scalar() {
        let value = "a\u{10437}z";
        for (maximum, expected) in [
            (0, ""),
            (1, "a"),
            (2, "a"),
            (3, "a"),
            (4, "a"),
            (5, "a\u{10437}"),
            (6, "a\u{10437}z"),
        ] {
            assert_eq!(utf8_prefix(value, maximum), expected);
        }
    }

    #[test]
    fn truncated_ime_preedit_drops_its_stale_active_range() {
        let event = sanitize_event(egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "éé".to_owned(),
            active_range_chars: Some(0..2),
        }));

        let egui::Event::Ime(egui::ImeEvent::Preedit {
            text,
            active_range_chars,
        }) = event
        else {
            panic!("the preedit event kind should be preserved");
        };
        assert_eq!(text, "é");
        assert_eq!(active_range_chars, None);
    }

    #[test]
    fn unfocused_editor_leaves_another_controls_input_untouched() {
        let context = egui::Context::default();
        let editor_id = egui::Id::new("unfocused-editor");
        context.memory_mut(|memory| memory.request_focus(egui::Id::new("focused-control")));
        let mut input = egui::RawInput::default();
        input
            .events
            .push(egui::Event::Paste("oversized".to_owned()));

        let _ = context.run_ui(input, |ui| {
            assert!(!sanitize_bounded_text_events(ui, editor_id, "full", 4));
            assert_eq!(
                ui.input(|input| input.events.first().cloned()),
                Some(egui::Event::Paste("oversized".to_owned()))
            );
        });
    }

    #[test]
    fn navigation_only_input_needs_no_document_or_cursor_scan() {
        let context = egui::Context::default();
        let editor_id = egui::Id::new("navigation-only-editor");
        context.memory_mut(|memory| memory.request_focus(editor_id));
        let mut input = egui::RawInput::default();
        input.events.push(key(egui::Key::ArrowRight));

        let _ = context.run_ui(input, |ui| {
            assert!(!sanitize_bounded_text_events(ui, editor_id, "", 64 << 20));
        });
    }

    #[test]
    fn mutation_boundary_preserves_an_exact_utf8_byte_ceiling() {
        let mut value = "aé".to_owned();
        {
            let mut buffer = BoundedTextBuffer::new(&mut value, 4);
            assert_eq!(buffer.insert_text("é", egui::text::CharIndex::ZERO), 0);
            assert!(buffer.was_limited());
        }
        assert_eq!(value, "aé");
    }

    #[test]
    fn rejected_multibyte_replacement_preserves_a_smaller_selection() {
        let (value, was_limited) = edit_with_events(
            "aéx",
            4,
            0..1,
            vec![egui::Event::Paste("é".to_owned())],
            false,
        );

        assert_eq!(value, "aéx");
        assert!(was_limited);
    }

    #[test]
    fn ordinary_ime_preedit_preserves_its_active_subrange() {
        let context = egui::Context::default();
        let id = egui::Id::new("ime-active-range-test");
        context.memory_mut(|memory| memory.request_focus(id));
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "é".to_owned(),
            active_range_chars: Some(0..1),
        }));

        let _ = context.run_ui(input, |ui| {
            assert!(!sanitize_bounded_text_events(ui, id, "", 4));
            let active_range = ui.input(|input| match &input.events[0] {
                egui::Event::Ime(egui::ImeEvent::Preedit {
                    active_range_chars, ..
                }) => active_range_chars.clone(),
                _ => panic!("the IME preedit event should remain available"),
            });
            assert_eq!(active_range, Some(0..1));
        });
    }

    #[test]
    fn pointer_motion_without_a_drag_preserves_ime_subrange() {
        assert_eq!(
            sanitized_preedit_range(vec![
                egui::Event::PointerMoved(egui::pos2(12.0, 8.0)),
                ime_preedit(),
            ]),
            Some(0..1)
        );
    }

    #[test]
    fn pointer_button_activity_invalidates_precomputed_ime_subrange() {
        assert_eq!(
            sanitized_preedit_range(vec![
                egui::Event::PointerButton {
                    pos: egui::pos2(12.0, 8.0),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                ime_preedit(),
            ]),
            None
        );
    }

    #[test]
    fn selection_changing_event_classification_is_precise() {
        let command = egui::Modifiers {
            command: true,
            ..egui::Modifiers::NONE
        };
        let control = egui::Modifiers {
            ctrl: true,
            ..egui::Modifiers::NONE
        };

        assert!(event_may_change_selection_or_text(&egui::Event::Cut));
        assert!(event_may_change_selection_or_text(&key(
            egui::Key::ArrowLeft
        )));
        assert!(!event_may_change_selection_or_text(&key(egui::Key::A)));
        assert!(event_may_change_selection_or_text(&key_with_modifiers(
            egui::Key::A,
            command
        )));
        assert!(!event_may_change_selection_or_text(&key_with_modifiers(
            egui::Key::Q,
            command
        )));
        assert!(event_may_change_selection_or_text(&key_with_modifiers(
            egui::Key::H,
            control
        )));
        assert!(!event_may_change_selection_or_text(&key_with_modifiers(
            egui::Key::Q,
            control
        )));
    }

    #[test]
    fn text_buffer_trait_operations_preserve_their_contract() {
        let mut value = "aéx".to_owned();
        {
            let mut buffer = BoundedTextBuffer::new(&mut value, 8);
            assert!(buffer.is_mutable());
            buffer.delete_char_range(egui::text::CharIndex(1)..egui::text::CharIndex(2));
            assert_eq!(buffer.as_str(), "ax");
            buffer.replace_with("ééééé");
            assert_eq!(buffer.as_str(), "éééé");
            buffer.clear();
            assert_eq!(buffer.as_str(), "");
            buffer.replace_with("kept");
            assert_eq!(buffer.take(), "kept");
            assert_eq!(buffer.as_str(), "");
        }
        assert_eq!(value, "");
    }

    #[test]
    fn inverted_character_ranges_sort_instead_of_panicking() {
        let mut value = "abcd".to_owned();
        let mut buffer = BoundedTextBuffer::new(&mut value, 8);
        buffer.delete_char_range(egui::text::CharIndex(3)..egui::text::CharIndex(1));
        assert_eq!(buffer.as_str(), "ad");
    }

    #[test]
    fn navigation_before_ime_preedit_drops_an_unprovable_active_subrange() {
        let context = egui::Context::default();
        let id = egui::Id::new("ime-navigation-range-test");
        context.memory_mut(|memory| memory.request_focus(id));
        let mut state = egui::TextEdit::load_state(&context, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(1),
            )));
        egui::TextEdit::store_state(&context, id, state);
        let input = egui::RawInput {
            events: vec![
                key(egui::Key::ArrowRight),
                egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: "é".to_owned(),
                    active_range_chars: Some(0..1),
                }),
            ],
            ..Default::default()
        };

        let _ = context.run_ui(input, |ui| {
            assert!(!sanitize_bounded_text_events(ui, id, "aé", 4));
            let active_range = ui.input(|input| match &input.events[1] {
                egui::Event::Ime(egui::ImeEvent::Preedit {
                    active_range_chars, ..
                }) => active_range_chars.clone(),
                _ => panic!("the IME preedit event should remain available"),
            });
            assert_eq!(active_range, None);
        });
    }

    #[test]
    fn enter_and_locked_focus_tab_are_blocked_at_the_exact_ceiling() {
        for (event, lock_focus) in [(key(egui::Key::Enter), false), (key(egui::Key::Tab), true)] {
            let (value, was_limited) = edit_with_events("full", 4, 4..4, vec![event], lock_focus);
            assert_eq!(value, "full");
            assert!(was_limited);
        }
    }

    #[test]
    fn selection_replacements_receive_the_freed_byte_budget() {
        for (event, expected, lock_focus) in [
            (key(egui::Key::Enter), "f\nl", false),
            (key(egui::Key::Tab), "f\tl", true),
        ] {
            let (value, was_limited) = edit_with_events("full", 4, 1..3, vec![event], lock_focus);
            assert_eq!(value, expected);
            assert!(!was_limited);
        }
    }

    #[test]
    fn cursor_change_before_paste_cannot_reuse_a_stale_selection_budget() {
        let events = vec![
            key(egui::Key::ArrowRight),
            egui::Event::Paste("extra".to_owned()),
        ];
        let (value, was_limited) = edit_with_events("full", 4, 0..2, events, false);
        assert_eq!(value, "full");
        assert!(was_limited);
    }

    #[test]
    fn cursor_change_before_ime_commit_cannot_exceed_the_ceiling() {
        let events = vec![
            key(egui::Key::ArrowRight),
            egui::Event::Ime(egui::ImeEvent::Commit("extra".to_owned())),
        ];
        let (value, was_limited) = edit_with_events("full", 4, 0..2, events, false);
        assert_eq!(value, "full");
        assert!(was_limited);
    }
}
