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

fn snap_to_char_boundary(source: &str, offset: usize) -> usize {
    if offset >= source.len() {
        return source.len();
    }
    if source.is_char_boundary(offset) {
        return offset;
    }
    // Walk backward at most the maximum UTF-8 scalar width.
    for distance in 1..=3 {
        let candidate = offset.saturating_sub(distance);
        if source.is_char_boundary(candidate) {
            return candidate;
        }
    }
    0
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

fn move_by_word(source: &str, offset: usize, direction: MoveDirection) -> usize {
    match direction {
        MoveDirection::Forward => {
            if offset >= source.len() {
                return source.len();
            }
            let rest = &source[offset..];
            let mut chars = rest.char_indices();
            let Some((_, first)) = chars.next() else {
                return source.len();
            };
            // Skip pure whitespace so Ctrl+Right lands on the next token start.
            if first.is_whitespace() {
                for (rel, ch) in chars {
                    if !ch.is_whitespace() {
                        return offset + rel;
                    }
                }
                return source.len();
            }
            let class = is_word_scalar(first);
            for (rel, ch) in chars {
                if ch.is_whitespace() || is_word_scalar(ch) != class {
                    return offset + rel;
                }
            }
            source.len()
        }
        MoveDirection::Backward => {
            if offset == 0 {
                return 0;
            }
            // Finite reverse walk over scalars (allocation is bounded by offset).
            let mut chars: Vec<(usize, char)> = source[..offset].char_indices().collect();
            // Skip trailing whitespace so Ctrl+Left lands on the prior token start.
            while chars.last().is_some_and(|(_, ch)| ch.is_whitespace()) {
                chars.pop();
            }
            let Some((_, last)) = chars.pop() else {
                return 0;
            };
            let class = is_word_scalar(last);
            while let Some((index, ch)) = chars.last().copied() {
                if ch.is_whitespace() || is_word_scalar(ch) != class {
                    return index + ch.len_utf8();
                }
                chars.pop();
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
            let line = logical_line_at(source, offset);
            let line_start = line_start_offset(source, line).unwrap_or(0);
            if offset == line_start {
                if line == 1 {
                    return 0;
                }
                line_start_offset(source, line - 1).unwrap_or(0)
            } else {
                line_start
            }
        }
        MoveDirection::Forward => {
            let line = logical_line_at(source, offset);
            match line_start_offset(source, line + 1) {
                Ok(next_start) => next_start,
                Err(LineNavigationError::OutOfRange { .. } | LineNavigationError::Zero) => {
                    source.len()
                }
            }
        }
    }
}

/// One-based logical line of the caret at `offset`.
///
/// Counts terminators that end strictly before `offset`. A CRLF pair counts as
/// one terminator. The scan is a single forward pass over the prefix bytes so
/// it always terminates under mutation.
fn logical_line_at(source: &str, offset: usize) -> usize {
    let prefix = &source.as_bytes()[..offset.min(source.len())];
    let mut line = 1_usize;
    let mut pending_cr = false;
    for &byte in prefix {
        if pending_cr {
            pending_cr = false;
            if byte == b'\n' {
                // Second half of CRLF: already counted with the CR.
                continue;
            }
            // Bare CR was already counted; still process this byte.
        }
        match byte {
            b'\r' => {
                line += 1;
                pending_cr = true;
            }
            b'\n' => line += 1,
            _ => {}
        }
    }
    line
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
        // Backward from start of "world_1" skips spaces and lands on ",".
        assert_eq!(
            move_caret(source, 8, MoveDirection::Backward, MoveUnit::Word),
            5
        );
        // One more step lands on "hello".
        assert_eq!(
            move_caret(source, 5, MoveDirection::Backward, MoveUnit::Word),
            0
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

    #[test]
    fn snap_to_char_boundary_handles_exact_and_interior_offsets() {
        let source = "aé你"; // 1 + 2 + 3 bytes
        assert_eq!(snap_to_char_boundary(source, 0), 0);
        assert_eq!(snap_to_char_boundary(source, 1), 1);
        assert_eq!(snap_to_char_boundary(source, 2), 1); // mid é
        assert_eq!(snap_to_char_boundary(source, 3), 3); // start of 你
        assert_eq!(snap_to_char_boundary(source, 4), 3); // mid 你
        assert_eq!(snap_to_char_boundary(source, 5), 3); // mid 你
        assert_eq!(snap_to_char_boundary(source, source.len()), source.len());
        assert_eq!(
            snap_to_char_boundary(source, source.len() + 10),
            source.len()
        );
        assert_eq!(snap_to_char_boundary("", 0), 0);
        assert_eq!(snap_to_char_boundary("", 3), 0);
    }

    #[test]
    fn word_moves_skip_whitespace_and_land_on_token_starts() {
        let source = "ab  cd!";
        // Forward from start of spaces lands on "cd".
        assert_eq!(
            move_caret(source, 2, MoveDirection::Forward, MoveUnit::Word),
            4
        );
        // Forward from trailing punctuation.
        assert_eq!(
            move_caret(source, 6, MoveDirection::Forward, MoveUnit::Word),
            7
        );
        // Backward from spaces lands at start of "ab".
        assert_eq!(
            move_caret(source, 3, MoveDirection::Backward, MoveUnit::Word),
            0
        );
        // Backward from end of "cd" lands at start of "cd".
        assert_eq!(
            move_caret(source, 6, MoveDirection::Backward, MoveUnit::Word),
            4
        );
        // Backward from document end over punctuation.
        assert_eq!(
            move_caret(source, 7, MoveDirection::Backward, MoveUnit::Word),
            6
        );
        assert_eq!(
            move_caret(source, 0, MoveDirection::Backward, MoveUnit::Word),
            0
        );
        assert_eq!(
            move_caret(source, source.len(), MoveDirection::Forward, MoveUnit::Word),
            source.len()
        );
        // Trailing whitespace only: forward reaches end; backward reaches 0.
        assert_eq!(
            move_caret("  ", 0, MoveDirection::Forward, MoveUnit::Word),
            2
        );
        assert_eq!(
            move_caret("  ", 2, MoveDirection::Backward, MoveUnit::Word),
            0
        );
    }

    #[test]
    fn line_moves_from_start_middle_and_mixed_terminators() {
        let source = "a\r\nb\nc\rd";
        // Line starts: 0, 3 ("b"), 5 ("c"), 7 ("d")
        assert_eq!(line_start_offset(source, 1), Ok(0));
        assert_eq!(line_start_offset(source, 2), Ok(3));
        assert_eq!(line_start_offset(source, 3), Ok(5));
        assert_eq!(line_start_offset(source, 4), Ok(7));

        // Middle of line 1 -> line start.
        assert_eq!(
            move_caret(source, 1, MoveDirection::Backward, MoveUnit::Line),
            0
        );
        // Already at line 2 start -> previous line start.
        assert_eq!(
            move_caret(source, 3, MoveDirection::Backward, MoveUnit::Line),
            0
        );
        // Forward from line 1 middle -> line 2 start.
        assert_eq!(
            move_caret(source, 1, MoveDirection::Forward, MoveUnit::Line),
            3
        );
        // Forward from last line -> document end.
        assert_eq!(
            move_caret(source, 7, MoveDirection::Forward, MoveUnit::Line),
            source.len()
        );
        // Backward from first line start stays put.
        assert_eq!(
            move_caret(source, 0, MoveDirection::Backward, MoveUnit::Line),
            0
        );
        // CR-only boundary.
        assert_eq!(
            move_caret(source, 6, MoveDirection::Forward, MoveUnit::Line),
            7
        );
        assert_eq!(
            move_caret(source, 7, MoveDirection::Backward, MoveUnit::Line),
            5
        );
        // Empty and single-line documents.
        assert_eq!(move_caret("", 0, MoveDirection::Forward, MoveUnit::Line), 0);
        assert_eq!(
            move_caret("solo", 2, MoveDirection::Forward, MoveUnit::Line),
            4
        );
        assert_eq!(
            move_caret("solo", 2, MoveDirection::Backward, MoveUnit::Line),
            0
        );
    }

    #[test]
    fn logical_line_at_matches_line_start_table() {
        let source = "a\r\nb\nc\rd";
        // a \r \n b \n c \r d
        // 0 1  2  3 4  5 6  7
        assert_eq!(logical_line_at(source, 0), 1);
        assert_eq!(logical_line_at(source, 1), 1);
        // Mid-CRLF counts CR alone once CR is strictly before offset.
        assert_eq!(logical_line_at(source, 2), 2);
        assert_eq!(logical_line_at(source, 3), 2);
        assert_eq!(logical_line_at(source, 4), 2);
        assert_eq!(logical_line_at(source, 5), 3);
        assert_eq!(logical_line_at(source, 6), 3);
        assert_eq!(logical_line_at(source, 7), 4);
        assert_eq!(logical_line_at(source, source.len()), 4);
        assert_eq!(logical_line_at(source, source.len() + 5), 4);
        assert_eq!(logical_line_at("", 0), 1);
        assert_eq!(logical_line_at("\n\n", 0), 1);
        assert_eq!(logical_line_at("\n\n", 1), 2);
        assert_eq!(logical_line_at("\n\n", 2), 3);
        assert_eq!(logical_line_at("\r\n", 1), 2);
        // CRLF is one terminator: double-counting would yield 3.
        assert_eq!(logical_line_at("\r\n", 2), 2);
        assert_eq!(logical_line_at("\r\n\n", 3), 3);
        assert_eq!(logical_line_at("\r\rx", 3), 3);
        assert_eq!(logical_line_at("solo", 4), 1);
    }
}
