//! Generated lifecycle-reducer model and safety-invariant checks.

use noter::core::lifecycle::{
    DestructiveIntent, DirtyDecision, LifecycleCommand, LifecycleEffect, LifecycleState,
    SaveContinuation,
};
use noter::core::revision::Revision;
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

#[derive(Clone, Copy, Debug)]
enum ModelCommand {
    Request(DestructiveIntent, bool, Revision),
    Decide(DirtyDecision),
    SaveSettled(Revision, bool, bool, bool),
}

#[derive(Clone, Copy, Default, Debug)]
enum ReferencePhase {
    #[default]
    Idle,
    Prompting(DestructiveIntent, Revision),
    Saving(DestructiveIntent, Revision),
    Closing(Revision),
}

#[derive(Clone, Copy, Default, Debug)]
struct ReferenceState {
    phase: ReferencePhase,
}

impl ReferenceState {
    fn reduce(&mut self, command: ModelCommand) -> LifecycleEffect {
        match command {
            ModelCommand::Request(intent, dirty, revision) => {
                match self.phase {
                    ReferencePhase::Prompting(_, _) | ReferencePhase::Saving(_, _) => {
                        return LifecycleEffect::None;
                    }
                    ReferencePhase::Closing(approved)
                        if intent == DestructiveIntent::Quit && approved == revision && !dirty =>
                    {
                        return LifecycleEffect::Continue(intent);
                    }
                    ReferencePhase::Idle | ReferencePhase::Closing(_) => {
                        self.phase = ReferencePhase::Idle;
                    }
                }
                if dirty {
                    self.phase = ReferencePhase::Prompting(intent, revision);
                    LifecycleEffect::PromptDirty(intent)
                } else {
                    self.finish(intent, revision)
                }
            }
            ModelCommand::Decide(decision) => {
                let ReferencePhase::Prompting(intent, revision) = self.phase else {
                    return LifecycleEffect::None;
                };
                match decision {
                    DirtyDecision::Save => {
                        self.phase = ReferencePhase::Saving(intent, revision);
                        LifecycleEffect::StartSave
                    }
                    DirtyDecision::Discard => self.finish(intent, revision),
                    DirtyDecision::Cancel => {
                        self.phase = ReferencePhase::Idle;
                        LifecycleEffect::None
                    }
                }
            }
            ModelCommand::SaveSettled(revision, dirty, interaction, blocking) => {
                let ReferencePhase::Saving(intent, expected) = self.phase else {
                    return LifecycleEffect::None;
                };
                if revision != expected {
                    self.phase = ReferencePhase::Idle;
                    LifecycleEffect::None
                } else if interaction {
                    LifecycleEffect::None
                } else if blocking {
                    self.phase = ReferencePhase::Idle;
                    LifecycleEffect::None
                } else if dirty {
                    self.phase = ReferencePhase::Prompting(intent, revision);
                    LifecycleEffect::PromptDirty(intent)
                } else {
                    self.finish(intent, revision)
                }
            }
        }
    }

    fn finish(&mut self, intent: DestructiveIntent, revision: Revision) -> LifecycleEffect {
        self.phase = if intent == DestructiveIntent::Quit {
            ReferencePhase::Closing(revision)
        } else {
            ReferencePhase::Idle
        };
        LifecycleEffect::Continue(intent)
    }

    const fn pending(self) -> Option<DestructiveIntent> {
        match self.phase {
            ReferencePhase::Prompting(intent, _) | ReferencePhase::Saving(intent, _) => {
                Some(intent)
            }
            ReferencePhase::Idle | ReferencePhase::Closing(_) => None,
        }
    }

    fn close_authorized(self, revision: Revision) -> bool {
        matches!(self.phase, ReferencePhase::Closing(approved) if approved == revision)
    }
}

fn intent() -> impl Strategy<Value = DestructiveIntent> {
    prop_oneof![
        Just(DestructiveIntent::New),
        Just(DestructiveIntent::Open),
        Just(DestructiveIntent::Reload),
        Just(DestructiveIntent::Quit),
    ]
}

fn decision() -> impl Strategy<Value = DirtyDecision> {
    prop_oneof![
        Just(DirtyDecision::Save),
        Just(DirtyDecision::Discard),
        Just(DirtyDecision::Cancel),
    ]
}

fn revision() -> impl Strategy<Value = Revision> {
    (0_u64..8).prop_map(Revision::new)
}

fn command() -> impl Strategy<Value = ModelCommand> {
    prop_oneof![
        4 => (intent(), any::<bool>(), revision())
            .prop_map(|(intent, dirty, revision)| ModelCommand::Request(intent, dirty, revision)),
        3 => decision().prop_map(ModelCommand::Decide),
        3 => (revision(), any::<bool>(), any::<bool>(), any::<bool>())
            .prop_map(|(revision, dirty, interaction, blocking)| {
                ModelCommand::SaveSettled(revision, dirty, interaction, blocking)
            }),
    ]
}

fn property_config() -> ProptestConfig {
    ProptestConfig {
        cases: 512,
        failure_persistence: None,
        rng_seed: RngSeed::Fixed(0x4E4F_5445_525F_4C43),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(property_config())]

    #[test]
    fn lifecycle_reducer_matches_the_small_reference_model(
        commands in proptest::collection::vec(command(), 0..128),
    ) {
        let mut actual = LifecycleState::default();
        let mut reference = ReferenceState::default();

        for command in commands {
            let actual_command = match command {
                ModelCommand::Request(intent, document_dirty, revision) => {
                    LifecycleCommand::Request {
                        intent,
                        document_dirty,
                        revision,
                    }
                }
                ModelCommand::Decide(decision) => LifecycleCommand::Decide(decision),
                ModelCommand::SaveSettled(revision, dirty, interaction, blocking) => {
                    LifecycleCommand::SaveSettled(SaveContinuation::new(
                        revision,
                        dirty,
                        interaction,
                        blocking,
                    ))
                }
            };

            prop_assert_eq!(actual.reduce(actual_command), reference.reduce(command));
            prop_assert_eq!(actual.pending_intent(), reference.pending());
            for candidate in 0_u64..8 {
                let revision = Revision::new(candidate);
                prop_assert_eq!(
                    actual.close_authorized(revision),
                    reference.close_authorized(revision)
                );
            }
        }
    }

    #[test]
    fn unsolicited_save_completion_never_authorizes_any_intent(
        intent in intent(),
        revision in revision(),
        dirty in any::<bool>(),
        interaction in any::<bool>(),
        blocking in any::<bool>(),
    ) {
        let mut state = LifecycleState::default();
        state.reduce(LifecycleCommand::Request {
            intent,
            document_dirty: true,
            revision,
        });

        let effect = state.reduce(LifecycleCommand::SaveSettled(SaveContinuation::new(
            revision,
            dirty,
            interaction,
            blocking,
        )));

        prop_assert_ne!(effect, LifecycleEffect::Continue(intent));
        prop_assert!(!state.close_authorized(revision));
        prop_assert_eq!(state.pending_intent(), Some(intent));
    }

    #[test]
    fn close_authorization_never_crosses_a_revision(
        approved in revision(),
        changed in revision().prop_filter("revision must change", |candidate| *candidate != Revision::INITIAL),
    ) {
        let mut state = LifecycleState::default();
        state.reduce(LifecycleCommand::Request {
            intent: DestructiveIntent::Quit,
            document_dirty: false,
            revision: approved,
        });

        prop_assert!(state.close_authorized(approved));
        if changed != approved {
            prop_assert!(!state.close_authorized(changed));
        }
    }
}
