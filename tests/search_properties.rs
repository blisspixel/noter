//! Generated literal-search and replacement invariants.

use noter::core::edit::TextRange;
use noter::core::search::{LiteralSearch, MatchCase, SearchDirection, SearchNavigation};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

fn property_config() -> ProptestConfig {
    ProptestConfig {
        cases: 512,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(0x4E4F_5445_525F_4D34),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(property_config())]

    #[test]
    fn case_sensitive_search_matches_the_standard_literal_reference(
        source_chars in proptest::collection::vec(any::<char>(), 0..128),
        query_chars in proptest::collection::vec(any::<char>(), 1..8),
        replacement_chars in proptest::collection::vec(any::<char>(), 0..8),
        position in any::<usize>(),
    ) {
        let source = source_chars.into_iter().collect::<String>();
        let query = query_chars.into_iter().collect::<String>();
        let replacement = replacement_chars.into_iter().collect::<String>();
        let search = LiteralSearch::new(&query, MatchCase::Sensitive)
            .expect("bounded generated literal should compile");
        let expected = source
            .match_indices(&query)
            .map(|(start, matched)| TextRange::new(start, start + matched.len()))
            .collect::<Vec<_>>();

        prop_assert_eq!(search.match_count(&source), expected.len());
        let normalized = position.min(source.len());
        let expected_next = expected
            .iter()
            .position(|range| range.start() >= normalized)
            .or_else(|| (!expected.is_empty()).then_some(0));
        let expected_previous = expected
            .iter()
            .rposition(|range| range.end() <= normalized)
            .or_else(|| (!expected.is_empty()).then(|| expected.len() - 1));

        let next = search.navigate(&source, position, SearchDirection::Next);
        prop_assert_eq!(next.map(SearchNavigation::range), expected_next.map(|index| expected[index]));
        prop_assert_eq!(
            next.map(SearchNavigation::wrapped),
            expected_next.map(|index| expected[index].start() < normalized),
        );
        let previous = search.navigate(&source, position, SearchDirection::Previous);
        prop_assert_eq!(
            previous.map(SearchNavigation::range),
            expected_previous.map(|index| expected[index]),
        );
        prop_assert_eq!(
            previous.map(SearchNavigation::wrapped),
            expected_previous.map(|index| expected[index].end() > normalized),
        );

        let replaced = search
            .replace_all(
                &source,
                TextRange::new(0, source.len()),
                &replacement,
                usize::MAX,
            )
            .expect("bounded generated replacement should remain valid");
        if expected.is_empty() {
            prop_assert!(replaced.is_none());
        } else {
            let replaced = replaced.expect("a reference match should produce a replacement");
            prop_assert_eq!(replaced.replacement_count(), expected.len());
            prop_assert_eq!(replaced.text(), source.replace(&query, &replacement));
        }
    }
}
