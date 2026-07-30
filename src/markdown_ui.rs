use std::ops::Range;

use eframe::egui;
use noter::core::edit::{EditOrigin, Selection};
use noter::core::line_endings::logical_lines;
use noter::core::markdown::recoverable_emphasis_spans;
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
    Changed(EditOrigin),
    ProjectionLimitExceeded {
        limit: MarkdownProjectionLimit,
        origin: EditOrigin,
    },
}

impl MarkdownShowOutcome {
    pub const fn changed(self) -> bool {
        !matches!(self, Self::Unchanged)
    }

    pub const fn projection_limit(self) -> Option<MarkdownProjectionLimit> {
        match self {
            Self::ProjectionLimitExceeded { limit, .. } => Some(limit),
            Self::Unchanged | Self::Changed(_) => None,
        }
    }

    pub const fn origin(self) -> Option<EditOrigin> {
        match self {
            Self::Changed(origin) | Self::ProjectionLimitExceeded { origin, .. } => Some(origin),
            Self::Unchanged => None,
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
    selection: CharSelection,
    editor_serial: u64,
    dirty: bool,
    pending_origin: Option<EditOrigin>,
    request_focus: bool,
}

impl ActiveBlock {
    fn new(source_range: Range<usize>, draft: String, editor_serial: u64) -> Self {
        let end = draft.chars().count();
        Self {
            source_range,
            draft,
            selection: CharSelection::caret(end),
            editor_serial,
            dirty: false,
            pending_origin: None,
            request_focus: true,
        }
    }

    fn editor_id(&self) -> egui::Id {
        egui::Id::new((ACTIVE_EDITOR_ID, self.editor_serial))
    }

    fn apply(&mut self, command: MarkdownCommand) {
        let result = apply_markdown_command(&self.draft, self.selection.ordered_range(), command);
        self.draft = result.text;
        self.selection = CharSelection::new(result.selection.start, result.selection.end);
        self.dirty = true;
        self.pending_origin = Some(EditOrigin::MarkdownFormatting);
        self.request_focus = true;
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct CharSelection {
    anchor: usize,
    active: usize,
}

impl CharSelection {
    const fn new(anchor: usize, active: usize) -> Self {
        Self { anchor, active }
    }

    const fn caret(position: usize) -> Self {
        Self::new(position, position)
    }

    fn ordered_range(self) -> Range<usize> {
        self.anchor.min(self.active)..self.anchor.max(self.active)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RenderedDragSelection {
    widget_id: egui::Id,
    anchor: usize,
}

struct RenderedActivation {
    source_range: Range<usize>,
    selection: CharSelection,
}

#[derive(Debug)]
struct RenderedSourceMap {
    source_span_for_rendered_character: Vec<Range<usize>>,
}

impl RenderedSourceMap {
    fn source_selection(&self, rendered: CharSelection) -> CharSelection {
        let rendered_count = self.source_span_for_rendered_character.len();
        let anchor = rendered.anchor.min(rendered_count);
        let active = rendered.active.min(rendered_count);
        match anchor.cmp(&active) {
            std::cmp::Ordering::Less => {
                CharSelection::new(self.start_boundary(anchor), self.end_boundary(active))
            }
            std::cmp::Ordering::Equal => CharSelection::caret(self.start_boundary(anchor)),
            std::cmp::Ordering::Greater => {
                CharSelection::new(self.end_boundary(anchor), self.start_boundary(active))
            }
        }
    }

    fn start_boundary(&self, rendered_cursor: usize) -> usize {
        self.source_span_for_rendered_character
            .get(rendered_cursor)
            .map(|span| span.start)
            .or_else(|| {
                self.source_span_for_rendered_character
                    .last()
                    .map(|span| span.end)
            })
            .unwrap_or(0)
    }

    fn end_boundary(&self, rendered_cursor: usize) -> usize {
        rendered_cursor
            .checked_sub(1)
            .and_then(|index| self.source_span_for_rendered_character.get(index))
            .map_or_else(|| self.start_boundary(0), |span| span.end)
    }
}

struct MarkdownRenderProjection {
    job: egui::text::LayoutJob,
    source_map: RenderedSourceMap,
}

struct RenderedBlockLabel {
    response: egui::Response,
    galley: std::sync::Arc<egui::Galley>,
    source_map: RenderedSourceMap,
}

impl RenderedBlockLabel {
    fn cursor_at(&self, position: egui::Pos2) -> usize {
        let local_position = position - self.response.rect.left_top();
        self.galley.cursor_from_pos(local_position).index.into()
    }
}

#[derive(Default)]
pub struct MarkdownEditor {
    active: Option<ActiveBlock>,
    next_editor_serial: u64,
    rendered_drag: Option<RenderedDragSelection>,
    finish_requested: bool,
}

impl MarkdownEditor {
    pub fn reset(&mut self) {
        self.active = None;
        self.rendered_drag = None;
        self.finish_requested = false;
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
                self.finish_requested = true;
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
                        self.finish_requested = true;
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
        self.rendered_drag = None;
        self.active = Some(ActiveBlock::new(
            source_range,
            draft,
            self.next_editor_serial,
        ));
    }

    fn activate_with_selection(
        &mut self,
        source_range: Range<usize>,
        draft: String,
        selection: CharSelection,
    ) {
        let character_count = draft.chars().count();
        self.activate(source_range, draft);
        if let Some(active) = self.active.as_mut() {
            active.selection = CharSelection::new(
                selection.anchor.min(character_count),
                selection.active.min(character_count),
            );
        }
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
        if let Some(active) = self.active.as_mut() {
            active.request_focus = false;
        }
    }

    #[cfg(feature = "screenshot-qa")]
    pub(crate) fn suppress_capture_focus(&self, context: &egui::Context) {
        if let Some(active) = self.active.as_ref() {
            context.memory_mut(|memory| memory.surrender_focus(active.editor_id()));
        }
    }

    pub const fn is_editing(&self) -> bool {
        self.active.is_some()
    }

    pub fn source_selection(&self) -> Option<Selection> {
        let active = self.active.as_ref()?;
        let anchor = active
            .source_range
            .start
            .checked_add(char_index_to_byte(&active.draft, active.selection.anchor))?;
        let caret = active
            .source_range
            .start
            .checked_add(char_index_to_byte(&active.draft, active.selection.active))?;
        Some(Selection::new(anchor, caret))
    }

    pub fn restore_source_selection(&mut self, source: &str, selection: Selection) -> bool {
        let ordered = selection.ordered_range();
        let Some(range) = markdown_block_ranges(source).into_iter().find(|range| {
            range.start <= ordered.start()
                && ordered.end() <= range.end
                && source.is_char_boundary(selection.anchor())
                && source.is_char_boundary(selection.active())
        }) else {
            return false;
        };
        let Some(block) = source.get(range.clone()) else {
            return false;
        };
        let anchor_byte = selection.anchor() - range.start;
        let active_byte = selection.active() - range.start;
        let anchor = block[..anchor_byte].chars().count();
        let active = block[..active_byte].chars().count();
        self.activate(range, block.to_owned());
        if let Some(block) = self.active.as_mut() {
            block.selection = CharSelection::new(anchor, active);
        }
        true
    }

    pub fn show(&mut self, ui: &mut egui::Ui, source: &mut String) -> MarkdownShowOutcome {
        let mut changed_origin = self.sync_pending_command(source);
        if let Some(origin) = changed_origin
            && let Some(limit) = markdown_projection_limit(source)
        {
            return MarkdownShowOutcome::ProjectionLimitExceeded { limit, origin };
        }
        let ranges = markdown_block_ranges(source);

        if ranges.is_empty() {
            if self.active.is_none() {
                self.activate(0..0, String::new());
            }
            if self.show_active_editor(ui)
                && let Some(origin) = self.sync_pending_command(source)
            {
                changed_origin.get_or_insert(origin);
                if let Some(limit) = markdown_projection_limit(source) {
                    return MarkdownShowOutcome::ProjectionLimitExceeded { limit, origin };
                }
            }
            self.finish_active_if_requested();
            return changed_origin
                .map_or(MarkdownShowOutcome::Unchanged, MarkdownShowOutcome::Changed);
        }

        let active_range = self
            .active
            .as_ref()
            .map(|active| active.source_range.clone());
        let mut active_shown = false;
        let mut pending_activation = None;

        for range in ranges {
            let overlaps_active = active_range
                .as_ref()
                .is_some_and(|active| ranges_overlap(active, &range));
            if overlaps_active {
                if !active_shown {
                    let _ = self.show_active_editor(ui);
                    active_shown = true;
                }
                continue;
            }
            if let Some(activation) = self.show_rendered_block(ui, source, range) {
                // The newly activated range was rendered as formatted content
                // this pass and becomes its TextEdit on the next pass.
                pending_activation = Some(activation);
            }
        }
        if ui.input(|input| input.pointer.any_released()) {
            self.rendered_drag = None;
        }

        if self.active.is_some() && !active_shown && pending_activation.is_none() {
            self.active = None;
        }
        let pending_replacement = self.active.as_ref().and_then(|active| {
            active
                .dirty
                .then(|| (active.source_range.clone(), active.draft.len()))
        });
        let synchronized = self.sync_pending_command(source);
        if let Some(origin) = synchronized {
            changed_origin.get_or_insert(origin);
            if let Some(limit) = markdown_projection_limit(source) {
                return MarkdownShowOutcome::ProjectionLimitExceeded { limit, origin };
            }
        }
        if let Some(activation) = pending_activation {
            let source_range = if synchronized.is_some() {
                pending_replacement.map_or_else(
                    || activation.source_range.clone(),
                    |(replaced, replacement_len)| {
                        remap_disjoint_range(
                            activation.source_range.clone(),
                            &replaced,
                            replacement_len,
                        )
                    },
                )
            } else {
                activation.source_range
            };
            if let Some(block) = source.get(source_range.clone()) {
                self.activate_with_selection(source_range, block.to_owned(), activation.selection);
            }
        }
        self.finish_active_if_requested();
        changed_origin.map_or(MarkdownShowOutcome::Unchanged, MarkdownShowOutcome::Changed)
    }

    fn finish_active_if_requested(&mut self) {
        if self.finish_requested {
            self.active = None;
            self.rendered_drag = None;
            self.finish_requested = false;
        }
    }

    fn sync_pending_command(&mut self, source: &mut String) -> Option<EditOrigin> {
        let active = self.active.as_mut().filter(|active| active.dirty)?;
        if active.source_range.end > source.len()
            || !source.is_char_boundary(active.source_range.start)
            || !source.is_char_boundary(active.source_range.end)
        {
            self.active = None;
            return None;
        }
        source.replace_range(active.source_range.clone(), &active.draft);
        active.source_range.end = active.source_range.start + active.draft.len();
        active.dirty = false;
        Some(
            active
                .pending_origin
                .take()
                .unwrap_or(EditOrigin::MarkdownInput),
        )
    }

    fn show_rendered_block(
        &mut self,
        ui: &mut egui::Ui,
        source: &str,
        range: Range<usize>,
    ) -> Option<RenderedActivation> {
        let block = &source[range.clone()];
        let frame = egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(8, 2))
            .corner_radius(4);
        let label = frame
            .show(ui, |ui| {
                let projection = if is_reference_definition(block) {
                    reference_definition_projection(block, ui.style())
                } else {
                    markdown_render_projection(block, ui.style())
                };
                if is_block_quote(block) {
                    let label = ui
                        .horizontal_top(|ui| {
                            ui.add_space(12.0);
                            show_rendered_projection(ui, projection)
                        })
                        .inner;
                    ui.painter().vline(
                        label.response.rect.left() - 8.0,
                        label.response.rect.y_range(),
                        egui::Stroke::new(2.0, ui.visuals().weak_text_color()),
                    );
                    label
                } else {
                    show_rendered_projection(ui, projection)
                }
            })
            .inner;

        label
            .response
            .clone()
            .on_hover_cursor(egui::CursorIcon::Text)
            .on_hover_text("Drag to select or click to edit this formatted content");

        if label.response.drag_started_by(egui::PointerButton::Primary)
            && let Some(position) = ui.input(|input| input.pointer.press_origin())
        {
            self.rendered_drag = Some(RenderedDragSelection {
                widget_id: label.response.id,
                anchor: label.cursor_at(position),
            });
        }
        let rendered_selection = if label.response.drag_stopped_by(egui::PointerButton::Primary) {
            let drag = self.rendered_drag.take();
            let position = ui.input(|input| input.pointer.latest_pos());
            drag.filter(|drag| drag.widget_id == label.response.id)
                .zip(position)
                .map(|(drag, position)| CharSelection::new(drag.anchor, label.cursor_at(position)))
        } else if label.response.clicked_by(egui::PointerButton::Primary) {
            label
                .response
                .interact_pointer_pos()
                .map(|position| CharSelection::caret(label.cursor_at(position)))
        } else {
            None
        };
        let activation = rendered_selection.map(|selection| {
            let source_selection = label.source_map.source_selection(selection);
            RenderedActivation {
                source_range: range,
                selection: source_selection,
            }
        });

        ui.add_space(2.0);
        activation
    }

    fn show_active_editor(&mut self, ui: &mut egui::Ui) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };
        let rows = logical_lines(&active.draft).count().max(1) + 1;
        let editor_id = active.editor_id();
        let selection = active.selection.ordered_range();
        let mut state = egui::TextEdit::load_state(ui.ctx(), editor_id).unwrap_or_default();
        let restore_focus = active.request_focus;
        if restore_focus {
            let anchor = egui::text::CCursor::new(active.selection.anchor);
            let caret = egui::text::CCursor::new(active.selection.active);
            state
                .cursor
                .set_char_range(Some(egui::text::CCursorRange::two(anchor, caret)));
            ui.memory_mut(|memory| memory.request_focus(editor_id));
            active.request_focus = false;
        }
        state.clear_undoer();
        state.store(ui.ctx(), editor_id);

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
        if restore_focus {
            // Toolbar and mode clicks can surrender the focus requested before
            // TextEdit processes input. Reassert it after that pointer pass.
            output.response.request_focus();
        }
        if let Some(cursor_range) = output.cursor_range {
            active.selection = CharSelection::new(
                cursor_range.secondary.index.into(),
                cursor_range.primary.index.into(),
            );
        }

        let changed = output.response.changed();
        if changed {
            active.dirty = true;
            active
                .pending_origin
                .get_or_insert(EditOrigin::MarkdownInput);
        }
        // Shared document history owns Undo and Redo. Discard egui's whole-string
        // snapshots so an active block cannot retain an independent history.
        output.state.clear_undoer();
        output.state.store(ui.ctx(), output.response.id);
        changed
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct MarkdownSourceStyle {
    flags: u8,
    heading_level: u8,
}

struct SynthesizedTextSpan {
    source_bytes: Range<usize>,
    rendered: String,
}

struct MarkdownSourceAnalysis {
    styles: Vec<MarkdownSourceStyle>,
    synthesized_text: Vec<SynthesizedTextSpan>,
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

#[cfg(test)]
fn markdown_render_layout(source: &str, style: &egui::Style) -> egui::text::LayoutJob {
    markdown_render_projection(source, style).job
}

fn markdown_render_projection(source: &str, style: &egui::Style) -> MarkdownRenderProjection {
    let analysis = markdown_source_analysis(source);
    let source_styles = analysis.styles;
    let mut job = egui::text::LayoutJob::default();
    let mut run = String::new();
    let mut run_style = None;
    let mut suppress_quote_space = false;
    let mut source_span_for_rendered_character = Vec::new();

    for (source_character, (index, character)) in source.char_indices().enumerate() {
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
        if run_style.is_some_and(|current| current != source_style) {
            append_render_run(&mut job, &mut run, run_style.take(), style);
        }
        run_style = Some(source_style);
        run.push(formatted_block_marker(source, index, character));
        source_span_for_rendered_character.push(source_character..source_character + 1);
    }
    append_render_run(&mut job, &mut run, run_style, style);
    extend_synthesized_source_spans(
        source,
        &mut source_span_for_rendered_character,
        &analysis.synthesized_text,
    );
    debug_assert_eq!(
        job.text.chars().count(),
        source_span_for_rendered_character.len()
    );
    MarkdownRenderProjection {
        job,
        source_map: RenderedSourceMap {
            source_span_for_rendered_character,
        },
    }
}

fn reference_definition_projection(source: &str, style: &egui::Style) -> MarkdownRenderProjection {
    let mut job = egui::text::LayoutJob::default();
    let format = egui::TextFormat {
        font_id: style.text_styles[&egui::TextStyle::Monospace].clone(),
        color: style.visuals.text_color(),
        ..Default::default()
    };
    job.append(source, 0.0, format);
    MarkdownRenderProjection {
        job,
        source_map: RenderedSourceMap {
            source_span_for_rendered_character: (0..source.chars().count())
                .map(|character| character..character + 1)
                .collect(),
        },
    }
}

fn extend_synthesized_source_spans(
    source: &str,
    spans: &mut [Range<usize>],
    synthesized_text: &[SynthesizedTextSpan],
) {
    let characters = source.chars().collect::<Vec<_>>();
    let source_boundaries = source
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(source.len()))
        .collect::<Vec<_>>();
    let mut visible = vec![false; characters.len()];
    for span in spans.iter() {
        if let Some(is_visible) = visible.get_mut(span.start) {
            *is_visible = true;
        }
    }

    for synthesized in synthesized_text {
        let (Ok(source_start), Ok(source_end)) = (
            source_boundaries.binary_search(&synthesized.source_bytes.start),
            source_boundaries.binary_search(&synthesized.source_bytes.end),
        ) else {
            continue;
        };
        let rendered_start = spans.partition_point(|span| span.start < source_start);
        let rendered_end = spans.partition_point(|span| span.start < source_end);
        let projected = spans[rendered_start..rendered_end]
            .iter()
            .filter_map(|span| characters.get(span.start))
            .collect::<String>();
        if projected == synthesized.rendered.as_str() {
            for span in &mut spans[rendered_start..rendered_end] {
                *span = source_start..source_end;
            }
        }
    }

    for span in spans {
        if span.start > 0
            && characters.get(span.start - 1) == Some(&'\\')
            && !visible[span.start - 1]
        {
            span.start -= 1;
        }
    }
}

fn show_rendered_projection(
    ui: &mut egui::Ui,
    projection: MarkdownRenderProjection,
) -> RenderedBlockLabel {
    let MarkdownRenderProjection {
        mut job,
        source_map,
    } = projection;
    job.wrap.max_width = ui.available_width();
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let response = ui.add(
        egui::Label::new(galley.clone())
            .selectable(true)
            .sense(egui::Sense::click_and_drag()),
    );
    RenderedBlockLabel {
        response,
        galley,
        source_map,
    }
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
    markdown_source_analysis(source).styles
}

fn markdown_source_analysis(source: &str) -> MarkdownSourceAnalysis {
    let mut styles = vec![MarkdownSourceStyle::default(); source.len()];
    let mut synthesized_text = Vec::new();
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
                if source
                    .get(range.clone())
                    .is_some_and(|raw| raw != text.as_ref())
                {
                    synthesized_text.push(SynthesizedTextSpan {
                        source_bytes: range.clone(),
                        rendered: text.to_string(),
                    });
                }
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
    apply_recoverable_emphasis_styles(source, &mut styles);
    MarkdownSourceAnalysis {
        styles,
        synthesized_text,
    }
}

fn apply_recoverable_emphasis_styles(source: &str, styles: &mut [MarkdownSourceStyle]) {
    for span in recoverable_emphasis_spans(source) {
        let untouched = span
            .opening()
            .clone()
            .chain(span.content().clone())
            .chain(span.closing().clone())
            .all(|index| {
                styles
                    .get(index)
                    .is_some_and(|style| *style == MarkdownSourceStyle::default())
            });
        if !untouched {
            continue;
        }
        let mut content_style = MarkdownSourceStyle::default();
        content_style.add(if span.is_strong() {
            STYLE_STRONG
        } else {
            STYLE_EMPHASIS
        });
        set_source_style(styles, span.opening().clone(), hidden_source_style());
        set_source_style(styles, span.content().clone(), content_style);
        set_source_style(styles, span.closing().clone(), hidden_source_style());
    }
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
    char_index_to_byte(source, range.start)..char_index_to_byte(source, range.end)
}

fn char_index_to_byte(source: &str, char_index: usize) -> usize {
    source
        .char_indices()
        .nth(char_index)
        .map_or(source.len(), |(index, _)| index)
}

fn remap_disjoint_range(
    range: Range<usize>,
    replaced: &Range<usize>,
    replacement_len: usize,
) -> Range<usize> {
    if range.end <= replaced.start {
        return range;
    }
    if replaced.end <= range.start {
        let removed_len = replaced.end - replaced.start;
        if replacement_len >= removed_len {
            let shift = replacement_len - removed_len;
            return (range.start + shift)..(range.end + shift);
        }
        let shift = removed_len - replacement_len;
        return (range.start - shift)..(range.end - shift);
    }

    debug_assert!(
        false,
        "rendered activation must not overlap the active block"
    );
    range
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_text_rect(shape: &egui::Shape) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) => Some(text.visual_bounding_rect()),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(first_text_rect),
            _ => None,
        }
    }

    fn text_rect(shape: &egui::Shape, expected: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Text(text) if text.galley.job.text == expected => {
                Some(text.visual_bounding_rect())
            }
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| text_rect(shape, expected)),
            _ => None,
        }
    }

    fn append_primary_click(input: &mut egui::RawInput, position: egui::Pos2) {
        input.events.push(egui::Event::PointerMoved(position));
        for pressed in [true, false] {
            input.events.push(egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
    }

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
    fn reference_definitions_use_explicit_source_typography_and_color() {
        let mut style = egui::Style::default();
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::new(17.0, egui::FontFamily::Monospace),
        );
        style.visuals.override_text_color = Some(egui::Color32::from_rgb(23, 47, 89));
        let source = "[Noter]: https://github.com/blisspixel/noter";

        let projection = reference_definition_projection(source, &style);
        let format = projection.job.format_at_byte(egui::text::ByteIndex(0));

        assert_eq!(projection.job.text, source);
        assert_eq!(
            format.font_id,
            style.text_styles[&egui::TextStyle::Monospace]
        );
        assert_eq!(format.color, style.visuals.text_color());
        assert_eq!(
            projection
                .source_map
                .source_selection(CharSelection::new(0, source.chars().count())),
            CharSelection::new(0, source.chars().count())
        );
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
    fn clicking_formatted_content_keeps_its_editor_active() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "Select this text".to_owned();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert!(!editor.show(ui, &mut source).changed());
        });
        let position = output
            .shapes
            .iter()
            .find_map(|shape| first_text_rect(&shape.shape))
            .expect("formatted content should emit a text shape")
            .center();

        let mut press = egui::RawInput::default();
        press.events.push(egui::Event::PointerMoved(position));
        press.events.push(egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = context.run_ui(press, |ui| {
            ui.set_width(800.0);
            let _ = editor.show(ui, &mut source);
        });

        let mut release = egui::RawInput::default();
        release.events.push(egui::Event::PointerMoved(position));
        release.events.push(egui::Event::PointerButton {
            pos: position,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = context.run_ui(release, |ui| {
            ui.set_width(800.0);
            let _ = editor.show(ui, &mut source);
        });

        assert!(editor.is_editing());
        assert_eq!(source, "Select this text");
    }

    #[test]
    fn first_drag_on_formatted_content_becomes_an_actionable_source_selection() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "Select this text directly".to_owned();
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert!(!editor.show(ui, &mut source).changed());
        });
        let rect = output
            .shapes
            .iter()
            .find_map(|shape| first_text_rect(&shape.shape))
            .expect("formatted content should emit a text shape");
        let start = egui::pos2(rect.width().mul_add(0.30, rect.left()), rect.center().y);
        let end = egui::pos2(rect.width().mul_add(0.72, rect.left()), rect.center().y);

        let mut press = egui::RawInput::default();
        press.events.push(egui::Event::PointerMoved(start));
        press.events.push(egui::Event::PointerButton {
            pos: start,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = context.run_ui(press, |ui| {
            ui.set_width(800.0);
            let _ = editor.show(ui, &mut source);
        });

        let mut drag = egui::RawInput::default();
        drag.events.push(egui::Event::PointerMoved(end));
        let _ = context.run_ui(drag, |ui| {
            ui.set_width(800.0);
            let _ = editor.show(ui, &mut source);
        });

        let mut release = egui::RawInput::default();
        release.events.push(egui::Event::PointerMoved(end));
        release.events.push(egui::Event::PointerButton {
            pos: end,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = context.run_ui(release, |ui| {
            ui.set_width(800.0);
            let _ = editor.show(ui, &mut source);
        });

        let selection = editor
            .source_selection()
            .expect("releasing the drag should activate its source range");
        assert_ne!(selection.anchor(), selection.active());
        assert!(selection.anchor() <= source.len());
        assert!(selection.active() <= source.len());

        editor.apply_command(MarkdownCommand::Bold);
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert_eq!(
                editor.show(ui, &mut source),
                MarkdownShowOutcome::Changed(EditOrigin::MarkdownFormatting)
            );
        });
        assert_eq!(source.matches("**").count(), 2);
    }

    #[test]
    fn rendered_selection_mapping_excludes_hidden_markdown_delimiters() {
        let projection = markdown_render_projection("Make **this** bold", &egui::Style::default());
        assert_eq!(projection.job.text, "Make this bold");
        assert_eq!(
            projection
                .source_map
                .source_selection(CharSelection::new(9, 5)),
            CharSelection::new(11, 7)
        );
    }

    #[test]
    fn rendered_entity_mapping_never_places_a_caret_inside_source_syntax() {
        for (source, rendered) in [
            ("&amp;", "&"),
            ("&#38;", "&"),
            ("&#x26;", "&"),
            ("&semi;", ";"),
            ("&fjlig;", "fj"),
            ("&#53;", "5"),
            ("&#x35;", "5"),
        ] {
            let projection = markdown_render_projection(source, &egui::Style::default());
            assert_eq!(projection.job.text, rendered);
            let source_end = source.chars().count();
            let rendered_end = rendered.chars().count();
            assert_eq!(
                projection
                    .source_map
                    .source_selection(CharSelection::caret(rendered_end)),
                CharSelection::caret(source_end)
            );
            for rendered_character in 0..rendered_end {
                assert_eq!(
                    projection.source_map.source_selection(CharSelection::new(
                        rendered_character,
                        rendered_character + 1,
                    )),
                    CharSelection::new(0, source_end)
                );
            }
            for rendered_cursor in 0..=rendered_end {
                let source_cursor = projection
                    .source_map
                    .source_selection(CharSelection::caret(rendered_cursor))
                    .active;
                assert!(matches!(source_cursor, 0) || source_cursor == source_end);
            }
        }

        let source = "A &semi; &fjlig; &#53; Z";
        let projection = markdown_render_projection(source, &egui::Style::default());
        assert_eq!(projection.job.text, "A ; fj 5 Z");
        for (entity, rendered) in [("&semi;", ";"), ("&fjlig;", "fj"), ("&#53;", "5")] {
            let source_byte = source.find(entity).expect("fixture entity should exist");
            let source_start = source[..source_byte].chars().count();
            let source_end = source_start + entity.chars().count();
            let rendered_byte = projection
                .job
                .text
                .find(rendered)
                .expect("rendered entity should exist");
            let rendered_start = projection.job.text[..rendered_byte].chars().count();
            let rendered_end = rendered_start + rendered.chars().count();
            assert_eq!(
                projection
                    .source_map
                    .source_selection(CharSelection::new(rendered_start, rendered_end)),
                CharSelection::new(source_start, source_end)
            );
        }
    }

    #[test]
    fn synthesized_text_without_a_source_substring_remains_visible_and_editable_as_source() {
        let source = "&copy;";
        let projection = markdown_render_projection(source, &egui::Style::default());

        assert_eq!(projection.job.text, source);
        assert_eq!(
            projection
                .source_map
                .source_selection(CharSelection::new(1, 5)),
            CharSelection::new(1, 5)
        );
    }

    #[test]
    fn formatting_a_rendered_entity_wraps_the_complete_source_entity() {
        let context = egui::Context::default();
        for (entity, rendered) in [
            ("&amp;", "&"),
            ("&semi;", ";"),
            ("&fjlig;", "fj"),
            ("&#53;", "5"),
            ("&#x35;", "5"),
        ] {
            let mut editor = MarkdownEditor::default();
            let mut source = entity.to_owned();
            let projection = markdown_render_projection(&source, &egui::Style::default());
            let selection = projection
                .source_map
                .source_selection(CharSelection::new(0, rendered.chars().count()));
            editor.activate_with_selection(0..source.len(), source.clone(), selection);
            editor.apply_command(MarkdownCommand::Bold);

            let _ = context.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(800.0);
                assert_eq!(
                    editor.show(ui, &mut source),
                    MarkdownShowOutcome::Changed(EditOrigin::MarkdownFormatting)
                );
            });

            assert_eq!(source, format!("**{entity}**"));
        }
    }

    #[test]
    fn typing_after_a_rendered_entity_inserts_after_its_complete_source() {
        let context = egui::Context::default();
        for (entity, rendered) in [
            ("&amp;", "&"),
            ("&semi;", ";"),
            ("&fjlig;", "fj"),
            ("&#53;", "5"),
            ("&#x35;", "5"),
        ] {
            let mut editor = MarkdownEditor::default();
            let mut source = entity.to_owned();
            let projection = markdown_render_projection(&source, &egui::Style::default());
            let selection = projection
                .source_map
                .source_selection(CharSelection::caret(rendered.chars().count()));
            editor.activate_with_selection(0..source.len(), source.clone(), selection);

            let _ = context.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(800.0);
                assert!(!editor.show(ui, &mut source).changed());
            });
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Text("X".to_owned()));
            let _ = context.run_ui(input, |ui| {
                ui.set_width(800.0);
                assert_eq!(
                    editor.show(ui, &mut source),
                    MarkdownShowOutcome::Changed(EditOrigin::MarkdownInput)
                );
            });

            assert_eq!(source, format!("{entity}X"));
        }
    }

    #[test]
    fn text_selection_transferred_to_markdown_can_be_made_bold() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "Make this bold".to_owned();
        let selected = Selection::new(5, 9);
        assert!(editor.restore_source_selection(&source, selected));

        editor.apply_command(MarkdownCommand::Bold);
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert_eq!(
                editor.show(ui, &mut source),
                MarkdownShowOutcome::Changed(EditOrigin::MarkdownFormatting)
            );
        });

        assert!(!output.shapes.is_empty());
        assert_eq!(source, "Make **this** bold");
        assert_eq!(editor.source_selection(), Some(Selection::new(7, 11)));
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
    fn recoverable_emphasis_spacing_projects_as_formatted_text_in_both_states() {
        let style = egui::Style::default();
        let source = "*The sum of the square root of any two sides. *";

        let rendered = markdown_render_layout(source, &style);
        let active = markdown_edit_layout(source, &style, None);

        assert_eq!(
            rendered.text,
            "The sum of the square root of any two sides. "
        );
        assert!(rendered.format_at_byte(egui::text::ByteIndex(0)).italics);
        assert_eq!(active.text, source);
        assert_eq!(
            active.format_at_byte(egui::text::ByteIndex(0)).color,
            egui::Color32::TRANSPARENT
        );
        assert!(active.format_at_byte(egui::text::ByteIndex(1)).italics);
        assert_eq!(
            active
                .format_at_byte(egui::text::ByteIndex(source.len() - 1))
                .color,
            egui::Color32::TRANSPARENT
        );
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
        assert!(!active.request_focus);

        let context = egui::Context::default();
        let editor_id = active.editor_id();
        context.memory_mut(|memory| memory.request_focus(editor_id));
        assert!(context.memory(|memory| memory.has_focus(editor_id)));
        editor.suppress_capture_focus(&context);
        assert!(!context.memory(|memory| memory.has_focus(editor_id)));
    }

    #[test]
    fn active_block_command_updates_source_and_renders_editor() {
        let context = egui::Context::default();
        let mut active = ActiveBlock::new(0..4, "text".to_owned(), 1);
        active.selection = CharSelection::new(0, 4);
        active.apply(MarkdownCommand::Bold);
        let mut editor = MarkdownEditor {
            active: Some(active),
            next_editor_serial: 1,
            rendered_drag: None,
            finish_requested: false,
        };
        let mut source = "text".to_owned();

        let mut outcome = MarkdownShowOutcome::Unchanged;
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            editor.toolbar(ui);
            outcome = editor.show(ui, &mut source);
        });

        assert_eq!(source, "**text**");
        assert_eq!(outcome.origin(), Some(EditOrigin::MarkdownFormatting));
        assert!(editor.is_editing());
        assert!(!output.shapes.is_empty());
    }

    #[test]
    fn source_selection_restoration_preserves_unicode_boundaries_and_direction() {
        let source = "# é\n\nParagraph";
        let selection = Selection::new(4, 2);
        let mut editor = MarkdownEditor::default();

        assert!(editor.restore_source_selection(source, selection));
        assert_eq!(editor.source_selection(), Some(selection));
        assert!(editor.is_editing());
    }

    #[test]
    fn source_selection_restoration_rejects_cross_block_and_invalid_utf8_ranges() {
        let source = "# First\n\nSecond é block";
        let second_start = source
            .find("Second")
            .expect("fixture contains a second block");
        let unicode_start = source
            .find('é')
            .expect("fixture contains a multibyte character");
        let unicode_end = unicode_start + 'é'.len_utf8();
        let mut editor = MarkdownEditor::default();

        let selected = Selection::new(unicode_end, unicode_start);
        assert!(editor.restore_source_selection(source, selected));
        assert_eq!(editor.source_selection(), Some(selected));

        for invalid in [
            Selection::new(0, second_start + 1),
            Selection::new(unicode_start, unicode_start + 1),
            Selection::new(unicode_start + 1, unicode_end),
        ] {
            editor.reset();
            assert!(!editor.restore_source_selection(source, invalid));
            assert!(!editor.is_editing());
        }
    }

    #[test]
    fn pending_format_command_is_bounded_before_projecting_the_changed_source() {
        let mut source = format!("{}\n", "x".repeat(1_023)).repeat(63) + &"x".repeat(1_024);
        assert_eq!(markdown_projection_limit(&source), None);
        let mut active = ActiveBlock::new(0..source.len(), source.clone(), 1);
        active.selection = CharSelection::new(0, source.len());
        active.apply(MarkdownCommand::Quote);
        let mut editor = MarkdownEditor {
            active: Some(active),
            next_editor_serial: 1,
            rendered_drag: None,
            finish_requested: false,
        };
        let context = egui::Context::default();
        let mut outcome = MarkdownShowOutcome::Unchanged;

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            outcome = editor.show(ui, &mut source);
        });

        assert_eq!(
            outcome,
            MarkdownShowOutcome::ProjectionLimitExceeded {
                limit: MarkdownProjectionLimit::BlockBytes,
                origin: EditOrigin::MarkdownFormatting,
            }
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
        active.selection = CharSelection::new(0, 3);
        let mut editor = MarkdownEditor {
            active: Some(active),
            next_editor_serial: 1,
            rendered_drag: None,
            finish_requested: false,
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
    fn same_frame_input_commits_before_a_later_rendered_block_activates() {
        let context = egui::Context::default();
        let mut active = ActiveBlock::new(0..3, "# A".to_owned(), 1);
        active.selection = CharSelection::new(0, 3);
        let mut editor = MarkdownEditor {
            active: Some(active),
            next_editor_serial: 1,
            rendered_drag: None,
            finish_requested: false,
        };
        let mut source = "# A\n\nParagraph\n".to_owned();

        let initial = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert_eq!(editor.show(ui, &mut source), MarkdownShowOutcome::Unchanged);
        });
        let paragraph = initial
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, "Paragraph"))
            .expect("the later rendered block should have a text shape")
            .center();

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Text("x".to_owned()));
        append_primary_click(&mut input, paragraph);
        let _ = context.run_ui(input, |ui| {
            ui.set_width(800.0);
            assert_eq!(
                editor.show(ui, &mut source),
                MarkdownShowOutcome::Changed(EditOrigin::MarkdownInput)
            );
        });

        assert_eq!(source, "x\n\nParagraph\n");
        let active = editor
            .active
            .as_ref()
            .expect("the clicked rendered block should become active");
        assert_eq!(active.source_range, 3..12);
        assert_eq!(active.draft, "Paragraph");
    }

    #[test]
    fn same_frame_input_commits_before_an_earlier_rendered_block_activates() {
        let context = egui::Context::default();
        let mut active = ActiveBlock::new(11..14, "# A".to_owned(), 1);
        active.selection = CharSelection::new(0, 3);
        let mut editor = MarkdownEditor {
            active: Some(active),
            next_editor_serial: 1,
            rendered_drag: None,
            finish_requested: false,
        };
        let mut source = "Paragraph\n\n# A".to_owned();

        let initial = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert_eq!(editor.show(ui, &mut source), MarkdownShowOutcome::Unchanged);
        });
        let paragraph = initial
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, "Paragraph"))
            .expect("the earlier rendered block should have a text shape")
            .center();

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Text("x".to_owned()));
        append_primary_click(&mut input, paragraph);
        let _ = context.run_ui(input, |ui| {
            ui.set_width(800.0);
            assert_eq!(
                editor.show(ui, &mut source),
                MarkdownShowOutcome::Changed(EditOrigin::MarkdownInput)
            );
        });

        assert_eq!(source, "Paragraph\n\nx");
        let active = editor
            .active
            .as_ref()
            .expect("the clicked rendered block should become active");
        assert_eq!(active.source_range, 0..9);
        assert_eq!(active.draft, "Paragraph");
    }

    #[test]
    fn same_frame_input_commits_before_done_finishes_editing() {
        let context = egui::Context::default();
        let mut active = ActiveBlock::new(0..3, "# A".to_owned(), 1);
        active.selection = CharSelection::new(0, 3);
        let mut editor = MarkdownEditor {
            active: Some(active),
            next_editor_serial: 1,
            rendered_drag: None,
            finish_requested: false,
        };
        let mut source = "# A".to_owned();

        let initial = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            editor.toolbar(ui);
            assert_eq!(editor.show(ui, &mut source), MarkdownShowOutcome::Unchanged);
        });
        let done = initial
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, "Done"))
            .expect("the expanded toolbar should have a Done button")
            .center();

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Text("x".to_owned()));
        append_primary_click(&mut input, done);
        let _ = context.run_ui(input, |ui| {
            ui.set_width(800.0);
            editor.toolbar(ui);
            assert_eq!(
                editor.show(ui, &mut source),
                MarkdownShowOutcome::Changed(EditOrigin::MarkdownInput)
            );
        });

        assert_eq!(source, "x");
        assert!(!editor.is_editing());
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

    #[test]
    fn active_editor_discards_widget_local_undo_snapshots() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        editor.activate(0..7, "bounded".to_owned());
        let editor_id = editor
            .active
            .as_ref()
            .expect("the fixture should have an active block")
            .editor_id();
        let mut source = "bounded".to_owned();

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(640.0);
            let _ = editor.show(ui, &mut source);
        });
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Text("!".to_owned()));
        let _ = context.run_ui(input, |ui| {
            ui.set_width(640.0);
            assert!(editor.show(ui, &mut source).changed());
        });
        assert_eq!(source, "bounded!");

        let state = egui::TextEdit::load_state(&context, editor_id)
            .expect("the rendered editor should persist its state");
        let cursor = state
            .cursor
            .char_range()
            .unwrap_or_else(|| egui::text::CCursorRange::one(egui::text::CCursor::new(0)));
        let current = (cursor, source);
        assert!(!state.undoer().has_undo(&current));
        assert!(!state.undoer().has_redo(&current));
    }
}
