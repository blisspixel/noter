//! Revision-checked, reversible text transactions.

use std::time::Duration;

use ropey::Rope;
use thiserror::Error;

use super::revision::Revision;

/// A half-open UTF-8 byte range in document source.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct TextRange {
    start: usize,
    end: usize,
}

impl TextRange {
    /// Creates a half-open byte range without consulting document content.
    ///
    /// Applying a transaction validates order, bounds, and UTF-8 boundaries
    /// before any content changes.
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Returns the inclusive start byte offset.
    pub const fn start(self) -> usize {
        self.start
    }

    /// Returns the exclusive end byte offset.
    pub const fn end(self) -> usize {
        self.end
    }

    /// Returns the number of source bytes covered by this range.
    const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

/// A directional source selection measured in UTF-8 byte offsets.
///
/// `anchor` remains fixed while `active` is the moving caret. Keeping both
/// positions preserves selection direction across undo and redo.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct Selection {
    anchor: usize,
    active: usize,
}

impl Selection {
    /// Creates a directional selection.
    pub const fn new(anchor: usize, active: usize) -> Self {
        Self { anchor, active }
    }

    /// Creates an empty selection at one caret position.
    pub const fn caret(position: usize) -> Self {
        Self::new(position, position)
    }

    /// Returns the fixed selection endpoint.
    pub const fn anchor(self) -> usize {
        self.anchor
    }

    /// Returns the active caret endpoint.
    pub const fn active(self) -> usize {
        self.active
    }

    /// Returns the ordered source range covered by the selection.
    pub fn ordered_range(self) -> TextRange {
        TextRange::new(self.anchor.min(self.active), self.anchor.max(self.active))
    }
}

/// Monotonic observation time supplied by the editor adapter.
///
/// The transaction layer stores but does not read a system clock. This keeps
/// future coalescing policy deterministic and straightforward to test.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct EditTimestamp(Duration);

impl EditTimestamp {
    /// Creates a timestamp from an elapsed monotonic duration.
    pub const fn new(elapsed: Duration) -> Self {
        Self(elapsed)
    }

    /// Returns the stored elapsed duration.
    pub const fn elapsed(self) -> Duration {
        self.0
    }
}

/// The user-visible operation family that produced an edit.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EditOrigin {
    /// Direct input through the plain-text editor adapter.
    TextInput,
    /// Direct input through the formatted Markdown editor adapter.
    MarkdownInput,
    /// An explicit Markdown formatting command.
    MarkdownFormatting,
    /// An explicit paste command.
    Paste,
    /// An explicit replacement command.
    Replace,
    /// An explicit line-ending conversion command.
    LineEndingConversion,
    /// A trusted internal content replacement outside an interactive editor.
    Programmatic,
}

/// The deterministic user intent represented by an edit transaction.
///
/// Direct editor adapters provide an operation family through [`EditOrigin`].
/// The transaction then classifies the exact source delta and directional
/// selections. Keeping this intent on the transaction lets Undo use one shared,
/// testable coalescing policy without reading platform input state or a clock.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EditIntent {
    /// Insert source at a collapsed caret.
    Insert,
    /// Delete source immediately before a collapsed caret.
    Backspace,
    /// Delete source immediately after a collapsed caret.
    Delete,
    /// Replace or delete an explicit source selection.
    ReplaceSelection,
    /// Paste clipboard content.
    Paste,
    /// Apply a Markdown formatting command.
    Formatting,
    /// Apply an explicit search replacement.
    Replace,
    /// Convert line endings explicitly.
    LineEndingConversion,
    /// Replace content through trusted application logic.
    Programmatic,
    /// A source delta that cannot be classified conservatively.
    Unclassified,
}

/// One source replacement within an [`EditTransaction`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TextEdit {
    range: TextRange,
    inserted: String,
    removed: String,
}

impl TextEdit {
    /// Describes replacing `range` with `inserted`, provided the range contains
    /// exactly `removed` when the transaction is applied.
    pub fn replace(
        range: TextRange,
        inserted: impl Into<String>,
        removed: impl Into<String>,
    ) -> Self {
        Self {
            range,
            inserted: inserted.into(),
            removed: removed.into(),
        }
    }

    /// Returns the source range replaced by this edit.
    pub const fn range(&self) -> TextRange {
        self.range
    }

    /// Returns the inserted UTF-8 source.
    pub fn inserted(&self) -> &str {
        &self.inserted
    }

    /// Returns the exact source expected to be removed.
    pub fn removed(&self) -> &str {
        &self.removed
    }

    const fn retained_bytes(&self) -> usize {
        self.inserted.len().saturating_add(self.removed.len())
    }
}

/// A complete, revision-tagged user edit with before and after selections.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EditTransaction {
    base_revision: Revision,
    edits: Vec<TextEdit>,
    selection_before: Selection,
    selection_after: Selection,
    origin: EditOrigin,
    intent: EditIntent,
    observed_at: EditTimestamp,
}

impl EditTransaction {
    /// Creates a proposed transaction.
    ///
    /// Content-dependent validation intentionally happens atomically at apply
    /// time so a stale or malformed proposal cannot partially mutate a document.
    pub fn new(
        base_revision: Revision,
        edits: Vec<TextEdit>,
        selection_before: Selection,
        selection_after: Selection,
        origin: EditOrigin,
        observed_at: EditTimestamp,
    ) -> Self {
        let intent = classify_edit_intent(&edits, selection_before, selection_after, origin);
        Self::new_with_intent(
            base_revision,
            edits,
            selection_before,
            selection_after,
            origin,
            intent,
            observed_at,
        )
    }

    pub(crate) const fn new_with_intent(
        base_revision: Revision,
        edits: Vec<TextEdit>,
        selection_before: Selection,
        selection_after: Selection,
        origin: EditOrigin,
        intent: EditIntent,
        observed_at: EditTimestamp,
    ) -> Self {
        Self {
            base_revision,
            edits,
            selection_before,
            selection_after,
            origin,
            intent,
            observed_at,
        }
    }

    /// Computes the single minimal replacement between two UTF-8 strings.
    ///
    /// Equal content returns `None`; a selection-only movement is not a content
    /// transaction and does not advance the document revision.
    ///
    /// # Errors
    ///
    /// Returns an invalid-selection error if either selection is outside its
    /// corresponding string or is not on a UTF-8 boundary.
    pub fn between(
        base_revision: Revision,
        before: &str,
        after: &str,
        selection_before: Selection,
        selection_after: Selection,
        origin: EditOrigin,
        observed_at: EditTimestamp,
    ) -> Result<Option<Self>, EditError> {
        validate_selection_str(selection_before, before, SelectionState::Before)?;
        validate_selection_str(selection_after, after, SelectionState::After)?;
        if before == after {
            return Ok(None);
        }

        let prefix_bytes = before
            .chars()
            .zip(after.chars())
            .take_while(|(left, right)| left == right)
            .map(|(character, _)| character.len_utf8())
            .sum::<usize>();
        let mut suffix_bytes = 0;
        for (left, right) in before[prefix_bytes..]
            .chars()
            .rev()
            .zip(after[prefix_bytes..].chars().rev())
        {
            if left != right {
                break;
            }
            suffix_bytes += left.len_utf8();
        }

        let before_end = before.len() - suffix_bytes;
        let after_end = after.len() - suffix_bytes;
        let edit = TextEdit::replace(
            TextRange::new(prefix_bytes, before_end),
            &after[prefix_bytes..after_end],
            &before[prefix_bytes..before_end],
        );
        Ok(Some(Self::new(
            base_revision,
            vec![edit],
            selection_before,
            selection_after,
            origin,
            observed_at,
        )))
    }

    /// Returns the revision this transaction must observe before applying.
    pub const fn base_revision(&self) -> Revision {
        self.base_revision
    }

    /// Returns the directional selection before applying.
    pub const fn selection_before(&self) -> Selection {
        self.selection_before
    }

    /// Returns the directional selection after applying.
    pub const fn selection_after(&self) -> Selection {
        self.selection_after
    }

    /// Returns the operation family that produced this transaction.
    pub const fn origin(&self) -> EditOrigin {
        self.origin
    }

    /// Returns the deterministic user intent represented by this transaction.
    pub const fn intent(&self) -> EditIntent {
        self.intent
    }

    /// Returns the ordered source replacements in base-source coordinates.
    pub fn edits(&self) -> &[TextEdit] {
        &self.edits
    }

    /// Returns when the editor adapter observed this transaction.
    pub const fn observed_at(&self) -> EditTimestamp {
        self.observed_at
    }

    /// Returns the source bytes retained for exact forward and inverse edits.
    pub fn retained_bytes(&self) -> usize {
        self.edits.iter().fold(0usize, |total, edit| {
            total.saturating_add(edit.retained_bytes())
        })
    }

    pub(crate) fn rebased(&self, base_revision: Revision) -> Self {
        let mut rebased = self.clone();
        rebased.base_revision = base_revision;
        rebased
    }

    pub(crate) fn apply_to(
        &self,
        rope: &Rope,
        actual_revision: Revision,
        maximum_result_bytes: usize,
    ) -> Result<(Rope, AppliedTransaction), EditError> {
        if self.base_revision != actual_revision {
            return Err(EditError::StaleRevision {
                expected: self.base_revision,
                actual: actual_revision,
            });
        }
        if self.edits.is_empty() {
            return Err(EditError::EmptyTransaction);
        }
        validate_selection_rope(self.selection_before, rope, SelectionState::Before)?;
        let next_revision = actual_revision
            .checked_next()
            .ok_or(EditError::RevisionExhausted)?;

        let source_len = rope.len_bytes();
        let mut previous_end = None;
        let mut result_len = source_len;

        for (index, edit) in self.edits.iter().enumerate() {
            validate_edit(edit, rope, source_len, previous_end, index)?;
            previous_end = Some(edit.range.end);
            result_len = result_len
                .checked_sub(edit.range.len())
                .and_then(|length| length.checked_add(edit.inserted.len()))
                .ok_or(EditError::ResultTooLarge {
                    projected: usize::MAX,
                    maximum: maximum_result_bytes,
                })?;
        }
        if result_len > maximum_result_bytes {
            return Err(EditError::ResultTooLarge {
                projected: result_len,
                maximum: maximum_result_bytes,
            });
        }

        let mut inverse_edits = Vec::new();
        inverse_edits
            .try_reserve_exact(self.edits.len())
            .map_err(|_| EditError::AllocationUnavailable {
                requested_edits: self.edits.len(),
            })?;
        let mut original_cursor = 0usize;
        let mut result_cursor = 0usize;
        for edit in &self.edits {
            let unchanged_bytes = edit.range.start - original_cursor;
            result_cursor =
                result_cursor
                    .checked_add(unchanged_bytes)
                    .ok_or(EditError::ResultTooLarge {
                        projected: usize::MAX,
                        maximum: maximum_result_bytes,
                    })?;
            let inverse_end = result_cursor.checked_add(edit.inserted.len()).ok_or(
                EditError::ResultTooLarge {
                    projected: usize::MAX,
                    maximum: maximum_result_bytes,
                },
            )?;
            inverse_edits.push(TextEdit::replace(
                TextRange::new(result_cursor, inverse_end),
                edit.removed.clone(),
                edit.inserted.clone(),
            ));
            result_cursor = inverse_end;
            original_cursor = edit.range.end;
        }

        let mut result = rope.clone();
        for edit in self.edits.iter().rev() {
            let start_char = exact_byte_to_char(&result, edit.range.start)?;
            let end_char = exact_byte_to_char(&result, edit.range.end)?;
            result.remove(start_char..end_char);
            result.insert(start_char, &edit.inserted);
        }
        debug_assert_eq!(result.len_bytes(), result_len);
        validate_selection_rope(self.selection_after, &result, SelectionState::After)?;

        let inverse = Self::new_with_intent(
            next_revision,
            inverse_edits,
            self.selection_after,
            self.selection_before,
            self.origin,
            self.intent,
            self.observed_at,
        );
        Ok((
            result,
            AppliedTransaction {
                base_revision: actual_revision,
                revision: next_revision,
                selection: self.selection_after,
                inverse,
            },
        ))
    }
}

fn classify_edit_intent(
    edits: &[TextEdit],
    selection_before: Selection,
    selection_after: Selection,
    origin: EditOrigin,
) -> EditIntent {
    match origin {
        EditOrigin::Paste => return EditIntent::Paste,
        EditOrigin::MarkdownFormatting => return EditIntent::Formatting,
        EditOrigin::Replace => return EditIntent::Replace,
        EditOrigin::LineEndingConversion => return EditIntent::LineEndingConversion,
        EditOrigin::Programmatic => return EditIntent::Programmatic,
        EditOrigin::TextInput | EditOrigin::MarkdownInput => {}
    }

    let [edit] = edits else {
        return EditIntent::Unclassified;
    };
    if selection_before.anchor != selection_before.active {
        return EditIntent::ReplaceSelection;
    }
    if selection_after.anchor != selection_after.active {
        return EditIntent::Unclassified;
    }

    let before = selection_before.active;
    let after = selection_after.active;
    let range = edit.range();
    if range.start == range.end
        && edit.removed.is_empty()
        && !edit.inserted.is_empty()
        && before == range.start
        && after == range.start.saturating_add(edit.inserted.len())
    {
        return EditIntent::Insert;
    }
    if edit.inserted.is_empty() && !edit.removed.is_empty() {
        if before == range.end && after == range.start {
            return EditIntent::Backspace;
        }
        if before == range.start && after == range.start {
            return EditIntent::Delete;
        }
    }
    EditIntent::Unclassified
}

/// Successful transaction evidence, including its exact inverse.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct AppliedTransaction {
    base_revision: Revision,
    revision: Revision,
    selection: Selection,
    inverse: EditTransaction,
}

impl AppliedTransaction {
    /// Returns the revision observed before the successful mutation.
    pub const fn base_revision(&self) -> Revision {
        self.base_revision
    }

    /// Returns the revision produced by the successful mutation.
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// Returns the selection produced by the successful mutation.
    pub const fn selection(&self) -> Selection {
        self.selection
    }

    /// Returns the exact transaction that reverses this mutation.
    pub const fn inverse(&self) -> &EditTransaction {
        &self.inverse
    }

    /// Consumes the evidence and returns its exact inverse transaction.
    pub fn into_inverse(self) -> EditTransaction {
        self.inverse
    }
}

/// Whether a selection belongs to source before or after a transaction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectionState {
    /// Selection in the transaction's base source.
    Before,
    /// Selection in the transaction's resulting source.
    After,
}

/// Reasons a proposed edit transaction can be rejected without mutation.
#[derive(Clone, PartialEq, Eq, Error, Debug)]
pub enum EditError {
    /// The document advanced after the transaction was created.
    #[error("stale edit revision: expected {expected:?}, current {actual:?}")]
    StaleRevision {
        /// Revision captured by the transaction.
        expected: Revision,
        /// Current authoritative document revision.
        actual: Revision,
    },
    /// The monotonic revision counter cannot advance safely.
    #[error("the document revision counter is exhausted")]
    RevisionExhausted,
    /// A content transaction contains no replacements.
    #[error("an edit transaction must contain at least one replacement")]
    EmptyTransaction,
    /// A replacement range is reversed or outside the current source.
    #[error("invalid edit range {start}..{end} for {source_len} source bytes")]
    InvalidRange {
        /// Proposed start byte.
        start: usize,
        /// Proposed end byte.
        end: usize,
        /// Current source length in bytes.
        source_len: usize,
    },
    /// A byte offset splits a UTF-8 scalar value.
    #[error("byte offset {offset} is not a UTF-8 boundary")]
    InvalidBoundary {
        /// Invalid byte offset.
        offset: usize,
    },
    /// Replacements are not ordered and disjoint in base-source coordinates.
    #[error("edit {index} starts at {start}, before the previous edit ends at {previous_end}")]
    EditsOverlapOrOutOfOrder {
        /// Zero-based edit position.
        index: usize,
        /// Proposed start byte.
        start: usize,
        /// Previous exclusive end byte.
        previous_end: usize,
    },
    /// A replacement would not change the matched source.
    #[error("edit {index} is a no-op")]
    NoOpEdit {
        /// Zero-based edit position.
        index: usize,
    },
    /// The exact expected removed source no longer matches.
    #[error("source differs from the text captured for edit {index}")]
    RemovedTextMismatch {
        /// Zero-based edit position.
        index: usize,
    },
    /// A selection endpoint is outside its corresponding source.
    #[error("{state:?} selection endpoint {position} exceeds the {source_len}-byte source")]
    InvalidSelection {
        /// Whether this is the before or after selection.
        state: SelectionState,
        /// Invalid byte offset.
        position: usize,
        /// Corresponding source length in bytes.
        source_len: usize,
    },
    /// The resulting source exceeds the caller's authoritative byte ceiling.
    #[error("the edit would create {projected} bytes; the maximum is {maximum} bytes")]
    ResultTooLarge {
        /// Projected result length, or `usize::MAX` after arithmetic overflow.
        projected: usize,
        /// Maximum accepted source length.
        maximum: usize,
    },
    /// The inverse-edit index could not reserve its bounded allocation.
    #[error("the edit could not reserve space for {requested_edits} inverse edits")]
    AllocationUnavailable {
        /// Exact number of inverse edits requested.
        requested_edits: usize,
    },
}

fn validate_edit(
    edit: &TextEdit,
    rope: &Rope,
    source_len: usize,
    previous_end: Option<usize>,
    index: usize,
) -> Result<(), EditError> {
    if edit.range.start > edit.range.end || edit.range.end > source_len {
        return Err(EditError::InvalidRange {
            start: edit.range.start,
            end: edit.range.end,
            source_len,
        });
    }
    if let Some(previous_end) = previous_end
        && edit.range.start < previous_end
    {
        return Err(EditError::EditsOverlapOrOutOfOrder {
            index,
            start: edit.range.start,
            previous_end,
        });
    }
    let start_char = exact_byte_to_char(rope, edit.range.start)?;
    let end_char = exact_byte_to_char(rope, edit.range.end)?;
    if edit.inserted == edit.removed {
        return Err(EditError::NoOpEdit { index });
    }
    if rope.slice(start_char..end_char) != edit.removed.as_str() {
        return Err(EditError::RemovedTextMismatch { index });
    }
    Ok(())
}

fn validate_selection_str(
    selection: Selection,
    source: &str,
    state: SelectionState,
) -> Result<(), EditError> {
    for position in [selection.anchor, selection.active] {
        if position > source.len() {
            return Err(EditError::InvalidSelection {
                state,
                position,
                source_len: source.len(),
            });
        }
        if !source.is_char_boundary(position) {
            return Err(EditError::InvalidBoundary { offset: position });
        }
    }
    Ok(())
}

fn validate_selection_rope(
    selection: Selection,
    source: &Rope,
    state: SelectionState,
) -> Result<(), EditError> {
    for position in [selection.anchor, selection.active] {
        if position > source.len_bytes() {
            return Err(EditError::InvalidSelection {
                state,
                position,
                source_len: source.len_bytes(),
            });
        }
        exact_byte_to_char(source, position)?;
    }
    Ok(())
}

fn exact_byte_to_char(source: &Rope, position: usize) -> Result<usize, EditError> {
    let character = source
        .try_byte_to_char(position)
        .map_err(|_| EditError::InvalidBoundary { offset: position })?;
    if source.char_to_byte(character) != position {
        return Err(EditError::InvalidBoundary { offset: position });
    }
    Ok(character)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transaction(
        revision: Revision,
        edits: Vec<TextEdit>,
        before: Selection,
        after: Selection,
    ) -> EditTransaction {
        EditTransaction::new(
            revision,
            edits,
            before,
            after,
            EditOrigin::TextInput,
            EditTimestamp::default(),
        )
    }

    #[test]
    fn value_types_preserve_byte_ranges_and_selection_direction() {
        let range = TextRange::new(2, 7);
        assert_eq!(range.start(), 2);
        assert_eq!(range.end(), 7);
        assert_eq!(range.len(), 5);

        let selection = Selection::new(9, 3);
        assert_eq!(selection.anchor(), 9);
        assert_eq!(selection.active(), 3);
        assert_eq!(selection.ordered_range(), TextRange::new(3, 9));
        assert_eq!(Selection::caret(4), Selection::new(4, 4));
    }

    #[test]
    fn direct_source_deltas_have_conservative_explicit_intents() {
        let cases = [
            (
                TextEdit::replace(TextRange::new(1, 1), "é", ""),
                Selection::caret(1),
                Selection::caret(3),
                EditOrigin::TextInput,
                EditIntent::Insert,
            ),
            (
                TextEdit::replace(TextRange::new(1, 3), "", "é"),
                Selection::caret(3),
                Selection::caret(1),
                EditOrigin::TextInput,
                EditIntent::Backspace,
            ),
            (
                TextEdit::replace(TextRange::new(1, 3), "", "é"),
                Selection::caret(1),
                Selection::caret(1),
                EditOrigin::MarkdownInput,
                EditIntent::Delete,
            ),
            (
                TextEdit::replace(TextRange::new(1, 3), "x", "é"),
                Selection::new(3, 1),
                Selection::caret(2),
                EditOrigin::TextInput,
                EditIntent::ReplaceSelection,
            ),
            (
                TextEdit::replace(TextRange::new(1, 1), "x", ""),
                Selection::caret(1),
                Selection::caret(2),
                EditOrigin::Paste,
                EditIntent::Paste,
            ),
        ];

        for (edit, before, after, origin, expected) in cases {
            let transaction = EditTransaction::new(
                Revision::INITIAL,
                vec![edit],
                before,
                after,
                origin,
                EditTimestamp::default(),
            );
            assert_eq!(transaction.intent(), expected);
            assert_eq!(transaction.edits().len(), 1);
        }
    }

    #[test]
    fn direct_intent_classification_rejects_each_near_miss_independently() {
        let cases = [
            (
                TextEdit::replace(TextRange::new(1, 2), "x", ""),
                Selection::caret(1),
                Selection::caret(2),
            ),
            (
                TextEdit::replace(TextRange::new(1, 1), "x", "y"),
                Selection::caret(1),
                Selection::caret(1),
            ),
            (
                TextEdit::replace(TextRange::new(1, 2), "", "x"),
                Selection::caret(0),
                Selection::caret(1),
            ),
        ];

        for (edit, before, after) in cases {
            let transaction = EditTransaction::new(
                Revision::INITIAL,
                vec![edit],
                before,
                after,
                EditOrigin::TextInput,
                EditTimestamp::default(),
            );
            assert_eq!(transaction.intent(), EditIntent::Unclassified);
        }
    }

    #[test]
    fn explicit_operation_origins_define_intent_without_shape_guessing() {
        for (origin, expected) in [
            (EditOrigin::MarkdownFormatting, EditIntent::Formatting),
            (EditOrigin::Replace, EditIntent::Replace),
            (
                EditOrigin::LineEndingConversion,
                EditIntent::LineEndingConversion,
            ),
            (EditOrigin::Programmatic, EditIntent::Programmatic),
        ] {
            let transaction = EditTransaction::new(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(0, 0), "x", "")],
                Selection::caret(0),
                Selection::caret(1),
                origin,
                EditTimestamp::default(),
            );
            assert_eq!(transaction.intent(), expected);
        }
    }

    #[test]
    fn inverse_retains_the_forward_intent() {
        let source = Rope::from_str("");
        let transaction = EditTransaction::new(
            Revision::INITIAL,
            vec![TextEdit::replace(TextRange::new(0, 0), "x", "")],
            Selection::caret(0),
            Selection::caret(1),
            EditOrigin::TextInput,
            EditTimestamp::default(),
        );

        let (_, applied) = transaction
            .apply_to(&source, Revision::INITIAL, usize::MAX)
            .expect("insertion should apply");

        assert_eq!(transaction.intent(), EditIntent::Insert);
        assert_eq!(applied.inverse().intent(), EditIntent::Insert);
    }

    #[test]
    fn minimal_difference_keeps_shared_unicode_prefix_and_suffix() {
        let edit = EditTransaction::between(
            Revision::INITIAL,
            "a\u{301} middle \u{1f642}",
            "a\u{301} better \u{1f642}",
            Selection::caret(3),
            Selection::caret(3),
            EditOrigin::TextInput,
            EditTimestamp::new(Duration::from_millis(12)),
        )
        .expect("valid selections should produce a transaction")
        .expect("different text should produce one edit");

        assert_eq!(edit.edits.len(), 1);
        assert_eq!(edit.edits[0].removed(), "middle");
        assert_eq!(edit.edits[0].inserted(), "better");
        assert_eq!(edit.observed_at().elapsed(), Duration::from_millis(12));
    }

    #[test]
    fn equal_content_is_not_a_transaction() {
        let transaction = EditTransaction::between(
            Revision::INITIAL,
            "same",
            "same",
            Selection::caret(0),
            Selection::caret(4),
            EditOrigin::TextInput,
            EditTimestamp::default(),
        )
        .expect("both selections are valid");

        assert!(transaction.is_none());
    }

    #[test]
    fn between_rejects_invalid_selections_even_when_content_is_equal() {
        assert_eq!(
            EditTransaction::between(
                Revision::INITIAL,
                "é",
                "é",
                Selection::caret(1),
                Selection::caret(2),
                EditOrigin::TextInput,
                EditTimestamp::default(),
            ),
            Err(EditError::InvalidBoundary { offset: 1 })
        );
        assert_eq!(
            EditTransaction::between(
                Revision::INITIAL,
                "text",
                "text",
                Selection::caret(0),
                Selection::caret(5),
                EditOrigin::TextInput,
                EditTimestamp::default(),
            ),
            Err(EditError::InvalidSelection {
                state: SelectionState::After,
                position: 5,
                source_len: 4,
            })
        );
    }

    #[test]
    fn apply_is_atomic_and_inverse_restores_content_and_directional_selection() {
        let source = Rope::from_str("alpha beta gamma");
        let edit = transaction(
            Revision::INITIAL,
            vec![TextEdit::replace(TextRange::new(6, 10), "BETA", "beta")],
            Selection::new(10, 6),
            Selection::new(6, 10),
        );

        let (changed, applied) = edit
            .apply_to(&source, Revision::INITIAL, usize::MAX)
            .expect("valid edit should apply");
        assert_eq!(changed.to_string(), "alpha BETA gamma");
        assert_eq!(applied.selection(), Selection::new(6, 10));

        let (restored, undone) = applied
            .inverse()
            .apply_to(&changed, applied.revision(), usize::MAX)
            .expect("exact inverse should apply");
        assert_eq!(restored, source);
        assert_eq!(undone.selection(), Selection::new(10, 6));
    }

    #[test]
    fn multiple_edits_produce_correct_shifted_inverse_ranges() {
        let source = Rope::from_str("one two three");
        let edit = transaction(
            Revision::INITIAL,
            vec![
                TextEdit::replace(TextRange::new(0, 3), "1", "one"),
                TextEdit::replace(TextRange::new(8, 13), "33333", "three"),
            ],
            Selection::caret(0),
            Selection::caret(7),
        );

        let (changed, applied) = edit
            .apply_to(&source, Revision::INITIAL, usize::MAX)
            .expect("disjoint edits should apply atomically");
        assert_eq!(changed.to_string(), "1 two 33333");
        assert_eq!(applied.inverse.edits[0].range, TextRange::new(0, 1));
        assert_eq!(applied.inverse.edits[1].range, TextRange::new(6, 11));

        let (restored, _) = applied
            .inverse
            .apply_to(&changed, applied.revision, usize::MAX)
            .expect("shifted inverse ranges should restore the source");
        assert_eq!(restored, source);
    }

    #[test]
    fn every_validation_failure_preserves_the_input_rope() {
        let cases = [
            transaction(
                Revision::new(1),
                vec![TextEdit::replace(TextRange::new(0, 1), "x", "a")],
                Selection::caret(0),
                Selection::caret(1),
            ),
            transaction(
                Revision::INITIAL,
                Vec::new(),
                Selection::caret(0),
                Selection::caret(0),
            ),
            transaction(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(2, 1), "x", "")],
                Selection::caret(0),
                Selection::caret(0),
            ),
            transaction(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(1, 2), "x", "")],
                Selection::caret(0),
                Selection::caret(0),
            ),
            transaction(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(0, 1), "a", "a")],
                Selection::caret(0),
                Selection::caret(1),
            ),
            transaction(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(0, 1), "x", "z")],
                Selection::caret(0),
                Selection::caret(1),
            ),
            transaction(
                Revision::INITIAL,
                vec![
                    TextEdit::replace(TextRange::new(1, 2), "x", "b"),
                    TextEdit::replace(TextRange::new(0, 1), "y", "a"),
                ],
                Selection::caret(0),
                Selection::caret(0),
            ),
            transaction(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(0, 1), "x", "a")],
                Selection::caret(4),
                Selection::caret(1),
            ),
        ];

        for candidate in cases {
            let source = Rope::from_str("abc");
            assert!(
                candidate
                    .apply_to(&source, Revision::INITIAL, usize::MAX)
                    .is_err()
            );
            assert_eq!(source.to_string(), "abc");
        }
    }

    #[test]
    fn invalid_unicode_boundaries_are_rejected() {
        let source = Rope::from_str("é");
        let edit = transaction(
            Revision::INITIAL,
            vec![TextEdit::replace(TextRange::new(1, 2), "e", "")],
            Selection::caret(0),
            Selection::caret(1),
        );

        assert_eq!(
            edit.apply_to(&source, Revision::INITIAL, usize::MAX),
            Err(EditError::InvalidBoundary { offset: 1 })
        );
        assert_eq!(source.to_string(), "é");
    }

    #[test]
    fn result_ceiling_is_exact_for_typing_paste_and_replace_origins() {
        let source = Rope::from_str("abc");
        for origin in [
            EditOrigin::TextInput,
            EditOrigin::Paste,
            EditOrigin::Replace,
        ] {
            let edit = EditTransaction::new(
                Revision::INITIAL,
                vec![TextEdit::replace(TextRange::new(3, 3), "d", "")],
                Selection::caret(3),
                Selection::caret(4),
                origin,
                EditTimestamp::default(),
            );

            assert!(
                edit.apply_to(&source, Revision::INITIAL, 4).is_ok(),
                "{origin:?} should reach the exact result ceiling"
            );
            assert_eq!(
                edit.apply_to(&source, Revision::INITIAL, 3),
                Err(EditError::ResultTooLarge {
                    projected: 4,
                    maximum: 3,
                }),
                "{origin:?} must not exceed the result ceiling"
            );
            assert_eq!(source.to_string(), "abc");
        }
    }
}
