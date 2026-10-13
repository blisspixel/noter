//! Latest-save diagnostics, using the core's exact retained-artifact messages.

use super::{TuiPalette, TuiSession, push_bar};
use noter::core::terminal_text::{cell_width, sanitize_line};

pub(super) fn append_diagnostic(lines: &mut Vec<String>, message: &str, columns: usize) {
    let mut line = String::from(" ");
    let mut column = 1;
    for character in sanitize_line(message).chars() {
        let width = cell_width(character, column);
        if column + width > columns.max(3) {
            lines.push(line);
            line = String::from(" ");
            column = 1;
        }
        line.push(character);
        column += width;
    }
    lines.push(line);
}

pub(super) fn lines(session: &TuiSession, columns: usize) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(warnings) = &session.last_save_warnings {
        for warning in warnings.cleanup().iter().chain(warnings.durability()) {
            append_diagnostic(&mut lines, warning.message(), columns);
            lines.push(String::new());
        }
    }
    lines
}

pub(super) fn render(session: &TuiSession, cols: u16, rows: u16, palette: &TuiPalette) -> String {
    let columns = usize::from(cols);
    let height = usize::from(rows);
    let mut out = String::with_capacity(columns * height * 4);
    out.push_str("\x1b[?25l\x1b[H");
    push_bar(
        &mut out,
        " Latest save warnings ",
        columns,
        palette.bar_bg,
        palette.bar_fg,
        true,
    );
    out.push_str("\r\n");
    let lines = lines(session, columns);
    let viewport = height.saturating_sub(2);
    let start = session
        .reconcile_scroll
        .min(lines.len().saturating_sub(viewport));
    for row in 0..viewport {
        push_bar(
            &mut out,
            lines.get(start + row).map_or("", String::as_str),
            columns,
            palette.bg,
            palette.fg,
            false,
        );
        out.push_str("\r\n");
    }
    let hint = if session.retained_removal_is_armed() {
        " x again removes the verified sibling | Esc cancels "
    } else {
        " Up/Down scroll | x then x removes a verified sibling | Enter/Esc close | Closing does not remove files "
    };
    push_bar(
        &mut out,
        hint,
        columns,
        palette.bar_bg,
        palette.bar_fg,
        true,
    );
    out
}
