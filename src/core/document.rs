use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use ropey::Rope;
use serde::{Deserialize, Serialize};

use crate::error::NoterError;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum LineEnding {
    Lf,
    CrLf,
    Cr,
}

impl LineEnding {
    pub fn as_str(&self) -> &'static str {
        match self {
            LineEnding::Lf => "\n",
            LineEnding::CrLf => "\r\n",
            LineEnding::Cr => "\r",
        }
    }

    /// Returns the detected line ending and the length of the BOM if present.
    pub fn detect_from_bytes(bytes: &[u8]) -> (Self, usize) {
        let mut bom_len = 0;
        if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            bom_len = 3;
        }

        // Extremely naive detection for v1: just check the first newline we see.
        let mut cr_seen = false;
        for &b in &bytes[bom_len..] {
            if b == b'\r' {
                cr_seen = true;
            } else if b == b'\n' {
                if cr_seen {
                    return (LineEnding::CrLf, bom_len);
                } else {
                    return (LineEnding::Lf, bom_len);
                }
            } else if cr_seen {
                return (LineEnding::Cr, bom_len);
            }
        }
        
        // Default to OS native if no newlines found, but since we are cross-platform, 
        // we'll default to LF on unix, CRLF on windows.
        #[cfg(windows)]
        return (LineEnding::CrLf, bom_len);
        #[cfg(not(windows))]
        return (LineEnding::Lf, bom_len);
    }
}

pub struct Document {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub line_ending: LineEnding,
    pub had_bom: bool,
    pub is_dirty: bool,
}

impl Document {
    pub fn new() -> Self {
        Self {
            rope: Rope::new(),
            path: None,
            #[cfg(windows)]
            line_ending: LineEnding::CrLf,
            #[cfg(not(windows))]
            line_ending: LineEnding::Lf,
            had_bom: false,
            is_dirty: false,
        }
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, NoterError> {
        let path = path.as_ref();
        let mut bytes = Vec::new();
        File::open(path)?.read_to_end(&mut bytes)?;
        let doc = Self::from_bytes(&bytes, Some(path.to_path_buf()));
        Ok(doc)
    }

    pub fn from_bytes(bytes: &[u8], path: Option<PathBuf>) -> Self {
        let (line_ending, bom_len) = LineEnding::detect_from_bytes(bytes);
        let had_bom = bom_len > 0;
        
        let text = String::from_utf8_lossy(&bytes[bom_len..]);
        let rope = Rope::from_str(&text);

        Self {
            rope,
            path,
            line_ending,
            had_bom,
            is_dirty: false,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        if self.had_bom {
            bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }

        // We use rope.chars() and normalize line endings as we emit them.
        // For Ropey 1.x, iterating chunks or chars is fine.
        let mut in_cr = false;
        let le_bytes = self.line_ending.as_str().as_bytes();
        
        for chunk in self.rope.chunks() {
            for c in chunk.chars() {
                if c == '\r' {
                    in_cr = true;
                } else if c == '\n' {
                    bytes.extend_from_slice(le_bytes);
                    in_cr = false;
                } else {
                    if in_cr {
                        bytes.extend_from_slice(le_bytes);
                        in_cr = false;
                    }
                    let mut b = [0; 4];
                    bytes.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
                }
            }
        }
        
        if in_cr {
            bytes.extend_from_slice(le_bytes);
        }

        bytes
    }

    pub fn save_atomic(&mut self) -> Result<(), NoterError> {
        let path = self.path.as_ref().ok_or(NoterError::NoPath)?;
        
        // Use a UUID or timestamp to avoid collisions
        let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));

        {
            let mut f = File::create(&tmp_path)?;
            let bytes = self.to_bytes();
            f.write_all(&bytes)?;
            f.sync_all()?; // S1 Guarantee: sync before rename
        }

        // Atomic rename
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            // Clean up the temp file if rename failed
            let _ = std::fs::remove_file(&tmp_path);
            return Err(NoterError::AtomicRenameFailed(e.to_string()));
        }

        self.is_dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_line_ending_detection() {
        let (le, bom) = LineEnding::detect_from_bytes(b"hello\nworld");
        assert_eq!(le, LineEnding::Lf);
        assert_eq!(bom, 0);

        let (le, bom) = LineEnding::detect_from_bytes(b"hello\r\nworld");
        assert_eq!(le, LineEnding::CrLf);
        assert_eq!(bom, 0);

        let (le, bom) = LineEnding::detect_from_bytes(b"\xEF\xBB\xBFhello\rworld");
        assert_eq!(le, LineEnding::Cr);
        assert_eq!(bom, 3);
    }

    #[test]
    fn test_roundtrip_preservation() {
        // S2 Guarantee: Line ending & BOM preservation
        let original = b"\xEF\xBB\xBFhello\r\nworld\r\n";
        let doc = Document::from_bytes(original, None);
        assert_eq!(doc.line_ending, LineEnding::CrLf);
        assert_eq!(doc.had_bom, true);
        
        let output = doc.to_bytes();
        assert_eq!(original.as_slice(), output.as_slice());
    }

    #[test]
    fn test_atomic_save() -> Result<(), NoterError> {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();
        
        let mut doc = Document::new();
        doc.path = Some(path.clone());
        doc.rope = Rope::from_str("Hello Atomic!");
        doc.is_dirty = true;
        
        doc.save_atomic()?;
        assert!(!doc.is_dirty);
        
        let mut saved = Vec::new();
        File::open(&path).unwrap().read_to_end(&mut saved).unwrap();
        let expected = doc.to_bytes();
        assert_eq!(saved, expected);
        
        Ok(())
    }
}
