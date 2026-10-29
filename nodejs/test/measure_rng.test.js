// The measure bootstrap generator (SMYSL-2.6 §3.5, OQ-57), reproduced from the spec alone.
//
// The Rust draws from `rand_chacha`. What the spec promises is narrower and portable: the stream
// is RFC 8439's ChaCha20 keystream under a 256-bit key, block counter 0 and a zero nonce, read as
// successive little-endian 64-bit words. The key is BLAKE3("smysl/measure-rng/1\0" ‖ recipe hash
// ‖ seed as eight little-endian bytes). So any standard ChaCha20 reproduces it — here Node's
// own, which is OpenSSL's — and nothing of a crate's internals has to be ported.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import crypto from "node:crypto";

import { blake3 } from "../src/blake3.js";

const here = dirname(fileURLToPath(import.meta.url));
const golden = Object.fromEntries(
  readFileSync(join(here, "..", "..", "fixtures", "measure", "golden.tsv"), "utf8")
    .split("\n")
    .filter((l) => l && !l.startsWith("#"))
    .map((l) => l.split("\t")),
);

function draws(recipeHash, seed, n) {
  const seedBytes = Buffer.alloc(8);
  seedBytes.writeBigUInt64LE(BigInt(seed));
  const key = Buffer.from(
    blake3(Buffer.concat([Buffer.from("smysl/measure-rng/1\0", "latin1"), recipeHash, seedBytes])),
  );
  const keystream = crypto.createCipheriv("chacha20", key, Buffer.alloc(16)).update(Buffer.alloc(8 * n));
  return Array.from({ length: n }, (_, i) => keystream.readBigUInt64LE(8 * i));
}

test("the first four draws for a zero recipe hash and seed 0 are the Rust's", () => {
  const got = draws(Buffer.alloc(32), 0, 4)
    .map((x) => x.toString(16).padStart(16, "0"))
    .join("");
  assert.equal(got, golden["rng.zero.first4"]);
});
