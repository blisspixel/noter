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
| Weak and remote roots | Linux-only `network_and_user_space_file_systems_are_classified_as_shared` and macOS-only `only_mounts_marked_local_are_local` in the Unix namespace; Windows-only `device_policy_rejects_nonlocal_or_weak_volume_flags` in the Windows namespace | Real remote/removable/weak filesystems and redirected roots must refuse before recovery payload writes or receive explicit adversarial evidence. Policy-unit inputs alone are insufficient. |

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
