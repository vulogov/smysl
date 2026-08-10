package smysl

// C-Produce, checked in four layers, because a failure in each means something different.
//
//  1. BLAKE3 against the published vectors — in `blake3_test.go`, and it runs first because
//     if the hash is wrong nothing below is evidence.
//  2. Canonical bytes against the Rust's, *separately* from the uid. A fixture that agrees on
//     the hash but not the layout, or the reverse, says which half to look at. The fixture
//     file carries `core_bytes_hex` for exactly this.
//  3. The uid itself.
//  4. §2.3 as a property rather than one example, and §7's shape clause as a set of refusals.
//
// The third implementation to reach §2.1. The Rust defined it, `python/` reproduced it in
// 0.10, and until now `go/` and `nodejs/` could round-trip every byte of every fixture while
// having no idea what a uid was.

import (
	"encoding/hex"
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

type uidCase struct {
	Name string `json:"name"`
	Core struct {
		Schema  string   `json:"schema"`
		Gist    string   `json:"gist"`
		Body    *string  `json:"body"`
		Detail  *string  `json:"detail"`
		Deps    []string `json:"deps"`
		Grounds []string `json:"grounds"`
		Status  uint64   `json:"status"`
		Source  *struct {
			Kind      uint64  `json:"kind"`
			Reference string  `json:"reference"`
			Captured  *string `json:"captured"`
		} `json:"source"`
		PayloadHex *string `json:"payload_hex"`
	} `json:"core"`
	CoreBytesHex string `json:"core_bytes_hex"`
	UidHex       string `json:"uid_hex"`
}

func loadUidCases(t *testing.T) []uidCase {
	t.Helper()
	path := filepath.Join("..", "fixtures", "wire", "uid", "cases.json")
	raw, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("reading %s: %v", path, err)
	}
	var doc struct {
		Cases []uidCase `json:"cases"`
	}
	if err := json.Unmarshal(raw, &doc); err != nil {
		t.Fatalf("parsing %s: %v", path, err)
	}
	return doc.Cases
}

func mustHex(t *testing.T, s string) []byte {
	t.Helper()
	b, err := hex.DecodeString(s)
	if err != nil {
		t.Fatalf("bad hex %q: %v", s, err)
	}
	return b
}

func build(t *testing.T, c uidCase) *UnitCore {
	t.Helper()
	u := &UnitCore{
		Schema: c.Core.Schema,
		Gist:   c.Core.Gist,
		Status: Status(c.Core.Status),
	}
	if c.Core.Body != nil {
		u.Body = *c.Core.Body
	}
	if c.Core.Detail != nil {
		u.Detail = *c.Core.Detail
	}
	for _, d := range c.Core.Deps {
		u.Deps = append(u.Deps, mustHex(t, d))
	}
	for _, g := range c.Core.Grounds {
		u.Grounds = append(u.Grounds, mustHex(t, g))
	}
	if c.Core.Source != nil {
		s := &Source{Kind: c.Core.Source.Kind, Reference: c.Core.Source.Reference}
		if c.Core.Source.Captured != nil {
			s.Captured = *c.Core.Source.Captured
		}
		u.Source = s
	}
	if c.Core.PayloadHex != nil {
		u.Payload = mustHex(t, *c.Core.PayloadHex)
	}
	return u
}

// A glob that matched nothing would make every case-driven test below vacuous — the failure
// this repository has hit more than once, most recently a doc-output run that reported
// "ran 0, MISMATCHED 0" as a pass.
func TestThereAreUidFixtures(t *testing.T) {
	if n := len(loadUidCases(t)); n < 16 {
		t.Fatalf("expected at least 16 fixture cases; got %d", n)
	}
}

// Layer 2: the layout, checked before the uid so a mismatch says which half broke.
func TestCanonicalBytesMatchTheReference(t *testing.T) {
	for _, c := range loadUidCases(t) {
		t.Run(c.Name, func(t *testing.T) {
			got, err := build(t, c).CanonicalBytes()
			if err != nil {
				t.Fatalf("encoding: %v", err)
			}
			if hex.EncodeToString(got) != c.CoreBytesHex {
				t.Errorf("canonical bytes differ:\n  got  %s\n  want %s",
					hex.EncodeToString(got), c.CoreBytesHex)
			}
		})
	}
}

// Layer 3: the uid. §2.1 over the bytes layer 2 just agreed on.
func TestUidsMatchTheReference(t *testing.T) {
	for _, c := range loadUidCases(t) {
		t.Run(c.Name, func(t *testing.T) {
			b, err := build(t, c).CanonicalBytes()
			if err != nil {
				t.Fatalf("encoding: %v", err)
			}
			if got := hex.EncodeToString(Blake3(b)); got != c.UidHex {
				t.Errorf("uid differs:\n  got  %s\n  want %s", got, c.UidHex)
			}
		})
	}
}

// Layer 4a: §2.3, as a property.
//
// Every field identical, one status apart: the uids must be unrelated. The fixture pair
// `status-pair-a` / `status-pair-b` is one witness; this is the general claim, over every
// status a unit may legally carry.
func TestStatusIsPartOfIdentity(t *testing.T) {
	seen := map[string]Status{}
	for _, s := range []Status{StatusSpeculative, StatusInferred, StatusDerived, StatusCited, StatusMeasured} {
		u := &UnitCore{Schema: "claim", Gist: "the same words either way", Status: s}
		// Satisfy the shape rules without touching the words: these are what §7 demands of
		// each status, and demanding them is the point of the class.
		if s.RequiresGrounds() {
			u.Grounds = [][]byte{make([]byte, UidLen)}
		}
		if s.RequiresSource() {
			u.Source = &Source{Kind: 0, Reference: "https://example.invalid/a"}
		}
		uid, err := u.Uid()
		if err != nil {
			t.Fatalf("%s: %v", s, err)
		}
		if prior, clash := seen[string(uid)]; clash {
			t.Fatalf("%s and %s produced the same uid; status is not in the hash", prior, s)
		}
		seen[string(uid)] = s
	}
	if len(seen) != 5 {
		t.Fatalf("expected five distinct uids; got %d", len(seen))
	}
}

// The control on the test above: with status held fixed, the same content *must* collide.
//
// Without this, `TestStatusIsPartOfIdentity` would pass for an implementation whose uids were
// simply always distinct — a counter, or a bad hash — and prove nothing about status at all.
func TestIdenticalContentProducesOneUid(t *testing.T) {
	mk := func() *UnitCore {
		return &UnitCore{Schema: "claim", Gist: "the same words either way", Status: StatusSpeculative}
	}
	a, err := mk().Uid()
	if err != nil {
		t.Fatal(err)
	}
	b, err := mk().Uid()
	if err != nil {
		t.Fatal(err)
	}
	if hex.EncodeToString(a) != hex.EncodeToString(b) {
		t.Fatal("identity is content, and the same content gave two uids")
	}
}

// Layer 4b: §7's shape clause. C-Produce is the class that refuses to emit these.
func TestMalformedUnitsAreRefused(t *testing.T) {
	cases := []struct {
		name string
		core *UnitCore
		want string
	}{
		{"no gist", &UnitCore{Schema: "claim", Status: StatusSpeculative}, "gist"},
		{"blank gist", &UnitCore{Schema: "claim", Gist: "   \t ", Status: StatusSpeculative}, "gist"},
		{"no schema", &UnitCore{Gist: "a claim", Status: StatusSpeculative}, "schema"},
		{"authored unfounded", &UnitCore{Schema: "claim", Gist: "a claim", Status: StatusUnfounded}, "retraction"},
		{"derived without grounds", &UnitCore{Schema: "claim", Gist: "a claim", Status: StatusDerived}, "grounds"},
		{"inferred without grounds", &UnitCore{Schema: "claim", Gist: "a claim", Status: StatusInferred}, "grounds"},
		{"measured without source", &UnitCore{Schema: "claim", Gist: "a claim", Status: StatusMeasured}, "source"},
		{"cited without source", &UnitCore{Schema: "claim", Gist: "a claim", Status: StatusCited}, "source"},
		{"detail without body", &UnitCore{Schema: "claim", Gist: "a claim", Detail: "d", Status: StatusSpeculative}, "detail"},
		{"status out of range", &UnitCore{Schema: "claim", Gist: "a claim", Status: Status(9)}, "not one of the six"},
		{"kernel key as extension", &UnitCore{
			Schema: "claim", Gist: "a claim", Status: StatusSpeculative,
			Extra: map[uint64][]byte{keyBody: {0x60}},
		}, "kernel key"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			err := tc.core.Validate()
			if err == nil {
				t.Fatalf("Validate accepted it")
			}
			if !contains(err.Error(), tc.want) {
				t.Errorf("error should mention %q; got %q", tc.want, err)
			}
			// And the refusal has to reach Uid, or a caller routes around it.
			if _, err := tc.core.Uid(); err == nil {
				t.Errorf("Uid returned a uid for a unit Validate rejects")
			}
		})
	}
}

// The control on the refusals: a well-formed unit of every status must be accepted.
//
// A `Validate` that returned an error unconditionally would satisfy every case above.
func TestWellFormedUnitsAreAccepted(t *testing.T) {
	for _, s := range []Status{StatusSpeculative, StatusInferred, StatusDerived, StatusCited, StatusMeasured} {
		u := &UnitCore{Schema: "claim", Gist: "a well-formed claim", Status: s}
		if s.RequiresGrounds() {
			u.Grounds = [][]byte{make([]byte, UidLen)}
		}
		if s.RequiresSource() {
			u.Source = &Source{Kind: 0, Reference: "https://example.invalid/a"}
		}
		if err := u.Validate(); err != nil {
			t.Errorf("%s: %v", s, err)
		}
	}
}

// §3 constraint 6: NFC happens in the encoder, not in the caller.
//
// This and the `unicode-decomposed` fixture catch *different* failures, which was established
// by breaking each in turn rather than reasoned about — the first version of this comment had
// it backwards:
//
//   - **no normalisation at all** fails here, because two spellings then give two units. The
//     fixture missed it until the fixture itself was repaired: it had been recording the gist
//     post-normalisation, so both unicode cases carried the same composed string.
//   - **the wrong normalisation** (NFD rather than NFC) passes here, because both spellings
//     still collide — they just collide on the wrong bytes. Only the fixture catches that,
//     since only the fixture knows what the bytes should be.
//
// Neither test subsumes the other, and an implementation with only one of them has a hole.
func TestNormalisationIsPartOfIdentity(t *testing.T) {
	composed := &UnitCore{Schema: "claim", Gist: "café", Status: StatusSpeculative}
	decomposed := &UnitCore{Schema: "claim", Gist: "café", Status: StatusSpeculative}
	a, err := composed.Uid()
	if err != nil {
		t.Fatal(err)
	}
	b, err := decomposed.Uid()
	if err != nil {
		t.Fatal(err)
	}
	if hex.EncodeToString(a) != hex.EncodeToString(b) {
		t.Fatal("two spellings of one word gave two units; NFC is not being applied")
	}
}

// §2.2: deps and grounds are sets — deduplicated and sorted by uid bytes, so insertion order
// cannot reach the hash.
func TestUidSetsAreOrderAndDuplicateIndependent(t *testing.T) {
	x := make([]byte, UidLen)
	x[0] = 0x01
	y := make([]byte, UidLen)
	y[0] = 0x02

	mk := func(g [][]byte) []byte {
		u := &UnitCore{Schema: "claim", Gist: "a derived claim", Status: StatusDerived, Grounds: g}
		uid, err := u.Uid()
		if err != nil {
			t.Fatal(err)
		}
		return uid
	}
	want := hex.EncodeToString(mk([][]byte{x, y}))
	for _, g := range [][][]byte{{y, x}, {x, y, x}, {y, x, y, x}} {
		if got := hex.EncodeToString(mk(g)); got != want {
			t.Errorf("a set's encoding depended on its input order or duplicates")
		}
	}
}

func contains(haystack, needle string) bool {
	for i := 0; i+len(needle) <= len(haystack); i++ {
		if haystack[i:i+len(needle)] == needle {
			return true
		}
	}
	return false
}
