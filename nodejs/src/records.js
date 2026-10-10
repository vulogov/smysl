// Record framing and the unit core, per §2 and §3.1 of the format spec.

import { CborError, Decoder, encodeOne } from "./cbor.js";

/** §3.1, every code the format has allocated.
 *
 * A code is a permanent wire commitment the moment it is allocated, so the reserved and
 * not-yet-defined ones are named too: 9 is checkpoint, 16 is reserved, and 17 and 19 are
 * specified but land in later releases. Naming them is how a reader says *what* it met rather
 * than only that it met something.
 */
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
  [14, "manifest"],
  [15, "part_text"],
  [16, "reserved"],
  [17, "dating"],
  [18, "part_reading"],
  [19, "redaction"],
]);

/** The codes whose **bodies** this implementation decodes.
 *
 * Not the same question as RECORD_NAMES, and conflating the two was wrong before it was
 * consequential: `isKnown` was derived from the name table and documented as whether this
 * version understands the record, which was already false for 9 — a checkpoint nothing has
 * ever implemented — and harmless only because nothing emits one. 1.10 writes manifests, part
 * texts and part readings, and a reader calling those "known" while interpreting none of them
 * would be exactly the silence `SMY-W014` exists to break.
 *
 * An unknown record is still preserved verbatim and re-encoded byte for byte; that is C-Read
 * and it is unaffected.
 *
 * 14, 15, 18 and 19 joined in 1.10: `library.js` decodes their bodies and derives the four
 * identities over the first three — a redaction has none, being a statement about a part rather
 * than a thing the format refers to. 9, 16 and 17 are still named and not understood — nothing
 * emits a checkpoint, 16 is reserved, and 17 lands with the release that writes it. The
 * separation above is what makes that sentence sayable.
 */
export const UNDERSTOOD_RECORDS = new Set([
  1, 2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 13, 14, 15, 17, 18, 19,
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

  /** Whether this implementation decodes this record's body.
   *
   * A code the spec names but this reader does not interpret — a manifest, a checkpoint — is
   * **not** known. `name` still answers, so a report can say "a manifest, which this build does
   * not interpret" rather than "something".
   */
  get isKnown() {
    return UNDERSTOOD_RECORDS.has(this.code);
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
