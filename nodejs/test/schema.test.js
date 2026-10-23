// FC-9 decoding and L9 against the fixtures the Rust wrote (SMYSL-2.5 §5.8, TX-P12 step 1).
//
// `fixtures/schema/fc9/cases.json` names, per file and declaration, the field count or
// `opaque`, each step's op tags and bytes, and the codes decoding reports; and the lens-chain
// hashes of the series. Lens application is not a conformance obligation, so nothing here
// applies a lens.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { encodeOne } from "../src/cbor.js";
import { decodeStore } from "../src/records.js";
import { chainHash, decodeSchemaDecl, encodeLenses } from "../src/schema.js";
import { toHex } from "../src/uid.js";

const FC9 = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "fixtures", "schema", "fc9");
const CASES = JSON.parse(readFileSync(join(FC9, "cases.json"), "utf8"));

const read = (name) => new Uint8Array(readFileSync(join(FC9, name)));

function decls(name) {
  const records = decodeStore(read(name));
  for (const r of records) assert.equal(r.code, 8);
  return records;
}

test("there are cases", () => {
  assert.ok(CASES.files.length > 0 && CASES.chains.length > 0, "no FC-9 cases");
});

for (const c of CASES.files) {
  test(`${c.file} decodes as the Rust says and writes back`, () => {
    const data = read(c.file);
    const records = decls(c.file);
    // From the body, not the kept bytes: the encoder has to reproduce them.
    const again = records.map((r) => encodeOne([r.code, r.body]));
    assert.deepEqual(Buffer.concat(again), Buffer.from(data));
    assert.equal(records.length, c.decls.length);
    records.forEach((r, i) => {
      const want = c.decls[i];
      assert.equal(r.body.get(0), want.id);
      assert.equal(Number(r.body.get(1)), want.version);
      const got = decodeSchemaDecl(r.body);
      assert.equal(got.fields, want.fields);
      assert.equal(got.lensesOpaque, want.lenses_opaque);
      assert.deepEqual(got.codes, want.codes);
      assert.deepEqual(
        got.steps.map((s) => [s.from, s.to, s.ops, toHex(s.bytes)]),
        want.steps.map((s) => [s.from, s.to, s.ops, s.bytes]),
      );
      // L9: key 5 written back from its steps is what arrived.
      if (r.body.has(5) && !got.lensesOpaque) {
        assert.deepEqual(encodeLenses(got.steps), encodeOne(r.body.get(5)));
      }
    });
  });
}

for (const ch of CASES.chains) {
  test(`chain ${ch.from}→${ch.to} hashes as the Rust's`, () => {
    const steps = decls("series.cbor").flatMap((r) => decodeSchemaDecl(r.body).steps);
    const path = [];
    let at = ch.from;
    while (at < ch.to) {
      const s = steps.find((x) => x.from === at);
      assert.ok(s, `no step from ${at}`);
      path.push(s);
      at = s.to;
    }
    assert.equal(at, ch.to);
    assert.equal(toHex(chainHash(ch.id, ch.from, ch.to, path)), ch.hash);
  });
}
