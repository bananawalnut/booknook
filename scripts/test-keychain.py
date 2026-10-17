#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Opt-in actual macOS Keychain CRUD; never part of ordinary tests or CI.

Stores synthetic opaque bytes under one fresh random service/account. Uses the
current Swift adapter with only its service literal changed in a staged copy.
The standalone process disables optional Keychain UI and restores its own state.
No credentials are enumerated, production items queried, or settings changed.
"""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time
import uuid


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def run_bounded(command, output, name, seconds):
    started = time.monotonic()
    with (output / f"{name}.stdout.log").open("w") as stdout, \
            (output / f"{name}.stderr.log").open("w") as stderr:
        try:
            result = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=seconds, check=False)
            details = {"exit_code": result.returncode, "timed_out": False}
        except subprocess.TimeoutExpired:
            details = {"exit_code": 124, "timed_out": True}
    details.update(timeout_seconds=seconds, elapsed_seconds=round(time.monotonic() - started, 3))
    write_json(output / f"{name}.execution.json", details)
    return details


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-synthetic-keychain-record", action="store_true",
                        help="Acknowledge creation and exact-account deletion of one new synthetic Keychain item")
    parser.add_argument("--output-dir", type=Path, help="A new evidence directory; existing directories are rejected")
    args = parser.parse_args()
    if not args.allow_synthetic_keychain_record:
        parser.error("No Keychain operation attempted. Explicit --allow-synthetic-keychain-record is required.")
    if sys.platform != "darwin" or os.environ.get("CI"):
        parser.error("This opt-in local macOS acceptance must not run in CI or on other platforms.")

    repo = Path(__file__).resolve().parent.parent
    token = str(uuid.uuid4()).lower()
    output = (args.output_dir or repo / "target" / "keychain-acceptance" / token).resolve()
    output.mkdir(parents=True, exist_ok=False)
    service, account = "org.booknook.qa.keychain." + token, "synthetic-" + str(uuid.uuid4()).lower()
    os.environ.setdefault("DEVELOPER_DIR", "/Applications/Xcode.app/Contents/Developer")
    manifest = {
        "schema": "booknook.keychain-acceptance.v1", "service": service, "account": account,
        "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "system": {"macos": platform.mac_ver()[0], "architecture": platform.machine()},
        "value_classification": "synthetic opaque test bytes, not a validated custody envelope",
        "source_override": "Only CustodySecureStorage.service literal; production source unchanged",
        "harness_difference": "Standalone process sets SecKeychainSetUserInteractionAllowed(false), verifies it, and restores previous process state; deprecated API warnings expected",
        "scope": "One newly-created UUID service/account, no enumeration, production access, trust/settings changes, or UI interaction",
        "limitations": "Not app prompt/signing-denial behavior, complete Castalia custody, real accounts, or physical iPad qualification",
        "sources": {}, "commands": [], "result": "prepared",
    }
    for filename in ("SecureStorage.swift", "RecoveryImport.swift"):
        source = repo / "apps/apple/BookNook" / filename
        data = source.read_bytes()
        if filename == "SecureStorage.swift":
            literal = b'let service = "org.booknook.castalia"'
            if data.count(literal) != 1:
                raise RuntimeError("Adapter service changed; review staging instead of guessing")
            data = data.replace(literal, f'let service = "{service}"'.encode())
        staged = output / filename
        staged.write_bytes(data)
        manifest["sources"][str(source.relative_to(repo))] = {
            "source_sha256": digest(source), "staged_sha256": digest(staged)}
    harness = repo / "tests/KeychainAcceptance.swift"
    manifest["harness_sha256"] = digest(harness)
    manifest["runner_sha256"] = digest(Path(__file__))
    write_json(output / "manifest.json", manifest)
    binary = output / "keychain-acceptance"
    command = ["xcrun", "swiftc", "-parse-as-library", "-swift-version", "5", "-target",
               f"{platform.machine()}-apple-macos14.0", "-module-cache-path", str(output / "module-cache"),
               str(output / "RecoveryImport.swift"), str(output / "SecureStorage.swift"),
               str(harness), "-framework", "Security", "-o", str(binary)]
    manifest["commands"].append(command)
    build = run_bounded(command, output, "build", 120)
    if build["exit_code"] != 0:
        manifest["result"] = "build_failed_no_keychain_operations"
        write_json(output / "manifest.json", manifest)
        print(f"Build failed; see {output}", file=sys.stderr)
        return build["exit_code"]
    manifest["binary_sha256"] = digest(binary)
    report_path = output / "result.json"
    command = [str(binary), "lifecycle", account, str(report_path)]
    manifest["commands"].append(command)
    write_json(output / "manifest.json", manifest)
    print(f"Testing synthetic Keychain CRUD only; optional prompts disabled. Evidence: {output}", flush=True)
    execution = run_bounded(command, output, "lifecycle", 20)
    report = json.loads(report_path.read_text()) if report_path.exists() else {}
    manifest["result"] = report.get("result", "failed_no_report")
    manifest["cleanup"] = report.get("cleanup", "no operation report; inspect execution")
    if execution["timed_out"] or execution["exit_code"] < 0:
        # A killed process cannot run its cleanup. Only delete if its durable
        # checkpoints prove this exact UUID record was initially absent/owned.
        manifest["result"] = "timed_out" if execution["timed_out"] else "process_failed"
        steps = [step.get("step") for step in report.get("steps", [])]
        if "initial_read_missing" in steps and "create_begin" in steps:
            (output / "interrupted-result.json").write_bytes(report_path.read_bytes())
            command = [str(binary), "cleanup", account, str(report_path)]
            manifest["commands"].append(command)
            cleanup = run_bounded(command, output, "cleanup", 10)
            cleanup_report = json.loads(report_path.read_text()) if report_path.exists() else {}
            manifest["cleanup"] = cleanup_report.get("cleanup", "cleanup did not report")
            manifest["cleanup_execution"] = cleanup
        else:
            manifest["cleanup"] = "no owned-write checkpoint; no deletion attempted"
    manifest["source_unchanged_after_test"] = all(
        digest(repo / path) == value["source_sha256"] for path, value in manifest["sources"].items())
    manifest["evidence_sha256"] = {
        path.name: digest(path) for path in sorted(output.iterdir())
        if path.is_file() and path.name != "manifest.json"}
    write_json(output / "manifest.json", manifest)
    print(json.dumps({"result": manifest["result"], "cleanup": manifest["cleanup"],
                      "evidence": str(output), "manifest_sha256": digest(output / "manifest.json")}, indent=2))
    return 0 if (execution["exit_code"] == 0 and manifest["result"] == "passed"
                 and manifest["source_unchanged_after_test"]) else max(1, execution["exit_code"])


if __name__ == "__main__":
    sys.exit(main())
