// The record-set digest of SMYSL-2.7 §5.4, against the vectors the Rust produced.
//
// `fixtures/wire/rsd/cases.json` names each input and its digest. Order and repeats do not
// count, which `repeats.cbor` (F13 twice, the first time reversed) holds the implementation to.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { decodeStore } from "../src/records.js";
import { recordSetDigest } from "../src/export.js";
import { toHex } from "../src/uid.js";

const RSD = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "fixtures", "wire", "rsd");
const CASES = JSON.parse(readFileSync(join(RSD, "cases.json"), "utf8"));

test("there are vectors", () => {
  assert.ok(CASES.length > 0, "no rsd vectors; the suite would pass vacuously");
});

for (const c of CASES) {
  test(`record-set digest of ${c.file} matches the Rust`, () => {
    const records = decodeStore(new Uint8Array(readFileSync(join(RSD, c.file))));
    assert.equal(records.length, c.records);
    assert.equal(toHex(recordSetDigest(records.map((r) => r.raw))), c.rsd);
  });
}
