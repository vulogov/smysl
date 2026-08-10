package smysl

// BLAKE3 against the published vectors.
//
// This runs before anything that uses the hash, because if the hash is wrong then no uid
// derived from it is evidence of anything. The vectors are external ground truth — they come
// from the BLAKE3 specification, not from this repository — which matters: every other fixture
// here was produced by the Rust, so agreeing with it proves interoperability but could not
// catch both implementations being wrong the same way.

import (
	"encoding/hex"
	"testing"
)

// Input of length n is the bytes `i % 251`, as the specification defines its test inputs.
var blake3Vectors = map[int]string{
	0: "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
	1: "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
	2: "7b7015bb92cf0b318037702a6cdd81dee41224f734684c2c122cd6359cb1ee63",
	3: "e1be4d7a8ab5560aa4199eea339849ba8e293d55ca0a81006726d184519e647f",
	// The chunk boundary is 1024 bytes. These straddle it and exercise the tree: a
	// single-chunk shortcut passes every case above and fails from here down.
	1023: "10108970eeda3eb932baac1428c7a2163b0e924c9a9e25b35bba72b28f70bd11",
	1024: "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7",
	1025: "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444",
	2048: "e776b6028c7cd22a4d0ba182a8bf62205d2ef576467e838ed6f2529b85fba24a",
	3072: "b98cb0ff3623be03326b373de6b9095218513e64f1ee2edd2525c7ad1e5cffd2",
}

func vectorInput(n int) []byte {
	b := make([]byte, n)
	for i := range b {
		b[i] = byte(i % 251)
	}
	return b
}

func TestBlake3MatchesThePublishedVectors(t *testing.T) {
	if len(blake3Vectors) < 9 {
		t.Fatalf("the vector table is the whole check; got %d entries", len(blake3Vectors))
	}
	for n, want := range blake3Vectors {
		got := hex.EncodeToString(Blake3(vectorInput(n)))
		if got != want {
			t.Errorf("length %d:\n  got  %s\n  want %s", n, got, want)
		}
	}
}

// The incremental surface must agree with the one-shot one, whatever the split.
//
// `Update` is called once per fixture below and repeatedly by nothing, so a bug in the
// carry between calls would go unseen. Splitting across the block and chunk boundaries is
// where such a bug lives.
func TestUpdateIsSplitIndependent(t *testing.T) {
	data := vectorInput(3072)
	want := hex.EncodeToString(Blake3(data))
	for _, split := range []int{1, 63, 64, 65, 1023, 1024, 1025, 2047} {
		h := NewHasher()
		h.Update(data[:split])
		h.Update(data[split:])
		if got := hex.EncodeToString(h.Digest(outLen)); got != want {
			t.Errorf("split at %d:\n  got  %s\n  want %s", split, got, want)
		}
	}
}
