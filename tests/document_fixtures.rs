//! Golden document decoding and byte-fidelity cases.

use noter::core::document::Document;
use noter::core::line_endings::{LineEnding, LineEndingCounts, LineEndingProfile};
use noter::core::text_format::Bom;
use noter::error::NoterError;

const CASES: &str = include_str!("fixtures/document_cases.txt");

fn decode_hex(encoded: &str) -> Vec<u8> {
    assert!(encoded.len().is_multiple_of(2), "fixture hex must be even");
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).expect("fixture hex must be ASCII");
            u8::from_str_radix(pair, 16).expect("fixture hex must contain hexadecimal bytes")
        })
        .collect()
}

fn ending(token: &str) -> LineEnding {
    match token {
        "lf" => LineEnding::Lf,
        "crlf" => LineEnding::CrLf,
        "cr" => LineEnding::Cr,
        "platform" => LineEnding::platform_default(),
        _ => panic!("unknown line-ending token: {token}"),
    }
}

fn expected_profile(fields: &[&str]) -> LineEndingProfile {
    let counts = LineEndingCounts {
        lf: fields[5].parse().expect("LF count must be numeric"),
        crlf: fields[6].parse().expect("CRLF count must be numeric"),
        cr: fields[7].parse().expect("CR count must be numeric"),
    };
    let fallback = ending(fields[8]);

    match fields[4] {
        "none" => LineEndingProfile::None {
            insertion: fallback,
        },
        "uniform" => LineEndingProfile::Uniform {
            ending: fallback,
            count: counts.total(),
        },
        "mixed" => LineEndingProfile::Mixed {
            counts,
            insertion: fallback,
        },
        profile => panic!("unknown profile token: {profile}"),
    }
}

#[test]
fn golden_document_matrix_is_exact() {
    for line in CASES.lines().filter(|line| !line.starts_with('#')) {
        let fields: Vec<_> = line.split('|').collect();
        assert_eq!(fields.len(), 9, "malformed fixture row: {line}");
        let name = fields[0];
        let bytes = decode_hex(fields[1]);

        if fields[2] == "invalid" {
            let error = Document::from_bytes(&bytes, None)
                .err()
                .unwrap_or_else(|| panic!("{name}: invalid UTF-8 was accepted"));
            assert!(
                matches!(error, NoterError::InvalidUtf8(_)),
                "{name}: wrong error: {error}"
            );
            continue;
        }

        let document = Document::from_bytes(&bytes, None)
            .unwrap_or_else(|error| panic!("{name}: valid fixture failed: {error}"));
        let expected_bom = if fields[3] == "true" {
            Bom::Utf8
        } else {
            Bom::Absent
        };
        assert_eq!(document.bom, expected_bom, "{name}: BOM");
        assert_eq!(
            document.line_endings,
            expected_profile(&fields),
            "{name}: profile"
        );
        assert_eq!(document.to_bytes(), bytes, "{name}: byte round-trip");
    }
}
