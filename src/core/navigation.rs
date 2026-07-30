//! Allocation-free logical-line navigation for mixed line endings.

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

    let bytes = source.as_bytes();
    let mut offset = 0_usize;
    let mut current_line = 1_usize;
    while offset < bytes.len() {
        let starts_line_ending = matches!(bytes[offset], b'\r' | b'\n');
        let ending_length =
            usize::from(bytes[offset] == b'\r' && bytes.get(offset + 1) == Some(&b'\n')) + 1;
        offset += if starts_line_ending { ending_length } else { 1 };
        if starts_line_ending {
            current_line += 1;
            if current_line == requested {
                return Ok(offset);
            }
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
}
