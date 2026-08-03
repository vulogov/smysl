"""C-Produce: deriving uids, and the claim that rests on it.

§2.3 — *status is part of identity* — is the paragraph the whole format rests on, and until now
no implementation but the Rust had ever checked it. C-Read does not reach it: reading a document
never requires computing a uid, so three independent readers round-tripped every fixture byte
for byte while remaining ignorant of what a uid is.

Three layers here, because a failure in each means something different:

1. **BLAKE3 against the published vectors.** If the hash is wrong, nothing below is evidence.
   Includes the multi-chunk lengths — a single-chunk shortcut is correct on every small input
   and wrong on the first large one.
2. **Canonical bytes against the Rust's**, separately from the uid. A fixture that agrees on
   the hash but not the layout, or the reverse, says which half to look at.
3. **§2.3 itself**, as a property rather than one example.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

from smysl.blake3 import blake3
from smysl.uid import Source, UnitCore

CASES = json.loads(
    (Path(__file__).parents[2] / "fixtures" / "wire" / "uid" / "cases.json").read_text()
)["cases"]


def build(spec: dict) -> UnitCore:
    k = spec["core"]
    src = None
    if k["source"]:
        src = Source(
            kind=k["source"]["kind"],
            reference=k["source"]["reference"],
            captured=k["source"]["captured"],
        )
    return UnitCore(
        schema=k["schema"],
        gist=k["gist"],
        status=k["status"],
        body=k["body"],
        detail=k["detail"],
        deps=[bytes.fromhex(x) for x in k["deps"]],
        grounds=[bytes.fromhex(x) for x in k["grounds"]],
        source=src,
        payload=bytes.fromhex(k["payload_hex"]) if k["payload_hex"] else None,
    )


# -- 1. the hash --------------------------------------------------------------------------

# The published BLAKE3 vectors. Input of length n is the bytes `i % 251`.
BLAKE3_VECTORS = {
    0: "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
    1: "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
    2: "7b7015bb92cf0b318037702a6cdd81dee41224f734684c2c122cd6359cb1ee63",
    3: "e1be4d7a8ab5560aa4199eea339849ba8e293d55ca0a81006726d184519e647f",
    # The chunk boundary is 1024 bytes; these three straddle it and exercise the tree.
    1023: "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11",
    1024: "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7",
    1025: "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444",
    2048: "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a",
    3072: "b98cb0ff3623be03326b373de6b9095218513e64f1ee2edd2525c7ad1e5cffd2",
}


@pytest.mark.parametrize("n,expected", sorted(BLAKE3_VECTORS.items()))
def test_blake3_matches_the_published_vectors(n, expected):
    assert blake3(bytes(i % 251 for i in range(n))).hex() == expected


# -- 2. the layout and the uid, checked apart ----------------------------------------------


def test_there_are_fixtures():
    """A glob that matched nothing would make every parametrised test below vacuous."""
    assert len(CASES) >= 16


@pytest.mark.parametrize("spec", CASES, ids=lambda s: s["name"])
def test_canonical_bytes_match_the_reference(spec):
    """Checked before the uid, because a hash comparison alone cannot say which half broke."""
    assert build(spec).canonical_bytes().hex() == spec["core_bytes_hex"]


@pytest.mark.parametrize("spec", CASES, ids=lambda s: s["name"])
def test_uid_matches_the_reference(spec):
    assert build(spec).uid().hex() == spec["uid_hex"]


# -- 3. §2.3 --------------------------------------------------------------------------------


def test_status_is_part_of_identity():
    """The witness pair: every field identical, only the status differs.

    `status-cited` and `status-measured` are the clean isolation. The other status cases in
    the fixture differ in more than one field, because `inferred` and `derived` require grounds
    and `speculative` does not — so a pair drawn from those would not isolate the claim.
    """
    by_name = {c["name"]: c for c in CASES}
    a, b = by_name["status-cited"], by_name["status-measured"]

    for key in ("schema", "gist", "body", "detail", "deps", "grounds", "source", "payload_hex"):
        assert a["core"][key] == b["core"][key], f"the pair differs in {key}; it is not isolated"
    assert a["core"]["status"] != b["core"]["status"]

    ua, ub = build(a).uid(), build(b).uid()
    assert ua != ub, "§2.3: a status change must be an identity change"
    assert ua.hex() == a["uid_hex"] and ub.hex() == b["uid_hex"]


def test_every_status_produces_a_distinct_identity():
    """The property, not one example: six statuses over one gist, six distinct uids."""
    statuses = [c for c in CASES if c["name"].startswith("status-") and "pair" not in c["name"]]
    assert len(statuses) >= 5, "the fixture should carry a status per authorable value"
    uids = {build(c).uid() for c in statuses}
    assert len(uids) == len(statuses), "two statuses collided into one identity"


def test_changing_only_the_status_changes_the_uid_for_any_core():
    """Constructed rather than drawn from the fixture, so it holds beyond what was generated."""
    base = UnitCore(schema="claim", gist="a claim that could be held at any strength", status=1)
    seen = {}
    for status in range(1, 6):
        core = UnitCore(schema=base.schema, gist=base.gist, status=status)
        u = core.uid()
        assert u not in seen, f"status {status} collided with status {seen.get(u)}"
        seen[u] = status
    assert len(seen) == 5


# -- controls: the tests above must be capable of failing ----------------------------------


def test_identical_cores_have_identical_uids():
    """Without this, 'a status change changes the uid' would pass if every uid were unique."""
    a = UnitCore(schema="claim", gist="the very same words", status=4)
    b = UnitCore(schema="claim", gist="the very same words", status=4)
    assert a.uid() == b.uid()


def test_normalisation_form_is_not_part_of_identity():
    """The other side of it: text differing only in NFC form is one unit, not two.

    Identity must be sensitive to what was claimed and insensitive to how it was typed.

    Written with explicit escapes, and asserted to differ before being used. A literal `\u00e9`
    and a literal `e` + `\u0301` look identical in a source file, and an editor that normalises
    on save turns this into a test of `x == x`. That has happened in this repository before, in
    the Go suite, and it is invisible in review.
    """
    composed_text = "caf\u00e9 latency"
    decomposed_text = "cafe\u0301 latency"
    assert composed_text != decomposed_text, "the two inputs collapsed; this test is vacuous"

    composed = UnitCore(schema="claim", gist=composed_text, status=1)
    decomposed = UnitCore(schema="claim", gist=decomposed_text, status=1)
    assert composed.uid() == decomposed.uid()


def test_an_absent_optional_and_an_empty_one_are_the_same_unit():
    """§3 constraint 5, from the producing side: an empty set is omitted, not encoded empty.

    Encoding it would give one unit two encodings, and therefore two uids.
    """
    absent = UnitCore(schema="claim", gist="no grounds either way", status=1)
    empty = UnitCore(schema="claim", gist="no grounds either way", status=1, grounds=[])
    assert absent.canonical_bytes() == empty.canonical_bytes()
    assert absent.uid() == empty.uid()


def test_a_different_gist_is_a_different_unit():
    a = UnitCore(schema="claim", gist="one thing", status=1)
    b = UnitCore(schema="claim", gist="another thing", status=1)
    assert a.uid() != b.uid()
