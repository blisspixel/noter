//! Pure destructive-action lifecycle state machine.

use super::revision::Revision;

/// An action that can replace the current document or close its window.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DestructiveIntent {
    /// Replace the current document with a new untitled document.
    New,
    /// Choose and open another document.
    Open,
    /// Reload the current path from disk.
    Reload,
    /// Close the current application window.
    Quit,
}

/// The user's explicit response to an unsaved-changes decision.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum DirtyDecision {
    /// Save before continuing the pending intent.
    Save,
    /// Continue the pending intent without saving current changes.
    Discard,
    /// Return to editing without continuing the pending intent.
    Cancel,
}

/// Facts observed after the save initiated by an explicit dirty decision.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SaveContinuation {
    revision: Revision,
    document_dirty: bool,
    save_interaction_pending: bool,
    blocking_follow_up: bool,
}

impl SaveContinuation {
    /// Creates a complete post-save observation for the current document revision.
    pub const fn new(
        revision: Revision,
        document_dirty: bool,
        save_interaction_pending: bool,
        blocking_follow_up: bool,
    ) -> Self {
        Self {
            revision,
            document_dirty,
            save_interaction_pending,
            blocking_follow_up,
        }
    }
}

/// One input to the lifecycle reducer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LifecycleCommand {
    /// Request a destructive intent against the current document state.
    Request {
        /// Requested action.
        intent: DestructiveIntent,
        /// Whether current authoritative content differs from saved content.
        document_dirty: bool,
        /// Exact content revision against which the request was made.
        revision: Revision,
    },
    /// Resolve the currently visible unsaved-changes decision.
    Decide(DirtyDecision),
    /// Re-evaluate the save started by the retained dirty decision.
    SaveSettled(SaveContinuation),
}

/// The single effect emitted by a lifecycle transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LifecycleEffect {
    /// No adapter action is required.
    None,
    /// Show the unsaved-changes decision for the retained intent.
    PromptDirty(DestructiveIntent),
    /// Start the normal Save command while retaining the pending intent.
    StartSave,
    /// Continue an authorized destructive intent.
    Continue(DestructiveIntent),
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum LifecyclePhase {
    #[default]
    Idle,
    Prompting {
        intent: DestructiveIntent,
        revision: Revision,
    },
    Saving {
        intent: DestructiveIntent,
        revision: Revision,
    },
    Closing {
        revision: Revision,
    },
}

/// Pure state for destructive document and window actions.
///
/// Save completion can authorize continuation only while a matching explicit
/// Save decision is active. Native close authorization is bound to the exact
/// revision that was approved, so later edits cannot inherit it.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct LifecycleState {
    phase: LifecyclePhase,
}

impl LifecycleState {
    /// Returns the intent currently waiting for Save, Discard, or Cancel.
    pub const fn pending_intent(self) -> Option<DestructiveIntent> {
        match self.phase {
            LifecyclePhase::Prompting { intent, .. } | LifecyclePhase::Saving { intent, .. } => {
                Some(intent)
            }
            LifecyclePhase::Idle | LifecyclePhase::Closing { .. } => None,
        }
    }

    /// Returns whether a native Quit may pass for this exact document revision.
    pub fn close_authorized(self, revision: Revision) -> bool {
        matches!(self.phase, LifecyclePhase::Closing { revision: approved } if approved == revision)
    }

    /// Applies one command and returns its adapter effect.
    pub fn reduce(&mut self, command: LifecycleCommand) -> LifecycleEffect {
        match command {
            LifecycleCommand::Request {
                intent,
                document_dirty,
                revision,
            } => self.request(intent, document_dirty, revision),
            LifecycleCommand::Decide(decision) => self.decide(decision),
            LifecycleCommand::SaveSettled(observation) => self.save_settled(observation),
        }
    }

    fn request(
        &mut self,
        intent: DestructiveIntent,
        document_dirty: bool,
        revision: Revision,
    ) -> LifecycleEffect {
        match self.phase {
            LifecyclePhase::Prompting { .. } | LifecyclePhase::Saving { .. } => {
                return LifecycleEffect::None;
            }
            LifecyclePhase::Closing { revision: approved }
                if intent == DestructiveIntent::Quit && approved == revision && !document_dirty =>
            {
                return LifecycleEffect::Continue(intent);
            }
            LifecyclePhase::Idle | LifecyclePhase::Closing { .. } => {
                self.phase = LifecyclePhase::Idle;
            }
        }

        if document_dirty {
            self.phase = LifecyclePhase::Prompting { intent, revision };
            LifecycleEffect::PromptDirty(intent)
        } else {
            self.finish(intent, revision)
        }
    }

    fn decide(&mut self, decision: DirtyDecision) -> LifecycleEffect {
        let LifecyclePhase::Prompting { intent, revision } = self.phase else {
            return LifecycleEffect::None;
        };
        match decision {
            DirtyDecision::Save => {
                self.phase = LifecyclePhase::Saving { intent, revision };
                LifecycleEffect::StartSave
            }
            DirtyDecision::Discard => self.finish(intent, revision),
            DirtyDecision::Cancel => {
                self.phase = LifecyclePhase::Idle;
                LifecycleEffect::None
            }
        }
    }

    fn save_settled(&mut self, observation: SaveContinuation) -> LifecycleEffect {
        let LifecyclePhase::Saving {
            intent,
            revision: expected_revision,
        } = self.phase
        else {
            return LifecycleEffect::None;
        };

        if observation.revision != expected_revision {
            self.phase = LifecyclePhase::Idle;
            return LifecycleEffect::None;
        }
        if observation.save_interaction_pending {
            return LifecycleEffect::None;
        }
        if observation.blocking_follow_up {
            self.phase = LifecyclePhase::Idle;
            return LifecycleEffect::None;
        }
        if observation.document_dirty {
            self.phase = LifecyclePhase::Prompting {
                intent,
                revision: observation.revision,
            };
            return LifecycleEffect::PromptDirty(intent);
        }
        self.finish(intent, observation.revision)
    }

    fn finish(&mut self, intent: DestructiveIntent, revision: Revision) -> LifecycleEffect {
        self.phase = if intent == DestructiveIntent::Quit {
            LifecyclePhase::Closing { revision }
        } else {
            LifecyclePhase::Idle
        };
        LifecycleEffect::Continue(intent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INITIAL: Revision = Revision::INITIAL;

    const fn request(dirty: bool, intent: DestructiveIntent) -> LifecycleCommand {
        LifecycleCommand::Request {
            intent,
            document_dirty: dirty,
            revision: INITIAL,
        }
    }

    fn begin_save(state: &mut LifecycleState, intent: DestructiveIntent) {
        assert_eq!(
            state.reduce(request(true, intent)),
            LifecycleEffect::PromptDirty(intent)
        );
        assert_eq!(
            state.reduce(LifecycleCommand::Decide(DirtyDecision::Save)),
            LifecycleEffect::StartSave
        );
    }

    #[test]
    fn every_dirty_intent_requires_the_same_explicit_decision() {
        for intent in [
            DestructiveIntent::New,
            DestructiveIntent::Open,
            DestructiveIntent::Reload,
            DestructiveIntent::Quit,
        ] {
            let mut state = LifecycleState::default();
            assert_eq!(
                state.reduce(request(true, intent)),
                LifecycleEffect::PromptDirty(intent)
            );
            assert_eq!(state.pending_intent(), Some(intent));
            assert!(!state.close_authorized(INITIAL));
        }
    }

    #[test]
    fn repeated_requests_cannot_replace_a_visible_or_saving_decision() {
        let mut state = LifecycleState::default();
        state.reduce(request(true, DestructiveIntent::New));

        for replacement in [
            DestructiveIntent::Open,
            DestructiveIntent::Reload,
            DestructiveIntent::Quit,
        ] {
            assert_eq!(
                state.reduce(request(true, replacement)),
                LifecycleEffect::None
            );
            assert_eq!(state.pending_intent(), Some(DestructiveIntent::New));
        }
        state.reduce(LifecycleCommand::Decide(DirtyDecision::Save));
        assert_eq!(
            state.reduce(request(false, DestructiveIntent::Quit)),
            LifecycleEffect::None
        );
        assert_eq!(state.pending_intent(), Some(DestructiveIntent::New));
    }

    #[test]
    fn cancel_discard_and_save_have_distinct_transitions() {
        let mut cancelled = LifecycleState::default();
        cancelled.reduce(request(true, DestructiveIntent::Open));
        assert_eq!(
            cancelled.reduce(LifecycleCommand::Decide(DirtyDecision::Cancel)),
            LifecycleEffect::None
        );
        assert_eq!(cancelled, LifecycleState::default());

        let mut discarded = LifecycleState::default();
        discarded.reduce(request(true, DestructiveIntent::New));
        assert_eq!(
            discarded.reduce(LifecycleCommand::Decide(DirtyDecision::Discard)),
            LifecycleEffect::Continue(DestructiveIntent::New)
        );

        let mut saving = LifecycleState::default();
        begin_save(&mut saving, DestructiveIntent::Reload);
        assert_eq!(saving.pending_intent(), Some(DestructiveIntent::Reload));
        assert_eq!(
            saving.reduce(LifecycleCommand::Decide(DirtyDecision::Discard)),
            LifecycleEffect::None
        );
    }

    #[test]
    fn save_completion_requires_an_explicit_save_decision() {
        let clean = SaveContinuation::new(INITIAL, false, false, false);

        let mut idle = LifecycleState::default();
        assert_eq!(
            idle.reduce(LifecycleCommand::SaveSettled(clean)),
            LifecycleEffect::None
        );

        let mut prompting = LifecycleState::default();
        prompting.reduce(request(true, DestructiveIntent::Open));
        assert_eq!(
            prompting.reduce(LifecycleCommand::SaveSettled(clean)),
            LifecycleEffect::None
        );
        assert_eq!(prompting.pending_intent(), Some(DestructiveIntent::Open));
    }

    #[test]
    fn save_continuation_truth_table_never_discards_dirty_or_uncertain_work() {
        for (dirty, interaction, blocking, expected, retained) in [
            (
                true,
                false,
                false,
                LifecycleEffect::PromptDirty(DestructiveIntent::Open),
                true,
            ),
            (false, true, false, LifecycleEffect::None, true),
            (true, true, true, LifecycleEffect::None, true),
            (false, false, true, LifecycleEffect::None, false),
            (
                false,
                false,
                false,
                LifecycleEffect::Continue(DestructiveIntent::Open),
                false,
            ),
        ] {
            let mut state = LifecycleState::default();
            begin_save(&mut state, DestructiveIntent::Open);
            assert_eq!(
                state.reduce(LifecycleCommand::SaveSettled(SaveContinuation::new(
                    INITIAL,
                    dirty,
                    interaction,
                    blocking,
                ))),
                expected
            );
            assert_eq!(state.pending_intent().is_some(), retained);
        }
    }

    #[test]
    fn stale_save_completion_cannot_authorize_destructive_work() {
        let mut state = LifecycleState::default();
        begin_save(&mut state, DestructiveIntent::Quit);

        assert_eq!(
            state.reduce(LifecycleCommand::SaveSettled(SaveContinuation::new(
                Revision::new(1),
                false,
                false,
                false,
            ))),
            LifecycleEffect::None
        );
        assert_eq!(state.pending_intent(), None);
        assert!(!state.close_authorized(INITIAL));
        assert!(!state.close_authorized(Revision::new(1)));
    }

    #[test]
    fn cancelled_follow_up_returns_to_an_explicit_dirty_decision() {
        let mut state = LifecycleState::default();
        begin_save(&mut state, DestructiveIntent::New);

        assert_eq!(
            state.reduce(LifecycleCommand::SaveSettled(SaveContinuation::new(
                INITIAL, true, true, false,
            ))),
            LifecycleEffect::None
        );
        assert_eq!(
            state.reduce(LifecycleCommand::SaveSettled(SaveContinuation::new(
                INITIAL, true, false, false,
            ))),
            LifecycleEffect::PromptDirty(DestructiveIntent::New)
        );
        assert_eq!(state.pending_intent(), Some(DestructiveIntent::New));
        assert!(!state.close_authorized(INITIAL));
    }

    #[test]
    fn quit_authorization_is_bound_to_one_exact_revision() {
        let mut state = LifecycleState::default();
        state.reduce(request(true, DestructiveIntent::Quit));
        assert_eq!(
            state.reduce(LifecycleCommand::Decide(DirtyDecision::Discard)),
            LifecycleEffect::Continue(DestructiveIntent::Quit)
        );
        assert!(state.close_authorized(INITIAL));
        assert!(!state.close_authorized(Revision::new(1)));
        assert_eq!(
            state.reduce(LifecycleCommand::Request {
                intent: DestructiveIntent::Quit,
                document_dirty: true,
                revision: Revision::new(1),
            }),
            LifecycleEffect::PromptDirty(DestructiveIntent::Quit)
        );
        assert!(!state.close_authorized(INITIAL));
    }

    #[test]
    fn clean_requests_continue_and_only_quit_enters_closing_phase() {
        for intent in [
            DestructiveIntent::New,
            DestructiveIntent::Open,
            DestructiveIntent::Reload,
            DestructiveIntent::Quit,
        ] {
            let mut state = LifecycleState::default();
            assert_eq!(
                state.reduce(request(false, intent)),
                LifecycleEffect::Continue(intent)
            );
            assert_eq!(
                state.close_authorized(INITIAL),
                intent == DestructiveIntent::Quit
            );
        }
    }
}
