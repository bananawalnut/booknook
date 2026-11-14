#!/usr/bin/env python3
"""Check both locks without advisory fetch/builds; the caller controls registry offline policy."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import selectors
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[1]
MAX_OUTPUT = 2 * 1024 * 1024
OFFICIAL_DATABASE = "https://github.com/RustSec/advisory-db"


class AuditError(Exception):
    pass


def capture(command, *, cwd, timeout=45, limit=MAX_OUTPUT):
    """Bound each pipe while the process runs, and always reap failed children."""
    with subprocess.Popen(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE) as process:
        output = {"stdout": bytearray(), "stderr": bytearray()}
        deadline = time.monotonic() + timeout
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ, "stdout")
                selector.register(process.stderr, selectors.EVENT_READ, "stderr")
                while selector.get_map():
                    remaining = deadline - time.monotonic()
                    if remaining <= 0:
                        raise AuditError("Auditor process timed out")
                    for key, _ in selector.select(min(remaining, 0.1)):
                        chunk = os.read(key.fileobj.fileno(), 65536)
                        if not chunk:
                            selector.unregister(key.fileobj)
                            continue
                        output[key.data].extend(chunk)
                        if len(output[key.data]) > limit:
                            raise AuditError("Auditor output exceeded its byte bound")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise AuditError("Auditor process timed out")
            return process.wait(timeout=remaining), bytes(output["stdout"]), bytes(output["stderr"])
        except BaseException:
            process.kill()
            process.wait()
            raise


def validate_report(stdout, stderr, exit_code):
    if exit_code != 0:
        raise AuditError(f"cargo-audit exited {exit_code}")
    if any(line.lstrip().lower().startswith("error:") for line in stderr.decode("utf-8", errors="replace").splitlines()):
        raise AuditError("Auditor reported an error, including potentially incomplete yanked lookups")
    try:
        report = json.loads(stdout)
        settings = report["settings"]
        if any(settings.get(key) != [] for key in ["ignore", "target_arch", "target_os"]):
            raise AuditError("Ignored advisory IDs or target filters are forbidden")
        if settings.get("severity", "missing") is not None:
            raise AuditError("Severity filtering is forbidden")
        if not {"unmaintained", "unsound", "notice"}.issubset(settings["informational_warnings"]):
            raise AuditError("Required informational advisory classes are missing")
        vulnerabilities = report["vulnerabilities"]
        if vulnerabilities["found"] is not False or type(vulnerabilities["count"]) is not int or vulnerabilities["count"] != 0 or vulnerabilities["list"] != []:
            raise AuditError("Known vulnerability matches remain")
        if type(report["database"]["advisory-count"]) is not int or report["database"]["advisory-count"] <= 0:
            raise AuditError("Advisory database is empty or malformed")
        warnings = report["warnings"]
        if not isinstance(warnings, dict) or any(not isinstance(value, list) for value in warnings.values()):
            raise AuditError("Malformed warning report")
        for kind, values in warnings.items():
            if kind not in {"unmaintained", "notice", "unsound", "yanked"}:
                raise AuditError("Unknown advisory warning category")
            if values and kind in {"unsound", "yanked"}:
                raise AuditError(f"Denied {kind} findings remain")
        return report
    except (KeyError, TypeError, ValueError, UnicodeError) as error:
        raise AuditError("Missing or malformed cargo-audit JSON") from error


def write_private(path, content):
    path.write_bytes(content)
    path.chmod(0o600)


def validate_auditor_version(stdout):
    if stdout.decode("utf-8").strip() not in {"cargo-audit 0.22.1", "cargo-audit-audit 0.22.1"}:
        raise AuditError("This gate requires the exact cargo-audit 0.22.1 release")


def lock_hash(path):
    if path.is_symlink() or not path.is_file() or path.stat().st_size > 512 * 1024:
        raise AuditError("Lockfile must be a bounded regular file")
    return hashlib.sha256(path.read_bytes()).hexdigest()


def audit_command(auditor, database, lockfile):
    return [auditor, "audit", "--color", "never", "--no-fetch", "--deny", "unsound", "--deny", "yanked", "--json", "--db", str(database), "--file", str(lockfile)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--auditor", default="cargo-audit")
    args = parser.parse_args()
    auditor = shutil.which(args.auditor)
    if auditor is None:
        raise AuditError("Install cargo-audit 0.22.1 before this check")
    database = args.database.resolve(strict=True)
    output = args.output_dir.absolute()
    output.mkdir(mode=0o700, exist_ok=False)
    locks = {"root": ROOT / "Cargo.lock", "relay-read-probe": ROOT / "tests/relay-read-probe/Cargo.lock"}
    before = {name: lock_hash(path) for name, path in locks.items()}

    def git(*arguments):
        code, stdout, _ = capture(["git", "--no-optional-locks", "-C", str(database), *arguments], cwd=output, timeout=10)
        if code:
            raise AuditError("Cannot inspect the supplied advisory database")
        return stdout.decode("utf-8").strip()

    if git("config", "--local", "--get", "remote.origin.url").removesuffix(".git") != OFFICIAL_DATABASE:
        raise AuditError("Advisory database origin must be the official RustSec repository")
    if git("status", "--porcelain"):
        raise AuditError("Advisory database must have a clean worktree")
    database_commit = git("rev-parse", "HEAD")
    code, version, _ = capture([auditor, "audit", "--version"], cwd=output, timeout=10)
    if code:
        raise AuditError("Cannot inspect the installed cargo-audit version")
    validate_auditor_version(version)
    summary = {"schema": "booknook.dependency-advisory-gate.v1", "database_commit": database_commit, "locks": {}, "errors": []}
    for name, path in locks.items():
        command = audit_command(auditor, database, path)
        try:
            code, stdout, stderr = capture(command, cwd=output)
            write_private(output / f"{name}.stdout.json", stdout)
            write_private(output / f"{name}.stderr.log", stderr)
            print(f"\n{name}: {path.relative_to(ROOT)}", flush=True)
            print(stdout.decode("utf-8", errors="replace"), flush=True)
            print(stderr.decode("utf-8", errors="replace"), file=sys.stderr, flush=True)
            report = validate_report(stdout, stderr, code)
            summary["locks"][name] = {"sha256": before[name], "exit_code": code, "warning_counts": {key: len(value) for key, value in report["warnings"].items()}}
        except (AuditError, OSError, subprocess.SubprocessError) as error:
            summary["errors"].append(f"{name}: {error}")
    if any(lock_hash(path) != before[name] for name, path in locks.items()):
        summary["errors"].append("A lockfile changed during the audit")
    if git("rev-parse", "HEAD") != database_commit:
        summary["errors"].append("Advisory database changed during the audit")
    if git("status", "--porcelain"):
        summary["errors"].append("Advisory database worktree changed during the audit")
    write_private(output / "summary.json", (json.dumps(summary, indent=2) + "\n").encode())
    for error in summary["errors"]:
        print(error, file=sys.stderr)
    return bool(summary["errors"])


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (AuditError, OSError, subprocess.SubprocessError) as error:
        sys.exit(str(error))
