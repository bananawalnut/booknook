#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Exercise allowlisted Pages staging; never deploy or change GitHub settings."""
import hashlib
import importlib.util
import json
import os
import shutil
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location("stage_ontology_pages", ROOT / "scripts/stage-ontology-pages.py")
PAGES = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PAGES)


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value) + "\n")


def fixture(root):
    discovery = {"schema": "booknook.ontology-discovery.v1", "current": "0.1.0"}
    for group, prefix, version in (("releases", "v0.1.0", "0.1.0"),
                                   ("provider_contracts", "providers/v1", "1"),
                                   ("portable_contracts", "portable/v1", "1")):
        raw = b'{"public":"fixture"}\n'
        artifact = root / prefix / "conformance.json"
        artifact.parent.mkdir(parents=True)
        artifact.write_bytes(raw)
        write_json(root / prefix / "manifest.json", {"schema": "fixture.manifest.v1", "version": version,
                   "artifacts": [{"path": "conformance.json", "sha256": hashlib.sha256(raw).hexdigest()}]})
        discovery[group] = [{"version": version, "manifest": prefix + "/manifest.json"}]
    write_json(root / "discovery.json", discovery)


class StagingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="booknook-pages-")
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name).resolve()
        self.source = self.directory / "ontology"
        fixture(self.source)
        issued_manifests = patch.object(PAGES, "ISSUED_MANIFESTS", {})
        issued_manifests.start()
        self.addCleanup(issued_manifests.stop)
        self.output = self.directory / "site"

    def test_only_recursive_closure_is_copied_from_arbitrary_private_cwd(self):
        private = self.directory / "private-cwd"
        private.mkdir()
        (private / "account.json").write_text("private account sentinel")
        (self.source / "unreferenced.json").write_text("private source sentinel")
        (self.source / ".env").write_text("private environment sentinel")
        previous = Path.cwd()
        try:
            os.chdir(private)
            report = PAGES.stage(self.source, self.output)
        finally:
            os.chdir(previous)
        expected = PAGES.directory_inventory(self.source)
        files = {path.relative_to(self.output).as_posix() for path in self.output.rglob("*") if path.is_file()}
        self.assertEqual(files, {".nojekyll", "index.html"} | {"ontology/" + name for name in expected["files"]})
        self.assertEqual(report["inventory"], expected)
        self.assertEqual(PAGES.directory_inventory(self.output / "ontology"), expected)
        for name in expected["files"]:
            self.assertEqual((self.output / "ontology" / name).read_bytes(), (self.source / name).read_bytes())
        self.assertNotIn(b"<script", (self.output / "index.html").read_bytes().lower())

    def test_corrupt_nested_provider_artifact_leaves_no_output(self):
        with (self.source / "providers/v1/conformance.json").open("ab") as stream:
            stream.write(b" ")
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            PAGES.stage(self.source, self.output)
        self.assertFalse(self.output.exists())

    def test_unsafe_manifest_references_are_refused(self):
        manifest = self.source / "portable/v1/manifest.json"
        original = json.loads(manifest.read_text())
        for value in ("../private.json", "/private.json", "https://outside.invalid/private.json", "%2e%2e/private.json"):
            changed = json.loads(json.dumps(original))
            changed["artifacts"][0]["path"] = value
            write_json(manifest, changed)
            with self.subTest(value=value), self.assertRaises(ValueError):
                PAGES.stage(self.source, self.output)
            self.assertFalse(self.output.exists())

    def test_symlinked_artifact_directory_or_source_is_refused(self):
        artifact = self.source / "v0.1.0/conformance.json"
        external = self.directory / "private.json"
        artifact.rename(external)
        artifact.symlink_to(external)
        with self.assertRaises((ValueError, OSError)):
            PAGES.stage(self.source, self.output)
        artifact.unlink()
        external.rename(artifact)
        external_directory = self.directory / "external-provider"
        (self.source / "providers").rename(external_directory)
        (self.source / "providers").symlink_to(external_directory, target_is_directory=True)
        with self.assertRaises((ValueError, OSError)):
            PAGES.stage(self.source, self.output)
        alias = self.directory / "source-alias"
        alias.symlink_to(self.source, target_is_directory=True)
        with self.assertRaises((ValueError, OSError)):
            PAGES.stage(alias, self.output)
        self.assertFalse(self.output.exists())

    def test_fifo_and_hardlinked_inputs_are_refused(self):
        artifact = self.source / "v0.1.0/conformance.json"
        raw = artifact.read_bytes()
        artifact.unlink()
        os.mkfifo(artifact)
        with self.assertRaises((ValueError, OSError)):
            PAGES.stage(self.source, self.output)
        artifact.unlink()
        artifact.write_bytes(raw)
        os.link(artifact, self.directory / "private-hardlink")
        with self.assertRaises((ValueError, OSError)):
            PAGES.stage(self.source, self.output)
        self.assertFalse(self.output.exists())

    def test_existing_or_symlinked_output_is_not_overwritten(self):
        self.output.mkdir()
        sentinel = self.output / "keep.txt"
        sentinel.write_bytes(b"keep")
        with self.assertRaises((ValueError, FileExistsError)):
            PAGES.stage(self.source, self.output)
        self.assertEqual(sentinel.read_bytes(), b"keep")
        alias = self.directory / "output-alias"
        alias.symlink_to(self.output, target_is_directory=True)
        with self.assertRaises((ValueError, FileExistsError)):
            PAGES.stage(self.source, alias)
        self.assertEqual(sentinel.read_bytes(), b"keep")

    def test_output_inside_source_is_refused(self):
        with self.assertRaises(ValueError):
            PAGES.stage(self.source, self.source / "published")
        self.assertFalse((self.source / "published").exists())

    def test_snapshot_limits_fail_whole_stage(self):
        for name, limit in (("MAX_FILE_BYTES", 8), ("MAX_TOTAL_BYTES", 32), ("MAX_FILES", 2)):
            with self.subTest(limit=name), patch.object(PAGES, name, limit), self.assertRaises(ValueError):
                PAGES.stage(self.source, self.output)
            self.assertFalse(self.output.exists())

    def test_additional_discovery_group_is_not_silently_omitted(self):
        raw = b'{"new":"public"}'
        artifact = self.source / "future/v1/fixture.json"
        artifact.parent.mkdir(parents=True)
        artifact.write_bytes(raw)
        write_json(artifact.parent / "manifest.json", {"schema": "fixture.manifest.v1", "version": "1",
                   "artifacts": [{"path": "fixture.json", "sha256": hashlib.sha256(raw).hexdigest()}]})
        discovery = json.loads((self.source / "discovery.json").read_text())
        discovery["future_contracts"] = [{"version": "1", "manifest": "future/v1/manifest.json"}]
        write_json(self.source / "discovery.json", discovery)
        result = PAGES.stage(self.source, self.output)
        self.assertEqual(len(result["inventory"]["manifests"]), 4)
        self.assertEqual((self.output / "ontology/future/v1/fixture.json").read_bytes(), raw)

    def test_failed_write_does_not_expose_partial_stage(self):
        original = Path.write_bytes

        def failed_index(path, raw):
            if path.name == "index.html":
                raise OSError("synthetic staging write failure")
            return original(path, raw)

        with patch.object(Path, "write_bytes", failed_index), self.assertRaisesRegex(OSError, "synthetic"):
            PAGES.stage(self.source, self.output)
        self.assertFalse(self.output.exists())
        self.assertFalse(list(self.directory.glob(".booknook-ontology-*")))


class IssuedManifestTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="booknook-issued-ontology-")
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name).resolve()
        self.source = root / "ontology"
        self.output = root / "site"
        shutil.copytree(ROOT / "ontology", self.source)

    def test_new_release_can_be_discovered_without_rewriting_issued_manifests(self):
        folder = self.source / "future-release"
        artifact = folder / "test.json"
        write_json(artifact, {"future": True})
        write_json(folder / "manifest.json", {"schema": "fixture.manifest.v1", "version": "future",
                   "artifacts": [{"path": artifact.name, "sha256": hashlib.sha256(artifact.read_bytes()).hexdigest()}]})
        path = self.source / "discovery.json"
        discovery = json.loads(path.read_text())
        discovery["releases"].append({"version": "future", "manifest": "future-release/manifest.json"})
        discovery["current"] = "future"
        write_json(path, discovery)
        report = PAGES.stage(self.source, self.output)
        self.assertEqual(report["inventory"]["current"], "future")

    def test_consistent_rewrite_of_issued_manifest_and_artifact_is_refused(self):
        path = self.source / "commands/v1/manifest.json"
        manifest = json.loads(path.read_text())
        artifact = manifest["artifacts"][0]
        changed = path.parent / artifact["path"]
        changed.write_bytes(changed.read_bytes() + b" ")
        artifact["sha256"] = hashlib.sha256(changed.read_bytes()).hexdigest()
        write_json(path, manifest)
        PAGES.directory_inventory(self.source)
        with self.assertRaisesRegex(ValueError, "issued ontology manifest: commands/v1/manifest.json"):
            PAGES.stage(self.source, self.output)
        self.assertFalse(self.output.exists())

    def test_removing_issued_discovery_link_leaves_no_output(self):
        path = self.source / "discovery.json"
        discovery = json.loads(path.read_text())
        discovery["releases"] = [entry for entry in discovery["releases"] if entry["version"] != "0.1.0"]
        write_json(path, discovery)
        with self.assertRaisesRegex(ValueError, "issued ontology manifest: v0.1.0/manifest.json"):
            PAGES.stage(self.source, self.output)
        self.assertFalse(self.output.exists())


    def test_moving_issued_discovery_link_to_a_different_group_is_refused(self):
        path = self.source / "discovery.json"
        discovery = json.loads(path.read_text())
        discovery["retired_contracts"] = discovery.pop("command_contracts")
        write_json(path, discovery)
        with self.assertRaisesRegex(ValueError, "issued ontology manifest: commands/v1/manifest.json"):
            PAGES.stage(self.source, self.output)
        self.assertFalse(self.output.exists())


if __name__ == "__main__":
    unittest.main()
