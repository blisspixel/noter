//! Exact line-ending classification and insertion policy.

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

impl LineEndingProfile {
    /// Classifies every line-ending sequence in strict UTF-8 text.
    pub fn detect(text: &str) -> Self {
        let bytes = text.as_bytes();
        let mut counts = LineEndingCounts::default();
        let mut first_positions = [usize::MAX; 3];
        let mut first_ending = None;
        let mut index = 0;

        while index < bytes.len() {
            let (ending, width) = match bytes[index] {
                b'\r' if bytes.get(index + 1) == Some(&b'\n') => (LineEnding::CrLf, 2),
                b'\r' => (LineEnding::Cr, 1),
                b'\n' => (LineEnding::Lf, 1),
                _ => {
                    index += 1;
                    continue;
                }
            };

            counts.increment(ending);
            first_ending.get_or_insert(ending);
            let slot = ending.slot();
            if first_positions[slot] == usize::MAX {
                first_positions[slot] = index;
            }
            index += width;
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
            let candidate_count = counts.get(candidate);
            let insertion_count = counts.get(insertion);
            if candidate_count > insertion_count
                || (candidate_count == insertion_count
                    && first_positions[candidate.slot()] < first_positions[insertion.slot()])
            {
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
            && char_index < text.len_chars()
            && text.get_char(char_index - 1) == Some('\r')
            && text.get_char(char_index) == Some('\n')
        {
            return None;
        }

        let Self::Mixed { insertion, .. } = self else {
            return Some(self.fallback_insertion());
        };

        let mut preceding = text.chars_at(char_index);
        while let Some(character) = preceding.prev() {
            match character {
                '\n' => {
                    return Some(if preceding.prev() == Some('\r') {
                        LineEnding::CrLf
                    } else {
                        LineEnding::Lf
                    });
                }
                '\r' => return Some(LineEnding::Cr),
                _ => {}
            }
        }

        let mut following = text.chars_at(char_index);
        while let Some(character) = following.next() {
            match character {
                '\r' => {
                    return Some(if following.next() == Some('\n') {
                        LineEnding::CrLf
                    } else {
                        LineEnding::Cr
                    });
                }
                '\n' => return Some(LineEnding::Lf),
                _ => {}
            }
        }

        Some(insertion)
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
}
