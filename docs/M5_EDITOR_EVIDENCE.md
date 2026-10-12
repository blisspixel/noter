# M5 Editor Gate Inputs and Measurement Protocol

**Prepared:** 2026-10-11

**Status:** Reproducible inputs and a current-adapter CPU measurement harness;
no complete editor performance or native accessibility gate has passed.

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

## Current-adapter CPU runner

From a clean checkout, generate the frozen full corpus as above, then run:

```sh
python scripts/run_m5_cpu_baseline.py --corpus .agent/m5-corpus-run --output .agent/m5-cpu.json
```

The default matrix is ordinary English prose at 1 MiB and 8 MiB, wrapped and
unwrapped, with AccessKit inactive and active: eight fresh test processes. Each
process records one first editor-frame CPU duration, five discarded warmup
frames per idle/edit phase, and 30 ordered raw samples per phase. Select other
frozen profiles with `--profiles`, sizes in bytes with `--sizes`, and bounded
sample, warmup, and per-case timeout counts with their corresponding options.
The runner accepts no 50 MiB case and never raises the production editor limit.

The harness exists only under `cfg(test)` in `src/app/measurement.rs`. Cargo
builds the actual binary tests with `--locked --release`; the runner selects
exactly one optimized binary-test artifact, records its SHA-256, and requires
clean, unchanged source and matching frozen input bytes. Ordinary tests exercise
the same harness on small English and mixed-EOL/Unicode/control fixtures.

Each measured frame calls the actual `NoterApp::show_editor` path through egui,
including source/display projection, TextEdit layout and event handling,
source-to-core transaction creation, history recording, font observation of the
inserted ASCII character, and recovery scheduling in an isolated private test
store. Idle frames reuse current state. Edit frames replace the first ASCII byte
of the middle search marker, alternating `x` and `y` without growing the file.
Selection restoration occurs in a separate untimed frame before each input.
Outside the timer, the harness requires one new revision, the exact caret,
unchanged length, rope agreement, and available shared Undo. At phase end it
checks every non-target source byte and exact Undo to the preceding revision's
content and directional selection. AccessKit node counts describe each emitted update, not retained
tree size or real screen-reader cost.

The timer records monotonic wall elapsed around CPU-side `Context::run_ui`
work, not process or thread CPU time, ending before output inspection and
destruction. File load, application/context/store construction, selection setup,
assertions, tessellation, GPU work, output destruction,
native event delivery, compositor presentation, system fallback-font discovery,
and the rest of the application frame are excluded. First editor-frame CPU time
is a single observation, not a launch percentile. The fixed headless viewport
is 1200 by 760 logical points at scale 1 with bundled fonts. Clock-only raw
samples expose timer overhead; zero can mean clock quantization. Instrumentation
is not subtracted from durations, and no outliers are removed.
Each case uses a fresh worker with warmed in-process phases. OS caches are
uncontrolled; this runner makes no cold-cache observation.

Recovery work reached synchronously through `on_edited` remains inside the timer.
When due, this can include outcome polling, snapshot serialization, worker
dispatch, and inline persistence fallback. Background completion is not a
measured endpoint and can overlap later samples. This is an adapter diagnostic,
not an isolated layout microbenchmark.

Receipts retain source/tree, locked dependency hash, binary hash, build command,
host CPU/RAM/OS/toolchain context, each corpus hash, raw timings, revision
sequences, update counts, and the existing M1 nearest-rank summaries. A failed
or timed-out worker is retained as a failed attempt and stops sampling; it cannot
become a zero-duration success. Output is exclusive and never overwrites an
earlier receipt. Runner regressions cover artifact/input validation, wrong or
missing timings, inconsistent revisions and accessibility, timeout propagation,
environment restoration, and retained failures.

This establishes a repeatable CPU diagnostic subset for issue #51. Native
presentation, active screen-reader and IME matrices, memory/allocation evidence,
scroll/search measurements, the 50 MiB slice, and the final gate decision remain.

## Recorded Windows baseline

The [2026-10-12 receipt](evidence/m5-cpu-windows-2026-10-12.json) contains all
eight completed cases and every raw sample. It measures clean commit
`00edcdccc9b82e40d6bb9216bd87f6004ab44f50`, tree
`9e61171ced33e1430b9fc1a223b3df518530cd6d`, with binary SHA-256
`31b3db9219bfe8aad75110d20d5a041ba5af7a43c264d42f67c0884ae1901158`.
The receipt SHA-256 is
`461e862062ac6f3632a726547323039b6dc653cb664e1a936be18365b99d0a9d`.
Its timestamp records sampling start, not completion. Later documentation and
accessibility tests do not retroactively change the measured source identity.

Command, from that clean checkout:

```sh
python scripts/run_m5_cpu_baseline.py --corpus C:/GitHub/noter/.agent/m5-corpus-v1 --output C:/GitHub/noter/.agent/m5-cpu-windows.json
```

Host: Windows 11 build 10.0.26200, AMD Ryzen 9 5950X, 32 logical processors,
64 GiB RAM, local NTFS, Rust/Cargo 1.97.1, Python 3.14.7. The headless viewport,
bundled fonts, fresh-worker/warmed-phase protocol, and timing exclusions above
apply. GPU, monitor refresh, native accessibility, and power mode were not
measured or controlled; these numbers establish no native paint budget.

| MiB | Wrap | AccessKit | First frame ms | Idle p95 ms | Edit p95 ms | Edit p99 ms | Maximum edit update nodes |
| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |
| 1 | Off | Off | 9.489 | 2.411 | 15.076 | 17.152 | 0 |
| 1 | Off | On | 45.532 | 39.404 | 67.669 | 68.506 | 17417 |
| 1 | On | Off | 9.443 | 2.704 | 12.276 | 12.535 | 0 |
| 1 | On | On | 42.295 | 39.490 | 98.020 | 100.216 | 17417 |
| 8 | Off | Off | 46.509 | 19.200 | 148.691 | 148.774 | 0 |
| 8 | Off | On | 356.565 | 476.362 | 797.570 | 1067.594 | 139273 |
| 8 | On | Off | 53.359 | 21.232 | 199.595 | 202.670 | 0 |
| 8 | On | On | 404.558 | 569.479 | 673.534 | 701.639 | 139273 |

Whole-document accessibility updates are a measured bottleneck to investigate
in the M5 semantic/layout prototype. Do not disable accessibility or infer a
specific fix from timing correlation alone. Compare update count, exact text,
selection, and native range operations on the same frozen inputs.

## Accessible text fidelity

Active headless tests reconstruct the named editor's ordered child `TextRun`
values and compare them with the canonical LF source display projection. They
cover wrapped English, mixed line endings, combining/wide/non-BMP characters,
and the frozen bidi-edge line, while asserting unchanged document bytes.
These bundled-font tests pass and remain active in ordinary CI.

[Issue #60](https://github.com/blisspixel/noter/issues/60) separately reports
native AT-SPI RTL duplication (74 source characters versus 83 exposed). This
is not fixed by the bundled-font checks. A separate local-font fixture now
reproduces duplication in the actual editor's AccessKit TextRuns: 52 expected
characters become 59 with the identified Arial fallback. The
[negative receipt](evidence/m5-rtl-font-2026-10-12.json) records exact source,
binary, font hashes, environment, command, and the retained equality failure.
This headless observation supports a font/shaping cause; it is not a native
AT-SPI interoperability observation or a repair.

The local-font test is explicitly ignored by ordinary CI because its identified
system font is not redistributed. It requires `NOTER_M5_RTL_FONT`, reads through
the existing bounded font loader, validates coverage of every tested RTL
character, and keeps source equality as the repair criterion. This narrow
environment-dependent exception applies only to the additional reproduction;
the bundled-font RTL test stays active. Run it explicitly on Windows:

```powershell
$env:NOTER_M5_RTL_FONT = 'C:/Windows/Fonts/arial.ttf'
cargo test --locked --bin noter app::measurement::local_font_rtl_accessible_text_runs_match_source_projection --all-features -- --exact --ignored --nocapture
Remove-Item Env:NOTER_M5_RTL_FONT
```

With the receipt's font, exit 101 and the 52-versus-59 mismatch are retained
negative evidence. A pass with another font does not establish a repair.
Any proposed repair must pass this same identified fixture and native text,
caret, selection, and editing checks before the exception is removed.
Track the upstream
[RTL shaping defect](https://github.com/emilk/egui/issues/8576) and proposed
[cluster correction](https://github.com/emilk/egui/pull/8698); these remain open
as of 2026-10-12, and no dependency switch or patch is accepted here.
