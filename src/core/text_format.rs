//! Explicit text encoding and byte-order-mark metadata.

/// The on-disk text encoding supported by Noter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Encoding {
    /// Strict Unicode Transformation Format, 8-bit form.
    Utf8,
}

impl Encoding {
    /// Returns a compact status-bar label.
    pub const fn status_label(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
        }
    }
}

/// The optional byte-order mark at the beginning of a UTF-8 file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bom {
    /// No byte-order mark was present.
    Absent,
    /// The exact UTF-8 byte-order mark was present.
    Utf8,
}

impl Bom {
    /// The exact UTF-8 byte-order-mark sequence.
    pub const UTF8_BYTES: &'static [u8; 3] = b"\xEF\xBB\xBF";

    /// Splits an optional leading UTF-8 BOM from the remaining bytes.
    pub fn split_utf8(bytes: &[u8]) -> (Self, &[u8]) {
        bytes
            .strip_prefix(Self::UTF8_BYTES)
            .map_or((Self::Absent, bytes), |content| (Self::Utf8, content))
    }

    /// Returns the bytes emitted before document content.
    pub const fn as_bytes(self) -> &'static [u8] {
        match self {
            Self::Absent => &[],
            Self::Utf8 => Self::UTF8_BYTES,
        }
    }

    /// Returns whether a byte-order mark is present.
    pub const fn is_present(self) -> bool {
        matches!(self, Self::Utf8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_utf8_format_labels_and_prefixes_are_exact() {
        assert_eq!(Encoding::Utf8.status_label(), "UTF-8");
        assert_eq!(Bom::Absent.as_bytes(), b"");
        assert_eq!(Bom::Utf8.as_bytes(), b"\xEF\xBB\xBF");
        assert!(!Bom::Absent.is_present());
        assert!(Bom::Utf8.is_present());
    }

    #[test]
    fn only_one_leading_utf8_bom_is_removed() {
        assert_eq!(
            Bom::split_utf8(b"plain"),
            (Bom::Absent, b"plain".as_slice())
        );
        assert_eq!(
            Bom::split_utf8(b"\xEF\xBB\xBFplain"),
            (Bom::Utf8, b"plain".as_slice())
        );
        assert_eq!(
            Bom::split_utf8(b"\xEF\xBB\xBF\xEF\xBB\xBF"),
            (Bom::Utf8, b"\xEF\xBB\xBF".as_slice())
        );
    }
}
