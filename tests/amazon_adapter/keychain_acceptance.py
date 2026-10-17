#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Opt-in real SystemVault acceptance using one owned synthetic Keychain record.

No Audible backend/account calls, production service/account lookup, enumeration,
network, signing or Keychain configuration changes. Optional UI is disabled only
inside the bounded worker process; the prior process setting is restored.
"""
import argparse
import ctypes
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import uuid

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / "services/optional-audible-adapter/booknook_audible.py"


def write_report(path, value):
    staging = path.with_suffix(".tmp")
    staging.write_text(json.dumps(value, indent=2) + "\n")
    staging.replace(path)


def worker(mode, namespace, account, report_path):
    # The parent generates both UUIDs; cleanup still requires ownership checkpoints.
    uuid.UUID(namespace.removeprefix("qa-atomic-"))
    uuid.UUID(account.removeprefix("synthetic-"))
    if mode not in {"lifecycle", "cleanup"} or not namespace.startswith("qa-atomic-") or not account.startswith("synthetic-"):
        return 2
    spec = importlib.util.spec_from_file_location("vault_acceptance", SOURCE)
    m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)
    from keyring.backends.macOS import api
    get_allowed = api._sec.SecKeychainGetUserInteractionAllowed
    get_allowed.argtypes = (ctypes.POINTER(ctypes.c_bool),); get_allowed.restype = ctypes.c_int32
    set_allowed = api._sec.SecKeychainSetUserInteractionAllowed
    set_allowed.argtypes = (ctypes.c_bool,); set_allowed.restype = ctypes.c_int32
    prior = ctypes.c_bool()
    if get_allowed(ctypes.byref(prior)) or set_allowed(False):
        return 2
    report = json.loads(report_path.read_text())
    if report["namespace"] != namespace or report["account"] != account:
        set_allowed(prior.value); return 2
    def checkpoint(name):
        report["steps"].append(name); write_report(report_path, report)
    owned = False
    vault = None
    result = 1
    try:
        interaction = ctypes.c_bool(True)
        if get_allowed(ctypes.byref(interaction)) or interaction.value:
            raise RuntimeError("interaction_disabled_check_failed")
        vault = m.SystemVault(namespace)
        if not vault.is_macos:
            raise RuntimeError("unexpected_native_backend")
        checkpoint("process_ui_disabled")
        if mode == "cleanup":
            if "initial_missing" not in report["steps"] or "owned_create_begin" not in report["steps"]:
                raise RuntimeError("ownership_not_proven")
            owned = True
        else:
            if vault.backend.get_password(vault.service, account) is not None:
                raise RuntimeError("preexisting_record_refused")
            checkpoint("initial_missing")
            initial = {"credential": {"token": "synthetic only café 雨 🌿", "counter": 1}}
            replacement = {"registration_in_progress": True, "serial": "synthetic replacement 雨"}
            checkpoint("owned_create_begin"); owned = True
            vault.write(account, initial)
            if vault.read(account) != initial: raise RuntimeError("created_json_mismatch")
            checkpoint("create_read_exact")
            reopened = m.SystemVault(namespace)
            if reopened.read(account) != initial: raise RuntimeError("existing_namespace_not_readable")
            checkpoint("existing_json_and_namespace_readable")
            reopened.write(account, replacement)
            if reopened.read(account) != replacement: raise RuntimeError("replacement_json_mismatch")
            checkpoint("atomic_update_read_exact")
            reopened.write(account, {})
            if reopened.read(account) != {}: raise RuntimeError("empty_state_mismatch")
            checkpoint("disconnect_empty_state_compatible")
        result = 0
    except Exception as error:
        report["result"] = "failed_or_blocked"
        # Only controlled error categories, never OS/account messages or record values.
        report["error_type"] = type(error).__name__
    finally:
        if owned and vault is not None:
            try:
                if vault.backend.get_password(vault.service, account) is not None:
                    vault.backend.delete_password(vault.service, account)
                if vault.backend.get_password(vault.service, account) is not None:
                    raise RuntimeError("cleanup_absence_failed")
                checkpoint("owned_item_removed_absence_verified")
            except Exception:
                report["cleanup"] = "blocked_exact_owned_record_only"; result = 1
        restored = set_allowed(prior.value) == 0
        report["process_interaction_restored"] = restored
        if not restored: result = 1
        if result == 0: report["result"] = "passed"
        write_report(report_path, report)
    return result


def main():
    if len(sys.argv) == 6 and sys.argv[1] == "--worker":
        return worker(sys.argv[2], sys.argv[3], sys.argv[4], Path(sys.argv[5]))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--allow-synthetic-keychain-record", action="store_true")
    parser.add_argument("--output-dir", type=Path, required=True)
    args = parser.parse_args()
    if not args.allow_synthetic_keychain_record or sys.platform != "darwin" or os.environ.get("CI"):
        parser.error("Explicit opt-in on local macOS is required; ordinary tests/CI never perform this check.")
    output = args.output_dir.resolve(); output.mkdir(parents=True, exist_ok=False)
    namespace, account = "qa-atomic-" + str(uuid.uuid4()), "synthetic-" + str(uuid.uuid4())
    report_path = output / "result.json"
    write_report(report_path, {"schema": "booknook.optional-audible-keychain-acceptance.v1", "namespace": namespace,
        "account": account, "steps": [], "result": "prepared", "record_kind": "synthetic opaque JSON only; no account credentials",
        "source_sha256": hashlib.sha256(SOURCE.read_bytes()).hexdigest(),
        "harness_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest()})
    command = [sys.executable, str(Path(__file__).resolve()), "--worker", "lifecycle", namespace, account, str(report_path)]
    with (output / "worker.stdout.log").open("w") as stdout, (output / "worker.stderr.log").open("w") as stderr:
        try: code = subprocess.run(command, timeout=20, stdout=stdout, stderr=stderr).returncode
        except subprocess.TimeoutExpired: code = 124
    if code != 0:
        prior = json.loads(report_path.read_text())
        if "initial_missing" in prior["steps"] and "owned_create_begin" in prior["steps"] and "owned_item_removed_absence_verified" not in prior["steps"]:
            command[3] = "cleanup"
            with (output / "cleanup.stdout.log").open("w") as stdout, (output / "cleanup.stderr.log").open("w") as stderr:
                try: subprocess.run(command, timeout=10, stdout=stdout, stderr=stderr)
                except subprocess.TimeoutExpired: pass
    report = json.loads(report_path.read_text()); report["lifecycle_exit_code"] = code
    if code != 0: report["result"] = "failed_or_blocked"
    write_report(report_path, report)
    print(json.dumps({"result": report["result"], "lifecycle_exit_code": code, "evidence": str(report_path),
                      "cleanup_verified": "owned_item_removed_absence_verified" in report["steps"]}))
    return code if code != 0 else (0 if report["result"] == "passed" else 1)


if __name__ == "__main__": sys.exit(main())
