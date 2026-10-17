#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Optional local source companion. No Amazon credentials or DRM implementation is bundled.

A source adapter is a separately installed executable speaking the documented JSON contract.
The local Kindle source imports reader-selected JSON, metadata XML, or a standalone
Kindle-for-Mac SQLite metadata snapshot. It never discovers local Kindle files.
"""
from __future__ import annotations
import argparse
import hashlib
import hmac
import http.server
import ipaddress
import json
import os
import plistlib
import re
import secrets
import selectors
import sqlite3
import ssl
import subprocess
import threading
import time
import unicodedata
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path

MAX_BYTES = 32 * 1024 * 1024
SQLITE_MAGIC = b"SQLite format 3\x00"
MAX_CATALOG_ITEMS = 10000
MAX_METADATA_BYTES = 1024 * 1024
CAPABILITIES_TIMEOUT = 3.0  # Deliver local fallback before the native 25-second request deadline.

class ClosedError(Exception):
    def __init__(self, status, message, code=None):
        self.status, self.message = status, message
        self.code = code or {400: "invalid_request", 401: "pairing_denied", 403: "pairing_denied", 404: "unsupported_operation", 413: "payload_too_large", 422: "invalid_source_data", 502: "source_failure", 503: "source_unavailable", 504: "source_timeout"}.get(status, "internal_error")

def strict_json(raw):
    def unique(pairs):
        obj = {}
        for key, value in pairs:
            if key in obj:
                raise ClosedError(400, "Duplicate JSON key")
            obj[key] = value
        return obj
    try:
        return json.loads(raw, object_pairs_hook=unique, parse_constant=lambda _: (_ for _ in ()).throw(ValueError("nonfinite")))
    except (ValueError, UnicodeError) as error:
        raise ClosedError(400, "Invalid JSON") from error

def valid_asin(value):
    return isinstance(value, str) and len(value) == 10 and value.isascii() and value.isalnum()

def validate_provider_connections(value):
    """Strict public Castalia descriptors; no credentials cross this boundary."""
    def identifier(text, lower=False):
        pattern = r"[a-z0-9][a-z0-9._:-]{0,127}" if lower else r"[A-Za-z0-9][A-Za-z0-9._:-]{0,127}"
        return isinstance(text, str) and re.fullmatch(pattern, text) is not None
    if not isinstance(value, list) or len(value) > 64:
        raise ClosedError(502, "Invalid provider connection list", "invalid_source_data")
    seen = set()
    for item in value:
        if not isinstance(item, dict) or set(item) != {"schema", "connection_id", "provider_id", "display_name", "capabilities", "status", "credential"}:
            raise ClosedError(502, "Invalid provider connection descriptor", "invalid_source_data")
        label, caps, credential = item["display_name"], item["capabilities"], item["credential"]
        try:
            valid_label = isinstance(label, str) and 1 <= len(label.encode("utf-8")) <= 160 and not any(ord(c) < 32 or 127 <= ord(c) <= 159 or ord(c) in (0x061C, 0x200E, 0x200F) or 0x202A <= ord(c) <= 0x202E or 0x2066 <= ord(c) <= 0x2069 for c in label)
            bounded = len(json.dumps(item, ensure_ascii=False, separators=(",", ":")).encode("utf-8")) <= 16384
        except UnicodeError:
            valid_label = bounded = False
        if (item["schema"] != "castalia.provider-connection.v1" or not identifier(item["connection_id"])
                or item["connection_id"] in seen or not identifier(item["provider_id"], True) or not valid_label or not bounded
                or not isinstance(caps, list) or len(caps) > 64 or not all(identifier(cap, True) for cap in caps)
                or len(set(caps)) != len(caps) or item["status"] not in ("available", "reauthorization_required", "unavailable")):
            raise ClosedError(502, "Invalid provider connection descriptor", "invalid_source_data")
        if credential is not None and (not isinstance(credential, dict) or set(credential) != {"reference", "storage", "portability"}
                or not identifier(credential["reference"]) or credential["storage"] not in ("secure_local", "wallet_managed")
                or credential["portability"] not in ("device_bound", "reauthorization_required", "provider_defined")):
            raise ClosedError(502, "Invalid public credential reference", "invalid_source_data")
        seen.add(item["connection_id"])
    return value

def kindle_snapshot_error(message, code="kindle_snapshot_invalid"):
    return ClosedError(422, message, code)

def inert_metadata(raw):
    """Parse data, never instantiate archived classes or call NSKeyedUnarchiver.

    Original bounded parser of the format documented by listKindleBooks2; see
    SOURCE-EVIDENCE.md. Unknown classes, cycles and duplicate keys fail closed.
    """
    if not isinstance(raw, bytes) or not 40 <= len(raw) <= MAX_METADATA_BYTES or not raw.startswith(b"bplist00"):
        raise kindle_snapshot_error("Expected bounded binary Kindle metadata archive")
    # Bound allocation before plistlib creates its object table.
    if not 1 <= int.from_bytes(raw[-24:-16], "big") <= 10000:
        raise kindle_snapshot_error("Kindle metadata object count is unsupported")
    try:
        archive = plistlib.loads(raw)
    except (ValueError, TypeError, OverflowError, RecursionError, IndexError) as error:
        raise kindle_snapshot_error("Invalid Kindle metadata property list") from error
    if (not isinstance(archive, dict) or set(archive) != {"$archiver", "$version", "$objects", "$top"}
            or archive["$archiver"] != "NSKeyedArchiver" or archive["$version"] != 100000
            or not isinstance(archive["$objects"], list) or not 1 <= len(archive["$objects"]) <= 10000
            or archive["$objects"][0] != "$null" or not isinstance(archive["$top"], dict)
            or set(archive["$top"]) != {"root"}):
        raise kindle_snapshot_error("Unsupported Kindle metadata archive schema")
    objects, active, budget = archive["$objects"], set(), [50000]
    allowed = {"SyncMetadataAttributes", "NSDictionary", "NSMutableDictionary", "NSArray", "NSMutableArray", "NSString", "NSMutableString", "NSNumber", "NSData", "NSMutableData"}

    def index(ref):
        if not isinstance(ref, plistlib.UID) or not 0 <= ref.data < len(objects):
            raise kindle_snapshot_error("Invalid metadata object reference")
        return ref.data

    def kind(value):
        descriptor = objects[index(value.get("$class"))]
        if (not isinstance(descriptor, dict) or set(descriptor) != {"$classname", "$classes"}
                or not isinstance(descriptor["$classname"], str) or descriptor["$classname"] not in allowed or not isinstance(descriptor["$classes"], list)
                or not 2 <= len(descriptor["$classes"]) <= 4
                or descriptor["$classes"][0] != descriptor["$classname"] or descriptor["$classes"][-1] != "NSObject"
                or any(not isinstance(c, str) or c not in allowed | {"NSObject"} for c in descriptor["$classes"])):
            raise kindle_snapshot_error("Unsupported metadata archive class")
        return descriptor["$classname"]

    def resolve(ref, depth=0):
        slot = index(ref)
        budget[0] -= 1
        if depth > 40 or budget[0] < 0 or slot in active:
            raise kindle_snapshot_error("Cyclic or excessively nested metadata archive")
        value = objects[slot]
        if slot == 0:
            return None
        if isinstance(value, (str, bytes, bool, int, float)):
            return value
        if not isinstance(value, dict):
            raise kindle_snapshot_error("Unsupported metadata archive value")
        active.add(slot)
        try:
            name = kind(value)
            if name == "SyncMetadataAttributes" and set(value) == {"$class", "attributes"}:
                return {"attributes": resolve(value["attributes"], depth + 1)}
            if name in {"NSDictionary", "NSMutableDictionary"} and set(value) == {"$class", "NS.keys", "NS.objects"}:
                keys, values = value["NS.keys"], value["NS.objects"]
                if not isinstance(keys, list) or not isinstance(values, list) or len(keys) != len(values) or len(keys) > 1000:
                    raise kindle_snapshot_error("Invalid metadata dictionary")
                result = {}
                for key, child in zip(keys, values):
                    key = resolve(key, depth + 1)
                    if not isinstance(key, str) or key in result:
                        raise kindle_snapshot_error("Invalid or repeated metadata key")
                    result[key] = resolve(child, depth + 1)
                return result
            if name in {"NSArray", "NSMutableArray"} and set(value) == {"$class", "NS.objects"}:
                if not isinstance(value["NS.objects"], list) or len(value["NS.objects"]) > 1000:
                    raise kindle_snapshot_error("Invalid metadata array")
                return [resolve(child, depth + 1) for child in value["NS.objects"]]
            if name in {"NSString", "NSMutableString", "NSData", "NSMutableData"}:
                key, expected = ("NS.string", str) if "String" in name else ("NS.data", bytes)
                if set(value) == {"$class", key} and isinstance(value[key], expected):
                    return value[key]
            if name == "NSNumber":
                for key, expected in (("NS.boolval", bool), ("NS.intval", int), ("NS.real", float)):
                    if set(value) == {"$class", key} and type(value[key]) is expected:
                        return value[key]
            raise kindle_snapshot_error("Unsupported metadata object layout")
        finally:
            active.remove(slot)

    root = resolve(archive["$top"]["root"])
    if not isinstance(root, dict) or set(root) != {"attributes"} or not isinstance(root["attributes"], dict):
        raise kindle_snapshot_error("Expected Kindle SyncMetadataAttributes root")
    return root["attributes"]

def snapshot_text(value, max_bytes):
    if (not isinstance(value, str) or not value.strip() or len(value.encode("utf-8")) > max_bytes
            or any(unicodedata.category(c) == "Cc" for c in value)):
        raise kindle_snapshot_error("Unsupported title or author in Kindle metadata")
    return value.strip()

def snapshot_authors(value, depth=0):
    if depth > 8:
        raise kindle_snapshot_error("Excessively nested Kindle authors")
    if value is None:
        return []
    if isinstance(value, str):
        return [snapshot_text(value, 500)] if value.strip() else []
    if isinstance(value, dict) and set(value) == {"author"}:
        return snapshot_authors(value["author"], depth + 1)
    if isinstance(value, list) and len(value) <= 100:
        authors = [author for child in value for author in snapshot_authors(child, depth + 1)]
        if len(authors) <= 100:
            return authors
    raise kindle_snapshot_error("Unsupported Kindle author metadata")

def normalize_sqlite_catalog(raw):
    """Normalize a detached, rollback-mode SQLite snapshot entirely in memory."""
    if len(raw) > MAX_BYTES:
        raise ClosedError(413, "Catalog exceeds 32 MiB")
    if len(raw) < 100 or not raw.startswith(SQLITE_MAGIC):
        raise kindle_snapshot_error("Invalid Kindle SQLite snapshot")
    if raw[18:20] != b"\x01\x01":
        raise kindle_snapshot_error("A WAL-mode database is not a standalone snapshot. Create a consistent SQLite backup, then set journal_mode=DELETE on the backup only.", "kindle_snapshot_wal")
    page_size = int.from_bytes(raw[16:18], "big")
    page_size = 65536 if page_size == 1 else page_size
    if page_size < 512 or page_size > 65536 or page_size & (page_size - 1) or len(raw) % page_size:
        raise kindle_snapshot_error("Truncated or unsupported SQLite snapshot")
    if not hasattr(sqlite3.Connection, "deserialize"):
        raise ClosedError(503, "SQLite metadata import requires Python 3.11 or newer with SQLite deserialize support", "kindle_snapshot_runtime")
    db = sqlite3.connect(":memory:")
    try:
        db.deserialize(raw)
        db.execute("PRAGMA query_only=ON")
        db.execute("PRAGMA trusted_schema=OFF")
        if hasattr(db, "setlimit"):
            db.setlimit(sqlite3.SQLITE_LIMIT_LENGTH, MAX_METADATA_BYTES + 65536)
            db.setlimit(sqlite3.SQLITE_LIMIT_SQL_LENGTH, 65536)
            db.setlimit(sqlite3.SQLITE_LIMIT_ATTACHED, 0)
        deadline = time.monotonic() + 5
        steps = [0]
        def limit_work():
            steps[0] += 1000
            return int(steps[0] > 2000000 or time.monotonic() > deadline)
        db.set_progress_handler(limit_work, 1000)
        schema = db.execute("SELECT type, name, sql FROM sqlite_schema LIMIT 257").fetchall()
        books = [row for row in schema if row[1] == "ZBOOK"]
        if (len(schema) > 256 or len(books) != 1 or books[0][0] != "table"
                or not isinstance(books[0][2], str) or not books[0][2].upper().startswith("CREATE TABLE ")
                or any(row[2] is not None and len(row[2]) > 65536 for row in schema)):
            raise kindle_snapshot_error("Unsupported Kindle SQLite table schema", "kindle_snapshot_schema")
        columns = db.execute("PRAGMA table_xinfo('ZBOOK')").fetchall()
        fields = {column[1]: column for column in columns}
        expected = {"Z_PK": {"INTEGER"}, "ZDISPLAYTITLE": {"VARCHAR", "TEXT"}, "ZSYNCMETADATAATTRIBUTES": {"BLOB"}}
        if (len(columns) > 256 or any(name not in fields or fields[name][2].upper() not in types or fields[name][6] != 0 for name, types in expected.items())
                or fields["Z_PK"][5] != 1):
            raise kindle_snapshot_error("Unsupported Kindle SQLite metadata columns", "kindle_snapshot_schema")
        # Fixed SQL only. Views, triggers, extension loading and external databases
        # are not used; no source path is ever opened by this in-memory connection.
        def authorize(action, arg1, arg2, _database, _trigger):
            return sqlite3.SQLITE_OK if action == sqlite3.SQLITE_SELECT or (action == sqlite3.SQLITE_READ and arg1 == "ZBOOK" and arg2 in expected) else sqlite3.SQLITE_DENY
        db.set_authorizer(authorize)
        rows = db.execute("SELECT Z_PK, ZDISPLAYTITLE, ZSYNCMETADATAATTRIBUTES FROM ZBOOK ORDER BY Z_PK LIMIT 10001")
        items, seen, total = [], set(), 0
        for _pk, title, blob in rows:
            if time.monotonic() > deadline:
                raise kindle_snapshot_error("Kindle snapshot normalization exceeded its time bound")
            if len(items) >= MAX_CATALOG_ITEMS:
                raise kindle_snapshot_error("Kindle snapshot exceeds 10,000 records")
            attrs = inert_metadata(blob)
            if time.monotonic() > deadline:
                raise kindle_snapshot_error("Kindle snapshot normalization exceeded its time bound")
            asin = attrs.get("ASIN")
            if not valid_asin(asin) or asin in seen:
                raise kindle_snapshot_error("Missing, invalid or repeated explicit ASIN; no identifier was inferred")
            seen.add(asin)
            item = {"asin": asin, "title": snapshot_text(title, 2000), "authors": snapshot_authors(attrs.get("authors")), "position": None, "observed_at": None}
            total += len(json.dumps(item, ensure_ascii=True).encode()) + 2
            if total > MAX_BYTES - 1024:
                raise ClosedError(413, "Normalized catalog exceeds 32 MiB")
            items.append(item)
        return {"schema": "booknook.kindle-catalog.v1", "items": items}
    except sqlite3.Error as error:
        raise kindle_snapshot_error("Unreadable, oversized or unsupported Kindle SQLite snapshot", "kindle_snapshot_schema") from error
    finally:
        db.close()

def normalize_catalog(raw):
    if len(raw) > MAX_BYTES:
        raise ClosedError(413, "Catalog exceeds 32 MiB")
    xml_text = None
    try:
        if raw.startswith((b"\xff\xfe\x00\x00", b"\x00\x00\xfe\xff")):
            xml_text = raw.decode("utf-32")
        elif raw.startswith((b"\xff\xfe", b"\xfe\xff")):
            xml_text = raw.decode("utf-16")
        elif raw.startswith(b"\xef\xbb\xbf"):
            xml_text = raw.decode("utf-8-sig")
    except UnicodeError as error:
        raise ClosedError(400, "Invalid catalog text encoding") from error
    if raw.startswith(SQLITE_MAGIC):
        value = normalize_sqlite_catalog(raw)
    elif raw.lstrip().startswith(b"<") or (xml_text is not None and xml_text.lstrip().startswith("<")):
        inspected = xml_text.upper().encode("utf-8") if xml_text is not None else raw.upper()
        if b"<!DOCTYPE" in inspected or b"<!ENTITY" in inspected:
            raise ClosedError(400, "External entities and DTDs are not accepted")
        try:
            root = ET.fromstring(xml_text if xml_text is not None else raw)
        except ET.ParseError as error:
            raise ClosedError(400, "Invalid Kindle metadata XML") from error
        def tag(node): return node.tag.rsplit("}", 1)[-1].lower()
        items = []
        for record in root.iter():
            if tag(record) not in ("meta_data", "metadata"):
                continue
            fields = {tag(c): (c.text or "").strip() for c in record}
            authors = [(n.text or "").strip() for n in record.iter() if tag(n) == "author"]
            if "asin" in fields and "title" in fields:
                items.append({"asin": fields["asin"], "title": fields["title"], "authors": authors, "position": None, "observed_at": None})
        if not items:
            raise ClosedError(422, "No Kindle metadata records found; no library was inferred")
        value = {"schema": "booknook.kindle-catalog.v1", "items": items}
    else:
        value = strict_json(raw)
    if not isinstance(value, dict) or set(value) != {"schema", "items"} or value["schema"] != "booknook.kindle-catalog.v1" or not isinstance(value["items"], list):
        raise ClosedError(422, "Unsupported catalog schema")
    seen = set()
    for item in value["items"]:
        if not isinstance(item, dict) or set(item) - {"asin", "title", "authors", "position", "observed_at"} or not valid_asin(item.get("asin")) or not isinstance(item.get("title"), str) or not item["title"].strip() or not isinstance(item.get("authors"), list) or not all(isinstance(a, str) for a in item["authors"]):
            raise ClosedError(422, "Invalid Kindle metadata record")
        if item["asin"] in seen:
            raise ClosedError(422, "Repeated ASIN in source catalog; resolve source ambiguity first")
        seen.add(item["asin"])
        if (item.get("position") is None) != (item.get("observed_at") is None):
            raise ClosedError(422, "Position requires a source observation timestamp")
        if item.get("observed_at") is not None and (type(item["observed_at"]) is not int or item["observed_at"] < 0 or not isinstance(item["position"], str)):
            raise ClosedError(422, "Invalid position evidence")
    return value

class Companion:
    def __init__(self, root: Path, provider=None, catalog=None):
        self.snapshot_catalog = None
        # A saved successful upload is the durable user selection, including
        # after restarting with the same operator arguments or a live adapter.
        self.selected_upload = (root / "kindle-catalog.json").exists()
        if catalog and not self.selected_upload:
            with catalog.open("rb") as source:
                raw = source.read(MAX_BYTES + 1)
            if raw.startswith(SQLITE_MAGIC):
                # Never retain the raw database or read any adjacent WAL/SHM file.
                # Refresh a selected database by restarting with a new snapshot.
                self.snapshot_catalog = normalize_catalog(raw)
        root.mkdir(parents=True, exist_ok=True, mode=0o700)
        self.root, self.provider, self.catalog = root, provider, catalog
        self.db = sqlite3.connect(root / "pairings.sqlite", check_same_thread=False)
        self.db.execute("CREATE TABLE IF NOT EXISTS devices (digest TEXT PRIMARY KEY, name TEXT, created INTEGER)")
        self.db.execute("CREATE TABLE IF NOT EXISTS connection_observations (device TEXT, marketplace TEXT, descriptors TEXT, PRIMARY KEY(device, marketplace))")
        self.db.commit()
        os.chmod(root / "pairings.sqlite", 0o600)
        self.lock = threading.Lock()
        self.code, self.code_expiry = secrets.token_hex(16), int(time.time()) + 300
        self.failures = 0

    def adapter(self, operation, parameters=None):
        if not self.provider:
            raise ClosedError(503, "No live source adapter is configured", "source_not_configured")
        payload = json.dumps({"schema": "booknook.source-request.v1", "operation": operation, "parameters": parameters or {}}, separators=(",", ":")).encode()
        # argv is operator-configured; request data is only JSON on stdin, never shell code.
        child = subprocess.Popen(self.provider, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, shell=False)
        selector = selectors.DefaultSelector()
        selector.register(child.stdout, selectors.EVENT_READ, "stdout")
        selector.register(child.stderr, selectors.EVENT_READ, "stderr")
        timeout = CAPABILITIES_TIMEOUT if operation == "capabilities" else 25
        output, total, deadline = bytearray(), 0, time.monotonic() + timeout
        try:
            child.stdin.write(payload)
            child.stdin.close()
            while selector.get_map():
                if time.monotonic() > deadline:
                    raise ClosedError(504, "Source adapter timed out")
                for key, _ in selector.select(timeout=0.1):
                    chunk = os.read(key.fd, 65536)
                    if not chunk:
                        selector.unregister(key.fileobj)
                        continue
                    total += len(chunk)
                    if total > MAX_BYTES:
                        raise ClosedError(413, "Source adapter exceeded output limit")
                    if key.data == "stdout":
                        output.extend(chunk)
            if child.wait(timeout=1) != 0:
                raise ClosedError(502, "Source adapter failed; reconnect the source account")
            result = strict_json(output)
            if isinstance(result, dict) and result.get("schema") == "booknook.source-error.v1":
                messages = {"invalid_request": "Invalid source account request", "capability_unavailable": "This source capability is unavailable", "secure_storage_unavailable": "The source adapter needs a native OS credential store", "account_exists": "Renew or disconnect this device's existing source account first", "account_missing": "Connect this device's source account first", "session_expired": "Login session expired or was already used; start again", "registration_uncertain": "Registration outcome is uncertain; check Amazon's device list before restarting", "source_failure": "Source operation failed", "renewal_failed": "Account renewal failed; existing credentials were preserved", "invalid_source_data": "Source response does not match the inspected contract", "reset_denied": "Only an unfinished registration without credentials can be reset after Amazon-device review"}
                code = result.get("code")
                raise ClosedError(502, messages.get(code, "Source operation failed"), code if code in messages else "source_failure")
            return result
        finally:
            try:
                child.stdin.close()
            except OSError:
                pass
            selector.close()
            if child.poll() is None:
                child.kill()
            child.wait()
            child.stdout.close()
            child.stderr.close()

    def provider_capabilities(self):
        try:
            result = self.adapter("capabilities")
        except (ClosedError, OSError, subprocess.SubprocessError) as error:
            raise ClosedError(503, "Optional source adapter is unavailable; check its configured executable and dependencies, then refresh capabilities", "source_unavailable") from error
        if (not isinstance(result, dict) or result.get("schema") != "booknook.source-capabilities.v1"
                or not isinstance(result.get("capabilities"), list) or len(result["capabilities"]) > 100
                or not all(isinstance(cap, str) for cap in result["capabilities"])):
            raise ClosedError(502, "Optional source adapter returned an invalid capability contract; check its protocol version", "invalid_source_data")
        known = {"kindle.catalog", "audible.positions", "kindle.authorize", "audible.authorize", "account.renew", "account.authorize.complete", "account.authorize.cancel", "account.disconnect", "account.registration.reset", "audible.positions.probe", "provider.connections.list"}
        return [cap for cap in result["capabilities"] if cap in known], result.get("qualified") is True

    def manifest(self):
        capabilities = ["kindle.catalog.import", "provider_connections_v1"]
        qualified = False
        name = "Book Nook Library Companion"
        if self.catalog or (self.root / "kindle-catalog.json").exists():
            capabilities.append("kindle.catalog")
        if self.provider:
            try:
                provided, qualified = self.provider_capabilities()
                capabilities += provided
            except ClosedError:
                # The closed v1 manifest has no issue field. This fixed display
                # text surfaces the outage without adding fields or source data.
                name += " (optional source adapter unavailable; check its configuration)"
        return {"schema": "booknook.companion.v1", "name": name, "capabilities": sorted(set(capabilities)), "qualified": qualified}

    def provider_connections(self, device, marketplace):
        connections = []
        if self.catalog or (self.root / "kindle-catalog.json").exists():
            handle = hashlib.sha256(("castalia.provider-connection.v1:kindle:" + str(self.root.resolve()) + ":" + device).encode()).hexdigest()
            connections.append({"schema": "castalia.provider-connection.v1", "connection_id": "kindle." + handle,
                "provider_id": "com.amazon.kindle", "display_name": "Selected Kindle catalog",
                "capabilities": ["kindle.catalog", "kindle.catalog.import"], "status": "available", "credential": None})
        provided = []
        if self.provider:
            try:
                capabilities, _qualified = self.provider_capabilities()
                if "provider.connections.list" in capabilities:
                    provided = validate_provider_connections(self.adapter("provider.connections.list", {"device_id": device, "marketplace": marketplace}))
                else:
                    # Old adapters provide data without a credential registry.
                    # Name the selected adapter connection, never guess its secret
                    # backend or claim a transferable credential handle.
                    source = json.dumps(self.provider, ensure_ascii=True, separators=(",", ":"))
                    for provider, capability, label in [("com.amazon.kindle", "kindle.catalog", "Kindle source adapter"), ("com.audible", "audible.positions", "Audible source adapter")]:
                        if capability not in capabilities or (provider == "com.amazon.kindle" and connections):
                            continue
                        handle = hashlib.sha256(("castalia.provider-connection.v1:legacy:" + str(self.root.resolve()) + ":" + device + ":" + marketplace + ":" + provider + ":" + source).encode()).hexdigest()
                        provided.append({"schema": "castalia.provider-connection.v1", "connection_id": "source." + handle,
                            "provider_id": provider, "display_name": label, "capabilities": [capability], "status": "available", "credential": None})
                with self.lock:
                    self.db.execute("INSERT OR REPLACE INTO connection_observations VALUES(?,?,?)", [device, marketplace, json.dumps(provided)])
                    self.db.commit()
            except (ClosedError, OSError, subprocess.SubprocessError):
                # Keep local catalogs usable during an optional adapter outage.
                # Cached account references are observations, never live grants.
                with self.lock:
                    row = self.db.execute("SELECT descriptors FROM connection_observations WHERE device=? AND marketplace=?", [device, marketplace]).fetchone()
                provided = validate_provider_connections(strict_json(row[0])) if row else []
                for connection in provided:
                    connection["status"] = "unavailable"
        if connections:
            # Catalog reads prefer the explicitly selected local source, including
            # after an upload. Do not expose a second, shadowed Kindle source.
            provided = [connection for connection in provided if connection["provider_id"] != "com.amazon.kindle"]
        return validate_provider_connections(connections + provided)

    def require_capability(self, capability):
        if not self.provider:
            raise ClosedError(503, "No live source adapter is configured", "source_not_configured")
        provided, _qualified = self.provider_capabilities()
        if capability not in provided:
            raise ClosedError(503, "The source adapter does not provide this capability", "capability_unavailable")

    def authenticate(self, header):
        if not header or not header.startswith("Bearer "):
            raise ClosedError(401, "Pair this device first")
        token = header[7:]
        if len(token) != 64 or any(c not in "0123456789abcdef" for c in token):
            raise ClosedError(401, "Invalid device token")
        digest = hashlib.sha256(token.encode()).hexdigest()
        with self.lock:
            found = self.db.execute("SELECT 1 FROM devices WHERE digest=?", [digest]).fetchone()
        if not found:
            raise ClosedError(401, "Device access revoked")
        return digest

    def handle(self, method, path, headers, body, *, raw_sqlite_allowed=False):
        if path == "/v1/pair" and method == "POST":
            value = strict_json(body)
            if set(value) != {"schema", "code", "device_name"} or value["schema"] != "booknook.companion-pair.v1" or not isinstance(value["code"], str) or not isinstance(value["device_name"], str) or not 1 <= len(value["device_name"]) <= 160:
                raise ClosedError(400, "Invalid pairing request")
            manifest = self.manifest()
            with self.lock:
                if self.failures >= 10 or time.time() >= self.code_expiry or not hmac.compare_digest(value["code"], self.code):
                    self.failures += 1
                    raise ClosedError(403, "Pairing code invalid, used, expired, or locked; restart companion for a fresh code")
                token = secrets.token_hex(32)
                self.code_expiry = 0
                self.db.execute("INSERT INTO devices VALUES(?,?,?)", [hashlib.sha256(token.encode()).hexdigest(), value["device_name"], int(time.time())])
                self.db.commit()
            return {"schema": "booknook.companion-paired.v1", "token": token, "manifest": manifest}
        device = self.authenticate(headers.get("Authorization"))
        if path == "/v1/unpair" and method == "POST":
            with self.lock:
                self.db.execute("DELETE FROM connection_observations WHERE device=?", [device])
                self.db.execute("DELETE FROM devices WHERE digest=?", [device]); self.db.commit()
            return {"disconnected": True}
        if path == "/v1/capabilities" and method == "GET":
            return self.manifest()
        if (path == "/v1/provider-connections" or path.startswith("/v1/provider-connections/")) and method == "GET":
            marketplace = "us" if path == "/v1/provider-connections" else path.removeprefix("/v1/provider-connections/")
            if marketplace not in ("us", "uk", "de", "ca", "au"):
                raise ClosedError(400, "Invalid marketplace")
            return self.provider_connections(device, marketplace)
        if path == "/v1/kindle/catalog" and method == "PUT":
            if body.startswith(SQLITE_MAGIC) and not raw_sqlite_allowed:
                raise ClosedError(403, "Raw Kindle databases are accepted only by a loopback-bound companion from a loopback client. Export normalized JSON locally for a remote companion.", "kindle_snapshot_local_only")
            catalog = normalize_catalog(body)
            target = self.root / "kindle-catalog.json"
            staging = self.root / ("catalog-" + secrets.token_hex(8) + ".tmp")
            descriptor = os.open(staging, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(descriptor, "w") as output:
                json.dump(catalog, output); output.flush(); os.fsync(output.fileno())
            staging.replace(target)
            self.selected_upload = True
            return {"imported": len(catalog["items"])}
        if path == "/v1/kindle/catalog" and method == "GET":
            if self.selected_upload:
                with (self.root / "kindle-catalog.json").open("rb") as file:
                    return normalize_catalog(file.read(MAX_BYTES + 1))
            if self.snapshot_catalog is not None:
                return self.snapshot_catalog
            source = self.catalog or self.root / "kindle-catalog.json"
            if not self.catalog and not source.exists() and self.provider:
                provided, _qualified = self.provider_capabilities()
                if "kindle.catalog" in provided:
                    return normalize_catalog(json.dumps(self.adapter("kindle.catalog")).encode())
            if not source.exists():
                raise ClosedError(503, "Select a Kindle catalog export in Book Nook or configure a live adapter")
            with source.open("rb") as file:
                return normalize_catalog(file.read(MAX_BYTES + 1))
        if path == "/v1/accounts/authorize" and method == "POST":
            value = strict_json(body)
            if set(value) != {"provider", "marketplace"} or value["provider"] not in ("kindle", "audible") or value["marketplace"] not in ("us", "uk", "de", "ca", "au"):
                raise ClosedError(400, "Invalid account authorization request")
            self.require_capability(value["provider"] + ".authorize")
            if "account.authorize.complete" in self.manifest()["capabilities"]:
                value["device_id"] = device
            return self.adapter("account.authorize", value)
        if path in ("/v1/accounts/authorize/complete", "/v1/accounts/authorize/cancel", "/v1/accounts/disconnect", "/v1/accounts/registration/reset", "/v1/audible/positions/probe") and method == "POST":
            value = strict_json(body)
            operation, fields, schema, flag = {
                "/v1/accounts/authorize/complete": ("account.authorize.complete", {"session_id", "return_url", "consent_virtual_device"}, "booknook.account-connected.v1", "connected"),
                "/v1/accounts/authorize/cancel": ("account.authorize.cancel", {"session_id"}, "booknook.account-cancelled.v1", "cancelled"),
                "/v1/accounts/disconnect": ("account.disconnect", set(), "booknook.account-disconnected.v1", "disconnected"),
                "/v1/accounts/registration/reset": ("account.registration.reset", {"confirmed_amazon_device_review"}, "booknook.account-registration-reset.v1", "reset"),
                "/v1/audible/positions/probe": ("audible.positions.probe", {"asins"}, "booknook.audible-probe.v1", "received_json"),
            }[path]
            if not isinstance(value, dict) or set(value) != fields | {"provider", "marketplace"} or value["provider"] not in ("kindle", "audible") or value["marketplace"] not in ("us", "uk", "de", "ca", "au"):
                raise ClosedError(400, "Invalid source account request")
            if len(body) > 65536:
                raise ClosedError(413, "Account request exceeds 64 KiB")
            self.require_capability(operation)
            # Pair identity is server-derived, never a caller-supplied authority claim.
            value["device_id"] = device
            result = self.adapter(operation, value)
            if not isinstance(result, dict) or result.get("schema") != schema or result.get(flag) is not True:
                raise ClosedError(502, "Source did not confirm the account operation")
            return result
        if path == "/v1/accounts/renew" and method == "POST":
            value = strict_json(body)
            if set(value) != {"provider", "marketplace"} or value["provider"] not in ("kindle", "audible") or value["marketplace"] not in ("us", "uk", "de", "ca", "au"):
                raise ClosedError(400, "Invalid account renewal request")
            self.require_capability("account.renew")
            if "account.authorize.complete" in self.manifest()["capabilities"]:
                value["device_id"] = device
            result = self.adapter("account.renew", value)
            if not isinstance(result, dict) or set(result) != {"schema", "provider", "renewed"} or result["schema"] != "booknook.account-renewal.v1" or result["provider"] != value["provider"] or result["renewed"] is not True:
                raise ClosedError(502, "Source adapter did not confirm account renewal", "renewal_failed")
            return result
        if path == "/v1/audible/positions" and method == "POST":
            value = strict_json(body)
            if set(value) not in ({"asins"}, {"asins", "marketplace"}) or value.get("marketplace", "us") not in ("us", "uk", "de", "ca", "au") or not isinstance(value["asins"], list) or not 1 <= len(value["asins"]) <= 100 or not all(valid_asin(a) for a in value["asins"]):
                raise ClosedError(400, "Expected 1–100 ASINs")
            self.require_capability("audible.positions")
            if "account.authorize.complete" in self.manifest()["capabilities"]:
                value.update(device_id=device, provider="audible", marketplace=value.get("marketplace", "us"))
            else:
                value.pop("marketplace", None)  # Preserve legacy adapter contract.
            result = self.adapter("audible.positions", value)
            if result.get("schema") != "booknook.audible-positions.v1":
                raise ClosedError(502, "Source adapter has not normalized its position evidence")
            return result
        raise ClosedError(404, "Unknown companion operation")

class Handler(http.server.BaseHTTPRequestHandler):
    server_version = "BookNookCompanion/0.1"
    def log_message(self, *_):
        pass  # Account payloads, pairing codes, device tokens, and paths are never logged.
    def run_request(self):
        try:
            if self.headers.get("Origin") or self.headers.get("Transfer-Encoding"):
                raise ClosedError(403, "Browser cross-origin and chunked requests are not accepted")
            length = int(self.headers.get("Content-Length", "0"))
            if length < 0 or length > MAX_BYTES:
                raise ClosedError(413, "Request exceeds 32 MiB")
            if self.path != urllib.parse.urlsplit(self.path).path:
                raise ClosedError(400, "Queries and fragments are unsupported")
            self.connection.settimeout(30)
            body = self.rfile.read(length)
            if len(body) != length:
                raise ClosedError(400, "Truncated request")
            local_source = ipaddress.ip_address(self.server.server_address[0]).is_loopback and ipaddress.ip_address(self.client_address[0]).is_loopback
            result = self.server.companion.handle(self.command, self.path, self.headers, body, raw_sqlite_allowed=local_source)
            status = 200
        except ClosedError as error:
            status, result = error.status, {"error": error.message, "code": error.code}
        except (ValueError, OSError, TypeError, KeyError):
            status, result = 400, {"error": "Invalid request or unavailable source", "code": "invalid_request"}
        except Exception:
            status, result = 500, {"error": "Companion operation failed", "code": "internal_error"}
        encoded = json.dumps(result, separators=(",", ":"), allow_nan=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(encoded)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.end_headers()
        self.wfile.write(encoded)
    do_GET = run_request
    do_POST = run_request
    do_PUT = run_request

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--data", type=Path)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--cert", type=Path)
    parser.add_argument("--key", type=Path)
    source = parser.add_mutually_exclusive_group()
    source.add_argument("--kindle-catalog", type=Path, help="Explicit local JSON/XML or standalone Kindle SQLite metadata snapshot")
    source.add_argument("--export-kindle-catalog", type=Path, help="Normalize only the selected source to JSON; do not start a server")
    parser.add_argument("--output", type=Path, help="New private JSON output path; existing files are never overwritten")
    parser.add_argument("--provider-command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.export_kindle_catalog:
        if not args.output or args.data or args.provider_command or args.cert or args.key:
            parser.error("Export requires --output and cannot be combined with server data, adapters or TLS options")
        try:
            with args.export_kindle_catalog.open("rb") as source_file:
                catalog = normalize_catalog(source_file.read(MAX_BYTES + 1))
            encoded = json.dumps(catalog, separators=(",", ":"), ensure_ascii=True, allow_nan=False).encode() + b"\n"
            if len(encoded) > MAX_BYTES:
                raise ClosedError(413, "Normalized catalog exceeds 32 MiB")
            descriptor = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            try:
                with os.fdopen(descriptor, "wb") as output:
                    output.write(encoded); output.flush(); os.fsync(output.fileno())
            except BaseException:
                args.output.unlink(missing_ok=True)
                raise
        except ClosedError as error:
            parser.exit(2, f"Catalog export failed ({error.code}): {error.message}\n")
        except (OSError, ValueError, TypeError, RecursionError):
            parser.exit(2, "Catalog export failed: source unavailable or output already exists; no existing output was overwritten.\n")
        print(f"Exported {len(catalog['items'])} catalog records. Cached metadata does not establish ownership, reading progress or content access.")
        return
    if args.output or not args.data:
        parser.error("Server requires --data; --output is only for --export-kindle-catalog")
    if bool(args.cert) != bool(args.key):
        parser.error("Supply both --cert and --key together")
    if not ipaddress.ip_address(args.host).is_loopback and not (args.cert and args.key):
        parser.error("LAN service requires --cert and --key; unencrypted service is loopback-only")
    try:
        companion = Companion(args.data, args.provider_command, args.kindle_catalog)
    except ClosedError as error:
        parser.exit(2, f"Catalog source rejected ({error.code}): {error.message}\n")
    except OSError:
        parser.exit(2, "Catalog source is unavailable. Select an accessible local export.\n")
    server = http.server.ThreadingHTTPServer((args.host, args.port), Handler)
    server.companion = companion
    if args.cert and args.key:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER); context.minimum_version = ssl.TLSVersion.TLSv1_2
        context.load_cert_chain(args.cert, args.key); server.socket = context.wrap_socket(server.socket, server_side=True)
    print(f"Book Nook companion: {'https' if args.cert else 'http'}://{args.host}:{args.port}")
    print(f"Pairing code (one use, expires in five minutes): {companion.code}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close(); companion.db.close()

if __name__ == "__main__":
    main()
