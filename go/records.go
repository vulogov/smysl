package smysl

// Record framing and the unit core, per §2 and §3.1 of the format spec.

// RecordNames is the table in §3.1: every code the format has allocated.
//
// A code is a permanent wire commitment the moment it is allocated, so the reserved and
// not-yet-defined ones are named too — 9 is checkpoint and 16 is reserved. Naming them is how a
// reader says *what* it met rather than only that it met something.
var RecordNames = map[uint64]string{
	1: "unit", 2: "attestation", 3: "relation", 4: "thread", 5: "view",
	6: "contention", 7: "pack_info", 8: "schema_decl", 9: "checkpoint", 10: "label_binding",
	11: "withdrawal", 12: "resolution", 13: "commitment", 14: "manifest", 15: "part_text",
	16: "reserved", 17: "dating", 18: "part_reading", 19: "redaction",
}

// understoodRecords holds the codes whose bodies this implementation decodes.
//
// Not the same question as RecordNames, and conflating the two was wrong before it was
// consequential: IsKnown was derived from the name table and documented as whether this version
// understands the record, which was already false for 9 — a checkpoint nothing has ever
// implemented — and harmless only because nothing emits one. 1.10 writes manifests, part texts,
// part readings and redactions, and a reader calling those known while interpreting none of them
// would be exactly the silence SMY-W014 exists to break.
//
// An unknown record is still preserved verbatim and re-encoded byte for byte; that is C-Read
// and it is unaffected.
// 14, 15, 18 and 19 joined in 1.10 and 17 with TX-P3: library.go decodes their bodies and
// derives an identity for each but the redaction, which has none — a redaction is a statement
// about a part rather than a thing the format refers to. 9 and 16 are still named and not
// understood: nothing emits a checkpoint and 16 is reserved. The separation above is what makes
// that sentence sayable.
var understoodRecords = map[uint64]bool{
	1: true, 2: true, 3: true, 4: true, 5: true, 6: true,
	7: true, 8: true, 10: true, 11: true, 12: true, 13: true,
	14: true, 15: true, 17: true, 18: true, 19: true,
}

// UnitKeys is the table in §2.2. Anything at 9 or above is an unknown key that rule X says
// must survive a round trip verbatim.
var UnitKeys = map[uint64]string{
	0: "schema", 1: "gist", 2: "body", 3: "detail", 4: "deps",
	5: "grounds", 6: "status", 7: "source", 8: "payload",
}

// Record is one record: a type code, its body, and the bytes it came from.
type Record struct {
	Code uint64
	Body any
	Raw  []byte
}

// Name reports the record's kind, or `unknown(N)` for a code from a later version.
func (r *Record) Name() string {
	if n, ok := RecordNames[r.Code]; ok {
		return n
	}
	return "unknown(" + itoa(r.Code) + ")"
}

// IsKnown reports whether this implementation decodes this record's body.
//
// A code the spec names but this reader does not interpret — a manifest, a checkpoint — is not
// known. Name still answers, so a report can say "a manifest, which this build does not
// interpret" rather than "something".
func (r *Record) IsKnown() bool {
	return understoodRecords[r.Code]
}

// Reencode emits the record again. For a conformant decoder this equals Raw.
func (r *Record) Reencode() ([]byte, error) {
	return EncodeOne([]any{r.Code, r.Body})
}

// UnitField returns a named field of a unit core, and whether it was present.
func (r *Record) UnitField(name string) (any, bool) {
	m, ok := r.Body.(*Map)
	if r.Code != 1 || !ok {
		return nil, false
	}
	for code, n := range UnitKeys {
		if n == name {
			return m.Get(code)
		}
	}
	return nil, false
}

func itoa(v uint64) string {
	if v == 0 {
		return "0"
	}
	var buf [20]byte
	i := len(buf)
	for v > 0 {
		i--
		buf[i] = byte('0' + v%10)
		v /= 10
	}
	return string(buf[i:])
}

// DecodeStore decodes a concatenation of records (§3.1: no framing envelope).
func DecodeStore(data []byte) ([]*Record, error) {
	var out []*Record
	off := 0
	for off < len(data) {
		d := &Decoder{Data: data[off:]}
		major, arg, _, err := d.Head()
		if err != nil {
			return nil, err
		}
		if major != 4 || arg != 2 {
			return nil, cborErr("a record is a two-element array")
		}
		code, err := d.Value(0)
		if err != nil {
			return nil, err
		}
		c, ok := code.(uint64)
		if !ok {
			return nil, cborErr("a record's type code is an unsigned integer")
		}
		body, err := d.Value(0)
		if err != nil {
			return nil, err
		}
		out = append(out, &Record{Code: c, Body: body, Raw: data[off : off+d.I]})
		off += d.I
	}
	return out, nil
}

// EncodeStore emits records back to back.
func EncodeStore(records []*Record) ([]byte, error) {
	var out []byte
	for _, r := range records {
		raw, err := r.Reencode()
		if err != nil {
			return nil, err
		}
		out = append(out, raw...)
	}
	return out, nil
}
