#!/usr/bin/env python3
# SPDX-License-Identifier: MPL-2.0
"""Optional personal issuer HTTP transport; Rust owns authentication/policy/signing.

No key, token, grant or trust configuration is generated. No request bodies are logged.
"""
from __future__ import annotations
import argparse
import http.server
import ipaddress
import json
import os
from pathlib import Path
import selectors
import ssl
import subprocess
import threading
import time

MAX_BYTES = 262_144

class IssuerFailure(Exception):
    def __init__(self, status, code):
        self.status, self.code = status, code

def strict_json(raw):
    def unique(pairs):
        out = {}
        for key, value in pairs:
            if key in out:
                raise ValueError("duplicate")
            out[key] = value
        return out
    def invalid(_):
        raise ValueError("nonfinite")
    try:
        return json.loads(raw, object_pairs_hook=unique, parse_constant=invalid)
    except (ValueError, UnicodeError) as error:
        raise IssuerFailure(400, "issuer_invalid_request") from error

class Issuer:
    def __init__(self, executable, configuration):
        self.executable, self.configuration = str(executable), str(configuration)
        self.slots = threading.BoundedSemaphore(4)

    def issue(self, authorization, body):
        if len(body) > MAX_BYTES or not isinstance(authorization, str) or len(authorization) > 1031:
            raise IssuerFailure(413, "payload_too_large")
        value = strict_json(body)
        if not isinstance(value, dict):
            raise IssuerFailure(400, "issuer_invalid_request")
        payload = json.dumps({"schema":"booknook.issuer-call.v1", "authorization":authorization, "request":value}, ensure_ascii=False, separators=(",", ":")).encode()
        if not self.slots.acquire(blocking=False):
            raise IssuerFailure(503, "issuer_busy")
        child = None
        try:
            child = subprocess.Popen([self.executable, "--config", self.configuration, "issue"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, shell=False)
            # Input, output and elapsed time are bounded, including a broken operator-selected executable.
            for stream in (child.stdin, child.stdout, child.stderr):
                os.set_blocking(stream.fileno(), False)
            selector = selectors.DefaultSelector()
            selector.register(child.stdin, selectors.EVENT_WRITE, "stdin")
            selector.register(child.stdout, selectors.EVENT_READ, "stdout")
            selector.register(child.stderr, selectors.EVENT_READ, "stderr")
            output, sent, total, deadline = bytearray(), 0, 0, time.monotonic() + 10
            try:
                while selector.get_map():
                    if time.monotonic() > deadline:
                        raise IssuerFailure(504, "issuer_timeout")
                    for entry, _ in selector.select(timeout=0.1):
                        if entry.data == "stdin":
                            try:
                                sent += os.write(entry.fd, payload[sent:sent+65536])
                            except BrokenPipeError:
                                sent = len(payload)
                            if sent == len(payload):
                                selector.unregister(entry.fileobj)
                                entry.fileobj.close()
                        else:
                            chunk = os.read(entry.fd, 65536)
                            if not chunk:
                                selector.unregister(entry.fileobj)
                                continue
                            total += len(chunk)
                            if total > MAX_BYTES:
                                raise IssuerFailure(502, "issuer_invalid_response")
                            if entry.data == "stdout":
                                output.extend(chunk)
                result = child.wait(timeout=1)
            finally:
                selector.close()
            if result:
                status, code = {2:(400,"issuer_invalid_request"), 3:(403,"issuer_denied"), 4:(503,"issuer_not_configured")}.get(result, (503,"issuer_unavailable"))
                raise IssuerFailure(status, code)
            response = strict_json(output)
            if not isinstance(response, dict) or response.get("schema") != "castalia.credential-presentation-request.v2":
                raise IssuerFailure(502, "issuer_invalid_response")
            return output
        except (OSError, subprocess.SubprocessError) as error:
            raise IssuerFailure(503, "issuer_unavailable") from error
        finally:
            if child is not None:
                if child.poll() is None:
                    child.kill()
                child.wait()
                for stream in (child.stdin, child.stdout, child.stderr):
                    stream.close()
            self.slots.release()

class Handler(http.server.BaseHTTPRequestHandler):
    server_version = "BookNookIssuer/0.1"
    def log_message(self, *_):
        pass
    def setup(self):
        super().setup()
        self.connection.settimeout(10)
    def do_POST(self):
        try:
            if self.path != "/v1/credentials":
                raise IssuerFailure(404, "unsupported_operation")
            if any(self.headers.get_all(name) for name in ("Origin", "Referer", "Cookie", "Transfer-Encoding")):
                raise IssuerFailure(403, "issuer_denied")
            if len(self.headers.get_all("Authorization", [])) != 1:
                raise IssuerFailure(401, "issuer_denied")
            if len(self.headers.get_all("Content-Length", [])) != 1 or len(self.headers.get_all("Content-Type", [])) != 1:
                raise IssuerFailure(400, "issuer_invalid_request")
            if self.headers.get("Content-Type").split(";")[0].strip().lower() != "application/json":
                raise IssuerFailure(415, "issuer_invalid_request")
            length = self.headers.get("Content-Length")
            if not length.isascii() or not length.isdecimal() or not 0 < int(length) <= MAX_BYTES:
                raise IssuerFailure(413, "payload_too_large")
            body = self.rfile.read(int(length))
            if len(body) != int(length):
                raise IssuerFailure(400, "issuer_invalid_request")
            output = self.server.issuer.issue(self.headers.get("Authorization"), body)
            status = 200
        except IssuerFailure as error:
            status, output = error.status, json.dumps({"code":error.code, "error":"Credential issuance unavailable; check current issuer configuration and grants."}).encode()
        except (OSError, ValueError, OverflowError):
            status, output = 400, b'{"code":"issuer_invalid_request"}'
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Cache-Control", "no-store")
        self.send_header("Connection", "close")
        self.send_header("Content-Length", str(len(output)))
        self.end_headers()
        self.wfile.write(output)
        self.close_connection = True

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--issuer-bin", type=Path, required=True)
    parser.add_argument("--config", type=Path, required=True)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8779)
    parser.add_argument("--cert", type=Path)
    parser.add_argument("--key", type=Path)
    args = parser.parse_args()
    if not args.issuer_bin.is_absolute() or not args.config.is_absolute():
        parser.error("issuer-bin and config must be absolute paths")
    try:
        loopback = ipaddress.ip_address(args.host).is_loopback
    except ValueError:
        loopback = False
    if bool(args.cert) != bool(args.key) or (not loopback and not args.cert):
        parser.error("non-loopback binding requires an explicit TLS certificate and key")
    server = http.server.ThreadingHTTPServer((args.host, args.port), Handler)
    server.issuer = Issuer(args.issuer_bin, args.config)
    if args.cert:
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.minimum_version = ssl.TLSVersion.TLSv1_2
        context.load_cert_chain(args.cert, args.key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
    print(json.dumps({"schema":"booknook.issuer-listener.v1", "port":server.server_address[1], "tls":bool(args.cert)}), flush=True)
    try:
        server.serve_forever()
    finally:
        server.server_close()

if __name__ == "__main__":
    main()
