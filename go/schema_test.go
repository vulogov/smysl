package smysl

// FC-9 decoding and L9 against the fixtures the Rust wrote (SMYSL-2.5 §5.8, TX-P12 step 1).
//
// fixtures/schema/fc9/cases.json names, per file and declaration, the field count or "opaque",
// each step's op tags and bytes, and the codes decoding reports; and the lens-chain hashes of
// the series. Lens application is not a conformance obligation, so nothing here applies a lens.

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

type fc9Cases struct {
	Files []struct {
		File  string `json:"file"`
		Decls []struct {
			ID           string          `json:"id"`
			Version      uint64          `json:"version"`
			Fields       json.RawMessage `json:"fields"`
			LensesOpaque bool            `json:"lenses_opaque"`
			Steps        []struct {
				From  uint64   `json:"from"`
				To    uint64   `json:"to"`
				Ops   []string `json:"ops"`
				Bytes string   `json:"bytes"`
			} `json:"steps"`
			Codes []string `json:"codes"`
		} `json:"decls"`
	} `json:"files"`
	Chains []struct {
		ID   string `json:"id"`
		From uint64 `json:"from"`
		To   uint64 `json:"to"`
		Hash string `json:"hash"`
	} `json:"chains"`
}

var fc9Dir = filepath.Join("..", "fixtures", "schema", "fc9")

func fc9Records(t *testing.T, name string) ([]byte, []*Record) {
	t.Helper()
	data, err := os.ReadFile(filepath.Join(fc9Dir, name))
	if err != nil {
		t.Fatal(err)
	}
	records, err := DecodeStore(data)
	if err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	for _, r := range records {
		if r.Code != 8 {
			t.Fatalf("%s: record %d is not a schema declaration", name, r.Code)
		}
	}
	return data, records
}

func TestFC9MatchesTheRust(t *testing.T) {
	raw, err := os.ReadFile(filepath.Join(fc9Dir, "cases.json"))
	if err != nil {
		t.Fatal(err)
	}
	var cases fc9Cases
	if err := json.Unmarshal(raw, &cases); err != nil {
		t.Fatal(err)
	}
	if len(cases.Files) == 0 || len(cases.Chains) == 0 {
		t.Fatal("no FC-9 cases; the suite would pass vacuously")
	}
	for _, c := range cases.Files {
		data, records := fc9Records(t, c.File)
		var again []byte
		for i := range records {
			b, err := records[i].Reencode()
			if err != nil {
				t.Fatal(err)
			}
			again = append(again, b...)
		}
		if !bytes.Equal(again, data) {
			t.Errorf("%s does not re-encode byte for byte", c.File)
		}
		if len(records) != len(c.Decls) {
			t.Fatalf("%s: %d declarations, want %d", c.File, len(records), len(c.Decls))
		}
		for i, want := range c.Decls {
			body := records[i].Body.(*Map)
			id, _ := body.Get(uint64(0))
			ver, _ := body.Get(uint64(1))
			if id != want.ID || ver != want.Version {
				t.Errorf("%s[%d]: %v@%v, want %s@%d", c.File, i, id, ver, want.ID, want.Version)
			}
			got := DecodeSchemaDecl(body)
			var fields string
			switch {
			case got.Fields == nil:
				fields = "null"
			case *got.Fields < 0:
				fields = `"opaque"`
			default:
				fields = itoa(uint64(*got.Fields))
			}
			if fields != string(want.Fields) {
				t.Errorf("%s[%d] fields %s, want %s", c.File, i, fields, want.Fields)
			}
			if got.LensesOpaque != want.LensesOpaque {
				t.Errorf("%s[%d] lenses_opaque %v", c.File, i, got.LensesOpaque)
			}
			codes := got.Codes
			if codes == nil {
				codes = []string{}
			}
			if !reflect.DeepEqual(codes, want.Codes) && !(len(codes) == 0 && len(want.Codes) == 0) {
				t.Errorf("%s[%d] codes %v, want %v", c.File, i, codes, want.Codes)
			}
			if len(got.Steps) != len(want.Steps) {
				t.Fatalf("%s[%d] %d steps, want %d", c.File, i, len(got.Steps), len(want.Steps))
			}
			for j, s := range got.Steps {
				w := want.Steps[j]
				if s.From != w.From || s.To != w.To || !reflect.DeepEqual(s.Ops, w.Ops) ||
					hex.EncodeToString(s.Bytes) != w.Bytes {
					t.Errorf("%s[%d] step %d differs from the Rust's", c.File, i, j)
				}
			}
			// L9: key 5 written back from its steps is what arrived.
			if k5, ok := body.Get(uint64(5)); ok && !got.LensesOpaque {
				b, err := EncodeOne(k5)
				if err != nil {
					t.Fatal(err)
				}
				if !bytes.Equal(EncodeLenses(got.Steps), b) {
					t.Errorf("%s[%d]: L9 fails", c.File, i)
				}
			}
		}
	}
	_, series := fc9Records(t, "series.cbor")
	var steps []LensStep
	for _, r := range series {
		steps = append(steps, DecodeSchemaDecl(r.Body.(*Map)).Steps...)
	}
	for _, ch := range cases.Chains {
		var path []LensStep
		at := ch.From
		for at < ch.To {
			found := false
			for _, s := range steps {
				if s.From == at {
					path, at, found = append(path, s), s.To, true
					break
				}
			}
			if !found {
				t.Fatalf("no step from %d", at)
			}
		}
		if got := hex.EncodeToString(ChainHash(ch.ID, ch.From, ch.To, path)); got != ch.Hash {
			t.Errorf("chain %d→%d: %s, want %s", ch.From, ch.To, got, ch.Hash)
		}
	}
}
