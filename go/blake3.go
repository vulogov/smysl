package smysl

// BLAKE3, written here rather than imported.
//
// This package takes one dependency — `x/text` for the NFC table §3 constraint 6 needs, which
// Python and JavaScript get from their standard libraries. The hash is a different matter.
//
// §2.1 is `uid = BLAKE3-256(canonical_cbor(unit_core))`, and the point of a second
// implementation is to answer "can somebody else derive the same uid from the specification
// alone". Reaching for a Go binding to the same C library the reference implementation uses
// would answer a weaker question: it would test two callers of one hash rather than two hashes.
// The Python package hand-rolled it for exactly this reason and says so; this follows it.
//
// It is slow, and that does not matter. It hashes unit cores, which are kilobytes, and it is
// checked against the published BLAKE3 vectors — external ground truth, not something this
// repository produced — before any uid derived from it is believed.
//
// The tree is implemented in full rather than the single-chunk case alone. A unit core with a
// long body exceeds 1024 bytes, and a single-chunk shortcut is correct on every small fixture
// and wrong on the first large one. `blake3_test.go` covers 1023, 1024, 1025, 2048 and 3072
// for that reason: the boundary, and either side of it.

import "encoding/binary"

const (
	blockLen = 64
	chunkLen = 1024
	outLen   = 32

	flagChunkStart = 1 << 0
	flagChunkEnd   = 1 << 1
	flagParent     = 1 << 2
	flagRoot       = 1 << 3
)

var iv = [8]uint32{
	0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
	0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
}

var msgPermutation = [16]int{2, 6, 3, 10, 7, 0, 4, 13, 1, 11, 12, 5, 9, 14, 15, 8}

func rotr32(x uint32, n uint) uint32 { return x>>n | x<<(32-n) }

func g(s *[16]uint32, a, b, c, d int, mx, my uint32) {
	s[a] = s[a] + s[b] + mx
	s[d] = rotr32(s[d]^s[a], 16)
	s[c] = s[c] + s[d]
	s[b] = rotr32(s[b]^s[c], 12)
	s[a] = s[a] + s[b] + my
	s[d] = rotr32(s[d]^s[a], 8)
	s[c] = s[c] + s[d]
	s[b] = rotr32(s[b]^s[c], 7)
}

func round(s *[16]uint32, m *[16]uint32) {
	// Columns.
	g(s, 0, 4, 8, 12, m[0], m[1])
	g(s, 1, 5, 9, 13, m[2], m[3])
	g(s, 2, 6, 10, 14, m[4], m[5])
	g(s, 3, 7, 11, 15, m[6], m[7])
	// Diagonals.
	g(s, 0, 5, 10, 15, m[8], m[9])
	g(s, 1, 6, 11, 12, m[10], m[11])
	g(s, 2, 7, 8, 13, m[12], m[13])
	g(s, 3, 4, 9, 14, m[14], m[15])
}

func permute(m *[16]uint32) {
	var out [16]uint32
	for i := 0; i < 16; i++ {
		out[i] = m[msgPermutation[i]]
	}
	*m = out
}

func compress(cv *[8]uint32, block *[16]uint32, counter uint64, blen uint32, flags uint32) [16]uint32 {
	state := [16]uint32{
		cv[0], cv[1], cv[2], cv[3], cv[4], cv[5], cv[6], cv[7],
		iv[0], iv[1], iv[2], iv[3],
		uint32(counter), uint32(counter >> 32), blen, flags,
	}
	m := *block
	for i := 0; i < 6; i++ {
		round(&state, &m)
		permute(&m)
	}
	round(&state, &m) // the seventh round does not permute afterwards

	for i := 0; i < 8; i++ {
		state[i] ^= state[i+8]
		state[i+8] ^= cv[i]
	}
	return state
}

// words reads a 64-byte block as sixteen little-endian words.
func words(b []byte) [16]uint32 {
	var w [16]uint32
	for i := 0; i < 16; i++ {
		w[i] = binary.LittleEndian.Uint32(b[i*4 : i*4+4])
	}
	return w
}

// output is a node's compression, deferred so the root can be flagged and extended.
type output struct {
	inputCV  [8]uint32
	block    [16]uint32
	counter  uint64
	blockLen uint32
	flags    uint32
}

func (o *output) chainingValue() [8]uint32 {
	full := compress(&o.inputCV, &o.block, o.counter, o.blockLen, o.flags)
	var cv [8]uint32
	copy(cv[:], full[:8])
	return cv
}

func (o *output) rootBytes(length int) []byte {
	out := make([]byte, 0, length+blockLen)
	for i := uint64(0); len(out) < length; i++ {
		w := compress(&o.inputCV, &o.block, i, o.blockLen, o.flags|flagRoot)
		var buf [4]byte
		for _, word := range w {
			binary.LittleEndian.PutUint32(buf[:], word)
			out = append(out, buf[:]...)
		}
	}
	return out[:length]
}

type chunkState struct {
	cv         [8]uint32
	counter    uint64
	block      []byte
	compressed int
	flags      uint32
}

func newChunkState(key [8]uint32, counter uint64, flags uint32) *chunkState {
	return &chunkState{cv: key, counter: counter, block: make([]byte, 0, blockLen), flags: flags}
}

func (c *chunkState) len() int { return blockLen*c.compressed + len(c.block) }

func (c *chunkState) startFlag() uint32 {
	if c.compressed == 0 {
		return flagChunkStart
	}
	return 0
}

func (c *chunkState) update(data []byte) {
	for len(data) > 0 {
		if len(c.block) == blockLen {
			w := words(c.block)
			full := compress(&c.cv, &w, c.counter, blockLen, c.flags|c.startFlag())
			copy(c.cv[:], full[:8])
			c.compressed++
			c.block = c.block[:0]
		}
		take := blockLen - len(c.block)
		if take > len(data) {
			take = len(data)
		}
		c.block = append(c.block, data[:take]...)
		data = data[take:]
	}
}

func (c *chunkState) output() *output {
	var padded [blockLen]byte
	copy(padded[:], c.block)
	return &output{
		inputCV:  c.cv,
		block:    words(padded[:]),
		counter:  c.counter,
		blockLen: uint32(len(c.block)),
		flags:    c.flags | c.startFlag() | flagChunkEnd,
	}
}

func parentOutput(left, right [8]uint32, key [8]uint32, flags uint32) *output {
	var block [16]uint32
	copy(block[:8], left[:])
	copy(block[8:], right[:])
	return &output{inputCV: key, block: block, counter: 0, blockLen: blockLen, flags: flagParent | flags}
}

// Hasher is incremental BLAKE3. `NewHasher().Update(b).Digest()` is the whole surface used here.
type Hasher struct {
	key     [8]uint32
	flags   uint32
	chunk   *chunkState
	cvStack [][8]uint32
}

func NewHasher() *Hasher {
	h := &Hasher{key: iv}
	h.chunk = newChunkState(h.key, 0, 0)
	return h
}

// push merges whenever the count of completed chunks has a trailing zero bit — the binary
// counter trick that keeps the stack the depth of the tree rather than its width.
func (h *Hasher) push(cv [8]uint32, totalChunks uint64) {
	for totalChunks&1 == 0 {
		last := h.cvStack[len(h.cvStack)-1]
		h.cvStack = h.cvStack[:len(h.cvStack)-1]
		cv = parentOutput(last, cv, h.key, h.flags).chainingValue()
		totalChunks >>= 1
	}
	h.cvStack = append(h.cvStack, cv)
}

func (h *Hasher) Update(data []byte) *Hasher {
	for len(data) > 0 {
		if h.chunk.len() == chunkLen {
			cv := h.chunk.output().chainingValue()
			counter := h.chunk.counter + 1
			h.push(cv, counter)
			h.chunk = newChunkState(h.key, counter, h.flags)
		}
		take := chunkLen - h.chunk.len()
		if take > len(data) {
			take = len(data)
		}
		h.chunk.update(data[:take])
		data = data[take:]
	}
	return h
}

func (h *Hasher) Digest(length int) []byte {
	out := h.chunk.output()
	for i := len(h.cvStack) - 1; i >= 0; i-- {
		out = parentOutput(h.cvStack[i], out.chainingValue(), h.key, h.flags)
	}
	return out.rootBytes(length)
}

// Blake3 is the 32-byte BLAKE3 hash of data.
func Blake3(data []byte) []byte {
	return NewHasher().Update(data).Digest(outLen)
}
