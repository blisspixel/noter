# M1 Mutation Evidence

**Executed:** 2026-07-26

**Latest hosted continuation:** 2026-07-28

**Scope:** `src/core/*.rs` and `crates/noter-platform/src/*.rs`, the document,
durable I/O, and native adapter trust kernel

**Reference environment:** Windows 11 Pro build 26200, Rust 1.97.1
(`8bab26f4f`), cargo-mutants 27.1.0

This report records the M1 mutation campaign. Mutation testing complements line
coverage by deliberately changing decisions in product code and requiring the
test suite to reject each compiling behavioral change.

## Reproducible configuration

The checked-in [`.cargo/mutants.toml`](../.cargo/mutants.toml) limits mutation
to the trust kernel, enables all features and the workspace test suite, and
passes `--locked` to Cargo. An unfiltered single-platform run is useful only for
candidate discovery because it also lists inactive target-specific code. The
authoritative local Windows native-adapter command is target-filtered:

```text
$env:CARGO_INCREMENTAL='1'
$env:CARGO_TARGET_DIR='.agent/target-mutants-platform-windows'
cargo +1.97.1 mutants -vV --in-place --colors never --workspace -p noter-platform --exclude-re '(required_metadata|unix|linux|[Mm]acos)' -o .agent/mutants-platform-windows
```

The CI commands are intentionally different:

```text
Linux common scope:
cargo mutants -vV --in-place --colors never --workspace \
  --exclude-re '(is_final_link|reconcile_existing_failure|replacement_backup_path|is_documented_partial_replacement|finalize_unexpected_displaced_destination|TemporaryFile::discard|TemporaryFile::preserve_artifact|Drop for TemporaryFile|closed_temporary_matches_intended|remove_verified_backup|[Ww]indows|[Mm]acos)'

Windows-applicable scope:
cargo mutants -vV --in-place --colors never --workspace \
  --minimum-test-timeout 60 \
  --exclude-re '(required_metadata|metadata_source_status|post_exchange_source_facts_match|finalize_unix_displaced_destination|unix|linux|[Mm]acos)'

macOS native-adapter scope:
cargo mutants -vV --in-place --colors never --workspace \
  -p noter-platform --re '[Mm]acos'
```

The `[Mm]acos` form is intentional because cargo-mutants matches generated
descriptions that can contain both lower-case function names and PascalCase type
names.

The [cargo-mutants CI guidance](https://mutants.rs/ci.html) recommends
`--in-place` for a disposable CI checkout. The tool documents that
[`--in-place` cannot be combined with `--jobs`](https://mutants.rs/in-place.html),
so each CI gate runs serially and uploads `mutants.out` even on failure. The
current Linux job covers 617 candidates, the Windows job covers 557, and the
macOS adapter job covers 49 macOS-specific candidates. The scopes overlap where
the common runner assignments require it;
deduplicating exact mutation descriptions produces all 741 configured
supported-platform candidates with no missing entry. The filters are runner
assignments, not a claim that every exclusion is inactive. Linux assigns several
active cross-platform decisions with Windows-specific branches to the Windows
runner, Windows excludes Unix snapshot APIs absent from its build, and macOS
selects descriptions for its native implementation. Generic Unix candidates
without target-specific expressions are owned by the Linux job. Platform-only
predicates use platform-specific function names so their generated descriptions
remain assignable to the active runner. This prevents inactive `cfg` code from
being misclassified as a survivor while retaining full set-union coverage.
Incremental
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
| Independent-review-expanded Windows-applicable run | 418 | 265 | 149 | 4 | 0 | 28.48 minutes |
| Independent-review-expanded composite result | 418 | 270 | 148 | 0 | 0 | 31.04 minutes cumulative |
| Windows native-adapter run before descriptor Drop proof | 57 | 39 | 18 | 0 | 0 | 10.68 minutes |
| Last completed pre-creation-hardening Windows native-adapter run | 58 | 40 | 18 | 0 | 0 | 3.25 minutes |
| Hosted run 30213398323, Linux scope before current repair | 638 | 423 | 181 | 32 | 2 | 24.12 minutes job wall time |
| Hosted run 30213398323, Windows scope before current repair | 559 | 381 | 176 | 0 | 2 | 44.30 minutes job wall time |
| Hosted run 30213398323, macOS scope | 49 | 43 | 6 | 0 | 0 | 8.90 minutes job wall time |
| Rejected run 30219731527, Linux report | 617 | 438 | 179 | 0 | 0 | 21.77 minutes job wall time |
| Rejected run 30219731527, Windows report | 557 | 381 | 176 | 0 | 0 | 41.48 minutes job wall time |
| Rejected run 30219731527, macOS raw report | 49 | 42 | 7 | 0 | 0 | 5.82 minutes job wall time |
| Exact run 30221793209, Linux scope | 617 | 438 | 179 | 0 | 0 | 21.75 minutes job wall time |
| Exact run 30221793209, Windows scope | 557 | 381 | 176 | 0 | 0 | 41.05 minutes job wall time |
| Exact run 30221793209, macOS scope | 49 | 43 | 6 | 0 | 0 | 5.40 minutes job wall time |

`Unviable` means the mutation did not compile. It is distinct from a survivor.
The completed local Windows core and predecessor adapter results have no missed
mutation and no timeout. The current source enumerates 66 Windows native-adapter
candidates, all included in the clean exact Windows scope above. Earlier lower
mutant counts reflect removal of redundant branches and mutation-prone loop
bookkeeping, not an excluded source path.

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

Independent review expanded the scope with exact Save As expectation retention, typed
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
The Unix metadata repair captures an immutable, handle-ratified snapshot before
commit. Atomic exchange can legitimately change Unix `ctime`, so a stable
post-exchange observation ratifies the new token with native identity, link
count, content, and length. The displaced file's stable ownership, mode, ACL,
and visible extended attributes must then match the snapshot exactly before it
can be applied. A final-window metadata change leaves the committed file private
and adds a warning. This removes both the prior possibility of applying
unratified metadata read after commit and the later stale-snapshot overwrite
window.

Subsequent security review found that macOS resource forks can be file-sized
xattrs.
The snapshot reader now queries every native xattr size before allocation,
enforces a 4,096-entry and 64 MiB aggregate names-and-values budget, and retries
size races only within a fixed bound. macOS serializes the ACL before commit and
replays it through the destination descriptor; resource forks and other xattrs
are applied from the bounded snapshot rather than copied live.

Exact-commit run
[30202690197](https://github.com/blisspixel/noter/actions/runs/30202690197)
then proved that macOS reports a missing extended ACL from `acl_get_fd` as
`ENOENT` rather than returning an allocated empty ACL. The current repair
retains that as a distinct `Absent` snapshot state, replays it through the
native remove-ACL sentinel, and distinguishes absence from present ACLs with
entries.

Exact-commit run
[30211571501](https://github.com/blisspixel/noter/actions/runs/30211571501)
then reached the macOS mutation baseline and exposed two stale native test
expectations. The inheritable-parent fixture proved that an ordinary control
file receives the parent ACE while the `openx_np` protected file immediately
reports true ACL absence. Replaying explicit zero-entry ACL text also reports
absence. macOS therefore canonicalizes the zero-entry representation rather
than retaining it as a separately observable stored ACL. The product security
path behaved correctly; the baseline failed because its assertions and
documentation expected `Present` in both cases. Those expectations are now
aligned with the native result, and the full platform campaign must rerun on the
corrected commit.

Exact-commit run
[30211952848](https://github.com/blisspixel/noter/actions/runs/30211952848)
passed the macOS product tests and then exposed an overly broad macOS mutation
assignment. Thirty-one of its 36 survivors were generic cross-platform or Unix
decisions assigned to the Linux or Windows jobs. One was a macOS-only `ENOATTR`
comparison hidden inside a generically named Unix helper. The platform predicates
now have explicit Linux and macOS names plus exact truth-table tests, making each
mutation assignable to a runner where its expression is active. The other four
survivors exposed missing observability for creation-time mode application,
private-creation finalization, the two ACL deallocation results, and ACL
verification. The macOS scope now selects macOS-specific mutation descriptions.
Native tests use a non-default creation mode, deliberately relax a protected
file before finalization, cover the complete deallocation-result truth table,
and require ACL verification to reject a mismatch. The revised three-runner
union still includes every configured candidate.

Mutation scope now includes the native platform adapter. The focused Windows
adapter campaign caught all 40 compiling behavioral mutations, classified 18
type-level mutations as unviable, and had no miss, timeout, or recognized
infrastructure failure. Exact `FileChangeToken` accessors, native `FileIdInfo`,
failed persistence barriers, and descriptor deallocation are observable through
native fixtures. The descriptor owns an injected `LocalFree`-compatible
deallocator, so removing its `Drop` body is now caught without dereferencing a
freed allocation. The configuration excludes unsupported-platform shims and
OR-to-XOR changes over disjoint Windows API flag bits that cargo-mutants cannot
distinguish semantically. AND substitutions for the same flags remain in scope
and are caught.

A preliminary 58-candidate run contained one transient `LNK1104` executable-lock
failure that cargo-mutants labeled unviable. The infrastructure validator
rejected that classification. The exact candidate was caught in an isolated
target, then the entire 58-candidate campaign was repeated in one fresh target
directory. The final single report is 40 caught and 18 genuine compiler
rejections, and the infrastructure validator reports no recognized failure.

Exact-commit run
[30213398323](https://github.com/blisspixel/noter/actions/runs/30213398323)
passed format, Clippy, rustdoc, dependency policy, documentation, and all native
product tests. Its macOS mutation job completed cleanly. The Windows job had no
miss and only two timeouts in mutable line-scanner progress arithmetic. The
Linux job reported the same two timeouts plus 32 survivors in repeated native
decision expressions. Infrastructure validation passed for every runner.

The repair replaces scanner index arithmetic with structurally bounded slice
splits, extracts repeated Unix syscall and boundary decisions into named
predicates with exact truth-table tests, and injects ownership application into
its decision test so coverage is independent of process privilege. Focused
local campaigns classify the corrected scanner as seven caught and two genuine
compiler rejections, the applicable Unix platform correction as 51 caught and
one genuine compiler rejection, and ownership application as four caught. No
focused campaign missed or timed out a viable mutation.

The settled worktree now enumerates 741 candidates: 617 assigned to Linux, 557
to Windows, and 49 macOS-specific candidates assigned to macOS. Deduplicating
the three scopes yields all 741
configured candidates with no missing or outside entry. The focused Windows
adapter scope is 66 candidates. The ten-site reduction from the preceding 751
total comes from removing repeated inline native decisions and mutable scanner
progress arithmetic, not from excluding a source path or supported platform.

The final focused Markdown diagnostics run enumerated 58 candidates. It caught
54 and initially labeled four unviable, but the infrastructure validator found
a Windows linker failure in the `FenceMarker::closes` result. A fresh isolated
two-candidate rerun caught both variants, producing a composite 55 caught and
three genuine compile-time rejections with zero missed and zero timed out. This
is focused local evidence, not a substitute for the full platform matrix.

Exact-commit run
[30217724043](https://github.com/blisspixel/noter/actions/runs/30217724043)
then passed every non-Windows-mutation job. Its Windows scope had zero misses but
timed out the `partial_state_is_completable` `&&`-to-`||` mutant after the direct
truth-table test had already printed `FAILED`. Local reproduction made the same
test fail in 0.00 seconds and the full library suite exit in under one second.
A focused campaign caught all four mutations of that predicate. CI now gives
the Windows test process a 60-second minimum while retaining the 90-minute job
limit.

Exact-commit run
[30219731527](https://github.com/blisspixel/noter/actions/runs/30219731527)
reported a green aggregate status at `daaeeff`, but post-run artifact review
rejected it as baseline evidence. The macOS report classified an ANSI-decorated
Clang linker crash as unviable. That failure was infrastructure, not a genuine
compiler rejection. The validator now strips ANSI control sequences and rejects
linker crashes reported through either Clang diagnostic form.

Corrected exact-commit run
[30221793209](https://github.com/blisspixel/noter/actions/runs/30221793209)
passes the complete matrix at `97371d8`. Linux reports 617 total, 438 caught, and
179 genuine compiler rejections. Windows reports 557 total, 381 caught, and 176
genuine compiler rejections. macOS reports 49 total, 43 caught, and 6 genuine
compiler rejections; the formerly misclassified mutation is caught. Every scope
has zero missed and zero timed out. The strengthened infrastructure validator
passes all three artifacts, and the deduplicated union contains all 741
configured candidates with no missing or outside entry.

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

These changes strengthened the implementation as well as the tests. They were
rerun across the then-configured scope after the last cleanup-race survivor was
fixed. The later Unix snapshot and native-adapter additions are covered by the
three-platform exact-commit run recorded above.

## Interpretation and remaining limits

A clean mutation run is evidence that this mutation operator set cannot find an
uncaught behavioral change in the configured scope. It does not prove native
metadata preservation, crash durability, weak-filesystem behavior, GUI
semantics, or performance. Those remain separate M1 and later milestone gates in
[ROADMAP.md](ROADMAP.md) and [manual-test-matrix.md](manual-test-matrix.md).

The deduplicated 741-candidate union is hosted exact-commit evidence at
`97371d8`. Run
[30221793209](https://github.com/blisspixel/noter/actions/runs/30221793209)
remains the baseline for that explicitly reconciled union.

Later implementation increased the configured scopes. At commit `efb8675`,
exact-commit run
[30415383710](https://github.com/blisspixel/noter/actions/runs/30415383710)
reports 817 Linux candidates with 586 caught and 231 unviable, 751 Windows
candidates with 524 caught and 227 unviable, and 47 macOS candidates with 41
caught and 6 unviable. Every scope has zero missed and zero timed out, and the
strengthened infrastructure validator reports no recognized failure hidden as
unviable. These overlapping per-platform results are not presented as a new
deduplicated union without a matching reconciliation artifact.
