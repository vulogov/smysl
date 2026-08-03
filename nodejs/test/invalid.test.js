// The shared rejection corpus.
//
// `conformance.test.js` checks that this implementation and the Rust agree on *accepting*
// four documents. That is the weaker half of the claim. §3 exists so that one value has
// exactly one encoding, and it enforces that entirely by refusing the alternatives — so
// agreement about what is *not* a smysl document is the half that carries the property.
//
// Before this corpus existed each implementation invented its own invalid inputs and no two
// used the same bytes, so nothing would have noticed if JavaScript accepted something Python
// rejected. Building it found the Rust walker accepting seven of these twenty-eight, with
// the consequence that a non-canonical extension payload could reach a uid.

import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { decodeOne } from "../src/cbor.js";

const here = dirname(fileURLToPath(import.meta.url));
const CORPUS = join(here, "..", "..", "fixtures", "wire", "invalid");
const manifest = JSON.parse(readFileSync(join(CORPUS, "manifest.json"), "utf8"));

// Refusing, or consuming less than the whole file, both count: a decoder that read the first
// valid-looking item and ignored a trailing violation would also be wrong.
function rejects(data) {
  try {
    const { used } = decodeOne(data);
    return used !== data.length;
  } catch {
    return true;
  }
}

// Guards every other assertion in this file. A manifest gone missing would otherwise make
// the loop below vacuously true, which is the failure this project keeps finding.
test("the corpus is present", () => {
  assert.ok(manifest.cases.length >= 28, `only ${manifest.cases.length} cases`);
  for (const c of manifest.cases) {
    assert.ok(readFileSync(join(CORPUS, c.file)).length > 0, `empty: ${c.file}`);
  }
});

for (const c of manifest.cases) {
  test(`rejects ${c.file} (§3 constraint ${c.constraint})`, () => {
    const data = new Uint8Array(readFileSync(join(CORPUS, c.file)));
    assert.ok(rejects(data), `${c.file}: ${c.why}`);
  });
}

// The control. If the decoder threw on everything the loop above would pass while meaning
// nothing, so canonical counterparts of the same shapes must still be accepted.
const canonical = [
  ["shortest integer", [0x01]],
  ["definite array", [0x81, 0x01]],
  ["sorted map", [0xa2, 0x00, 0x01, 0x01, 0x02]],
  ["composed text", [0x62, 0xc3, 0xa9]],
  ["quantised float", [0xfa, 0x3f, 0x00, 0x00, 0x00]],
];

for (const [what, bytes] of canonical) {
  test(`accepts the canonical ${what}`, () => {
    assert.ok(!rejects(Uint8Array.from(bytes)), `${what} is valid and was rejected`);
  });
}
