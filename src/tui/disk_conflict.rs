//! One-line terminal wording for a save that found a disk change.
//!
//! The session drives `ConflictState` for the decision. This module only
//! chooses words. Prompts name no path and quote no file text, and every
//! line fits an 80-column status row.

use noter::core::conflict::ExternalChangeKind;
use noter::core::save::{SpecialFileKind, TargetExpectation, TargetState};

/// The reviewed destination from a conflicted save.
#[derive(Clone, Copy, Debug)]
pub(super) struct ReviewedDisk {
    pub(super) expected: TargetExpectation,
    pub(super) actual: TargetState,
    pub(super) cleanup_failed: bool,
}

/// Facts that select one status line.
#[derive(Clone, Copy, Debug)]
pub(super) struct NoticeFacts {
    pub(super) kind: ExternalChangeKind,
    pub(super) metadata_only: bool,
    pub(super) cleanup_failed: bool,
    pub(super) overwrite_allowed: bool,
}

/// Second confirmation before the open text replaces the reviewed disk version.
pub(super) const OVERWRITE_CONFIRM: &str = "Replace the disk version with this text? (y)es (n)o";

/// Second confirmation before unsaved text is discarded and the disk version is loaded.
pub(super) const RELOAD_CONFIRM: &str = "Discard this text and load the disk version? (y)es (n)o";

/// Keep Editing left the open text in place and did not write it.
pub(super) const KEPT: &str = "Kept this text. It is not written over the disk version.";

/// The reviewed observation was gone before overwrite could write.
pub(super) const CHANGED_AGAIN: &str =
    "The disk version changed again. Review it before replacing it.";

/// Reload could not read a replacement, so the open text is unchanged.
pub(super) const RELOAD_FAILED: &str = "Could not load the disk version. This text is still open.";

/// Reload replaced the open text with the disk version.
pub(super) const LOADED: &str = "Loaded the disk version.";

/// Overwrite stopped because the path could not be inspected.
pub(super) const UNREADABLE_OVERWRITE: &str =
    "The disk version could not be read. It was not replaced.";

/// The path matches the trusted baseline again, so Save can write normally.
pub(super) const MATCHES_AGAIN: &str = "The disk version matches again. Save to write this text.";

/// Returns whether the same regular file still has the same bytes.
///
/// A different identity is a replacement, not a metadata change. The link
/// count and change token may differ while the bytes still match.
pub(super) fn metadata_only(reviewed: &ReviewedDisk) -> bool {
    match (reviewed.expected, reviewed.actual) {
        (TargetExpectation::Existing(expected), TargetState::Regular(actual)) => {
            expected.identity() == actual.identity()
                && expected.fingerprint() == actual.fingerprint()
                && expected.length() == actual.length()
                && expected != actual
        }
        _ => false,
    }
}

/// Returns the status line for one reviewed conflict.
///
/// Overwrite is offered only for a regular file whose identity or metadata
/// changed. A private cleanup failure uses a fixed sentence, never the
/// storage diagnostic.
pub(super) const fn notice(facts: NoticeFacts) -> &'static str {
    let overwrite = facts.overwrite_allowed
        && matches!(facts.kind, ExternalChangeKind::ContentOrIdentityChanged);
    match facts.kind {
        ExternalChangeKind::ContentOrIdentityChanged => {
            content_notice(facts.metadata_only, facts.cleanup_failed, overwrite)
        }
        ExternalChangeKind::Deleted => deleted_notice(facts.cleanup_failed),
        ExternalChangeKind::ReplacedBySpecial(kind) => special_notice(kind, facts.cleanup_failed),
        ExternalChangeKind::Unreadable => unreadable_notice(facts.cleanup_failed),
        ExternalChangeKind::Unchanged => unchanged_notice(facts.cleanup_failed),
    }
}

const fn content_notice(
    metadata_only: bool,
    cleanup_failed: bool,
    overwrite: bool,
) -> &'static str {
    match (metadata_only, cleanup_failed, overwrite) {
        (true, false, true) => {
            "Metadata changed; text matches. (r)eload (k)eep ^O Save As (o)verwrite"
        }
        (true, false, false) => "Metadata changed; text matches. (r)eload (k)eep ^O Save As",
        (true, true, true) => {
            "Metadata; text matches. Private file remains. (r)eload (k)eep ^O Save As (o)"
        }
        (true, true, false) => {
            "Metadata; text matches. Private file remains. (r)eload (k)eep ^O Save As"
        }
        (false, false, true) => "Text changed on disk. (r)eload (k)eep ^O Save As (o)verwrite",
        (false, false, false) => "Text changed on disk. (r)eload (k)eep ^O Save As",
        (false, true, true) => {
            "Text changed. Private file remains. (r)eload (k)eep ^O Save As (o)verwrite"
        }
        (false, true, false) => "Text changed. Private file remains. (r)eload (k)eep ^O Save As",
    }
}

const fn deleted_notice(cleanup_failed: bool) -> &'static str {
    if cleanup_failed {
        "File missing. Private file remains. (r)eload (k)eep ^O Save As"
    } else {
        "The file on disk is missing. (r)eload (k)eep ^O Save As"
    }
}

const fn special_notice(kind: SpecialFileKind, cleanup_failed: bool) -> &'static str {
    match (kind, cleanup_failed) {
        (SpecialFileKind::SymbolicLink, false) => {
            "The path is now a symbolic link. (r)eload (k)eep ^O Save As"
        }
        (SpecialFileKind::SymbolicLink, true) => {
            "Now a symbolic link. Private file remains. (r)eload (k)eep ^O Save As"
        }
        (SpecialFileKind::Directory, false) => {
            "The path is now a directory. (r)eload (k)eep ^O Save As"
        }
        (SpecialFileKind::Directory, true) => {
            "Now a directory. Private file remains. (r)eload (k)eep ^O Save As"
        }
        (SpecialFileKind::Other, false) => {
            "The path is no longer an ordinary file. (r)eload (k)eep ^O Save As"
        }
        (SpecialFileKind::Other, true) => {
            "Not an ordinary file. Private file remains. (r)eload (k)eep ^O Save As"
        }
    }
}

const fn unreadable_notice(cleanup_failed: bool) -> &'static str {
    if cleanup_failed {
        "Could not read the file. Private file remains. (r)eload (k)eep ^O Save As"
    } else {
        "The file on disk could not be read. (r)eload (k)eep ^O Save As"
    }
}

const fn unchanged_notice(cleanup_failed: bool) -> &'static str {
    if cleanup_failed {
        "Disk changed. Private file remains. (r)eload (k)eep ^O Save As"
    } else {
        "The file on disk changed. (r)eload (k)eep ^O Save As"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use noter::core::save::{ContentFingerprint, FileChangeToken, FileIdentity, FileObservation};
    use noter::core::terminal_text::{display_width, is_terminal_unsafe};

    #[test]
    fn every_conflict_notice_fits_and_names_the_decision() {
        let kinds = [
            ExternalChangeKind::Unchanged,
            ExternalChangeKind::ContentOrIdentityChanged,
            ExternalChangeKind::Deleted,
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::SymbolicLink),
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::Directory),
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::Other),
            ExternalChangeKind::Unreadable,
        ];
        for kind in kinds {
            for metadata_only in [false, true] {
                for cleanup_failed in [false, true] {
                    for overwrite_allowed in [false, true] {
                        let text = notice(NoticeFacts {
                            kind,
                            metadata_only,
                            cleanup_failed,
                            overwrite_allowed,
                        });
                        assert!(
                            display_width(text) <= 80,
                            "{text} is {} columns",
                            display_width(text)
                        );
                        assert!(
                            text.contains("(r)")
                                && text.contains("(k)")
                                && text.contains("Save As"),
                            "{text}"
                        );
                        assert!(!text.contains('\n'), "{text}");
                        assert!(!text.chars().any(is_terminal_unsafe), "{text}");
                        let offers_overwrite = text.contains("(o)");
                        let should_offer = overwrite_allowed
                            && matches!(kind, ExternalChangeKind::ContentOrIdentityChanged);
                        assert_eq!(
                            offers_overwrite, should_offer,
                            "{kind:?} metadata={metadata_only} cleanup={cleanup_failed} {text}"
                        );
                        assert_wording(kind, metadata_only, cleanup_failed, text);
                    }
                }
            }
        }
    }

    fn assert_wording(
        kind: ExternalChangeKind,
        metadata_only: bool,
        cleanup_failed: bool,
        text: &str,
    ) {
        if cleanup_failed {
            assert!(text.contains("Private file remains"), "{text}");
        } else {
            assert!(!text.contains("Private file"), "{text}");
        }
        match kind {
            ExternalChangeKind::ContentOrIdentityChanged if metadata_only => {
                assert!(
                    text.contains("Metadata") && text.contains("matches"),
                    "{text}"
                );
                assert!(!text.contains("Text changed"), "{text}");
            }
            ExternalChangeKind::ContentOrIdentityChanged => {
                assert!(text.contains("Text changed"), "{text}");
                assert!(!text.contains("Metadata"), "{text}");
            }
            ExternalChangeKind::Deleted => assert!(text.contains("missing"), "{text}"),
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::SymbolicLink) => {
                assert!(text.contains("symbolic link"), "{text}");
            }
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::Directory) => {
                assert!(text.contains("directory"), "{text}");
            }
            ExternalChangeKind::ReplacedBySpecial(SpecialFileKind::Other) => {
                assert!(text.contains("ordinary"), "{text}");
            }
            ExternalChangeKind::Unreadable => assert!(
                text.contains("could not be read") || text.contains("Could not read"),
                "{text}"
            ),
            ExternalChangeKind::Unchanged => {
                assert!(!text.contains("Metadata"), "{text}");
                assert!(!text.contains("Text changed"), "{text}");
            }
        }
    }

    #[test]
    fn confirmation_lines_fit_an_80_column_status_row() {
        for text in [
            OVERWRITE_CONFIRM,
            RELOAD_CONFIRM,
            KEPT,
            CHANGED_AGAIN,
            RELOAD_FAILED,
            LOADED,
            UNREADABLE_OVERWRITE,
            MATCHES_AGAIN,
        ] {
            assert!(display_width(text) <= 80, "{text}");
            assert!(!text.chars().any(is_terminal_unsafe), "{text}");
            assert!(!text.contains('\n'), "{text}");
        }
        assert!(OVERWRITE_CONFIRM.contains("(y)") && OVERWRITE_CONFIRM.contains("(n)"));
        assert!(RELOAD_CONFIRM.contains("Discard"));
        assert!(KEPT.contains("not written"));
        assert!(RELOAD_FAILED.contains("still open"));
        assert!(UNREADABLE_OVERWRITE.contains("not replaced"));
    }

    fn observation(identity: u128, bytes: &[u8], links: u64, token: i64) -> FileObservation {
        FileObservation::new(
            FileIdentity::new(identity, identity),
            ContentFingerprint::from_bytes(bytes),
            bytes.len() as u64,
            links,
            FileChangeToken::new(token, 0),
        )
    }

    #[test]
    fn metadata_only_is_the_same_file_with_the_same_bytes() {
        let bytes = "café".as_bytes();
        let expected = observation(7, bytes, 1, 1);
        let reviewed = |actual| ReviewedDisk {
            expected: TargetExpectation::Existing(expected),
            actual,
            cleanup_failed: false,
        };
        assert!(metadata_only(&reviewed(TargetState::Regular(observation(
            7, bytes, 1, 2
        )))));
        assert!(metadata_only(&reviewed(TargetState::Regular(observation(
            7, bytes, 2, 1
        )))));
        assert!(!metadata_only(&reviewed(TargetState::Regular(
            observation(8, bytes, 1, 2)
        ))));
        assert!(!metadata_only(&reviewed(TargetState::Regular(
            observation(7, b"other", 1, 2)
        ))));
        assert!(!metadata_only(&reviewed(TargetState::Regular(expected))));
        assert!(!metadata_only(&reviewed(TargetState::Missing)));
        assert!(!metadata_only(&reviewed(TargetState::Special(
            SpecialFileKind::SymbolicLink
        ))));
    }
}
