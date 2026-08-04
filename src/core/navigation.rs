//! Allocation-free logical-line and UTF-8 caret navigation.
//!
//! Character and word movement use validated UTF-8 byte offsets. Word
//! boundaries follow a classic editor rule: a maximal run of Unicode letters
//! and numbers is one word; every other non-empty scalar cluster is a separate
//! token. Line endings are never split, and every returned offset is a char
//! boundary. Grapheme-cluster visual movement belongs to the editor adapter
//! (M5) and is intentionally out of scope here.

/// Direction of a pure caret move.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MoveDirection {
    /// Toward the start of the source.
    Backward,
    /// Toward the end of the source.
    Forward,
}

/// Granularity of a pure caret move at UTF-8 byte offsets.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MoveUnit {
    /// One Unicode scalar value.
    Character,
    /// One classic alphanumeric or non-alphanumeric token.
    Word,
    /// Start of the current logical line, or the previous line when already there.
    Line,
    /// Document start or end.
    Document,
}

/// A rejected one-based logical line request.
#[derive(Clone, Copy, PartialEq, Eq, Debug, thiserror::Error)]
pub enum LineNavigationError {
    /// Line numbers are one-based.
    #[error("line numbers begin at 1")]
    Zero,
    /// The requested line is beyond the document.
    #[error("line {requested} is outside this {maximum}-line document")]
    OutOfRange {
        /// The requested one-based line number.
        requested: usize,
        /// The document's exact logical line count.
        maximum: usize,
    },
}

/// Moves a UTF-8 byte caret by one pure navigation unit.
///
/// Offsets that are not character boundaries are snapped to the previous valid
/// boundary before the move. Empty documents always return `0`.
#[must_use]
pub fn move_caret(source: &str, offset: usize, direction: MoveDirection, unit: MoveUnit) -> usize {
    let offset = snap_to_char_boundary(source, offset);
    match unit {
        MoveUnit::Character => move_by_character(source, offset, direction),
        MoveUnit::Word => move_by_word(source, offset, direction),
        MoveUnit::Line => move_by_line(source, offset, direction),
        MoveUnit::Document => match direction {
            MoveDirection::Backward => 0,
            MoveDirection::Forward => source.len(),
        },
    }
}

/// Extends a directional selection by moving only the active caret.
#[must_use]
pub fn extend_selection(
    source: &str,
    selection: super::edit::Selection,
    direction: MoveDirection,
    unit: MoveUnit,
) -> super::edit::Selection {
    let active = move_caret(source, selection.active(), direction, unit);
    super::edit::Selection::new(selection.anchor(), active)
}

const fn snap_to_char_boundary(source: &str, offset: usize) -> usize {
    if offset >= source.len() {
        return source.len();
    }
    if source.is_char_boundary(offset) {
        return offset;
    }
    let mut candidate = offset;
    while candidate > 0 && !source.is_char_boundary(candidate) {
        candidate -= 1;
    }
    candidate
}

fn move_by_character(source: &str, offset: usize, direction: MoveDirection) -> usize {
    match direction {
        MoveDirection::Backward => {
            if offset == 0 {
                return 0;
            }
            source
                .char_indices()
                .take_while(|(index, _)| *index < offset)
                .last()
                .map_or(0, |(index, _)| index)
        }
        MoveDirection::Forward => {
            if offset >= source.len() {
                return source.len();
            }
            source[offset..]
                .chars()
                .next()
                .map_or(source.len(), |ch| offset + ch.len_utf8())
        }
    }
}

fn is_word_scalar(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn previous_char(source: &str, offset: usize) -> Option<(usize, char)> {
    if offset == 0 {
        return None;
    }
    source[..offset].char_indices().next_back()
}

fn move_by_word(source: &str, offset: usize, direction: MoveDirection) -> usize {
    match direction {
        MoveDirection::Forward => {
            if offset >= source.len() {
                return source.len();
            }
            let mut chars = source[offset..].char_indices();
            let Some((_, first)) = chars.next() else {
                return source.len();
            };
            // Skip pure whitespace so Ctrl+Right lands on the next token start.
            if first.is_whitespace() {
                let mut end = offset + first.len_utf8();
                for (rel, ch) in chars.by_ref() {
                    if !ch.is_whitespace() {
                        return offset + rel;
                    }
                    end = offset + rel + ch.len_utf8();
                }
                return end.min(source.len());
            }
            let class = is_word_scalar(first);
            let mut end = offset + first.len_utf8();
            for (rel, ch) in chars {
                if ch.is_whitespace() || is_word_scalar(ch) != class {
                    return offset + rel;
                }
                end = offset + rel + ch.len_utf8();
            }
            end
        }
        MoveDirection::Backward => {
            let Some((mut cursor, last)) = previous_char(source, offset) else {
                return 0;
            };
            if last.is_whitespace() {
                // Land after the last non-whitespace scalar (or 0).
                while let Some((index, ch)) = previous_char(source, cursor) {
                    if !ch.is_whitespace() {
                        return index + ch.len_utf8();
                    }
                    cursor = index;
                }
                return 0;
            }
            let class = is_word_scalar(last);
            while let Some((index, ch)) = previous_char(source, cursor) {
                if ch.is_whitespace() || is_word_scalar(ch) != class {
                    return cursor;
                }
                cursor = index;
            }
            0
        }
    }
}

fn move_by_line(source: &str, offset: usize, direction: MoveDirection) -> usize {
    match direction {
        MoveDirection::Backward => {
            if offset == 0 {
                return 0;
            }
            // Find the logical line that contains `offset` (or ends just before it).
            let mut line = 1_usize;
            let mut line_start = 0_usize;
            let mut bytes = source.bytes().enumerate().peekable();
            while let Some((index, byte)) = bytes.next() {
                if index >= offset {
                    break;
                }
                if byte == b'\r' && bytes.peek().is_some_and(|(_, next)| *next == b'\n') {
                    bytes.next();
                    line += 1;
                    line_start = index + 2;
                } else if byte == b'\r' || byte == b'\n' {
                    line += 1;
                    line_start = index + 1;
                }
            }
            if offset == line_start {
                // Already at line start: jump to previous line start when possible.
                if line <= 1 {
                    return 0;
                }
                line_start_offset(source, line - 1).unwrap_or(0)
            } else {
                line_start
            }
        }
        MoveDirection::Forward => {
            // Move to the start of the next logical line, or document end.
            let mut bytes = source.bytes().enumerate().peekable();
            while let Some((index, byte)) = bytes.next() {
                if index < offset {
                    if byte == b'\r' && bytes.peek().is_some_and(|(_, next)| *next == b'\n') {
                        bytes.next();
                    }
                    continue;
                }
                if byte == b'\r' && bytes.peek().is_some_and(|(_, next)| *next == b'\n') {
                    bytes.next();
                    return bytes.peek().map_or(source.len(), |(next, _)| *next);
                }
                if byte == b'\r' || byte == b'\n' {
                    return bytes.peek().map_or(source.len(), |(next, _)| *next);
                }
            }
            source.len()
        }
    }
}

/// Returns the UTF-8 byte offset at the start of a one-based logical line.
///
/// LF, CRLF, CR, and mixed documents are handled without normalizing source
/// bytes. An empty document has one logical line, and a final terminator starts
/// a trailing empty line.
///
/// # Errors
///
/// Returns [`LineNavigationError::Zero`] for line zero or
/// [`LineNavigationError::OutOfRange`] when the requested line does not exist.
pub fn line_start_offset(source: &str, requested: usize) -> Result<usize, LineNavigationError> {
    if requested == 0 {
        return Err(LineNavigationError::Zero);
    }
    if requested == 1 {
        return Ok(0);
    }

    let mut bytes = source.bytes().enumerate().peekable();
    let mut current_line = 1_usize;
    while let Some((_, byte)) = bytes.next() {
        if byte == b'\r' && bytes.peek().is_some_and(|(_, next)| *next == b'\n') {
            bytes.next();
        } else if byte != b'\r' && byte != b'\n' {
            continue;
        }

        current_line += 1;
        if current_line == requested {
            return Ok(bytes.peek().map_or(source.len(), |(offset, _)| *offset));
        }
    }

    Err(LineNavigationError::OutOfRange {
        requested,
        maximum: current_line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_single_line_documents_have_one_addressable_line() {
        assert_eq!(line_start_offset("", 1), Ok(0));
        assert_eq!(line_start_offset("hello", 1), Ok(0));
        assert_eq!(
            line_start_offset("hello", 2),
            Err(LineNavigationError::OutOfRange {
                requested: 2,
                maximum: 1,
            })
        );
    }

    #[test]
    fn mixed_line_endings_and_unicode_return_exact_utf8_boundaries() {
        let source = "é\r\ntwo\r三\nfour";

        assert_eq!(line_start_offset(source, 1), Ok(0));
        assert_eq!(line_start_offset(source, 2), Ok("é\r\n".len()));
        assert_eq!(line_start_offset(source, 3), Ok("é\r\ntwo\r".len()));
        assert_eq!(line_start_offset(source, 4), Ok("é\r\ntwo\r三\n".len()));
        assert!(source.is_char_boundary(line_start_offset(source, 4).unwrap()));
    }

    #[test]
    fn crlf_counts_as_one_terminator_and_trailing_empty_lines_are_addressable() {
        assert_eq!(line_start_offset("a\r\n", 2), Ok(3));
        assert_eq!(
            line_start_offset("a\r\n", 3),
            Err(LineNavigationError::OutOfRange {
                requested: 3,
                maximum: 2,
            })
        );
        assert_eq!(line_start_offset("\n\n", 3), Ok(2));
    }

    #[test]
    fn zero_is_rejected_without_scanning_the_document() {
        assert_eq!(
            line_start_offset("anything", 0),
            Err(LineNavigationError::Zero)
        );
    }

    #[test]
    fn character_moves_respect_utf8_scalar_boundaries() {
        let source = "aé你";
        assert_eq!(
            move_caret(source, 0, MoveDirection::Forward, MoveUnit::Character),
            1
        );
        assert_eq!(
            move_caret(source, 1, MoveDirection::Forward, MoveUnit::Character),
            "aé".len()
        );
        assert_eq!(
            move_caret(
                source,
                "aé".len(),
                MoveDirection::Forward,
                MoveUnit::Character
            ),
            source.len()
        );
        assert_eq!(
            move_caret(
                source,
                source.len(),
                MoveDirection::Backward,
                MoveUnit::Character
            ),
            "aé".len()
        );
        // Mid-scalar offset snaps to the previous boundary before the move.
        assert_eq!(
            move_caret(source, 2, MoveDirection::Backward, MoveUnit::Character),
            0
        );
        assert_eq!(
            move_caret(source, 2, MoveDirection::Forward, MoveUnit::Character),
            "aé".len()
        );
    }

    #[test]
    fn word_moves_use_classic_token_classes() {
        let source = "hello,  world_1!";
        assert_eq!(
            move_caret(source, 0, MoveDirection::Forward, MoveUnit::Word),
            5
        );
        assert_eq!(
            move_caret(source, 5, MoveDirection::Forward, MoveUnit::Word),
            6
        );
        assert_eq!(
            move_caret(source, 6, MoveDirection::Forward, MoveUnit::Word),
            8
        );
        assert_eq!(
            move_caret(source, 8, MoveDirection::Forward, MoveUnit::Word),
            15
        );
        assert_eq!(
            move_caret(source, 15, MoveDirection::Forward, MoveUnit::Word),
            16
        );
        assert_eq!(
            move_caret(source, 16, MoveDirection::Backward, MoveUnit::Word),
            15
        );
        assert_eq!(
            move_caret(source, 15, MoveDirection::Backward, MoveUnit::Word),
            8
        );
        assert_eq!(
            move_caret(source, 8, MoveDirection::Backward, MoveUnit::Word),
            6
        );
    }

    #[test]
    fn line_and_document_moves_honor_mixed_endings() {
        let source = "one\r\ntwo\nthree";
        assert_eq!(
            move_caret(source, 2, MoveDirection::Backward, MoveUnit::Line),
            0
        );
        assert_eq!(
            move_caret(source, 0, MoveDirection::Backward, MoveUnit::Line),
            0
        );
        assert_eq!(
            move_caret(source, 0, MoveDirection::Forward, MoveUnit::Line),
            5
        );
        assert_eq!(
            move_caret(source, 5, MoveDirection::Forward, MoveUnit::Line),
            9
        );
        assert_eq!(
            move_caret(source, 5, MoveDirection::Backward, MoveUnit::Document),
            0
        );
        assert_eq!(
            move_caret(source, 5, MoveDirection::Forward, MoveUnit::Document),
            source.len()
        );
    }

    #[test]
    fn extend_selection_keeps_anchor() {
        use super::super::edit::Selection;
        let source = "abcdef";
        let selection = Selection::new(1, 1);
        let extended = extend_selection(source, selection, MoveDirection::Forward, MoveUnit::Word);
        assert_eq!(extended.anchor(), 1);
        assert_eq!(extended.active(), 6);
    }
}
