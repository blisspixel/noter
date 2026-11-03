//! Semantic terminal paint.
//!
//! The host terminal chooses the colors. Noter selects a role and never emits
//! a 24-bit color sequence. A non-empty `NO_COLOR` suppresses color and leaves
//! bold, dim, italic, and reverse, as <https://no-color.org/> specifies.
//! An empty `NO_COLOR` does not count.

use std::ffi::OsStr;

use noter::core::terminal_text::fit_line;

/// Whether `NO_COLOR` disables color.
///
/// The variable counts only when it is present and not empty.
#[must_use]
pub(super) fn no_color_is_set(value: Option<&OsStr>) -> bool {
    value.is_some_and(|value| !value.is_empty())
}

/// Reads `NO_COLOR` once for a new session.
#[must_use]
pub(super) fn no_color_from_env() -> bool {
    no_color_is_set(std::env::var_os("NO_COLOR").as_deref())
}

/// Color policy for one frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Paint {
    no_color: bool,
}

impl Paint {
    #[must_use]
    pub(super) const fn new(no_color: bool) -> Self {
        Self { no_color }
    }

    /// Resets the cell, then opens `role`.
    pub(super) fn open(self, out: &mut String, role: Role) {
        out.push_str("\x1b[0m");
        if let Some(sequence) = self.sequence(role) {
            out.push_str(sequence);
        }
    }

    const fn sequence(self, role: Role) -> Option<&'static str> {
        match role {
            Role::Text => None,
            Role::Accent => Some(if self.no_color { "\x1b[1m" } else { "\x1b[36m" }),
            Role::AccentStrong => Some(if self.no_color {
                "\x1b[1m"
            } else {
                "\x1b[1;36m"
            }),
            Role::Muted => Some("\x1b[2m"),
            Role::Match => Some("\x1b[7m"),
            Role::Warning => {
                if self.no_color {
                    None
                } else {
                    Some("\x1b[33m")
                }
            }
            Role::Error => {
                if self.no_color {
                    None
                } else {
                    Some("\x1b[31m")
                }
            }
            Role::Success => {
                if self.no_color {
                    None
                } else {
                    Some("\x1b[32m")
                }
            }
            Role::Quote => Some("\x1b[3m"),
            Role::BarStrong => Some("\x1b[1;7m"),
            Role::BarMuted => Some("\x1b[2;7m"),
        }
    }
}

/// One drawing role. None of these sequences use `38;2` or `48;2`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Role {
    /// Terminal default foreground and background.
    Text,
    /// Cyan. Bold when color is off, so the role remains visible.
    Accent,
    /// Bold cyan, or bold when color is off.
    AccentStrong,
    /// Dim. Used for line numbers and shortcut labels.
    Muted,
    /// Reverse video. Find matches stay reverse when color is off.
    Match,
    /// Yellow durability or storage warning. Plain when color is off.
    Warning,
    /// Red failed save. Plain when color is off.
    Error,
    /// Green committed save. Plain when color is off.
    Success,
    /// Italic quotation. Italic is not a color.
    Quote,
    /// Bold reverse video bar.
    BarStrong,
    /// Dim reverse video, for the shortcut bars.
    BarMuted,
}

/// Draws `text` in `role`, padded or cut to exactly `columns` cells, then resets.
pub(super) fn push_bar(out: &mut String, text: &str, columns: usize, paint: Paint, role: Role) {
    paint.open(out, role);
    out.push_str(&fit_line(text, columns));
    out.push_str("\x1b[0m");
}

/// Reports whether a frame contains a 24-bit color sequence.
#[cfg(test)]
#[must_use]
pub(super) fn contains_truecolor(frame: &str) -> bool {
    frame.contains("38;2") || frame.contains("48;2")
}

/// Reports whether an SGR sequence sets a color.
///
/// Cursor, erase, and mode sequences are ignored. The forbidden parameters are
/// the standard and bright foreground and background colors, plus the `38` and
/// `48` introducers used by 256-color and 24-bit color.
#[cfg(test)]
#[must_use]
pub(super) fn contains_color_sgr(frame: &str) -> bool {
    for parameters in sgr_parameters(frame) {
        for parameter in parameters.split(';') {
            if parameter.is_empty() {
                continue;
            }
            let Ok(code) = parameter.parse::<u16>() else {
                return true;
            };
            if is_color_sgr(code) {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
const fn is_color_sgr(code: u16) -> bool {
    matches!(code, 30..=37 | 90..=97 | 38 | 40..=47 | 48)
}

#[cfg(test)]
fn sgr_parameters(frame: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let bytes = frame.as_bytes();
    let mut index = 0;
    while index + 2 < bytes.len() {
        if bytes[index] != 0x1b || bytes[index + 1] != b'[' {
            index += 1;
            continue;
        }
        let start = index + 2;
        let mut end = start;
        while end < bytes.len() && !bytes[end].is_ascii_alphabetic() {
            end += 1;
        }
        if end < bytes.len() && bytes[end] == b'm' {
            found.push(&frame[start..end]);
        }
        index = end.saturating_add(1);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_no_color_does_not_disable_color() {
        assert!(!no_color_is_set(None));
        assert!(!no_color_is_set(Some(OsStr::new(""))));
        assert!(no_color_is_set(Some(OsStr::new("1"))));
        assert!(no_color_is_set(Some(OsStr::new("0"))));
    }

    #[test]
    fn colored_roles_avoid_truecolor_and_keep_their_codes() {
        let paint = Paint::new(false);
        let mut frame = String::new();
        for role in [
            Role::Accent,
            Role::AccentStrong,
            Role::Muted,
            Role::Match,
            Role::Warning,
            Role::Error,
            Role::Success,
            Role::Quote,
            Role::BarStrong,
            Role::BarMuted,
        ] {
            paint.open(&mut frame, role);
        }
        assert!(!contains_truecolor(&frame));
        assert!(frame.contains("\x1b[36m"));
        assert!(frame.contains("\x1b[1;36m"));
        assert!(frame.contains("\x1b[2m"));
        assert!(frame.contains("\x1b[7m"));
        assert!(frame.contains("\x1b[33m"));
        assert!(frame.contains("\x1b[31m"));
        assert!(frame.contains("\x1b[32m"));
        assert!(frame.contains("\x1b[3m"));
        assert!(frame.contains("\x1b[1;7m"));
        assert!(frame.contains("\x1b[2;7m"));
    }

    #[test]
    fn no_color_omits_color_and_keeps_emphasis() {
        let paint = Paint::new(true);
        let mut frame = String::new();
        for role in [
            Role::Text,
            Role::Accent,
            Role::AccentStrong,
            Role::Muted,
            Role::Match,
            Role::Warning,
            Role::Error,
            Role::Success,
            Role::Quote,
            Role::BarStrong,
            Role::BarMuted,
        ] {
            paint.open(&mut frame, role);
            frame.push('x');
        }
        assert!(!contains_truecolor(&frame));
        assert!(!contains_color_sgr(&frame));
        assert!(frame.contains("\x1b[1m"));
        assert!(frame.contains("\x1b[2m"));
        assert!(frame.contains("\x1b[3m"));
        assert!(frame.contains("\x1b[7m"));
        assert!(!frame.contains("\x1b[39m"));
        assert!(!frame.contains("\x1b[49m"));
    }

    #[test]
    fn color_scan_ignores_cursor_sequences() {
        assert!(!contains_color_sgr("\x1b[2;5H\x1b[?25h"));
        assert!(contains_color_sgr("\x1b[1;36m"));
        assert!(contains_color_sgr("\x1b[48;2;1;2;3m"));
        assert!(contains_truecolor("\x1b[48;2;1;2;3m"));
    }
}
