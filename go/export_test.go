package smysl

// The record-set digest of SMYSL-2.7 §5.4, against the vectors the Rust produced.
//
// fixtures/wire/rsd/cases.json names each input and its digest. Order and repeats do not count,
// which repeats.cbor (F13 twice, the first time reversed) holds the implementation to.

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestRecordSetDigestMatchesTheRust(t *testing.T) {
	dir := filepath.Join("..", "fixtures", "wire", "rsd")
	raw, err := os.ReadFile(filepath.Join(dir, "cases.json"))
	if err != nil {
		t.Fatal(err)
	}
	var cases []struct {
		File    string `json:"file"`
		Records int    `json:"records"`
		Rsd     string `json:"rsd"`
	}
	if err := json.Unmarshal(raw, &cases); err != nil {
		t.Fatal(err)
	}
	if len(cases) == 0 {
		t.Fatal("no rsd vectors; the suite would pass vacuously")
	}
	for _, c := range cases {
		data, err := os.ReadFile(filepath.Join(dir, c.File))
		if err != nil {
			t.Fatal(err)
		}
		records, err := DecodeStore(data)
		if err != nil {
			t.Fatalf("%s: %v", c.File, err)
		}
		if len(records) != c.Records {
			t.Fatalf("%s: %d records, want %d", c.File, len(records), c.Records)
		}
		var raws [][]byte
		for _, r := range records {
			raws = append(raws, r.Raw)
		}
		if got := hex.EncodeToString(RecordSetDigest(raws)); got != c.Rsd {
			t.Fatalf("%s: rsd %s, want %s", c.File, got, c.Rsd)
		}
	}
}
