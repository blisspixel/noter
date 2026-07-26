# M1 Mutation Evidence

**Executed:** 2026-07-25

**Scope:** `src/core/*.rs`, the document and durable I/O trust kernel

**Reference environment:** Windows 11 Pro build 26200, Rust 1.97.1
(`8bab26f4f`), cargo-mutants 27.1.0

This report records the M1 mutation campaign. Mutation testing complements line
coverage by deliberately changing decisions in product code and requiring the
test suite to reject each compiling behavioral change.

## Reproducible configuration

The checked-in [`.cargo/mutants.toml`](../.cargo/mutants.toml) limits mutation
to the trust kernel, enables all features and the workspace test suite, and
passes `--locked` to Cargo. The local reference command is:

```text
cargo mutants --jobs 4 --colors never
```

The CI commands are intentionally different:

```text
Linux common scope:
cargo mutants -vV --in-place --colors never \
  --exclude-re '(is_final_link|reconcile_existing_failure|replacement_backup_path|is_documented_partial_replacement|finalize_unexpected_displaced_destination)'

Windows-applicable scope:
cargo mutants -vV --in-place --colors never \
  --exclude-re '(metadata_source_status|finalize_unix_displaced_destination)'
```

The [cargo-mutants CI guidance](https://mutants.rs/ci.html) recommends
`--in-place` for a disposable CI checkout. The tool documents that
[`--in-place` cannot be combined with `--jobs`](https://mutants.rs/in-place.html),
so each CI gate runs serially and uploads `mutants.out` even on failure. The
current Linux job covers 394 applicable mutants, and the Windows job covers
418 applicable mutants. Their union covers all 423 configured mutants with no
missing entry. Each
command excludes only decisions compiled exclusively for the other platform,
so inactive `cfg` branches are not misclassified as survivors. Incremental
compilation is enabled explicitly for the mutation steps because the cache
action disables it by default. The installer action and cargo-mutants version
are both pinned, checksums stay enabled, and fallback installation is disabled.
CI also parses every unviable build log and rejects recognized infrastructure
failures such as linker invocation errors, compiler internal errors, storage or
process exhaustion, and tool-lock contention. These failures require an
isolated rerun and cannot count as type-level compiler rejection.
The property harness uses a fixed seed and disables failure persistence. An
intentionally failing mutant therefore cannot write a regression file that
changes the test corpus seen by later mutants, and exact coverage commands see
the same 512 generated cases on every run.

GUI code is not included to inflate the trust-kernel score. Application and UI
behavior require semantic UI tests, targeted state-model tests, limited visual
snapshots, and manual accessibility verification in later milestones.

## Results

| Campaign | Total | Caught | Unviable | Missed | Timed out | Duration |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Initial full scope | 380 | 228 | 111 | 38 | 3 | 10 minutes |
| Final full scope | 341 | 230 | 111 | 0 | 0 | 9.62 minutes |
| First hosted Linux full scope | 341 | 220 | 111 | 10 | 0 | 33.33 minutes |
| Paired hosted Linux common scope | 316 | 210 | 106 | 0 | 0 | 18.45 minutes |
| Paired hosted Windows full scope | 341 | 230 | 111 | 0 | 0 | 35.43 minutes |
| Security-maintenance first run | 373 | 247 | 117 | 9 | 0 | 11 minutes |
| Security-maintenance repaired run | 369 | 252 | 117 | 0 | 0 | 11 minutes |
| Cleanup-redesign first Windows run | 394 | 253 | 129 | 12 | 0 | 24 minutes |
| Cleanup-redesign Windows-applicable run | 383 | 254 | 129 | 0 | 0 | 24 minutes |
| Checker-expanded Windows-applicable run | 418 | 265 | 149 | 4 | 0 | 28.48 minutes |
| Checker-expanded composite result | 418 | 270 | 148 | 0 | 0 | 31.04 minutes cumulative |

`Unviable` means the mutation did not compile. It is distinct from a survivor.
The current composite result has no missed mutation and no timeout. Earlier
lower mutant counts reflect removal of redundant branches and mutation-prone
loop bookkeeping, not an excluded source path.

The first hosted Linux run at commit `958cf2d` is intentionally retained as
negative evidence in
[GitHub Actions run 30183095388](https://github.com/blisspixel/noter/actions/runs/30183095388).
All 10 Linux survivors were operations inside Windows-only `cfg` branches. The
run exposed that a single-platform mutation gate cannot interpret inactive
platform code correctly and that the cache action's disabled incremental builds
made the serial campaign take 33.33 minutes. It directly caused the paired
Linux-common and Windows-full gate described above.

The corrected paired gate passed at commit
`3830cdd6e487a35bdd2adeecb3d45bb080ade114` in
[GitHub Actions run 30184163737](https://github.com/blisspixel/noter/actions/runs/30184163737).
The Linux-common job caught 210 of 316 mutants and classified 106 as unviable.
The Windows-full job caught 230 of 341 and classified 111 as unviable. Neither
job missed or timed out a mutant, and every other required CI job passed on the
same immutable commit.

The M1 security-maintenance pass added bounded file-size decisions and stronger
cleanup reconciliation. Its first local run found nine survivors at exact size
boundaries. The repair made the inclusive 64 MiB limit, a larger announced
length, limit arithmetic overflow, and read-growth sentinel independently
observable. Refactoring the shared bound calculation removed four redundant
mutation sites. The repeated 369-mutant campaign then completed with zero missed
and zero timed out.

The cleanup redesign replaced pathname deletion with Windows handle-bound
deletion and conservative Unix retention. Its first Windows run included
inactive Unix-only mutations and also exposed four retry-classification
survivors in the new cleanup observer. Removing a redundant retry branch and
adding exact missing-versus-invalid-path tests closed the applicable survivors.
The repeated Windows-applicable campaign classified all 383 mutations as 254
caught and 129 unviable, with zero missed and zero timed out. The five Unix-only
mutations remain in the paired Linux CI scope, including the post-exchange
metadata decision truth table and its native Unix fixture.

The checker-expanded scope added exact Save As expectation retention, typed
recovery artifacts, creation-time cleanup reporting, Windows staging handoff
verification, and associated decision accessors. The full 418-mutant Windows
run found four survivors: the retained-creation-artifact formatter, the
creation cleanup accessor, the durability-warning accessor, and the rule that a
successful parent barrier cannot upgrade a failed file barrier. Four additive,
focused tests made each behavior independently observable. An iterative rerun
caught three immediately. The fourth first encountered Windows linker error
`LNK1104` because a test executable was still locked; it was not accepted as a
semantic compiler rejection. A focused rerun in an isolated target directory
caught that mutation. Scanning the retained full-run logs with the new
infrastructure validator found another `LNK1104` result hidden among the 149
unviable cases. An isolated five-mutant rerun caught that exact
`IntendedContent::matches` equality mutation and its neighboring decisions. The
other generated variant at the creation cleanup accessor remained a genuine
type-level compiler rejection. Product source did not change between the full
run and focused reruns, so the combined local classification is 270 caught and
148 unviable.
The complete paired CI gate must still rerun the full 423-mutant union on one
immutable commit before this composite result becomes hosted exact-commit
evidence.

## Defects in the proof found by mutation

The first run identified test or structure gaps in safety-relevant decisions:

- stable reads did not independently prove every before/after and reopened-path
  comparison;
- line-ending scanning relied on mutable index progress, allowing mutations to
  turn a finite scan into a timeout;
- content, identity, file-kind, parent-path, random-name, and platform-token
  decisions lacked exact independent checks;
- partial replacement reconciliation and post-commit verification had
  insufficient truth-table coverage;
- cleanup races and cleanup error classification were not distinguished exactly;
- cleanup and durability warnings were not tested in every combination;
- creation-time retained-artifact reporting was not observable on Windows; and
- a successful parent barrier could conceal a file-barrier classification
  mutation.

The repair extracted small decision functions where a truth table is the clearest
specification, replaced index bookkeeping with a structurally bounded iterator,
and added exact tests for both sides of each safety decision. It also added real
filesystem tests for independent file identity and content, OS randomness,
replaced temporary paths, mismatched committed destinations, Windows change
tokens, and documented Windows partial-replacement classification.

These changes strengthened the implementation as well as the tests. The final
campaign was rerun across the complete configured scope after the last cleanup
race survivor was fixed.

## Interpretation and remaining limits

A clean mutation run is evidence that this mutation operator set cannot find an
uncaught behavioral change in the configured scope. It does not prove native
metadata preservation, crash durability, weak-filesystem behavior, GUI
semantics, or performance. Those remain separate M1 and later milestone gates in
[ROADMAP.md](ROADMAP.md) and [manual-test-matrix.md](manual-test-matrix.md).

The checker-expanded counts are composite local evidence until both current
paired jobs pass on one exact commit. The immutable `3830cdd` paired result
remains the verified CI baseline in the meantime.
