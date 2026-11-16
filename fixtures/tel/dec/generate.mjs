// Generates fixtures/tel/dec/cases.json: what a `dec` is (RFC SMYSL-3 §5), decided here by a
// second implementation rather than copied from the first.
//
// The binary64 cases are the reason this is JavaScript. §5.3 says a double becomes the decimal
// ECMA-262's `Number::toString` selects, and the only way to know what that is without trusting
// a reimplementation is to ask an engine. `String(x)` is that function. Everything else here —
// the token reader, the spelling, the expected CBOR — is written from the RFC's text,
// independently of `smysl-core/src/dec.rs`, so that the Rust test comparing against this file
// compares two readings of the RFC and not one reading with itself.
//
// Run: node fixtures/tel/dec/generate.mjs > fixtures/tel/dec/cases.json
// The random cases come from a fixed seed, so the output is the same on every run.

const M_MAX = (1n << 64n) - 1n;

/** Normalise m × 10^e: trailing zeros into e, zero to [0, 0], then the range check. */
function normalise(m, e) {
  if (m === 0n) return { m: 0n, e: 0 };
  while (m % 10n === 0n) { m /= 10n; e += 1; }
  const mag = m < 0n ? -m : m;
  if (mag > M_MAX || e < -400 || e > 400) return null;
  return { m, e };
}

/** §5.3's token reader, from the RFC's words: digits are read as written, never as a double. */
function readToken(t) {
  const low = t.replace(/^[+-]/, '').toLowerCase();
  if (low === 'nan' || low === 'inf' || low === 'infinity') return { error: 'not-a-value' };
  const r = /^([+-]?)(\d*)(?:\.(\d*))?(?:[eE]([+-]?\d+))?$/.exec(t);
  if (!r || (r[2] + (r[3] ?? '')).length === 0) return { error: 'syntax' };
  const digits = (r[2] + (r[3] ?? '')).replace(/^0+/, '');
  if (digits.replace(/0+$/, '') === '') return { m: '0', e: 0 };
  const exp = r[4] === undefined ? 0n : BigInt(r[4]);
  let e = exp - BigInt((r[3] ?? '').length);
  const trimmed = digits.replace(/0+$/, '');
  e += BigInt(digits.length - trimmed.length);
  if (trimmed.length > 20 || e < -400n || e > 400n) return { error: 'out-of-range' };
  let m = BigInt(trimmed);
  if (r[1] === '-') m = -m;
  const n = normalise(m, Number(e));
  return n ? { m: n.m.toString(), e: n.e } : { error: 'out-of-range' };
}

/** §5.3's one spelling. */
function spell(m, e) {
  const neg = m < 0n;
  let digits = (neg ? -m : m).toString();
  let out;
  if (e >= 0) out = m === 0n ? '0' : digits + '0'.repeat(e);
  else {
    const p = -e;
    out = digits.length > p
      ? digits.slice(0, digits.length - p) + '.' + digits.slice(digits.length - p)
      : '0.' + '0'.repeat(p - digits.length) + digits;
  }
  return (neg ? '-' : '') + out;
}

/** Deterministic CBOR of [m, e]: two shortest-form integers. */
function head(major, arg) {
  const b = [];
  if (arg < 24n) b.push((major << 5) | Number(arg));
  else if (arg < 0x100n) b.push((major << 5) | 24, Number(arg));
  else if (arg < 0x10000n) b.push((major << 5) | 25, Number(arg >> 8n), Number(arg & 0xffn));
  else if (arg < 0x100000000n) {
    b.push((major << 5) | 26);
    for (let s = 24n; s >= 0n; s -= 8n) b.push(Number((arg >> s) & 0xffn));
  } else {
    b.push((major << 5) | 27);
    for (let s = 56n; s >= 0n; s -= 8n) b.push(Number((arg >> s) & 0xffn));
  }
  return b;
}
const int = (v) => (v >= 0n ? head(0, v) : head(1, -1n - v));
const hex = (bytes) => bytes.map((x) => x.toString(16).padStart(2, '0')).join('');
const cbor = (m, e) => hex([0x82, ...int(m), ...int(BigInt(e))]);

// --- binary64 ------------------------------------------------------------------------------

let state = 0x9e3779b97f4a7c15n;
function next64() {
  // xorshift64*, fixed seed: the same "random" doubles on every run.
  state ^= state >> 12n; state ^= (state << 25n) & 0xffffffffffffffffn; state ^= state >> 27n;
  return (state * 0x2545f4914f6cdd1dn) & 0xffffffffffffffffn;
}
function fromBits(bits) {
  const v = new DataView(new ArrayBuffer(8));
  v.setBigUint64(0, bits);
  return v.getFloat64(0);
}
function bitsOf(x) {
  const v = new DataView(new ArrayBuffer(8));
  v.setFloat64(0, x);
  return v.getBigUint64(0);
}

const doubles = [
  0.1, 0.2, 0.1 + 0.2, 0.3, 1, -1, 41, 41.2, 1e21, 1e-7, 123456789012345680000, 5e-324,
  2.2250738585072014e-308, 1.7976931348623157e308, 2 ** 53, 2 ** 53 + 2, 0.0004, 1 / 3, -2 / 3,
  9007199254740993, 1e300, 1e-300, 4.35, 0.5, 100, 1e15, 1e16, 1e17, 123.456, -0,
];
for (let i = 0; i < 1500; i++) {
  const x = fromBits(next64());
  if (Number.isFinite(x)) doubles.push(x);
}
// Doubles a decimal reading produces most often: short decimals, which have the most ties.
for (let i = 0; i < 500; i++) {
  const m = Number(next64() % 1000000n);
  const e = Number(next64() % 40n) - 20;
  doubles.push(Number(`${m}e${e}`));
}

const f64 = doubles.map((x) => {
  const s = String(x); // ECMA-262 Number::toString: the function §5.3 names.
  const r = readToken(s);
  return { bits: bitsOf(x).toString(16).padStart(16, '0'), m: r.m, e: r.e };
});

// --- tokens --------------------------------------------------------------------------------

const tokens = [
  '41', '41.0', '4.10e1', '+41', '0041.000', '410e-1', '0', '-0', '0.000', '-0.0e-5',
  '0e999999999999999999999', '0.3', '9007199254740993', '0.0004', '18446744073709551615',
  '-18446744073709551615', '18446744073709551616', '18446744073709551615000', '1e400', '1e401',
  '1e-400', '1e-401', '1e99999999999999999999', '.5', '5.', '1.5E+3', '-12.340', '1000000',
  'NaN', 'nan', 'Inf', '+Inf', '-Inf', 'infinity', '', '-', '.', '1.2.3', '1e', '1e+', '0x10',
  '1_000', '12a', ' 1', '1 ', '100e-2', '0.00000000000000000001',
].map((t) => ({ token: t, ...readToken(t) }));

// --- spellings -----------------------------------------------------------------------------

const pairs = [
  [412n, -1], [1n, 3], [15n, -10], [0n, 0], [-5n, -1], [-123n, 0], [123456n, -3], [M_MAX, 0],
  [-M_MAX, -400], [7n, 400], [1n, -1], [-1n, -20],
];
const spellings = pairs.map(([m, e]) => ({ m: m.toString(), e, text: spell(m, e), cbor: cbor(m, e) }));

// --- CBOR refusals -------------------------------------------------------------------------
//
// Each is refused with SMY-E602. The accepted forms are the `cbor` of every spelling above.

const refused = [
  ['820a00', 'a trailing-zero mantissa: [10, 0]'],
  ['820001', 'zero with an exponent'],
  ['82180500', 'a non-shortest integer'],
  ['83010000', 'three elements'],
  ['8101', 'one element'],
  ['82fa3f80000000', 'a float mantissa'],
  ['82010000', 'bytes after the dec'],
  ['9f0100ff', 'an indefinite-length array'],
  ['8201190191', 'exponent 401'],
  ['823bffffffffffffffff00', '-2^64'],
  ['82616100', 'a text mantissa'],
].map(([h, why]) => ({ cbor: h, why }));

const out = {
  '//': 'RFC SMYSL-3 §5. Generated by generate.mjs; do not edit. m is a string because it can exceed 2^53.',
  f64,
  tokens,
  spellings,
  refused,
};
process.stdout.write(JSON.stringify(out, null, 1) + '\n');
