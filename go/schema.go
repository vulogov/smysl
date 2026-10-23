package smysl

// FC-9: schema declaration keys 4 and 5, read by a fourth implementation (SMYSL-2.5 §5.8).
//
// TX-P12 step 1 gives record 8 a field table (key 4) and lens steps (key 5), SMYSL-2.3 A-7 and
// Appendix B. Lens application is not a conformance obligation (D-8); decoding the structure,
// writing key 5 back byte for byte (L9) and the lens-chain hash are. The checks mirror
// crates/smysl-core/src/schema/decode.rs: a key that does not match its structure is opaque and
// reported SMY-W430; an op whose tag, or one of whose keys, this reader does not know is kept as
// written and makes its step unusable, SMY-W469.

import (
	"errors"
	"fmt"
	"sort"
)

// OpKeys is the set of keys each known lens op may carry. A key outside it makes the op unknown.
var OpKeys = map[string][]uint64{
	"rename-type":     {0, 1, 2},
	"rename-relation": {0, 1, 2},
	"rename-key":      {0, 1, 2},
	"map-value":       {0, 1, 2},
	"split-value":     {0, 1, 2, 3, 4},
	"require":         {0, 1, 2},
	"default":         {0, 1, 2, 3},
	"retire":          {0, 1},
}

var errMismatch = errors.New("FC-9 structure mismatch")

func mismatch(format string, args ...any) error {
	return fmt.Errorf("%w: %s", errMismatch, fmt.Sprintf(format, args...))
}

// LensStep is one decoded step: its revisions, op tags, and the bytes it arrived as.
type LensStep struct {
	From, To uint64
	Ops      []string
	Usable   bool
	Bytes    []byte
}

// SchemaReading is one declaration's FC-9 reading. Fields is the field count, or -1 when key 4
// is opaque, or nil when the declaration has no key 4.
type SchemaReading struct {
	Fields       *int
	LensesOpaque bool
	Steps        []LensStep
	Codes        []string
}

func intMap(v any) (*Map, error) {
	m, ok := v.(*Map)
	if !ok {
		return nil, mismatch("not a map")
	}
	for _, p := range m.Entries {
		if _, ok := p.Key.(uint64); !ok {
			return nil, mismatch("not an integer-keyed map")
		}
	}
	return m, nil
}

func textMap(v any) (*Map, error) {
	m, ok := v.(*Map)
	if !ok {
		return nil, mismatch("not a map")
	}
	for _, p := range m.Entries {
		if _, ok := p.Key.(string); !ok {
			return nil, mismatch("not a text-keyed map")
		}
	}
	return m, nil
}

func isText(v any) error {
	if _, ok := v.(string); !ok {
		return mismatch("not text")
	}
	return nil
}

func isRev(v any) error {
	n, ok := v.(uint64)
	if !ok || n > 0xffffffff {
		return mismatch("not a revision")
	}
	return nil
}

func isTexts(v any) error {
	a, ok := v.([]any)
	if !ok {
		return mismatch("not an array")
	}
	for _, x := range a {
		if err := isText(x); err != nil {
			return err
		}
	}
	return nil
}

func isCond(v any) error {
	m, err := textMap(v)
	if err != nil {
		return err
	}
	if len(m.Entries) != 1 {
		return mismatch("a condition has one key")
	}
	return nil
}

func isTextMap(v any) error {
	_, err := textMap(v)
	return err
}

func shapeFields(raw []byte) (int, error) {
	v, used, err := DecodeOne(raw)
	if err != nil {
		return 0, mismatch("%v", err)
	}
	if used != len(raw) {
		return 0, mismatch("a shape is one map")
	}
	m, err := intMap(v)
	if err != nil {
		return 0, err
	}
	fv, ok := m.Get(uint64(0))
	if !ok {
		return 0, mismatch("a shape is {0: fields}")
	}
	fields, err := textMap(fv)
	if err != nil {
		return 0, err
	}
	checks := []struct {
		key   uint64
		check func(any) error
	}{{1, isTexts}, {2, isRev}, {3, isText}, {4, isRev}, {5, isCond}, {6, isTexts}}
	for _, p := range fields.Entries {
		f, err := intMap(p.Value)
		if err != nil {
			return 0, err
		}
		ty, ok := f.Get(uint64(0))
		if !ok {
			return 0, mismatch("a field has a type")
		}
		if err := isText(ty); err != nil {
			return 0, err
		}
		for _, c := range checks {
			if x, ok := f.Get(c.key); ok {
				if err := c.check(x); err != nil {
					return 0, err
				}
			}
		}
	}
	return len(fields.Entries), nil
}

// lensOp returns the op's tag and whether it was read in full (a known op).
func lensOp(v any) (string, bool, error) {
	o, err := intMap(v)
	if err != nil {
		return "", false, err
	}
	t, ok := o.Get(uint64(0))
	if !ok {
		return "", false, mismatch("an op has a tag")
	}
	tag, ok := t.(string)
	if !ok {
		return "", false, mismatch("a tag is text")
	}
	allowed, known := OpKeys[tag]
	if !known {
		return tag, false, nil
	}
	for _, p := range o.Entries {
		k := p.Key.(uint64)
		found := false
		for _, a := range allowed {
			found = found || a == k
		}
		if !found {
			return tag, false, nil
		}
	}
	anything := func(any) error { return nil }
	values := func(v any) error {
		m, err := textMap(v)
		if err != nil {
			return err
		}
		for _, p := range m.Entries {
			if err := isText(p.Value); err != nil {
				return err
			}
		}
		return nil
	}
	type need struct {
		key   uint64
		check func(any) error
	}
	needs := map[string][]need{
		"rename-type":     {{1, isText}, {2, isText}},
		"rename-relation": {{1, isText}, {2, isText}},
		"rename-key":      {{1, isText}, {2, isText}},
		"map-value":       {{1, isText}, {2, values}},
		"split-value":     {{1, isText}, {2, isText}, {3, isText}, {4, isTextMap}},
		"require":         {{1, isText}},
		"default":         {{1, isText}, {2, anything}},
		"retire":          {{1, isText}},
	}[tag]
	for _, n := range needs {
		x, ok := o.Get(n.key)
		if !ok {
			return "", false, mismatch("%s needs key %d", tag, n.key)
		}
		if err := n.check(x); err != nil {
			return "", false, err
		}
	}
	when := map[string]uint64{"require": 2, "default": 3}
	if k, ok := when[tag]; ok {
		if x, ok := o.Get(k); ok {
			if err := isCond(x); err != nil {
				return "", false, err
			}
		}
	}
	return tag, true, nil
}

func lensSteps(v any) ([]LensStep, error) {
	a, ok := v.([]any)
	if !ok {
		return nil, mismatch("lenses are an array")
	}
	out := make([]LensStep, 0, len(a))
	for _, sv := range a {
		s, err := intMap(sv)
		if err != nil {
			return nil, err
		}
		keys := make([]uint64, 0, len(s.Entries))
		for _, p := range s.Entries {
			keys = append(keys, p.Key.(uint64))
		}
		sort.Slice(keys, func(i, j int) bool { return keys[i] < keys[j] })
		if len(keys) != 3 || keys[0] != 0 || keys[1] != 1 || keys[2] != 2 {
			return nil, mismatch("a step is {0: from, 1: to, 2: ops}")
		}
		from, _ := s.Get(uint64(0))
		to, _ := s.Get(uint64(1))
		if isRev(from) != nil || isRev(to) != nil || to.(uint64) <= from.(uint64) {
			return nil, mismatch("a step goes up")
		}
		opsv, _ := s.Get(uint64(2))
		ops, ok := opsv.([]any)
		if !ok {
			return nil, mismatch("a step has an op array")
		}
		step := LensStep{From: from.(uint64), To: to.(uint64), Usable: true}
		for _, o := range ops {
			tag, known, err := lensOp(o)
			if err != nil {
				return nil, err
			}
			step.Ops = append(step.Ops, tag)
			step.Usable = step.Usable && known
		}
		if step.Bytes, err = EncodeOne(s); err != nil {
			return nil, mismatch("%v", err)
		}
		out = append(out, step)
	}
	return out, nil
}

// DecodeSchemaDecl reads one schema declaration's body: fields, steps and the codes decoding
// reports, key 4's first, then key 5's, then one SMY-W469 per unusable step.
func DecodeSchemaDecl(body *Map) SchemaReading {
	var r SchemaReading
	if v, ok := body.Get(uint64(4)); ok {
		n := -1
		if raw, ok := v.([]byte); ok {
			if c, err := shapeFields(raw); err == nil {
				n = c
			}
		}
		if n < 0 {
			r.Codes = append(r.Codes, "SMY-W430")
		}
		r.Fields = &n
	}
	if v, ok := body.Get(uint64(5)); ok {
		steps, err := lensSteps(v)
		if err != nil {
			r.LensesOpaque = true
			r.Codes = append(r.Codes, "SMY-W430")
		} else {
			r.Steps = steps
		}
	}
	for _, s := range r.Steps {
		if !s.Usable {
			r.Codes = append(r.Codes, "SMY-W469")
		}
	}
	return r
}

func cborHead(major byte, n uint64) []byte {
	e := &Encoder{}
	e.head(major, n)
	return e.Out
}

// EncodeLenses is key 5's bytes: an array of each step's bytes, as received (L9).
func EncodeLenses(steps []LensStep) []byte {
	out := cborHead(4, uint64(len(steps)))
	for _, s := range steps {
		out = append(out, s.Bytes...)
	}
	return out
}

// ChainHash is Appendix B: BLAKE3 over "smysl/lens-chain/1" ‖ 0x00 and [id, from, to, [step…]].
func ChainHash(id string, from, to uint64, steps []LensStep) []byte {
	in := append([]byte("smysl/lens-chain/1"), 0)
	in = append(in, cborHead(4, 4)...)
	for _, v := range []any{id, from, to} {
		b, _ := EncodeOne(v)
		in = append(in, b...)
	}
	in = append(in, EncodeLenses(steps)...)
	return Blake3(in)
}
