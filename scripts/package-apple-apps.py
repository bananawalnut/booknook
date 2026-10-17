#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Archive existing Apple development builds and verify extracted bundles; never build/run/sign."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import plistlib
import re
import stat
import subprocess
import tarfile
import tempfile
import tomllib
from ontology_artifacts import directory_inventory

ROOT = Path(__file__).resolve().parents[1]
PRODUCTS = {
    "mac": ("mac/Build/Products/Debug/BookNookMac.app", "MacOSX", "MACOS", "org.booknook.desktop"),
    "ipad-sim": ("ipad-sim/Build/Products/Debug-iphonesimulator/BookNookPad.app", "iPhoneSimulator", "IOSSIMULATOR", "org.booknook.ipad"),
    "ipad-device": ("ipad-device/Build/Products/Debug-iphoneos/BookNookPad.app", "iPhoneOS", "IOS", "org.booknook.ipad"),
}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def manifest(app):
    result = {}
    for path in sorted(app.rglob("*")):
        name = path.relative_to(app).as_posix()
        if path.is_symlink():
            if not path.resolve().is_relative_to(app.resolve()):
                raise ValueError(f"Bundle symlink escapes app: {name}")
            result[name] = {"symlink": os.readlink(path)}
        elif path.is_file():
            result[name] = {"sha256": digest(path), "bytes": path.stat().st_size,
                            "mode": stat.S_IMODE(path.stat().st_mode)}
        elif path.is_dir():
            result[name] = {"directory": True, "mode": stat.S_IMODE(path.stat().st_mode)}
        else:
            raise ValueError(f"Unsupported bundle member: {name}")
    return result


def tool(*args):
    return subprocess.check_output(args, text=True).strip()


def inspect(app, key):
    _, supported, platform, bundle_id = PRODUCTS[key]
    info_path = app / ("Contents/Info.plist" if key == "mac" else "Info.plist")
    info = plistlib.loads(info_path.read_bytes())
    assert info["CFBundleIdentifier"] == bundle_id
    assert info["CFBundlePackageType"] == "APPL"
    assert info["CFBundleSupportedPlatforms"] == [supported]
    assert info.get("LSMinimumSystemVersion" if key == "mac" else "MinimumOSVersion") == ("14.0" if key == "mac" else "17.0")
    if key != "mac":
        assert info["UIDeviceFamily"] == [2]
    binaries = app / ("Contents/MacOS" if key == "mac" else "")
    executable = binaries / info["CFBundleExecutable"]
    assert executable.is_file() and os.access(executable, os.X_OK)
    assert (binaries / f'{info["CFBundleExecutable"]}.debug.dylib').is_file()
    resources = app / ("Contents/Resources" if key == "mac" else "")
    for required in ["reader.js", "reader-geometry.js", "ontology/discovery.json"]:
        assert (resources / required).is_file(), required
    ontology = directory_inventory(resources / "ontology")
    assert ontology == directory_inventory(ROOT / "ontology"), "Bundled ontology differs from source"
    machos = {}
    for path in sorted(app.rglob("*")):
        if not path.is_file() or path.is_symlink():
            continue
        with path.open("rb") as stream:
            magic = stream.read(4)
        if magic not in [b"\xcf\xfa\xed\xfe", b"\xfe\xed\xfa\xcf", b"\xca\xfe\xba\xbe", b"\xbe\xba\xfe\xca"]:
            continue
        assert tool("/usr/bin/lipo", "-archs", str(path)) == "arm64"
        build = tool("/usr/bin/vtool", "-show-build", str(path))
        assert re.search(rf"\bplatform {platform}\s", build), build
        loads = tool("/usr/bin/otool", "-l", str(path))
        rpaths = re.findall(r"cmd LC_RPATH\s+cmdsize \d+\s+path (.+?) \(offset", loads)
        libraries = [line.strip().split(" (compatibility version", 1)[0]
                     for line in tool("/usr/bin/otool", "-L", str(path)).splitlines()[1:]]
        def expand(value):
            return Path(value.replace("@executable_path", str(binaries)).replace("@loader_path", str(path.parent)))
        for dependency in libraries:
            if dependency.startswith(("/usr/lib/", "/System/Library/")):
                continue
            candidates = ([expand(rpath) / dependency[len("@rpath/"):]
                           for rpath in rpaths + ["@executable_path"]]
                          if dependency.startswith("@rpath/") else [expand(dependency)])
            assert any(candidate.resolve().is_relative_to(app.resolve()) and candidate.is_file()
                       for candidate in candidates), f"Unbundled dependency: {dependency}"
        signature = subprocess.run(["/usr/bin/codesign", "-dvv", str(path)], capture_output=True, text=True)
        sign_text = signature.stderr
        signing = "linker-ad-hoc" if "Signature=adhoc" in sign_text else "unsigned" if "not signed at all" in sign_text else "other"
        machos[path.relative_to(app).as_posix()] = {
            "architecture": "arm64", "platform": platform, "dependencies": libraries,
            "rpaths": rpaths, "signature": signing,
        }
    assert len(machos) >= 2
    return {"info": {name: info.get(name) for name in [
        "CFBundleExecutable", "CFBundleIdentifier", "CFBundleSupportedPlatforms",
        "CFBundleShortVersionString", "CFBundleVersion", "LSMinimumSystemVersion",
        "MinimumOSVersion", "UIDeviceFamily"]}, "mach_o": machos, "ontology": ontology}


def normalized(info):
    info.uid = info.gid = 0
    info.uname = info.gname = ""
    info.mtime = 0
    return info


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT.parent / "booknook-apps")
    parser.add_argument("--provenance", type=Path, required=True,
                        help="Verified build-inputs.json containing app_trees from successful builds")
    args = parser.parse_args()
    provenance = json.loads(args.provenance.read_text())
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    args.output.mkdir(parents=True, exist_ok=True)
    report = {"workspace_version": version, "build_provenance_sha256": digest(args.provenance), "apps": {}}
    for key, (relative, _, _, _) in PRODUCTS.items():
        app = ROOT / "apps/apple/Build" / relative
        before = manifest(app)
        files = {name: {k: v for k, v in entry.items() if k != "mode"}
                 for name, entry in before.items() if "directory" not in entry}
        assert files == provenance["app_trees"][key]["files"], f"Build provenance differs: {key}"
        inspection = inspect(app, key)
        output = args.output / f"booknook-{key}-arm64-development-{version}.tar.gz"
        with output.open("wb") as raw, gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w") as archive:
                archive.add(app, arcname=app.name, filter=normalized)
                for name in ["LICENSE-MPL-2.0", "LICENSE-MIT", "LICENSE-APACHE"]:
                    archive.add(ROOT / name, arcname=name, filter=normalized)
                archive.add(ROOT / "docs/dependency-licenses.csv", arcname="DEPENDENCY-LICENSES.csv", filter=normalized)
        # Retain at most one extracted bundle, and remove it even on verification failure.
        with tempfile.TemporaryDirectory(prefix="booknook-app-package-") as temporary:
            with tarfile.open(output) as archive:
                archive.extractall(temporary, filter="data")
            extracted = Path(temporary) / app.name
            assert manifest(extracted) == before, f"Extracted bundle tree differs: {key}"
            assert inspect(extracted, key) == inspection, f"Extracted executable metadata differs: {key}"
        assert manifest(app) == before, f"Source bundle changed during packaging: {key}"
        report["apps"][key] = {"archive": output.name, "archive_bytes": output.stat().st_size,
                               "archive_sha256": digest(output), "bundle": app.name,
                               "files": before, "inspection": inspection}
        print(f"{key}: verified {len(files)} files, {len(inspection['mach_o'])} Mach-O binaries, {output.stat().st_size} archive bytes; {digest(output)}")
    (args.output / "VERIFICATION.json").write_text(json.dumps(report, indent=2) + "\n")
    (args.output / "README.md").write_text(f"""# Book Nook native development apps

These archives preserve the already-built Book Nook {version} development bundles. They are not signed/notarized release distributions. Packaging did not rebuild, sign, install or launch any application.

| Archive | Intended environment | Limitations |
| --- | --- | --- |
| `booknook-mac-arm64-development-{version}.tar.gz` | Apple Silicon Mac, macOS 14 or newer | Launchable-format native development app with linker ad-hoc Mach-O signatures; no Developer ID, sealed resource signature or notarization. Local execution remains subject to macOS security and compatibility checks. No launch was performed during packaging. |
| `booknook-ipad-sim-arm64-development-{version}.tar.gz` | arm64 iPad Simulator in Xcode, iOS 17 or newer runtime | Simulator build only; cannot run on a physical iPad. Requires a compatible installed simulator runtime and separate developer installation. |
| `booknook-ipad-device-arm64-development-{version}.tar.gz` | arm64 physical iPad build target, iPadOS 17 or newer | Unsigned app bundle, not an installable IPA. Proper developer signing/provisioning and device qualification are still required. |

Check `shasum -a 256 -c SHA256SUMS`, then extract the appropriate tarball. Each archive contains the entire `.app`, including the required `.debug.dylib`, reader JavaScript, offline ontology resources and license inventory. The `mac` and `ipad-sim` binary linker signatures are pre-existing; the device bundle is unsigned. There are no third-party dynamic dependencies outside the bundles: remaining dynamic links are Apple system libraries/frameworks. The Rust implementation is statically linked into the debug library.

The archive version is the source workspace version. These development Info.plists do not declare marketing/build version fields. Physical Pencil input, current OS/device compatibility and live account qualification are not established by compilation or archive checks. The apps use their normal application identifiers and storage locations; the archives contain no user library, account credentials or test desk database.

`VERIFICATION.json` records every bundled file hash/mode, each Mach-O architecture/platform/dependency, Info.plist fields and comparison to the successful-build provenance. Every archive was independently extracted, its full bundle tree and executable metadata compared, and the temporary copy removed. `SHA256SUMS` covers the three archives and this verification record.

Reproduce from the adjacent source workspace after successful builds with `python3 scripts/package-apple-apps.py --provenance docs/build-inputs.json`. The required provenance prevents accidentally packaging stale or changed builds. Outputs default to the adjacent `booknook-apps` directory. The script only reads build products and uses at most one temporary extracted bundle at a time. Source and license boundaries are documented in the adjacent `booknook` workspace; new reference-app code is MPL-2.0 and new core/SDK code is MIT OR Apache-2.0. Dependencies retain their declared licenses.
""")
    checked = [args.output / app["archive"] for app in report["apps"].values()]
    checked += [args.output / "VERIFICATION.json", args.output / "README.md"]
    (args.output / "SHA256SUMS").write_text("".join(f"{digest(path)}  {path.name}\n" for path in checked))


if __name__ == "__main__":
    main()
