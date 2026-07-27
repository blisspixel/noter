//! Conservative Markdown diagnostics that never mutate document source.

use super::line_endings::logical_lines;

/// One source-based Markdown diagnostic.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MarkdownDiagnostic {
    code: &'static str,
    line: usize,
    message: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct FenceMarker {
    character: char,
    length: usize,
    bare: bool,
}

impl FenceMarker {
    const fn closes(self, opening: Self) -> bool {
        self.character == opening.character && self.length >= opening.length && self.bare
    }
}

impl MarkdownDiagnostic {
    /// Returns the stable diagnostic identifier.
    pub const fn code(&self) -> &'static str {
        self.code
    }

    /// Returns the one-based source line.
    pub const fn line(&self) -> usize {
        self.line
    }

    /// Returns the concise diagnostic explanation.
    pub const fn message(&self) -> &'static str {
        self.message
    }
}

/// Analyzes Markdown source with a deliberately small, deterministic rule set.
///
/// The initial rules report skipped ATX heading levels, trailing spaces that are
/// neither absent nor the two-space hard-break form, repeated blank lines, and
/// a missing final newline. Fenced code is excluded from whitespace and heading
/// checks because those bytes may be intentional.
#[must_use]
pub fn analyze_markdown(source: &str) -> Vec<MarkdownDiagnostic> {
    let mut diagnostics = Vec::new();
    visit_markdown_diagnostics(source, |diagnostic| diagnostics.push(diagnostic));
    diagnostics
}

/// Counts diagnostics without retaining a result vector.
///
/// This follows the same deterministic rule traversal as [`analyze_markdown`]
/// and is intended for status summaries that do not display individual items.
#[must_use]
pub fn count_markdown_diagnostics(source: &str) -> usize {
    let mut count = 0_usize;
    visit_markdown_diagnostics(source, |_| count += 1);
    count
}

fn visit_markdown_diagnostics(source: &str, mut emit: impl FnMut(MarkdownDiagnostic)) {
    let mut fence = None;
    let mut previous_heading_level = None;
    let mut consecutive_blank_lines = 0_usize;
    let mut line_count = 0_usize;
    let mut final_line_has_ending = false;

    for (line_index, segment) in logical_lines(source).enumerate() {
        let line_number = line_index + 1;
        let line = segment.content();
        line_count = line_number;
        final_line_has_ending = segment.ending().is_some();
        if let Some(marker) = fence_marker(line) {
            if fence.is_some_and(|opening| marker.closes(opening)) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            } else {
                continue;
            }
            consecutive_blank_lines = 0;
            continue;
        }

        if fence.is_some() {
            continue;
        }

        let trailing_spaces = line.len() - line.trim_end_matches(' ').len();
        let content_before_spaces = &line[..line.len() - trailing_spaces];
        let is_hard_break =
            trailing_spaces == 2 && !content_before_spaces.trim_matches([' ', '\t']).is_empty();
        if trailing_spaces != 0 && !is_hard_break {
            emit(MarkdownDiagnostic {
                code: "MD009",
                line: line_number,
                message: "Trailing spaces should be removed or use exactly two for a hard break",
            });
        }

        if line.trim_matches([' ', '\t']).is_empty() {
            consecutive_blank_lines += 1;
            if consecutive_blank_lines > 1 {
                emit(MarkdownDiagnostic {
                    code: "MD012",
                    line: line_number,
                    message: "Multiple consecutive blank lines",
                });
            }
            continue;
        }
        consecutive_blank_lines = 0;

        if let Some(level) = atx_heading_level(line)
            && previous_heading_level.is_some_and(|previous| level > previous + 1)
        {
            emit(MarkdownDiagnostic {
                code: "MD001",
                line: line_number,
                message: "Heading levels should increment by one",
            });
            previous_heading_level = Some(level);
        } else if let Some(level) = atx_heading_level(line) {
            previous_heading_level = Some(level);
        }
    }

    if !source.is_empty() && !final_line_has_ending {
        emit(MarkdownDiagnostic {
            code: "MD047",
            line: line_count,
            message: "File should end with a newline",
        });
    }
}

fn fence_marker(line: &str) -> Option<FenceMarker> {
    let trimmed = markdown_block_start(line)?;
    let marker = trimmed.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let length = trimmed
        .chars()
        .take_while(|character| *character == marker)
        .count();
    if length < 3 {
        return None;
    }
    Some(FenceMarker {
        character: marker,
        length,
        bare: trimmed[length..].trim_matches([' ', '\t']).is_empty(),
    })
}

fn atx_heading_level(line: &str) -> Option<usize> {
    let trimmed = markdown_block_start(line)?;
    let level = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&level) {
        return None;
    }
    trimmed
        .as_bytes()
        .get(level)
        .is_some_and(u8::is_ascii_whitespace)
        .then_some(level)
}

fn markdown_block_start(line: &str) -> Option<&str> {
    let trimmed = line.trim_start_matches(' ');
    (line.len() - trimmed.len() <= 3).then_some(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_document_has_no_diagnostics() {
        let source =
            "# Noter\n\nA focused **Markdown** editor.\n\n## Details\n\n- Fast\n- Private\n";

        assert!(analyze_markdown(source).is_empty());
    }

    #[test]
    fn diagnostics_have_stable_codes_and_lines() {
        let source = "# Heading \n\n\n### Skipped\nNo final newline";

        let diagnostics = analyze_markdown(source);

        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| { (diagnostic.code(), diagnostic.line(), diagnostic.message(),) })
                .collect::<Vec<_>>(),
            vec![
                (
                    "MD009",
                    1,
                    "Trailing spaces should be removed or use exactly two for a hard break",
                ),
                ("MD012", 3, "Multiple consecutive blank lines"),
                ("MD001", 4, "Heading levels should increment by one"),
                ("MD047", 5, "File should end with a newline"),
            ]
        );
    }

    #[test]
    fn count_uses_the_exact_diagnostic_traversal_without_retaining_items() {
        for source in [
            "",
            "# Clean\n",
            "# Heading \n\n\n### Skipped\nNo final newline",
            "```text\nintentional  \n```\n",
        ] {
            assert_eq!(
                count_markdown_diagnostics(source),
                analyze_markdown(source).len()
            );
        }
    }

    #[test]
    fn fenced_code_preserves_intentional_whitespace_and_heading_text() {
        let source = "# Heading\n\n```text\n### Not a heading   \n\n\n```\n\n## Next\n";

        assert!(analyze_markdown(source).is_empty());
    }

    #[test]
    fn two_space_hard_break_is_not_reported() {
        assert!(analyze_markdown("# Heading\n\nFirst line  \nSecond line\n").is_empty());
    }

    #[test]
    fn closing_fence_must_use_the_opening_marker() {
        let source =
            "# Heading\n\n```text\n### Hidden\n~~~\n#### Still hidden\n```\n\n## Visible\n";

        assert!(analyze_markdown(source).is_empty());
    }

    #[test]
    fn fence_closer_must_be_bare_and_at_least_as_long_as_the_opener() {
        let source = "# Heading\n\n````text\n```\n### Hidden\n````still code\n#### Hidden too\n````\n\n## Visible\n";

        assert!(analyze_markdown(source).is_empty());
        assert_eq!(
            fence_marker("````text"),
            Some(FenceMarker {
                character: '`',
                length: 4,
                bare: false,
            })
        );
        assert!(fence_marker("``").is_none());
    }

    #[test]
    fn a_valid_fence_closer_restores_diagnostics() {
        let source = "# Heading\n\n```text\n### Hidden\n```\n\n### Visible skip\n";

        assert_eq!(
            analyze_markdown(source)
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.line()))
                .collect::<Vec<_>>(),
            vec![("MD001", 7)]
        );
    }

    #[test]
    fn invalid_and_code_indented_atx_markers_are_ignored() {
        let source = "# Heading\n\n####### Too deep\n\n###No separator\n\n    ### Indented code\n\n## Visible\n";

        assert!(analyze_markdown(source).is_empty());
    }

    #[test]
    fn whitespace_only_lines_are_blank_and_not_hard_breaks() {
        let source = "# Heading\n \n  \n## Visible\n";

        assert_eq!(
            analyze_markdown(source)
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.line()))
                .collect::<Vec<_>>(),
            vec![("MD009", 2), ("MD009", 3), ("MD012", 3)]
        );
    }

    #[test]
    fn cr_and_mixed_endings_have_exact_line_numbers_and_final_newline_state() {
        let source = "# Heading\r\r\r### Skipped\rLast line\r";

        assert_eq!(
            analyze_markdown(source)
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.line()))
                .collect::<Vec<_>>(),
            vec![("MD012", 3), ("MD001", 4)]
        );

        let mixed_without_final_ending = "# Heading\r\n\r### Skipped\nLast line";
        assert_eq!(
            analyze_markdown(mixed_without_final_ending)
                .iter()
                .map(|diagnostic| (diagnostic.code(), diagnostic.line()))
                .collect::<Vec<_>>(),
            vec![("MD001", 3), ("MD047", 4)]
        );
    }
}
