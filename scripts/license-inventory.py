#!/usr/bin/env python3
"""Inventory all Cargo features, including optional bindgen/build/platform dependencies."""
import argparse
import csv
import io
import json
from pathlib import Path
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--check", action="store_true", help="Check the committed inventory without writing.")
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
metadata = json.loads(subprocess.check_output([
    "cargo", "metadata", "--locked", "--offline", "--all-features", "--format-version", "1"
], cwd=root))
output = io.StringIO(newline="")
out = csv.writer(output)
out.writerow(["package", "version", "license", "repository", "scope"])
for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
    out.writerow([
        package["name"], package["version"],
        package["license"] or "REVIEW REQUIRED: " + str(package["license_file"]),
        package["repository"] or "",
        "workspace" if package["id"] in metadata["workspace_members"] else "dependency (all features, build/platform graph)"
    ])
expected = output.getvalue().encode("utf-8")
destination = root / "docs/dependency-licenses.csv"
if args.check:
    if not destination.is_file() or destination.read_bytes() != expected:
        raise SystemExit("Stale dependency license inventory; run scripts/license-inventory.py")
else:
    destination.write_bytes(expected)
review_count = sum(not package["license"] for package in metadata["packages"])
print(f"{'Verified' if args.check else 'Inventoried'} {len(metadata['packages'])} all-feature Rust packages; {review_count} license declarations require review.")
