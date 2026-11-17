#!/usr/bin/env python3
"""Publish schemas from Rust definitions; derive TS data types and a checksummed manifest."""
from copy import deepcopy
import hashlib
import json
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
if "--schema-probe" in sys.argv:
    probe = Path(sys.argv[sys.argv.index("--schema-probe") + 1]).resolve(strict=True)
    schemas = json.loads(subprocess.check_output([str(probe)], cwd=ROOT))
else:
    schemas = json.loads(subprocess.check_output(["cargo", "run", "--quiet", "--locked", "-p", "booknook-core", "--example", "schema"], cwd=ROOT))

def ts(schema):
    if schema is True:
        return "unknown"
    if schema is False:
        return "never"
    if not schema:
        return "unknown"
    if "$ref" in schema:
        return schema["$ref"].split("/")[-1]
    for key in ["oneOf", "anyOf"]:
        if key in schema:
            return " | ".join("(" + ts(s) + ")" for s in schema[key])
    if "const" in schema:
        return json.dumps(schema["const"])
    if "enum" in schema:
        return " | ".join(json.dumps(x) for x in schema["enum"])
    typ = schema.get("type")
    if isinstance(typ, list):
        return " | ".join(ts({**schema, "type": t}) for t in typ)
    if typ in ("integer", "number"):
        return "number"
    if typ in ("string", "boolean", "null"):
        return typ
    if typ == "array":
        if "prefixItems" in schema:
            return "[" + ", ".join(ts(x) for x in schema["prefixItems"]) + "]"
        return "Array<" + ts(schema.get("items", {})) + ">"
    if typ == "object":
        props = schema.get("properties", {})
        if not props:
            additional = schema.get("additionalProperties", {})
            return "Record<string, " + (ts(additional) if isinstance(additional, dict) else "unknown") + ">"
        required = schema.get("required", [])
        return "{ " + "; ".join(json.dumps(k) + ("" if k in required else "?") + ": " + ts(v) for k, v in props.items()) + " }"
    return "unknown"

outputs = {}
issued_comparisons = {}
PROVIDER_SCHEMAS = {"provider-registry", "provider-operation", "provider-request", "provider-response", "provider-execution"}
NEW_ONTOLOGY = "0.4.0"
NEW_PORTABLE_EDITION = "3"
NEW_ONTOLOGY_DIR = f"ontology/v{NEW_ONTOLOGY}"
NEW_PORTABLE_DIR = f"ontology/portable/v{NEW_PORTABLE_EDITION}"


def render_schema(schema):
    return (json.dumps(schema, indent=2, ensure_ascii=False) + "\n").encode("utf-8")


def restrict_to_ordered_state(state):
    state["properties"]["schema"] = {"type": "string", "const": "booknook.archive.v4"}
    state["properties"]["ontology_version"] = {"type": "string", "const": NEW_ONTOLOGY}


for name, live in schemas.items():
    # Publication projections must not narrow either of the current SDK State declarations.
    schema = deepcopy(live)
    if name in {"state", "portable-export"}:
        if name == "state":
            directory = NEW_ONTOLOGY_DIR
            schema["$id"] = f"urn:booknook:schema:{NEW_ONTOLOGY}:state"
            restrict_to_ordered_state(schema)
            historical = ROOT / "ontology/v0.3.0/state.schema.json"
        else:
            directory = NEW_PORTABLE_DIR
            schema["$id"] = f"urn:booknook:schema:portable-export:v{NEW_PORTABLE_EDITION}"
            schema["properties"]["schema"] = {"type": "string", "const": "booknook.portable-export.v1"}
            restrict_to_ordered_state(schema["$defs"]["State"])
            historical = ROOT / "ontology/portable/v2/portable-export.schema.json"
        old_notebook = json.loads(historical.read_bytes())["$defs"]["Notebook"]
        notebook = schema["$defs"]["Notebook"]
        new_fields = set(notebook["properties"]) - set(old_notebook["properties"])
        if len(new_fields) != 1 or new_fields & set(notebook["required"]):
            raise SystemExit(f"Expected the real optional Rust ordering payload in {name}")
        outputs[ROOT / f"{directory}/{name}.schema.json"] = render_schema(schema)
        continue
    if name in {"notebook-order-command", "pending-notebook-order-review"}:
        schema["$id"] = f"urn:booknook:schema:{NEW_ONTOLOGY}:{name}"
        outputs[ROOT / f"{NEW_ONTOLOGY_DIR}/{name}.schema.json"] = render_schema(schema)
        continue
    if name == "social-archive":
        schema["$id"] = "urn:booknook:schema:social-archive:v2"
        directory = "ontology/portable/v2"
    elif name == "card-context-command":
        schema["$id"] = "urn:booknook:schema:card-context-command:v1"
        directory = "ontology/commands/v1"
    elif name in PROVIDER_SCHEMAS:
        schema["$id"] = f"urn:booknook:schema:{name}:v1"
        directory = "ontology/providers/v1"
    else:
        schema["$id"] = {
            "castalia-provider-connection": "urn:castalia:schema:provider-connection:v1",
            "castalia-native-authority-profile": "urn:castalia:schema:native-authority-profile:v1",
        }.get(name, f"urn:booknook:schema:0.3.0:{name}")
        directory = "ontology/v0.3.0"
    issued_comparisons[ROOT / f"{directory}/{name}.schema.json"] = render_schema(schema)

def typed_module(names, output):
    definitions = {}
    for name in names:
        definitions.update(schemas[name].get("$defs", {}))
    lines = ["// SPDX-License-Identifier: MIT OR Apache-2.0", "// Generated by scripts/generate-contracts.py; do not edit."]
    for name, definition in sorted(definitions.items()):
        if name not in names.values():
            lines.append(f"export type {name} = {ts(definition)};")
    for name, typename in names.items():
        lines.append(f"export type {typename} = {ts(schemas[name])};")
    outputs[ROOT / output] = ("\n".join(lines) + "\n").encode("utf-8")

typed_module({"state": "State", "command": "Command", "card-context-command": "CardContextCommand", "notebook-order-command": "NotebookOrderCommand", "pending-notebook-order-review": "PendingNotebookOrderReview", "connections": "Connections", "castalia-provider-connection": "ProviderConnection", "castalia-native-authority-profile": "NativeAuthorityProfile", "identity": "AttributionLink", "audio-observation": "AudioObservation", "audio-candidates": "AudioEditionCandidates"}, "sdk/typescript/types.ts")
typed_module({"social": "SocialState", "social-action": "SocialAction", "social-local": "LocalSocialCommand", "social-event": "SocialEvent", "publication": "ApprovedPublication", "authority-policy": "AuthorityPolicy"}, "sdk/typescript/social-types.ts")

typed_module({"provider-registry": "ProviderRegistry", "provider-operation": "ProviderOperation", "provider-request": "ProviderRequest", "provider-response": "ProviderResponse", "provider-execution": "ProviderExecution"}, "sdk/typescript/provider-types.ts")

typed_module({"portable-export": "PortableArchive", "social-archive": "SocialArchiveBundle"}, "sdk/typescript/archive-types.ts")

def new_manifest(directory, initial, members):
    root = ROOT / directory
    artifacts = []
    for member in sorted(members):
        path = root / member
        prospective = outputs[path] if path in outputs else path.read_bytes()
        artifacts.append({"path": member, "sha256": hashlib.sha256(prospective).hexdigest()})
    outputs[root / "manifest.json"] = (json.dumps({**initial, "artifacts": artifacts}, indent=2) + "\n").encode("utf-8")


new_manifest(NEW_ONTOLOGY_DIR, {
    "schema": "booknook.ontology-manifest.v1", "version": NEW_ONTOLOGY, "namespace": "urn:booknook:ontology:",
    "castalia": {"version": "0.6.0", "canonical_base_url": "https://zenith-research.ca/ontology/v0.6.0", "bundle_digest": "11d670a57b2052b4de00509559d081b524bf953c6da2544a25f33976857f8f4f"},
}, {"state.schema.json", "ontology.jsonld", "conformance.json",
    "notebook-order-command.schema.json", "pending-notebook-order-review.schema.json"})
new_manifest(NEW_PORTABLE_DIR, {
    "schema": "booknook.portable-contract-manifest.v1", "version": NEW_PORTABLE_EDITION, "namespace": "urn:booknook:schema:",
}, {"portable-export.schema.json"})

# Keep the historical schemas independently generated and compared except for the two
# explicitly evolved documents. All issued manifests themselves are Pages-pinned.
# Historical and unrelated outputs are never written, even in regular generation mode.
for relative in ("sdk/typescript/social-types.ts", "sdk/typescript/provider-types.ts"):
    path = ROOT / relative
    issued_comparisons[path] = outputs.pop(path)
for path, expected in issued_comparisons.items():
    if not path.is_file() or path.read_bytes() != expected:
        raise SystemExit(f"Generated contract differs from unchanged edition: {path}")

for path, content in outputs.items():
    if "--check" in sys.argv:
        if not path.is_file() or path.read_bytes() != content:
            raise SystemExit(f"Generated contract is stale: {path}")
    else:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
print("Contracts and manifests verified." if "--check" in sys.argv else "Contracts and manifests generated.")
