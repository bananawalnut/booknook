// SPDX-License-Identifier: MIT OR Apache-2.0
import {execFileSync} from 'node:child_process';
import {mkdirSync, writeFileSync} from 'node:fs';
for (const [directory, module, type] of [['cjs', 'commonjs', 'commonjs'], ['esm', 'esnext', 'module']]) {
  execFileSync(process.execPath, ['node_modules/typescript/bin/tsc', '--strict', '--declaration',
    '--target', 'es2022', '--module', module, '--moduleResolution', 'node',
    '--outDir', `dist/${directory}`, 'index.ts'], {stdio:'inherit'});
  mkdirSync(`dist/${directory}`, {recursive:true});
  writeFileSync(`dist/${directory}/package.json`, JSON.stringify({type}) + '\n');
}
