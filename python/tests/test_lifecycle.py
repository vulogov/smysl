"""1.4: relation identity and contention identity, against the vectors the Rust produced.

A withdrawal names an edge by its rid, and a resolution names a contention by its id. An
implementation that derived either differently would read every other implementation's
withdrawals and resolutions as naming nothing — so each is checked here against
``fixtures/wire/relation-id`` and ``fixtures/wire/contention-id``, preimage and digest apart.
"""

from __future__ import annotations

import base64
import json
from pathlib import Path

import pytest

import smysl
from smysl.uid import contention_digest, relation_id

WIRE = Path(__file__).parents[2] / "fixtures" / "wire"
RELATIONS = json.loads((WIRE / "relation-id" / "cases.json").read_text())["cases"]
CONTENTIONS = json.loads((WIRE / "contention-id" / "cases.json").read_text())["cases"]


def test_there_are_vectors():
    assert RELATIONS and CONTENTIONS


@pytest.mark.parametrize("case", RELATIONS, ids=lambda c: c["name"])
def test_relation_ids_match(case):
    frm, to = bytes.fromhex(case["from_hex"]), bytes.fromhex(case["to_hex"])
    pre = b"\x03" + case["kind"].encode() + b"\x00" + frm + to
    assert pre.hex() == case["preimage_hex"], "the preimage differs"
    assert relation_id(case["kind"], frm, to).hex() == case["rid_hex"]


@pytest.mark.parametrize("case", CONTENTIONS, ids=lambda c: c["name"])
def test_contention_ids_match(case):
    positions = [bytes.fromhex(p) for p in case["positions_hex"]]
    digest = contention_digest(case["kind"], bytes.fromhex(case["over_hex"]), positions)
    assert digest.hex() == case["digest_hex"]
    # §2.1's base32: RFC 4648, lowercased, unpadded, first 130 bits as 26 characters.
    text = base64.b32encode(digest).decode().lower().rstrip("=")[:26]
    assert "k/c" + text == case["id"]


def test_records_11_and_12_are_known_and_round_trip():
    data = (WIRE / "F10-lifecycle.cbor").read_bytes()
    records = smysl.decode_store(data)
    names = [r.name for r in records]
    assert "withdrawal" in names and "resolution" in names
    assert all(r.is_known for r in records)
    assert smysl.encode_store(records) == data


def test_a_pack_manifests_new_key_survives_a_round_trip():
    """§8.1's test of "permitted", for `packinfo` key 7 (1.6).

    This implementation does not decode a pack manifest's body, which is the point: what it has to
    prove is that a key it has never heard of comes back out exactly as it went in. The fixture
    holds a manifest that reserved nothing — the key absent, as every pack before 1.6 encoded it —
    and two that reserved something.
    """
    data = (WIRE / "F12-reserved-pack.cbor").read_bytes()
    records = smysl.decode_store(data)
    assert [r.name for r in records] == ["pack_info"] * 3
    assert all(r.is_known for r in records)
    assert smysl.encode_store(records) == data
