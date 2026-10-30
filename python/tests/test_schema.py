"""FC-9 decoding and L9 against the fixtures the Rust wrote (SMYSL-2.5 §5.8, TX-P12 step 1).

``fixtures/schema/fc9/cases.json`` names, per file and declaration, the field count or
``opaque``, each step's op tags and bytes, and the codes decoding reports; and the lens-chain
hashes of the series. Lens application is not a conformance obligation, so nothing here applies
a lens.
"""

import json
from pathlib import Path

import pytest

import smysl
from smysl.schema import chain_hash, decode, encode_lenses
from smysl.cbor import encode_one

FC9 = Path(__file__).parents[2] / "fixtures" / "schema" / "fc9"
CASES = json.loads((FC9 / "cases.json").read_text())


def decls(name):
    records = smysl.decode_store((FC9 / name).read_bytes())
    assert all(r.code == 8 for r in records)
    return records


def test_there_are_cases():
    assert CASES["files"] and CASES["chains"], "no FC-9 cases; the suite would pass vacuously"


@pytest.mark.parametrize("case", CASES["files"], ids=lambda c: c["file"])
def test_each_file_decodes_as_the_rust_says_and_writes_back(case):
    data = (FC9 / case["file"]).read_bytes()
    records = decls(case["file"])
    assert smysl.encode_store(records) == data
    assert len(records) == len(case["decls"])
    for r, want in zip(records, case["decls"]):
        assert r.body[0] == want["id"] and r.body[1] == want["version"]
        got = decode(r.body)
        assert got["fields"] == want["fields"]
        assert got["lenses_opaque"] == want["lenses_opaque"]
        assert got["codes"] == want["codes"]
        assert [s["ops"] for s in got["steps"]] == [s["ops"] for s in want["steps"]]
        assert [(s["from"], s["to"]) for s in got["steps"]] == [
            (s["from"], s["to"]) for s in want["steps"]
        ]
        assert [s["bytes"].hex() for s in got["steps"]] == [s["bytes"] for s in want["steps"]]
        # L9: key 5 written back from its steps is what arrived.
        if 5 in r.body and not got["lenses_opaque"]:
            assert encode_lenses(got["steps"]) == encode_one(r.body[5])


@pytest.mark.parametrize("chain", CASES["chains"], ids=lambda c: f"{c['from']}-{c['to']}")
def test_chain_hashes_match_the_rust(chain):
    steps = [s for r in decls("series.cbor") for s in decode(r.body)["steps"]]
    path, at = [], chain["from"]
    while at < chain["to"]:
        step = next(s for s in steps if s["from"] == at)
        path.append(step)
        at = step["to"]
    assert at == chain["to"]
    assert chain_hash(chain["id"], chain["from"], chain["to"], path).hex() == chain["hash"]
