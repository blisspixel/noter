use std::{any::TypeId, cell::RefCell, ops::Range};

use eframe::egui;
use noter::core::line_endings::{LineEndingInsertionContext, normalize_inserted_text};

/// The authoritative action implied by the focused editor's IME events for
/// this frame. A composing value belongs only to the widget's transient buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImeFrameState {
    None,
    Composing,
    Committed,
    Cancelled,
}

/// Focus to restore after an active composition consumes its final Commit.
pub struct ImeCommitFocusRestore {
    displaced: Option<egui::Id>,
}

impl ImeCommitFocusRestore {
    pub fn restore(self, ui: &egui::Ui, editor_id: egui::Id) {
        ui.memory_mut(|memory| {
            if let Some(displaced) = self.displaced {
                memory.request_focus(displaced);
            } else {
                memory.surrender_focus(editor_id);
            }
        });
    }
}

/// Lets an already-active composition consume a final nonempty Commit even
/// when another control claimed focus earlier in the same UI frame. The new
/// focus owner is restored immediately after the editor processes the event.
pub fn retain_active_ime_commit_focus(
    ui: &egui::Ui,
    editor_id: egui::Id,
    composition_was_active: bool,
) -> Option<ImeCommitFocusRestore> {
    if !composition_was_active || ui.memory(|memory| memory.has_focus(editor_id)) {
        return None;
    }
    let has_commit = ui.input(|input| {
        input.events.iter().any(|event| {
            matches!(event, egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty())
        })
    });
    if !has_commit {
        return None;
    }
    let displaced = ui.memory(egui::Memory::focused);
    ui.memory_mut(|memory| memory.request_focus(editor_id));
    Some(ImeCommitFocusRestore { displaced })
}

/// Removes an active composition's final Commit until the document editor is
/// rendered. Events before the Commit remain in place; only later events are
/// returned for the caller to defer so event order is preserved.
pub fn isolate_active_ime_commit(
    ui: &egui::Ui,
    composition_was_active: bool,
) -> Option<(egui::Event, Vec<egui::Event>)> {
    if !composition_was_active {
        return None;
    }
    ui.input_mut(|input| {
        let position = input.events.iter().position(|event| {
            matches!(event, egui::Event::Ime(egui::ImeEvent::Commit(text)) if !text.is_empty())
        })?;
        let deferred = input.events.split_off(position + 1);
        let commit = input
            .events
            .pop()
            .expect("the located IME commit must still be present");
        Some((commit, deferred))
    })
}

/// Keeps a completed composition and any following input in separate frames.
/// This lets callers publish the commit before a second composition begins.
pub fn take_events_after_ime_terminal(
    ui: &egui::Ui,
    owns_events: bool,
    composition_was_active: bool,
) -> Vec<egui::Event> {
    if !owns_events {
        return Vec::new();
    }
    ui.input_mut(|input| {
        deduplicate_adjacent_newline_commits(&mut input.events);
        let mut composing = composition_was_active;
        let terminal = input.events.iter().position(|event| match event {
            egui::Event::Ime(egui::ImeEvent::Preedit { text, .. })
                if text != "\n" && text != "\r" =>
            {
                if text.is_empty() {
                    let was_composing = composing;
                    composing = false;
                    was_composing
                } else {
                    composing = true;
                    false
                }
            }
            egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                let is_terminal = !text.is_empty() || composing;
                composing = false;
                is_terminal
            }
            _ => false,
        });
        terminal
            .filter(|position| position + 1 < input.events.len())
            .map_or_else(Vec::new, |position| input.events.split_off(position + 1))
    })
}

fn deduplicate_adjacent_newline_commits(events: &mut [egui::Event]) {
    for enter in 0..events.len() {
        if !is_plain_enter_press(&events[enter]) {
            continue;
        }
        let paired_commit = enter
            .checked_sub(1)
            .filter(|&index| is_newline_commit(&events[index]))
            .or_else(|| {
                let index = enter + 1;
                (index < events.len() && is_newline_commit(&events[index])).then_some(index)
            });
        if let Some(index) = paired_commit
            && let egui::Event::Ime(egui::ImeEvent::Commit(text)) = &mut events[index]
        {
            // Native integrations can emit adjacent Enter and newline Commit
            // events for one action. Empty only that one matched terminal so
            // additional commits remain ordered user input.
            text.clear();
        }
    }
}

fn is_newline_commit(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Ime(egui::ImeEvent::Commit(text)) if is_single_logical_newline(text)
    )
}

/// Mirrors egui 0.35's `TextEdit` IME lifecycle without exposing its private
/// cursor-purpose state. Empty preedit/commit payloads only cancel an active
/// composition. Projected editors adapt newline-only commits before the widget
/// so every supported payload form completes exactly one logical insertion.
pub fn focused_ime_frame_state(
    ui: &egui::Ui,
    id: egui::Id,
    composition_was_active: bool,
) -> ImeFrameState {
    if !ui.memory(|memory| memory.has_focus(id)) {
        return if composition_was_active {
            ImeFrameState::Cancelled
        } else {
            ImeFrameState::None
        };
    }

    ui.input(|input| {
        let enter_pressed = input.events.iter().any(is_plain_enter_press);
        let mut composing = composition_was_active;
        let mut terminal = ImeFrameState::None;
        for event in &input.events {
            match event {
                egui::Event::Ime(egui::ImeEvent::Preedit { text, .. })
                    if text != "\n" && text != "\r" =>
                {
                    if text.is_empty() {
                        if composing {
                            composing = false;
                            terminal = ImeFrameState::Cancelled;
                        }
                    } else {
                        composing = true;
                        terminal = ImeFrameState::Composing;
                    }
                }
                egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    if text.is_empty() {
                        if composing {
                            composing = false;
                            terminal = if enter_pressed {
                                ImeFrameState::Committed
                            } else {
                                ImeFrameState::Cancelled
                            };
                        }
                    } else {
                        composing = false;
                        terminal = ImeFrameState::Committed;
                    }
                }
                _ => {}
            }
        }
        if composing {
            ImeFrameState::Composing
        } else {
            terminal
        }
    })
}

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

/// Bounds text-bearing events for a projected editor against exact source bytes.
///
/// Display selections are mapped back to source before calculating replacement
/// capacity. Newline expansion is measured using the insertion convention that
/// the projected buffer will apply at the pre-edit source position.
pub fn sanitize_projected_text_events(
    ui: &egui::Ui,
    id: egui::Id,
    current_source: &str,
    maximum: usize,
    insertion_context: LineEndingInsertionContext,
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

    let selection = egui::TextEdit::load_state(ui.ctx(), id)
        .and_then(|state| state.cursor.char_range())
        .and_then(|range| display_selection_to_source(current_source, range))
        .unwrap_or_else(|| noter::core::edit::Selection::caret(current_source.len()));
    let selected = selection.ordered_range();
    let selected_bytes = selected.end() - selected.start();
    let retained = current_source.len().saturating_sub(selected_bytes);
    let Some(ending) = insertion_context.insertion_at(current_source, selected.start()) else {
        return false;
    };
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
                egui::Event::Paste(text) | egui::Event::Text(text) => {
                    let normalized = normalize_inserted_text(text, ending, remaining);
                    let consumed = normalized.consumed_input_bytes();
                    let was_limited = normalized.was_limited();
                    remaining = remaining.saturating_sub(normalized.text().len());
                    canonicalize_projected_payload(text, consumed);
                    clamped |= was_limited;
                }
                egui::Event::Ime(egui::ImeEvent::Commit(text)) => {
                    let is_newline_only = is_single_logical_newline(text);
                    let normalized = normalize_inserted_text(text, ending, remaining);
                    let consumed = normalized.consumed_input_bytes();
                    let was_limited = normalized.was_limited();
                    remaining = remaining.saturating_sub(normalized.text().len());
                    if is_newline_only {
                        text.clear();
                        if normalized.text().is_empty() {
                            // An active composition must observe a rejected
                            // terminal as cancellation, not publish its
                            // transient pre-edit draft as a committed edit.
                        } else {
                            // egui 0.35 ignores sole LF and CR IME commits.
                            // Keep a two-scalar logical-newline sentinel through
                            // its event guard after exact normalization accepts
                            // the complete logical newline.
                            text.push_str("\r\n");
                        }
                    } else {
                        canonicalize_projected_payload(text, consumed);
                    }
                    clamped |= was_limited;
                }
                egui::Event::Ime(egui::ImeEvent::Preedit {
                    text,
                    active_range_chars,
                }) => {
                    let normalized = normalize_inserted_text(text, ending, remaining);
                    let consumed = normalized.consumed_input_bytes();
                    let was_limited = normalized.was_limited();
                    let projected_active_range = active_range_chars.as_ref().and_then(|range| {
                        project_payload_character_range(&text[..consumed], range.clone())
                    });
                    canonicalize_projected_payload(text, consumed);
                    if was_limited || selection_budget_may_be_stale {
                        *active_range_chars = None;
                    } else {
                        *active_range_chars = projected_active_range;
                    }
                    clamped |= was_limited;
                }
                event => {
                    selection_budget_may_be_stale |= event_may_change_selection_or_text(event);
                }
            }
        }
        clamped
    })
}

fn is_single_logical_newline(text: &str) -> bool {
    matches!(text, "\n" | "\r" | "\r\n")
}

fn is_plain_enter_press(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key {
            key: egui::Key::Enter,
            pressed: true,
            modifiers,
            ..
        } if *modifiers == egui::Modifiers::NONE
    )
}

fn canonicalize_projected_payload(text: &mut String, accepted_bytes: usize) {
    if text[..accepted_bytes].contains('\r') {
        let canonical = SourceDisplayProjection::new(&text[..accepted_bytes]);
        canonical.display().clone_into(text);
    } else if accepted_bytes < text.len() {
        text.truncate(accepted_bytes);
    }
}

fn project_payload_character_range(
    payload: &str,
    character_range: Range<usize>,
) -> Option<Range<usize>> {
    if character_range.start > character_range.end || character_range.end > payload.chars().count()
    {
        return None;
    }
    if !payload.contains('\r') {
        return Some(character_range);
    }

    let source_bytes = byte_range_from_char_range(
        payload,
        egui::text::CharIndex(character_range.start)..egui::text::CharIndex(character_range.end),
    );
    let projected = SourceDisplayProjection::new(payload).selection_to_display(
        noter::core::edit::Selection::new(source_bytes.start, source_bytes.end),
    )?;
    let [start, end] = projected.sorted_cursors();
    Some(usize::from(start.index)..usize::from(end.index))
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

mod projected {
    use std::{any::TypeId, cell::RefCell, ops::Range};

    use eframe::egui;
    use noter::core::{
        edit::Selection,
        line_endings::{LineEnding, LineEndingInsertionContext, normalize_inserted_text},
    };

    use super::{byte_range_from_char_range, utf8_prefix};

    /// A canonical LF display of source that retains exact source byte mappings.
    ///
    /// CRLF projects to one LF display character and bare CR projects to LF.
    /// Every other Unicode scalar is preserved exactly.
    #[derive(Clone, PartialEq, Eq, Debug)]
    pub struct SourceDisplayProjection {
        display: String,
        source_length: usize,
        collapsed_source_ends: Vec<usize>,
    }

    /// Reusable canonical display storage for an unchanged source generation.
    ///
    /// A cache returned by [`ProjectedTextBuffer::into_cache`] is valid only
    /// while the caller knows the source has not changed outside a projected
    /// buffer. Passing `None` to [`ProjectedTextBuffer::new_reusing`] rebuilds
    /// the exact projection.
    #[derive(Debug)]
    pub struct ProjectedTextCache {
        projection: Option<SourceDisplayProjection>,
    }

    #[cfg(test)]
    impl ProjectedTextCache {
        pub(crate) fn storage_identity(&self) -> Option<(*const u8, *const usize, usize)> {
            self.projection.as_ref().map(|projection| {
                (
                    projection.display.as_ptr(),
                    projection.collapsed_source_ends.as_ptr(),
                    projection.collapsed_source_ends.len(),
                )
            })
        }
    }

    impl SourceDisplayProjection {
        /// Builds a canonical display and its exact boundary mapping.
        pub fn new(source: &str) -> Self {
            let mut display = String::with_capacity(source.len());
            let mut collapsed_source_ends = Vec::new();
            let mut source_offset = 0;
            while source_offset < source.len() {
                let bytes = source.as_bytes();
                if bytes[source_offset] == b'\r' {
                    display.push('\n');
                    if bytes.get(source_offset + 1) == Some(&b'\n') {
                        source_offset += 2;
                        collapsed_source_ends.push(source_offset);
                    } else {
                        source_offset += 1;
                    }
                } else {
                    let character_length = source[source_offset..]
                        .chars()
                        .next()
                        .map_or(source.len() - source_offset, char::len_utf8);
                    display.push_str(&source[source_offset..source_offset + character_length]);
                    source_offset += character_length;
                }
            }

            Self {
                display,
                source_length: source.len(),
                collapsed_source_ends,
            }
        }

        /// Returns the canonical LF text presented to `TextEdit`.
        pub fn display(&self) -> &str {
            &self.display
        }

        /// Maps one canonical display character boundary to its exact source byte boundary.
        pub fn display_char_to_source_byte(
            &self,
            display_index: egui::text::CharIndex,
        ) -> Option<usize> {
            let display_byte = byte_index_at_char(&self.display, display_index)?;
            let collapsed = self.collapsed_count_at_display_byte(display_byte);
            Some(display_byte + collapsed)
        }

        /// Maps one exact source byte boundary to a canonical display character boundary.
        ///
        /// A byte offset between CR and LF is not a valid boundary and returns `None`.
        pub fn source_byte_to_display_char(
            &self,
            source_byte: usize,
        ) -> Option<egui::text::CharIndex> {
            if source_byte > self.source_length || self.is_crlf_midpoint(source_byte) {
                return None;
            }
            let collapsed = self
                .collapsed_source_ends
                .partition_point(|&end| end <= source_byte);
            let display_byte = source_byte.checked_sub(collapsed)?;
            self.display
                .is_char_boundary(display_byte)
                .then(|| egui::text::CharIndex(self.display[..display_byte].chars().count()))
        }

        /// Converts a directional display selection to exact source byte offsets.
        pub fn selection_to_source(&self, range: egui::text::CCursorRange) -> Option<Selection> {
            let anchor = self.display_char_to_source_byte(range.secondary.index)?;
            let active = self.display_char_to_source_byte(range.primary.index)?;
            Some(Selection::new(anchor, active))
        }

        /// Converts a directional source selection to canonical display cursors.
        ///
        /// A caret between CR and LF snaps before the pair. A nonempty selection
        /// expands outward so selecting either byte selects the displayed newline.
        pub fn selection_to_display(
            &self,
            selection: Selection,
        ) -> Option<egui::text::CCursorRange> {
            let anchor = self.canonical_source_endpoint(selection, selection.anchor())?;
            let active = self.canonical_source_endpoint(selection, selection.active())?;
            Some(egui::text::CCursorRange {
                primary: egui::text::CCursor::new(self.source_byte_to_display_char(active)?),
                secondary: egui::text::CCursor::new(self.source_byte_to_display_char(anchor)?),
                h_pos: None,
            })
        }

        fn canonical_source_endpoint(
            &self,
            selection: Selection,
            endpoint: usize,
        ) -> Option<usize> {
            if endpoint > self.source_length {
                return None;
            }
            let Ok(pair_index) = self
                .collapsed_source_ends
                .binary_search(&endpoint.saturating_add(1))
            else {
                return self.source_byte_to_display_char(endpoint).map(|_| endpoint);
            };
            let pair_end = self.collapsed_source_ends[pair_index];
            if selection.anchor() == selection.active()
                || endpoint == selection.ordered_range().start()
            {
                Some(pair_end - 2)
            } else {
                Some(pair_end)
            }
        }

        fn is_crlf_midpoint(&self, source_byte: usize) -> bool {
            self.collapsed_source_ends
                .binary_search(&source_byte.saturating_add(1))
                .is_ok()
        }

        fn collapsed_count_at_display_byte(&self, display_byte: usize) -> usize {
            let mut start = 0;
            let mut end = self.collapsed_source_ends.len();
            while start < end {
                let midpoint = start + (end - start) / 2;
                let collapsed_display_end = self.collapsed_source_ends[midpoint] - (midpoint + 1);
                if collapsed_display_end <= display_byte {
                    start = midpoint + 1;
                } else {
                    end = midpoint;
                }
            }
            start
        }
    }

    /// Maps a canonical display selection without allocating for LF-only text.
    pub fn display_selection_to_source(
        source: &str,
        range: egui::text::CCursorRange,
    ) -> Option<Selection> {
        if source.contains('\r') {
            SourceDisplayProjection::new(source).selection_to_source(range)
        } else {
            let anchor = byte_index_at_char(source, range.secondary.index)?;
            let active = byte_index_at_char(source, range.primary.index)?;
            Some(Selection::new(anchor, active))
        }
    }

    /// Maps an exact source selection without allocating for LF-only text.
    pub fn source_selection_to_display(
        source: &str,
        selection: Selection,
    ) -> Option<egui::text::CCursorRange> {
        if source.contains('\r') {
            SourceDisplayProjection::new(source).selection_to_display(selection)
        } else {
            let cursor = |byte: usize| {
                (byte <= source.len() && source.is_char_boundary(byte))
                    .then(|| egui::text::CCursor::new(source[..byte].chars().count()))
            };
            Some(egui::text::CCursorRange {
                primary: cursor(selection.active())?,
                secondary: cursor(selection.anchor())?,
                h_pos: None,
            })
        }
    }

    /// A bounded `TextEdit` buffer backed by exact, non-normalized source text.
    ///
    /// The widget sees canonical LF text. Mutations are mapped back to source,
    /// inserted newlines follow the supplied document context, and untouched line
    /// endings retain their original bytes.
    pub struct ProjectedTextBuffer<'a> {
        source: &'a mut String,
        projection: Option<SourceDisplayProjection>,
        insertion_context: LineEndingInsertionContext,
        maximum: usize,
        was_limited: bool,
        recent_deletion: RefCell<Option<ProjectedRecentDeletion>>,
    }

    impl<'a> ProjectedTextBuffer<'a> {
        /// Creates a projected buffer with an exact source-byte ceiling.
        pub fn new(
            source: &'a mut String,
            maximum: usize,
            insertion_context: LineEndingInsertionContext,
        ) -> Self {
            Self::new_reusing(source, maximum, insertion_context, None)
        }

        /// Creates a projected buffer, reusing an unchanged source projection.
        pub fn new_reusing(
            source: &'a mut String,
            maximum: usize,
            insertion_context: LineEndingInsertionContext,
            cache: Option<ProjectedTextCache>,
        ) -> Self {
            let was_limited = truncate_source_to_byte_limit(source, maximum);
            let projection = if was_limited {
                projection_for_source(source)
            } else {
                cache.map_or_else(
                    || projection_for_source(source),
                    |cache| match (source.contains('\r'), cache.projection) {
                        (true, Some(projection)) => Some(projection),
                        (false, None) => None,
                        _ => projection_for_source(source),
                    },
                )
            };
            Self {
                source,
                projection,
                insertion_context,
                maximum,
                was_limited,
                recent_deletion: RefCell::new(None),
            }
        }

        /// Reports whether the source-byte ceiling excluded any content.
        pub const fn was_limited(&self) -> bool {
            self.was_limited
        }

        /// Returns the current projection for reuse with unchanged source.
        pub fn into_cache(self) -> ProjectedTextCache {
            ProjectedTextCache {
                projection: self.projection,
            }
        }

        /// Converts a directional display selection to exact source byte offsets.
        pub fn selection_to_source(&self, range: egui::text::CCursorRange) -> Option<Selection> {
            self.projection.as_ref().map_or_else(
                || display_selection_to_source(self.source, range),
                |projection| projection.selection_to_source(range),
            )
        }

        /// Converts a directional source selection to canonical display cursors.
        pub fn selection_to_display(
            &self,
            selection: Selection,
        ) -> Option<egui::text::CCursorRange> {
            self.projection.as_ref().map_or_else(
                || source_selection_to_display(self.source, selection),
                |projection| projection.selection_to_display(selection),
            )
        }

        fn rebuild_projection(&mut self) {
            self.projection = projection_for_source(self.source);
        }

        fn display(&self) -> &str {
            self.projection
                .as_ref()
                .map_or(self.source.as_str(), SourceDisplayProjection::display)
        }

        fn display_char_to_source_byte(
            &self,
            display_index: egui::text::CharIndex,
        ) -> Option<usize> {
            self.projection.as_ref().map_or_else(
                || byte_index_at_char(self.source, display_index),
                |projection| projection.display_char_to_source_byte(display_index),
            )
        }

        fn source_byte_to_display_char(&self, source_byte: usize) -> Option<egui::text::CharIndex> {
            self.projection.as_ref().map_or_else(
                || {
                    (source_byte <= self.source.len() && self.source.is_char_boundary(source_byte))
                        .then(|| egui::text::CharIndex(self.source[..source_byte].chars().count()))
                },
                |projection| projection.source_byte_to_display_char(source_byte),
            )
        }

        fn insert_source_text(
            &mut self,
            text: &str,
            display_index: egui::text::CharIndex,
        ) -> usize {
            let Some(source_byte) = self.display_char_to_source_byte(display_index) else {
                return 0;
            };
            let recent_deletion = self.recent_deletion.borrow_mut().take();
            let ending = recent_deletion
                .as_ref()
                .filter(|deletion| deletion.display_start == display_index)
                .map_or_else(
                    || {
                        self.insertion_context
                            .insertion_at(self.source, source_byte)
                            .expect("a projected boundary cannot split source CRLF")
                    },
                    |deletion| deletion.ending,
                );
            let remaining = self.maximum.saturating_sub(self.source.len());
            let normalized = normalize_inserted_text(text, ending, remaining);
            self.was_limited |= normalized.was_limited();

            if !text.is_empty()
                && normalized.text().is_empty()
                && let Some(deletion) = recent_deletion
                && deletion.display_start == display_index
                && let Some(removed) = deletion.removed
            {
                self.source.insert_str(deletion.source_start, &removed);
                self.rebuild_projection();
                return 0;
            }

            let inserted_source_length = normalized.text().len();
            self.source.insert_str(source_byte, normalized.text());
            self.rebuild_projection();
            let display_end = self
                .source_byte_to_display_char(source_byte + inserted_source_length)
                .expect("normalized insertion must end on a projected boundary");
            usize::from(display_end).saturating_sub(usize::from(display_index))
        }
    }

    struct ProjectedTextBufferType;

    struct ProjectedRecentDeletion {
        display_start: egui::text::CharIndex,
        source_start: usize,
        removed: Option<String>,
        ending: LineEnding,
    }

    impl egui::TextBuffer for ProjectedTextBuffer<'_> {
        fn is_mutable(&self) -> bool {
            true
        }

        fn as_str(&self) -> &str {
            self.recent_deletion.borrow_mut().take();
            self.display()
        }

        fn insert_text(&mut self, text: &str, char_index: egui::text::CharIndex) -> usize {
            self.insert_source_text(text, char_index)
        }

        fn delete_char_range(&mut self, char_range: Range<egui::text::CharIndex>) {
            self.recent_deletion.borrow_mut().take();
            let start = char_range.start.min(char_range.end);
            let end = char_range.start.max(char_range.end);
            let Some(source_start) = self.display_char_to_source_byte(start) else {
                return;
            };
            let Some(source_end) = self.display_char_to_source_byte(end) else {
                return;
            };
            self.source.drain(source_start..source_end);
            self.rebuild_projection();
        }

        fn insert_text_at(
            &mut self,
            ccursor: &mut egui::text::CCursor,
            text_to_insert: &str,
            char_limit: usize,
        ) {
            // Counting characters is proportional to the document. An absent
            // limit cannot clamp anything, so the count must not run on the
            // ordinary keystroke path. egui's own buffer guards this the same
            // way; only the bounded find fields set a limit.
            let text_to_insert = if char_limit == usize::MAX {
                text_to_insert
            } else {
                let current_characters = self.display().chars().count();
                let available_characters = char_limit.saturating_sub(current_characters);
                logical_prefix(text_to_insert, available_characters)
            };
            ccursor.index += self.insert_source_text(text_to_insert, ccursor.index);
        }

        fn delete_selected(
            &mut self,
            cursor_range: &egui::text::CCursorRange,
        ) -> egui::text::CCursor {
            let [start, end] = cursor_range.sorted_cursors();
            let Some(source_start) = self.display_char_to_source_byte(start.index) else {
                return start;
            };
            let Some(source_end) = self.display_char_to_source_byte(end.index) else {
                return start;
            };
            let ending = self
                .insertion_context
                .insertion_at(self.source, source_start)
                .expect("a projected boundary cannot split source CRLF");
            let removed_length = source_end - source_start;
            let removed = (1..=4)
                .contains(&removed_length)
                .then(|| self.source[source_start..source_end].to_owned());
            self.source.drain(source_start..source_end);
            self.rebuild_projection();
            *self.recent_deletion.borrow_mut() = Some(ProjectedRecentDeletion {
                display_start: start.index,
                source_start,
                removed,
                ending,
            });
            egui::text::CCursor {
                index: start.index,
                prefer_next_row: true,
            }
        }

        fn clear(&mut self) {
            self.recent_deletion.borrow_mut().take();
            self.source.clear();
            self.rebuild_projection();
        }

        fn replace_with(&mut self, text: &str) {
            self.recent_deletion.borrow_mut().take();
            let canonical_replacement = text
                .contains('\r')
                .then(|| SourceDisplayProjection::new(text));
            let text = canonical_replacement
                .as_ref()
                .map_or(text, SourceDisplayProjection::display);
            let current = self.display();
            let prefix_characters = common_prefix_characters(current, text);
            let maximum_suffix = current
                .chars()
                .count()
                .min(text.chars().count())
                .saturating_sub(prefix_characters);
            let suffix_characters = common_suffix_characters(current, text, maximum_suffix);
            let current_end = current.chars().count() - suffix_characters;
            let replacement_end = text.chars().count() - suffix_characters;
            let source_start = self
                .display_char_to_source_byte(egui::text::CharIndex(prefix_characters))
                .expect("a common display prefix must end at a source boundary");
            let source_end = self
                .display_char_to_source_byte(egui::text::CharIndex(current_end))
                .expect("a common display suffix must start at a source boundary");
            let replacement_bytes = byte_range_from_char_range(
                text,
                egui::text::CharIndex(prefix_characters)..egui::text::CharIndex(replacement_end),
            );
            let retained = self
                .source
                .len()
                .saturating_sub(source_end.saturating_sub(source_start));
            let ending = self
                .insertion_context
                .insertion_at(self.source, source_start)
                .expect("a projected boundary cannot split source CRLF");
            self.source.replace_range(source_start..source_end, "");
            let normalized = normalize_inserted_text(
                &text[replacement_bytes],
                ending,
                self.maximum.saturating_sub(retained),
            );
            self.was_limited |= normalized.was_limited();
            self.source.insert_str(source_start, normalized.text());
            self.rebuild_projection();
        }

        fn take(&mut self) -> String {
            self.recent_deletion.borrow_mut().take();
            let display = self.display().to_owned();
            self.source.clear();
            self.rebuild_projection();
            display
        }

        fn type_id(&self) -> TypeId {
            TypeId::of::<ProjectedTextBufferType>()
        }
    }

    fn projection_for_source(source: &str) -> Option<SourceDisplayProjection> {
        source
            .contains('\r')
            .then(|| SourceDisplayProjection::new(source))
    }

    fn truncate_source_to_byte_limit(source: &mut String, maximum: usize) -> bool {
        if source.len() <= maximum {
            return false;
        }
        let mut boundary = utf8_prefix(source, maximum).len();
        if boundary > 0
            && source.as_bytes()[boundary - 1] == b'\r'
            && source.as_bytes().get(boundary) == Some(&b'\n')
        {
            boundary -= 1;
        }
        source.truncate(boundary);
        true
    }

    fn byte_index_at_char(source: &str, character: egui::text::CharIndex) -> Option<usize> {
        let character = usize::from(character);
        if character == source.chars().count() {
            return Some(source.len());
        }
        source.char_indices().nth(character).map(|(byte, _)| byte)
    }

    fn logical_prefix(source: &str, maximum_characters: usize) -> &str {
        if maximum_characters == usize::MAX {
            return source;
        }
        let mut offset = 0;
        let mut characters = 0;
        while offset < source.len() && characters < maximum_characters {
            if source.as_bytes()[offset] == b'\r'
                && source.as_bytes().get(offset + 1) == Some(&b'\n')
            {
                offset += 2;
            } else {
                offset += source[offset..]
                    .chars()
                    .next()
                    .expect("an in-range UTF-8 offset must contain a scalar")
                    .len_utf8();
            }
            characters += 1;
        }
        &source[..offset]
    }

    fn common_prefix_characters(left: &str, right: &str) -> usize {
        left.chars()
            .zip(right.chars())
            .take_while(|(left, right)| left == right)
            .count()
    }

    fn common_suffix_characters(left: &str, right: &str, maximum: usize) -> usize {
        left.chars()
            .rev()
            .zip(right.chars().rev())
            .take(maximum)
            .take_while(|(left, right)| left == right)
            .count()
    }
}

pub use projected::{
    ProjectedTextBuffer, ProjectedTextCache, SourceDisplayProjection, display_selection_to_source,
    source_selection_to_display,
};

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
        let text_to_insert = if char_limit == usize::MAX {
            text_to_insert
        } else {
            let cutoff = char_limit.saturating_sub(self.value.chars().count());
            text_to_insert
                .char_indices()
                .nth(cutoff)
                .map_or(text_to_insert, |(index, _)| &text_to_insert[..index])
        };
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
    use std::ops::Range;

    use super::{
        BoundedTextBuffer, ImeFrameState, ProjectedTextBuffer, SourceDisplayProjection,
        event_may_change_selection_or_text, focused_ime_frame_state, isolate_active_ime_commit,
        sanitize_bounded_text_events, sanitize_projected_text_events,
        take_events_after_ime_terminal, utf8_prefix,
    };
    use eframe::egui;
    use egui::TextBuffer as _;
    use noter::core::{
        edit::Selection,
        line_endings::{LineEnding, LineEndingInsertionContext, LineEndingProfile},
    };

    /// An absent character limit must not clamp, and a present one must.
    ///
    /// The document editor sets no character limit, so the skipped branch is
    /// the ordinary keystroke path; the bounded find fields do set one, so the
    /// clamping branch has to keep working exactly as before.
    #[test]
    fn a_character_limit_clamps_and_its_absence_costs_nothing() {
        let mut unbounded = String::from("existing");
        let mut buffer = BoundedTextBuffer::new(&mut unbounded, 1024);
        let mut cursor = egui::text::CCursor::new(8);
        buffer.insert_text_at(&mut cursor, " and more", usize::MAX);
        assert_eq!(unbounded, "existing and more");

        // A real limit still truncates to the characters that remain.
        let mut bounded = String::from("abc");
        let mut buffer = BoundedTextBuffer::new(&mut bounded, 1024);
        let mut cursor = egui::text::CCursor::new(3);
        buffer.insert_text_at(&mut cursor, "defgh", 5);
        assert_eq!(bounded, "abcde", "a present limit must still clamp");

        // A limit already reached admits nothing further.
        let mut full = String::from("abcde");
        let mut buffer = BoundedTextBuffer::new(&mut full, 1024);
        let mut cursor = egui::text::CCursor::new(5);
        buffer.insert_text_at(&mut cursor, "fgh", 5);
        assert_eq!(full, "abcde");
    }

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

    #[test]
    fn ime_terminal_defers_the_next_composition_without_reordering() {
        let context = egui::Context::default();
        let mut input = egui::RawInput::default();
        let commit = egui::Event::Ime(egui::ImeEvent::Commit("漢".to_owned()));
        let preedit = egui::Event::Ime(egui::ImeEvent::Preedit {
            text: "次".to_owned(),
            active_range_chars: Some(0..1),
        });
        let text = egui::Event::Text("tail".to_owned());
        input
            .events
            .extend([commit.clone(), preedit.clone(), text.clone()]);

        let mut retained = Vec::new();
        let mut deferred = Vec::new();
        let _ = context.run_ui(input, |ui| {
            deferred = take_events_after_ime_terminal(ui, true, false);
            retained = ui.input(|input| input.events.clone());
        });

        assert_eq!(retained, [commit]);
        assert_eq!(deferred, [preedit, text]);
    }

    #[test]
    fn isolated_ime_commit_keeps_prefix_and_defers_only_suffix() {
        let context = egui::Context::default();
        let before = egui::Event::Text("before".to_owned());
        let commit = egui::Event::Ime(egui::ImeEvent::Commit("committed".to_owned()));
        let after = egui::Event::Text("after".to_owned());
        let mut input = egui::RawInput::default();
        input
            .events
            .extend([before.clone(), commit.clone(), after.clone()]);
        let mut isolated = None;
        let mut retained = Vec::new();

        let _ = context.run_ui(input, |ui| {
            isolated = isolate_active_ime_commit(ui, true);
            retained = ui.input(|input| input.events.clone());
        });

        assert_eq!(retained, [before]);
        assert_eq!(isolated, Some((commit, vec![after])));
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

    fn context_for(source: &str) -> LineEndingInsertionContext {
        LineEndingProfile::detect(source)
            .insertion_context(source, 0..source.len())
            .expect("the full source must be a valid editable range")
    }

    fn run_serialized_projected_newlines(
        ending: LineEnding,
        composition_active: bool,
        events: Vec<egui::Event>,
    ) -> String {
        let context = egui::Context::default();
        let id = egui::Id::new("serialized-projected-newline-matrix");
        let mut source = format!("a{}b", ending.as_str());
        let insertion_context = context_for(&source);
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            let mut buffer = ProjectedTextBuffer::new(&mut source, 64, insertion_context);
            ui.add(egui::TextEdit::multiline(&mut buffer).id(id));
        });
        let mut state = egui::TextEdit::load_state(&context, id).unwrap_or_default();
        state.cursor.set_char_range(Some(if composition_active {
            egui::text::CCursorRange::two(egui::text::CCursor::new(2), egui::text::CCursor::new(3))
        } else {
            egui::text::CCursorRange::one(egui::text::CCursor::new(3))
        }));
        egui::TextEdit::store_state(&context, id, state);
        context.memory_mut(|memory| memory.request_focus(id));

        if composition_active {
            let preedit = egui::RawInput {
                events: vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: "x".to_owned(),
                    active_range_chars: None,
                })],
                ..Default::default()
            };
            let _ = context.run_ui(preedit, |ui| {
                assert!(take_events_after_ime_terminal(ui, true, false).is_empty());
                assert!(!sanitize_projected_text_events(
                    ui,
                    id,
                    &source,
                    64,
                    insertion_context,
                ));
                let mut buffer = ProjectedTextBuffer::new(&mut source, 64, insertion_context);
                ui.add(egui::TextEdit::multiline(&mut buffer).id(id));
            });
            assert_eq!(source, format!("a{}x", ending.as_str()));
        }

        let mut queued = events;
        let mut active = composition_active;
        for _ in 0..4 {
            if queued.is_empty() {
                break;
            }
            let input = egui::RawInput {
                events: std::mem::take(&mut queued),
                ..Default::default()
            };
            let mut deferred = Vec::new();
            let _ = context.run_ui(input, |ui| {
                deferred = take_events_after_ime_terminal(ui, true, active);
                assert!(!sanitize_projected_text_events(
                    ui,
                    id,
                    &source,
                    64,
                    insertion_context,
                ));
                let mut buffer = ProjectedTextBuffer::new(&mut source, 64, insertion_context);
                ui.add(egui::TextEdit::multiline(&mut buffer).id(id));
            });
            queued = deferred;
            active = false;
        }
        assert!(queued.is_empty(), "the bounded event queue must drain");
        source
    }

    fn sanitize_projected_preedit(
        active_range_chars: Range<usize>,
    ) -> (String, Option<Range<usize>>) {
        let source = "x\r\n";
        let context = egui::Context::default();
        let id = egui::Id::new("projected-preedit-sanitizer-test");
        context.memory_mut(|memory| memory.request_focus(id));
        let mut state = egui::TextEdit::load_state(&context, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(0),
            )));
        egui::TextEdit::store_state(&context, id, state);
        let input = egui::RawInput {
            events: vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                text: "a\r\nbc".to_owned(),
                active_range_chars: Some(active_range_chars),
            })],
            ..Default::default()
        };
        let mut sanitized = None;

        let _ = context.run_ui(input, |ui| {
            let insertion_context = context_for(source);
            assert!(!sanitize_projected_text_events(
                ui,
                id,
                source,
                64,
                insertion_context,
            ));
            sanitized = ui.input(|input| input.events.first().cloned());
        });

        let Some(egui::Event::Ime(egui::ImeEvent::Preedit {
            text,
            active_range_chars,
        })) = sanitized
        else {
            panic!("the projected preedit should remain available")
        };
        (text, active_range_chars)
    }

    #[test]
    fn projection_canonicalizes_crlf_and_cr_with_exact_boundary_mapping() {
        let projection = SourceDisplayProjection::new("a\r\né\rb\n");
        assert_eq!(projection.display(), "a\né\nb\n");

        for (display_character, source_byte) in [0, 1, 3, 5, 6, 7, 8].into_iter().enumerate() {
            let display_character = egui::text::CharIndex(display_character);
            assert_eq!(
                projection.display_char_to_source_byte(display_character),
                Some(source_byte)
            );
            assert_eq!(
                projection.source_byte_to_display_char(source_byte),
                Some(display_character)
            );
        }
        assert_eq!(projection.source_byte_to_display_char(2), None);
        assert_eq!(projection.source_byte_to_display_char(4), None);
        assert_eq!(
            projection.display_char_to_source_byte(egui::text::CharIndex(7)),
            None
        );
    }

    #[test]
    fn projected_sanitizer_maps_crlf_ime_ranges_before_across_and_after_newline() {
        for (original, expected) in [(0..1, 0..1), (1..3, 1..2), (3..5, 2..4)] {
            let (text, active_range) = sanitize_projected_preedit(original.clone());
            assert_eq!(text, "a\nbc", "{original:?}");
            assert_eq!(active_range, Some(expected), "{original:?}");
        }
    }

    #[test]
    fn projected_text_edit_accepts_mapped_crlf_ime_ranges_without_invalid_cursors() {
        for (original, expected) in [(0..1, 0..1), (1..3, 1..2), (3..5, 2..4)] {
            let context = egui::Context::default();
            let id = egui::Id::new(("projected-preedit-widget-test", original.start));
            let mut source = "x\r\n".to_owned();
            let insertion_context = context_for(&source);
            let _ = context.run_ui(egui::RawInput::default(), |ui| {
                let mut buffer = ProjectedTextBuffer::new(&mut source, 64, insertion_context);
                ui.add(egui::TextEdit::multiline(&mut buffer).id(id));
            });
            let mut state = egui::TextEdit::load_state(&context, id).unwrap_or_default();
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::one(
                    egui::text::CCursor::new(0),
                )));
            egui::TextEdit::store_state(&context, id, state);
            context.memory_mut(|memory| memory.request_focus(id));
            let input = egui::RawInput {
                events: vec![egui::Event::Ime(egui::ImeEvent::Preedit {
                    text: "a\r\nbc".to_owned(),
                    active_range_chars: Some(original.clone()),
                })],
                ..Default::default()
            };
            let mut sanitized_range = None;

            let output = context.run_ui(input, |ui| {
                assert!(!sanitize_projected_text_events(
                    ui,
                    id,
                    &source,
                    64,
                    insertion_context,
                ));
                sanitized_range = ui.input(|input| match &input.events[0] {
                    egui::Event::Ime(egui::ImeEvent::Preedit {
                        text,
                        active_range_chars,
                    }) => {
                        assert_eq!(text, "a\nbc");
                        active_range_chars.clone()
                    }
                    _ => panic!("the IME event should remain a preedit"),
                });
                let mut buffer = ProjectedTextBuffer::new(&mut source, 64, insertion_context);
                ui.add(egui::TextEdit::multiline(&mut buffer).id(id));
            });

            assert_eq!(sanitized_range, Some(expected), "{original:?}");
            assert_eq!(source, "a\r\nbcx\r\n", "{original:?}");
            assert!(!output.shapes.is_empty(), "{original:?}");
            let state = egui::TextEdit::load_state(&context, id)
                .expect("the projected editor should persist cursor state");
            assert_eq!(
                state
                    .cursor
                    .char_range()
                    .map(|range| range.as_sorted_char_range()),
                Some(egui::text::CharIndex(0)..egui::text::CharIndex(4)),
                "{original:?}"
            );
        }
    }

    #[test]
    fn projected_text_edit_inserts_each_newline_only_ime_commit_exactly_once() {
        for (source, expected) in [
            ("a\nb", "a\n\nb"),
            ("a\r\nb", "a\r\n\r\nb"),
            ("a\rb", "a\r\rb"),
        ] {
            for payload in ["\n", "\r", "\r\n"] {
                let context = egui::Context::default();
                let id = egui::Id::new(("newline-only-ime-commit", source, payload));
                let mut actual = source.to_owned();
                let insertion_context = context_for(&actual);
                let _ = context.run_ui(egui::RawInput::default(), |ui| {
                    let mut buffer = ProjectedTextBuffer::new(&mut actual, 64, insertion_context);
                    ui.add(egui::TextEdit::multiline(&mut buffer).id(id));
                });
                let mut state = egui::TextEdit::load_state(&context, id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(
                        egui::text::CCursor::new(1),
                    )));
                egui::TextEdit::store_state(&context, id, state);
                context.memory_mut(|memory| memory.request_focus(id));
                let input = egui::RawInput {
                    events: vec![egui::Event::Ime(egui::ImeEvent::Commit(payload.to_owned()))],
                    ..Default::default()
                };

                let _ = context.run_ui(input, |ui| {
                    assert!(!sanitize_projected_text_events(
                        ui,
                        id,
                        &actual,
                        64,
                        insertion_context,
                    ));
                    assert!(matches!(
                        ui.input(|input| input.events.first().cloned()),
                        Some(egui::Event::Ime(egui::ImeEvent::Commit(text))) if text == "\r\n"
                    ));
                    let mut buffer = ProjectedTextBuffer::new(&mut actual, 64, insertion_context);
                    ui.add(egui::TextEdit::multiline(&mut buffer).id(id));
                });

                assert_eq!(actual, expected, "source {source:?}, payload {payload:?}");
            }
        }
    }

    #[test]
    fn projected_sanitizer_cancels_a_newline_terminal_rejected_by_crlf_ceiling() {
        let context = egui::Context::default();
        let id = egui::Id::new("rejected-newline-ime-terminal");
        let source = "a\r\nb";
        let insertion_context = context_for(source);
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            let mut draft = source.to_owned();
            let mut buffer = ProjectedTextBuffer::new(&mut draft, source.len(), insertion_context);
            ui.add(egui::TextEdit::multiline(&mut buffer).id(id));
        });
        let mut state = egui::TextEdit::load_state(&context, id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(
                egui::text::CCursor::new(source.chars().count()),
            )));
        egui::TextEdit::store_state(&context, id, state);
        context.memory_mut(|memory| memory.request_focus(id));
        let input = egui::RawInput {
            events: vec![egui::Event::Ime(egui::ImeEvent::Commit("\n".to_owned()))],
            ..Default::default()
        };

        let _ = context.run_ui(input, |ui| {
            assert_eq!(
                focused_ime_frame_state(ui, id, true),
                ImeFrameState::Committed
            );
            assert!(sanitize_projected_text_events(
                ui,
                id,
                source,
                source.len(),
                insertion_context,
            ));
            assert!(matches!(
                ui.input(|input| input.events.first().cloned()),
                Some(egui::Event::Ime(egui::ImeEvent::Commit(text))) if text.is_empty()
            ));
            assert_eq!(
                focused_ime_frame_state(ui, id, true),
                ImeFrameState::Cancelled
            );
        });
    }

    #[test]
    fn projected_text_edit_deduplicates_enter_with_a_newline_ime_commit() {
        for payload in ["\n", "\r", "\r\n"] {
            for events in [
                vec![
                    key(egui::Key::Enter),
                    egui::Event::Ime(egui::ImeEvent::Commit(payload.to_owned())),
                ],
                vec![
                    egui::Event::Ime(egui::ImeEvent::Commit(payload.to_owned())),
                    key(egui::Key::Enter),
                ],
            ] {
                let context = egui::Context::default();
                let id = egui::Id::new(("deduplicated-newline-ime-commit", payload, events.len()));
                let mut source = "a\r\nb".to_owned();
                let insertion_context = context_for(&source);
                let _ = context.run_ui(egui::RawInput::default(), |ui| {
                    let mut buffer = ProjectedTextBuffer::new(&mut source, 64, insertion_context);
                    ui.add(egui::TextEdit::multiline(&mut buffer).id(id));
                });
                let mut state = egui::TextEdit::load_state(&context, id).unwrap_or_default();
                state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(
                        egui::text::CCursor::new(1),
                    )));
                egui::TextEdit::store_state(&context, id, state);
                context.memory_mut(|memory| memory.request_focus(id));
                let input = egui::RawInput {
                    events,
                    ..Default::default()
                };

                let _ = context.run_ui(input, |ui| {
                    assert!(take_events_after_ime_terminal(ui, true, false).is_empty());
                    assert!(!sanitize_projected_text_events(
                        ui,
                        id,
                        &source,
                        64,
                        insertion_context,
                    ));
                    let mut buffer = ProjectedTextBuffer::new(&mut source, 64, insertion_context);
                    ui.add(egui::TextEdit::multiline(&mut buffer).id(id));
                });

                assert_eq!(source, "a\r\n\r\nb", "payload {payload:?}");
            }
        }
    }

    #[test]
    fn projected_newline_deduplication_preserves_unmatched_ordered_actions() {
        let commit = || egui::Event::Ime(egui::ImeEvent::Commit("\n".to_owned()));
        for ending in [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr] {
            for composition_active in [false, true] {
                for events in [
                    vec![commit(), commit(), key(egui::Key::Enter)],
                    vec![key(egui::Key::Enter), commit(), commit()],
                    vec![commit(), key(egui::Key::Enter), key(egui::Key::Enter)],
                    vec![key(egui::Key::Enter), key(egui::Key::Enter), commit()],
                ] {
                    let actual = run_serialized_projected_newlines(
                        ending,
                        composition_active,
                        events.clone(),
                    );
                    let expected = if composition_active {
                        format!("a{0}{0}{0}", ending.as_str())
                    } else {
                        format!("a{0}b{0}{0}", ending.as_str())
                    };
                    assert_eq!(
                        actual, expected,
                        "{ending:?} active={composition_active} events={events:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn projected_cache_reuses_large_crlf_and_cr_storage_and_lf_needs_none() {
        const LARGE_SOURCE_BYTES: usize = 8 << 20;

        for ending in ["\r\n", "\r"] {
            let mut source = ending.repeat(LARGE_SOURCE_BYTES / ending.len());
            let insertion_context = context_for(&source);
            let buffer =
                ProjectedTextBuffer::new(&mut source, LARGE_SOURCE_BYTES, insertion_context);
            let cache = buffer.into_cache();
            let first_storage = cache
                .storage_identity()
                .expect("CR source should own canonical display storage");
            assert_eq!(
                first_storage.2,
                LARGE_SOURCE_BYTES / 2 * usize::from(ending == "\r\n")
            );

            let buffer = ProjectedTextBuffer::new_reusing(
                &mut source,
                LARGE_SOURCE_BYTES,
                insertion_context,
                Some(cache),
            );
            let cache = buffer.into_cache();
            assert_eq!(cache.storage_identity(), Some(first_storage));
        }

        let mut source = "x".repeat(LARGE_SOURCE_BYTES);
        let source_storage = source.as_ptr();
        let insertion_context = context_for(&source);
        let buffer = ProjectedTextBuffer::new(&mut source, LARGE_SOURCE_BYTES, insertion_context);
        assert_eq!(buffer.as_str().as_ptr(), source_storage);
        let cache = buffer.into_cache();
        assert_eq!(cache.storage_identity(), None);
    }

    #[test]
    fn projection_preserves_selection_direction_and_canonicalizes_crlf_midpoints() {
        let projection = SourceDisplayProjection::new("a\r\né");
        let display = egui::text::CCursorRange {
            primary: egui::text::CCursor::new(1),
            secondary: egui::text::CCursor::new(3),
            h_pos: None,
        };
        assert_eq!(
            projection.selection_to_source(display),
            Some(Selection::new(5, 1))
        );

        let caret = projection
            .selection_to_display(Selection::caret(2))
            .expect("a CRLF midpoint caret should snap deterministically");
        assert_eq!(caret.primary.index, egui::text::CharIndex(1));
        assert_eq!(caret.secondary.index, egui::text::CharIndex(1));

        let forward = projection
            .selection_to_display(Selection::new(1, 2))
            .expect("a partial CRLF selection should expand outward");
        assert_eq!(forward.secondary.index, egui::text::CharIndex(1));
        assert_eq!(forward.primary.index, egui::text::CharIndex(2));

        let reverse = projection
            .selection_to_display(Selection::new(5, 2))
            .expect("a reverse selection should retain its direction");
        assert_eq!(reverse.secondary.index, egui::text::CharIndex(3));
        assert_eq!(reverse.primary.index, egui::text::CharIndex(1));
    }

    #[test]
    fn projected_buffer_exposes_directional_selection_mapping() {
        let mut source = "a\r\né".to_owned();
        let context = context_for(&source);
        let buffer = ProjectedTextBuffer::new(&mut source, 16, context);
        assert_eq!(buffer.as_str(), "a\né");

        let display = egui::text::CCursorRange {
            primary: egui::text::CCursor::new(1),
            secondary: egui::text::CCursor::new(3),
            h_pos: None,
        };
        let source_selection = buffer
            .selection_to_source(display)
            .expect("display boundaries should map to source");
        assert_eq!(source_selection, Selection::new(5, 1));
        assert_eq!(
            buffer
                .selection_to_display(source_selection)
                .expect("source boundaries should map to display"),
            display
        );
    }

    #[test]
    fn projected_buffer_deletes_crlf_atomically_in_both_directions() {
        let initial = "a\r\nb";
        let context = context_for(initial);

        let mut backward = initial.to_owned();
        {
            let mut buffer = ProjectedTextBuffer::new(&mut backward, 16, context);
            let cursor = buffer.delete_previous_char(egui::text::CCursor::new(2));
            assert_eq!(cursor.index, egui::text::CharIndex(1));
            assert_eq!(buffer.as_str(), "ab");
        }
        assert_eq!(backward, "ab");

        let mut forward = initial.to_owned();
        {
            let mut buffer = ProjectedTextBuffer::new(&mut forward, 16, context);
            let cursor = buffer.delete_next_char(egui::text::CCursor::new(1));
            assert_eq!(cursor.index, egui::text::CharIndex(1));
            assert_eq!(buffer.as_str(), "ab");
        }
        assert_eq!(forward, "ab");
    }

    #[test]
    fn projected_buffer_uses_internal_and_external_mixed_ending_context() {
        let outer = "left\r\nEDIT\nright\r";
        let external_context = LineEndingProfile::detect(outer)
            .insertion_context(outer, 6..10)
            .expect("the block range should be valid");
        let mut block = "EDIT".to_owned();
        {
            let mut buffer = ProjectedTextBuffer::new(&mut block, 32, external_context);
            assert_eq!(
                buffer.insert_text("\nX\r\n", egui::text::CharIndex::ZERO),
                3
            );
        }
        assert_eq!(block, "\r\nX\r\nEDIT");

        let mut source = "a\r\nb\nc\r".to_owned();
        let context = context_for(&source);
        {
            let mut buffer = ProjectedTextBuffer::new(&mut source, 64, context);
            assert_eq!(buffer.insert_text("\n", egui::text::CharIndex(3)), 1);
        }
        assert_eq!(source, "a\r\nb\r\n\nc\r");
    }

    #[test]
    fn projected_buffer_enforces_source_bytes_and_never_splits_crlf_or_unicode() {
        let crlf_profile = LineEndingProfile::Uniform {
            ending: LineEnding::CrLf,
            count: 1,
        };
        let mut source = "a".to_owned();
        let context = crlf_profile
            .insertion_context(&source, 0..source.len())
            .expect("the full source should be valid");
        {
            let mut buffer = ProjectedTextBuffer::new(&mut source, 2, context);
            assert_eq!(buffer.insert_text("\n", egui::text::CharIndex(1)), 0);
            assert!(buffer.was_limited());
        }
        assert_eq!(source, "a");

        let mut source = "a".to_owned();
        let context = crlf_profile
            .insertion_context(&source, 0..source.len())
            .expect("the full source should be valid");
        {
            let mut buffer = ProjectedTextBuffer::new(&mut source, 5, context);
            assert_eq!(buffer.insert_text("\né", egui::text::CharIndex(1)), 2);
            assert_eq!(buffer.insert_text("字", egui::text::CharIndex(3)), 0);
            assert!(buffer.was_limited());
        }
        assert_eq!(source, "a\r\né");

        let mut split = "a\r\nb".to_owned();
        let context = context_for(&split);
        {
            let buffer = ProjectedTextBuffer::new(&mut split, 2, context);
            assert!(buffer.was_limited());
        }
        assert_eq!(split, "a");
    }

    #[test]
    fn rejected_projected_replacement_restores_the_deleted_source() {
        let mut source = "ax".to_owned();
        let context = context_for(&source);
        {
            let mut buffer = ProjectedTextBuffer::new(&mut source, 2, context);
            let selection = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(1),
            );
            let mut cursor = buffer.delete_selected(&selection);
            buffer.insert_text_at(&mut cursor, "é", usize::MAX);
            assert_eq!(buffer.as_str(), "ax");
            assert!(buffer.was_limited());
        }
        assert_eq!(source, "ax");
    }

    #[test]
    fn minimal_diff_replacement_preserves_unrelated_line_ending_bytes() {
        let mut source = "head\r\nold\nkeep\rtail".to_owned();
        let context = context_for(&source);
        {
            let mut buffer = ProjectedTextBuffer::new(&mut source, 64, context);
            buffer.replace_with("head\nnew\nline\nkeep\ntail");
            assert_eq!(buffer.as_str(), "head\nnew\nline\nkeep\ntail");
        }
        assert_eq!(source, "head\r\nnew\r\nline\nkeep\rtail");
    }

    #[test]
    fn projected_trait_replacement_canonicalizes_every_raw_newline_form() {
        for replacement in ["a\nb", "a\r\nb", "a\rb"] {
            let mut source = "old\r\nvalue".to_owned();
            let context = context_for(&source);
            {
                let mut buffer = ProjectedTextBuffer::new(&mut source, 64, context);
                buffer.replace_with(replacement);
                assert_eq!(buffer.as_str(), "a\nb", "{replacement:?}");
                assert!(!buffer.was_limited(), "{replacement:?}");
            }
            assert_eq!(source, "a\r\nb", "{replacement:?}");
        }

        let mut source = "a\r\nb".to_owned();
        let context = context_for(&source);
        {
            let mut buffer = ProjectedTextBuffer::new(&mut source, 64, context);
            buffer.replace_with("a\r\nb");
            assert_eq!(buffer.as_str(), "a\nb");
        }
        assert_eq!(source, "a\r\nb");
    }

    #[test]
    fn projected_trait_clear_take_and_bounded_rejection_preserve_contracts() {
        let mut source = "a\r\nb".to_owned();
        let context = context_for(&source);
        {
            let mut buffer = ProjectedTextBuffer::new(&mut source, 64, context);
            assert_eq!(buffer.take(), "a\nb");
            assert_eq!(buffer.as_str(), "");
            buffer.replace_with("x\r\ny");
            buffer.clear();
            assert_eq!(buffer.as_str(), "");
        }
        assert!(source.is_empty());

        let profile = LineEndingProfile::Uniform {
            ending: LineEnding::CrLf,
            count: 1,
        };
        let mut source = "ab".to_owned();
        let context = profile
            .insertion_context(&source, 0..source.len())
            .expect("the fixture should provide CRLF insertion policy");
        {
            let mut buffer = ProjectedTextBuffer::new(&mut source, 2, context);
            buffer.replace_with("a\r\nb");
            assert_eq!(buffer.as_str(), "ab");
            assert!(buffer.was_limited());
        }
        assert_eq!(source, "ab");
    }

    #[test]
    fn replacements_capture_mixed_ending_policy_before_removing_the_selection() {
        let initial = "gone\r\nkept\nfar\r";
        let expected = "X\r\nYkept\nfar\r";
        let context = context_for(initial);

        let mut event_replacement = initial.to_owned();
        {
            let mut buffer = ProjectedTextBuffer::new(&mut event_replacement, 64, context);
            let selection = egui::text::CCursorRange::two(
                egui::text::CCursor::new(0),
                egui::text::CCursor::new(5),
            );
            let mut cursor = buffer.delete_selected(&selection);
            buffer.insert_text_at(&mut cursor, "X\nY", usize::MAX);
        }
        assert_eq!(event_replacement, expected);

        let mut direct_replacement = initial.to_owned();
        {
            let mut buffer = ProjectedTextBuffer::new(&mut direct_replacement, 64, context);
            buffer.replace_with("X\nYkept\nfar\n");
        }
        assert_eq!(direct_replacement, expected);
    }

    #[test]
    fn projected_deletion_matrix_removes_only_the_exact_mapped_source_range() {
        for initial in [
            "",
            "a",
            "a\nb",
            "a\r\nb",
            "\r\n\r\n",
            "a\rb",
            "é\r\n字\rX\n",
        ] {
            let projection = SourceDisplayProjection::new(initial);
            let display_characters = projection.display().chars().count();
            for start in 0..=display_characters {
                for end in start..=display_characters {
                    let expected_start = projection
                        .display_char_to_source_byte(egui::text::CharIndex(start))
                        .expect("a display boundary should map to source");
                    let expected_end = projection
                        .display_char_to_source_byte(egui::text::CharIndex(end))
                        .expect("a display boundary should map to source");
                    assert_eq!(
                        projection.source_byte_to_display_char(expected_start),
                        Some(egui::text::CharIndex(start))
                    );
                    assert_eq!(
                        projection.source_byte_to_display_char(expected_end),
                        Some(egui::text::CharIndex(end))
                    );
                    let mut expected = initial.to_owned();
                    expected.replace_range(expected_start..expected_end, "");

                    let mut actual = initial.to_owned();
                    let context = context_for(&actual);
                    {
                        let mut buffer = ProjectedTextBuffer::new(&mut actual, 64, context);
                        buffer.delete_char_range(
                            egui::text::CharIndex(start)..egui::text::CharIndex(end),
                        );
                    }
                    assert_eq!(actual, expected, "range {start}..{end} in {initial:?}");
                }
            }
        }
    }

    #[test]
    fn projected_insert_text_at_counts_crlf_payload_as_one_display_character() {
        let profile = LineEndingProfile::Uniform {
            ending: LineEnding::CrLf,
            count: 1,
        };
        let mut source = String::new();
        let context = profile
            .insertion_context(&source, 0..0)
            .expect("the empty source should be valid");
        {
            let mut buffer = ProjectedTextBuffer::new(&mut source, 16, context);
            let mut cursor = egui::text::CCursor::new(0);
            buffer.insert_text_at(&mut cursor, "\r\nX", 1);
            assert_eq!(cursor.index, egui::text::CharIndex(1));
        }
        assert_eq!(source, "\r\n");
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
