#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Exercise the real optional static host and complete discovery graph on loopback."""
import argparse
import hashlib
import json
from pathlib import Path
import selectors
import subprocess
import sys
import tempfile
from urllib.error import HTTPError
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
from ontology_artifacts import directory_inventory, inventory  # noqa: E402


def run():
    expected = directory_inventory(ROOT / "ontology")
    # A corrupt artifact in a nested provider manifest must fail, not be skipped.
    nested = next(name for name in expected["files"] if name.startswith("providers/") and name.endswith(".schema.json"))
    def corrupted_read(name):
        raw = (ROOT / "ontology" / name).read_bytes()
        return raw + b"\n" if name == nested else raw
    try:
        inventory(corrupted_read)
    except ValueError as error:
        assert str(error) == f"Ontology checksum mismatch: {nested}", error
    else:
        raise AssertionError("Corrupt nested artifact was accepted")

    with tempfile.TemporaryDirectory(prefix="booknook-ontology-host-") as temporary:
        sentinel = Path(temporary) / "private-cwd-sentinel.txt"
        sentinel.write_text("not an ontology resource")
        with tempfile.TemporaryFile(mode="w+b") as errors:
            process = subprocess.Popen([sys.executable, str(ROOT / "scripts/serve-ontology.py"), "--port", "0"],
                                       cwd=temporary, stdout=subprocess.PIPE, stderr=errors)
            try:
                with selectors.DefaultSelector() as selector:
                    selector.register(process.stdout, selectors.EVENT_READ)
                    assert selector.select(timeout=10), "Static host did not start"
                    line = process.stdout.readline().decode().strip()
                prefix = "Serving bundled ontology at "
                if not (line.startswith(prefix) and line.endswith("discovery.json")):
                    errors.seek(0)
                    raise AssertionError(f"Static host failed to start: {line}\n{errors.read().decode()}")
                base = line[len(prefix):-len("discovery.json")]
                assert base.startswith("http://127.0.0.1:"), base
                def fetch(name):
                    with urlopen(base + name, timeout=5) as response:
                        assert response.status == 200
                        return response.read()
                actual = inventory(fetch)
                assert actual == expected, "HTTP discovery graph differs from offline resources"
                try:
                    fetch(sentinel.name)
                except HTTPError as error:
                    assert error.code == 404
                else:
                    raise AssertionError("Host exposed a file from its working directory")
            finally:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
                process.stdout.close()
    return {"schema": "booknook.ontology-host-acceptance.v1", "transport": "loopback HTTP",
            "private_working_directory_file": "404", "corrupt_nested_artifact": "rejected",
            "inputs": {name: hashlib.sha256((ROOT / name).read_bytes()).hexdigest() for name in
                       ["scripts/serve-ontology.py", "scripts/ontology_artifacts.py", "tests/ontology-host.py"]},
            **actual}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--receipt", type=Path)
    args = parser.parse_args()
    result = run()
    if args.receipt:
        args.receipt.parent.mkdir(parents=True, exist_ok=True)
        args.receipt.write_text(json.dumps(result, indent=2) + "\n")
    print(f"Static HTTP discovery passed: {len(result['manifests'])} manifests, {result['artifact_count']} checked artifacts; corrupt provider artifact rejected; private cwd excluded.")
