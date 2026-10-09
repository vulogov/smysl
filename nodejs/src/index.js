// A third implementation of the smysl format, written from the specification.
//
// Conformance target: C-Produce — structural + epistemic + shape (§7). It decodes and
// re-encodes byte-identically, preserves what it does not understand, derives uids, and
// refuses to give one to a unit whose shape §7 forbids. See ../README.md for why a third
// reading is worth more than a second one, and for what this still does not reach.

export { CborError, MAX_NESTING, decodeOne, encodeOne } from "./cbor.js";
export {
  RECORD_NAMES,
  Record,
  UNDERSTOOD_RECORDS,
  UNIT_KEYS,
  decodeStore,
  encodeStore,
} from "./records.js";
export {
  CARRY,
  DID_DOMAIN,
  DID_PREFIX,
  LibraryError,
  MANIFEST_KEYS,
  MANIFEST_REQUIRED,
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
  SEGMENT_KEYS,
  TID_DOMAIN,
  TID_PREFIX,
  bodyBytes,
  canonicalBodyMatches,
  decodeLibraryRecord,
  did,
  identityText,
  isNormalised,
  mid,
  rdid,
  structureHash,
  tid,
} from "./library.js";
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
export const VERSION = "1.10.0";
