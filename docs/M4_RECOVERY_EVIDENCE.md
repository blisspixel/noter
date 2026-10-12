# M4 Recovery Evidence Inventory

**Inventory date:** 2026-10-11

**Inspected source:** `f68997185b0476c5139306947900f3e5045f9682`

**Status:** Partial automated evidence; M4-H1 native matrix remains open in
[issue #50](https://github.com/blisspixel/noter/issues/50).

This inventory separates existing regression fixtures from the environment
experiments still needed. The inspected main commit passed its
[complete CI](https://github.com/blisspixel/noter/actions/runs/38157480793),
including native workspace tests on Windows, macOS, and Linux. That establishes
the automated fixtures on those runners. It does not establish second-user,
remote-volume, power-loss, or interactive desktop behavior.

The product contract is FR-062 through FR-068 in
[REQUIREMENTS.md](REQUIREMENTS.md). Decisions remain in
[ADR-0004](adr/0004-unix-recovery-namespace.md) and
[ADR-0005](adr/0005-windows-recovery-replacement.md); sequence and completion
criteria remain in [ROADMAP.md](ROADMAP.md).

## Existing fixtures to reuse

Names below identify tests, not new support claims. Unix fixtures run only on
their declared platforms; injected failures exercise the named boundary and do
not simulate a real power interruption.

| Property | Existing fixture and source | Remaining environment proof |
| --- | --- | --- |
| Unix ancestor protection | `an_ancestor_others_can_write_is_rejected_unless_it_is_sticky`, `a_group_writable_ancestor_is_rejected_unless_it_is_sticky`, and Linux `a_named_user_acl_write_grant_cannot_hide_in_group_mode_bits` in [Unix namespace](../crates/noter-platform/src/unix_recovery_namespace.rs) | Attempt create, rename, read, and delete from a distinct unprivileged identity against live private directories and staged bytes. Record effective ACLs and identity. |
| Unix retained namespace | `operations_follow_the_bound_directory_after_an_ancestor_is_renamed`, `a_removed_directory_refuses_new_content`, and `removal_refuses_an_entry_replaced_after_it_was_opened` in the Unix namespace | Enumerate final pathname-operation windows under the ADR-0004 threat model. Preserve its explicit same-user unlink limitation. |
| Windows held traversal | `native_drive_remap_keeps_state_binding_under_the_held_parent`, `retained_traversal_handles_prevent_ancestor_path_swap`, and `native_reparse_private_child_is_rejected_without_creating_target_content` in [Windows namespace](../crates/noter-platform/src/windows_recovery_namespace.rs) | Add only uncovered race boundaries; repeat on the named Windows/NTFS environment with before/after identity and target-content assertions. |
| Registered cloud roots | `native_cloud_sync_root_query_refuses_a_registered_subtree` and `cloud_root_checks_cover_every_bound_directory_before_content` in the Windows namespace | Inventory non-Cloud-Files sync and redirected roots separately. A registered-root query is not a detector for every synchronization product. |
| Access policy | `owner_controlled_existing_recovery_directories_are_tightened` in the Windows namespace; `private_file_is_exclusive_writable_and_dacl_protected` and macOS `recovery_state_directory_loses_an_inherited_acl` in [platform primitives](../crates/noter-platform/src/lib.rs) | Distinct-user access attempts, including inherited policy and existing directories. Descriptor inspection alone does not replace access attempts. |
| Windows replacement and barriers | Windows-only `failed_predecessor_barrier_retains_both_recovery_snapshots`, `failed_new_record_barrier_retains_durable_predecessor_backup`, `failed_cleanup_barrier_keeps_the_committed_new_record`, and `failed_first_record_barrier_retains_the_installed_snapshot` in [recovery store](../src/core/recovery_store.rs) | Interrupt a disposable process at each real protocol boundary, restart, and compare all available candidates. Do not infer power-loss durability from process termination. |
| Exact retirement | Windows-only `reconciled_windows_cleanup_deletes_only_the_verified_backup_handle` and `reconciled_windows_cleanup_blocks_destination_path_rebind`; cross-platform `quarantine_parent_sync_failure_retains_the_bound_source` in the recovery store | Complete sharing, cleanup, and final-window matrix per operating system; assert both intended bytes retained and unrelated bytes unchanged. |
| Weak and remote roots | Linux-only `network_user_space_and_volatile_file_systems_are_refused`, native `tmpfs_roots_are_refused_before_recovery_entries_are_created`, and macOS-only `only_mounts_marked_local_are_local` in the Unix namespace; Windows-only `device_policy_rejects_nonlocal_or_weak_volume_flags` in the Windows namespace | Complete the remaining real remote/removable/weak filesystems and redirected-root cases. Policy-unit inputs alone are insufficient. |

## Windows process-interruption fixture

`terminated_windows_writer_retains_exact_restart_candidates` in
[recovery store](../src/core/recovery_store.rs) runs eight disposable native
Windows child processes through the real handle-bound replacement protocol.
The existing barrier callback pauses immediately before or after the selected
directory sync. The parent terminates and waits for the child while it holds
the actual artifact handles, directory handles, and live lease, then opens a
fresh store and scans through the normal startup seam.

| Boundary | Cases | Expected latest artifact | Retained predecessor |
| --- | --- | --- | --- |
| Predecessor backup directory sync | Before and after | Reserved stage | Reserved backup |
| Replacement installation directory sync | Before and after | Canonical record | Reserved backup |
| Predecessor cleanup directory sync | Before and after | Canonical record | None |
| First-record installation directory sync | Before and after | Canonical record | None |

Each case checks exact encoded bytes, expected artifact presence and count,
complete startup scanning, the latest offered revision, superseded handles,
successful claimed loading after the child's live lease is released by process
termination, and unchanged unrelated recovery and outside original-file bytes.
The restart scan does not authorize automatic deletion of a predecessor. A
30-second readiness deadline, a child cleanup guard, and a 60-second child
self-timeout bound ordinary fixture failures. The test uses synthetic English
payloads in an isolated temporary state root, not the user's recovery store.

Run on native Windows with the pinned toolchain:

```powershell
cargo test --locked --lib process_interruption -- --nocapture
```

The ordinary workspace test gate and every native Windows mutation shard also
execute the fixture. It adds no runtime switch, dependency, or production
protocol change. This is process-interruption evidence at the named boundaries,
not power-loss, arbitrary instruction-level interruption, a complete desktop
restart, distinct-user, remote-volume, or cross-platform evidence. The remaining
environment and race rows above remain open in issue #50. Exact-commit test,
CI, and review receipts belong with the implementing pull request before any
matrix row is promoted to Verified.

## Linux distinct-identity fixture

`scripts/check_recovery_identity.py` drives the test-only
`identity_probe_owner` through `RecoveryStore::open_in_state` and the real Unix
staging callback. An elevated broker launches two different unused numeric
UID/GID pairs with `setpriv`. Neither child has supplementary groups or any
inheritable, permitted, effective, bounding, or ambient capabilities;
`no_new_privs` prevents privilege gain across exec. No accounts are created or
modified. Ordinary workspace tests do not change process identities.

Four cases cover a new private hierarchy, preexisting owner-controlled
directories initially shared with mode `0777`, explicit named-user POSIX ACL
grants, and inherited named-user access/default ACLs. The ACL cases grant only
the owner and the unrelated numeric identity; owning-group and other entries
grant nothing. The outsider has neither the owner's group nor supplementary
groups, so successful access establishes the named-user grant. For all three
shared cases, the unrelated identity first verifies the exact recorded policy,
then lists, creates, and deletes in all four directories, independently
establishing the adverse precondition. The inherited case sets ACLs only on the
state directory and verifies their inheritance by every descendant before
Noter opens them. Noter then tightens the hierarchy before staging recovery
bytes. While the owner holds the exact staged successor and
live lease, the unrelated identity must successfully read, write, create,
rename, and delete in an accessible control. It must receive `EACCES` for 28
operations against the private hierarchy and its five live artifacts. Missing
paths, failed process startup, incorrect identities, and other errors fail the
fixture rather than counting as access denial.

The broker records UID/GID, groups, capability sets, effective modes, POSIX
access/default ACL bytes when present, filesystem type, kernel, and test-executable hash.
It checks artifact identity and bytes before and after the probe. The owner
then releases its lease, reopens through startup scanning, and claims the exact
latest staged successor, canonical predecessor, and unrelated snapshot. An
outside sentinel remains unchanged. Readiness, probe, completion, and child
lifetime are bounded; cleanup stays within an isolated synthetic temporary root.

Run only in an explicitly isolated Linux environment with the pinned toolchain,
Python, util-linux `setpriv`/`findmnt`, and permission to use a root broker:

```sh
cargo test --locked --lib --no-run --message-format=json > .agent/recovery-identity-build.json
sudo python3 scripts/check_recovery_identity.py --disposable --cargo-artifacts .agent/recovery-identity-build.json
```

Linux native CI runs this fixture inside the required workspace-test job and
uploads `recovery-identity-linux`. Eleven portable script regressions protect
identity validation, capability dropping, test-binary selection, exact access
denial, positive controls, readiness failure, explicit opt-in, named-user ACL
construction, mandatory ACL cases, policy-change detection, and ACL-query
failure propagation. Existing
recovery and platform mutation scopes remain unchanged; this privileged
orchestration is separate from ordinary per-mutant workspace tests.

This establishes the named Linux identity and mode-tightening cases on the
filesystem recorded by each receipt. It does not establish Windows token/DACL
behavior, macOS identities or ACLs, every POSIX ACL combination, hostile
same-user writes, remote roots, power-loss durability, or the full M4-H1 matrix.
Exact-commit native output, CI, and reviews belong with the implementing PR.

## Linux volatile-root refusal

FR-062's 2026-10-12 clarification excludes known volatile recovery roots.
The Linux classifier now refuses tmpfs, ramfs and hugetlbfs before binding
recovery children. Private permissions do not make memory-backed storage
persistent. Existing records are preserved; this does not change ordinary
document editing or saving.

The active native test
`unix_recovery_namespace::tests::tmpfs_roots_are_refused_before_recovery_entries_are_created`
uses an isolated directory on the existing `/dev/shm` mount. It independently
requires tmpfs through `fstatfs`, then proves accessible control read/write.
New-state refusal leaves the created state directory empty. Existing-state
refusal preserves the exact predecessor sentinel identity, length and bytes,
its directory inventory, and unrelated control bytes without creating
quarantine or other recovery entries. The test fails on the previous policy,
which explicitly classified tmpfs as acceptable local storage.

```sh
cargo test --locked -p noter-platform --lib unix_recovery_namespace::tests::tmpfs_roots_are_refused_before_recovery_entries_are_created -- --exact --nocapture
```

This establishes real tmpfs refusal and classifier-unit coverage for ramfs and
hugetlbfs. It does not establish native ramfs/hugetlbfs observations, detect
volatile devices that present another filesystem type, prove power-loss
durability, or complete M4-H1. The ordinary filesystem fixtures require a
persistent temporary root; see [DEVELOPMENT.md](DEVELOPMENT.md).

## Next fixture protocol

For each missing environment case, retain a bounded, reproducible fixture in
the repository before recording a passing result:

1. Use synthetic English payloads with distinct predecessor, successor, and
   unrelated sentinel bytes. Record their hashes, not user document content.
2. Record exact commit, command, operating system/build, filesystem and mount or
   volume properties, execution identity, and effective access policy.
3. Establish the adverse precondition independently, then invoke the public
   recovery seam. A fixture that cannot establish its precondition is
   unavailable evidence, not a passing refusal case.
4. Assert the typed outcome, candidate identities and hashes, untouched outside
   targets, bounded artifacts, and visible failure propagation where applicable.
5. Restart through startup scanning and assert the offered latest revision,
   retained predecessors, and explicit cleanup/review outcome.
6. Attach exact-head CI, native fixture output, the mutation result, and two
   independent reviews before promoting a matrix row to Verified.

Native CI can automate established ordinary local filesystem and fault-injection
fixtures. Disposable VMs or isolated native accounts are needed for experiments
that change users, mounts, sharing, or machine state. Keep those experiments
separate from ordinary developer-machine tests. Existing M1 document-save
observations in [M1_FILESYSTEM_EVIDENCE.md](M1_FILESYSTEM_EVIDENCE.md) remain
historical evidence for that protocol; they do not close this recovery matrix.

No missing row is waived for beta.3. The postponed Omarchy check remains in
[issue #44](https://github.com/blisspixel/noter/issues/44) and is independent of
the storage fixture inventory.
