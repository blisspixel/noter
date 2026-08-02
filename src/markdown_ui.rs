use std::ops::Range;

use eframe::egui;
use noter::core::edit::{EditOrigin, Selection};
use noter::core::line_endings::logical_lines;
use noter::core::markdown::recoverable_emphasis_spans;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag};

use crate::bounded_text_input::{BoundedTextBuffer, sanitize_bounded_text_events};

const ACTIVE_EDITOR_ID: &str = "noter-markdown-active-block";
const EXPANDED_FORMAT_MIN_WIDTH: f32 = 480.0;
const FORMAT_BUTTON_SIZE: egui::Vec2 = egui::vec2(32.0, 28.0);
const CODE_BUTTON_SIZE: egui::Vec2 = egui::vec2(38.0, 28.0);
const BLOCK_STYLE_BUTTON_WIDTH: f32 = 112.0;
const BLOCK_HORIZONTAL_PADDING: i8 = 0;
const BLOCK_VERTICAL_PADDING: i8 = 3;
const BLOCK_GAP: f32 = 3.0;
const DRAG_AUTOSCROLL_EDGE: f32 = 28.0;
const DRAG_AUTOSCROLL_MAX_SPEED: f32 = 1080.0;
const DRAG_AUTOSCROLL_MAX_FRAME_SECONDS: f32 = 0.1;
const MARKER_FONT_SIZE: f32 = 0.1;
const BODY_WEIGHT: f32 = 400.0;
const HEADING_WEIGHT: f32 = 600.0;
const STRONG_WEIGHT: f32 = 700.0;
const BODY_LINE_HEIGHT_RATIO: f32 = 4.0 / 3.0;
const HEADING_LINE_HEIGHT_RATIO: f32 = 1.2;
const HEADING_SIZE_RATIOS: [f32; 6] = [
    28.0 / 15.0,
    24.0 / 15.0,
    21.0 / 15.0,
    18.0 / 15.0,
    16.0 / 15.0,
    1.0,
];
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

fn direct_input_origin(ui: &egui::Ui) -> EditOrigin {
    ui.input(|input| {
        if input
            .events
            .iter()
            .any(|event| matches!(event, egui::Event::Paste(_)))
        {
            EditOrigin::Paste
        } else {
            EditOrigin::MarkdownInput
        }
    })
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
enum BlockStyle {
    Paragraph,
    Heading1,
    Heading2,
    Heading3,
    Heading4,
    Heading5,
    Heading6,
}

impl BlockStyle {
    const ALL: [Self; 7] = [
        Self::Paragraph,
        Self::Heading1,
        Self::Heading2,
        Self::Heading3,
        Self::Heading4,
        Self::Heading5,
        Self::Heading6,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::Paragraph => "Paragraph",
            Self::Heading1 => "Heading 1",
            Self::Heading2 => "Heading 2",
            Self::Heading3 => "Heading 3",
            Self::Heading4 => "Heading 4",
            Self::Heading5 => "Heading 5",
            Self::Heading6 => "Heading 6",
        }
    }

    const fn heading_level(self) -> Option<usize> {
        match self {
            Self::Paragraph => None,
            Self::Heading1 => Some(1),
            Self::Heading2 => Some(2),
            Self::Heading3 => Some(3),
            Self::Heading4 => Some(4),
            Self::Heading5 => Some(5),
            Self::Heading6 => Some(6),
        }
    }

    const fn from_heading_level(level: u8) -> Option<Self> {
        match level {
            1 => Some(Self::Heading1),
            2 => Some(Self::Heading2),
            3 => Some(Self::Heading3),
            4 => Some(Self::Heading4),
            5 => Some(Self::Heading5),
            6 => Some(Self::Heading6),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BlockStyleState {
    Uniform(BlockStyle),
    Mixed,
    Unavailable,
}

impl BlockStyleState {
    const fn current(self) -> Option<BlockStyle> {
        match self {
            Self::Uniform(style) => Some(style),
            Self::Mixed | Self::Unavailable => None,
        }
    }

    const fn label(self) -> &'static str {
        match self {
            Self::Uniform(style) => style.label(),
            Self::Mixed => "Mixed",
            Self::Unavailable => "Unavailable",
        }
    }

    const fn is_available(self) -> bool {
        !matches!(self, Self::Unavailable)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MarkdownCommand {
    Bold,
    Italic,
    Link,
    InlineCode,
    BulletedList,
    Quote,
}

impl MarkdownCommand {
    const WITH_SHORTCUTS: [Self; 3] = [Self::Bold, Self::Italic, Self::Link];

    const INLINE_AND_LINE: [Self; 6] = [
        Self::Bold,
        Self::Italic,
        Self::Link,
        Self::InlineCode,
        Self::BulletedList,
        Self::Quote,
    ];

    const fn description(self) -> &'static str {
        match self {
            Self::Bold => "Toggle strong emphasis on the selection",
            Self::Italic => "Toggle emphasis on the selection",
            Self::Link => "Toggle a Markdown link without inventing text or a URL",
            Self::InlineCode => "Toggle inline code on the selection",
            Self::BulletedList => "Toggle bullet markers on the active lines",
            Self::Quote => "Toggle quote markers on the active lines",
        }
    }

    const fn menu_label(self) -> &'static str {
        match self {
            Self::Bold => "Bold",
            Self::Italic => "Italic",
            Self::Link => "Link",
            Self::InlineCode => "Inline code",
            Self::BulletedList => "Bulleted list",
            Self::Quote => "Quote",
        }
    }

    fn button_text(self) -> Option<egui::RichText> {
        match self {
            Self::Bold => Some(egui::RichText::new("B").strong().size(16.0)),
            Self::InlineCode => Some(egui::RichText::new("</>").monospace().size(11.0)),
            Self::Italic | Self::Link | Self::BulletedList | Self::Quote => None,
        }
    }

    const fn button_size(self) -> egui::Vec2 {
        if matches!(self, Self::InlineCode) {
            CODE_BUTTON_SIZE
        } else {
            FORMAT_BUTTON_SIZE
        }
    }

    const fn ends_group(self) -> bool {
        matches!(self, Self::Italic | Self::InlineCode)
    }

    const fn shortcut(self) -> Option<egui::KeyboardShortcut> {
        let key = match self {
            Self::Bold => egui::Key::B,
            Self::Italic => egui::Key::I,
            Self::Link => egui::Key::K,
            Self::InlineCode | Self::BulletedList | Self::Quote => return None,
        };
        Some(egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key))
    }

    fn hover_text(self, context: &egui::Context) -> String {
        self.shortcut().map_or_else(
            || self.description().to_owned(),
            |shortcut| {
                format!(
                    "{} ({})",
                    self.description(),
                    context.format_shortcut(&shortcut)
                )
            },
        )
    }

    fn paint_icon(self, ui: &egui::Ui, response: &egui::Response, enabled: bool) {
        if !matches!(
            self,
            Self::Italic | Self::Link | Self::BulletedList | Self::Quote
        ) || !ui.is_rect_visible(response.rect)
        {
            return;
        }
        let color = if enabled {
            ui.style().interact(response).fg_stroke.color
        } else {
            ui.visuals().widgets.noninteractive.fg_stroke.color
        };
        let painter = ui.painter();
        let center = response.rect.center();
        let stroke = egui::Stroke::new(1.6, color);
        match self {
            Self::Italic => {
                painter.line_segment(
                    [
                        center + egui::vec2(-1.0, -6.0),
                        center + egui::vec2(6.0, -6.0),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        center + egui::vec2(3.0, -6.0),
                        center + egui::vec2(-3.0, 6.0),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        center + egui::vec2(-6.0, 6.0),
                        center + egui::vec2(1.0, 6.0),
                    ],
                    stroke,
                );
            }
            Self::Link => {
                let left = center + egui::vec2(-3.5, 1.5);
                let right = center + egui::vec2(3.5, -1.5);
                painter.circle_stroke(left, 4.0, stroke);
                painter.circle_stroke(right, 4.0, stroke);
                painter.line_segment(
                    [
                        center + egui::vec2(-1.5, 0.7),
                        center + egui::vec2(1.5, -0.7),
                    ],
                    stroke,
                );
            }
            Self::BulletedList => {
                for offset in [-5.0, 0.0, 5.0] {
                    painter.circle_filled(center + egui::vec2(-6.0, offset), 1.4, color);
                    painter.line_segment(
                        [
                            center + egui::vec2(-2.0, offset),
                            center + egui::vec2(7.0, offset),
                        ],
                        stroke,
                    );
                }
            }
            Self::Quote => {
                for offset in [-3.5, 3.5] {
                    let mark = center + egui::vec2(offset, -1.5);
                    painter.circle_filled(mark, 2.1, color);
                    painter.line_segment(
                        [mark + egui::vec2(0.0, 1.0), mark + egui::vec2(-1.5, 5.0)],
                        stroke,
                    );
                }
            }
            Self::Bold | Self::InlineCode => {}
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
        self.selection = self.selection.with_ordered_range(result.selection);
        self.dirty = true;
        self.pending_origin = Some(EditOrigin::MarkdownFormatting);
        self.request_focus = true;
    }

    fn apply_block_style(&mut self, style: BlockStyle) {
        let result = apply_block_style(&self.draft, self.selection.ordered_range(), style);
        let changed = result.text != self.draft;
        self.draft = result.text;
        self.selection = self.selection.with_ordered_range(result.selection);
        if changed {
            self.dirty = true;
            self.pending_origin = Some(EditOrigin::MarkdownFormatting);
        }
        self.request_focus = true;
    }

    fn command_is_active(&self, command: MarkdownCommand) -> bool {
        markdown_command_is_active(&self.draft, self.selection.ordered_range(), command)
    }

    fn block_style(&self) -> BlockStyleState {
        selected_block_style(&self.draft, self.selection.ordered_range())
    }

    fn source_selection(&self) -> Option<Selection> {
        let anchor = self
            .source_range
            .start
            .checked_add(char_index_to_byte(&self.draft, self.selection.anchor))?;
        let active = self
            .source_range
            .start
            .checked_add(char_index_to_byte(&self.draft, self.selection.active))?;
        Some(Selection::new(anchor, active))
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

    const fn with_ordered_range(self, range: Range<usize>) -> Self {
        if self.anchor <= self.active {
            Self::new(range.start, range.end)
        } else {
            Self::new(range.end, range.start)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RenderedDragSelection {
    anchor: RenderedSourceCursor,
    active: RenderedSourceCursor,
}

impl RenderedDragSelection {
    const fn new(anchor: RenderedSourceCursor) -> Self {
        Self {
            anchor,
            active: anchor,
        }
    }

    fn source_selection(self) -> Selection {
        match self.anchor.order_key().cmp(&self.active.order_key()) {
            std::cmp::Ordering::Less => {
                Selection::new(self.anchor.selection_start, self.active.selection_end)
            }
            std::cmp::Ordering::Equal => Selection::caret(self.anchor.selection_start),
            std::cmp::Ordering::Greater => {
                Selection::new(self.anchor.selection_end, self.active.selection_start)
            }
        }
    }

    fn remap_after_replacement(
        self,
        replaced: &Range<usize>,
        replacement_len: usize,
    ) -> Option<Self> {
        Some(Self {
            anchor: self
                .anchor
                .remap_after_replacement(replaced, replacement_len)?,
            active: self
                .active
                .remap_after_replacement(replaced, replacement_len)?,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RenderedSourceCursor {
    block_start: usize,
    rendered_index: usize,
    selection_start: usize,
    selection_end: usize,
}

impl RenderedSourceCursor {
    const fn order_key(self) -> (usize, usize) {
        (self.block_start, self.rendered_index)
    }

    fn remap_after_replacement(
        self,
        replaced: &Range<usize>,
        replacement_len: usize,
    ) -> Option<Self> {
        Some(Self {
            block_start: remap_disjoint_position(self.block_start, replaced, replacement_len)?,
            rendered_index: self.rendered_index,
            selection_start: remap_disjoint_position(
                self.selection_start,
                replaced,
                replacement_len,
            )?,
            selection_end: remap_disjoint_position(self.selection_end, replaced, replacement_len)?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
struct RenderedPointerTarget {
    cursor: RenderedSourceCursor,
    vertical_distance: f32,
}

impl RenderedPointerTarget {
    fn replace_if_nearer(self, target: &mut Option<Self>) {
        if target.as_ref().is_none_or(|current| {
            self.vertical_distance
                .total_cmp(&current.vertical_distance)
                .is_lt()
        }) {
            *target = Some(self);
        }
    }
}

struct RenderedActivation {
    source_selection: Selection,
}

impl RenderedActivation {
    fn remap_after_replacement(
        self,
        replaced: &Range<usize>,
        replacement_len: usize,
    ) -> Option<Self> {
        Some(Self {
            source_selection: remap_disjoint_selection(
                self.source_selection,
                replaced,
                replacement_len,
            )?,
        })
    }
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

    fn source_cursor_at(
        &self,
        position: egui::Pos2,
        source_range: &Range<usize>,
        source: &str,
    ) -> Option<RenderedSourceCursor> {
        let rendered_count = self.source_map.source_span_for_rendered_character.len();
        let rendered_index = if position.y < self.response.rect.top() {
            0
        } else if self.response.rect.bottom() < position.y {
            rendered_count
        } else {
            let clamped = egui::pos2(
                position
                    .x
                    .clamp(self.response.rect.left(), self.response.rect.right()),
                position
                    .y
                    .clamp(self.response.rect.top(), self.response.rect.bottom()),
            );
            self.cursor_at(clamped).min(rendered_count)
        };
        let source_character_count = source.chars().count();
        let selection_start_index = self.source_map.start_boundary(rendered_index);
        let selection_end_index = self.source_map.end_boundary(rendered_index);
        if source_character_count < selection_start_index
            || source_character_count < selection_end_index
        {
            return None;
        }
        let selection_start = source_range
            .start
            .checked_add(char_index_to_byte(source, selection_start_index))?;
        let selection_end = source_range
            .start
            .checked_add(char_index_to_byte(source, selection_end_index))?;
        if source_range.end < selection_start || source_range.end < selection_end {
            return None;
        }
        Some(RenderedSourceCursor {
            block_start: source_range.start,
            rendered_index,
            selection_start,
            selection_end,
        })
    }

    fn pointer_target(
        &self,
        position: egui::Pos2,
        source_range: &Range<usize>,
        source: &str,
    ) -> Option<RenderedPointerTarget> {
        let vertical_distance = if position.y < self.response.rect.top() {
            self.response.rect.top() - position.y
        } else if self.response.rect.bottom() < position.y {
            position.y - self.response.rect.bottom()
        } else {
            0.0
        };
        Some(RenderedPointerTarget {
            cursor: self.source_cursor_at(position, source_range, source)?,
            vertical_distance,
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ToolbarAction {
    BlockStyle(BlockStyle),
    Command(MarkdownCommand),
}

#[derive(Default)]
pub struct MarkdownEditor {
    active: Option<ActiveBlock>,
    finished_selection: Option<Selection>,
    next_editor_serial: u64,
    rendered_drag: Option<RenderedDragSelection>,
    input_was_limited: bool,
}

impl MarkdownEditor {
    pub fn reset(&mut self) {
        self.active = None;
        self.finished_selection = None;
        self.rendered_drag = None;
        self.input_was_limited = false;
        self.next_editor_serial = self.next_editor_serial.wrapping_add(1);
    }

    pub fn take_input_was_limited(&mut self) -> bool {
        std::mem::take(&mut self.input_was_limited)
    }

    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        if !expanded_toolbar_fits(ui.available_width()) {
            let action = self.compact_toolbar(ui);
            self.apply_toolbar_action(action);
            return;
        }

        let enabled = self.active.is_some();
        let mut requested_action = self
            .block_style_selector(ui, enabled)
            .map(ToolbarAction::BlockStyle);
        ui.add_space(8.0);
        for command in MarkdownCommand::INLINE_AND_LINE {
            let selected = self
                .active
                .as_ref()
                .is_some_and(|active| active.command_is_active(command));
            let button = command
                .button_text()
                .map_or_else(|| egui::Button::new(""), egui::Button::new)
                .selected(selected);
            let response = ui
                .add_enabled(enabled, button.min_size(command.button_size()))
                .on_hover_text(if enabled {
                    command.hover_text(ui.ctx())
                } else {
                    "Click or drag in formatted text to activate formatting".to_owned()
                });
            response.widget_info(|| {
                egui::WidgetInfo::selected(
                    egui::WidgetType::Button,
                    enabled,
                    selected,
                    command.menu_label(),
                )
            });
            command.paint_icon(ui, &response, enabled);
            if response.clicked() {
                requested_action.get_or_insert(ToolbarAction::Command(command));
            }
            if command.ends_group() {
                ui.add_space(8.0);
            }
        }
        self.apply_toolbar_action(requested_action);
    }

    fn block_style_selector(&self, ui: &mut egui::Ui, enabled: bool) -> Option<BlockStyle> {
        let state = self.active.as_ref().map(ActiveBlock::block_style);
        let style_enabled = state.is_some_and(BlockStyleState::is_available);
        let current = state.and_then(BlockStyleState::current);
        let visible_label = state.map_or("Style", BlockStyleState::label);
        let mut requested = None;
        let response = ui
            .add_enabled_ui(style_enabled, |ui| {
                egui::ComboBox::from_id_salt("markdown-block-style")
                    .width(BLOCK_STYLE_BUTTON_WIDTH)
                    .selected_text(visible_label)
                    .show_ui(ui, |ui| {
                        ui.set_min_width(BLOCK_STYLE_BUTTON_WIDTH);
                        requested = show_block_style_options(ui, current);
                    })
                    .response
            })
            .inner;
        response.widget_info(|| {
            let mut info = egui::WidgetInfo::labeled(
                egui::WidgetType::ComboBox,
                style_enabled,
                "Paragraph style",
            );
            info.current_text_value = Some(visible_label.to_owned());
            info
        });
        if style_enabled {
            response.on_hover_text("Set paragraph style");
        } else if enabled {
            response.on_disabled_hover_text(
                "Paragraph styles are unavailable for this Markdown structure",
            );
        } else {
            response
                .on_disabled_hover_text("Click or drag in formatted text to activate formatting");
        }
        requested
    }

    fn compact_toolbar(&self, ui: &mut egui::Ui) -> Option<ToolbarAction> {
        let enabled = self.active.is_some();
        let style_state = self.active.as_ref().map(ActiveBlock::block_style);
        let current_style = style_state.and_then(BlockStyleState::current);
        let style_enabled = style_state.is_some_and(BlockStyleState::is_available);
        let style_value = style_state.map_or("Style", BlockStyleState::label);
        let style_label = format!("Paragraph style: {style_value}");
        let mut requested_style = None;
        let mut requested_command = None;
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                ui.menu_button("Format", |ui| {
                    let style_response = ui
                        .add_enabled_ui(style_enabled, |ui| {
                            ui.menu_button(style_label, |ui| {
                                requested_style = show_block_style_options(ui, current_style);
                            })
                            .response
                        })
                        .inner;
                    style_response.widget_info(|| {
                        let mut info = egui::WidgetInfo::labeled(
                            egui::WidgetType::ComboBox,
                            style_enabled,
                            "Paragraph style",
                        );
                        info.current_text_value = Some(style_value.to_owned());
                        info
                    });
                    ui.separator();
                    for command in MarkdownCommand::INLINE_AND_LINE {
                        let selected = self
                            .active
                            .as_ref()
                            .is_some_and(|active| active.command_is_active(command));
                        let mut button = egui::Button::selectable(selected, command.menu_label());
                        if let Some(shortcut) = command.shortcut() {
                            button = button.shortcut_text(ui.ctx().format_shortcut(&shortcut));
                        }
                        if ui
                            .add(button)
                            .on_hover_text(command.description())
                            .clicked()
                        {
                            requested_command = Some(command);
                            ui.close();
                        }
                    }
                });
            })
            .response;
        if !enabled {
            response
                .on_disabled_hover_text("Click or drag in formatted text to activate formatting");
        }
        requested_style
            .map(ToolbarAction::BlockStyle)
            .or_else(|| requested_command.map(ToolbarAction::Command))
    }

    fn apply_toolbar_action(&mut self, action: Option<ToolbarAction>) {
        let Some(active) = self.active.as_mut() else {
            return;
        };
        let Some(action) = action else {
            return;
        };
        match action {
            ToolbarAction::BlockStyle(style) => active.apply_block_style(style),
            ToolbarAction::Command(command) => active.apply(command),
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
        self.active
            .as_ref()
            .and_then(ActiveBlock::source_selection)
            .or(self.finished_selection)
    }

    pub fn can_restore_source_selection(source: &str, selection: Selection) -> bool {
        restorable_source_edit_range(source, selection).is_some()
    }

    /// Restores a source-backed selection and optionally focuses its editor.
    ///
    /// Non-modal controls use `request_focus = false` so their keyboard input
    /// remains active while the selected document match stays visible.
    pub fn restore_source_selection_with_focus(
        &mut self,
        source: &str,
        selection: Selection,
        request_focus: bool,
    ) -> bool {
        let Some((range, char_selection)) = source_edit_activation(source, selection) else {
            return false;
        };
        let block = &source[range.clone()];
        self.activate(range, block.to_owned());
        if let Some(block) = self.active.as_mut() {
            block.selection = char_selection;
            block.request_focus = request_focus;
        }
        true
    }

    pub fn show(&mut self, ui: &mut egui::Ui, source: &mut String) -> MarkdownShowOutcome {
        self.show_with_source_byte_limit(ui, source, PROTOTYPE_MARKDOWN_MAX_BYTES)
    }

    fn show_with_source_byte_limit(
        &mut self,
        ui: &mut egui::Ui,
        source: &mut String,
        maximum_source_bytes: usize,
    ) -> MarkdownShowOutcome {
        self.input_was_limited = false;
        self.finished_selection = None;
        let mut finish_after_frame = false;
        let mut changed_origin = self.sync_pending_command(source);
        if let Some(origin) = changed_origin
            && let Some(limit) = markdown_projection_limit(source)
        {
            return MarkdownShowOutcome::ProjectionLimitExceeded { limit, origin };
        }
        if !self.cancel_rendered_drag_on_escape_or_input_loss(ui) && self.rendered_drag.is_some() {
            self.retire_active();
        }
        let ranges = markdown_block_ranges(source);

        if ranges.is_empty() {
            if self.active.is_none() {
                self.activate(0..0, String::new());
            }
            let maximum_draft_bytes = self.maximum_active_draft_bytes(source, maximum_source_bytes);
            let (active_changed, finish_requested, active_draft_limit) =
                self.show_active_editor(ui, maximum_draft_bytes);
            finish_after_frame |= finish_requested;
            if active_changed && let Some(origin) = self.sync_pending_command(source) {
                changed_origin.get_or_insert(origin);
                if let Some(limit) =
                    active_draft_limit.or_else(|| markdown_projection_limit(source))
                {
                    return MarkdownShowOutcome::ProjectionLimitExceeded { limit, origin };
                }
            }
            ui.add_space(BLOCK_GAP);
            self.finish_if_requested(finish_after_frame);
            return changed_origin
                .map_or(MarkdownShowOutcome::Unchanged, MarkdownShowOutcome::Changed);
        }

        let active_range = self
            .active
            .as_ref()
            .map(|active| active.source_range.clone());
        let mut active_shown = false;
        let mut pending_activation = None;
        let mut active_draft_limit = None;
        let mut pointer_target = None;

        for range in ranges {
            let overlaps_active = active_range
                .as_ref()
                .is_some_and(|active| ranges_overlap(active, &range));
            if overlaps_active {
                if !active_shown {
                    let maximum_draft_bytes =
                        self.maximum_active_draft_bytes(source, maximum_source_bytes);
                    let (_, finish_requested, projection_limit) =
                        self.show_active_editor(ui, maximum_draft_bytes);
                    finish_after_frame |= finish_requested;
                    active_draft_limit = active_draft_limit.or(projection_limit);
                    ui.add_space(BLOCK_GAP);
                    active_shown = true;
                }
                continue;
            }
            if let Some(activation) =
                self.show_rendered_block(ui, source, range, &mut pointer_target)
            {
                // The newly activated range was rendered as formatted content
                // this pass and becomes its TextEdit on the next pass.
                pending_activation = Some(activation);
            }
        }

        self.update_rendered_drag(ui, pointer_target, &mut pending_activation);

        if self.active.is_some() && !active_shown && pending_activation.is_none() {
            self.active = None;
        }
        let pending_replacement = self.active.as_ref().and_then(|active| {
            active
                .dirty
                .then(|| (active.source_range.clone(), active.draft.len()))
        });
        let synchronized = self.sync_pending_command(source);
        if synchronized.is_some()
            && let Some((replaced, replacement_len)) = pending_replacement.as_ref()
        {
            self.remap_rendered_interaction(
                ui,
                &mut pending_activation,
                replaced,
                *replacement_len,
            );
        }
        if let Some(origin) = synchronized {
            changed_origin.get_or_insert(origin);
            if let Some(limit) = active_draft_limit.or_else(|| markdown_projection_limit(source)) {
                return MarkdownShowOutcome::ProjectionLimitExceeded { limit, origin };
            }
        }
        self.apply_rendered_activation(source, pending_activation);
        self.finish_if_requested(finish_after_frame);
        changed_origin.map_or(MarkdownShowOutcome::Unchanged, MarkdownShowOutcome::Changed)
    }

    fn update_rendered_drag(
        &mut self,
        ui: &egui::Ui,
        pointer_target: Option<RenderedPointerTarget>,
        pending_activation: &mut Option<RenderedActivation>,
    ) {
        if self.cancel_rendered_drag_on_escape_or_input_loss(ui) {
            return;
        }
        if let Some(drag) = self.rendered_drag.as_mut() {
            if let Some(target) = pointer_target {
                drag.active = target.cursor;
            }
            self.finished_selection = Some(drag.source_selection());
        }
        let (primary_down, primary_released) = ui.input(|input| {
            (
                input.pointer.button_down(egui::PointerButton::Primary),
                input.pointer.button_released(egui::PointerButton::Primary),
            )
        });
        if primary_down
            && self.rendered_drag.is_some()
            && let Some(position) = ui.input(|input| input.pointer.latest_pos())
        {
            let frame_seconds = ui.input(|input| input.stable_dt);
            let delta = rendered_drag_scroll_delta(position.y, ui.clip_rect(), frame_seconds);
            if delta != 0.0 {
                ui.scroll_with_delta(egui::vec2(0.0, delta));
                ui.ctx().request_repaint();
            }
        }
        if primary_released {
            if let Some(drag) = self.rendered_drag.take() {
                let source_selection = drag.source_selection();
                self.finished_selection = Some(source_selection);
                *pending_activation = Some(RenderedActivation { source_selection });
                clear_label_selection(ui);
            }
        } else if !primary_down && self.rendered_drag.is_some() {
            self.rendered_drag = None;
            self.finished_selection = None;
            clear_label_selection(ui);
        }
    }

    fn cancel_rendered_drag_on_escape_or_input_loss(&mut self, ui: &egui::Ui) -> bool {
        if self.rendered_drag.is_none() {
            return false;
        }
        let (escape_pressed, interaction_lost) = ui.input(|input| {
            let pointer_lost_without_release = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::PointerGone))
                && !input.pointer.button_released(egui::PointerButton::Primary);
            let window_focus_lost = input
                .events
                .iter()
                .any(|event| matches!(event, egui::Event::WindowFocused(false)));
            (
                input.events.iter().any(is_plain_escape_press),
                pointer_lost_without_release || window_focus_lost,
            )
        });
        if !escape_pressed && !interaction_lost {
            return false;
        }
        self.rendered_drag = None;
        self.finished_selection = None;
        clear_label_selection(ui);
        if escape_pressed {
            ui.input_mut(|input| input.events.retain(|event| !is_plain_escape_press(event)));
        }
        true
    }

    fn remap_rendered_interaction(
        &mut self,
        ui: &egui::Ui,
        pending_activation: &mut Option<RenderedActivation>,
        replaced: &Range<usize>,
        replacement_len: usize,
    ) {
        let mut invalidated = false;
        if let Some(drag) = self.rendered_drag.take() {
            self.rendered_drag = drag.remap_after_replacement(replaced, replacement_len);
            if let Some(remapped) = self.rendered_drag {
                self.finished_selection = Some(remapped.source_selection());
            } else {
                self.finished_selection = None;
                invalidated = true;
            }
        }
        if let Some(activation) = pending_activation.take() {
            *pending_activation = activation.remap_after_replacement(replaced, replacement_len);
            if let Some(remapped) = pending_activation.as_ref() {
                self.finished_selection = Some(remapped.source_selection);
            } else {
                self.finished_selection = None;
                invalidated = true;
            }
        }
        if invalidated {
            clear_label_selection(ui);
        }
    }

    fn apply_rendered_activation(&mut self, source: &str, activation: Option<RenderedActivation>) {
        let Some(activation) = activation else {
            return;
        };
        let Some((source_range, selection)) =
            source_edit_activation(source, activation.source_selection)
        else {
            self.finished_selection = None;
            return;
        };
        let block = &source[source_range.clone()];
        self.finished_selection = Some(activation.source_selection);
        self.activate_with_selection(source_range, block.to_owned(), selection);
    }

    fn maximum_active_draft_bytes(&self, source: &str, maximum_source_bytes: usize) -> usize {
        self.active.as_ref().map_or(maximum_source_bytes, |active| {
            let retained_source_bytes = source.len().saturating_sub(active.source_range.len());
            maximum_source_bytes.saturating_sub(retained_source_bytes)
        })
    }

    fn finish_active(&mut self) {
        self.retire_active();
        self.rendered_drag = None;
    }

    fn retire_active(&mut self) {
        self.finished_selection = self
            .active
            .take()
            .as_ref()
            .and_then(ActiveBlock::source_selection);
    }

    fn finish_if_requested(&mut self, requested: bool) {
        if requested {
            self.finish_active();
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
        pointer_target: &mut Option<RenderedPointerTarget>,
    ) -> Option<RenderedActivation> {
        let block = &source[range.clone()];
        let frame = egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(
                BLOCK_HORIZONTAL_PADDING,
                BLOCK_VERTICAL_PADDING,
            ))
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
            && let Some(anchor) = label.source_cursor_at(position, &range, block)
        {
            self.rendered_drag = Some(RenderedDragSelection::new(anchor));
        }
        if self.rendered_drag.is_some()
            && let Some(position) = ui.input(|input| input.pointer.interact_pos())
            && let Some(target) = label.pointer_target(position, &range, block)
        {
            target.replace_if_nearer(pointer_target);
        }
        let rendered_selection = if label.response.clicked_by(egui::PointerButton::Primary) {
            label
                .response
                .interact_pointer_pos()
                .map(|position| CharSelection::caret(label.cursor_at(position)))
        } else {
            None
        };
        let activation = rendered_selection.and_then(|selection| {
            let source_selection = label.source_map.source_selection(selection);
            absolute_source_selection(&range, block, source_selection)
                .map(|source_selection| RenderedActivation { source_selection })
        });

        ui.add_space(BLOCK_GAP);
        activation
    }

    fn show_active_editor(
        &mut self,
        ui: &mut egui::Ui,
        maximum_draft_bytes: usize,
    ) -> (bool, bool, Option<MarkdownProjectionLimit>) {
        let editor_id = self.active.as_ref().map(ActiveBlock::editor_id);
        let format_command = editor_id
            .filter(|editor_id| ui.memory(|memory| memory.has_focus(*editor_id)))
            .and_then(|_| {
                ui.input_mut(|input| {
                    MarkdownCommand::WITH_SHORTCUTS.into_iter().find(|command| {
                        command
                            .shortcut()
                            .is_some_and(|shortcut| consume_format_shortcut(input, shortcut))
                    })
                })
            });
        if let Some(command) = format_command {
            self.apply_command(command);
        }
        let Some(active) = self.active.as_mut() else {
            return (false, false, None);
        };
        let editor_id = active.editor_id();
        let finish_requested = prepare_escape_finish(ui, editor_id);
        let rows = logical_lines(&active.draft).count().max(1) + 1;
        let input_origin = direct_input_origin(ui);
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
        self.input_was_limited |=
            sanitize_bounded_text_events(ui, editor_id, &active.draft, maximum_draft_bytes);

        let mut live_projection_limit = None;
        let mut layouter = |ui: &egui::Ui, buffer: &dyn egui::TextBuffer, wrap_width: f32| {
            let source = buffer.as_str();
            let selection =
                char_range_to_byte_range(source, bounded_char_range(source, selection.clone()));
            let layout = active_edit_layout(source, ui.style(), selection, wrap_width);
            live_projection_limit = live_projection_limit.or(layout.projection_limit);
            ui.fonts_mut(|fonts| fonts.layout_job(layout.job))
        };
        let (mut output, buffer_was_limited) = {
            let mut buffer = BoundedTextBuffer::new(&mut active.draft, maximum_draft_bytes);
            let editor = egui::TextEdit::multiline(&mut buffer)
                .id(editor_id)
                .font(egui::TextStyle::Body)
                .desired_width(f32::INFINITY)
                .desired_rows(rows)
                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(
                    BLOCK_HORIZONTAL_PADDING,
                    BLOCK_VERTICAL_PADDING,
                )))
                .layouter(&mut layouter);
            let output = editor.show(ui);
            (output, buffer.was_limited())
        };
        self.input_was_limited |= buffer_was_limited;
        if restore_focus {
            // Toolbar and mode clicks can surrender the focus requested before
            // TextEdit processes input. Reassert it after that pointer pass.
            output.response.request_focus();
        }
        retain_escape_focus(ui, editor_id);
        if let Some(cursor_range) = output.cursor_range {
            active.selection = CharSelection::new(
                cursor_range.secondary.index.into(),
                cursor_range.primary.index.into(),
            );
        }

        let changed = output.response.changed();
        if finish_requested {
            ui.input_mut(|input| input.events.retain(|event| !is_plain_escape_press(event)));
        }
        if changed {
            active.dirty = true;
            active.pending_origin.get_or_insert(input_origin);
        }
        // Shared document history owns Undo and Redo. Discard egui's whole-string
        // snapshots so an active block cannot retain an independent history.
        output.state.clear_undoer();
        output.state.store(ui.ctx(), output.response.id);
        (changed, finish_requested, live_projection_limit)
    }
}

struct ActiveEditLayout {
    job: egui::text::LayoutJob,
    projection_limit: Option<MarkdownProjectionLimit>,
}

fn active_edit_layout(
    source: &str,
    style: &egui::Style,
    selection: Range<usize>,
    wrap_width: f32,
) -> ActiveEditLayout {
    if let Some(limit) = markdown_projection_limit(source) {
        let font_id = egui::TextStyle::Body.resolve(style);
        let text_color = style
            .visuals
            .override_text_color
            .unwrap_or_else(|| style.visuals.widgets.inactive.text_color());
        let mut job =
            egui::text::LayoutJob::simple(source.to_owned(), font_id, text_color, wrap_width);
        job.keep_trailing_whitespace = true;
        return ActiveEditLayout {
            job,
            projection_limit: Some(limit),
        };
    }

    let reveal = semantic_target_at_selection(source, &selection);
    let mut job = markdown_edit_layout(source, style, reveal);
    job.wrap.max_width = wrap_width;
    ActiveEditLayout {
        job,
        projection_limit: None,
    }
}

fn show_block_style_options(ui: &mut egui::Ui, current: Option<BlockStyle>) -> Option<BlockStyle> {
    for style in BlockStyle::ALL {
        if ui
            .add(egui::Button::selectable(
                current == Some(style),
                style.label(),
            ))
            .on_hover_text(match style {
                BlockStyle::Paragraph => "Remove an ATX heading marker from the selected lines",
                BlockStyle::Heading1 => "Set the selected lines to level-one headings",
                BlockStyle::Heading2 => "Set the selected lines to level-two headings",
                BlockStyle::Heading3 => "Set the selected lines to level-three headings",
                BlockStyle::Heading4 => "Set the selected lines to level-four headings",
                BlockStyle::Heading5 => "Set the selected lines to level-five headings",
                BlockStyle::Heading6 => "Set the selected lines to level-six headings",
            })
            .clicked()
        {
            ui.close();
            return Some(style);
        }
    }
    None
}

fn prepare_escape_finish(ui: &egui::Ui, editor_id: egui::Id) -> bool {
    let finish_requested = ui.input(|input| input.events.iter().any(is_plain_escape_press))
        && ui.memory(|memory| memory.had_focus_last_frame(editor_id));
    if finish_requested && !ui.memory(|memory| memory.has_focus(editor_id)) {
        // egui normally releases focus at the start of an Escape frame, before
        // TextEdit can consume text, paste, or IME commit events delivered in
        // that frame. Restore it for one pass so every pending event reaches
        // the backing source before the active range is finished.
        ui.memory_mut(|memory| memory.request_focus(editor_id));
    }
    finish_requested
}

fn retain_escape_focus(ui: &egui::Ui, editor_id: egui::Id) {
    if ui.memory(|memory| memory.has_focus(editor_id)) {
        ui.memory_mut(|memory| {
            memory.set_focus_lock_filter(
                editor_id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    escape: true,
                    ..Default::default()
                },
            );
        });
    }
}

fn is_plain_escape_press(event: &egui::Event) -> bool {
    matches!(
        event,
        egui::Event::Key {
            key: egui::Key::Escape,
            pressed: true,
            modifiers,
            ..
        } if !modifiers.any()
    )
}

fn consume_format_shortcut(input: &mut egui::InputState, shortcut: egui::KeyboardShortcut) -> bool {
    let pressed_once = input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key,
                pressed: true,
                repeat: false,
                modifiers,
                ..
            } if *key == shortcut.logical_key && modifiers.matches_logically(shortcut.modifiers)
        )
    });
    pressed_once && input.consume_shortcut(&shortcut)
}

fn restorable_source_edit_range(source: &str, selection: Selection) -> Option<Range<usize>> {
    if selection.anchor() > source.len()
        || selection.active() > source.len()
        || !source.is_char_boundary(selection.anchor())
        || !source.is_char_boundary(selection.active())
    {
        return None;
    }
    let selected = selection.ordered_range();
    let ranges = markdown_block_ranges(source);
    if ranges.is_empty() {
        return Some(0..source.len());
    }

    if selection.anchor() == selection.active()
        && let Some(block) = ranges
            .iter()
            .find(|range| range.start <= selected.start() && selected.start() <= range.end)
    {
        return Some(block.clone());
    }

    let mut related = ranges
        .iter()
        .filter(|range| range.start < selected.end() && selected.start() < range.end);
    if let Some(first) = related.next() {
        let last = related.next_back().unwrap_or(first);
        return Some(selected.start().min(first.start)..selected.end().max(last.end));
    }
    if let Some(previous) = ranges
        .iter()
        .rev()
        .find(|range| range.end <= selected.start())
    {
        return Some(previous.start..selected.end().max(previous.end));
    }
    let next = ranges.iter().find(|range| selected.end() <= range.start)?;
    Some(selected.start().min(next.start)..next.end)
}

fn source_edit_activation(
    source: &str,
    selection: Selection,
) -> Option<(Range<usize>, CharSelection)> {
    let range = restorable_source_edit_range(source, selection)?;
    let block = source.get(range.clone())?;
    let anchor_byte = selection.anchor().checked_sub(range.start)?;
    let active_byte = selection.active().checked_sub(range.start)?;
    if anchor_byte > block.len()
        || active_byte > block.len()
        || !block.is_char_boundary(anchor_byte)
        || !block.is_char_boundary(active_byte)
    {
        return None;
    }
    Some((
        range,
        CharSelection::new(
            block[..anchor_byte].chars().count(),
            block[..active_byte].chars().count(),
        ),
    ))
}

fn absolute_source_selection(
    source_range: &Range<usize>,
    source: &str,
    selection: CharSelection,
) -> Option<Selection> {
    let character_count = source.chars().count();
    if character_count < selection.anchor || character_count < selection.active {
        return None;
    }
    let anchor = source_range
        .start
        .checked_add(char_index_to_byte(source, selection.anchor))?;
    let active = source_range
        .start
        .checked_add(char_index_to_byte(source, selection.active))?;
    if source_range.end < anchor || source_range.end < active {
        return None;
    }
    Some(Selection::new(anchor, active))
}

fn clear_label_selection(ui: &egui::Ui) {
    ui.ctx()
        .plugin::<egui::text_selection::LabelSelectionState>()
        .lock()
        .clear_selection();
}

fn rendered_drag_scroll_delta(pointer_y: f32, clip_rect: egui::Rect, frame_seconds: f32) -> f32 {
    if !pointer_y.is_finite()
        || !clip_rect.is_finite()
        || clip_rect.height() <= 0.0
        || !frame_seconds.is_finite()
        || frame_seconds <= 0.0
    {
        return 0.0;
    }
    let edge = DRAG_AUTOSCROLL_EDGE.min(clip_rect.height() / 2.0);
    if edge <= 0.0 {
        return 0.0;
    }
    let maximum_delta =
        DRAG_AUTOSCROLL_MAX_SPEED * frame_seconds.min(DRAG_AUTOSCROLL_MAX_FRAME_SECONDS);
    if pointer_y < clip_rect.top() + edge {
        let intensity = ((clip_rect.top() + edge - pointer_y) / edge).clamp(0.0, 1.0);
        return maximum_delta * intensity;
    }
    if clip_rect.bottom() - edge < pointer_y {
        let intensity = ((pointer_y - (clip_rect.bottom() - edge)) / edge).clamp(0.0, 1.0);
        return -maximum_delta * intensity;
    }
    0.0
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
    let mut line_prefix_is_indentation = true;

    for (source_character, (index, character)) in source.char_indices().enumerate() {
        let at_indented_line_start = line_prefix_is_indentation;
        let followed_by_space = source[index + character.len_utf8()..].starts_with(' ');
        line_prefix_is_indentation = match character {
            '\n' | '\r' => true,
            ' ' | '\t' => line_prefix_is_indentation,
            _ => false,
        };
        let source_style = source_styles[index];
        if !source_style.has(STYLE_VISIBLE) {
            append_render_run(&mut job, &mut run, run_style.take(), style);
            continue;
        }
        if is_quote_marker(character, at_indented_line_start, followed_by_space) {
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
        run.push(formatted_block_marker(
            character,
            at_indented_line_start,
            followed_by_space,
        ));
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
        line_height: Some(markdown_line_height(
            style.text_styles[&egui::TextStyle::Body].size,
            false,
        )),
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

const fn formatted_block_marker(
    character: char,
    at_indented_line_start: bool,
    followed_by_space: bool,
) -> char {
    if at_indented_line_start && followed_by_space && matches!(character, '-' | '+' | '*') {
        '•'
    } else {
        character
    }
}

const fn is_quote_marker(
    character: char,
    at_indented_line_start: bool,
    followed_by_space: bool,
) -> bool {
    character == '>' && at_indented_line_start && followed_by_space
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
            let target = if destination.is_empty() {
                let opening = raw.rfind("](")? + 1;
                if raw.as_bytes().get(opening + 1) != Some(&b')') {
                    return None;
                }
                source_range.start + opening..source_range.start + opening + 2
            } else {
                let offset = raw.rfind(destination.as_ref())?;
                source_range.start + offset..source_range.start + offset + destination.len()
            };
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
            line_height: Some(markdown_line_height(
                style.text_styles[&egui::TextStyle::Body].size,
                false,
            )),
            color: egui::Color32::TRANSPARENT,
            ..Default::default()
        };
    }

    let mut font_id = style.text_styles[&egui::TextStyle::Body].clone();
    let is_heading = source_style.heading_level > 0;
    let heading_weight = if is_heading {
        let ratio = HEADING_SIZE_RATIOS
            .get(usize::from(source_style.heading_level.saturating_sub(1)))
            .copied()
            .unwrap_or(1.0);
        font_id.size *= ratio;
        HEADING_WEIGHT
    } else {
        BODY_WEIGHT
    };
    let line_height = markdown_line_height(font_id.size, is_heading);
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
        line_height: Some(line_height),
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

fn markdown_line_height(font_size: f32, is_heading: bool) -> f32 {
    let ratio = if is_heading {
        HEADING_LINE_HEIGHT_RATIO
    } else {
        BODY_LINE_HEIGHT_RATIO
    };
    (font_size * ratio).round()
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
        MarkdownCommand::Bold => toggle_inline_marker(source, selection, "**"),
        MarkdownCommand::Italic => toggle_inline_marker(source, selection, "*"),
        MarkdownCommand::InlineCode => toggle_inline_marker(source, selection, "`"),
        MarkdownCommand::Link => insert_link(source, selection),
        MarkdownCommand::BulletedList => toggle_line_prefix(source, selection, "- "),
        MarkdownCommand::Quote => toggle_line_prefix(source, selection, "> "),
    }
}

fn apply_block_style(source: &str, selection: Range<usize>, style: BlockStyle) -> CommandResult {
    let selection = bounded_char_range(source, selection);
    let state = selected_block_style(source, selection.clone());
    if state == BlockStyleState::Uniform(style) || state == BlockStyleState::Unavailable {
        return CommandResult {
            text: source.to_owned(),
            selection,
        };
    }
    set_heading_style(source, selection, style.heading_level())
}

fn selected_block_style(source: &str, selection: Range<usize>) -> BlockStyleState {
    if source.is_empty() {
        return BlockStyleState::Uniform(BlockStyle::Paragraph);
    }
    let lines = logical_line_ranges(source);
    let selected = selected_line_indexes(source, selection, &lines);
    let styles = styleable_line_styles(source, &lines);
    let mut current = None;
    let mut mixed = false;
    for index in selected {
        let style = match styles[index] {
            StyleableLine::Paragraph => BlockStyle::Paragraph,
            StyleableLine::Heading { style, .. } => style,
            StyleableLine::Unavailable => return BlockStyleState::Unavailable,
        };
        if current.is_some_and(|current| current != style) {
            mixed = true;
        }
        current = Some(style);
    }
    if mixed {
        BlockStyleState::Mixed
    } else {
        BlockStyleState::Uniform(current.unwrap_or(BlockStyle::Paragraph))
    }
}

fn markdown_command_is_active(
    source: &str,
    selection: Range<usize>,
    command: MarkdownCommand,
) -> bool {
    match command {
        MarkdownCommand::Bold => inline_marker_location(source, selection, "**").is_some(),
        MarkdownCommand::Italic => inline_marker_location(source, selection, "*").is_some(),
        MarkdownCommand::InlineCode => inline_marker_location(source, selection, "`").is_some(),
        MarkdownCommand::Link => link_span_at_selection(source, selection).is_some(),
        MarkdownCommand::BulletedList => {
            selected_lines_all(source, selection, |content| content.starts_with("- "))
        }
        MarkdownCommand::Quote => {
            selected_lines_all(source, selection, |content| content.starts_with("> "))
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum InlineMarkerLocation {
    Outside {
        prefix: Range<usize>,
        suffix: Range<usize>,
    },
    Inside {
        prefix: Range<usize>,
        suffix: Range<usize>,
    },
}

fn toggle_inline_marker(source: &str, selection: Range<usize>, marker: &str) -> CommandResult {
    let selection = bounded_char_range(source, selection);
    if let Some(location) = inline_marker_location(source, selection.clone(), marker) {
        return remove_inline_markers(source, selection, marker, location);
    }
    let byte_range = char_range_to_byte_range(source, selection.clone());
    let selected = &source[byte_range.clone()];
    if inline_marker_insertion_is_ambiguous(source, &byte_range, marker) {
        return CommandResult {
            text: source.to_owned(),
            selection,
        };
    }
    let mut text = String::with_capacity(source.len() + 2 * marker.len());
    text.push_str(&source[..byte_range.start]);
    text.push_str(marker);
    text.push_str(selected);
    text.push_str(marker);
    text.push_str(&source[byte_range.end..]);
    let semantic_range = byte_range.start..byte_range.end + 2 * marker.len();
    if !selected.is_empty()
        && !semantic_candidate_is_valid_or_requires_text_fallback(&text, || {
            inline_marker_range_is_semantic(&text, marker, &semantic_range)
        })
    {
        return CommandResult {
            text: source.to_owned(),
            selection,
        };
    }
    let start = selection.start + marker.chars().count();
    CommandResult {
        text,
        selection: start..selection.end + marker.chars().count(),
    }
}

fn inline_marker_insertion_is_ambiguous(
    source: &str,
    selected: &Range<usize>,
    marker: &str,
) -> bool {
    let marker_bytes = marker.as_bytes();
    let Some(&needle) = marker_bytes.first() else {
        return true;
    };
    if marker_bytes.iter().any(|byte| *byte != needle) {
        return true;
    }
    let bytes = source.as_bytes();
    let left_run = repeated_byte_before(bytes, selected.start, needle);
    let right_run = repeated_byte_after(bytes, selected.end, needle);
    if left_run == 0 && right_run == 0 {
        return false;
    }
    if selected.is_empty() {
        return true;
    }
    match (needle, marker.len()) {
        (b'*', 1) => left_run != 2 || right_run != 2,
        (b'*', 2) => left_run != 1 || right_run != 1,
        _ => true,
    }
}

fn remove_inline_markers(
    source: &str,
    selection: Range<usize>,
    marker: &str,
    location: InlineMarkerLocation,
) -> CommandResult {
    let marker_characters = marker.chars().count();
    let (prefix, suffix, adjusted_selection) = match location {
        InlineMarkerLocation::Outside { prefix, suffix } => {
            let start = selection.start.saturating_sub(marker_characters);
            let end = selection.end.saturating_sub(marker_characters);
            (prefix, suffix, start..end)
        }
        InlineMarkerLocation::Inside { prefix, suffix } => (
            prefix,
            suffix,
            selection.start..selection.end.saturating_sub(2 * marker_characters),
        ),
    };
    let mut text = String::with_capacity(source.len().saturating_sub(2 * marker.len()));
    text.push_str(&source[..prefix.start]);
    text.push_str(&source[prefix.end..suffix.start]);
    text.push_str(&source[suffix.end..]);
    CommandResult {
        text,
        selection: adjusted_selection,
    }
}

fn inline_marker_location(
    source: &str,
    selection: Range<usize>,
    marker: &str,
) -> Option<InlineMarkerLocation> {
    let selection = bounded_char_range(source, selection);
    let bytes = source.as_bytes();
    let selected = char_range_to_byte_range(source, selection);
    let width = marker.len();
    if width == 0 {
        return None;
    }

    let repeated_marker = marker
        .as_bytes()
        .first()
        .is_some_and(|first| marker.as_bytes().iter().all(|byte| byte == first));
    let outside_active = if marker.as_bytes().iter().all(|byte| *byte == b'*') {
        star_marker_is_active(
            repeated_byte_before(bytes, selected.start, b'*'),
            repeated_byte_after(bytes, selected.end, b'*'),
            width,
        )
    } else if repeated_marker {
        let needle = marker.as_bytes()[0];
        repeated_byte_before(bytes, selected.start, needle) == width
            && repeated_byte_after(bytes, selected.end, needle) == width
    } else {
        selected.start >= width
            && selected.end + width <= source.len()
            && bytes.get(selected.start - width..selected.start) == Some(marker.as_bytes())
            && bytes.get(selected.end..selected.end + width) == Some(marker.as_bytes())
    };
    if outside_active {
        let location = InlineMarkerLocation::Outside {
            prefix: selected.start - width..selected.start,
            suffix: selected.end..selected.end + width,
        };
        if !selected.is_empty() && inline_marker_location_is_semantic(source, marker, &location) {
            return Some(location);
        }
    }

    if selected.is_empty() || selected.end.saturating_sub(selected.start) < 2 * width {
        return None;
    }
    let inside_active = if marker.as_bytes().iter().all(|byte| *byte == b'*') {
        star_marker_is_active(
            repeated_byte_after(bytes, selected.start, b'*'),
            repeated_byte_before(bytes, selected.end, b'*'),
            width,
        )
    } else if repeated_marker {
        let needle = marker.as_bytes()[0];
        repeated_byte_after(bytes, selected.start, needle) == width
            && repeated_byte_before(bytes, selected.end, needle) == width
    } else {
        bytes.get(selected.start..selected.start + width) == Some(marker.as_bytes())
            && bytes.get(selected.end - width..selected.end) == Some(marker.as_bytes())
    };
    let location = InlineMarkerLocation::Inside {
        prefix: selected.start..selected.start + width,
        suffix: selected.end - width..selected.end,
    };
    (inside_active && inline_marker_location_is_semantic(source, marker, &location))
        .then_some(location)
}

fn inline_marker_location_is_semantic(
    source: &str,
    marker: &str,
    location: &InlineMarkerLocation,
) -> bool {
    let (prefix, suffix) = match location {
        InlineMarkerLocation::Outside { prefix, suffix }
        | InlineMarkerLocation::Inside { prefix, suffix } => (prefix, suffix),
    };
    inline_marker_range_is_semantic(source, marker, &(prefix.start..suffix.end))
}

fn inline_marker_range_is_semantic(
    source: &str,
    marker: &str,
    expected_range: &Range<usize>,
) -> bool {
    let Some(fragment) = source.get(expected_range.clone()) else {
        return false;
    };
    let fragment_is_semantic = Parser::new_ext(fragment, markdown_parser_options())
        .into_offset_iter()
        .any(|(event, source_range)| {
            source_range == (0..fragment.len()) && inline_event_matches_marker(marker, &event)
        });
    fragment_is_semantic
        && Parser::new_ext(source, markdown_parser_options())
            .into_offset_iter()
            .any(|(event, source_range)| {
                let range_matches = if marker == "`" {
                    source_range.start == expected_range.start
                        && source_range.end == expected_range.end
                } else {
                    source_range.start <= expected_range.start
                        && source_range.end >= expected_range.end
                };
                range_matches && inline_event_matches_marker(marker, &event)
            })
}

fn inline_event_matches_marker(marker: &str, event: &Event<'_>) -> bool {
    matches!(
        (marker, event),
        ("**", Event::Start(Tag::Strong))
            | ("*", Event::Start(Tag::Emphasis))
            | ("`", Event::Code(_))
    )
}

const fn star_marker_is_active(left_run: usize, right_run: usize, width: usize) -> bool {
    if left_run != right_run {
        return false;
    }
    match width {
        1 => left_run == 1 || left_run == 3,
        2 => left_run == 2 || left_run == 3,
        _ => false,
    }
}

fn repeated_byte_before(bytes: &[u8], index: usize, needle: u8) -> usize {
    bytes[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == needle)
        .count()
}

fn repeated_byte_after(bytes: &[u8], index: usize, needle: u8) -> usize {
    bytes[index..]
        .iter()
        .take_while(|byte| **byte == needle)
        .count()
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct LinkSpan {
    entire: Range<usize>,
    label: Range<usize>,
}

fn insert_link(source: &str, selection: Range<usize>) -> CommandResult {
    let selection = bounded_char_range(source, selection);
    if let Some(link) = link_span_at_selection(source, selection.clone()) {
        let label = &source[link.label.clone()];
        let label_characters = label.chars().count();
        let selection_start = source[..link.entire.start].chars().count();
        let mut text = source.to_owned();
        text.replace_range(link.entire, label);
        return CommandResult {
            text,
            selection: selection_start..selection_start + label_characters,
        };
    }
    if selected_link_source_range(source, selection.clone()).is_some() {
        return CommandResult {
            text: source.to_owned(),
            selection,
        };
    }
    let byte_range = char_range_to_byte_range(source, selection.clone());
    let selected = &source[byte_range.clone()];
    let replacement = format!("[{selected}]()");
    let mut text = source.to_owned();
    text.replace_range(byte_range.clone(), &replacement);
    let label_start = selection.start + 1;
    let label_selection = label_start..label_start + selected.chars().count();
    let expected_entire = byte_range.start..byte_range.start + replacement.len();
    let expected_label = byte_range.start + 1..byte_range.start + 1 + selected.len();
    let candidate_is_valid = semantic_candidate_is_valid_or_requires_text_fallback(&text, || {
        link_span_at_selection(&text, label_selection)
            .is_some_and(|link| link.entire == expected_entire && link.label == expected_label)
    });
    if !candidate_is_valid {
        return CommandResult {
            text: source.to_owned(),
            selection,
        };
    }
    let selection_start = if selected.is_empty() {
        selection.start + 1
    } else {
        selection.start + selected.chars().count() + 3
    };
    CommandResult {
        text,
        selection: selection_start..selection_start,
    }
}

fn semantic_candidate_is_valid_or_requires_text_fallback(
    source: &str,
    validate: impl FnOnce() -> bool,
) -> bool {
    markdown_projection_limit(source).is_some() || validate()
}

fn link_span_at_selection(source: &str, selection: Range<usize>) -> Option<LinkSpan> {
    let entire = selected_link_source_range(source, selection)?;
    let raw = source.get(entire.clone())?;
    if !raw.starts_with('[') {
        return None;
    }
    let mut nested_brackets = 0_usize;
    let mut escaped = false;
    let bytes = raw.as_bytes();
    let mut protected_inline_ranges = Parser::new_ext(raw, markdown_parser_options())
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(event, Event::Code(_) | Event::InlineHtml(_)).then_some(range)
        })
        .collect::<Vec<_>>();
    protected_inline_ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut protected_index = 0_usize;
    for index in 1..bytes.len().saturating_sub(1) {
        let byte = bytes[index];
        while protected_inline_ranges
            .get(protected_index)
            .is_some_and(|range| range.end <= index)
        {
            protected_index += 1;
        }
        if protected_inline_ranges
            .get(protected_index)
            .is_some_and(|range| range.start <= index && index < range.end)
        {
            continue;
        }
        if escaped {
            escaped = false;
            continue;
        }
        if byte == b'\\' {
            escaped = true;
            continue;
        }
        if byte == b'[' {
            nested_brackets += 1;
            continue;
        }
        if byte != b']' {
            continue;
        }
        if nested_brackets > 0 {
            nested_brackets -= 1;
        } else if bytes.get(index + 1) == Some(&b'(') {
            return Some(LinkSpan {
                label: entire.start + 1..entire.start + index,
                entire,
            });
        }
    }
    None
}

fn selected_link_source_range(source: &str, selection: Range<usize>) -> Option<Range<usize>> {
    let selected = char_range_to_byte_range(source, bounded_char_range(source, selection));
    Parser::new_ext(source, markdown_parser_options())
        .into_offset_iter()
        .find_map(|(event, source_range)| {
            if !matches!(event, Event::Start(Tag::Link { .. })) {
                return None;
            }
            let contained = if selected.is_empty() {
                selected.start > source_range.start && selected.end < source_range.end
            } else {
                selected.start >= source_range.start && selected.end <= source_range.end
            };
            contained.then_some(source_range)
        })
}

fn set_heading_style(source: &str, selection: Range<usize>, level: Option<usize>) -> CommandResult {
    let selection = bounded_char_range(source, selection);
    if source.is_empty() {
        let text = level.map_or_else(String::new, |level| format!("{} ", "#".repeat(level)));
        let caret = text.chars().count();
        return CommandResult {
            text,
            selection: caret..caret,
        };
    }
    let lines = logical_line_ranges(source);
    let selected = selected_line_indexes(source, selection.clone(), &lines);
    let styles = styleable_line_styles(source, &lines);
    if selected
        .iter()
        .any(|index| styles[*index] == StyleableLine::Unavailable)
    {
        return CommandResult {
            text: source.to_owned(),
            selection,
        };
    }
    let prefix = level.map_or_else(String::new, |level| format!("{} ", "#".repeat(level)));
    rewrite_selected_lines(source, selection, |index, content| {
        match (level, styles[index]) {
            (Some(level), StyleableLine::Heading { style, syntax })
                if style.heading_level() == Some(level) =>
            {
                (content.to_owned(), syntax.opening_end)
            }
            (Some(_), StyleableLine::Heading { syntax, .. }) => (
                format!("{prefix}{}", &content[syntax.opening_end..]),
                prefix.len(),
            ),
            (Some(_), StyleableLine::Paragraph) => (format!("{prefix}{content}"), prefix.len()),
            (None, StyleableLine::Heading { syntax, .. }) => {
                let content_end = syntax.closing_start.unwrap_or(content.len());
                (content[syntax.opening_end..content_end].to_owned(), 0)
            }
            (None, StyleableLine::Paragraph) => (content.to_owned(), 0),
            (_, StyleableLine::Unavailable) => {
                unreachable!("unsupported lines are rejected before rewriting")
            }
        }
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct AtxHeadingSyntax {
    level: u8,
    opening_end: usize,
    closing_start: Option<usize>,
}

fn atx_heading_syntax(content: &str) -> Option<AtxHeadingSyntax> {
    let bytes = content.as_bytes();
    let mut marker_start = 0;
    while marker_start < bytes.len() && bytes[marker_start] == b' ' {
        marker_start += 1;
    }
    if marker_start > 3 {
        return None;
    }

    let mut marker_end = marker_start;
    while marker_end < bytes.len() && bytes[marker_end] == b'#' {
        marker_end += 1;
    }
    let level = marker_end.saturating_sub(marker_start);
    if !(1..=6).contains(&level) {
        return None;
    }
    if marker_end < bytes.len() && !matches!(bytes[marker_end], b' ' | b'\t') {
        return None;
    }

    let mut opening_end = marker_end;
    while opening_end < bytes.len() && matches!(bytes[opening_end], b' ' | b'\t') {
        opening_end += 1;
    }

    let mut trimmed_end = bytes.len();
    while trimmed_end > opening_end && matches!(bytes[trimmed_end - 1], b' ' | b'\t') {
        trimmed_end -= 1;
    }
    let mut closing_hash_start = trimmed_end;
    while closing_hash_start > opening_end && bytes[closing_hash_start - 1] == b'#' {
        closing_hash_start -= 1;
    }
    let has_closing_sequence = closing_hash_start < trimmed_end
        && closing_hash_start > 0
        && matches!(bytes[closing_hash_start - 1], b' ' | b'\t');
    let closing_start = has_closing_sequence.then(|| {
        let mut start = closing_hash_start;
        while start > opening_end && matches!(bytes[start - 1], b' ' | b'\t') {
            start -= 1;
        }
        start
    });

    Some(AtxHeadingSyntax {
        level: level as u8,
        opening_end,
        closing_start,
    })
}

fn paragraph_round_trips_through_heading(content: &str) -> bool {
    let candidate = format!("# {content}");
    let Some(syntax) = atx_heading_syntax(&candidate) else {
        return false;
    };
    let content_end = syntax.closing_start.unwrap_or(candidate.len());
    candidate
        .get(syntax.opening_end..content_end)
        .is_some_and(|round_tripped| round_tripped == content)
}

fn toggle_line_prefix(source: &str, selection: Range<usize>, prefix: &str) -> CommandResult {
    let remove = selected_lines_all(source, selection.clone(), |content| {
        content.starts_with(prefix)
    });
    rewrite_selected_lines(source, selection, |_, content| {
        if content.is_empty() {
            return (String::new(), 0);
        }
        if remove {
            (
                content.strip_prefix(prefix).unwrap_or(content).to_owned(),
                0,
            )
        } else {
            (format!("{prefix}{content}"), prefix.len())
        }
    })
}

#[derive(Clone, PartialEq, Eq, Debug)]
struct LogicalLineRange {
    content: Range<usize>,
    full: Range<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StyleableLine {
    Paragraph,
    Heading {
        style: BlockStyle,
        syntax: AtxHeadingSyntax,
    },
    Unavailable,
}

fn logical_line_ranges(source: &str) -> Vec<LogicalLineRange> {
    let mut offset = 0;
    logical_lines(source)
        .map(|line| {
            let content = offset..offset + line.content().len();
            let ending_length = line.ending().map_or(0, |ending| ending.as_str().len());
            let full = offset..content.end + ending_length;
            offset = full.end;
            LogicalLineRange { content, full }
        })
        .collect()
}

fn styleable_line_styles(source: &str, lines: &[LogicalLineRange]) -> Vec<StyleableLine> {
    let parser = Parser::new_ext(source, markdown_parser_options()).into_offset_iter();
    let mut unsupported_ranges = parser
        .reference_definitions()
        .iter()
        .map(|(_, definition)| definition.span.clone())
        .collect::<Vec<_>>();
    let mut paragraph_ranges = Vec::new();
    let mut heading_ranges = Vec::new();
    let mut depth = 0_usize;

    for (event, range) in parser {
        match event {
            Event::Start(tag) => {
                if depth == 0 {
                    match tag {
                        Tag::Paragraph => paragraph_ranges.push(range),
                        Tag::Heading { level, .. } => {
                            let style = BlockStyle::from_heading_level(heading_level_number(level))
                                .expect(
                                    "pulldown-cmark exposes only heading levels one through six",
                                );
                            heading_ranges.push((style, range));
                        }
                        _ => unsupported_ranges.push(range),
                    }
                }
                depth += 1;
            }
            Event::End(_) => depth = depth.saturating_sub(1),
            _ if depth == 0 => unsupported_ranges.push(range),
            _ => {}
        }
    }

    lines
        .iter()
        .map(|line| {
            let content = &source[line.content.clone()];
            let probe = if content.is_empty() {
                &line.full
            } else {
                &line.content
            };
            if let Some((style, _)) = heading_ranges
                .iter()
                .find(|(_, range)| ranges_overlap(probe, range))
            {
                return match atx_heading_syntax(content) {
                    Some(syntax)
                        if BlockStyle::from_heading_level(syntax.level) == Some(*style) =>
                    {
                        StyleableLine::Heading {
                            style: *style,
                            syntax,
                        }
                    }
                    _ => StyleableLine::Unavailable,
                };
            }
            if paragraph_ranges
                .iter()
                .any(|range| ranges_overlap(probe, range))
            {
                return if paragraph_round_trips_through_heading(content) {
                    StyleableLine::Paragraph
                } else {
                    StyleableLine::Unavailable
                };
            }
            if unsupported_ranges
                .iter()
                .any(|range| ranges_overlap(probe, range))
            {
                return StyleableLine::Unavailable;
            }
            if content.is_empty() {
                StyleableLine::Paragraph
            } else {
                StyleableLine::Unavailable
            }
        })
        .collect()
}

fn selected_line_indexes(
    source: &str,
    selection: Range<usize>,
    lines: &[LogicalLineRange],
) -> Vec<usize> {
    let selected = char_range_to_byte_range(source, bounded_char_range(source, selection));
    if selected.is_empty() {
        return lines
            .iter()
            .position(|line| {
                selected.start >= line.full.start
                    && (selected.start < line.full.end
                        || (selected.start == line.full.end && line.content.end == line.full.end))
            })
            .into_iter()
            .collect();
    }
    lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| {
            (line.full.start < selected.end && selected.start < line.full.end).then_some(index)
        })
        .collect()
}

fn selected_lines_all(
    source: &str,
    selection: Range<usize>,
    predicate: impl Fn(&str) -> bool,
) -> bool {
    let lines = logical_line_ranges(source);
    let selected = selected_line_indexes(source, selection, &lines);
    let mut nonempty = 0;
    for index in selected {
        let content = &source[lines[index].content.clone()];
        if content.is_empty() {
            continue;
        }
        nonempty += 1;
        if !predicate(content) {
            return false;
        }
    }
    nonempty > 0
}

fn rewrite_selected_lines(
    source: &str,
    selection: Range<usize>,
    rewrite: impl Fn(usize, &str) -> (String, usize),
) -> CommandResult {
    let selection = bounded_char_range(source, selection);
    let lines = logical_line_ranges(source);
    let selected = selected_line_indexes(source, selection.clone(), &lines);
    if selected.is_empty() {
        return CommandResult {
            text: source.to_owned(),
            selection,
        };
    }
    let mut is_selected = vec![false; lines.len()];
    for index in selected {
        is_selected[index] = true;
    }
    let mut text = String::with_capacity(source.len());
    let mut selection_start = None;
    let mut selection_end = 0;
    for (index, line) in lines.iter().enumerate() {
        let content = &source[line.content.clone()];
        if is_selected[index] {
            let (rewritten, prefix_length) = rewrite(index, content);
            selection_start.get_or_insert_with(|| text.len() + prefix_length.min(rewritten.len()));
            text.push_str(&rewritten);
            selection_end = text.len();
        } else {
            text.push_str(content);
        }
        text.push_str(&source[line.content.end..line.full.end]);
    }
    let selection_start = selection_start.unwrap_or(0);
    let start = text[..selection_start].chars().count();
    let end = text[..selection_end].chars().count();
    CommandResult {
        text,
        selection: start..end,
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

fn remap_disjoint_selection(
    selection: Selection,
    replaced: &Range<usize>,
    replacement_len: usize,
) -> Option<Selection> {
    Some(Selection::new(
        remap_disjoint_position(selection.anchor(), replaced, replacement_len)?,
        remap_disjoint_position(selection.active(), replaced, replacement_len)?,
    ))
}

fn remap_disjoint_position(
    position: usize,
    replaced: &Range<usize>,
    replacement_len: usize,
) -> Option<usize> {
    if position <= replaced.start {
        return Some(position);
    }
    if replaced.end <= position {
        let removed_len = replaced.end.checked_sub(replaced.start)?;
        return if replacement_len >= removed_len {
            position.checked_add(replacement_len - removed_len)
        } else {
            position.checked_sub(removed_len - replacement_len)
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accesskit_bounds(output: &egui::FullOutput, label: &str) -> egui::accesskit::Rect {
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled")
            .nodes;
        nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
            .unwrap_or_else(|| {
                let labels = nodes
                    .iter()
                    .filter_map(|(_, node)| node.label())
                    .collect::<Vec<_>>();
                panic!("expected an AccessKit node labeled `{label}` with bounds among {labels:?}")
            })
    }

    fn accesskit_node_id(output: &egui::FullOutput, label: &str) -> egui::accesskit::NodeId {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled")
            .nodes
            .iter()
            .find_map(|(id, node)| (node.label() == Some(label)).then_some(*id))
            .unwrap_or_else(|| panic!("expected an AccessKit node labeled `{label}`"))
    }

    fn accesskit_bounds_starting_with(
        output: &egui::FullOutput,
        prefix: &str,
    ) -> egui::accesskit::Rect {
        let nodes = &output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled")
            .nodes;
        nodes
            .iter()
            .find_map(|(_, node)| {
                node.label()
                    .is_some_and(|label| label.starts_with(prefix))
                    .then(|| node.bounds())
                    .flatten()
            })
            .unwrap_or_else(|| {
                let labels = nodes
                    .iter()
                    .filter_map(|(_, node)| node.label())
                    .collect::<Vec<_>>();
                panic!(
                    "expected an AccessKit node starting with `{prefix}` with bounds among {labels:?}"
                )
            })
    }

    fn accesskit_toggled(
        output: &egui::FullOutput,
        label: &str,
    ) -> Option<egui::accesskit::Toggled> {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled")
            .nodes
            .iter()
            .find_map(|(_, node)| (node.label() == Some(label)).then(|| node.toggled()))
            .unwrap_or_else(|| panic!("expected an AccessKit node labeled `{label}`"))
    }

    fn accesskit_role_and_value(
        output: &egui::FullOutput,
        label: &str,
    ) -> (egui::accesskit::Role, Option<String>) {
        output
            .platform_output
            .accesskit_update
            .as_ref()
            .expect("AccessKit must produce an update when enabled")
            .nodes
            .iter()
            .find_map(|(_, node)| {
                (node.label() == Some(label))
                    .then(|| (node.role(), node.value().map(str::to_owned)))
            })
            .unwrap_or_else(|| panic!("expected an AccessKit node labeled `{label}`"))
    }

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

    fn viewport_input(width: f32, height: f32) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, height),
            )),
            ..Default::default()
        }
    }

    fn pointer_input(position: egui::Pos2, pressed: Option<bool>) -> egui::RawInput {
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::PointerMoved(position));
        if let Some(pressed) = pressed {
            input.events.push(egui::Event::PointerButton {
                pos: position,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        input
    }

    fn show_markdown_frame(
        context: &egui::Context,
        editor: &mut MarkdownEditor,
        source: &mut String,
        input: egui::RawInput,
    ) -> egui::FullOutput {
        context.run_ui(input, |ui| {
            ui.set_width(800.0);
            let _ = editor.show(ui, source);
        })
    }

    fn rendered_text_rect(output: &egui::FullOutput, expected: &str) -> egui::Rect {
        output
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, expected))
            .unwrap_or_else(|| panic!("expected rendered text `{expected}`"))
    }

    fn origin_from_input(input: egui::RawInput) -> EditOrigin {
        let context = egui::Context::default();
        let mut origin = EditOrigin::MarkdownInput;
        let _ = context.run_ui(input, |ui| origin = direct_input_origin(ui));
        origin
    }

    #[test]
    fn markdown_paste_is_an_explicit_non_coalescing_origin() {
        let mut paste = egui::RawInput::default();
        paste.events.push(egui::Event::Paste("content".to_owned()));
        assert_eq!(origin_from_input(paste), EditOrigin::Paste);

        let mut text = egui::RawInput::default();
        text.events.push(egui::Event::Text("x".to_owned()));
        assert_eq!(origin_from_input(text), EditOrigin::MarkdownInput);
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
        assert!(format.line_height.is_some_and(
            |line_height| line_height > style.text_styles[&egui::TextStyle::Body].size
        ));
        assert_eq!(
            projection
                .source_map
                .source_selection(CharSelection::new(0, source.chars().count())),
            CharSelection::new(0, source.chars().count())
        );
    }

    #[test]
    fn format_toolbar_uses_compact_layout_only_below_its_minimum() {
        for (width, expected) in [(420.0, false), (479.0, false), (480.0, true)] {
            assert_eq!(expanded_toolbar_fits(std::hint::black_box(width)), expected);
        }
    }

    #[test]
    fn active_format_toolbar_stays_inside_compact_and_expanded_viewports() {
        let context = egui::Context::default();
        context.enable_accesskit();
        crate::theme::configure_styles(&context);
        let mut editor = MarkdownEditor::default();
        editor.activate(0..1, "x".to_owned());

        let compact = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(420.0);
            editor.toolbar(ui);
        });
        assert!(
            accesskit_bounds(&compact, "Format").x1 <= 420.0,
            "the compact Format control must stay inside the minimum viewport"
        );

        let expanded = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(EXPANDED_FORMAT_MIN_WIDTH);
            editor.toolbar(ui);
        });
        for label in [
            "Paragraph style",
            "Bold",
            "Italic",
            "Link",
            "Inline code",
            "Bulleted list",
            "Quote",
        ] {
            let bounds = accesskit_bounds(&expanded, label);
            assert!(
                bounds.x1 <= f64::from(EXPANDED_FORMAT_MIN_WIDTH),
                "`{label}` extends beyond the expanded toolbar: {bounds:?}"
            );
        }
        assert!(
            expanded
                .shapes
                .iter()
                .all(|shape| text_rect(&shape.shape, "Done").is_none()),
            "the permanent format bar must not expose a modal Done control"
        );
    }

    #[test]
    fn compact_format_submenus_keep_every_action_inside_the_minimum_viewport() {
        const WIDTH: f32 = 420.0;
        const HEIGHT: f32 = 300.0;

        let context = egui::Context::default();
        context.enable_accesskit();
        crate::theme::configure_styles(&context);
        let mut editor = MarkdownEditor::default();
        editor.activate(0..1, "x".to_owned());

        let initial = context.run_ui(viewport_input(WIDTH, HEIGHT), |ui| {
            ui.set_width(WIDTH);
            editor.toolbar(ui);
        });
        let format_position = initial
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, "Format"))
            .expect("the compact toolbar must render Format")
            .center();
        let mut open_format = viewport_input(WIDTH, HEIGHT);
        append_primary_click(&mut open_format, format_position);
        let format_menu = context.run_ui(open_format, |ui| {
            ui.set_width(WIDTH);
            editor.toolbar(ui);
        });

        let top_level_labels = [
            "Paragraph style",
            "Bold",
            "Italic",
            "Link",
            "Inline code",
            "Bulleted list",
            "Quote",
        ];
        for label in top_level_labels {
            let bounds = accesskit_bounds_starting_with(&format_menu, label);
            assert!(
                bounds.y0 >= 0.0 && bounds.y1 <= f64::from(HEIGHT),
                "compact action `{label}` is outside the minimum viewport: {bounds:?}"
            );
        }
        assert_eq!(
            accesskit_role_and_value(&format_menu, "Paragraph style"),
            (
                egui::accesskit::Role::ComboBox,
                Some("Paragraph".to_owned())
            )
        );

        let style_node = accesskit_node_id(&format_menu, "Paragraph style");
        let mut open_style = viewport_input(WIDTH, HEIGHT);
        open_style.events.push(egui::Event::AccessKitActionRequest(
            egui::accesskit::ActionRequest {
                action: egui::accesskit::Action::Click,
                target_tree: egui::accesskit::TreeId::ROOT,
                target_node: style_node,
                data: None,
            },
        ));
        let style_menu = context.run_ui(open_style, |ui| {
            ui.set_width(WIDTH);
            editor.toolbar(ui);
        });
        for style in BlockStyle::ALL {
            let label = style.label();
            let bounds = accesskit_bounds(&style_menu, label);
            assert!(
                bounds.x0 >= 0.0
                    && bounds.x1 <= f64::from(WIDTH)
                    && bounds.y0 >= 0.0
                    && bounds.y1 <= f64::from(HEIGHT),
                "compact style `{label}` is outside the minimum viewport: {bounds:?}"
            );
        }
    }

    #[test]
    fn paragraph_style_selector_exposes_the_current_style_accessibly() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut editor = MarkdownEditor::default();
        editor.activate_with_selection(0..9, "# Heading".to_owned(), CharSelection::new(2, 9));

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(EXPANDED_FORMAT_MIN_WIDTH);
            editor.toolbar(ui);
        });

        assert!(
            accesskit_bounds(&output, "Paragraph style").x1 <= f64::from(EXPANDED_FORMAT_MIN_WIDTH)
        );
        assert_eq!(
            accesskit_role_and_value(&output, "Paragraph style"),
            (
                egui::accesskit::Role::ComboBox,
                Some("Heading 1".to_owned())
            )
        );
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
    fn inline_formatting_preflights_an_expanded_candidate_before_semantic_parsing() {
        use std::cell::Cell;

        let source = format!("{}\n\n", "x".repeat(4_094)).repeat(256);
        assert_eq!(source.len(), PROTOTYPE_MARKDOWN_MAX_BYTES);
        assert_eq!(markdown_projection_limit(&source), None);
        let end = source.chars().count();

        for command in [
            MarkdownCommand::Bold,
            MarkdownCommand::Italic,
            MarkdownCommand::InlineCode,
            MarkdownCommand::Link,
        ] {
            let result = apply_markdown_command(&source, end - 1..end, command);
            assert_eq!(
                markdown_projection_limit(&result.text),
                Some(MarkdownProjectionLimit::SourceBytes)
            );
            let semantic_parser_called = Cell::new(false);
            assert!(semantic_candidate_is_valid_or_requires_text_fallback(
                &result.text,
                || {
                    semantic_parser_called.set(true);
                    false
                }
            ));
            assert!(!semantic_parser_called.get());
        }
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
    fn toolbar_style_mutation_checks_the_complete_synchronized_document() {
        let retained_blocks = 200;
        let formatted_blocks = 400;
        let prefix = "# retained\n\n".repeat(retained_blocks);
        let draft = "plain line\n".repeat(formatted_blocks);
        let mut source = format!("{prefix}{draft}");
        let mut editor = MarkdownEditor::default();
        let draft_start = prefix.len();
        editor.activate_with_selection(
            draft_start..source.len(),
            draft.clone(),
            CharSelection::new(0, draft.chars().count()),
        );

        editor.apply_toolbar_action(Some(ToolbarAction::BlockStyle(BlockStyle::Heading1)));
        let formatted_draft = &editor.active.as_ref().expect("active block").draft;
        assert_eq!(markdown_projection_limit(formatted_draft), None);
        let context = egui::Context::default();
        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            assert_eq!(
                editor.show(ui, &mut source),
                MarkdownShowOutcome::ProjectionLimitExceeded {
                    limit: MarkdownProjectionLimit::Blocks,
                    origin: EditOrigin::MarkdownFormatting,
                }
            );
        });
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
    fn exact_ceiling_line_renders_without_rescanning_line_prefixes() {
        let source = "x".repeat(PROTOTYPE_MARKDOWN_MAX_LINE_BYTES);

        let projection = markdown_render_projection(&source, &egui::Style::default());

        assert_eq!(projection.job.text, source);
        assert_eq!(
            projection
                .source_map
                .source_span_for_rendered_character
                .len(),
            PROTOTYPE_MARKDOWN_MAX_LINE_BYTES
        );
    }

    #[test]
    fn inline_commands_preserve_unicode_selection_boundaries() {
        let result = apply_markdown_command("caf\u{e9} cafe", 0..4, MarkdownCommand::Bold);

        assert_eq!(result.text, "**caf\u{e9}** cafe");
        assert_eq!(result.selection, 2..6);

        let after_unicode = apply_markdown_command("\u{e9}cafe", 1..5, MarkdownCommand::InlineCode);
        assert_eq!(after_unicode.text, "\u{e9}`cafe`");
        assert_eq!(after_unicode.selection, 2..6);

        let before_unicode =
            apply_markdown_command("cafe\u{e9}", 0..4, MarkdownCommand::InlineCode);
        assert_eq!(before_unicode.text, "`cafe`\u{e9}");
        assert_eq!(before_unicode.selection, 1..5);
    }

    #[test]
    fn empty_inline_selection_inserts_only_markdown_delimiters() {
        let result = apply_markdown_command("Text ", 5..5, MarkdownCommand::Italic);

        assert_eq!(result.text, "Text **");
        assert_eq!(result.selection, 6..6);
    }

    #[test]
    fn inline_commands_toggle_existing_markers_without_losing_the_selection() {
        let bold = apply_markdown_command("cafe", 0..4, MarkdownCommand::Bold);
        let plain = apply_markdown_command(&bold.text, bold.selection, MarkdownCommand::Bold);
        assert_eq!(plain.text, "cafe");
        assert_eq!(plain.selection, 0..4);

        let both = apply_markdown_command("**cafe**", 2..6, MarkdownCommand::Italic);
        assert_eq!(both.text, "***cafe***");
        assert_eq!(both.selection, 3..7);
        let strong = apply_markdown_command(&both.text, both.selection, MarkdownCommand::Italic);
        assert_eq!(strong.text, "**cafe**");
        assert_eq!(strong.selection, 2..6);

        let empty = apply_markdown_command("Text ", 5..5, MarkdownCommand::Bold);
        assert_eq!(empty.text, "Text ****");
        assert_eq!(empty.selection, 7..7);
        let restored =
            apply_markdown_command(&empty.text, empty.selection.clone(), MarkdownCommand::Bold);
        assert_eq!(restored, empty);
    }

    #[test]
    fn inline_commands_do_not_remove_literal_or_malformed_marker_runs() {
        let source = "** text **";
        let selection = 2..8;

        assert!(!markdown_command_is_active(
            source,
            selection.clone(),
            MarkdownCommand::Bold,
        ));
        assert_eq!(
            apply_markdown_command(source, selection.clone(), MarkdownCommand::Bold),
            CommandResult {
                text: source.to_owned(),
                selection,
            }
        );
    }

    #[test]
    fn inline_commands_fail_closed_for_ambiguous_delimiter_depths() {
        for (source, selection, command) in [
            ("``code``", 2..6, MarkdownCommand::InlineCode),
            ("``code`x``", 2..8, MarkdownCommand::InlineCode),
            ("`` `x` ``", 4..5, MarkdownCommand::InlineCode),
            ("****text****", 4..8, MarkdownCommand::Bold),
            ("*****text*****", 5..9, MarkdownCommand::Italic),
        ] {
            assert!(
                !markdown_command_is_active(source, selection.clone(), command,),
                "{source}"
            );
            assert_eq!(
                apply_markdown_command(source, selection.clone(), command),
                CommandResult {
                    text: source.to_owned(),
                    selection,
                }
            );
        }
    }

    #[test]
    fn empty_caret_commands_never_remove_unparsed_literal_delimiters() {
        for (source, selection, command) in [
            ("**", 1..1, MarkdownCommand::Italic),
            ("****", 2..2, MarkdownCommand::Bold),
            ("``", 1..1, MarkdownCommand::InlineCode),
        ] {
            assert!(!markdown_command_is_active(
                source,
                selection.clone(),
                command,
            ));
            assert_eq!(
                apply_markdown_command(source, selection.clone(), command),
                CommandResult {
                    text: source.to_owned(),
                    selection,
                }
            );
        }
    }

    #[test]
    fn empty_link_selection_inserts_blank_label_and_target() {
        let result = apply_markdown_command("before after", 7..7, MarkdownCommand::Link);

        assert_eq!(result.text, "before []()after");
        assert_eq!(result.selection, 8..8);
    }

    #[test]
    fn paragraph_styles_are_exact_idempotent_operations() {
        let source = "one\n## two\n";
        let selection = 0..source.chars().count();

        let heading = apply_block_style(source, selection, BlockStyle::Heading1);
        assert_eq!(heading.text, "# one\n# two\n");
        let repeated = apply_block_style(
            &heading.text,
            heading.selection.clone(),
            BlockStyle::Heading1,
        );
        assert_eq!(repeated, heading);

        let paragraph = apply_block_style(&heading.text, heading.selection, BlockStyle::Paragraph);
        assert_eq!(paragraph.text, "one\ntwo\n");
        let repeated = apply_block_style(
            &paragraph.text,
            paragraph.selection.clone(),
            BlockStyle::Paragraph,
        );
        assert_eq!(repeated, paragraph);
    }

    #[test]
    fn every_atx_heading_level_is_an_exact_idempotent_style() {
        for (level, style) in BlockStyle::ALL.iter().copied().skip(1).enumerate() {
            let level = level + 1;
            let heading = apply_block_style("plain", 0..5, style);
            assert_eq!(heading.text, format!("{} plain", "#".repeat(level)));
            assert_eq!(
                selected_block_style(&heading.text, heading.selection.clone()),
                BlockStyleState::Uniform(style)
            );

            let repeated = apply_block_style(&heading.text, heading.selection.clone(), style);
            assert_eq!(repeated, heading);
        }
    }

    #[test]
    fn paragraph_styles_preserve_native_and_mixed_line_endings() {
        for source in ["one\r\n## two\r\n", "one\rtwo\r", "one\r\n## two\nthree\r"] {
            let selection = 0..source.chars().count();
            let heading = apply_block_style(source, selection, BlockStyle::Heading2);
            let paragraph =
                apply_block_style(&heading.text, heading.selection, BlockStyle::Paragraph);

            assert_eq!(paragraph.text, source.replace("## ", ""));
        }
    }

    #[test]
    fn paragraph_style_state_is_exact_and_reports_mixed_content() {
        assert_eq!(
            selected_block_style("plain", 0..5),
            BlockStyleState::Uniform(BlockStyle::Paragraph)
        );
        assert_eq!(
            selected_block_style("# one\n# two", 0..11),
            BlockStyleState::Uniform(BlockStyle::Heading1)
        );
        assert_eq!(
            selected_block_style("### three", 0..9),
            BlockStyleState::Uniform(BlockStyle::Heading3)
        );
        assert_eq!(
            selected_block_style("###### six", 0..10),
            BlockStyleState::Uniform(BlockStyle::Heading6)
        );
        assert_eq!(
            selected_block_style("# one\ntwo", 0..9),
            BlockStyleState::Mixed
        );
        assert_eq!(
            selected_block_style("### three\n#### four", 0..19),
            BlockStyleState::Mixed
        );
        let mixed_with_code = "plain\n# heading\n```\ncode\n```";
        assert_eq!(
            selected_block_style(mixed_with_code, 0..mixed_with_code.chars().count()),
            BlockStyleState::Unavailable
        );
    }

    #[test]
    fn mixed_style_application_preserves_existing_target_lines_byte_exact() {
        let source = " # keep\n## change";
        let result = apply_block_style(source, 0..source.chars().count(), BlockStyle::Heading1);

        assert_eq!(result.text, " # keep\n# change");
        assert_eq!(
            selected_block_style(&result.text, result.selection.clone()),
            BlockStyleState::Uniform(BlockStyle::Heading1)
        );
    }

    #[test]
    fn choosing_the_current_paragraph_style_preserves_a_backward_selection() {
        let mut active = ActiveBlock::new(0..5, "plain".to_owned(), 1);
        active.selection = CharSelection::new(4, 1);
        active.request_focus = false;

        active.apply_block_style(BlockStyle::Paragraph);

        assert!(!active.dirty);
        assert_eq!(active.pending_origin, None);
        assert!(active.request_focus);
        assert_eq!(active.draft, "plain");
        assert_eq!(active.selection, CharSelection::new(4, 1));
    }

    #[test]
    fn paragraph_styles_follow_parser_verified_atx_syntax() {
        let cases = [
            (" # Heading", BlockStyle::Heading1, "Heading"),
            ("#\tHeading", BlockStyle::Heading1, "Heading"),
            ("## Heading ##", BlockStyle::Heading2, "Heading"),
            ("  ####\tHeading  ### \t", BlockStyle::Heading4, "Heading"),
            ("######", BlockStyle::Heading6, ""),
        ];

        for (source, style, expected_paragraph) in cases {
            let selection = 0..source.chars().count();
            assert_eq!(
                selected_block_style(source, selection.clone()),
                BlockStyleState::Uniform(style),
                "unexpected style for {source:?}"
            );
            assert_eq!(
                apply_block_style(source, selection.clone(), style),
                CommandResult {
                    text: source.to_owned(),
                    selection: selection.clone(),
                },
                "reselecting {style:?} must be byte-exact for {source:?}"
            );
            assert_eq!(
                apply_block_style(source, selection, BlockStyle::Paragraph).text,
                expected_paragraph,
                "demoting {source:?} must remove only parser-owned ATX syntax"
            );
        }
    }

    #[test]
    fn paragraph_styles_fail_closed_outside_top_level_paragraphs_and_atx_headings() {
        for (source, selection) in [
            ("```\n# keep\n```", 4..10),
            ("    # keep", 0..10),
            ("Heading\n=======", 0..15),
            ("> # keep", 0..8),
            ("x ###", 0..5),
            ("#######", 0..7),
            ("trailing ## \t", 0..13),
            ("  leading", 0..9),
        ] {
            assert_eq!(
                selected_block_style(source, selection.clone()),
                BlockStyleState::Unavailable,
                "unsupported structure must be reported for {source:?}"
            );
            assert_eq!(
                apply_block_style(source, selection.clone(), BlockStyle::Paragraph),
                CommandResult {
                    text: source.to_owned(),
                    selection: selection.clone(),
                },
                "unsupported structure must remain byte-exact for {source:?}"
            );
            assert_eq!(
                apply_block_style(source, selection.clone(), BlockStyle::Heading1),
                CommandResult {
                    text: source.to_owned(),
                    selection,
                },
                "unsafe promotion must remain byte-exact for {source:?}"
            );
        }
    }

    #[test]
    fn line_commands_toggle_without_losing_final_newline() {
        let added = apply_markdown_command("one\ntwo\n", 0..7, MarkdownCommand::BulletedList);
        assert_eq!(added.text, "- one\n- two\n");

        let removed = apply_markdown_command(
            &added.text,
            0..added.text.chars().count(),
            MarkdownCommand::BulletedList,
        );
        assert_eq!(removed.text, "one\ntwo\n");
    }

    #[test]
    fn line_commands_change_only_the_selected_logical_lines() {
        let result = apply_markdown_command("one\ntwo\nthree", 4..7, MarkdownCommand::Quote);

        assert_eq!(result.text, "one\n> two\nthree");
        assert_eq!(result.selection, 6..9);
    }

    #[test]
    fn line_commands_preserve_crlf_cr_and_mixed_endings() {
        for source in ["one\r\ntwo\r\n", "one\rtwo\r", "one\r\ntwo\nthree\r"] {
            let added =
                apply_markdown_command(source, 0..source.chars().count(), MarkdownCommand::Quote);
            let removed = apply_markdown_command(
                &added.text,
                0..added.text.chars().count(),
                MarkdownCommand::Quote,
            );

            assert_eq!(removed.text, source);
        }
    }

    #[test]
    fn selected_link_keeps_label_without_inventing_a_target_and_toggles_off() {
        let result = apply_markdown_command("Read Noter", 5..10, MarkdownCommand::Link);

        assert_eq!(result.text, "Read [Noter]()");
        assert_eq!(result.selection, 13..13);

        let plain = apply_markdown_command(&result.text, 6..11, MarkdownCommand::Link);
        assert_eq!(plain.text, "Read Noter");
        assert_eq!(plain.selection, 5..10);

        let balanced = "Read [Noter](https://example.com/a_(b))";
        let plain = apply_markdown_command(balanced, 6..11, MarkdownCommand::Link);
        assert_eq!(plain.text, "Read Noter");
        assert_eq!(plain.selection, 5..10);
    }

    #[test]
    fn link_toggle_ignores_destination_like_text_inside_a_code_span_label() {
        for (source, selection, expected) in [
            ("[a `](` b](https://example.com)", 1..9, "a `](` b"),
            ("[a `[oops` b](https://example.com)", 1..12, "a `[oops` b"),
        ] {
            assert!(markdown_command_is_active(
                source,
                selection.clone(),
                MarkdownCommand::Link,
            ));

            let plain = apply_markdown_command(source, selection, MarkdownCommand::Link);
            assert_eq!(plain.text, expected);
            assert_eq!(plain.selection, 0..expected.chars().count());
        }
    }

    #[test]
    fn link_command_at_an_existing_link_boundary_does_not_remove_the_link() {
        let source = "[Noter](https://example.com)";

        let before = apply_markdown_command(source, 0..0, MarkdownCommand::Link);
        assert_eq!(before.text, "[]()[Noter](https://example.com)");

        let end = source.chars().count();
        let after = apply_markdown_command(source, end..end, MarkdownCommand::Link);
        assert_eq!(after.text, "[Noter](https://example.com)[]()");
    }

    #[test]
    fn unsupported_link_forms_do_not_claim_a_reversible_active_state() {
        let reference = "[Noter][site]\n\n[site]: https://example.com";
        assert!(!markdown_command_is_active(
            reference,
            1..6,
            MarkdownCommand::Link,
        ));
        assert_eq!(
            apply_markdown_command(reference, 1..6, MarkdownCommand::Link).text,
            reference,
        );

        let autolink = "<https://example.com>";
        assert!(!markdown_command_is_active(
            autolink,
            2..10,
            MarkdownCommand::Link,
        ));
        assert_eq!(
            apply_markdown_command(autolink, 2..10, MarkdownCommand::Link).text,
            autolink,
        );
    }

    #[test]
    fn link_command_fails_closed_when_selected_source_breaks_the_label() {
        for source in ["a]b", "a\\"] {
            let selection = 0..source.chars().count();
            assert_eq!(
                apply_markdown_command(source, selection.clone(), MarkdownCommand::Link),
                CommandResult {
                    text: source.to_owned(),
                    selection,
                }
            );
        }
    }

    #[test]
    fn selected_link_target_is_visible_while_it_is_being_edited() {
        let style = egui::Style::default();
        let source = "[Noter](https://example.com)";
        let target_start = source.find("https://").expect("fixture contains a URL");
        let selection = target_start..target_start + "https://example.com".len();
        let reveal = semantic_target_at_selection(source, &selection)
            .expect("the selected URL must be recognized as editable semantic content");

        let job = markdown_edit_layout(source, &style, Some(reveal.clone()));
        let target = job.format_at_byte(egui::text::ByteIndex(reveal.start));

        assert_ne!(target.color, egui::Color32::TRANSPARENT);
        assert!(target.font_id.size > MARKER_FONT_SIZE);
        assert_eq!(target.color, style.visuals.hyperlink_color);
        assert_ne!(target.underline, egui::Stroke::NONE);
    }

    #[test]
    fn empty_link_target_reveals_only_its_editable_parentheses() {
        let style = egui::Style::default();
        let result = apply_markdown_command("Read Noter", 5..10, MarkdownCommand::Link);
        let selection = char_range_to_byte_range(&result.text, result.selection);
        let reveal = semantic_target_at_selection(&result.text, &selection)
            .expect("the empty destination delimiters must be editable");

        assert_eq!(&result.text[reveal.clone()], "()");
        let job = markdown_edit_layout(&result.text, &style, Some(reveal.clone()));
        let opening = job.format_at_byte(egui::text::ByteIndex(reveal.start));
        assert_ne!(opening.color, egui::Color32::TRANSPARENT);
        assert!(opening.font_id.size > MARKER_FONT_SIZE);
        assert_eq!(opening.color, style.visuals.hyperlink_color);
    }

    #[test]
    fn formatting_active_state_matches_markers_and_selected_lines() {
        assert!(markdown_command_is_active(
            "**strong**",
            2..8,
            MarkdownCommand::Bold,
        ));
        assert!(!markdown_command_is_active(
            "**strong**",
            2..8,
            MarkdownCommand::Italic,
        ));
        assert!(markdown_command_is_active(
            "***both***",
            3..7,
            MarkdownCommand::Italic,
        ));
        assert!(markdown_command_is_active(
            "one\n- two\nthree",
            6..9,
            MarkdownCommand::BulletedList,
        ));
        assert!(markdown_command_is_active(
            "Read [Noter]()",
            6..11,
            MarkdownCommand::Link,
        ));
    }

    #[test]
    fn formatting_buttons_expose_stable_accessible_toggle_state() {
        let context = egui::Context::default();
        context.enable_accesskit();
        let mut editor = MarkdownEditor::default();
        editor.activate_with_selection(0..8, "**text**".to_owned(), CharSelection::new(2, 6));

        let output = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            editor.toolbar(ui);
        });

        assert_eq!(
            accesskit_toggled(&output, "Bold"),
            Some(egui::accesskit::Toggled::True)
        );
        assert_eq!(
            accesskit_toggled(&output, "Italic"),
            Some(egui::accesskit::Toggled::False)
        );
    }

    #[test]
    fn focused_editor_shortcuts_use_the_same_reversible_formatting_path() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "text".to_owned();
        editor.activate_with_selection(0..4, source.clone(), CharSelection::new(0, 4));

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert!(!editor.show(ui, &mut source).changed());
        });
        let mut pressed = egui::RawInput::default();
        pressed.events.push(egui::Event::Key {
            key: egui::Key::B,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        let _ = context.run_ui(pressed, |ui| {
            ui.set_width(800.0);
            assert_eq!(
                editor.show(ui, &mut source),
                MarkdownShowOutcome::Changed(EditOrigin::MarkdownFormatting)
            );
        });
        assert_eq!(source, "**text**");

        let mut repeated = egui::RawInput::default();
        repeated.events.push(egui::Event::Key {
            key: egui::Key::B,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: egui::Modifiers::COMMAND,
        });
        let _ = context.run_ui(repeated, |ui| {
            ui.set_width(800.0);
            assert!(!editor.show(ui, &mut source).changed());
        });
        assert_eq!(source, "**text**");

        let mut released = egui::RawInput::default();
        released.events.push(egui::Event::Key {
            key: egui::Key::B,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        let _ = context.run_ui(released, |ui| {
            ui.set_width(800.0);
            assert!(!editor.show(ui, &mut source).changed());
        });

        let mut pressed_again = egui::RawInput::default();
        pressed_again.events.push(egui::Event::Key {
            key: egui::Key::B,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        });
        let _ = context.run_ui(pressed_again, |ui| {
            ui.set_width(800.0);
            assert_eq!(
                editor.show(ui, &mut source),
                MarkdownShowOutcome::Changed(EditOrigin::MarkdownFormatting)
            );
        });
        assert_eq!(source, "text");
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
    fn active_and_rendered_blocks_keep_identical_text_bounds_and_wrapping() {
        let source = "A deliberately long paragraph that wraps at a narrow reading width.";
        let width = 240.0;

        let context = egui::Context::default();
        let mut rendered_editor = MarkdownEditor::default();
        let mut rendered_source = source.to_owned();
        let rendered = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(width);
            assert!(!rendered_editor.show(ui, &mut rendered_source).changed());
        });
        let rendered_bounds = rendered
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, source))
            .expect("the rendered block should emit its text bounds");

        let context = egui::Context::default();
        let mut active_editor = MarkdownEditor::default();
        active_editor.activate_first_block(source);
        let mut active_source = source.to_owned();
        let active = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(width);
            assert!(!active_editor.show(ui, &mut active_source).changed());
        });
        let active_bounds = active
            .shapes
            .iter()
            .find_map(|shape| text_rect(&shape.shape, source))
            .expect("the active block should emit its text bounds");

        assert_eq!(active_source, source);
        assert!(
            (active_bounds.left() - rendered_bounds.left()).abs() <= f32::EPSILON,
            "editing shifted the block horizontally: rendered={rendered_bounds:?}, active={active_bounds:?}"
        );
        assert!(
            (active_bounds.size() - rendered_bounds.size()).length() <= f32::EPSILON,
            "editing changed the block's wrapping: rendered={rendered_bounds:?}, active={active_bounds:?}"
        );
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
    fn rendered_drag_selection_preserves_source_boundaries_and_direction() {
        let first = RenderedSourceCursor {
            block_start: 0,
            rendered_index: 1,
            selection_start: 2,
            selection_end: 1,
        };
        let second = RenderedSourceCursor {
            block_start: 20,
            rendered_index: 4,
            selection_start: 28,
            selection_end: 26,
        };

        let mut drag = RenderedDragSelection::new(first);
        assert_eq!(drag.source_selection(), Selection::caret(2));
        drag.active = second;
        assert_eq!(drag.source_selection(), Selection::new(2, 26));

        let mut drag = RenderedDragSelection::new(second);
        drag.active = first;
        assert_eq!(drag.source_selection(), Selection::new(26, 2));
    }

    #[test]
    fn pointer_gone_cancels_cross_block_drag_while_primary_button_remains_down() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "First\n\nSecond".to_owned();
        let original = source.clone();
        let initial = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            egui::RawInput::default(),
        );
        let start = rendered_text_rect(&initial, "First").center();
        let end = rendered_text_rect(&initial, "Second").center();

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(start, Some(true)),
        );
        let _ = show_markdown_frame(&context, &mut editor, &mut source, pointer_input(end, None));
        assert!(editor.rendered_drag.is_some());

        let mut pointer_gone = egui::RawInput::default();
        pointer_gone.events.push(egui::Event::PointerGone);
        let _ = show_markdown_frame(&context, &mut editor, &mut source, pointer_gone);

        assert!(context.input(|input| { input.pointer.button_down(egui::PointerButton::Primary) }));
        assert!(editor.rendered_drag.is_none());
        assert_eq!(editor.source_selection(), None);
        assert_eq!(source, original);
        assert!(
            !context
                .plugin::<egui::text_selection::LabelSelectionState>()
                .lock()
                .has_selection()
        );
    }

    #[test]
    fn window_focus_loss_cancels_cross_block_drag_while_primary_button_remains_down() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "First\n\nSecond".to_owned();
        let original = source.clone();
        let initial = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            egui::RawInput::default(),
        );
        let start = rendered_text_rect(&initial, "First").center();
        let end = rendered_text_rect(&initial, "Second").center();

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(start, Some(true)),
        );
        let _ = show_markdown_frame(&context, &mut editor, &mut source, pointer_input(end, None));
        assert!(editor.rendered_drag.is_some());

        let mut focus_lost = egui::RawInput::default();
        focus_lost.events.push(egui::Event::WindowFocused(false));
        let _ = show_markdown_frame(&context, &mut editor, &mut source, focus_lost);

        assert!(context.input(|input| { input.pointer.button_down(egui::PointerButton::Primary) }));
        assert!(editor.rendered_drag.is_none());
        assert!(!editor.is_editing());
        assert_eq!(editor.source_selection(), None);
        assert_eq!(source, original);
        assert!(
            !context
                .plugin::<egui::text_selection::LabelSelectionState>()
                .lock()
                .has_selection()
        );

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(end, Some(false)),
        );
        assert!(!editor.is_editing());
        assert_eq!(editor.source_selection(), None);
        assert_eq!(source, original);
    }

    #[test]
    fn touch_release_followed_by_pointer_gone_activates_the_completed_selection() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "First\n\nSecond".to_owned();
        let original = source.clone();
        let ranges = markdown_block_ranges(&source);
        let initial = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            egui::RawInput::default(),
        );
        let first = rendered_text_rect(&initial, "First");
        let second = rendered_text_rect(&initial, "Second");
        let start = egui::pos2(first.left() + 0.1, first.center().y);
        let drag_point = second.center();
        let end = egui::pos2(second.right() - 0.1, second.center().y);

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(start, Some(true)),
        );
        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(drag_point, None),
        );
        assert!(editor.rendered_drag.is_some());

        let mut touch_release = pointer_input(end, Some(false));
        touch_release.events.push(egui::Event::PointerGone);
        let _ = show_markdown_frame(&context, &mut editor, &mut source, touch_release);

        assert!(editor.rendered_drag.is_none());
        assert_eq!(source, original);
        assert_eq!(
            editor.source_selection(),
            Some(Selection::new(ranges[0].start, ranges[1].end))
        );
        assert_eq!(
            editor
                .active
                .as_ref()
                .map(|active| active.source_range.clone()),
            Some(ranges[0].start..ranges[1].end)
        );
    }

    #[test]
    fn drag_autoscroll_is_time_based_bounded_and_directional() {
        let clip = egui::Rect::from_min_max(egui::pos2(0.0, 10.0), egui::pos2(100.0, 110.0));
        let sixty_hz = 1.0 / 60.0;
        let one_twenty_hz = 1.0 / 120.0;
        let sixty_hz_maximum = DRAG_AUTOSCROLL_MAX_SPEED * sixty_hz;
        let delayed_frame_maximum = DRAG_AUTOSCROLL_MAX_SPEED * DRAG_AUTOSCROLL_MAX_FRAME_SECONDS;

        assert!(rendered_drag_scroll_delta(60.0, clip, sixty_hz).abs() <= f32::EPSILON);
        assert!(
            (rendered_drag_scroll_delta(10.0, clip, sixty_hz) - sixty_hz_maximum).abs()
                <= f32::EPSILON
        );
        assert!(
            (rendered_drag_scroll_delta(110.0, clip, sixty_hz) + sixty_hz_maximum).abs()
                <= f32::EPSILON
        );
        assert!(rendered_drag_scroll_delta(20.0, clip, sixty_hz) > 0.0);
        assert!(rendered_drag_scroll_delta(100.0, clip, sixty_hz) < 0.0);
        assert!(rendered_drag_scroll_delta(f32::NAN, clip, sixty_hz).abs() <= f32::EPSILON);
        assert!(rendered_drag_scroll_delta(10.0, clip, f32::NAN).abs() <= f32::EPSILON);
        assert!(rendered_drag_scroll_delta(10.0, clip, 0.0).abs() <= f32::EPSILON);
        assert!(
            (rendered_drag_scroll_delta(10.0, clip, one_twenty_hz) * 2.0
                - rendered_drag_scroll_delta(10.0, clip, sixty_hz))
            .abs()
                <= f32::EPSILON
        );
        assert!(
            (rendered_drag_scroll_delta(10.0, clip, 1.0) - delayed_frame_maximum).abs()
                <= f32::EPSILON
        );
    }

    #[test]
    fn pending_activation_remap_preserves_direction_and_fails_on_overlap() {
        let replaced = 4..8;
        assert_eq!(
            remap_disjoint_selection(Selection::new(12, 2), &replaced, 7),
            Some(Selection::new(15, 2))
        );
        assert_eq!(
            remap_disjoint_selection(Selection::new(12, 2), &replaced, 1),
            Some(Selection::new(9, 2))
        );
        assert_eq!(
            remap_disjoint_selection(Selection::new(6, 2), &replaced, 4),
            None
        );
    }

    #[test]
    fn pointer_drag_selects_forward_across_inactive_markdown_blocks() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "Alpha\r\n\r\nBeta é\n\nGamma".to_owned();
        let original = source.clone();
        let ranges = markdown_block_ranges(&source);
        assert_eq!(ranges.len(), 3);

        let initial = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            egui::RawInput::default(),
        );
        let first = rendered_text_rect(&initial, "Alpha");
        let second = rendered_text_rect(&initial, "Beta é");
        let start = egui::pos2(first.left() + 0.1, first.center().y);
        let end = egui::pos2(second.right() - 0.1, second.center().y);

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(start, Some(true)),
        );
        let _ = show_markdown_frame(&context, &mut editor, &mut source, pointer_input(end, None));
        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(end, Some(false)),
        );

        assert_eq!(source, original);
        assert_eq!(
            editor.source_selection(),
            Some(Selection::new(ranges[0].start, ranges[1].end))
        );
        let active = editor
            .active
            .as_ref()
            .expect("cross-block selection should activate one native editor");
        assert_eq!(active.source_range, ranges[0].start..ranges[1].end);
        assert_eq!(active.draft, &original[ranges[0].start..ranges[1].end]);

        let mut replace = egui::RawInput::default();
        replace.events.push(egui::Event::Text("X".to_owned()));
        let _ = show_markdown_frame(&context, &mut editor, &mut source, replace);
        assert_eq!(source, format!("X{}", &original[ranges[1].end..]));
    }

    #[test]
    fn pointer_drag_selects_backward_across_unicode_and_hidden_syntax() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "**Alpha**\n\n_Béta_\r\n\rGamma".to_owned();
        let original = source.clone();
        let ranges = markdown_block_ranges(&source);
        assert_eq!(ranges.len(), 3);

        let initial = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            egui::RawInput::default(),
        );
        let first = rendered_text_rect(&initial, "Alpha");
        let third = rendered_text_rect(&initial, "Gamma");
        let start = egui::pos2(third.right() - 0.1, third.center().y);
        let end = egui::pos2(first.left() + 0.1, first.center().y);

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(start, Some(true)),
        );
        let _ = show_markdown_frame(&context, &mut editor, &mut source, pointer_input(end, None));
        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(end, Some(false)),
        );

        let selection = editor
            .source_selection()
            .expect("reverse cross-block drag should activate its source selection");
        assert_eq!(source, original);
        assert_eq!(selection.anchor(), ranges[2].end);
        assert_eq!(selection.active(), ranges[0].start + "**".len());
        assert!(selection.anchor() > selection.active());
        assert!(source.is_char_boundary(selection.anchor()));
        assert!(source.is_char_boundary(selection.active()));
    }

    #[test]
    fn cross_block_drag_retires_an_older_active_edit_without_losing_input() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "First\n\nSecond\n\nThird".to_owned();
        assert!(editor.restore_source_selection_with_focus(
            &source,
            Selection::caret("First".len()),
            true,
        ));

        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Text("!".to_owned()));
        let _ = show_markdown_frame(&context, &mut editor, &mut source, input);
        assert_eq!(source, "First!\n\nSecond\n\nThird");
        let ranges = markdown_block_ranges(&source);

        let rendered = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            egui::RawInput::default(),
        );
        let second = rendered_text_rect(&rendered, "Second");
        let third = rendered_text_rect(&rendered, "Third");
        let start = egui::pos2(second.left() + 0.1, second.center().y);
        let end = egui::pos2(third.right() - 0.1, third.center().y);

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(start, Some(true)),
        );
        let _ = show_markdown_frame(&context, &mut editor, &mut source, pointer_input(end, None));
        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(end, Some(false)),
        );

        assert_eq!(source, "First!\n\nSecond\n\nThird");
        assert_eq!(
            editor.source_selection(),
            Some(Selection::new(ranges[1].start, ranges[2].end))
        );
        assert_eq!(
            editor
                .active
                .as_ref()
                .map(|active| active.source_range.clone()),
            Some(ranges[1].start..ranges[2].end)
        );
    }

    #[test]
    fn same_frame_text_commit_remaps_a_new_cross_block_drag() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "First\n\nSecond\n\nThird".to_owned();
        assert!(editor.restore_source_selection_with_focus(
            &source,
            Selection::caret("First".len()),
            true,
        ));

        let rendered = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            egui::RawInput::default(),
        );
        let second = rendered_text_rect(&rendered, "Second");
        let third = rendered_text_rect(&rendered, "Third");
        let start = egui::pos2(second.left() + 0.1, second.center().y);
        let end = egui::pos2(third.right() - 0.1, third.center().y);

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(start, Some(true)),
        );
        assert!(editor.rendered_drag.is_none());

        let mut commit_and_drag = pointer_input(end, None);
        commit_and_drag
            .events
            .insert(0, egui::Event::Text("é".to_owned()));
        let _ = show_markdown_frame(&context, &mut editor, &mut source, commit_and_drag);
        assert_eq!(source, "Firsté\n\nSecond\n\nThird");
        assert!(editor.rendered_drag.is_some());

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(end, Some(false)),
        );

        let ranges = markdown_block_ranges(&source);
        assert_eq!(
            editor.source_selection(),
            Some(Selection::new(ranges[1].start, ranges[2].end))
        );
        assert_eq!(
            editor
                .active
                .as_ref()
                .map(|active| active.source_range.clone()),
            Some(ranges[1].start..ranges[2].end)
        );
        let selection = editor
            .source_selection()
            .expect("remapped drag should remain actionable");
        assert!(source.is_char_boundary(selection.anchor()));
        assert!(source.is_char_boundary(selection.active()));
    }

    #[test]
    fn escape_cancels_cross_block_drag_without_residual_selection() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "First\n\nSecond".to_owned();
        let original = source.clone();
        let initial = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            egui::RawInput::default(),
        );
        let first = rendered_text_rect(&initial, "First");
        let second = rendered_text_rect(&initial, "Second");
        let start = first.center();
        let end = second.center();

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(start, Some(true)),
        );
        let _ = show_markdown_frame(&context, &mut editor, &mut source, pointer_input(end, None));
        assert!(editor.rendered_drag.is_some());

        let mut cancel = pointer_input(end, None);
        cancel.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = show_markdown_frame(&context, &mut editor, &mut source, cancel);
        assert!(editor.rendered_drag.is_none());
        assert!(!editor.is_editing());
        assert_eq!(editor.source_selection(), None);
        assert_eq!(source, original);
        assert!(
            !context
                .plugin::<egui::text_selection::LabelSelectionState>()
                .lock()
                .has_selection()
        );

        let _ = show_markdown_frame(
            &context,
            &mut editor,
            &mut source,
            pointer_input(end, Some(false)),
        );
        assert!(!editor.is_editing());
        assert_eq!(source, original);
    }

    #[test]
    fn cross_block_drag_keeps_native_scroll_selection_moving() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = (1..=30)
            .map(|index| format!("Block {index:02}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let original = source.clone();
        let scroll_offset = std::cell::Cell::new(0.0);
        {
            let mut show = |input| {
                context.run_ui(input, |ui| {
                    ui.set_width(320.0);
                    scroll_offset.set(
                        egui::ScrollArea::vertical()
                            .id_salt("cross-block-drag-scroll")
                            .max_height(96.0)
                            .show(ui, |ui| editor.show(ui, &mut source))
                            .state
                            .offset
                            .y,
                    );
                })
            };

            let initial = show(egui::RawInput::default());
            let first = rendered_text_rect(&initial, "Block 01");
            let start = egui::pos2(first.left() + 2.0, first.center().y);
            let end = egui::pos2(first.left() + 36.0, first.top() + 92.0);
            let _ = show(pointer_input(start, Some(true)));
            for _ in 0..6 {
                let _ = show(pointer_input(end, None));
            }

            assert!(
                scroll_offset.get() > 0.0,
                "native selection should advance the scroll area"
            );
            let _ = show(pointer_input(end, Some(false)));
        }

        let selection = editor
            .source_selection()
            .expect("scrolled drag should activate its source selection");
        assert!(selection.active() > "Block 01".len());
        assert_eq!(source, original);
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
        assert!(editor.restore_source_selection_with_focus(&source, selected, true));

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

        assert!(is_quote_marker('>', true, true));
        assert_eq!(
            markdown_render_layout(source, &egui::Style::default()).text,
            "paragraph\nquoted line"
        );
        assert!(!is_quote_marker('>', false, true));
        assert!(!is_quote_marker('>', true, false));
    }

    #[test]
    fn block_marker_projection_tracks_indentation_across_native_line_endings() {
        let source = "- first\r\n  * second\rparagraph * prose\n  > nested";

        assert_eq!(
            markdown_render_layout(source, &egui::Style::default()).text,
            "• first\r\n  • second\rparagraph * prose\n  nested"
        );
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
        let body_size = style.text_styles[&egui::TextStyle::Body].size;
        let expected_sizes = HEADING_SIZE_RATIOS.map(|ratio| body_size * ratio);

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
        assert!(
            body.line_height
                .is_some_and(|line_height| line_height > body.font_id.size)
        );
    }

    #[test]
    fn markdown_line_height_preserves_heading_hierarchy_and_code_rhythm() {
        let style = egui::Style::default();
        let heading = markdown_edit_layout("# Heading", &style, None)
            .format_at_byte(egui::text::ByteIndex(2))
            .clone();
        let body = markdown_edit_layout("Body", &style, None)
            .format_at_byte(egui::text::ByteIndex(0))
            .clone();
        let code = markdown_edit_layout("`code`", &style, None)
            .format_at_byte(egui::text::ByteIndex(1))
            .clone();

        assert!(heading.font_id.size > body.font_id.size);
        assert!(heading.line_height > body.line_height);
        assert_eq!(code.line_height, body.line_height);
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
            finished_selection: None,
            next_editor_serial: 1,
            rendered_drag: None,
            input_was_limited: false,
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

        assert!(editor.restore_source_selection_with_focus(source, selection, true));
        assert_eq!(editor.source_selection(), Some(selection));
        assert!(editor.is_editing());
        let active = editor.active.as_ref().expect("selection should be active");
        assert_eq!(active.source_range, 0.."# é".len());
        assert_eq!(active.draft, "# é");

        editor.reset();
        assert!(editor.restore_source_selection_with_focus(source, selection, false));
        assert_eq!(editor.source_selection(), Some(selection));
        assert!(
            editor
                .active
                .as_ref()
                .is_some_and(|active| !active.request_focus)
        );
    }

    #[test]
    fn source_selection_restoration_accepts_cross_block_and_rejects_invalid_ranges() {
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
        assert!(MarkdownEditor::can_restore_source_selection(
            source, selected
        ));
        assert!(editor.restore_source_selection_with_focus(source, selected, true));
        assert_eq!(editor.source_selection(), Some(selected));

        let cross_block = Selection::new(source.len(), 2);
        editor.reset();
        assert!(MarkdownEditor::can_restore_source_selection(
            source,
            cross_block
        ));
        assert!(editor.restore_source_selection_with_focus(source, cross_block, true));
        assert_eq!(editor.source_selection(), Some(cross_block));
        let active = editor.active.as_ref().expect("selection should be active");
        assert_eq!(active.source_range, 0..source.len());
        assert_eq!(active.draft, source);

        for boundary_selection in [
            Selection::new(0, second_start),
            Selection::new(second_start, 0),
        ] {
            editor.reset();
            assert!(editor.restore_source_selection_with_focus(source, boundary_selection, false,));
            assert_eq!(editor.source_selection(), Some(boundary_selection));
            let active = editor
                .active
                .as_ref()
                .expect("boundary selection should be active");
            assert_eq!(active.source_range, 0..second_start);
            assert_eq!(active.draft, &source[..second_start]);
        }

        let separator_caret = Selection::caret(second_start - 1);
        editor.reset();
        assert!(editor.restore_source_selection_with_focus(source, separator_caret, false));
        assert_eq!(editor.source_selection(), Some(separator_caret));
        let active = editor
            .active
            .as_ref()
            .expect("separator caret should be active");
        assert_eq!(active.source_range.end, separator_caret.active());
        assert!(active.source_range.end < source.len());

        for invalid in [
            Selection::new(0, source.len() + 1),
            Selection::new(unicode_start, unicode_start + 1),
            Selection::new(unicode_start + 1, unicode_end),
        ] {
            editor.reset();
            assert!(!MarkdownEditor::can_restore_source_selection(
                source, invalid
            ));
            assert!(!editor.restore_source_selection_with_focus(source, invalid, true));
            assert!(!editor.is_editing());
        }
    }

    #[test]
    fn cross_block_selection_replacement_changes_only_selected_source() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        let mut source = "# é\r\n\r\nSecond".to_owned();
        let selection = Selection::new(source.len(), "# ".len());
        assert!(editor.restore_source_selection_with_focus(&source, selection, true));

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(800.0);
            assert_eq!(editor.show(ui, &mut source), MarkdownShowOutcome::Unchanged);
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

        assert_eq!(source, "# X");
        assert_eq!(editor.source_selection(), Some(Selection::caret(3)));
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
            finished_selection: None,
            next_editor_serial: 1,
            rendered_drag: None,
            input_was_limited: false,
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
            finished_selection: None,
            next_editor_serial: 1,
            rendered_drag: None,
            input_was_limited: false,
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
            finished_selection: None,
            next_editor_serial: 1,
            rendered_drag: None,
            input_was_limited: false,
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
            finished_selection: None,
            next_editor_serial: 1,
            rendered_drag: None,
            input_was_limited: false,
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
    fn same_frame_input_commits_before_escape_finishes_editing() {
        let escape = || egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let cases = [
            (
                "text",
                egui::Event::Text("x".to_owned()),
                EditOrigin::MarkdownInput,
            ),
            (
                "paste",
                egui::Event::Paste("x".to_owned()),
                EditOrigin::Paste,
            ),
            (
                "IME commit",
                egui::Event::Ime(egui::ImeEvent::Commit("x".to_owned())),
                EditOrigin::MarkdownInput,
            ),
        ];

        for (label, edit_event, expected_origin) in cases {
            for escape_first in [false, true] {
                let context = egui::Context::default();
                let mut active = ActiveBlock::new(0..6, "abcXYZ".to_owned(), 1);
                active.selection = CharSelection::new(0, 3);
                let mut editor = MarkdownEditor {
                    active: Some(active),
                    finished_selection: None,
                    next_editor_serial: 1,
                    rendered_drag: None,
                    input_was_limited: false,
                };
                let mut source = "abcXYZ".to_owned();

                let _ = context.run_ui(egui::RawInput::default(), |ui| {
                    ui.set_width(800.0);
                    assert_eq!(editor.show(ui, &mut source), MarkdownShowOutcome::Unchanged);
                });

                let mut input = egui::RawInput::default();
                if escape_first {
                    input.events.push(escape());
                }
                input.events.push(edit_event.clone());
                if !escape_first {
                    input.events.push(escape());
                }
                let _ = context.run_ui(input, |ui| {
                    ui.set_width(800.0);
                    assert_eq!(
                        editor.show(ui, &mut source),
                        MarkdownShowOutcome::Changed(expected_origin),
                        "{label} must commit regardless of same-frame event order"
                    );
                });

                assert_eq!(
                    source, "xXYZ",
                    "{label} must reach source regardless of same-frame event order"
                );
                assert_eq!(
                    editor.source_selection(),
                    Some(Selection::caret(1)),
                    "{label} must carry the final caret through Escape"
                );
                assert!(
                    !editor.is_editing(),
                    "Escape must finish editing after the {label} commit"
                );
            }
        }
    }

    #[test]
    fn modified_escape_never_finishes_active_editing() {
        for (label, modifiers) in [
            ("Control", egui::Modifiers::CTRL),
            ("Alt", egui::Modifiers::ALT),
            ("Shift", egui::Modifiers::SHIFT),
            ("Command", egui::Modifiers::COMMAND),
            ("Mac Command", egui::Modifiers::MAC_CMD),
        ] {
            let context = egui::Context::default();
            let active = ActiveBlock::new(0..3, "# A".to_owned(), 1);
            let mut editor = MarkdownEditor {
                active: Some(active),
                finished_selection: None,
                next_editor_serial: 1,
                rendered_drag: None,
                input_was_limited: false,
            };
            let mut source = "# A".to_owned();

            let _ = context.run_ui(egui::RawInput::default(), |ui| {
                ui.set_width(800.0);
                assert_eq!(editor.show(ui, &mut source), MarkdownShowOutcome::Unchanged);
            });

            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            });
            let _ = context.run_ui(input, |ui| {
                ui.set_width(800.0);
                assert_eq!(
                    editor.show(ui, &mut source),
                    MarkdownShowOutcome::Unchanged,
                    "{label}+Escape must not finish the active editor"
                );
            });

            assert_eq!(source, "# A");
            assert!(
                editor.is_editing(),
                "{label}+Escape must leave the active editor open"
            );
        }
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

    #[test]
    fn active_editor_bounds_paste_before_layout_and_reports_the_limit() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        editor.activate(0..4, "text".to_owned());
        let mut source = "text".to_owned();

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(640.0);
            assert!(
                !editor
                    .show_with_source_byte_limit(ui, &mut source, 8)
                    .changed()
            );
        });
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Paste("12345é".to_owned()));
        let _ = context.run_ui(input, |ui| {
            ui.set_width(640.0);
            assert!(
                editor
                    .show_with_source_byte_limit(ui, &mut source, 8)
                    .changed()
            );
        });

        assert_eq!(source, "text1234");
        assert!(editor.take_input_was_limited());
        assert!(!editor.take_input_was_limited());
    }

    #[test]
    fn over_budget_active_layout_fails_closed_to_one_plain_text_section() {
        let source = "*x* ".repeat(3_000);
        assert_eq!(
            markdown_projection_limit(&source),
            Some(MarkdownProjectionLimit::ParserEvents)
        );

        let layout = active_edit_layout(
            &source,
            &egui::Style::default(),
            source.len()..source.len(),
            640.0,
        );

        assert_eq!(
            layout.projection_limit,
            Some(MarkdownProjectionLimit::ParserEvents)
        );
        assert_eq!(layout.job.text, source);
        assert_eq!(layout.job.sections.len(), 1);
    }

    #[test]
    fn same_frame_adversarial_paste_reports_the_limit_after_bounded_layout() {
        let context = egui::Context::default();
        let mut editor = MarkdownEditor::default();
        editor.activate(0..4, "text".to_owned());
        let mut source = "text".to_owned();

        let _ = context.run_ui(egui::RawInput::default(), |ui| {
            ui.set_width(640.0);
            assert_eq!(editor.show(ui, &mut source), MarkdownShowOutcome::Unchanged);
        });
        let paste = "*x* ".repeat(3_000);
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Paste(paste.clone()));
        let mut outcome = MarkdownShowOutcome::Unchanged;
        let _ = context.run_ui(input, |ui| {
            ui.set_width(640.0);
            outcome = editor.show(ui, &mut source);
        });

        assert_eq!(
            outcome,
            MarkdownShowOutcome::ProjectionLimitExceeded {
                limit: MarkdownProjectionLimit::ParserEvents,
                origin: EditOrigin::Paste,
            }
        );
        assert_eq!(source, format!("text{paste}"));
    }
}
