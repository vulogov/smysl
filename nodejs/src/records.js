// Record framing and the unit core, per §2 and §3.1 of the format spec.

import { CborError, Decoder, encodeOne } from "./cbor.js";

/** §3.1. An unknown code is preserved verbatim and skipped semantically, never rejected. */
export const RECORD_NAMES = new Map([
  [1, "unit"],
  [2, "attestation"],
  [3, "relation"],
  [4, "thread"],
  [5, "view"],
  [6, "contention"],
  [7, "pack_info"],
  [8, "schema_decl"],
  [9, "checkpoint"],
  [10, "label_binding"],
  [11, "withdrawal"],
  [12, "resolution"],
  [13, "commitment"],
]);

/** §2.2. Anything at 9 or above is an unknown key that rule X says must survive verbatim. */
export const UNIT_KEYS = new Map([
  [0, "schema"],
  [1, "gist"],
  [2, "body"],
  [3, "detail"],
  [4, "deps"],
  [5, "grounds"],
  [6, "status"],
  [7, "source"],
  [8, "payload"],
]);

export class Record {
  constructor(code, body, raw) {
    this.code = code;
    this.body = body;
    this.raw = raw;
  }

  get name() {
    return RECORD_NAMES.get(this.code) ?? `unknown(${this.code})`;
  }

  get isKnown() {
    return RECORD_NAMES.has(this.code);
  }

  /** The bytes this record was decoded from, or an encoding of its body if it was built here.
   *
   * Re-encoding a decoded record from its body would be wrong in this implementation and is not
   * wrong in the Python or Go ones, for a reason that is entirely JavaScript's: there is one
   * number type. A `binary32` zero — a pack manifest's optimality gap, a relation's `weight: 1.0`
   * — decodes to the number `0`, and an encoder asked to write `0` writes an integer. The record
   * then re-encodes to different bytes than it was read from, which is the one thing rule X
   * forbids of a reader that does not understand a record.
   *
   * Caught by `fixtures/wire/F12-reserved-pack.cbor`, whose manifests carry a gap of `0.0`. The
   * lifecycle fixture never found it: its only float is a `weight: 0.5`, which is not an integer
   * and so took the branch that was right.
   *
   * The consequence is that `body` is for reading. A record built here — `new Record(code, body)`
   * — has no bytes to carry and is encoded from its body as before. */
  reencode() {
    return this.raw ?? encodeOne([this.code, this.body]);
  }

  /** Name a unit core's known keys. Unknown keys keep their integer, per rule X. */
  unitFields() {
    if (this.code !== 1 || !(this.body instanceof Map)) throw new CborError("not a unit core");
    const out = new Map();
    for (const [k, v] of this.body) out.set(UNIT_KEYS.get(k) ?? k, v);
    return out;
  }
}

/** Decode a concatenation of records (§3.1: no framing envelope). */
export function decodeStore(data) {
  const out = [];
  let off = 0;
  while (off < data.length) {
    const d = new Decoder(data.subarray(off));
    const { major, arg } = d.head();
    if (major !== 4 || arg !== 2) throw new CborError("a record is a two-element array");
    const code = d.value();
    if (!Number.isInteger(code) || code < 0) {
      throw new CborError("a record's type code is an unsigned integer");
    }
    const body = d.value();
    out.push(new Record(code, body, data.subarray(off, off + d.i)));
    off += d.i;
  }
  return out;
}

export function encodeStore(records) {
  const parts = records.map((r) => r.reencode());
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}
