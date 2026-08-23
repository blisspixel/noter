//! Exact line-ending classification and insertion policy.

use std::{cmp::Ordering, ops::Range};

use ropey::Rope;

/// The byte sequence used to terminate one logical line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineEnding {
    /// Unix line feed (`\n`).
    Lf,
    /// Windows carriage return plus line feed (`\r\n`).
    CrLf,
    /// Legacy carriage return (`\r`).
    Cr,
}

/// One logical source line with its original terminator, when present.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineSegment<'a> {
    content: &'a str,
    ending: Option<LineEnding>,
}

impl<'a> LineSegment<'a> {
    /// Returns the source bytes before the line ending.
    pub const fn content(self) -> &'a str {
        self.content
    }

    /// Returns the exact original line ending, or `None` for a final unterminated line.
    pub const fn ending(self) -> Option<LineEnding> {
        self.ending
    }
}

/// Iterator over logical lines that preserves LF, CRLF, CR, and mixed endings.
pub struct LogicalLines<'a> {
    remaining: &'a str,
}

impl<'a> Iterator for LogicalLines<'a> {
    type Item = LineSegment<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining.is_empty() {
            return None;
        }

        let bytes = self.remaining.as_bytes();
        let ending_start = bytes.iter().position(|byte| matches!(byte, b'\r' | b'\n'));
        let Some(ending_start) = ending_start else {
            let content = self.remaining;
            self.remaining = "";
            return Some(LineSegment {
                content,
                ending: None,
            });
        };

        let (ending, ending_length) =
            if bytes[ending_start] == b'\r' && bytes.get(ending_start + 1) == Some(&b'\n') {
                (LineEnding::CrLf, 2)
            } else if bytes[ending_start] == b'\r' {
                (LineEnding::Cr, 1)
            } else {
                (LineEnding::Lf, 1)
            };
        let (content, ending_and_rest) = self.remaining.split_at(ending_start);
        let (_, remaining) = ending_and_rest.split_at(ending_length);
        self.remaining = remaining;
        Some(LineSegment {
            content,
            ending: Some(ending),
        })
    }
}

/// Splits strict UTF-8 text into logical lines without normalizing terminators.
#[must_use]
pub const fn logical_lines(text: &str) -> LogicalLines<'_> {
    LogicalLines { remaining: text }
}

impl LineEnding {
    /// Returns the encoded line-ending sequence.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }

    /// Returns the line ending used when a document has no existing convention.
    pub const fn platform_default() -> Self {
        #[cfg(windows)]
        {
            Self::CrLf
        }
        #[cfg(not(windows))]
        {
            Self::Lf
        }
    }

    const fn slot(self) -> usize {
        match self {
            Self::Lf => 0,
            Self::CrLf => 1,
            Self::Cr => 2,
        }
    }
}

/// Counts of every line-ending sequence in a document.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct LineEndingCounts {
    /// Number of standalone line feeds.
    pub lf: usize,
    /// Number of carriage-return plus line-feed pairs.
    pub crlf: usize,
    /// Number of standalone carriage returns.
    pub cr: usize,
}

impl LineEndingCounts {
    /// Returns the total number of logical line endings.
    pub const fn total(self) -> usize {
        self.lf + self.crlf + self.cr
    }

    /// Returns the count for one line-ending kind.
    pub const fn get(self, ending: LineEnding) -> usize {
        match ending {
            LineEnding::Lf => self.lf,
            LineEnding::CrLf => self.crlf,
            LineEnding::Cr => self.cr,
        }
    }

    const fn kinds(self) -> usize {
        (self.lf > 0) as usize + (self.crlf > 0) as usize + (self.cr > 0) as usize
    }

    const fn increment(&mut self, ending: LineEnding) {
        match ending {
            LineEnding::Lf => self.lf += 1,
            LineEnding::CrLf => self.crlf += 1,
            LineEnding::Cr => self.cr += 1,
        }
    }
}

/// The detected line-ending shape and deterministic insertion fallback.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineEndingProfile {
    /// The document contains no line ending.
    None {
        /// Convention to use for the first inserted logical newline.
        insertion: LineEnding,
    },
    /// Every existing line ending uses the same convention.
    Uniform {
        /// Existing line-ending convention.
        ending: LineEnding,
        /// Number of existing line endings.
        count: usize,
    },
    /// At least two line-ending conventions occur in the document.
    Mixed {
        /// Counts for all three conventions.
        counts: LineEndingCounts,
        /// Dominant convention, with first occurrence breaking count ties.
        insertion: LineEnding,
    },
}

/// The document context needed to choose line endings inside one editable range.
///
/// The range itself may be rendered through a canonical display projection.
/// Endings immediately outside it remain available so an insertion at either
/// edge of a mixed-ending range still follows the nearest source convention.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LineEndingInsertionContext {
    profile: LineEndingProfile,
    preceding: Option<LineEnding>,
    following: Option<LineEnding>,
}

impl LineEndingInsertionContext {
    /// Chooses the ending for a newline inserted at a UTF-8 byte offset.
    ///
    /// Returns `None` when the offset is outside `editable`, is not a UTF-8
    /// boundary, or would split an existing CRLF pair.
    pub fn insertion_at(self, editable: &str, byte_offset: usize) -> Option<LineEnding> {
        if byte_offset > editable.len()
            || !editable.is_char_boundary(byte_offset)
            || splits_crlf(editable, byte_offset)
        {
            return None;
        }

        let preceding = last_line_ending(&editable[..byte_offset]).or(self.preceding);
        let following = first_line_ending(&editable[byte_offset..]).or(self.following);
        Some(select_insertion(self.profile, preceding, following))
    }
}

/// A newline-normalized insertion that fits an exact source-byte ceiling.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NormalizedInsertion {
    text: String,
    consumed_input_bytes: usize,
    was_limited: bool,
}

impl NormalizedInsertion {
    /// Returns the normalized source text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Consumes the result and returns the normalized source text.
    pub fn into_text(self) -> String {
        self.text
    }

    /// Returns the accepted UTF-8 prefix length from the original payload.
    pub const fn consumed_input_bytes(&self) -> usize {
        self.consumed_input_bytes
    }

    /// Reports whether the byte ceiling excluded any input.
    pub const fn was_limited(&self) -> bool {
        self.was_limited
    }
}

/// Normalizes every LF, CRLF, and CR in inserted text to `ending` while
/// enforcing an exact UTF-8 source-byte ceiling.
///
/// Truncation never splits a Unicode scalar or the selected line-ending
/// sequence. Non-newline source bytes are preserved exactly.
pub fn normalize_inserted_text(
    inserted: &str,
    ending: LineEnding,
    maximum_bytes: usize,
) -> NormalizedInsertion {
    let mut normalized = String::with_capacity(inserted.len().min(maximum_bytes));
    let mut consumed_input_bytes = 0;
    let mut characters = inserted.char_indices().peekable();
    while let Some((input_start, character)) = characters.next() {
        let is_crlf = character == '\r' && characters.peek().is_some_and(|(_, next)| *next == '\n');
        if is_crlf {
            let _ = characters.next();
        }
        let input_end = characters
            .peek()
            .map_or(inserted.len(), |(offset, _)| *offset);
        let output = if is_crlf || matches!(character, '\r' | '\n') {
            ending.as_str()
        } else {
            &inserted[input_start..input_end]
        };

        if output.len() > maximum_bytes.saturating_sub(normalized.len()) {
            break;
        }
        normalized.push_str(output);
        consumed_input_bytes = input_end;
    }

    NormalizedInsertion {
        text: normalized,
        consumed_input_bytes,
        was_limited: consumed_input_bytes != inserted.len(),
    }
}

impl LineEndingProfile {
    /// Classifies every line-ending sequence in strict UTF-8 text.
    pub fn detect(text: &str) -> Self {
        let mut counts = LineEndingCounts::default();
        let mut first_positions = [usize::MAX; 3];
        let mut first_ending = None;
        let mut bytes = text.as_bytes().iter().copied().enumerate().peekable();

        while let Some((index, byte)) = bytes.next() {
            let ending = match byte {
                b'\r' if matches!(bytes.peek(), Some((_, b'\n'))) => {
                    bytes.next();
                    LineEnding::CrLf
                }
                b'\r' => LineEnding::Cr,
                b'\n' => LineEnding::Lf,
                _ => continue,
            };

            counts.increment(ending);
            first_ending.get_or_insert(ending);
            let slot = ending.slot();
            if first_positions[slot] == usize::MAX {
                first_positions[slot] = index;
            }
        }

        let Some(mut insertion) = first_ending else {
            return Self::None {
                insertion: LineEnding::platform_default(),
            };
        };

        if counts.kinds() == 1 {
            return Self::Uniform {
                ending: insertion,
                count: counts.total(),
            };
        }

        for candidate in [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr] {
            if ending_precedes(candidate, insertion, counts, first_positions) {
                insertion = candidate;
            }
        }

        Self::Mixed { counts, insertion }
    }

    /// Returns counts for all line-ending conventions.
    pub const fn counts(self) -> LineEndingCounts {
        match self {
            Self::None { .. } => LineEndingCounts {
                lf: 0,
                crlf: 0,
                cr: 0,
            },
            Self::Uniform { ending, count } => match ending {
                LineEnding::Lf => LineEndingCounts {
                    lf: count,
                    crlf: 0,
                    cr: 0,
                },
                LineEnding::CrLf => LineEndingCounts {
                    lf: 0,
                    crlf: count,
                    cr: 0,
                },
                LineEnding::Cr => LineEndingCounts {
                    lf: 0,
                    crlf: 0,
                    cr: count,
                },
            },
            Self::Mixed { counts, .. } => counts,
        }
    }

    /// Returns the deterministic fallback used when no nearby ending exists.
    pub const fn fallback_insertion(self) -> LineEnding {
        match self {
            Self::None { insertion } | Self::Mixed { insertion, .. } => insertion,
            Self::Uniform { ending, .. } => ending,
        }
    }

    /// Returns a compact, truthful status-bar label.
    pub const fn status_label(self) -> &'static str {
        match self {
            Self::None { .. } => "No EOL",
            Self::Uniform {
                ending: LineEnding::Lf,
                ..
            } => "LF",
            Self::Uniform {
                ending: LineEnding::CrLf,
                ..
            } => "CRLF",
            Self::Uniform {
                ending: LineEnding::Cr,
                ..
            } => "CR",
            Self::Mixed { .. } => "Mixed",
        }
    }

    /// Captures insertion policy for one UTF-8 source byte range.
    ///
    /// Returns `None` for an inverted or out-of-bounds range, a non-UTF-8
    /// boundary, or a boundary that would split an existing CRLF pair.
    pub fn insertion_context(
        self,
        source: &str,
        editable_range: Range<usize>,
    ) -> Option<LineEndingInsertionContext> {
        if editable_range.start > editable_range.end
            || editable_range.end > source.len()
            || !source.is_char_boundary(editable_range.start)
            || !source.is_char_boundary(editable_range.end)
            || splits_crlf(source, editable_range.start)
            || splits_crlf(source, editable_range.end)
        {
            return None;
        }

        Some(LineEndingInsertionContext {
            profile: self,
            preceding: last_line_ending(&source[..editable_range.start]),
            following: first_line_ending(&source[editable_range.end..]),
        })
    }

    /// Captures insertion policy when the entire supplied string is editable.
    pub const fn full_insertion_context(self) -> LineEndingInsertionContext {
        LineEndingInsertionContext {
            profile: self,
            preceding: None,
            following: None,
        }
    }

    /// Chooses the ending for a logical newline inserted at a rope character index.
    ///
    /// Mixed documents prefer the nearest preceding ending, then the nearest
    /// following ending, then the profile fallback. Returns `None` when the index
    /// is outside the rope or would split an existing CRLF pair.
    pub fn insertion_at(self, text: &Rope, char_index: usize) -> Option<LineEnding> {
        if char_index > text.len_chars() {
            return None;
        }

        if char_index > 0
            && text.get_char(char_index - 1) == Some('\r')
            && text.get_char(char_index) == Some('\n')
        {
            return None;
        }

        if !matches!(self, Self::Mixed { .. }) {
            return Some(self.fallback_insertion());
        }

        let mut preceding = text.chars_at(char_index);
        let preceding = loop {
            let Some(character) = preceding.prev() else {
                break None;
            };
            match character {
                '\n' => {
                    break Some(if preceding.prev() == Some('\r') {
                        LineEnding::CrLf
                    } else {
                        LineEnding::Lf
                    });
                }
                '\r' => break Some(LineEnding::Cr),
                _ => {}
            }
        };

        let mut following = text.chars_at(char_index);
        let following = loop {
            let Some(character) = following.next() else {
                break None;
            };
            match character {
                '\r' => {
                    break Some(if following.next() == Some('\n') {
                        LineEnding::CrLf
                    } else {
                        LineEnding::Cr
                    });
                }
                '\n' => break Some(LineEnding::Lf),
                _ => {}
            }
        };

        Some(select_insertion(self, preceding, following))
    }
}

const fn select_insertion(
    profile: LineEndingProfile,
    preceding: Option<LineEnding>,
    following: Option<LineEnding>,
) -> LineEnding {
    match profile {
        LineEndingProfile::Mixed { insertion, .. } => match (preceding, following) {
            (Some(ending), _) | (None, Some(ending)) => ending,
            (None, None) => insertion,
        },
        _ => profile.fallback_insertion(),
    }
}

fn first_line_ending(text: &str) -> Option<LineEnding> {
    logical_lines(text).find_map(LineSegment::ending)
}

fn last_line_ending(text: &str) -> Option<LineEnding> {
    logical_lines(text).filter_map(LineSegment::ending).last()
}

fn splits_crlf(text: &str, byte_offset: usize) -> bool {
    byte_offset > 0
        && text.as_bytes().get(byte_offset - 1) == Some(&b'\r')
        && text.as_bytes().get(byte_offset) == Some(&b'\n')
}

fn ending_precedes(
    candidate: LineEnding,
    current: LineEnding,
    counts: LineEndingCounts,
    first_positions: [usize; 3],
) -> bool {
    match counts.get(candidate).cmp(&counts.get(current)) {
        Ordering::Greater => true,
        Ordering::Equal => matches!(
            first_positions[candidate.slot()].cmp(&first_positions[current.slot()]),
            Ordering::Less
        ),
        Ordering::Less => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_none_uniform_and_mixed_profiles() {
        assert_eq!(
            LineEndingProfile::detect("plain text"),
            LineEndingProfile::None {
                insertion: LineEnding::platform_default()
            }
        );
        assert_eq!(
            LineEndingProfile::detect("one\r\ntwo\r\n"),
            LineEndingProfile::Uniform {
                ending: LineEnding::CrLf,
                count: 2
            }
        );
        let mixed = LineEndingProfile::detect("one\ntwo\r\nthree\r");
        let expected_counts = LineEndingCounts {
            lf: 1,
            crlf: 1,
            cr: 1,
        };
        assert_eq!(
            mixed,
            LineEndingProfile::Mixed {
                counts: expected_counts,
                insertion: LineEnding::Lf
            }
        );
        assert_eq!(mixed.counts(), expected_counts);
    }

    #[test]
    fn dominant_count_wins_before_first_occurrence_tie_breaking() {
        assert_eq!(
            LineEndingProfile::detect("a\rb\r\nc\r\nd\n"),
            LineEndingProfile::Mixed {
                counts: LineEndingCounts {
                    lf: 1,
                    crlf: 2,
                    cr: 1
                },
                insertion: LineEnding::CrLf
            }
        );
        assert_eq!(
            LineEndingProfile::detect("a\rb\nc\r\n"),
            LineEndingProfile::Mixed {
                counts: LineEndingCounts {
                    lf: 1,
                    crlf: 1,
                    cr: 1
                },
                insertion: LineEnding::Cr
            }
        );
    }

    #[test]
    fn mixed_insertion_prefers_preceding_then_following() {
        let rope = Rope::from_str("head\r\nmiddle\nlast\rend");
        let profile = LineEndingProfile::detect(&rope.to_string());

        assert_eq!(profile.insertion_at(&rope, 0), Some(LineEnding::CrLf));
        assert_eq!(profile.insertion_at(&rope, 12), Some(LineEnding::CrLf));
        assert_eq!(profile.insertion_at(&rope, 13), Some(LineEnding::Lf));
        assert_eq!(
            profile.insertion_at(&rope, rope.len_chars()),
            Some(LineEnding::Cr)
        );
    }

    #[test]
    fn insertion_rejects_invalid_or_split_crlf_positions() {
        let rope = Rope::from_str("a\r\nb\n");
        let profile = LineEndingProfile::detect(&rope.to_string());

        assert_eq!(profile.insertion_at(&rope, 2), None);
        assert_eq!(profile.insertion_at(&rope, rope.len_chars() + 1), None);
    }

    #[test]
    fn mixed_insertion_uses_fallback_when_content_has_no_endings() {
        let profile = LineEndingProfile::Mixed {
            counts: LineEndingCounts {
                lf: 2,
                crlf: 1,
                cr: 0,
            },
            insertion: LineEnding::Lf,
        };

        assert_eq!(
            profile.insertion_at(&Rope::from_str("temporarily stale"), 4),
            Some(LineEnding::Lf)
        );
    }

    #[test]
    fn counts_labels_and_encodings_are_exact() {
        for (ending, expected, label) in [
            (LineEnding::Lf, "\n", "LF"),
            (LineEnding::CrLf, "\r\n", "CRLF"),
            (LineEnding::Cr, "\r", "CR"),
        ] {
            let profile = LineEndingProfile::Uniform { ending, count: 3 };
            assert_eq!(ending.as_str(), expected);
            assert_eq!(profile.counts().get(ending), 3);
            assert_eq!(profile.counts().total(), 3);
            assert_eq!(profile.fallback_insertion(), ending);
            assert_eq!(profile.status_label(), label);
        }

        let none = LineEndingProfile::detect("");
        assert_eq!(none.counts().total(), 0);
        assert_eq!(none.status_label(), "No EOL");
        assert_eq!(LineEndingProfile::detect("\n\r").status_label(), "Mixed");
    }

    #[test]
    fn logical_lines_preserve_mixed_terminators_without_a_synthetic_tail() {
        let segments = logical_lines("one\r\ntwo\rthree\nfour").collect::<Vec<_>>();

        assert_eq!(
            segments,
            vec![
                LineSegment {
                    content: "one",
                    ending: Some(LineEnding::CrLf),
                },
                LineSegment {
                    content: "two",
                    ending: Some(LineEnding::Cr),
                },
                LineSegment {
                    content: "three",
                    ending: Some(LineEnding::Lf),
                },
                LineSegment {
                    content: "four",
                    ending: None,
                },
            ]
        );
        assert!(logical_lines("").next().is_none());
        assert_eq!(logical_lines("\n").count(), 1);
    }

    #[test]
    fn insertion_context_covers_none_uniform_and_mixed_external_neighbors() {
        for source in ["plain", "one\r\ntwo\r\n"] {
            let profile = LineEndingProfile::detect(source);
            let context = profile
                .insertion_context(source, 0..source.len())
                .expect("the full source range should be valid");
            assert_eq!(
                context.insertion_at(source, source.len()),
                Some(profile.fallback_insertion())
            );
        }

        let source = "left\r\nEDIT\nright\r";
        let profile = LineEndingProfile::detect(source);
        let context = profile
            .insertion_context(source, 6..10)
            .expect("the editable range should be valid");
        assert_eq!(context.insertion_at("EDIT", 0), Some(LineEnding::CrLf));
        assert_eq!(context.insertion_at("EDIT", 4), Some(LineEnding::CrLf));

        let source = "EDIT\rrest\n";
        let context = LineEndingProfile::detect(source)
            .insertion_context(source, 0..4)
            .expect("the leading editable range should be valid");
        assert_eq!(context.insertion_at("EDIT", 0), Some(LineEnding::Cr));

        let source = "left\r\nA\nB\rright";
        let context = LineEndingProfile::detect(source)
            .insertion_context(source, 6..10)
            .expect("the mixed editable range should be valid");
        assert_eq!(context.insertion_at("A\nB\r", 2), Some(LineEnding::Lf));
        assert_eq!(context.insertion_at("A\nB\r", 4), Some(LineEnding::Cr));
    }

    #[test]
    fn leading_edit_prefers_the_first_following_ending_over_mixed_fallback() {
        let source = "EDIT\rrest\nmore\n";
        let profile = LineEndingProfile::detect(source);
        assert_eq!(profile.fallback_insertion(), LineEnding::Lf);
        let context = profile
            .insertion_context(source, 0..4)
            .expect("the leading editable range should be valid");

        assert_eq!(context.insertion_at("EDIT", 0), Some(LineEnding::Cr));
        assert_eq!(context.insertion_at("EDIT", 4), Some(LineEnding::Cr));
    }

    #[test]
    fn insertion_context_rejects_invalid_utf8_and_split_crlf_boundaries() {
        let source = "é\r\ntext";
        let profile = LineEndingProfile::detect(source);

        assert!(profile.insertion_context(source, 1..source.len()).is_none());
        assert!(profile.insertion_context(source, 0..3).is_none());
        assert!(profile.insertion_context(source, source.len()..0).is_none());
        assert!(
            profile
                .insertion_context(source, 0..source.len() + 1)
                .is_none()
        );

        let context = profile
            .insertion_context(source, 0..source.len())
            .expect("the full source range should be valid");
        assert_eq!(context.insertion_at(source, 1), None);
        assert_eq!(context.insertion_at(source, 3), None);
        assert_eq!(context.insertion_at(source, source.len() + 1), None);
    }

    #[test]
    fn inserted_text_normalizes_every_newline_form() {
        let inserted = "a\nb\r\nc\rd";
        for (ending, expected) in [
            (LineEnding::Lf, "a\nb\nc\nd"),
            (LineEnding::CrLf, "a\r\nb\r\nc\r\nd"),
            (LineEnding::Cr, "a\rb\rc\rd"),
        ] {
            let normalized = normalize_inserted_text(inserted, ending, usize::MAX);
            assert_eq!(normalized.text(), expected);
            assert_eq!(normalized.consumed_input_bytes(), inserted.len());
            assert!(!normalized.was_limited());
            assert_eq!(normalized.into_text(), expected);
        }
    }

    #[test]
    fn inserted_text_ceiling_never_splits_unicode_or_crlf() {
        for (maximum, expected, consumed) in [
            (0, "", 0),
            (1, "a", 1),
            (2, "a", 1),
            (3, "aé", 3),
            (4, "aé", 3),
            (5, "aé\r\n", 4),
            (6, "aé\r\n", 4),
            (7, "aé\r\n", 4),
            (8, "aé\r\n字", 7),
        ] {
            let normalized = normalize_inserted_text("aé\n字", LineEnding::CrLf, maximum);
            assert_eq!(normalized.text(), expected);
            assert_eq!(normalized.consumed_input_bytes(), consumed);
            assert!(normalized.text().len() <= maximum);
            assert!(!normalized.text().ends_with('\r'));
            assert_eq!(normalized.was_limited(), maximum < 8);
        }
    }

    #[test]
    fn inserted_text_consumption_follows_exact_multibyte_boundaries() {
        for (maximum, expected, consumed, limited) in [
            (0, "", 0, true),
            (1, "", 0, true),
            (2, "é", 2, true),
            (4, "é", 2, true),
            (5, "é字", 5, true),
            (6, "é字x", 6, false),
        ] {
            let normalized = normalize_inserted_text("é字x", LineEnding::Lf, maximum);
            assert_eq!(normalized.text(), expected, "maximum={maximum}");
            assert_eq!(
                normalized.consumed_input_bytes(),
                consumed,
                "maximum={maximum}"
            );
            assert_eq!(normalized.was_limited(), limited, "maximum={maximum}");
        }
    }

    #[test]
    fn bounded_normalization_properties_hold_for_payload_and_ceiling_matrix() {
        let payloads = ["", "\n", "\r", "\r\n", "é\r\n字\rX\n", "\r\r\n\n"];
        for payload in payloads {
            for ending in [LineEnding::Lf, LineEnding::CrLf, LineEnding::Cr] {
                for maximum in 0..=payload.len().saturating_mul(2).saturating_add(4) {
                    let normalized = normalize_inserted_text(payload, ending, maximum);
                    assert!(normalized.text().len() <= maximum);
                    assert!(normalized.text().is_char_boundary(normalized.text().len()));
                    assert!(payload.is_char_boundary(normalized.consumed_input_bytes()));
                    assert!(normalized.consumed_input_bytes() <= payload.len());
                    assert!(
                        logical_lines(normalized.text())
                            .filter_map(LineSegment::ending)
                            .all(|actual| actual == ending)
                    );
                    if !normalized.was_limited() {
                        assert_eq!(normalized.consumed_input_bytes(), payload.len());
                        let expected_logical_characters = payload
                            .replace("\r\n", "\n")
                            .replace('\r', "\n")
                            .chars()
                            .count();
                        let actual_logical_characters = normalized
                            .text()
                            .replace("\r\n", "\n")
                            .replace('\r', "\n")
                            .chars()
                            .count();
                        assert_eq!(actual_logical_characters, expected_logical_characters);
                    }
                }
            }
        }
    }
}
