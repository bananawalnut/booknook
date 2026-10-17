#!/bin/bash
# SPDX-License-Identifier: MIT OR Apache-2.0
set -euo pipefail
cd "$(dirname "$0")/.."
export CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0
export NPM_CONFIG_CACHE="$PWD/target/npm-package-cache"
python3 scripts/sync-package-resources.py --check
if [ ! -x sdk/typescript/node_modules/.bin/tsc ]; then npm --prefix sdk/typescript ci; fi
bash scripts/build-wasm.sh web
bash scripts/build-wasm.sh nodejs
npm --prefix sdk/typescript run build
mkdir -p target/packages
npm --prefix sdk/typescript pack ./sdk/typescript --pack-destination target/packages --json > target/packages/npm-pack.json
python3 - <<'PY'
import json, shutil
from pathlib import Path
root=Path.cwd()
output=root/'target/sdk-package-check/npm'
if output.exists(): shutil.rmtree(output)
output.mkdir(parents=True)
(output/'package.json').write_text('{"name":"booknook-installed-sdk-check","private":true,"type":"module"}\n')
for name in ['PackagedSDKSmoke.cjs','PackagedSDKSmoke.mjs','PackagedSDKTypes.mts','PackagedSDKTypes.cts','castalia-generic-fixture.cjs','provider-service-acceptance.cjs']:
    shutil.copy2(root/'tests'/name, output/name)
pack=json.loads((root/'target/packages/npm-pack.json').read_text())[0]
(root/'target/packages/npm-package-path.txt').write_text(str(root/'target/packages'/pack['filename']))
PY
package_tarball="$(cat target/packages/npm-package-path.txt)"
npm --prefix target/sdk-package-check/npm install --offline --ignore-scripts --no-audit --no-fund "$package_tarball"
node target/sdk-package-check/npm/PackagedSDKSmoke.cjs
node target/sdk-package-check/npm/PackagedSDKSmoke.mjs
sdk/typescript/node_modules/.bin/tsc --noEmit --strict --target es2022 --module nodenext --moduleResolution nodenext \
  target/sdk-package-check/npm/PackagedSDKTypes.mts target/sdk-package-check/npm/PackagedSDKTypes.cts
echo 'Installed npm archive passed CommonJS, ESM, browser-WASM initialization and consumer type checks; nothing published.'
