"""BLAKE3, in pure Python.

Why hand-rolled rather than `pip install blake3`
------------------------------------------------

This package has no dependencies, on purpose. It exists to answer "can somebody else implement
this format from the specification alone", and an implementation that reaches for a binding to
the same C library the reference implementation uses answers a weaker question. Taking the
hash from a package would leave §2.1 tested by two callers of one implementation.

It is slow — a few hundred kilobytes a second — and that is fine. It hashes unit cores, which
are kilobytes, and it is checked against fixtures the Rust produced rather than raced against
it.

Follows the reference implementation in the BLAKE3 specification. The tree structure is
implemented in full rather than only the single-chunk case: a unit core carrying a long body
exceeds 1024 bytes, and a single-chunk shortcut would be correct on every small fixture and
wrong on the first large one — which is exactly the kind of gap this project keeps finding.
"""

from __future__ import annotations

OUT_LEN = 32
KEY_LEN = 32
BLOCK_LEN = 64
CHUNK_LEN = 1024

CHUNK_START = 1 << 0
CHUNK_END = 1 << 1
PARENT = 1 << 2
ROOT = 1 << 3

IV = [
    0x6A09E667,
    0xBB67AE85,
    0x3C6EF372,
    0xA54FF53A,
    0x510E527F,
    0x9B05688C,
    0x1F83D9AB,
    0x5BE0CD19,
]

MSG_PERMUTATION = [2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8]


def _mask32(x: int) -> int:
    return x & 0xFFFFFFFF


def _add32(x: int, y: int) -> int:
    return _mask32(x + y)


def _rotr32(x: int, n: int) -> int:
    return _mask32(x << (32 - n)) | (x >> n)


def _g(state: list[int], a: int, b: int, c: int, d: int, mx: int, my: int) -> None:
    state[a] = _add32(state[a], _add32(state[b], mx))
    state[d] = _rotr32(state[d] ^ state[a], 16)
    state[c] = _add32(state[c], state[d])
    state[b] = _rotr32(state[b] ^ state[c], 12)
    state[a] = _add32(state[a], _add32(state[b], my))
    state[d] = _rotr32(state[d] ^ state[a], 8)
    state[c] = _add32(state[c], state[d])
    state[b] = _rotr32(state[b] ^ state[c], 7)


def _round(state: list[int], m: list[int]) -> None:
    # Columns.
    _g(state, 0, 4, 8, 12, m[0], m[1])
    _g(state, 1, 5, 9, 13, m[2], m[3])
    _g(state, 2, 6, 10, 14, m[4], m[5])
    _g(state, 3, 7, 11, 15, m[6], m[7])
    # Diagonals.
    _g(state, 0, 5, 10, 15, m[8], m[9])
    _g(state, 1, 6, 11, 12, m[10], m[11])
    _g(state, 2, 7, 8, 13, m[12], m[13])
    _g(state, 3, 4, 9, 14, m[14], m[15])


def _permute(m: list[int]) -> list[int]:
    return [m[MSG_PERMUTATION[i]] for i in range(16)]


def _compress(
    chaining_value: list[int],
    block_words: list[int],
    counter: int,
    block_len: int,
    flags: int,
) -> list[int]:
    state = [
        *chaining_value,
        IV[0],
        IV[1],
        IV[2],
        IV[3],
        _mask32(counter),
        _mask32(counter >> 32),
        block_len,
        flags,
    ]
    block = list(block_words)
    for _ in range(6):
        _round(state, block)
        block = _permute(block)
    _round(state, block)  # the seventh round does not permute afterwards

    for i in range(8):
        state[i] ^= state[i + 8]
        state[i + 8] ^= chaining_value[i]
    return state


def _words(b: bytes) -> list[int]:
    return [int.from_bytes(b[i : i + 4], "little") for i in range(0, len(b), 4)]


class _Output:
    """A node's compression, deferred so the root can be flagged and extended."""

    def __init__(self, cv, block_words, counter, block_len, flags):
        self.input_chaining_value = cv
        self.block_words = block_words
        self.counter = counter
        self.block_len = block_len
        self.flags = flags

    def chaining_value(self) -> list[int]:
        return _compress(
            self.input_chaining_value,
            self.block_words,
            self.counter,
            self.block_len,
            self.flags,
        )[:8]

    def root_output_bytes(self, length: int) -> bytes:
        out = bytearray()
        i = 0
        while len(out) < length:
            words = _compress(
                self.input_chaining_value,
                self.block_words,
                i,
                self.block_len,
                self.flags | ROOT,
            )
            for w in words:
                out += _mask32(w).to_bytes(4, "little")
            i += 1
        return bytes(out[:length])


class _ChunkState:
    def __init__(self, key_words: list[int], chunk_counter: int, flags: int):
        self.chaining_value = list(key_words)
        self.chunk_counter = chunk_counter
        self.block = b""
        self.blocks_compressed = 0
        self.flags = flags

    def len(self) -> int:
        return BLOCK_LEN * self.blocks_compressed + len(self.block)

    def start_flag(self) -> int:
        return CHUNK_START if self.blocks_compressed == 0 else 0

    def update(self, data: bytes) -> None:
        while data:
            if len(self.block) == BLOCK_LEN:
                self.chaining_value = _compress(
                    self.chaining_value,
                    _words(self.block),
                    self.chunk_counter,
                    BLOCK_LEN,
                    self.flags | self.start_flag(),
                )[:8]
                self.blocks_compressed += 1
                self.block = b""
            take = min(BLOCK_LEN - len(self.block), len(data))
            self.block += data[:take]
            data = data[take:]

    def output(self) -> _Output:
        padded = self.block + b"\x00" * (BLOCK_LEN - len(self.block))
        return _Output(
            self.chaining_value,
            _words(padded),
            self.chunk_counter,
            len(self.block),
            self.flags | self.start_flag() | CHUNK_END,
        )


def _parent_output(left: list[int], right: list[int], key_words, flags) -> _Output:
    return _Output(list(key_words), left + right, 0, BLOCK_LEN, PARENT | flags)


class Hasher:
    """Incremental BLAKE3. `Hasher().update(b).digest()` is the whole surface used here."""

    def __init__(self, key_words: list[int] | None = None, flags: int = 0):
        self.key_words = list(key_words) if key_words else list(IV)
        self.flags = flags
        self.chunk_state = _ChunkState(self.key_words, 0, flags)
        self.cv_stack: list[list[int]] = []

    def _push(self, cv: list[int], total_chunks: int) -> None:
        # Merge whenever the count of completed chunks has a trailing zero bit — the standard
        # binary-counter trick that keeps the stack the depth of the tree rather than its width.
        while total_chunks & 1 == 0:
            cv = _parent_output(self.cv_stack.pop(), cv, self.key_words, self.flags).chaining_value()
            total_chunks >>= 1
        self.cv_stack.append(cv)

    def update(self, data: bytes) -> "Hasher":
        while data:
            if self.chunk_state.len() == CHUNK_LEN:
                cv = self.chunk_state.output().chaining_value()
                counter = self.chunk_state.chunk_counter + 1
                self._push(cv, counter)
                self.chunk_state = _ChunkState(self.key_words, counter, self.flags)
            take = min(CHUNK_LEN - self.chunk_state.len(), len(data))
            self.chunk_state.update(data[:take])
            data = data[take:]
        return self

    def digest(self, length: int = OUT_LEN) -> bytes:
        out = self.chunk_state.output()
        for cv in reversed(self.cv_stack):
            out = _parent_output(cv, out.chaining_value(), self.key_words, self.flags)
        return out.root_output_bytes(length)


def blake3(data: bytes) -> bytes:
    """The 32-byte BLAKE3 hash of `data`."""
    return Hasher().update(data).digest()
