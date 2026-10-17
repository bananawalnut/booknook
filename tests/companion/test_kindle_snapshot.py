# SPDX-License-Identifier: MIT OR Apache-2.0
"""Synthetic SQLite snapshots with a real Apple Foundation-produced archive.

No reader account, real cache, credential or content file is used by this suite.
"""
import http.client
import importlib.util
import json
import os
import plistlib
import sqlite3
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest.mock import patch

ROOT = Path(__file__).parents[2]
SCRIPT = ROOT / "services/library-companion/booknook_companion.py"
spec = importlib.util.spec_from_file_location("kindle_companion", SCRIPT)
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
FIXTURE = Path(__file__).parent / "fixtures/kindle-metadata.binary.plist"


def snapshot(blob=None, title="SQLite display title", rows=1, schema=None):
    db = sqlite3.connect(":memory:")
    try:
        db.execute(schema or "CREATE TABLE ZBOOK (Z_PK INTEGER PRIMARY KEY, ZDISPLAYTITLE VARCHAR, ZSYNCMETADATAATTRIBUTES BLOB, ZRAWCURRENTPOSITION INTEGER, ZRAWLASTACCESSTIME INTEGER)")
        if schema is None:
            for i in range(rows):
                db.execute("INSERT INTO ZBOOK VALUES (?,?,?,?,?)", (i + 1, title, FIXTURE.read_bytes() if blob is None else blob, 9876, 123456789))
            db.execute("CREATE TABLE PRIVATE_DATA (value TEXT)")
            db.execute("INSERT INTO PRIVATE_DATA VALUES ('synthetic-private-field-never-export')")
        db.commit()
        return db.serialize()
    finally:
        db.close()


def altered(change):
    archive = plistlib.loads(FIXTURE.read_bytes())
    change(archive)
    return plistlib.dumps(archive, fmt=plistlib.FMT_BINARY)


class KindleSnapshotParserTests(unittest.TestCase):
    def test_bom_encoded_metadata_xml_keeps_entity_rejection(self):
        xml = '<response><meta_data><ASIN>B012345678</ASIN><title>Unicode 二</title><authors><author>Écrivain</author></authors></meta_data></response>'
        forbidden = '<!DOCTYPE x [<!ENTITY a SYSTEM "file:///private/unread">]><x>&a;</x>'
        for encoding in ("utf-8-sig", "utf-16", "utf-32"):
            with self.subTest(encoding=encoding):
                self.assertEqual(m.normalize_catalog(xml.encode(encoding))["items"][0]["title"], "Unicode 二")
                with self.assertRaises(m.ClosedError): m.normalize_catalog(forbidden.encode(encoding))

    def test_foundation_archive_imports_only_explicit_metadata(self):
        # This archive was emitted by NSKeyedArchiver, not our decoder's own encoder.
        value = m.normalize_catalog(snapshot())
        self.assertEqual(value, {"schema": "booknook.kindle-catalog.v1", "items": [{
            "asin": "B012345678", "title": "SQLite display title",
            "authors": ["Synthetic Writer", "Écrivain 二"], "position": None, "observed_at": None}]})
        self.assertNotIn("synthetic-do-not-import", json.dumps(value))
        self.assertNotIn("9876", json.dumps(value))

    def test_rejects_missing_columns_views_wrong_affinity_and_truncation(self):
        schemas = [
            "CREATE TABLE OTHER (a TEXT)",
            "CREATE VIEW ZBOOK AS SELECT 1 AS Z_PK, 'x' AS ZDISPLAYTITLE, NULL AS ZSYNCMETADATAATTRIBUTES",
            "CREATE TABLE ZBOOK (Z_PK INTEGER PRIMARY KEY, ZDISPLAYTITLE VARCHAR, ZSYNCMETADATAATTRIBUTES TEXT)",
            "CREATE TABLE ZBOOK (Z_PK INTEGER PRIMARY KEY, ZDISPLAYTITLE VARCHAR GENERATED ALWAYS AS ('x'), ZSYNCMETADATAATTRIBUTES BLOB)",
        ]
        for schema in schemas:
            with self.subTest(schema=schema), self.assertRaises(m.ClosedError) as error:
                m.normalize_catalog(snapshot(schema=schema))
            self.assertEqual(error.exception.code, "kindle_snapshot_schema")
        with self.assertRaises(m.ClosedError):
            m.normalize_catalog(snapshot()[:-1])

    def test_rejects_cycles_unknown_classes_bad_references_and_duplicate_keys(self):
        def cycle(a): a["$objects"][1]["attributes"] = plistlib.UID(1)
        def unknown(a):
            c = a["$objects"][a["$objects"][1]["$class"].data]
            c["$classname"] = "ExecutableUnarchiverClass"; c["$classes"][0] = c["$classname"]
        def out_of_range(a): a["$top"]["root"] = plistlib.UID(100000)
        def duplicate(a):
            d = a["$objects"][2]; d["NS.keys"].append(d["NS.keys"][0]); d["NS.objects"].append(d["NS.objects"][0])
        def malformed_class(a): a["$objects"][a["$objects"][1]["$class"].data]["$classname"] = []
        for mutation in (cycle, unknown, out_of_range, duplicate, malformed_class):
            with self.subTest(mutation=mutation.__name__), self.assertRaises(m.ClosedError):
                m.normalize_catalog(snapshot(altered(mutation)))

    def test_no_asin_fallback_duplicates_invalid_authors_and_titles(self):
        def replace_string(old, new):
            return altered(lambda a: a["$objects"].__setitem__(a["$objects"].index(old), new))
        for blob in (replace_string("B012345678", ""), replace_string("Synthetic Writer", "line\nbreak"), replace_string("Synthetic Writer", "二" * 167)):
            with self.assertRaises(m.ClosedError): m.normalize_catalog(snapshot(blob))
        with self.assertRaises(m.ClosedError): m.normalize_catalog(snapshot(rows=2))
        with self.assertRaises(m.ClosedError): m.normalize_catalog(snapshot(title="\tcontrol"))
        with self.assertRaises(m.ClosedError): m.normalize_catalog(snapshot(title="二" * 667))

    def test_strict_bounds_for_file_blob_archive_objects_rows_and_schema(self):
        with self.assertRaises(m.ClosedError) as error:
            m.normalize_catalog(m.SQLITE_MAGIC + b"\x00" * m.MAX_BYTES)
        self.assertEqual(error.exception.status, 413)
        with self.assertRaises(m.ClosedError): m.normalize_catalog(snapshot(b"bplist00" + b"a" * m.MAX_METADATA_BYTES))
        too_many = bytearray(FIXTURE.read_bytes()); too_many[-24:-16] = (10001).to_bytes(8, "big")
        with self.assertRaises(m.ClosedError): m.inert_metadata(bytes(too_many))
        with patch.object(m, "MAX_CATALOG_ITEMS", 0), self.assertRaises(m.ClosedError):
            m.normalize_catalog(snapshot())
        db = sqlite3.connect(":memory:")
        try:
            for i in range(257): db.execute(f"CREATE TABLE T{i} (a TEXT)")
            with self.assertRaises(m.ClosedError): m.normalize_catalog(db.serialize())
        finally: db.close()

    def test_wal_rejected_and_consistent_backup_delete_mode_accepted(self):
        with tempfile.TemporaryDirectory() as tmp:
            live_path, backup_path = Path(tmp) / "selected.sqlite", Path(tmp) / "snapshot.sqlite"
            live = sqlite3.connect(live_path)
            live.deserialize(snapshot())
            # Materialize the independent fixture through SQLite's backup API.
            disk = sqlite3.connect(live_path); live.backup(disk); live.close()
            disk.execute("PRAGMA journal_mode=WAL"); disk.execute("UPDATE ZBOOK SET ZDISPLAYTITLE='WAL committed title'"); disk.commit()
            with self.assertRaises(m.ClosedError) as error: m.normalize_catalog(live_path.read_bytes())
            self.assertEqual(error.exception.code, "kindle_snapshot_wal")
            backup = sqlite3.connect(backup_path); disk.backup(backup); backup.execute("PRAGMA journal_mode=DELETE"); backup.close(); disk.close()
            self.assertEqual(m.normalize_catalog(backup_path.read_bytes())["items"][0]["title"], "WAL committed title")


class KindleSnapshotServiceTests(unittest.TestCase):
    def test_hung_capability_process_returns_local_fallback_before_native_timeout(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            hung = root / "hung.py"; hung.write_text("import time\ntime.sleep(30)\n")
            source = root / "selected.sqlite"; source.write_bytes(snapshot())
            service = m.Companion(root / "data", provider=[sys.executable, str(hung)], catalog=source)
            try:
                # Real child process and kill/reap path; shorten only the probe's
                # test clock budget, not source behavior or the production API.
                with patch.object(m, "CAPABILITIES_TIMEOUT", 0.1):
                    started = time.monotonic()
                    paired = service.handle("POST", "/v1/pair", {}, json.dumps({"schema":"booknook.companion-pair.v1","code":service.code,"device_name":"timeout test"}).encode())
                    self.assertLess(time.monotonic() - started, 3)
                    self.assertEqual(paired["manifest"]["capabilities"], ["kindle.catalog", "kindle.catalog.import", "provider_connections_v1"])
                    self.assertFalse(paired["manifest"]["qualified"])
                    headers = {"Authorization": "Bearer " + paired["token"]}
                    self.assertEqual(service.handle("GET", "/v1/kindle/catalog", headers, b"")["items"][0]["asin"], "B012345678")
                    with self.assertRaises(m.ClosedError) as error:
                        service.handle("POST", "/v1/audible/positions", headers, b'{"asins":["B012345678"]}')
                    self.assertEqual(error.exception.code, "source_unavailable")
            finally: service.db.close()

    def test_unavailable_adapter_does_not_block_local_pairing_or_catalog(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            crashed = root / "crashed.py"; crashed.write_text("import sys\nsys.stderr.write('synthetic-private-failure')\nsys.exit(1)\n")
            invalid = root / "invalid.py"; invalid.write_text("print('[1,2,3]')\n")
            recovered = root / "recovered.py"; recovered.write_text("print('{\"schema\":\"booknook.source-capabilities.v1\",\"capabilities\":[\"audible.positions\"],\"qualified\":true}')\n")
            providers = [[str(root / "synthetic-private-missing")], [sys.executable, str(crashed)], [sys.executable, str(invalid)]]
            for mode in ("configured", "uploaded"):
                for i, provider in enumerate(providers):
                    with self.subTest(mode=mode, provider=i):
                        data = root / f"{mode}-{i}"; data.mkdir()
                        source = root / "selected.sqlite"; source.write_bytes(snapshot(title="Selected B"))
                        if mode == "uploaded":
                            (data / "kindle-catalog.json").write_text(json.dumps(m.normalize_catalog(source.read_bytes())))
                        service = m.Companion(data, provider=provider, catalog=source if mode == "configured" else None)
                        try:
                            paired = service.handle("POST", "/v1/pair", {}, json.dumps({"schema":"booknook.companion-pair.v1","code":service.code,"device_name":"outage test"}).encode())
                            headers = {"Authorization": "Bearer " + paired["token"]}
                            manifests = [paired["manifest"], service.handle("GET", "/v1/capabilities", headers, b"")]
                            for manifest in manifests:
                                self.assertEqual(set(manifest), {"schema", "name", "capabilities", "qualified"})
                                self.assertEqual(manifest["capabilities"], ["kindle.catalog", "kindle.catalog.import", "provider_connections_v1"])
                                self.assertFalse(manifest["qualified"])
                                self.assertIn("adapter unavailable", manifest["name"])
                                self.assertNotIn("synthetic-private", json.dumps(manifest))
                            self.assertEqual(service.handle("GET", "/v1/kindle/catalog", headers, b"")["items"][0]["title"], "Selected B")
                            for path, body in (("/v1/audible/positions", b'{"asins":["B012345678"]}'), ("/v1/accounts/authorize", b'{"provider":"audible","marketplace":"us"}')):
                                with self.assertRaises(m.ClosedError) as error: service.handle("POST", path, headers, body)
                                self.assertIn(error.exception.code, {"source_unavailable", "invalid_source_data"})
                                self.assertNotIn("synthetic-private", error.exception.message)
                            # Recovery is checked anew, not replaced by stale local-only state.
                            service.provider = [sys.executable, str(recovered)]
                            ready = service.handle("GET", "/v1/capabilities", headers, b"")
                            self.assertIn("audible.positions", ready["capabilities"])
                            self.assertTrue(ready["qualified"])
                            self.assertNotIn("unavailable", ready["name"])
                        finally: service.db.close()

    def test_user_upload_replaces_configured_source_and_survives_restart(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); source = root / "selected.sqlite"; source.write_bytes(snapshot(title="Source A"))
            data = root / "companion"
            service = m.Companion(data, catalog=source)
            try:
                pair = service.handle("POST", "/v1/pair", {}, json.dumps({"schema":"booknook.companion-pair.v1","code":service.code,"device_name":"source switch"}).encode())
                headers = {"Authorization": "Bearer " + pair["token"]}
                self.assertEqual(service.handle("GET", "/v1/kindle/catalog", headers, b"")["items"][0]["title"], "Source A")
                service.handle("PUT", "/v1/kindle/catalog", headers, snapshot(title="Selected B"), raw_sqlite_allowed=True)
                # Even a configured live provider must not shadow the uploaded selection.
                service.provider = ["unavailable-provider-must-not-be-called"]
                self.assertEqual(service.handle("GET", "/v1/kindle/catalog", headers, b"")["items"][0]["title"], "Selected B")
            finally: service.db.close()
            reopened = m.Companion(data, provider=["unavailable-provider-must-not-be-called"])
            try:
                self.assertEqual(reopened.handle("GET", "/v1/kindle/catalog", headers, b"")["items"][0]["title"], "Selected B")
            finally: reopened.db.close()
            # Restarting the same command must not silently restore old source A.
            override = m.Companion(data, catalog=source)
            try:
                self.assertEqual(override.handle("GET", "/v1/kindle/catalog", headers, b"")["items"][0]["title"], "Selected B")
            finally: override.db.close()

    def test_operator_selected_source_is_normalized_before_serving_and_never_saved_raw(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); source = root / "selected.sqlite"; source.write_bytes(snapshot())
            service = m.Companion(root / "companion", catalog=source)
            try:
                paired = service.handle("POST", "/v1/pair", {}, json.dumps({"schema":"booknook.companion-pair.v1","code":service.code,"device_name":"test"}).encode())
                headers = {"Authorization": "Bearer " + paired["token"]}
                source.unlink()  # Serving uses the checked snapshot, never lazy raw cache access.
                result = service.handle("GET", "/v1/kindle/catalog", headers, b"")
                self.assertEqual(result["items"][0]["asin"], "B012345678")
                self.assertFalse(paired["manifest"]["qualified"])
                self.assertNotIn("kindle.authorize", paired["manifest"]["capabilities"])
                for file in (root / "companion").iterdir():
                    self.assertNotIn(b"synthetic-private-field-never-export", file.read_bytes())
            finally: service.db.close()

    def test_http_upload_persists_only_catalog_and_failure_preserves_last_import(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp); service = m.Companion(root)
            server = m.http.server.ThreadingHTTPServer(("127.0.0.1", 0), m.Handler); server.companion = service
            worker = threading.Thread(target=server.serve_forever, daemon=True); worker.start()
            def call(method, path, body=b"", headers=None):
                client = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=5)
                try:
                    client.request(method, path, body, headers or {})
                    response = client.getresponse(); return response.status, json.loads(response.read())
                finally: client.close()
            try:
                _, pair = call("POST", "/v1/pair", json.dumps({"schema":"booknook.companion-pair.v1","code":service.code,"device_name":"HTTP test"}).encode())
                headers = {"Authorization": "Bearer " + pair["token"]}
                self.assertEqual(call("PUT", "/v1/kindle/catalog", snapshot(), headers), (200, {"imported": 1}))
                persisted = (root / "kindle-catalog.json").read_bytes()
                self.assertEqual(json.loads(persisted)["items"][0]["asin"], "B012345678")
                self.assertEqual(os.stat(root / "kindle-catalog.json").st_mode & 0o777, 0o600)
                self.assertEqual(call("PUT", "/v1/kindle/catalog", snapshot(b"unsupported"), headers)[0], 422)
                self.assertEqual((root / "kindle-catalog.json").read_bytes(), persisted)
                # No caller-controlled header/parameter can opt into raw remote upload.
                with self.assertRaises(m.ClosedError) as denied:
                    service.handle("PUT", "/v1/kindle/catalog", dict(headers, Host="localhost", **{"X-Forwarded-For":"127.0.0.1"}), snapshot())
                self.assertEqual(denied.exception.code, "kindle_snapshot_local_only")
                for file in root.iterdir(): self.assertNotIn(b"synthetic-private-field-never-export", file.read_bytes())
            finally:
                server.shutdown(); server.server_close(); worker.join(); service.db.close()

    def test_actual_cli_export_is_private_no_overwrite_no_secret_logging(self):
        with tempfile.TemporaryDirectory() as tmp:
            source, output = Path(tmp) / "selected.sqlite", Path(tmp) / "catalog.json"
            source.write_bytes(snapshot())
            args = [sys.executable, str(SCRIPT), "--export-kindle-catalog", str(source), "--output", str(output)]
            result = subprocess.run(args, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertNotIn(b"synthetic-private", result.stdout + result.stderr)
            self.assertEqual(os.stat(output).st_mode & 0o777, 0o600)
            expected = output.read_bytes(); self.assertEqual(json.loads(expected)["items"][0]["authors"], ["Synthetic Writer", "Écrivain 二"])
            self.assertNotEqual(subprocess.run(args, capture_output=True).returncode, 0)
            self.assertEqual(output.read_bytes(), expected)
            source.write_bytes(snapshot(b"synthetic-secret-not-an-archive")); output.unlink()
            failed = subprocess.run(args, capture_output=True)
            self.assertNotEqual(failed.returncode, 0); self.assertFalse(output.exists())
            self.assertNotIn(b"synthetic-secret", failed.stdout + failed.stderr)


if __name__ == "__main__":
    unittest.main()
