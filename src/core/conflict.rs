//! Pure external-change classification and conflict decisions.
//!
//! I/O stays in the adapter. This module only compares a trusted baseline with
//! one already-captured observation and retains the user's explicit choice.

use super::revision::Revision;
use super::save::{FileObservation, SpecialFileKind, TargetExpectation, TargetState};

/// How the current path differs from the trusted save baseline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExternalChangeKind {
    /// The path still matches the trusted baseline, or no baseline exists.
    Unchanged,
    /// A regular file is present but identity, length, fingerprint, or change
    /// evidence no longer matches the baseline.
    ContentOrIdentityChanged,
    /// The baseline expected an existing file and the path is now absent.
    Deleted,
    /// The path is now a link, directory, or other non-regular entry.
    ReplacedBySpecial(SpecialFileKind),
    /// Inspection failed and the adapter refused to invent a state.
    Unreadable,
}

impl ExternalChangeKind {
    /// Returns whether this observation should interrupt the user.
    pub const fn requires_prompt(self) -> bool {
        !matches!(self, Self::Unchanged)
    }

    /// Returns a short, user-facing description without paths or content.
    pub const fn description(self) -> &'static str {
        match self {
            Self::Unchanged => "The file on disk still matches Noter's last trusted version.",
            Self::ContentOrIdentityChanged => {
                "The file on disk changed since Noter last loaded or saved it."
            }
            Self::Deleted => "The file on disk is missing.",
            Self::ReplacedBySpecial(SpecialFileKind::SymbolicLink) => {
                "The path is now a symbolic link or reparse point."
            }
            Self::ReplacedBySpecial(SpecialFileKind::Directory) => "The path is now a directory.",
            Self::ReplacedBySpecial(SpecialFileKind::Other) => {
                "The path is no longer an ordinary file."
            }
            Self::Unreadable => "Noter could not safely inspect the file on disk.",
        }
    }
}

/// Classifies one captured destination observation against the trusted baseline.
///
/// `None` for the baseline means the document has no save expectation (untitled
/// or never baselined), so external inspection cannot produce a conflict.
/// `None` for the observation means the adapter did not run inspection.
pub fn classify_external_change(
    expected: Option<TargetExpectation>,
    observed: Option<Result<TargetState, ()>>,
) -> ExternalChangeKind {
    let Some(expected) = expected else {
        return ExternalChangeKind::Unchanged;
    };
    let Some(observed) = observed else {
        return ExternalChangeKind::Unchanged;
    };
    match observed {
        Err(()) => ExternalChangeKind::Unreadable,
        Ok(state) if expected.matches_state(state) => ExternalChangeKind::Unchanged,
        Ok(TargetState::Missing) => ExternalChangeKind::Deleted,
        Ok(TargetState::Special(kind)) => ExternalChangeKind::ReplacedBySpecial(kind),
        Ok(TargetState::Regular(_)) => ExternalChangeKind::ContentOrIdentityChanged,
    }
}

/// An explicit user response to an external-change prompt.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum ConflictDecision {
    /// Request a reload while retaining the prompt until the adapter succeeds.
    ReloadDisk,
    /// Keep the in-memory document without authorizing overwrite of the disk version.
    KeepEditing,
    /// Save the in-memory document to a different path.
    SaveAs,
    /// Request the second confirmation step before replacing the disk version.
    RequestOverwrite,
    /// Confirm replacement of the disk version after the second confirmation.
    ConfirmOverwrite,
    /// Cancel the overwrite second-confirm and return to the ordinary prompt.
    CancelOverwrite,
}

/// One input to the conflict reducer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictCommand {
    /// Report one classified observation for the current document revision.
    Observed {
        /// Pure classification of the captured destination state.
        kind: ExternalChangeKind,
        /// Exact content revision against which the observation was made.
        revision: Revision,
    },
    /// Report a classified observation together with the exact captured state.
    /// Keep Editing is then bound to that evidence rather than a coarse class.
    ObservedExact {
        /// Pure classification of the captured destination state.
        kind: ExternalChangeKind,
        /// Exact state or inspection failure behind the classification.
        evidence: Result<TargetState, ()>,
        /// Exact content revision against which the observation was made.
        revision: Revision,
    },
    /// Resolve the currently visible external-change decision.
    Decide(ConflictDecision),
    /// Clear retained conflict state after path replacement, reload, or a new
    /// document.
    Reset,
}

/// The single effect emitted by a conflict transition.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ConflictEffect {
    /// No adapter action is required.
    None,
    /// Show the external-change decision for the retained observation.
    Prompt(ExternalChangeKind),
    /// Show the second confirmation before replacing the disk version.
    PromptOverwriteConfirm(ExternalChangeKind),
    /// Request Reload without clearing the retained conflict observation.
    RequestReload,
    /// Request Save As without authorizing overwrite of the conflicting path.
    RequestSaveAs,
    /// Authorize overwrite of the exact regular-file observation reviewed.
    AuthorizeOverwrite(FileObservation),
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
enum ConflictPhase {
    #[default]
    Idle,
    Prompting {
        kind: ExternalChangeKind,
        evidence: Option<Result<TargetState, ()>>,
        revision: Revision,
    },
    ConfirmOverwrite {
        kind: ExternalChangeKind,
        evidence: Option<Result<TargetState, ()>>,
        revision: Revision,
    },
    KeptEditing {
        kind: ExternalChangeKind,
        evidence: Option<Result<TargetState, ()>>,
        revision: Revision,
    },
}

/// Pure state for proactive external-change decisions.
///
/// Keep Editing never rebaselines the trusted save expectation. Ordinary Save
/// therefore continues to use the save protocol's conflict detection instead of
/// silently overwriting the external revision.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct ConflictState {
    phase: ConflictPhase,
}

impl ConflictState {
    /// Returns whether an external-change decision is currently visible.
    pub const fn is_prompting(self) -> bool {
        matches!(
            self.phase,
            ConflictPhase::Prompting { .. } | ConflictPhase::ConfirmOverwrite { .. }
        )
    }

    /// Returns whether the second overwrite confirmation is visible.
    pub const fn is_confirming_overwrite(self) -> bool {
        matches!(self.phase, ConflictPhase::ConfirmOverwrite { .. })
    }

    /// Returns the observation currently shown to the user, if any.
    pub const fn prompt_kind(self) -> Option<ExternalChangeKind> {
        match self.phase {
            ConflictPhase::Prompting { kind, .. }
            | ConflictPhase::ConfirmOverwrite { kind, .. } => Some(kind),
            ConflictPhase::Idle | ConflictPhase::KeptEditing { .. } => None,
        }
    }

    /// Returns whether ordinary Save should wait for the visible conflict choice.
    ///
    /// After Keep Editing, Save remains available and still fails closed through
    /// the durable save protocol if the disk version differs.
    pub const fn blocks_ordinary_save(self) -> bool {
        matches!(
            self.phase,
            ConflictPhase::Prompting { .. } | ConflictPhase::ConfirmOverwrite { .. }
        )
    }

    /// Applies one command and returns its adapter effect.
    pub fn reduce(&mut self, command: ConflictCommand) -> ConflictEffect {
        match command {
            ConflictCommand::Observed { kind, revision } => self.observed(kind, None, revision),
            ConflictCommand::ObservedExact {
                kind,
                evidence,
                revision,
            } => self.observed(kind, Some(evidence), revision),
            ConflictCommand::Decide(decision) => self.decide(decision),
            ConflictCommand::Reset => {
                self.phase = ConflictPhase::Idle;
                ConflictEffect::None
            }
        }
    }

    fn observed(
        &mut self,
        kind: ExternalChangeKind,
        evidence: Option<Result<TargetState, ()>>,
        revision: Revision,
    ) -> ConflictEffect {
        if !kind.requires_prompt() {
            if matches!(
                self.phase,
                ConflictPhase::Prompting { .. }
                    | ConflictPhase::ConfirmOverwrite { .. }
                    | ConflictPhase::KeptEditing { .. }
            ) {
                self.phase = ConflictPhase::Idle;
            }
            return ConflictEffect::None;
        }

        match self.phase {
            ConflictPhase::Prompting {
                kind: current,
                evidence: current_evidence,
                revision: current_revision,
            }
            | ConflictPhase::ConfirmOverwrite {
                kind: current,
                evidence: current_evidence,
                revision: current_revision,
            } if current == kind
                && current_evidence == evidence
                && current_revision == revision =>
            {
                ConflictEffect::None
            }
            // Keep Editing is a decision about the disk version. Local typing
            // must not reopen the prompt; a different disk classification must.
            ConflictPhase::KeptEditing {
                kind: current,
                evidence: current_evidence,
                ..
            } if current == kind
                && evidence.is_none_or(|next| next.is_ok() && current_evidence == Some(next)) =>
            {
                ConflictEffect::None
            }
            ConflictPhase::Idle
            | ConflictPhase::Prompting { .. }
            | ConflictPhase::ConfirmOverwrite { .. }
            | ConflictPhase::KeptEditing { .. } => {
                self.phase = ConflictPhase::Prompting {
                    kind,
                    evidence,
                    revision,
                };
                ConflictEffect::Prompt(kind)
            }
        }
    }

    const fn decide(&mut self, decision: ConflictDecision) -> ConflictEffect {
        match (self.phase, decision) {
            (ConflictPhase::Prompting { .. }, ConflictDecision::ReloadDisk) => {
                ConflictEffect::RequestReload
            }
            (
                ConflictPhase::Prompting {
                    kind,
                    evidence,
                    revision,
                },
                ConflictDecision::KeepEditing,
            ) => {
                self.phase = ConflictPhase::KeptEditing {
                    kind,
                    evidence,
                    revision,
                };
                ConflictEffect::None
            }
            (ConflictPhase::Prompting { .. }, ConflictDecision::SaveAs) => {
                ConflictEffect::RequestSaveAs
            }
            (
                ConflictPhase::Prompting {
                    kind,
                    evidence,
                    revision,
                },
                ConflictDecision::RequestOverwrite,
            ) => {
                self.phase = ConflictPhase::ConfirmOverwrite {
                    kind,
                    evidence,
                    revision,
                };
                ConflictEffect::PromptOverwriteConfirm(kind)
            }
            (
                ConflictPhase::ConfirmOverwrite {
                    evidence: Some(Ok(TargetState::Regular(observation))),
                    ..
                },
                ConflictDecision::ConfirmOverwrite,
            ) => {
                self.phase = ConflictPhase::Idle;
                ConflictEffect::AuthorizeOverwrite(observation)
            }
            (
                ConflictPhase::ConfirmOverwrite {
                    kind,
                    evidence,
                    revision,
                },
                ConflictDecision::ConfirmOverwrite | ConflictDecision::CancelOverwrite,
            ) => {
                self.phase = ConflictPhase::Prompting {
                    kind,
                    evidence,
                    revision,
                };
                ConflictEffect::Prompt(kind)
            }
            // Stale or out-of-phase decisions never authorize destructive work.
            _ => ConflictEffect::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::save::{ContentFingerprint, FileChangeToken, FileIdentity, FileObservation};

    fn observation(marker: u64) -> FileObservation {
        FileObservation::new(
            FileIdentity::new(u128::from(marker), u128::from(marker.wrapping_add(1))),
            ContentFingerprint::from_bytes(&[marker as u8]),
            marker,
            1,
            FileChangeToken::new(i64::try_from(marker).unwrap_or(i64::MAX), 0),
        )
    }

    #[test]
    fn descriptions_are_exact_and_nontrivial() {
        assert_eq!(
            ExternalChangeKind::Unchanged.description(),
            "The file on disk still matches Noter's last trusted version."
        );
        assert_eq!(
            ExternalChangeKind::ContentOrIdentityChanged.description(),
            "The file on disk changed since Noter last loaded or saved it."
        );
        assert_eq!(
            ExternalChangeKind::Deleted.description(),
            "The file on disk is missing."
        );
        assert_eq!(
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::SymbolicLink).description(),
            "The path is now a symbolic link or reparse point."
        );
        assert_eq!(
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::Directory).description(),
            "The path is now a directory."
        );
        assert_eq!(
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::Other).description(),
            "The path is no longer an ordinary file."
        );
        assert_eq!(
            ExternalChangeKind::Unreadable.description(),
            "Noter could not safely inspect the file on disk."
        );
        for kind in [
            ExternalChangeKind::Unchanged,
            ExternalChangeKind::ContentOrIdentityChanged,
            ExternalChangeKind::Deleted,
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::SymbolicLink),
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::Directory),
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::Other),
            ExternalChangeKind::Unreadable,
        ] {
            let text = kind.description();
            assert!(!text.is_empty());
            assert_ne!(text, "xyzzy");
        }
    }

    #[test]
    fn classify_reports_exact_boundaries() {
        let first = observation(1);
        let second = observation(2);
        assert_eq!(
            classify_external_change(None, Some(Ok(TargetState::Regular(first)))),
            ExternalChangeKind::Unchanged
        );
        assert_eq!(
            classify_external_change(
                Some(TargetExpectation::Existing(first)),
                Some(Ok(TargetState::Regular(first)))
            ),
            ExternalChangeKind::Unchanged
        );
        assert_eq!(
            classify_external_change(
                Some(TargetExpectation::Existing(first)),
                Some(Ok(TargetState::Regular(second)))
            ),
            ExternalChangeKind::ContentOrIdentityChanged
        );
        assert_eq!(
            classify_external_change(
                Some(TargetExpectation::Existing(first)),
                Some(Ok(TargetState::Missing))
            ),
            ExternalChangeKind::Deleted
        );
        assert_eq!(
            classify_external_change(
                Some(TargetExpectation::Existing(first)),
                Some(Ok(TargetState::Special(SpecialFileKind::Directory)))
            ),
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::Directory)
        );
        assert_eq!(
            classify_external_change(Some(TargetExpectation::Existing(first)), Some(Err(()))),
            ExternalChangeKind::Unreadable
        );
        assert_eq!(
            classify_external_change(
                Some(TargetExpectation::Missing),
                Some(Ok(TargetState::Missing))
            ),
            ExternalChangeKind::Unchanged
        );
        assert_eq!(
            classify_external_change(
                Some(TargetExpectation::Missing),
                Some(Ok(TargetState::Regular(first)))
            ),
            ExternalChangeKind::ContentOrIdentityChanged
        );
    }

    #[test]
    fn conflict_state_prompts_once_and_keep_editing_does_not_authorize_reload() {
        let mut state = ConflictState::default();
        let revision = Revision::new(3);
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                revision,
            }),
            ConflictEffect::Prompt(ExternalChangeKind::ContentOrIdentityChanged)
        );
        assert!(state.is_prompting());
        assert!(state.blocks_ordinary_save());
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                revision,
            }),
            ConflictEffect::None
        );
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::KeepEditing)),
            ConflictEffect::None
        );
        assert!(!state.is_prompting());
        assert!(!state.blocks_ordinary_save());
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                revision,
            }),
            ConflictEffect::None
        );
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::Deleted,
                revision,
            }),
            ConflictEffect::Prompt(ExternalChangeKind::Deleted)
        );
    }

    #[test]
    fn keep_editing_does_not_reprompt_when_only_the_local_revision_moves() {
        let mut state = ConflictState::default();
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                revision: Revision::new(3),
            }),
            ConflictEffect::Prompt(ExternalChangeKind::ContentOrIdentityChanged)
        );
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::KeepEditing)),
            ConflictEffect::None
        );

        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                revision: Revision::new(4),
            }),
            ConflictEffect::None
        );
        assert!(!state.is_prompting());
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::Deleted,
                revision: Revision::new(5),
            }),
            ConflictEffect::Prompt(ExternalChangeKind::Deleted)
        );
    }

    #[test]
    fn prompting_updates_when_kind_or_revision_changes() {
        let mut state = ConflictState::default();
        let first = Revision::new(1);
        let second = Revision::new(2);
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                revision: first,
            }),
            ConflictEffect::Prompt(ExternalChangeKind::ContentOrIdentityChanged)
        );
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::Deleted,
                revision: first,
            }),
            ConflictEffect::Prompt(ExternalChangeKind::Deleted)
        );
        assert_eq!(state.prompt_kind(), Some(ExternalChangeKind::Deleted));
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::Deleted,
                revision: second,
            }),
            ConflictEffect::Prompt(ExternalChangeKind::Deleted)
        );
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::Deleted,
                revision: second,
            }),
            ConflictEffect::None
        );
    }

    #[test]
    fn reload_and_save_as_effects_are_exact() {
        let mut state = ConflictState::default();
        let revision = Revision::INITIAL;
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::Deleted,
                revision,
            }),
            ConflictEffect::Prompt(ExternalChangeKind::Deleted)
        );
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::SaveAs)),
            ConflictEffect::RequestSaveAs
        );
        assert!(state.is_prompting());
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::ReloadDisk)),
            ConflictEffect::RequestReload
        );
        assert!(state.is_prompting());
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::ReloadDisk)),
            ConflictEffect::RequestReload
        );
    }

    #[test]
    fn overwrite_requires_a_second_confirmation() {
        let mut state = ConflictState::default();
        let revision = Revision::new(9);
        let reviewed = observation(9);
        assert_eq!(
            state.reduce(ConflictCommand::ObservedExact {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                evidence: Ok(TargetState::Regular(reviewed)),
                revision,
            }),
            ConflictEffect::Prompt(ExternalChangeKind::ContentOrIdentityChanged)
        );
        // First click only enters the second-confirm phase.
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::RequestOverwrite)),
            ConflictEffect::PromptOverwriteConfirm(ExternalChangeKind::ContentOrIdentityChanged)
        );
        assert!(state.is_confirming_overwrite());
        assert!(state.blocks_ordinary_save());
        // A first-phase decision is inert while confirming overwrite.
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::KeepEditing)),
            ConflictEffect::None
        );
        assert!(state.is_confirming_overwrite());
        // Cancel returns to the ordinary prompt without authorizing overwrite.
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::CancelOverwrite)),
            ConflictEffect::Prompt(ExternalChangeKind::ContentOrIdentityChanged)
        );
        assert!(!state.is_confirming_overwrite());
        assert!(state.is_prompting());
        let _ = state.reduce(ConflictCommand::Decide(ConflictDecision::RequestOverwrite));
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::ConfirmOverwrite)),
            ConflictEffect::AuthorizeOverwrite(reviewed)
        );
        assert!(!state.is_prompting());
        // Stale second-confirm after idle is inert.
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::ConfirmOverwrite)),
            ConflictEffect::None
        );
    }

    #[test]
    fn overwrite_confirmation_is_bound_to_the_exact_reviewed_observation() {
        let first = observation(10);
        let second = observation(11);
        let revision = Revision::new(9);
        let mut state = ConflictState::default();

        let _ = state.reduce(ConflictCommand::ObservedExact {
            kind: ExternalChangeKind::ContentOrIdentityChanged,
            evidence: Ok(TargetState::Regular(first)),
            revision,
        });
        let _ = state.reduce(ConflictCommand::Decide(ConflictDecision::RequestOverwrite));
        assert!(state.is_confirming_overwrite());

        assert_eq!(
            state.reduce(ConflictCommand::ObservedExact {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                evidence: Ok(TargetState::Regular(second)),
                revision,
            }),
            ConflictEffect::Prompt(ExternalChangeKind::ContentOrIdentityChanged)
        );
        assert!(state.is_prompting());
        assert!(!state.is_confirming_overwrite());
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::ConfirmOverwrite)),
            ConflictEffect::None
        );

        let _ = state.reduce(ConflictCommand::Decide(ConflictDecision::RequestOverwrite));
        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::ConfirmOverwrite)),
            ConflictEffect::AuthorizeOverwrite(second)
        );
    }

    #[test]
    fn overwrite_requires_exact_regular_file_evidence() {
        let mut state = ConflictState::default();
        let kind = ExternalChangeKind::ContentOrIdentityChanged;
        let _ = state.reduce(ConflictCommand::Observed {
            kind,
            revision: Revision::new(4),
        });
        let _ = state.reduce(ConflictCommand::Decide(ConflictDecision::RequestOverwrite));

        assert_eq!(
            state.reduce(ConflictCommand::Decide(ConflictDecision::ConfirmOverwrite)),
            ConflictEffect::Prompt(kind)
        );
        assert!(state.is_prompting());
        assert!(!state.is_confirming_overwrite());
    }

    #[test]
    fn unchanged_observation_clears_a_stale_prompt() {
        let mut state = ConflictState::default();
        let revision = Revision::new(1);
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                revision,
            }),
            ConflictEffect::Prompt(ExternalChangeKind::ContentOrIdentityChanged)
        );
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::Unchanged,
                revision,
            }),
            ConflictEffect::None
        );
        assert!(!state.is_prompting());
    }

    #[test]
    fn reset_clears_kept_editing() {
        let mut state = ConflictState::default();
        let revision = Revision::new(2);
        let _ = state.reduce(ConflictCommand::Observed {
            kind: ExternalChangeKind::Deleted,
            revision,
        });
        let _ = state.reduce(ConflictCommand::Decide(ConflictDecision::KeepEditing));
        assert_eq!(state.reduce(ConflictCommand::Reset), ConflictEffect::None);
        assert_eq!(
            state.reduce(ConflictCommand::Observed {
                kind: ExternalChangeKind::Deleted,
                revision,
            }),
            ConflictEffect::Prompt(ExternalChangeKind::Deleted)
        );
    }

    #[test]
    fn keep_editing_is_bound_to_the_exact_regular_file_observation() {
        let first = TargetState::Regular(observation(31));
        let second = TargetState::Regular(observation(32));
        let mut state = ConflictState::default();

        assert_eq!(
            state.reduce(ConflictCommand::ObservedExact {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                evidence: Ok(first),
                revision: Revision::new(7),
            }),
            ConflictEffect::Prompt(ExternalChangeKind::ContentOrIdentityChanged)
        );
        let _ = state.reduce(ConflictCommand::Decide(ConflictDecision::KeepEditing));
        assert_eq!(
            state.reduce(ConflictCommand::ObservedExact {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                evidence: Ok(first),
                revision: Revision::new(8),
            }),
            ConflictEffect::None
        );
        assert_eq!(
            state.reduce(ConflictCommand::ObservedExact {
                kind: ExternalChangeKind::ContentOrIdentityChanged,
                evidence: Ok(second),
                revision: Revision::new(8),
            }),
            ConflictEffect::Prompt(ExternalChangeKind::ContentOrIdentityChanged)
        );
    }

    #[test]
    fn keep_editing_never_suppresses_a_later_uninspectable_state() {
        let mut state = ConflictState::default();
        let observe_error = || ConflictCommand::ObservedExact {
            kind: ExternalChangeKind::Unreadable,
            evidence: Err(()),
            revision: Revision::new(7),
        };

        assert_eq!(
            state.reduce(observe_error()),
            ConflictEffect::Prompt(ExternalChangeKind::Unreadable)
        );
        let _ = state.reduce(ConflictCommand::Decide(ConflictDecision::KeepEditing));
        assert_eq!(
            state.reduce(observe_error()),
            ConflictEffect::Prompt(ExternalChangeKind::Unreadable)
        );
    }
}
