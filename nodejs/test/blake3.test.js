// BLAKE3-256 against the published vectors.
//
// External ground truth, which is what makes this file different from every other test here.
// The rest of the suite checks this implementation against fixtures produced by the Rust; these
// digests come from the BLAKE3 project, so a shared misreading of `SMYSL_FORMAT_SPEC.md` cannot
// make them agree.

import { test } from "node:test";
import assert from "node:assert/strict";

import { Blake3, blake3 } from "../src/blake3.js";
import { toHex } from "../src/uid.js";

/** The specification's test input of length n is the bytes `i % 251`. */
function vectorInput(n) {
  const b = new Uint8Array(n);
  for (let i = 0; i < n; i++) b[i] = i % 251;
  return b;
}

const VECTORS = new Map([
  [0, "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"],
  [1, "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213"],
  [2, "7b7015bb92cf0b318037702a6cdd81dee41224f734684c2c122cd6359cb1ee63"],
  [3, "e1be4d7a8ab5560aa4199eea339849ba8e293d55ca0a81006726d184519e647f"],
  // The chunk boundary is 1024 bytes. A hasher that only ever compresses a single chunk passes
  // every vector above and produces garbage from here on, so these are the ones that matter:
  // 1023 is one short of a chunk, 1024 is exactly one, and 1025 is the first two-leaf tree.
  [1023, "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11"],
  [1024, "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7"],
  [1025, "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444"],
  // 2048 is a balanced two-chunk tree; 3072 is three chunks, where the stack has to merge an
  // unbalanced remainder rather than a clean pair.
  [2048, "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a"],
  [3072, "b98cb0ff3623be03326b373de6b9095218513e64f1ee2edd2525c7ad1e5cffd2"],
]);

for (const [n, want] of VECTORS) {
  test(`blake3 of ${n} bytes`, () => {
    assert.equal(toHex(blake3(vectorInput(n))), want);
  });
}

test("the digest does not depend on how the input was split", () => {
  // Buffering bugs live here rather than in the vectors above, which all arrive in one call.
  // A chunk is 1024 bytes and a block is 64, so a split landing inside either is the case that
  // catches a hasher holding the wrong amount of state between updates.
  const data = vectorInput(3072);
  const want = toHex(blake3(data));
  for (const at of [1, 63, 64, 65, 1023, 1024, 1025, 2047, 2048, 3071]) {
    const h = new Blake3();
    h.update(data.subarray(0, at));
    h.update(data.subarray(at));
    assert.equal(toHex(h.digest()), want, `split at ${at}`);
  }
});

test("byte-at-a-time agrees with all-at-once", () => {
  const data = vectorInput(1025);
  const h = new Blake3();
  for (const b of data) h.update(Uint8Array.of(b));
  assert.equal(toHex(h.digest()), toHex(blake3(data)));
});

test("extended output is a prefix relationship, not a rehash", () => {
  // §2.1 takes 32 bytes, but the XOF is where the ROOT flag and the output-block counter live,
  // and getting the counter wrong is invisible at exactly 32 bytes.
  const data = vectorInput(700);
  const long = blake3(data, 131);
  assert.equal(toHex(long.subarray(0, 32)), toHex(blake3(data)));
  assert.equal(long.length, 131);
});
