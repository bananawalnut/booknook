#!/usr/bin/env python3
"""Use an explicitly supplied Zig C compiler for the freestanding Rust WASM target."""
import os
import sys

zig = os.environ.get("BOOKNOOK_ZIG")
if not zig:
    raise SystemExit("Set BOOKNOOK_ZIG to an installed Zig executable")
args = []
for arg in sys.argv[1:]:
    args.extend(["-target", "wasm32-freestanding"] if arg == "--target=wasm32-unknown-unknown" else [arg])
os.execv(zig, [zig, "cc", *args])
