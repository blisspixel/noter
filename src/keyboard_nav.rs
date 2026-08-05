//! Platform keyboard policy for pure caret navigation.
//!
//! Resolves word, line-home/end, and document gestures to
//! [`noter::core::navigation`] moves. Character arrows remain with egui so
//! grapheme and visual column behavior stay platform-native until M5.

use eframe::egui::{self, Key, Modifiers};
use noter::core::edit::Selection;
use noter::core::navigation::{MoveDirection, MoveUnit, extend_selection, move_caret};

/// One resolved navigation gesture for the document editor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct NavigationGesture {
    /// Movement direction for word and document steps.
    pub direction: MoveDirection,
    /// Navigation unit.
    pub unit: MoveUnit,
    /// When true, keep the selection anchor and move only the active caret.
    pub extend: bool,
}

impl NavigationGesture {
    /// Applies this gesture to `selection` over `source`.
    ///
    /// Without Shift, a non-empty selection collapses to the edge in the
    /// movement direction before the caret steps (forward uses the high edge,
    /// backward the low edge). With Shift, only the active caret moves.
    #[must_use]
    pub fn apply(self, source: &str, selection: Selection) -> Selection {
        if self.extend {
            return extend_selection(source, selection, self.direction, self.unit);
        }
        let from = match self.direction {
            MoveDirection::Backward => selection.ordered_range().start(),
            MoveDirection::Forward => selection.ordered_range().end(),
        };
        let active = move_caret(source, from, self.direction, self.unit);
        Selection::caret(active)
    }
}

/// Platform family used only to choose word-modifier keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyboardPlatform {
    /// Windows and Linux: Ctrl+Arrow for word steps.
    WindowsLike,
    /// macOS: Alt/Option+Arrow for word steps; Cmd+Arrow for line home/end.
    Mac,
}

impl KeyboardPlatform {
    /// Maps an egui OS enum onto the keyboard policy family.
    #[must_use]
    pub const fn from_egui(os: egui::os::OperatingSystem) -> Self {
        match os {
            egui::os::OperatingSystem::Mac => Self::Mac,
            _ => Self::WindowsLike,
        }
    }
}

/// Resolves a key press into a document navigation gesture.
///
/// Returns `None` for keys the platform text widget should handle (plain
/// arrows, printable input, edit commands).
#[must_use]
pub fn resolve_navigation_gesture(
    key: Key,
    modifiers: Modifiers,
    platform: KeyboardPlatform,
) -> Option<NavigationGesture> {
    let extend = modifiers.shift;
    // Ignore command/ctrl/alt combinations that belong to other commands
    // (for example Ctrl+S) by requiring exact modifier shapes below.
    match platform {
        KeyboardPlatform::WindowsLike => resolve_windows_like(key, modifiers, extend),
        KeyboardPlatform::Mac => resolve_mac(key, modifiers, extend),
    }
}

fn resolve_windows_like(key: Key, modifiers: Modifiers, extend: bool) -> Option<NavigationGesture> {
    // Word: Ctrl+Left/Right (Shift optional). Command is the same as Ctrl on Windows.
    if (modifiers.ctrl || modifiers.command)
        && !modifiers.alt
        && matches!(key, Key::ArrowLeft | Key::ArrowRight)
    {
        return Some(NavigationGesture {
            direction: if key == Key::ArrowLeft {
                MoveDirection::Backward
            } else {
                MoveDirection::Forward
            },
            unit: MoveUnit::Word,
            extend,
        });
    }
    // Document: Ctrl+Home / Ctrl+End.
    if (modifiers.ctrl || modifiers.command)
        && !modifiers.alt
        && matches!(key, Key::Home | Key::End)
    {
        return Some(NavigationGesture {
            direction: if key == Key::Home {
                MoveDirection::Backward
            } else {
                MoveDirection::Forward
            },
            unit: MoveUnit::Document,
            extend,
        });
    }
    // Line home/end without Ctrl/Alt/Cmd (Shift optional for extend).
    if !modifiers.alt && !modifiers.ctrl && !modifiers.command {
        match key {
            Key::Home => {
                return Some(NavigationGesture {
                    direction: MoveDirection::Backward,
                    unit: MoveUnit::LineHome,
                    extend,
                });
            }
            Key::End => {
                return Some(NavigationGesture {
                    direction: MoveDirection::Forward,
                    unit: MoveUnit::LineEnd,
                    extend,
                });
            }
            _ => {}
        }
    }
    None
}

fn resolve_mac(key: Key, modifiers: Modifiers, extend: bool) -> Option<NavigationGesture> {
    // Word: Option/Alt+Left/Right.
    if modifiers.alt
        && !modifiers.ctrl
        && !modifiers.command
        && matches!(key, Key::ArrowLeft | Key::ArrowRight)
    {
        return Some(NavigationGesture {
            direction: if key == Key::ArrowLeft {
                MoveDirection::Backward
            } else {
                MoveDirection::Forward
            },
            unit: MoveUnit::Word,
            extend,
        });
    }
    // Line home/end: Cmd+Left/Right.
    if modifiers.command
        && !modifiers.alt
        && !modifiers.ctrl
        && matches!(key, Key::ArrowLeft | Key::ArrowRight)
    {
        return Some(NavigationGesture {
            direction: if key == Key::ArrowLeft {
                MoveDirection::Backward
            } else {
                MoveDirection::Forward
            },
            unit: if key == Key::ArrowLeft {
                MoveUnit::LineHome
            } else {
                MoveUnit::LineEnd
            },
            extend,
        });
    }
    // Document: Cmd+Up/Down or Cmd+Home/End.
    if modifiers.command
        && !modifiers.alt
        && !modifiers.ctrl
        && matches!(key, Key::ArrowUp | Key::Home)
    {
        return Some(NavigationGesture {
            direction: MoveDirection::Backward,
            unit: MoveUnit::Document,
            extend,
        });
    }
    if modifiers.command
        && !modifiers.alt
        && !modifiers.ctrl
        && matches!(key, Key::ArrowDown | Key::End)
    {
        return Some(NavigationGesture {
            direction: MoveDirection::Forward,
            unit: MoveUnit::Document,
            extend,
        });
    }
    // Physical Home/End without Cmd still map to line home/end.
    if !modifiers.alt && !modifiers.ctrl && !modifiers.command {
        match key {
            Key::Home => {
                return Some(NavigationGesture {
                    direction: MoveDirection::Backward,
                    unit: MoveUnit::LineHome,
                    extend,
                });
            }
            Key::End => {
                return Some(NavigationGesture {
                    direction: MoveDirection::Forward,
                    unit: MoveUnit::LineEnd,
                    extend,
                });
            }
            _ => {}
        }
    }
    None
}

/// Consumes matching navigation key events for this frame.
///
/// Events are removed so `TextEdit` does not also apply them. Every matching
/// event is returned in order so key-repeat buffers still advance the caret
/// one step per event.
pub fn consume_navigation_gestures(
    ui: &egui::Ui,
    platform: KeyboardPlatform,
) -> Vec<NavigationGesture> {
    let mut resolved = Vec::new();
    ui.input_mut(|input| {
        input.events.retain(|event| {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            resolve_navigation_gesture(*key, *modifiers, platform).is_none_or(|gesture| {
                resolved.push(gesture);
                false
            })
        });
    });
    resolved
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> Modifiers {
        Modifiers::NONE
    }

    fn ctrl() -> Modifiers {
        Modifiers::CTRL
    }

    fn ctrl_shift() -> Modifiers {
        Modifiers::CTRL | Modifiers::SHIFT
    }

    fn alt() -> Modifiers {
        Modifiers::ALT
    }

    fn command() -> Modifiers {
        Modifiers::COMMAND
    }

    fn shift() -> Modifiers {
        Modifiers::SHIFT
    }

    #[test]
    fn windows_word_and_document_gestures() {
        let platform = KeyboardPlatform::WindowsLike;
        let word =
            resolve_navigation_gesture(Key::ArrowRight, ctrl(), platform).expect("ctrl+right");
        assert_eq!(word.unit, MoveUnit::Word);
        assert_eq!(word.direction, MoveDirection::Forward);
        assert!(!word.extend);

        let extend_word = resolve_navigation_gesture(Key::ArrowLeft, ctrl_shift(), platform)
            .expect("ctrl+shift+left");
        assert_eq!(extend_word.unit, MoveUnit::Word);
        assert!(extend_word.extend);

        let home = resolve_navigation_gesture(Key::Home, none(), platform).expect("home");
        assert_eq!(home.unit, MoveUnit::LineHome);

        let doc = resolve_navigation_gesture(Key::End, ctrl(), platform).expect("ctrl+end");
        assert_eq!(doc.unit, MoveUnit::Document);
        assert_eq!(doc.direction, MoveDirection::Forward);
    }

    #[test]
    fn mac_option_word_and_cmd_line_gestures() {
        let platform = KeyboardPlatform::Mac;
        let word =
            resolve_navigation_gesture(Key::ArrowLeft, alt(), platform).expect("option+left");
        assert_eq!(word.unit, MoveUnit::Word);
        assert_eq!(word.direction, MoveDirection::Backward);

        let line =
            resolve_navigation_gesture(Key::ArrowRight, command(), platform).expect("cmd+right");
        assert_eq!(line.unit, MoveUnit::LineEnd);

        let doc = resolve_navigation_gesture(Key::ArrowUp, command(), platform).expect("cmd+up");
        assert_eq!(doc.unit, MoveUnit::Document);
        assert_eq!(doc.direction, MoveDirection::Backward);
    }

    #[test]
    fn plain_arrows_are_left_to_egui() {
        assert!(
            resolve_navigation_gesture(Key::ArrowLeft, none(), KeyboardPlatform::WindowsLike)
                .is_none()
        );
        assert!(
            resolve_navigation_gesture(Key::ArrowRight, shift(), KeyboardPlatform::Mac).is_none()
        );
    }

    #[test]
    fn apply_word_move_and_extend() {
        let source = "hello world";
        let caret = Selection::caret(0);
        let gesture = NavigationGesture {
            direction: MoveDirection::Forward,
            unit: MoveUnit::Word,
            extend: false,
        };
        assert_eq!(gesture.apply(source, caret), Selection::caret(5));

        let extend = NavigationGesture {
            direction: MoveDirection::Forward,
            unit: MoveUnit::Word,
            extend: true,
        };
        let selected = extend.apply(source, Selection::caret(0));
        assert_eq!(selected.anchor(), 0);
        assert_eq!(selected.active(), 5);
    }

    #[test]
    fn non_extend_collapses_selection_to_direction_edge() {
        let source = "hello world";
        // Selection active is on the left (backward drag).
        let selection = Selection::new(5, 1);
        let forward = NavigationGesture {
            direction: MoveDirection::Forward,
            unit: MoveUnit::Word,
            extend: false,
        };
        // Collapse to high edge (5, the space) then word-forward to "world" (6).
        assert_eq!(forward.apply(source, selection), Selection::caret(6));

        let backward = NavigationGesture {
            direction: MoveDirection::Backward,
            unit: MoveUnit::Word,
            extend: false,
        };
        // Collapse to low edge (1) then word-backward to 0.
        assert_eq!(backward.apply(source, selection), Selection::caret(0));
    }

    #[test]
    fn apply_line_home_end_on_mixed_endings() {
        let source = "ab\r\ncd";
        let home = NavigationGesture {
            direction: MoveDirection::Backward,
            unit: MoveUnit::LineHome,
            extend: false,
        };
        assert_eq!(home.apply(source, Selection::caret(1)), Selection::caret(0));
        let end = NavigationGesture {
            direction: MoveDirection::Forward,
            unit: MoveUnit::LineEnd,
            extend: false,
        };
        assert_eq!(end.apply(source, Selection::caret(1)), Selection::caret(2));
        assert_eq!(end.apply(source, Selection::caret(4)), Selection::caret(6));
    }
}
