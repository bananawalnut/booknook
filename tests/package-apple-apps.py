#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Synthetic Apple-tool metadata regressions; no compiler, signing or runtime claim."""
import importlib.util
import json
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


class SimulatorSigningTests(unittest.TestCase):
    def test_distinguishes_a_sealed_signature_from_linker_ad_hoc(self):
        sealed = "Signature=adhoc\nCodeDirectory flags=0x2(adhoc)\nSealed Resources version=2"
        linker = "Signature=adhoc\nCodeDirectory flags=0x20002(adhoc,linker-signed)"
        self.assertEqual(PACKAGE.inspect_signature(sealed, bundle=True), "bundle-ad-hoc")
        self.assertEqual(PACKAGE.inspect_signature("Signature=adhoc"), "ad-hoc")
        self.assertEqual(PACKAGE.inspect_signature(linker, bundle=True), "linker-ad-hoc")

    def verify_fixture(self, identity=PACKAGE.SIMULATOR_APPLICATION_ID, der=b"matching DER", duplicate=False):
        xml = plistlib.dumps({"application-identifier": identity})

        def section(name, size, offset):
            return f"Section\n  sectname {name}\n  segname __TEXT\n  addr 0x1000\n  size 0x{size:x}\n  offset {offset}\n"

        loads = section("__entitlements", len(xml), 4) + section("__ents_der", len(der), 4 + len(xml))
        if duplicate:
            loads += section("__ents_der", len(der), 4 + len(xml))
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "BookNookPad"
            binary.write_bytes(b"test" + xml + der)

            def encode(arguments, **_):
                self.assertEqual(Path(arguments[arguments.index("-i") + 1]).read_bytes(), xml)
                Path(arguments[arguments.index("-o") + 1]).write_bytes(b"matching DER")

            with patch.object(PACKAGE.subprocess, "run", side_effect=encode):
                PACKAGE.verify_simulator_entitlements(binary, loads)

    def test_app_scoped_matching_entitlements_are_required(self):
        self.verify_fixture()
        for options in ({"identity": "OTHERTEAM0.org.booknook.ipad"}, {"der": b"other DER"}, {"duplicate": True}):
            with self.subTest(options=options), self.assertRaises(ValueError):
                self.verify_fixture(**options)


class LinkerSignatureTests(unittest.TestCase):
    def test_verification_uses_standalone_copy_of_exact_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            executable = Path(directory) / "BookNookMac.app/Contents/MacOS/BookNookMac"
            executable.parent.mkdir(parents=True)
            binary = b"linker-signed executable bytes"
            executable.write_bytes(binary)
            observed = []

            def verify(command, check):
                self.assertTrue(check)
                self.assertEqual(command[:-1], ["/usr/bin/codesign", "--verify", "--strict"])
                copy = Path(command[-1])
                self.assertEqual(copy.name, executable.name)
                self.assertFalse(any(parent.suffix == ".app" for parent in copy.parents))
                self.assertEqual(copy.read_bytes(), binary)
                observed.append(copy)

            with patch.object(PACKAGE.subprocess, "run", side_effect=verify):
                PACKAGE.verify_linker_signature(executable)
            self.assertEqual(len(observed), 1)
            self.assertFalse(observed[0].exists())
            self.assertEqual(executable.read_bytes(), binary)


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


class PublicContentTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="booknook-package-public-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.app = self.root / "BookNookPad.app"
        self.app.mkdir()
        (self.app / "Info.plist").write_bytes(plistlib.dumps({}))

    def test_local_paths_are_refused_in_each_encoding_without_echoing_contents(self):
        roots = ("/Users/", "/Volumes/", "/var/folders/", "/private/tmp/", "/tmp/", "/home/")
        for root in roots:
            for encoding in ("ascii", "utf-16-le", "utf-16-be"):
                with self.subTest(root=root, encoding=encoding):
                    (self.app / "BookNookPad").write_bytes(b"\x01" + (root + "private-path-sentinel").encode(encoding))
                    with self.assertRaisesRegex(ValueError, "private absolute path") as rejected:
                        PACKAGE.check_public_content(self.app, "ipad-device")
                    self.assertNotIn("private-path-sentinel", str(rejected.exception))
                    self.assertNotIn(root, str(rejected.exception))

    def test_path_signatures_are_detected_across_read_boundaries(self):
        for encoding in ("ascii", "utf-16-le", "utf-16-be"):
            with self.subTest(encoding=encoding), patch.object(PACKAGE, "PUBLIC_CONTENT_CHUNK_BYTES", 64):
                (self.app / "BookNookPad").write_bytes(b"x" * 63 + "/var/folders/private".encode(encoding))
                with self.assertRaisesRegex(ValueError, "private absolute path"):
                    PACKAGE.check_public_content(self.app, "ipad-sim")

    def test_provisioning_and_sqlite_store_members_are_refused(self):
        for name, contents in (("embedded.mobileprovision", b"synthetic"),
                               ("booknook.sqlite-wal", b"synthetic"),
                               ("feeds.sqlite-shm", b"synthetic"),
                               ("scratch.db-journal", b"synthetic"),
                               ("renamed.bin", b"SQLite format 3\x00" + b"synthetic")):
            with self.subTest(name=name):
                path = self.app / name
                path.write_bytes(contents)
                try:
                    with self.assertRaisesRegex(ValueError, "provisioning|SQLite"):
                        PACKAGE.check_public_content(self.app, "ipad-device")
                finally:
                    path.unlink()

    def test_qa_launch_override_is_refused_even_without_an_absolute_value(self):
        plist = self.app / "Info.plist"
        plist.write_bytes(plistlib.dumps({"LSEnvironment": {"BOOKNOOK_DATA_DIR": "qa-desk"}}, fmt=plistlib.FMT_BINARY))
        with self.assertRaisesRegex(ValueError, "QA launch-path override"):
            PACKAGE.check_public_content(self.app, "ipad-device")

    def test_remapped_paths_and_the_executable_environment_key_are_accepted(self):
        contents = ("/booknook-source/crates/core/lib.rs\x00"
                    "/booknook-rust/registry/package.rs\x00"
                    "/usr/lib/libSystem.B.dylib\x00BOOKNOOK_DATA_DIR")
        (self.app / "BookNookPad").write_bytes(contents.encode("ascii") + contents.encode("utf-16-le"))
        PACKAGE.check_public_content(self.app, "ipad-device")

    def test_opt_in_requires_release_configuration(self):
        argv = ["package-apple-apps.py", "--provenance", "unused", "--check-public-content"]
        with patch.object(sys, "argv", argv), patch("sys.stderr") as stderr, self.assertRaises(SystemExit) as rejected:
            PACKAGE.main()
        self.assertEqual(rejected.exception.code, 2)
        self.assertIn("--check-public-content requires --configuration Release", "".join(
            call.args[0] for call in stderr.write.call_args_list))

    def test_opt_in_preflights_all_inputs_and_screens_each_verified_extraction(self):
        source = self.root / "source"
        (source / "docs").mkdir(parents=True)
        (source / "Cargo.toml").write_text('[workspace.package]\nversion = "0.2.2"\n')
        for name in ("LICENSE-MPL-2.0", "LICENSE-MIT", "LICENSE-APACHE", "docs/dependency-licenses.csv"):
            (source / name).touch()
        build = (self.root / "build").resolve()
        apps = {key: PACKAGE.product_path(build, key, "Release") for key in PACKAGE.PRODUCTS}
        def info_path(app, key):
            return app / ("Contents/Info.plist" if key == "mac" else "Info.plist")

        app_info = {"CFBundleShortVersionString": "0.2.1", "CFBundleVersion": "2026111607"}
        for key, app in apps.items():
            plist = info_path(app, key)
            plist.parent.mkdir(parents=True)
            plist.write_bytes(plistlib.dumps(app_info))
        last = apps["ipad-device"] / "BookNookPad"
        last.write_bytes(b"/booknook-source/synthetic")
        provenance = self.root / "provenance.json"

        def save_provenance():
            provenance.write_text(json.dumps({"app_trees": {
                key: {"files": {name: {field: value for field, value in entry.items() if field != "mode"}
                                for name, entry in PACKAGE.manifest(app).items() if "directory" not in entry}}
                for key, app in apps.items()}}))

        output = self.root / "output"
        argv = ["package-apple-apps.py", "--output", str(output), "--build-root", str(build),
                "--provenance", str(provenance), "--configuration", "Release"]
        def inspect_fixture(app, key, _configuration):
            return {"info": plistlib.loads(info_path(app, key).read_bytes()), "mach_o": {}}

        with patch.object(PACKAGE, "ROOT", source), patch.object(PACKAGE, "inspect", side_effect=inspect_fixture), \
                patch("builtins.print"):
            device_info = info_path(apps["ipad-device"], "ipad-device")
            for field, value, diagnostic in [
                ("CFBundleShortVersionString", "0.2.0", "ipad-device: expected app version"),
                ("CFBundleVersion", "2026111606", "ipad-device: expected app version"),
                ("CFBundleVersion", "1/2", "ipad-device: invalid CFBundleVersion"),
            ]:
                with self.subTest(field=field, value=value):
                    device_info.write_bytes(plistlib.dumps({**app_info, field: value}))
                    save_provenance()
                    with patch.object(sys, "argv", argv + ["--check-public-content"]), \
                            patch.object(PACKAGE.tarfile, "open", wraps=PACKAGE.tarfile.open) as tar_open, \
                            self.assertRaisesRegex(ValueError, diagnostic):
                        PACKAGE.main()
                    self.assertFalse(output.exists())
                    tar_open.assert_not_called()
            device_info.write_bytes(plistlib.dumps(app_info))
            last.write_bytes(b"/home/private-path-sentinel")
            save_provenance()
            with patch.object(sys, "argv", argv + ["--check-public-content"]), \
                    self.assertRaisesRegex(ValueError, "private absolute path"):
                PACKAGE.main()
            self.assertFalse(output.exists())
            with patch.object(sys, "argv", argv):
                PACKAGE.main()
            self.assertNotIn("public_content_checked", json.loads((output / "VERIFICATION.json").read_text()))
            last.write_bytes(b"/booknook-source/crates/core/lib.rs")
            save_provenance()
            with patch.object(sys, "argv", argv + ["--check-public-content"]), \
                    patch.object(PACKAGE, "check_public_content", wraps=PACKAGE.check_public_content) as check:
                PACKAGE.main()
            self.assertEqual([call.args[0] for call in check.call_args_list[:3]], list(apps.values()))
            self.assertEqual([call.args[1] for call in check.call_args_list], list(apps) * 2)
            self.assertTrue(all(call.args[0] != apps[call.args[1]] for call in check.call_args_list[3:]))
            report = json.loads((output / "VERIFICATION.json").read_text())
            self.assertTrue(report["public_content_checked"])
            self.assertEqual(report["workspace_version"], "0.2.2")
            expected = {f"booknook-{key}-arm64-development-0.2.1.tar.gz" for key in apps}
            self.assertEqual({app["archive"] for app in report["apps"].values()}, expected)
            self.assertEqual({path.name for path in output.glob("*.tar.gz")}, expected)
            readme = (output / "README.md").read_text()
            self.assertIn("Book Nook 0.2.1 (build 2026111607)", readme)
            for name in expected:
                self.assertIn(f"`{name}`", readme)
            self.assertNotIn("0.2.2", readme)


if __name__ == "__main__":
    unittest.main()
