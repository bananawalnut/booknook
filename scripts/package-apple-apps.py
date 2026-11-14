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
    "mac": ("mac/Build/Products/{configuration}/BookNookMac.app", "MacOSX", "MACOS", "org.booknook.desktop"),
    "ipad-sim": ("ipad-sim/Build/Products/{configuration}-iphonesimulator/BookNookPad.app", "iPhoneSimulator", "IOSSIMULATOR", "org.booknook.ipad"),
    "ipad-device": ("ipad-device/Build/Products/{configuration}-iphoneos/BookNookPad.app", "iPhoneOS", "IOS", "org.booknook.ipad"),
}
MINIMUM_OS = {"mac": "14.0", "ipad-sim": "17.0", "ipad-device": "17.0"}


def product_path(build_root, key, configuration):
    assert configuration in ("Debug", "Release"), configuration
    return build_root / PRODUCTS[key][0].format(configuration=configuration)


def os_version(value):
    assert re.fullmatch(r"[0-9]+(?:\.[0-9]+){0,2}", value), f"Invalid minimum OS: {value!r}"
    parts = tuple(int(part) for part in value.split("."))
    parts += (0,) * (3 - len(parts))
    assert 0 < parts[0] <= 65535 and all(part <= 255 for part in parts[1:]), value
    return parts


def inspect_macho_build(build, platform, minimum):
    """Check tool-reported deployment metadata, not compatibility on an older OS."""
    commands = re.findall(r"^\s*cmd\s+(\S+)\s*$", build, re.MULTILINE)
    platforms = re.findall(r"^\s*platform\s+(\S+)\s*$", build, re.MULTILINE)
    minima = re.findall(r"^\s*minos\s+(\S+)\s*$", build, re.MULTILINE)
    assert commands == ["LC_BUILD_VERSION"], f"Missing or ambiguous build command: {build}"
    assert platforms == [platform], f"Wrong or ambiguous platform: {build}"
    assert len(minima) == 1, f"Missing or ambiguous minimum OS: {build}"
    assert os_version(minima[0]) <= os_version(minimum), f"Mach-O requires {minima[0]}, above declared {minimum}: {build}"
    return minima[0]


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


def inspect(app, key, configuration="Debug"):
    _, supported, platform, bundle_id = PRODUCTS[key]
    info_path = app / ("Contents/Info.plist" if key == "mac" else "Info.plist")
    info = plistlib.loads(info_path.read_bytes())
    assert info["CFBundleIdentifier"] == bundle_id
    assert info["CFBundlePackageType"] == "APPL"
    assert info["CFBundleSupportedPlatforms"] == [supported]
    assert configuration in ("Debug", "Release"), configuration
    assert info.get("LSMinimumSystemVersion" if key == "mac" else "MinimumOSVersion") == MINIMUM_OS[key]
    if key != "mac":
        assert info["UIDeviceFamily"] == [2]
    binaries = app / ("Contents/MacOS" if key == "mac" else "")
    executable = binaries / info["CFBundleExecutable"]
    assert executable.is_file() and os.access(executable, os.X_OK)
    if configuration == "Debug":
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
        minimum = inspect_macho_build(build, platform, MINIMUM_OS[key])
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
            "architecture": "arm64", "platform": platform, "minimum_os": minimum, "dependencies": libraries,
            "rpaths": rpaths, "signature": signing,
        }
    assert executable.relative_to(app).as_posix() in machos, "Main executable is not a verified Mach-O"
    assert len(machos) >= (2 if configuration == "Debug" else 1)
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
    parser.add_argument("--build-root", type=Path, default=ROOT / "apps/apple/Build",
                        help="Existing build-apple output root; source/license/ontology roots do not change")
    parser.add_argument("--configuration", choices=("Debug", "Release"), default="Debug")
    parser.add_argument("--provenance", type=Path, required=True,
                        help="Verified build-inputs.json containing app_trees from successful builds")
    args = parser.parse_args()
    provenance = json.loads(args.provenance.read_text())
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    args.output.mkdir(parents=True, exist_ok=True)
    report = {"workspace_version": version, "configuration": args.configuration,
              "build_provenance_sha256": digest(args.provenance), "apps": {}}
    for key in PRODUCTS:
        app = product_path(args.build_root.resolve(), key, args.configuration)
        before = manifest(app)
        files = {name: {k: v for k, v in entry.items() if k != "mode"}
                 for name, entry in before.items() if "directory" not in entry}
        assert files == provenance["app_trees"][key]["files"], f"Build provenance differs: {key}"
        inspection = inspect(app, key, args.configuration)
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
            assert inspect(extracted, key, args.configuration) == inspection, f"Extracted executable metadata differs: {key}"
        assert manifest(app) == before, f"Source bundle changed during packaging: {key}"
        report["apps"][key] = {"archive": output.name, "archive_bytes": output.stat().st_size,
                               "archive_sha256": digest(output), "bundle": app.name,
                               "files": before, "inspection": inspection}
        print(f"{key}: verified {len(files)} files, {len(inspection['mach_o'])} Mach-O binaries, {output.stat().st_size} archive bytes; {digest(output)}")
    (args.output / "VERIFICATION.json").write_text(json.dumps(report, indent=2) + "\n")
    (args.output / "README.md").write_text(f"""# Book Nook native development apps

These archives preserve the already-built Book Nook {version} {args.configuration}-configuration development bundles. They are not signed/notarized release distributions. Packaging did not rebuild, sign, install or launch any application.

| Archive | Intended environment | Limitations |
| --- | --- | --- |
| `booknook-mac-arm64-development-{version}.tar.gz` | Apple Silicon Mac, macOS 14 or newer | Launchable-format native development app with linker ad-hoc Mach-O signatures; no Developer ID, sealed resource signature or notarization. Local execution remains subject to macOS security and compatibility checks. No launch was performed during packaging. |
| `booknook-ipad-sim-arm64-development-{version}.tar.gz` | arm64 iPad Simulator in Xcode, iOS 17 or newer runtime | Simulator build only; cannot run on a physical iPad. Requires a compatible installed simulator runtime and separate developer installation. |
| `booknook-ipad-device-arm64-development-{version}.tar.gz` | arm64 physical iPad build target, iPadOS 17 or newer | Unsigned app bundle, not an installable IPA. Proper developer signing/provisioning and device qualification are still required. |

Check `shasum -a 256 -c SHA256SUMS`, then extract the appropriate tarball. Each archive contains the entire `.app`, reader JavaScript, offline ontology resources and license inventory. Debug bundles must include their `.debug.dylib`; Release bundles may contain a single main binary. Existing binary signatures are recorded per Mach-O; packaging does not add or renew them. There are no third-party dynamic dependencies outside the bundles: remaining dynamic links are Apple system libraries/frameworks. The Rust implementation is statically linked into the app's binaries.

The archive version is the source workspace version; actual Info.plist marketing/build fields are recorded separately. Every bundled Mach-O must declare the expected platform and a minimum OS no newer than the app's declared macOS 14.0 or iPadOS 17.0 floor. This metadata check does not establish execution on those OS versions. Physical Pencil input, current OS/device compatibility and live account qualification are not established by compilation or archive checks. The apps use their normal application identifiers and storage locations; the archives contain no user library, account credentials or test desk database.

`VERIFICATION.json` records the selected configuration, every bundled file hash/mode, each Mach-O architecture/platform/minimum-OS/dependency, Info.plist fields and comparison to the successful-build provenance. Every archive was independently extracted, its full bundle tree and executable metadata compared, and the temporary copy removed. `SHA256SUMS` covers the three archives and this verification record.

Reproduce from the adjacent source workspace after successful matching builds with `python3 scripts/package-apple-apps.py --configuration {args.configuration} --provenance <matching-build-inputs.json>`. `--build-root <directory>` selects an external build-apple output root; the default is `apps/apple/Build`. The required provenance prevents accidentally packaging stale or changed builds. Outputs default to the adjacent `booknook-apps` directory. The script only reads build products and uses at most one temporary extracted bundle at a time. Source and license boundaries are documented in the adjacent `booknook` workspace; new reference-app code is MPL-2.0 and new core/SDK code is MIT OR Apache-2.0. Dependencies retain their declared licenses.
""")
    checked = [args.output / app["archive"] for app in report["apps"].values()]
    checked += [args.output / "VERIFICATION.json", args.output / "README.md"]
    (args.output / "SHA256SUMS").write_text("".join(f"{digest(path)}  {path.name}\n" for path in checked))


if __name__ == "__main__":
    main()
