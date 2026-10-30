package smysl

// Export collections, read by a fourth implementation (SMYSL-2.7 §5.4, TX-P11 step 11).
//
// RecordSetDigest is the spec's §2.7 digest over the raw bytes every decoded record keeps;
// fixtures/wire/rsd/ holds vectors the Rust produced. ReadExportIndex reads an index from
// generic CBOR and SMYSL-2.7 §4.2's field numbers. VerifyExport checks each file against its
// BLAKE3, record count and record-set digest, that every record re-encodes to the bytes it was
// read from, and the collection's digest. `go run ./cmd/xi verify DIR` prints one JSON line.

import (
	"bytes"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

// IndexFile is the export-index file name.
const IndexFile = "export-index.cbor"

// IndexVersion is the version an index declares at key 0.
const IndexVersion = "smysl/export-index/1"

// RecordSetDigest is §2.7: BLAKE3 over "smysl/rsd/1" ‖ 0x00 and the sorted unique hashes.
func RecordSetDigest(raws [][]byte) []byte {
	seen := map[string][]byte{}
	for _, r := range raws {
		h := Blake3(r)
		seen[string(h)] = h
	}
	hashes := make([][]byte, 0, len(seen))
	for _, h := range seen {
		hashes = append(hashes, h)
	}
	sort.Slice(hashes, func(i, j int) bool { return bytes.Compare(hashes[i], hashes[j]) < 0 })
	in := append([]byte("smysl/rsd/1"), 0)
	for _, h := range hashes {
		in = append(in, h...)
	}
	return Blake3(in)
}

// ExportFile is one file an index names.
type ExportFile struct {
	Path    string
	Blake3  []byte
	Records uint64
	Rsd     []byte
}

// ExportIndex is what verification reads of an index.
type ExportIndex struct {
	Files  []ExportFile
	Format string
	Rsd    []byte
}

func get(m *Map, k uint64) (any, bool) { return m.Get(k) }

// ReadExportIndex decodes an index, refusing anything that is not one.
func ReadExportIndex(data []byte) (*ExportIndex, error) {
	v, used, err := DecodeOne(data)
	if err != nil {
		return nil, err
	}
	m, ok := v.(*Map)
	if used != len(data) || !ok {
		return nil, fmt.Errorf("an export index is one CBOR map")
	}
	if ver, _ := get(m, 0); ver != IndexVersion {
		return nil, fmt.Errorf("not %s", IndexVersion)
	}
	out := &ExportIndex{}
	if f, ok := get(m, 8); ok {
		out.Format, _ = f.(string)
	}
	if r, ok := get(m, 11); ok {
		out.Rsd, _ = r.([]byte)
	}
	files, _ := get(m, 1)
	list, _ := files.([]any)
	for _, item := range list {
		fm, ok := item.(*Map)
		if !ok {
			return nil, fmt.Errorf("a file entry is a map")
		}
		var f ExportFile
		p, _ := get(fm, 0)
		f.Path, _ = p.(string)
		b, _ := get(fm, 1)
		f.Blake3, _ = b.([]byte)
		n, _ := get(fm, 2)
		f.Records, _ = n.(uint64)
		r, _ := get(fm, 3)
		f.Rsd, _ = r.([]byte)
		out.Files = append(out.Files, f)
	}
	return out, nil
}

// VerifyResult is what VerifyExport found.
type VerifyResult struct {
	Ok       bool     `json:"ok"`
	Files    int      `json:"files"`
	Rsd      string   `json:"rsd"`
	Problems []string `json:"problems"`
}

// VerifyExport checks a collection on disk.
func VerifyExport(dir string) (*VerifyResult, error) {
	data, err := os.ReadFile(filepath.Join(dir, IndexFile))
	if err != nil {
		return nil, err
	}
	index, err := ReadExportIndex(data)
	if err != nil {
		return nil, err
	}
	problems := []string{}
	var raws [][]byte
	surface := false
	for _, f := range index.Files {
		b, err := os.ReadFile(filepath.Join(dir, f.Path))
		if err != nil {
			problems = append(problems, f.Path+": absent")
			continue
		}
		if !bytes.Equal(Blake3(b), f.Blake3) {
			problems = append(problems, f.Path+": its BLAKE3 is not the index's")
			continue
		}
		if strings.HasSuffix(f.Path, ".smy") {
			// A surface file is checked by its hash; reading surface text is the Rust's.
			surface = true
			continue
		}
		records, err := DecodeStore(b)
		if err != nil {
			problems = append(problems, f.Path+": "+err.Error())
			continue
		}
		again, err := EncodeStore(records)
		if err != nil || !bytes.Equal(again, b) {
			problems = append(problems, f.Path+": does not re-encode to its own bytes")
		}
		if uint64(len(records)) != f.Records {
			problems = append(problems, fmt.Sprintf("%s: %d records, the index says %d", f.Path, len(records), f.Records))
		}
		var mine [][]byte
		for _, r := range records {
			mine = append(mine, r.Raw)
		}
		if !bytes.Equal(RecordSetDigest(mine), f.Rsd) {
			problems = append(problems, f.Path+": its record-set digest is not the index's")
		}
		raws = append(raws, mine...)
	}
	rsd := RecordSetDigest(raws)
	if !surface && !bytes.Equal(rsd, index.Rsd) {
		problems = append(problems, "the collection's record-set digest is not the index's")
	}
	return &VerifyResult{Ok: len(problems) == 0, Files: len(index.Files), Rsd: hex.EncodeToString(rsd), Problems: problems}, nil
}

// WriteProduced writes what this implementation produces, for the Rust to import: every wire
// fixture re-encoded by this package, and a small store built here, two units and an edge.
func WriteProduced(wire, out string) ([]string, error) {
	if err := os.MkdirAll(out, 0o755); err != nil {
		return nil, err
	}
	var written []string
	names, err := filepath.Glob(filepath.Join(wire, "*.cbor"))
	if err != nil {
		return nil, err
	}
	sort.Strings(names)
	for _, p := range names {
		data, err := os.ReadFile(p)
		if err != nil {
			return nil, err
		}
		records, err := DecodeStore(data)
		if err != nil {
			return nil, fmt.Errorf("%s: %w", p, err)
		}
		again, err := EncodeStore(records)
		if err != nil {
			return nil, err
		}
		name := filepath.Base(p)
		if err := os.WriteFile(filepath.Join(out, name), again, 0o644); err != nil {
			return nil, err
		}
		written = append(written, name)
	}
	a := &UnitCore{Schema: "claim", Gist: "Written by the Go implementation.", Status: StatusSpeculative}
	b := &UnitCore{Schema: "claim", Gist: "And grounded on nothing but its own say-so.", Status: StatusSpeculative}
	var store []byte
	var uids [][]byte
	for _, u := range []*UnitCore{a, b} {
		core, err := u.CanonicalBytes()
		if err != nil {
			return nil, err
		}
		id, err := u.Uid()
		if err != nil {
			return nil, err
		}
		uids = append(uids, id)
		store = append(store, head(4, 2)...)
		store = append(store, encodeUint(1)...)
		store = append(store, core...)
	}
	edge, err := encodeMap([]entry{
		{key: 0, val: encodeText("x.xi/related")},
		{key: 1, val: encodeBytes(uids[0])},
		{key: 2, val: encodeBytes(uids[1])},
	})
	if err != nil {
		return nil, err
	}
	store = append(store, head(4, 2)...)
	store = append(store, encodeUint(3)...)
	store = append(store, edge...)
	if err := os.WriteFile(filepath.Join(out, "go-small.cbor"), store, 0o644); err != nil {
		return nil, err
	}
	return append(written, "go-small.cbor"), nil
}
