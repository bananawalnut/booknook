#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
python3 scripts/sync-package-resources.py --check
# Cargo 1.96's workspace temporary registry can fail verification with "no hash listed".
# Verify the actual archives below, using local patches only for these unpublished siblings.
cargo package --workspace --allow-dirty --offline --no-verify
python3 - <<'PY'
from pathlib import Path
import os, shutil, tarfile, tomllib
root = Path.cwd()
version = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version']
output = root / 'target/sdk-package-check/rust'
if output.exists(): shutil.rmtree(output)
output.mkdir(parents=True)
names = ['booknook-core', 'booknook-connectors', 'booknook-native', 'booknook-wasm']
for name in names:
    with tarfile.open(root / f'target/package/{name}-{version}.crate') as archive:
        archive.extractall(output, filter='data')
# Cargo archives normalize timestamps. Fresh contents extracted at a reused path
# must invalidate Cargo's mtime cache while preserving the actual archive bytes.
for source in output.rglob('*'):
    if source.is_file(): os.utime(source, None)
members = ', '.join(f'"{name}-{version}"' for name in names)
patches = '\n'.join(f'{name} = {{ path = "{name}-{version}" }}' for name in names)
(output / 'Cargo.toml').write_text(f'[workspace]\nresolver = "2"\nmembers = [{members}]\n\n[patch.crates-io]\n{patches}\n')
shutil.copy2(root / 'Cargo.lock', output / 'Cargo.lock')
print(f'Verifying extracted package files at {output}')
PY
cargo test --offline --workspace --all-targets \
  --manifest-path target/sdk-package-check/rust/Cargo.toml --target-dir target
echo 'All four local .crate files and their packaged tests/examples verified; nothing published.'
