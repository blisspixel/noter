//! Trust-kernel cost of one keystroke in the window.
//!
//! The editor text is diffed against the rope and the resulting transaction
//! is applied. Run with `cargo bench --bench edit_latency`; it prints the
//! median and slowest of the timed samples for each document size.

use std::hint::black_box;
use std::time::{Duration, Instant};

use noter::core::document::Document;
use noter::core::edit::{EditOrigin, EditTimestamp, EditTransaction, Selection};

const SAMPLES: usize = 200;
const WARMUP: usize = 20;

fn keystroke_samples(bytes: usize) -> Vec<Duration> {
    let line = "The quick brown fox jumps over the lazy dog. 素早い茶色の狐。\r\n";
    let source = line.repeat(bytes / line.len());
    let mut document = Document::from_bytes(source.as_bytes()).expect("the corpus loads");
    let mut text = source;
    let mut middle = text.len() / 2;
    while !text.is_char_boundary(middle) {
        middle += 1;
    }
    let mut samples = Vec::with_capacity(SAMPLES);
    for round in 0..WARMUP + SAMPLES {
        text.insert(middle, 'x');
        let started = Instant::now();
        let transaction = EditTransaction::between_rope(
            document.revision(),
            document.rope(),
            black_box(&text),
            Selection::caret(middle),
            Selection::caret(middle + 1),
            EditOrigin::TextInput,
            EditTimestamp::default(),
        )
        .expect("the carets are valid")
        .expect("the text differs");
        document
            .apply_transaction(&transaction)
            .expect("the edit applies");
        let elapsed = started.elapsed();
        if round >= WARMUP {
            samples.push(elapsed);
        }
    }
    assert_eq!(document.rope().len_bytes(), text.len());
    assert!(document.is_dirty());
    samples.sort_unstable();
    samples
}

fn main() {
    for mebibytes in [1, 8, 50] {
        let samples = keystroke_samples(mebibytes << 20);
        println!(
            "{mebibytes:>2} MiB: median {:>8.1} us, slowest {:>8.1} us over {SAMPLES} keystrokes",
            samples[SAMPLES / 2].as_secs_f64() * 1e6,
            samples[SAMPLES - 1].as_secs_f64() * 1e6,
        );
    }
}
