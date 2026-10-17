# SPDX-License-Identifier: AGPL-3.0-only
"""Real CoreFoundation encoding, intercepted OS operations; never real Keychain data."""
import ctypes
import importlib.util
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).parents[2]
spec = importlib.util.spec_from_file_location("atomic_adapter", ROOT / "services/optional-audible-adapter/booknook_audible.py")
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)


@unittest.skipUnless(sys.platform == "darwin" and importlib.util.find_spec("keyring"), "Pinned macOS keyring environment required")
class AtomicVaultTests(unittest.TestCase):
    def setUp(self):
        import keyring
        from keyring.backends.macOS import Keyring, api
        self.api = api
        self.backend = Keyring()
        self.calls = []
        self.value = {"credential": "synthetic original café 雨"}
        self.update_status = 0
        self.add_status = 0
        lookup = api._found.CFDictionaryGetValue
        lookup.restype = ctypes.c_void_p
        lookup.argtypes = (ctypes.c_void_p, ctypes.c_void_p)
        self.lookup = lookup
        def update(query, attributes):
            self.assert_scope(query)
            self.calls.append("update")
            if self.update_status:
                return self.update_status
            self.value = self.decode(attributes)
            return 0
        def add(query, _result):
            self.assert_scope(query)
            self.calls.append("add")
            if not self.add_status:
                self.value = self.decode(query)
            return self.add_status
        self.update = self.enterContext(patch.object(api._sec, "SecItemUpdate", side_effect=update))
        self.add = self.enterContext(patch.object(api, "SecItemAdd", side_effect=add))
        self.delete = self.enterContext(patch.object(api, "SecItemDelete", side_effect=AssertionError("Replacement must never delete")))
        self.enterContext(patch.object(keyring, "get_keyring", return_value=self.backend))
        self.enterContext(patch.object(self.backend, "get_password", side_effect=lambda _s, _k: json.dumps(self.value) if self.value else None))
        self.vault = m.SystemVault("memory-only-regression")
    def assert_scope(self, query):
        equal = self.api._found.CFEqual
        equal.restype = ctypes.c_bool
        equal.argtypes = (ctypes.c_void_p, ctypes.c_void_p)
        release = self.api._found.CFRelease
        release.argtypes = (ctypes.c_void_p,)
        for field, expected in (("kSecAttrService", "org.booknook.optional-audible.memory-only-regression"), ("kSecAttrAccount", "synthetic")):
            value = self.api.create_cf(expected)
            try: self.assertTrue(equal(self.lookup(query, self.api.k_(field)), value))
            finally: release(value)
    def decode(self, query):
        pointer = self.lookup(query, self.api.k_("kSecValueData"))
        return json.loads(self.api.cfstr_to_str(pointer))
    def test_denied_update_keeps_credentials_and_never_adds_or_deletes(self):
        before = self.vault.read("synthetic")
        self.update_status = -25293
        with self.assertRaises(m.AdapterError) as failure:
            self.vault.write("synthetic", {"credential": "replacement"})
        self.assertEqual(failure.exception.code, "secure_storage_unavailable")
        self.assertEqual(self.vault.read("synthetic"), before)
        self.assertEqual(self.calls, ["update"])
        self.delete.assert_not_called()
    def test_success_updates_unicode_exactly_without_delete_or_add(self):
        replacement = {"credential": {"token": "synthetic 雨 café 🌿\u0000"}}
        self.vault.write("synthetic", replacement)
        self.assertEqual(self.vault.read("synthetic"), replacement)
        self.assertEqual(self.calls, ["update"])
        self.delete.assert_not_called()
    def test_initial_add_failure_leaves_absent_item_and_never_deletes(self):
        self.value = {}
        self.update_status = self.api.error.item_not_found
        self.add_status = -25308
        with self.assertRaises(m.AdapterError): self.vault.write("synthetic", {"session": "candidate"})
        self.assertEqual(self.vault.read("synthetic"), {})
        self.assertEqual(self.calls, ["update", "add"])
        self.delete.assert_not_called()
    def test_missing_item_is_added_and_readable_in_existing_json_format(self):
        self.value = {}
        self.update_status = self.api.error.item_not_found
        replacement = {"session": {"secret": "synthetic PKCE 雨"}}
        self.vault.write("synthetic", replacement)
        self.assertEqual(self.vault.read("synthetic"), replacement)
        self.assertEqual(self.calls, ["update", "add"])
    def test_failed_registration_persistence_keeps_uncertainty_marker(self):
        self.value = {"registration_in_progress": True, "serial": "synthetic-serial"}
        before = self.vault.read("synthetic")
        self.update_status = -25308
        with self.assertRaises(m.AdapterError): self.vault.write("synthetic", {"credential": "new account"})
        self.assertEqual(self.vault.read("synthetic"), before)
        self.assertEqual(self.calls, ["update"])
    def test_duplicate_add_race_retries_update_without_deletion(self):
        original_update = self.update.side_effect
        def raced_update(query, attributes):
            self.update_status = self.api.error.item_not_found if not self.calls else 0
            return original_update(query, attributes)
        self.update.side_effect = raced_update
        self.add_status = -25299
        replacement = {"session": "candidate"}
        self.vault.write("synthetic", replacement)
        self.assertEqual(self.vault.read("synthetic"), replacement)
        self.assertEqual(self.calls, ["update", "add", "update"])
        self.assertEqual(self.update.call_count, 2)
        self.assertEqual(self.add.call_count, 1)
        self.delete.assert_not_called()
    def test_duplicate_race_denied_retry_retains_other_writers_item(self):
        before = self.vault.read("synthetic")
        original_update = self.update.side_effect
        def raced_update(query, attributes):
            self.update_status = self.api.error.item_not_found if not self.calls else -25293
            return original_update(query, attributes)
        self.update.side_effect = raced_update
        self.add_status = -25299
        with self.assertRaises(m.AdapterError): self.vault.write("synthetic", {"session": "candidate"})
        self.assertEqual(self.vault.read("synthetic"), before)
        self.assertEqual(self.calls, ["update", "add", "update"])
        self.assertEqual(self.update.call_count, 2)
        self.delete.assert_not_called()


if __name__ == "__main__": unittest.main()
