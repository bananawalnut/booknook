#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Stage only consumer-tested SDK archives and sanitized provenance from hosted CI."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import stat
import subprocess
import tarfile
import tomllib

MIB = 1024 * 1024
PATH_ENCODINGS = ("utf-8", "utf-16-le", "utf-16-be")
# Apple's official Rust standard libraries can retain this public upstream
# checkout prefix. It does not identify the current Book Nook build runner.
PUBLIC_RUST_PREFIXES = {
    home.encode(encoding): (home.rstrip("/") + "/work/rust/rust/").encode(encoding)
    for home in ("/Users/runner", "/Users/runner/", "/home/runner", "/home/runner/")
    for encoding in PATH_ENCODINGS
}
PRIVATE_MEMBER = re.compile(
    r"(^|/)(?:\.git|\.env|credentials|private|xcuserdata)(?:/|$)"
    r"|\.(?:log|key|p8|p12|pfx|mobileprovision|sqlite3?|db)(?:-(?:wal|shm|journal))?$",
    re.IGNORECASE,
)


def tool_version(*command):
    return " ".join(subprocess.check_output(command, text=True, timeout=20).splitlines())


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def private_paths(workspace, scratch, target):
    paths = [workspace, scratch, target, Path.home()]
    paths += [Path(os.environ[name]) for name in ("CARGO_HOME", "RUSTUP_HOME") if os.environ.get(name)]
    if os.environ.get("BOOKNOOK_APP_BUILD_DIR"):
        paths.append(Path(os.environ["BOOKNOOK_APP_BUILD_DIR"]))
    roots = {str(path) for path in paths} | {str(path.resolve()) for path in paths}
    if any(not Path(root).is_absolute() or len(root) < 5 for root in roots):
        raise ValueError("Unexpected runner path")
    return tuple(
        root.encode(encoding)
        for root in sorted(roots | {"/home/runner/", "/Users/runner/"})
        for encoding in PATH_ENCODINGS
    )


def has_private_path(data, markers, cutoff):
    for marker in markers:
        # Only inspect start positions below cutoff, leaving enough following
        # bytes to distinguish a split public toolchain prefix.
        end = cutoff + len(marker) - 1
        public_prefix = PUBLIC_RUST_PREFIXES.get(marker)
        at = data.find(marker, 0, end)
        while at >= 0:
            if public_prefix is None or not data.startswith(public_prefix, at):
                return True
            at = data.find(marker, at + 1, end)
    return False


def inspect_archive(path, member_root, limit, markers):
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or not 1 <= metadata.st_size <= limit * MIB:
        raise ValueError(f"Unexpected SDK archive type or size: {path.name}")
    seen, total = set(), 0
    overlap = max(map(len, (*markers, *PUBLIC_RUST_PREFIXES.values()))) - 1
    with tarfile.open(path, "r|gz") as archive:
        for member in archive:
            parts = PurePosixPath(member.name).parts
            relative = "/".join(parts[1:])
            if (not parts or parts[0] != member_root or ".." in parts
                or "\\" in member.name or PRIVATE_MEMBER.search(relative)
                or relative in seen or len(seen) >= 2048
                or not (member.isdir() or member.isreg())):
                raise ValueError(f"Unexpected SDK archive member: {path.name}")
            seen.add(relative)
            if member.isdir():
                continue
            total += member.size
            if not 0 <= member.size <= 256 * MIB or total > 768 * MIB:
                raise ValueError(f"SDK archive exceeds expansion bound: {path.name}")
            with archive.extractfile(member) as stream:
                chunk = stream.read(MIB)
                if chunk.startswith(b"SQLite format 3\x00"):
                    raise ValueError(f"Private database in SDK archive: {path.name}")
                tail = b""
                while chunk:
                    window = tail + chunk
                    if has_private_path(window, markers, max(0, len(window) - overlap)):
                        raise ValueError(f"Absolute runner path in SDK archive: {path.name}")
                    tail = window[-overlap:]
                    chunk = stream.read(MIB)
                if has_private_path(tail, markers, len(tail)):
                    raise ValueError(f"Absolute runner path in SDK archive: {path.name}")
    if not total:
        raise ValueError(f"Empty SDK archive: {path.name}")
    return {"name": path.name, "bytes": metadata.st_size, "sha256": sha256(path)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("job", choices=("portable", "apple"))
    job = parser.parse_args().job
    os.umask(0o077)
    source = os.environ["GITHUB_SHA"]
    workspace = Path(os.environ["GITHUB_WORKSPACE"])
    scratch = Path(os.environ["RUNNER_TEMP"])
    target = Path(os.environ["CARGO_TARGET_DIR"])
    if not re.fullmatch(r"[0-9a-f]{40}", source) or tool_version("git", "rev-parse", "HEAD") != source:
        raise ValueError("SDK provenance must name the exact checkout")
    if tool_version("git", "status", "--porcelain"):
        raise ValueError("Tracked or unignored files changed during SDK packaging")
    version = tomllib.loads((workspace / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    npm_version = json.loads((workspace / "sdk/typescript/package.json").read_text())["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", version) or version != npm_version:
        raise ValueError("Inconsistent or unsupported SDK package version")
    tools = {"rustc": tool_version("rustc", "--version"), "cargo": tool_version("cargo", "--version")}
    if job == "portable":
        sources = [
            (target / f"package/booknook-{kind}-{version}.crate", f"booknook-{kind}-{version}", 8)
            for kind in ("core", "connectors", "native", "wasm")
        ]
        sources.append((target / f"packages/booknook-sdk-{version}.tgz", "package", 24))
        tools.update(node=tool_version("node", "--version"), npm=tool_version("npm", "--version"),
                     wasm_bindgen=tool_version("wasm-bindgen", "--version"),
                     clang=tool_version("clang", "--version").split(" InstalledDir:")[0])
        tests = ["extracted Rust workspace tests and examples", "installed npm CommonJS/ESM, WASM and types"]
    else:
        prefix = f"booknook-swift-sdk-{version}"
        sources = [(target / f"packages/{prefix}.tar.gz", prefix, 256)]
        tools.update(xcode=tool_version("xcodebuild", "-version"), swift=tool_version("xcrun", "swift", "--version"))
        tests = ["extracted Swift package arm64 macOS runtime consumer", "three arm64 XCFramework slices"]
    markers = private_paths(workspace, scratch, target)
    assets = [inspect_archive(path, member_root, limit, markers) for path, member_root, limit in sources]
    provenance = {
        "schema": "booknook.hosted-sdk-archives.v1",
        "repository": os.environ["GITHUB_REPOSITORY"],
        "source_commit": source,
        "workflow": ".github/workflows/sdk-release.yml",
        "run_id": int(os.environ["GITHUB_RUN_ID"]),
        "run_attempt": int(os.environ["GITHUB_RUN_ATTEMPT"]),
        "job": job,
        "package_version": version,
        "cargo_lock_sha256": sha256(workspace / "Cargo.lock"),
        "toolchain": tools,
        "consumer_checks": tests,
        "assets": assets,
    }
    output = scratch / "booknook-sdk-archives"
    provenance_bytes = (json.dumps(provenance, indent=2, sort_keys=True) + "\n").encode()
    if any(marker in provenance_bytes for marker in markers):
        raise ValueError("Absolute runner path in SDK provenance")
    output.mkdir(mode=0o700)
    for (path, _, _), asset in zip(sources, assets, strict=True):
        destination = output / path.name
        shutil.copyfile(path, destination)
        if destination.stat().st_size != asset["bytes"] or sha256(destination) != asset["sha256"]:
            raise ValueError(f"SDK archive changed during staging: {path.name}")
    (output / f"BUILD-PROVENANCE-{job}.json").write_bytes(provenance_bytes)
    print(f"Staged {len(assets)} {job} SDK archives and SHA-256 provenance; no package registry was changed.")


if __name__ == "__main__":
    main()
