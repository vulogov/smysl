package smysl

// GE-T1's chat half: the identities of a chat corpus, recomputed here.
//
// ../fixtures/library/wire/ gives this implementation a scripture-shaped expression whose records
// were hand-built in smysl-core — correctly, in a crate with no reader. chat-wire gives it the
// records a real `text add` emitted from three chat exports, which is where the identities are
// derived over things scripture has none of: segment tables with speakers, timestamps and
// platform ids in them, `mul` manifests, and `raw` metadata inside the mid.
//
// This implementation implements no reader and is not asked to. It decodes and hashes.

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"os"
	"testing"
)

type chatPart struct {
	TidHex       string `json:"tid_hex"`
	Length       uint64 `json:"length"`
	StructureHex string `json:"structure_hex"`
	RdidHex      string `json:"rdid_hex"`
	Segments     int    `json:"segments"`
}

type chatExpression struct {
	File   string     `json:"file"`
	Reader string     `json:"reader"`
	Alias  string     `json:"alias"`
	MidHex string     `json:"mid_hex"`
	Parts  []chatPart `json:"parts"`
}

type chatVectors struct {
	Expressions []chatExpression `json:"expressions"`
}

func chatFixtures(t *testing.T) (chatVectors, []*Record) {
	t.Helper()
	raw, err := os.ReadFile("../fixtures/library/chat-wire/ids.json")
	if err != nil {
		t.Fatalf("the vectors are missing: %v", err)
	}
	var v chatVectors
	if err := json.Unmarshal(raw, &v); err != nil {
		t.Fatalf("the vectors do not parse: %v", err)
	}
	// A suite with no fixtures passes vacuously, which is the failure this catches.
	if len(v.Expressions) != 3 {
		t.Fatalf("one expression per export, got %d", len(v.Expressions))
	}
	data, err := os.ReadFile("../fixtures/library/chat-wire/records.cbor")
	if err != nil {
		t.Fatalf("the records are missing: %v", err)
	}
	records, err := DecodeStore(data)
	if err != nil {
		t.Fatalf("the records do not decode: %v", err)
	}
	return v, records
}

func TestTheChatFixtureRoundTrips(t *testing.T) {
	_, records := chatFixtures(t)
	data, err := os.ReadFile("../fixtures/library/chat-wire/records.cbor")
	if err != nil {
		t.Fatal(err)
	}
	out, err := EncodeStore(records)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(out, data) {
		t.Error("the chat fixture did not round-trip")
	}
	for _, r := range records {
		switch r.Code {
		case 14, 15, 18:
		default:
			t.Errorf("record %d does not belong in this fixture", r.Code)
		}
	}
}

// The exit: each tid, structure hash, rdid and mid, derived here and compared.
func TestEveryChatIdentityIsRecomputedFromTheRecords(t *testing.T) {
	v, records := chatFixtures(t)

	manifests := map[string]*Manifest{}
	for _, r := range records {
		if r.Code != 14 {
			continue
		}
		m, err := DecodeManifest(r)
		if err != nil {
			t.Fatal(err)
		}
		mid, err := m.Mid()
		if err != nil {
			t.Fatal(err)
		}
		manifests[hex.EncodeToString(mid)] = m
	}
	texts := map[string]*PartText{}
	for _, r := range records {
		if r.Code != 15 {
			continue
		}
		p, err := DecodePartText(r)
		if err != nil {
			t.Fatal(err)
		}
		texts[hex.EncodeToString(p.Tid)] = p
	}
	readings := map[string]*PartReading{}
	for _, r := range records {
		if r.Code != 18 {
			continue
		}
		g, err := DecodePartReading(r)
		if err != nil {
			t.Fatal(err)
		}
		rdid, err := g.Rdid()
		if err != nil {
			t.Fatal(err)
		}
		readings[hex.EncodeToString(rdid)] = g
	}

	for _, want := range v.Expressions {
		m, ok := manifests[want.MidHex]
		if !ok {
			t.Fatalf("no manifest hashes to %s", want.MidHex)
		}
		if m.Fields["reader"] != want.Reader {
			t.Errorf("reader %v, want %q", m.Fields["reader"], want.Reader)
		}
		if m.Fields["lang"] != "mul" {
			t.Errorf("a chat export is not one language: %v", m.Fields["lang"])
		}
		if len(m.Parts) != len(want.Parts) {
			t.Fatalf("%d parts, want %d", len(m.Parts), len(want.Parts))
		}
		for i, entry := range m.Parts {
			w := want.Parts[i]
			if hex.EncodeToString(entry.Tid) != w.TidHex {
				t.Errorf("tid %s, want %s", hex.EncodeToString(entry.Tid), w.TidHex)
			}
			if entry.Length != w.Length {
				t.Errorf("length %d, want %d", entry.Length, w.Length)
			}
			if hex.EncodeToString(entry.Structure) != w.StructureHex {
				t.Errorf("structure %s, want %s", hex.EncodeToString(entry.Structure), w.StructureHex)
			}
			if hex.EncodeToString(entry.Rdid) != w.RdidHex {
				t.Errorf("rdid %s, want %s", hex.EncodeToString(entry.Rdid), w.RdidHex)
			}

			part, ok := texts[w.TidHex]
			if !ok {
				t.Fatalf("no part text for %s", w.TidHex)
			}
			if got := hex.EncodeToString(Tid(part.Text)); got != w.TidHex {
				t.Errorf("the part's bytes hash to %s, not %s", got, w.TidHex)
			}
			reading, ok := readings[w.RdidHex]
			if !ok {
				t.Fatalf("no reading for %s", w.RdidHex)
			}
			structure, err := reading.StructureHash()
			if err != nil {
				t.Fatal(err)
			}
			if hex.EncodeToString(structure) != w.StructureHex {
				t.Errorf("the table hashes to %s, not %s", hex.EncodeToString(structure), w.StructureHex)
			}
			rows, err := reading.Rows()
			if err != nil {
				t.Fatal(err)
			}
			if len(rows) != w.Segments {
				t.Errorf("%d segments, want %d", len(rows), w.Segments)
			}
			if err := reading.VerifyEntry(entry); err != nil {
				t.Errorf("the reading does not match its entry: %v", err)
			}
		}
	}
}

// A row's speaker, timestamp and ids are inside the structure hash and therefore inside the
// rdid, so an implementation that dropped them would decode the record and disagree about the
// identity.
func TestAChatReadingNamesWhoSaidWhat(t *testing.T) {
	_, records := chatFixtures(t)
	var speakers, observed, ids int
	for _, r := range records {
		if r.Code != 18 {
			continue
		}
		g, err := DecodePartReading(r)
		if err != nil {
			t.Fatal(err)
		}
		rows, err := g.Rows()
		if err != nil {
			t.Fatal(err)
		}
		for _, row := range rows {
			if _, ok := row["speaker"]; ok {
				speakers++
			}
			if _, ok := row["observed"]; ok {
				observed++
			}
			if _, ok := row["ids"]; ok {
				ids++
			}
		}
	}
	if speakers == 0 || observed == 0 || ids == 0 {
		t.Errorf("a chat reading names its speakers (%d), their times (%d) and their ids (%d)",
			speakers, observed, ids)
	}
}

// Carried as opaque canonical CBOR, so nothing here decodes it — which is why it is worth
// checking: an implementation that dropped an opaque key would re-encode a shorter body and
// compute a different mid, with nothing else to notice.
func TestRawMetadataIsInsideTheMid(t *testing.T) {
	v, records := chatFixtures(t)
	mids := map[string]bool{}
	for _, e := range v.Expressions {
		mids[e.MidHex] = true
	}
	var withRaw int
	for _, r := range records {
		if r.Code != 14 {
			continue
		}
		body, ok := r.Body.(*Map)
		if !ok {
			t.Fatal("a manifest body is a map")
		}
		if _, has := body.Get(uint64(16)); !has {
			continue
		}
		withRaw++
		m, err := DecodeManifest(r)
		if err != nil {
			t.Fatal(err)
		}
		mid, err := m.Mid()
		if err != nil {
			t.Fatal(err)
		}
		if !mids[hex.EncodeToString(mid)] {
			t.Errorf("a manifest with raw metadata hashes to an identity the fixture does not name")
		}
	}
	if withRaw != 2 {
		t.Errorf("telegram and slack carry raw metadata; got %d", withRaw)
	}
}
