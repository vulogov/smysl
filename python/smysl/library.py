"""The library records and their identities — §2.6 and §3.1 of ``SMYSL_FORMAT_SPEC.md``.

Why this exists
---------------

Until now this implementation had the *names* of records 14, 15 and 18 and nothing else: it
preserved them verbatim, re-encoded them byte for byte, and reported each as named but not
understood. That is C-Read and it was honest. It also meant the four library identities — tid,
mid, did, rdid — had been derived by exactly one implementation, the one that specified them.

§2.6 makes a claim no reading test can reach: that the one-byte domain prefix keeps the kinds
apart, so that **no library identity can equal a uid and no two kinds can equal each other**,
however their preimages collide. A uid's preimage is a canonical CBOR map, whose first byte is
``0xa0``–``0xbf``; a tid's is the part's raw normalised bytes; the others are bodies prefixed by
their own record code. The claim is about the construction, and the construction is four lines
of code — which is exactly the kind of thing that is read as obviously right and implemented
wrong once.

What is verified against the reference
--------------------------------------

``fixtures/library/wire/ids.json`` carries, for each identity, the preimage and the body bytes
*apart* from the digest — for the reason ``fixtures/wire/uid/cases.json`` does: a disagreement
then says whether the encoding or the hashing was wrong rather than only that something was.
And ``fixtures/library/wire/records.cbor`` carries the records themselves, so the identities can
be re-derived from the **decoded and re-encoded** body rather than from a hex string a fixture
handed over, which is the only version of the test that exercises canonical encoding.

What is still not here
----------------------

Readers. No implementation but the reference one reads USFM or OSIS, deliberately: the exit
criterion asks these three to recompute identities from the records the reference emits, not to
re-derive the records. A second implementation of a file format would be testing two parsers
against each other with nothing to appeal to.
"""

from __future__ import annotations

import unicodedata
from dataclasses import dataclass, field
from typing import Any, Optional

from .blake3 import blake3
from .cbor import CborError, encode_one

#: §2.6. The domain byte of each identity **is the record type code it names**, which is what
#: makes the table self-explaining rather than four magic numbers.
TID_DOMAIN = 0x0F
MID_DOMAIN = 0x0E
DID_DOMAIN = 0x11
RDID_DOMAIN = 0x12

#: §2.6's text prefixes.
TID_PREFIX = "t3:"
MID_PREFIX = "m3:"
DID_PREFIX = "d3:"
RDID_PREFIX = "r3:"

#: §3.1, the manifest body's keys.
MANIFEST_KEYS = {
    0: "alias",
    1: "parts",
    2: "lang",
    3: "reader",
    4: "licence",
    5: "carry",
    6: "title",
    7: "creators",
    8: "published",
    9: "identifiers",
    10: "origin",
    11: "parent",
    12: "parent-kind",
    13: "supersedes",
    14: "versification",
    15: "lossy",
    16: "raw",
    17: "part-policy",
    18: "calendar",
}

#: The five the spec marks required, plus the pairing rule for 11/12.
MANIFEST_REQUIRED = frozenset({0, 1, 2, 3, 4, 5, 17})

#: §3.1, a part entry inside key 1.
PART_ENTRY_KEYS = {0: "tid", 1: "length", 2: "structure", 3: "rdid", 4: "lang"}

#: §3.1, part text (15).
PART_TEXT_KEYS = {
    0: "tid",
    1: "text",
}

#: §3.1, part reading (18).
PART_READING_KEYS = {
    0: "tid",
    1: "reader",
    2: "segments",
    3: "raw",
}

#: §3.1, a segment row inside a reading's key 2.
SEGMENT_KEYS = {
    0: "start",
    1: "end",
    2: "level",
    3: "locator",
    4: "lang",
    5: "speaker",
    6: "observed",
    7: "ids",
    8: "tz_offset",
}

#: §3.1, key 5 of a manifest.
CARRY = {0: "none", 1: "ref", 2: "text"}


class LibraryError(Exception):
    """A library record that cannot mean what it says."""


# -- identities ----------------------------------------------------------------------------


def _digest(domain: int, preimage: bytes) -> bytes:
    return blake3(bytes([domain]) + preimage)


def tid(normalised: bytes) -> bytes:
    """§2.6. BLAKE3-256 over ``0x0f`` and the part's **normalised** bytes.

    The caller's bytes are hashed as given. :func:`is_normalised` is the separate question, and
    separate on purpose: a decoder must be able to compute the tid a record *claims* in order to
    report that it is wrong, which it cannot do if computing it requires the bytes to be right.
    """
    return _digest(TID_DOMAIN, normalised)


def mid(manifest_body: bytes) -> bytes:
    """§2.6. BLAKE3-256 over ``0x0e`` and the canonical CBOR of the manifest body."""
    return _digest(MID_DOMAIN, manifest_body)


def did(dating_body: bytes) -> bytes:
    """§2.6. BLAKE3-256 over ``0x11`` and the canonical CBOR of the dating body.

    Record 17 lands in a later release and this implementation decodes no dating body. The
    derivation is here anyway, because it is the same four lines as the other three and the
    domain-separation claim is about all four together: a `did` that collided with a `mid`
    would be a defect in the table, and a table with a hole in it cannot be checked.
    """
    return _digest(DID_DOMAIN, dating_body)


def rdid(reading_body: bytes) -> bytes:
    """§2.6. BLAKE3-256 over ``0x12`` and the canonical CBOR of the reading body."""
    return _digest(RDID_DOMAIN, reading_body)


def structure_hash(segments: bytes) -> bytes:
    """§3.1. BLAKE3-256 of a reading's segment table — **with no domain byte**.

    It names a table rather than a record, so there is no record code to prefix it with. That
    is also what lets a reading gain raw metadata, keep its structure hash and change its rdid,
    which is why a part entry carries both.
    """
    return blake3(segments)


#: §2.1's alphabet, bit order and lengths, as ``uid.py`` has them: RFC 4648 base32, lowercase,
#: no padding, most significant bit first.
_ALPHABET = "abcdefghijklmnopqrstuvwxyz234567"


def text_form(prefix: str, digest: bytes, chars: int = 52) -> str:
    """§2.6: the prefix and 52 base32 characters. 26 is a display abbreviation.

    A record carrying the 26-character form is ``SMY-E071``, as it is for a uid, so an
    implementation that can print one needs to know which length is canonical.
    """
    if len(digest) != 32:
        raise LibraryError(f"an identity is 32 bytes; got {len(digest)}")
    out = prefix
    for i in range(chars):
        v = 0
        for k in range(5):
            bit = i * 5 + k
            on = bit < 256 and (digest[bit >> 3] >> (7 - (bit & 7))) & 1
            v = (v << 1) | (1 if on else 0)
        out += _ALPHABET[v]
    return out


# -- normalisation -------------------------------------------------------------------------


def is_normalised(data: bytes) -> bool:
    """§2.6's normalised bytes: UTF-8, NFC, LF line endings, no byte order mark.

    Nothing else: no whitespace collapsing and no case folding. A part that is not normalised
    is ``SMY-E446`` on the way in, because two libraries given the same text would otherwise
    name two different parts.
    """
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        return False
    if text.startswith("﻿"):
        return False
    if "\r" in text:
        return False
    return unicodedata.normalize("NFC", text) == text


# -- records -------------------------------------------------------------------------------


@dataclass
class PartEntry:
    """One row of a manifest's key 1."""

    tid: bytes
    length: int
    structure: bytes
    rdid: bytes
    lang: Optional[str] = None
    extra: dict[int, Any] = field(default_factory=dict)

    @classmethod
    def decode(cls, body: Any) -> "PartEntry":
        if not isinstance(body, dict):
            raise LibraryError("a part entry is a map")
        for required in (0, 1, 2, 3):
            if required not in body:
                raise LibraryError(
                    f"a part entry needs key {required} ({PART_ENTRY_KEYS[required]})"
                )
        for key, name, size in ((0, "tid", 32), (2, "structure", 32), (3, "rdid", 32)):
            if not isinstance(body[key], bytes) or len(body[key]) != size:
                raise LibraryError(f"a part entry's {name} is {size} bytes")
        if not isinstance(body[1], int) or body[1] < 0:
            raise LibraryError("a part entry's length is an unsigned integer")
        return cls(
            tid=body[0],
            length=body[1],
            structure=body[2],
            rdid=body[3],
            lang=body.get(4),
            extra={k: v for k, v in body.items() if k > 4},
        )


@dataclass
class Manifest:
    """Record 14, decoded. ``body`` is kept because the mid is a hash of its bytes."""

    fields: dict[str, Any]
    parts: list[PartEntry]
    body: Any

    @classmethod
    def decode(cls, body: Any) -> "Manifest":
        if not isinstance(body, dict):
            raise LibraryError("a manifest body is a map")
        missing = sorted(MANIFEST_REQUIRED - set(body))
        if missing:
            names = ", ".join(f"{k} ({MANIFEST_KEYS[k]})" for k in missing)
            raise LibraryError(f"a manifest needs key(s) {names}")
        # Key 12 is required with key 11 and meaningless without it; §3.1 says either alone
        # MUST be rejected, for the reason a resolution with one target is.
        if (11 in body) != (12 in body):
            raise LibraryError("manifest keys 11 (parent) and 12 (parent-kind) travel together")
        # Key 15 has no `false` encoding: an absent key is false, and admitting `false` on the
        # wire would give one manifest two byte strings and therefore two mids.
        if body.get(15) is False:
            raise LibraryError("manifest key 15 (lossy) has no `false` encoding")
        if not isinstance(body[1], list):
            raise LibraryError("a manifest's parts are an array")
        if body[5] not in CARRY:
            raise LibraryError(f"carry {body[5]!r} is not 0, 1 or 2")
        return cls(
            fields={MANIFEST_KEYS.get(k, k): v for k, v in body.items()},
            parts=[PartEntry.decode(p) for p in body[1]],
            body=body,
        )

    @property
    def alias(self) -> str:
        return self.fields["alias"]

    @property
    def carry(self) -> str:
        return CARRY[self.fields["carry"]]

    def mid(self) -> bytes:
        """The identity, derived from the body **re-encoded canonically**.

        Not from the bytes the record arrived in. Hashing those would make this agree with the
        reference implementation while saying nothing about whether this implementation can lay
        a manifest out in canonical form — and a producer that cannot do that cannot write one.
        """
        return mid(encode_one(self.body))


@dataclass
class PartText:
    """Record 15."""

    tid: bytes
    text: bytes
    extra: dict[int, Any] = field(default_factory=dict)

    @classmethod
    def decode(cls, body: Any) -> "PartText":
        if not isinstance(body, dict):
            raise LibraryError("a part text body is a map")
        if 0 not in body or 1 not in body:
            raise LibraryError("a part text needs keys 0 (tid) and 1 (text)")
        if not isinstance(body[0], bytes) or len(body[0]) != 32:
            raise LibraryError("a part text's tid is 32 bytes")
        if not isinstance(body[1], bytes):
            raise LibraryError("a part text's text is a byte string")
        return cls(
            tid=body[0],
            text=body[1],
            extra={k: v for k, v in body.items() if k > 1},
        )

    def verify(self) -> bool:
        """Whether the bytes hash to the claimed tid **and** are normalised (``SMY-E446``).

        Both, because either alone leaves a hole: unnormalised bytes that hash to their own tid
        are a part no other library would name the same way, and normalised bytes under the
        wrong tid are a substituted file.

        A decode never fails on this. §2.6 is explicit: a record that cannot be decoded cannot
        be reported, and one bad part would otherwise stop a whole store from opening.
        """
        return tid(self.text) == self.tid and is_normalised(self.text)


@dataclass
class PartReading:
    """Record 18."""

    tid: bytes
    reader: str
    segments: Any
    raw: Optional[Any] = None
    extra: dict[int, Any] = field(default_factory=dict)
    body: Any = None

    @classmethod
    def decode(cls, body: Any) -> "PartReading":
        if not isinstance(body, dict):
            raise LibraryError("a reading body is a map")
        for required in (0, 1, 2):
            if required not in body:
                raise LibraryError(
                    f"a reading needs key {required} ({PART_READING_KEYS[required]})"
                )
        if not isinstance(body[0], bytes) or len(body[0]) != 32:
            raise LibraryError("a reading's tid is 32 bytes")
        if not isinstance(body[1], str):
            raise LibraryError("a reading's reader id is text")
        return cls(
            tid=body[0],
            reader=body[1],
            segments=body[2],
            raw=body.get(3),
            extra={k: v for k, v in body.items() if k > 3},
            body=body,
        )

    def rdid(self) -> bytes:
        """The identity, over the canonically re-encoded body."""
        return rdid(encode_one(self.body))

    def structure_hash(self) -> bytes:
        """§3.1: BLAKE3-256 of the canonical CBOR of the segment table, with no domain byte."""
        return structure_hash(encode_one(self.segments))

    def rows(self) -> list[dict[str, Any]]:
        """The segment table with its keys named. Unknown keys keep their integer (rule X)."""
        if not isinstance(self.segments, list):
            raise LibraryError("a segment table is an array")
        out = []
        for row in self.segments:
            if not isinstance(row, dict):
                raise LibraryError("a segment row is a map")
            out.append({SEGMENT_KEYS.get(k, k): v for k, v in row.items()})
        return out

    def verify_entry(self, entry: PartEntry) -> None:
        """``SMY-E401``: a re-read part whose reading no longer matches the manifest's entry.

        The structure hash is checked first, because when a reader is upgraded under a corpus
        both identities move and the structure is the one that says what changed. An rdid-only
        mismatch is narrower — same segmentation, different raw metadata.
        """
        if self.tid != entry.tid:
            raise LibraryError("this reading is not of that part")
        if self.structure_hash() != entry.structure:
            raise LibraryError("SMY-E401: the structure hash does not match the part entry")
        if self.rdid() != entry.rdid:
            raise LibraryError("SMY-E401: the rdid does not match the part entry")


def decode_library_record(code: int, body: Any):
    """Decode a record 14, 15 or 18 body. Anything else is not this module's business."""
    if code == 14:
        return Manifest.decode(body)
    if code == 15:
        return PartText.decode(body)
    if code == 18:
        return PartReading.decode(body)
    raise CborError(f"record {code} is not a library record this module decodes")
