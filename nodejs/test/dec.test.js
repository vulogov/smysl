// fixtures/tel/dec/cases.json: `dec` (RFC SMYSL-3 §5), C-Tel-Read.
//
// The generator beside the fixture is JavaScript too, so the binary64 cases agreeing here says
// only that this port and the generator share `String(x)`. What this file tests is everything
// else: the token reader, the spelling and the strict decoder, written from the RFC and held to
// a fixture that Rust and Python are held to as well.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { DecError, fromCbor, fromF64, parseToken, spell, toCbor } from "../src/dec.js";

const FIXTURE = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "fixtures", "tel", "dec");
const CASES = JSON.parse(readFileSync(join(FIXTURE, "cases.json"), "utf8"));
const hex = (s) => Uint8Array.from(s.match(/../g).map((b) => parseInt(b, 16)));
const toHex = (b) => Array.from(b, (x) => x.toString(16).padStart(2, "0")).join("");
const pair = (c) => ({ m: BigInt(c.m), e: c.e });

test("a binary64 is the decimal ECMA-262 selects", () => {
  assert.ok(CASES.f64.length > 2000);
  for (const c of CASES.f64) {
    const v = new DataView(new ArrayBuffer(8));
    v.setBigUint64(0, BigInt(`0x${c.bits}`));
    assert.deepEqual(fromF64(v.getFloat64(0)), pair(c), c.bits);
  }
});

test("tokens are read as the RFC says", () => {
  for (const c of CASES.tokens) {
    if (c.error) {
      assert.throws(() => parseToken(c.token), (err) => err instanceof DecError && err.kind === c.error, c.token);
    } else {
      assert.deepEqual(parseToken(c.token), pair(c), c.token);
    }
  }
});

test("one spelling and one encoding per value", () => {
  for (const c of CASES.spellings) {
    const v = pair(c);
    assert.equal(spell(v.m, v.e), c.text);
    assert.deepEqual(parseToken(c.text), v);
    assert.equal(toHex(toCbor(v.m, v.e)), c.cbor);
    assert.deepEqual(fromCbor(hex(c.cbor)), v);
  }
});

test("non-canonical CBOR is SMY-E602", () => {
  for (const c of CASES.refused) {
    assert.throws(() => fromCbor(hex(c.cbor)), (err) => err instanceof DecError && err.kind === "E602", c.why);
  }
});
