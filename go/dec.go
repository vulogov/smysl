package smysl

// dec, the canonical decimal of RFC SMYSL-3 §5 (C-Tel-Read).
//
// A value is m × 10^e. On the wire it is a CBOR array of two shortest-form integers, and one
// value has exactly one encoding: no trailing zero in the mantissa, zero as [0, 0], a mantissa
// magnitude of at most 2^64 − 1 (the sign rides on the major type, so the one CBOR integer
// outside the range, −2^64, is refused) and an exponent in [−400, 400] (§5.2). A decoder refuses
// every other encoding rather than normalising it, because two encodings of one value would be
// two uids. That refusal is SMY-E602.
//
// Written from the RFC's text. The mantissa is a big.Int: it can need 65 bits, sign plus uint64,
// and §5.2 says so.

import (
	"fmt"
	"math"
	"math/big"
	"strconv"
	"strings"
)

const (
	decEMin = -400
	decEMax = 400
)

var (
	decMMax = new(big.Int).Sub(new(big.Int).Lsh(big.NewInt(1), 64), big.NewInt(1))
	bigTen  = big.NewInt(10)
)

// Dec is a canonical decimal, M × 10^E.
type Dec struct {
	M *big.Int
	E int
}

// DecError says why a value is not a dec. Kind is "syntax", "not-a-value", "out-of-range" or
// "E602".
type DecError struct {
	Kind string
	Why  string
}

func (e *DecError) Error() string { return e.Kind + ": " + e.Why }

func decErr(kind, format string, args ...any) error {
	return &DecError{Kind: kind, Why: fmt.Sprintf(format, args...)}
}

// DecNormalise moves trailing zeros into the exponent, makes zero [0, 0], and checks the range.
func DecNormalise(m *big.Int, e int64) (Dec, error) {
	if m.Sign() == 0 {
		return Dec{M: new(big.Int), E: 0}, nil
	}
	m = new(big.Int).Set(m)
	q, r := new(big.Int), new(big.Int)
	for {
		q.QuoRem(m, bigTen, r)
		if r.Sign() != 0 {
			break
		}
		m.Set(q)
		e++
	}
	if new(big.Int).Abs(m).Cmp(decMMax) > 0 || e < decEMin || e > decEMax {
		return Dec{}, decErr("out-of-range", "%se%d", m, e)
	}
	return Dec{M: m, E: int(e)}, nil
}

// DecParseToken reads a decimal token exactly as written, never through binary64 (§5.3).
//
// SPEC: §5.3 names the sources but not one grammar. Read here as an optional sign, digits with
// at most one point and at least one digit, and an optional exponent. NaN and the infinities
// are refused as not values (§5.2 rule 5), not as syntax.
func DecParseToken(token string) (Dec, error) {
	low := strings.ToLower(strings.TrimLeft(token, "+-"))
	if low == "nan" || low == "inf" || low == "infinity" {
		return Dec{}, decErr("not-a-value", "%s", token)
	}
	s := token
	negative := false
	if s != "" && (s[0] == '+' || s[0] == '-') {
		negative = s[0] == '-'
		s = s[1:]
	}
	mant, exp := s, ""
	if k := strings.IndexAny(s, "eE"); k >= 0 {
		mant, exp = s[:k], s[k+1:]
		if exp == "" {
			return Dec{}, decErr("syntax", "%s", token)
		}
	}
	intPart, fracPart := mant, ""
	if k := strings.IndexByte(mant, '.'); k >= 0 {
		intPart, fracPart = mant[:k], mant[k+1:]
	}
	if intPart+fracPart == "" || !allDigits(intPart) || !allDigits(fracPart) {
		return Dec{}, decErr("syntax", "%s", token)
	}
	e := new(big.Int)
	if exp != "" {
		digits := strings.TrimLeft(exp, "+-")
		if len(exp)-len(digits) > 1 || digits == "" || !allDigits(digits) {
			return Dec{}, decErr("syntax", "%s", token)
		}
		e.SetString(digits, 10)
		if exp[0] == '-' {
			e.Neg(e)
		}
	}
	digits := strings.TrimLeft(intPart+fracPart, "0")
	trimmed := strings.TrimRight(digits, "0")
	if trimmed == "" {
		return Dec{M: new(big.Int), E: 0}, nil
	}
	if len(trimmed) > 20 {
		return Dec{}, decErr("out-of-range", "%s", token)
	}
	e.Sub(e, big.NewInt(int64(len(fracPart))))
	e.Add(e, big.NewInt(int64(len(digits)-len(trimmed))))
	if !e.IsInt64() || e.Int64() < decEMin-20 || e.Int64() > decEMax+20 {
		return Dec{}, decErr("out-of-range", "%s", token)
	}
	m, _ := new(big.Int).SetString(trimmed, 10)
	if negative {
		m.Neg(m)
	}
	return DecNormalise(m, e.Int64())
}

func allDigits(s string) bool {
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return false
		}
	}
	return true
}

// DecFromF64 is a binary64 as ECMA-262's Number::toString selects it (§5.3): the shortest
// digits that round-trip and, among those, the closest. strconv's shortest formatting makes
// the same choice, and the fixture written by a JavaScript engine holds it to that.
func DecFromF64(x float64) (Dec, error) {
	if math.IsNaN(x) || math.IsInf(x, 0) {
		return Dec{}, decErr("not-a-value", "%v", x)
	}
	if x == 0 {
		return Dec{M: new(big.Int), E: 0}, nil
	}
	return DecParseToken(strconv.FormatFloat(x, 'e', -1, 64))
}

// Spell is the one text spelling (§5.3): plain decimal, no exponent, no '+'.
func (d Dec) Spell() string {
	neg := d.M.Sign() < 0
	digits := new(big.Int).Abs(d.M).String()
	var out string
	switch {
	case d.E >= 0 && d.M.Sign() == 0:
		out = "0"
	case d.E >= 0:
		out = digits + strings.Repeat("0", d.E)
	case len(digits) > -d.E:
		out = digits[:len(digits)+d.E] + "." + digits[len(digits)+d.E:]
	default:
		out = "0." + strings.Repeat("0", -d.E-len(digits)) + digits
	}
	if neg {
		return "-" + out
	}
	return out
}

func decHead(out []byte, major byte, arg uint64) []byte {
	switch {
	case arg < 24:
		return append(out, major<<5|byte(arg))
	case arg <= 0xff:
		return append(out, major<<5|24, byte(arg))
	case arg <= 0xffff:
		return append(out, major<<5|25, byte(arg>>8), byte(arg))
	case arg <= 0xffffffff:
		return append(out, major<<5|26, byte(arg>>24), byte(arg>>16), byte(arg>>8), byte(arg))
	}
	out = append(out, major<<5|27)
	for s := 56; s >= 0; s -= 8 {
		out = append(out, byte(arg>>uint(s)))
	}
	return out
}

func decInt(out []byte, v *big.Int) []byte {
	if v.Sign() >= 0 {
		return decHead(out, 0, v.Uint64())
	}
	// −1 − v, which is |v| − 1 and fits a uint64 by the range.
	return decHead(out, 1, new(big.Int).Sub(new(big.Int).Neg(v), big.NewInt(1)).Uint64())
}

// ToCbor is [m, e] as two shortest-form integers.
func (d Dec) ToCbor() []byte {
	out := []byte{0x82}
	out = decInt(out, d.M)
	return decInt(out, big.NewInt(int64(d.E)))
}

// DecFromCbor reads a dec from the whole of data, refusing anything not canonical (SMY-E602).
func DecFromCbor(data []byte) (Dec, error) {
	d := &Decoder{Data: data}
	major, n, _, err := d.Head()
	if err != nil {
		return Dec{}, decErr("E602", "%v", err)
	}
	if major != 4 || n != 2 {
		return Dec{}, decErr("E602", "not an array of two integers")
	}
	var parts [2]*big.Int
	for k := range parts {
		major, arg, _, err := d.Head()
		if err != nil {
			return Dec{}, decErr("E602", "%v", err)
		}
		v := new(big.Int).SetUint64(arg)
		switch major {
		case 0:
		case 1:
			v.Neg(v).Sub(v, big.NewInt(1))
		default:
			return Dec{}, decErr("E602", "not an array of two integers")
		}
		parts[k] = v
	}
	if d.I != len(data) {
		return Dec{}, decErr("E602", "bytes after the dec")
	}
	m, e := parts[0], parts[1]
	if new(big.Int).Abs(m).Cmp(decMMax) > 0 {
		return Dec{}, decErr("E602", "the mantissa is -2^64")
	}
	if !e.IsInt64() || e.Int64() < decEMin || e.Int64() > decEMax {
		return Dec{}, decErr("E602", "the exponent is outside [-400, 400]")
	}
	if m.Sign() == 0 && e.Sign() != 0 {
		return Dec{}, decErr("E602", "zero is [0, 0]")
	}
	if m.Sign() != 0 && new(big.Int).Rem(m, bigTen).Sign() == 0 {
		return Dec{}, decErr("E602", "the mantissa has a trailing zero")
	}
	return Dec{M: m, E: int(e.Int64())}, nil
}
