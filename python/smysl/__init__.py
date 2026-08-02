"""A second implementation of the smysl format, written from the specification.

Its purpose is not to be useful — the Rust crate is useful — but to be *independent*. The
whole proposition of an interchange format is that two implementations agree on what a
document says, and until this existed that had never been tested by anything except the
implementation that defined it.

Conformance target: **C-Read**, the floor the spec names — decode, re-encode byte-identically,
preserve what is not understood. Nothing above it is attempted, and the spec says an
implementation should declare what it does rather than how complete it is.
"""

from .cbor import CborError, decode_one, encode_one
from .records import RECORD_NAMES, UNIT_KEYS, Record, decode_store, encode_store

__all__ = [
    "CborError",
    "Record",
    "RECORD_NAMES",
    "UNIT_KEYS",
    "decode_one",
    "decode_store",
    "encode_one",
    "encode_store",
]
__version__ = "0.9.0"
