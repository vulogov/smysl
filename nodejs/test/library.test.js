// The library records and the four identities — §2.6 and §3.1, added in 1.10.
//
// Reading a document never requires deriving an identity, which is why this file exists: three
// independent readers round-tripped every library fixture byte for byte while knowing nothing
// about what a tid *is*. §2.6's claim — that a one-byte domain prefix keeps four kinds of
// identity apart from each other and from a uid — had been checked by exactly one
// implementation, the one that wrote it down.
//
// Two levels, and the second is the one that matters. Deriving an identity from the hex
// preimage the fixture hands over checks the hash. Deriving it from the record as decoded, and
// separately asking whether this implementation's *encoder* reproduces those body bytes,
// checks the half a producer needs — which a fixture of hex strings cannot reach.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";

import * as smysl from "../src/index.js";
import { blake3 } from "../src/blake3.js";
import { toHex } from "../src/uid.js";

const WIRE = new URL("../../fixtures/library/wire/", import.meta.url);
const IDS = JSON.parse(fs.readFileSync(new URL("ids.json", WIRE), "utf8"));
const RECORDS = new Uint8Array(fs.readFileSync(new URL("records.cbor", WIRE)));

const unhex = (h) => Uint8Array.from(h.match(/../g).map((x) => parseInt(x, 16)));
const records = () => smysl.decodeStore(RECORDS);
const only = (code) => records().filter((r) => r.code === code);

test("there are vectors to check", () => {
  // A suite with no fixtures passes vacuously, which is the failure this catches.
  assert.ok(IDS.manifest.mid_hex);
  assert.ok(IDS.parts.length >= 2, "one part would not exercise a second reading");
  assert.ok(RECORDS.length > 0);
});

test("the domain bytes are the record codes they name", () => {
  // §2.6: "The domain byte is the record type code the identity names." Not an arbitrary
  // table, and worth asserting rather than transcribing: a tid prefixed with the manifest's
  // byte would still produce stable, agreeing identities inside any one implementation and
  // would silently be a different format.
  assert.equal(smysl.TID_DOMAIN, 15);
  assert.equal(smysl.MID_DOMAIN, 14);
  assert.equal(smysl.DID_DOMAIN, 17);
  assert.equal(smysl.RDID_DOMAIN, 18);
  assert.deepEqual(IDS.domain_bytes, {
    tid: smysl.TID_DOMAIN,
    mid: smysl.MID_DOMAIN,
    did: smysl.DID_DOMAIN,
    rdid: smysl.RDID_DOMAIN,
  });
});

test("the identities come out of their preimages", () => {
  assert.equal(toHex(smysl.mid(unhex(IDS.manifest.body_hex))), IDS.manifest.mid_hex);
  for (const part of IDS.parts) {
    const text = unhex(part.text_hex);
    assert.equal(toHex(smysl.tid(text)), part.tid_hex);
    assert.equal(text.length, part.length);
    assert.equal(toHex(smysl.rdid(unhex(part.reading_body_hex))), part.rdid_hex);
  }
});

test("no two kinds of identity can collide", () => {
  // §2.6's claim, on a preimage shared by all four. The guarantee is not probabilistic and is
  // not about BLAKE3: the digests differ because their *inputs* differ in the first byte,
  // whatever the rest is. A construction that hashed the preimage alone and labelled the
  // result afterwards would pass every other test in this file.
  const shared = new TextEncoder().encode("the same bytes under four names");
  const four = [
    toHex(smysl.tid(shared)),
    toHex(smysl.mid(shared)),
    toHex(smysl.did(shared)),
    toHex(smysl.rdid(shared)),
  ];
  assert.equal(new Set(four).size, 4, four.join(" "));

  // And none is the uid of anything: a uid's preimage is a canonical CBOR map, whose first
  // byte is 0xa0-0xbf, and a domain byte is 0x0e-0x12. The ranges cannot overlap.
  for (const d of [smysl.TID_DOMAIN, smysl.MID_DOMAIN, smysl.DID_DOMAIN, smysl.RDID_DOMAIN]) {
    assert.ok(d < 0xa0);
  }

  // The structure hash is the one digest with no domain byte, because it names a table rather
  // than a record. It must not be computed as if it had one.
  assert.notEqual(toHex(smysl.structureHash(shared)), toHex(smysl.tid(shared)));
  assert.equal(toHex(smysl.structureHash(shared)), toHex(blake3(shared)));
});

test("the text form is the prefix and fifty-two characters", () => {
  // §2.6 and §2.1: 52 is canonical, 26 is a display abbreviation.
  const digest = unhex(IDS.manifest.mid_hex);
  const canonical = smysl.identityText(smysl.MID_PREFIX, digest);
  assert.ok(canonical.startsWith("m3:"));
  assert.equal(canonical.length, 3 + 52);
  const short = smysl.identityText(smysl.MID_PREFIX, digest, 26);
  assert.equal(short.length, 3 + 26);
  assert.ok(canonical.startsWith(short), "the short form is a prefix of the canonical one");
  assert.throws(() => smysl.identityText(smysl.TID_PREFIX, new Uint8Array(8)), smysl.LibraryError);
});

// -- the records themselves -------------------------------------------------------------

test("the record fixture round trips and names every library code", () => {
  const rs = records();
  assert.deepEqual([...smysl.encodeStore(rs)], [...RECORDS]);
  const codes = new Set(rs.map((r) => r.code));
  for (const code of [14, 15, 18, 19]) {
    assert.ok(codes.has(code), `record ${code} is not in it`);
  }
  // The fixture also carries codes this implementation still does not understand, which keeps
  // the distinction in UNDERSTOOD_RECORDS an observed fact rather than a claim.
  assert.ok([16, 17].some((c) => codes.has(c)));
  for (const r of rs) assert.equal(r.isKnown, smysl.UNDERSTOOD_RECORDS.has(r.code));
});

test("a redaction decodes into its named fields", () => {
  // Record 19, rule Z. The part it names is **not** in the fixture, which is the state
  // honouring a redaction leaves behind: the record remains and the bytes are gone.
  const r = smysl.Redaction.decode(only(19)[0]);
  assert.equal(toHex(r.tid), IDS.redaction.tid_hex);
  assert.equal(r.agent, IDS.redaction.agent);
  assert.equal(r.reason, null);
  const parts = new Set(IDS.parts.map((p) => p.tid_hex));
  assert.ok(!parts.has(IDS.redaction.tid_hex), "the part it names is not in the fixture");
});

test("a manifest decodes into its named fields", () => {
  const m = smysl.Manifest.decode(only(14)[0]);
  assert.equal(m.alias, IDS.manifest.alias);
  assert.equal(m.carry, "text");
  assert.equal(m.fields.get("reader"), "osis/1");
  assert.equal(m.fields.get("licence"), "public-domain");
  assert.ok(m.fields.get("part-policy"));
  assert.equal(m.parts.length, IDS.parts.length);
  m.parts.forEach((entry, i) => {
    assert.equal(toHex(entry.tid), IDS.parts[i].tid_hex);
    assert.equal(entry.length, IDS.parts[i].length);
    assert.equal(toHex(entry.structure), IDS.parts[i].structure_hex);
    assert.equal(toHex(entry.rdid), IDS.parts[i].rdid_hex);
  });
});

test("the mid is over the body's own bytes, and the encoder reproduces them", () => {
  // **The two halves, kept apart deliberately — and the reason is this language's alone.**
  //
  // `Record.reencode()` returns the bytes a record arrived in rather than re-encoding its
  // body, because JavaScript has one number type: a `binary32` zero decodes to `0`, and an
  // encoder asked to write `0` writes an integer. A manifest's key 16 is the reader's metadata
  // *verbatim* — an arbitrary map, which may hold a float — so a mid taken from a re-encoded
  // body would be a different mid for the same manifest, in this implementation only.
  //
  // So the identity is over the sliced body bytes, which is exactly the preimage §2.6 names.
  // Whether the encoder would have produced those bytes is a separate question with its own
  // answer, and it is the producer's one: a mismatch is a disagreement about canonical CBOR —
  // key order, a shortest-form head, an omitted optional — that would surface the first time
  // this implementation *wrote* a manifest.
  const record = only(14)[0];
  assert.equal(toHex(smysl.Manifest.decode(record).mid()), IDS.manifest.mid_hex);
  assert.ok(smysl.canonicalBodyMatches(record), "the encoder disagrees about canonical form");
  assert.deepEqual([...smysl.bodyBytes(record)], [...unhex(IDS.manifest.body_hex)]);
});

test("a part text verifies against its tid and its normalisation", () => {
  const parts = only(15).map((r) => smysl.PartText.decode(r));
  assert.equal(parts.length, IDS.parts.length);
  parts.forEach((part, i) => {
    assert.equal(toHex(part.tid), IDS.parts[i].tid_hex);
    assert.ok(part.verify(), "a fixture part must verify");
    assert.ok(smysl.isNormalised(part.text));
  });
  // One of them is Cyrillic, which is what makes the NFC half more than a formality: a reader
  // that normalised on the way in or out would move the tid.
  assert.ok(parts.some((p) => p.text.some((b) => b > 0x7f)));
});

test("a reading reproduces its rdid and its structure hash", () => {
  const readings = only(18).map((r) => smysl.PartReading.decode(r));
  assert.equal(readings.length, IDS.parts.length);
  readings.forEach((reading, i) => {
    assert.equal(toHex(reading.tid), IDS.parts[i].tid_hex);
    assert.equal(toHex(reading.rdid()), IDS.parts[i].rdid_hex);
    assert.equal(toHex(reading.structureHash()), IDS.parts[i].structure_hex);
    assert.equal(reading.reader, "osis/1");
    const rows = reading.rows();
    assert.ok(rows.length > 0);
    for (const row of rows) assert.ok(row.has("start") && row.has("locator"));
  });
});

test("a reading checks itself against the manifest entry", () => {
  // `SMY-E401`, which is why a part entry carries both identities: they move together when a
  // reader changes its segmentation and separately when it changes only its raw metadata.
  const manifest = smysl.Manifest.decode(only(14)[0]);
  const readings = only(18).map((r) => smysl.PartReading.decode(r));
  for (const entry of manifest.parts) {
    const reading = readings.find((r) => toHex(r.tid) === toHex(entry.tid));
    reading.verifyEntry(entry);
  }
  const entry = manifest.parts[0];
  const reading = readings.find((r) => toHex(r.tid) === toHex(entry.tid));
  const tampered = new smysl.PartEntry({
    tid: entry.tid,
    length: entry.length,
    structure: new Uint8Array(32),
    rdid: entry.rdid,
  });
  assert.throws(() => reading.verifyEntry(tampered), /SMY-E401/);
});

// -- the rules a decoder has to enforce -------------------------------------------------

test("a part whose bytes do not match its tid decodes and fails verification", () => {
  // §2.6: it MUST NOT fail the decode. "A record that cannot be decoded cannot be reported,
  // and one bad part would otherwise stop a whole store from opening." So the decode succeeds,
  // `verify` is false, and the caller is the one that raises `SMY-E446`.
  const body = new Map([
    [0, new Uint8Array(32)],
    [1, new TextEncoder().encode("not the bytes that hash to zero\n")],
  ]);
  const liar = smysl.encodeOne([15, body]);
  const [record] = smysl.decodeStore(liar);
  assert.ok(!smysl.PartText.decode(record).verify());
  assert.deepEqual([...record.reencode()], [...liar], "and it still round-trips");
});

test("unnormalised bytes fail verification even under their own tid", () => {
  // Both halves of `SMY-E446`, because either alone leaves a hole: bytes that hash to their
  // own tid but carry CRLF are a part no other library would name the same way.
  const crlf = new TextEncoder().encode("a line\r\nand another\n");
  const part = new smysl.PartText(smysl.tid(crlf), crlf);
  assert.ok(!part.verify());
  assert.ok(!smysl.isNormalised(crlf));
  assert.ok(!smysl.isNormalised(new TextEncoder().encode("﻿with a byte order mark\n")));
  // Decomposed: e + combining acute, which NFC composes.
  assert.ok(!smysl.isNormalised(new TextEncoder().encode("café\n")));
  assert.ok(smysl.isNormalised(new TextEncoder().encode("café\n")));
  // And an invalid UTF-8 sequence is not text at all.
  assert.ok(!smysl.isNormalised(Uint8Array.from([0xff, 0xfe, 0x0a])));
});

const base = () =>
  new Map([
    [0, "a"],
    [1, []],
    [2, "en"],
    [3, "txt/1"],
    [4, "unknown"],
    [5, 0],
    [17, "p"],
  ]);

test("manifest keys eleven and twelve travel together", () => {
  // §3.1: "either alone MUST be rejected, for the reason a resolution with one target is".
  smysl.Manifest.decode(base()); // the control
  const withParent = base();
  withParent.set(11, new Uint8Array(32));
  assert.throws(() => smysl.Manifest.decode(withParent), /travel together/);
  const withKind = base();
  withKind.set(12, "translation");
  assert.throws(() => smysl.Manifest.decode(withKind), /travel together/);
  const both = base();
  both.set(11, new Uint8Array(32));
  both.set(12, "translation");
  smysl.Manifest.decode(both);
});

test("lossy false is refused because it would give one manifest two mids", () => {
  // §3.1: key 15 has no `false` encoding. Admitting it would mean two byte strings for one
  // manifest, and therefore two mids for one expression.
  const falsey = base();
  falsey.set(15, false);
  assert.throws(() => smysl.Manifest.decode(falsey), /no `false` encoding/);
  const truthy = base();
  truthy.set(15, true);
  smysl.Manifest.decode(truthy);
  assert.notEqual(
    toHex(smysl.mid(smysl.encodeOne(base()))),
    toHex(smysl.mid(smysl.encodeOne(truthy))),
  );
});

test("a manifest missing a required key is refused", () => {
  for (const key of smysl.MANIFEST_REQUIRED) {
    const without = base();
    without.delete(key);
    assert.throws(() => smysl.Manifest.decode(without), smysl.LibraryError, `key ${key}`);
  }
});

test("carry outside its three values is refused", () => {
  const bad = base();
  bad.set(5, 3);
  assert.throws(() => smysl.Manifest.decode(bad), /carry/);
  assert.deepEqual([...smysl.CARRY.entries()], [
    [0, "none"],
    [1, "ref"],
    [2, "text"],
  ]);
});
