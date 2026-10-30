"""A second implementation of the smysl format, written from the specification.

Its purpose is not to be useful — the Rust crate is useful — but to be *independent*. The
whole proposition of an interchange format is that two implementations agree on what a
document says, and until this existed that had never been tested by anything except the
implementation that defined it.

Conformance target: **C-Read and C-Produce**. C-Read is the floor the spec names — decode,
re-encode byte-identically, preserve what is not understood. C-Produce was added in 0.10.0 and
is the one that matters: it reaches §2.1 and §2.3, *status is part of identity*, which is the
paragraph the whole format rests on and which reading a document never requires. Three
independent readers round-tripped every fixture byte for byte while remaining ignorant of what
a uid is; this package now derives them, over a hand-rolled BLAKE3, and reproduces the Rust's.
"""

from .blake3 import blake3
from .cbor import CborError, decode_one, encode_one
from .records import (
    RECORD_NAMES,
    UNDERSTOOD_RECORDS,
    UNIT_KEYS,
    Record,
    decode_store,
    encode_store,
)
from .library import (
    ALLEN,
    AXES,
    DATING_KEYS,
    DATING_VALUE_KEYS,
    Dating,
    DatingTarget,
    TARGET_KINDS,
    CARRY,
    DID_DOMAIN,
    DID_PREFIX,
    LibraryError,
    MANIFEST_KEYS,
    MID_DOMAIN,
    MID_PREFIX,
    Manifest,
    PART_ENTRY_KEYS,
    PART_READING_KEYS,
    PART_TEXT_KEYS,
    PartEntry,
    PartReading,
    PartText,
    RDID_DOMAIN,
    RDID_PREFIX,
    REDACTION_KEYS,
    Redaction,
    SEGMENT_KEYS,
    TID_DOMAIN,
    TID_PREFIX,
    decode_library_record,
    did,
    is_normalised,
    mid,
    rdid,
    structure_hash,
    text_form,
    tid,
)
from .uid import Source, UnitCore

__all__ = [
    "ALLEN",
    "AXES",
    "DATING_KEYS",
    "DATING_VALUE_KEYS",
    "Dating",
    "DatingTarget",
    "TARGET_KINDS",
    "CARRY",
    "CborError",
    "DID_DOMAIN",
    "DID_PREFIX",
    "LibraryError",
    "MANIFEST_KEYS",
    "MID_DOMAIN",
    "MID_PREFIX",
    "Manifest",
    "PART_ENTRY_KEYS",
    "PART_READING_KEYS",
    "PART_TEXT_KEYS",
    "PartEntry",
    "PartReading",
    "PartText",
    "RDID_DOMAIN",
    "RDID_PREFIX",
    "SEGMENT_KEYS",
    "TID_DOMAIN",
    "TID_PREFIX",
    "Record",
    "RECORD_NAMES",
    "REDACTION_KEYS",
    "Redaction",
    "UNDERSTOOD_RECORDS",
    "Source",
    "UNIT_KEYS",
    "UnitCore",
    "blake3",
    "decode_one",
    "decode_store",
    "decode_library_record",
    "did",
    "encode_one",
    "encode_store",
    "is_normalised",
    "mid",
    "rdid",
    "structure_hash",
    "text_form",
    "tid",
]
__version__ = "0.10.0"
