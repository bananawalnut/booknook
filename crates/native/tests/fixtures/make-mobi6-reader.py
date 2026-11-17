#!/usr/bin/env python3
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Generate a tiny original, synthetic MOBI6 reader test input."""

import hashlib
from pathlib import Path
import struct


def main():
    text = (
        "<html><head><title>The Quiet Margin</title></head><body>"
        "<h1>The Quiet Margin</h1><p>A window opened above the reading desk.</p>"
        "<p>Café, 日本語, 🌿.</p><p>The notebook still held yesterday’s question.</p>"
        "</body></html>"
    ).encode()
    # Keep the multibyte character split across record payloads and duplicate
    # the overlap in the record's external trailer, as a legacy writer does.
    cut = text.index("🌿".encode()) + 2
    chunks = [text[:cut], text[cut:]]
    header = bytearray(16 + 0xE8)
    struct.pack_into(">HHIHHHH", header, 0, 2, 0, len(text), len(chunks), 4096, 0, 0)
    struct.pack_into(">4sIIIII", header, 16, b"MOBI", 0xE8, 2, 65001, 1, 6)
    struct.pack_into(">II", header, 16 + 0x18, 0xFFFFFFFF, 0xFFFFFFFF)
    struct.pack_into(">II", header, 16 + 0x94, 0xFFFFFFFF, 0xFFFFFFFF)
    struct.pack_into(">I", header, 16 + 0xE0, 3)
    records = [bytes(header)]
    for index, chunk in enumerate(chunks):
        # Construct PalmDOC literal runs independently of the reader.
        payload = b"".join(bytes([len(part)]) + part for start in range(0, len(chunk), 8)
                           if (part := chunk[start:start + 8]))
        overlap = chunks[1][:2] if index == 0 else b""
        records.append(payload + overlap + bytes([len(overlap)]) + b"\x81")
    records.append(b"\xe9\x8e\r\n")
    pdb = bytearray(78 + 8 * len(records) + 2)
    pdb[:16] = b"The Quiet Margin"
    pdb[60:68] = b"BOOKMOBI"
    struct.pack_into(">H", pdb, 76, len(records))
    offset = len(pdb)
    for i, record in enumerate(records):
        struct.pack_into(">II", pdb, 78 + 8 * i, offset, 2 * i)
        offset += len(record)
    output = Path(__file__).resolve().parent / "mobi6-reader.mobi"
    data = bytes(pdb) + b"".join(records)
    with output.open("xb") as file:
        file.write(data)
    print(len(data), hashlib.sha256(data).hexdigest())


if __name__ == "__main__":
    main()
