#!/usr/bin/env python3
"""Optional static ontology host. No notebooks, credentials, or user files are served."""
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import argparse
parser = argparse.ArgumentParser()
parser.add_argument("--bind", default="127.0.0.1")
parser.add_argument("--port", type=int, default=8787)
args = parser.parse_args()
directory = Path(__file__).resolve().parents[1]/"ontology"
handler = partial(SimpleHTTPRequestHandler, directory=str(directory))
server = ThreadingHTTPServer((args.bind, args.port), handler)
print(f"Serving bundled ontology at http://{args.bind}:{server.server_port}/discovery.json", flush=True)
server.serve_forever()
