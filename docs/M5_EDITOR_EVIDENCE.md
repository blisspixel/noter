# M5 Editor Gate Inputs and Measurement Protocol

**Prepared:** 2026-10-11

**Status:** Reproducible input preparation; no editor performance or native
accessibility gate has passed through this record.

[Issue #51](https://github.com/blisspixel/noter/issues/51) tracks execution.
[ROADMAP.md](ROADMAP.md#one-week-feasibility-gate) owns the sequence, budgets,
and Go/Conditional go/No-go decision.
[DESIGN.md](DESIGN.md#92-m5-feasibility-gate) owns the architecture.

## Frozen inputs

Generate the full suite into a new empty directory:

```sh
python scripts/generate_m5_corpus.py --output .agent/m5-corpus-run --scale full
```

The seven profiles each have exact 1 MiB, 8 MiB, and 50 MiB variants, totaling
413 MiB in 21 files. The small preparation check uses `--scale smoke` and creates
seven 64 KiB inputs. File names record their actual size in KiB. Generation
refuses a nonempty output directory and exclusively creates each file.

| Profile | Purpose |
| --- | --- |
| `prose` | Ordinary English drafting and wrapped navigation |
| `short-lines` | Many hard lines, text-run count, and offscreen navigation |
| `long-line` | One hard line, horizontal scrolling, and bounded shaping |
| `mixed-eol` | Preserved CR, LF, and CRLF boundaries and accessible positions |
| `unicode-edge` | Combining sequence, wide characters, non-BMP emoji, and ZWJ sequence |
| `bidi-edge` | Mixed direction, digits, isolates, and caret/selection mapping |
| `markdown` | Source-backed headings, emphasis, lists, code, and local links |

Routine visual captures use `prose` and `markdown`. The two edge profiles are
explicit targeted tests; they do not replace ordinary English visual review.
These synthetic profiles are repeatable stress inputs, not a representative
sample of every user's documents or a Markdown conformance corpus.

[m5-corpus-v1.json](evidence/m5-corpus-v1.json) freezes every file's size and
SHA-256 plus the ordered-manifest digest:

```text
5ec139c17e08b577995cb83ed9968aa64d5ad8ad1cec9f51c123d97823b47ab9
```

Each file contains exactly one early, middle, and late literal-search marker,
with byte offsets in the manifest. The generator reuses the existing M1
exclusive repeated-file writer, marker strings, and manifest digest function.
The M1 corpus and its historical hashes remain unchanged. Regression tests
regenerate the full suite and compare it with the frozen manifest, then check
UTF-8, exact search offsets, English visual profiles, EOL boundaries, the
newline-free long line, and short-line lengths in the smoke suite. Changing
inputs requires a new versioned reference and explicit review.

## Measurement receipt

The next runner must record raw samples and the following fields together.
Generating inputs alone provides none of these observations.

| Observation | Required context and result |
| --- | --- |
| Provenance | Exact candidate commit and binary hash; command; release profile; locked dependency versions; clean/dirty source state |
| Environment | CPU, RAM, GPU/driver, OS/build, compositor/backend, monitor refresh, physical/logical scale, window dimensions, fonts, and power mode |
| Corpus | Generator version, scale, manifest digest, individual file hash, actual bytes, wrap mode, and measured view |
| Sampling | Warmup count, at least 30 raw samples for any percentile, timing unit, percentile method, failures/timeouts, and process/cache state |
| Launch | Process start to first editable painted frame; distinguish OS cache state and successful editing from file-size refusal |
| Input and scroll | Input event to presented paint, warm-scroll frame times, long-line horizontal scroll, and separately measured layout/worker time |
| Resource use | Peak and steady RSS, allocation/copy counts, cache limits, accessibility node/update counts, and accessibility inactive/active runs |
| Search | Early/middle/late marker and absent query, first-result latency, selected source byte range, reveal, cancellation, and stale-result rejection |
| Native semantics | Full-document read-all and offscreen native range select/reveal, edit actions, events, directional selection, IME preedit/commit/cancel, and candidate placement |

Use the existing M1 percentile implementation when reporting nearest-rank
percentiles. Include every sample and failed attempt; state any discarded
warmups before measurement. Core edit timing, screenshot timing, and
application-presented paint timing measure different things and must have
separate names and claims.

## Execution boundaries

1. Baseline the existing adapter on 1 MiB and 8 MiB inputs. Its normal 50 MiB
   refusal is a limit check, not a successful launch measurement. Do not bypass
   the production size guard to collect an apparent performance pass.
2. Use an isolated vertical slice for 50 MiB measurements. Keep the existing
   core as the only authority for edits, history, save, and recovery. Compare
   coordinate mapping and edit behavior against the reference model before
   evaluating frame budgets.
3. Measure the same inputs, hardware, fonts, and native semantics for each layout
   candidate. Pin candidate dependencies and inspect their features, licenses,
   advisories, binary size, and network capability before acceptance.
4. Require real NVDA, VoiceOver, Orca, and IME observations under the roadmap
   matrix. Headless AccessKit tree tests cannot establish native interoperability.
5. Keep M4-H1 ahead of expanded UI limits and M6. Preparation can run while native
   environments are pending, but no successful corpus generation or research
   result changes the current 8 MiB support claim.

The next deliverable is a measured current-adapter baseline and a bounded
semantic/layout prototype with raw receipts. A renderer or dependency switch
is a later evidence-backed decision; this input suite does not accept one.
