#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only
"""Explicitly optional Audible source process. Never imported by a Book Nook SDK.

Only upstream audible 0.10.0 APIs are used. No passwords, OTPs, content licensing,
DRM, guessed position fields, or plaintext credential files are implemented.
"""
from __future__ import annotations
import argparse
import contextlib
import ctypes
from datetime import datetime, timezone
import fcntl
import hashlib
import importlib.metadata
import json
import logging
import os
from pathlib import Path
import re
import secrets
import sys
import time
import urllib.parse

MAX_INPUT = 65536
DOMAINS = {"us": "com", "uk": "co.uk", "de": "de", "ca": "ca", "au": "com.au"}
CAPABILITIES = ["audible.authorize", "account.authorize.complete", "account.authorize.cancel", "account.renew", "account.disconnect", "account.registration.reset", "audible.positions.probe", "audible.positions", "provider.connections.list"]
ERRORS = {
    "invalid_request": "Invalid source request.",
    "capability_unavailable": "This source capability is not implemented or qualified.",
    "secure_storage_unavailable": "An OS credential store is required; no plaintext fallback is allowed.",
    "account_exists": "This device already has account credentials; renew or disconnect them first.",
    "account_missing": "Connect this device's Audible account first.",
    "session_expired": "The login session expired or was already used. Start a new login.",
    "registration_uncertain": "Registration did not finish safely. Check Amazon's device list before starting again; a virtual device may need removal.",
    "source_failure": "The source operation failed. No source details or credentials were returned.",
    "renewal_failed": "Account renewal failed; stored credentials were preserved.",
    "invalid_source_data": "Audible position data does not match the inspected source contract.",
    "reset_denied": "Only an unfinished registration without saved credentials can be reset after explicit Amazon-device review.",
}


def normalize_positions(payload, requested, fetched_at):
    """Original strict parser for the published audible-api-ts position contract.

    Evidence: a88fc1cf589184a3ca44c4e095c838745627c4a8 src/library.ts,
    src/schemas.ts and src/library.unit.test.ts. Upstream explicitly dates this
    zone-less last_updated format in UTC. Missing times are never fetch times.
    """
    if not isinstance(payload, dict) or not isinstance(payload.get("asin_last_position_heard_annots"), list):
        raise AdapterError("invalid_source_data")
    seen, observations = set(), []
    for item in payload["asin_last_position_heard_annots"]:
        if not isinstance(item, dict) or not isinstance(item.get("asin"), str) or item["asin"] not in requested or item["asin"] in seen or not isinstance(item.get("last_position_heard"), dict):
            raise AdapterError("invalid_source_data")
        asin = item["asin"]; seen.add(asin); position = item["last_position_heard"]
        if position.get("status") == "DoesNotExist":
            continue
        if position.get("status") != "Exists":
            raise AdapterError("invalid_source_data")
        millis, source_time = position.get("position_ms"), position.get("last_updated")
        if millis is None or source_time is None:
            continue
        if type(millis) is not int or not 0 <= millis <= 9_007_199_254_740_991 or not isinstance(source_time, str) or not re.fullmatch(r"\d{4}-\d\d-\d\d \d\d:\d\d:\d\d(?:\.\d{1,6})?", source_time):
            raise AdapterError("invalid_source_data")
        try:
            observed = int(datetime.fromisoformat(source_time).replace(tzinfo=timezone.utc).timestamp())
        except ValueError:
            raise AdapterError("invalid_source_data") from None
        if observed <= 0 or observed > fetched_at + 300:
            raise AdapterError("invalid_source_data")
        observations.append({"asin": asin, "position_milliseconds": millis, "source_updated_at": observed, "fetched_at": fetched_at, "duration_milliseconds": None, "edition_identifier": None, "alignment": "unmatched", "metadata": None})
    return observations


def normalize_metadata(payload, requested):
    # /catalog/products and the fields below are documented in the inspected
    # upstream API/schema. This never treats an audiobook identifier as an ebook.
    if not isinstance(payload, dict) or not isinstance(payload.get("products"), list):
        raise AdapterError("invalid_source_data")
    result = {}
    for item in payload["products"]:
        if not isinstance(item, dict) or not isinstance(item.get("asin"), str) or item["asin"] not in requested or item["asin"] in result:
            raise AdapterError("invalid_source_data")
        title, authors, language = item.get("title"), item.get("authors", []), item.get("language")
        def text(value, limit):
            return isinstance(value, str) and bool(value.strip()) and len(value.encode("utf-8")) <= limit and not any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value)
        if not text(title, 2000) or not isinstance(authors, list) or len(authors) > 100 or not all(isinstance(a, dict) and text(a.get("name"), 500) for a in authors) or (language is not None and not text(language, 100)):
            raise AdapterError("invalid_source_data")
        result[item["asin"]] = {"title": title, "authors": [a["name"] for a in authors], "ebook_identifiers": [], "language": language}
    return result

class AdapterError(Exception):
    def __init__(self, code):
        self.code = code
        super().__init__(ERRORS[code])


def strict_json(raw):
    def unique(pairs):
        out = {}
        for k, v in pairs:
            if k in out:
                raise AdapterError("invalid_request")
            out[k] = v
        return out
    try:
        return json.loads(raw, object_pairs_hook=unique, parse_constant=lambda _: (_ for _ in ()).throw(ValueError()))
    except (ValueError, UnicodeError):
        raise AdapterError("invalid_request") from None


def macos_atomic_write(service, account, value):
    """Update the exact existing item; never delete credentials before replacement.

    keyring 25.6.0's macOS set_password deletes then adds. Use Apple's supported
    SecItemUpdate/SecItemAdd operations with that pinned backend's CF bridge so
    failed renewal or registration persistence leaves the previous item intact.
    """
    from keyring.backends.macOS import api
    update = api._sec.SecItemUpdate
    update.restype = ctypes.c_int32
    update.argtypes = (ctypes.c_void_p, ctypes.c_void_p)
    data_create = api._found.CFDataCreate
    data_create.restype = ctypes.c_void_p
    data_create.argtypes = (ctypes.c_void_p, ctypes.c_void_p, ctypes.c_long)
    release = api._found.CFRelease
    release.restype = None
    release.argtypes = (ctypes.c_void_p,)
    owned = []
    def retain_created(pointer):
        if not pointer:
            raise AdapterError("secure_storage_unavailable")
        owned.append(pointer)
        return pointer
    try:
        raw = value.encode("utf-8")
        data = retain_created(data_create(None, ctypes.c_char_p(raw), len(raw)))
        fields = {"kSecClass": api.k_("kSecClassGenericPassword"),
                  "kSecAttrService": service, "kSecAttrAccount": account}
        query = retain_created(api.create_query(**fields))
        attributes = retain_created(api.create_query(kSecValueData=ctypes.c_void_p(data)))
        status = update(query, attributes)
        if status == api.error.item_not_found:
            addition = retain_created(api.create_query(**fields, kSecValueData=ctypes.c_void_p(data)))
            status = api.SecItemAdd(addition, None)
            if status == -25299:  # errSecDuplicateItem: another writer created it.
                status = update(query, attributes)
        api.Error.raise_for_status(status)
    finally:
        for pointer in reversed(owned):
            release(pointer)


class SystemVault:
    """Only native credential backends; keyrings.alt/plaintext/chaining are rejected."""
    def __init__(self, namespace):
        import keyring
        self.backend = keyring.get_keyring()
        backend_name = type(self.backend).__module__ + "." + type(self.backend).__name__
        if backend_name not in {"keyring.backends.macOS.Keyring", "keyring.backends.SecretService.Keyring"}:
            raise AdapterError("secure_storage_unavailable")
        self.service = "org.booknook.optional-audible." + namespace
        self.is_macos = backend_name == "keyring.backends.macOS.Keyring"

    def read(self, key):
        try:
            value = self.backend.get_password(self.service, key)
            return strict_json(value) if value else {}
        except Exception:
            raise AdapterError("secure_storage_unavailable") from None

    def write(self, key, value):
        try:
            encoded = json.dumps(value, separators=(",", ":"))
            if self.is_macos:
                macos_atomic_write(self.service, key, encoded)
            else:
                self.backend.set_password(self.service, key, encoded)
        except Exception:
            raise AdapterError("secure_storage_unavailable") from None


class AudibleBackend:
    def __init__(self):
        if importlib.metadata.version("audible") != "0.10.0":
            raise AdapterError("capability_unavailable")
        import audible
        self.audible = audible

    def begin(self, marketplace):
        from audible.login import create_code_verifier, build_oauth_url
        from audible.localization import Locale
        locale = Locale(marketplace)
        verifier = create_code_verifier()
        url, serial = build_oauth_url(locale.country_code, locale.domain, locale.market_place_id, verifier)
        return url, {"verifier": verifier.decode("ascii"), "serial": serial}

    def register(self, marketplace, secret, code):
        from audible.register import register
        data = register(code, secret["verifier"].encode("ascii"), DOMAINS[marketplace], secret["serial"], with_username=False)
        return self.audible.Authenticator.from_dict(data, locale=marketplace).to_dict()

    def renew(self, credential):
        auth = self.audible.Authenticator.from_dict(dict(credential))
        auth.refresh_access_token(force=True)
        return auth.to_dict()

    def disconnect(self, credential):
        auth = self.audible.Authenticator.from_dict(dict(credential))
        auth.refresh_access_token()  # Current device only. Never deregister_all.
        auth.deregister_device(deregister_all=False)

    def positions(self, credential, asins, clock=time.time, probe=False):
        auth = self.audible.Authenticator.from_dict(dict(credential))
        observations = []
        with self.audible.Client(auth=auth, timeout=10) as client:
            url = "https://api.audible." + DOMAINS[credential["locale_code"]] + "/1.0/annotations/lastpositions"
            for start in range(0, len(asins), 25):
                batch = asins[start:start + 25]
                with client.raw_request("GET", url, params={"asins": ",".join(batch)}, stream=True, apply_auth_flow=True, follow_redirects=False) as response:
                    if response.status_code != 200:
                        raise AdapterError("source_failure")
                    raw = bytearray()
                    for chunk in response.iter_bytes():
                        raw.extend(chunk)
                        if len(raw) > 4 * 1024 * 1024:
                            raise AdapterError("source_failure")
                    payload = strict_json(raw)
                    if not probe:
                        batch_observations = normalize_positions(payload, batch, int(clock()))
                        # Metadata is optional. A separate short, bounded catalog
                        # read enriches edition review without blocking positions.
                        try:
                            metadata_url = "https://api.audible." + DOMAINS[credential["locale_code"]] + "/1.0/catalog/products"
                            with client.raw_request("GET", metadata_url, params={"asins": ",".join(batch), "response_groups": "contributors,product_attrs"}, stream=True, apply_auth_flow=True, follow_redirects=False, timeout=2) as metadata_response:
                                if metadata_response.status_code != 200:
                                    raise AdapterError("source_failure")
                                metadata_raw = bytearray()
                                for chunk in metadata_response.iter_bytes():
                                    metadata_raw.extend(chunk)
                                    if len(metadata_raw) > 1024 * 1024:
                                        raise AdapterError("source_failure")
                                metadata = normalize_metadata(strict_json(metadata_raw), batch)
                            for observation in batch_observations:
                                observation["metadata"] = metadata.get(observation["asin"])
                        except Exception:
                            pass  # No inferred metadata, time, duration or edition.
                        observations.extend(batch_observations)
        return auth.to_dict(), observations

    def probe(self, credential, asins):
        return self.positions(credential, asins, probe=True)[0]


class Adapter:
    def __init__(self, root, backend, vault, clock=time.time):
        self.root = Path(root)
        self.root.mkdir(mode=0o700, parents=True, exist_ok=True)
        if self.root.is_symlink() or self.root.stat().st_uid != os.getuid() or self.root.stat().st_mode & 0o077:
            raise AdapterError("secure_storage_unavailable")
        self.backend, self.vault, self.clock = backend, vault, clock

    @contextlib.contextmanager
    def locked(self):
        # All one-process-per-request mutations serialize, including network calls.
        fd = os.open(self.root / "operation.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX)
            yield
        finally:
            os.close(fd)

    def handle(self, request):
        if not isinstance(request, dict) or set(request) != {"schema", "operation", "parameters"} or request["schema"] != "booknook.source-request.v1" or not isinstance(request["parameters"], dict):
            raise AdapterError("invalid_request")
        operation, p = request["operation"], request["parameters"]
        if operation == "capabilities":
            return {"schema": "booknook.source-capabilities.v1", "capabilities": CAPABILITIES, "qualified": False}
        if operation == "provider.connections.list":
            if set(p) != {"device_id", "marketplace"} or not isinstance(p["device_id"], str) or not re.fullmatch(r"[0-9a-f]{64}", p["device_id"]) or p["marketplace"] not in DOMAINS:
                raise AdapterError("invalid_request")
            connections = []
            # This only inspects the already selected secure store. No network,
            # sign-in, registration, refresh or credential export occurs here.
            with self.locked():
                for marketplace in [p["marketplace"]]:
                    key = p["device_id"] + ":" + marketplace
                    state = self.vault.read(key)
                    if not state.get("credential"):
                        continue
                    # Existing stores retain a stable legacy reference. Every new
                    # registration gets a fresh generation, even in the same slot.
                    generation = state.get("connection_generation", "legacy")
                    if not isinstance(generation, str) or (generation != "legacy" and not re.fullmatch(r"[0-9a-f]{32}", generation)):
                        raise AdapterError("invalid_source_data")
                    handle = hashlib.sha256(("castalia.provider-connection.v1:audible:" + str(self.root.resolve()) + ":" + key + ":" + generation).encode()).hexdigest()
                    connections.append({
                        "schema": "castalia.provider-connection.v1",
                        "connection_id": "audible." + handle,
                        "provider_id": "com.audible",
                        "display_name": "Audible (" + marketplace.upper() + ")",
                        "capabilities": ["audible.positions", "account.renew", "account.disconnect"],
                        "status": "available",
                        "credential": {"reference": "audible." + handle, "storage": "secure_local", "portability": "reauthorization_required"},
                    })
            return connections
        required = {
            "account.authorize": {"provider", "marketplace", "device_id"},
            "account.authorize.complete": {"provider", "marketplace", "device_id", "session_id", "return_url", "consent_virtual_device"},
            "account.authorize.cancel": {"provider", "marketplace", "device_id", "session_id"},
            "account.renew": {"provider", "marketplace", "device_id"},
            "account.disconnect": {"provider", "marketplace", "device_id"},
            "account.registration.reset": {"provider", "marketplace", "device_id", "confirmed_amazon_device_review"},
            "audible.positions.probe": {"provider", "marketplace", "device_id", "asins"},
            "audible.positions": {"provider", "marketplace", "device_id", "asins"},
        }
        if operation not in required:
            raise AdapterError("capability_unavailable")
        if set(p) != required[operation] or p.get("provider") != "audible" or p.get("marketplace") not in DOMAINS or not re.fullmatch(r"[0-9a-f]{64}", p.get("device_id", "")):
            raise AdapterError("invalid_request")
        key = p["device_id"] + ":" + p["marketplace"]
        with self.locked():
            state = self.vault.read(key)
            now = int(self.clock())
            if operation == "account.registration.reset":
                if p["confirmed_amazon_device_review"] is not True or state.get("credential") or state.get("registration_in_progress") is not True:
                    raise AdapterError("reset_denied")
                self.vault.write(key, {})
                return {"schema": "booknook.account-registration-reset.v1", "provider": "audible", "reset": True}
            if operation == "account.authorize":
                if state.get("credential"):
                    raise AdapterError("account_exists")
                if state.get("registration_in_progress"):
                    raise AdapterError("registration_uncertain")
                url, secret = self.backend.begin(p["marketplace"])
                session = {"id": secrets.token_hex(32), "expires_at": now + 600, "secret": secret}
                self.vault.write(key, {"session": session})
                return {"authorization_url": url, "session_id": session["id"], "expires_at": session["expires_at"], "requires_virtual_device_registration": True}
            if operation.startswith("account.authorize."):
                session = state.get("session", {})
                if not isinstance(p["session_id"], str) or not re.fullmatch(r"[0-9a-f]{64}", p["session_id"]):
                    raise AdapterError("session_expired")
                if operation.endswith("cancel"):
                    if session and not secrets.compare_digest(session.get("id", ""), p["session_id"]):
                        raise AdapterError("session_expired")
                    # Idempotent after timeout/completion; never erase credentials or
                    # uncertain-registration evidence while cancelling a login.
                    state.pop("session", None)
                    self.vault.write(key, state)
                    return {"schema": "booknook.account-cancelled.v1", "cancelled": True}
                if not secrets.compare_digest(session.get("id", ""), p["session_id"]) or session.get("expires_at", 0) <= now:
                    raise AdapterError("session_expired")
                if p["consent_virtual_device"] is not True:
                    raise AdapterError("invalid_request")
                code = self.return_code(p["return_url"], p["marketplace"])
                # Persist consumed BEFORE registration. Crashes never silently repeat it.
                self.vault.write(key, {"registration_in_progress": True, "serial": session["secret"]["serial"]})
                try:
                    credential = self.backend.register(p["marketplace"], session["secret"], code)
                    self.vault.write(key, {"credential": credential, "connection_generation": secrets.token_hex(16)})
                except Exception:
                    raise AdapterError("registration_uncertain") from None
                return {"schema": "booknook.account-connected.v1", "provider": "audible", "connected": True, "positions_qualified": False}
            credential = state.get("credential")
            if not credential:
                raise AdapterError("account_missing")
            if operation == "account.disconnect":
                self.backend.disconnect(credential)
                self.vault.write(key, {})
                return {"schema": "booknook.account-disconnected.v1", "provider": "audible", "disconnected": True}
            if operation == "account.renew":
                try:
                    updated = self.backend.renew(credential)
                    self.vault.write(key, dict(state, credential=updated))
                except Exception:
                    raise AdapterError("renewal_failed") from None
                return {"schema": "booknook.account-renewal.v1", "provider": "audible", "renewed": True}
            if not isinstance(p["asins"], list) or not 1 <= len(p["asins"]) <= 100 or not all(isinstance(a, str) and re.fullmatch(r"[A-Za-z0-9]{10}", a) for a in p["asins"]):
                raise AdapterError("invalid_request")
            if operation == "audible.positions":
                updated, observations = self.backend.positions(credential, list(dict.fromkeys(p["asins"])), self.clock)
                self.vault.write(key, dict(state, credential=updated))
                return {"schema": "booknook.audible-positions.v1", "observations": observations}
            updated = self.backend.probe(credential, p["asins"])
            self.vault.write(key, dict(state, credential=updated))
            return {"schema": "booknook.audible-probe.v1", "received_json": True, "positions_qualified": False}

    @staticmethod
    def return_code(raw, marketplace):
        if not isinstance(raw, str) or len(raw) > 32768 or any(ord(c) < 32 for c in raw):
            raise AdapterError("invalid_request")
        try:
            url = urllib.parse.urlsplit(raw)
            if url.scheme != "https" or url.netloc != "www.amazon." + DOMAINS[marketplace] or url.path != "/ap/maplanding" or url.fragment:
                raise ValueError()
            pairs = urllib.parse.parse_qsl(url.query, keep_blank_values=True, max_num_fields=100)
            query = dict(pairs)
            if len(query) != len(pairs):
                raise ValueError()
            code = query["openid.oa2.authorization_code"]
            if not 1 <= len(code) <= 8192:
                raise ValueError()
            return code
        except (ValueError, KeyError):
            raise AdapterError("invalid_request") from None


def main():
    logging.disable(logging.CRITICAL)
    parser = argparse.ArgumentParser(description="Optional AGPL Audible adapter; credentials require native OS keyring")
    parser.add_argument("--data", required=True, type=Path)
    args = parser.parse_args()
    try:
        raw = sys.stdin.buffer.read(MAX_INPUT + 1)
        if len(raw) > MAX_INPUT:
            raise AdapterError("invalid_request")
        namespace = hashlib.sha256(str(args.data.resolve()).encode()).hexdigest()[:32]
        adapter = Adapter(args.data, AudibleBackend(), SystemVault(namespace))
        result = adapter.handle(strict_json(raw))
    except AdapterError as error:
        result = {"schema": "booknook.source-error.v1", "code": error.code}
    except Exception:
        result = {"schema": "booknook.source-error.v1", "code": "source_failure"}
    print(json.dumps(result, separators=(",", ":")))

if __name__ == "__main__":
    main()
