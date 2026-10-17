#!/usr/bin/env python3
"""Mirror canonical licenses/test vectors inside package boundaries; --check never writes."""
from pathlib import Path
import sys
import posixpath
import re

ROOT = Path(__file__).resolve().parents[1]
copies = {}
for package in ["crates/core", "crates/connectors", "crates/native", "crates/wasm", "sdk/typescript", "sdk/swift"]:
    for name in ["LICENSE-MIT", "LICENSE-APACHE"]:
        copies[f"{package}/{name}"] = name
    copies[f"{package}/DEPENDENCY-LICENSES.csv"] = "docs/dependency-licenses.csv"
    copies[f"{package}/LICENSE-BOUNDARIES.md"] = "docs/LICENSING.md"
    copies[f"{package}/INTEGRATION-DECISIONS.md"] = "docs/decisions/integrations.md"
for package in ["core", "connectors", "native"]:
    for name in ["receipts.rs", "social_receipts.rs"]:
        copies[f"crates/{package}/tests/fixtures/{name}"] = f"fixtures/rust/{name}"
copies["crates/core/tests/fixtures/conformance.json"] = "ontology/v0.1.0/conformance.json"
copies["crates/core/tests/fixtures/notebook-ink.json"] = "ontology/v0.2.0/notebook-ink.json"
copies["crates/core/tests/fixtures/note-whiteboard.json"] = "ontology/v0.3.0/note-whiteboard.json"
copies["crates/core/tests/fixtures/portable-archive.json"] = "ontology/portable/v1/conformance.json"
copies["crates/native/tests/fixtures/castalia-custody-vector.json"] = "fixtures/castalia-custody-vector.json"
copies["crates/native/tests/fixtures/castalia-native-authority-profile-v1.json"] = "crates/core/tests/fixtures/castalia/native-authority-profile-v1.json"
copies["crates/native/REFERENCE-ISSUER.md"] = "services/castalia-issuer/README.md"
for destination, source in copies.items():
    expected = (ROOT / source).read_bytes()
    if destination.endswith(".md"):
        # A standalone archive cannot resolve links to a sibling service/repository.
        # Link packaged companions locally; retain other references as explicit source paths.
        package = posixpath.dirname(destination)
        local_copies = {origin: posixpath.basename(target) for target, origin in copies.items()
                        if posixpath.dirname(target) == package}
        def package_link(match):
            label, target = match.groups()
            if "://" in target or target.startswith(("#", "mailto:")):
                return match.group(0)
            path, separator, fragment = target.partition("#")
            origin = posixpath.normpath(posixpath.join(posixpath.dirname(source), path))
            if origin in local_copies:
                suffix = separator + fragment if separator else ""
                return f"[{label}]({local_copies[origin]}{suffix})"
            return f"{label} (source workspace: `{origin}{separator}{fragment}`)"
        expected = re.sub(r"\[([^\]]+)\]\(([^\s)]+)\)", package_link,
                          expected.decode("utf-8")).encode("utf-8")
    output = ROOT / destination
    if "--check" in sys.argv:
        if not output.is_file() or output.read_bytes() != expected:
            raise SystemExit(f"Stale package resource: {destination}; run scripts/sync-package-resources.py")
    else:
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(expected)
print(f"Package resources {'verified' if '--check' in sys.argv else 'synchronized'} ({len(copies)} files).")
