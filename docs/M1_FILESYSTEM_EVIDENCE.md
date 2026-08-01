# M1 Filesystem Evidence

**Observed:** 2026-07-31

**Source-equivalent commit:** `65ac25f1b6c83a75bc4b00abe177aff98a4f2c03`

**Latest Rust validation commit:** `994e0a3dc66a67cc22b1c0590436b92953a42747`

**Status:** Partial local M1 evidence. This is not M1 milestone sign-off.

## Scope and provenance

This record covers synthetic document fixtures on native Windows NTFS, native
WSL2 ext4, and the Windows-to-WSL UNC boundary. The probes exercised Noter's
public document loading and saving APIs from isolated local test binaries. No
real user document, account, remote service, or telemetry was involved.

The final fixtures ran before the commit above was created. A post-run blob
comparison checked all 38 tracked Rust, Cargo, and pinned-toolchain inputs in
both retained source copies against that commit and found zero mismatches. The
record is therefore source-equivalent, not an exact-commit execution claim. The
tracked automated suite independently covers the security-verification and
reconciliation decisions; the platform-specific metadata observations are
recorded only here.

The temporary probe binaries and fixture directories were not committed, so
this is an observed local record rather than a fully reproducible evidence
artifact. Checksums, initial conditions, outcomes, and limitations are included
so a later independent run can compare the same invariants.

## Environment

| Item | Value |
| --- | --- |
| Windows host | Windows 11 Pro, build 26200 |
| Windows filesystems | Healthy fixed NTFS volumes |
| Linux guest | Ubuntu 20.04.4 LTS under WSL2, kernel 6.6.114.1-microsoft-standard-WSL2 |
| Linux fixture filesystem | Native WSL2 ext4, mounted with ordered data mode |
| Rust | 1.97.1 |
| Cargo | 1.97.1 |

## Validation commands and observation method

The Windows checkout ran the repository's required commands, including:

```text
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-targets --all-features
$env:RUSTDOCFLAGS="-D warnings"; cargo doc --locked --workspace --no-deps
cargo audit --deny warnings
cargo deny --locked check
ruff check scripts
ruff format --check scripts
python -m unittest discover -s scripts -p "test_*.py"
python scripts/check_doc_links.py
python scripts/check_readme_assets.py
python -I scripts/check_release_config.py
```

The same locked workspace test command ran from the native ext4 source copy in
WSL2. Coverage used both `cargo llvm-cov` commands defined in
[CODE-QUALITY-STANDARDS.md](CODE-QUALITY-STANDARDS.md); Python coverage used
branch mode over the complete script test discovery.

Source equivalence enumerated the commit's 38 tracked Rust, `Cargo.toml`,
`Cargo.lock`, and `rust-toolchain.toml` inputs with `git ls-tree`. For each path,
the commit blob from `git rev-parse 65ac25f:<path>` matched `git hash-object` on
both retained fixture source copies. No input was missing and no blob differed.

Temporary integration-test targets invoked public Noter APIs for each save.
PowerShell captured SDDL, stream, compression, volume, and hash facts through
native commands. Linux inspection used `findmnt`, `stat`, `sha256sum`, and
Python's byte-oriented extended-attribute APIs. Each probe verified fixture
state before the save, printed the typed result, reopened the destination, and
verified bytes and metadata afterward. Because those temporary targets are not
tracked, their invocation is not presented as a repository reproduction
command. This is an explicit evidence limitation.

## Common fixture bytes

| Revision | Length | SHA-256 | BLAKE3-256 where recorded |
| --- | ---: | --- | --- |
| Original existing document | 21 bytes | `97ce82919003a98f4ee3be9bac9a0e5a623534e772bd94497671ae7a995fe933` | Not separately recorded |
| Updated existing document | 29 bytes | `2557db2d03eddaf7a2c27a3666329f1379e887f83d3517343b23303412df9d22` | `fb1ea0d26e4a1e6f0cd1e06718817f4ab95e69f6a1e16c72f176807d68426885` |
| New document | 13 bytes | `49173a9e43d07eb6bf6817cdd0fde0dcbcf1d24ebfdb653ec9dff66f8dced770` | `e81f2ec82401f28717904e8709062ca9d74e3b59b50377082bea694717677d64` |

## Native Windows NTFS

### Existing-file replacement

The existing fixture started with a deliberately non-default security
descriptor, NTFS compression, and an alternate data stream named
`noter.evidence` containing `preserve-me`.

| Observation | Result |
| --- | --- |
| Save classification | `Committed` |
| Durability classification | `FileSynced` |
| Cleanup warnings | 0 |
| Durability warnings | 0 |
| Destination bytes | Exact updated checksum above |
| Security descriptor | Exact original SDDL preserved |
| Alternate data stream | Name and exact value preserved |
| NTFS compression | Preserved |
| Temporary or backup artifacts | None remained |

`FileSynced` is the strongest result claimed. Windows exposes no supported
parent-directory barrier for this protocol, and this fixture did not simulate a
power loss or kernel crash.

### New-file installation

The save committed the exact new-document bytes with no warning. Before the
first application byte, the created handle reported the process user as owner
and an exact protected DACL containing only two explicit allow entries: full
control for that user and full control for SYSTEM. The numeric user SID is
intentionally omitted from this public record. No temporary sibling remained.

### Read-only failure

A save over a read-only destination returned `NotCommitted` at
`ApplyMetadata`. The original 21 bytes and read-only attribute remained intact,
the document remained dirty, and no temporary or backup artifact remained.

## Native WSL2 ext4

The Linux fixture ran from a source checkout and target directory both located
inside the WSL2 ext4 filesystem, not through a Windows-mounted path.

### Existing-file replacement

| Observation | Result |
| --- | --- |
| Save classification | `Committed` |
| Durability classification | `FileAndDirectorySynced` |
| Destination bytes | Exact updated checksum above |
| Destination mode | 0640 preserved |
| Extended attribute | `user.noter.evidence=preserve-me` preserved |
| Cleanup warnings | 1 expected portable-Unix retention warning |
| Durability warnings | 0 |

The exact displaced original remained as the reported random sibling because
portable Unix cannot unlink a previously verified object by handle. Its bytes
matched the original checksum, its mode was restricted to 0600, and its
extended attribute was preserved. This is the designed recoverable outcome,
not an unreported cleanup success.

### New-file installation

The new save returned `Committed` and `FileAndDirectorySynced`, produced the
exact new-document checksum, used mode 0600, emitted no warning, and retained no
temporary artifact.

`FileAndDirectorySynced` records successful file and directory synchronization
calls inside the guest. It does not prove persistence through failure of the
Windows host, virtual disk, storage controller, or physical device.

## Windows-to-WSL UNC boundary

The first evidence pass exposed a privacy defect. Before the remediation later
recorded in the source-equivalent commit above, Windows Save As could install a
new file through the WSL UNC bridge with Linux mode 0644 even though the Windows
security request appeared to succeed. That result violated the owner-only
new-file contract.

The remediation binds the Windows owner to the process token user, constructs
the protected user-and-SYSTEM DACL from canonical native data, and verifies the
owner plus exact DACL through the created handle before any document byte is
written. A filesystem boundary that cannot report the requested policy is now
rejected.

The final bridge fixture produced these results:

| Case | Result |
| --- | --- |
| Existing document | `NotCommitted` at `CreateTemporary`, operating-system code 1 |
| Existing bytes | Unchanged, BLAKE3-256 `1ad1c3107e74eae42ef2af5a57167c3907d8fbea0057024c67ef402e5e64ef6d` |
| Existing mode | 0640 unchanged |
| Existing document state | Dirty state preserved |
| New document | `NotCommitted` at `CreateTemporary`, operating-system code 1 |
| New destination | Absent |
| Temporary artifacts | None remained in either case |

This is a fail-closed privacy result, not evidence that the bridge supports
durable save. The error remains stage-specific and path-redacted.

## Automated validation of the latest follow-up source

The exact Rust source at `994e0a3` passed formatting, strict workspace Clippy,
rustdoc with warnings denied, dependency audit and policy, and the following
test and coverage gates. The current evidence worktree separately passes
documentation, release-configuration, and screenshot validation after
regenerating byte-identical Light and Dark captures and approving their current
native-input digest. Exact-head validation of that approval remains pending.

| Gate | Result |
| --- | --- |
| Windows workspace tests | 425 passed, 0 failed at `994e0a3` |
| Native WSL2 ext4 workspace tests | 428 passed, 0 failed against source-equivalent `65ac25f` inputs |
| Whole-workspace line coverage | 93.49 percent |
| Trust-kernel line coverage | 95.23 percent |
| Platform-adapter line coverage | 92.14 percent |
| Python validation tests | 139 passed, 0 failed |
| Python branch coverage | 86 percent |

An exact clean-detached focused campaign against `65ac25f` generated 20 Windows
owner and descriptor candidates: 17 were caught and three token-length boundary
mutations survived. The follow-up extracted the two length decisions into pure
helpers and added exact lower, equal, and upper boundary assertions without
changing runtime behavior. Repeating the exact campaign from clean detached
`994e0a3` caught all 20 candidates with no unviable, missed, or timed-out result;
the infrastructure validator passed. One bit-mask rewrite was excluded with an
explicit equivalence proof because the two constant operands have no common set
bit. Commands, complete candidate descriptions, source trees, outcomes, and
local artifact hashes are retained in the
[focused mutation record](evidence/m1-windows-private-security-mutation-2026-07-31.json).

## Availability and remaining evidence

| Requested environment or fault | Availability on 2026-07-31 | Disposition |
| --- | --- | --- |
| Native NTFS | Available | Executed as described above |
| Native WSL2 ext4 | Available | Executed as described above |
| Windows-to-WSL UNC | Available | Executed; unsupported security semantics fail closed |
| SMB mapping or network drive | None configured | Not executed |
| Removable drive | None attached | Not executed |
| FAT or exFAT | No usable local volume | Not executed |
| Cloud-synchronized directory | Configured | Not written because that would send fixture data to an external party |
| Second disposable Windows identity | No authorized credentials or disposable identity | Not executed |
| Native macOS | Unavailable | Not executed |
| Power-loss, kernel-crash, or reboot persistence | Destructive system operation | Not executed |

Four healthy fixed NTFS volumes were visible, but repeating the same synthetic
contract on each would not substitute for the missing filesystem classes.
These availability limits are evidence gaps, not passes.

M1 remains in progress. In particular, this record does not prove Windows
encryption, creation-time and file-identifier policy, native macOS metadata,
SMB, cloud synchronization, removable or weak filesystems, cross-identity
denial, or crash persistence. Exact-head hosted CI must also validate the final
evidence commit before this record can become part of a verified checkpoint.
