package smysl_test

// 1.4: relation identity and contention identity, against the vectors the Rust produced.
//
// A withdrawal names an edge by its rid and a resolution names a contention by its id. An
// implementation that derived either differently would read every other implementation's
// withdrawals and resolutions as naming nothing, so both are checked against
// fixtures/wire/relation-id and fixtures/wire/contention-id — digest and text apart.

import (
	"bytes"
	hexenc "encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	smysl "github.com/vulogov/smysl/go"
)

func wirePath(parts ...string) string {
	return filepath.Join(append([]string{"..", "fixtures", "wire"}, parts...)...)
}

func loadCases(t *testing.T, name string, into any) {
	t.Helper()
	raw, err := os.ReadFile(wirePath(name, "cases.json"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(raw, into); err != nil {
		t.Fatal(err)
	}
}

func unhex(t *testing.T, s string) []byte {
	t.Helper()
	b, err := hexenc.DecodeString(s)
	if err != nil {
		t.Fatal(err)
	}
	return b
}

func TestRelationIdsMatchTheVectors(t *testing.T) {
	var f struct {
		Cases []struct {
			Name, Kind string
			FromHex    string `json:"from_hex"`
			ToHex      string `json:"to_hex"`
			RidHex     string `json:"rid_hex"`
		}
	}
	loadCases(t, "relation-id", &f)
	if len(f.Cases) == 0 {
		t.Fatal("no vectors")
	}
	for _, c := range f.Cases {
		rid, err := smysl.RelationId(c.Kind, unhex(t, c.FromHex), unhex(t, c.ToHex))
		if err != nil {
			t.Fatalf("%s: %v", c.Name, err)
		}
		if hexenc.EncodeToString(rid) != c.RidHex {
			t.Errorf("%s: rid %x, want %s", c.Name, rid, c.RidHex)
		}
	}
}

func TestContentionIdsMatchTheVectors(t *testing.T) {
	var f struct {
		Cases []struct {
			Name         string
			Kind         byte
			OverHex      string   `json:"over_hex"`
			PositionsHex []string `json:"positions_hex"`
			DigestHex    string   `json:"digest_hex"`
			Id           string
		}
	}
	loadCases(t, "contention-id", &f)
	if len(f.Cases) == 0 {
		t.Fatal("no vectors")
	}
	for _, c := range f.Cases {
		var positions [][]byte
		for _, p := range c.PositionsHex {
			positions = append(positions, unhex(t, p))
		}
		digest, id := smysl.ContentionId(c.Kind, unhex(t, c.OverHex), positions)
		if hexenc.EncodeToString(digest) != c.DigestHex {
			t.Errorf("%s: digest %x, want %s", c.Name, digest, c.DigestHex)
			continue
		}
		if id != c.Id {
			t.Errorf("%s: the digest agrees and the text does not: %s, want %s", c.Name, id, c.Id)
		}
	}
}

func TestRecords11And12AreKnownAndRoundTrip(t *testing.T) {
	data, err := os.ReadFile(wirePath("F10-lifecycle.cbor"))
	if err != nil {
		t.Fatal(err)
	}
	records, err := smysl.DecodeStore(data)
	if err != nil {
		t.Fatal(err)
	}
	seen := map[string]bool{}
	for _, r := range records {
		if !r.IsKnown() {
			t.Errorf("record %d is unknown", r.Code)
		}
		seen[r.Name()] = true
	}
	if !seen["withdrawal"] || !seen["resolution"] {
		t.Errorf("records: %v", seen)
	}
	out, err := smysl.EncodeStore(records)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(out, data) {
		t.Error("F10-lifecycle.cbor did not re-encode byte for byte")
	}
}
