#!/usr/bin/env python3
"""Generate `fixtures/wire/invalid/` — byte strings every implementation must reject.

Why this exists
---------------

0.9 established that four implementations agree on *accepting* four documents. That is the
weaker half of the claim. Determinism lives in what gets **rejected**: the whole point of §3
is that one value has exactly one encoding, and that property is enforced entirely by
refusing the alternatives. If Python accepted a non-shortest integer that Go rejected, both
suites would still be green, both would still round-trip every valid fixture, and two
implementations would disagree about whether a given byte string is a smysl document.

Nothing checked that, because each implementation invented its own invalid inputs — fifteen
cases in Python, sixteen in JavaScript, eight in Go, no two the same bytes.

These cases cannot be produced by the Rust encoder, which is the point: a correct encoder
never emits them. So they are hand-authored here, and this script is checked in so the corpus
is reproducible rather than a pile of mystery bytes.

The manifest records the §3 constraint each case violates, not an error message. Independent
implementations will word their errors differently and should — agreeing on the *reason* is
the meaningful claim, and it is coarse enough to be honest.
"""

import json
import struct
from pathlib import Path

OUT = Path(__file__).resolve().parents[1] / "fixtures" / "wire" / "invalid"

CASES = []


def case(name, constraint, why, data):
    CASES.append({"file": f"{name}.cbor", "constraint": constraint, "why": why})
    (OUT / f"{name}.cbor").write_bytes(data)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    for stale in OUT.glob("*.cbor"):
        stale.unlink()

    # -- constraint 2: shortest form, for integers and lengths ---------------------------
    case("int-two-bytes-for-one", 2,
         "1 encoded as 0x18 0x01; it fits in the additional-information field",
         bytes([0x18, 0x01]))
    case("int-three-bytes-for-one", 2,
         "1 encoded in a two-byte argument",
         bytes([0x19, 0x00, 0x01]))
    case("int-five-bytes-for-small", 2,
         "1 encoded in a four-byte argument",
         bytes([0x1A, 0x00, 0x00, 0x00, 0x01]))
    case("int-nine-bytes-for-small", 2,
         "1 encoded in an eight-byte argument",
         bytes([0x1B, 0, 0, 0, 0, 0, 0, 0, 1]))
    case("negint-two-bytes-for-one", 2,
         "-1 encoded as 0x38 0x00 rather than 0x20",
         bytes([0x38, 0x00]))
    case("text-length-not-shortest", 2,
         'the text "a" with its length in two bytes',
         bytes([0x78, 0x01]) + b"a")
    case("array-length-not-shortest", 2,
         "a one-element array with its count in two bytes",
         bytes([0x98, 0x01, 0x01]))

    # -- constraint 3: definite lengths only ---------------------------------------------
    case("indefinite-array", 3,
         "indefinite-length array; one value would have two encodings",
         bytes([0x9F, 0x01, 0xFF]))
    case("indefinite-map", 3,
         "indefinite-length map",
         bytes([0xBF, 0x00, 0x01, 0xFF]))
    case("indefinite-text", 3,
         "chunked text string",
         bytes([0x7F, 0x61, 0x61, 0xFF]))
    case("indefinite-bytes", 3,
         "chunked byte string",
         bytes([0x5F, 0x41, 0x01, 0xFF]))

    # -- constraint 4: map keys ascending by encoded bytes, no duplicates ----------------
    case("map-keys-unsorted", 4,
         "keys 1 then 0; ascending by encoded key bytes is required",
         bytes([0xA2, 0x01, 0x01, 0x00, 0x00]))
    case("map-keys-duplicate", 4,
         "the key 0 twice; two encodings of one map, and an ambiguous read",
         bytes([0xA2, 0x00, 0x01, 0x00, 0x02]))

    # -- constraint 5: no null ------------------------------------------------------------
    case("null-value", 5,
         "null on the wire; an absent optional is omitted instead",
         bytes([0xF6]))
    case("null-inside-map", 5,
         "null as a map value",
         bytes([0xA1, 0x00, 0xF6]))
    case("undefined-value", 5,
         "the undefined simple value, which the format does not define",
         bytes([0xF7]))

    # -- constraint 6: NFC, valid UTF-8 ---------------------------------------------------
    decomposed = "é"  # e + COMBINING ACUTE ACCENT; NFC is U+00E9
    raw = decomposed.encode("utf-8")
    case("text-not-nfc", 6,
         "e + U+0301 rather than the composed U+00E9; two encodings of one string",
         bytes([0x60 | len(raw)]) + raw)
    case("text-invalid-utf8", 6,
         "a lone continuation byte",
         bytes([0x61, 0x80]))

    # -- constraint 7: binary32, finite, a multiple of 1/1024 ------------------------------
    case("float-not-quantised", 7,
         "0.1 is not a multiple of 1/1024",
         bytes([0xFA]) + struct.pack(">f", 0.1))
    case("float-infinity", 7,
         "infinity is not a finite value",
         bytes([0xFA]) + struct.pack(">f", float("inf")))
    case("float-nan", 7,
         "NaN is not a finite value, and has many encodings",
         bytes([0xFA]) + struct.pack(">f", float("nan")))
    case("float-binary64", 7,
         "binary64; the format quantises to binary32",
         bytes([0xFB]) + struct.pack(">d", 1.0))
    case("float-half", 7,
         "binary16; not a form this format uses",
         bytes([0xF9, 0x3C, 0x00]))

    # -- constraint 8: no tags -------------------------------------------------------------
    case("tag-datetime", 8,
         "tag 0 (standard date/time); major type 6 is not part of this format",
         bytes([0xC0, 0x60]))
    case("tag-bignum", 8,
         "tag 2 (positive bignum)",
         bytes([0xC2, 0x41, 0x01]))

    # -- constraint 9: nesting bounded ------------------------------------------------------
    # 129 nested one-element arrays: one deeper than the limit.
    case("nesting-too-deep", 9,
         "129 nested arrays against a limit of 128",
         bytes([0x81] * 129) + bytes([0x00]))

    # -- truncation: not a numbered constraint, but every decoder must refuse ---------------
    case("truncated-text", 0,
         "a text head claiming four bytes with two present",
         bytes([0x64, 0x61, 0x62]))
    case("truncated-array", 0,
         "an array claiming three elements with one present",
         bytes([0x83, 0x01]))

    manifest = {
        "purpose": (
            "Byte strings that are not smysl documents. Every implementation must reject "
            "every one of them. The `constraint` field is the section-3 clause violated; 0 "
            "means malformed input rather than a numbered clause."
        ),
        "cases": sorted(CASES, key=lambda c: (c["constraint"], c["file"])),
    }
    (OUT / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(f"wrote {len(CASES)} cases to {OUT}")


if __name__ == "__main__":
    main()
