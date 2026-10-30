"""Proposition classes — SMYSL-2.3 A-12.4, the Python side of TX-P2 step 5's exit.

A-12.4 is normative because two implementations that disagree about how many classes a store
holds disagree about how many propositions it holds, and that is the number every corpus measure
is divided by. The exit is therefore two implementations agreeing, which needs each to check the
other's work:

  - Rust writes ``fixtures/proposition/store.cbor`` and compares against ``classes.json``.
  - ``scripts/gen-proposition-classes.py`` — an independent reading of A-12.4 over this port's
    decoder — writes ``classes.json`` and compares against the store.
  - This file keeps that reading **live**. Without it the committed JSON would stand in for the
    Python implementation, and the agreement would be Rust agreeing with a file it could have
    written itself.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import importlib.util  # noqa: E402

spec = importlib.util.spec_from_file_location(
    "gen_proposition_classes", ROOT / "scripts" / "gen-proposition-classes.py"
)
reference = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reference)

FIXTURES = ROOT / "fixtures" / "proposition"
STORE = FIXTURES / "store.cbor"
EXPECTED = json.loads((FIXTURES / "classes.json").read_text())


def test_there_is_a_fixture_to_check():
    """A suite with no fixture passes vacuously, which is the failure this catches."""
    assert STORE.is_file()
    assert EXPECTED


def test_the_reference_computes_the_committed_expectation():
    """The committed JSON is what this implementation computes, not a file somebody edited."""
    assert reference.compute(STORE) == EXPECTED


def test_strict_is_a_clique_partition_and_a_path_is_not_one_class():
    """The property that distinguishes ``strict`` from ``component``.

    Every strict class is a clique: each member is adjacent to every other. That is what makes a
    path two classes rather than one, and it is the half of A-12.4 a union-find implementation
    would silently get wrong — it would agree about the triangle and merge the path.
    """
    units, edges, withdrawn, agents = reference.read(STORE)
    adjacency = reference.adjacency(units, edges, withdrawn, agents, 0)
    for members in reference.strict(adjacency):
        for a in members:
            for b in members:
                if a != b:
                    assert b in adjacency[a], "a strict class is a clique"
    sizes = sorted(len(c) for c in EXPECTED["strict"])
    assert sizes == [1, 1, 2, 2, 3], sizes


#: §2.1's base32 alphabet. The digits sort **after** the letters, so ordering a uid's text with
#: Python's `sorted` is not ordering its bytes — which is the mistake the first version of
#: `test_a_class_is_identified_by_its_smallest_member` made, and which
#: `test_the_order_is_over_the_bytes_and_not_over_the_text` below exists to keep anybody from
#: making again.
ALPHABET = "abcdefghijklmnopqrstuvwxyz234567"


def by_bytes(uid: str) -> list[int]:
    """A sort key over a uid's 32 bytes, given its base32 text."""
    return [ALPHABET.index(ch) for ch in uid[3:]]


def test_a_class_is_identified_by_its_smallest_member():
    """A-12.4: the class's id is the unit that opened it, which is its smallest."""
    for members in EXPECTED["strict"]:
        assert members == sorted(members, key=by_bytes), "members are in uid order"
    firsts = [c[0] for c in EXPECTED["strict"]]
    assert firsts == sorted(firsts, key=by_bytes), "classes are listed by their id"
    # And the ordering is not the one `sorted` would give without the key, which is what makes
    # the key worth having rather than decoration.
    assert firsts != sorted(firsts), "this fixture distinguishes the two orders"


def test_the_order_is_over_the_bytes_and_not_over_the_text():
    """§2.1's alphabet is not ASCII-ascending, so the two orders could differ.

    ``abcdefghijklmnopqrstuvwxyz234567``: the digits sort *after* the letters. A reference that
    ordered the base32 text instead of the 32 bytes would be a second, silently different
    procedure — and this fixture has a uid beginning with ``4``, so the two orders are
    distinguishable here rather than in principle.
    """
    flat = [u for c in EXPECTED["strict"] for u in c]
    assert any(u[3] in "234567" for u in flat), "a uid starting past 'z' is in the fixture"
    assert sorted(flat, key=by_bytes) != sorted(flat), "the two orders differ here"
    # The committed file is in byte order, class by class and within each class.
    in_file = [u for c in EXPECTED["strict"] for u in c]
    expected = [
        u
        for c in sorted(EXPECTED["strict"], key=lambda c: by_bytes(c[0]))
        for u in sorted(c, key=by_bytes)
    ]
    assert in_file == expected


def test_component_merges_what_strict_keeps_apart():
    """The two policies answer different questions, and the fixture shows it."""
    assert len(EXPECTED["component"]) == 3
    assert len(EXPECTED["strict"]) == 5
    strict_members = {u for c in EXPECTED["strict"] for u in c}
    component_members = {u for c in EXPECTED["component"] for u in c}
    assert strict_members == component_members, "the same vertex set, partitioned differently"


def test_attested_two_keeps_only_the_doubly_attested_edge():
    """``attested:n`` counts distinct agents, which is why the fixture has an edge with two.

    SMYSL-2.4 §3.6 defaults corpus measures to ``attested:2`` because no S0 proposer reached the
    precision bar: one agent's opinion that two units are one proposition is a proposal.
    """
    assert len(EXPECTED["attested:2"]) == 1
    assert len(EXPECTED["attested:2"][0]) == 2
    kept = set(EXPECTED["attested:2"][0])
    assert any(kept <= set(c) for c in EXPECTED["component"])


def test_a_withdrawn_edge_is_not_an_edge():
    """Removing the withdrawal would merge the triangle and the path into one component."""
    units, edges, withdrawn, agents = reference.read(STORE)
    assert withdrawn, "the fixture holds a withdrawal"
    live = reference.adjacency(units, edges, withdrawn, agents, 0)
    ignored = reference.adjacency(units, edges, set(), agents, 0)
    assert len(reference.components(live)) == 3
    assert len(reference.components(ignored)) == 2, "the withdrawal is load-bearing"


def test_an_edge_to_an_absent_unit_is_not_live():
    """The fixture has one, so "both endpoints present" is tested rather than asserted."""
    units, edges, withdrawn, agents = reference.read(STORE)
    endpoints = {frm for _, frm, _ in edges} | {to for _, _, to in edges}
    assert endpoints - units, "the fixture names a unit it does not hold"
    live = reference.adjacency(units, edges, withdrawn, agents, 0)
    assert all(u in units for u in live), "no absent unit reached the adjacency"


@pytest.mark.parametrize("policy", ["strict", "component", "attested:2"])
def test_no_unit_is_in_two_classes(policy):
    """A partition, under every policy. A unit in two classes would be counted twice."""
    seen = set()
    for members in EXPECTED[policy]:
        for u in members:
            assert u not in seen, f"{u} is in two {policy} classes"
            seen.add(u)
