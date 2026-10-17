#!/usr/bin/env python3
"""Creates an explicitly synthetic desk for UI qualification. Refuses nonempty desks."""
from pathlib import Path
import json
import os
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parents[1]
directory = Path(sys.argv[1]).resolve()
binary = Path(os.environ.get("BOOKNOOK_CLI", ROOT / "target/debug/booknook"))
def cli(*args, data=None):
    raw = subprocess.check_output([str(binary), str(directory), *args], input=json.dumps(data).encode() if data is not None else None)
    return json.loads(raw) if raw else None
def command(data): return cli("command", data=data)
state = cli("snapshot")
if state["books"]:
    raise SystemExit("Preview seed requires an empty desk; existing data left untouched.")
command({"op":"import_goodreads","csv":(ROOT/"fixtures/goodreads.csv").read_text(),"at":1791676800})
state = cli("snapshot")
book = next(b for b in state["books"].values() if b["title"] == "The Quiet Library")
response = command({"op":"open_notebook","book_id":book["id"],"at":1791676800})
n = response["result"]
command({"op":"edit_block","notebook_id":n["id"],"block_id":n["blocks"][0]["id"],"base_update":n["blocks"][0]["update"],"text":"The opening makes attention feel like a form of care. Mara notices what everyone else passes by.\n\nIs the library really changing, or is she learning how to see it?\n\nI want to return to that question after the next chapter.","at":1791676860})
response = command({"op":"add_character","notebook_id":n["id"],"block_id":n["blocks"][0]["id"],"name":"Mara","at":1791676900})
card = response["state"]["cards"][response["result"]["card_id"]]
command({"op":"save_card","card_id":card["id"],"parents":[card["versions"][0]["id"]],"interpretation":{"description":"Observant and quietly persistent. She seems to trust places more easily than people.","traits":["attentive","reserved","curious"],"relationships":[],"observations":["She notices the missing chair before the missing book."],"questions":["What brought her back?"],"passages":[]},"positions":[],"at":1791676920})
epub = directory/"preview.epub"
with zipfile.ZipFile(epub, "w") as z:
    z.writestr("mimetype", "application/epub+zip")
    z.writestr("META-INF/container.xml", '<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
    z.writestr("EPUB/package.opf", '<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">booknook-original-fixture</dc:identifier><dc:title>The Quiet Library</dc:title><dc:language>en</dc:language></metadata><manifest><item id="c1" href="chapter.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="c1"/></spine></package>')
    z.writestr("EPUB/chapter.xhtml", '<html xmlns="http://www.w3.org/1999/xhtml"><head><title>The Quiet Library</title></head><body><p style="letter-spacing:0.2em;font-size:12px">CHAPTER ONE</p><h1>The chair by the window</h1><p>Mara returned to the library on a Thursday, when the rain had made the street the color of old silver.</p><p>Nothing seemed to have changed. The same clock measured the afternoon. The same green lamp waited on the desk. But the chair by the window was gone.</p><p>She stood for a moment with her hand on the door, listening to the small sounds that a room makes when it believes itself empty.</p><p>“You noticed,” said the librarian.</p><p>“It was always there.”</p><p>“Most things are,” he said, “until someone remembers to look.”</p><p>Mara set her bag on the floor. Somewhere beyond the shelves, a page turned.</p><hr/><p style="font-size:12px">Original Book Nook test fixture. Not an imported commercial book.</p></body></html>')
cli("import-content",book["id"],str(epub))
print("Synthetic preview created at", directory)
