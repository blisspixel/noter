//! Terminal-safe display of untrusted text.
//!
//! A terminal interprets control characters in its output stream. Document
//! text, file names, and typed input can all carry them, and a file that
//! reaches the terminal verbatim can move the cursor, rewrite the window
//! title, or set the system clipboard through OSC 52. Every character drawn
//! from untrusted text goes through `push_display`, which replaces anything
//! the terminal could interpret with a visible substitute.
//!
//! Columns here are terminal cells. Wide East Asian characters take two cells
//! and combining marks take none, following Unicode Standard Annex #11 as
//! implemented by `unicode-width`.

use unicode_width::UnicodeWidthChar;

/// Column interval between tab stops when a tab character is drawn.
pub const TAB_STOP: usize = 4;

/// The visible substitute for a character the terminal must not receive.
pub const REPLACEMENT: char = '\u{FFFD}';

/// Returns whether drawing `character` could change terminal state or
/// reorder surrounding text instead of showing a glyph.
///
/// This covers the C0 and C1 control ranges, DEL, the Unicode line and
/// paragraph separators, and the bidirectional formatting characters that
/// can make displayed text read differently from its source order.
pub const fn is_terminal_unsafe(character: char) -> bool {
    matches!(
        character,
        '\u{0}'..='\u{1F}'
            | '\u{7F}'..='\u{9F}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{061C}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
    )
}

/// Returns the cells `character` occupies when drawn starting at `column`.
pub fn cell_width(character: char, column: usize) -> usize {
    if character == '\t' {
        TAB_STOP - column % TAB_STOP
    } else if is_terminal_unsafe(character) {
        1
    } else {
        character.width().unwrap_or(1)
    }
}

/// Appends `character` as it is drawn at `column` and returns its width.
///
/// Tabs become spaces to the next tab stop. Other unsafe characters become
/// [`REPLACEMENT`]. Nothing that reaches `out` is a terminal control.
pub fn push_display(out: &mut String, character: char, column: usize) -> usize {
    let width = cell_width(character, column);
    if character == '\t' {
        out.extend(std::iter::repeat_n(' ', width));
    } else if is_terminal_unsafe(character) {
        out.push(REPLACEMENT);
    } else {
        out.push(character);
    }
    width
}

/// Returns `text` with every unsafe character replaced, for one-line labels
/// such as file names, status messages, and prompt input.
pub fn sanitize_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut column = 0;
    for character in text.chars() {
        column += push_display(&mut out, character, column);
    }
    out
}

/// Returns `text` sanitized and cut or padded to exactly `width` cells.
///
/// A wide character that would straddle the last cell is replaced by a
/// space, so the result never draws past `width`.
pub fn fit_line(text: &str, width: usize) -> String {
    let mut out = String::with_capacity(width);
    let mut column = 0;
    for character in text.chars() {
        let cells = cell_width(character, column);
        if column + cells > width {
            break;
        }
        column += push_display(&mut out, character, column);
    }
    out.extend(std::iter::repeat_n(' ', width - column));
    out
}

/// Returns the cells `text` occupies when drawn from column zero.
pub fn display_width(text: &str) -> usize {
    text.chars().fold(0, |column, character| {
        column + cell_width(character, column)
    })
}

/// Returns the column where the character at byte `offset` of `line` starts.
///
/// An offset past the end, or inside a character, counts every character
/// that starts before it.
pub fn column_of(line: &str, offset: usize) -> usize {
    line.char_indices()
        .take_while(|(start, _)| *start < offset)
        .fold(0, |column, (_, character)| {
            column + cell_width(character, column)
        })
}

/// Returns the byte offset of the character drawn at `column` of `line`.
///
/// A column inside a wide character or a tab resolves to that character's
/// start, and a column past the end resolves to the end of the line, so the
/// result is always a character boundary.
pub fn offset_at_column(line: &str, column: usize) -> usize {
    let mut start_column = 0;
    for (offset, character) in line.char_indices() {
        let width = cell_width(character, start_column);
        if column < start_column + width {
            return offset;
        }
        start_column += width;
    }
    line.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn control_and_reordering_characters_are_unsafe() {
        for character in [
            '\u{0}', '\u{7}', '\u{1B}', '\u{7F}', '\u{80}', '\u{9B}', '\u{9F}', '\u{2028}',
            '\u{2029}', '\u{061C}', '\u{200E}', '\u{200F}', '\u{202A}', '\u{202E}', '\u{2066}',
            '\u{2069}',
        ] {
            assert!(is_terminal_unsafe(character), "{character:?}");
        }
        for character in [
            'a', ' ', '\u{A0}', 'é', '世', '\u{200D}', '\u{2030}', '\u{206A}',
        ] {
            assert!(!is_terminal_unsafe(character), "{character:?}");
        }
    }

    #[test]
    fn escape_sequences_are_shown_not_executed() {
        let osc52 = "before\u{1B}]52;c;ZWNobyBvd25lZA==\u{7}after";
        let shown = sanitize_line(osc52);
        assert_eq!(shown, "before\u{FFFD}]52;c;ZWNobyBvd25lZA==\u{FFFD}after");
        assert_eq!(sanitize_line("\u{9B}31m"), "\u{FFFD}31m");
    }

    #[test]
    fn tabs_expand_to_the_next_stop() {
        assert_eq!(sanitize_line("\tx"), "    x");
        assert_eq!(sanitize_line("ab\tx"), "ab  x");
        assert_eq!(sanitize_line("abcd\tx"), "abcd    x");
        assert_eq!(display_width("ab\tx"), 5);
    }

    #[test]
    fn wide_and_combining_characters_use_their_cell_widths() {
        assert_eq!(display_width("世界"), 4);
        assert_eq!(display_width("e\u{301}"), 1);
        assert_eq!(display_width("a\u{1B}b"), 3);
        // Bidirectional controls are zero width to unicode-width, but they are
        // drawn as a one-cell replacement and must be measured that way.
        for control in ['\u{200E}', '\u{202E}', '\u{2066}', '\u{061C}'] {
            assert_eq!(display_width(&control.to_string()), 1, "{control:?}");
        }
    }

    #[test]
    fn columns_and_offsets_round_trip_on_character_boundaries() {
        let line = "a世\tb";
        assert_eq!(column_of(line, 0), 0);
        assert_eq!(column_of(line, 1), 1);
        assert_eq!(column_of(line, 4), 3);
        assert_eq!(column_of(line, 5), 4);
        assert_eq!(column_of(line, 99), 5);
        // Inside the three-byte character counts only what starts before it.
        assert_eq!(column_of(line, 2), 3);

        assert_eq!(offset_at_column(line, 0), 0);
        assert_eq!(offset_at_column(line, 1), 1);
        assert_eq!(offset_at_column(line, 2), 1);
        assert_eq!(offset_at_column(line, 3), 4);
        assert_eq!(offset_at_column(line, 4), 5);
        assert_eq!(offset_at_column(line, 5), line.len());
        assert_eq!(offset_at_column(line, 50), line.len());
    }

    #[test]
    fn fitted_lines_are_exactly_the_requested_width() {
        assert_eq!(fit_line("abc", 5), "abc  ");
        assert_eq!(fit_line("abcdef", 3), "abc");
        assert_eq!(fit_line("a世", 2), "a ");
        assert_eq!(fit_line("\u{1B}[2J", 4), "\u{FFFD}[2J");
        assert_eq!(fit_line("anything", 0), "");
    }

    proptest! {
        #[test]
        fn fitted_lines_fill_the_width_with_safe_characters(text in any::<String>(), width in 0_usize..120) {
            let fitted = fit_line(&text, width);
            prop_assert_eq!(display_width(&fitted), width);
            prop_assert!(!fitted.chars().any(is_terminal_unsafe));
        }

        #[test]
        fn sanitized_text_never_contains_unsafe_characters(text in any::<String>()) {
            let shown = sanitize_line(&text);
            prop_assert!(!shown.chars().any(is_terminal_unsafe));
            prop_assert_eq!(display_width(&shown), display_width(&text));
        }

        #[test]
        fn every_column_maps_to_a_character_boundary(text in any::<String>(), column in 0_usize..200) {
            let offset = offset_at_column(&text, column);
            prop_assert!(text.is_char_boundary(offset));
            prop_assert!(column_of(&text, offset) <= column.max(display_width(&text)));
        }
    }
}
