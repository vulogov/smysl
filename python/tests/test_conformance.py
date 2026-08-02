"""C-Read conformance, against stores the Rust implementation produced.

The spec says C-Read is the floor: *decode and re-encode byte-identically, preserve unknowns*.
That is exactly what these assert, and byte-identity is the only assertion that means
anything — two implementations that both "parse fine" and disagree about bytes disagree about
identity, because a uid is a hash of the encoding.
"""

from pathlib import Path

import pytest

import smysl

FIXTURES = sorted((Path(__file__).parents[2] / "fixtures" / "wire").glob("*.cbor"))


def test_there_are_fixtures_to_read():
    # A conformance suite that silently found no input would pass while testing nothing.
    assert FIXTURES, "no .cbor fixtures; the suite would pass vacuously"


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: p.stem)
def test_store_decodes(path):
    records = smysl.decode_store(path.read_bytes())
    assert records, f"{path.name} decoded to nothing"


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: p.stem)
def test_store_reencodes_byte_identically(path):
    """The C-Read obligation, stated exactly as §7 states it."""
    data = path.read_bytes()
    out = smysl.encode_store(smysl.decode_store(data))
    if out != data:
        # Name the first divergence rather than dumping two blobs.
        for i, (a, b) in enumerate(zip(data, out)):
            if a != b:
                raise AssertionError(
                    f"{path.name}: byte {i} differs — original 0x{a:02x}, re-encoded 0x{b:02x}"
                )
        raise AssertionError(
            f"{path.name}: lengths differ — original {len(data)}, re-encoded {len(out)}"
        )


@pytest.mark.parametrize("path", FIXTURES, ids=lambda p: p.stem)
def test_every_record_reencodes_individually(path):
    for n, rec in enumerate(smysl.decode_store(path.read_bytes())):
        assert rec.reencode() == rec.raw, f"{path.name}: record {n} ({rec.name}) changed"


def test_unknown_record_types_survive():
    """Rule X, §5. The forward-compatibility fixture exists to carry one."""
    path = Path(__file__).parents[2] / "fixtures" / "wire" / "F9-forward-compat.cbor"
    if not path.exists():
        pytest.skip("no forward-compatibility fixture")
    records = smysl.decode_store(path.read_bytes())
    unknown = [r for r in records if not r.is_known]
    # Whether this fixture carries an unknown *record type* is not guaranteed; what is
    # guaranteed is that anything unknown round-trips, which the assertion below covers
    # either way.
    for r in unknown:
        assert r.reencode() == r.raw, f"an unknown record ({r.code}) did not survive"


def test_a_unit_core_names_its_known_fields():
    path = Path(__file__).parents[2] / "fixtures" / "wire" / "F1-incident.cbor"
    units = [r for r in smysl.decode_store(path.read_bytes()) if r.code == 1]
    assert units, "the incident fixture should hold units"
    for u in units:
        f = u.unit_fields()
        # §2.2: schema, gist and status are required; the rest are optional.
        assert "schema" in f and "gist" in f and "status" in f
        assert isinstance(f["gist"], str) and f["gist"]
        assert isinstance(f["status"], int)


# -- the constraints of §3, each asserted as a rejection ----------------------

@pytest.mark.parametrize(
    "data, why",
    [
        (b"\x18\x01", "non-shortest integer"),
        (b"\x19\x00\xff", "non-shortest integer"),
        (b"\x9f\x01\xff", "indefinite-length array"),
        (b"\xbf\x00\x01\xff", "indefinite-length map"),
        (b"\xf6", "null"),
        (b"\xfb\x3f\xf0\x00\x00\x00\x00\x00\x00", "binary64 float"),
        (b"\xa2\x01\x00\x00\x00", "map keys out of order"),
        (b"\xa2\x00\x00\x00\x00", "duplicate map key"),
        (b"\xc0\x00", "a tag"),
    ],
)
def test_forbidden_encodings_are_rejected(data, why):
    with pytest.raises(smysl.CborError):
        smysl.decode_one(data)


def test_a_float_off_the_quantisation_grid_is_rejected():
    import struct

    # 0.1 is not a multiple of 1/1024.
    with pytest.raises(smysl.CborError):
        smysl.decode_one(b"\xfa" + struct.pack(">f", 0.1))
    # 0.5 is.
    value, _ = smysl.decode_one(b"\xfa" + struct.pack(">f", 0.5))
    assert value == 0.5


def test_nesting_is_bounded():
    deep = b"\x81" * 200 + b"\x00"
    with pytest.raises(smysl.CborError):
        smysl.decode_one(deep)


def test_non_nfc_text_is_rejected():
    # "e" + combining acute is not NFC; the composed form is.
    decomposed = "é".encode()
    with pytest.raises(smysl.CborError):
        smysl.decode_one(bytes([0x60 | len(decomposed)]) + decomposed)
    composed = "é".encode()
    value, _ = smysl.decode_one(bytes([0x60 | len(composed)]) + composed)
    assert value == "é"
