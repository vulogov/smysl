"""Export collections, read by a second implementation (SMYSL-2.7 §5.4, TX-P11 step 11).

Three things, with no new dependency:

- :func:`record_set_digest`, the spec's §2.7 digest, over the raw bytes every decoded record
  keeps. ``fixtures/wire/rsd/`` holds vectors the Rust produced.
- :func:`read_export_index`, the index of a collection, from generic CBOR and the field numbers
  of SMYSL-2.7 §4.2.
- :func:`verify_export`, which checks each file the index names against its BLAKE3, its record
  count and its record-set digest, re-encodes every record to the bytes it was read from, and
  checks the collection's digest.

``python -m smysl.export verify DIR`` prints one JSON line and exits 0 when the collection
verifies; ``python -m smysl.export write DIR`` writes what this implementation produces, for the
Rust to import (§5.4 item 3).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from .blake3 import blake3
from .cbor import decode_one
from .records import decode_store, encode_store
from .uid import UnitCore, relation_id, _head, _text, _bytes, _map

INDEX_FILE = "export-index.cbor"
INDEX_VERSION = "smysl/export-index/1"


def record_set_digest(raws) -> bytes:
    """§2.7: BLAKE3 over ``"smysl/rsd/1" ‖ 0x00`` and the sorted unique hashes of the records."""
    hashes = sorted({blake3(r) for r in raws})
    return blake3(b"smysl/rsd/1\x00" + b"".join(hashes))


def read_export_index(data: bytes) -> dict:
    """The index's fields by name. Raises ``ValueError`` on anything that is not one."""
    value, used = decode_one(data)
    if used != len(data) or not isinstance(value, dict):
        raise ValueError("an export index is one CBOR map")
    if value.get(0) != INDEX_VERSION:
        raise ValueError(f"not {INDEX_VERSION}")
    files = []
    for f in value.get(1, []):
        files.append(
            {
                "path": f[0],
                "blake3": f[1],
                "records": f[2],
                "rsd": f[3],
                "view": f.get(4),
                "requires": f.get(5, []),
                "role": f[6],
            }
        )
    return {
        "files": files,
        "scope": value.get(2, ""),
        "layout": value.get(6),
        "closure": value.get(7),
        "format": value.get(8),
        "rsd": value.get(11),
        "flags": value.get(12, []),
        "complete": value.get(14),
    }


def verify_export(directory: Path) -> dict:
    """Check a collection on disk. ``ok`` is true when every check holds."""
    directory = Path(directory)
    index = read_export_index((directory / INDEX_FILE).read_bytes())
    problems: list[str] = []
    raws: list[bytes] = []
    for f in index["files"]:
        p = directory / f["path"]
        if not p.is_file():
            problems.append(f"{f['path']}: absent")
            continue
        data = p.read_bytes()
        if blake3(data) != f["blake3"]:
            problems.append(f"{f['path']}: its BLAKE3 is not the index's")
            continue
        if f["path"].endswith(".smy"):
            # A surface file is checked by its hash; reading surface text is the Rust's.
            continue
        records = decode_store(data)
        if encode_store(records) != data:
            problems.append(f"{f['path']}: does not re-encode to its own bytes")
        if len(records) != f["records"]:
            problems.append(f"{f['path']}: {len(records)} records, the index says {f['records']}")
        if record_set_digest(r.raw for r in records) != f["rsd"]:
            problems.append(f"{f['path']}: its record-set digest is not the index's")
        raws.extend(r.raw for r in records)
    rsd = record_set_digest(raws)
    surface = any(f["path"].endswith(".smy") for f in index["files"])
    if not surface and rsd != index["rsd"]:
        problems.append("the collection's record-set digest is not the index's")
    return {
        "ok": not problems,
        "files": len(index["files"]),
        "rsd": rsd.hex(),
        "problems": problems,
    }


def _unit_record(core: UnitCore) -> bytes:
    return _head(4, 2) + b"\x01" + core.canonical_bytes()


def write_produced(out: Path) -> list[Path]:
    """What this implementation writes for the Rust to import: every wire fixture re-encoded by
    this package's encoder, and a small store built here, two units and an edge between them."""
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    written = []
    wire = Path(__file__).resolve().parents[2] / "fixtures" / "wire"
    for p in sorted(wire.glob("*.cbor")):
        dest = out / p.name
        dest.write_bytes(encode_store(decode_store(p.read_bytes())))
        written.append(dest)
    a = UnitCore(schema="claim", gist="Written by the Python implementation.", status=1)
    b = UnitCore(schema="claim", gist="And grounded on nothing but its own say-so.", status=1)
    edge = _head(4, 2) + b"\x03" + _map(
        [(0, _text("x.xi/related")), (1, _bytes(a.uid())), (2, _bytes(b.uid()))]
    )
    assert len(relation_id("x.xi/related", a.uid(), b.uid())) == 32
    dest = out / "python-small.cbor"
    dest.write_bytes(_unit_record(a) + _unit_record(b) + edge)
    written.append(dest)
    return written


def main(argv: list[str]) -> int:
    if len(argv) == 2 and argv[0] == "verify":
        result = verify_export(Path(argv[1]))
        print(json.dumps(result))
        return 0 if result["ok"] else 1
    if len(argv) == 2 and argv[0] == "write":
        for p in write_produced(Path(argv[1])):
            print(p.name)
        return 0
    print("usage: python -m smysl.export verify DIR | write DIR", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
