// `dec`, the canonical decimal of RFC SMYSL-3 §5 (C-Tel-Read).
//
// A value is m × 10^e. On the wire it is a CBOR array of two shortest-form integers, and one
// value has exactly one encoding: no trailing zero in the mantissa, zero as [0, 0], a mantissa
// magnitude of at most 2^64 − 1 (the sign rides on the major type, so the one CBOR integer
// outside the range, −2^64, is refused) and an exponent in [−400, 400] (§5.2). A decoder
// refuses every other encoding rather than normalising it, because two encodings of one value
// would be two uids. That refusal is SMY-E602.
//
// Written from the RFC's text, as the rest of this port is written from the format spec. The
// mantissa is a BigInt throughout: it can need 65 bits, and §5.2 says so.

import { CborError, Decoder } from "./cbor.js";

const M_MAX = (1n << 64n) - 1n;
const E_MIN = -400;
const E_MAX = 400;

export class DecError extends Error {
  // `kind` is `syntax`, `not-a-value`, `out-of-range` or `E602`.
  constructor(kind, why) {
    super(`${kind}: ${why}`);
    this.kind = kind;
  }
}

/** Trailing zeros into the exponent, zero to [0, 0], then the range check (§5.2). */
export function normalise(m, e) {
  if (m === 0n) return { m: 0n, e: 0 };
  while (m % 10n === 0n) {
    m /= 10n;
    e += 1;
  }
  const mag = m < 0n ? -m : m;
  if (mag > M_MAX || e < E_MIN || e > E_MAX) throw new DecError("out-of-range", `${m}e${e}`);
  return { m, e };
}

/**
 * A decimal token read exactly as written, never through binary64 (§5.3).
 *
 * SPEC: §5.3 names the sources but not one grammar. Read here as an optional sign, digits with
 * at most one point and at least one digit, and an optional exponent. NaN and the infinities
 * are refused as not values (§5.2 rule 5), not as syntax.
 */
export function parseToken(token) {
  const low = token.replace(/^[+-]/, "").toLowerCase();
  if (low === "nan" || low === "inf" || low === "infinity") {
    throw new DecError("not-a-value", token);
  }
  const r = /^([+-]?)([0-9]*)(?:\.([0-9]*))?(?:[eE]([+-]?[0-9]+))?$/.exec(token);
  const intPart = r ? r[2] : "";
  const fracPart = r && r[3] !== undefined ? r[3] : "";
  if (!r || intPart.length + fracPart.length === 0) throw new DecError("syntax", token);
  const digits = (intPart + fracPart).replace(/^0+/, "");
  const trimmed = digits.replace(/0+$/, "");
  if (trimmed === "") return { m: 0n, e: 0 };
  if (trimmed.length > 20) throw new DecError("out-of-range", token);
  // The exponent as a BigInt, so a token's 1e99999999999999999999 is out of range rather than
  // a lost-precision Number.
  const exp = r[4] === undefined ? 0n : BigInt(r[4]);
  const e = exp - BigInt(fracPart.length) + BigInt(digits.length - trimmed.length);
  if (e < BigInt(E_MIN) - 20n || e > BigInt(E_MAX) + 20n) {
    throw new DecError("out-of-range", token);
  }
  const m = BigInt(trimmed);
  return normalise(r[1] === "-" ? -m : m, Number(e));
}

/** A binary64 as ECMA-262's Number::toString selects it (§5.3) — which is String(x). */
export function fromF64(x) {
  if (!Number.isFinite(x)) throw new DecError("not-a-value", String(x));
  if (x === 0) return { m: 0n, e: 0 };
  return parseToken(String(x));
}

/** The one text spelling (§5.3): plain decimal, no exponent, no `+`. */
export function spell(m, e) {
  const neg = m < 0n;
  const digits = (neg ? -m : m).toString();
  let out;
  if (e >= 0) out = m === 0n ? "0" : digits + "0".repeat(e);
  else if (digits.length > -e) {
    out = `${digits.slice(0, digits.length + e)}.${digits.slice(digits.length + e)}`;
  } else out = `0.${"0".repeat(-e - digits.length)}${digits}`;
  return (neg ? "-" : "") + out;
}

function head(major, arg) {
  const out = [];
  if (arg < 24n) out.push((major << 5) | Number(arg));
  else {
    const [ai, width] = arg < 0x100n ? [24, 1] : arg < 0x10000n ? [25, 2] : arg < 0x100000000n ? [26, 4] : [27, 8];
    out.push((major << 5) | ai);
    for (let s = BigInt(8 * (width - 1)); s >= 0n; s -= 8n) out.push(Number((arg >> s) & 0xffn));
  }
  return out;
}

const int = (v) => (v >= 0n ? head(0, v) : head(1, -1n - v));

/** [m, e] as two shortest-form integers. */
export function toCbor(m, e) {
  return Uint8Array.from([0x82, ...int(m), ...int(BigInt(e))]);
}

/** A dec from the whole of `data`, refusing anything not canonical (SMY-E602). */
export function fromCbor(data) {
  const d = new Decoder(data);
  const parts = [];
  try {
    const a = d.head();
    if (a.major !== 4 || a.arg !== 2) throw new DecError("E602", "not an array of two integers");
    for (let k = 0; k < 2; k++) {
      const { major, arg } = d.head();
      if (major === 0) parts.push(BigInt(arg));
      else if (major === 1) parts.push(-1n - BigInt(arg));
      else throw new DecError("E602", "not an array of two integers");
    }
  } catch (err) {
    if (err instanceof CborError) throw new DecError("E602", err.message);
    throw err;
  }
  if (d.i !== data.length) throw new DecError("E602", "bytes after the dec");
  const [m, e] = parts;
  if ((m < 0n ? -m : m) > M_MAX) throw new DecError("E602", "the mantissa is -2^64");
  if (e < BigInt(E_MIN) || e > BigInt(E_MAX)) {
    throw new DecError("E602", "the exponent is outside [-400, 400]");
  }
  if (m === 0n && e !== 0n) throw new DecError("E602", "zero is [0, 0]");
  if (m !== 0n && m % 10n === 0n) throw new DecError("E602", "the mantissa has a trailing zero");
  return { m, e: Number(e) };
}
