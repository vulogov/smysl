// FC-9: schema declaration keys 4 and 5, read by a third implementation (SMYSL-2.5 §5.8).
//
// TX-P12 step 1 gives record 8 a field table (key 4) and lens steps (key 5), SMYSL-2.3 A-7 and
// Appendix B. Lens *application* is not a conformance obligation (D-8); decoding the
// structure, writing key 5 back byte for byte (L9) and the lens-chain hash are. The checks
// mirror `crates/smysl-core/src/schema/decode.rs`: a key that does not match its structure is
// opaque and reported `SMY-W430`; an op whose tag, or one of whose keys, this reader does not
// know is kept as written and makes its step unusable, `SMY-W469`.

import { blake3 } from "./blake3.js";
import { CborError, decodeOne, encodeOne } from "./cbor.js";

// The keys each known op may carry. A key outside its set makes the op unknown.
export const OP_KEYS = new Map([
  ["rename-type", [0, 1, 2]],
  ["rename-relation", [0, 1, 2]],
  ["rename-key", [0, 1, 2]],
  ["map-value", [0, 1, 2]],
  ["split-value", [0, 1, 2, 3, 4]],
  ["require", [0, 1, 2]],
  ["default", [0, 1, 2, 3]],
  ["retire", [0, 1]],
]);

const CHAIN_DOMAIN = new TextEncoder().encode("smysl/lens-chain/1\0");

class Mismatch extends Error {}

const isInt = (v) => (typeof v === "number" && Number.isInteger(v)) || typeof v === "bigint";

function uint(v) {
  if (!isInt(v) || v < 0 || v > 0xffffffff) throw new Mismatch("not a revision");
  return Number(v);
}

function text(v) {
  if (typeof v !== "string") throw new Mismatch("not text");
  return v;
}

function texts(v) {
  if (!Array.isArray(v)) throw new Mismatch("not an array");
  return v.map(text);
}

function intMap(v) {
  if (!(v instanceof Map) || [...v.keys()].some((k) => !isInt(k))) {
    throw new Mismatch("not an integer-keyed map");
  }
  return v;
}

function textMap(v) {
  if (!(v instanceof Map) || [...v.keys()].some((k) => typeof k !== "string")) {
    throw new Mismatch("not a text-keyed map");
  }
  return v;
}

function cond(v) {
  if (textMap(v).size !== 1) throw new Mismatch("a condition has one key");
}

function shape(raw) {
  let value, used;
  try {
    ({ value, used } = decodeOne(raw));
  } catch (e) {
    if (e instanceof CborError) throw new Mismatch(e.message);
    throw e;
  }
  if (used !== raw.length) throw new Mismatch("a shape is one map");
  intMap(value);
  if (!value.has(0)) throw new Mismatch("a shape is {0: fields}");
  const fields = textMap(value.get(0));
  const checks = [[1, texts], [2, uint], [3, text], [4, uint], [5, cond], [6, texts]];
  for (const f of fields.values()) {
    intMap(f);
    if (!f.has(0)) throw new Mismatch("a field has a type");
    text(f.get(0));
    for (const [k, check] of checks) if (f.has(k)) check(f.get(k));
  }
  return fields.size;
}

// The op's tag, and whether it was read in full (a known op). Throws on a mismatch.
function op(o) {
  intMap(o);
  if (!o.has(0)) throw new Mismatch("an op has a tag");
  const tag = text(o.get(0));
  const allowed = OP_KEYS.get(tag);
  if (!allowed || [...o.keys()].some((k) => !allowed.includes(Number(k)))) {
    return { tag, known: false };
  }
  const any = (v) => v;
  const values = (m) => [...textMap(m).values()].map(text);
  const need = {
    "rename-type": [[1, text], [2, text]],
    "rename-relation": [[1, text], [2, text]],
    "rename-key": [[1, text], [2, text]],
    "map-value": [[1, text], [2, values]],
    "split-value": [[1, text], [2, text], [3, text], [4, textMap]],
    require: [[1, text]],
    default: [[1, text], [2, any]],
    retire: [[1, text]],
  }[tag];
  for (const [k, check] of need) {
    if (!o.has(k)) throw new Mismatch(`${tag} needs key ${k}`);
    check(o.get(k));
  }
  const when = { require: 2, default: 3 }[tag];
  if (when !== undefined && o.has(when)) cond(o.get(when));
  return { tag, known: true };
}

function steps(lenses) {
  if (!Array.isArray(lenses)) throw new Mismatch("lenses are an array");
  return lenses.map((s) => {
    intMap(s);
    const keys = [...s.keys()].map(Number).sort();
    if (keys.join() !== "0,1,2") throw new Mismatch("a step is {0: from, 1: to, 2: ops}");
    const from = uint(s.get(0));
    const to = uint(s.get(1));
    if (to <= from || !Array.isArray(s.get(2))) {
      throw new Mismatch("a step goes up and has an op array");
    }
    const ops = s.get(2).map(op);
    return {
      from,
      to,
      ops: ops.map((o) => o.tag),
      usable: ops.every((o) => o.known),
      bytes: encodeOne(s),
    };
  });
}

/** One schema declaration's FC-9 reading: fields, steps and the codes decoding reports. */
export function decodeSchemaDecl(body) {
  const codes = [];
  let fields = null;
  if (body.has(4)) {
    try {
      const raw = body.get(4);
      if (!(raw instanceof Uint8Array)) throw new Mismatch("payload_shape is a byte string");
      fields = shape(raw);
    } catch (e) {
      if (!(e instanceof Mismatch)) throw e;
      fields = "opaque";
      codes.push("SMY-W430");
    }
  }
  let decoded = [];
  let lensesOpaque = false;
  if (body.has(5)) {
    try {
      decoded = steps(body.get(5));
    } catch (e) {
      if (!(e instanceof Mismatch)) throw e;
      decoded = [];
      lensesOpaque = true;
      codes.push("SMY-W430");
    }
  }
  for (const s of decoded) if (!s.usable) codes.push("SMY-W469");
  return { fields, lensesOpaque, steps: decoded, codes };
}

function head(major, n) {
  if (n < 24) return Uint8Array.of((major << 5) | n);
  for (const [extra, size] of [[24, 1], [25, 2], [26, 4]]) {
    if (n < 2 ** (8 * size)) {
      const out = new Uint8Array(1 + size);
      out[0] = (major << 5) | extra;
      for (let i = 0; i < size; i++) out[size - i] = (n >>> (8 * i)) & 0xff;
      return out;
    }
  }
  throw new Error("too long");
}

function concat(parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

/** Key 5's bytes: an array of each step's bytes, as received (L9). */
export function encodeLenses(steps) {
  return concat([head(4, steps.length), ...steps.map((s) => s.bytes)]);
}

/** Appendix B: BLAKE3 over the domain and `[id, from, to, [step…]]`. */
export function chainHash(id, from, to, steps) {
  return blake3(
    concat([
      CHAIN_DOMAIN,
      head(4, 4),
      encodeOne(id),
      encodeOne(from),
      encodeOne(to),
      encodeLenses(steps),
    ]),
  );
}
