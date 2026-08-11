// Uid derivation and the C-Produce shape clause, per §2 and §7 of the format spec.
//
//     uid = BLAKE3-256( canonical_cbor( unit_core ) )                             — §2.1
//
// C-Read never touches this. Decoding a document does not require computing a uid, so three
// independent readers round-tripped every fixture byte for byte while remaining ignorant of
// what a uid *is* — and §2.3, *status is part of identity*, is the format's central claim.
// `python/` gave it a second derivation in 0.10 and `go/` a third in 1.1; this is the fourth,
// and the third independent of the Rust.
//
// # Four places §2.2 was not enough, all of which move the uid
//
// Marked `SPEC:` at the point of use, as the CBOR reader marks its own. All four were settled
// by decoding `fixtures/wire/uid/cases.json` rather than by reading the document, which means
// the fixtures are normative content the specification does not admit to having. `python/` and
// `go/` reached the same answers — necessarily, since they reproduce the same bytes — but
// neither recorded that it had to guess, so the gaps survived two readings unnoticed.
//
//   1. §2.2 lists `deps` and `grounds` as "required, MAY be empty", and the encoder *omits*
//      them when empty. This is not silence; it is the document saying the opposite of what
//      the reference does, and a literal reading produces a different uid for every unit that
//      has neither. See `putSet`.
//   2. `status` is typed `uint` and its values appear nowhere. §6 and §7 name six statuses
//      without mapping one to a number. See `STATUS`.
//   3. `source` is typed `map` with no key layout, and `kind` is a second undocumented enum.
//      Recoverable only from `core_bytes_hex`, since the fixture JSON gives names. See
//      `putSource`.
//   4. §2.1 says the text form is `b3:` plus 52 base32 characters without naming an alphabet,
//      a case or a bit order. This one does not move the uid — the wire carries raw bytes —
//      but §2.1 also says a parser MUST accept 26 to 52 characters, and two implementations
//      with different alphabets cannot read each other's surface text. See `uidText`.

import { encodeOne } from "./cbor.js";
import { blake3 } from "./blake3.js";

export class ShapeError extends Error {
  constructor(code, message) {
    super(`${code}: ${message}`);
    this.name = "ShapeError";
    this.code = code;
  }
}

/**
 * SPEC: gap 2. The six statuses of §6/§7, and the integers they are on the wire.
 *
 * The order is load bearing beyond naming: the reference pins these discriminants because rule
 * M compares them as integers, so a reader that guessed a different order would not merely
 * derive wrong uids — it would enforce a different monotonicity rule and believe it conformant.
 */
export const STATUS = Object.freeze({
  unfounded: 0,
  speculative: 1,
  inferred: 2,
  derived: 3,
  cited: 4,
  measured: 5,
});

/** SPEC: gap 3, second half. The `kind` of a source reference. */
export const SOURCE_KIND = Object.freeze({
  url: 0,
  file: 1,
  metric: 2,
  tool: 3,
  doc: 4,
});

const STATUS_NAME = new Map(Object.entries(STATUS).map(([name, n]) => [n, name]));

/** §7: `derived` and `inferred` are claims about other units, so they must have grounds. */
const REQUIRES_GROUNDS = new Set([STATUS.inferred, STATUS.derived]);

/** §7: `measured` and `cited` ground out externally, so they must carry a source. */
const REQUIRES_SOURCE = new Set([STATUS.cited, STATUS.measured]);

const UID_BYTES = 32;

/**
 * The shape clause of §7, which is the half of C-Produce that separates it from C-Consume.
 *
 * > **C-Produce** — structural + epistemic + *shape*: emit well-formed units: a gist present,
 * > grounds where the status demands them, a source where `measured` or `cited` demands one.
 *
 * Rule M is *not* checked here, and the omission is deliberate rather than an oversight. M says
 * a `derived` or `inferred` unit must not exceed the status of its weakest **present** ground,
 * and a unit core carries its grounds as uids — the statuses they name are not in hand. M is
 * checkable against a store and not against a unit, so a function claiming to enforce it here
 * would be a check that cannot fail.
 *
 * Throws rather than returning a flag, and `uid()` calls it first: a malformed unit must not be
 * able to obtain an identity from this package. That is what makes the class a claim about
 * *emitting* rather than about validating on request.
 */
export function validate(core) {
  if (typeof core.schema !== "string" || core.schema.length === 0) {
    throw new ShapeError("SMY-E030", "a unit core has a schema (§2.2, key 0, required)");
  }
  // §4 trims the assembled gist, so whitespace is not the presence §7 asks for.
  if (typeof core.gist !== "string" || core.gist.trim().length === 0) {
    throw new ShapeError("SMY-E031", "a unit core has a gist (§7: 'a gist present')");
  }

  const status = core.status;
  if (!STATUS_NAME.has(status)) {
    throw new ShapeError("SMY-E033", `status ${status} is not one of the six in §6`);
  }
  const name = STATUS_NAME.get(status);

  // §7's C-Consume clause, which C-Produce includes: "reject an authored `unfounded`".
  // §0 puts it the same way. Unfounded is reachable only by retraction — it is what a unit
  // becomes when its support is withdrawn, never what an author may assert.
  if (status === STATUS.unfounded) {
    throw new ShapeError("SMY-E034", "`unfounded` is reached by retraction, never authored");
  }

  if (REQUIRES_GROUNDS.has(status) && (core.grounds ?? []).length === 0) {
    throw new ShapeError("SMY-E032", `\`${name}\` is a claim about other units and needs grounds`);
  }
  if (REQUIRES_SOURCE.has(status) && !core.source) {
    throw new ShapeError("SMY-E035", `\`${name}\` grounds out externally and needs a source`);
  }
  return core;
}

function compareBytes(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  }
  return a.length - b.length;
}

/**
 * SPEC: gap 1. `deps` and `grounds`: deduplicated, sorted by uid bytes — and **omitted when
 * empty**, which §2.2 does not say and its presence column contradicts.
 *
 * The column reads "required, MAY be empty" for both. Read literally that is a key which is
 * always present and sometimes holds an empty array, and an implementer who does that emits a
 * five-key map where the reference emits three. The `minimal` fixture is `a3` over keys 0, 1
 * and 6; the literal reading gives `a5` and a uid that names nothing.
 *
 * "MAY be empty" is evidently about the *set* being allowed to have no members rather than
 * about the encoding, but nothing in §2.2 says so, and the two readings disagree about identity
 * rather than about style.
 */
function putSet(map, key, uids) {
  const list = uids ?? [];
  const seen = new Map();
  for (const u of list) {
    const bytes = asUidBytes(u);
    seen.set(bytes.join(","), bytes);
  }
  if (seen.size === 0) return; // <- the clause above
  const sorted = [...seen.values()].sort(compareBytes);
  map.set(key, sorted);
}

/**
 * SPEC: gap 3. The source map's layout: `{0: kind, 1: reference, 2: captured?}`.
 *
 * §2.2 types key 7 as `map` and stops. The fixture JSON names the fields but not their
 * integers, so the only statement of this anywhere is the hex in `core_bytes_hex` — which is
 * to say a C-Produce implementer must decode a fixture to learn a part of the format.
 *
 * `captured` follows the same omit-when-absent rule as every other optional (constraint 5).
 */
function putSource(map, source) {
  if (!source) return;
  const kind = typeof source.kind === "string" ? SOURCE_KIND[source.kind] : source.kind;
  if (!Number.isInteger(kind)) {
    throw new ShapeError("SMY-E036", `source kind ${source.kind} is not one of the five in §2.2`);
  }
  const inner = new Map();
  inner.set(0, kind);
  inner.set(1, source.reference);
  if (source.captured != null) inner.set(2, source.captured);
  map.set(7, inner);
}

function asUidBytes(u) {
  if (u instanceof Uint8Array) {
    if (u.length !== UID_BYTES) throw new ShapeError("SMY-E071", `a uid is ${UID_BYTES} bytes`);
    return u;
  }
  if (typeof u === "string") {
    const hex = u.startsWith("b3:") ? null : u;
    if (hex === null) throw new ShapeError("SMY-E071", "pass uid bytes, not the display form");
    if (hex.length !== UID_BYTES * 2) {
      throw new ShapeError("SMY-E071", `a uid is ${UID_BYTES * 2} hex characters`);
    }
    return Uint8Array.from(hex.match(/../g).map((h) => parseInt(h, 16)));
  }
  throw new ShapeError("SMY-E071", "a uid is bytes or hex");
}

/**
 * The unit core of §2.2 as a CBOR map: integer keys, ascending, absent optionals omitted.
 *
 * Ascending order is not imposed here. The encoder sorts by encoded key bytes for constraint 4
 * and would fix a wrong order silently, so relying on insertion order would be relying on
 * something untested. The keys go in numbered rather than in sequence for that reason: the
 * order below is documentation, not mechanism.
 *
 * Unknown keys — §2.2's `≥9` row, rule X — are accepted and passed to the encoder, which is
 * what makes them part of the uid. §3's scope paragraph is the reason they cannot be waved
 * through unchecked: preserved bytes are hashed, so a unit whose unknown value had two
 * encodings would have two uids.
 */
export function canonicalCore(core) {
  const m = new Map();
  m.set(0, core.schema);
  m.set(1, core.gist);
  if (core.body != null) m.set(2, core.body);
  if (core.detail != null) m.set(3, core.detail);
  putSet(m, 4, core.deps);
  putSet(m, 5, core.grounds);
  m.set(6, core.status);
  putSource(m, core.source);
  if (core.payload != null) m.set(8, core.payload);
  for (const [k, v] of core.unknown ?? []) {
    if (!Number.isInteger(k) || k < 9) {
      throw new ShapeError("SMY-E037", `unknown kernel keys are 9 and above, not ${k}`);
    }
    m.set(k, v);
  }
  return m;
}

/** The canonical CBOR §2.1 hashes. Separated from `uid` so a mismatch says which half broke. */
export function coreBytes(core) {
  return encodeOne(canonicalCore(core));
}

/**
 * §2.1, and it validates first.
 *
 * The ordering is the point: §7 defines C-Produce as *emitting* well-formed units, so a
 * malformed one must not be able to acquire an identity from this package at all. Deriving
 * first and validating afterwards would leave the caller holding a usable uid for a unit the
 * class forbids.
 */
export function uid(core) {
  validate(core);
  return blake3(coreBytes(core));
}

/**
 * SPEC: gap 4. RFC 4648 base32, lowercased, no padding, most-significant bit first.
 *
 * §2.1 says "52 base32 characters" and names no alphabet, no case and no bit order. It does not
 * affect a uid — CBOR carries the raw 32 bytes — but §2.1 also requires a parser to accept 26
 * to 52 characters, and that obligation is meaningless between implementations that disagree
 * about which 32 symbols are meant. base32hex would have been an equally faithful reading of
 * the text and would produce different, mutually unreadable names for the same unit.
 *
 * The 52nd character covers bits 255–259; the four past the end are zero, which is what makes
 * the length exact rather than padded.
 */
const ALPHABET = "abcdefghijklmnopqrstuvwxyz234567";

export function uidText(bytes, chars = 52) {
  let s = "b3:";
  for (let i = 0; i < chars; i++) {
    let v = 0;
    for (let k = 0; k < 5; k++) {
      const bit = i * 5 + k;
      const set = bit < 256 && ((bytes[bit >> 3] >> (7 - (bit & 7))) & 1) === 1;
      v = (v << 1) | (set ? 1 : 0);
    }
    s += ALPHABET[v];
  }
  return s;
}

/** §2.1's display form: the first 130 bits. Never appears in canonical CBOR. */
export function uidShort(bytes) {
  return uidText(bytes, 26);
}

export function toHex(bytes) {
  return [...bytes].map((b) => b.toString(16).padStart(2, "0")).join("");
}
