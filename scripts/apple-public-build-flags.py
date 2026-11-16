#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Emit Rust, cc-rs, and Xcode release flags for the public Apple build."""

import os
from pathlib import Path
import shlex
import sys


def mappings(roots: list[tuple[str, str]]) -> dict[str, str]:
    result = {}
    for root, replacement in roots:
        logical = os.path.abspath(os.path.expanduser(root))
        for path in (logical, str(Path(logical).resolve())):
            # The shell receives one flag vector per line. Xcode also expands
            # dollar references in build settings before splitting arguments.
            if path == "/" or any(ord(char) < 32 or char in "$=" for char in path):
                raise ValueError("Public build roots cannot be / or contain control characters, $ or =")
            if result.setdefault(path, replacement) != replacement:
                raise ValueError("Public build roots must have distinct paths")
    # A tool that honors the last matching rule should prefer nested roots.
    return dict(sorted(result.items(), key=lambda rule: len(rule[0])))


def xcode_flags(flags: list[str]) -> str:
    return " ".join('"' + flag.replace("\\", "\\\\").replace('"', '\\"') + '"' for flag in flags)


def main() -> None:
    source, target, derived_data = sys.argv[1:]
    roots = mappings([
        (source, "/booknook-source"),
        (os.environ.get("CARGO_HOME") or str(Path.home() / ".cargo"), "/booknook-cargo"),
        (os.environ.get("RUSTUP_HOME") or str(Path.home() / ".rustup"), "/booknook-rustup"),
        (target, "/booknook-build/cargo"),
        (derived_data, "/booknook-build/xcode"),
        (os.environ.get("TMPDIR") or "/tmp", "/booknook-build/tmp"),
    ])
    rust = ["--remap-path-scope", "all"]
    clang = ["-ffile-compilation-dir=/booknook-build"]
    swift = ["-gnone", "-Xfrontend", "-file-compilation-dir", "-Xfrontend", "/booknook-build"]
    for old, new in roots.items():
        mapping = f"{old}={new}"
        rust.extend(("--remap-path-prefix", mapping))
        clang.append("-ffile-prefix-map=" + mapping)
        swift.extend(("-Xfrontend", "-file-prefix-map", "-Xfrontend", mapping))
    print("\x1f".join(rust))
    print(shlex.join(clang))
    print(xcode_flags(clang))
    print(xcode_flags(swift))


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError) as error:
        raise SystemExit(f"Cannot configure public Apple build: {error}") from error
