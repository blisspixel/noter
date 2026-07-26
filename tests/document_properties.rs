//! Generated document and line-ending invariants.

use noter::core::document::Document;
use noter::core::line_endings::{LineEnding, LineEndingCounts, LineEndingProfile};
use proptest::prelude::*;
use proptest::test_runner::FileFailurePersistence;
use ropey::Rope;

fn line_ending_strategy() -> impl Strategy<Value = LineEnding> {
    prop_oneof![
        Just(LineEnding::Lf),
        Just(LineEnding::CrLf),
        Just(LineEnding::Cr),
    ]
}

fn expected_profile(endings: &[LineEnding]) -> LineEndingProfile {
    if endings.is_empty() {
        return LineEndingProfile::None {
            insertion: LineEnding::platform_default(),
        };
    }

    let mut counts = LineEndingCounts::default();
    for ending in endings {
        match ending {
            LineEnding::Lf => counts.lf += 1,
            LineEnding::CrLf => counts.crlf += 1,
            LineEnding::Cr => counts.cr += 1,
        }
    }

    let kinds =
        usize::from(counts.lf > 0) + usize::from(counts.crlf > 0) + usize::from(counts.cr > 0);
    if kinds == 1 {
        return LineEndingProfile::Uniform {
            ending: endings[0],
            count: endings.len(),
        };
    }

    let highest_count = counts.lf.max(counts.crlf).max(counts.cr);
    let insertion = endings
        .iter()
        .copied()
        .find(|ending| counts.get(*ending) == highest_count)
        .expect("a non-empty sequence has a dominant ending");
    LineEndingProfile::Mixed { counts, insertion }
}

fn property_config() -> ProptestConfig {
    ProptestConfig {
        cases: 512,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(
            "tests/document_properties.proptest-regressions",
        ))),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(property_config())]

    #[test]
    fn arbitrary_strict_utf8_round_trips_byte_for_byte(text in any::<String>(), bom in any::<bool>()) {
        let mut bytes = Vec::with_capacity(text.len() + usize::from(bom) * 3);
        if bom {
            bytes.extend_from_slice(b"\xEF\xBB\xBF");
        }
        bytes.extend_from_slice(text.as_bytes());

        let document = Document::from_bytes(&bytes).expect("generated strings are valid UTF-8");

        prop_assert_eq!(document.to_bytes(), bytes);
    }

    #[test]
    fn profiles_count_all_generated_endings_exactly(
        endings in proptest::collection::vec(line_ending_strategy(), 0..256)
    ) {
        let mut text = String::new();
        for ending in &endings {
            text.push('x');
            text.push_str(ending.as_str());
        }

        prop_assert_eq!(LineEndingProfile::detect(&text), expected_profile(&endings));
    }

    #[test]
    fn insertion_uses_last_preceding_then_first_following(
        preceding in proptest::collection::vec(line_ending_strategy(), 0..32),
        following in proptest::collection::vec(line_ending_strategy(), 0..32),
    ) {
        let mut text = String::new();
        for ending in &preceding {
            text.push('x');
            text.push_str(ending.as_str());
        }
        let insertion_index = text.chars().count();
        for ending in &following {
            text.push('x');
            text.push_str(ending.as_str());
        }

        let profile = LineEndingProfile::detect(&text);
        let expected = preceding
            .last()
            .copied()
            .or_else(|| following.first().copied())
            .unwrap_or_else(LineEnding::platform_default);

        prop_assert_eq!(
            profile.insertion_at(&Rope::from_str(&text), insertion_index),
            Some(expected)
        );
    }
}
