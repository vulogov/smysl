// C-Produce: every uid in fixtures/wire/uid/cases.json, derived here and compared.
//
// The fixtures were produced by the Rust. Reproducing them is the whole claim — §2.1 is a
// function from a unit core to 32 bytes, and an implementation that agrees on the bytes agrees
// about identity, which is the one thing the format cannot be wrong about.
//
// Canonical bytes are checked *before* the hash, and separately, because a mismatch has two
// possible causes and they need different fixes. `core_bytes_hex` is in the fixture for exactly
// this reason: a wrong layout and a wrong hash are indistinguishable from the uid alone.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { STATUS, ShapeError, coreBytes, toHex, uid, uidShort, uidText, validate } from "../src/uid.js";
import { decodeOne } from "../src/cbor.js";

const here = dirname(fileURLToPath(import.meta.url));
const cases = JSON.parse(
  readFileSync(join(here, "..", "..", "fixtures", "wire", "uid", "cases.json"), "utf8"),
).cases;

/** The fixture's JSON shape, which spells optionals as `null` and bytes as hex. */
function fromFixture(c) {
  const k = c.core;
  return {
    schema: k.schema,
    gist: k.gist,
    body: k.body,
    detail: k.detail,
    deps: k.deps ?? [],
    grounds: k.grounds ?? [],
    status: k.status,
    source: k.source,
    payload: k.payload_hex ? Uint8Array.from(k.payload_hex.match(/../g).map((h) => parseInt(h, 16))) : null,
  };
}

test("the fixture file is the one this expects", () => {
  // A guard on the guard. Every assertion below is a loop over `cases`, and a loop over an
  // empty array passes — the same shape of vacuity `tests/doc_output.rs` records finding twice.
  assert.equal(cases.length, 16, "16 cases, or this file is testing something else");
});

for (const c of cases) {
  test(`${c.name}: canonical bytes`, () => {
    assert.equal(toHex(coreBytes(fromFixture(c))), c.core_bytes_hex);
  });

  test(`${c.name}: uid`, () => {
    assert.equal(toHex(uid(fromFixture(c))), c.uid_hex);
  });
}

test("§2.3 — status is part of identity", () => {
  // The claim the format rests on, and the one most likely to be implemented by accident. Two
  // units whose every other field agrees have unrelated uids.
  //
  // The pair is not status-only, and that is forced rather than sloppy: §7 requires grounds for
  // `inferred`, so a core differing *purely* in status would be one this package refuses to
  // derive a uid for at all. The weaker statement below is the strongest true one.
  const a = cases.find((x) => x.name === "status-pair-a");
  const b = cases.find((x) => x.name === "status-pair-b");
  assert.equal(a.core.gist, b.core.gist);
  assert.equal(a.core.schema, b.core.schema);
  assert.notEqual(a.core.status, b.core.status);
  assert.notEqual(toHex(uid(fromFixture(a))), toHex(uid(fromFixture(b))));

  // And the same gist under every authorable status is five different units.
  const six = cases.filter((x) => x.core.gist === "one gist, six statuses");
  assert.equal(six.length, 5, "five statuses are authorable; `unfounded` is not");
  const uids = new Set(six.map((x) => toHex(uid(fromFixture(x)))));
  assert.equal(uids.size, 5, "one gist, five statuses, five identities");
});

test("§3 constraint 6 — the gist is normalised at the encoder", () => {
  // `unicode-decomposed` records the gist as *authored*: e + U+0301, not the composed form.
  // The recorded bytes are of its NFC form, so this passes only if something normalised, and
  // the fixture and the composed case landing on one uid is the proof that it happened.
  const composed = cases.find((x) => x.name === "unicode-composed");
  const decomposed = cases.find((x) => x.name === "unicode-decomposed");
  assert.notEqual(decomposed.core.gist, composed.core.gist, "the fixture holds two spellings");
  assert.equal(decomposed.core.gist.normalize("NFC"), composed.core.gist);
  assert.equal(toHex(uid(fromFixture(decomposed))), toHex(uid(fromFixture(composed))));
});

test("the canonical core round-trips through the reader", () => {
  // C-Produce over C-Read: what this package emits, it must also accept. Encoding without
  // decoding could satisfy every fixture above while emitting bytes the decoder's own
  // constraint checks reject — ascending keys, shortest form, NFC.
  for (const c of cases) {
    const bytes = coreBytes(fromFixture(c));
    const { value, used } = decodeOne(bytes);
    assert.equal(used, bytes.length, `${c.name}: trailing bytes`);
    assert.equal(value.get(0), c.core.schema, `${c.name}: schema survived`);
    assert.equal(value.get(6), c.core.status, `${c.name}: status survived`);
  }
});

test("§2.2 — an empty set is omitted, not encoded empty", () => {
  // The gap this file's header calls gap 1. Both readings are defensible from the sentence
  // "required, MAY be empty"; only one reproduces the fixture, and the other is off by two
  // keys and therefore by a whole identity.
  const minimal = cases.find((x) => x.name === "minimal");
  const bytes = coreBytes(fromFixture(minimal));
  assert.equal(bytes[0], 0xa3, "a three-key map: schema, gist, status");
  const { value } = decodeOne(bytes);
  assert.equal(value.has(4), false, "no empty deps key");
  assert.equal(value.has(5), false, "no empty grounds key");
});

test("§2.2 — deps and grounds are sets: deduplicated and sorted", () => {
  const one = "01".repeat(32);
  const three = "03".repeat(32);
  const base = {
    schema: "claim",
    gist: "a claim resting on grounds",
    status: STATUS.inferred,
  };
  const shuffled = uid({ ...base, grounds: [three, one, three] });
  const sorted = uid({ ...base, grounds: [one, three] });
  assert.equal(toHex(shuffled), toHex(sorted), "order and repetition are not content");
});

test("§7 — the shape clause refuses what it must", () => {
  const ok = { schema: "claim", gist: "a claim", status: STATUS.speculative };
  assert.doesNotThrow(() => uid(ok));

  const refuses = [
    ["a gist", { ...ok, gist: "" }, "SMY-E031"],
    ["a whitespace-only gist", { ...ok, gist: "   \n " }, "SMY-E031"],
    ["a schema", { ...ok, schema: "" }, "SMY-E030"],
    ["an authored unfounded", { ...ok, status: STATUS.unfounded }, "SMY-E034"],
    ["inferred without grounds", { ...ok, status: STATUS.inferred }, "SMY-E032"],
    ["derived without grounds", { ...ok, status: STATUS.derived }, "SMY-E032"],
    ["cited without a source", { ...ok, status: STATUS.cited }, "SMY-E035"],
    ["measured without a source", { ...ok, status: STATUS.measured }, "SMY-E035"],
    ["a status outside the six", { ...ok, status: 9 }, "SMY-E033"],
  ];
  for (const [what, core, code] of refuses) {
    assert.throws(() => uid(core), (e) => e instanceof ShapeError && e.code === code, what);
  }
});

test("§7 — a refused unit gets no identity at all", () => {
  // Not merely "validate reports it". The class is about *emitting*, so the failure has to be
  // on the path that hands out a uid — validating on request while `uid()` derived anyway
  // would satisfy the test above and miss the obligation.
  const unfounded = { schema: "claim", gist: "a claim", status: STATUS.unfounded };
  assert.throws(() => uid(unfounded), ShapeError);
  assert.throws(() => validate(unfounded), ShapeError);
});

test("§2.1 — the text form is 52 characters after `b3:`, and 26 abbreviated", () => {
  const bytes = uid({ schema: "claim", gist: "a claim", status: STATUS.speculative });
  const full = uidText(bytes);
  assert.equal(full.length, 3 + 52);
  assert.match(full, /^b3:[a-z2-7]{52}$/);
  assert.equal(uidShort(bytes), full.slice(0, 3 + 26), "the display form is a prefix");
});
