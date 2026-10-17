# SPDX-License-Identifier: AGPL-3.0-only
"""Synthetic only: no real accounts, credentials, OS keyring writes, or HTTP."""
import concurrent.futures
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).parents[2]
spec = importlib.util.spec_from_file_location("adapter", ROOT / "services/optional-audible-adapter/booknook_audible.py")
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

class MemoryVault:
    def __init__(self): self.data = {}
    def read(self, key): return copy.deepcopy(self.data.get(key, {}))
    def write(self, key, value): self.data[key] = copy.deepcopy(value)

class SyntheticBackend:
    def __init__(self): self.registrations = 0; self.disconnects = 0; self.renewals = 0
    def begin(self, marketplace): return "https://www.amazon.com/ap/signin?synthetic=1", {"verifier": "secret-verifier", "serial": "synthetic-device"}
    def register(self, marketplace, secret, code):
        assert secret["verifier"] == "secret-verifier" and code == "secret-code"
        self.registrations += 1
        return {"access_token": "synthetic-access", "refresh_token": "synthetic-refresh", "locale_code": marketplace}
    def renew(self, credential): self.renewals += 1; return dict(credential, access_token="synthetic-renewed")
    def disconnect(self, credential): self.disconnects += 1
    def probe(self, credential, asins): return credential

class AdapterTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.vault = MemoryVault(); self.backend = SyntheticBackend(); self.now = 1000
        self.adapter = m.Adapter(Path(self.tmp.name), self.backend, self.vault, lambda: self.now)
    def call(self, op, **params):
        p = dict(provider="audible", marketplace="us", device_id="a" * 64); p.update(params)
        return self.adapter.handle(dict(schema="booknook.source-request.v1", operation=op, parameters=p))
    def begin(self): return self.call("account.authorize")
    def connections(self, device="a" * 64, marketplace="us"):
        return self.adapter.handle(dict(schema="booknook.source-request.v1", operation="provider.connections.list", parameters=dict(device_id=device, marketplace=marketplace)))

    def test_public_connections_are_device_and_marketplace_bound_without_secret_export(self):
        self.assertEqual(self.connections(), [])
        self.complete(self.begin())
        result = self.connections()
        self.assertEqual(len(result), 1)
        self.assertEqual(result[0]["provider_id"], "com.audible")
        self.assertEqual(result[0]["credential"]["portability"], "reauthorization_required")
        self.assertEqual(self.connections("b" * 64), [])
        self.assertEqual(self.connections(marketplace="ca"), [])
        serialized = json.dumps(result)
        for secret in ["synthetic-access", "synthetic-refresh", "secret-code", "a" * 64]:
            self.assertNotIn(secret, serialized)
        self.adapter = m.Adapter(Path(self.tmp.name), self.backend, self.vault, lambda: self.now)
        self.assertEqual(self.connections(), result)
        self.assertEqual(self.backend.renewals, 0)
        self.call("account.renew")
        self.assertEqual(self.connections(), result)
        self.call("account.disconnect")
        self.assertEqual(self.connections(), [])
        self.complete(self.begin())
        self.assertNotEqual(self.connections()[0]["connection_id"], result[0]["connection_id"])
    def complete(self, session, **params):
        p = dict(session_id=session["session_id"], return_url="https://www.amazon.com/ap/maplanding?openid.oa2.authorization_code=secret-code", consent_virtual_device=True); p.update(params)
        return self.call("account.authorize.complete", **p)
    def test_one_use_restart_registration_refresh_probe_and_per_device_disconnect(self):
        session = self.begin()
        # New process object, same durable backend store.
        self.adapter = m.Adapter(Path(self.tmp.name), self.backend, self.vault, lambda: self.now)
        result = self.complete(session)
        self.assertTrue(result["connected"]); self.assertFalse(result["positions_qualified"])
        with self.assertRaises(m.AdapterError): self.complete(session)
        with self.assertRaises(m.AdapterError): self.begin()
        self.assertEqual(self.backend.registrations, 1)
        self.assertTrue(self.call("account.renew")["renewed"])
        self.assertFalse(self.call("audible.positions.probe", asins=["B012345678"])["positions_qualified"])
        with self.assertRaises(m.AdapterError): self.call("account.disconnect", device_id="b" * 64)
        self.assertTrue(self.call("account.disconnect")["disconnected"])
        self.assertEqual(self.backend.disconnects, 1)
        self.assertEqual(self.vault.read("a"*64+":us"), {})
    def test_expiry_device_marketplace_session_mismatch_and_consent_reject_before_registration(self):
        session = self.begin()
        for overrides in [dict(device_id="b"*64), dict(marketplace="ca"), dict(session_id="f"*64), dict(consent_virtual_device=False)]:
            with self.assertRaises(m.AdapterError): self.complete(session, **overrides)
        self.now += 600
        with self.assertRaises(m.AdapterError) as expired: self.complete(session)
        self.assertEqual(expired.exception.code, "session_expired")
        self.assertTrue(self.call("account.authorize.cancel", session_id=session["session_id"])["cancelled"])
        self.assertEqual(self.backend.registrations, 0)
    def test_cancel_consumes_only_pending_session_and_is_idempotent_after_completion(self):
        session = self.begin()
        self.call("account.authorize.cancel", session_id=session["session_id"])
        with self.assertRaises(m.AdapterError): self.complete(session)
        session = self.begin(); self.complete(session)
        self.call("account.authorize.cancel", session_id=session["session_id"])
        self.assertIn("credential", self.vault.read("a"*64+":us"))
    def test_concurrent_completion_registers_once(self):
        session = self.begin()
        def finish():
            try: return self.complete(session)["connected"]
            except m.AdapterError: return False
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            results = list(pool.map(lambda _: finish(), range(2)))
        self.assertEqual(sorted(results), [False, True]); self.assertEqual(self.backend.registrations, 1)
    def test_failed_registration_is_consumed_and_does_not_leak_upstream_exception(self):
        session = self.begin()
        with patch.object(self.backend, "register", side_effect=RuntimeError("raw-secret-code-and-token")):
            with self.assertRaises(m.AdapterError) as error: self.complete(session)
        self.assertEqual(error.exception.code, "registration_uncertain")
        self.assertNotIn("raw-secret", str(error.exception))
        with self.assertRaises(m.AdapterError): self.complete(session)
        with self.assertRaises(m.AdapterError): self.begin()
        self.call("account.authorize.cancel", session_id=session["session_id"])
        self.assertTrue(self.vault.read("a"*64+":us")["registration_in_progress"])
    def test_failed_refresh_retains_original_credentials(self):
        self.complete(self.begin()); before = copy.deepcopy(self.vault.data)
        with patch.object(self.backend, "renew", side_effect=RuntimeError("raw-refresh-token")):
            with self.assertRaises(m.AdapterError) as error: self.call("account.renew")
        self.assertEqual(error.exception.code, "renewal_failed"); self.assertEqual(before, self.vault.data)
    def test_explicit_uncertain_reset_is_isolated_and_never_erases_valid_credentials(self):
        params = dict(confirmed_amazon_device_review=True)
        with self.assertRaises(m.AdapterError): self.call("account.registration.reset", **params)
        session = self.begin()
        with self.assertRaises(m.AdapterError): self.call("account.registration.reset", **params)
        with patch.object(self.backend, "register", side_effect=RuntimeError("failure")):
            with self.assertRaises(m.AdapterError): self.complete(session)
        before = copy.deepcopy(self.vault.data)
        for overrides in [dict(confirmed_amazon_device_review=False), dict(confirmed_amazon_device_review="true"), dict(device_id="b"*64), dict(marketplace="ca")]:
            with self.assertRaises(m.AdapterError): self.call("account.registration.reset", **(params | overrides))
            self.assertEqual(before, self.vault.data)
        self.assertTrue(self.call("account.registration.reset", **params)["reset"])
        self.assertEqual(self.backend.disconnects, 0)
        self.complete(self.begin()); before = copy.deepcopy(self.vault.data)
        with self.assertRaises(m.AdapterError): self.call("account.registration.reset", **params)
        self.assertEqual(before, self.vault.data)
    def test_callback_origin_path_duplicate_code_and_scheme_validation(self):
        session = self.begin()
        for url in ["http://www.amazon.com/ap/maplanding?openid.oa2.authorization_code=x", "https://www.amazon.com.evil.invalid/ap/maplanding?openid.oa2.authorization_code=x", "https://www.amazon.ca/ap/maplanding?openid.oa2.authorization_code=x", "https://www.amazon.com/ap/signin?openid.oa2.authorization_code=x", "https://www.amazon.com/ap/maplanding?openid.oa2.authorization_code=x&openid.oa2.authorization_code=y", "https://user@www.amazon.com/ap/maplanding?openid.oa2.authorization_code=x", "https://www.amazon.com/ap/maplanding?openid.oa2.authorization_code=x#secret"]:
            with self.assertRaises(m.AdapterError): self.complete(session, return_url=url)
        self.assertEqual(self.backend.registrations, 0)
        self.assertTrue(self.complete(session)["connected"])
    def test_capabilities_fail_closed_for_positions_kindle_and_plaintext_storage(self):
        caps = self.adapter.handle(dict(schema="booknook.source-request.v1", operation="capabilities", parameters={}))
        self.assertFalse(caps["qualified"])
        self.assertNotIn("kindle.catalog", caps["capabilities"]); self.assertIn("audible.positions", caps["capabilities"])
        with self.assertRaises(m.AdapterError): self.call("audible.positions", asins=["B012345678"])
        with self.assertRaises(m.AdapterError): self.call("account.authorize", provider="kindle")
        try: import keyring
        except ImportError: return
        with patch.object(keyring, "get_keyring", return_value=object()):
            with self.assertRaises(m.AdapterError) as error: m.SystemVault("synthetic")
        self.assertEqual(error.exception.code, "secure_storage_unavailable")
    def test_duplicate_json_and_world_readable_directory_rejected(self):
        with self.assertRaises(m.AdapterError): m.strict_json('{"x":1,"x":2}')
        Path(self.tmp.name).chmod(0o755)
        with self.assertRaises(m.AdapterError): m.Adapter(Path(self.tmp.name), self.backend, self.vault)

@unittest.skipUnless(importlib.util.find_spec("audible"), "Optional pinned upstream environment not installed")
class PinnedUpstreamTests(unittest.TestCase):
    def test_real_pkce_builder_and_real_registration_refresh_deregister_serialization(self):
        import base64, hashlib, httpx, urllib.parse
        backend = m.AudibleBackend()
        url, secret = backend.begin("ca")
        query = urllib.parse.parse_qs(urllib.parse.urlsplit(url).query)
        self.assertEqual(urllib.parse.urlsplit(url).hostname, "www.amazon.ca")
        self.assertEqual(query["openid.oa2.code_challenge_method"], ["S256"])
        challenge = base64.urlsafe_b64encode(hashlib.sha256(secret["verifier"].encode()).digest()).rstrip(b"=").decode()
        self.assertEqual(query["openid.oa2.code_challenge"], [challenge])
        posts = []
        def post(url, **kwargs):
            posts.append((url, kwargs))
            if url.endswith("/register"):
                payload = {"response": {"success": {"tokens": {"mac_dms": {"adp_token": "{enc:x}{key:x}{iv:x}{name:x}{serial:Mg==}", "device_private_key": "-----BEGIN RSA PRIVATE KEY-----\nsynthetic\n-----END RSA PRIVATE KEY-----\n"}, "store_authentication_cookie": {}, "bearer": {"access_token": "Atna|synthetic", "refresh_token": "Atnr|synthetic", "expires_in": "3600"}, "website_cookies": [{"Name": "synthetic", "Value": "synthetic-cookie"}]}, "extensions": {"device_info": {}, "customer_info": {}}}}}
            elif url.endswith("/token"): payload = {"access_token": "Atna|renewed", "expires_in": "3600"}
            else: payload = {"success": True}
            return httpx.Response(200, json=payload, request=httpx.Request("POST", url))
        with patch("httpx.post", side_effect=post):
            credential = backend.register("ca", secret, "synthetic-code")
            renewed = backend.renew(credential)
            self.assertEqual(renewed["access_token"], "Atna|renewed")
            backend.disconnect(renewed)
        self.assertEqual(posts[0][1]["json"]["auth_data"]["code_verifier"], secret["verifier"])
        self.assertEqual(posts[0][1]["json"]["registration_data"]["device_serial"], secret["serial"])
        self.assertEqual(posts[1][1]["data"]["source_token"], "Atnr|synthetic")
        self.assertEqual(posts[-1][1]["json"], {"deregister_all_existing_accounts": False})

if __name__ == "__main__": unittest.main()

class PositionNormalizerTests(unittest.TestCase):
    def sample(self, **changes):
        position = dict(status="Exists", position_ms=8275705, last_updated="2026-09-16 20:55:15.357"); position.update(changes)
        return {"asin_last_position_heard_annots": [{"asin": "B012345678", "last_position_heard": position}]}
    def test_published_shape_preserves_utc_source_time_and_omits_absent_evidence(self):
        from datetime import datetime, timezone
        fetched = int(datetime(2026, 10, 10, tzinfo=timezone.utc).timestamp())
        result = m.normalize_positions(self.sample(), ["B012345678"], fetched)
        self.assertEqual(result[0]["source_updated_at"], int(datetime(2026, 9, 16, 20, 55, 15, tzinfo=timezone.utc).timestamp()))
        self.assertEqual(result[0]["fetched_at"], fetched); self.assertNotEqual(result[0]["source_updated_at"], fetched)
        self.assertEqual(result[0]["position_milliseconds"], 8275705)
        self.assertEqual(result[0]["alignment"], "unmatched"); self.assertIsNone(result[0]["metadata"])
        for changes in [dict(status="DoesNotExist"), dict(last_updated=None), dict(position_ms=None)]:
            self.assertEqual(m.normalize_positions(self.sample(**changes), ["B012345678"], fetched), [])
    def test_malformed_duplicate_unrequested_and_future_evidence_rejected(self):
        for changes in [dict(position_ms=-1), dict(position_ms=True), dict(position_ms="12"), dict(status="Maybe"), dict(last_updated="2099-01-01 01:00:00"), dict(last_updated="2026-09-16T20:55:15Z")]:
            with self.assertRaises(m.AdapterError): m.normalize_positions(self.sample(**changes), ["B012345678"], 1800000000)
        with self.assertRaises(m.AdapterError): m.normalize_positions(self.sample(), ["B098765432"], 1800000000)
        data = self.sample(); data["asin_last_position_heard_annots"] *= 2
        with self.assertRaises(m.AdapterError): m.normalize_positions(data, ["B012345678"], 1800000000)
        with self.assertRaises(m.AdapterError): m.normalize_positions({}, ["B012345678"], 1800000000)
    def test_optional_metadata_preserves_identifiers_and_never_fabricates_ebook_match(self):
        data = {"products": [{"asin": "B012345678", "title": "Title", "authors": [{"name": "Writer"}], "language": "English", "isbn": "not-an-ebook-isbn"}]}
        metadata = m.normalize_metadata(data, ["B012345678"])["B012345678"]
        self.assertEqual(metadata, dict(title="Title", authors=["Writer"], language="English", ebook_identifiers=[]))
        data["products"] *= 2
        with self.assertRaises(m.AdapterError): m.normalize_metadata(data, ["B012345678"])
    def test_optional_metadata_uses_rust_utf8_limits_and_rejects_control_characters(self):
        for fields in [dict(title="書"*667), dict(authors=[{"name":"書"*167}]), dict(title="Title\nsecond line"), dict(language="en\u0085"), dict(authors=[{"name":"Reader\tWriter"}])]:
            data = {"products": [dict(asin="B012345678", title="Title", authors=[{"name":"Writer"}], language="en") | fields]}
            with self.assertRaises(m.AdapterError): m.normalize_metadata(data, ["B012345678"])

@unittest.skipUnless(importlib.util.find_spec("audible"), "Optional pinned upstream environment not installed")
class PinnedPositionTransportTests(unittest.TestCase):
    def test_real_upstream_signed_client_batches_25_and_enriches_only_source_metadata(self):
        import httpx, rsa
        from datetime import datetime, timezone
        _, private = rsa.newkeys(1024)
        credential = dict(access_token="Atna|synthetic", refresh_token="Atnr|synthetic", adp_token="{enc:x}{key:x}{iv:x}{name:x}{serial:Mg==}", device_private_key=private.save_pkcs1().decode(), expires=9999999999, locale_code="us", with_username=False)
        requests = []
        def handler(request):
            requests.append(request)
            self.assertTrue(request.headers.get("x-adp-signature"))
            asins = request.url.params["asins"].split(",")
            self.assertLessEqual(len(asins), 25)
            if "lastpositions" in request.url.path:
                return httpx.Response(200, json={"asin_last_position_heard_annots": [{"asin": a, "last_position_heard": {"status": "Exists", "position_ms": 15000, "last_updated": "2026-09-16 20:55:15.357"}} for a in asins]})
            if asins[0] == "B000000025": return httpx.Response(503, json={"error": "metadata unavailable"})
            return httpx.Response(200, json={"products": [{"asin": a, "title": "Actual metadata " + a, "authors": [{"name": "Writer"}], "language": "en"} for a in asins]})
        backend = m.AudibleBackend(); original = backend.audible.Client
        def client(*args, **kwargs): return original(*args, transport=httpx.MockTransport(handler), **kwargs)
        fetched = int(datetime(2026, 10, 10, tzinfo=timezone.utc).timestamp())
        with patch.object(backend.audible, "Client", side_effect=client):
            saved, observations = backend.positions(credential, [f"B{i:09d}" for i in range(26)], lambda: fetched)
        self.assertEqual(len(observations), 26)
        self.assertEqual([len(r.url.params["asins"].split(",")) for r in requests], [25, 25, 1, 1])
        self.assertEqual(observations[0]["metadata"]["authors"], ["Writer"])
        self.assertEqual(observations[0]["metadata"]["ebook_identifiers"], [])
        self.assertIsNone(observations[-1]["metadata"])
        self.assertEqual(saved["refresh_token"], credential["refresh_token"])
