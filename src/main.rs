//! Noter - A pure, reliable, cross-platform plain text editor.
//!
//! This is currently a planning skeleton. See README.md, REQUIREMENTS.md,
//! DESIGN.md, and ROADMAP.md for the full vision, architecture, and phased
//! implementation plan with strict quality gates.
//!
//! Philosophy (short version):
//! - Classic Notepad spirit: open file, edit text, save file, get out of the way.
//! - Zero telemetry, zero bloat, zero "smart" rewriting of user content.
//! - System light/dark theme plus optional Markdown preview as the only 2026 QOL additions.
//! - Reliability (atomic saves, recovery, line-ending fidelity) is the top feature.

use std::io::{self, Write};

const TITLE: &str = "Noter (planning skeleton)";
const README_LINE: &str = "See README.md for build instructions and current status.";
const DOCS_LINE: &str = "All planning documents live in the repo root and are part of the product.";

fn status_text(version: &str) -> String {
    format!("{TITLE}\nVersion: {version}\n{README_LINE}\n{DOCS_LINE}\n")
}

fn write_status(mut writer: impl Write, version: &str) -> io::Result<()> {
    writer.write_all(status_text(version).as_bytes())
}

fn main() {
    write_status(io::stdout(), env!("CARGO_PKG_VERSION"))
        .expect("stdout should accept Noter status output");
}

#[cfg(test)]
mod tests {
    use std::io::{self, Write};

    use crate::{DOCS_LINE, README_LINE, TITLE, status_text, write_status};

    #[test]
    fn status_text_includes_version_and_guidance() {
        let text = status_text("9.8.7");

        assert!(text.contains(TITLE));
        assert!(text.contains("Version: 9.8.7"));
        assert!(text.contains(README_LINE));
        assert!(text.contains(DOCS_LINE));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn write_status_writes_exact_status_text() {
        let mut output = Vec::new();

        write_status(&mut output, "1.2.3").expect("vec writes should succeed");

        assert_eq!(output, status_text("1.2.3").into_bytes());
    }

    #[test]
    fn write_status_surfaces_writer_errors() {
        struct FailingWriter;

        impl Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let error = write_status(FailingWriter, "1.2.3").expect_err("write should fail");

        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }
}
