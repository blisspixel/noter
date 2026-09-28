//! Subprocess fixture for the bounded-memory backward-word regression.

use std::io::{self, BufRead, Write};

use noter::core::navigation::{MoveDirection, MoveUnit, move_caret};

fn main() {
    let hold = std::env::args().any(|argument| argument == "--hold");
    let prefix_bytes = if hold { 16 * 1024 * 1024 } else { 1024 };
    let mut source = "a".repeat(prefix_bytes);
    source.push_str("  界\u{202E}");

    let control_start = source.len() - '\u{202E}'.len_utf8();
    let wide_start = control_start - '界'.len_utf8();
    assert_eq!(
        move_caret(
            &source,
            source.len(),
            MoveDirection::Backward,
            MoveUnit::Word
        ),
        control_start
    );
    assert_eq!(
        move_caret(
            &source,
            control_start,
            MoveDirection::Backward,
            MoveUnit::Word
        ),
        wide_start
    );
    assert_eq!(
        move_caret(&source, wide_start, MoveDirection::Backward, MoveUnit::Word),
        0
    );

    if hold {
        println!("ready");
        io::stdout().flush().expect("fixture output should flush");
        let mut response = String::new();
        io::stdin()
            .lock()
            .read_line(&mut response)
            .expect("fixture hold should be released");
    }
}
