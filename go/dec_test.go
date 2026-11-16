package smysl

// fixtures/tel/dec/cases.json: dec (RFC SMYSL-3 §5), C-Tel-Read.
//
// The fixture's binary64 cases are what a JavaScript engine's String(x) gives — the
// Number::toString §5.3 names — and its token, spelling and CBOR cases are the generator's own
// reading of the RFC. Rust, Python and Node read the same file.

import (
	"encoding/hex"
	"encoding/json"
	"errors"
	"math"
	"math/big"
	"os"
	"path/filepath"
	"strconv"
	"testing"
)

type decCase struct {
	Bits  string `json:"bits"`
	Token string `json:"token"`
	M     string `json:"m"`
	E     int    `json:"e"`
	Error string `json:"error"`
	Text  string `json:"text"`
	Cbor  string `json:"cbor"`
	Why   string `json:"why"`
}

type decCases struct {
	F64       []decCase `json:"f64"`
	Tokens    []decCase `json:"tokens"`
	Spellings []decCase `json:"spellings"`
	Refused   []decCase `json:"refused"`
}

func loadDecCases(t *testing.T) decCases {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join("..", "fixtures", "tel", "dec", "cases.json"))
	if err != nil {
		t.Fatal(err)
	}
	var c decCases
	if err := json.Unmarshal(raw, &c); err != nil {
		t.Fatal(err)
	}
	return c
}

func wantDec(t *testing.T, c decCase) Dec {
	t.Helper()
	m, ok := new(big.Int).SetString(c.M, 10)
	if !ok {
		t.Fatalf("m %q", c.M)
	}
	return Dec{M: m, E: c.E}
}

func sameDec(a, b Dec) bool { return a.M.Cmp(b.M) == 0 && a.E == b.E }

func TestDecBinary64IsTheDecimalECMA262Selects(t *testing.T) {
	c := loadDecCases(t)
	if len(c.F64) < 2000 {
		t.Fatalf("%d cases", len(c.F64))
	}
	for _, k := range c.F64 {
		bits, err := strconv.ParseUint(k.Bits, 16, 64)
		if err != nil {
			t.Fatal(err)
		}
		got, err := DecFromF64(math.Float64frombits(bits))
		if err != nil || !sameDec(got, wantDec(t, k)) {
			t.Errorf("%s: got %v (%v), want %se%d", k.Bits, got, err, k.M, k.E)
		}
	}
}

func TestDecTokensAreReadAsTheRFCSays(t *testing.T) {
	for _, k := range loadDecCases(t).Tokens {
		got, err := DecParseToken(k.Token)
		if k.Error != "" {
			var de *DecError
			if !errors.As(err, &de) || de.Kind != k.Error {
				t.Errorf("%q: want %s, got %v", k.Token, k.Error, err)
			}
			continue
		}
		if err != nil || !sameDec(got, wantDec(t, k)) {
			t.Errorf("%q: got %v (%v)", k.Token, got, err)
		}
	}
}

func TestDecOneSpellingAndOneEncodingPerValue(t *testing.T) {
	for _, k := range loadDecCases(t).Spellings {
		v := wantDec(t, k)
		if v.Spell() != k.Text {
			t.Errorf("spell %se%d = %q, want %q", k.M, k.E, v.Spell(), k.Text)
		}
		if back, err := DecParseToken(k.Text); err != nil || !sameDec(back, v) {
			t.Errorf("%q does not read back: %v", k.Text, err)
		}
		if hex.EncodeToString(v.ToCbor()) != k.Cbor {
			t.Errorf("%q encodes as %x, want %s", k.Text, v.ToCbor(), k.Cbor)
		}
		raw, _ := hex.DecodeString(k.Cbor)
		if got, err := DecFromCbor(raw); err != nil || !sameDec(got, v) {
			t.Errorf("%s does not decode: %v", k.Cbor, err)
		}
	}
}

func TestDecNonCanonicalCborIsE602(t *testing.T) {
	for _, k := range loadDecCases(t).Refused {
		raw, _ := hex.DecodeString(k.Cbor)
		_, err := DecFromCbor(raw)
		var de *DecError
		if !errors.As(err, &de) || de.Kind != "E602" {
			t.Errorf("%s (%s): got %v", k.Cbor, k.Why, err)
		}
	}
}
