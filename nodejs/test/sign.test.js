// RFC SMYSL-7's golden vector, verified without the Rust: a detached COSE_Sign1 (RFC 9052)
// over an attestation's canonical CBOR, with Ed25519 from node:crypto. The envelope is taken
// apart by position and the Sig_structure built by hand, so nothing here trusts the encoder
// that wrote it.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createPublicKey, verify } from "node:crypto";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { blake3 } from "../src/blake3.js";

const here = dirname(fileURLToPath(import.meta.url));
const DIR = join(here, "..", "..", "fixtures", "sign");
// RFC 8032 §7.1 TEST 1's public key.
const PUBLIC = Buffer.from("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a", "hex");
// SubjectPublicKeyInfo for an Ed25519 key: the fixed 12-byte prefix, then the key.
const SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

const bstrHead = (n) => (n < 24 ? [0x40 | n] : n < 256 ? [0x58, n] : [0x59, n >> 8, n & 0xff]);

test("SMYSL-7: the golden signature verifies", () => {
  const payload = readFileSync(join(DIR, "attestation.cbor"));
  const cose = readFileSync(join(DIR, "signature.cose"));

  assert.deepEqual([...cose.subarray(0, 3)], [0xd2, 0x84, 0x58], "tag 18 over an array of four");
  const plen = cose[3];
  const prot = cose.subarray(4, 4 + plen);
  const rest = cose.subarray(4 + plen);
  assert.equal(rest[40], 0xf6, "a null payload: detached");
  assert.deepEqual([...rest.subarray(41, 43)], [0x58, 0x40]);
  assert.equal(rest.length, 43 + 64);
  const sig = rest.subarray(43);

  const want = Buffer.concat([
    Buffer.from([0xa5, 0x01, 0x27, 0x04, 0x50]),
    Buffer.from(blake3(PUBLIC)).subarray(0, 16),
    Buffer.from([0x3a, 0x00, 0x01, 0x00, 0x00, 0x6b]),
    Buffer.from("human:alice"),
    Buffer.from([0x3a, 0x00, 0x01, 0x00, 0x01, 0x6b]),
    Buffer.from("attestation"),
    Buffer.from([0x3a, 0x00, 0x01, 0x00, 0x02, 0x58, 0x20]),
    Buffer.from(blake3(payload)),
  ]);
  assert.deepEqual(prot, want, "the protected header");

  const msg = Buffer.concat([
    Buffer.from([0x84, 0x6a]),
    Buffer.from("Signature1"),
    Buffer.from(bstrHead(prot.length)),
    prot,
    Buffer.from(bstrHead(12)),
    Buffer.from("smysl.sign/1"),
    Buffer.from(bstrHead(payload.length)),
    payload,
  ]);
  const key = createPublicKey({ key: Buffer.concat([SPKI_PREFIX, PUBLIC]), format: "der", type: "spki" });
  assert.ok(verify(null, msg, key, sig), "the golden signature verifies");
  const bad = Buffer.from(msg);
  bad[bad.length - 1] ^= 1;
  assert.ok(!verify(null, bad, key, sig), "a changed record does not");
});
