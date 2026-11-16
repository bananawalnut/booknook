#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Resolve Cargo's configured output root without building or changing configuration."""
import json
from pathlib import Path
import subprocess


def cargo_target_directory(workspace: Path) -> Path:
    result = subprocess.run(
        ["cargo", "metadata", "--locked", "--offline", "--no-deps", "--format-version", "1"],
        cwd=workspace, check=True, stdout=subprocess.PIPE, text=True,
    )
    metadata = json.loads(result.stdout)
    target = metadata.get("target_directory") if isinstance(metadata, dict) else None
    if not isinstance(target, str) or not target:
        raise ValueError("Cargo metadata must provide a target_directory string")
    if any(character in target for character in "\x00\r\n"):
        raise ValueError("Cargo metadata target_directory must not contain NUL or line breaks")
    path = Path(target)
    if not path.is_absolute():
        raise ValueError("Cargo metadata target_directory must be absolute")
    return path


if __name__ == "__main__":
    try:
        print(cargo_target_directory(Path(__file__).resolve().parents[1]))
    except (OSError, subprocess.CalledProcessError, ValueError) as error:
        raise SystemExit(f"Cannot resolve Cargo target directory: {error}")
