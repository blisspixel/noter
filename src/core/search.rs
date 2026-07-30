//! Bounded literal search and replacement policy.

use regex::{Regex, RegexBuilder};
use thiserror::Error;

use super::edit::TextRange;
/// Maximum UTF-8 byte length accepted for one literal query.
pub const MAX_LITERAL_QUERY_BYTES: usize = 16_384;
/// Maximum UTF-8 byte length accepted for one literal replacement.
pub const MAX_LITERAL_REPLACEMENT_BYTES: usize = 16_384;
const MAX_COMPILED_PATTERN_BYTES: usize = 4_194_304;
const MAX_DFA_CACHE_BYTES: usize = 2_097_152;

/// Whether literal matching distinguishes Unicode letter case.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub enum MatchCase {
    /// Match Unicode case exactly.
    Sensitive,
    /// Use the regex engine's Unicode-aware simple case folding.
    #[default]
    Insensitive,
}

/// Direction used to select one match relative to a byte position.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SearchDirection {
    /// Select the first match starting at or after the position.
    Next,
    /// Select the last match ending at or before the position.
    Previous,
}

/// One selected match and its position among all non-overlapping matches.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SearchNavigation {
    range: TextRange,
    ordinal: usize,
    match_count: usize,
    wrapped: bool,
}

impl SearchNavigation {
    /// Returns the selected UTF-8 source range.
    pub const fn range(self) -> TextRange {
        self.range
    }

    /// Returns the one-based index of the selected match.
    pub const fn ordinal(self) -> usize {
        self.ordinal
    }

    /// Returns the total number of non-overlapping matches.
    pub const fn match_count(self) -> usize {
        self.match_count
    }

    /// Returns whether navigation crossed a document boundary.
    pub const fn wrapped(self) -> bool {
        self.wrapped
    }
}

/// A complete replacement for one validated source scope.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LiteralReplacement {
    text: String,
    replacement_count: usize,
}

impl LiteralReplacement {
    /// Returns the replacement text for the requested scope.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Consumes the result and returns its replacement text.
    pub fn into_text(self) -> String {
        self.text
    }

    /// Returns the number of replaced non-overlapping matches.
    pub const fn replacement_count(&self) -> usize {
        self.replacement_count
    }
}

/// A compiled, literal-only search query.
///
/// Metacharacters are escaped before compilation. The engine guarantees linear
/// matching time, reports source byte ranges directly, and avoids retaining a
/// potentially document-sized match vector.
#[derive(Clone)]
pub struct LiteralSearch {
    matcher: Option<Regex>,
}

impl LiteralSearch {
    /// Compiles a bounded literal query with the requested case policy.
    ///
    /// An empty query is valid and intentionally matches nothing.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::QueryTooLong`] when the query exceeds the public
    /// ceiling, or [`SearchError::PatternRejected`] when the escaped literal
    /// cannot compile inside the internal memory limits.
    pub fn new(query: &str, match_case: MatchCase) -> Result<Self, SearchError> {
        if query.len() > MAX_LITERAL_QUERY_BYTES {
            return Err(SearchError::QueryTooLong {
                actual: query.len(),
                maximum: MAX_LITERAL_QUERY_BYTES,
            });
        }
        if query.is_empty() {
            return Ok(Self { matcher: None });
        }

        let matcher = RegexBuilder::new(&regex::escape(query))
            .case_insensitive(match_case == MatchCase::Insensitive)
            .unicode(true)
            .size_limit(MAX_COMPILED_PATTERN_BYTES)
            .dfa_size_limit(MAX_DFA_CACHE_BYTES)
            .build()
            .map_err(|_| SearchError::PatternRejected)?;
        Ok(Self {
            matcher: Some(matcher),
        })
    }

    /// Counts non-overlapping matches without retaining their ranges.
    pub fn match_count(&self, source: &str) -> usize {
        self.matcher.as_ref().map_or(0, |matcher| {
            matcher
                .find_iter(source)
                .fold(0_usize, |count, _| count.saturating_add(1))
        })
    }

    /// Selects one match relative to `position` and reports wrap behavior.
    ///
    /// Positions beyond the source are clamped to its byte length. They do not
    /// need to be UTF-8 boundaries because matching comparisons use byte
    /// offsets and returned ranges always come from the UTF-8-aware engine.
    pub fn navigate(
        &self,
        source: &str,
        position: usize,
        direction: SearchDirection,
    ) -> Option<SearchNavigation> {
        let matcher = self.matcher.as_ref()?;
        let position = position.min(source.len());
        let mut first = None;
        let mut last = None;
        let mut selected = None;
        let mut match_count = 0_usize;

        for found in matcher.find_iter(source) {
            match_count = match_count.saturating_add(1);
            let candidate = (TextRange::new(found.start(), found.end()), match_count);
            first.get_or_insert(candidate);
            last = Some(candidate);
            match direction {
                SearchDirection::Next if selected.is_none() && found.start() >= position => {
                    selected = Some(candidate);
                }
                SearchDirection::Previous if found.end() <= position => {
                    selected = Some(candidate);
                }
                SearchDirection::Next | SearchDirection::Previous => {}
            }
        }

        let (range, ordinal, wrapped) = match (direction, selected) {
            (_, Some((range, ordinal))) => (range, ordinal, false),
            (SearchDirection::Next, None) => {
                let (range, ordinal) = first?;
                (range, ordinal, true)
            }
            (SearchDirection::Previous, None) => {
                let (range, ordinal) = last?;
                (range, ordinal, true)
            }
        };
        Some(SearchNavigation {
            range,
            ordinal,
            match_count,
            wrapped,
        })
    }

    /// Returns whether `range` is exactly one complete match.
    pub fn matches_range(&self, source: &str, range: TextRange) -> bool {
        let Some(matcher) = self.matcher.as_ref() else {
            return false;
        };
        let Some(candidate) = source.get(range.start()..range.end()) else {
            return false;
        };
        matcher
            .find(candidate)
            .is_some_and(|found| found.start() == 0 && found.end() == candidate.len())
    }

    /// Replaces every non-overlapping match inside one validated source scope.
    ///
    /// The replacement is always literal, including `$` characters. Source
    /// outside the scope is not copied or returned. `None` means no match was
    /// found and lets callers avoid an unnecessary document mutation.
    ///
    /// # Errors
    ///
    /// Returns [`SearchError::InvalidScope`] when the scope is unordered, out
    /// of bounds, or splits a UTF-8 scalar value. It returns a resource error
    /// before allocation when the resulting document would exceed the shared
    /// document ceiling or when bounded allocation is unavailable.
    pub fn replace_all(
        &self,
        source: &str,
        scope: TextRange,
        replacement: &str,
        maximum_result_bytes: usize,
    ) -> Result<Option<LiteralReplacement>, SearchError> {
        if replacement.len() > MAX_LITERAL_REPLACEMENT_BYTES {
            return Err(SearchError::ReplacementTooLong {
                actual: replacement.len(),
                maximum: MAX_LITERAL_REPLACEMENT_BYTES,
            });
        }
        let Some(segment) = source.get(scope.start()..scope.end()) else {
            return Err(SearchError::InvalidScope {
                start: scope.start(),
                end: scope.end(),
                source_len: source.len(),
            });
        };
        let Some(matcher) = self.matcher.as_ref() else {
            return Ok(None);
        };

        let mut replacement_count = 0_usize;
        let mut removed_bytes = 0_usize;
        for found in matcher.find_iter(segment) {
            removed_bytes = removed_bytes
                .checked_add(found.end().saturating_sub(found.start()))
                .ok_or(SearchError::ResultTooLarge {
                    projected: usize::MAX,
                    maximum: maximum_result_bytes,
                })?;
            replacement_count = replacement_count.saturating_add(1);
        }
        if replacement_count == 0 {
            return Ok(None);
        }
        let inserted_bytes = replacement.len().checked_mul(replacement_count).ok_or(
            SearchError::ResultTooLarge {
                projected: usize::MAX,
                maximum: maximum_result_bytes,
            },
        )?;
        let scoped_result_bytes = segment
            .len()
            .checked_sub(removed_bytes)
            .and_then(|retained| retained.checked_add(inserted_bytes))
            .ok_or(SearchError::ResultTooLarge {
                projected: usize::MAX,
                maximum: maximum_result_bytes,
            })?;
        let projected = source
            .len()
            .checked_sub(segment.len())
            .and_then(|outside| outside.checked_add(scoped_result_bytes))
            .ok_or(SearchError::ResultTooLarge {
                projected: usize::MAX,
                maximum: maximum_result_bytes,
            })?;
        if projected > maximum_result_bytes {
            return Err(SearchError::ResultTooLarge {
                projected,
                maximum: maximum_result_bytes,
            });
        }

        let mut text = String::new();
        text.try_reserve_exact(scoped_result_bytes).map_err(|_| {
            SearchError::AllocationUnavailable {
                requested: scoped_result_bytes,
            }
        })?;
        let mut consumed = 0;
        for found in matcher.find_iter(segment) {
            text.push_str(&segment[consumed..found.start()]);
            text.push_str(replacement);
            consumed = found.end();
        }
        text.push_str(&segment[consumed..]);
        debug_assert_eq!(text.len(), scoped_result_bytes);
        Ok(Some(LiteralReplacement {
            text,
            replacement_count,
        }))
    }
}

/// A bounded literal-search validation failure.
#[derive(Clone, PartialEq, Eq, Error, Debug)]
pub enum SearchError {
    /// The query exceeds the public resource ceiling.
    #[error("search query is {actual} bytes; the maximum is {maximum} bytes")]
    QueryTooLong {
        /// Actual query length in UTF-8 bytes.
        actual: usize,
        /// Maximum accepted query length in UTF-8 bytes.
        maximum: usize,
    },
    /// The literal replacement exceeds the public resource ceiling.
    #[error("replacement text is {actual} bytes; the maximum is {maximum} bytes")]
    ReplacementTooLong {
        /// Actual replacement length in UTF-8 bytes.
        actual: usize,
        /// Maximum accepted replacement length in UTF-8 bytes.
        maximum: usize,
    },
    /// The escaped query exceeded an internal regex compilation safety limit.
    #[error("search query could not be prepared within the configured resource limits")]
    PatternRejected,
    /// The replacement scope was unordered, out of bounds, or not UTF-8 aligned.
    #[error("replacement scope {start}..{end} is invalid for {source_len} source bytes")]
    InvalidScope {
        /// Requested inclusive start byte offset.
        start: usize,
        /// Requested exclusive end byte offset.
        end: usize,
        /// Available UTF-8 source bytes.
        source_len: usize,
    },
    /// The operation would create a document beyond the shared byte ceiling.
    #[error("replacement would create {projected} bytes; the maximum is {maximum} bytes")]
    ResultTooLarge {
        /// Projected result length, or `usize::MAX` after arithmetic overflow.
        projected: usize,
        /// Maximum supported document length.
        maximum: usize,
    },
    /// The bounded result allocation could not be reserved.
    #[error("replacement could not reserve {requested} bytes of result memory")]
    AllocationUnavailable {
        /// Exact scoped result allocation requested.
        requested: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::limits::MAX_DOCUMENT_BYTES;

    #[test]
    fn empty_query_matches_nothing_and_performs_no_replacement() {
        let search = LiteralSearch::new("", MatchCase::Insensitive)
            .expect("empty query should be a valid disabled search");

        assert_eq!(search.match_count("anything"), 0);
        assert_eq!(search.navigate("anything", 0, SearchDirection::Next), None);
        assert_eq!(
            search
                .replace_all(
                    "anything",
                    TextRange::new(0, 8),
                    "replacement",
                    MAX_DOCUMENT_BYTES,
                )
                .expect("valid scope should be accepted"),
            None
        );
    }

    #[test]
    fn query_is_literal_and_case_policy_is_unicode_aware() {
        let literal = LiteralSearch::new("a.b [x]", MatchCase::Sensitive)
            .expect("literal metacharacters should compile");
        assert_eq!(literal.match_count("a0b x a.b [x]"), 1);

        let sensitive = LiteralSearch::new("école", MatchCase::Sensitive)
            .expect("Unicode query should compile");
        let insensitive = LiteralSearch::new("école", MatchCase::Insensitive)
            .expect("Unicode folded query should compile");
        assert_eq!(sensitive.match_count("ÉCOLE école"), 1);
        assert_eq!(insensitive.match_count("ÉCOLE école"), 2);
    }

    #[test]
    fn next_and_previous_report_ordinals_and_wrap_without_storing_an_index() {
        let search =
            LiteralSearch::new("two", MatchCase::Sensitive).expect("fixture query should compile");
        let source = "two one two three two";

        let second = search
            .navigate(source, 1, SearchDirection::Next)
            .expect("the second match should be selected");
        assert_eq!(second.ordinal(), 2);
        assert_eq!(second.match_count(), 3);

        assert_eq!(
            Some(second),
            Some(SearchNavigation {
                range: TextRange::new(8, 11),
                ordinal: 2,
                match_count: 3,
                wrapped: false,
            })
        );
        assert_eq!(
            search.navigate(source, source.len(), SearchDirection::Next),
            Some(SearchNavigation {
                range: TextRange::new(0, 3),
                ordinal: 1,
                match_count: 3,
                wrapped: true,
            })
        );
        assert_eq!(
            search.navigate(source, 8, SearchDirection::Previous),
            Some(SearchNavigation {
                range: TextRange::new(0, 3),
                ordinal: 1,
                match_count: 3,
                wrapped: false,
            })
        );
        assert_eq!(
            search.navigate(source, 0, SearchDirection::Previous),
            Some(SearchNavigation {
                range: TextRange::new(18, 21),
                ordinal: 3,
                match_count: 3,
                wrapped: true,
            })
        );
    }

    #[test]
    fn exact_match_check_rejects_partial_and_invalid_ranges() {
        let search =
            LiteralSearch::new("é", MatchCase::Insensitive).expect("fixture query should compile");

        assert!(search.matches_range("xÉy", TextRange::new(1, 3)));
        assert!(!search.matches_range("xÉy", TextRange::new(0, 3)));
        assert!(!search.matches_range("xÉy", TextRange::new(2, 3)));
        assert!(!search.matches_range("xÉy", TextRange::new(4, 3)));
    }

    #[test]
    fn replace_all_is_literal_and_confined_to_the_validated_scope() {
        let search = LiteralSearch::new("cat", MatchCase::Insensitive)
            .expect("fixture query should compile");
        let source = "Cat cat CAT outside";
        let replaced = search
            .replace_all(source, TextRange::new(4, 11), "$1", MAX_DOCUMENT_BYTES)
            .expect("valid scope should be accepted")
            .expect("scope should contain matches");

        assert_eq!(replaced.text(), "$1 $1");
        assert_eq!(replaced.replacement_count(), 2);
        assert_eq!(replaced.into_text(), "$1 $1");
        assert_eq!(source, "Cat cat CAT outside");
    }

    #[test]
    fn replacement_rejects_unordered_out_of_bounds_and_split_utf8_scopes() {
        let search =
            LiteralSearch::new("x", MatchCase::Sensitive).expect("fixture query should compile");

        for range in [
            TextRange::new(2, 1),
            TextRange::new(0, 4),
            TextRange::new(1, 2),
        ] {
            assert!(matches!(
                search.replace_all("éx", range, "y", MAX_DOCUMENT_BYTES),
                Err(SearchError::InvalidScope { .. })
            ));
        }
    }

    #[test]
    fn replacement_rejects_document_growth_before_allocating_the_result() {
        let search =
            LiteralSearch::new("x", MatchCase::Sensitive).expect("fixture query should compile");
        let source = "xxx";
        let replacement = "yyyy";

        assert!(matches!(
            search.replace_all(
                source,
                TextRange::new(0, source.len()),
                replacement,
                8,
            ),
            Err(SearchError::ResultTooLarge {
                projected,
                maximum: 8,
            }) if projected == 12
        ));
    }

    #[test]
    fn replacement_accepts_the_exact_document_size_ceiling() {
        let search =
            LiteralSearch::new("x", MatchCase::Sensitive).expect("fixture query should compile");
        let replacement = "yyyy";
        let replaced = search
            .replace_all("xx", TextRange::new(0, 2), replacement, 8)
            .expect("the exact result ceiling should be accepted")
            .expect("both literals should be replaced");

        assert_eq!(replaced.replacement_count(), 2);
        assert_eq!(replaced.text().len(), 8);
    }

    #[test]
    fn query_resource_ceiling_is_exact() {
        let accepted = "x".repeat(MAX_LITERAL_QUERY_BYTES);
        let rejected = "x".repeat(MAX_LITERAL_QUERY_BYTES + 1);

        assert!(LiteralSearch::new(&accepted, MatchCase::Sensitive).is_ok());
        assert!(matches!(
            LiteralSearch::new(&rejected, MatchCase::Sensitive),
            Err(SearchError::QueryTooLong {
                actual,
                maximum: MAX_LITERAL_QUERY_BYTES,
            }) if actual == MAX_LITERAL_QUERY_BYTES + 1
        ));
    }

    #[test]
    fn replacement_resource_ceiling_is_exact() {
        let search =
            LiteralSearch::new("x", MatchCase::Sensitive).expect("fixture query should compile");
        let accepted = "y".repeat(MAX_LITERAL_REPLACEMENT_BYTES);
        let rejected = "y".repeat(MAX_LITERAL_REPLACEMENT_BYTES + 1);

        assert!(
            search
                .replace_all("x", TextRange::new(0, 1), &accepted, MAX_DOCUMENT_BYTES,)
                .is_ok()
        );
        assert!(matches!(
            search.replace_all(
                "x",
                TextRange::new(0, 1),
                &rejected,
                MAX_DOCUMENT_BYTES,
            ),
            Err(SearchError::ReplacementTooLong {
                actual,
                maximum: MAX_LITERAL_REPLACEMENT_BYTES,
            }) if actual == MAX_LITERAL_REPLACEMENT_BYTES + 1
        ));
    }
}
