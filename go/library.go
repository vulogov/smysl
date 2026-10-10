package smysl

// The library records and their identities — §2.6 and §3.1 of the format spec.
//
// Until now this implementation had the *names* of records 14, 15 and 18 and nothing else: it
// preserved them verbatim, re-encoded them byte for byte, and reported each as named but not
// understood. That is C-Read and it was honest. It also left the four library identities —
// tid, mid, did, rdid — derived by exactly one implementation, the one that specified them.
//
// §2.6 makes a claim no reading test can reach: that a one-byte domain prefix keeps the four
// kinds apart from each other and from a uid, however their preimages collide. The claim is
// about the construction, and the construction is four lines — the kind of thing that reads as
// obviously right and gets implemented wrong once.

import (
	"bytes"
	"errors"
	"fmt"
	"unicode/utf8"

	"golang.org/x/text/unicode/norm"
)

// The domain byte of each identity is the record type code it names (§2.6), which is what
// makes the table self-explaining rather than four magic numbers.
const (
	TidDomain  byte = 0x0f
	MidDomain  byte = 0x0e
	DidDomain  byte = 0x11
	RdidDomain byte = 0x12
)

// The text prefixes of §2.6.
const (
	TidPrefix  = "t3:"
	MidPrefix  = "m3:"
	DidPrefix  = "d3:"
	RdidPrefix = "r3:"
)

// ErrLibrary is a library record that cannot mean what it says.
var ErrLibrary = errors.New("smysl: library record")

func libErr(format string, args ...any) error {
	return fmt.Errorf("%w: "+format, append([]any{ErrLibrary}, args...)...)
}

// ManifestKeys is the manifest body's key table (§3.1).
var ManifestKeys = map[uint64]string{
	0: "alias", 1: "parts", 2: "lang", 3: "reader", 4: "licence",
	5: "carry", 6: "title", 7: "creators", 8: "published", 9: "identifiers",
	10: "origin", 11: "parent", 12: "parent-kind", 13: "supersedes", 14: "versification",
	15: "lossy", 16: "raw", 17: "part-policy", 18: "calendar",
}

// ManifestRequired lists the keys §3.1 marks required. 11 and 12 are a pair, checked apart.
var ManifestRequired = []uint64{0, 1, 2, 3, 4, 5, 17}

// PartEntryKeys is a part entry's key table, inside a manifest's key 1 (§3.1).
var PartEntryKeys = map[uint64]string{
	0: "tid", 1: "length", 2: "structure", 3: "rdid", 4: "lang",
}

// PartTextKeys is record 15's key table (§3.1).
var PartTextKeys = map[uint64]string{0: "tid", 1: "text"}

// PartReadingKeys is record 18's key table (§3.1).
var PartReadingKeys = map[uint64]string{
	0: "tid", 1: "reader", 2: "segments", 3: "raw",
}

// RedactionKeys is record 19's key table (§3.1). Rule Z: the part this names is to be held no
// longer.
//
// The same four keys a withdrawal has, in the same order — both say "this is no longer to be
// acted on", by whom, when, and optionally why.
var RedactionKeys = map[uint64]string{
	0: "tid", 1: "agent", 2: "ts", 3: "reason",
}

// DatingKeys is record 17's key table (§3.1). A statement about when something happened.
//
// The key numbers are the did's preimage, so they are permanent in a stronger sense than the
// other tables here: renumbering one would change the identity of every dating ever written.
var DatingKeys = map[uint64]string{
	0: "target", 1: "axis", 2: "value", 3: "basis", 4: "agent", 5: "ts",
}

// DatingValueKeys names the three entries of a dating's value map, which holds exactly one.
var DatingValueKeys = map[uint64]string{0: "absolute", 1: "offset", 2: "relative"}

// Axes is a dating's key 1 (§3.1). Rule E names a fourth — known — which no dating can speak
// about: it is the earliest attestation clock of a unit and is never corrected.
var Axes = map[uint64]string{0: "said", 1: "composed", 2: "about"}

// TargetKinds is a dating target's kind (key 0, first element).
var TargetKinds = map[uint64]string{0: "unit", 1: "part", 2: "manifest", 3: "window"}

// Allen holds the interval relations a relative dating may use.
var Allen = map[string]bool{
	"before": true, "after": true, "meets": true, "overlaps": true,
	"during": true, "contains": true, "equals": true,
}

// SegmentKeys is a segment row's key table, inside a reading's key 2 (§3.1).
var SegmentKeys = map[uint64]string{
	0: "start", 1: "end", 2: "level", 3: "locator", 4: "lang",
	5: "speaker", 6: "observed", 7: "ids", 8: "tz_offset",
}

// Carry is a manifest's key 5 (§3.1).
var Carry = map[uint64]string{0: "none", 1: "ref", 2: "text"}

func domainDigest(domain byte, preimage []byte) []byte {
	input := make([]byte, 0, len(preimage)+1)
	input = append(input, domain)
	input = append(input, preimage...)
	return Blake3(input)
}

// Tid derives a part's identity: BLAKE3-256 over 0x0f and the part's normalised bytes (§2.6).
//
// The caller's bytes are hashed as given. IsNormalised is the separate question, and separate
// on purpose: a decoder must be able to compute the tid a record *claims* in order to report
// that it is wrong, which it cannot do if computing it requires the bytes to be right.
func Tid(normalised []byte) []byte { return domainDigest(TidDomain, normalised) }

// Mid derives a manifest's identity: BLAKE3-256 over 0x0e and the canonical CBOR of its body.
func Mid(body []byte) []byte { return domainDigest(MidDomain, body) }

// Did derives a dating's identity: BLAKE3-256 over 0x11 and the canonical CBOR of its body.
//
// The derivation is domain-separated from the other three: a did that collided
// with a mid would be a defect in the table, and a table with a hole in it cannot be checked.
func Did(body []byte) []byte { return domainDigest(DidDomain, body) }

// Rdid derives a reading's identity: BLAKE3-256 over 0x12 and the canonical CBOR of its body.
func Rdid(body []byte) []byte { return domainDigest(RdidDomain, body) }

// StructureHash is BLAKE3-256 of a reading's segment table, with no domain byte (§3.1).
//
// It names a table rather than a record, so there is no record code to prefix it with. That is
// also what lets a reading gain raw metadata, keep its structure hash and change its rdid,
// which is why a part entry carries both.
func StructureHash(segments []byte) []byte { return Blake3(segments) }

const identityAlphabet = "abcdefghijklmnopqrstuvwxyz234567"

// IdentityText writes an identity as its prefix and chars base32 characters (§2.6, §2.1).
//
// 52 is canonical and 26 is a display abbreviation; a record carrying the short form is
// SMY-E071, as it is for a uid.
func IdentityText(prefix string, digest []byte, chars int) (string, error) {
	if len(digest) != 32 {
		return "", libErr("an identity is 32 bytes; got %d", len(digest))
	}
	out := make([]byte, 0, len(prefix)+chars)
	out = append(out, prefix...)
	for i := 0; i < chars; i++ {
		v := 0
		for k := 0; k < 5; k++ {
			bit := i*5 + k
			on := bit < 256 && digest[bit>>3]>>(7-(bit&7))&1 == 1
			v <<= 1
			if on {
				v |= 1
			}
		}
		out = append(out, identityAlphabet[v])
	}
	return string(out), nil
}

// IsNormalised reports whether bytes are a part's normalised form: UTF-8, NFC, LF line
// endings, no byte order mark (§2.6).
//
// Nothing else: no whitespace collapsing and no case folding. A part that is not normalised is
// SMY-E446 on the way in, because two libraries given the same text would otherwise name two
// different parts.
//
// The byte order mark is checked in the bytes rather than after decoding, which is the spelling
// the Node port had to be fixed into: its TextDecoder strips a leading BOM by default, so the
// obvious version of this function decoded EF BB BF away and then truthfully reported that the
// text did not start with U+FEFF. Go has no such default, and the byte check has none to know
// about.
func IsNormalised(data []byte) bool {
	if bytes.HasPrefix(data, []byte{0xef, 0xbb, 0xbf}) {
		return false
	}
	if !utf8.Valid(data) {
		return false
	}
	if bytes.ContainsRune(data, '\r') {
		return false
	}
	return norm.NFC.IsNormal(data)
}

// BodyBytes returns the byte range of a record's body inside the bytes it arrived in.
//
// §2.6 names a mid's preimage as "the canonical CBOR of the manifest body". For a decoded
// record that is a slice of what was read, and taking it rather than re-encoding keeps the
// identity independent of this implementation's encoder — which CanonicalBody then checks on
// its own terms.
func BodyBytes(r *Record) ([]byte, error) {
	if r.Raw == nil {
		return nil, libErr("this record was built here and has no bytes")
	}
	d := &Decoder{Data: r.Raw}
	major, arg, _, err := d.Head()
	if err != nil {
		return nil, err
	}
	if major != 4 || arg != 2 {
		return nil, cborErr("a record is a two-element array")
	}
	if _, err := d.Value(0); err != nil {
		return nil, err
	}
	return r.Raw[d.I:], nil
}

// CanonicalBody reports whether this implementation's encoder reproduces a record's body bytes.
//
// The producer's half of the identity question, asked separately. A false here with a correct
// mid means the encoder disagrees with the reference about canonical form — map key order, a
// shortest-form head, an omitted optional — which is a defect that would surface the first time
// this implementation *wrote* a manifest rather than read one.
func CanonicalBody(r *Record) (bool, error) {
	want, err := BodyBytes(r)
	if err != nil {
		return false, err
	}
	got, err := EncodeOne(r.Body)
	if err != nil {
		return false, err
	}
	return bytes.Equal(got, want), nil
}

func mapBytes(m *Map, key uint64, size int, what string) ([]byte, error) {
	v, ok := m.Get(key)
	if !ok {
		return nil, libErr("%s is missing", what)
	}
	b, ok := v.([]byte)
	if !ok || len(b) != size {
		return nil, libErr("%s is %d bytes", what, size)
	}
	return b, nil
}

// PartEntry is one row of a manifest's key 1 (§3.1).
type PartEntry struct {
	Tid       []byte
	Length    uint64
	Structure []byte
	Rdid      []byte
	Lang      string
	Extra     map[uint64]any
}

// DecodePartEntry reads a part entry.
func DecodePartEntry(v any) (*PartEntry, error) {
	m, ok := v.(*Map)
	if !ok {
		return nil, libErr("a part entry is a map")
	}
	tid, err := mapBytes(m, 0, 32, "a part entry's tid")
	if err != nil {
		return nil, err
	}
	structure, err := mapBytes(m, 2, 32, "a part entry's structure hash")
	if err != nil {
		return nil, err
	}
	rdid, err := mapBytes(m, 3, 32, "a part entry's rdid")
	if err != nil {
		return nil, err
	}
	raw, ok := m.Get(uint64(1))
	if !ok {
		return nil, libErr("a part entry needs key 1 (length)")
	}
	length, ok := raw.(uint64)
	if !ok {
		return nil, libErr("a part entry's length is an unsigned integer")
	}
	e := &PartEntry{Tid: tid, Length: length, Structure: structure, Rdid: rdid,
		Extra: map[uint64]any{}}
	if lang, ok := m.Get(uint64(4)); ok {
		if s, ok := lang.(string); ok {
			e.Lang = s
		}
	}
	for _, p := range m.Entries {
		if k, ok := p.Key.(uint64); ok && k > 4 {
			e.Extra[k] = p.Value
		}
	}
	return e, nil
}

// Manifest is record 14, decoded. Raw is the body's own bytes, which the mid is a hash of.
type Manifest struct {
	Fields map[string]any
	Parts  []*PartEntry
	Body   *Map
	Raw    []byte
}

// DecodeManifest reads a manifest body, enforcing the rules §3.1 states for it.
func DecodeManifest(r *Record) (*Manifest, error) {
	body, ok := r.Body.(*Map)
	if !ok {
		return nil, libErr("a manifest body is a map")
	}
	for _, key := range ManifestRequired {
		if _, ok := body.Get(key); !ok {
			return nil, libErr("a manifest needs key %d (%s)", key, ManifestKeys[key])
		}
	}
	// Key 12 is required with key 11 and meaningless without it; §3.1 says either alone MUST
	// be rejected, for the reason a resolution with one target is.
	_, hasParent := body.Get(uint64(11))
	_, hasKind := body.Get(uint64(12))
	if hasParent != hasKind {
		return nil, libErr("manifest keys 11 (parent) and 12 (parent-kind) travel together")
	}
	// Key 15 has no false encoding: admitting it would give one manifest two byte strings and
	// therefore two mids for one expression.
	if lossy, ok := body.Get(uint64(15)); ok {
		if b, isBool := lossy.(bool); isBool && !b {
			return nil, libErr("manifest key 15 (lossy) has no false encoding")
		}
	}
	carry, _ := body.Get(uint64(5))
	code, ok := carry.(uint64)
	if !ok {
		return nil, libErr("carry is an unsigned integer")
	}
	if _, ok := Carry[code]; !ok {
		return nil, libErr("carry %d is not 0, 1 or 2", code)
	}
	partsAny, _ := body.Get(uint64(1))
	parts, ok := partsAny.([]any)
	if !ok {
		return nil, libErr("a manifest's parts are an array")
	}
	entries := make([]*PartEntry, 0, len(parts))
	for _, p := range parts {
		e, err := DecodePartEntry(p)
		if err != nil {
			return nil, err
		}
		entries = append(entries, e)
	}
	fields := map[string]any{}
	for _, p := range body.Entries {
		if k, ok := p.Key.(uint64); ok {
			if name, named := ManifestKeys[k]; named {
				fields[name] = p.Value
				continue
			}
		}
		fields[fmt.Sprint(p.Key)] = p.Value
	}
	var raw []byte
	if r.Raw != nil {
		b, err := BodyBytes(r)
		if err != nil {
			return nil, err
		}
		raw = b
	}
	return &Manifest{Fields: fields, Parts: entries, Body: body, Raw: raw}, nil
}

// Alias is the expression alias, key 0.
func (m *Manifest) Alias() string {
	s, _ := m.Fields["alias"].(string)
	return s
}

// CarryMode names key 5.
func (m *Manifest) CarryMode() string {
	code, _ := m.Fields["carry"].(uint64)
	return Carry[code]
}

// Mid derives the manifest's identity from the body's own bytes.
func (m *Manifest) Mid() ([]byte, error) {
	if m.Raw == nil {
		return nil, libErr("this manifest has no body bytes")
	}
	return Mid(m.Raw), nil
}

// PartText is record 15 (§3.1).
type PartText struct {
	Tid   []byte
	Text  []byte
	Extra map[uint64]any
}

// DecodePartText reads a part text body.
func DecodePartText(r *Record) (*PartText, error) {
	body, ok := r.Body.(*Map)
	if !ok {
		return nil, libErr("a part text body is a map")
	}
	tid, err := mapBytes(body, 0, 32, "a part text's tid")
	if err != nil {
		return nil, err
	}
	textAny, ok := body.Get(uint64(1))
	if !ok {
		return nil, libErr("a part text needs key 1 (text)")
	}
	text, ok := textAny.([]byte)
	if !ok {
		return nil, libErr("a part text's text is a byte string")
	}
	p := &PartText{Tid: tid, Text: text, Extra: map[uint64]any{}}
	for _, e := range body.Entries {
		if k, ok := e.Key.(uint64); ok && k > 1 {
			p.Extra[k] = e.Value
		}
	}
	return p, nil
}

// Verify reports whether the bytes hash to the claimed tid and are normalised (SMY-E446).
//
// Both, because either alone leaves a hole: unnormalised bytes that hash to their own tid are a
// part no other library would name the same way, and normalised bytes under the wrong tid are a
// substituted file.
//
// A decode never fails on this. §2.6 is explicit: a record that cannot be decoded cannot be
// reported, and one bad part would otherwise stop a whole store from opening.
func (p *PartText) Verify() bool {
	return bytes.Equal(Tid(p.Text), p.Tid) && IsNormalised(p.Text)
}

// PartReading is record 18 (§3.1).
type PartReading struct {
	Tid      []byte
	Reader   string
	Segments any
	RawMeta  any
	Extra    map[uint64]any
	Body     *Map
	BodyRaw  []byte
}

// DecodePartReading reads a reading body.
func DecodePartReading(r *Record) (*PartReading, error) {
	body, ok := r.Body.(*Map)
	if !ok {
		return nil, libErr("a reading body is a map")
	}
	tid, err := mapBytes(body, 0, 32, "a reading's tid")
	if err != nil {
		return nil, err
	}
	readerAny, ok := body.Get(uint64(1))
	if !ok {
		return nil, libErr("a reading needs key 1 (reader)")
	}
	reader, ok := readerAny.(string)
	if !ok {
		return nil, libErr("a reading's reader id is text")
	}
	segments, ok := body.Get(uint64(2))
	if !ok {
		return nil, libErr("a reading needs key 2 (segments)")
	}
	g := &PartReading{Tid: tid, Reader: reader, Segments: segments, Body: body,
		Extra: map[uint64]any{}}
	if meta, ok := body.Get(uint64(3)); ok {
		g.RawMeta = meta
	}
	for _, e := range body.Entries {
		if k, ok := e.Key.(uint64); ok && k > 3 {
			g.Extra[k] = e.Value
		}
	}
	if r.Raw != nil {
		b, err := BodyBytes(r)
		if err != nil {
			return nil, err
		}
		g.BodyRaw = b
	}
	return g, nil
}

// Redaction is record 19: this part's text is to be held no longer (rule Z).
//
// No identity of its own — a redaction is a statement about a part, named by that part's tid —
// so there is nothing here to derive. What a reader of a store does with it is refuse to hold a
// record 15 or 18 for that tid, including one arriving from a peer that never saw the redaction.
type Redaction struct {
	Tid    []byte
	Agent  string
	Ts     any
	Reason []byte
	Extra  map[uint64]any
	Body   *Map
}

// DecodeRedaction decodes a record 19 body.
func DecodeRedaction(r *Record) (*Redaction, error) {
	body, ok := r.Body.(*Map)
	if !ok {
		return nil, libErr("a redaction body is a map")
	}
	tid, err := mapBytes(body, 0, 32, "a redaction's tid")
	if err != nil {
		return nil, err
	}
	agentAny, ok := body.Get(uint64(1))
	if !ok {
		return nil, libErr("a redaction needs key 1 (agent)")
	}
	agent, ok := agentAny.(string)
	if !ok {
		return nil, libErr("a redaction's agent is text")
	}
	ts, ok := body.Get(uint64(2))
	if !ok {
		return nil, libErr("a redaction needs key 2 (ts)")
	}
	g := &Redaction{Tid: tid, Agent: agent, Ts: ts, Body: body, Extra: map[uint64]any{}}
	if _, ok := body.Get(uint64(3)); ok {
		reason, err := mapBytes(body, 3, 32, "a redaction's reason")
		if err != nil {
			return nil, err
		}
		g.Reason = reason
	}
	for _, e := range body.Entries {
		if k, ok := e.Key.(uint64); ok && k > 3 {
			g.Extra[k] = e.Value
		}
	}
	return g, nil
}

// Rdid derives the reading's identity from the body's own bytes.
func (g *PartReading) Rdid() ([]byte, error) {
	if g.BodyRaw == nil {
		return nil, libErr("this reading has no body bytes")
	}
	return Rdid(g.BodyRaw), nil
}

// StructureHash is BLAKE3-256 of the canonical CBOR of the segment table (§3.1).
//
// The table is re-encoded rather than sliced, which makes this a check of the encoder: a
// segment row holds integers and text and no float, so there is nothing for a number type to
// disagree about.
func (g *PartReading) StructureHash() ([]byte, error) {
	encoded, err := EncodeOne(g.Segments)
	if err != nil {
		return nil, err
	}
	return StructureHash(encoded), nil
}

// Rows names a segment table's keys. Unknown keys keep their integer, per rule X.
func (g *PartReading) Rows() ([]map[string]any, error) {
	rows, ok := g.Segments.([]any)
	if !ok {
		return nil, libErr("a segment table is an array")
	}
	out := make([]map[string]any, 0, len(rows))
	for _, r := range rows {
		m, ok := r.(*Map)
		if !ok {
			return nil, libErr("a segment row is a map")
		}
		named := map[string]any{}
		for _, p := range m.Entries {
			if k, ok := p.Key.(uint64); ok {
				if name, isNamed := SegmentKeys[k]; isNamed {
					named[name] = p.Value
					continue
				}
			}
			named[fmt.Sprint(p.Key)] = p.Value
		}
		out = append(out, named)
	}
	return out, nil
}

// VerifyEntry checks a reading against the entry a manifest recorded for it (SMY-E401).
//
// The structure hash is checked first, because when a reader is upgraded under a corpus both
// identities move and the structure is the one that says what changed. An rdid-only mismatch is
// narrower — same segmentation, different raw metadata.
func (g *PartReading) VerifyEntry(e *PartEntry) error {
	if !bytes.Equal(g.Tid, e.Tid) {
		return libErr("this reading is not of that part")
	}
	structure, err := g.StructureHash()
	if err != nil {
		return err
	}
	if !bytes.Equal(structure, e.Structure) {
		return libErr("SMY-E401: the structure hash does not match the part entry")
	}
	rdid, err := g.Rdid()
	if err != nil {
		return err
	}
	if !bytes.Equal(rdid, e.Rdid) {
		return libErr("SMY-E401: the rdid does not match the part entry")
	}
	return nil
}

// DatingTarget is what a dating is about: a unit, a part, a manifest, or a window over a part.
//
// Ident is 32 bytes for the first three kinds. For a window it is the part's tid and the range
// is in From and To, half-open and against the as-recorded observed instant — not the effective
// one, which would make the set of selected units move as the datings applied.
type DatingTarget struct {
	Kind  uint64
	Ident []byte
	From  uint64
	To    uint64
}

// KindName is the word §3.1 gives this target kind.
func (t *DatingTarget) KindName() string { return TargetKinds[t.Kind] }

func decodeDatingTarget(v any) (*DatingTarget, error) {
	outer, ok := v.([]any)
	if !ok || len(outer) != 2 {
		return nil, libErr("a dating target is [kind, id]")
	}
	kind, ok := outer[0].(uint64)
	if !ok {
		return nil, libErr("a dating target kind is an integer")
	}
	if _, named := TargetKinds[kind]; !named {
		return nil, libErr("a dating target kind is 0 to 3, not %d", kind)
	}
	bytes32 := func(v any, what string) ([]byte, error) {
		b, ok := v.([]byte)
		if !ok || len(b) != 32 {
			return nil, libErr("%s is 32 bytes", what)
		}
		return b, nil
	}
	if kind == 3 {
		inner, ok := outer[1].([]any)
		if !ok || len(inner) != 3 {
			return nil, libErr("a window is [tid, from_ms, to_ms]")
		}
		tid, err := bytes32(inner[0], "a window's tid")
		if err != nil {
			return nil, err
		}
		from, okFrom := inner[1].(uint64)
		to, okTo := inner[2].(uint64)
		if !okFrom || !okTo {
			return nil, libErr("a window's bounds are non-negative integers")
		}
		return &DatingTarget{Kind: 3, Ident: tid, From: from, To: to}, nil
	}
	ident, err := bytes32(outer[1], "a dating target's id")
	if err != nil {
		return nil, err
	}
	return &DatingTarget{Kind: kind, Ident: ident}, nil
}

// Dating is record 17: a statement about when something happened.
//
// A record about units, parts and manifests, never an edit to them — correcting a unit's
// observed in place would change its uid, and a store that re-identified its contents whenever
// a clock turned out to be wrong could not be cited.
//
// Its identity is a did (§2.6), because the records that name a dating need one: a withdrawal
// makes it not live and a canonical commitment holds it.
type Dating struct {
	Target *DatingTarget
	Axis   uint64
	// ValueKind is "absolute", "offset" or "relative", and exactly one of the three fields
	// below carries the value.
	ValueKind string
	Absolute  string
	Offset    int64
	Relation  string
	RelTarget *DatingTarget
	Agent     string
	Ts        any
	Basis     []byte
	Extra     map[uint64]any
	Body      *Map
	BodyRaw   []byte
}

// AxisName is the word §3.1 gives this dating's axis.
func (d *Dating) AxisName() string { return Axes[d.Axis] }

// DecodeDating decodes a record 17 body.
func DecodeDating(r *Record) (*Dating, error) {
	body, ok := r.Body.(*Map)
	if !ok {
		return nil, libErr("a dating body is a map")
	}
	for _, key := range []uint64{0, 1, 2, 4, 5} {
		if _, ok := body.Get(key); !ok {
			return nil, libErr("a dating needs key %d (%s)", key, DatingKeys[key])
		}
	}
	targetAny, _ := body.Get(uint64(0))
	target, err := decodeDatingTarget(targetAny)
	if err != nil {
		return nil, err
	}
	axisAny, _ := body.Get(uint64(1))
	axis, ok := axisAny.(uint64)
	if !ok {
		return nil, libErr("a dating's axis is an integer")
	}
	if _, named := Axes[axis]; !named {
		return nil, libErr("a dating's axis is 0, 1 or 2, not %d", axis)
	}
	agentAny, _ := body.Get(uint64(4))
	agent, ok := agentAny.(string)
	if !ok {
		return nil, libErr("a dating's agent is text")
	}
	ts, _ := body.Get(uint64(5))
	d := &Dating{
		Target: target,
		Axis:   axis,
		Agent:  agent,
		Ts:     ts,
		Body:   body,
		Extra:  map[uint64]any{},
	}
	if r.Raw != nil {
		b, err := BodyBytes(r)
		if err != nil {
			return nil, err
		}
		d.BodyRaw = b
	}
	valueAny, _ := body.Get(uint64(2))
	if err := d.decodeValue(valueAny); err != nil {
		return nil, err
	}
	if _, ok := body.Get(uint64(3)); ok {
		basis, err := mapBytes(body, 3, 32, "a dating's basis")
		if err != nil {
			return nil, err
		}
		d.Basis = basis
	}
	for _, e := range body.Entries {
		if k, ok := e.Key.(uint64); ok && k > 5 {
			d.Extra[k] = e.Value
		}
	}
	return d, nil
}

// decodeValue reads the one-entry map of §3.1's key 2.
//
// Exactly one entry, and the count is checked: a two-entry map would be a dating that says two
// things with no rule for which wins.
func (d *Dating) decodeValue(v any) error {
	m, ok := v.(*Map)
	if !ok || len(m.Entries) != 1 {
		return libErr("a dating's value is a one-entry map")
	}
	key, ok := m.Entries[0].Key.(uint64)
	if !ok {
		return libErr("a dating's value key is an integer")
	}
	inner := m.Entries[0].Value
	switch key {
	case 0:
		text, ok := inner.(string)
		if !ok {
			return libErr("an absolute dating's value is EDTF text")
		}
		d.ValueKind, d.Absolute = "absolute", text
		return nil
	case 1:
		// Signed. The first field in this format to carry a negative integer, so a decoder
		// that read CBOR major type 1 as a large positive number would fail here and nowhere
		// else: a clock can be fast as well as slow.
		switch n := inner.(type) {
		case uint64:
			d.ValueKind, d.Offset = "offset", int64(n)
		case int64:
			d.ValueKind, d.Offset = "offset", n
		default:
			return libErr("an offset is an integer of milliseconds")
		}
		return nil
	case 2:
		pair, ok := inner.([]any)
		if !ok || len(pair) != 2 {
			return libErr("a relative dating's value is [allen, target]")
		}
		relation, ok := pair[0].(string)
		if !ok || !Allen[relation] {
			return libErr("that is not an Allen relation this format uses")
		}
		target, err := decodeDatingTarget(pair[1])
		if err != nil {
			return err
		}
		d.ValueKind, d.Relation, d.RelTarget = "relative", relation, target
		return nil
	default:
		return libErr("a dating's value has no key %d", key)
	}
}

// Did derives this dating's identity from the body's own bytes.
func (d *Dating) Did() ([]byte, error) {
	if d.BodyRaw == nil {
		return nil, libErr("this dating has no body bytes")
	}
	return Did(d.BodyRaw), nil
}
