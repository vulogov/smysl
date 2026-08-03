"""The shared rejection corpus.

`test_conformance.py` checks that this implementation and the Rust agree on *accepting* four
documents. That is the weaker half of the claim. §3 exists so that one value has exactly one
encoding, and it enforces that entirely by refusing the alternatives — so agreement about
what is *not* a smysl document is the half that carries the property.

Before this corpus existed each implementation invented its own invalid inputs and no two
used the same bytes, so nothing would have noticed if Python accepted something Go rejected.
Building it found exactly that: the Rust walker accepted seven of these twenty-eight, and one
consequence was that a non-canonical extension payload could reach a uid.

The manifest records the §3 constraint each case violates rather than an error message.
Independent implementations word their errors differently and should; agreeing on the reason
is the meaningful claim.
"""

from __future__ import annotations

import json
import struct
from pathlib import Path

import pytest

from smysl.cbor import CborError, decode_one

CORPUS = Path(__file__).parents[2] / "fixtures" / "wire" / "invalid"
MANIFEST = json.loads((CORPUS / "manifest.json").read_text())
CASES = MANIFEST["cases"]


def rejects(data: bytes) -> bool:
    """Refusing, or consuming less than the whole file, both count.

    A decoder that read the first valid-looking item and ignored a trailing violation would
    also be wrong, so a short read is not a pass.
    """
    try:
        _, used = decode_one(data)
    except CborError:
        return True
    except (ValueError, UnicodeDecodeError, struct.error, RecursionError):
        return True
    return used != len(data)


def test_the_corpus_is_present():
    """Guards every other assertion here.

    A manifest that had gone missing, or a glob that matched nothing, would make the
    parametrised test below vacuously true — which is the failure mode this project keeps
    finding in its own suite.
    """
    assert len(CASES) >= 28
    for c in CASES:
        assert (CORPUS / c["file"]).exists(), f"manifest names a missing file: {c['file']}"


@pytest.mark.parametrize("case", CASES, ids=lambda c: c["file"])
def test_every_invalid_fixture_is_rejected(case):
    data = (CORPUS / case["file"]).read_bytes()
    assert rejects(data), (
        f"{case['file']} is not a smysl document (§3 constraint {case['constraint']}: "
        f"{case['why']}) and was accepted"
    )


# The control. If the decoder refused everything, the test above would pass while meaning
# nothing — so the canonical counterparts of the same shapes must still be accepted.
CANONICAL = [
    ("shortest integer", bytes([0x01])),
    ("definite array", bytes([0x81, 0x01])),
    ("sorted map", bytes([0xA2, 0x00, 0x01, 0x01, 0x02])),
    ("composed text", bytes([0x62]) + "é".encode()),
    ("quantised float", bytes([0xFA]) + struct.pack(">f", 0.5)),
]


@pytest.mark.parametrize("what,data", CANONICAL, ids=[c[0] for c in CANONICAL])
def test_the_canonical_counterparts_are_accepted(what, data):
    assert not rejects(data), f"{what} is valid and was rejected"
