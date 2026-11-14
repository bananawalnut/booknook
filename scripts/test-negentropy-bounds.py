#!/usr/bin/env python3
"""Exercise the actual locked dependency with an allocation guard, without OOM.

No workspace manifest/dependency changes or shared target are needed. CI runs
cargo fetch --locked first; metadata and this isolated build are offline.
"""
from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
FIXTURE = ROOT / "tests/security/negentropy-bounds.rs"
LIMIT = 1024 * 1024


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command: list[str], *, cwd: Path, env: dict[str, str], timeout: int = 180) -> subprocess.CompletedProcess[str]:
    result = subprocess.run(command, cwd=cwd, env=env, text=True, capture_output=True, timeout=timeout)
    if result.returncode:
        raise RuntimeError(f"{command[0]} exited {result.returncode}:\n{result.stdout[-4000:]}\n{result.stderr[-4000:]}")
    return result


def main() -> None:
    before = digest(ROOT / "Cargo.lock")
    with tempfile.TemporaryDirectory(prefix="booknook-negentropy-bounds-") as directory:
        temp = Path(directory)
        env = os.environ.copy()
        env.update(CARGO_TARGET_DIR=str(temp / "target"), CARGO_INCREMENTAL="0", CARGO_PROFILE_DEV_DEBUG="0", CARGO_PROFILE_TEST_DEBUG="0")
        env.pop("CARGO_BUILD_TARGET", None)  # This local executable must target the host.
        metadata = json.loads(run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1", "--manifest-path", str(ROOT / "Cargo.toml")], cwd=ROOT, env=env).stdout)
        nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
        pending = list(metadata["workspace_members"])
        reachable = set()
        while pending:
            package_id = pending.pop()
            if package_id not in reachable:
                reachable.add(package_id)
                pending.extend(nodes[package_id]["dependencies"])
        selected = [package for package in metadata["packages"] if package["id"] in reachable and package["name"] == "negentropy"]
        if len(selected) != 1:
            raise RuntimeError(f"Expected one workspace-resolved negentropy package, found {len(selected)}")
        package = selected[0]
        manifest = Path(package["manifest_path"]).resolve()
        (temp / "src").mkdir()
        shutil.copyfile(FIXTURE, temp / "src/main.rs")
        # JSON quoting is valid for these TOML basic strings; no shell is involved.
        dependency_path = json.dumps(str(manifest.parent), ensure_ascii=False)
        (temp / "Cargo.toml").write_text(
            '[package]\nname = "booknook-negentropy-bounds"\nversion = "0.0.0"\nedition = "2021"\npublish = false\n'
            '[workspace]\n[dependencies.negentropy]\npath = ' + dependency_path + '\ndefault-features = false\nfeatures = ["std"]\n'
        )
        run(["cargo", "build", "--offline", "--manifest-path", str(temp / "Cargo.toml")], cwd=temp, env=env)
        executable = temp / "target/debug/booknook-negentropy-bounds"
        cases = []
        for mode in ("responder", "initiator"):
            result = run([str(executable), mode], cwd=temp, env=env, timeout=10)
            fields = dict(part.split("=", 1) for part in result.stdout.strip().split() if "=" in part)
            if fields.get("mode") != mode or fields.get("result") != "ParseEndsPrematurely" or int(fields.get("limit", "0")) != LIMIT or not 0 < int(fields.get("max_allocation", "0")) <= LIMIT or result.stderr:
                raise RuntimeError(f"Unexpected guarded parser result: {result.stdout!r} {result.stderr!r}")
            cases.append({"mode": mode, "result": fields["result"], "maximum_observed_allocation_bytes": int(fields["max_allocation"]), "exit_code": result.returncode})
        if digest(ROOT / "Cargo.lock") != before:
            raise RuntimeError("Workspace Cargo.lock changed during the isolated check")
        print(json.dumps({"result": "pass", "resolved_version": package["version"], "resolved_package_id": package["id"], "std_enabled_in_probe": True, "workspace_lock_sha256": before, "resolved_manifest_sha256": digest(manifest), "fixture_sha256": digest(FIXTURE), "allocation_guard_bytes": LIMIT, "cases": cases}, indent=2))


if __name__ == "__main__":
    main()
