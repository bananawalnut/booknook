# SPDX-License-Identifier: MIT OR Apache-2.0
import importlib.util
import json
import http.client
import threading
import sys
import tempfile
import unittest
from unittest.mock import patch
from pathlib import Path
spec = importlib.util.spec_from_file_location("companion", Path(__file__).parents[2] / "services/library-companion/booknook_companion.py")
m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

class CompanionTests(unittest.TestCase):
    def test_shared_castalia_provider_fixture_matrix(self):
        cases = json.loads((Path(__file__).parents[2] / "crates/core/tests/fixtures/castalia/provider-connections-v1-cases.json").read_text())["cases"]
        for case in cases:
            with self.subTest(case=case["name"]):
                if case["valid"]:
                    self.assertEqual(m.validate_provider_connections([case["value"]]), [case["value"]])
                else:
                    with self.assertRaises(m.ClosedError): m.validate_provider_connections([case["value"]])

    def test_connection_references_survive_restart_and_do_not_grant_access(self):
        with tempfile.TemporaryDirectory() as root:
            service = m.Companion(Path(root))
            request = json.dumps({"schema": "booknook.companion-pair.v1", "code": service.code, "device_name": "test"}).encode()
            pair = service.handle("POST", "/v1/pair", {}, request)
            headers = {"Authorization": "Bearer " + pair["token"]}
            self.assertIn("provider_connections_v1", pair["manifest"]["capabilities"])
            self.assertEqual(service.handle("GET", "/v1/provider-connections/us", headers, b""), [])
            source = b'{"schema":"booknook.kindle-catalog.v1","items":[]}'
            service.handle("PUT", "/v1/kindle/catalog", headers, source)
            connections = service.handle("GET", "/v1/provider-connections/us", headers, b"")
            self.assertEqual(connections[0]["provider_id"], "com.amazon.kindle")
            self.assertIsNone(connections[0]["credential"])
            self.assertNotIn(pair["token"], json.dumps(connections))
            service.db.close()
            service = m.Companion(Path(root))
            self.assertEqual(service.handle("GET", "/v1/provider-connections/us", headers, b""), connections)
            with self.assertRaises(m.ClosedError):
                service.handle("GET", "/v1/provider-connections/us", {"Authorization": "Bearer " + connections[0]["connection_id"]}, b"")
            service.handle("POST", "/v1/unpair", headers, b"{}")
            with self.assertRaises(m.ClosedError): service.handle("GET", "/v1/provider-connections/us", headers, b"")
            service.db.close()

    def test_optional_outage_marks_cached_refs_unavailable_without_disabling_local_catalog(self):
        with tempfile.TemporaryDirectory() as root:
            service = m.Companion(Path(root), provider=["synthetic-source"])
            descriptor = {"schema": "castalia.provider-connection.v1", "connection_id": "audible.example", "provider_id": "com.audible", "display_name": "Audible", "capabilities": ["audible.positions"], "status": "available", "credential": {"reference": "audible.example", "storage": "secure_local", "portability": "reauthorization_required"}}
            kindle = dict(descriptor, connection_id="kindle.live", provider_id="com.amazon.kindle", display_name="Live Kindle source", capabilities=["kindle.catalog"], credential=None)
            def adapter(operation, parameters=None):
                if operation == "capabilities": return {"schema": "booknook.source-capabilities.v1", "capabilities": ["provider.connections.list"], "qualified": False}
                self.assertEqual(parameters, {"device_id": "a" * 64, "marketplace": "us"})
                return [descriptor, kindle]
            with patch.object(service, "adapter", side_effect=adapter):
                result = service.provider_connections("a" * 64, "us")
            self.assertEqual(result, [descriptor, kindle])
            (Path(root) / "kindle-catalog.json").write_text('{"schema":"booknook.kindle-catalog.v1","items":[]}')
            with patch.object(service, "adapter", side_effect=adapter):
                selected = service.provider_connections("a" * 64, "us")
                self.assertEqual([item["provider_id"] for item in selected], ["com.amazon.kindle", "com.audible"])
                self.assertEqual(selected[0]["display_name"], "Selected Kindle catalog")
            with patch.object(service, "adapter", side_effect=OSError("synthetic outage")):
                result = service.provider_connections("a" * 64, "us")
                self.assertEqual([item["status"] for item in result], ["available", "unavailable"])
                self.assertEqual(len(service.provider_connections("b" * 64, "us")), 1)
            with self.assertRaises(m.ClosedError):
                m.validate_provider_connections([dict(descriptor, access_token="secret")])
            with self.assertRaises(m.ClosedError): m.validate_provider_connections([descriptor, descriptor])
            service.db.close()

    def test_pairing_revocation_import_and_no_invented_account_access(self):
        with tempfile.TemporaryDirectory() as root:
            service = m.Companion(Path(root))
            req = json.dumps({"schema":"booknook.companion-pair.v1","code":service.code,"device_name":"test"}).encode()
            result = service.handle("POST", "/v1/pair", {}, req)
            headers = {"Authorization":"Bearer " + result["token"]}
            self.assertFalse(result["manifest"]["qualified"])
            with self.assertRaises(m.ClosedError): service.handle("POST","/v1/pair",{},req)
            xml = b'<response><add_update_list><meta_data><ASIN>B012345678</ASIN><title>A real export shape</title><authors><author>Example</author></authors></meta_data></add_update_list></response>'
            self.assertEqual(service.handle("PUT","/v1/kindle/catalog",headers,xml)["imported"],1)
            catalog = service.handle("GET","/v1/kindle/catalog",headers,b"")
            self.assertEqual(catalog["items"][0]["authors"],["Example"])
            self.assertIsNone(catalog["items"][0]["position"])
            with self.assertRaises(m.ClosedError) as unavailable: service.handle("POST","/v1/audible/positions",headers,b'{"asins":["B012345678"]}')
            self.assertEqual(unavailable.exception.status,503)
            service.handle("POST","/v1/unpair",headers,b"{}")
            with self.assertRaises(m.ClosedError): service.handle("GET","/v1/capabilities",headers,b"")
            service.db.close()

    def test_duplicate_keys_and_xml_entities_are_rejected(self):
        with self.assertRaises(m.ClosedError): m.strict_json(b'{"code":"a","code":"b"}')
        with self.assertRaises(m.ClosedError): m.normalize_catalog(b'<!DOCTYPE x [<!ENTITY a SYSTEM "file:///etc/passwd">]><x>&a;</x>')
        with self.assertRaises(m.ClosedError): m.normalize_catalog(b'<empty/>')

    def test_expired_code_never_registers_device(self):
        with tempfile.TemporaryDirectory() as root:
            service=m.Companion(Path(root));service.code_expiry=0
            req=json.dumps({"schema":"booknook.companion-pair.v1","code":service.code,"device_name":"test"}).encode()
            with self.assertRaises(m.ClosedError): service.handle("POST","/v1/pair",{},req)
            self.assertEqual(service.db.execute("SELECT COUNT(*) FROM devices").fetchone()[0],0)
            service.db.close()

class CompanionAdapterTests(unittest.TestCase):
    def test_legacy_source_remains_consumable_without_inventing_credential_storage(self):
        with tempfile.TemporaryDirectory() as root:
            script = Path(root) / "legacy_contract_fixture.py"
            script.write_text("import sys,json\nr=json.load(sys.stdin)\nif r['operation']=='capabilities': print(json.dumps({'schema':'booknook.source-capabilities.v1','capabilities':['kindle.catalog','audible.positions'],'qualified':False}))\nelse: print(json.dumps({'schema':'booknook.kindle-catalog.v1','items':[]}))\n")
            service = m.Companion(Path(root) / "data", provider=[sys.executable, str(script)])
            refs = service.provider_connections("a" * 64, "us")
            self.assertEqual([c["provider_id"] for c in refs], ["com.amazon.kindle", "com.audible"])
            self.assertTrue(all(c["credential"] is None for c in refs))
            self.assertNotEqual(refs, service.provider_connections("b" * 64, "us"))
            self.assertNotEqual(refs, service.provider_connections("a" * 64, "ca"))
            (service.root / "kindle-catalog.json").write_text('{"schema":"booknook.kindle-catalog.v1","items":[]}')
            selected = service.provider_connections("a" * 64, "us")
            self.assertEqual(len([c for c in selected if c["provider_id"] == "com.amazon.kindle"]), 1)
            self.assertEqual(selected[0]["display_name"], "Selected Kindle catalog")
            service.db.close()

    def test_optional_renewal_uses_real_process_contract_and_rejects_missing_capability(self):
        # Explicit contract fixture process, not an Amazon authentication implementation.
        with tempfile.TemporaryDirectory() as root:
            script = Path(root) / "source_contract_fixture.py"
            script.write_text("import sys,json\nr=json.load(sys.stdin)\nif r['operation']=='capabilities': print(json.dumps({'schema':'booknook.source-capabilities.v1','capabilities':['account.renew'],'qualified':False}))\nelse: print(json.dumps({'schema':'booknook.account-renewal.v1','provider':r['parameters']['provider'],'renewed':True}))\n")
            service = m.Companion(Path(root) / "data", provider=[sys.executable, str(script)])
            pair = service.handle("POST", "/v1/pair", {}, json.dumps({"schema":"booknook.companion-pair.v1","code":service.code,"device_name":"contract test"}).encode())
            headers = {"Authorization":"Bearer " + pair["token"]}
            result = service.handle("POST", "/v1/accounts/renew", headers, b'{"provider":"audible","marketplace":"us"}')
            self.assertTrue(result["renewed"])
            self.assertFalse(pair["manifest"]["qualified"])
            with self.assertRaises(m.ClosedError) as unsupported:
                service.handle("POST", "/v1/accounts/authorize", headers, b'{"provider":"audible","marketplace":"us"}')
            self.assertEqual(unsupported.exception.code, "capability_unavailable")
            script.write_text(script.read_text().replace("'renewed':True", "'renewed':False"))
            with self.assertRaises(m.ClosedError) as failed:
                service.handle("POST", "/v1/accounts/renew", headers, b'{"provider":"audible","marketplace":"us"}')
            self.assertEqual(failed.exception.code, "renewal_failed")
            service.db.close()

class CompanionHTTPTests(unittest.TestCase):
    def test_actual_http_pairing_authentication_csrf_and_catalog_upload(self):
        with tempfile.TemporaryDirectory() as root:
            service=m.Companion(Path(root))
            server=m.http.server.ThreadingHTTPServer(("127.0.0.1",0),m.Handler)
            server.companion=service
            worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
            def call(method,path,body=b"",headers=None):
                client=http.client.HTTPConnection("127.0.0.1",server.server_port,timeout=5)
                try:
                    client.request(method,path,body=body,headers=headers or {})
                    response=client.getresponse();return response.status,json.loads(response.read())
                finally: client.close()
            try:
                self.assertEqual(call("GET","/v1/capabilities")[0],401)
                self.assertEqual(call("GET","/v1/provider-connections/us")[0],401)
                pair=json.dumps({"schema":"booknook.companion-pair.v1","code":service.code,"device_name":"HTTP test"}).encode()
                status,paired=call("POST","/v1/pair",pair);self.assertEqual(status,200)
                headers={"Authorization":"Bearer "+paired["token"]}
                self.assertEqual(call("GET","/v1/capabilities",headers=headers)[0],200)
                self.assertEqual(call("GET","/v1/provider-connections/us",headers=headers),(200, []))
                self.assertEqual(call("GET","/v1/capabilities",headers=dict(headers,Origin="https://example.invalid"))[0],403)
                source=b'{"schema":"booknook.kindle-catalog.v1","items":[{"asin":"B012345678","title":"Selected source export","authors":["Writer"]}]}'
                self.assertEqual(call("PUT","/v1/kindle/catalog",source,headers)[0],200)
                status,catalog=call("GET","/v1/kindle/catalog",headers=headers);self.assertEqual(status,200);self.assertEqual(catalog["items"][0]["title"],"Selected source export")
                missing_status, missing = call("POST","/v1/accounts/renew",b'{"provider":"audible","marketplace":"us"}',headers)
                self.assertEqual(missing_status,503);self.assertEqual(missing["code"],"source_not_configured")
                self.assertEqual(call("POST","/v1/unpair",b"{}",headers)[0],200)
                self.assertEqual(call("GET","/v1/capabilities",headers=headers)[0],401)
            finally:
                server.shutdown();server.server_close();worker.join();service.db.close()

if __name__ == '__main__': unittest.main()

class CompanionAccountHTTPTests(unittest.TestCase):
    def test_real_http_and_adapter_process_bind_login_to_device_and_do_not_echo_secrets(self):
        # Synthetic upstream + synthetic file vault for test subprocesses only.
        # Production's optional adapter exclusively selects a native OS keyring.
        with tempfile.TemporaryDirectory() as root:
            root = Path(root); adapter = Path(__file__).parents[2] / "services/optional-audible-adapter/booknook_audible.py"
            script = root / "fixture.py"
            script.write_text('''import importlib.util,json,sys
from pathlib import Path
spec=importlib.util.spec_from_file_location("optional_adapter",sys.argv[1]);m=importlib.util.module_from_spec(spec);spec.loader.exec_module(m)
root=Path(sys.argv[2]);root.mkdir(mode=0o700,exist_ok=True)
class Vault:
 def read(self,key): return json.loads((root/"synthetic.json").read_text()).get(key,{}) if (root/"synthetic.json").exists() else {}
 def write(self,key,value):
  data=json.loads((root/"synthetic.json").read_text()) if (root/"synthetic.json").exists() else {};data[key]=value;(root/"synthetic.json").write_text(json.dumps(data))
class Backend:
 def begin(self,market): return "https://www.amazon.com/ap/signin",{"verifier":"fixture-pkce","serial":"fixture"}
 def register(self,market,secret,code):
  if code!="synthetic-code": raise RuntimeError("raw secret must not escape")
  return {"refresh_token":"synthetic-credential"}
 def renew(self,credential): return credential
 def disconnect(self,credential): pass
try: result=m.Adapter(root,Backend(),Vault()).handle(m.strict_json(sys.stdin.buffer.read()))
except m.AdapterError as error: result={"schema":"booknook.source-error.v1","code":error.code,"raw":"raw secret must not escape"}
print(json.dumps(result))
''')
            service=m.Companion(root / "companion", provider=[sys.executable, str(script), str(adapter), str(root / "adapter")])
            server=m.http.server.ThreadingHTTPServer(("127.0.0.1",0),m.Handler);server.companion=service
            worker=threading.Thread(target=server.serve_forever,daemon=True);worker.start()
            def call(path, value, token=None):
                client=http.client.HTTPConnection("127.0.0.1",server.server_port,timeout=5)
                try:
                    headers={"Authorization":"Bearer "+token} if token else {}
                    client.request("POST",path,json.dumps(value).encode(),headers)
                    response=client.getresponse();raw=response.read();self.assertNotIn(b"raw secret",raw);return response.status,json.loads(raw)
                finally: client.close()
            try:
                code,pair=call("/v1/pair",dict(schema="booknook.companion-pair.v1",code=service.code,device_name="one"));self.assertEqual(code,200)
                token=pair["token"]
                code,begun=call("/v1/accounts/authorize",dict(provider="audible",marketplace="us"),token);self.assertEqual(code,200)
                self.assertTrue(begun["requires_virtual_device_registration"])
                # A newly paired second reader cannot complete the first reader's session.
                service.code_expiry=int(m.time.time())+300
                _,two=call("/v1/pair",dict(schema="booknook.companion-pair.v1",code=service.code,device_name="two"))
                complete=dict(provider="audible",marketplace="us",session_id=begun["session_id"],return_url="https://www.amazon.com/ap/maplanding?openid.oa2.authorization_code=synthetic-code",consent_virtual_device=True)
                self.assertEqual(call("/v1/accounts/authorize/complete",complete,two["token"])[1]["code"],"session_expired")
                self.assertEqual(call("/v1/accounts/authorize/complete",dict(complete,device_id="a"*64),token)[0],400)
                self.assertEqual(call("/v1/accounts/authorize/complete",dict(complete,consent_virtual_device=False),token)[1]["code"],"invalid_request")
                self.assertTrue(call("/v1/accounts/authorize/complete",complete,token)[1]["connected"])
                self.assertEqual(call("/v1/accounts/authorize/complete",complete,token)[1]["code"],"session_expired")
                self.assertTrue(call("/v1/accounts/renew",dict(provider="audible",marketplace="us"),token)[1]["renewed"])
                self.assertTrue(call("/v1/accounts/authorize/cancel",dict(provider="audible",marketplace="us",session_id=begun["session_id"]),token)[1]["cancelled"])
                self.assertTrue(call("/v1/accounts/disconnect",dict(provider="audible",marketplace="us"),token)[1]["disconnected"])
                _,next_login=call("/v1/accounts/authorize",dict(provider="audible",marketplace="us"),token)
                self.assertTrue(call("/v1/accounts/authorize/cancel",dict(provider="audible",marketplace="us",session_id=next_login["session_id"]),token)[1]["cancelled"])
                _,failed_login=call("/v1/accounts/authorize",dict(provider="audible",marketplace="us"),token)
                failed=dict(complete,session_id=failed_login["session_id"],return_url="https://www.amazon.com/ap/maplanding?openid.oa2.authorization_code=wrong")
                self.assertEqual(call("/v1/accounts/authorize/complete",failed,token)[1]["code"],"registration_uncertain")
                reset=dict(provider="audible",marketplace="us",confirmed_amazon_device_review=True)
                self.assertEqual(call("/v1/accounts/registration/reset",dict(reset,confirmed_amazon_device_review=False),token)[1]["code"],"reset_denied")
                self.assertTrue(call("/v1/accounts/registration/reset",reset,token)[1]["reset"])
                self.assertEqual(call("/v1/accounts/authorize",dict(provider="audible",marketplace="us"),token)[0],200)
            finally:
                server.shutdown();server.server_close();worker.join();service.db.close()
