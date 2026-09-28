# M3 Editing Evidence

**Recorded:** 2026-07-30

**Scope:** the UI-independent transaction, Undo, literal search, logical-line
navigation, and destructive-lifecycle decision core

This record supports the implemented M3 editing foundation. It does not mark
M3 complete. Cross-platform manual keyboard evidence and remaining milestone
items remain open in the [roadmap](ROADMAP.md).

## Subject revision

- Commit: `8df294d09b1fa1aa150e5d7a2b22ec0a9fca56a9`
- Platform: Microsoft Windows NT 10.0.26200.0
- Rust: `rustc 1.97.1 (8bab26f4f 2026-07-14)`
- Cargo: `cargo 1.97.1 (c980f4866 2026-06-30)`
- Mutation runner: `cargo-mutants 27.1.0`
- Coverage runner: `cargo-llvm-cov 0.8.7`

The subject revision is a clean committed source tree. Later changes do not
inherit this evidence by assertion. In particular, the current tree factors
the `Document::replace_text` preflight limit into a private test seam and adds
an exact boundary regression. This record and every count below remain scoped
to the named subject revision; current-tree mutation results belong to the
separate hosted CI gate.

## Mutation campaign

The settled campaign used the following command:

```text
cargo mutants --no-config --workspace --all-features -C --locked --baseline run --no-shuffle --colors never --minimum-test-timeout 20 --jobs 1 -o .agent\m3-mutation-8df294d -f src/core/edit.rs -f src/core/undo.rs -f src/core/search.rs -f src/core/navigation.rs -f src/core/lifecycle.rs
```

| Outcome | Count |
| --- | ---: |
| Caught by tests | 216 |
| Missed | 0 |
| Timed out | 0 |
| Compiler-unviable | 40 |
| Total generated | 256 |

The unmutated baseline built in 74 seconds and tested in 8 seconds. The complete
campaign finished in 27 minutes. Every one of the 40 unviable outcomes contains
a Rust compiler diagnostic. The repository's mutation-artifact validator
reported no recognized tool, compiler, linker, process, or storage failure
misclassified as an ordinary unviable mutation.

## First-pass findings and correction

The first full campaign against commit `c663c85` generated 283 mutations: 228
were caught, 41 were compiler-unviable, 13 survived, and one timed out. That
result was rejected rather than presented as positive evidence.

The correction made four narrow changes:

- removed a lifecycle branch whose false-guard mutation was behaviorally
  equivalent to the following transition;
- asserted the public search ordinal and match-count accessors independently;
- expressed exact resource ceilings as canonical literal byte counts instead
  of mutation-equivalent arithmetic; and
- replaced mutable byte-offset progress in logical-line navigation with a
  structurally terminating iterator.

The settled campaign above reran the complete declared scope after those
changes. It was not a survivor-only rerun.

## Tests and coverage

The subject revision passes 376 Rust tests with:

```text
cargo test --locked --workspace --all-features
```

Windows-local line coverage measured with the repository's declared commands
is:

| Scope | Covered lines | Total lines | Coverage |
| --- | ---: | ---: | ---: |
| Whole workspace | 13,506 | 14,648 | 92.20% |
| UI-independent trust kernel | 6,843 | 7,160 | 95.57% |

The trust-kernel result uses the UI-adapter exclusions declared for the subject
revision. Coverage remains a supporting measure; the reference-model properties
and mutation result carry the stronger decision-path evidence for this scope.

## Limits of this record

- This is Windows-local evidence, not the cross-platform M3 keyboard matrix.
- The mutation scope is the five named core modules, not the GUI adapters,
  platform I/O crate, Markdown renderer, or complete repository.
- It does not establish accessibility, IME, visual behavior, installed-product
  behavior, long-session memory bounds, or release readiness.
- M3 remains In Progress until every roadmap exit criterion has same-commit
  evidence.

## 2026-09-28 UTC backward-word memory follow-up

This is separate Windows-local evidence for a navigation fix based on
`49eaab40fad1b78cc58f0c5a14c0c6190e42c2de`. The candidate
`src/core/navigation.rs` has Git blob
`d43ea14f7c8e4a1c76f8e02d3404bf26874ece8e`. Hosted exact-head CI and
non-Windows peak-memory evidence are pending.

`python scripts/navigation_memory_check.py` builds the real navigation fixture,
moves backward through a 16 MiB ASCII prefix with wide and bidi-control
characters at the end, and checks the held process's peak working set against a
160 MiB ceiling. On Windows, the prior vector implementation failed at
289,267,712 bytes. The candidate passed at 37,576,704 bytes. Restoring the
candidate after that negative check made the same test pass again. A separate
allocation-count probe measured 234,881,136 requested allocation bytes for the
old 8 MiB backward move, zero for the candidate at 8 MiB, and zero for the
candidate at 64 MiB. Single-run timing was not used as an acceptance threshold.

The candidate passed the full Windows workspace tests, 206 repository script
tests with 11 skips, Clippy, Rustdoc, formatting, Ruff, documentation links,
and release-config validation. Local line coverage was 93.36 percent for the
whole workspace and 92.63 percent with the declared UI-adapter exclusions. The
memory regression runs on Linux and Windows in the cached CI test job. macOS
still runs the Rust fixture's Unicode offset checks, but its current baseline
can sample only held resident memory, not the transient peak required for this
assertion.

A focused Windows-local `cargo mutants --regex 'move_by_word' -j 4` run on
navigation source blob `d43ea14f7c8e4a1c76f8e02d3404bf26874ece8e`
completed 17 of 17 mutants as caught, with zero missed, timed out, or
unviable. The full workspace test baseline passed. This is evidence for the
word-movement mutation subset, not the complete CI mutation campaign.
