//! Bounded-latency backward character movement on a supported-size document.

use std::hint::black_box;
use std::time::{Duration, Instant};

use noter::core::limits::MAX_DOCUMENT_BYTES;
use noter::core::navigation::{MoveDirection, MoveUnit, move_caret};

fn main() {
    const SUFFIX: &str = "界\u{202E}e\u{301}";
    const PREFIX_BYTES: usize = MAX_DOCUMENT_BYTES - SUFFIX.len();
    const STEPS: usize = 128;
    const LIMIT: Duration = Duration::from_secs(2);

    let mut source = "a".repeat(PREFIX_BYTES);
    source.push_str(SUFFIX);
    assert_eq!(source.len(), MAX_DOCUMENT_BYTES);

    let mut caret = source.len();
    for character in SUFFIX.chars().rev() {
        caret = move_caret(&source, caret, MoveDirection::Backward, MoveUnit::Character);
        assert!(source.is_char_boundary(caret));
        assert_eq!(source[caret..].chars().next(), Some(character));
    }
    assert_eq!(caret, PREFIX_BYTES);

    let start = Instant::now();
    for _ in 0..STEPS {
        caret = move_caret(
            black_box(source.as_str()),
            black_box(caret),
            MoveDirection::Backward,
            MoveUnit::Character,
        );
    }
    let elapsed = start.elapsed();
    assert_eq!(caret, PREFIX_BYTES - STEPS);
    eprintln!("64 MiB backward character steps: {STEPS} in {elapsed:?}");
    assert!(
        elapsed < LIMIT,
        "backward character movement exceeded the {LIMIT:?} latency bound"
    );
}
