# M3 Editing Evidence

**Recorded:** 2026-07-30

**Scope:** the UI-independent transaction, Undo, literal search, logical-line
navigation, and destructive-lifecycle decision core

This record supports the implemented M3 editing foundation. It does not mark
M3 complete. Clipboard and navigation parity, Markdown document-wide
selection, long-session resource evidence, and cross-platform keyboard checks
remain open in the [roadmap](ROADMAP.md).

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
