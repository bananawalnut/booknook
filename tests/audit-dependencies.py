#!/usr/bin/env python3
"""Policy/negative controls; real resolved-lock auditor runs are separate evidence."""
import copy
import importlib.util
import json
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("audit_gate", ROOT / "scripts/audit-dependencies.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)


class AuditGateTests(unittest.TestCase):
    def report(self):
        return {"database": {"advisory-count": 1509}, "settings": {"ignore": [], "target_arch": [], "target_os": [], "severity": None, "informational_warnings": ["unmaintained", "unsound", "notice"]}, "vulnerabilities": {"found": False, "count": 0, "list": []}, "warnings": {"unmaintained": [{"advisory": {"id": "RUSTSEC-2026-0215"}}]}}

    def validate(self, report, stderr=b"", exit_code=0):
        return gate.validate_report(json.dumps(report).encode(), stderr, exit_code)

    def test_maintenance_warning_is_retained_and_allowed(self):
        report = self.report()
        self.assertEqual(self.validate(report), report)

    def test_vulnerability_fails_even_if_tool_exit_is_zero(self):
        report = self.report()
        report["vulnerabilities"] = {"found": True, "count": 1, "list": [{}]}
        with self.assertRaises(gate.AuditError):
            self.validate(report)

    def test_unsound_and_yanked_cannot_hide_behind_zero_exit(self):
        for kind in ["unsound", "yanked"]:
            with self.subTest(kind=kind):
                report = self.report()
                report["warnings"][kind] = [{}]
                with self.assertRaises(gate.AuditError):
                    self.validate(report)

    def test_lookup_errors_fail_even_if_no_advisories_match(self):
        with self.assertRaises(gate.AuditError):
            self.validate(self.report(), b"error: couldn't check if the package is yanked: not found\n")

    def test_nonzero_tool_exit_fails(self):
        with self.assertRaises(gate.AuditError):
            self.validate(self.report(), exit_code=1)

    def test_ignores_filters_and_hidden_unsound_class_fail(self):
        mutations = [{"ignore": ["RUSTSEC-2026-0215"]}, {"target_arch": ["x86_64"]}, {"target_os": ["linux"]}, {"severity": "high"}, {"informational_warnings": ["unmaintained"]}]
        for change in mutations:
            with self.subTest(change=change):
                report = self.report()
                report["settings"].update(change)
                with self.assertRaises(gate.AuditError):
                    self.validate(report)

    def test_unknown_warning_and_malformed_or_empty_reports_fail(self):
        report = self.report()
        report["warnings"]["future-category"] = [{}]
        with self.assertRaises(gate.AuditError):
            self.validate(report)
        empty_database = copy.deepcopy(self.report())
        empty_database["database"]["advisory-count"] = 0
        with self.assertRaises(gate.AuditError):
            self.validate(empty_database)
        for raw in [b"{", b"{}", b"null", b"[]", b"\xff"]:
            with self.subTest(raw=raw), self.assertRaises(gate.AuditError):
                gate.validate_report(raw, b"", 0)

    def test_actual_child_oversized_output_is_refused(self):
        with self.assertRaises(gate.AuditError):
            gate.capture([sys.executable, "-c", "import os; os.write(1,b'x'*65536)"], cwd=ROOT, limit=1024)

    def test_actual_child_timeout_is_reaped(self):
        with self.assertRaises(gate.AuditError):
            gate.capture([sys.executable, "-c", "import time; time.sleep(5)"], cwd=ROOT, timeout=0.05)

    def test_exact_published_version_banners_only(self):
        for banner in [b"cargo-audit 0.22.1\n", b"cargo-audit-audit 0.22.1\n"]:
            gate.validate_auditor_version(banner)
        for banner in [b"cargo-audit 0.22.10", b"cargo-audit 0.22.2", b"other 0.22.1", b"cargo-audit-extra 0.22.1", b"cargo-audit 0.22.1 extra"]:
            with self.subTest(banner=banner), self.assertRaises(gate.AuditError):
                gate.validate_auditor_version(banner)

    def test_invocation_is_unfiltered_without_advisory_fetch_plain_and_denies_both_classes(self):
        self.assertEqual(gate.audit_command("cargo-audit", Path("database"), Path("Cargo.lock")), ["cargo-audit", "audit", "--color", "never", "--no-fetch", "--deny", "unsound", "--deny", "yanked", "--json", "--db", "database", "--file", "Cargo.lock"])


if __name__ == "__main__":
    unittest.main()
