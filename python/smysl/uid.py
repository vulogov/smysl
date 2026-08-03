"""Deriving a uid — §2.1 and §2.3, the conformance class C-Read cannot reach.

Why this exists
---------------

`test_conformance.py` and `test_spec.py` read documents. Reading never requires deriving a
uid, so both could pass in full while this implementation had no idea what a uid *is*. That
left §2.3 — *status is part of identity*, the paragraph the whole format rests on — verified by
the Rust alone, across nine releases and three independent readers.

C-Produce is what reaches it, and it needs two things the reading classes do not: a hash, and
the ability to lay out a unit core in canonical form. The hash is in `blake3.py`, hand-rolled
because a binding to the same C library the reference implementation uses would have tested two
callers of one implementation rather than two implementations.

The layout is here. It is written from §2.2 and §3 of the specification, and checked against
`fixtures/wire/uid/cases.json`, which carries the Rust's canonical bytes as well as its uids —
so a disagreement says whether the encoding or the hash was wrong.

What §2.3 actually claims
-------------------------

That a unit's epistemic status is *inside* its identity, not metadata attached to it. Promote a
speculation to a finding and you have a different unit with a different uid, not the same unit
updated. Nothing downstream can quietly launder a hedge into a fact, because the laundered
version does not answer to the original's name.

The fixture pair `status-cited` / `status-measured` is the witness: every field identical, one
byte different in the canonical encoding, two unrelated uids.
"""

from __future__ import annotations

import unicodedata
from dataclasses import dataclass, field
from typing import Optional

from .blake3 import blake3

# §2.2, unit core keys. Anything ≥ 9 is an unknown key and is carried verbatim (rule X).
SCHEMA = 0
GIST = 1
BODY = 2
DETAIL = 3
DEPS = 4
GROUNDS = 5
STATUS = 6
SOURCE = 7
PAYLOAD = 8

# The source sub-map.
SOURCE_KIND = 0
SOURCE_REFERENCE = 1
SOURCE_CAPTURED = 2

UID_LEN = 32


def _head(major: int, arg: int) -> bytes:
    """A CBOR head in shortest form (§3, constraint 2)."""
    if arg < 24:
        return bytes([(major << 5) | arg])
    if arg <= 0xFF:
        return bytes([(major << 5) | 24, arg])
    if arg <= 0xFFFF:
        return bytes([(major << 5) | 25]) + arg.to_bytes(2, "big")
    if arg <= 0xFFFFFFFF:
        return bytes([(major << 5) | 26]) + arg.to_bytes(4, "big")
    return bytes([(major << 5) | 27]) + arg.to_bytes(8, "big")


def _text(s: str) -> bytes:
    """NFC first (§3, constraint 6). Normalisation is part of identity, not presentation.

    Two editors typing the same word differently must produce one unit, so this happens at the
    encoder rather than being assumed of the caller — which is what §3 constraint 6 says to do,
    in the sentence recording that the reference implementation once assumed it.
    """
    raw = unicodedata.normalize("NFC", s).encode("utf-8")
    return _head(3, len(raw)) + raw


def _bytes(b: bytes) -> bytes:
    return _head(2, len(b)) + b


def _uint(n: int) -> bytes:
    return _head(0, n)


def _uid_set(uids) -> bytes:
    # Ascending by uid bytes. The Rust holds these in a `BTreeSet`, which is the same order —
    # a set has no insertion order to preserve, and a canonical encoding cannot invent one.
    ordered = sorted(set(uids))
    out = _head(4, len(ordered))
    for u in ordered:
        if len(u) != UID_LEN:
            raise ValueError(f"a uid is {UID_LEN} bytes; got {len(u)}")
        out += _bytes(u)
    return out


@dataclass
class Source:
    """§1.1. `kind` is the integer code, not the name."""

    kind: int
    reference: str
    captured: Optional[str] = None  # "YYYY-MM-DD"

    def encode(self) -> bytes:
        entries = [
            (SOURCE_KIND, _uint(self.kind)),
            (SOURCE_REFERENCE, _text(self.reference)),
        ]
        if self.captured is not None:
            entries.append((SOURCE_CAPTURED, _text(self.captured)))
        return _map(entries)


def _map(entries: list[tuple[int, bytes]]) -> bytes:
    """Entries sorted by integer key, ascending, no duplicates (§3, constraint 4)."""
    entries = sorted(entries, key=lambda kv: kv[0])
    keys = [k for k, _ in entries]
    if len(set(keys)) != len(keys):
        raise ValueError("duplicate key in a canonical map")
    out = _head(5, len(entries))
    for k, v in entries:
        out += _uint(k) + v
    return out


@dataclass
class UnitCore:
    """The hashed content of a unit. Not the envelope: the type code is framing, not content."""

    schema: str
    gist: str
    status: int
    body: Optional[str] = None
    detail: Optional[str] = None
    deps: list[bytes] = field(default_factory=list)
    grounds: list[bytes] = field(default_factory=list)
    source: Optional[Source] = None
    payload: Optional[bytes] = None
    # Unknown keys (≥ 9) carried verbatim, so a unit that round-tripped through an older
    # implementation still hashes to the value it had (rule X).
    extra: dict[int, bytes] = field(default_factory=dict)

    def canonical_bytes(self) -> bytes:
        """The hash input of §2.1."""
        entries: list[tuple[int, bytes]] = [
            (SCHEMA, _text(self.schema)),
            (GIST, _text(self.gist)),
            (STATUS, _uint(self.status)),
        ]
        # An absent optional is *omitted*, never null (§3, constraint 5). An empty set is
        # likewise omitted — it is indistinguishable from an absent one, so encoding it would
        # give one unit two encodings.
        if self.body is not None:
            entries.append((BODY, _text(self.body)))
        if self.detail is not None:
            entries.append((DETAIL, _text(self.detail)))
        if self.deps:
            entries.append((DEPS, _uid_set(self.deps)))
        if self.grounds:
            entries.append((GROUNDS, _uid_set(self.grounds)))
        if self.source is not None:
            entries.append((SOURCE, self.source.encode()))
        if self.payload is not None:
            entries.append((PAYLOAD, _bytes(self.payload)))
        for k, raw in self.extra.items():
            if k <= PAYLOAD:
                raise ValueError(f"key {k} is a kernel key, not an extension")
            entries.append((k, raw))
        return _map(entries)

    def uid(self) -> bytes:
        """§2.1. BLAKE3 over the canonical bytes — status included, which is §2.3."""
        return blake3(self.canonical_bytes())
