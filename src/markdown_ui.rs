use std::ops::Range;

use eframe::egui;
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use noter::core::line_endings::logical_lines;
use pulldown_cmark::{Event, Options, Parser};

const ACTIVE_EDITOR_ID: &str = "noter-markdown-active-block";
const EXPANDED_FORMAT_MIN_WIDTH: f32 = 540.0;

fn expanded_toolbar_fits(available_width: f32) -> bool {
    available_width >= EXPANDED_FORMAT_MIN_WIDTH
}

fn markdown_parser_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_DEFINITION_LIST
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarkdownCommand {
    Heading1,
    Heading2,
    Bold,
    Italic,
    Link,
    InlineCode,
    BulletedList,
    Quote,
}

impl MarkdownCommand {
    const ALL: [Self; 8] = [
        Self::Heading1,
        Self::Heading2,
        Self::Bold,
        Self::Italic,
        Self::Link,
        Self::InlineCode,
        Self::BulletedList,
        Self::Quote,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Heading1 => "H1",
            Self::Heading2 => "H2",
            Self::Bold => "Bold",
            Self::Italic => "Italic",
            Self::Link => "Link",
            Self::InlineCode => "Code",
            Self::BulletedList => "List",
            Self::Quote => "Quote",
        }
    }

    const fn description(self) -> &'static str {
        match self {
            Self::Heading1 => "Format the active block as a level-one heading",
            Self::Heading2 => "Format the active block as a level-two heading",
            Self::Bold => "Wrap the selection in strong emphasis",
            Self::Italic => "Wrap the selection in emphasis",
            Self::Link => "Insert a standard Markdown link",
            Self::InlineCode => "Wrap the selection as inline code",
            Self::BulletedList => "Toggle bullet markers on the active lines",
            Self::Quote => "Toggle quote markers on the active lines",
        }
    }

    const fn menu_label(self) -> &'static str {
        match self {
            Self::Heading1 => "Heading 1",
            Self::Heading2 => "Heading 2",
            Self::Bold => "Bold",
            Self::Italic => "Italic",
            Self::Link => "Link",
            Self::InlineCode => "Inline code",
            Self::BulletedList => "Bulleted list",
            Self::Quote => "Quote",
        }
    }

    const fn button_width(self) -> f32 {
        match self {
            Self::Heading1 | Self::Heading2 => 40.0,
            Self::Bold | Self::Link | Self::InlineCode | Self::Quote => 54.0,
            Self::Italic | Self::BulletedList => 50.0,
        }
    }
}

#[derive(Debug)]
struct ActiveBlock {
    source_range: Range<usize>,
    draft: String,
    selection: Range<usize>,
    editor_serial: u64,
    dirty: bool,
    request_focus: bool,
}

impl ActiveBlock {
    fn new(source_range: Range<usize>, draft: String, editor_serial: u64) -> Self {
        let end = draft.chars().count();
        Self {
            source_range,
            draft,
            selection: end..end,
            editor_serial,
            dirty: false,
            request_focus: true,
        }
    }

    fn editor_id(&self) -> egui::Id {
        egui::Id::new((ACTIVE_EDITOR_ID, self.editor_serial))
    }

    fn apply(&mut self, command: MarkdownCommand) {
        let result = apply_markdown_command(&self.draft, self.selection.clone(), command);
        self.draft = result.text;
        self.selection = result.selection;
        self.dirty = true;
        self.request_focus = true;
    }
}

#[derive(Default)]
pub struct MarkdownEditor {
    cache: CommonMarkCache,
    active: Option<ActiveBlock>,
    next_editor_serial: u64,
}

impl MarkdownEditor {
    pub fn reset(&mut self) {
        self.active = None;
        self.cache = CommonMarkCache::default();
        self.next_editor_serial = self.next_editor_serial.wrapping_add(1);
    }

    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        if !expanded_toolbar_fits(ui.available_width()) {
            self.compact_toolbar(ui);
            return;
        }

        ui.label(
            egui::RichText::new("Format")
                .text_style(egui::TextStyle::Button)
                .weak(),
        );
        let enabled = self.active.is_some();
        for command in MarkdownCommand::ALL {
            let response = ui
                .add_enabled(
                    enabled,
                    egui::Button::new(command.label())
                        .min_size(egui::vec2(command.button_width(), 28.0)),
                )
                .on_hover_text(if enabled {
                    command.description()
                } else {
                    "Click a formatted block before applying a format"
                });
            if response.clicked() {
                self.apply_command(command);
            }
        }
        if enabled {
            ui.separator();
            if ui
                .add(egui::Button::new("Done").min_size(egui::vec2(54.0, 28.0)))
                .on_hover_text("Return to the formatted block")
                .clicked()
            {
                self.active = None;
            }
        }
    }

    fn compact_toolbar(&mut self, ui: &mut egui::Ui) {
        let enabled = self.active.is_some();
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                ui.menu_button("Format", |ui| {
                    for command in MarkdownCommand::ALL {
                        if ui
                            .button(command.menu_label())
                            .on_hover_text(command.description())
                            .clicked()
                        {
                            self.apply_command(command);
                            ui.close();
                        }
                    }
                    ui.separator();
                    if ui.button("Done editing").clicked() {
                        self.active = None;
                        ui.close();
                    }
                });
            })
            .response;
        if !enabled {
            response.on_disabled_hover_text("Click a formatted block before applying a format");
        }
    }

    fn apply_command(&mut self, command: MarkdownCommand) {
        if let Some(active) = self.active.as_mut() {
            active.apply(command);
        }
    }

    fn activate(&mut self, source_range: Range<usize>, draft: String) {
        self.next_editor_serial = self.next_editor_serial.wrapping_add(1);
        self.active = Some(ActiveBlock::new(
            source_range,
            draft,
            self.next_editor_serial,
        ));
    }

    #[cfg(feature = "screenshot-qa")]
    /// Opens the first source block so release screenshots show direct editing.
    pub(crate) fn activate_first_block(&mut self, source: &str) {
        let Some(range) = markdown_block_ranges(source).into_iter().next() else {
            return;
        };
        let Some(block) = source.get(range.clone()) else {
            return;
        };
        self.activate(range, block.to_owned());
    }

    pub const fn is_editing(&self) -> bool {
        self.active.is_some()
    }

    pub fn show(&mut self, ui: &mut egui::Ui, source: &mut String) -> bool {
        let mut changed = self.sync_pending_command(source);
        let ranges = markdown_block_ranges(source);

        if ranges.is_empty() {
            if self.active.is_none() {
                self.activate(0..0, String::new());
            }
            if self.show_active_editor(ui) {
                changed |= self.sync_pending_command(source);
            }
            return changed;
        }

        let active_range = self
            .active
            .as_ref()
            .map(|active| active.source_range.clone());
        let mut active_shown = false;

        for range in ranges {
            let overlaps_active = active_range
                .as_ref()
                .is_some_and(|active| ranges_overlap(active, &range));
            if overlaps_active {
                if !active_shown {
                    changed |= self.show_active_editor(ui);
                    active_shown = true;
                }
                continue;
            }
            self.show_rendered_block(ui, source, range);
        }

        if self.active.is_some() && !active_shown {
            self.active = None;
        }
        if self.active.as_ref().is_some_and(|active| active.dirty) {
            changed |= self.sync_pending_command(source);
        }
        changed
    }

    fn sync_pending_command(&mut self, source: &mut String) -> bool {
        let Some(active) = self.active.as_mut().filter(|active| active.dirty) else {
            return false;
        };
        if active.source_range.end > source.len()
            || !source.is_char_boundary(active.source_range.start)
            || !source.is_char_boundary(active.source_range.end)
        {
            self.active = None;
            return false;
        }
        source.replace_range(active.source_range.clone(), &active.draft);
        active.source_range.end = active.source_range.start + active.draft.len();
        active.dirty = false;
        true
    }

    fn show_rendered_block(&mut self, ui: &mut egui::Ui, source: &str, range: Range<usize>) {
        let block = &source[range.clone()];
        let frame = egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(8, 2))
            .corner_radius(4);
        let response = frame
            .show(ui, |ui| {
                if is_reference_definition(block) {
                    ui.label(egui::RichText::new(block).monospace());
                } else {
                    CommonMarkViewer::new()
                        .explicit_image_uri_scheme(true)
                        .show(ui, &mut self.cache, block);
                }
            })
            .response
            .interact(egui::Sense::click());

        if response.hovered() {
            ui.painter().rect_stroke(
                response.rect,
                4,
                egui::Stroke::new(1.0_f32, ui.visuals().widgets.hovered.bg_stroke.color),
                egui::StrokeKind::Inside,
            );
            response
                .clone()
                .on_hover_text("Click to edit this Markdown block");
        }
        if response.clicked() {
            self.activate(range, block.to_owned());
        }
        ui.add_space(2.0);
    }

    fn show_active_editor(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        let rows = logical_lines(&active.draft).count().max(1) + 1;
        let editor_id = active.editor_id();
        let editor = egui::TextEdit::multiline(&mut active.draft)
            .id(editor_id)
            .font(egui::TextStyle::Body)
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .margin(egui::Margin::same(10));
        let mut output = editor.show(ui);

        if active.request_focus {
            output.response.request_focus();
            let start = egui::text::CCursor::new(active.selection.start);
            let end = egui::text::CCursor::new(active.selection.end);
            output
                .state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(start, end)));
            output.state.store(ui.ctx(), output.response.id);
            active.request_focus = false;
        }
        if let Some(cursor_range) = output.cursor_range {
            let range = cursor_range.as_sorted_char_range();
            active.selection = range.start.0..range.end.0;
        }

        let changed = output.response.changed();
        if changed {
            active.dirty = true;
        }
        changed
    }
}

const fn ranges_overlap(left: &Range<usize>, right: &Range<usize>) -> bool {
    left.start < right.end && right.start < left.end
}

fn markdown_block_ranges(source: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut current_start = None;
    let mut current_end = 0_usize;
    let mut depth = 0_usize;
    let parser = Parser::new_ext(source, markdown_parser_options()).into_offset_iter();
    let source_only_ranges = parser
        .reference_definitions()
        .iter()
        .map(|(_, definition)| definition.span.clone())
        .collect::<Vec<_>>();
    ranges.extend(source_only_ranges.iter().cloned());

    for (event, range) in parser {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    current_start = Some(range.start);
                    current_end = range.end;
                }
                depth += 1;
            }
            Event::End(_) => {
                current_end = current_end.max(range.end);
                depth = depth.saturating_sub(1);
                if depth == 0
                    && let Some(start) = current_start.take()
                {
                    ranges.push(start..current_end);
                }
            }
            _ if depth == 0 => ranges.push(range),
            _ => current_end = current_end.max(range.end),
        }
    }

    ranges.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::new();
    for mut range in ranges.into_iter().filter(|range| range.start < range.end) {
        while range.end > range.start && matches!(source.as_bytes()[range.end - 1], b'\r' | b'\n') {
            range.end -= 1;
        }
        if range.start == range.end {
            continue;
        }
        if let Some(previous) = merged.last_mut()
            && range.start < previous.end
        {
            previous.end = previous.end.max(range.end);
        } else {
            merged.push(range);
        }
    }
    merged
}

fn is_reference_definition(source: &str) -> bool {
    Parser::new_ext(source, markdown_parser_options())
        .reference_definitions()
        .iter()
        .next()
        .is_some()
}

#[derive(PartialEq, Eq, Debug)]
struct CommandResult {
    text: String,
    selection: Range<usize>,
}

fn apply_markdown_command(
    source: &str,
    selection: Range<usize>,
    command: MarkdownCommand,
) -> CommandResult {
    match command {
        MarkdownCommand::Heading1 => set_heading(source, 1),
        MarkdownCommand::Heading2 => set_heading(source, 2),
        MarkdownCommand::Bold => wrap_selection(source, selection, "**", "**", "bold text"),
        MarkdownCommand::Italic => wrap_selection(source, selection, "*", "*", "italic text"),
        MarkdownCommand::InlineCode => wrap_selection(source, selection, "`", "`", "code"),
        MarkdownCommand::Link => insert_link(source, selection),
        MarkdownCommand::BulletedList => toggle_line_prefix(source, "- "),
        MarkdownCommand::Quote => toggle_line_prefix(source, "> "),
    }
}

fn set_heading(source: &str, level: usize) -> CommandResult {
    let content = source
        .trim_end_matches(['\r', '\n'])
        .trim_start_matches('#')
        .trim_start();
    let prefix = format!("{} ", "#".repeat(level));
    let text = format!("{prefix}{content}");
    let end = text.chars().count();
    CommandResult {
        text,
        selection: prefix.chars().count()..end,
    }
}

fn wrap_selection(
    source: &str,
    selection: Range<usize>,
    prefix: &str,
    suffix: &str,
    empty_text: &str,
) -> CommandResult {
    let selection = bounded_char_range(source, selection);
    let byte_range = char_range_to_byte_range(source, selection.clone());
    let selected = &source[byte_range.clone()];
    let content = if selected.is_empty() {
        empty_text
    } else {
        selected
    };
    let mut text =
        String::with_capacity(source.len() + prefix.len() + suffix.len() + content.len());
    text.push_str(&source[..byte_range.start]);
    text.push_str(prefix);
    text.push_str(content);
    text.push_str(suffix);
    text.push_str(&source[byte_range.end..]);
    let start = selection.start + prefix.chars().count();
    CommandResult {
        text,
        selection: start..start + content.chars().count(),
    }
}

fn insert_link(source: &str, selection: Range<usize>) -> CommandResult {
    let selection = bounded_char_range(source, selection);
    let byte_range = char_range_to_byte_range(source, selection.clone());
    let selected = &source[byte_range.clone()];
    let label = if selected.is_empty() {
        "link text"
    } else {
        selected
    };
    let target = "https://example.com";
    let replacement = format!("[{label}]({target})");
    let mut text = source.to_owned();
    text.replace_range(byte_range, &replacement);
    let selection_start = if selected.is_empty() {
        selection.start + 1
    } else {
        selection.start + label.chars().count() + 3
    };
    let selection_length = if selected.is_empty() {
        label.chars().count()
    } else {
        target.chars().count()
    };
    CommandResult {
        text,
        selection: selection_start..selection_start + selection_length,
    }
}

fn toggle_line_prefix(source: &str, prefix: &str) -> CommandResult {
    let lines = logical_lines(source).collect::<Vec<_>>();
    let remove = !lines.is_empty()
        && lines
            .iter()
            .all(|line| line.content().is_empty() || line.content().starts_with(prefix));
    let mut text = String::with_capacity(source.len() + prefix.len() * lines.len());
    for line in lines {
        let content = line.content();
        if !content.is_empty() {
            if remove {
                text.push_str(content.strip_prefix(prefix).unwrap_or(content));
            } else {
                text.push_str(prefix);
                text.push_str(content);
            }
        }
        if let Some(ending) = line.ending() {
            text.push_str(ending.as_str());
        }
    }
    let end = text.chars().count();
    CommandResult {
        text,
        selection: 0..end,
    }
}

fn bounded_char_range(source: &str, range: Range<usize>) -> Range<usize> {
    let char_count = source.chars().count();
    range.start.min(char_count)..range.end.max(range.start).min(char_count)
}

fn char_range_to_byte_range(source: &str, range: Range<usize>) -> Range<usize> {
    fn byte_index(source: &str, char_index: usize) -> usize {
        source
            .char_indices()
            .nth(char_index)
            .map_or(source.len(), |(index, _)| index)
    }
    byte_index(source, range.start)..byte_index(source, range.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parser_groups_nested_list_as_one_editable_block() {
        let source = "# Heading\n\n- first\n  - nested\n- second\n\nParagraph\n";
        let ranges = markdown_block_ranges(source);

        assert_eq!(
            ranges
                .iter()
                .map(|range| &source[range.clone()])
                .collect::<Vec<_>>(),
            vec!["# Heading", "- first\n  - nested\n- second", "Paragraph"]
        );
    }

    #[test]
    fn parser_exposes_reference_definitions_as_editable_source_blocks() {
        let source = "Paragraph using [Noter].\n\n[Noter]: https://github.com/blisspixel/noter\n";
        let ranges = markdown_block_ranges(source);

        assert_eq!(
            ranges
                .iter()
                .map(|range| &source[range.clone()])
                .collect::<Vec<_>>(),
            vec![
                "Paragraph using [Noter].",
                "[Noter]: https://github.com/blisspixel/noter"
            ]
        );
        assert!(is_reference_definition(&source[ranges[1].clone()]));
    }

    #[test]
    fn narrow_toolbars_use_the_compact_format_menu() {
        for (width, expected) in [(420.0, false), (800.0, true)] {
            assert_eq!(expanded_toolbar_fits(std::hint::black_box(width)), expected);
        }
    }

    #[test]
    fn block_parser_uses_the_same_markdown_extensions_as_the_renderer() {
        let expected = Options::ENABLE_TABLES
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_FOOTNOTES
            | Options::ENABLE_DEFINITION_LIST;

        assert_eq!(markdown_parser_options(), expected);
    }

    #[test]
    fn inline_commands_preserve_unicode_selection_boundaries() {
        let result = apply_markdown_command("cafe cafe", 0..4, MarkdownCommand::Bold);

        assert_eq!(result.text, "**cafe** cafe");
        assert_eq!(result.selection, 2..6);
    }

    #[test]
    fn empty_inline_selection_inserts_editable_text() {
        let result = apply_markdown_command("Text ", 5..5, MarkdownCommand::Italic);

        assert_eq!(result.text, "Text *italic text*");
        assert_eq!(&result.text[6..17], "italic text");
    }

    #[test]
    fn heading_command_replaces_existing_atx_marker() {
        let result = apply_markdown_command("#### Details", 0..0, MarkdownCommand::Heading2);

        assert_eq!(result.text, "## Details");
        assert_eq!(result.selection, 3..10);
    }

    #[test]
    fn line_commands_toggle_without_losing_final_newline() {
        let added = apply_markdown_command("one\ntwo\n", 0..0, MarkdownCommand::BulletedList);
        assert_eq!(added.text, "- one\n- two\n");

        let removed = apply_markdown_command(&added.text, 0..0, MarkdownCommand::BulletedList);
        assert_eq!(removed.text, "one\ntwo\n");
    }

    #[test]
    fn line_commands_preserve_crlf_cr_and_mixed_endings() {
        for source in ["one\r\ntwo\r\n", "one\rtwo\r", "one\r\ntwo\nthree\r"] {
            let added = apply_markdown_command(source, 0..0, MarkdownCommand::Quote);
            let removed = apply_markdown_command(&added.text, 0..0, MarkdownCommand::Quote);

            assert_eq!(removed.text, source);
        }
    }

    #[test]
    fn selected_link_keeps_label_and_selects_target() {
        let result = apply_markdown_command("Read Noter", 5..10, MarkdownCommand::Link);

        assert_eq!(result.text, "Read [Noter](https://example.com)");
        let target = result
            .text
            .chars()
            .skip(result.selection.start)
            .take(result.selection.len())
            .collect::<String>();
        assert_eq!(target, "https://example.com");
    }

    #[test]
    fn formatted_view_renders_without_mutating_source() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "# Heading\n\nA **formatted** paragraph.\n".to_owned();
        let original = source.clone();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            editor.toolbar(ui);
            assert!(!editor.show(ui, &mut source));
        });

        assert_eq!(source, original);
        assert!(!editor.is_editing());
        assert!(!output.shapes.is_empty());
    }

    #[cfg(feature = "screenshot-qa")]
    #[test]
    fn screenshot_state_opens_the_first_source_block_for_editing() {
        let mut editor = MarkdownEditor::default();
        let source = "# Heading\n\nParagraph\n";

        editor.activate_first_block(source);

        let active = editor
            .active
            .as_ref()
            .expect("first block should be active");
        assert_eq!(active.source_range, 0..9);
        assert_eq!(active.draft, "# Heading");
    }

    #[test]
    fn active_block_command_updates_source_and_renders_editor() {
        let context = egui::Context::default();
        let mut active = ActiveBlock::new(0..4, "text".to_owned(), 1);
        active.selection = 0..4;
        active.apply(MarkdownCommand::Bold);
        let mut editor = MarkdownEditor {
            cache: CommonMarkCache::default(),
            active: Some(active),
            next_editor_serial: 1,
        };
        let mut source = "text".to_owned();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            editor.toolbar(ui);
            assert!(editor.show(ui, &mut source));
        });

        assert_eq!(source, "**text**");
        assert!(editor.is_editing());
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn direct_edit_of_an_early_block_commits_after_later_blocks_render() {
        let context = egui::Context::default();
        let mut active = ActiveBlock::new(0..3, "# A".to_owned(), 1);
        active.selection = 0..3;
        let mut editor = MarkdownEditor {
            cache: CommonMarkCache::default(),
            active: Some(active),
            next_editor_serial: 1,
        };
        let mut source = "# A\n\nParagraph\n".to_owned();

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert!(!editor.show(ui, &mut source));
        });

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Text("x".to_owned()));
        let output = context.run_ui(input, |ui| {
            ui.set_width(800.0);
            assert!(editor.show(ui, &mut source));
        });

        assert_eq!(source, "x\n\nParagraph\n");
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn each_activated_block_gets_an_isolated_undo_identity() {
        let mut editor = MarkdownEditor::default();
        editor.activate(0..3, "one".to_owned());
        let first_id = editor
            .active
            .as_ref()
            .expect("the first block should be active")
            .editor_id();

        editor.activate(5..8, "two".to_owned());
        let second_id = editor
            .active
            .as_ref()
            .expect("the second block should be active")
            .editor_id();

        assert_ne!(first_id, second_id);
    }
}
