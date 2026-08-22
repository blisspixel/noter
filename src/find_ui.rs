use eframe::egui;
use noter::core::edit::Selection;
use noter::core::revision::Revision;
use noter::core::search::{
    LiteralSearch, MAX_LITERAL_QUERY_BYTES, MAX_LITERAL_REPLACEMENT_BYTES, MatchCase, SearchError,
    SearchNavigation,
};

use crate::bounded_text_input::{
    BoundedTextBuffer, sanitize_bounded_text_events, truncate_to_utf8_byte_limit,
};

const FIND_QUERY_ID: &str = "noter-find-query";
const REPLACEMENT_ID: &str = "noter-find-replacement";
const EXPANDED_FIND_BAR_MIN_WIDTH: f32 = 860.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FindBarAction {
    Close,
    Next,
    Previous,
    Replace,
    ReplaceAll,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum ReplaceScope {
    Selection,
    #[default]
    Document,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum FindFeedback {
    Navigation(SearchNavigation),
    Replaced { count: usize, revision: Revision },
}

#[derive(Clone)]
struct SearchCache {
    revision: Revision,
    query: String,
    match_case: MatchCase,
    search: Result<LiteralSearch, SearchError>,
    match_count: usize,
}

#[derive(Default)]
pub struct FindBar {
    open: bool,
    replace_visible: bool,
    query: String,
    replacement: String,
    match_case: MatchCase,
    replace_scope: ReplaceScope,
    request_query_focus: bool,
    cache: Option<SearchCache>,
    feedback: Option<FindFeedback>,
    input_notice: Option<&'static str>,
    deferred_input_events: Vec<egui::Event>,
}

impl FindBar {
    pub(crate) fn open(&mut self, replace_visible: bool, source: &str, selection: Selection) {
        self.open = true;
        self.replace_visible = replace_visible;
        self.request_query_focus = true;
        if let Some(selected) = selected_query(source, selection) {
            self.query.clear();
            self.query.push_str(selected);
            self.invalidate_query();
            if replace_visible {
                self.replace_scope = ReplaceScope::Selection;
            }
        }
    }

    #[cfg(test)]
    pub(crate) const fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }

    pub(crate) fn owns_text_focus(&self, ctx: &egui::Context) -> bool {
        self.open
            && ctx.memory(|memory| {
                memory.has_focus(egui::Id::new(FIND_QUERY_ID))
                    || memory.has_focus(egui::Id::new(REPLACEMENT_ID))
            })
    }

    pub(crate) const fn has_query(&self) -> bool {
        !self.query.is_empty()
    }

    pub(crate) fn show(
        &mut self,
        ui: &mut egui::Ui,
        revision: Revision,
        source: &str,
        selection: Selection,
    ) -> Option<FindBarAction> {
        if !self.open {
            return None;
        }
        self.restore_deferred_input(ui);

        let query_has_focus = ui.memory(|memory| memory.has_focus(egui::Id::new(FIND_QUERY_ID)));
        let replacement_has_focus =
            ui.memory(|memory| memory.has_focus(egui::Id::new(REPLACEMENT_ID)));
        let previous_focused_field = ui.memory(|memory| {
            if memory.had_focus_last_frame(egui::Id::new(REPLACEMENT_ID)) {
                Some(egui::Id::new(REPLACEMENT_ID))
            } else if memory.had_focus_last_frame(egui::Id::new(FIND_QUERY_ID)) {
                Some(egui::Id::new(FIND_QUERY_ID))
            } else {
                None
            }
        });
        let replace_on_enter = replacement_has_focus
            && self
                .prepared_search(revision, source)
                .ok()
                .is_some_and(|search| search.matches_range(source, selection.ordered_range()));
        let mut action = self.take_key_action(
            ui,
            query_has_focus || replacement_has_focus,
            replace_on_enter,
            previous_focused_field,
        );
        egui::Panel::top("find_replace_bar").show(ui, |ui| {
            self.show_find_row(ui, revision, source, selection, &mut action);
            if self.replace_visible {
                self.show_replace_row(ui, revision, source, selection, &mut action);
            }
        });
        if action == Some(FindBarAction::Close) {
            self.open = false;
            self.deferred_input_events.clear();
        }

        action
    }

    pub(crate) fn restore_deferred_input(&mut self, ui: &egui::Ui) {
        if self.deferred_input_events.is_empty() {
            return;
        }
        ui.input_mut(|input| {
            self.deferred_input_events.append(&mut input.events);
            std::mem::swap(&mut self.deferred_input_events, &mut input.events);
        });
    }

    pub(crate) fn discard_deferred_input(&mut self) {
        self.deferred_input_events.clear();
    }

    fn take_key_action(
        &mut self,
        ui: &egui::Ui,
        accept_enter: bool,
        replace_on_enter: bool,
        previous_focused_field: Option<egui::Id>,
    ) -> Option<FindBarAction> {
        let mut deferred = Vec::new();
        let action = ui.input_mut(|input| {
            let (position, action) =
                input
                    .events
                    .iter()
                    .enumerate()
                    .find_map(|(position, event)| {
                        find_key_action(event, accept_enter, replace_on_enter)
                            .map(|action| (position, action))
                    })?;
            let prefix = &input.events[..position];
            let must_defer = if action == FindBarAction::Close {
                prefix
                    .iter()
                    .any(crate::keyboard_nav::editor_event_orders_input)
            } else {
                prefix
                    .iter()
                    .any(crate::keyboard_nav::editor_event_may_change_focus)
            };
            if must_defer {
                deferred = input.events.split_off(position);
                return None;
            }
            input.events.remove(position);
            let suffix = input.events.split_off(position);
            if action != FindBarAction::Close {
                deferred = suffix;
            }
            Some(action)
        });
        if !deferred.is_empty() {
            self.deferred_input_events = deferred;
            ui.ctx().request_repaint();
        }
        if action.is_none()
            && previous_focused_field.is_some()
            && self.deferred_input_events.iter().any(|event| {
                find_key_action(event, false, replace_on_enter) == Some(FindBarAction::Close)
            })
            && let Some(field) = previous_focused_field
        {
            ui.memory_mut(|memory| memory.request_focus(field));
        }
        action
    }

    fn show_find_row(
        &mut self,
        ui: &mut egui::Ui,
        revision: Revision,
        source: &str,
        selection: Selection,
        action: &mut Option<FindBarAction>,
    ) {
        if ui.available_width() >= EXPANDED_FIND_BAR_MIN_WIDTH {
            ui.horizontal(|ui| {
                self.show_query_field(ui);
                self.show_find_controls(ui, revision, source, selection, action);
                if ui.button("Close").clicked() {
                    self.open = false;
                    *action = Some(FindBarAction::Close);
                }
            });
            return;
        }
        ui.horizontal(|ui| {
            self.show_query_field(ui);
            if ui.button("Close").clicked() {
                self.open = false;
                *action = Some(FindBarAction::Close);
            }
        });
        ui.horizontal(|ui| {
            self.show_find_controls(ui, revision, source, selection, action);
        });
    }

    fn show_query_field(&mut self, ui: &mut egui::Ui) {
        ui.label("Find");
        let id = egui::Id::new(FIND_QUERY_ID);
        let query_was_clamped =
            truncate_to_utf8_byte_limit(&mut self.query, MAX_LITERAL_QUERY_BYTES);
        let accepts_input = self.request_query_focus || ui.memory(|memory| memory.has_focus(id));
        let event_was_clamped = accepts_input
            && sanitize_bounded_text_events(ui, id, &self.query, MAX_LITERAL_QUERY_BYTES);
        let (query_response, buffer_was_clamped) = {
            let mut buffer = BoundedTextBuffer::new(&mut self.query, MAX_LITERAL_QUERY_BYTES);
            let response = ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .id(id)
                    .desired_width(240.0)
                    .char_limit(MAX_LITERAL_QUERY_BYTES)
                    .hint_text("Literal text"),
            );
            (response, buffer.was_limited())
        };
        let input_was_clamped = event_was_clamped || buffer_was_clamped;
        if self.request_query_focus {
            query_response.request_focus();
            self.request_query_focus = false;
        }
        let result_was_clamped =
            truncate_to_utf8_byte_limit(&mut self.query, MAX_LITERAL_QUERY_BYTES);
        if query_response.changed() || query_was_clamped {
            self.invalidate_query();
        }
        if query_was_clamped || input_was_clamped || result_was_clamped {
            self.input_notice = Some("Find text is limited to 16 KiB");
        } else if query_response.changed() {
            self.input_notice = None;
        }
    }

    fn show_find_controls(
        &mut self,
        ui: &mut egui::Ui,
        revision: Revision,
        source: &str,
        selection: Selection,
        action: &mut Option<FindBarAction>,
    ) {
        if ui.button("Previous").clicked() {
            *action = Some(FindBarAction::Previous);
        }
        if ui.button("Next").clicked() {
            *action = Some(FindBarAction::Next);
        }
        let mut case_sensitive = self.match_case == MatchCase::Sensitive;
        if ui.checkbox(&mut case_sensitive, "Match case").changed() {
            self.match_case = if case_sensitive {
                MatchCase::Sensitive
            } else {
                MatchCase::Insensitive
            };
            self.invalidate_query();
        }
        ui.label(self.status_text(revision, source, selection));
    }

    fn show_replace_row(
        &mut self,
        ui: &mut egui::Ui,
        revision: Revision,
        source: &str,
        selection: Selection,
        action: &mut Option<FindBarAction>,
    ) {
        if ui.available_width() >= EXPANDED_FIND_BAR_MIN_WIDTH {
            ui.horizontal(|ui| {
                self.show_replacement_field(ui);
                self.show_replace_controls(ui, revision, source, selection, action);
            });
            return;
        }
        ui.horizontal(|ui| self.show_replacement_field(ui));
        ui.horizontal(|ui| {
            self.show_replace_controls(ui, revision, source, selection, action);
        });
    }

    fn show_replacement_field(&mut self, ui: &mut egui::Ui) {
        ui.label("Replace");
        let id = egui::Id::new(REPLACEMENT_ID);
        let replacement_was_clamped =
            truncate_to_utf8_byte_limit(&mut self.replacement, MAX_LITERAL_REPLACEMENT_BYTES);
        let event_was_clamped = ui.memory(|memory| memory.has_focus(id))
            && sanitize_bounded_text_events(
                ui,
                id,
                &self.replacement,
                MAX_LITERAL_REPLACEMENT_BYTES,
            );
        let (replacement_response, buffer_was_clamped) = {
            let mut buffer =
                BoundedTextBuffer::new(&mut self.replacement, MAX_LITERAL_REPLACEMENT_BYTES);
            let response = ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .id(id)
                    .desired_width(240.0)
                    .char_limit(MAX_LITERAL_REPLACEMENT_BYTES)
                    .hint_text("Literal replacement"),
            );
            (response, buffer.was_limited())
        };
        let input_was_clamped = event_was_clamped || buffer_was_clamped;
        let result_was_clamped =
            truncate_to_utf8_byte_limit(&mut self.replacement, MAX_LITERAL_REPLACEMENT_BYTES);
        if replacement_was_clamped || input_was_clamped || result_was_clamped {
            self.input_notice = Some("Replacement text is limited to 16 KiB");
        } else if replacement_response.changed() {
            self.input_notice = None;
        }
    }

    fn show_replace_controls(
        &mut self,
        ui: &mut egui::Ui,
        revision: Revision,
        source: &str,
        selection: Selection,
        action: &mut Option<FindBarAction>,
    ) {
        ui.label("Scope");
        ui.selectable_value(
            &mut self.replace_scope,
            ReplaceScope::Selection,
            "Selection",
        );
        ui.selectable_value(&mut self.replace_scope, ReplaceScope::Document, "Document");

        let search = self.prepared_search(revision, source).ok();
        let selected_match = search
            .as_ref()
            .is_some_and(|search| search.matches_range(source, selection.ordered_range()));
        if ui
            .add_enabled(selected_match, egui::Button::new("Replace"))
            .on_disabled_hover_text("Select a complete match before replacing it")
            .clicked()
        {
            *action = Some(FindBarAction::Replace);
        }
        let scope_is_available = self.replace_scope == ReplaceScope::Document
            || selection.anchor() != selection.active();
        let has_matches = self
            .cached_match_count(revision, source)
            .is_some_and(|count| count > 0);
        if ui
            .add_enabled(
                scope_is_available && has_matches,
                egui::Button::new("Replace All"),
            )
            .on_disabled_hover_text("Choose a non-empty selection or a document with matches")
            .clicked()
        {
            *action = Some(FindBarAction::ReplaceAll);
        }
    }

    pub(crate) fn prepared_search(
        &mut self,
        revision: Revision,
        source: &str,
    ) -> Result<LiteralSearch, SearchError> {
        self.ensure_cache(revision, source).search.clone()
    }

    pub(crate) fn replacement(&self) -> &str {
        &self.replacement
    }

    #[cfg(test)]
    pub(crate) fn set_replacement_for_test(&mut self, replacement: String) {
        self.replacement = replacement;
    }

    pub(crate) const fn replace_scope(&self) -> ReplaceScope {
        self.replace_scope
    }

    pub(crate) const fn record_navigation(&mut self, navigation: SearchNavigation) {
        self.feedback = Some(FindFeedback::Navigation(navigation));
    }

    pub(crate) fn record_replacements(&mut self, replacement_count: usize, revision: Revision) {
        self.feedback = Some(FindFeedback::Replaced {
            count: replacement_count,
            revision,
        });
        self.cache = None;
    }

    fn status_text(&mut self, revision: Revision, source: &str, selection: Selection) -> String {
        if let Some(notice) = self.input_notice {
            return notice.to_owned();
        }
        if self.query.is_empty() {
            return "Enter text to find".to_owned();
        }
        let (search, match_count) = {
            let cache = self.ensure_cache(revision, source);
            (cache.search.clone(), cache.match_count)
        };
        let search = match search {
            Ok(search) => search,
            Err(error) => return error.to_string(),
        };
        if match_count == 0 {
            return "No matches".to_owned();
        }
        if let Some(FindFeedback::Navigation(navigation)) = self.feedback
            && navigation.match_count() == match_count
            && search.matches_range(source, selection.ordered_range())
            && navigation.range() == selection.ordered_range()
        {
            let wrap = if navigation.wrapped() {
                ", wrapped"
            } else {
                ""
            };
            return format!(
                "{} of {}{wrap}",
                navigation.ordinal(),
                navigation.match_count()
            );
        }
        if let Some(FindFeedback::Replaced {
            count,
            revision: at,
        }) = self.feedback
            && at == revision
        {
            return format!("Replaced {count}; {match_count} remain");
        }
        format!("{match_count} matches")
    }

    fn cached_match_count(&mut self, revision: Revision, source: &str) -> Option<usize> {
        let cache = self.ensure_cache(revision, source);
        cache.search.as_ref().ok().map(|_| cache.match_count)
    }

    fn ensure_cache(&mut self, revision: Revision, source: &str) -> &SearchCache {
        let cache_is_current = self.cache.as_ref().is_some_and(|cache| {
            cache.revision == revision
                && cache.query == self.query
                && cache.match_case == self.match_case
        });
        if !cache_is_current {
            if matches!(self.feedback, Some(FindFeedback::Navigation(_))) {
                self.feedback = None;
            }
            self.cache = None;
        }
        let query = self.query.clone();
        let match_case = self.match_case;
        self.cache.get_or_insert_with(|| {
            let search = LiteralSearch::new(&query, match_case);
            let match_count = search
                .as_ref()
                .map_or(0, |search| search.match_count(source));
            SearchCache {
                revision,
                query,
                match_case,
                search,
                match_count,
            }
        })
    }

    fn invalidate_query(&mut self) {
        self.cache = None;
        self.feedback = None;
    }
}

fn find_key_action(
    event: &egui::Event,
    accept_enter: bool,
    replace_on_enter: bool,
) -> Option<FindBarAction> {
    let egui::Event::Key {
        key,
        pressed: true,
        modifiers,
        ..
    } = event
    else {
        return None;
    };
    if *key == egui::Key::Escape && modifiers.matches_logically(egui::Modifiers::NONE) {
        Some(FindBarAction::Close)
    } else if !accept_enter || *key != egui::Key::Enter {
        None
    } else if modifiers.matches_logically(egui::Modifiers::SHIFT) {
        Some(FindBarAction::Previous)
    } else if modifiers.matches_logically(egui::Modifiers::NONE) {
        Some(if replace_on_enter {
            FindBarAction::Replace
        } else {
            FindBarAction::Next
        })
    } else {
        None
    }
}

fn selected_query(source: &str, selection: Selection) -> Option<&str> {
    let range = selection.ordered_range();
    let selected = source.get(range.start()..range.end())?;
    (!selected.is_empty()
        && selected.len() <= MAX_LITERAL_QUERY_BYTES
        && !selected.contains('\r')
        && !selected.contains('\n'))
    .then_some(selected)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enter_key() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    fn focused_bar(replace_visible: bool) -> (egui::Context, FindBar) {
        let source = "one two one";
        let selection = Selection::new(0, 3);
        let context = egui::Context::default();
        let mut bar = FindBar::default();
        bar.open(replace_visible, source, selection);
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(bar.show(ui, Revision::INITIAL, source, selection), None);
        });
        if replace_visible {
            context.memory_mut(|memory| {
                memory.request_focus(egui::Id::new(REPLACEMENT_ID));
            });
            let _ = context.run_ui(egui::RawInput::default(), |ui| {
                assert_eq!(bar.show(ui, Revision::INITIAL, source, selection), None);
            });
        }
        (context, bar)
    }

    #[test]
    fn opening_prefills_a_bounded_single_line_selection_and_uses_safe_scope() {
        let mut bar = FindBar::default();
        bar.open(false, "one two", Selection::new(4, 7));

        assert!(bar.is_open());
        assert_eq!(bar.query, "two");
        assert_eq!(bar.replace_scope(), ReplaceScope::Document);

        bar.open(true, "one two", Selection::new(4, 7));
        assert!(bar.replace_visible);
        assert_eq!(bar.query, "two");
        assert_eq!(bar.replace_scope(), ReplaceScope::Selection);
    }

    #[test]
    fn cache_tracks_revision_query_and_case_without_retaining_match_ranges() {
        let mut bar = FindBar {
            query: "rust".to_owned(),
            ..FindBar::default()
        };
        let first_revision = Revision::INITIAL;

        assert_eq!(bar.cached_match_count(first_revision, "Rust rust"), Some(2));
        assert_eq!(
            bar.cache.as_ref().map(|cache| cache.revision),
            Some(first_revision)
        );
        bar.match_case = MatchCase::Sensitive;
        assert_eq!(bar.cached_match_count(first_revision, "Rust rust"), Some(1));

        let next_revision = first_revision
            .checked_next()
            .expect("fixture revision should advance");
        assert_eq!(
            bar.cached_match_count(next_revision, "rust rust rust"),
            Some(3)
        );
        assert_eq!(
            bar.cache.as_ref().map(|cache| cache.revision),
            Some(next_revision)
        );
    }

    #[test]
    fn replacement_feedback_survives_the_revision_refresh_it_caused() {
        let mut bar = FindBar {
            query: "x".to_owned(),
            ..FindBar::default()
        };
        bar.record_replacements(2, Revision::INITIAL);
        let status = bar.status_text(Revision::INITIAL, "x", Selection::caret(0));

        assert_eq!(status, "Replaced 2; 1 remain");
    }

    #[test]
    fn selected_query_rejects_empty_multiline_split_and_oversized_ranges() {
        assert_eq!(selected_query("text", Selection::caret(0)), None);
        assert_eq!(selected_query("a\nb", Selection::new(0, 3)), None);
        assert_eq!(selected_query("é", Selection::new(1, 2)), None);
        let oversized = "x".repeat(MAX_LITERAL_QUERY_BYTES + 1);
        assert_eq!(
            selected_query(&oversized, Selection::new(0, oversized.len())),
            None
        );
    }

    #[test]
    fn replace_scope_defaults_to_document_until_an_explicit_selection_is_prefilled() {
        let mut bar = FindBar::default();
        assert_eq!(bar.replace_scope(), ReplaceScope::Document);
        bar.open(true, "text", Selection::caret(0));
        assert_eq!(bar.replace_scope(), ReplaceScope::Document);
    }

    #[test]
    fn focused_find_field_handles_navigation_and_escape_before_the_editor() {
        let source = "one two one";
        let selection = Selection::new(0, 3);
        let mut bar = FindBar::default();
        bar.open(false, source, selection);
        let context = egui::Context::default();
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            let action = bar.show(ui, Revision::INITIAL, source, selection);
            assert_eq!(action, None);
        });
        assert!(context.memory(|memory| memory.has_focus(egui::Id::new(FIND_QUERY_ID))));

        let mut next = egui::RawInput::default();
        next.events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = context.run_ui(next, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Next)
            );
        });

        let mut close = egui::RawInput::default();
        close.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = context.run_ui(close, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Close)
            );
        });
        assert!(!bar.is_open());
    }

    #[test]
    fn find_and_replace_enter_actions_preserve_input_event_order() {
        let source = "one two one";
        let selection = Selection::new(0, 3);

        let (context, mut bar) = focused_bar(false);
        let prefix = egui::RawInput {
            events: vec![egui::Event::Text("x".to_owned()), enter_key()],
            ..Default::default()
        };
        let _ = context.run_ui(prefix, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Next)
            );
        });
        assert_eq!(bar.query, "onex");

        let (context, mut bar) = focused_bar(false);
        let suffix = egui::RawInput {
            events: vec![enter_key(), egui::Event::Text("x".to_owned())],
            ..Default::default()
        };
        let _ = context.run_ui(suffix, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Next)
            );
        });
        assert_eq!(bar.query, "one");
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(bar.show(ui, Revision::INITIAL, source, selection), None);
        });
        assert_eq!(bar.query, "onex");

        let (context, mut bar) = focused_bar(false);
        let repeated = egui::RawInput {
            events: vec![enter_key(), enter_key()],
            ..Default::default()
        };
        let _ = context.run_ui(repeated, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Next)
            );
        });
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Next)
            );
        });
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(bar.show(ui, Revision::INITIAL, source, selection), None);
        });
    }

    #[test]
    fn replacement_enter_actions_preserve_input_event_order() {
        let source = "one two one";
        let selection = Selection::new(0, 3);

        let (context, mut bar) = focused_bar(true);
        let prefix = egui::RawInput {
            events: vec![egui::Event::Text("x".to_owned()), enter_key()],
            ..Default::default()
        };
        let _ = context.run_ui(prefix, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Replace)
            );
        });
        assert_eq!(bar.replacement, "x");

        let (context, mut bar) = focused_bar(true);
        let suffix = egui::RawInput {
            events: vec![enter_key(), egui::Event::Text("x".to_owned())],
            ..Default::default()
        };
        let _ = context.run_ui(suffix, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Replace)
            );
        });
        assert!(bar.replacement.is_empty());
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(bar.show(ui, Revision::INITIAL, source, selection), None);
        });
        assert_eq!(bar.replacement, "x");

        let (context, mut bar) = focused_bar(true);
        let repeated = egui::RawInput {
            events: vec![enter_key(), enter_key()],
            ..Default::default()
        };
        let _ = context.run_ui(repeated, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Replace)
            );
        });
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Replace)
            );
        });
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(bar.show(ui, Revision::INITIAL, source, selection), None);
        });
    }

    #[test]
    fn find_enter_waits_for_an_earlier_focus_change() {
        let source = "one two one";
        let selection = Selection::new(0, 3);

        let context = egui::Context::default();
        context.enable_accesskit();
        let mut bar = FindBar::default();
        bar.open(true, source, selection);
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(bar.show(ui, Revision::INITIAL, source, selection), None);
        });
        let focus_replacement =
            egui::Event::AccessKitActionRequest(egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Focus,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: egui::Id::new(REPLACEMENT_ID).accesskit_id(),
                data: None,
            });
        let focus_then_enter = egui::RawInput {
            events: vec![focus_replacement, enter_key()],
            ..Default::default()
        };
        let _ = context.run_ui(focus_then_enter, |ui| {
            assert_eq!(bar.show(ui, Revision::INITIAL, source, selection), None);
        });
        assert!(context.memory(|memory| memory.has_focus(egui::Id::new(REPLACEMENT_ID))));
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Replace)
            );
        });
    }

    #[test]
    fn find_escape_keeps_field_text_on_its_correct_side_of_close() {
        let source = "one two one";
        let selection = Selection::new(0, 3);
        let escape = || egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };

        let (context, mut bar) = focused_bar(false);
        let text_then_escape = egui::RawInput {
            events: vec![egui::Event::Text("x".to_owned()), escape()],
            ..Default::default()
        };
        let _ = context.run_ui(text_then_escape, |ui| {
            assert_eq!(bar.show(ui, Revision::INITIAL, source, selection), None);
        });
        assert_eq!(bar.query, "onex");
        assert!(bar.is_open());
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Close)
            );
        });
        assert!(!bar.is_open());

        let (context, mut bar) = focused_bar(false);
        let escape_then_text = egui::RawInput {
            events: vec![escape(), egui::Event::Text("x".to_owned())],
            ..Default::default()
        };
        let _ = context.run_ui(escape_then_text, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selection),
                Some(FindBarAction::Close)
            );
        });
        assert_eq!(bar.query, "one");
        assert!(!bar.is_open());
    }

    #[test]
    fn focused_replace_field_enter_replaces_a_selected_match_otherwise_finds_next() {
        let source = "one two one";
        let selected = Selection::new(0, 3);
        let mut bar = FindBar::default();
        bar.open(true, source, selected);
        let context = egui::Context::default();
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(bar.show(ui, Revision::INITIAL, source, selected), None);
        });
        context.memory_mut(|memory| memory.request_focus(egui::Id::new(REPLACEMENT_ID)));
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(bar.show(ui, Revision::INITIAL, source, selected), None);
        });

        let mut enter = egui::RawInput::default();
        enter.events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = context.run_ui(enter, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, selected),
                Some(FindBarAction::Replace)
            );
        });

        context.memory_mut(|memory| memory.request_focus(egui::Id::new(REPLACEMENT_ID)));
        let mut miss = egui::RawInput::default();
        miss.events.push(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = context.run_ui(miss, |ui| {
            assert_eq!(
                bar.show(ui, Revision::INITIAL, source, Selection::caret(4)),
                Some(FindBarAction::Next)
            );
        });
    }

    #[test]
    fn find_and_replacement_inputs_truncate_only_at_utf8_boundaries() {
        let mut exact_query = "x".repeat(MAX_LITERAL_QUERY_BYTES);
        assert!(!truncate_to_utf8_byte_limit(
            &mut exact_query,
            MAX_LITERAL_QUERY_BYTES
        ));

        let mut oversized_query = format!("{}é", "x".repeat(MAX_LITERAL_QUERY_BYTES - 1));
        assert!(truncate_to_utf8_byte_limit(
            &mut oversized_query,
            MAX_LITERAL_QUERY_BYTES
        ));
        assert_eq!(oversized_query.len(), MAX_LITERAL_QUERY_BYTES - 1);
        assert!(oversized_query.is_char_boundary(oversized_query.len()));

        let mut oversized_replacement = "y".repeat(MAX_LITERAL_REPLACEMENT_BYTES + 1);
        assert!(truncate_to_utf8_byte_limit(
            &mut oversized_replacement,
            MAX_LITERAL_REPLACEMENT_BYTES
        ));
        assert_eq!(oversized_replacement.len(), MAX_LITERAL_REPLACEMENT_BYTES);
    }

    #[test]
    fn oversized_paste_is_sanitized_before_text_edit_records_undo_state() {
        let context = egui::Context::default();
        let id = egui::Id::new(FIND_QUERY_ID);
        context.memory_mut(|memory| memory.request_focus(id));
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Paste(format!(
            "{}é",
            "x".repeat(MAX_LITERAL_QUERY_BYTES - 1)
        )));
        let mut value = String::new();

        let _ = context.run_ui(input, |ui| {
            assert!(sanitize_bounded_text_events(
                ui,
                id,
                &value,
                MAX_LITERAL_QUERY_BYTES
            ));
            let event_bytes = ui.input(|input| {
                input
                    .events
                    .iter()
                    .find_map(|event| match event {
                        egui::Event::Paste(text) => Some(text.len()),
                        _ => None,
                    })
                    .expect("the sanitized paste should remain available to the widget")
            });
            assert_eq!(event_bytes, MAX_LITERAL_QUERY_BYTES - 1);
            let mut buffer = BoundedTextBuffer::new(&mut value, MAX_LITERAL_QUERY_BYTES);
            ui.add(
                egui::TextEdit::singleline(&mut buffer)
                    .id(id)
                    .char_limit(MAX_LITERAL_QUERY_BYTES),
            );
        });

        assert_eq!(value.len(), MAX_LITERAL_QUERY_BYTES - 1);
    }
}
