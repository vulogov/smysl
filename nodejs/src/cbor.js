// Deterministic CBOR, per §3 of ../../Documentation/SMYSL_FORMAT_SPEC.md
//
// A third reading of the same specification. The Python package was written from the spec to
// test whether the spec was sufficient; this one is written from the spec *without consulting
// the Python*, because two implementations that agree could still both have made the same
// guess where the document is silent. Where a guess was unavoidable it is marked `SPEC:`, and
// those marks are compared against Python's afterwards — a disagreement between them is worth
// more than either agreeing with the Rust.
//
// Strict throughout. Every rule in §3 is a rejection rather than a normalisation: a decoder
// that quietly accepted a non-shortest integer would let two byte strings mean one record,
// and a uid would stop naming exactly one thing.

export const MAX_NESTING = 128; // §3, constraint 9

export class CborError extends Error {
  constructor(message) {
    super(message);
    this.name = "CborError";
  }
}

export class Decoder {
  constructor(data) {
    this.data = data;
    this.i = 0;
  }

  #byte() {
    if (this.i >= this.data.length) throw new CborError("input ended inside a value");
    return this.data[this.i++];
  }

  #take(n) {
    if (n < 0 || this.i + n > this.data.length) {
      throw new CborError("length runs past the end of the input");
    }
    const out = this.data.subarray(this.i, this.i + n);
    this.i += n;
    return out;
  }

  // Returns { major, arg, extra }. `extra` is kept because major type 7 needs it: there the
  // trailing bytes are a float's payload, not an argument, so constraint 2 cannot apply.
  //
  // §3 constraint 2 is scoped to integers and lengths. It used not to be, and applied
  // literally to a float head it rejects 1.0, whose payload 0x3F800000 looks like an
  // over-long encoding of 1065353216. Both independent implementations hit this.
  head() {
    const b = this.#byte();
    const major = b >> 5;
    const extra = b & 0x1f;
    const floaty = major === 7;

    if (extra < 24) return { major, arg: extra, extra };
    if (extra === 24) {
      const v = this.#byte();
      if (!floaty && v < 24) throw new CborError(`${v} in two bytes; shortest form is one`);
      return { major, arg: v, extra };
    }
    if (extra === 25) {
      const d = this.#take(2);
      const v = (d[0] << 8) | d[1];
      if (!floaty && v <= 0xff) throw new CborError(`${v} in three bytes; a shorter form exists`);
      return { major, arg: v, extra };
    }
    if (extra === 26) {
      const d = this.#take(4);
      const v = ((d[0] << 24) >>> 0) + (d[1] << 16) + (d[2] << 8) + d[3];
      if (!floaty && v <= 0xffff) throw new CborError(`${v} in five bytes; a shorter form exists`);
      return { major, arg: v, extra };
    }
    if (extra === 27) {
      const d = this.#take(8);
      let v = 0n;
      for (const byte of d) v = (v << 8n) | BigInt(byte);
      if (!floaty && v <= 0xffffffffn) {
        throw new CborError(`${v} in nine bytes; a shorter form exists`);
      }
      return { major, arg: v <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(v) : v, extra };
    }
    if (extra === 31) {
      throw new CborError("indefinite-length item; definite lengths only"); // constraint 3
    }
    throw new CborError(`reserved additional-information value ${extra}`);
  }

  value(depth = 0) {
    if (depth > MAX_NESTING) throw new CborError(`nesting deeper than ${MAX_NESTING}`);
    const { major, arg, extra } = this.head();

    switch (major) {
      case 0:
        return arg;
      case 1:
        return typeof arg === "bigint" ? -1n - arg : -1 - arg;
      case 2:
        return Uint8Array.from(this.#take(Number(arg)));
      case 3: {
        const raw = this.#take(Number(arg));
        const text = new TextDecoder("utf-8", { fatal: true }).decode(raw);
        // §3, constraint 6. Checked, not applied — normalising here would accept two
        // encodings of one string, which is the thing being forbidden.
        if (text.normalize("NFC") !== text) throw new CborError("text is not NFC-normalised");
        return text;
      }
      case 4: {
        const out = [];
        for (let n = 0; n < arg; n++) out.push(this.value(depth + 1));
        return out;
      }
      case 5:
        return this.#map(Number(arg), depth);
      case 7:
        return this.#simple(arg, extra);
      default:
        // §3 constraint 8: no tags. Rejected here before the spec said to, on the reasoning
        // the clause was eventually written from.
        throw new CborError(`major type ${major} is not part of this format`);
    }
  }

  // A Map rather than an object: §3 permits integer keys in the kernel and text keys inside a
  // payload, and a JavaScript object would turn 0 into "0" and lose that distinction — which
  // is exactly the distinction constraint 1 draws.
  #map(n, depth) {
    const out = new Map();
    let prev = null;
    for (let k = 0; k < n; k++) {
      const start = this.i;
      const key = this.value(depth + 1);
      const keyBytes = this.data.subarray(start, this.i);
      // §3, constraint 4: ascending by *encoded* key bytes.
      if (prev !== null && compareBytes(keyBytes, prev) <= 0) {
        throw new CborError("map keys are not in ascending order, or are duplicated");
      }
      prev = keyBytes;
      out.set(key, this.value(depth + 1));
    }
    return out;
  }

  #simple(arg, extra) {
    if (extra === 20) return false;
    if (extra === 21) return true;
    if (extra === 22) {
      // §3, constraint 5. An absent optional is omitted, so `null` on the wire is a violation.
      throw new CborError("null is forbidden; omit the key instead");
    }
    if (extra === 26) {
      const buf = new ArrayBuffer(4);
      new DataView(buf).setUint32(0, Number(arg));
      const v = new DataView(buf).getFloat32(0);
      if (!Number.isFinite(v)) throw new CborError("float is not finite");
      // §3, constraint 7: a multiple of 1/1024.
      if (!Number.isInteger(v * 1024)) throw new CborError(`float ${v} is not a multiple of 1/1024`);
      return v;
    }
    if (extra === 27) throw new CborError("binary64 float; the format uses binary32");
    throw new CborError(`simple value ${extra} is not part of this format`);
  }
}

function compareBytes(a, b) {
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    if (a[i] !== b[i]) return a[i] < b[i] ? -1 : 1;
  }
  return a.length - b.length;
}

export class Encoder {
  constructor() {
    this.out = [];
  }

  #head(major, arg) {
    const n = typeof arg === "bigint" ? arg : BigInt(arg);
    if (n < 24n) this.out.push((major << 5) | Number(n));
    else if (n <= 0xffn) this.out.push((major << 5) | 24, Number(n));
    else if (n <= 0xffffn) {
      this.out.push((major << 5) | 25, Number((n >> 8n) & 0xffn), Number(n & 0xffn));
    } else if (n <= 0xffffffffn) {
      this.out.push((major << 5) | 26);
      for (let s = 24n; s >= 0n; s -= 8n) this.out.push(Number((n >> s) & 0xffn));
    } else {
      this.out.push((major << 5) | 27);
      for (let s = 56n; s >= 0n; s -= 8n) this.out.push(Number((n >> s) & 0xffn));
    }
  }

  value(v) {
    if (v === true) this.out.push(0xf5);
    else if (v === false) this.out.push(0xf4);
    else if (typeof v === "bigint") {
      if (v >= 0n) this.#head(0, v);
      else this.#head(1, -1n - v);
    } else if (typeof v === "number") {
      if (Number.isInteger(v) && !Object.is(v, -0)) {
        if (v >= 0) this.#head(0, v);
        else this.#head(1, -1 - v);
      } else {
        const buf = new ArrayBuffer(4);
        new DataView(buf).setFloat32(0, v);
        this.out.push(0xfa, ...new Uint8Array(buf));
      }
    } else if (v instanceof Uint8Array) {
      this.#head(2, v.length);
      this.out.push(...v);
    } else if (typeof v === "string") {
      const raw = new TextEncoder().encode(v);
      this.#head(3, raw.length);
      this.out.push(...raw);
    } else if (Array.isArray(v)) {
      this.#head(4, v.length);
      for (const item of v) this.value(item);
    } else if (v instanceof Map) {
      this.#head(5, v.size);
      // Sorted by encoded key bytes, per constraint 4 — not by the key's JavaScript value,
      // which would order an integer key against a text key differently.
      const keys = [...v.keys()].sort((a, b) => compareBytes(encodeOne(a), encodeOne(b)));
      for (const key of keys) {
        this.value(key);
        this.value(v.get(key));
      }
    } else {
      throw new CborError(`cannot encode ${Object.prototype.toString.call(v)}`);
    }
  }
}

export function encodeOne(v) {
  const e = new Encoder();
  e.value(v);
  return Uint8Array.from(e.out);
}

export function decodeOne(data) {
  const d = new Decoder(data);
  return { value: d.value(), used: d.i };
}
