"""The record-set digest of SMYSL-2.7 §5.4, against the vectors the Rust produced.

``fixtures/wire/rsd/cases.json`` names each input and its digest. Order and repeats do not
count, which ``repeats.cbor`` (F13 twice, the first time reversed) holds the implementation to.
"""

import json
from pathlib import Path

import pytest

import smysl
from smysl.export import record_set_digest

RSD = Path(__file__).parents[2] / "fixtures" / "wire" / "rsd"
CASES = json.loads((RSD / "cases.json").read_text())


def test_there_are_vectors():
    assert CASES, "no rsd vectors; the suite would pass vacuously"


@pytest.mark.parametrize("case", CASES, ids=lambda c: c["file"])
def test_record_set_digest_matches_the_rust(case):
    records = smysl.decode_store((RSD / case["file"]).read_bytes())
    assert len(records) == case["records"]
    assert record_set_digest(r.raw for r in records).hex() == case["rsd"]
