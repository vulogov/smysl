// 1.4: relation identity and contention identity, against the vectors the Rust produced.
//
// A withdrawal names an edge by its rid and a resolution names a contention by its id. An
// implementation that derived either differently would read every other implementation's
// withdrawals and resolutions as naming nothing, so both are checked against
// fixtures/wire/relation-id and fixtures/wire/contention-id — digest and text apart.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { contentionId, relationId, toHex } from "../src/uid.js";
import { decodeStore, encodeStore } from "../src/records.js";

const wire = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "fixtures", "wire");
const load = (p) => JSON.parse(readFileSync(join(wire, p), "utf8")).cases;
const hex = (h) => Uint8Array.from(h.match(/../g).map((b) => parseInt(b, 16)));

test("there are vectors", () => {
  assert.ok(load("relation-id/cases.json").length > 0);
  assert.ok(load("contention-id/cases.json").length > 0);
});

for (const c of load("relation-id/cases.json")) {
  test(`rid: ${c.name}`, () => {
    assert.equal(toHex(relationId(c.kind, hex(c.from_hex), hex(c.to_hex))), c.rid_hex);
  });
}

for (const c of load("contention-id/cases.json")) {
  test(`contention id: ${c.name}`, () => {
    const { digest, id } = contentionId(c.kind, hex(c.over_hex), c.positions_hex.map(hex));
    assert.equal(toHex(digest), c.digest_hex, "the digest differs");
    assert.equal(id, c.id, "the digest agrees and the text does not");
  });
}

test("records 11 and 12 are known and round-trip", () => {
  const data = readFileSync(join(wire, "F10-lifecycle.cbor"));
  const records = decodeStore(new Uint8Array(data));
  const names = records.map((r) => r.name);
  assert.ok(names.includes("withdrawal") && names.includes("resolution"), names.join(","));
  assert.ok(records.every((r) => r.isKnown));
  assert.deepEqual(Buffer.from(encodeStore(records)), Buffer.from(data));
});
