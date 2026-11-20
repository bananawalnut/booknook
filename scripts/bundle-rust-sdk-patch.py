#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Add the consumer workspace, lock and patched dependency to the built Core archive."""
import gzip
import io
from pathlib import Path
import sys
import tarfile
import tempfile
import tomllib


def bundle(target):
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    members = [f"booknook-{name}-{version}" for name in ("core", "connectors", "native", "wasm")]
    core = members[0]
    vendor = root / "vendor/faster-hex"
    manifest = tomllib.loads((vendor / "Cargo.toml").read_text())
    if (manifest["package"]["name"], manifest["package"]["version"]) != ("faster-hex", "0.10.1"):
        raise ValueError("Unexpected patched package")
    extra = {}
    for path in sorted(vendor.rglob("*")):
        relative = path.relative_to(vendor)
        if path.is_symlink() or any(part in {".git", "target", "Cargo.lock", ".cargo-ok"} for part in relative.parts):
            raise ValueError(f"Unexpected vendor entry: {relative}")
        if path.is_file():
            extra[f"vendor/faster-hex/{relative}"] = path.read_bytes()
    for name in ("LICENSE", "src/lib.rs", "BOOKNOOK-PATCH.md"):
        if f"vendor/faster-hex/{name}" not in extra:
            raise ValueError(f"Missing patched dependency resource: {name}")
    quoted = ", ".join(f'"{member}"' for member in members)
    lines = [f'[workspace]\nresolver = "2"\nmembers = [{quoted}]',
             f'exclude = ["{core}/vendor/faster-hex"]', '', '[patch.crates-io]']
    for member in members:
        name = member.removesuffix("-" + version)
        lines.append(f'{name} = {{ path = "{member}" }}')
    lines.append(f'faster-hex = {{ path = "{core}/vendor/faster-hex" }}')
    extra["sdk-workspace.toml"] = ("\n".join(lines) + "\n").encode()
    lock = (root / "Cargo.lock").read_bytes()
    for member in members:
        files = {"Cargo.lock": lock}
        if member == core:
            files.update(extra)
        rewrite(target / "package" / f"{member}.crate", member, files)


def rewrite(path, member_root, extra):
    with tempfile.TemporaryDirectory(dir=path.parent) as scratch:
        staged = Path(scratch) / path.name
        with tarfile.open(path, "r:gz") as src, staged.open("wb") as raw:
            with gzip.GzipFile(filename="", fileobj=raw, mode="wb", mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w|") as dst:
                    for member in src:
                        if member.name.removeprefix(member_root + "/") in extra:
                            continue
                        dst.addfile(member, src.extractfile(member) if member.isfile() else None)
                    for name, data in sorted(extra.items()):
                        entry = tarfile.TarInfo(f"{member_root}/{name}")
                        entry.size = len(data)
                        entry.mode = 0o644
                        dst.addfile(entry, io.BytesIO(data))
        staged.replace(path)


if __name__ == "__main__":
    bundle(Path(sys.argv[1]))
