# RFC SMYSL-7 golden vector

A detached `COSE_Sign1` (RFC 9052) over one attestation, made with RFC 8032 §7.1's first test
key. Anything that verifies these two files verifies a smysl signature.

| File | What it is |
|---|---|
| `attestation.cbor` | the payload: `det_cbor(Record::Attestation)`, by `human:alice` |
| `signature.cose` | `18([protected, {-65540: uid}, null, signature])` |

- Secret key (RFC 8032 TEST 1, public): `9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60`
- Public key: `d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a`
- Signed bytes: `["Signature1", protected, h'736d79736c2e7369676e2f31', payload]`, the
  external AAD being `smysl.sign/1`.

Checked by `crates/smysl-sign/tests/golden.rs`, `go/sign_test.go` and
`nodejs/test/sign.test.js`. `SMYSL_REGEN_SIGN=1 cargo test -p smysl-sign --test golden`
rewrites the files; nothing else should change them. Specification:
`Documentation/SMYSL-7_RFC/SMYSL-7_SIGN.md` §4.
