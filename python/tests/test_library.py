"""The library records and the four identities — §2.6 and §3.1, added in 1.10.

Reading a document never requires deriving an identity, which is why this file exists at all:
three independent readers round-tripped every library fixture byte for byte while knowing
nothing about what a tid *is*. §2.6's claim — that the one-byte domain prefix keeps four kinds
of identity apart from each other and from a uid — had been checked by exactly one
implementation, the one that wrote it down.

Two levels of test, and the second is the one that matters. Deriving an identity from the hex
preimage the fixture hands over checks the hash. Deriving it from the record *decoded and
re-encoded* checks the canonical encoder as well, which is the half a producer needs and the
half a fixture of hex strings cannot reach.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest

import smysl

WIRE = Path(__file__).parents[2] / "fixtures" / "library" / "wire"
IDS = json.loads((WIRE / "ids.json").read_text())
RECORDS = (WIRE / "records.cbor").read_bytes()


def test_there_are_vectors_to_check():
    """A suite with no fixtures passes vacuously, which is the failure this catches."""
    assert IDS["manifest"]["mid_hex"]
    assert len(IDS["parts"]) >= 2, "one part would not exercise a second reading"
    assert RECORDS


def test_the_domain_bytes_are_the_record_codes_they_name():
    """§2.6: "The domain byte is the record type code the identity names."

    Not an arbitrary table, and worth asserting rather than transcribing: a tid prefixed with
    the manifest's byte would still produce stable, agreeing identities in any one
    implementation and would silently be a different format.
    """
    assert smysl.TID_DOMAIN == 15
    assert smysl.MID_DOMAIN == 14
    assert smysl.DID_DOMAIN == 17
    assert smysl.RDID_DOMAIN == 18
    assert IDS["domain_bytes"] == {
        "tid": smysl.TID_DOMAIN,
        "mid": smysl.MID_DOMAIN,
        "did": smysl.DID_DOMAIN,
        "rdid": smysl.RDID_DOMAIN,
    }


def test_the_identities_come_out_of_their_preimages():
    """The hash, checked against the reference's vectors."""
    manifest_body = bytes.fromhex(IDS["manifest"]["body_hex"])
    assert smysl.mid(manifest_body).hex() == IDS["manifest"]["mid_hex"]

    for part in IDS["parts"]:
        text = bytes.fromhex(part["text_hex"])
        assert smysl.tid(text).hex() == part["tid_hex"]
        assert len(text) == part["length"]
        assert smysl.rdid(bytes.fromhex(part["reading_body_hex"])).hex() == part["rdid_hex"]


def test_no_two_kinds_of_identity_can_collide():
    """§2.6's claim, exercised on a preimage shared by all four.

    The guarantee is not probabilistic and is not about BLAKE3: the four digests differ because
    their *inputs* differ in the first byte, whatever the rest of the preimage is. A
    construction that hashed the preimage alone and labelled the result afterwards would pass
    every other test in this file.
    """
    shared = b"the same bytes under four names"
    digests = {
        "tid": smysl.tid(shared),
        "mid": smysl.mid(shared),
        "did": smysl.did(shared),
        "rdid": smysl.rdid(shared),
    }
    assert len(set(digests.values())) == 4, digests

    # And none of them is the uid of anything: a uid's preimage is a canonical CBOR map, whose
    # first byte is 0xa0-0xbf, and a domain byte is 0x0e-0x12. The two ranges cannot overlap.
    for domain in (smysl.TID_DOMAIN, smysl.MID_DOMAIN, smysl.DID_DOMAIN, smysl.RDID_DOMAIN):
        assert domain < 0xA0

    # The structure hash is the one digest with **no** domain byte, because it names a table
    # rather than a record. So it is free to equal nothing in particular — but it must not be
    # computed as if it had one.
    assert smysl.structure_hash(shared) != smysl.tid(shared)
    assert smysl.structure_hash(shared) == smysl.blake3(shared)


def test_the_text_form_is_the_prefix_and_fifty_two_characters():
    """§2.6 and §2.1: 52 is canonical, 26 is a display abbreviation."""
    digest = bytes.fromhex(IDS["manifest"]["mid_hex"])
    canonical = smysl.text_form(smysl.MID_PREFIX, digest)
    assert canonical.startswith("m3:")
    assert len(canonical) == 3 + 52
    short = smysl.text_form(smysl.MID_PREFIX, digest, 26)
    assert len(short) == 3 + 26
    assert canonical.startswith(short), "the short form is a prefix of the canonical one"
    for prefix, fn in (
        (smysl.TID_PREFIX, smysl.tid),
        (smysl.DID_PREFIX, smysl.did),
        (smysl.RDID_PREFIX, smysl.rdid),
    ):
        assert smysl.text_form(prefix, fn(b"x")).startswith(prefix)
    with pytest.raises(smysl.LibraryError):
        smysl.text_form(smysl.TID_PREFIX, b"too short")


# -- the records themselves ----------------------------------------------------------------


def _records():
    return smysl.decode_store(RECORDS)


def test_the_record_fixture_round_trips_and_names_every_library_code():
    """C-Read, unchanged: the decode is new, the byte-for-byte promise is not."""
    records = _records()
    assert smysl.encode_store(records) == RECORDS
    codes = [r.code for r in records]
    for code in (14, 15, 18, 19):
        assert code in codes, f"record {code} is not in the fixture"
    # The fixture also carries codes this implementation still does not understand, which is
    # what keeps the distinction in `UNDERSTOOD_RECORDS` an observed fact rather than a claim.
    assert {16, 17} & set(codes)
    for r in records:
        assert r.is_known == (r.code in smysl.UNDERSTOOD_RECORDS)


def test_a_manifest_decodes_into_its_named_fields():
    manifest = next(
        smysl.Manifest.decode(r.body) for r in _records() if r.code == 14
    )
    assert manifest.alias == IDS["manifest"]["alias"]
    assert manifest.carry == "text"
    assert manifest.fields["reader"] == "osis/1"
    assert manifest.fields["licence"] == "public-domain"
    assert manifest.fields["part-policy"]
    assert len(manifest.parts) == len(IDS["parts"])
    for entry, expected in zip(manifest.parts, IDS["parts"]):
        assert entry.tid.hex() == expected["tid_hex"]
        assert entry.length == expected["length"]
        assert entry.structure.hex() == expected["structure_hex"]
        assert entry.rdid.hex() == expected["rdid_hex"]


def test_a_redaction_decodes_into_its_named_fields():
    """Record 19, rule Z (1.10).

    The part it names is **not** in the fixture, which is the state honouring a redaction leaves
    behind: the record remains and the bytes are gone. A reader that expected the two to travel
    together would have nothing to decode here.
    """
    expected = IDS["redaction"]
    redaction = next(
        smysl.Redaction.decode(r.body) for r in _records() if r.code == 19
    )
    assert redaction.tid.hex() == expected["tid_hex"]
    assert redaction.agent == expected["agent"]
    assert redaction.reason is None
    # The body re-encodes to the bytes the fixture carries, which is the half a hash would
    # otherwise be hiding: a redaction has no identity of its own, so the encoding is the only
    # thing two implementations can disagree about.
    assert smysl.encode_one(redaction.body).hex() == expected["body_hex"]
    # And the part it names is nowhere in the fixture.
    tids = {p["tid_hex"] for p in IDS["parts"]}
    assert expected["tid_hex"] not in tids


def test_a_dating_decodes_into_its_named_fields():
    """Record 17 (TX-P3 step 1): a statement about when something happened.

    The three in the fixture are one per target kind that has an identity — a manifest, a part,
    a window over a part — and one per value kind: an absolute EDTF year, a **negative** offset
    and an Allen relation. The negative one is not decoration. A dating's offset is the first
    signed integer any record in this format carries, so until it existed this decoder had never
    read CBOR major type 1 out of a smysl store, and a decoder that read it as a very large
    positive number would have agreed with the fixture about every byte and disagreed about what
    they mean.
    """
    datings = [smysl.Dating.decode(r.body) for r in _records() if r.code == 17]
    assert len(datings) == len(IDS["datings"])
    kinds, values = set(), set()
    for dating, expected in zip(datings, IDS["datings"]):
        body = smysl.encode_one(dating.body)
        assert body.hex() == expected["body_hex"]
        # The did over the body this implementation re-encodes, not over bytes the fixture
        # supplied: the same claim the mid and the rdid tests make, for the fourth identity.
        assert dating.dating_id().hex() == expected["did_hex"]
        assert dating.axis_name == expected["axis"]
        kinds.add(dating.target.kind_name)
        values.add(dating.value[0])
    assert kinds == {"manifest", "part", "window"}
    assert values == {"absolute", "offset", "relative"}
    offset = next(d for d in datings if d.value[0] == "offset")
    assert offset.value[1] == -10800000
    assert offset.target.window is not None and offset.target.window[0] < offset.target.window[1]
    relative = next(d for d in datings if d.value[0] == "relative")
    assert relative.value[1][0] == "after"


def test_a_dating_refuses_a_value_that_says_two_things():
    """A dating's value is a one-entry map, and the count is what makes it one claim.

    Hand-built rather than taken from the fixture, because a conforming producer cannot write
    this: it is the record a *non*-conforming one would write, and the point of checking is that
    a reader meeting it says so instead of silently taking whichever entry came first.
    """
    good = next(r.body for r in _records() if r.code == 17)
    two = dict(good)
    two[2] = {0: "1611", 1: 0}
    with pytest.raises(smysl.LibraryError):
        smysl.Dating.decode(two)
    no_axis = {k: v for k, v in good.items() if k != 1}
    with pytest.raises(smysl.LibraryError):
        smysl.Dating.decode(no_axis)
    bad_axis = dict(good)
    bad_axis[1] = 3
    with pytest.raises(smysl.LibraryError):
        smysl.Dating.decode(bad_axis)


def test_the_mid_is_derived_from_the_body_this_implementation_re_encodes():
    """**The test the exit criterion is about.**

    `test_the_identities_come_out_of_their_preimages` hashes bytes the fixture supplied. This
    decodes the record, lays the body out canonically with this implementation's own encoder,
    and hashes *that*. A disagreement here is a disagreement about canonical CBOR — map key
    order, shortest-form heads, an omitted optional — reported as an identity mismatch, which
    is exactly how it would be reported in the field.
    """
    for r in _records():
        if r.code == 14:
            assert smysl.Manifest.decode(r.body).mid().hex() == IDS["manifest"]["mid_hex"]


def test_a_part_text_verifies_against_its_tid_and_its_normalisation():
    parts = [smysl.PartText.decode(r.body) for r in _records() if r.code == 15]
    assert len(parts) == len(IDS["parts"])
    for part, expected in zip(parts, IDS["parts"]):
        assert part.tid.hex() == expected["tid_hex"]
        assert part.verify(), "a fixture part must verify"
        assert smysl.is_normalised(part.text)
    # One of them is Cyrillic, which is what makes the NFC half of the check more than a
    # formality: a reader that normalised on the way in or out would move the tid.
    assert any(not part.text.isascii() for part in parts)


def test_a_reading_reproduces_its_rdid_and_its_structure_hash():
    readings = [smysl.PartReading.decode(r.body) for r in _records() if r.code == 18]
    assert len(readings) == len(IDS["parts"])
    for reading, expected in zip(readings, IDS["parts"]):
        assert reading.tid.hex() == expected["tid_hex"]
        assert reading.rdid().hex() == expected["rdid_hex"]
        assert reading.structure_hash().hex() == expected["structure_hex"]
        assert reading.reader == "osis/1"
        rows = reading.rows()
        assert rows and all("start" in row and "locator" in row for row in rows)


def test_a_reading_checks_itself_against_the_manifest_entry():
    """``SMY-E401``, which is the pair of identities a part entry carries both of.

    The structure hash and the rdid move together when a reader changes its segmentation and
    separately when it changes only its raw metadata, which is why the entry records both. A
    tampered entry is the only way to reach this from a fixture that is correct.
    """
    records = _records()
    manifest = next(smysl.Manifest.decode(r.body) for r in records if r.code == 14)
    readings = {r.tid: r for r in (smysl.PartReading.decode(x.body) for x in records if x.code == 18)}
    for entry in manifest.parts:
        readings[entry.tid].verify_entry(entry)

    entry = manifest.parts[0]
    reading = readings[entry.tid]
    tampered = smysl.PartEntry(
        tid=entry.tid,
        length=entry.length,
        structure=bytes(32),
        rdid=entry.rdid,
    )
    with pytest.raises(smysl.LibraryError, match="SMY-E401"):
        reading.verify_entry(tampered)


# -- the rules a decoder has to enforce ----------------------------------------------------


def test_a_part_whose_bytes_do_not_match_its_tid_decodes_and_fails_verification():
    """§2.6: it MUST NOT fail the decode.

    "A record that cannot be decoded cannot be reported, and one bad part would otherwise stop
    a whole store from opening." So the decode succeeds, `verify` is False, and the caller is
    the one that raises ``SMY-E446``.
    """
    liar = smysl.encode_one([15, {0: bytes(32), 1: b"not the bytes that hash to zero\n"}])
    (record,) = smysl.decode_store(liar)
    part = smysl.PartText.decode(record.body)
    assert not part.verify()
    assert record.reencode() == liar, "and it still round-trips"


def test_unnormalised_bytes_fail_verification_even_under_their_own_tid():
    """Both halves of ``SMY-E446``, because either alone leaves a hole.

    Bytes that hash to their own tid but carry CRLF are a part no other library would name the
    same way; the tid agrees and the part is still wrong.
    """
    crlf = b"a line\r\nand another\n"
    assert smysl.tid(crlf) == smysl.tid(crlf)
    part = smysl.PartText(tid=smysl.tid(crlf), text=crlf)
    assert not part.verify()
    assert not smysl.is_normalised(crlf)
    assert not smysl.is_normalised("﻿with a byte order mark\n".encode())
    # Decomposed: e + combining acute, which NFC composes.
    assert not smysl.is_normalised("café\n".encode())
    assert smysl.is_normalised("café\n".encode())


def test_manifest_keys_eleven_and_twelve_travel_together():
    """§3.1: "either alone MUST be rejected, for the reason a resolution with one target is"."""
    base = {0: "a", 1: [], 2: "en", 3: "txt/1", 4: "unknown", 5: 0, 17: "p"}
    smysl.Manifest.decode(base)  # the control
    with pytest.raises(smysl.LibraryError, match="travel together"):
        smysl.Manifest.decode({**base, 11: bytes(32)})
    with pytest.raises(smysl.LibraryError, match="travel together"):
        smysl.Manifest.decode({**base, 12: "translation"})
    smysl.Manifest.decode({**base, 11: bytes(32), 12: "translation"})


def test_lossy_false_is_refused_because_it_would_give_one_manifest_two_mids():
    """§3.1: key 15 has no ``false`` encoding.

    Admitting it would mean two byte strings for one manifest, and therefore two mids for one
    expression — the failure content addressing cannot survive.
    """
    base = {0: "a", 1: [], 2: "en", 3: "txt/1", 4: "unknown", 5: 0, 17: "p"}
    with pytest.raises(smysl.LibraryError, match="no `false` encoding"):
        smysl.Manifest.decode({**base, 15: False})
    smysl.Manifest.decode({**base, 15: True})
    # And the two differ, which is the point of refusing the third spelling.
    absent = smysl.mid(smysl.encode_one(base))
    present = smysl.mid(smysl.encode_one({**base, 15: True}))
    assert absent != present


def test_a_manifest_missing_a_required_key_is_refused():
    base = {0: "a", 1: [], 2: "en", 3: "txt/1", 4: "unknown", 5: 0, 17: "p"}
    for key in sorted(smysl.MANIFEST_KEYS):
        if key not in base:
            continue
        without = {k: v for k, v in base.items() if k != key}
        with pytest.raises(smysl.LibraryError):
            smysl.Manifest.decode(without)


def test_carry_outside_its_three_values_is_refused():
    base = {0: "a", 1: [], 2: "en", 3: "txt/1", 4: "unknown", 5: 3, 17: "p"}
    with pytest.raises(smysl.LibraryError, match="carry"):
        smysl.Manifest.decode(base)
    assert smysl.CARRY == {0: "none", 1: "ref", 2: "text"}
