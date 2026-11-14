#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Synthetic Apple-tool metadata regressions; no compiler, signing or runtime claim."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import plistlib
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "scripts"))
SPEC = importlib.util.spec_from_file_location("package_apple_apps", ROOT / "scripts/package-apple-apps.py")
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


def build_metadata(platform="MACOS", minimum="14.0"):
    return f"""synthetic-binary:
Load command 11
      cmd LC_BUILD_VERSION
  cmdsize 32
 platform {platform}
    minos {minimum}
      sdk 26.4
   ntools 1
     tool LD
  version 1266.8
"""


class BundleInspectionTests(unittest.TestCase):
    def inspect_fixture(self, main_minimum="14.0", library_minimum="14.0", platform="MACOS",
                        configuration="Debug", debug_library=True, main_magic=b"\xcf\xfa\xed\xfe"):
        with tempfile.TemporaryDirectory(prefix="booknook-package-metadata-") as directory:
            app = Path(directory) / "BookNookMac.app"
            binaries = app / "Contents/MacOS"
            resources = app / "Contents/Resources"
            binaries.mkdir(parents=True)
            (resources / "ontology").mkdir(parents=True)
            for name in ("reader.js", "reader-geometry.js", "ontology/discovery.json"):
                (resources / name).touch()
            (app / "Contents/Info.plist").write_bytes(plistlib.dumps({
                "CFBundleIdentifier": "org.booknook.desktop",
                "CFBundlePackageType": "APPL",
                "CFBundleSupportedPlatforms": ["MacOSX"],
                "LSMinimumSystemVersion": "14.0",
                "CFBundleExecutable": "BookNookMac",
            }))
            names = ["BookNookMac"] + (["BookNookMac.debug.dylib"] if debug_library else [])
            for name in names:
                binary = binaries / name
                binary.write_bytes(b"\xcf\xfa\xed\xfe" if name.endswith(".dylib") else main_magic)
                binary.chmod(0o755)

            def synthetic_tool(command, *arguments):
                if command.endswith("lipo"):
                    return "arm64"
                if command.endswith("vtool"):
                    minimum = library_minimum if arguments[-1].endswith(".dylib") else main_minimum
                    return build_metadata(platform, minimum)
                if command.endswith("otool"):
                    return "synthetic-binary:" if arguments[0] == "-L" else ""
                raise AssertionError(command)

            with patch.object(PACKAGE, "tool", side_effect=synthetic_tool), \
                    patch.object(PACKAGE, "directory_inventory", return_value={}), \
                    patch.object(PACKAGE.subprocess, "run", return_value=SimpleNamespace(stderr="Signature=adhoc")):
                return PACKAGE.inspect(app, "mac", configuration)

    def test_each_macho_minimum_must_fit_declared_floor(self):
        for main, library in (("15.2", "14.0"), ("14.0", "15.2")):
            with self.subTest(main=main, library=library), self.assertRaises(AssertionError):
                self.inspect_fixture(main, library)

    def test_matching_platform_and_minimum_are_accepted(self):
        inspected = self.inspect_fixture()
        self.assertEqual(len(inspected["mach_o"]), 2)
        self.assertTrue(all(binary["minimum_os"] == "14.0" for binary in inspected["mach_o"].values()))

    def test_wrong_platform_is_refused(self):
        with self.assertRaises(AssertionError):
            self.inspect_fixture(platform="IOS")

    def test_release_main_binary_does_not_require_debug_dylib(self):
        self.assertEqual(len(self.inspect_fixture(configuration="Release", debug_library=False)["mach_o"]), 1)
        with self.assertRaises(AssertionError):
            self.inspect_fixture(debug_library=False)

    def test_main_executable_itself_must_be_verified(self):
        with self.assertRaises(AssertionError):
            self.inspect_fixture(configuration="Release", main_magic=b"text")


class BuildMetadataTests(unittest.TestCase):
    def test_all_platforms_accept_matching_or_lower_minimum(self):
        for platform, floor, minima in (("MACOS", "14.0", ("11.0", "14", "14.0.0")),
                                       ("IOS", "17.0", ("16.0", "17.0")),
                                       ("IOSSIMULATOR", "17.0", ("16.4", "17.0.0"))):
            for minimum in minima:
                with self.subTest(platform=platform, minimum=minimum):
                    self.assertEqual(PACKAGE.inspect_macho_build(build_metadata(platform, minimum), platform, floor), minimum)

    def test_newer_minimum_is_rejected_including_patch_level(self):
        for platform, floor, minimum in (("MACOS", "14.0", "14.0.1"), ("IOS", "17.0", "18.0"),
                                        ("IOSSIMULATOR", "17.0", "17.1")):
            with self.subTest(platform=platform), self.assertRaises(AssertionError):
                PACKAGE.inspect_macho_build(build_metadata(platform, minimum), platform, floor)

    def test_missing_ambiguous_or_malformed_metadata_is_rejected(self):
        valid = build_metadata()
        invalid = (valid.replace("    minos 14.0\n", ""), valid + "    minos 14.0\n",
                   valid + " platform IOS\n", valid + " cmd LC_BUILD_VERSION\n",
                   valid.replace("LC_BUILD_VERSION", "LC_VERSION_MIN_MACOSX"),
                   *(build_metadata(minimum=value) for value in ("0.0", "14.x", "14.0.0.0", "14.256", "65536.0")))
        for metadata in invalid:
            with self.subTest(metadata=metadata), self.assertRaises(AssertionError):
                PACKAGE.inspect_macho_build(metadata, "MACOS", "14.0")

    def test_external_product_roots_and_configurations(self):
        root = Path("/external/booknook-apple")
        self.assertEqual(PACKAGE.product_path(root, "mac", "Debug"), root / "mac/Build/Products/Debug/BookNookMac.app")
        self.assertEqual(PACKAGE.product_path(root, "mac", "Release"), root / "mac/Build/Products/Release/BookNookMac.app")
        self.assertEqual(PACKAGE.product_path(root, "ipad-sim", "Release"), root / "ipad-sim/Build/Products/Release-iphonesimulator/BookNookPad.app")
        self.assertEqual(PACKAGE.product_path(root, "ipad-device", "Release"), root / "ipad-device/Build/Products/Release-iphoneos/BookNookPad.app")
        with self.assertRaises(AssertionError):
            PACKAGE.product_path(root, "mac", "Profile")


if __name__ == "__main__":
    unittest.main()
