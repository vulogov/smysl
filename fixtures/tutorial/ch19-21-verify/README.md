Fixtures for chapter 21's "Signatures that act" (RFC SMYSL-8 TR-P4), replayed by
`scripts/verify-doc-output.py`.

- `ops.cbor` and `day.cbor`: `smysl import` of two readings tables (`--key host --key metric`).
- `ops-copy.cbor`: a byte copy of `ops.cbor`, without its signature file.
- `ops.cbor.sig`: `ops.cbor`'s attestations by `tool:smysl-import`, signed with a key made by
  `smysl keygen` for this fixture and then deleted. Only its public half is kept, in
  `.smysl/keys.hjson`, written by `smysl keys add`. No secret key is committed.

The stores are committed rather than regenerated because `import` stamps the wall clock.
