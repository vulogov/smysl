// Package smysl is a fourth independent implementation of the smysl wire format, written
// from Documentation/SMYSL_FORMAT_SPEC.md.
//
// The Python and JavaScript packages were written from the specification as it stood, and
// both had to guess in the same three places. Those guesses became clauses. This one is the
// first written against the *revised* document, which makes it a test of the revision: if
// constraints 1, 2 and 8 now say enough, a fresh reader should not have to invent anything
// there. Places where this reader still had to guess are marked SPEC: and are the interesting
// output.
//
// Conformance target: C-Read — decode, re-encode byte-identically, preserve what is not
// understood.
package smysl

import (
	"encoding/binary"
	"errors"
	"fmt"
	"math"
	"sort"
	"unicode/utf8"

	"golang.org/x/text/unicode/norm"
)

// MaxNesting is §3 constraint 9.
const MaxNesting = 128

// ErrCbor marks input the format forbids. Corresponds to SMY-E080 / SMY-E004.
var ErrCbor = errors.New("smysl: forbidden encoding")

func cborErr(format string, args ...any) error {
	return fmt.Errorf("%w: %s", ErrCbor, fmt.Sprintf(format, args...))
}

// Decoder reads deterministic CBOR strictly: every rule in §3 is a rejection rather than a
// normalisation, because a decoder that quietly accepted a non-shortest integer would let two
// byte strings mean one record, and a uid would stop naming exactly one thing.
type Decoder struct {
	Data []byte
	I    int
}

func (d *Decoder) byteAt() (byte, error) {
	if d.I >= len(d.Data) {
		return 0, cborErr("input ended inside a value")
	}
	b := d.Data[d.I]
	d.I++
	return b, nil
}

func (d *Decoder) take(n int) ([]byte, error) {
	if n < 0 || d.I+n > len(d.Data) {
		return nil, cborErr("length runs past the end of the input")
	}
	out := d.Data[d.I : d.I+n]
	d.I += n
	return out, nil
}

// Head returns the major type, its argument, and the raw additional-information value.
//
// `extra` is returned because major type 7 needs it: there the trailing bytes are a float's
// payload rather than an argument, which is why §3 constraint 2 is scoped to integers and
// lengths. The revised spec says so outright; the two earlier implementations each had to
// work it out from a rejected fixture.
func (d *Decoder) Head() (major byte, arg uint64, extra byte, err error) {
	b, err := d.byteAt()
	if err != nil {
		return 0, 0, 0, err
	}
	major, extra = b>>5, b&0x1f
	floaty := major == 7

	switch {
	case extra < 24:
		return major, uint64(extra), extra, nil
	case extra == 24:
		v, err := d.byteAt()
		if err != nil {
			return 0, 0, 0, err
		}
		if !floaty && v < 24 {
			return 0, 0, 0, cborErr("%d in two bytes; shortest form is one", v)
		}
		return major, uint64(v), extra, nil
	case extra == 25:
		raw, err := d.take(2)
		if err != nil {
			return 0, 0, 0, err
		}
		v := uint64(binary.BigEndian.Uint16(raw))
		if !floaty && v <= 0xff {
			return 0, 0, 0, cborErr("%d in three bytes; a shorter form exists", v)
		}
		return major, v, extra, nil
	case extra == 26:
		raw, err := d.take(4)
		if err != nil {
			return 0, 0, 0, err
		}
		v := uint64(binary.BigEndian.Uint32(raw))
		if !floaty && v <= 0xffff {
			return 0, 0, 0, cborErr("%d in five bytes; a shorter form exists", v)
		}
		return major, v, extra, nil
	case extra == 27:
		raw, err := d.take(8)
		if err != nil {
			return 0, 0, 0, err
		}
		v := binary.BigEndian.Uint64(raw)
		if !floaty && v <= 0xffffffff {
			return 0, 0, 0, cborErr("%d in nine bytes; a shorter form exists", v)
		}
		return major, v, extra, nil
	case extra == 31:
		// §3 constraint 3. Indefinite length is how one value gets two encodings.
		return 0, 0, 0, cborErr("indefinite-length item; definite lengths only")
	}
	return 0, 0, 0, cborErr("reserved additional-information value %d", extra)
}

// Value decodes one item.
func (d *Decoder) Value(depth int) (any, error) {
	if depth > MaxNesting {
		return nil, cborErr("nesting deeper than %d", MaxNesting)
	}
	major, arg, extra, err := d.Head()
	if err != nil {
		return nil, err
	}

	switch major {
	case 0:
		return arg, nil
	case 1:
		return -1 - int64(arg), nil
	case 2:
		raw, err := d.take(int(arg))
		if err != nil {
			return nil, err
		}
		return append([]byte(nil), raw...), nil
	case 3:
		raw, err := d.take(int(arg))
		if err != nil {
			return nil, err
		}
		if !utf8.Valid(raw) {
			return nil, cborErr("text is not valid UTF-8")
		}
		// §3 constraint 6. Checked rather than applied: normalising here would accept two
		// encodings of one string, which is the thing being forbidden.
		if !norm.NFC.IsNormal(raw) {
			return nil, cborErr("text is not NFC-normalised")
		}
		return string(raw), nil
	case 4:
		out := make([]any, 0, arg)
		for i := uint64(0); i < arg; i++ {
			v, err := d.Value(depth + 1)
			if err != nil {
				return nil, err
			}
			out = append(out, v)
		}
		return out, nil
	case 5:
		return d.decodeMap(arg, depth)
	case 7:
		return d.simple(arg, extra)
	}
	// §3 constraint 8, added after two earlier implementations both had to guess here.
	return nil, cborErr("major type %d is not part of this format", major)
}

// Pair keeps a map's entries in wire order.
//
// A Go map would lose that order, and §3 constraint 4 makes order part of the encoding rather
// than a presentation detail — re-encoding has to reproduce it exactly. Keys are `any` because
// constraint 1 permits integers in the kernel and text inside a payload, and collapsing the
// two would erase the distinction that constraint draws.
type Pair struct {
	Key   any
	Value any
}

// Map is a CBOR map with its ordering preserved.
type Map struct {
	Entries []Pair
}

// Get returns the value for key, and whether it was present.
func (m *Map) Get(key any) (any, bool) {
	for _, p := range m.Entries {
		if p.Key == key {
			return p.Value, true
		}
	}
	return nil, false
}

func (d *Decoder) decodeMap(n uint64, depth int) (*Map, error) {
	out := &Map{Entries: make([]Pair, 0, n)}
	var prev []byte
	for i := uint64(0); i < n; i++ {
		start := d.I
		key, err := d.Value(depth + 1)
		if err != nil {
			return nil, err
		}
		keyBytes := d.Data[start:d.I]
		// §3 constraint 4: ascending by *encoded* key bytes, which orders an integer key
		// against a text key without needing to know which a map uses.
		if prev != nil && compareBytes(keyBytes, prev) <= 0 {
			return nil, cborErr("map keys are not in ascending order, or are duplicated")
		}
		prev = keyBytes
		value, err := d.Value(depth + 1)
		if err != nil {
			return nil, err
		}
		out.Entries = append(out.Entries, Pair{Key: key, Value: value})
	}
	return out, nil
}

func (d *Decoder) simple(arg uint64, extra byte) (any, error) {
	switch extra {
	case 20:
		return false, nil
	case 21:
		return true, nil
	case 22:
		// §3 constraint 5. An absent optional is omitted, so null on the wire is a violation.
		return nil, cborErr("null is forbidden; omit the key instead")
	case 26:
		v := math.Float32frombits(uint32(arg))
		if math.IsInf(float64(v), 0) || math.IsNaN(float64(v)) {
			return nil, cborErr("float is not finite")
		}
		// §3 constraint 7: a multiple of 1/1024.
		if scaled := float64(v) * 1024; scaled != math.Trunc(scaled) {
			return nil, cborErr("float %v is not a multiple of 1/1024", v)
		}
		return v, nil
	case 27:
		return nil, cborErr("binary64 float; the format uses binary32")
	}
	return nil, cborErr("simple value %d is not part of this format", extra)
}

func compareBytes(a, b []byte) int {
	n := len(a)
	if len(b) < n {
		n = len(b)
	}
	for i := 0; i < n; i++ {
		if a[i] != b[i] {
			if a[i] < b[i] {
				return -1
			}
			return 1
		}
	}
	return len(a) - len(b)
}

// Encoder emits exactly what §3 requires, so a decoded record re-encodes to its own bytes.
type Encoder struct {
	Out []byte
}

func (e *Encoder) head(major byte, arg uint64) {
	switch {
	case arg < 24:
		e.Out = append(e.Out, major<<5|byte(arg))
	case arg <= 0xff:
		e.Out = append(e.Out, major<<5|24, byte(arg))
	case arg <= 0xffff:
		e.Out = append(e.Out, major<<5|25)
		e.Out = binary.BigEndian.AppendUint16(e.Out, uint16(arg))
	case arg <= 0xffffffff:
		e.Out = append(e.Out, major<<5|26)
		e.Out = binary.BigEndian.AppendUint32(e.Out, uint32(arg))
	default:
		e.Out = append(e.Out, major<<5|27)
		e.Out = binary.BigEndian.AppendUint64(e.Out, arg)
	}
}

// Value encodes one item.
func (e *Encoder) Value(v any) error {
	switch t := v.(type) {
	case bool:
		if t {
			e.Out = append(e.Out, 0xf5)
		} else {
			e.Out = append(e.Out, 0xf4)
		}
	case uint64:
		e.head(0, t)
	case int:
		if t >= 0 {
			e.head(0, uint64(t))
		} else {
			e.head(1, uint64(-1-t))
		}
	case int64:
		if t >= 0 {
			e.head(0, uint64(t))
		} else {
			e.head(1, uint64(-1-t))
		}
	case float32:
		e.Out = append(e.Out, 0xfa)
		e.Out = binary.BigEndian.AppendUint32(e.Out, math.Float32bits(t))
	case []byte:
		e.head(2, uint64(len(t)))
		e.Out = append(e.Out, t...)
	case string:
		e.head(3, uint64(len(t)))
		e.Out = append(e.Out, t...)
	case []any:
		e.head(4, uint64(len(t)))
		for _, item := range t {
			if err := e.Value(item); err != nil {
				return err
			}
		}
	case *Map:
		e.head(5, uint64(len(t.Entries)))
		// Sorted by encoded key bytes, per constraint 4 — not by the key's Go value, which
		// would order an integer key against a text key differently.
		entries := append([]Pair(nil), t.Entries...)
		sort.SliceStable(entries, func(i, j int) bool {
			a, _ := EncodeOne(entries[i].Key)
			b, _ := EncodeOne(entries[j].Key)
			return compareBytes(a, b) < 0
		})
		for _, p := range entries {
			if err := e.Value(p.Key); err != nil {
				return err
			}
			if err := e.Value(p.Value); err != nil {
				return err
			}
		}
	default:
		return cborErr("cannot encode %T", v)
	}
	return nil
}

// EncodeOne encodes a single value.
func EncodeOne(v any) ([]byte, error) {
	e := &Encoder{}
	if err := e.Value(v); err != nil {
		return nil, err
	}
	return e.Out, nil
}

// DecodeOne decodes a single value, returning it and how many bytes it used.
func DecodeOne(data []byte) (any, int, error) {
	d := &Decoder{Data: data}
	v, err := d.Value(0)
	if err != nil {
		return nil, 0, err
	}
	return v, d.I, nil
}
