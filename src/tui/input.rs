//! Terminal input decoding.
//!
//! A read from the terminal can end anywhere: inside a UTF-8 character,
//! inside a control sequence, or inside a bracketed paste. The decoder holds
//! back an incomplete tail until the next read completes it, so no input is
//! split, garbled, or mistaken for a different key.

/// A parsed terminal key event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TuiKey {
    Char(char),
    Enter,
    Backspace,
    Delete,
    Tab,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    CtrlLeft,
    CtrlRight,
    Escape,
    Ctrl(char),
    F(u8),
}

/// A parsed mouse event from SGR mouse tracking.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TuiMouseEvent {
    Press { col: u16, row: u16 },
    ScrollUp { col: u16, row: u16 },
    ScrollDown { col: u16, row: u16 },
}

/// A parsed high-level TUI input event.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum TuiEvent {
    Key(TuiKey),
    Mouse(TuiMouseEvent),
    /// Text delivered by the terminal's bracketed paste, as one unit.
    Paste(String),
}

use noter::core::limits::MAX_DOCUMENT_BYTES;

/// Bytes that open and close a bracketed paste.
const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// Longest control sequence kept waiting for its final byte. A terminal
/// sends each sequence whole, so anything longer is noise and is dropped.
const MAX_SEQUENCE_BYTES: usize = 64;

/// Turns raw terminal input into events, holding back any character,
/// control sequence, or paste that a read split in two.
#[derive(Default, Debug)]
pub struct InputDecoder {
    pending: Vec<u8>,
    paste: Option<Vec<u8>>,
}

/// The first complete item at the start of the pending bytes.
enum Decoded {
    Event(TuiEvent, usize),
    PasteStart(usize),
    Ignored(usize),
    Incomplete,
}

impl InputDecoder {
    /// Decodes `bytes` after anything held back from earlier reads.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<TuiEvent> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        let mut index = 0;
        while index < self.pending.len() {
            let rest = &self.pending[index..];
            if let Some(paste) = &mut self.paste {
                if let Some(end) = find(rest, PASTE_END) {
                    extend_bounded(paste, &rest[..end]);
                    let text = String::from_utf8_lossy(paste).into_owned();
                    events.push(TuiEvent::Paste(text));
                    self.paste = None;
                    index += end + PASTE_END.len();
                    continue;
                }
                // Keep a possible start of the closing marker for the next read.
                let keep = partial_suffix(rest, PASTE_END);
                extend_bounded(paste, &rest[..rest.len() - keep]);
                index += rest.len() - keep;
                break;
            }
            match decode(rest) {
                Decoded::Event(event, used) => {
                    events.push(event);
                    index += used;
                }
                Decoded::PasteStart(used) => {
                    self.paste = Some(Vec::new());
                    index += used;
                }
                Decoded::Ignored(used) => index += used,
                Decoded::Incomplete => break,
            }
        }
        self.pending.drain(..index);
        events
    }
}

/// Decodes the first complete item of `bytes`.
fn decode(bytes: &[u8]) -> Decoded {
    let Some(&first) = bytes.first() else {
        return Decoded::Incomplete;
    };
    if first == 0x1b {
        return decode_escape(bytes);
    }
    if first < 0x20 || first == 0x7f {
        return control_key(first).map_or(Decoded::Ignored(1), |key| {
            Decoded::Event(TuiEvent::Key(key), 1)
        });
    }
    let length = match first {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        // A stray continuation byte or an invalid lead byte starts nothing.
        _ => return Decoded::Ignored(1),
    };
    if bytes.len() < length {
        // Wait for the rest only while every byte so far could continue it.
        return if bytes[1..].iter().all(|byte| byte & 0xc0 == 0x80) {
            Decoded::Incomplete
        } else {
            Decoded::Ignored(1)
        };
    }
    match std::str::from_utf8(&bytes[..length])
        .ok()
        .and_then(|text| text.chars().next())
    {
        Some(character) if !character.is_control() => {
            Decoded::Event(TuiEvent::Key(TuiKey::Char(character)), length)
        }
        Some(_) => Decoded::Ignored(length),
        None => Decoded::Ignored(1),
    }
}

/// Maps a C0 control byte to a key. Unbound controls produce no key, so
/// they are never inserted into the document.
const fn control_key(byte: u8) -> Option<TuiKey> {
    Some(match byte {
        0x08 | 0x7f => TuiKey::Backspace,
        0x09 => TuiKey::Tab,
        // Raw mode clears ICRNL, so Enter arrives as CR and Ctrl+J as LF.
        0x0d => TuiKey::Enter,
        0x01..=0x1a => TuiKey::Ctrl((b'a' + byte - 1) as char),
        0x1f => TuiKey::Ctrl('_'),
        _ => return None,
    })
}

/// Decodes a sequence that starts with ESC.
fn decode_escape(bytes: &[u8]) -> Decoded {
    match bytes.get(1) {
        // SS3 forms, sent for arrows and Home/End in application cursor mode
        // and for F1 to F4. Anything else after ESC O is Alt+Shift+O, and the
        // byte after it is decoded on its own.
        Some(b'O') => {
            let key = match bytes.get(2) {
                None => return Decoded::Incomplete,
                Some(b'A') => TuiKey::Up,
                Some(b'B') => TuiKey::Down,
                Some(b'C') => TuiKey::Right,
                Some(b'D') => TuiKey::Left,
                Some(b'H') => TuiKey::Home,
                Some(b'F') => TuiKey::End,
                Some(b'P') => TuiKey::F(1),
                Some(b'Q') => TuiKey::F(2),
                Some(b'R') => TuiKey::F(3),
                Some(b'S') => TuiKey::F(4),
                Some(_) => return Decoded::Ignored(2),
            };
            Decoded::Event(TuiEvent::Key(key), 3)
        }
        Some(b'[') => decode_csi(bytes),
        // Alt with a printable key has no binding; the chord types nothing.
        Some(0x20..=0x7e) => Decoded::Ignored(2),
        // A read that ends in ESC is the Escape key: terminals send every
        // sequence in one write, so a lone trailing ESC starts nothing. ESC
        // before a control or non-ASCII byte is also Escape, and that byte is
        // decoded on its own.
        None | Some(_) => Decoded::Event(TuiEvent::Key(TuiKey::Escape), 1),
    }
}

/// Decodes `ESC [ parameters final`.
fn decode_csi(bytes: &[u8]) -> Decoded {
    // Parameter and intermediate bytes, then one final byte (ECMA-48).
    let final_index = 2 + bytes[2..]
        .iter()
        .take_while(|byte| (0x20..=0x3f).contains(*byte))
        .count();
    match bytes.get(final_index) {
        Some(0x40..=0x7e) => {}
        // Anything else cannot continue a sequence: this was Alt+[, and the
        // byte that follows is decoded on its own.
        Some(_) => return Decoded::Ignored(2),
        None if bytes.len() < MAX_SEQUENCE_BYTES => return Decoded::Incomplete,
        None => return Decoded::Ignored(bytes.len()),
    }
    let used = final_index + 1;
    let Ok(parameters) = std::str::from_utf8(&bytes[2..final_index]) else {
        return Decoded::Ignored(used);
    };
    if &bytes[..used] == PASTE_START {
        return Decoded::PasteStart(used);
    }
    let final_byte = bytes[final_index];
    let event = parameters.strip_prefix('<').map_or_else(
        || csi_key(parameters, final_byte).map(TuiEvent::Key),
        |mouse| decode_sgr_mouse(mouse, final_byte).map(TuiEvent::Mouse),
    );
    event.map_or(Decoded::Ignored(used), |event| Decoded::Event(event, used))
}

/// Decodes an SGR mouse report: `code;column;row` then `M` or `m`.
fn decode_sgr_mouse(parameters: &str, final_byte: u8) -> Option<TuiMouseEvent> {
    let mut fields = parameters.split(';').map(str::parse::<u16>);
    let (Some(Ok(code)), Some(Ok(col)), Some(Ok(row)), None) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    else {
        return None;
    };
    match (code, final_byte) {
        (64, _) => Some(TuiMouseEvent::ScrollUp { col, row }),
        (65, _) => Some(TuiMouseEvent::ScrollDown { col, row }),
        (0, b'M') => Some(TuiMouseEvent::Press { col, row }),
        _ => None,
    }
}

/// Maps a CSI key report to a key.
fn csi_key(parameters: &str, final_byte: u8) -> Option<TuiKey> {
    Some(match (parameters, final_byte) {
        ("", b'A') => TuiKey::Up,
        ("", b'B') => TuiKey::Down,
        ("", b'C') => TuiKey::Right,
        ("", b'D') => TuiKey::Left,
        ("", b'H') | ("1" | "7", b'~') => TuiKey::Home,
        ("", b'F') | ("4" | "8", b'~') => TuiKey::End,
        ("3", b'~') => TuiKey::Delete,
        ("5", b'~') => TuiKey::PageUp,
        ("6", b'~') => TuiKey::PageDown,
        ("1;5", b'C') => TuiKey::CtrlRight,
        ("1;5", b'D') => TuiKey::CtrlLeft,
        ("11", b'~') => TuiKey::F(1),
        ("12", b'~') => TuiKey::F(2),
        ("13", b'~') => TuiKey::F(3),
        ("14", b'~') => TuiKey::F(4),
        ("15", b'~') => TuiKey::F(5),
        _ => return None,
    })
}

/// Most pasted bytes kept. One byte past the document limit is enough for
/// the insertion policy to report the paste as shortened; the rest of an
/// oversized or unterminated paste is read and dropped, so memory stays
/// bounded and pasted bytes are never run as keys.
const MAX_PASTE_BYTES: usize = MAX_DOCUMENT_BYTES + 1;

fn extend_bounded(paste: &mut Vec<u8>, bytes: &[u8]) {
    let room = MAX_PASTE_BYTES.saturating_sub(paste.len());
    paste.extend_from_slice(&bytes[..bytes.len().min(room)]);
}

/// Returns where `needle` first occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Returns the length of the longest suffix of `bytes` that begins `marker`.
fn partial_suffix(bytes: &[u8], marker: &[u8]) -> usize {
    (1..marker.len().min(bytes.len() + 1))
        .rev()
        .find(|&length| bytes.ends_with(&marker[..length]))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(bytes: &[u8]) -> Vec<TuiEvent> {
        InputDecoder::default().feed(bytes)
    }

    #[test]
    fn parse_input_bytes_recognizes_printable_ascii_and_unicode() {
        let events = decode_all("Hello, 世界!".as_bytes());
        assert_eq!(
            events,
            vec![
                TuiEvent::Key(TuiKey::Char('H')),
                TuiEvent::Key(TuiKey::Char('e')),
                TuiEvent::Key(TuiKey::Char('l')),
                TuiEvent::Key(TuiKey::Char('l')),
                TuiEvent::Key(TuiKey::Char('o')),
                TuiEvent::Key(TuiKey::Char(',')),
                TuiEvent::Key(TuiKey::Char(' ')),
                TuiEvent::Key(TuiKey::Char('世')),
                TuiEvent::Key(TuiKey::Char('界')),
                TuiEvent::Key(TuiKey::Char('!')),
            ]
        );
    }

    #[test]
    fn parse_input_bytes_recognizes_dual_shortcuts() {
        // Ctrl+O, Ctrl+S, Ctrl+X, Ctrl+Q, Ctrl+Z, Ctrl+K
        let events = decode_all(&[0x0f, 0x13, 0x18, 0x11, 0x1a, 0x0b]);
        assert_eq!(
            events,
            vec![
                TuiEvent::Key(TuiKey::Ctrl('o')),
                TuiEvent::Key(TuiKey::Ctrl('s')),
                TuiEvent::Key(TuiKey::Ctrl('x')),
                TuiEvent::Key(TuiKey::Ctrl('q')),
                TuiEvent::Key(TuiKey::Ctrl('z')),
                TuiEvent::Key(TuiKey::Ctrl('k')),
            ]
        );
    }

    #[test]
    fn parse_input_bytes_recognizes_csi_navigation_arrows_and_keys() {
        // Up (\x1b[A), Down (\x1b[B), Home (\x1b[H), Delete (\x1b[3~)
        let events = decode_all(b"\x1b[A\x1b[B\x1b[H\x1b[3~");
        assert_eq!(
            events,
            vec![
                TuiEvent::Key(TuiKey::Up),
                TuiEvent::Key(TuiKey::Down),
                TuiEvent::Key(TuiKey::Home),
                TuiEvent::Key(TuiKey::Delete),
            ]
        );
    }

    #[test]
    fn parse_input_bytes_recognizes_sgr_mouse_events() {
        // Left click at col 10, row 5: \x1b[<0;10;5M
        // Scroll up at col 20, row 8: \x1b[<64;20;8M
        // Scroll down at col 20, row 8: \x1b[<65;20;8M
        let events = decode_all(b"\x1b[<0;10;5M\x1b[<64;20;8M\x1b[<65;20;8M");
        assert_eq!(
            events,
            vec![
                TuiEvent::Mouse(TuiMouseEvent::Press { col: 10, row: 5 }),
                TuiEvent::Mouse(TuiMouseEvent::ScrollUp { col: 20, row: 8 }),
                TuiEvent::Mouse(TuiMouseEvent::ScrollDown { col: 20, row: 8 }),
            ]
        );
    }

    fn key(key: TuiKey) -> TuiEvent {
        TuiEvent::Key(key)
    }

    #[test]
    fn characters_split_across_reads_are_joined() {
        let mut decoder = InputDecoder::default();
        let bytes = "é世😀".as_bytes();
        let mut events = Vec::new();
        for byte in bytes {
            events.extend(decoder.feed(std::slice::from_ref(byte)));
        }
        assert_eq!(
            events,
            vec![
                key(TuiKey::Char('é')),
                key(TuiKey::Char('世')),
                key(TuiKey::Char('😀')),
            ]
        );
    }

    #[test]
    fn sequences_split_across_reads_are_joined() {
        let mut decoder = InputDecoder::default();
        assert!(decoder.feed(b"\x1b[3").is_empty());
        assert_eq!(decoder.feed(b"~"), vec![key(TuiKey::Delete)]);
        assert!(decoder.feed(b"\x1b[<0;12").is_empty());
        assert_eq!(
            decoder.feed(b";4M"),
            vec![TuiEvent::Mouse(TuiMouseEvent::Press { col: 12, row: 4 })]
        );
    }

    #[test]
    fn a_trailing_escape_is_the_escape_key_and_alt_chords_type_nothing() {
        assert_eq!(decode_all(b"\x1b"), vec![key(TuiKey::Escape)]);
        assert_eq!(decode_all(b"\x1bxy"), vec![key(TuiKey::Char('y'))]);
        assert_eq!(
            decode_all(b"\x1bOxy"),
            vec![key(TuiKey::Char('x')), key(TuiKey::Char('y'))]
        );
        assert_eq!(
            decode_all("\x1bé".as_bytes()),
            vec![key(TuiKey::Escape), key(TuiKey::Char('é'))]
        );
        // Alt+[ followed by a real sequence keeps the sequence.
        assert_eq!(decode_all(b"\x1b[\x1b[A"), vec![key(TuiKey::Up)]);
    }

    #[test]
    fn application_mode_arrows_are_keys() {
        assert_eq!(
            decode_all(b"\x1bOA\x1bOB\x1bOC\x1bOD\x1bOH\x1bOF"),
            vec![
                key(TuiKey::Up),
                key(TuiKey::Down),
                key(TuiKey::Right),
                key(TuiKey::Left),
                key(TuiKey::Home),
                key(TuiKey::End),
            ]
        );
    }

    #[test]
    fn an_unterminated_paste_keeps_bounded_memory() {
        let mut decoder = InputDecoder::default();
        assert!(decoder.feed(PASTE_START).is_empty());
        let chunk = vec![b'x'; 1 << 20];
        for _ in 0..70 {
            assert!(decoder.feed(&chunk).is_empty());
        }
        assert_eq!(decoder.paste.as_ref().map(Vec::len), Some(MAX_PASTE_BYTES));
        let events = decoder.feed(PASTE_END);
        assert!(matches!(&events[..], [TuiEvent::Paste(text)] if text.len() == MAX_PASTE_BYTES));
    }

    #[test]
    fn bracketed_paste_is_one_event_even_across_reads() {
        let mut decoder = InputDecoder::default();
        assert!(decoder.feed(b"\x1b[200~line one\r\nline \x0f").is_empty());
        assert!(decoder.feed(b"two\x1b[20").is_empty());
        assert_eq!(
            decoder.feed(b"1~x"),
            vec![
                TuiEvent::Paste("line one\r\nline \u{f}two".to_owned()),
                key(TuiKey::Char('x')),
            ]
        );
    }

    #[test]
    fn enter_and_ctrl_j_are_distinct_and_unbound_controls_are_dropped() {
        assert_eq!(
            decode_all(b"\r\n\x00\x1c\x04\x7f\x08\t"),
            vec![
                key(TuiKey::Enter),
                key(TuiKey::Ctrl('j')),
                key(TuiKey::Ctrl('d')),
                key(TuiKey::Backspace),
                key(TuiKey::Backspace),
                key(TuiKey::Tab),
            ]
        );
    }

    #[test]
    fn invalid_bytes_and_typed_c1_controls_insert_nothing() {
        assert_eq!(
            decode_all(b"a\x80\xffb\xc2\x85c"),
            vec![
                key(TuiKey::Char('a')),
                key(TuiKey::Char('b')),
                key(TuiKey::Char('c')),
            ]
        );
        // A lead byte followed by a non-continuation byte is dropped at once.
        assert_eq!(decode_all(b"\xe4a"), vec![key(TuiKey::Char('a'))]);
    }

    #[test]
    fn overlong_unterminated_sequences_are_dropped() {
        let mut decoder = InputDecoder::default();
        let mut noise = b"\x1b[".to_vec();
        noise.extend(std::iter::repeat_n(b'1', MAX_SEQUENCE_BYTES));
        assert!(decoder.feed(&noise).is_empty());
        assert_eq!(decoder.feed(b"z"), vec![key(TuiKey::Char('z'))]);
    }

    proptest::proptest! {
        #[test]
        fn chunking_never_changes_the_decoded_events(
            text in "[a-zé世😀\\x1b\\[0-9;<~MAPr\\n]{0,40}",
            cuts in proptest::collection::vec(0_usize..64, 0..6),
        ) {
            let bytes = text.as_bytes();
            let whole = InputDecoder::default().feed(bytes);
            let mut decoder = InputDecoder::default();
            let mut pieces = Vec::new();
            let mut start = 0;
            let mut boundaries: Vec<usize> =
                cuts.iter().map(|cut| cut % (bytes.len() + 1)).collect();
            boundaries.sort_unstable();
            // A read that ends right after ESC reads it as the Escape key: the
            // documented trade for never waiting on a bare Escape press.
            let cuts_after_escape = boundaries
                .iter()
                .any(|&boundary| boundary > 0 && bytes[boundary - 1] == 0x1b);
            for boundary in boundaries {
                pieces.extend(decoder.feed(&bytes[start..boundary]));
                start = boundary;
            }
            pieces.extend(decoder.feed(&bytes[start..]));
            if !cuts_after_escape {
                proptest::prop_assert_eq!(pieces, whole);
            }
        }

        #[test]
        fn decoding_never_panics(bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..200)) {
            let mut decoder = InputDecoder::default();
            let _ = decoder.feed(&bytes[..bytes.len() / 2]);
            let _ = decoder.feed(&bytes[bytes.len() / 2..]);
        }
    }
}
