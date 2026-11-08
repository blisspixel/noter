//! Characters a terminal must not receive as themselves.
//!
//! The editor and the locale clock share this predicate. A second copy would
//! let one caller trim a control away before the other caller sees it.

/// Returns whether drawing `character` could change terminal state or
/// reorder surrounding text instead of showing a glyph.
///
/// This covers the C0 and C1 control ranges, DEL, the Unicode line and
/// paragraph separators, and the bidirectional formatting characters that
/// can make displayed text read differently from its source order.
#[must_use]
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
            | '\u{206A}'..='\u{206F}'
    )
}
