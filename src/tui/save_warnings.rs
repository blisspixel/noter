//! Latest-save diagnostics, using the core's exact retained-artifact messages.

use super::TuiSession;
use super::paint::{Paint, Role, push_bar};
use noter::core::terminal_text::{cell_width, sanitize_line};

pub(super) fn append_diagnostic(lines: &mut Vec<String>, message: &str, columns: usize) {
    let text = sanitize_line(message);
    let limit = columns.max(3);
    let mut line = String::from(" ");
    let mut column = 1usize;
    let mut last_space: Option<usize> = None;

    for character in text.chars() {
        let mut width = cell_width(character, column);
        if column + width > limit && column > 1 {
            if let Some(index) = last_space {
                let space_end = index + 1;
                let rest = line[space_end..].to_owned();
                line.truncate(space_end);
                lines.push(std::mem::take(&mut line));
                line = String::from(" ");
                line.push_str(&rest);
                column = column_after_indent(&rest);
                last_space = None;
                width = cell_width(character, column);
                if column + width > limit && column > 1 {
                    lines.push(std::mem::take(&mut line));
                    line = String::from(" ");
                    column = 1;
                    width = cell_width(character, column);
                }
            } else {
                lines.push(std::mem::take(&mut line));
                line = String::from(" ");
                column = 1;
                width = cell_width(character, column);
            }
        }
        line.push(character);
        if character == ' ' {
            last_space = Some(line.len() - 1);
        }
        column += width;
    }
    lines.push(line);
}

fn column_after_indent(text: &str) -> usize {
    let mut column = 1usize;
    for character in text.chars() {
        column += cell_width(character, column);
    }
    column
}

pub(super) fn lines(session: &TuiSession, columns: usize) -> Vec<String> {
    let mut lines = Vec::new();
    if session.recovery.is_unavailable() {
        append_diagnostic(&mut lines, &session.recovery.unavailable_message(), columns);
        lines.push(String::new());
    }
    if let Some(warnings) = &session.last_save_warnings {
        for warning in warnings.cleanup().iter().chain(warnings.durability()) {
            append_diagnostic(&mut lines, warning.message(), columns);
            lines.push(String::new());
        }
    }
    lines
}

pub(super) fn render(session: &TuiSession, cols: u16, rows: u16, paint: Paint) -> String {
    let columns = usize::from(cols);
    let height = usize::from(rows);
    let mut out = String::with_capacity(columns * height * 4);
    out.push_str("\x1b[?25l\x1b[H");
    push_bar(
        &mut out,
        " Latest save warnings ",
        columns,
        paint,
        Role::BarStrong,
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
            paint,
            Role::Warning,
        );
        out.push_str("\r\n");
    }
    let hint = if session.retained_removal_is_armed() {
        " x again removes the verified sibling | Esc cancels "
    } else {
        " Closing does not remove files | x x removes verified sibling | Esc cancels "
    };
    push_bar(&mut out, hint, columns, paint, Role::BarStrong);
    out
}
