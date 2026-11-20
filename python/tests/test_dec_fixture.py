"""``fixtures/tel/dec/cases.json``: ``dec`` (RFC SMYSL-3 §5), C-Tel-Read.

The fixture is written by a JavaScript generator. Its binary64 cases are what an engine's own
``String(x)`` gives, which is the ``Number::toString`` §5.3 names, and its token, spelling and
CBOR cases are the generator's own reading of the RFC. Rust reads the same file, so this is
the third reading checked against the same decisions.
"""

from __future__ import annotations

import json
import struct
from pathlib import Path

import pytest

from smysl import dec

CASES = json.loads(
    (Path(__file__).resolve().parents[2] / "fixtures" / "tel" / "dec" / "cases.json")
    .read_text(encoding="utf-8")
)


def test_a_binary64_is_the_decimal_ecma_262_selects():
    assert len(CASES["f64"]) > 2000
    for case in CASES["f64"]:
        (x,) = struct.unpack(">d", bytes.fromhex(case["bits"]))
        assert dec.from_f64(x) == (int(case["m"]), case["e"]), case


def test_tokens_are_read_as_the_rfc_says():
    for case in CASES["tokens"]:
        if "error" in case:
            with pytest.raises(dec.DecError) as err:
                dec.parse_token(case["token"])
            assert err.value.kind == case["error"], case
        else:
            assert dec.parse_token(case["token"]) == (int(case["m"]), case["e"]), case


def test_one_spelling_and_one_encoding_per_value():
    for case in CASES["spellings"]:
        v = (int(case["m"]), case["e"])
        assert dec.spell(*v) == case["text"]
        assert dec.parse_token(case["text"]) == v
        assert dec.to_cbor(*v).hex() == case["cbor"]
        assert dec.from_cbor(bytes.fromhex(case["cbor"])) == v


def test_non_canonical_cbor_is_e602():
    for case in CASES["refused"]:
        with pytest.raises(dec.DecError) as err:
            dec.from_cbor(bytes.fromhex(case["cbor"]))
        assert err.value.kind == "E602", case["why"]
