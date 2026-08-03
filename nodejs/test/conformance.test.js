// C-Read conformance, and the specification walked clause by clause.
//
// Two questions, as in the Python package. Do we agree with the Rust — fixtures in,
// byte-identical bytes out. And do we do what the *document* says, section by section. A
// library can pass the first and fail the second, which is why both are here.
//
// `node --test`, no dependencies. A dependency doing part of the work would weaken the
// evidence this exists to provide.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as smysl from "../src/index.js";

const here = dirname(fileURLToPath(import.meta.url));
const WIRE = join(here, "..", "..", "fixtures", "wire");
const SPEC = join(here, "..", "..", "Documentation", "SMYSL_FORMAT_SPEC.md");
const fixtures = readdirSync(WIRE).filter((f) => f.endsWith(".cbor")).sort();

const bytes = (name) => new Uint8Array(readFileSync(join(WIRE, name)));
const hex = (s) => Uint8Array.from(s.match(/../g).map((b) => parseInt(b, 16)));

test("there are fixtures to read", () => {
  assert.ok(fixtures.length, "no .cbor fixtures; the suite would pass vacuously");
});

test("the specification is where we think it is", () => {
  assert.match(readFileSync(SPEC, "utf8"), /Deterministic CBOR/);
});

// -- C-Read: agreement with the reference ------------------------------------

for (const name of fixtures) {
  test(`${name} decodes`, () => {
    assert.ok(smysl.decodeStore(bytes(name)).length, `${name} decoded to nothing`);
  });

  test(`${name} re-encodes byte-identically`, () => {
    const data = bytes(name);
    const out = smysl.encodeStore(smysl.decodeStore(data));
    for (let i = 0; i < Math.min(data.length, out.length); i++) {
      assert.equal(out[i], data[i], `byte ${i} differs in ${name}`);
    }
    assert.equal(out.length, data.length, `length differs in ${name}`);
  });

  test(`${name}: every record re-encodes individually`, () => {
    smysl.decodeStore(bytes(name)).forEach((r, n) => {
      assert.deepEqual(r.reencode(), r.raw, `${name}: record ${n} (${r.name}) changed`);
    });
  });
}

// -- §2.2  What is hashed -----------------------------------------------------

test("§2.2 unit core keys match the table", () => {
  assert.deepEqual(
    [...smysl.UNIT_KEYS.entries()],
    [[0, "schema"], [1, "gist"], [2, "body"], [3, "detail"], [4, "deps"],
     [5, "grounds"], [6, "status"], [7, "source"], [8, "payload"]],
  );
});

test("§2.2/§5 keys at nine and above are unknown and kept", () => {
  const body = new Map([[0, "claim"], [1, "a gist"], [6, 1], [9, "a later addition"]]);
  const raw = smysl.encodeOne([1, body]);
  const [rec] = smysl.decodeStore(raw);
  assert.deepEqual(rec.reencode(), raw);
  assert.equal(rec.unitFields().get(9), "a later addition");
});

test("§2.2 an absent optional is omitted, not null", () => {
  assert.throws(() => smysl.decodeOne(hex("a102f6")), smysl.CborError);
});

// -- §3  Deterministic CBOR, one per constraint -------------------------------

for (const [data, why] of [
  ["1817", "non-shortest one-byte"],
  ["1900ff", "non-shortest two-byte"],
  ["1a0000ffff", "non-shortest four-byte"],
]) {
  test(`§3 constraint 2 rejects a ${why} integer`, () => {
    assert.throws(() => smysl.decodeOne(hex(data)), smysl.CborError);
  });
}

test("§3 constraint 2 does not apply to float payloads", () => {
  // 0x3F800000 is 1.0, not an over-long encoding of 1065353216.
  assert.equal(smysl.decodeOne(hex("fa3f800000")).value, 1.0);
});

for (const [data, why] of [["9fff", "array"], ["bfff", "map"], ["5fff", "bytes"], ["7fff", "text"]]) {
  test(`§3 constraint 3 rejects an indefinite-length ${why}`, () => {
    assert.throws(() => smysl.decodeOne(hex(data)), smysl.CborError);
  });
}

test("§3 constraint 4 requires ascending, non-duplicate keys", () => {
  smysl.decodeOne(hex("a20000010 0".replace(/ /g, "")));
  assert.throws(() => smysl.decodeOne(hex("a201000000")), smysl.CborError);
  assert.throws(() => smysl.decodeOne(hex("a200000000")), smysl.CborError);
});

test("§3 constraint 5 rejects null", () => {
  assert.throws(() => smysl.decodeOne(hex("f6")), smysl.CborError);
});

test("§3 constraint 6 requires NFC text", () => {
  const decomposed = new TextEncoder().encode("é");
  assert.throws(
    () => smysl.decodeOne(Uint8Array.from([0x60 | decomposed.length, ...decomposed])),
    smysl.CborError,
  );
  const composed = new TextEncoder().encode("é");
  assert.equal(
    smysl.decodeOne(Uint8Array.from([0x60 | composed.length, ...composed])).value,
    "é",
  );
});

test("§3 constraint 7 requires binary32 quantised to 1/1024", () => {
  const f32 = (v) => {
    const b = new ArrayBuffer(4);
    new DataView(b).setFloat32(0, v);
    return Uint8Array.from([0xfa, ...new Uint8Array(b)]);
  };
  for (const ok of [0.0, 0.5, 1 / 1024, -3.25]) {
    assert.equal(smysl.decodeOne(f32(ok)).value, ok);
  }
  assert.throws(() => smysl.decodeOne(f32(0.1)), smysl.CborError);
  assert.throws(() => smysl.decodeOne(f32(Infinity)), smysl.CborError);
  assert.throws(() => smysl.decodeOne(hex("fb3ff0000000000000")), smysl.CborError); // binary64
});

test("§3 constraint 9 bounds nesting at 128", () => {
  assert.equal(smysl.MAX_NESTING, 128);
  smysl.decodeOne(Uint8Array.from([...Array(100).fill(0x81), 0x00]));
  assert.throws(
    () => smysl.decodeOne(Uint8Array.from([...Array(200).fill(0x81), 0x00])),
    smysl.CborError,
  );
});

test("§3 rejects a tag (major type 6)", () => {
  assert.throws(() => smysl.decodeOne(hex("c000")), smysl.CborError);
});

// -- §3.1  Record framing -----------------------------------------------------

test("§3.1 record type codes match the table", () => {
  assert.deepEqual(
    [...smysl.RECORD_NAMES.entries()],
    [[1, "unit"], [2, "attestation"], [3, "relation"], [4, "thread"], [5, "view"],
     [6, "contention"], [7, "pack_info"], [8, "schema_decl"], [9, "checkpoint"],
     [10, "label_binding"]],
  );
});

test("§3.1 a record is a two-element array", () => {
  assert.throws(() => smysl.decodeStore(hex("8301a000")), smysl.CborError);
  assert.throws(() => smysl.decodeStore(hex("a101a0")), smysl.CborError);
});

test("§3.1 an unknown type code is preserved and skipped, not rejected", () => {
  const raw = smysl.encodeOne([99, new Map([[0, "a later shape"]])]);
  const [rec] = smysl.decodeStore(raw);
  assert.equal(rec.isKnown, false);
  assert.equal(rec.name, "unknown(99)");
  assert.deepEqual(rec.reencode(), raw);
});

test("§3.1 an unknown record's body is still parsed strictly", () => {
  assert.throws(() => smysl.decodeStore(hex("8218639fff")), smysl.CborError);
});

test("§3.1 a store is a concatenation with no envelope", () => {
  const a = smysl.encodeOne([1, new Map([[0, "claim"], [1, "first"], [6, 1]])]);
  const b = smysl.encodeOne([1, new Map([[0, "claim"], [1, "second"], [6, 1]])]);
  const joined = Uint8Array.from([...a, ...b]);
  const recs = smysl.decodeStore(joined);
  assert.deepEqual(recs.map((r) => r.unitFields().get("gist")), ["first", "second"]);
  assert.deepEqual(smysl.encodeStore(recs), joined);
});

test("§3.1 a truncated trailing record is an error, not a silent truncation", () => {
  const good = smysl.encodeOne([1, new Map([[0, "claim"], [1, "g"], [6, 1]])]);
  assert.throws(() => smysl.decodeStore(Uint8Array.from([...good, 0x82, 0x01])), smysl.CborError);
});

// -- §7  Conformance ----------------------------------------------------------

test("§7 what C-Read provably cannot check, kept as a list", () => {
  // Emptying this should be a decision, not an accident. The largest entry is §2.3 — status
  // is part of identity — the paragraph the whole format rests on, which needs uids and
  // therefore C-Produce.
  const unreached = {
    "§2.1 uid derivation": "needs BLAKE3; C-Produce, not C-Read",
    "§2.3 status is part of identity": "follows from uid derivation",
    "§4 canonical surface form": "surface syntax is not decoded here",
    "§6 rules M, T, L, R, U, I, S, V, D": "semantic; C-Consume and above",
  };
  assert.ok(Object.keys(unreached).length);
  assert.ok("§2.3 status is part of identity" in unreached);
});
