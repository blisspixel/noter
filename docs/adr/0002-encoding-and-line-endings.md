# ADR-0002: Strict UTF-8 and non-normalizing line endings

**Status:** Accepted

**Date:** 2026-07-25

## Context

The prototype previously used lossy UTF-8 conversion and represented a file
with one line-ending enum even though mixed-EOL files exist. Silent replacement
characters or whole-file normalization violates the product's trust model.

Clipboard and Enter operations also need a deterministic insertion policy that
does not rewrite unrelated existing lines.

## Decision

v0.1 accepts only strict UTF-8, optionally prefixed by one UTF-8 BOM. Invalid
UTF-8 returns a typed error and leaves the source untouched. Any future lossy
import creates an untitled dirty document and requires Save As.

Keep existing LF, CRLF, and CR sequences in authoritative content. Classify a
document as newline-free, uniform, or mixed:

- newline-free files insert CRLF on Windows and LF elsewhere;
- uniform files insert their existing ending;
- mixed files insert the nearest preceding ending, then nearest following
  ending, then a deterministic dominant ending;
- dominant-ending ties use first occurrence;
- pasted logical newlines use the insertion policy at the insertion point.

An untouched document must serialize byte for byte. Only an explicit Convert
Line Endings command rewrites unrelated newlines. That command is one undo
transaction.

## Consequences

- The current `LineEnding` field must become a profile with counts and insertion
  policy.
- Round-trip and edit-locality property tests are mandatory.
- Status can accurately report Mixed rather than pretending one convention.
- Non-UTF-8 save encodings remain out of scope.

## Required evidence

- Golden fixtures covering BOM, no BOM, LF, CRLF, CR, mixed, empty, trailing
  newline, newline-only, Unicode, and invalid UTF-8.
- Property tests proving untouched byte round-trip.
- Property tests proving edits do not alter bytes outside their intended range.
- UI tests for explicit EOL conversion, status, error, and undo behavior.
