//! Latest-save diagnostics, using the core's exact retained-artifact messages.

use super::{TuiPalette, TuiSession, push_bar};

pub(super) fn append_diagnostic(lines: &mut Vec<String>, message: &str, columns: usize) {
    // At most two terminal cells per scalar. push_bar applies the shared
    // control-character sanitizer after wrapping.
    let characters: Vec<char> = message.chars().collect();
    for chunk in characters.chunks(columns.saturating_sub(2).max(2) / 2) {
        let mut line = String::from(" ");
        line.extend(chunk);
        lines.push(line);
    }
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
    let mut lines = Vec::new();
    if let Some(warnings) = &session.last_save_warnings {
        for warning in warnings.cleanup().iter().chain(warnings.durability()) {
            append_diagnostic(&mut lines, warning.message(), columns);
            lines.push(String::new());
        }
    }
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
    push_bar(
        &mut out,
        " Up/Down scroll | Enter/Esc close | Closing does not remove files ",
        columns,
        palette.bar_bg,
        palette.bar_fg,
        true,
    );
    out
}
