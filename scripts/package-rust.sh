#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
target_directory="$(python3 scripts/cargo_paths.py)"
python3 scripts/sync-package-resources.py --check
# Cargo 1.96's workspace temporary registry can fail verification with "no hash listed".
# Verify the actual archives below, using local patches only for these unpublished siblings.
cargo package --workspace --allow-dirty --offline --no-verify
python3 scripts/bundle-rust-sdk-patch.py "$target_directory"
python3 - "$target_directory" <<'PY'
from pathlib import Path
import os, shutil, sys, tarfile, tomllib
root = Path.cwd()
target = Path(sys.argv[1])
version = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version']
output = target / 'sdk-package-check/rust'
if output.exists(): shutil.rmtree(output)
output.mkdir(parents=True)
names = ['booknook-core', 'booknook-connectors', 'booknook-native', 'booknook-wasm']
for name in names:
    with tarfile.open(target / f'package/{name}-{version}.crate') as archive:
        archive.extractall(output, filter='data')
# Cargo archives normalize timestamps. Fresh contents extracted at a reused path
# must invalidate Cargo's mtime cache while preserving the actual archive bytes.
for source in output.rglob('*'):
    if source.is_file(): os.utime(source, None)
core = output / f'booknook-core-{version}'
shutil.copyfile(core / 'sdk-workspace.toml', output / 'Cargo.toml')
shutil.copyfile(core / 'Cargo.lock', output / 'Cargo.lock')
print(f'Verifying extracted package files at {output}')
PY
consumer="$target_directory/sdk-package-check/rust"
cargo metadata --locked --offline --format-version 1 --manifest-path "$consumer/Cargo.toml" |
  python3 -c 'import json, pathlib, sys
root = pathlib.Path(sys.argv[1]).resolve()
packages = json.load(sys.stdin)["packages"]
patched = [p for p in packages if p["name"] == "faster-hex"]
if len(patched) != 1 or patched[0]["source"] is not None:
    raise SystemExit("Standalone workspace did not select the bundled faster-hex")
manifest = pathlib.Path(patched[0]["manifest_path"]).resolve()
if manifest != root / ("booknook-core-" + next(p["version"] for p in packages if p["name"] == "booknook-core")) / "vendor/faster-hex/Cargo.toml":
    raise SystemExit("Patched package resolved outside the extracted core archive")
versions = [p["version"] for p in packages if p["name"] == "heapless"]
if versions != ["0.9.4"]:
    raise SystemExit("Standalone SDK resolved an unpatched heapless")' "$consumer"
cargo test --locked --offline --workspace --all-targets \
  --manifest-path "$target_directory/sdk-package-check/rust/Cargo.toml" --target-dir "$target_directory"
echo 'All four local .crate files and their packaged tests/examples verified; nothing published.'
