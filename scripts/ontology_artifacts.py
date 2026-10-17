# SPDX-License-Identifier: MIT OR Apache-2.0
"""Validate the complete static discovery graph from files or an HTTP reader."""
import hashlib
import json
from pathlib import Path, PurePosixPath
from urllib.parse import urlsplit


def relative_path(value):
    if not isinstance(value, str) or not value or "\\" in value:
        raise ValueError("Ontology paths must be nonempty relative URL paths")
    url = urlsplit(value)
    path = PurePosixPath(value)
    if url.scheme or url.netloc or url.query or url.fragment or path.is_absolute() or any(
        part in ("", ".", "..") for part in value.split("/")
    ) or "%" in value:
        raise ValueError(f"Invalid relative ontology path: {value}")
    return path


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"Duplicate JSON field: {key}")
        result[key] = value
    return result


def inventory(read):
    """Follow all discovery manifest groups, including nested protocol releases."""
    files = {}

    def document(name):
        relative_path(name)
        raw = read(name)
        files[name] = {"sha256": hashlib.sha256(raw).hexdigest(), "bytes": len(raw)}
        return json.loads(raw, object_pairs_hook=unique_object)

    discovery = document("discovery.json")
    if discovery.get("schema") != "booknook.ontology-discovery.v1":
        raise ValueError("Unknown ontology discovery schema")
    releases = discovery.get("releases", [])
    if not releases or discovery.get("current") not in [entry["version"] for entry in releases]:
        raise ValueError("Discovery must include its current ontology release")
    manifests = []
    artifacts = set()
    manifest_paths = set()
    for group, entries in discovery.items():
        if not isinstance(entries, list):
            continue
        for entry in entries:
            name = str(relative_path(entry["manifest"]))
            if name in manifest_paths:
                raise ValueError(f"Duplicate discovered manifest: {name}")
            manifest_paths.add(name)
            manifest = document(name)
            if manifest.get("version") != entry["version"] or not manifest.get("schema"):
                raise ValueError(f"Manifest version/schema mismatch: {name}")
            records = manifest.get("artifacts", [])
            if not records:
                raise ValueError(f"Empty ontology manifest: {name}")
            names = set()
            for artifact in records:
                child = relative_path(artifact["path"])
                path = str(PurePosixPath(name).parent / child)
                if path in names:
                    raise ValueError(f"Duplicate manifest artifact: {path}")
                names.add(path)
                document(path)
                if files[path]["sha256"] != artifact["sha256"]:
                    raise ValueError(f"Ontology checksum mismatch: {path}")
                artifacts.add(path)
            manifests.append({"group": group, "path": name, "version": entry["version"],
                              "artifacts": len(names)})
    return {"current": discovery["current"], "manifests": manifests,
            "artifact_count": len(artifacts), "files": files}


def directory_inventory(directory):
    root = Path(directory).resolve()

    def read(name):
        path = (root / name).resolve()
        if not path.is_relative_to(root):
            raise ValueError(f"Ontology artifact escapes its directory: {name}")
        return path.read_bytes()

    return inventory(read)
