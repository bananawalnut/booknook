# SPDX-License-Identifier: MIT OR Apache-2.0
import concurrent.futures
import http.client
import importlib.util
import json
from pathlib import Path
import ssl
import subprocess
import tempfile
import threading
import unittest


def service(name, path):
    source = Path(__file__).resolve().parents[2] / "services" / path
    spec = importlib.util.spec_from_file_location(name, source)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class ServiceTLSTests(unittest.TestCase):
    def test_https_sessions_handshake_in_request_workers(self):
        issuer = service("issuer", "castalia-issuer/booknook_issuer.py")
        companion = service("companion", "library-companion/booknook_companion.py")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cert, key = root / "certificate.pem", root / "key.pem"
            config = root / "openssl.cnf"
            config.write_text("[req]\nprompt=no\ndistinguished_name=dn\nx509_extensions=ext\n"
                              "[dn]\nCN=localhost\n[ext]\nsubjectAltName=IP:127.0.0.1\n")
            subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "1",
                            "-config", str(config), "-keyout", str(key), "-out", str(cert)],
                           check=True, capture_output=True, timeout=20)
            client = ssl.create_default_context(cafile=str(cert))
            for server_type, handler, timeout, path, code in [
                (issuer.IssuerServer, issuer.Handler, 10, "/v1/credentials", "issuer_denied"),
                (companion.CompanionServer, companion.Handler, 30, "/v1/capabilities", "pairing_denied"),
            ]:
                with self.subTest(server=server_type.__name__), server_type(("127.0.0.1", 0), handler) as server:
                    if server_type is companion.CompanionServer:
                        server.companion = companion.Companion(root / "companion")
                        self.addCleanup(server.companion.db.close)
                    observations = []
                    class RecordingContext(ssl.SSLContext):
                        def wrap_socket(self, sock, *args, **kwargs):
                            observations.append((threading.current_thread(), sock.gettimeout()))
                            return super().wrap_socket(sock, *args, **kwargs)

                    context = RecordingContext(ssl.PROTOCOL_TLS_SERVER)
                    context.minimum_version = ssl.TLSVersion.TLSv1_2
                    context.load_cert_chain(cert, key)
                    server.tls_context = context
                    listener = threading.Thread(target=server.serve_forever)
                    listener.start()
                    def request(method, route, body=None, headers=None):
                        conn = http.client.HTTPSConnection(*server.server_address, context=client, timeout=5)
                        try:
                            conn.request(method, route, body, headers or {})
                            response = conn.getresponse()
                            return response.status, json.loads(response.read())
                        finally:
                            conn.close()
                    try:
                        with concurrent.futures.ThreadPoolExecutor(max_workers=2) as workers:
                            method = "POST" if server_type is issuer.IssuerServer else "GET"
                            requests = [workers.submit(request, method, path) for _ in range(2)]
                            for future in requests:
                                status, denied = future.result()
                                self.assertEqual((status, denied["code"]), (401, code))
                        if server_type is companion.CompanionServer:
                            payload = {"schema": "booknook.companion-pair.v1", "code": server.companion.code,
                                       "device_name": "Synthetic HTTPS reader"}
                            status, paired = request("POST", "/v1/pair", json.dumps(payload), {"Content-Type": "application/json"})
                            self.assertEqual((status, paired["schema"]), (200, "booknook.companion-paired.v1"))
                            status, manifest = request("GET", "/v1/capabilities", headers={"Authorization": "Bearer " + paired["token"]})
                            self.assertEqual((status, manifest), (200, paired["manifest"]))
                    finally:
                        server.shutdown()
                        listener.join()
                    self.assertEqual(len(observations), 4 if server_type is companion.CompanionServer else 2)
                    self.assertTrue(all(worker is not listener and limit == timeout for worker, limit in observations))


if __name__ == "__main__":
    unittest.main()
