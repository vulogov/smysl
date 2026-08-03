// The shared rejection corpus.
//
// conformance_test.go checks that this implementation and the Rust agree on *accepting* four
// documents. That is the weaker half of the claim. §3 exists so that one value has exactly
// one encoding, and it enforces that entirely by refusing the alternatives — so agreement
// about what is *not* a smysl document is the half that carries the property.
//
// Before this corpus existed each implementation invented its own invalid inputs and no two
// used the same bytes, so nothing would have noticed if Go accepted something JavaScript
// rejected. Building it found the Rust walker accepting seven of these twenty-eight, with
// the consequence that a non-canonical extension payload could reach a uid.

package smysl

import (
	"encoding/json"
	"math"
	"os"
	"path/filepath"
	"testing"
)

const invalidDir = "../fixtures/wire/invalid"

type invalidCase struct {
	File       string `json:"file"`
	Constraint int    `json:"constraint"`
	Why        string `json:"why"`
}

func loadManifest(t *testing.T) []invalidCase {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join(invalidDir, "manifest.json"))
	if err != nil {
		t.Fatalf("the shared corpus is missing: %v", err)
	}
	var m struct {
		Cases []invalidCase `json:"cases"`
	}
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatalf("manifest is not readable: %v", err)
	}
	return m.Cases
}

// rejects reports whether the decoder refuses the input, or consumes less than all of it. A
// decoder that read the first valid-looking item and ignored a trailing violation would also
// be wrong, so a short read is not a pass.
func rejects(data []byte) bool {
	_, used, err := DecodeOne(data)
	return err != nil || used != len(data)
}

// Guards every other assertion in this file: a corpus that had gone missing would otherwise
// make the loop below vacuously true, which is the failure this project keeps finding.
func TestTheCorpusIsPresent(t *testing.T) {
	cases := loadManifest(t)
	if len(cases) < 28 {
		t.Fatalf("expected the shared corpus, found %d cases", len(cases))
	}
	for _, c := range cases {
		if _, err := os.ReadFile(filepath.Join(invalidDir, c.File)); err != nil {
			t.Errorf("manifest names a missing file %s: %v", c.File, err)
		}
	}
}

func TestEveryInvalidFixtureIsRejected(t *testing.T) {
	for _, c := range loadManifest(t) {
		c := c
		t.Run(c.File, func(t *testing.T) {
			data, err := os.ReadFile(filepath.Join(invalidDir, c.File))
			if err != nil {
				t.Fatalf("unreadable: %v", err)
			}
			if !rejects(data) {
				t.Errorf("accepted, but §3 constraint %d says otherwise: %s", c.Constraint, c.Why)
			}
		})
	}
}

// The control. If the decoder refused everything the test above would pass while meaning
// nothing, so canonical counterparts of the same shapes must still be accepted.
func TestTheCanonicalCounterpartsAreAccepted(t *testing.T) {
	quantised := make([]byte, 5)
	quantised[0] = 0xFA
	bits := math.Float32bits(0.5)
	quantised[1], quantised[2] = byte(bits>>24), byte(bits>>16)
	quantised[3], quantised[4] = byte(bits>>8), byte(bits)

	for _, tc := range []struct {
		what string
		data []byte
	}{
		{"shortest integer", []byte{0x01}},
		{"definite array", []byte{0x81, 0x01}},
		{"sorted map", []byte{0xA2, 0x00, 0x01, 0x01, 0x02}},
		// U+00E9 written as explicit bytes: a composed literal in Go source would be
		// normalised by the editor and the case would stop testing anything.
		{"composed text", []byte{0x62, 0xC3, 0xA9}},
		{"quantised float", quantised},
	} {
		if rejects(tc.data) {
			t.Errorf("%s is valid and was rejected", tc.what)
		}
	}
}
