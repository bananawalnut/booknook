#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Stage only the validated ontology closure for Pages; never deploy or copy cwd."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import stat
import tempfile
from ontology_artifacts import directory_inventory, inventory, relative_path

ROOT = Path(__file__).resolve().parents[1]
MAX_FILE_BYTES = 1024 * 1024
MAX_TOTAL_BYTES = 8 * 1024 * 1024
MAX_FILES = 256
# SHA-256 of manifests already issued by the public ontology. Their hashes lock all referenced artifacts.
ISSUED_MANIFESTS = {
    "commands/v1/manifest.json": ("command_contracts", "4b06dd67ed1ac43e55b66524eceef739deb7f43cb9f73df5ee39f19f9d86a7a9"),
    "portable/v1/manifest.json": ("portable_contracts", "c9ae6fbf19e1dfe5dab30c7f70b15a238c9b7089eba4c7d1a8c0ec0662d77c1a"),
    "portable/v2/manifest.json": ("portable_contracts", "d63ded55dd4415c0dc2b3f4e3017d84206c66421b8e08492afb98fc212aab4de"),
    "providers/v1/manifest.json": ("provider_contracts", "1df9f744a1b5aea265078ea7dc33c76e55d64deff23c2aefc92fd4706caa5fb8"),
    "v0.1.0/manifest.json": ("releases", "1280c588363ecb240a8f439f835a336b84e1fc961b8f44ab264c3f34296812b4"),
    "v0.2.0/manifest.json": ("releases", "ad6e81bbafabfb4f8226b8af9a26aebf4ae1c6d686bbad53fba3d3b71bc4ea39"),
    "v0.3.0/manifest.json": ("releases", "49191c6f497cb57fab41d0dfce9fcceb7642ad4c937eaeef38f93321e0511059"),
}
INDEX = b'''<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Book Nook ontology</title>
<h1>Book Nook ontology</h1>
<p>Versioned ontology, schemas and conformance fixtures.</p>
<p><a href="ontology/discovery.json">Discover published releases</a></p>
</html>
'''


def checked_path(value):
    path = Path(value).absolute()
    if ".." in path.parts or any(part.is_symlink() for part in (path, *path.parents)):
        raise ValueError(f"Symlinked or noncanonical staging path: {path}")
    return path


def snapshot(source):
    """Read only named files through no-follow descriptors, including every directory."""
    files = {}
    total = 0
    root_fd = os.open(source, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        def read(name):
            nonlocal total
            path = relative_path(name)
            if any(ord(character) < 32 or ord(character) == 127 for character in name):
                raise ValueError("Control character in ontology path")
            if name in files:
                return files[name]
            if len(files) >= MAX_FILES:
                raise ValueError("Ontology file count exceeds staging limit")
            directory_fd = os.dup(root_fd)
            try:
                for component in path.parts[:-1]:
                    next_fd = os.open(component, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW,
                                      dir_fd=directory_fd)
                    os.close(directory_fd)
                    directory_fd = next_fd
                # NONBLOCK prevents a malicious FIFO from blocking before its type check.
                file_fd = os.open(path.name, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK,
                                  dir_fd=directory_fd)
                try:
                    before = os.fstat(file_fd)
                    if not stat.S_ISREG(before.st_mode) or before.st_nlink != 1:
                        raise ValueError(f"Ontology input is not an independent regular file: {name}")
                    if before.st_size > MAX_FILE_BYTES or total + before.st_size > MAX_TOTAL_BYTES:
                        raise ValueError("Ontology bytes exceed staging limit")
                    chunks = []
                    size = 0
                    while True:
                        chunk = os.read(file_fd, min(65536, MAX_FILE_BYTES + 1 - size))
                        if not chunk:
                            break
                        chunks.append(chunk)
                        size += len(chunk)
                        if size > MAX_FILE_BYTES or total + size > MAX_TOTAL_BYTES:
                            raise ValueError("Ontology bytes exceed staging limit")
                    after = os.fstat(file_fd)
                    if (before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
                            after.st_size, after.st_mtime_ns, after.st_ctime_ns) or size != before.st_size:
                        raise ValueError(f"Ontology input changed during staging: {name}")
                    raw = b"".join(chunks)
                    files[name] = raw
                    total += size
                    return raw
                finally:
                    os.close(file_fd)
            finally:
                os.close(directory_fd)

        validated = inventory(read)
    finally:
        os.close(root_fd)
    return validated, files


def stage(source, output):
    source = checked_path(source)
    output = checked_path(output)
    if output == source or output.is_relative_to(source) or source.is_relative_to(output):
        raise ValueError("Staging output and ontology source must be separate")
    if output.exists():
        raise FileExistsError(f"Refusing to overwrite existing staging output: {output}")
    if not output.parent.is_dir():
        raise ValueError("Staging output requires an existing parent directory")
    validated, files = snapshot(source)
    manifest_groups = {entry["path"]: entry["group"] for entry in validated["manifests"]}
    for name, (group, digest) in ISSUED_MANIFESTS.items():
        if manifest_groups.get(name) != group or validated["files"].get(name, {}).get("sha256") != digest:
            raise ValueError(f"Changed or undiscovered issued ontology manifest: {name}")
    with tempfile.TemporaryDirectory(prefix=".booknook-ontology-", dir=output.parent) as temporary:
        site = Path(temporary) / "site"
        site.mkdir()
        for name, raw in sorted(files.items()):
            target = site / "ontology" / name
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(raw)
        (site / "index.html").write_bytes(INDEX)
        (site / ".nojekyll").write_bytes(b"")
        if directory_inventory(site / "ontology") != validated:
            raise ValueError("Staged ontology differs from validated input")
        staged = {}
        for path in sorted(site.rglob("*")):
            if path.is_dir():
                path.chmod(0o755)
            else:
                path.chmod(0o644)
                raw = path.read_bytes()
                staged[path.relative_to(site).as_posix()] = {"bytes": len(raw), "sha256": hashlib.sha256(raw).hexdigest()}
        expected = {"index.html", ".nojekyll"} | {"ontology/" + name for name in files}
        if set(staged) != expected:
            raise ValueError("Unexpected file in staging output")
        site.chmod(0o755)
        # Reserve the destination exclusively; rename only over our own empty directory.
        # This operation expects an output parent controlled by the build owner.
        output.mkdir(mode=0o755)
        reserved = output.stat()
        try:
            current = output.lstat()
            if (current.st_dev, current.st_ino) != (reserved.st_dev, reserved.st_ino):
                raise ValueError("Staging output changed before publication")
            os.rename(site, output)
        except BaseException:
            current = output.lstat()
            if (current.st_dev, current.st_ino) == (reserved.st_dev, reserved.st_ino):
                output.rmdir()
            raise
    return {"schema": "booknook.ontology-pages-stage.v1", "inventory": validated, "staged_files": staged}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True,
                        help="New, absent output directory with an existing non-symlink parent")
    args = parser.parse_args()
    print(json.dumps(stage(ROOT / "ontology", args.output), indent=2))


if __name__ == "__main__":
    main()
