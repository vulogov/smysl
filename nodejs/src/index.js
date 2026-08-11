// A third implementation of the smysl format, written from the specification.
//
// Conformance target: C-Produce — structural + epistemic + shape (§7). It decodes and
// re-encodes byte-identically, preserves what it does not understand, derives uids, and
// refuses to give one to a unit whose shape §7 forbids. See ../README.md for why a third
// reading is worth more than a second one, and for what this still does not reach.

export { CborError, MAX_NESTING, decodeOne, encodeOne } from "./cbor.js";
export { RECORD_NAMES, Record, UNIT_KEYS, decodeStore, encodeStore } from "./records.js";
export { Blake3, blake3 } from "./blake3.js";
export {
  SOURCE_KIND,
  STATUS,
  ShapeError,
  canonicalCore,
  coreBytes,
  toHex,
  uid,
  uidShort,
  uidText,
  validate,
} from "./uid.js";
export const VERSION = "1.2.0";
