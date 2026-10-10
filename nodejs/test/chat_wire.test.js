// GE-T1's chat half: the identities of a chat corpus, recomputed here.
//
// `fixtures/library/wire/` gives this implementation a scripture-shaped expression whose records
// were hand-built in `smysl-core` — correctly, in a crate with no reader. `chat-wire` gives it
// the records a real `text add` emitted from three chat exports, which is where the identities
// are derived over things scripture has none of: segment tables with speakers, timestamps and
// platform ids in them, `mul` manifests, and `raw` metadata inside the mid.
//
// This implementation implements no reader and is not asked to. It decodes and hashes.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";

import * as smysl from "../src/index.js";
import { toHex } from "../src/uid.js";

const WIRE = new URL("../../fixtures/library/chat-wire/", import.meta.url);
const IDS = JSON.parse(fs.readFileSync(new URL("ids.json", WIRE), "utf8"));
const RECORDS = new Uint8Array(fs.readFileSync(new URL("records.cbor", WIRE)));

const records = () => smysl.decodeStore(RECORDS);
const only = (code) => records().filter((r) => r.code === code);

test("there are vectors to check", () => {
  assert.equal(IDS.expressions.length, 3, "one expression per export");
});

test("the chat fixture round trips", () => {
  assert.deepEqual([...smysl.encodeStore(records())], [...RECORDS]);
  const codes = [...new Set(records().map((r) => r.code))].sort((a, b) => a - b);
  assert.deepEqual(codes, [14, 15, 18]);
});

test("every identity is recomputed from the records", () => {
  // Derived from the records as they arrived, which is this language's own constraint: a
  // `binary32` zero decodes to `0` here and re-encodes shorter, so `Record.reencode()` returns
  // the arrival bytes. `canonicalBodyMatches` is the separate question, asked below.
  const byMid = new Map();
  for (const r of only(14)) {
    byMid.set(toHex(smysl.mid(smysl.bodyBytes(r))), smysl.Manifest.decode(r));
  }
  const texts = new Map(
    only(15).map((r) => {
      const p = smysl.PartText.decode(r);
      return [toHex(p.tid), p];
    }),
  );
  const readings = new Map(
    only(18).map((r) => {
      const g = smysl.PartReading.decode(r);
      return [toHex(g.rdid()), g];
    }),
  );

  for (const expression of IDS.expressions) {
    const manifest = byMid.get(expression.mid_hex);
    assert.ok(manifest, `no manifest hashes to ${expression.mid_hex}`);
    assert.equal(manifest.fields.get("reader"), expression.reader);
    assert.equal(manifest.fields.get("lang"), "mul", "a chat export is not one language");
    assert.equal(manifest.parts.length, expression.parts.length);

    manifest.parts.forEach((entry, i) => {
      const want = expression.parts[i];
      assert.equal(toHex(entry.tid), want.tid_hex);
      assert.equal(entry.length, want.length);
      assert.equal(toHex(entry.structure), want.structure_hex);
      assert.equal(toHex(entry.rdid), want.rdid_hex);

      const part = texts.get(want.tid_hex);
      assert.equal(toHex(smysl.tid(part.text)), want.tid_hex);
      assert.equal(part.text.length, want.length);

      const reading = readings.get(want.rdid_hex);
      assert.equal(toHex(reading.structureHash()), want.structure_hex);
      assert.equal(reading.rows().length, want.segments);
      reading.verifyEntry(entry);
    });
  }
});

test("a chat reading names who said what", () => {
  // A row's speaker, timestamp and ids are inside the structure hash and therefore inside the
  // rdid, so an implementation that dropped them would decode the record and disagree about the
  // identity.
  let speakers = 0;
  let observed = 0;
  let ids = 0;
  for (const r of only(18)) {
    for (const row of smysl.PartReading.decode(r).rows()) {
      if (row.has("speaker")) speakers += 1;
      if (row.has("observed")) observed += 1;
      if (row.has("ids")) ids += 1;
    }
  }
  assert.ok(speakers > 0, "a chat reading names its speakers");
  assert.ok(observed > 0, "and when each message was sent");
  assert.ok(ids > 0, "and the platform's own id for it");
});

test("raw metadata is inside the mid", () => {
  // Carried as opaque canonical CBOR, so nothing here decodes it — which is why it is worth
  // checking: an implementation that dropped an opaque key would re-encode a shorter body and
  // compute a different mid, with nothing else to notice.
  let withRaw = 0;
  const mids = new Set(IDS.expressions.map((e) => e.mid_hex));
  for (const r of only(14)) {
    if (!r.body.has(16)) continue;
    withRaw += 1;
    assert.ok(mids.has(toHex(smysl.mid(smysl.bodyBytes(r)))));
    assert.ok(smysl.canonicalBodyMatches(r), "the body is canonical as it arrived");
  }
  assert.equal(withRaw, 2, "telegram and slack carry `raw`; whatsapp has none");
});
