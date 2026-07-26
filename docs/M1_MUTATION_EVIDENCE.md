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
  --exclude-re '(is_final_link|reconcile_existing_failure|replacement_backup_path|is_documented_partial_replacement)'

Windows full scope:
cargo mutants -vV --in-place --colors never
```

The [cargo-mutants CI guidance](https://mutants.rs/ci.html) recommends
`--in-place` for a disposable CI checkout. The tool documents that
[`--in-place` cannot be combined with `--jobs`](https://mutants.rs/in-place.html),
so each CI gate runs serially and uploads `mutants.out` even on failure. The
Linux job covers 316 common mutants. The Windows job covers all 341 mutants,
including four functions containing Windows-only decisions that cannot be
executed on Linux. The union therefore retains the complete configured scope
without misclassifying inactive platform branches as survivors. Incremental
compilation is enabled explicitly for the mutation steps because the cache
action disables it by default. The installer action and cargo-mutants version
are both pinned, checksums stay enabled, and fallback installation is disabled.

GUI code is not included to inflate the trust-kernel score. Application and UI
behavior require semantic UI tests, targeted state-model tests, limited visual
snapshots, and manual accessibility verification in later milestones.

## Results

| Campaign | Total | Caught | Unviable | Missed | Timed out | Duration |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Initial full scope | 380 | 228 | 111 | 38 | 3 | 10 minutes |
| Final full scope | 341 | 230 | 111 | 0 | 0 | 9.62 minutes |
| First hosted Linux full scope | 341 | 220 | 111 | 10 | 0 | 33.33 minutes |

`Unviable` means the mutation did not compile. It is distinct from a survivor.
The final run has no missed mutation and no timeout. The lower final mutant count
reflects removal of redundant branches and mutation-prone loop bookkeeping, not
an excluded source path.

The first hosted Linux run at commit `958cf2d` is intentionally retained as
negative evidence in
[GitHub Actions run 30183095388](https://github.com/blisspixel/noter/actions/runs/30183095388).
All 10 Linux survivors were operations inside Windows-only `cfg` branches. The
run exposed that a single-platform mutation gate cannot interpret inactive
platform code correctly and that the cache action's disabled incremental builds
made the serial campaign take 33.33 minutes. It directly caused the paired
Linux-common and Windows-full gate described above.

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
- cleanup and durability warnings were not tested in every combination.

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

The exact-commit paired CI mutation results will be linked here after both gates
complete successfully.
