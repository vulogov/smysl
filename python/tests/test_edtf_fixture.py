"""``fixtures/library/edtf/cases.json`` is well-formed JSON and says what it claims.

A port reads it, and that is the point of this file rather than a side effect. The generator in
``crates/smysl-core/tests/edtf_fixture.rs`` writes the fixture out of the Rust parser's own
tables and compares it against the committed bytes, so it catches drift — and it caught none when
the file it wrote was **invalid JSON**, because nothing parsed it. Three gates passed over a
conformance fixture no implementation could load.

This module does not parse EDTF. Python has no EDTF parser here and does not need one for C-Read:
``published`` is text on the wire and a decoder preserves it byte for byte. What the fixture is
*for* on this side is the two properties below — that it loads, and that it is internally
consistent — plus being readable at all by whoever implements C-Produce next.
"""

from __future__ import annotations

import json
from pathlib import Path

CASES = json.loads(
    (Path(__file__).resolve().parents[2] / "fixtures" / "library" / "edtf" / "cases.json")
    .read_text(encoding="utf-8")
)


def test_the_fixture_has_both_halves():
    # A fixture of accepted forms alone would let an implementation pass while accepting every
    # second spelling of them, which is the failure the refused half exists to prevent.
    assert len(CASES["accepted"]) >= 30
    assert len(CASES["refused"]) >= 30
    assert CASES["purpose"] and CASES["diagnostic"]


def test_every_case_carries_what_a_reader_needs():
    for case in CASES["accepted"]:
        assert case["text"], "an accepted case with no text"
        assert case["level"] in (0, 1), case
        assert case["kind"] in ("date", "datetime", "interval"), case
        assert case["note"], case
    for case in CASES["refused"]:
        # The empty string is itself a refused value, so `text` may be empty here and the
        # reason may not: a refusal with no reason is a rule nobody can check.
        assert "text" in case and case["why"], case


def test_the_two_halves_are_disjoint_and_free_of_duplicates():
    accepted = [c["text"] for c in CASES["accepted"]]
    refused = [c["text"] for c in CASES["refused"]]
    assert len(set(accepted)) == len(accepted)
    assert len(set(refused)) == len(refused)
    assert not set(accepted) & set(refused)
