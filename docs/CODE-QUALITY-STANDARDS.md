# Code Quality Standards

**Reviewed:** 2026-07-30

**Status:** Repository merge contract

These standards are the merge contract for Noter. They apply to production code, tests, documentation, scripts, platform adapters, and CI. A roadmap checkbox cannot weaken them.

## 1. Correctness before features

- Every behavior change starts from an explicit requirement, invariant, failure mode, or reproduced defect.
- Safety properties are enforced in code and tests. A comment, prompt, or roadmap note is not a control.
- Error paths preserve user data and distinguish not committed, committed with a warning, conflict, and unknown commit state.
- Platform behavior is not assumed portable. Windows, macOS, and Linux differences live behind small typed adapters and receive native evidence.
- The repository must contain no unfinished `TODO`, placeholder branch, commented-out implementation, or unreachable ghost feature in a shipped path.

## 2. Simple ownership and boundaries

- `src/core/` owns text, document, revision, observation, and save policy without GUI dependencies.
- `src/app.rs` adapts user intent to the core. It does not duplicate trust-kernel decisions.
- `crates/noter-platform/` is the only home for narrow operating-system primitives and justified unsafe code.
- Domain logic has one authoritative implementation. Tiny local repetition is acceptable when an abstraction would make behavior harder to see.
- New abstractions must remove a present source of complexity. Speculative frameworks are not accepted.

## 3. Rust and unsafe code

- The application crate forbids unsafe code.
- Each unsafe platform block documents its pointer, buffer, handle, lifetime, initialization, and return-value obligations immediately beside the operation.
- Native wrappers expose safe, typed results and do not leak raw handles or platform error conventions into the core.
- Integer conversions that can fail are checked. Revision and size arithmetic never wrap silently.
- Panics are limited to tests and process bootstrap conditions that cannot be recovered inside the application contract.

## 4. Filesystem and data safety

- Reads and writes reject unsupported links, special entries, unstable observations, and unsupported resource sizes before destructive effects.
- Existing-file saves compare identity, content fingerprint, length, link count, and change evidence at every defined protocol boundary.
- New-file installation must be exclusive. Existing-file replacement must preserve any raced external revision or return a non-success state that keeps recoverable artifacts.
- Temporary names are unpredictable and exclusively created. Staged bytes are inaccessible to unrelated principals while live.
- Errors and logs do not expose document content, complete paths, credentials, or recovery payloads.
- A successful return is not enough for a durability claim. File and containing-directory persistence outcomes remain explicit.

## 5. Tests and evidence

Every change receives the smallest set of tests that would have failed before the change:

- unit tests for pure decisions and exact boundaries;
- golden tests for byte fidelity;
- property tests for broad input classes and invariants;
- fault-injection tests for state-machine outcomes;
- native fixtures for operating-system claims;
- mutation tests for safety-critical branches;
- benchmarks for performance claims.

The trust kernel must maintain at least 90 percent line coverage. New application behavior must maintain at least 80 percent meaningful coverage even when UI code is excluded from the trust-kernel percentage. Coverage is a floor, not a substitute for failure-path assertions. Mutation campaigns must finish with zero missed and zero timed-out compiling mutants in their declared scope.

## 6. Required automated gates

The following gates must pass on the exact commit proposed for merge:

```text
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
RUSTDOCFLAGS="-D warnings" cargo doc --locked --workspace --no-deps
cargo audit --deny warnings
cargo deny --locked check
ruff check scripts
ruff format --check scripts
python -m unittest discover -s scripts -p 'test_*.py'
cargo llvm-cov --locked --all-targets --all-features --workspace --fail-under-lines 80
cargo llvm-cov --locked --all-targets --all-features --workspace --ignore-filename-regex 'src[/\\](app|bounded_text_input|editor_settings|find_ui|go_to_line_ui|idle_screen|main|markdown_ui|theme)\.rs$' --fail-under-lines 90
```

The excluded binary modules are immediate-mode GUI, presentation, and local
preference adapters. They remain inside the 80 percent whole-workspace gate.
They are excluded only from the separate 90 percent UI-independent trust-kernel
percentage and additionally require state tests, native-render smoke tests,
deterministic screenshot checks, semantic UI automation, and the manual
platform matrix. CI also runs the platform matrix, documentation-link checks,
and the declared trust-kernel mutation matrix. A local pass does not replace
exact-commit CI evidence.

## 7. Dependencies and supply chain

- `Cargo.lock` is committed and every automated Cargo invocation that supports it uses `--locked`.
- Runtime dependencies require a design rationale, reverse-dependency inspection, advisory check, and measured binary-size impact at the relevant milestone gate.
- Git dependencies, unpinned CI actions, unexpected build scripts, default feature expansion, and new native libraries require explicit review.
- CI receives the least token permission needed and uses immutable action revisions.

## 8. Documentation and user truth

- README, requirements, design, roadmap, ADRs, changelog, UI labels, and measured evidence must agree on what works now.
- Claims include the command, platform, commit, artifact, or fixture that supports them. Unknowns and limits are stated beside the claim.
- Comments explain invariants, operating-system contracts, and non-obvious tradeoffs. They do not advertise quality or restate syntax.
- User-facing controls either work, are absent, or clearly explain why they are unavailable. Silent inert controls are defects.
- Completed roadmap items are marked only with same-commit evidence. Future scope remains in the roadmap, not in commented code.

## 9. Review and release

- The implementer performs a focused self-review after all automated gates pass.
- Two independent checker passes examine correctness, user-data safety, security and privacy, performance, tests, maintainability, and documentation truth.
- Every category in the active quality rubric must score at least 4 out of 5 with cited evidence. A score of 5 must name evidence beyond ordinary compliance.
- Release or milestone evidence records the exact commit, commands, results, coverage, mutation outcome, platform limitations, and unresolved risks.
- If a change cannot be isolated from unrelated working-tree changes, it is not committed until the scope can be separated safely.
