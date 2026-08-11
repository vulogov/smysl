// BLAKE3-256, hand-rolled, for §2.1: `uid = BLAKE3-256( canonical_cbor( unit_core ) )`.
//
// Hand-rolled for the reason the Python and Go packages give: a binding to the same C library
// the Rust calls would test two callers of one implementation, and prove nothing about whether
// the *specification* is enough to derive a uid from. Node has no BLAKE3 in its standard
// library, so there is no third option here anyway — but the choice would be the same if there
// were.
//
// This is the one file in the suite not written from `SMYSL_FORMAT_SPEC.md`. It implements
// BLAKE3 from the BLAKE3 specification, and §2.1 names that algorithm without describing it.
// The check is correspondingly different: the published test vectors are external ground truth,
// so this file is verified against something neither this repository nor its author controls.
//
// The vectors that matter are the ones straddling the 1024-byte chunk boundary. A hasher that
// only ever compresses one chunk passes every short vector and produces garbage the moment the
// tree has two leaves, which is the failure mode a suite of small inputs cannot see.

/** The SHA-256 initialisation vector, which BLAKE3 reuses. */
const IV = Uint32Array.of(
  0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
);

const MSG_PERMUTATION = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8];

const CHUNK_START = 1 << 0;
const CHUNK_END = 1 << 1;
const PARENT = 1 << 2;
const ROOT = 1 << 3;

const BLOCK_LEN = 64;
const CHUNK_LEN = 1024;

const rotr = (x, n) => ((x >>> n) | (x << (32 - n))) >>> 0;

function g(s, a, b, c, d, mx, my) {
  s[a] = (s[a] + s[b] + mx) >>> 0;
  s[d] = rotr(s[d] ^ s[a], 16);
  s[c] = (s[c] + s[d]) >>> 0;
  s[b] = rotr(s[b] ^ s[c], 12);
  s[a] = (s[a] + s[b] + my) >>> 0;
  s[d] = rotr(s[d] ^ s[a], 8);
  s[c] = (s[c] + s[d]) >>> 0;
  s[b] = rotr(s[b] ^ s[c], 7);
}

function round(s, m) {
  // Columns, then diagonals.
  g(s, 0, 4, 8, 12, m[0], m[1]);
  g(s, 1, 5, 9, 13, m[2], m[3]);
  g(s, 2, 6, 10, 14, m[4], m[5]);
  g(s, 3, 7, 11, 15, m[6], m[7]);
  g(s, 0, 5, 10, 15, m[8], m[9]);
  g(s, 1, 6, 11, 12, m[10], m[11]);
  g(s, 2, 7, 8, 13, m[12], m[13]);
  g(s, 3, 4, 9, 14, m[14], m[15]);
}

/**
 * The compression function: seven rounds, with the message permuted between each pair.
 *
 * Returns all sixteen words. Callers that want a chaining value take the first eight; the root
 * output needs the rest, so truncating here would work until the day someone asked for more
 * than 32 bytes of output.
 *
 * `counter` is a chunk index and is 64-bit in the specification. JavaScript numbers carry it
 * exactly to 2^53, which is 2^63 bytes of input, so it is split by division rather than by
 * shifting — `>>> 0` would silently truncate above 2^32 and the bug would only appear on
 * inputs nobody will hash.
 */
function compress(cv, blockWords, counter, blockLen, flags) {
  const s = new Uint32Array(16);
  s.set(cv, 0);
  s[8] = IV[0];
  s[9] = IV[1];
  s[10] = IV[2];
  s[11] = IV[3];
  s[12] = counter % 0x100000000;
  s[13] = Math.floor(counter / 0x100000000);
  s[14] = blockLen;
  s[15] = flags;

  let m = Uint32Array.from(blockWords);
  for (let r = 0; r < 7; r++) {
    round(s, m);
    if (r < 6) {
      const p = new Uint32Array(16);
      for (let i = 0; i < 16; i++) p[i] = m[MSG_PERMUTATION[i]];
      m = p;
    }
  }

  for (let i = 0; i < 8; i++) {
    s[i] = (s[i] ^ s[i + 8]) >>> 0;
    s[i + 8] = (s[i + 8] ^ cv[i]) >>> 0;
  }
  return s;
}

/** Sixteen little-endian words from a 64-byte block, zero-padded if the block is short. */
function wordsFromBlock(block) {
  const w = new Uint32Array(16);
  for (let i = 0; i < 16; i++) {
    const at = i * 4;
    w[i] = (block[at] | (block[at + 1] << 8) | (block[at + 2] << 16) | (block[at + 3] << 24)) >>> 0;
  }
  return w;
}

/** A compression that has not happened yet, kept so the root can be flagged differently. */
class Output {
  constructor(inputCv, blockWords, counter, blockLen, flags) {
    this.inputCv = inputCv;
    this.blockWords = blockWords;
    this.counter = counter;
    this.blockLen = blockLen;
    this.flags = flags;
  }

  chainingValue() {
    return compress(this.inputCv, this.blockWords, this.counter, this.blockLen, this.flags).slice(
      0,
      8,
    );
  }

  /**
   * The extended output, which is where ROOT is set.
   *
   * Only the root node carries the flag, and it is set *here* rather than when the node was
   * built — the same node is a chaining value to its parent and a root only if it turns out to
   * have no parent, which is not known until the input ends.
   */
  rootBytes(length) {
    const out = new Uint8Array(length);
    let at = 0;
    for (let block = 0; at < length; block++) {
      const words = compress(
        this.inputCv,
        this.blockWords,
        block,
        this.blockLen,
        this.flags | ROOT,
      );
      for (const word of words) {
        for (let b = 0; b < 4; b++) {
          if (at >= length) break;
          out[at++] = (word >>> (8 * b)) & 0xff;
        }
      }
    }
    return out;
  }
}

class ChunkState {
  constructor(key, chunkCounter, flags) {
    this.cv = Uint32Array.from(key);
    this.chunkCounter = chunkCounter;
    this.block = new Uint8Array(BLOCK_LEN);
    this.blockLen = 0;
    this.blocksCompressed = 0;
    this.flags = flags;
  }

  len() {
    return BLOCK_LEN * this.blocksCompressed + this.blockLen;
  }

  #startFlag() {
    return this.blocksCompressed === 0 ? CHUNK_START : 0;
  }

  update(input) {
    let off = 0;
    while (off < input.length) {
      // A full block is compressed only once something follows it. The last block of a chunk
      // has to carry CHUNK_END, and whether this block is the last is not known while it is
      // being filled — so it is held until the next byte arrives, or until output() is called.
      if (this.blockLen === BLOCK_LEN) {
        this.cv = compress(
          this.cv,
          wordsFromBlock(this.block),
          this.chunkCounter,
          BLOCK_LEN,
          this.flags | this.#startFlag(),
        ).slice(0, 8);
        this.blocksCompressed++;
        this.block.fill(0);
        this.blockLen = 0;
      }
      const take = Math.min(BLOCK_LEN - this.blockLen, input.length - off);
      this.block.set(input.subarray(off, off + take), this.blockLen);
      this.blockLen += take;
      off += take;
    }
  }

  output() {
    return new Output(
      this.cv,
      wordsFromBlock(this.block),
      this.chunkCounter,
      this.blockLen,
      this.flags | this.#startFlag() | CHUNK_END,
    );
  }
}

function parentOutput(left, right, key, flags) {
  const words = new Uint32Array(16);
  words.set(left, 0);
  words.set(right, 8);
  return new Output(key, words, 0, BLOCK_LEN, flags | PARENT);
}

export class Blake3 {
  constructor() {
    this.key = IV;
    this.flags = 0;
    this.chunkState = new ChunkState(IV, 0, 0);
    this.cvStack = [];
  }

  /**
   * Merge as many completed subtrees as the chunk count says are finished.
   *
   * The number of trailing zero bits in the *post-increment* chunk count is the number of
   * merges owed, which is what keeps the stack the size of a binary counter rather than the
   * size of the input. Tested by division rather than `& 1` for the reason compress() splits
   * its counter by division.
   */
  #addChunkChainingValue(cv, totalChunks) {
    let remaining = totalChunks;
    while (remaining % 2 === 0) {
      cv = parentOutput(this.cvStack.pop(), cv, this.key, this.flags).chainingValue();
      remaining = Math.floor(remaining / 2);
    }
    this.cvStack.push(cv);
  }

  update(input) {
    let off = 0;
    while (off < input.length) {
      if (this.chunkState.len() === CHUNK_LEN) {
        const cv = this.chunkState.output().chainingValue();
        const totalChunks = this.chunkState.chunkCounter + 1;
        this.#addChunkChainingValue(cv, totalChunks);
        this.chunkState = new ChunkState(this.key, totalChunks, this.flags);
      }
      const take = Math.min(CHUNK_LEN - this.chunkState.len(), input.length - off);
      this.chunkState.update(input.subarray(off, off + take));
      off += take;
    }
    return this;
  }

  digest(length = 32) {
    let out = this.chunkState.output();
    for (let i = this.cvStack.length - 1; i >= 0; i--) {
      out = parentOutput(this.cvStack[i], out.chainingValue(), this.key, this.flags);
    }
    return out.rootBytes(length);
  }
}

/** BLAKE3-256 of `input`, as the 32 bytes §2.1 hashes the unit core to. */
export function blake3(input, length = 32) {
  return new Blake3().update(input).digest(length);
}
