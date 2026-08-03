"""The library, walked against ``SMYSL_FORMAT_SPEC.md`` clause by clause.

`test_conformance.py` asks whether this implementation agrees with the Rust. This file asks
a different question: whether it does what the *document* says, section by section, with each
test naming the clause it is checking. The two can disagree — agreeing with a reference
implementation and obeying a specification are not the same thing, and the whole reason this
package exists is that only one of them had ever been tested.

Clauses this conformance class cannot reach are listed in `test_what_c_read_cannot_check`,
which fails if that list is ever silently emptied. An untested clause recorded is worth more
than an untested clause forgotten.
"""

from pathlib import Path

import pytest

import smysl
from smysl.cbor import MAX_NESTING

SPEC = Path(__file__).parents[2] / "Documentation" / "SMYSL_FORMAT_SPEC.md"


def test_the_specification_is_where_we_think_it_is():
    assert SPEC.exists(), f"{SPEC} is missing; every test below is about that file"
    assert "Deterministic CBOR" in SPEC.read_text()


# -- §2.2  What is hashed -----------------------------------------------------

def test_unit_core_keys_match_the_table_in_2_2():
    """The spec's table is normative; this implementation's table must be it, exactly."""
    expected = {
        0: "schema",
        1: "gist",
        2: "body",
        3: "detail",
        4: "deps",
        5: "grounds",
        6: "status",
        7: "source",
        8: "payload",
    }
    assert smysl.UNIT_KEYS == expected, "the unit core's key assignment is the format"


def test_keys_at_nine_and_above_are_unknown_and_kept():
    """§2.2 and §5: `>= 9` is an unknown key preserved verbatim under rule X."""
    body = {0: "claim", 1: "a gist", 6: 1, 9: "something a later version added"}
    raw = smysl.encode_one([1, body])
    records = smysl.decode_store(raw)
    assert records[0].reencode() == raw
    named = records[0].unit_fields()
    assert named[9] == "something a later version added", "an unknown key lost its integer"
    assert "schema" in named and "gist" in named


def test_an_absent_optional_is_omitted_not_null():
    """§2.2: 'An absent optional field MUST be omitted, never encoded as `null`.'"""
    with pytest.raises(smysl.CborError):
        smysl.decode_one(b"\xa1\x02\xf6")  # {2: null} — body present as null


# -- §3  Deterministic CBOR, one test per constraint --------------------------

def test_constraint_1_integer_keys_in_the_kernel():
    """Text keys are permitted only inside a payload, so a kernel map keyed by text is
    something this implementation reads but the format does not produce.

    SPEC: §3 constraint 1 says integer keys are used 'in the kernel' without defining a
    decoder's obligation when it meets a text key at kernel level. Read as descriptive
    rather than as a rejection, so this decodes it — recorded rather than assumed.
    """
    value, _ = smysl.decode_one(b"\xa1\x61a\x00")  # {"a": 0}
    assert value == {"a": 0}


@pytest.mark.parametrize(
    "data",
    [b"\x18\x17", b"\x19\x00\xff", b"\x1a\x00\x00\xff\xff", b"\x1b\x00\x00\x00\x00\xff\xff\xff\xff"],
)
def test_constraint_2_shortest_form(data):
    with pytest.raises(smysl.CborError):
        smysl.decode_one(data)


def test_constraint_2_does_not_apply_to_float_payloads():
    """The ambiguity this implementation found. 0x3F800000 is 1.0, not an over-long 1."""
    import struct

    value, _ = smysl.decode_one(b"\xfa" + struct.pack(">f", 1.0))
    assert value == 1.0


@pytest.mark.parametrize("data", [b"\x9f\xff", b"\xbf\xff", b"\x5f\xff", b"\x7f\xff"])
def test_constraint_3_definite_lengths_only(data):
    with pytest.raises(smysl.CborError):
        smysl.decode_one(data)


def test_constraint_4_ascending_key_order():
    smysl.decode_one(b"\xa2\x00\x00\x01\x00")  # {0:0, 1:0} — fine
    with pytest.raises(smysl.CborError):
        smysl.decode_one(b"\xa2\x01\x00\x00\x00")  # descending
    with pytest.raises(smysl.CborError):
        smysl.decode_one(b"\xa2\x00\x00\x00\x00")  # duplicate


def test_constraint_4_orders_by_encoded_bytes_not_by_value():
    """§3: 'sorted by encoded key bytes'. An integer key and a text key are ordered by their
    encodings, which is what makes payload maps well-defined without knowing the key type."""
    m = {0: "a", "z": "b"}
    out = smysl.encode_one(m)
    smysl.decode_one(out)  # must satisfy its own ordering rule


def test_constraint_5_no_null():
    with pytest.raises(smysl.CborError):
        smysl.decode_one(b"\xf6")


def test_constraint_6_nfc_text():
    with pytest.raises(smysl.CborError):
        smysl.decode_one(b"\x63" + "é".encode())  # e + combining acute
    value, _ = smysl.decode_one(b"\x62" + "é".encode())  # composed
    assert value == "é"


def test_constraint_7_binary32_quantised_to_1_1024():
    import struct

    for ok in (0.0, 0.5, 1.0 / 1024, -3.25):
        value, _ = smysl.decode_one(b"\xfa" + struct.pack(">f", ok))
        assert value == pytest.approx(ok)
    for bad in (0.1, 1.0 / 3):
        with pytest.raises(smysl.CborError):
            smysl.decode_one(b"\xfa" + struct.pack(">f", bad))
    with pytest.raises(smysl.CborError):  # binary64
        smysl.decode_one(b"\xfb" + struct.pack(">d", 0.5))
    with pytest.raises(smysl.CborError):  # non-finite
        smysl.decode_one(b"\xfa" + struct.pack(">f", float("inf")))


def test_constraint_8_nesting_bounded_at_128():
    assert MAX_NESTING == 128, "§3 constraint 9 names this number"
    smysl.decode_one(b"\x81" * 100 + b"\x00")
    with pytest.raises(smysl.CborError):
        smysl.decode_one(b"\x81" * 200 + b"\x00")


# -- §3.1  Record framing -----------------------------------------------------

def test_record_type_codes_match_the_table_in_3_1():
    assert smysl.RECORD_NAMES == {
        1: "unit",
        2: "attestation",
        3: "relation",
        4: "thread",
        5: "view",
        6: "contention",
        7: "pack_info",
        8: "schema_decl",
        9: "checkpoint",
        10: "label_binding",
    }


def test_a_record_is_a_two_element_array():
    with pytest.raises(smysl.CborError):
        smysl.decode_store(b"\x83\x01\xa0\x00")  # three elements
    with pytest.raises(smysl.CborError):
        smysl.decode_store(b"\xa1\x01\xa0")  # a map, not an array


def test_an_unknown_type_code_is_preserved_and_skipped_not_rejected():
    """§3.1 and §5. The central forward-compatibility promise."""
    raw = smysl.encode_one([99, {0: "a shape from a later version"}])
    records = smysl.decode_store(raw)
    assert len(records) == 1
    assert not records[0].is_known and records[0].name == "unknown(99)"
    assert records[0].reencode() == raw, "an unknown record did not survive verbatim"


def test_an_unknown_records_body_is_still_parsed_strictly():
    """§3.1: 'Its body is still parsed strictly, so an unknown record cannot smuggle in a
    non-deterministic encoding.'"""
    with pytest.raises(smysl.CborError):
        smysl.decode_store(b"\x82\x18\x63\x9f\xff")  # unknown code, indefinite-length body


def test_a_store_is_a_concatenation_with_no_envelope():
    a = smysl.encode_one([1, {0: "claim", 1: "first", 6: 1}])
    b = smysl.encode_one([1, {0: "claim", 1: "second", 6: 1}])
    records = smysl.decode_store(a + b)
    assert [r.unit_fields()["gist"] for r in records] == ["first", "second"]
    assert smysl.encode_store(records) == a + b


def test_trailing_bytes_are_an_error_not_a_silent_truncation():
    """A store is *records*; a partial one at the end must not be ignored."""
    good = smysl.encode_one([1, {0: "claim", 1: "g", 6: 1}])
    with pytest.raises(smysl.CborError):
        smysl.decode_store(good + b"\x82\x01")


# -- §7  Conformance ----------------------------------------------------------

def test_this_package_declares_only_c_read():
    """§7: 'An implementation declares what it does, not how complete it is.'"""
    assert "C-Read" in (Path(__file__).parents[1] / "README.md").read_text()


def test_what_c_read_cannot_check():
    """The clauses this class provably cannot reach, kept as a list rather than a silence.

    §7 makes C-Read the floor: byte-identical round trip and unknown preservation. It does
    *not* include producing uids, which is where the format's central claim actually lives —
    so the most consequential paragraph in the specification, §2.3 'status is part of
    identity', is untested by anything in this package.

    Emptying this list should be a decision, not an accident, so the test asserts it is not
    empty and names what is in it.
    """
    unreached = {
        "§2.1 uid derivation": "needs BLAKE3; C-Produce, not C-Read",
        "§2.3 status is part of identity": "follows from uid derivation",
        "§2.1 uid text form, 26–52 base32": "no uid parsing at this class",
        "§4 canonical surface form": "surface syntax is not decoded here",
        "§6 rules M, T, L, R, U, I, S, V, D": "semantic; C-Consume and above",
    }
    assert unreached, "if this is ever empty, say which class was implemented instead"
    assert "§2.3 status is part of identity" in unreached
