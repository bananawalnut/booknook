#!/usr/bin/env python3
"""Copy verified local SDK packages into a user-facing directory; never upload anything."""
from pathlib import Path
import argparse
import hashlib
import shutil
import tomllib

ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, default=ROOT.parent / 'booknook-sdk')
args = parser.parse_args()
version = tomllib.loads((ROOT / 'Cargo.toml').read_text())['workspace']['package']['version']
sources = [ROOT / f'target/package/booknook-{name}-{version}.crate'
           for name in ['core', 'connectors', 'native', 'wasm']]
sources += [ROOT / f'target/packages/booknook-sdk-{version}.tgz',
            ROOT / f'target/packages/booknook-swift-sdk-{version}.tar.gz']
for source in sources:
    if not source.is_file():
        raise SystemExit(f'Missing package {source}; complete the package scripts first')
args.output.mkdir(parents=True, exist_ok=True)
lines = []
for source in sources:
    destination = args.output / source.name
    shutil.copy2(source, destination)
    lines.append(f'{hashlib.sha256(destination.read_bytes()).hexdigest()}  {destination.name}')
(args.output / 'SHA256SUMS').write_text('\n'.join(lines) + '\n')
(args.output / 'README.md').write_text(f'''# Book Nook SDKs {version}

These local package artifacts were created without publishing to npm, crates.io, GitHub, or an Apple distribution service. Verify file integrity with `shasum -a 256 -c SHA256SUMS`.

- Four `.crate` files contain the Rust core, connectors, native bridge and WASM source. The Book Nook siblings are versioned but are not assumed to exist in a registry. Use the complete source workspace or extract all four packages into one local workspace with `[patch.crates-io]` paths. `scripts/package-rust.sh` creates that exact verification workspace and runs the packaged tests/examples.
- `booknook-sdk-{version}.tgz` installs with `npm install /path/to/booknook-sdk-{version}.tgz`. It includes compiled ESM/CommonJS facade code, declarations and both browser/Node WASM runtimes. Its installed archive is exercised by `scripts/package-typescript.sh`.
- `booknook-swift-sdk-{version}.tar.gz` extracts into a standalone local Swift Package. Add that directory in Xcode or SwiftPM. It includes arm64 macOS, iOS device and iOS Simulator static slices; no Intel slice, code signing or device qualification is claimed. `scripts/package-swift.sh` links and executes a separate Swift consumer.

The source workspace, build scripts, SDK guide and decision/validation logs are in the adjacent `booknook` directory. SDK code is MIT OR Apache-2.0; dependencies retain their declared licenses. Each package includes license texts and the reviewed inventory boundaries. Provider account qualification and physical-device acceptance remain distinct from package compilation and installation.
''')
print(f'Staged {len(sources)} local SDK packages at {args.output}')
