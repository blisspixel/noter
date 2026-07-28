//! Generated edit, inverse, undo, and redo invariants.

use noter::core::document::Document;
use noter::core::edit::{
    EditOrigin, EditTimestamp, EditTransaction, Selection, TextEdit, TextRange,
};
use noter::core::revision::Revision;
use noter::core::undo::{HistoryRecordOutcome, UndoHistory};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

fn byte_boundaries(source: &str) -> Vec<usize> {
    source
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(source.len()))
        .collect()
}

fn seeded_range(source: &str, first: usize, second: usize) -> TextRange {
    let boundaries = byte_boundaries(source);
    let left = boundaries[first % boundaries.len()];
    let right = boundaries[second % boundaries.len()];
    TextRange::new(left.min(right), left.max(right))
}

fn seeded_selection(source: &str, anchor: usize, active: usize) -> Selection {
    let boundaries = byte_boundaries(source);
    Selection::new(
        boundaries[anchor % boundaries.len()],
        boundaries[active % boundaries.len()],
    )
}

fn property_config() -> ProptestConfig {
    ProptestConfig {
        cases: 512,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(0x4E4F_5445_525F_4D33),
        ..ProptestConfig::default()
    }
}

#[derive(Clone, Debug)]
enum HistoryAction {
    Edit {
        inserted: String,
        first: usize,
        second: usize,
    },
    Undo,
    Redo,
}

fn history_action() -> impl Strategy<Value = HistoryAction> {
    prop_oneof![
        6 => (any::<String>(), any::<usize>(), any::<usize>()).prop_map(
            |(inserted, first, second)| HistoryAction::Edit {
                inserted,
                first,
                second,
            }
        ),
        2 => Just(HistoryAction::Undo),
        2 => Just(HistoryAction::Redo),
    ]
}

proptest! {
    #![proptest_config(property_config())]

    #[test]
    fn transaction_and_inverse_match_a_string_reference_model(
        source in any::<String>(),
        inserted in any::<String>(),
        range_start in any::<usize>(),
        range_end in any::<usize>(),
        before_anchor in any::<usize>(),
        before_active in any::<usize>(),
        after_anchor in any::<usize>(),
        after_active in any::<usize>(),
    ) {
        // A leading U+FEFF is decoded as the document BOM metadata rather than
        // source text, so it is outside this source-string reference model.
        prop_assume!(!source.starts_with('\u{feff}'));
        let range = seeded_range(&source, range_start, range_end);
        let removed = source[range.start()..range.end()].to_owned();
        prop_assume!(inserted != removed);
        let mut expected = source.clone();
        expected.replace_range(range.start()..range.end(), &inserted);
        let before = seeded_selection(&source, before_anchor, before_active);
        let after = seeded_selection(&expected, after_anchor, after_active);
        let transaction = EditTransaction::new(
            Revision::INITIAL,
            vec![TextEdit::replace(range, inserted, removed)],
            before,
            after,
            EditOrigin::TextInput,
            EditTimestamp::default(),
        );
        let mut document = Document::from_bytes(source.as_bytes())
            .expect("generated strings are valid UTF-8");

        let applied = document
            .apply_transaction(&transaction)
            .expect("generated valid transaction should apply");
        prop_assert_eq!(document.rope().to_string(), expected);
        prop_assert_eq!(applied.selection(), after);
        prop_assert!(document.is_dirty());

        let inverse = applied.into_inverse();
        let undone = document
            .apply_transaction(&inverse)
            .expect("the generated inverse should apply");
        prop_assert_eq!(document.rope().to_string(), source);
        prop_assert_eq!(undone.selection(), before);
        prop_assert!(!document.is_dirty());
    }

    #[test]
    fn ordered_multi_edit_transactions_match_a_string_reference_model(
        source in any::<String>(),
        candidates in proptest::collection::vec(
            (any::<usize>(), any::<usize>(), any::<String>()),
            1..9,
        ),
        before_anchor in any::<usize>(),
        before_active in any::<usize>(),
        after_anchor in any::<usize>(),
        after_active in any::<usize>(),
    ) {
        prop_assume!(!source.starts_with('\u{feff}'));
        let mut candidates = candidates
            .into_iter()
            .map(|(first, second, inserted)| (seeded_range(&source, first, second), inserted))
            .collect::<Vec<_>>();
        candidates.sort_by_key(|(range, _)| (range.start(), range.end()));

        let mut previous_end = 0;
        let mut edits = Vec::new();
        for (range, inserted) in candidates {
            if range.start() < previous_end {
                continue;
            }
            let removed = source[range.start()..range.end()].to_owned();
            previous_end = range.end();
            if inserted != removed {
                edits.push(TextEdit::replace(range, inserted, removed));
            }
        }
        prop_assume!(!edits.is_empty());

        let mut expected = source.clone();
        for edit in edits.iter().rev() {
            expected.replace_range(edit.range().start()..edit.range().end(), edit.inserted());
        }
        let before = seeded_selection(&source, before_anchor, before_active);
        let after = seeded_selection(&expected, after_anchor, after_active);
        let transaction = EditTransaction::new(
            Revision::INITIAL,
            edits,
            before,
            after,
            EditOrigin::Replace,
            EditTimestamp::default(),
        );
        let mut document = Document::from_bytes(source.as_bytes())
            .expect("generated strings are valid UTF-8");

        let applied = document
            .apply_transaction(&transaction)
            .expect("generated disjoint transaction should apply");
        prop_assert_eq!(document.rope().to_string(), expected);
        prop_assert_eq!(applied.selection(), after);

        let undone = document
            .apply_transaction(&applied.into_inverse())
            .expect("the generated multi-edit inverse should apply");
        prop_assert_eq!(document.rope().to_string(), source);
        prop_assert_eq!(undone.selection(), before);
        prop_assert!(!document.is_dirty());
    }

    #[test]
    fn arbitrary_edit_sequences_undo_and_redo_against_the_reference_model(
        initial in any::<String>(),
        actions in proptest::collection::vec(history_action(), 0..64),
    ) {
        prop_assume!(!initial.starts_with('\u{feff}'));
        let mut document = Document::from_bytes(initial.as_bytes())
            .expect("generated strings are valid UTF-8");
        let mut history = UndoHistory::default();
        let mut expected = initial.clone();
        let mut undo_states = Vec::new();
        let mut redo_states = Vec::new();

        for action in actions {
            match action {
                HistoryAction::Edit {
                    inserted,
                    first,
                    second,
                } => {
                    let range = seeded_range(&expected, first, second);
                    let mut next = expected.clone();
                    next.replace_range(range.start()..range.end(), &inserted);
                    if next == expected {
                        continue;
                    }
                    let transaction = EditTransaction::between(
                        document.revision(),
                        &expected,
                        &next,
                        Selection::caret(range.start()),
                        Selection::caret(range.start() + inserted.len()),
                        EditOrigin::TextInput,
                        EditTimestamp::default(),
                    )
                    .expect("generated selections should be valid")
                    .expect("different content should produce a transaction");
                    let applied = document
                        .apply_transaction(&transaction)
                        .expect("generated transaction should apply");
                    prop_assert_eq!(history.record(applied), HistoryRecordOutcome::Stored);
                    undo_states.push(expected);
                    redo_states.clear();
                    expected = next;
                }
                HistoryAction::Undo => {
                    let outcome = history
                        .undo(&mut document)
                        .expect("generated Undo should remain synchronized");
                    if let Some(previous) = undo_states.pop() {
                        prop_assert!(outcome.is_some());
                        redo_states.push(expected);
                        expected = previous;
                    } else {
                        prop_assert!(outcome.is_none());
                    }
                }
                HistoryAction::Redo => {
                    let outcome = history
                        .redo(&mut document)
                        .expect("generated Redo should remain synchronized");
                    if let Some(next) = redo_states.pop() {
                        prop_assert!(outcome.is_some());
                        undo_states.push(expected);
                        expected = next;
                    } else {
                        prop_assert!(outcome.is_none());
                    }
                }
            }
            prop_assert_eq!(document.rope().to_string(), expected.clone());
            prop_assert_eq!(history.len(), undo_states.len() + redo_states.len());
            prop_assert_eq!(history.can_undo(), !undo_states.is_empty());
            prop_assert_eq!(history.can_redo(), !redo_states.is_empty());
            prop_assert_eq!(document.is_dirty(), expected != initial);
        }
    }
}
