#!/usr/bin/env python3
"""Archive WASM C objects using an explicitly supplied Zig toolchain."""
import os
import sys

zig = os.environ.get("BOOKNOOK_ZIG")
if not zig:
    raise SystemExit("Set BOOKNOOK_ZIG to an installed Zig executable")
os.execv(zig, [zig, "ar", *sys.argv[1:]])
