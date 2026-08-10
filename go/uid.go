package smysl

// C-Produce: laying out a unit core canonically, deriving its uid, and refusing to emit one
// that is not well formed. §2.1, §2.2, §2.3 and §7.
//
// Why this exists
// ---------------
//
// `conformance_test.go` and `invalid_corpus_test.go` read documents, and reading never
// requires deriving a uid. Both could pass in full while this package had no idea what a uid
// *is* — which is exactly what was true of it until now, and of `nodejs/` still.
//
// §0 of the specification puts it plainly: three independent readers round-tripped every
// fixture byte for byte while remaining ignorant of §2.1, so §2.3 — *status is part of
// identity*, the paragraph the format rests on — was verified by the Rust and, since 0.10, by
// `python/`. Two implementations. This is the third.
//
// What C-Produce needs beyond reading
// -----------------------------------
//
// A hash (`blake3.go`, hand-rolled for the reason given there), a canonical layout (below),
// and the *shape* half of the class: §7 defines C-Produce as "structural + epistemic + shape —
// emit well-formed units: a gist present, grounds where the status demands them, a source
// where `measured` or `cited` demands one". `Validate` is that clause, and `Uid` refuses
// rather than hashing a unit that fails it — a producer that can emit a malformed unit has
// not implemented the class, whatever its uids agree with.
//
// Checked against `fixtures/wire/uid/cases.json`, which carries the Rust's canonical bytes as
// well as its uids, so a disagreement says whether the encoding or the hash was wrong.

import (
	"errors"
	"fmt"
	"sort"

	"golang.org/x/text/unicode/norm"
)

// Unit core keys, §2.2. Anything at 9 or above is an unknown key carried verbatim (rule X).
const (
	keySchema  = 0
	keyGist    = 1
	keyBody    = 2
	keyDetail  = 3
	keyDeps    = 4
	keyGrounds = 5
	keyStatus  = 6
	keySource  = 7
	keyPayload = 8
)

// The source sub-map, §1.1.
const (
	keySourceKind      = 0
	keySourceReference = 1
	keySourceCaptured  = 2
)

// UidLen is the digest width of §2.1: the full 32 bytes, never an abbreviation.
const UidLen = 32

// Status codes, §2.2 key 6. The integer is what is hashed; the name is presentation.
type Status uint64

const (
	// StatusUnfounded is reachable only by retraction and MUST NOT be authored.
	StatusUnfounded   Status = 0
	StatusSpeculative Status = 1
	StatusInferred    Status = 2
	StatusDerived     Status = 3
	StatusCited       Status = 4
	StatusMeasured    Status = 5
)

func (s Status) String() string {
	switch s {
	case StatusUnfounded:
		return "unfounded"
	case StatusSpeculative:
		return "speculative"
	case StatusInferred:
		return "inferred"
	case StatusDerived:
		return "derived"
	case StatusCited:
		return "cited"
	case StatusMeasured:
		return "measured"
	}
	return fmt.Sprintf("status(%d)", uint64(s))
}

// RequiresSource reports §7's "a source where `measured` or `cited` demands one".
//
// Both assert support outside the graph, and a claim to have measured or cited something
// without saying what is the assertion without the evidence.
func (s Status) RequiresSource() bool { return s == StatusMeasured || s == StatusCited }

// RequiresGrounds reports §7's "grounds where the status demands them".
//
// `derived` and `inferred` are claims *about other units*, so a unit with neither is claiming
// to follow from nothing.
func (s Status) RequiresGrounds() bool { return s == StatusDerived || s == StatusInferred }

// Source is the §1.1 sub-map. Kind is the integer code, not the name.
type Source struct {
	Kind      uint64
	Reference string
	Captured  string // "YYYY-MM-DD", empty when absent
}

// UnitCore is the hashed content of a unit — not the envelope. The record type code is
// framing, and framing is not content (§2.4).
type UnitCore struct {
	Schema  string
	Gist    string
	Status  Status
	Body    string // empty means absent
	Detail  string // empty means absent
	Deps    [][]byte
	Grounds [][]byte
	Source  *Source
	Payload []byte // nil means absent
	// Extra carries unknown keys (≥ 9) verbatim, so a unit that round-tripped through an
	// older implementation still hashes to the value it had (rule X). The value is the
	// already-encoded CBOR item, because this package cannot know its shape.
	Extra map[uint64][]byte
}

// head writes a CBOR head in shortest form (§3, constraint 2).
func head(major byte, arg uint64) []byte {
	switch {
	case arg < 24:
		return []byte{major<<5 | byte(arg)}
	case arg <= 0xFF:
		return []byte{major<<5 | 24, byte(arg)}
	case arg <= 0xFFFF:
		return []byte{major<<5 | 25, byte(arg >> 8), byte(arg)}
	case arg <= 0xFFFFFFFF:
		return []byte{major<<5 | 26, byte(arg >> 24), byte(arg >> 16), byte(arg >> 8), byte(arg)}
	}
	return []byte{
		major<<5 | 27,
		byte(arg >> 56), byte(arg >> 48), byte(arg >> 40), byte(arg >> 32),
		byte(arg >> 24), byte(arg >> 16), byte(arg >> 8), byte(arg),
	}
}

// encodeText normalises to NFC first (§3, constraint 6).
//
// Normalisation is part of identity rather than presentation: two editors typing the same word
// differently must produce one unit. So it happens in the encoder rather than being assumed of
// the caller — which is what constraint 6 says to do, in the sentence recording that the
// reference implementation once assumed it.
func encodeText(s string) []byte {
	raw := []byte(norm.NFC.String(s))
	return append(head(3, uint64(len(raw))), raw...)
}

func encodeBytes(b []byte) []byte {
	return append(head(2, uint64(len(b))), b...)
}

func encodeUint(n uint64) []byte { return head(0, n) }

// encodeUidSet writes a set of uids: deduplicated, ascending by uid bytes (§2.2).
//
// The Rust holds these in a BTreeSet, which is the same order. A set has no insertion order to
// preserve and a canonical encoding cannot invent one.
func encodeUidSet(uids [][]byte) ([]byte, error) {
	seen := make(map[string]struct{}, len(uids))
	ordered := make([][]byte, 0, len(uids))
	for _, u := range uids {
		if len(u) != UidLen {
			return nil, fmt.Errorf("a uid is %d bytes; got %d", UidLen, len(u))
		}
		if _, dup := seen[string(u)]; dup {
			continue
		}
		seen[string(u)] = struct{}{}
		ordered = append(ordered, u)
	}
	sort.Slice(ordered, func(i, j int) bool { return string(ordered[i]) < string(ordered[j]) })

	out := head(4, uint64(len(ordered)))
	for _, u := range ordered {
		out = append(out, encodeBytes(u)...)
	}
	return out, nil
}

type entry struct {
	key uint64
	val []byte
}

// encodeMap writes entries sorted by integer key, ascending, with no duplicates
// (§3, constraint 4).
func encodeMap(entries []entry) ([]byte, error) {
	sort.Slice(entries, func(i, j int) bool { return entries[i].key < entries[j].key })
	for i := 1; i < len(entries); i++ {
		if entries[i].key == entries[i-1].key {
			return nil, fmt.Errorf("duplicate key %d in a canonical map", entries[i].key)
		}
	}
	out := head(5, uint64(len(entries)))
	for _, e := range entries {
		out = append(out, encodeUint(e.key)...)
		out = append(out, e.val...)
	}
	return out, nil
}

func (s *Source) encode() ([]byte, error) {
	entries := []entry{
		{keySourceKind, encodeUint(s.Kind)},
		{keySourceReference, encodeText(s.Reference)},
	}
	if s.Captured != "" {
		entries = append(entries, entry{keySourceCaptured, encodeText(s.Captured)})
	}
	return encodeMap(entries)
}

// Validate is §7's shape clause, and the reason this package can call itself C-Produce rather
// than "C-Read that also hashes".
//
// Every one of these is a thing the reference implementation refuses to construct, so a
// producer that emits one has produced a unit the format says does not exist. They are checked
// here rather than left to the caller because `Uid` calls this: it is not possible to get a
// uid for a malformed unit out of this package, which is the property worth having.
func (u *UnitCore) Validate() error {
	var problems []string
	if u.Schema == "" {
		problems = append(problems, "schema is required (§2.2 key 0)")
	}
	// §7: "a gist present". Whitespace is not presence.
	if trimmed(u.Gist) == "" {
		problems = append(problems, "gist is required and must not be blank (§7)")
	}
	// `detail` elaborates `body`; without one there is nothing to elaborate.
	if u.Detail != "" && u.Body == "" {
		problems = append(problems, "detail without body")
	}
	if u.Status == StatusUnfounded {
		problems = append(problems, "unfounded is reachable only by retraction, never by authoring")
	}
	if u.Status > StatusMeasured {
		problems = append(problems, fmt.Sprintf("status %d is not one of the six", uint64(u.Status)))
	}
	if u.Status.RequiresSource() && u.Source == nil {
		problems = append(problems, fmt.Sprintf("%s without a source (§7)", u.Status))
	}
	if u.Status.RequiresGrounds() && len(u.Grounds) == 0 {
		problems = append(problems, fmt.Sprintf("%s with no grounds (§7)", u.Status))
	}
	for k := range u.Extra {
		if k <= keyPayload {
			problems = append(problems, fmt.Sprintf("key %d is a kernel key, not an extension", k))
		}
	}
	if len(problems) == 0 {
		return nil
	}
	return errors.New("not a well-formed unit: " + joinSemicolons(problems))
}

// CanonicalBytes is the hash input of §2.1.
//
// An absent optional is *omitted*, never encoded as null (§2.2). An empty set is likewise
// omitted: it is indistinguishable from an absent one, so encoding it would give one unit two
// encodings — and two encodings is two uids, which is the whole failure §1 describes.
//
// This does not call Validate. The layout is a separate question from well-formedness, and
// keeping them apart is what lets `uid_test.go` check the encoding against the Rust's bytes
// without every fixture having to satisfy the shape rules as well.
func (u *UnitCore) CanonicalBytes() ([]byte, error) {
	entries := []entry{
		{keySchema, encodeText(u.Schema)},
		{keyGist, encodeText(u.Gist)},
		{keyStatus, encodeUint(uint64(u.Status))},
	}
	if u.Body != "" {
		entries = append(entries, entry{keyBody, encodeText(u.Body)})
	}
	if u.Detail != "" {
		entries = append(entries, entry{keyDetail, encodeText(u.Detail)})
	}
	if len(u.Deps) > 0 {
		enc, err := encodeUidSet(u.Deps)
		if err != nil {
			return nil, fmt.Errorf("deps: %w", err)
		}
		entries = append(entries, entry{keyDeps, enc})
	}
	if len(u.Grounds) > 0 {
		enc, err := encodeUidSet(u.Grounds)
		if err != nil {
			return nil, fmt.Errorf("grounds: %w", err)
		}
		entries = append(entries, entry{keyGrounds, enc})
	}
	if u.Source != nil {
		enc, err := u.Source.encode()
		if err != nil {
			return nil, fmt.Errorf("source: %w", err)
		}
		entries = append(entries, entry{keySource, enc})
	}
	if u.Payload != nil {
		entries = append(entries, entry{keyPayload, encodeBytes(u.Payload)})
	}
	for k, raw := range u.Extra {
		if k <= keyPayload {
			return nil, fmt.Errorf("key %d is a kernel key, not an extension", k)
		}
		entries = append(entries, entry{k, raw})
	}
	return encodeMap(entries)
}

// Uid is §2.1: BLAKE3 over the canonical bytes, status included — which is §2.3.
//
// Validate runs first, on purpose. A uid for a unit that could never legally exist is a
// number, not an identity, and handing one out would let a caller build references on it.
func (u *UnitCore) Uid() ([]byte, error) {
	if err := u.Validate(); err != nil {
		return nil, err
	}
	b, err := u.CanonicalBytes()
	if err != nil {
		return nil, err
	}
	return Blake3(b), nil
}

func trimmed(s string) string {
	start, end := 0, len(s)
	for start < end && isSpace(s[start]) {
		start++
	}
	for end > start && isSpace(s[end-1]) {
		end--
	}
	return s[start:end]
}

func isSpace(c byte) bool {
	return c == ' ' || c == '\t' || c == '\n' || c == '\r' || c == '\v' || c == '\f'
}

func joinSemicolons(parts []string) string {
	out := ""
	for i, p := range parts {
		if i > 0 {
			out += "; "
		}
		out += p
	}
	return out
}
