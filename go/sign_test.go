// RFC SMYSL-7's golden vector, verified without the Rust: a detached COSE_Sign1 (RFC 9052)
// over an attestation's canonical CBOR, with Ed25519 from the standard library. The envelope is
// taken apart by position and the Sig_structure built by hand, so nothing here trusts the
// encoder that wrote it.
package smysl

import (
	"bytes"
	"crypto/ed25519"
	"encoding/hex"
	"os"
	"testing"
)

const signDir = "../fixtures/sign"

// RFC 8032 §7.1 TEST 1's public key.
const signPublic = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"

func bstrHead(n int) []byte {
	switch {
	case n < 24:
		return []byte{0x40 | byte(n)}
	case n < 256:
		return []byte{0x58, byte(n)}
	default:
		return []byte{0x59, byte(n >> 8), byte(n)}
	}
}

func TestSignGoldenVectorVerifies(t *testing.T) {
	payload, err := os.ReadFile(signDir + "/attestation.cbor")
	if err != nil {
		t.Fatal(err)
	}
	cose, err := os.ReadFile(signDir + "/signature.cose")
	if err != nil {
		t.Fatal(err)
	}
	pub, _ := hex.DecodeString(signPublic)

	// 18([protected, {-65540: uid}, null, signature])
	if !bytes.Equal(cose[:3], []byte{0xd2, 0x84, 0x58}) {
		t.Fatalf("not tag 18 over an array of four: % x", cose[:3])
	}
	plen := int(cose[3])
	protected := cose[4 : 4+plen]
	rest := cose[4+plen:]
	if rest[40] != 0xf6 || !bytes.Equal(rest[41:43], []byte{0x58, 0x40}) || len(rest) != 43+64 {
		t.Fatal("expected a null payload and a 64-byte signature")
	}
	sig := rest[43:]

	// The protected header: alg -8, kid = BLAKE3(pub)[:16], agent, kind, target hash.
	want := []byte{0xa5, 0x01, 0x27, 0x04, 0x50}
	want = append(want, Blake3(pub)[:16]...)
	want = append(want, 0x3a, 0x00, 0x01, 0x00, 0x00, 0x6b)
	want = append(want, "human:alice"...)
	want = append(want, 0x3a, 0x00, 0x01, 0x00, 0x01, 0x6b)
	want = append(want, "attestation"...)
	want = append(want, 0x3a, 0x00, 0x01, 0x00, 0x02, 0x58, 0x20)
	want = append(want, Blake3(payload)...)
	if !bytes.Equal(protected, want) {
		t.Fatalf("protected header\n got % x\nwant % x", protected, want)
	}

	// Sig_structure = ["Signature1", protected, "smysl.sign/1", payload]
	msg := []byte{0x84, 0x6a}
	msg = append(msg, "Signature1"...)
	msg = append(append(msg, bstrHead(len(protected))...), protected...)
	msg = append(append(msg, bstrHead(12)...), "smysl.sign/1"...)
	msg = append(append(msg, bstrHead(len(payload))...), payload...)
	if !ed25519.Verify(ed25519.PublicKey(pub), msg, sig) {
		t.Fatal("the golden signature does not verify")
	}
	// And one flipped bit of the record does not.
	bad := append([]byte(nil), msg...)
	bad[len(bad)-1] ^= 1
	if ed25519.Verify(ed25519.PublicKey(pub), bad, sig) {
		t.Fatal("a changed record verified")
	}
}
