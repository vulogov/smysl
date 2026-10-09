package smysl

// The library records and the four identities — §2.6 and §3.1, added in 1.10.
//
// Reading a document never requires deriving an identity, which is why this file exists: three
// independent readers round-tripped every library fixture byte for byte while knowing nothing
// about what a tid *is*. §2.6's claim — that a one-byte domain prefix keeps four kinds of
// identity apart from each other and from a uid — had been checked by exactly one
// implementation, the one that wrote it down.
//
// Two levels, and the second is the one that matters. Deriving an identity from the hex
// preimage the fixture hands over checks the hash. Deriving it from the record as decoded, and
// separately asking whether this implementation's encoder reproduces those body bytes, checks
// the half a producer needs — which a fixture of hex strings cannot reach.

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"os"
	"strings"
	"testing"
)

type libPart struct {
	TidHex         string `json:"tid_hex"`
	Length         uint64 `json:"length"`
	TextHex        string `json:"text_hex"`
	RdidHex        string `json:"rdid_hex"`
	ReadingBodyHex string `json:"reading_body_hex"`
	StructureHex   string `json:"structure_hex"`
}

type libVectors struct {
	Manifest struct {
		Alias   string `json:"alias"`
		MidHex  string `json:"mid_hex"`
		BodyHex string `json:"body_hex"`
	} `json:"manifest"`
	Parts       []libPart         `json:"parts"`
	DomainBytes map[string]uint64 `json:"domain_bytes"`
}

func libFixtures(t *testing.T) (libVectors, []*Record) {
	t.Helper()
	raw, err := os.ReadFile("../fixtures/library/wire/ids.json")
	if err != nil {
		t.Fatalf("the vectors are missing: %v", err)
	}
	var v libVectors
	if err := json.Unmarshal(raw, &v); err != nil {
		t.Fatalf("the vectors do not parse: %v", err)
	}
	// A suite with no fixtures passes vacuously, which is the failure this catches.
	if v.Manifest.MidHex == "" || len(v.Parts) < 2 {
		t.Fatal("one part would not exercise a second reading")
	}
	data, err := os.ReadFile("../fixtures/library/wire/records.cbor")
	if err != nil {
		t.Fatalf("the records are missing: %v", err)
	}
	records, err := DecodeStore(data)
	if err != nil {
		t.Fatalf("the records do not decode: %v", err)
	}
	return v, records
}

func unhex(t *testing.T, s string) []byte {
	t.Helper()
	b, err := hex.DecodeString(s)
	if err != nil {
		t.Fatalf("bad hex in a fixture: %v", err)
	}
	return b
}

func of(records []*Record, code uint64) []*Record {
	var out []*Record
	for _, r := range records {
		if r.Code == code {
			out = append(out, r)
		}
	}
	return out
}

// §2.6: "The domain byte is the record type code the identity names."
//
// Not an arbitrary table, and worth asserting rather than transcribing: a tid prefixed with
// the manifest's byte would still produce stable, agreeing identities inside any one
// implementation and would silently be a different format.
func TestDomainBytesAreTheRecordCodesTheyName(t *testing.T) {
	v, _ := libFixtures(t)
	if TidDomain != 15 || MidDomain != 14 || DidDomain != 17 || RdidDomain != 18 {
		t.Fatalf("domain bytes are %d/%d/%d/%d", TidDomain, MidDomain, DidDomain, RdidDomain)
	}
	want := map[string]uint64{"tid": 15, "mid": 14, "did": 17, "rdid": 18}
	for name, code := range want {
		if v.DomainBytes[name] != code {
			t.Errorf("%s: the fixture says %d, the spec says %d", name, v.DomainBytes[name], code)
		}
	}
}

// The hash, checked against the reference's vectors.
func TestIdentitiesComeOutOfTheirPreimages(t *testing.T) {
	v, _ := libFixtures(t)
	if got := hex.EncodeToString(Mid(unhex(t, v.Manifest.BodyHex))); got != v.Manifest.MidHex {
		t.Errorf("mid: got %s, want %s", got, v.Manifest.MidHex)
	}
	for i, p := range v.Parts {
		text := unhex(t, p.TextHex)
		if got := hex.EncodeToString(Tid(text)); got != p.TidHex {
			t.Errorf("part %d tid: got %s, want %s", i, got, p.TidHex)
		}
		if uint64(len(text)) != p.Length {
			t.Errorf("part %d length: %d bytes against a recorded %d", i, len(text), p.Length)
		}
		got := hex.EncodeToString(Rdid(unhex(t, p.ReadingBodyHex)))
		if got != p.RdidHex {
			t.Errorf("part %d rdid: got %s, want %s", i, got, p.RdidHex)
		}
	}
}

// §2.6's claim, on a preimage shared by all four.
//
// The guarantee is not probabilistic and is not about BLAKE3: the digests differ because their
// *inputs* differ in the first byte, whatever the rest is. A construction that hashed the
// preimage alone and labelled the result afterwards would pass every other test in this file.
func TestNoTwoKindsOfIdentityCollide(t *testing.T) {
	shared := []byte("the same bytes under four names")
	seen := map[string]string{}
	for name, digest := range map[string][]byte{
		"tid":  Tid(shared),
		"mid":  Mid(shared),
		"did":  Did(shared),
		"rdid": Rdid(shared),
	} {
		key := hex.EncodeToString(digest)
		if other, clash := seen[key]; clash {
			t.Fatalf("%s and %s are the same digest", name, other)
		}
		seen[key] = name
	}

	// And none is the uid of anything: a uid's preimage is a canonical CBOR map, whose first
	// byte is 0xa0-0xbf, and a domain byte is 0x0e-0x12. The ranges cannot overlap.
	for _, d := range []byte{TidDomain, MidDomain, DidDomain, RdidDomain} {
		if d >= 0xa0 {
			t.Errorf("domain byte %#x is in the range a CBOR map head occupies", d)
		}
	}

	// The structure hash is the one digest with no domain byte, because it names a table rather
	// than a record. It must not be computed as if it had one.
	if bytes.Equal(StructureHash(shared), Tid(shared)) {
		t.Error("the structure hash is being domain-separated")
	}
	if !bytes.Equal(StructureHash(shared), Blake3(shared)) {
		t.Error("the structure hash is not a plain digest of the table")
	}
}

// §2.6 and §2.1: 52 is canonical, 26 is a display abbreviation.
func TestTheTextFormIsThePrefixAndFiftyTwoCharacters(t *testing.T) {
	v, _ := libFixtures(t)
	digest := unhex(t, v.Manifest.MidHex)
	canonical, err := IdentityText(MidPrefix, digest, 52)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasPrefix(canonical, "m3:") || len(canonical) != 3+52 {
		t.Fatalf("canonical form is %q", canonical)
	}
	short, err := IdentityText(MidPrefix, digest, 26)
	if err != nil {
		t.Fatal(err)
	}
	if len(short) != 3+26 || !strings.HasPrefix(canonical, short) {
		t.Errorf("the short form %q is not a prefix of %q", short, canonical)
	}
	if _, err := IdentityText(TidPrefix, []byte("too short"), 52); err == nil {
		t.Error("a digest that is not 32 bytes must be refused")
	}
}

// C-Read, unchanged: the decode is new, the byte-for-byte promise is not.
func TestTheRecordFixtureRoundTripsAndNamesEveryLibraryCode(t *testing.T) {
	_, records := libFixtures(t)
	data, err := os.ReadFile("../fixtures/library/wire/records.cbor")
	if err != nil {
		t.Fatal(err)
	}
	out, err := EncodeStore(records)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(out, data) {
		t.Error("the fixture did not round-trip")
	}
	for _, code := range []uint64{14, 15, 18} {
		if len(of(records, code)) == 0 {
			t.Errorf("record %d is not in the fixture", code)
		}
	}
	// The fixture also carries codes this implementation still does not understand, which keeps
	// the distinction in understoodRecords an observed fact rather than a claim.
	var unknown int
	for _, code := range []uint64{16, 17, 19} {
		unknown += len(of(records, code))
	}
	if unknown == 0 {
		t.Error("nothing in the fixture is named-but-not-understood")
	}
	for _, r := range records {
		if r.IsKnown() != understoodRecords[r.Code] {
			t.Errorf("record %d: IsKnown disagrees with the table", r.Code)
		}
	}
}

func TestAManifestDecodesIntoItsNamedFields(t *testing.T) {
	v, records := libFixtures(t)
	m, err := DecodeManifest(of(records, 14)[0])
	if err != nil {
		t.Fatal(err)
	}
	if m.Alias() != v.Manifest.Alias {
		t.Errorf("alias %q, want %q", m.Alias(), v.Manifest.Alias)
	}
	if m.CarryMode() != "text" {
		t.Errorf("carry %q", m.CarryMode())
	}
	if m.Fields["reader"] != "osis/1" || m.Fields["licence"] != "public-domain" {
		t.Errorf("reader %v licence %v", m.Fields["reader"], m.Fields["licence"])
	}
	if m.Fields["part-policy"] == nil {
		t.Error("a manifest must record its part policy")
	}
	if len(m.Parts) != len(v.Parts) {
		t.Fatalf("%d parts, want %d", len(m.Parts), len(v.Parts))
	}
	for i, e := range m.Parts {
		if hex.EncodeToString(e.Tid) != v.Parts[i].TidHex {
			t.Errorf("part %d tid", i)
		}
		if e.Length != v.Parts[i].Length {
			t.Errorf("part %d length %d, want %d", i, e.Length, v.Parts[i].Length)
		}
		if hex.EncodeToString(e.Structure) != v.Parts[i].StructureHex {
			t.Errorf("part %d structure hash", i)
		}
		if hex.EncodeToString(e.Rdid) != v.Parts[i].RdidHex {
			t.Errorf("part %d rdid", i)
		}
	}
}

// The mid is over the body's own bytes, and the encoder is checked separately.
//
// Taking the identity from the sliced body is what §2.6 names as the preimage, and asking
// whether this implementation's encoder would have produced those bytes is a different question
// with a different answer. Keeping them apart is what the Node port *had* to do — JavaScript
// has one number type, so a float in a manifest's key 16 would re-encode as an integer and move
// the mid — and doing the same here means the three ports make the same claim in the same shape.
func TestTheMidIsOverTheBodyBytesAndTheEncoderAgrees(t *testing.T) {
	v, records := libFixtures(t)
	record := of(records, 14)[0]
	m, err := DecodeManifest(record)
	if err != nil {
		t.Fatal(err)
	}
	mid, err := m.Mid()
	if err != nil {
		t.Fatal(err)
	}
	if got := hex.EncodeToString(mid); got != v.Manifest.MidHex {
		t.Errorf("mid: got %s, want %s", got, v.Manifest.MidHex)
	}
	ok, err := CanonicalBody(record)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Error("the encoder disagrees with the reference about canonical form")
	}
	body, err := BodyBytes(record)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(body, unhex(t, v.Manifest.BodyHex)) {
		t.Error("the sliced body is not the preimage the vectors record")
	}
}

func TestAPartTextVerifiesAgainstItsTidAndItsNormalisation(t *testing.T) {
	v, records := libFixtures(t)
	parts := of(records, 15)
	if len(parts) != len(v.Parts) {
		t.Fatalf("%d part texts, want %d", len(parts), len(v.Parts))
	}
	var nonAscii bool
	for i, r := range parts {
		p, err := DecodePartText(r)
		if err != nil {
			t.Fatal(err)
		}
		if hex.EncodeToString(p.Tid) != v.Parts[i].TidHex {
			t.Errorf("part %d tid", i)
		}
		if !p.Verify() {
			t.Errorf("part %d does not verify", i)
		}
		if !IsNormalised(p.Text) {
			t.Errorf("part %d is not normalised", i)
		}
		for _, b := range p.Text {
			if b > 0x7f {
				nonAscii = true
			}
		}
	}
	// One of them is Cyrillic, which is what makes the NFC half more than a formality: a reader
	// that normalised on the way in or out would move the tid.
	if !nonAscii {
		t.Error("every fixture part is ASCII; NFC is then untested")
	}
}

func TestAReadingReproducesItsRdidAndItsStructureHash(t *testing.T) {
	v, records := libFixtures(t)
	readings := of(records, 18)
	if len(readings) != len(v.Parts) {
		t.Fatalf("%d readings, want %d", len(readings), len(v.Parts))
	}
	for i, r := range readings {
		g, err := DecodePartReading(r)
		if err != nil {
			t.Fatal(err)
		}
		rdid, err := g.Rdid()
		if err != nil {
			t.Fatal(err)
		}
		if hex.EncodeToString(rdid) != v.Parts[i].RdidHex {
			t.Errorf("reading %d rdid", i)
		}
		sh, err := g.StructureHash()
		if err != nil {
			t.Fatal(err)
		}
		if hex.EncodeToString(sh) != v.Parts[i].StructureHex {
			t.Errorf("reading %d structure hash", i)
		}
		if g.Reader != "osis/1" {
			t.Errorf("reading %d reader %q", i, g.Reader)
		}
		rows, err := g.Rows()
		if err != nil {
			t.Fatal(err)
		}
		if len(rows) == 0 {
			t.Errorf("reading %d has no rows", i)
		}
		for _, row := range rows {
			if _, ok := row["start"]; !ok {
				t.Errorf("reading %d: a row has no start", i)
			}
			if _, ok := row["locator"]; !ok {
				t.Errorf("reading %d: a row has no locator", i)
			}
		}
	}
}

// SMY-E401, which is why a part entry carries both identities: they move together when a reader
// changes its segmentation and separately when it changes only its raw metadata.
func TestAReadingChecksItselfAgainstTheManifestEntry(t *testing.T) {
	_, records := libFixtures(t)
	m, err := DecodeManifest(of(records, 14)[0])
	if err != nil {
		t.Fatal(err)
	}
	readings := map[string]*PartReading{}
	for _, r := range of(records, 18) {
		g, err := DecodePartReading(r)
		if err != nil {
			t.Fatal(err)
		}
		readings[hex.EncodeToString(g.Tid)] = g
	}
	for _, e := range m.Parts {
		g := readings[hex.EncodeToString(e.Tid)]
		if g == nil {
			t.Fatalf("no reading for part %s", hex.EncodeToString(e.Tid))
		}
		if err := g.VerifyEntry(e); err != nil {
			t.Errorf("a correct fixture must verify: %v", err)
		}
	}

	e := m.Parts[0]
	g := readings[hex.EncodeToString(e.Tid)]
	tampered := &PartEntry{Tid: e.Tid, Length: e.Length, Structure: make([]byte, 32), Rdid: e.Rdid}
	err = g.VerifyEntry(tampered)
	if err == nil || !strings.Contains(err.Error(), "SMY-E401") {
		t.Errorf("a tampered entry must be SMY-E401, got %v", err)
	}
}

// §2.6: a part whose bytes do not match its tid MUST NOT fail the decode.
//
// "A record that cannot be decoded cannot be reported, and one bad part would otherwise stop a
// whole store from opening." So the decode succeeds, Verify is false, and the caller is the one
// that raises SMY-E446.
func TestAPartWhoseBytesDoNotMatchItsTidDecodesAndFailsVerification(t *testing.T) {
	body := &Map{Entries: []Pair{
		{Key: uint64(0), Value: make([]byte, 32)},
		{Key: uint64(1), Value: []byte("not the bytes that hash to zero\n")},
	}}
	raw, err := EncodeOne([]any{uint64(15), body})
	if err != nil {
		t.Fatal(err)
	}
	records, err := DecodeStore(raw)
	if err != nil {
		t.Fatalf("a mismatched part must still decode: %v", err)
	}
	p, err := DecodePartText(records[0])
	if err != nil {
		t.Fatal(err)
	}
	if p.Verify() {
		t.Error("a part under the wrong tid must not verify")
	}
	back, err := EncodeStore(records)
	if err != nil {
		t.Fatal(err)
	}
	if !bytes.Equal(back, raw) {
		t.Error("and it still round-trips")
	}
}

// Both halves of SMY-E446, because either alone leaves a hole.
//
// Bytes that hash to their own tid but carry CRLF are a part no other library would name the
// same way; the tid agrees and the part is still wrong. The byte order mark case is the one the
// Node port got wrong — its TextDecoder strips a leading BOM by default, so the check passed
// exactly the input the spec forbids.
func TestUnnormalisedBytesFailVerificationEvenUnderTheirOwnTid(t *testing.T) {
	crlf := []byte("a line\r\nand another\n")
	p := &PartText{Tid: Tid(crlf), Text: crlf}
	if p.Verify() {
		t.Error("CRLF must not verify even under its own tid")
	}
	for _, bad := range [][]byte{
		crlf,
		append([]byte{0xef, 0xbb, 0xbf}, "with a byte order mark\n"...),
		[]byte("cafe\u0301\n"), // decomposed: e + combining acute, which NFC composes
		{0xff, 0xfe, 0x0a},     // not UTF-8 at all
	} {
		if IsNormalised(bad) {
			t.Errorf("%q must not be normalised", bad)
		}
	}
	if !IsNormalised([]byte("caf\u00e9\n")) {
		t.Error("composed NFC text must be normalised")
	}
}

func baseManifest() *Map {
	return &Map{Entries: []Pair{
		{Key: uint64(0), Value: "a"},
		{Key: uint64(1), Value: []any{}},
		{Key: uint64(2), Value: "en"},
		{Key: uint64(3), Value: "txt/1"},
		{Key: uint64(4), Value: "unknown"},
		{Key: uint64(5), Value: uint64(0)},
		{Key: uint64(17), Value: "p"},
	}}
}

func withKey(m *Map, key uint64, value any) *Map {
	out := &Map{Entries: append([]Pair{}, m.Entries...)}
	out.Entries = append(out.Entries, Pair{Key: key, Value: value})
	return out
}

func withoutKey(m *Map, key uint64) *Map {
	out := &Map{}
	for _, p := range m.Entries {
		if k, ok := p.Key.(uint64); ok && k == key {
			continue
		}
		out.Entries = append(out.Entries, p)
	}
	return out
}

// §3.1: "either alone MUST be rejected, for the reason a resolution with one target is".
func TestManifestKeysElevenAndTwelveTravelTogether(t *testing.T) {
	if _, err := DecodeManifest(&Record{Code: 14, Body: baseManifest()}); err != nil {
		t.Fatalf("the control must decode: %v", err)
	}
	for _, m := range []*Map{
		withKey(baseManifest(), 11, make([]byte, 32)),
		withKey(baseManifest(), 12, "translation"),
	} {
		_, err := DecodeManifest(&Record{Code: 14, Body: m})
		if err == nil || !strings.Contains(err.Error(), "travel together") {
			t.Errorf("one of 11/12 alone must be rejected, got %v", err)
		}
	}
	both := withKey(withKey(baseManifest(), 11, make([]byte, 32)), 12, "translation")
	if _, err := DecodeManifest(&Record{Code: 14, Body: both}); err != nil {
		t.Errorf("both together must decode: %v", err)
	}
}

// §3.1: key 15 has no false encoding.
//
// Admitting it would mean two byte strings for one manifest, and therefore two mids for one
// expression — the failure content addressing cannot survive.
func TestLossyFalseIsRefusedBecauseItWouldGiveOneManifestTwoMids(t *testing.T) {
	_, err := DecodeManifest(&Record{Code: 14, Body: withKey(baseManifest(), 15, false)})
	if err == nil || !strings.Contains(err.Error(), "no false encoding") {
		t.Errorf("lossy: false must be rejected, got %v", err)
	}
	truthy := withKey(baseManifest(), 15, true)
	if _, err := DecodeManifest(&Record{Code: 14, Body: truthy}); err != nil {
		t.Errorf("lossy: true must decode: %v", err)
	}
	absentBytes, err := EncodeOne(baseManifest())
	if err != nil {
		t.Fatal(err)
	}
	presentBytes, err := EncodeOne(truthy)
	if err != nil {
		t.Fatal(err)
	}
	if bytes.Equal(Mid(absentBytes), Mid(presentBytes)) {
		t.Error("an absent key 15 and a true one must be different manifests")
	}
}

func TestAManifestMissingARequiredKeyIsRefused(t *testing.T) {
	for _, key := range ManifestRequired {
		_, err := DecodeManifest(&Record{Code: 14, Body: withoutKey(baseManifest(), key)})
		if err == nil {
			t.Errorf("a manifest without key %d (%s) must be refused", key, ManifestKeys[key])
		}
	}
}

func TestCarryOutsideItsThreeValuesIsRefused(t *testing.T) {
	bad := withoutKey(baseManifest(), 5)
	bad = withKey(bad, 5, uint64(3))
	_, err := DecodeManifest(&Record{Code: 14, Body: bad})
	if err == nil || !strings.Contains(err.Error(), "carry") {
		t.Errorf("carry 3 must be refused, got %v", err)
	}
	if Carry[0] != "none" || Carry[1] != "ref" || Carry[2] != "text" {
		t.Error("the carry table has moved")
	}
}
