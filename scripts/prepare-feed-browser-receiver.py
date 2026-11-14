#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Prepare an isolated, test-only browser receiver from exact packaged SDK bytes."""
from pathlib import Path
import argparse
import hashlib
import io
import json
import tarfile


def prepare(package: Path, output: Path) -> dict:
    if output.exists() or output.is_symlink():
        raise ValueError("Use an unused fixture output directory; existing files are never replaced")
    root = Path(__file__).resolve().parents[1]
    selected = {
        "package/dist/esm/feeds.js": "sdk/feeds.js",
        "package/dist/esm/contract-json.js": "sdk/contract-json.js",
        "package/pkg/web/booknook_wasm.js": "wasm/booknook_wasm.js",
        "package/pkg/web/booknook_wasm_bg.wasm": "wasm/booknook_wasm_bg.wasm",
    }
    contents = {}
    package_bytes = package.read_bytes()
    if len(package_bytes) > 32 * 1024 * 1024:
        raise ValueError("SDK fixture archive exceeds its byte limit")
    with tarfile.open(fileobj=io.BytesIO(package_bytes)) as archive:
        members = archive.getmembers()
        for source, destination in selected.items():
            found = [m for m in members if m.name == source]
            if len(found) != 1 or not found[0].isfile() or not 0 < found[0].size <= 16 * 1024 * 1024:
                raise ValueError("Missing, duplicate or unsupported SDK fixture member: " + source)
            contents[destination] = archive.extractfile(found[0]).read()
        metadata_members = [m for m in members if m.name == "package/package.json"]
        if len(metadata_members) != 1 or not metadata_members[0].isfile() or metadata_members[0].size > 65536:
            raise ValueError("Unsupported package metadata")
        metadata = json.loads(archive.extractfile(metadata_members[0]).read())
        if metadata.get("name") != "@booknook/sdk":
            raise ValueError("Expected the Book Nook SDK archive")
    for name in ["index.html", "receiver.mjs"]:
        contents[name] = (root / "tests/browser-feed-receiver" / name).read_bytes()
    output.mkdir(parents=True, mode=0o700)
    hashes = {}
    for name, content in contents.items():
        destination = output / name
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(content)
        hashes[name] = hashlib.sha256(content).hexdigest()
    receipt = {"schema": "booknook.browser-feed-receiver-fixture.v1", "fixtureOnly": True,
               "sdkVersion": metadata["version"], "sdkArchiveSha256": hashlib.sha256(package_bytes).hexdigest(),
               "files": hashes, "productionQualification": False}
    (output / "fixture-provenance.json").write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(prepare(args.package, args.output), indent=2))
