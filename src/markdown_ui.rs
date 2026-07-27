use std::ops::Range;

use eframe::egui;
use noter::core::line_endings::logical_lines;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag};

const ACTIVE_EDITOR_ID: &str = "noter-markdown-active-block";
const EXPANDED_FORMAT_MIN_WIDTH: f32 = 540.0;
const MARKER_FONT_SIZE: f32 = 0.1;
const BODY_WEIGHT: f32 = 400.0;
const HEADING_WEIGHT: f32 = 600.0;
const STRONG_WEIGHT: f32 = 700.0;
pub const PROTOTYPE_MARKDOWN_MAX_BYTES: usize = 1024 * 1024;
const PROTOTYPE_MARKDOWN_MAX_LOGICAL_LINES: usize = 8192;
const PROTOTYPE_MARKDOWN_MAX_LINE_BYTES: usize = 64 * 1024;
const PROTOTYPE_MARKDOWN_MAX_BLOCKS: usize = 512;
const PROTOTYPE_MARKDOWN_MAX_BLOCK_BYTES: usize = 64 * 1024;
const PROTOTYPE_MARKDOWN_MAX_PARSER_EVENTS: usize = 8192;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarkdownProjectionLimit {
    SourceBytes,
    LogicalLines,
    LineBytes,
    Blocks,
    BlockBytes,
    ParserEvents,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MarkdownShowOutcome {
    Unchanged,
    Changed,
    ProjectionLimitExceeded(MarkdownProjectionLimit),
}

impl MarkdownShowOutcome {
    pub const fn changed(self) -> bool {
        !matches!(self, Self::Unchanged)
    }

    pub const fn projection_limit(self) -> Option<MarkdownProjectionLimit> {
        match self {
            Self::ProjectionLimitExceeded(limit) => Some(limit),
            Self::Unchanged | Self::Changed => None,
        }
    }
}

impl MarkdownProjectionLimit {
    pub const fn description(self) -> &'static str {
        match self {
            Self::SourceBytes => "1 MiB source-size limit",
            Self::LogicalLines => "8,192-line work budget",
            Self::LineBytes => "64 KiB line-length budget",
            Self::Blocks => "512-block layout budget",
            Self::BlockBytes => "64 KiB block-layout budget",
            Self::ParserEvents => "8,192-event parser budget",
        }
    }
}

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

pub fn markdown_projection_limit(source: &str) -> Option<MarkdownProjectionLimit> {
    if source.len() > PROTOTYPE_MARKDOWN_MAX_BYTES {
        return Some(MarkdownProjectionLimit::SourceBytes);
    }

    let mut logical_line_count = 0_usize;
    for line in logical_lines(source) {
        logical_line_count += 1;
        if logical_line_count > PROTOTYPE_MARKDOWN_MAX_LOGICAL_LINES {
            return Some(MarkdownProjectionLimit::LogicalLines);
        }
        if line.content().len() > PROTOTYPE_MARKDOWN_MAX_LINE_BYTES {
            return Some(MarkdownProjectionLimit::LineBytes);
        }
    }

    let parser = Parser::new_ext(source, markdown_parser_options()).into_offset_iter();
    let mut block_count = 0_usize;
    for (_, definition) in parser.reference_definitions().iter() {
        if let Some(limit) = count_projection_block(&mut block_count, &definition.span) {
            return Some(limit);
        }
    }

    let mut event_count = 0_usize;
    let mut depth = 0_usize;
    let mut current_start = None;
    let mut current_end = 0_usize;
    for (event, range) in parser {
        event_count += 1;
        if event_count > PROTOTYPE_MARKDOWN_MAX_PARSER_EVENTS {
            return Some(MarkdownProjectionLimit::ParserEvents);
        }
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
                    && let Some(limit) =
                        count_projection_block(&mut block_count, &(start..current_end))
                {
                    return Some(limit);
                }
            }
            _ if depth == 0 => {
                if let Some(limit) = count_projection_block(&mut block_count, &range) {
                    return Some(limit);
                }
            }
            _ => current_end = current_end.max(range.end),
        }
    }
    None
}

fn count_projection_block(
    block_count: &mut usize,
    range: &Range<usize>,
) -> Option<MarkdownProjectionLimit> {
    if range.end.saturating_sub(range.start) > PROTOTYPE_MARKDOWN_MAX_BLOCK_BYTES {
        return Some(MarkdownProjectionLimit::BlockBytes);
    }
    *block_count += 1;
    (*block_count > PROTOTYPE_MARKDOWN_MAX_BLOCKS).then_some(MarkdownProjectionLimit::Blocks)
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
            Self::Heading1 => "Format the active content as a level-one heading",
            Self::Heading2 => "Format the active content as a level-two heading",
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
    active: Option<ActiveBlock>,
    next_editor_serial: u64,
}

impl MarkdownEditor {
    pub fn reset(&mut self) {
        self.active = None;
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
                    "Select formatted content before applying a format"
                });
            if response.clicked() {
                self.apply_command(command);
            }
        }
        if enabled {
            ui.separator();
            if ui
                .add(egui::Button::new("Done").min_size(egui::vec2(54.0, 28.0)))
                .on_hover_text("Finish editing the active content")
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
            response.on_disabled_hover_text("Select formatted content before applying a format");
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

    #[cfg(any(test, feature = "screenshot-qa"))]
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

    pub fn show(&mut self, ui: &mut egui::Ui, source: &mut String) -> MarkdownShowOutcome {
        let mut changed = self.sync_pending_command(source);
        if changed && let Some(limit) = markdown_projection_limit(source) {
            return MarkdownShowOutcome::ProjectionLimitExceeded(limit);
        }
        let ranges = markdown_block_ranges(source);

        if ranges.is_empty() {
            if self.active.is_none() {
                self.activate(0..0, String::new());
            }
            if self.show_active_editor(ui) {
                let synchronized = self.sync_pending_command(source);
                changed |= synchronized;
                if synchronized && let Some(limit) = markdown_projection_limit(source) {
                    return MarkdownShowOutcome::ProjectionLimitExceeded(limit);
                }
            }
            return if changed {
                MarkdownShowOutcome::Changed
            } else {
                MarkdownShowOutcome::Unchanged
            };
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
            let synchronized = self.sync_pending_command(source);
            changed |= synchronized;
            if synchronized && let Some(limit) = markdown_projection_limit(source) {
                return MarkdownShowOutcome::ProjectionLimitExceeded(limit);
            }
        }
        if changed {
            MarkdownShowOutcome::Changed
        } else {
            MarkdownShowOutcome::Unchanged
        }
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
                    let mut job = markdown_render_layout(block, ui.style());
                    job.wrap.max_width = ui.available_width();
                    let label = egui::Label::new(job).wrap().sense(egui::Sense::click());
                    if is_block_quote(block) {
                        let response = ui
                            .horizontal_top(|ui| {
                                ui.add_space(12.0);
                                ui.add(label)
                            })
                            .inner;
                        ui.painter().vline(
                            response.rect.left() - 8.0,
                            response.rect.y_range(),
                            egui::Stroke::new(2.0, ui.visuals().weak_text_color()),
                        );
                    } else {
                        ui.add(label);
                    }
                }
            })
            .response
            .interact(egui::Sense::click());

        response
            .clone()
            .on_hover_cursor(egui::CursorIcon::Text)
            .on_hover_text("Click to edit this formatted content");
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
        let selection = active.selection.clone();
        let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
            let source = buffer.as_str();
            let selection =
                char_range_to_byte_range(source, bounded_char_range(source, selection.clone()));
            let reveal = semantic_target_at_selection(source, &selection);
            let mut job = markdown_edit_layout(source, ui.style(), reveal);
            job.wrap.max_width = wrap_width;
            ui.fonts_mut(|fonts| fonts.layout_job(job))
        };
        let editor = egui::TextEdit::multiline(&mut active.draft)
            .id(editor_id)
            .font(egui::TextStyle::Body)
            .desired_width(f32::INFINITY)
            .desired_rows(rows)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::symmetric(8, 2))
            .layouter(&mut layouter);
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct MarkdownSourceStyle {
    flags: u8,
    heading_level: u8,
}

const STYLE_VISIBLE: u8 = 1 << 0;
const STYLE_STRONG: u8 = 1 << 1;
const STYLE_EMPHASIS: u8 = 1 << 2;
const STYLE_CODE: u8 = 1 << 3;
const STYLE_LINK: u8 = 1 << 4;
const STYLE_STRIKETHROUGH: u8 = 1 << 5;

impl MarkdownSourceStyle {
    const fn has(self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    const fn add(&mut self, flag: u8) {
        self.flags |= flag;
    }
}

impl Default for MarkdownSourceStyle {
    fn default() -> Self {
        Self {
            flags: STYLE_VISIBLE,
            heading_level: 0,
        }
    }
}

fn markdown_edit_layout(
    source: &str,
    style: &egui::Style,
    revealed_semantic_target: Option<Range<usize>>,
) -> egui::text::LayoutJob {
    let mut source_styles = markdown_source_styles(source);
    if let Some(range) = revealed_semantic_target {
        let mut target_style = MarkdownSourceStyle::default();
        target_style.add(STYLE_LINK);
        set_source_style(&mut source_styles, range, target_style);
    }
    let mut job = egui::text::LayoutJob::default();
    let mut section_start = 0_usize;
    let mut section_style = source_styles.first().copied().unwrap_or_default();

    for (index, _) in source.char_indices().skip(1) {
        let current_style = source_styles[index];
        if current_style != section_style {
            job.append(
                &source[section_start..index],
                0.0,
                markdown_text_format(section_style, style),
            );
            section_start = index;
            section_style = current_style;
        }
    }
    if !source.is_empty() {
        job.append(
            &source[section_start..],
            0.0,
            markdown_text_format(section_style, style),
        );
    }
    job
}

fn markdown_render_layout(source: &str, style: &egui::Style) -> egui::text::LayoutJob {
    let source_styles = markdown_source_styles(source);
    let mut job = egui::text::LayoutJob::default();
    let mut run = String::new();
    let mut run_style = None;
    let mut suppress_quote_space = false;

    for (index, character) in source.char_indices() {
        let source_style = source_styles[index];
        if !source_style.has(STYLE_VISIBLE) {
            append_render_run(&mut job, &mut run, run_style.take(), style);
            continue;
        }
        if is_quote_marker(source, index, character) {
            append_render_run(&mut job, &mut run, run_style.take(), style);
            suppress_quote_space = true;
            continue;
        }
        if suppress_quote_space {
            suppress_quote_space = false;
            if character == ' ' {
                continue;
            }
        }
        run_style = Some(source_style);
        run.push(formatted_block_marker(source, index, character));
    }
    append_render_run(&mut job, &mut run, run_style, style);
    job
}

fn append_render_run(
    job: &mut egui::text::LayoutJob,
    run: &mut String,
    source_style: Option<MarkdownSourceStyle>,
    style: &egui::Style,
) {
    if let Some(source_style) = source_style
        && !run.is_empty()
    {
        job.append(run, 0.0, markdown_text_format(source_style, style));
    }
    run.clear();
}

fn formatted_block_marker(source: &str, index: usize, character: char) -> char {
    let line_start = source[..index]
        .rfind(['\n', '\r'])
        .map_or(0, |position| position + 1);
    let indented_line_start = source[line_start..index]
        .chars()
        .all(|prefix| matches!(prefix, ' ' | '\t'));
    let followed_by_space = source[index + character.len_utf8()..].starts_with(' ');

    if indented_line_start && followed_by_space && matches!(character, '-' | '+' | '*') {
        '•'
    } else {
        character
    }
}

fn is_quote_marker(source: &str, index: usize, character: char) -> bool {
    if character != '>' {
        return false;
    }
    let line_start = source[..index]
        .rfind(['\n', '\r'])
        .map_or(0, |position| position + 1);
    source[line_start..index]
        .chars()
        .all(|prefix| matches!(prefix, ' ' | '\t'))
        && source[index + 1..].starts_with(' ')
}

fn semantic_target_at_selection(source: &str, selection: &Range<usize>) -> Option<Range<usize>> {
    Parser::new_ext(source, markdown_parser_options())
        .into_offset_iter()
        .find_map(|(event, source_range)| {
            let Event::Start(
                Tag::Link {
                    dest_url: destination,
                    ..
                }
                | Tag::Image {
                    dest_url: destination,
                    ..
                },
            ) = event
            else {
                return None;
            };
            let raw = source.get(source_range.clone())?;
            let offset = raw.rfind(destination.as_ref())?;
            let target =
                source_range.start + offset..source_range.start + offset + destination.len();
            let selected = if selection.start == selection.end {
                (target.start..=target.end).contains(&selection.start)
            } else {
                ranges_overlap(selection, &target)
            };
            selected.then_some(target)
        })
}

fn markdown_source_styles(source: &str) -> Vec<MarkdownSourceStyle> {
    let mut styles = vec![MarkdownSourceStyle::default(); source.len()];
    let mut current = MarkdownSourceStyle::default();
    let mut stack = Vec::new();

    for (event, range) in Parser::new_ext(source, markdown_parser_options()).into_offset_iter() {
        match event {
            Event::Start(tag) => {
                stack.push(current);
                apply_markdown_tag(&mut current, &tag);
                if tag_hides_source_markup(&tag) {
                    set_source_style(&mut styles, range, hidden_source_style());
                }
            }
            Event::End(_) => {
                current = stack.pop().unwrap_or_default();
            }
            Event::Text(text) => {
                reveal_event_text(source, &mut styles, range, text.as_ref(), current);
            }
            Event::Code(code) => {
                let mut code_style = current;
                code_style.add(STYLE_CODE);
                reveal_event_text(source, &mut styles, range, code.as_ref(), code_style);
            }
            Event::SoftBreak | Event::HardBreak => {
                set_source_style(&mut styles, range, current);
            }
            _ => {}
        }
    }
    styles
}

const fn apply_markdown_tag(style: &mut MarkdownSourceStyle, tag: &Tag<'_>) {
    match tag {
        Tag::Heading { level, .. } => style.heading_level = heading_level_number(*level),
        Tag::CodeBlock(_) => style.add(STYLE_CODE),
        Tag::Emphasis => style.add(STYLE_EMPHASIS),
        Tag::Strong => style.add(STYLE_STRONG),
        Tag::Strikethrough => style.add(STYLE_STRIKETHROUGH),
        Tag::Link { .. } | Tag::Image { .. } => style.add(STYLE_LINK),
        _ => {}
    }
}

const fn tag_hides_source_markup(tag: &Tag<'_>) -> bool {
    matches!(
        tag,
        Tag::Heading { .. }
            | Tag::CodeBlock(_)
            | Tag::Emphasis
            | Tag::Strong
            | Tag::Strikethrough
            | Tag::Superscript
            | Tag::Subscript
            | Tag::Link { .. }
            | Tag::Image { .. }
    )
}

const fn heading_level_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn reveal_event_text(
    source: &str,
    styles: &mut [MarkdownSourceStyle],
    range: Range<usize>,
    rendered: &str,
    style: MarkdownSourceStyle,
) {
    set_source_style(styles, range.clone(), hidden_source_style());
    let Some(raw) = source.get(range.clone()) else {
        return;
    };
    let visible = raw.find(rendered).map_or_else(
        || range.clone(),
        |offset| range.start + offset..range.start + offset + rendered.len(),
    );
    set_source_style(styles, visible, style);
}

fn set_source_style(
    styles: &mut [MarkdownSourceStyle],
    range: Range<usize>,
    style: MarkdownSourceStyle,
) {
    if range.start <= range.end && range.end <= styles.len() {
        styles[range].fill(style);
    }
}

const fn hidden_source_style() -> MarkdownSourceStyle {
    MarkdownSourceStyle {
        flags: 0,
        heading_level: 0,
    }
}

fn markdown_text_format(
    source_style: MarkdownSourceStyle,
    style: &egui::Style,
) -> egui::TextFormat {
    if !source_style.has(STYLE_VISIBLE) {
        return egui::TextFormat {
            font_id: egui::FontId::new(MARKER_FONT_SIZE, egui::FontFamily::Proportional),
            color: egui::Color32::TRANSPARENT,
            ..Default::default()
        };
    }

    let mut font_id = style.text_styles[&egui::TextStyle::Body].clone();
    let heading_weight = if source_style.heading_level > 0 {
        font_id.size = match source_style.heading_level {
            1 => 28.0,
            2 => 24.0,
            3 => 21.0,
            4 => 18.0,
            5 => 16.0,
            _ => 15.0,
        };
        HEADING_WEIGHT
    } else {
        BODY_WEIGHT
    };
    if source_style.has(STYLE_CODE) {
        font_id = style.text_styles[&egui::TextStyle::Monospace].clone();
    }
    let weight = if source_style.has(STYLE_STRONG) {
        STRONG_WEIGHT
    } else {
        heading_weight
    };

    let color = if source_style.has(STYLE_LINK) {
        style.visuals.hyperlink_color
    } else if source_style.has(STYLE_STRONG) {
        style.visuals.strong_text_color()
    } else {
        style.visuals.text_color()
    };
    let mut format = egui::TextFormat {
        font_id,
        color,
        background: if source_style.has(STYLE_CODE) {
            style.visuals.code_bg_color
        } else {
            egui::Color32::TRANSPARENT
        },
        italics: source_style.has(STYLE_EMPHASIS),
        underline: if source_style.has(STYLE_LINK) {
            egui::Stroke::new(1.0, style.visuals.hyperlink_color)
        } else {
            egui::Stroke::NONE
        },
        strikethrough: if source_style.has(STYLE_STRIKETHROUGH) {
            egui::Stroke::new(1.0, color)
        } else {
            egui::Stroke::NONE
        },
        ..Default::default()
    };
    format.coords.push("wght", weight);
    format
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

fn is_block_quote(source: &str) -> bool {
    Parser::new_ext(source, markdown_parser_options())
        .any(|event| matches!(event, Event::Start(Tag::BlockQuote(_))))
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
    fn projection_budget_accepts_exact_byte_boundaries_for_ascii_and_unicode() {
        let ascii = format!("{}\n\n", "x".repeat(4_094)).repeat(256);
        let unicode = format!("{}\n\n", "é".repeat(2_047)).repeat(256);

        for source in [ascii, unicode] {
            assert_eq!(source.len(), PROTOTYPE_MARKDOWN_MAX_BYTES);
            assert_eq!(markdown_projection_limit(&source), None);
        }
    }

    #[test]
    fn projection_budget_rejects_a_source_one_byte_over_the_limit() {
        let mut source = format!("{}\n\n", "x".repeat(4_094)).repeat(256);
        source.push('x');

        assert_eq!(
            markdown_projection_limit(&source),
            Some(MarkdownProjectionLimit::SourceBytes)
        );
    }

    #[test]
    fn projection_budget_bounds_lines_blocks_and_parser_events() {
        let too_many_lines = "x\n".repeat(PROTOTYPE_MARKDOWN_MAX_LOGICAL_LINES + 1);
        let overlong_line = "x".repeat(PROTOTYPE_MARKDOWN_MAX_LINE_BYTES + 1);
        let too_many_blocks = "# x\n\n".repeat(PROTOTYPE_MARKDOWN_MAX_BLOCKS + 1);
        let oversized_block = format!("{}\n", "x".repeat(1_024)).repeat(65);
        let too_many_events = "*x* ".repeat(3_000);

        assert_eq!(
            markdown_projection_limit(&too_many_lines),
            Some(MarkdownProjectionLimit::LogicalLines)
        );
        assert_eq!(
            markdown_projection_limit(&overlong_line),
            Some(MarkdownProjectionLimit::LineBytes)
        );
        assert_eq!(
            markdown_projection_limit(&too_many_blocks),
            Some(MarkdownProjectionLimit::Blocks)
        );
        assert_eq!(
            markdown_projection_limit(&oversized_block),
            Some(MarkdownProjectionLimit::BlockBytes)
        );
        assert_eq!(
            markdown_projection_limit(&too_many_events),
            Some(MarkdownProjectionLimit::ParserEvents)
        );
    }

    #[test]
    fn projection_budget_accepts_each_exact_structural_ceiling() {
        let exact_lines = "\n".repeat(PROTOTYPE_MARKDOWN_MAX_LOGICAL_LINES);
        let exact_line_bytes = "x".repeat(PROTOTYPE_MARKDOWN_MAX_LINE_BYTES);
        let exact_blocks = "# x\n\n".repeat(PROTOTYPE_MARKDOWN_MAX_BLOCKS);
        let exact_block_span = format!("{}\n", "x".repeat(1_023)).repeat(63) + &"x".repeat(1_024);
        let exact_events = "`x` ".repeat(4_095) + "\n\n---\n";

        assert_eq!(exact_block_span.len(), PROTOTYPE_MARKDOWN_MAX_BLOCK_BYTES);
        assert_eq!(
            Parser::new_ext(&exact_events, markdown_parser_options()).count(),
            PROTOTYPE_MARKDOWN_MAX_PARSER_EVENTS
        );
        for source in [
            exact_lines,
            exact_line_bytes,
            exact_blocks,
            exact_block_span,
            exact_events,
        ] {
            assert_eq!(markdown_projection_limit(&source), None);
        }
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
    fn empty_link_selection_selects_the_inserted_label() {
        let result = apply_markdown_command("before after", 7..7, MarkdownCommand::Link);

        assert_eq!(result.text, "before [link text](https://example.com)after");
        assert_eq!(result.selection, 8..17);
        assert_eq!(&result.text[result.selection], "link text");
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
    fn selected_link_target_is_visible_while_it_is_being_edited() {
        let style = egui::Style::default();
        let result = apply_markdown_command("Read Noter", 5..10, MarkdownCommand::Link);
        let selection = char_range_to_byte_range(&result.text, result.selection);
        let reveal = semantic_target_at_selection(&result.text, &selection)
            .expect("the selected URL must be recognized as editable semantic content");

        let job = markdown_edit_layout(&result.text, &style, Some(reveal.clone()));
        let target = job.format_at_byte(egui::text::ByteIndex(reveal.start));

        assert_ne!(target.color, egui::Color32::TRANSPARENT);
        assert!(target.font_id.size > MARKER_FONT_SIZE);
        assert_eq!(target.color, style.visuals.hyperlink_color);
        assert_ne!(target.underline, egui::Stroke::NONE);
    }

    #[test]
    fn caret_inside_link_target_reveals_the_target() {
        let source = "[label](https://example.com)";
        let start = source
            .find("https://")
            .expect("the fixture contains a link target");
        let target = start..start + "https://example.com".len();
        for caret in [target.start, target.end] {
            assert_eq!(
                semantic_target_at_selection(source, &(caret..caret)),
                Some(target.clone())
            );
        }
    }

    #[test]
    fn link_target_hides_again_after_the_caret_leaves_it() {
        let source = "[Noter](https://example.com) after";
        let outside = source.len()..source.len();

        assert!(semantic_target_at_selection(source, &outside).is_none());

        let job = markdown_edit_layout(source, &egui::Style::default(), None);
        let target_start = source
            .find("https://")
            .expect("the fixture contains a link target");
        assert_eq!(
            job.format_at_byte(egui::text::ByteIndex(target_start))
                .color,
            egui::Color32::TRANSPARENT
        );
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
            assert!(!editor.show(ui, &mut source).changed());
        });

        assert_eq!(source, original);
        assert!(!editor.is_editing());
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn inactive_markdown_uses_real_heading_and_strong_weights() {
        type WeightedSections = Vec<(String, Vec<([u8; 4], f32)>)>;

        fn collect_weights(shape: &egui::Shape, weights: &mut WeightedSections) {
            match shape {
                egui::Shape::Text(text) => {
                    let job = &text.galley.job;
                    for section in &job.sections {
                        weights.push((
                            job.text[section.byte_range.start.0..section.byte_range.end.0]
                                .to_owned(),
                            section
                                .format
                                .coords
                                .as_ref()
                                .iter()
                                .map(|(tag, value)| (tag.to_be_bytes(), *value))
                                .collect(),
                        ));
                    }
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect_weights(shape, weights);
                    }
                }
                _ => {}
            }
        }

        fn has_weight(weights: &WeightedSections, text: &str, expected: f32) -> bool {
            weights.iter().any(|(section, coords)| {
                section == text
                    && coords.iter().any(|(tag, value)| {
                        *tag == *b"wght" && value.to_bits() == expected.to_bits()
                    })
            })
        }

        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "# Heading\n\nA **bold** paragraph.\n".to_owned();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert!(!editor.show(ui, &mut source).changed());
        });
        let mut weights = Vec::new();
        for clipped in &output.shapes {
            collect_weights(&clipped.shape, &mut weights);
        }

        assert!(has_weight(&weights, "Heading", HEADING_WEIGHT));
        assert!(has_weight(&weights, "bold", STRONG_WEIGHT));
    }

    #[test]
    fn rendered_markdown_preserves_word_spacing_and_formats_list_markers() {
        let source = "- **Every feature** should earn its place.\n- Plain item";
        let job = markdown_render_layout(source, &egui::Style::default());

        assert_eq!(
            job.text,
            "• Every feature should earn its place.\n• Plain item"
        );
        assert!(!job.text.contains("featureshould"));
        let feature = job
            .text
            .find("feature")
            .expect("the rendered list contains the strong phrase");
        let format = job.format_at_byte(egui::text::ByteIndex(feature));
        assert!(format.coords.as_ref().iter().any(|(tag, value)| {
            tag.to_be_bytes() == *b"wght" && value.to_bits() == STRONG_WEIGHT.to_bits()
        }));
    }

    #[test]
    fn rendered_markdown_coalesces_only_contiguous_equal_style_runs() {
        let job = markdown_render_layout("plain **bold** tail", &egui::Style::default());
        let sections = job
            .sections
            .iter()
            .map(|section| &job.text[section.byte_range.start.0..section.byte_range.end.0])
            .collect::<Vec<_>>();

        assert_eq!(job.text, "plain bold tail");
        assert_eq!(sections, ["plain ", "bold", " tail"]);
    }

    #[test]
    fn list_marker_projection_rejects_prose_and_missing_space() {
        let style = egui::Style::default();

        assert_eq!(
            markdown_render_layout("word - text", &style).text,
            "word - text"
        );
        assert_eq!(
            markdown_render_layout("-not a list", &style).text,
            "-not a list"
        );
    }

    #[test]
    fn rendered_quote_uses_a_native_visual_marker_without_source_delimiters() {
        let job = markdown_render_layout(
            "> 1984 was a warning, not an instruction manual.",
            &egui::Style::default(),
        );

        assert_eq!(job.text, "1984 was a warning, not an instruction manual.");
        assert!(!job.text.contains('>'));
        assert!(is_block_quote(
            "> 1984 was a warning, not an instruction manual."
        ));
        assert!(!is_block_quote("An ordinary paragraph."));
    }

    #[test]
    fn quote_marker_projection_respects_line_boundaries_and_required_space() {
        let source = "paragraph\n> quoted line";
        let marker = source
            .find('>')
            .expect("the fixture contains a quote marker");

        assert!(is_quote_marker(source, marker, '>'));
        assert_eq!(
            markdown_render_layout(source, &egui::Style::default()).text,
            "paragraph\nquoted line"
        );
        assert!(!is_quote_marker("inline > quote", 7, '>'));
        assert!(!is_quote_marker(">not a quote", 0, '>'));
    }

    #[test]
    fn active_markdown_layout_hides_delimiters_and_styles_content() {
        let style = egui::Style::default();
        let source = "A **bold** and *italic* [link](https://example.com).";

        let job = markdown_edit_layout(source, &style, None);
        let marker = job.format_at_byte(egui::text::ByteIndex(2));
        let bold = job.format_at_byte(egui::text::ByteIndex(4));
        let italic = job.format_at_byte(egui::text::ByteIndex(17));
        let link = job.format_at_byte(egui::text::ByteIndex(27));

        assert_eq!(job.text, source);
        assert_eq!(marker.color, egui::Color32::TRANSPARENT);
        assert!(marker.font_id.size <= MARKER_FONT_SIZE);
        assert!(
            bold.coords
                .as_ref()
                .iter()
                .any(|(tag, value)| tag.to_be_bytes() == *b"wght"
                    && value.to_bits() == 700.0_f32.to_bits())
        );
        assert!(italic.italics);
        assert_eq!(link.color, style.visuals.hyperlink_color);
        assert_ne!(link.underline, egui::Stroke::NONE);
    }

    #[test]
    fn source_style_state_enters_and_leaves_nested_markdown_exactly() {
        let source = "**bold** plain *first\nsecond*";
        let styles = markdown_source_styles(source);
        let bold = source.find("bold").expect("the fixture contains bold");
        let plain = source.find("plain").expect("the fixture contains plain");
        let emphasis = source.find("first").expect("the fixture contains emphasis");
        let soft_break = source
            .find('\n')
            .expect("the fixture contains a soft break");
        let second = source.find("second").expect("the fixture contains second");

        assert!(!styles[0].has(STYLE_VISIBLE));
        assert!(styles[bold].has(STYLE_VISIBLE));
        assert!(styles[bold].has(STYLE_STRONG));
        assert!(styles[plain].has(STYLE_VISIBLE));
        assert!(!styles[plain].has(STYLE_STRONG));
        assert!(styles[emphasis].has(STYLE_EMPHASIS));
        assert!(styles[soft_break].has(STYLE_EMPHASIS));
        assert!(styles[second].has(STYLE_EMPHASIS));
    }

    #[test]
    fn event_text_reveal_offsets_the_complete_rendered_range() {
        let source = "xx[visible]yy";
        let mut styles = vec![MarkdownSourceStyle::default(); source.len()];
        let mut strong = MarkdownSourceStyle::default();
        strong.add(STYLE_STRONG);

        reveal_event_text(source, &mut styles, 2..11, "visible", strong);

        assert!(!styles[2].has(STYLE_VISIBLE));
        assert!(styles[3].has(STYLE_VISIBLE));
        assert!(styles[9].has(STYLE_VISIBLE));
        assert!(styles[9].has(STYLE_STRONG));
        assert!(!styles[10].has(STYLE_VISIBLE));
    }

    #[test]
    fn active_heading_layout_hides_source_marker_and_uses_heading_type() {
        let style = egui::Style::default();
        let source = "## Formatted heading";

        let job = markdown_edit_layout(source, &style, None);
        let marker = job.format_at_byte(egui::text::ByteIndex(0));
        let heading = job.format_at_byte(egui::text::ByteIndex(3));

        assert_eq!(marker.color, egui::Color32::TRANSPARENT);
        assert!(heading.font_id.size > style.text_styles[&egui::TextStyle::Body].size);
    }

    #[test]
    fn active_code_and_strikethrough_layouts_keep_their_visual_meaning() {
        let style = egui::Style::default();
        let source = "~~removed~~ and `code`";

        let job = markdown_edit_layout(source, &style, None);
        let strike_marker = job.format_at_byte(egui::text::ByteIndex(0));
        let removed = job.format_at_byte(egui::text::ByteIndex(
            source
                .find("removed")
                .expect("the fixture contains removed"),
        ));
        let code_marker = job.format_at_byte(egui::text::ByteIndex(
            source.find('`').expect("the fixture contains code markers"),
        ));
        let code = job.format_at_byte(egui::text::ByteIndex(
            source.find("code").expect("the fixture contains code"),
        ));

        assert_eq!(strike_marker.color, egui::Color32::TRANSPARENT);
        assert_ne!(removed.strikethrough, egui::Stroke::NONE);
        assert_eq!(code_marker.color, egui::Color32::TRANSPARENT);
        assert_eq!(code.font_id.family, egui::FontFamily::Monospace);
        assert_eq!(code.background, style.visuals.code_bg_color);
    }

    #[test]
    fn active_fenced_code_hides_the_fence_and_preserves_code_styling() {
        let style = egui::Style::default();
        let source = "```rust\nlet value = 1;\n```";

        let job = markdown_edit_layout(source, &style, None);
        let fence = job.format_at_byte(egui::text::ByteIndex(0));
        let code = job.format_at_byte(egui::text::ByteIndex(
            source.find("let value").expect("the fixture contains code"),
        ));

        assert_eq!(fence.color, egui::Color32::TRANSPARENT);
        assert_eq!(code.font_id.family, egui::FontFamily::Monospace);
        assert_eq!(code.background, style.visuals.code_bg_color);
    }

    #[test]
    fn every_heading_level_has_a_deliberate_type_size() {
        let style = egui::Style::default();
        let expected_sizes = [28.0_f32, 24.0, 21.0, 18.0, 16.0, 15.0];

        for (level, expected_size) in (1..=6).zip(expected_sizes) {
            let source = format!("{} Heading", "#".repeat(level));
            let job = markdown_edit_layout(&source, &style, None);
            let marker = job.format_at_byte(egui::text::ByteIndex(0));
            let heading = job.format_at_byte(egui::text::ByteIndex(level + 1));

            assert_eq!(marker.color, egui::Color32::TRANSPARENT);
            assert_eq!(heading.font_id.size.to_bits(), expected_size.to_bits());
            assert!(
                heading
                    .coords
                    .as_ref()
                    .iter()
                    .any(|(tag, value)| tag.to_be_bytes() == *b"wght"
                        && value.to_bits() == HEADING_WEIGHT.to_bits())
            );
        }
    }

    #[test]
    fn body_text_keeps_the_configured_body_size_and_weight() {
        let style = egui::Style::default();
        let job = markdown_edit_layout("ordinary body text", &style, None);
        let body = job.format_at_byte(egui::text::ByteIndex(0));

        assert_eq!(body.font_id, style.text_styles[&egui::TextStyle::Body]);
        assert!(body.coords.as_ref().iter().any(|(tag, value)| {
            tag.to_be_bytes() == *b"wght" && value.to_bits() == BODY_WEIGHT.to_bits()
        }));
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
            active: Some(active),
            next_editor_serial: 1,
        };
        let mut source = "text".to_owned();

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            editor.toolbar(ui);
            assert!(editor.show(ui, &mut source).changed());
        });

        assert_eq!(source, "**text**");
        assert!(editor.is_editing());
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn pending_format_command_is_bounded_before_projecting_the_changed_source() {
        let mut source = format!("{}\n", "x".repeat(1_023)).repeat(63) + &"x".repeat(1_024);
        assert_eq!(markdown_projection_limit(&source), None);
        let mut active = ActiveBlock::new(0..source.len(), source.clone(), 1);
        active.selection = 0..source.len();
        active.apply(MarkdownCommand::Quote);
        let mut editor = MarkdownEditor {
            active: Some(active),
            next_editor_serial: 1,
        };
        let context = egui::Context::default();
        let mut outcome = MarkdownShowOutcome::Unchanged;

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            outcome = editor.show(ui, &mut source);
        });

        assert_eq!(
            outcome,
            MarkdownShowOutcome::ProjectionLimitExceeded(MarkdownProjectionLimit::BlockBytes)
        );
        assert_eq!(
            markdown_projection_limit(&source),
            Some(MarkdownProjectionLimit::BlockBytes)
        );
    }

    #[test]
    fn direct_edit_of_an_early_block_commits_after_later_blocks_render() {
        let context = egui::Context::default();
        let mut active = ActiveBlock::new(0..3, "# A".to_owned(), 1);
        active.selection = 0..3;
        let mut editor = MarkdownEditor {
            active: Some(active),
            next_editor_serial: 1,
        };
        let mut source = "# A\n\nParagraph\n".to_owned();

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert!(!editor.show(ui, &mut source).changed());
        });

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Text("x".to_owned()));
        let output = context.run_ui(input, |ui| {
            ui.set_width(800.0);
            assert!(editor.show(ui, &mut source).changed());
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
