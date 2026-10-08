"""Record framing and the unit core, per §2 and §3.1 of ``SMYSL_FORMAT_SPEC.md``.

Everything here is C-Read: decode a store, re-encode it byte-identically, and preserve what
this implementation does not understand. That is the floor the spec names, and the floor
everything above it rests on.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from .cbor import CborError, Decoder, encode_one

#: §3.1, every code the format has allocated. A code is a permanent wire commitment the moment
#: it is allocated, so this table names the reserved and not-yet-defined ones too: 9 is
#: checkpoint, 16 is reserved, and 17 and 19 are specified but land in later releases. Naming
#: them is how a reader says *what* it met rather than only that it met something.
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
    11: "withdrawal",
    12: "resolution",
    13: "commitment",
    14: "manifest",
    15: "part_text",
    16: "reserved",
    17: "dating",
    18: "part_reading",
    19: "redaction",
}

#: The codes whose **bodies** this implementation decodes.
#:
#: Not the same question as :data:`RECORD_NAMES`, and conflating the two was wrong before it
#: was consequential. `is_known` was derived from the name table and documented as whether this
#: version understands the record — which was already false for 9, a checkpoint nothing has
#: ever implemented, and harmless only because nothing emits one. 1.10 writes manifests, part
#: texts and part readings, and a reader that called those "known" while interpreting none of
#: them would be exactly the silence ``SMY-W014`` exists to break.
#:
#: An unknown record is still preserved verbatim and re-encoded byte for byte; that is C-Read
#: and it is unaffected.
UNDERSTOOD_RECORDS = frozenset({1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13})

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
        """Whether this implementation decodes this record's body.

        A code the spec names but this reader does not interpret — a manifest, a checkpoint —
        is **not** known. :attr:`name` still answers, so a report can say "a manifest, which
        this build does not interpret" rather than "something".
        """
        return self.code in UNDERSTOOD_RECORDS

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
