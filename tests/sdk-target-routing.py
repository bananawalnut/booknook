#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Synthetic routing regressions: no Cargo, npm, Apple or SDK compiler executes."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPTS = [
    "cargo_paths.py", "package-rust.sh", "package-typescript.sh",
    "package-swift.sh", "stage-sdk-deliverables.py", "build-apple.sh", "build-wasm.sh", "test-sdk.sh",
]
VERSION = "1.2.3"
NODE = shutil.which("node")
FAKE_TOOL = r'''
import io, json, os
from pathlib import Path
import shutil, sys, tarfile, tomllib

name = Path(sys.argv[0]).name
args = sys.argv[1:]
root = Path(os.environ["FIXTURE_ROOT"])
configured = root / ".cargo/config.toml"
config = tomllib.loads(configured.read_text()) if configured.exists() else {}
target = Path(os.environ.get("CARGO_TARGET_DIR", config.get("build", {}).get("target-dir", "target")))
if not target.is_absolute():
    target = root / target
target = target.resolve()
with Path(os.environ["FIXTURE_LOG"]).open("a") as log:
    log.write(json.dumps({"tool": name, "args": args, "cwd": str(Path.cwd()),
                         "npm_cache": os.environ.get("NPM_CONFIG_CACHE"),
                         "module_cache": os.environ.get("CLANG_MODULE_CACHE_PATH")}) + "\n")
def argument(flag):
    return args[args.index(flag) + 1]
def create(path, contents=b"synthetic fixture"):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(contents)
def executable(path, contents):
    create(path, contents.encode())
    path.chmod(0o755)
if name == "cargo":
    if args[0] == "metadata":
        assert args == ["metadata", "--locked", "--offline", "--no-deps", "--format-version", "1"]
        assert Path.cwd() == root, "Metadata must use the source workspace, not caller cwd"
        if "FIXTURE_METADATA_EXIT" in os.environ:
            sys.exit(int(os.environ["FIXTURE_METADATA_EXIT"]))
        print(os.environ.get("FIXTURE_METADATA_JSON", json.dumps({"target_directory": str(target)})))
    elif args[0] == "package":
        for crate in ["booknook-core", "booknook-connectors", "booknook-native", "booknook-wasm"]:
            archive = target / f"package/{crate}-1.2.3.crate"
            archive.parent.mkdir(parents=True, exist_ok=True)
            with tarfile.open(archive, "w:gz") as output:
                data = b"synthetic extracted crate"
                item = tarfile.TarInfo(f"{crate}-1.2.3/fixture.txt")
                item.size = len(data)
                output.addfile(item, io.BytesIO(data))
    elif args[0] == "test":
        assert Path(argument("--target-dir")) == target
        assert Path(argument("--manifest-path")) == target / "sdk-package-check/rust/Cargo.toml"
        assert Path(argument("--manifest-path")).is_file()
        for crate in ["core", "connectors", "native", "wasm"]:
            assert (target / f"sdk-package-check/rust/booknook-{crate}-1.2.3/fixture.txt").is_file()
    elif args[0] == "run":
        output = Path(argument("--out-dir"))
        assert Path(argument("--library")) == target / "release/libbooknook_native.dylib"
        assert Path(argument("--library")).is_file()
        for file in ["booknook_native.swift", "booknook_nativeFFI.h", "booknook_nativeFFI.modulemap"]:
            create(output / file, b"synthetic generated text  \n")
    elif args[0] == "build":
        if "--example" in args or "--bin" in args:
            for flag, folder in [("--example", "debug/examples"), ("--bin", "debug")]:
                if flag in args:
                    executable(target / folder / argument(flag),
                               f"#!{sys.executable}\n" + Path(__file__).read_text().split("\n", 1)[1])
        elif "--target" in args and argument("--target") == "wasm32-unknown-unknown":
            create(target / "wasm32-unknown-unknown/release/booknook_wasm.wasm")
        else:
            prefix = target / argument("--target") if "--target" in args else target
            create(prefix / "release/libbooknook_native.a")
            create(prefix / "release/libbooknook_native.dylib")
    else:
        raise AssertionError(args)
elif name == "wasm-bindgen":
    assert Path(args[0]) == target / "wasm32-unknown-unknown/release/booknook_wasm.wasm"
    assert Path(args[0]).is_file()
    create(Path(argument("--out-dir")) / "booknook_wasm.d.ts", b"export const fixture: number;\n")
    create(Path(argument("--out-dir")) / "booknook_wasm.js", b"// synthetic fixture\n")
elif name == "npm":
    if "pack" in args:
        destination = Path(argument("--pack-destination"))
        assert destination == target / "packages"
        create(destination / "booknook-sdk-1.2.3.tgz")
        print(json.dumps([{"filename": "booknook-sdk-1.2.3.tgz"}]))
    elif "install" in args:
        assert Path(argument("--prefix")) == target / "sdk-package-check/npm"
        assert Path(args[-1]) == target / "packages/booknook-sdk-1.2.3.tgz"
        assert Path(args[-1]).is_file()
    else:
        assert args[:3] == ["--prefix", "sdk/typescript", "run"] and args[3] in ["build", "check"], args
elif name == "node":
    assert Path(args[0]).is_file()
    if Path(args[0]).resolve().parent == root / "tests":
        no_facade = ["wasm-parity.cjs", "test-conformance.cjs"]
        direct_wasm = ["notebook-order-sdk.cjs", "feed-sdk-parity.cjs",
                       "feed-source-sdk.cjs", "feed-xml-sdk.cjs"]
        if Path(args[0]).name not in no_facade:
            assert Path(args[-1]) == target / "sdk-check/index.js", args
            assert Path(args[-1]).is_file()
        if Path(args[0]).name in direct_wasm:
            assert Path(args[1]).is_file()
    else:
        assert Path(args[0]).parent == target / "sdk-package-check/npm"
elif name == "tsc":
    if "--outDir" in args:
        assert Path(argument("--outDir")) == target / "sdk-check", args
        create(target / "sdk-check/index.js", b"// synthetic facade\n")
    elif Path(args[-1]).resolve().parent == root / "tests":
        assert Path(args[-1]).name in ["FeedSDKTypes.mts", "FeedSourceSDKTypes.mts", "FeedXMLSDKTypes.mts"]
        assert "--noEmit" in args and Path(args[-1]).is_file()
    else:
        assert all(Path(p).parent == target / "sdk-package-check/npm" for p in args[-2:])
        assert all(Path(p).is_file() for p in args[-2:])
elif name == "xcrun":
    if args[:2] == ["xcodebuild", "-create-xcframework"]:
        libraries = [Path(args[i+1]) for i, item in enumerate(args) if item == "-library"]
        expected = [target / "release/libbooknook_native.a",
                    target / "aarch64-apple-ios-sim/release/libbooknook_native.a",
                    target / "aarch64-apple-ios/release/libbooknook_native.a"]
        assert libraries == expected
        headers = Path(argument("-headers"))
        assert headers == target / "sdk-package-check/swift/headers"
        for platform, library in zip(["macos-arm64", "ios-arm64-simulator", "ios-arm64"], libraries):
            assert library.is_file()
            destination = Path(argument("-output")) / platform
            create(destination / "libbooknook_native.a", library.read_bytes())
            for header in ["booknook_nativeFFI.h", "module.modulemap"]:
                create(destination / "Headers" / header, (headers / header).read_bytes())
    else:
        assert args[:2] == ["swift", "build"], args
        assert Path(argument("--package-path")) == target / "sdk-package-check/swift/consumer"
        for flag, suffix in [("--scratch-path","build"),("--cache-path","cache"),
                             ("--config-path","config"),("--security-path","security")]:
            assert Path(argument(flag)) == target / f"swift-sdk-{suffix}"
        binary = target / "synthetic-swift-bin/Smoke"
        executable(binary, f"#!{sys.executable}\n" + Path(__file__).read_text().split("\n", 1)[1])
        if "--show-bin-path" in args:
            print(binary.parent)
elif name == "xcodebuild":
    library = next(a.split("=", 1)[1] for a in args if a.startswith("BOOKNOOK_RUST_LIBRARY="))
    assert Path(library).is_relative_to(target) and Path(library).is_file()
elif name == "xcodegen":
    assert args == ["generate"]
elif name == "Smoke":
    create(Path(os.environ["BOOKNOOK_SMOKE_FIXTURES"]) / "castalia-swift-receipt.json", b"{}")
elif name == "contract-probe":
    print(json.dumps({"executable": str(Path(__file__)), "input": json.loads(sys.stdin.read())}))
elif name == "booknook":
    print(json.dumps({"executable": str(Path(__file__)), "args": args, "input": sys.stdin.read()}))
elif name == "castalia-fixture":
    if args:
        assert args == ["verify"]
        assert sys.stdin.read().strip() == "{}"
    else:
        print("{}")
else:
    raise AssertionError((name, args))
'''


class RoutingFixture:
    def __init__(self, directory, mode="default"):
        directory = Path(directory).resolve()
        self.root = Path(directory) / "workspace with spaces"
        self.bin = Path(directory) / "fake tools"
        self.root.mkdir()
        self.bin.mkdir()
        self.log = Path(directory) / "commands.jsonl"
        self.env = {k: v for k, v in os.environ.items()
                    if k not in ["CARGO_TARGET_DIR", "CARGO_ENCODED_RUSTFLAGS"]
                    and not k.startswith(("FIXTURE_", "BOOKNOOK_"))}
        self.env.update(PATH=str(self.bin) + ":/usr/bin:/bin",
                        FIXTURE_ROOT=str(self.root), FIXTURE_LOG=str(self.log),
                        PYTHONDONTWRITEBYTECODE="1")
        if mode == "relative":
            self.env["CARGO_TARGET_DIR"] = "relative output 🌿"
        elif mode == "absolute":
            self.env["CARGO_TARGET_DIR"] = str(Path(directory) / "external output 🌿")
        elif mode == "edge-space":
            self.env["CARGO_TARGET_DIR"] = str(Path(directory) / " external output 🌿 ")
        elif mode == "config":
            self.write(".cargo/config.toml", '[build]\ntarget-dir = "configured output 🌿"\n')
        self.target = {"default": self.root / "target", "relative": self.root / "relative output 🌿",
                       "absolute": Path(directory) / "external output 🌿",
                       "config": self.root / "configured output 🌿",
                       "edge-space": Path(directory) / " external output 🌿 "}[mode]
        for script in SCRIPTS:
            self.write("scripts/" + script, (ROOT / "scripts" / script).read_text())
        for name in ["sdk-native.cjs", "sdk-target.cjs"]:
            source = ROOT / "tests" / name
            if source.exists():
                self.write("tests/" + name, source.read_text())
        self.write("scripts/sync-package-resources.py",
                   'import sys\nassert sys.argv[1:] == ["--check"]\n')
        self.write("Cargo.toml", '[workspace.package]\nversion = "1.2.3"\n')
        self.write("Cargo.lock", "# synthetic routing fixture\n")
        for name in ["cargo", "npm", "node", "wasm-bindgen", "xcrun", "xcodegen", "xcodebuild"]:
            path = self.bin / name
            path.write_text(f"#!{sys.executable}\n" + FAKE_TOOL)
            path.chmod(0o755)
        python = self.bin / "python3"
        python.write_text(f'#!/bin/sh\nexec "{sys.executable}" "$@"\n')
        python.chmod(0o755)
        tsc = self.write("sdk/typescript/node_modules/.bin/tsc", f"#!{sys.executable}\n" + FAKE_TOOL)
        tsc.chmod(0o755)
        for name in ["PackagedSDKSmoke.cjs","PackagedSDKSmoke.mjs","PackagedSDKTypes.mts",
                     "PackagedSDKTypes.cts","castalia-generic-fixture.cjs","provider-service-acceptance.cjs",
                     "notebook-order-sdk.cjs","public-group-preview-sdk.cjs",
                     "PackagedCastaliaSmoke.swift","PackagedProviderSmoke.swift",
                     "PackagedNotebookSmoke.swift","PackagedFeedSmoke.swift","fixtures/provider-sample.epub",
                     "sdk-storage.cjs","wasm-parity.cjs","test-conformance.cjs","castalia-generic-parity.cjs",
                     "provider-runtime.cjs","portable-archive-parity.cjs","feed-sdk-parity.cjs",
                     "feed-source-sdk.cjs","feed-xml-sdk.cjs","FeedSDKTypes.mts",
                     "FeedSourceSDKTypes.mts","FeedXMLSDKTypes.mts"]:
            self.write("tests/" + name, "synthetic fixture\n")
        self.write("crates/core/tests/fixtures/notebook-order.json", "synthetic fixture\n")
        self.write("crates/core/tests/fixtures/isbn-history.json", "synthetic fixture\n")
        for name in ["Package.swift","Sources/BookNookServices/BookNookProvider.swift",
                     "Sources/BookNookServices/BookNookNotebook.swift",
                     "Sources/CastaliaNative/CastaliaSession.swift",
                     "LICENSE-MIT","LICENSE-APACHE","DEPENDENCY-LICENSES.csv"]:
            self.write("sdk/swift/" + name, "synthetic fixture\n")
        for name in ["booknook_native.swift","booknook_nativeFFI.h","booknook_nativeFFI.modulemap"]:
            self.write("apps/apple/Generated/" + name, "synthetic fixture\n")

    def write(self, name, content):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content)
        return path

    def run(self, script, *args, cwd=None):
        executable = sys.executable if script.endswith(".py") else "/bin/bash"
        return subprocess.run([executable, str(self.root / "scripts" / script), *args],
                              env=self.env, cwd=cwd or self.root, capture_output=True, text=True, timeout=20)

    def commands(self):
        return [json.loads(line) for line in self.log.read_text().splitlines()] if self.log.exists() else []


class TargetRoutingTests(unittest.TestCase):
    def test_staging_uses_metadata_from_source_workspace_and_preserves_bytes(self):
        for mode in ["default", "relative", "absolute", "config"]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
                f = RoutingFixture(directory, mode)
                names = [f"package/booknook-{n}-{VERSION}.crate" for n in ["core","connectors","native","wasm"]]
                names += [f"packages/booknook-sdk-{VERSION}.tgz", f"packages/booknook-swift-sdk-{VERSION}.tar.gz"]
                for name in names:
                    p = f.target / name
                    p.parent.mkdir(parents=True, exist_ok=True)
                    p.write_bytes(("synthetic " + name).encode())
                output = Path(directory) / "staged files"
                r = f.run("stage-sdk-deliverables.py", "--output", str(output), cwd=Path(directory))
                self.assertEqual(r.returncode, 0, r.stderr)
                for name in names:
                    self.assertEqual((output / Path(name).name).read_bytes(), (f.target / name).read_bytes())
                sums = (output / "SHA256SUMS").read_text()
                self.assertEqual(len(sums.splitlines()), 6)
                for name in names:
                    self.assertIn(hashlib.sha256((f.target / name).read_bytes()).hexdigest(), sums)
                self.assertEqual(f.commands()[0]["cwd"], str(f.root))
                if mode != "default":
                    self.assertFalse((f.root / "target").exists())

    def test_rust_archive_and_verification_paths_share_cargo_target(self):
        for mode in ["default", "relative", "absolute", "config"]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
                f = RoutingFixture(directory, mode)
                r = f.run("package-rust.sh")
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertTrue((f.target / "sdk-package-check/rust/Cargo.lock").is_file())
                self.assertEqual([c["args"][0] for c in f.commands()], ["metadata","package","test"])
                if mode != "default":
                    self.assertFalse((f.root / "target").exists())

    def test_typescript_outputs_and_installed_checks_use_target(self):
        for mode in ["default", "absolute"]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
                f = RoutingFixture(directory, mode)
                r = f.run("package-typescript.sh")
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertTrue((f.target / f"packages/booknook-sdk-{VERSION}.tgz").is_file())
                installed_check = f.target / "sdk-package-check/npm"
                self.assertTrue((installed_check / "notebook-order-sdk.cjs").is_file())
                self.assertTrue((installed_check / "notebook-order.json").is_file())
                npm = [c for c in f.commands() if c["tool"] == "npm"]
                self.assertTrue(all(c["npm_cache"] == str(f.target / "npm-package-cache") for c in npm))
                if mode != "default":
                    self.assertFalse((f.root / "target").exists())

    def test_swift_use_built_routes_slices_and_consumer(self):
        with tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
            f = RoutingFixture(directory, "absolute")
            for suffix in ["release", "aarch64-apple-ios-sim/release", "aarch64-apple-ios/release"]:
                p = f.target / suffix / "libbooknook_native.a"
                p.parent.mkdir(parents=True, exist_ok=True)
                p.write_bytes(b"synthetic prebuilt slice")
            r = f.run("package-swift.sh", "--use-built")
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertTrue((f.target / f"packages/booknook-swift-sdk-{VERSION}.tar.gz").is_file())
            fixtures = f.target / "sdk-package-check/swift/fixtures"
            self.assertTrue((fixtures / "notebook-order.json").is_file())
            self.assertTrue((fixtures / "isbn-history.json").is_file())
            self.assertFalse(any(c["tool"] == "xcodebuild" for c in f.commands()))
            self.assertFalse((f.root / "target").exists())

    def test_default_swift_build_routes_every_apple_target_and_preserves_options(self):
        with tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
            f = RoutingFixture(directory, "config")
            f.env.update(BOOKNOOK_APP_BUILD_DIR=str(Path(directory) / "Apple products"),
                         BOOKNOOK_APP_CONFIGURATION="Release")
            r = f.run("package-swift.sh")
            self.assertEqual(r.returncode, 0, r.stderr)
            builds = [c for c in f.commands() if c["tool"] == "xcodebuild"]
            self.assertEqual(len(builds), 3)
            for c in builds:
                self.assertEqual(c["args"][c["args"].index("-configuration")+1], "Release")
                self.assertEqual(c["module_cache"], str(f.target / "swift-sdk-module-cache"))
                self.assertTrue(c["args"][c["args"].index("-derivedDataPath")+1].startswith(f.env["BOOKNOOK_APP_BUILD_DIR"]))
            self.assertTrue((f.target / "apple-release-generated-normalization/mac/receipt.json").is_file())
            self.assertFalse((f.root / "target").exists())

    def test_apple_default_debug_product_placement_is_unchanged(self):
        with tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
            f = RoutingFixture(directory)
            r = f.run("build-apple.sh")
            self.assertEqual(r.returncode, 0, r.stderr)
            build = next(c for c in f.commands() if c["tool"] == "xcodebuild")
            self.assertEqual(build["args"][build["args"].index("-configuration")+1], "Debug")
            self.assertEqual(build["args"][build["args"].index("-derivedDataPath")+1], "Build/mac")
            self.assertEqual(build["cwd"], str(f.root / "apps/apple"))

    def test_bad_metadata_refuses_every_entrypoint_before_output(self):
        invalid = ["not json", "{}", '{"target_directory":null}', '{"target_directory":17}',
                   '{"target_directory":"relative"}', '{"target_directory":"/tmp/not-used\\u0000"}']
        invalid.append(json.dumps({"target_directory": "/tmp/not-used" + chr(10)}))
        scripts = [("package-rust.sh",()),("package-typescript.sh",()),("package-swift.sh",("--use-built",)),
                   ("build-apple.sh",()),("build-wasm.sh",()),("test-sdk.sh",()),
                   ("stage-sdk-deliverables.py",("--output", "not-created"))]
        for metadata in [*invalid, None]:
            for script, args in scripts:
                with self.subTest(metadata=metadata, script=script), tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
                    f = RoutingFixture(directory, "absolute")
                    if metadata is None:
                        f.env["FIXTURE_METADATA_EXIT"] = "9"
                    else:
                        f.env["FIXTURE_METADATA_JSON"] = metadata
                    r = f.run(script, *args)
                    self.assertNotEqual(r.returncode, 0)
                    self.assertFalse(f.target.exists())
                    self.assertFalse((f.root / "target").exists())
                    self.assertFalse((f.root / "not-created").exists())
                    self.assertTrue(all(c["tool"] == "cargo" and c["args"][0] == "metadata" for c in f.commands()))

    def test_source_sdk_driver_routes_all_outputs_without_repository_target(self):
        for mode in ["default", "relative", "absolute", "config", "edge-space"]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
                f = RoutingFixture(directory, mode)
                r = f.run("test-sdk.sh", cwd=Path(directory))
                self.assertEqual(r.returncode, 0, r.stderr)
                self.assertTrue((f.target / "sdk-check/index.js").is_file())
                self.assertTrue((f.target / "debug/booknook").is_file())
                self.assertTrue((f.target / "debug/examples/contract-probe").is_file())
                self.assertTrue((f.target / "debug/examples/portable-fixture").is_file())
                order_checks = [c["args"][1] for c in f.commands() if c["tool"] == "node"
                                and c["args"][0] == "tests/notebook-order-sdk.cjs"]
                self.assertEqual(order_checks, [
                    "sdk/typescript/pkg/nodejs/booknook_wasm.js",
                    "sdk/typescript/pkg/web/booknook_wasm.js",
                ])
                if mode != "default":
                    self.assertFalse((f.root / "target").exists())

    @unittest.skipUnless(NODE, "A Node interpreter is required for actual helper routing, not SDK execution")
    def test_actual_node_native_helpers_use_authoritative_cargo_paths(self):
        for mode in ["default", "relative", "absolute", "config", "edge-space"]:
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
                f = RoutingFixture(directory, mode)
                source = f.bin / "cargo"
                for name in ["debug/booknook", "debug/examples/contract-probe"]:
                    p = f.target / name
                    p.parent.mkdir(parents=True, exist_ok=True)
                    p.write_bytes(source.read_bytes())
                    p.chmod(0o755)
                code = """const assert=require('node:assert/strict');
const native=require(process.argv[1]);const expected=process.argv[2];
assert.deepEqual(native.nativeProbe({op:'route'}),{executable:expected+'/debug/examples/contract-probe',input:{op:'route'}});
const cli=JSON.parse(native.nativeCli('desk with spaces',['snapshot'],{unicode:'🌿'}));
assert.equal(cli.executable,expected+'/debug/booknook');
assert.deepEqual(cli.args,['desk with spaces','snapshot']);
assert.equal(cli.input,JSON.stringify({unicode:'🌿'}));
const {targetPath}=require(process.argv[3]);
assert.equal(targetPath('sdk-check','index.js'),expected+'/sdk-check/index.js');
"""
                r = subprocess.run([NODE, "-e", code, str(f.root/"tests/sdk-native.cjs"),
                                    str(f.target), str(f.root/"tests/sdk-target.cjs")],
                                   env=f.env, cwd=Path(directory), capture_output=True, text=True, timeout=10)
                self.assertEqual(r.returncode, 0, r.stderr)
                metadata = [c for c in f.commands() if c["tool"] == "cargo"]
                self.assertEqual(len(metadata), 1, "One authoritative metadata query is reused per Node process")
                self.assertEqual(metadata[0]["cwd"], str(f.root))
                if mode != "default":
                    self.assertFalse((f.root / "target").exists())

    @unittest.skipUnless(NODE, "A Node interpreter is required for actual helper routing, not SDK execution")
    def test_actual_node_native_helpers_reject_bad_metadata_before_native_execution(self):
        invalid = ["not json", "{}", '{"target_directory":null}', '{"target_directory":17}',
                   '{"target_directory":"relative"}', '{"target_directory":"/tmp/not-used\\u0000"}',
                   json.dumps({"target_directory":"/tmp/not-used\n"})]
        for metadata in [*invalid, None]:
            with self.subTest(metadata=metadata), tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
                f = RoutingFixture(directory, "absolute")
                if metadata is None:
                    f.env["FIXTURE_METADATA_EXIT"] = "9"
                else:
                    f.env["FIXTURE_METADATA_JSON"] = metadata
                code = "require(process.argv[1]).nativeProbe({op:'must-not-execute'})"
                r = subprocess.run([NODE, "-e", code, str(f.root/"tests/sdk-native.cjs")],
                                   env=f.env, cwd=Path(directory), capture_output=True, text=True, timeout=10)
                self.assertNotEqual(r.returncode, 0)
                self.assertFalse(f.target.exists())
                self.assertFalse((f.root/"target").exists())
                self.assertEqual([(c["tool"],c["args"][0]) for c in f.commands()], [("cargo","metadata")])

    def test_staging_help_and_module_import_have_no_cargo_side_effect(self):
        with tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
            f = RoutingFixture(directory)
            r = f.run("stage-sdk-deliverables.py", "--help")
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertFalse(f.log.exists())
            script = f.root / "scripts/stage-sdk-deliverables.py"
            code = ("import importlib.util,sys; sys.path.insert(0, " + repr(str(script.parent)) + "); "
                    "spec=importlib.util.spec_from_file_location('stage_fixture', " + repr(str(script)) + "); "
                    "module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module)")
            r = subprocess.run([sys.executable, "-c", code], env=f.env, cwd=Path(directory),
                               capture_output=True, text=True, timeout=10)
            self.assertEqual(r.returncode, 0, r.stderr)
            self.assertFalse(f.log.exists())

    def test_existing_invalid_swift_flag_still_refuses_without_tools(self):
        with tempfile.TemporaryDirectory(prefix="booknook-routing-") as directory:
            f = RoutingFixture(directory)
            r = f.run("package-swift.sh", "--invalid")
            self.assertEqual(r.returncode, 2)
            self.assertFalse(f.log.exists())


if __name__ == "__main__":
    unittest.main()
