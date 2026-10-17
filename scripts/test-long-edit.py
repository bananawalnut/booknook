#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Opt-in bounded macOS TextKit/autosave acceptance with measured timings.

Requires a new explicit output directory and an existing native release archive.
Compiles current app sources; changes no production source, opens no window and
uses no global input, account, network or Keychain. Only integrity/setup/timeouts
fail acceptance; measured timings have no performance thresholds. This script is
not invoked by the ordinary test suite. All synthetic storage/evidence is kept.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import sqlite3
import subprocess
import sys
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def bounded(command, log, timeout, env):
    started = time.monotonic()
    with log.open("x") as output:
        try:
            result = subprocess.run(command, env=env, stdout=output,
                                    stderr=subprocess.STDOUT, timeout=timeout, check=False)
            details = {"exit_code": result.returncode, "timed_out": False}
        except subprocess.TimeoutExpired:
            details = {"exit_code": 124, "timed_out": True}
    return dict(details, elapsed_seconds=time.monotonic() - started, timeout_seconds=timeout)


def inspect_storage(path):
    with sqlite3.connect(path.as_uri() + "?mode=ro", uri=True) as connection:
        connection.execute("PRAGMA query_only=ON")
        revision, characters, size = connection.execute(
            "SELECT revision,length(json),length(CAST(json AS BLOB)) FROM desk WHERE singleton=1"
        ).fetchone()
        pending = connection.execute("SELECT count(*) FROM pending_edits").fetchone()[0]
        integrity = connection.execute("PRAGMA integrity_check").fetchone()[0]
    return {"revision": revision, "state_json_unicode_characters": characters,
            "state_json_utf8_bytes": size, "pending_edits": pending,
            "sqlite_integrity_check": integrity, "sqlite_file_bytes": path.stat().st_size,
            "inspection": "Read-only SQLite connection after the harness exited"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, required=True,
                        help="New evidence directory; existing paths are rejected without cleanup")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("The actual TextKit acceptance requires macOS.")
    repo = Path(__file__).resolve().parent.parent
    output = args.output_dir.expanduser().resolve()
    if output.exists() or args.output_dir.is_symlink():
        parser.error("Output must be a fresh path. No existing files were changed.")
    archive = repo / "target/release/libbooknook_native.a"
    generated = repo / "apps/apple/Generated"
    required = [archive, generated / "booknook_native.swift", generated / "booknook_nativeFFI.h"]
    missing = [str(path) for path in required if not path.is_file()]
    if missing:
        parser.error("Build the native release archive and Swift bindings first: " + ", ".join(missing))
    output.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ)
    env.setdefault("DEVELOPER_DIR", "/Applications/Xcode.app/Contents/Developer")
    sources = sorted((repo / "apps/apple/BookNook").glob("*.swift"))
    harness = repo / "tests/LongEditAcceptance.swift"
    rust_inputs = sorted((repo / "crates").glob("*/src/**/*.rs"))
    rust_inputs += sorted((repo / "crates").glob("*/Cargo.toml"))
    inputs = sorted(set([*sources, *required, *rust_inputs, repo / "Cargo.toml",
                         repo / "Cargo.lock", harness, Path(__file__).resolve()]))
    hashes = {str(path.relative_to(repo)): digest(path) for path in inputs}
    command = ["xcrun", "swiftc", "-parse-as-library", "-D", "BOOKNOOK_TEST", "-swift-version", "5",
               "-module-name", "LongEditAcceptance", "-module-cache-path", str(output / "module-cache"),
               "-import-objc-header", str(generated / "booknook_nativeFFI.h"),
               "-I", str(generated), str(generated / "booknook_native.swift"),
               *map(str, sources), str(harness), str(archive), "-lsqlite3", "-lc++",
               "-framework", "Security", "-o", str(output / "probe")]
    manifest = {
        "schema": "booknook.long-edit-acceptance.v1", "result": "prepared",
        "created_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "host": {"macos": platform.mac_ver()[0], "architecture": platform.machine()},
        "developer_dir": env["DEVELOPER_DIR"], "source_sha256": hashes,
        "commands": [command, [str(output / "probe"), str(output)]],
        "swift_optimization": "Unoptimized development Swift, linking existing release Rust",
        "storage_override": str(output / "synthetic-desk"),
        "production_source_changes": False,
        "timing_policy": "Observations only; fail on integrity/setup/timeout, not timing percentiles",
        "limits": "No visible window, global input, SwiftUI body/layout/word-count, physical keyboard/IME/Pencil, network, account or Keychain qualification. Synthetic single-block workload; observer and heartbeat add overhead.",
    }
    canonical = repo / "docs/build-inputs.json"
    if canonical.is_file():
        recorded = json.loads(canonical.read_text())
        files = recorded.get("files", {})
        matched = {name: old for name, old in files.items() if name in hashes}
        manifest["canonical_build_comparison"] = {
            "sha256": digest(canonical), "source_files_compared": len(matched),
            "source_drift": [name for name, old in matched.items() if hashes[name] != old],
            "release_archive_matches": recorded.get("artifacts", {}).get(
                "target/release/libbooknook_native.a", {}).get("sha256") == digest(archive),
            "qualification": "Comparison only; rebuild the archive/bindings when native sources change. This runner does not rebuild Rust.",
        }
    write_json(output / "manifest.json", manifest)
    try:
        manifest["build"] = bounded(command, output / "build.log", 180, env)
        if manifest["build"]["exit_code"] != 0:
            raise RuntimeError("Compilation failed or timed out; see build.log")
        manifest["binary_sha256"] = digest(output / "probe")
        manifest["execution"] = bounded(manifest["commands"][1], output / "execution.log", 180, env)
        if manifest["execution"]["exit_code"] != 0:
            raise RuntimeError("Acceptance failed or timed out; see execution.log and result.json")
        result = json.loads((output / "result.json").read_text())
        if result.get("result") != "passed":
            raise RuntimeError("Harness did not report success")
        storage = inspect_storage(output / "synthetic-desk/booknook.sqlite")
        write_json(output / "storage-observation.json", storage)
        if storage["pending_edits"] != 0 or storage["sqlite_integrity_check"] != "ok":
            raise RuntimeError("Pending edits remain or SQLite integrity failed")
        manifest["result"] = "passed"
    except Exception as error:
        manifest["result"] = "failed"
        manifest["failure"] = str(error)
    finally:
        manifest["source_unchanged"] = all((repo / name).is_file() and digest(repo / name) == old
                                            for name, old in hashes.items())
        if not manifest["source_unchanged"]:
            manifest["result"] = "failed"
            manifest["failure"] = "Measured inputs changed during acceptance"
        manifest["evidence_sha256"] = {name: digest(output / name) for name in
            ["build.log", "execution.log", "result.json", "storage-observation.json"]
            if (output / name).is_file()}
        write_json(output / "manifest.json", manifest)
    print(f"Long-edit acceptance: {manifest['result']}; evidence: {output}")
    return 0 if manifest["result"] == "passed" else 1


if __name__ == "__main__":
    sys.exit(main())
