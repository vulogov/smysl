package smysl_test

// C-Read conformance, and the specification walked clause by clause.
//
// Two questions, as in the Python and JavaScript packages. Do we agree with the Rust —
// fixtures in, byte-identical bytes out. And do we do what the *document* says, section by
// section. A library can pass the first and fail the second.

import (
	"bytes"
	"encoding/binary"
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"

	smysl "github.com/vulogov/smysl/go"
)

const wireDir = "../fixtures/wire"
const specPath = "../Documentation/SMYSL_FORMAT_SPEC.md"

func fixtures(t *testing.T) []string {
	t.Helper()
	paths, err := filepath.Glob(filepath.Join(wireDir, "*.cbor"))
	if err != nil {
		t.Fatal(err)
	}
	if len(paths) == 0 {
		t.Fatal("no .cbor fixtures; the suite would pass vacuously")
	}
	return paths
}

func hex(t *testing.T, s string) []byte {
	t.Helper()
	out := make([]byte, 0, len(s)/2)
	for i := 0; i < len(s); i += 2 {
		var b byte
		for _, c := range s[i : i+2] {
			b <<= 4
			switch {
			case c >= '0' && c <= '9':
				b |= byte(c - '0')
			case c >= 'a' && c <= 'f':
				b |= byte(c-'a') + 10
			}
		}
		out = append(out, b)
	}
	return out
}

func TestSpecificationIsWhereWeThinkItIs(t *testing.T) {
	body, err := os.ReadFile(specPath)
	if err != nil {
		t.Fatalf("%s: %v", specPath, err)
	}
	if !strings.Contains(string(body), "Deterministic CBOR") {
		t.Fatal("the specification does not look like the specification")
	}
}

// -- C-Read: agreement with the reference ------------------------------------

func TestStoresReencodeByteIdentically(t *testing.T) {
	for _, path := range fixtures(t) {
		t.Run(filepath.Base(path), func(t *testing.T) {
			data, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			records, err := smysl.DecodeStore(data)
			if err != nil {
				t.Fatalf("decode: %v", err)
			}
			if len(records) == 0 {
				t.Fatal("decoded to nothing")
			}
			out, err := smysl.EncodeStore(records)
			if err != nil {
				t.Fatalf("encode: %v", err)
			}
			if !bytes.Equal(out, data) {
				for i := range data {
					if i >= len(out) || out[i] != data[i] {
						t.Fatalf("byte %d differs: original 0x%02x, re-encoded 0x%02x",
							i, data[i], out[i])
					}
				}
				t.Fatalf("length differs: original %d, re-encoded %d", len(data), len(out))
			}
			// Record by record too, so a compensating pair of errors cannot hide.
			for n, r := range records {
				again, err := r.Reencode()
				if err != nil {
					t.Fatalf("record %d (%s): %v", n, r.Name(), err)
				}
				if !bytes.Equal(again, r.Raw) {
					t.Fatalf("record %d (%s) changed", n, r.Name())
				}
			}
		})
	}
}

// -- §2.2  What is hashed -----------------------------------------------------

func TestUnitCoreKeysMatchTheTable(t *testing.T) {
	want := map[uint64]string{
		0: "schema", 1: "gist", 2: "body", 3: "detail", 4: "deps",
		5: "grounds", 6: "status", 7: "source", 8: "payload",
	}
	if len(smysl.UnitKeys) != len(want) {
		t.Fatalf("key count: got %d, want %d", len(smysl.UnitKeys), len(want))
	}
	for k, v := range want {
		if smysl.UnitKeys[k] != v {
			t.Errorf("key %d: got %q, want %q", k, smysl.UnitKeys[k], v)
		}
	}
}

func TestUnknownKeysAtNineAndAboveSurvive(t *testing.T) {
	body := &smysl.Map{Entries: []smysl.Pair{
		{Key: uint64(0), Value: "claim"},
		{Key: uint64(1), Value: "a gist"},
		{Key: uint64(6), Value: uint64(1)},
		{Key: uint64(9), Value: "a later addition"},
	}}
	raw, err := smysl.EncodeOne([]any{uint64(1), body})
	if err != nil {
		t.Fatal(err)
	}
	records, err := smysl.DecodeStore(raw)
	if err != nil {
		t.Fatal(err)
	}
	again, _ := records[0].Reencode()
	if !bytes.Equal(again, raw) {
		t.Fatal("an unknown key did not survive a round trip")
	}
	m := records[0].Body.(*smysl.Map)
	if v, ok := m.Get(uint64(9)); !ok || v != "a later addition" {
		t.Fatalf("unknown key 9 lost: %v", v)
	}
}

// -- §3  Deterministic CBOR, one per constraint -------------------------------

func TestForbiddenEncodingsAreRejected(t *testing.T) {
	cases := map[string]string{
		"non-shortest one-byte":   "1817",
		"non-shortest two-byte":   "1900ff",
		"non-shortest four-byte":  "1a0000ffff",
		"indefinite-length array": "9fff",
		"indefinite-length map":   "bfff",
		"indefinite-length bytes": "5fff",
		"null":                    "f6",
		"binary64 float":          "fb3ff0000000000000",
		"map keys out of order":   "a201000000",
		"duplicate map key":       "a200000000",
		"a tag (constraint 8)":    "c000",
	}
	for why, encoded := range cases {
		if _, _, err := smysl.DecodeOne(hex(t, encoded)); err == nil {
			t.Errorf("%s was accepted", why)
		}
	}
}

func TestConstraintTwoDoesNotApplyToFloatPayloads(t *testing.T) {
	// The clause the two earlier implementations had to guess at, now stated in §3.
	v, _, err := smysl.DecodeOne(hex(t, "fa3f800000"))
	if err != nil {
		t.Fatalf("1.0 was rejected: %v", err)
	}
	if v != float32(1.0) {
		t.Fatalf("got %v, want 1.0", v)
	}
}

func TestFloatsAreBinary32OnTheQuantisationGrid(t *testing.T) {
	f32 := func(v float32) []byte {
		out := []byte{0xfa}
		return binary.BigEndian.AppendUint32(out, math.Float32bits(v))
	}
	for _, ok := range []float32{0, 0.5, 1.0 / 1024, -3.25} {
		if _, _, err := smysl.DecodeOne(f32(ok)); err != nil {
			t.Errorf("%v was rejected: %v", ok, err)
		}
	}
	for _, bad := range []float32{0.1, 1.0 / 3} {
		if _, _, err := smysl.DecodeOne(f32(bad)); err == nil {
			t.Errorf("%v was accepted", bad)
		}
	}
	if _, _, err := smysl.DecodeOne(f32(float32(math.Inf(1)))); err == nil {
		t.Error("infinity was accepted")
	}
}

func TestNfcTextIsRequired(t *testing.T) {
	// Written as explicit bytes rather than as source literals. The first attempt used two
	// "é" characters in the source and an editor normalised them to the same one, so the
	// test asserted that a string differs from itself and reported the decoder as broken.
	// A test whose two inputs are secretly equal is worse than no test.
	decomposed := []byte{0x65, 0xcc, 0x81} // e + U+0301
	if _, _, err := smysl.DecodeOne(append([]byte{0x60 | byte(len(decomposed))}, decomposed...)); err == nil {
		t.Error("non-NFC text was accepted")
	}
	composed := []byte{0xc3, 0xa9} // U+00E9
	v, _, err := smysl.DecodeOne(append([]byte{0x60 | byte(len(composed))}, composed...))
	if err != nil {
		t.Errorf("composed text was rejected: %v", err)
	}
	if v != "\u00e9" {
		t.Errorf("got %q", v)
	}
}

func TestNestingIsBoundedAt128(t *testing.T) {
	if smysl.MaxNesting != 128 {
		t.Fatalf("§3 constraint 9 names 128, not %d", smysl.MaxNesting)
	}
	shallow := append(bytes.Repeat([]byte{0x81}, 100), 0x00)
	if _, _, err := smysl.DecodeOne(shallow); err != nil {
		t.Errorf("100 levels was rejected: %v", err)
	}
	deep := append(bytes.Repeat([]byte{0x81}, 200), 0x00)
	if _, _, err := smysl.DecodeOne(deep); err == nil {
		t.Error("200 levels was accepted")
	}
}

// -- §3.1  Record framing -----------------------------------------------------

func TestRecordTypeCodesMatchTheTable(t *testing.T) {
	want := map[uint64]string{
		1: "unit", 2: "attestation", 3: "relation", 4: "thread", 5: "view",
		6: "contention", 7: "pack_info", 8: "schema_decl", 9: "checkpoint", 10: "label_binding",
	}
	if len(smysl.RecordNames) != len(want) {
		t.Fatalf("code count: got %d, want %d", len(smysl.RecordNames), len(want))
	}
	for k, v := range want {
		if smysl.RecordNames[k] != v {
			t.Errorf("code %d: got %q, want %q", k, smysl.RecordNames[k], v)
		}
	}
}

func TestARecordIsATwoElementArray(t *testing.T) {
	for _, bad := range []string{"8301a000", "a101a0"} {
		if _, err := smysl.DecodeStore(hex(t, bad)); err == nil {
			t.Errorf("%s was accepted as a record", bad)
		}
	}
}

func TestUnknownRecordTypesArePreservedNotRejected(t *testing.T) {
	body := &smysl.Map{Entries: []smysl.Pair{{Key: uint64(0), Value: "a later shape"}}}
	raw, err := smysl.EncodeOne([]any{uint64(99), body})
	if err != nil {
		t.Fatal(err)
	}
	records, err := smysl.DecodeStore(raw)
	if err != nil {
		t.Fatalf("an unknown record type was rejected: %v", err)
	}
	if records[0].IsKnown() || records[0].Name() != "unknown(99)" {
		t.Fatalf("got %s", records[0].Name())
	}
	again, _ := records[0].Reencode()
	if !bytes.Equal(again, raw) {
		t.Fatal("an unknown record did not survive verbatim")
	}
}

func TestAnUnknownRecordBodyIsStillParsedStrictly(t *testing.T) {
	// §3.1: an unknown record cannot smuggle in a non-deterministic encoding.
	if _, err := smysl.DecodeStore(hex(t, "8218639fff")); err == nil {
		t.Error("an indefinite-length body inside an unknown record was accepted")
	}
}

func TestAStoreIsAConcatenationWithNoEnvelope(t *testing.T) {
	mk := func(gist string) []byte {
		body := &smysl.Map{Entries: []smysl.Pair{
			{Key: uint64(0), Value: "claim"},
			{Key: uint64(1), Value: gist},
			{Key: uint64(6), Value: uint64(1)},
		}}
		raw, err := smysl.EncodeOne([]any{uint64(1), body})
		if err != nil {
			t.Fatal(err)
		}
		return raw
	}
	joined := append(mk("first"), mk("second")...)
	records, err := smysl.DecodeStore(joined)
	if err != nil {
		t.Fatal(err)
	}
	if len(records) != 2 {
		t.Fatalf("got %d records, want 2", len(records))
	}
	for i, want := range []string{"first", "second"} {
		if v, ok := records[i].UnitField("gist"); !ok || v != want {
			t.Errorf("record %d gist: got %v, want %q", i, v, want)
		}
	}
	out, _ := smysl.EncodeStore(records)
	if !bytes.Equal(out, joined) {
		t.Error("a concatenated store did not re-encode to itself")
	}
}

func TestATruncatedTrailingRecordIsAnError(t *testing.T) {
	body := &smysl.Map{Entries: []smysl.Pair{{Key: uint64(1), Value: "g"}}}
	good, _ := smysl.EncodeOne([]any{uint64(1), body})
	if _, err := smysl.DecodeStore(append(good, 0x82, 0x01)); err == nil {
		t.Error("a truncated trailing record was ignored rather than reported")
	}
}

// -- §7  Conformance ----------------------------------------------------------

func TestWhatCReadCannotCheck(t *testing.T) {
	// Kept as a list rather than a silence. The largest entry is §2.3 — status is part of
	// identity — the paragraph the whole format rests on, which needs uids and so C-Produce.
	unreached := map[string]string{
		"§2.1 uid derivation":                "needs BLAKE3; C-Produce, not C-Read",
		"§2.3 status is part of identity":    "follows from uid derivation",
		"§4 canonical surface form":          "surface syntax is not decoded here",
		"§6 rules M, T, L, R, U, I, S, V, D": "semantic; C-Consume and above",
	}
	if len(unreached) == 0 {
		t.Fatal("if this is ever empty, say which class was implemented instead")
	}
	if _, ok := unreached["§2.3 status is part of identity"]; !ok {
		t.Error("the format's central claim dropped off the list of what is untested")
	}
}
