"""Record framing and the unit core, per §2 and §3.1 of ``SMYSL_FORMAT_SPEC.md``.

Everything here is C-Read: decode a store, re-encode it byte-identically, and preserve what
this implementation does not understand. That is the floor the spec names, and the floor
everything above it rests on.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from .cbor import CborError, Decoder, encode_one

#: §3.1. An unknown code is preserved verbatim and skipped semantically, never rejected.
RECORD_NAMES = {
    1: "unit",
    2: "attestation",
    3: "relation",
    4: "thread",
    5: "view",
    6: "contention",
    7: "pack_info",
    8: "schema_decl",
    9: "checkpoint",
    10: "label_binding",
}

#: §2.2, the unit core's integer keys. Anything at 9 or above is an unknown key that rule X
#: says must survive a round trip verbatim.
UNIT_KEYS = {
    0: "schema",
    1: "gist",
    2: "body",
    3: "detail",
    4: "deps",
    5: "grounds",
    6: "status",
    7: "source",
    8: "payload",
}


@dataclass
class Record:
    """One record: a type code and its body, plus the bytes it came from."""

    code: int
    body: Any
    raw: bytes = field(repr=False)

    @property
    def name(self) -> str:
        return RECORD_NAMES.get(self.code, f"unknown({self.code})")

    @property
    def is_known(self) -> bool:
        return self.code in RECORD_NAMES

    def reencode(self) -> bytes:
        return encode_one([self.code, self.body])

    def unit_fields(self) -> dict[str, Any]:
        """Name a unit core's known keys. Unknown keys keep their integer, per rule X."""
        if self.code != 1 or not isinstance(self.body, dict):
            raise CborError("not a unit core")
        return {UNIT_KEYS.get(k, k): v for k, v in self.body.items()}


def decode_store(data: bytes) -> list[Record]:
    """Decode a concatenation of records (§3.1: no framing envelope)."""
    out: list[Record] = []
    off = 0
    while off < len(data):
        d = Decoder(data[off:])
        major, arg, _ = d._head()
        if major != 4 or arg != 2:
            raise CborError("a record is a two-element array")
        code = d.value()
        if not isinstance(code, int) or code < 0:
            raise CborError("a record's type code is an unsigned integer")
        body = d.value()
        out.append(Record(code=code, body=body, raw=data[off : off + d.i]))
        off += d.i
    return out


def encode_store(records: list[Record]) -> bytes:
    return b"".join(r.reencode() for r in records)
