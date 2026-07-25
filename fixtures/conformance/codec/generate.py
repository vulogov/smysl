#!/usr/bin/env python3
"""Generate the deterministic-CBOR conformance fixtures of RFC SMYSL-1 §15.4.

Each fixture is a single record envelope `[type_code, payload]`. `valid-*.cbor` files
MUST decode cleanly; every other file MUST be *rejected* with the codes listed in the
matching `.expected` file. A decoder that normalises any of these instead of rejecting it
breaks the guarantee that one uid corresponds to exactly one byte sequence (§7.1).

The fixtures are committed alongside this generator so the conformance suite stays usable
by a second implementation with no Python in the loop. Re-run only to add fixtures:

    python3 fixtures/conformance/codec/generate.py
"""

import pathlib

HERE = pathlib.Path(__file__).parent

# --- minimal deterministic-CBOR emitters ------------------------------------


def head(major: int, arg: int) -> bytes:
    """Shortest-form head for `major` with argument `arg` (§7.1 constraint 2)."""
    if arg < 24:
        return bytes([major << 5 | arg])
    if arg < 0x100:
        return bytes([major << 5 | 24, arg])
    if arg < 0x10000:
        return bytes([major << 5 | 25]) + arg.to_bytes(2, "big")
    if arg < 0x100000000:
        return bytes([major << 5 | 26]) + arg.to_bytes(4, "big")
    return bytes([major << 5 | 27]) + arg.to_bytes(8, "big")


def uint(v: int) -> bytes:
    return head(0, v)


def bstr(b: bytes) -> bytes:
    return head(2, len(b)) + b


def tstr(s: str) -> bytes:
    e = s.encode("utf-8")
    return head(3, len(e)) + e


def array(items) -> bytes:
    return head(4, len(items)) + b"".join(items)


def mapping(pairs) -> bytes:
    """`pairs` is a list of (key_int, encoded_value), emitted in the order given."""
    return head(5, len(pairs)) + b"".join(uint(k) + v for k, v in pairs)


def f32(v: float) -> bytes:
    import struct

    return b"\xfa" + struct.pack(">f", v)


def f64(v: float) -> bytes:
    import struct

    return b"\xfb" + struct.pack(">d", v)


def envelope(type_code: int, payload: bytes) -> bytes:
    return array([uint(type_code), payload])


UID_A = bytes([0x11] * 32)
UID_B = bytes([0x22] * 32)

GIST = "a speculative claim"
SPECULATIVE = 1
REBUTS = 9

# A conforming UnitCore payload: keys ascending, shortest-form ints, NFC text, no nulls.
VALID_UNIT = mapping(
    [
        (0, tstr("claim")),  # schema
        (1, tstr(GIST)),  # gist
        (6, uint(SPECULATIVE)),  # status
    ]
)

# A conforming Relation payload. 0.599609375 is 614/1024 - already quantised, which is
# what the encoder stores for a surface weight of 0.6 (§7.1 constraint 4).
VALID_RELATION = mapping(
    [
        (0, uint(REBUTS)),
        (1, bstr(UID_A)),
        (2, bstr(UID_B)),
        (3, f32(0.599609375)),
    ]
)

FIXTURES = [
    # name, bytes, expected codes, why
    (
        "valid-unit",
        envelope(1, VALID_UNIT),
        [],
        "control: a conforming UnitCore envelope",
    ),
    (
        "valid-relation",
        envelope(3, VALID_RELATION),
        [],
        "control: a conforming Relation with an already-quantised weight",
    ),
    (
        "nonshortest-int",
        envelope(
            1,
            mapping(
                [
                    (0, tstr("claim")),
                    (1, tstr(GIST)),
                    (6, b"\x18\x01"),  # uint8 1 where 0x01 would do
                ]
            ),
        ),
        ["SMY-E080"],
        "status encoded as uint8 rather than in shortest form",
    ),
    (
        "indefinite-length-map",
        envelope(
            1,
            b"\xbf" + uint(0) + tstr("claim") + uint(1) + tstr(GIST) + uint(6) + uint(1) + b"\xff",
        ),
        ["SMY-E080"],
        "indefinite-length map",
    ),
    (
        "indefinite-length-text",
        envelope(
            1,
            mapping(
                [
                    (0, tstr("claim")),
                    (1, b"\x7f" + tstr("a specul") + tstr("ative claim") + b"\xff"),
                    (6, uint(SPECULATIVE)),
                ]
            ),
        ),
        ["SMY-E080"],
        "indefinite-length text string",
    ),
    (
        "unsorted-map-keys",
        envelope(
            1,
            mapping(
                [
                    (1, tstr(GIST)),
                    (0, tstr("claim")),
                    (6, uint(SPECULATIVE)),
                ]
            ),
        ),
        ["SMY-E080"],
        "map keys not strictly ascending",
    ),
    (
        "duplicate-map-key",
        envelope(
            1,
            mapping(
                [
                    (0, tstr("claim")),
                    (0, tstr("finding")),
                    (1, tstr(GIST)),
                    (6, uint(SPECULATIVE)),
                ]
            ),
        ),
        ["SMY-E080"],
        "duplicate map key",
    ),
    (
        "null-optional",
        envelope(
            1,
            mapping(
                [
                    (0, tstr("claim")),
                    (1, tstr(GIST)),
                    (2, b"\xf6"),  # body: null, rather than absent
                    (6, uint(SPECULATIVE)),
                ]
            ),
        ),
        ["SMY-E080"],
        "absent optional encoded as null instead of omitted",
    ),
    (
        "non-nfc-text",
        envelope(
            1,
            mapping(
                [
                    (0, tstr("claim")),
                    # "cafe" + U+0301 COMBINING ACUTE ACCENT: NFD, not NFC.
                    (1, tstr("cafe\u0301 latency claim")),
                    (6, uint(SPECULATIVE)),
                ]
            ),
        ),
        ["SMY-E080"],
        "text is NFD; NFC is required before encoding",
    ),
    (
        "float-f64",
        envelope(
            3,
            mapping(
                [
                    (0, uint(REBUTS)),
                    (1, bstr(UID_A)),
                    (2, bstr(UID_B)),
                    (3, f64(0.599609375)),
                ]
            ),
        ),
        ["SMY-E081"],
        "weight encoded as binary64; binary32 is required",
    ),
    (
        "float-unquantised",
        envelope(
            3,
            mapping(
                [
                    (0, uint(REBUTS)),
                    (1, bstr(UID_A)),
                    (2, bstr(UID_B)),
                    (3, f32(0.6)),  # 0.6 * 1024 = 614.4, not integral
                ]
            ),
        ),
        ["SMY-E081"],
        "weight not quantised to 1/1024, so the hash would depend on the float path",
    ),
    (
        "unknown-envelope-code",
        envelope(99, mapping([(0, tstr("x.future/1"))])),
        ["SMY-W014"],
        "unknown record type: preserved verbatim, skipped semantically, never an error",
    ),
    (
        "truncated-envelope",
        envelope(1, VALID_UNIT)[:-4],
        ["SMY-E004"],
        "input ends mid-item",
    ),
    (
        "truncated-uid",
        envelope(
            3,
            mapping(
                [
                    (0, uint(REBUTS)),
                    (1, bstr(UID_A[:17])),  # 130 bits, the display prefix
                    (2, bstr(UID_B)),
                ]
            ),
        ),
        ["SMY-E071"],
        "a display-form uid prefix in a canonical record",
    ),
]


def main() -> None:
    index = ["# Fixture index - generated by generate.py, do not edit\n"]
    for name, data, codes, why in FIXTURES:
        (HERE / f"{name}.cbor").write_bytes(data)
        (HERE / f"{name}.expected").write_text(
            "".join(f"{c}\n" for c in codes), encoding="utf-8"
        )
        index.append(f"{name}: {'clean' if not codes else ' '.join(codes)} - {why}\n")
    (HERE / "INDEX.txt").write_text("".join(index), encoding="utf-8")
    print(f"wrote {len(FIXTURES)} fixtures to {HERE}")


if __name__ == "__main__":
    main()
