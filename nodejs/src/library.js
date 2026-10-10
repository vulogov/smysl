// The library records and their identities — §2.6 and §3.1 of the format spec.
//
// Until now this implementation had the *names* of records 14, 15 and 18 and nothing else: it
// preserved them verbatim, re-encoded them byte for byte, and reported each as named but not
// understood. That is C-Read and it was honest. It also left the four library identities —
// tid, mid, did, rdid — derived by exactly one implementation, the one that specified them.
//
// §2.6 makes a claim no reading test can reach: that a one-byte domain prefix keeps the four
// kinds apart from each other and from a uid, however their preimages collide. The claim is
// about the construction, and the construction is four lines — the kind of thing that reads as
// obviously right and gets implemented wrong once.
//
// # The number problem, which is this implementation's alone
//
// `Record.reencode()` returns the bytes a record arrived in rather than re-encoding its body,
// because JavaScript has one number type: a `binary32` zero decodes to `0`, and an encoder
// asked to write `0` writes an integer. That bug reaches identity here. A manifest's key 16 is
// "the reader's manifest-level metadata, verbatim" — an arbitrary map, which may hold a float
// — so a mid taken from a re-encoded body would be a different mid for the same manifest, in
// this language only.
//
// So `midOf` takes the **body's own bytes**, sliced out of the record, which is exactly the
// preimage §2.6 names: a decoded record's body bytes are canonical CBOR if the record was.
// `canonicalBodyMatches` is the separate check that this implementation's *encoder* would have
// produced those bytes, which is what a producer needs and what a fixture of hex strings
// cannot reach. Two questions, two answers, and neither borrowed from the other.

import { blake3 } from "./blake3.js";
import { CborError, Decoder, encodeOne } from "./cbor.js";

/** §2.6. The domain byte of each identity **is the record type code it names**. */
export const TID_DOMAIN = 0x0f;
export const MID_DOMAIN = 0x0e;
export const DID_DOMAIN = 0x11;
export const RDID_DOMAIN = 0x12;

export const TID_PREFIX = "t3:";
export const MID_PREFIX = "m3:";
export const DID_PREFIX = "d3:";
export const RDID_PREFIX = "r3:";

/** §3.1, the manifest body's keys. */
export const MANIFEST_KEYS = new Map([
  [0, "alias"],
  [1, "parts"],
  [2, "lang"],
  [3, "reader"],
  [4, "licence"],
  [5, "carry"],
  [6, "title"],
  [7, "creators"],
  [8, "published"],
  [9, "identifiers"],
  [10, "origin"],
  [11, "parent"],
  [12, "parent-kind"],
  [13, "supersedes"],
  [14, "versification"],
  [15, "lossy"],
  [16, "raw"],
  [17, "part-policy"],
  [18, "calendar"],
]);

/** The keys §3.1 marks required. 11 and 12 are a pair and are checked separately. */
export const MANIFEST_REQUIRED = [0, 1, 2, 3, 4, 5, 17];

export const PART_ENTRY_KEYS = new Map([
  [0, "tid"],
  [1, "length"],
  [2, "structure"],
  [3, "rdid"],
  [4, "lang"],
]);

export const PART_TEXT_KEYS = new Map([
  [0, "tid"],
  [1, "text"],
]);

export const PART_READING_KEYS = new Map([
  [0, "tid"],
  [1, "reader"],
  [2, "segments"],
  [3, "raw"],
]);

/** §3.1, redaction (19). Rule Z: the part this names is to be held no longer.
 *
 * The same four keys a withdrawal has, in the same order — both say "this is no longer to be
 * acted on", by whom, when, and optionally why.
 */
export const REDACTION_KEYS = new Map([
  [0, "tid"],
  [1, "agent"],
  [2, "ts"],
  [3, "reason"],
]);

/** §3.1, dating (17). A statement about when something happened.
 *
 * The key numbers are the did's preimage, so they are permanent in a stronger sense than the
 * other tables here: renumbering one would change the identity of every dating ever written.
 */
export const DATING_KEYS = new Map([
  [0, "target"],
  [1, "axis"],
  [2, "value"],
  [3, "basis"],
  [4, "agent"],
  [5, "ts"],
]);

/** §3.1, the three entries of a dating's value map, which holds exactly one. */
export const DATING_VALUE_KEYS = new Map([
  [0, "absolute"],
  [1, "offset"],
  [2, "relative"],
]);

/** §3.1, a dating's axis (key 1). Rule E names a fourth — *known* — which no dating can speak
 * about: it is the earliest attestation clock of a unit and is never corrected.
 */
export const AXES = new Map([
  [0, "said"],
  [1, "composed"],
  [2, "about"],
]);

/** §3.1, a dating's target kind (key 0, first element). */
export const TARGET_KINDS = new Map([
  [0, "unit"],
  [1, "part"],
  [2, "manifest"],
  [3, "window"],
]);

/** §3.1, the Allen relations a relative dating may use. */
export const ALLEN = new Set([
  "before",
  "after",
  "meets",
  "overlaps",
  "during",
  "contains",
  "equals",
]);

export const SEGMENT_KEYS = new Map([
  [0, "start"],
  [1, "end"],
  [2, "level"],
  [3, "locator"],
  [4, "lang"],
  [5, "speaker"],
  [6, "observed"],
  [7, "ids"],
  [8, "tz_offset"],
]);

/** §3.1, a manifest's key 5. */
export const CARRY = new Map([
  [0, "none"],
  [1, "ref"],
  [2, "text"],
]);

export class LibraryError extends Error {}

function digest(domain, preimage) {
  const input = new Uint8Array(preimage.length + 1);
  input[0] = domain;
  input.set(preimage, 1);
  return blake3(input);
}

/** §2.6. BLAKE3-256 over `0x0f` and the part's normalised bytes.
 *
 * The caller's bytes are hashed as given. `isNormalised` is the separate question, and separate
 * on purpose: a decoder must be able to compute the tid a record *claims* in order to report
 * that it is wrong, which it cannot do if computing it requires the bytes to be right.
 */
export function tid(normalised) {
  return digest(TID_DOMAIN, normalised);
}

/** §2.6. BLAKE3-256 over `0x0e` and the canonical CBOR of the manifest body. */
export function mid(body) {
  return digest(MID_DOMAIN, body);
}

/** §2.6. BLAKE3-256 over `0x11` and the canonical CBOR of the dating body.
 *
 * Record 17 lands in a later release and nothing here decodes a dating body. The derivation is
 * present because the domain-separation claim is about all four together: a table with a hole
 * in it cannot be checked.
 */
export function did(body) {
  return digest(DID_DOMAIN, body);
}

/** §2.6. BLAKE3-256 over `0x12` and the canonical CBOR of the reading body. */
export function rdid(body) {
  return digest(RDID_DOMAIN, body);
}

/** §3.1. BLAKE3-256 of a reading's segment table — **with no domain byte**.
 *
 * It names a table rather than a record, so there is no record code to prefix it with. That is
 * also what lets a reading gain raw metadata, keep its structure hash and change its rdid,
 * which is why a part entry carries both.
 */
export function structureHash(segments) {
  return blake3(segments);
}

const ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";

/** §2.6 and §2.1: the prefix and 52 base32 characters. 26 is a display abbreviation. */
export function identityText(prefix, bytes, chars = 52) {
  if (bytes.length !== 32) throw new LibraryError(`an identity is 32 bytes; got ${bytes.length}`);
  let s = prefix;
  for (let i = 0; i < chars; i++) {
    let v = 0;
    for (let k = 0; k < 5; k++) {
      const bit = i * 5 + k;
      const on = bit < 256 && ((bytes[bit >> 3] >> (7 - (bit & 7))) & 1) === 1;
      v = (v << 1) | (on ? 1 : 0);
    }
    s += ALPHABET[v];
  }
  return s;
}

/** §2.6's normalised bytes: UTF-8, NFC, LF line endings, no byte order mark.
 *
 * Nothing else: no whitespace collapsing and no case folding. A part that is not normalised is
 * `SMY-E446` on the way in, because two libraries given the same text would otherwise name two
 * different parts.
 */
export function isNormalised(data) {
  // The byte order mark is checked **in the bytes**, before any decoding, and that is not
  // fastidiousness. `TextDecoder` strips a leading BOM unless `ignoreBOM` is set — the flag is
  // named for what it does to the *output*, not for what it ignores — so the obvious spelling
  // of this function decoded `EF BB BF` away and then truthfully reported that the text does
  // not start with U+FEFF. It accepted exactly the input §2.6 forbids, silently, and in this
  // language only: the Python port's `bytes.decode` keeps the BOM, and its identical five
  // lines were right. A check on the bytes has no such default to know about.
  if (data.length >= 3 && data[0] === 0xef && data[1] === 0xbb && data[2] === 0xbf) return false;
  let text;
  try {
    text = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(data);
  } catch {
    return false;
  }
  // A U+FEFF anywhere else is a zero-width no-break space, which this rule is not about: only
  // a leading one is a byte order mark.
  if (text.includes("\r")) return false;
  return text.normalize("NFC") === text;
}

/** The byte range of a record's body, inside the bytes the record arrived in.
 *
 * §2.6 names the preimage of a mid as "the canonical CBOR of the manifest body". For a decoded
 * record that is a slice of what was read, and taking it rather than re-encoding is what keeps
 * the identity out of reach of the number problem above.
 */
export function bodyBytes(record) {
  if (!record.raw) {
    throw new LibraryError("this record was built here and has no bytes; encode it first");
  }
  const d = new Decoder(record.raw);
  const { major, arg } = d.head();
  if (major !== 4 || arg !== 2) throw new CborError("a record is a two-element array");
  d.value(); // the type code
  return record.raw.subarray(d.i);
}

/** Whether this implementation's encoder reproduces a record's body bytes exactly.
 *
 * The producer's half of the question, asked separately. A `false` here with a correct mid
 * means the encoder disagrees with the reference about canonical form — map key order, a
 * shortest-form head, an omitted optional — which is a defect that would surface the first
 * time this implementation *wrote* a manifest rather than read one.
 */
export function canonicalBodyMatches(record) {
  const want = bodyBytes(record);
  const got = encodeOne(record.body);
  if (got.length !== want.length) return false;
  for (let i = 0; i < got.length; i++) if (got[i] !== want[i]) return false;
  return true;
}

function needBytes(map, key, size, what) {
  const v = map.get(key);
  if (!(v instanceof Uint8Array) || v.length !== size) {
    throw new LibraryError(`${what} is ${size} bytes`);
  }
  return v;
}

export class PartEntry {
  constructor(fields) {
    Object.assign(this, fields);
  }

  /** §3.1, one row of a manifest's key 1. */
  static decode(body) {
    if (!(body instanceof Map)) throw new LibraryError("a part entry is a map");
    for (const key of [0, 1, 2, 3]) {
      if (!body.has(key)) {
        throw new LibraryError(`a part entry needs key ${key} (${PART_ENTRY_KEYS.get(key)})`);
      }
    }
    const length = body.get(1);
    if (!Number.isInteger(length) || length < 0) {
      throw new LibraryError("a part entry's length is an unsigned integer");
    }
    const extra = new Map();
    for (const [k, v] of body) if (k > 4) extra.set(k, v);
    return new PartEntry({
      tid: needBytes(body, 0, 32, "a part entry's tid"),
      length,
      structure: needBytes(body, 2, 32, "a part entry's structure hash"),
      rdid: needBytes(body, 3, 32, "a part entry's rdid"),
      lang: body.get(4),
      extra,
    });
  }
}

export class Manifest {
  constructor(fields, parts, body, raw) {
    this.fields = fields;
    this.parts = parts;
    this.body = body;
    this.raw = raw;
  }

  /** Record 14, decoded. `raw` is the body's own bytes, which the mid is a hash of. */
  static decode(record) {
    const body = record instanceof Map ? record : record.body;
    const raw = record instanceof Map ? null : bodyBytes(record);
    if (!(body instanceof Map)) throw new LibraryError("a manifest body is a map");
    const missing = MANIFEST_REQUIRED.filter((k) => !body.has(k));
    if (missing.length) {
      const names = missing.map((k) => `${k} (${MANIFEST_KEYS.get(k)})`).join(", ");
      throw new LibraryError(`a manifest needs key(s) ${names}`);
    }
    // Key 12 is required with key 11 and meaningless without it; §3.1 says either alone MUST
    // be rejected, for the reason a resolution with one target is.
    if (body.has(11) !== body.has(12)) {
      throw new LibraryError("manifest keys 11 (parent) and 12 (parent-kind) travel together");
    }
    // Key 15 has no `false` encoding: admitting it would give one manifest two byte strings
    // and therefore two mids.
    if (body.get(15) === false) {
      throw new LibraryError("manifest key 15 (lossy) has no `false` encoding");
    }
    if (!Array.isArray(body.get(1))) throw new LibraryError("a manifest's parts are an array");
    if (!CARRY.has(body.get(5))) {
      throw new LibraryError(`carry ${body.get(5)} is not 0, 1 or 2`);
    }
    const fields = new Map();
    for (const [k, v] of body) fields.set(MANIFEST_KEYS.get(k) ?? k, v);
    return new Manifest(fields, body.get(1).map(PartEntry.decode), body, raw);
  }

  get alias() {
    return this.fields.get("alias");
  }

  get carry() {
    return CARRY.get(this.fields.get("carry"));
  }

  /** The mid, over the body's own bytes. */
  mid() {
    if (!this.raw) throw new LibraryError("this manifest has no body bytes");
    return mid(this.raw);
  }
}

export class PartText {
  constructor(tid_, text, extra) {
    this.tid = tid_;
    this.text = text;
    this.extra = extra ?? new Map();
  }

  /** Record 15. */
  static decode(record) {
    const body = record instanceof Map ? record : record.body;
    if (!(body instanceof Map)) throw new LibraryError("a part text body is a map");
    if (!body.has(0) || !body.has(1)) {
      throw new LibraryError("a part text needs keys 0 (tid) and 1 (text)");
    }
    if (!(body.get(1) instanceof Uint8Array)) {
      throw new LibraryError("a part text's text is a byte string");
    }
    const extra = new Map();
    for (const [k, v] of body) if (k > 1) extra.set(k, v);
    return new PartText(needBytes(body, 0, 32, "a part text's tid"), body.get(1), extra);
  }

  /** Whether the bytes hash to the claimed tid **and** are normalised (`SMY-E446`).
   *
   * Both, because either alone leaves a hole: unnormalised bytes that hash to their own tid are
   * a part no other library would name the same way, and normalised bytes under the wrong tid
   * are a substituted file.
   *
   * A decode never fails on this. §2.6 is explicit: a record that cannot be decoded cannot be
   * reported, and one bad part would otherwise stop a whole store from opening.
   */
  verify() {
    const want = tid(this.text);
    if (want.length !== this.tid.length) return false;
    for (let i = 0; i < want.length; i++) if (want[i] !== this.tid[i]) return false;
    return isNormalised(this.text);
  }
}

export class PartReading {
  constructor(fields) {
    Object.assign(this, fields);
  }

  /** Record 18. */
  static decode(record) {
    const body = record instanceof Map ? record : record.body;
    const raw = record instanceof Map ? null : bodyBytes(record);
    if (!(body instanceof Map)) throw new LibraryError("a reading body is a map");
    for (const key of [0, 1, 2]) {
      if (!body.has(key)) {
        throw new LibraryError(`a reading needs key ${key} (${PART_READING_KEYS.get(key)})`);
      }
    }
    if (typeof body.get(1) !== "string") {
      throw new LibraryError("a reading's reader id is text");
    }
    const extra = new Map();
    for (const [k, v] of body) if (k > 3) extra.set(k, v);
    return new PartReading({
      tid: needBytes(body, 0, 32, "a reading's tid"),
      reader: body.get(1),
      segments: body.get(2),
      raw: body.get(3),
      extra,
      body,
      bodyRaw: raw,
    });
  }

  rdid() {
    if (!this.bodyRaw) throw new LibraryError("this reading has no body bytes");
    return rdid(this.bodyRaw);
  }

  /** §3.1: BLAKE3-256 of the canonical CBOR of the segment table, with no domain byte.
   *
   * The table is re-encoded rather than sliced. A segment row holds integers and text and no
   * float — §3.1's row shape has none — so the number problem does not reach it, and
   * re-encoding is what makes this a check of the encoder rather than a copy of the input.
   */
  structureHash() {
    return structureHash(encodeOne(this.segments));
  }

  /** The segment table with its keys named. Unknown keys keep their integer (rule X). */
  rows() {
    if (!Array.isArray(this.segments)) throw new LibraryError("a segment table is an array");
    return this.segments.map((row) => {
      if (!(row instanceof Map)) throw new LibraryError("a segment row is a map");
      const out = new Map();
      for (const [k, v] of row) out.set(SEGMENT_KEYS.get(k) ?? k, v);
      return out;
    });
  }

  /** `SMY-E401`: a re-read part whose reading no longer matches the manifest's entry.
   *
   * The structure hash is checked first, because when a reader is upgraded under a corpus both
   * identities move and the structure is the one that says what changed. An rdid-only mismatch
   * is narrower — same segmentation, different raw metadata.
   */
  verifyEntry(entry) {
    const same = (a, b) => a.length === b.length && a.every((x, i) => x === b[i]);
    if (!same([...this.tid], [...entry.tid])) {
      throw new LibraryError("this reading is not of that part");
    }
    if (!same([...this.structureHash()], [...entry.structure])) {
      throw new LibraryError("SMY-E401: the structure hash does not match the part entry");
    }
    if (!same([...this.rdid()], [...entry.rdid])) {
      throw new LibraryError("SMY-E401: the rdid does not match the part entry");
    }
  }
}

/** Record 19, rule Z: this part's text is to be held no longer.
 *
 * No identity of its own — a redaction is a statement *about* a part, named by that part's tid
 * — so there is nothing here to derive. What a reader of a store does with it is refuse to hold
 * a record 15 or 18 for that tid, including one arriving from a peer that never saw the
 * redaction.
 */
export class Redaction {
  constructor(fields) {
    Object.assign(this, fields);
  }

  static decode(record) {
    const body = record instanceof Map ? record : record.body;
    if (!(body instanceof Map)) throw new LibraryError("a redaction body is a map");
    for (const key of [0, 1, 2]) {
      if (!body.has(key)) {
        throw new LibraryError(`a redaction needs key ${key} (${REDACTION_KEYS.get(key)})`);
      }
    }
    if (typeof body.get(1) !== "string") {
      throw new LibraryError("a redaction's agent is text");
    }
    const extra = new Map();
    for (const [k, v] of body) if (k > 3) extra.set(k, v);
    return new Redaction({
      tid: needBytes(body, 0, 32, "a redaction's tid"),
      agent: body.get(1),
      ts: body.get(2),
      reason: body.has(3) ? needBytes(body, 3, 32, "a redaction's reason") : null,
      extra,
      body,
    });
  }
}

/** What a dating is about: a unit, a part, a manifest, or a window over a part.
 *
 * `ident` is 32 bytes for the first three kinds. For a window it is the part's tid and the
 * range is in `window`, half-open and against the **as-recorded** `observed` instant — not the
 * effective one, which would make the set of selected units move as the datings applied.
 */
export class DatingTarget {
  constructor(fields) {
    Object.assign(this, fields);
  }

  static decode(value) {
    if (!Array.isArray(value) || value.length !== 2) {
      throw new LibraryError("a dating target is [kind, id]");
    }
    const [kind, id] = value;
    if (!TARGET_KINDS.has(kind)) {
      throw new LibraryError(`a dating target kind is 0 to 3, not ${kind}`);
    }
    const bytes32 = (b, what) => {
      if (!(b instanceof Uint8Array) || b.length !== 32) throw new LibraryError(what);
      return b;
    };
    if (kind === 3) {
      if (!Array.isArray(id) || id.length !== 3) {
        throw new LibraryError("a window is [tid, from_ms, to_ms]");
      }
      if (!id.slice(1).every((n) => typeof n === "number" || typeof n === "bigint")) {
        throw new LibraryError("a window's bounds are integers");
      }
      return new DatingTarget({
        kind: 3,
        ident: bytes32(id[0], "a window's tid is 32 bytes"),
        window: [id[1], id[2]],
        kindName: "window",
      });
    }
    return new DatingTarget({
      kind,
      ident: bytes32(id, "a dating target's id is 32 bytes"),
      window: null,
      kindName: TARGET_KINDS.get(kind),
    });
  }
}

/** A dating's value: a one-entry map, so exactly one of three things.
 *
 * Exactly one, and the count is checked: a two-entry map would be a dating that says two
 * things with no rule for which wins.
 */
function datingValue(value) {
  if (!(value instanceof Map) || value.size !== 1) {
    throw new LibraryError("a dating's value is a one-entry map");
  }
  const [[key, inner]] = [...value];
  if (key === 0) {
    if (typeof inner !== "string") {
      throw new LibraryError("an absolute dating's value is EDTF text");
    }
    return ["absolute", inner];
  }
  if (key === 1) {
    // Signed. The first field in this format to carry a negative integer, so a decoder that
    // read CBOR major type 1 as a large positive number would fail here and nowhere else: a
    // clock can be fast as well as slow.
    if (typeof inner !== "number" && typeof inner !== "bigint") {
      throw new LibraryError("an offset is an integer of milliseconds");
    }
    return ["offset", inner];
  }
  if (key === 2) {
    if (!Array.isArray(inner) || inner.length !== 2) {
      throw new LibraryError("a relative dating's value is [allen, target]");
    }
    if (!ALLEN.has(inner[0])) {
      throw new LibraryError(`\`${inner[0]}\` is not an Allen relation this format uses`);
    }
    return ["relative", [inner[0], DatingTarget.decode(inner[1])]];
  }
  throw new LibraryError(`a dating's value has no key ${key}`);
}

/** Record 17: a statement about when something happened.
 *
 * A record *about* units, parts and manifests, never an edit to them — correcting a unit's
 * `observed` in place would change its uid, and a store that re-identified its contents
 * whenever a clock turned out to be wrong could not be cited.
 *
 * Its identity is a did (§2.6), because the records that name a dating need one: a withdrawal
 * makes it not live and a `canonical` commitment holds it.
 */
export class Dating {
  constructor(fields) {
    Object.assign(this, fields);
  }

  static decode(record) {
    const body = record instanceof Map ? record : record.body;
    if (!(body instanceof Map)) throw new LibraryError("a dating body is a map");
    for (const key of [0, 1, 2, 4, 5]) {
      if (!body.has(key)) {
        throw new LibraryError(`a dating needs key ${key} (${DATING_KEYS.get(key)})`);
      }
    }
    const axis = body.get(1);
    if (!AXES.has(axis)) {
      throw new LibraryError(`a dating's axis is 0, 1 or 2, not ${axis}`);
    }
    if (typeof body.get(4) !== "string") {
      throw new LibraryError("a dating's agent is text");
    }
    const extra = new Map();
    for (const [k, v] of body) if (k > 5) extra.set(k, v);
    return new Dating({
      target: DatingTarget.decode(body.get(0)),
      axis,
      axisName: AXES.get(axis),
      value: datingValue(body.get(2)),
      agent: body.get(4),
      ts: body.get(5),
      basis: body.has(3) ? needBytes(body, 3, 32, "a dating's basis") : null,
      extra,
      body,
    });
  }

  /** This dating's did, over the re-encoded body (§2.6). */
  datingId() {
    return did(encodeOne(this.body));
  }
}

/** Decode a record 14, 15, 17, 18 or 19. Anything else is not this module's business. */
export function decodeLibraryRecord(record) {
  switch (record.code) {
    case 17:
      return Dating.decode(record);
    case 14:
      return Manifest.decode(record);
    case 15:
      return PartText.decode(record);
    case 18:
      return PartReading.decode(record);
    case 19:
      return Redaction.decode(record);
    default:
      throw new CborError(`record ${record.code} is not a library record this module decodes`);
  }
}
