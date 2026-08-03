// A third implementation of the smysl format, written from the specification.
//
// Conformance target: C-Read — decode, re-encode byte-identically, preserve what is not
// understood. See ../README.md for why a third reading is worth more than a second one.

export { CborError, MAX_NESTING, decodeOne, encodeOne } from "./cbor.js";
export { RECORD_NAMES, Record, UNIT_KEYS, decodeStore, encodeStore } from "./records.js";
export const VERSION = "0.9.0";
