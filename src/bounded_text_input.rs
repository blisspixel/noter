use eframe::egui;

/// Truncates a string to an exact UTF-8 byte ceiling without splitting a
/// scalar value.
pub fn truncate_to_utf8_byte_limit(value: &mut String, maximum: usize) -> bool {
    if value.len() <= maximum {
        return false;
    }
    let mut boundary = maximum;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value.truncate(boundary);
    true
}

/// Bounds text-bearing input events before `TextEdit` can normalize or retain
/// their complete payload.
pub fn sanitize_bounded_text_events(
    ui: &egui::Ui,
    id: egui::Id,
    current: &str,
    maximum: usize,
) -> bool {
    let selected_bytes = egui::TextEdit::load_state(ui.ctx(), id)
        .and_then(|state| state.cursor.char_range())
        .map_or(0, |range| {
            let primary = char_index_to_byte(current, range.primary.index.into());
            let secondary = char_index_to_byte(current, range.secondary.index.into());
            primary.abs_diff(secondary)
        });
    let retained = current.len().saturating_sub(selected_bytes);
    let mut remaining = maximum.saturating_sub(retained);
    ui.input_mut(|input| {
        let mut clamped = false;
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
                    if truncate_to_utf8_byte_limit(text, remaining) {
                        *active_range_chars = None;
                        clamped = true;
                    }
                }
                _ => {}
            }
        }
        clamped
    })
}

fn char_index_to_byte(source: &str, character: usize) -> usize {
    source
        .char_indices()
        .nth(character)
        .map_or(source.len(), |(offset, _)| offset)
}
