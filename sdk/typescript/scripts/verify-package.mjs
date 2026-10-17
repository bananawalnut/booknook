// SPDX-License-Identifier: MIT OR Apache-2.0
import {statSync} from 'node:fs';
const required = ['dist/cjs/index.js', 'dist/esm/index.js', 'dist/cjs/index.d.ts', 'dist/esm/index.d.ts',
  'pkg/web/booknook_wasm.js', 'pkg/web/booknook_wasm_bg.wasm', 'pkg/web/booknook_wasm.d.ts',
  'pkg/nodejs/booknook_wasm.js', 'pkg/nodejs/booknook_wasm_bg.wasm', 'pkg/nodejs/booknook_wasm.d.ts',
  'LICENSE-MIT', 'LICENSE-APACHE', 'DEPENDENCY-LICENSES.csv'];
for (const file of required) {
  try { if (statSync(file).size <= 0) throw new Error('empty'); }
  catch { throw new Error(`Missing packaged runtime ${file}. Run bash scripts/package-typescript.sh from the source workspace first.`); }
}
