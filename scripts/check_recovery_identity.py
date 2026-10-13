#!/usr/bin/env python3
"""Exercise Linux recovery with two real unprivileged numeric identities.

Run only in an explicitly disposable root environment. No accounts are created.
The ordinary user's Cargo build supplies the unit-test executable via JSON.
"""

from __future__ import annotations

import argparse
import errno
import hashlib
import json
import os
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path


OWNER_TEST = "core::recovery_store::tests::identity_probe_owner"
READY = b"private stage and live lease held"
COMPLETE = b"exact private restart candidates retained"
PRIVATE_DIRECTORIES = (
    "state",
    "state/recovery",
    "state/recovery/records",
    "state/recovery/quarantine",
)
CASES = (
    "new-private",
    "existing-shared",
    "existing-named-acl",
    "inherited-named-acl",
)


def named_user_acl(uid: int) -> bytes:
    """Linux POSIX ACL v2: only the owner and one named user have access."""
    if not 0 < uid < 0xFFFFFFFF:
        raise ValueError("a non-root defined numeric user is required")
    entries = (
        (1, 7, 0xFFFFFFFF),
        (2, 7, uid),
        (4, 0, 0xFFFFFFFF),
        (16, 7, 0xFFFFFFFF),
        (32, 0, 0xFFFFFFFF),
    )
    return struct.pack("<I", 2) + b"".join(
        struct.pack("<HHI", *entry) for entry in entries
    )


def read_acl(path: Path, kind: str) -> bytes | None:
    try:
        return os.getxattr(path, f"system.posix_acl_{kind}")
    except OSError as error:
        if error.errno != errno.ENODATA:
            raise
        return None


def access_policy(path: Path) -> dict:
    facts = path.lstat()
    return {
        "uid": facts.st_uid,
        "gid": facts.st_gid,
        "mode": oct(stat.S_IMODE(facts.st_mode)),
        **{
            f"posix_{kind}_acl_hex": None if acl is None else acl.hex()
            for kind in ("access", "default")
            for acl in (read_acl(path, kind),)
        },
    }


def test_executable(artifacts: Path) -> Path:
    candidates = []
    for line in artifacts.read_text(encoding="utf-8").splitlines():
        message = json.loads(line)
        if (
            message.get("reason") == "compiler-artifact"
            and message.get("target", {}).get("name") == "noter"
            and message.get("target", {}).get("kind") == ["lib"]
            and message.get("profile", {}).get("test") is True
            and message.get("executable")
        ):
            candidates.append(Path(message["executable"]))
    if len(candidates) != 1 or not candidates[0].is_file():
        raise ValueError(
            "exactly one existing Noter library test executable is required"
        )
    return candidates[0]


def require_identity(uid: int, gid: int) -> dict:
    status = dict(
        line.split(":", 1)
        for line in Path("/proc/self/status").read_text().splitlines()
        if ":" in line
    )
    identity = {
        "uid": os.getuid(),
        "euid": os.geteuid(),
        "gid": os.getgid(),
        "egid": os.getegid(),
        "groups": os.getgroups(),
        "effective_capabilities": int(status["CapEff"].strip(), 16),
        "capability_sets": {
            key: int(status[key].strip(), 16)
            for key in ("CapInh", "CapPrm", "CapEff", "CapBnd", "CapAmb")
        },
        "no_new_privileges": int(status["NoNewPrivs"].strip()),
    }
    if not (
        uid > 0
        and gid > 0
        and identity["uid"] == identity["euid"] == uid
        and identity["gid"] == identity["egid"] == gid
        and not identity["groups"]
        and identity["effective_capabilities"] == 0
        and not any(identity["capability_sets"].values())
        and identity["no_new_privileges"] == 1
    ):
        raise AssertionError("child is not the required unprivileged identity")
    return identity


def expect_denied(operation) -> None:
    try:
        operation()
    except OSError as error:
        if error.errno != errno.EACCES:
            raise AssertionError(
                "probe failed for a reason other than access denial"
            ) from error
    else:
        raise AssertionError("unrelated identity accessed private recovery state")


def probe(root: Path, uid: int, gid: int, shared: bool = False) -> dict:
    identity = require_identity(uid, gid)
    if shared:
        expected = json.loads((root / "precondition.json").read_text())
        for relative in PRIVATE_DIRECTORIES:
            directory = root / relative
            assert access_policy(directory) == expected[relative]
            list(directory.iterdir())
            precondition = directory / "shared-precondition"
            with precondition.open("xb") as stream:
                stream.write(b"shared access independently established")
            precondition.unlink()
        return {"identity": identity, "precondition_operations": 12, "policy": expected}
    control = root / "control"
    original = control / "original"
    assert original.read_bytes() == b"accessible control"
    original.write_bytes(b"control write succeeded")
    created = control / "created"
    with created.open("xb") as stream:
        stream.write(b"control create succeeded")
    renamed = control / "renamed"
    created.rename(renamed)
    renamed.unlink()
    config = json.loads((root / "probe.json").read_text())
    attempts = 0
    for relative in config["directories"]:
        directory = root / relative
        expect_denied(lambda: list(directory.iterdir()))
        expect_denied(lambda: (directory / "hostile-create").open("xb"))
        attempts += 2
    for relative in config["files"]:
        path = root / relative
        expect_denied(path.read_bytes)
        expect_denied(lambda: path.write_bytes(b"hostile replacement"))
        expect_denied(lambda: path.rename(path.with_name("hostile-rename")))
        expect_denied(path.unlink)
        attempts += 4
    return {
        "identity": identity,
        "control_operations": 5,
        "denied_operations": attempts,
    }


def identity_command(uid: int, gid: int, arguments: list[str]) -> list[str]:
    executable = shutil.which("setpriv")
    if executable is None:
        raise RuntimeError("util-linux setpriv is required")
    last_capability = int(Path("/proc/sys/kernel/cap_last_cap").read_text())
    if not 0 <= last_capability < 64:
        raise RuntimeError("unsupported Linux capability range")
    # Numeric names also cover kernels newer than the installed libcap-ng.
    drop_caps = ",".join(f"-cap_{index}" for index in range(last_capability + 1))
    return [
        executable,
        "--reuid",
        str(uid),
        "--regid",
        str(gid),
        "--clear-groups",
        f"--inh-caps={drop_caps}",
        f"--ambient-caps={drop_caps}",
        f"--bounding-set={drop_caps}",
        "--no-new-privs",
        "--",
        sys.executable,
        *arguments,
    ]


def wait_ready(process: subprocess.Popen, marker: Path) -> None:
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if process.poll() is not None:
            raise AssertionError("owner exited before holding the live private stage")
        if marker.exists() and marker.read_bytes() == READY:
            return
        time.sleep(0.01)
    raise TimeoutError("owner did not establish the private-stage precondition")


def run_case(
    base: Path, executable: Path, mode: str, owner: int, outsider: int
) -> dict:
    root = base / mode
    root.mkdir(mode=0o755)
    root.chmod(0o755)
    os.chown(root, owner, owner)
    control = root / "control"
    control.mkdir(mode=0o700)
    os.chown(control, outsider, outsider)
    (control / "original").write_bytes(b"accessible control")
    os.chown(control / "original", outsider, outsider)
    sentinel = root / "outside.txt"
    sentinel.write_bytes(b"unrelated outside bytes")
    initial = None
    if mode not in CASES:
        raise ValueError("unknown native identity case")
    if mode != "new-private":
        initial_policy = {}
        for relative in PRIVATE_DIRECTORIES:
            path = root / relative
            path.mkdir(mode=0o777)
            os.chown(path, owner, owner)
            if mode == "existing-shared":
                path.chmod(0o777)
            elif mode == "existing-named-acl" or relative == "state":
                os.setxattr(path, "system.posix_acl_access", named_user_acl(outsider))
            if mode == "inherited-named-acl" and relative == "state":
                os.setxattr(path, "system.posix_acl_default", named_user_acl(outsider))
            policy = access_policy(path)
            assert policy["uid"] == policy["gid"] == owner
            if mode == "existing-shared":
                assert policy["mode"] == "0o777"
                assert policy["posix_access_acl_hex"] is None
            else:
                assert policy["mode"] == "0o770"
                assert policy["posix_access_acl_hex"] == named_user_acl(outsider).hex()
            assert policy["posix_default_acl_hex"] == (
                named_user_acl(outsider).hex()
                if mode == "inherited-named-acl"
                else None
            )
            initial_policy[relative] = policy
        (root / "precondition.json").write_text(json.dumps(initial_policy))
        (root / "precondition.json").chmod(0o644)
    environment = {
        "PATH": "/usr/bin:/bin",
        "HOME": str(root),
        "LANG": "C.UTF-8",
        "NOTER_TEST_RECOVERY_IDENTITY_ROOT": str(root),
    }
    script = str(base / "probe.py")
    if mode != "new-private":
        initial = json.loads(
            subprocess.check_output(
                identity_command(
                    outsider,
                    outsider,
                    [script, "--shared", str(root), str(outsider), str(outsider)],
                ),
                env=environment,
                cwd=base,
                text=True,
                timeout=15,
            )
        )
        assert initial["precondition_operations"] == 12
    command = identity_command(
        owner,
        owner,
        [script, "--owner", str(root), str(owner), str(owner), str(executable)],
    )
    with (base / f"{mode}.log").open("wb") as output:
        process = subprocess.Popen(
            command,
            env=environment,
            cwd=base,
            stdout=output,
            stderr=subprocess.STDOUT,
            stdin=subprocess.DEVNULL,
        )
        try:
            wait_ready(process, root / "ready")
            directories = [root / name for name in PRIVATE_DIRECTORIES]
            files = sorted((root / "state/recovery/records").iterdir())
            assert len(files) == 5 and sum(p.suffix == ".stage" for p in files) == 1
            policies = []
            before = {}
            for path in [*directories, *files]:
                facts = path.lstat()
                assert (
                    facts.st_uid == owner and stat.S_IMODE(facts.st_mode) & 0o077 == 0
                )
                policies.append(
                    {"name": str(path.relative_to(root)), **access_policy(path)}
                )
                if path in files:
                    before[path] = (
                        facts.st_dev,
                        facts.st_ino,
                        hashlib.sha256(path.read_bytes()).hexdigest(),
                    )
            config = {
                "directories": [str(p.relative_to(root)) for p in directories],
                "files": [str(p.relative_to(root)) for p in files],
            }
            (root / "probe.json").write_text(json.dumps(config))
            (root / "probe.json").chmod(0o644)
            result = subprocess.run(
                identity_command(
                    outsider,
                    outsider,
                    [script, "--probe", str(root), str(outsider), str(outsider)],
                ),
                env=environment,
                cwd=base,
                check=True,
                capture_output=True,
                text=True,
                timeout=15,
            )
            observed = json.loads(result.stdout)
            assert (
                observed["control_operations"] == 5
                and observed["denied_operations"] == 28
            )
            for path, expected in before.items():
                facts = path.stat()
                assert (
                    facts.st_dev,
                    facts.st_ino,
                    hashlib.sha256(path.read_bytes()).hexdigest(),
                ) == expected
            (root / "release").touch()
            if (
                process.wait(timeout=15) != 0
                or (root / "complete").read_bytes() != COMPLETE
            ):
                raise AssertionError("owner failed exact restart verification")
            assert sentinel.read_bytes() == b"unrelated outside bytes"
            return {
                "case": mode,
                "owner": json.loads((root / "owner.json").read_text()),
                "probe": observed,
                "shared_precondition": initial,
                "private_policy": policies,
                "restart": "exact latest, predecessor, and unrelated bytes retained",
            }
        except Exception:
            output.flush()
            print((base / f"{mode}.log").read_text(errors="replace"), file=sys.stderr)
            raise
        finally:
            if process.poll() is None:
                process.kill()
            process.wait()


def run(artifacts: Path, disposable: bool) -> dict:
    if not disposable or sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("explicit --disposable and a Linux root broker are required")
    import pwd
    import grp

    identities = []
    for candidate in range(200001, 200101):
        occupied = False
        for lookup in (pwd.getpwuid, grp.getgrgid):
            try:
                lookup(candidate)
                occupied = True
            except KeyError:
                pass
        if not occupied:
            identities.append(candidate)
        if len(identities) == 2:
            break
    if len(identities) != 2:
        raise RuntimeError("two unused numeric identities are required")
    original = test_executable(artifacts)
    with tempfile.TemporaryDirectory(
        prefix="noter-native-identities-", dir="/tmp"
    ) as name:
        base = Path(name)
        base.chmod(0o755)
        filesystem = subprocess.check_output(
            ["findmnt", "-n", "-T", str(base), "-o", "FSTYPE"], text=True, timeout=10
        ).strip()
        if not filesystem:
            raise RuntimeError("native filesystem type was not established")
        executable = base / "owner-tests"
        shutil.copyfile(original, executable)
        executable.chmod(0o755)
        executable_digest = hashlib.sha256(executable.read_bytes()).hexdigest()
        shutil.copyfile(__file__, base / "probe.py")
        (base / "probe.py").chmod(0o644)
        cases = [run_case(base, executable, mode, *identities) for mode in CASES]
        assert hashlib.sha256(executable.read_bytes()).hexdigest() == executable_digest
    return {
        "platform": {
            key: getattr(os.uname(), key)
            for key in ("sysname", "release", "version", "machine")
        },
        "filesystem": filesystem,
        "executable_sha256": executable_digest,
        "cases": cases,
    }


def main() -> None:
    if not __debug__:
        raise RuntimeError("optimized Python would disable fixture assertions")
    if len(sys.argv) > 1 and sys.argv[1] in ("--owner", "--probe", "--shared"):
        action, root, uid, gid = sys.argv[1:5]
        directory = Path(root)
        if action in ("--probe", "--shared"):
            print(
                json.dumps(
                    probe(directory, int(uid), int(gid), shared=action == "--shared")
                )
            )
        else:
            identity = require_identity(int(uid), int(gid))
            (directory / "owner.json").write_text(json.dumps(identity))
            os.execv(sys.argv[5], [sys.argv[5], "--exact", OWNER_TEST, "--nocapture"])
        return
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo-artifacts", type=Path, required=True)
    parser.add_argument("--disposable", action="store_true")
    arguments = parser.parse_args()
    print(json.dumps(run(arguments.cargo_artifacts, arguments.disposable), indent=2))


if __name__ == "__main__":
    main()
