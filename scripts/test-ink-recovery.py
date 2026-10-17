#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Opt-in actual process-kill acceptance on a fresh synthetic desk; never user data."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import time

repo = Path(__file__).resolve().parents[1]
p = argparse.ArgumentParser(description=__doc__)
p.add_argument('--output-dir', required=True, type=Path)
p.add_argument('--binary', type=Path, default=repo / 'target/ui-workflow-tests/desk')
a = p.parse_args()
output = a.output_dir.resolve()
output.mkdir(parents=True, exist_ok=False)
binary = a.binary.resolve(strict=True)
desk = output / 'synthetic-desk'
desk.mkdir()
def sha(path): return hashlib.sha256(path.read_bytes()).hexdigest()
inputs = list((repo/'apps/apple/BookNook').glob('*.swift')) + [repo/'tests/InkRecoveryAcceptance.swift', repo/'tests/DeskWorkflowTests.swift', Path(__file__), binary]
manifest = {'schema': 'booknook.ink-recovery-process-test.v1', 'command': [str(binary), '--ink-journal-child', str(desk)],
            'inputs': {str(f.relative_to(repo)): sha(f) for f in inputs if f.is_relative_to(repo)},
            'limits': 'Synthetic task-only desk. SIGKILL after acknowledged file+directory fsync; not a power-loss guarantee or physical Pencil test.'}
(output/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
started = time.monotonic()
with (output/'child.log').open('wb') as log:
    child = subprocess.Popen(manifest['command'], stdout=log, stderr=subprocess.STDOUT, cwd=repo)
    try:
        deadline = time.monotonic() + 40
        while not (desk/'ack.json').exists():
            if child.poll() is not None: raise RuntimeError(f'Child exited before acknowledgement: {child.returncode}')
            if time.monotonic() > deadline: raise TimeoutError('Journal acknowledgement timed out')
            time.sleep(.02)
        ack = json.loads((desk/'ack.json').read_text())
        assert ack['journal_acknowledged'] and ack['anchor_waiting'] and ack['native_save_failed']
        files = sorted((desk/'pending-ink-v1').glob('*.json'))
        assert len(files) == 4 and child.poll() is None
        manifest['journal_before_kill'] = {f.name: {'sha256': sha(f), 'bytes': f.stat().st_size} for f in files}
        os.kill(child.pid, signal.SIGKILL)
        assert child.wait(timeout=10) == -signal.SIGKILL
    finally:
        if child.poll() is None:
            child.kill(); child.wait(timeout=10)
with (output/'recovery.log').open('wb') as log:
    result = subprocess.run([str(binary), '--ink-journal-verify', str(desk)], cwd=repo, stdout=log, stderr=subprocess.STDOUT, timeout=60)
manifest['result'] = {'child_termination': 'SIGKILL', 'recovery_exit_code': result.returncode, 'elapsed_seconds': time.monotonic()-started}
manifest['source_drift'] = [str(f.relative_to(repo)) for f in inputs if f.is_relative_to(repo) and sha(f) != manifest['inputs'][str(f.relative_to(repo))]]
manifest['artifacts'] = {str(f.relative_to(output)): {'sha256': sha(f), 'bytes': f.stat().st_size} for f in sorted(output.rglob('*')) if f.is_file() and f.name != 'manifest.json'}
(output/'manifest.json').write_text(json.dumps(manifest, indent=2)+'\n')
assert result.returncode == 0, 'Recovery acceptance failed; see recovery.log'
assert not manifest['source_drift'], 'Inputs changed during acceptance'
print(json.dumps(manifest['result'], indent=2))
