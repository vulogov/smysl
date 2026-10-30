// Export collections, read by a third implementation (SMYSL-2.7 §5.4, TX-P11 step 11).
//
// `recordSetDigest` is the spec's §2.7 digest over the raw bytes every decoded record keeps;
// `fixtures/wire/rsd/` holds vectors the Rust produced. `readExportIndex` reads an index from
// generic CBOR and SMYSL-2.7 §4.2's field numbers. `verifyExport` checks each file against its
// BLAKE3, record count and record-set digest, that every record re-encodes to the bytes it was
// read from, and the collection's digest.
//
//   node src/export.js verify DIR   one JSON line; exit 0 when the collection verifies
//   node src/export.js write DIR    what this implementation produces, for the Rust to import

import { readFileSync, writeFileSync, mkdirSync, readdirSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import { blake3 } from "./blake3.js";
import { decodeOne, encodeOne } from "./cbor.js";
import { decodeStore, encodeStore } from "./records.js";
import { uid, toHex } from "./uid.js";

export const INDEX_FILE = "export-index.cbor";
export const INDEX_VERSION = "smysl/export-index/1";

function concat(parts) {
  const total = parts.reduce((n, p) => n + p.length, 0);
  const out = new Uint8Array(total);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

function compare(a, b) {
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    if (a[i] !== b[i]) return a[i] - b[i];
  }
  return a.length - b.length;
}

function equal(a, b) {
  return a.length === b.length && compare(a, b) === 0;
}

/** §2.7: BLAKE3 over "smysl/rsd/1" ‖ 0x00 and the sorted unique hashes of the records. */
export function recordSetDigest(raws) {
  const seen = new Map();
  for (const r of raws) {
    const h = blake3(r);
    seen.set(toHex(h), h);
  }
  const hashes = [...seen.values()].sort(compare);
  const prefix = new TextEncoder().encode("smysl/rsd/1");
  return blake3(concat([prefix, new Uint8Array([0]), ...hashes]));
}

/** The index's fields by name. Throws on anything that is not one. */
export function readExportIndex(data) {
  const { value, used } = decodeOne(data);
  if (used !== data.length || !(value instanceof Map)) {
    throw new Error("an export index is one CBOR map");
  }
  if (value.get(0) !== INDEX_VERSION) throw new Error(`not ${INDEX_VERSION}`);
  const files = (value.get(1) ?? []).map((f) => ({
    path: f.get(0),
    blake3: f.get(1),
    records: f.get(2),
    rsd: f.get(3),
    view: f.get(4) ?? null,
    requires: f.get(5) ?? [],
    role: f.get(6),
  }));
  return {
    files,
    scope: value.get(2) ?? "",
    layout: value.get(6),
    closure: value.get(7),
    format: value.get(8),
    rsd: value.get(11),
    flags: value.get(12) ?? [],
    complete: value.get(14),
  };
}

/** Check a collection on disk. `ok` is true when every check holds. */
export function verifyExport(dir) {
  const index = readExportIndex(new Uint8Array(readFileSync(join(dir, INDEX_FILE))));
  const problems = [];
  const raws = [];
  let surface = false;
  for (const f of index.files) {
    const p = join(dir, f.path);
    if (!existsSync(p)) {
      problems.push(`${f.path}: absent`);
      continue;
    }
    const data = new Uint8Array(readFileSync(p));
    if (!equal(blake3(data), f.blake3)) {
      problems.push(`${f.path}: its BLAKE3 is not the index's`);
      continue;
    }
    if (f.path.endsWith(".smy")) {
      // A surface file is checked by its hash; reading surface text is the Rust's.
      surface = true;
      continue;
    }
    const records = decodeStore(data);
    if (!equal(encodeStore(records), data)) {
      problems.push(`${f.path}: does not re-encode to its own bytes`);
    }
    if (records.length !== Number(f.records)) {
      problems.push(`${f.path}: ${records.length} records, the index says ${f.records}`);
    }
    if (!equal(recordSetDigest(records.map((r) => r.raw)), f.rsd)) {
      problems.push(`${f.path}: its record-set digest is not the index's`);
    }
    for (const r of records) raws.push(r.raw);
  }
  const rsd = recordSetDigest(raws);
  if (!surface && !equal(rsd, index.rsd)) {
    problems.push("the collection's record-set digest is not the index's");
  }
  return { ok: problems.length === 0, files: index.files.length, rsd: toHex(rsd), problems };
}

/** What this implementation writes for the Rust to import: every wire fixture re-encoded by
 * this package, and a small store built here, two units and an edge between them. */
export function writeProduced(out) {
  mkdirSync(out, { recursive: true });
  const written = [];
  const wire = join(dirname(fileURLToPath(import.meta.url)), "..", "..", "fixtures", "wire");
  for (const name of readdirSync(wire).filter((n) => n.endsWith(".cbor")).sort()) {
    const data = new Uint8Array(readFileSync(join(wire, name)));
    writeFileSync(join(out, name), encodeStore(decodeStore(data)));
    written.push(name);
  }
  const a = { schema: "claim", gist: "Written by the JavaScript implementation.", status: 1 };
  const b = { schema: "claim", gist: "And grounded on nothing but its own say-so.", status: 1 };
  const unit = (core) => {
    const m = new Map([
      [0, core.schema],
      [1, core.gist],
      [6, core.status],
    ]);
    return encodeOne([1, m]);
  };
  const edge = encodeOne([
    3,
    new Map([
      [0, "x.xi/related"],
      [1, uid(a)],
      [2, uid(b)],
    ]),
  ]);
  writeFileSync(join(out, "nodejs-small.cbor"), concat([unit(a), unit(b), edge]));
  written.push("nodejs-small.cbor");
  return written;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const [cmd, dir] = process.argv.slice(2);
  if (cmd === "verify" && dir) {
    const r = verifyExport(dir);
    console.log(JSON.stringify(r));
    process.exit(r.ok ? 0 : 1);
  } else if (cmd === "write" && dir) {
    for (const n of writeProduced(dir)) console.log(n);
  } else {
    console.error("usage: node src/export.js verify DIR | write DIR");
    process.exit(2);
  }
}
