# RFC SMYSL-7 — detached attestation signatures (`smysl-sign`)

**Author:** Vladimir Ulogov
**Status:** amended in review against the code (R1, 2026-11-23). SG-P1 to SG-P3 are built; SG-P4 is next (1.20).
**Target compatibility:** smysl 1.x, strictly additive; format `smysl/1.0`, unchanged.
**Crate:** `crates/smysl-sign`, new, behind the facade's `sign` feature, on by default.
**Cryptographic profile:** Ed25519 (RFC 8032) in detached `COSE_Sign1` envelopes (RFC 9052).
**License:** MPL-2.0.
**Evidence baseline:** `dev/1.20.0` at `f2ec430`. Every code pointer below was read there.

---

## 1. Summary and motivation

An `AgentId` (`human:*`, `tool:*`, `model:*`) is a string a writer chose. The code says so where
the type is declared: "Self-asserted until COSE signing lands (N9). Corroboration is gameable by
fabricated attestations until then" (`crates/smysl-core/src/ids.rs`). In a store one person
writes, that costs nothing. Where several agents or organisations write, it leaves three gaps:

1. **Corroboration can be inflated.** Anyone can write attestations under several agent ids, and
   `smysl-graph`'s salience counts them as independent (`crates/smysl-graph/src/salience.rs`,
   which is why low-trust deployments set the corroboration weight to 0).
2. **Origin authority is advisory.** `plan_retraction` under `RetractionAuthority::Origin`
   compares the requesting agent's string with the strings on the unit's attestations
   (`crates/smysl-graph/src/merge/retraction.rs`). Anyone can type the originator's id.
3. **Nothing downstream can check a trust boundary**: that a `measured` unit came from an
   authorised importer's key, or that a staged commit was confirmed by the person it names.
   SMYSL-2.7 OQ-25 keeps staging the default for the library's own exports for this reason:
   without signatures, "our own export" cannot be told from a file that claims to be one.

This RFC specifies **`smysl-sign`**, a pure Rust crate that signs a record's canonical CBOR with
Ed25519 and keeps the signature beside the store, not in it, as a detached `COSE_Sign1`
envelope. No uid changes, no record changes, and every `smysl/1.0` reader, in Rust, Python,
JavaScript and Go, reads a signed store exactly as it reads an unsigned one.

## 2. Invariants

| | Requirement | How this RFC meets it |
|---|---|---|
| **G1. Semver and format** | `smysl/1.0`; `cargo-semver-checks` on every published crate | A new crate. No record, key or wire change: signatures live in a separate file (§4.1). Attestation key 7, `sig`, stays reserved (R1-1). |
| **G2. Purity** | Rule B (`xtask check-purity`) | `smysl-sign` is a pure crate in every configuration: no file, clock, environment or randomness. A key is made from 32 bytes the caller drew, and a key file is parsed from its text (R1-6). It joins the pure list, with the PRNG, clock and environment bans `smysl-tel` has. No C compiler: `ed25519-dalek` is pure Rust. |
| **G3. Determinism** | Rule D: a pure operation is a function of its inputs | Ed25519 signing (RFC 8032) derives its nonce from the key and the message, so one key and one record give the same signature bytes every time, with no random draw. The envelope is deterministic CBOR (RFC 8949 §4.2.1). Reports are `BTreeMap`s in record-hash order. |
| **G4. Epistemic safety** | A signature never raises a status | A signature says who attested a record, and nothing about whether the record is true. Rule M, rule T and every status are untouched. A forged or invalid signature is an error in every mode (§6). |
| **G5. Library first** | Rule A: the CLI reaches the library only through the facade | Signing, the envelope, key resolution and verification live in `smysl-sign`, re-exported as `smysl::sign` behind `sign` (R1-4). The commands are thin dispatch (§8). |

## 3. The crate

```
smysl-core   smysl-graph
      \         /
      smysl-sign        pure: Ed25519, COSE_Sign1, KeyResolver, key file text, verification
          |
   smysl (facade, `sign`)   smysl::sign
          |
   smysl (CLI)              keygen, sign, verify; --sign on merge --staged and retract
```

Dependencies: `smysl-core`, `smysl-graph`, `ed25519-dalek` 2.2 (default features off; `std`,
`zeroize`, `fast`) and `zeroize`. No `coset` and no `ciborium`: the envelope is four CBOR items,
written with `smysl-core`'s own deterministic encoder (R1-5).

- **No C toolchain.** `CC=false CXX=false cargo build -p smysl-sign` succeeds, and the crate
  builds for `wasm32-unknown-unknown`.
- **Secrets.** A `SigningKey`'s secret half is wiped on drop (`ZeroizeOnDrop`), is never shown by
  `Debug` (which prints the agent and the key id only), and is never written to a store.

## 4. The signature format

### 4.1 Why detached

A unit's uid is `BLAKE3-256(det_cbor(UnitCore))`, and a store is an append-only log
(`smysl_graph::Store`). Attestations are not hashed (format spec §2.4), and the format has
reserved a field for a signature in one since 0.1 (`Attestation.sig`, key 7, "COSE_Sign1,
reserved. Not implemented in 0.1 (N9)"). It is not used, because a record already in a log
cannot be changed: signing it would mean appending a second copy with the field filled, and the
store would then hold an unsigned and a signed copy of one attestation (R1-1).

So signatures are kept in a second append-only file, a CBOR sequence of `COSE_Sign1` envelopes:
`<store>.sig` beside a store, and `<library>/log/sigs.cbor` in a library (§8). A signature file is
a set, as a store is: two agents who sign the same record independently write two envelopes, and
merging the files in either order gives the same set.

### 4.2 The envelope (`COSE_Sign1`, RFC 9052 §4.2)

```
18([
  protected:   bstr,       ; det_cbor of the protected header map
  unprotected: map,        ; an index hint, not covered by the signature
  payload:     null,       ; detached (RFC 9052 §4.1)
  signature:   bstr .size 64
])
```

Protected header (signed), in deterministic key order:

| Key | Name | Type | Value |
|---|---|---|---|
| `1` | `alg` | int | `-8`, EdDSA |
| `4` | `kid` | bstr, 16 bytes | the first 16 bytes of `BLAKE3-256(public key)` |
| `-65537` | `smysl.agent` | tstr | the signing agent, as an `AgentId` |
| `-65538` | `smysl.target_kind` | tstr | `"attestation"` or `"record"` (R1-2) |
| `-65539` | `smysl.target_hash` | bstr, 32 bytes | `BLAKE3-256(det_cbor(target))`, `Store::record_hash` |

Unprotected header (a hint, never trusted until the signature verifies):

| Key | Name | Type | Value |
|---|---|---|---|
| `-65540` | `smysl.subject_uid` | bstr, 32 bytes | the uid the attestation names (a unit, or a relation's rid); absent for a `record` |

Labels below −65536 are private use (RFC 9052 §3.1).

### 4.3 What is signed (`Sig_structure`, RFC 9052 §4.4)

```
Sig_structure = [
  "Signature1",
  protected,                  ; the exact bytes of the protected header
  h'736d79736c2e7369676e2f31', ; external_aad = "smysl.sign/1", a domain separator
  det_cbor(target)            ; the detached payload: smysl_core::to_cbor(record)
]
```

The payload is the record's canonical encoding, so a verifier in any language with a CBOR writer
and Ed25519 checks a signature as RFC 9052 says, with no smysl-specific step. The target hash in
the protected header binds the record a second time, so a signature can be matched to its record
without encoding every record in the store.

### 4.4 What gets signed

- **An attestation** (`target_kind = "attestation"`): the `Record::Attestation`, which binds the
  uid it names, the agent, op, rung, hop, parents, clock and recipe. The protected `smysl.agent`
  must equal the attestation's `agent`; `sign_record` refuses a key bound to another agent, and
  a verifier reports `AgentMismatch`.
- **A retraction** is an attestation too: `smysl retract --as A --sign` records an attestation
  by `A` on the `retracts` edge (its uid is the edge's rid, which format spec §2.4 already
  allows), and signs that (R1-2).
- **Any other record** (`target_kind = "record"`), a unit say: signed as it is. There is no agent
  in the record to compare with, so the signature says only that the key's agent vouched for
  those bytes. Unsigned records of this kind are never an error.

## 5. Keys

```rust
pub struct KeyId(pub [u8; 16]);              // BLAKE3-256(public key)[..16]

#[non_exhaustive]
pub enum KeyStatus {
    Active,
    /// Signatures on attestations whose clock reads `after_ms` or later are refused; all of them
    /// when `None`.
    Revoked { after_ms: Option<u64>, reason: Option<String> },
    /// Valid for attestations whose clock reads within `[not_before_ms, not_after_ms]`.
    Bounded { not_before_ms: u64, not_after_ms: u64 },
}

#[non_exhaustive]
pub struct ResolvedKey {
    pub kid: KeyId,
    pub agent: AgentId,
    pub public_key: [u8; 32],
    pub status: KeyStatus,
    /// The highest status this key may vouch for (R1-9).
    pub max_status: Option<Status>,
}

pub trait KeyResolver {
    fn resolve(&self, agent: &AgentId, kid: &KeyId) -> Result<ResolvedKey, KeyError>;
}
```

A verifier recomputes the key id from the resolved public key, and a resolver that returns a key
whose id differs is an error, not a match (R1-10).

- **`MemoryKeyResolver`**: a `BTreeMap<(AgentId, KeyId), ResolvedKey>`.
- **The key file** (`.smysl/keys.hjson`), parsed from its text by `KeyFile::parse` with
  `smysl-core`'s HJSON parser into a `MemoryKeyResolver` (R1-6). The CLI reads the file.

```hjson
{
  keys: [
    { agent: "human:alice", ed25519: "hex:d75a9801…", status: "active" }
    { agent: "tool:ci-telemetry", ed25519: "base64:MCowBQYDK2VwAyEA…", max_status: "measured" }
    { agent: "tool:old-bot", ed25519: "hex:3d4017c3…", status: "revoked",
      revoked_after_ms: 1762000000000, reason: "rotated in Q4" }
  ]
}
```

A public key is `hex:` and 64 hex digits, or `base64:` and either the raw 32 bytes or the 44-byte
SubjectPublicKeyInfo DER that `openssl pkey -pubout -outform DER` writes.

## 6. Verification modes

| | `Advisory` (default) | `Selective` | `Strict` |
|---|---|---|---|
| Valid signature | verified | verified | verified |
| Unsigned attestation | warning | error if in scope, else warning | error |
| Unknown key | warning | error if in scope, else warning | error |
| Invalid signature | error | error | error |
| Revoked or out-of-bounds key | error | error | error |
| Agent mismatch | error | error | error |
| Status over the key's cap | error | error | error |

`Selective`'s scope is an attestation by an agent whose id starts with one of the given prefixes,
or one on a unit whose status is at or above the given one (R1-13).

**A forged signature is never a warning.** Advisory tolerates a signature that is absent, or made
with a key the resolver does not know; it does not tolerate one that is present and wrong.

When several signatures name one record, the record's verdict is the worst error among them, else
`Verified`, else `UnknownKey` (R1-11). A signature whose target is not in the store is counted,
not judged.

A signature names its record twice: by hash, and by agent and subject uid. An unsigned
attestation and a signature left over that names its agent and its unit are judged against each
other, so an attestation changed after it was signed fails `Advisory` too (R1-14). The subject
uid sits in the unprotected header, so the verifier checks it against the attestation's uid; a
`record` signature carries none.

Corroboration and Origin authority (§1) read verification through `verified_attesters`, which
gives each unit the agents whose attestations carry a verified signature. `smysl-graph` sits
below `smysl-sign` and does not call it; the caller filters (R1-8).

## 7. API (`smysl_sign`)

```rust
pub struct SigningKey { /* ed25519_dalek::SigningKey + AgentId */ }
impl SigningKey {
    pub fn from_seed(agent: AgentId, seed: [u8; 32]) -> SigningKey;
    pub fn agent(&self) -> &AgentId;
    pub fn public_key(&self) -> [u8; 32];
    pub fn kid(&self) -> KeyId;
}

#[non_exhaustive]
pub enum TargetKind { Attestation, Record }

#[non_exhaustive]
pub struct DetachedSignature {
    pub kid: KeyId,
    pub agent: AgentId,
    pub target_kind: TargetKind,
    pub target_hash: [u8; 32],
    pub subject_uid: Option<Uid>,
    pub signature: [u8; 64],
}
impl DetachedSignature {
    pub fn to_cose(&self) -> Vec<u8>;                         // the COSE_Sign1 bytes
    pub fn from_cose(bytes: &[u8]) -> Result<DetachedSignature, SignError>;
}
pub fn read_signatures(seq: &[u8]) -> Result<Vec<DetachedSignature>, SignError>;

pub fn sign_record(key: &SigningKey, record: &Record) -> Result<DetachedSignature, SignError>;
pub fn verify_record<R: KeyResolver>(sig: &DetachedSignature, record: &Record, resolver: &R)
    -> RecordVerdict;
pub fn verify_store<R: KeyResolver>(store: &Store, signatures: &[DetachedSignature],
    resolver: &R, mode: &VerificationMode) -> VerificationReport;
pub fn verified_attesters<R: KeyResolver>(store: &Store, signatures: &[DetachedSignature],
    resolver: &R) -> BTreeMap<Uid, BTreeSet<(AgentId, KeyId)>>;
```

```rust
#[non_exhaustive]
pub enum VerificationMode {
    Advisory,
    Selective { agent_prefixes: Vec<String>, require_status_at_least: Option<Status> },
    Strict,
}

pub struct KeyFile;
impl KeyFile {
    pub fn parse(text: &str) -> Result<MemoryKeyResolver, KeyFileError>;
    pub fn entry(key: &ResolvedKey) -> String;              // one line, as `parse` reads it
}
pub fn parse_public_key(s: &str) -> Result<[u8; 32], String>;
```

`RecordVerdict` is `Verified`, `Unsigned`, `UnknownKey`, `StatusOverCap`, `RevokedKey`,
`AgentMismatch` or `InvalidSignature`, ranked in that order. `VerificationReport` holds the mode,
`passed`, a verdict per record hash, the hashes that fail under the mode, and the verified,
unsigned, warning, error and unmatched counts.

`verify_record` reads a unit's own status against the key's cap. An attestation's cap is read
against the unit it names, which only a store holds, so `verify_store` and `verified_attesters`
apply it and `verify_record` alone does not (R1-9).

## 8. Command line (`sign` feature)

```sh
smysl keygen --agent human:alice --out ~/.config/smysl/alice.key   # 0600; prints the keys.hjson entry
smysl sign -s notes.store --key ~/.config/smysl/alice.key          # appends to notes.store.sig
smysl verify -s notes.store --keys .smysl/keys.hjson --mode strict
```

- `smysl merge --staged --sign --key K` signs the attestations the commit writes.
- `smysl retract UID --as A --sign --key K` records and signs `A`'s attestation on the edge.
- `smysl check --verify-sigs MODE --keys F` adds the verification findings to `check`'s.
- `smysl verify` exits 0 when the report passes, 1 when it does not, 2 on a usage error, a
  missing key file or a malformed one.

`keygen` draws the seed with `getrandom`, which the CLI already links; the crate never does.

The signature file is a CBOR sequence of envelopes, written whole in one order (by target hash,
then key id, then bytes) with duplicates dropped, so merging the same stores in either order
writes the same file (R1-16). `merge` carries its inputs' files to `<output>.sig`.

## 9. Compatibility and risks

- **Readers.** A signature file is a file the readers do not open. Uids, records and views are
  what they were.
- **Other languages.** The envelope is RFC 9052 and the payload is the record's canonical CBOR,
  so a verifier needs CBOR and Ed25519 and nothing else. The golden vector in `fixtures/sign/` is
  verified by the Go and Node conformance tests with their standard libraries' Ed25519 (R1-5).
- **Rotation.** A new key is a new entry with a new key id. Old signatures stay valid while the
  old key is `Bounded`, or `Revoked` after a time; a record can be signed again with the new key,
  and no uid changes.
- **Clocks.** Revocation compares `after_ms` with the attestation's clock, `ts.wall_ms`, which the
  signer wrote. A compromised key can backdate, so a key revoked for compromise is revoked with no
  time, and every signature it made is refused.

## 10. Phases

- **SG-P1 — the crate.** `KeyId`, `SigningKey`, `KeyResolver`, `MemoryKeyResolver`, the envelope,
  `sign_record`, `verify_record`, `verify_store` in `Advisory` and `Strict`, `read_signatures`.
  The crate on the pure list. **Gate:** the crate builds with `CC=false CXX=false` and for
  `wasm32-unknown-unknown`; signing one record twice gives identical bytes; a golden vector in
  `fixtures/sign/` (RFC 8032's first test key, an attestation, its envelope) is checked in Rust
  against a `Sig_structure` built by hand, and verified by the Go and Node conformance tests.
- **SG-P2 — modes and keys.** `Selective`, revocation and bounds, status caps, `KeyFile::parse`,
  `verified_attesters`. **Gate:** flipping any byte of the record, the protected header or the
  signature fails in every mode; an attestation signed under another agent's id is
  `AgentMismatch`; an unsigned `measured` unit passes `Advisory` with a warning and fails
  `Selective` and `Strict`.
- **SG-P3 — the commands.** `keygen`, `sign`, `verify`, the `--sign` flags, `check --verify-sigs`,
  the signature files. **Gate:** two agents sign overlapping attestations in two stores; merged
  in either order (stores and signature files), both verify under `Strict`.
- **SG-P4 — the gates.** `check-purity`, `cargo semver-checks` on every published crate, the
  manual.

## 11. Amendments

**R1, 2026-11-23: the draft read against the code.**

- **R1-1.** The format has reserved `Attestation.sig` (key 7) for a `COSE_Sign1` since 0.1. It is
  not used: a store is an append-only log, so filling it means a second copy of the attestation.
  It stays reserved, and the signatures are detached as the draft proposed.
- **R1-2.** The `retracts` edge `retract` writes carries no attestation
  (`Relation::new(RelKind::Retracts, target, target)` in `merge/retraction.rs`), so a signature
  over it has no agent to compare. `retract --as A --sign` records `A`'s attestation on the edge
  and signs that. The target kinds are `attestation` and `record`; the draft's `retraction` is an
  attestation whose uid is a `retracts` edge's rid.
- **R1-3.** Names: the status type is `smysl_core::Status`, not `EpistemicStatus`; an
  attestation's clock is `ts: Hlc`, read as `ts.wall_ms`; the store is `smysl_graph::Store`, and
  its records come from `Store::iter`.
- **R1-4.** The crate takes the workspace version, 1.20.0, like every other crate. The draft's
  "never re-exported from the facade" contradicts Rule A, which `check-purity` enforces; it is
  re-exported as `smysl::sign` behind `sign`, as SMYSL-6 R1-8 did for `smysl::lsp`. `sign` is on
  by default, so a stock binary can verify a store someone else signed (decided 2026-11-23).
- **R1-5.** No `coset` (and with it `ciborium`). The envelope is a tag, a four-element array and
  a five-entry map, written with `smysl-core`'s deterministic encoder, whose determinism the
  workspace already tests (decided 2026-11-23). That it is standard RFC 9052 is shown by
  verifying the golden vector outside Rust: Go's `crypto/ed25519` and Node's `crypto` both have
  Ed25519 in their standard libraries. Python's does not, and no package is added for it.
- **R1-6.** No `keystore-fs` feature and no `smysl-text` dependency. Keeping the crate pure in
  every configuration is simpler than a pure default and an impure feature: `KeyFile::parse`
  takes the file's text, `SigningKey::from_seed` takes 32 bytes, and the CLI reads files and
  draws randomness.
- **R1-7.** "Rule V1", "Principle P8", "Guarantee A5" and "Guarantees A2–A3" name nothing in this
  repository. §2 states what each meant against the rules that exist (B, D, M, T, A).
- **R1-8.** Salience and retraction planning live in `smysl-graph`, below `smysl-sign`, so they
  cannot ask it. `verified_attesters` gives the caller the verified agents to filter by.
- **R1-9.** An attestation has no status, so a key's cap is checked against the status of the
  unit the attestation names. An attestation on a relation, which has no status, is never over
  a cap.
- **R1-10.** A resolver is found by `(agent, kid)`, and the verifier recomputes the kid from the
  key it is given. A resolver that answers with another key is reported, not trusted.
- **R1-11.** The draft gives one verdict per record and does not say which, when a record has
  several signatures: the worst error, else verified, else unknown key. A signature with no
  record in the store is counted as unmatched.
- **R1-12.** `KeyStatus::Revoked`'s field is `after_ms` and the cap is `max_status`: shorter
  names, the same meaning. `DetachedSignature` holds the 64 signature bytes and builds its
  envelope on demand, so the struct cannot disagree with its own bytes.
- **R1-13.** `Selective`'s "at or above these statuses" took a set where one status says the
  same: `Status` is ordered, the order rule M compares, so "at or above any of them" is "at or
  above the lowest". The field is `require_status_at_least: Option<Status>`.

**R1, continued, 2026-11-23: SG-P2 built.**

- **R1-14.** Three things the gate's "flipping any byte fails in every mode" needed, and one it
  cannot have.
  - A changed attestation no longer hashes to its signature's target, so by hash alone it reads
    as unsigned, a warning in `Advisory`. A signature left over that names the attestation's
    agent and subject uid is judged against it, and fails. A signature file that holds another
    store's signature on the same agent and unit, without its record, reads as a forgery; `merge`
    moves records and signatures together (SG-P3), so a store's own file does not.
  - The subject uid is unprotected, so `verify_record` checks it: an attestation's must be its
    uid, and a `record` signature has none.
  - A unit signed directly is read against the key's cap by its own status, as an attestation is
    by its unit's.
  - A flipped byte of the kid names a key no resolver knows: `UnknownKey`, which `Advisory`
    warns of and `Selective` (in scope) and `Strict` refuse. A kid nobody has published cannot
    be told from one not distributed yet, and §6's table keeps the warning. Every other byte of
    the envelope and the record fails every mode, as `tests/modes.rs` checks byte by byte.
- **R1-15.** The key file parser is strict: an unknown key, a field the entry's status does not
  take, a key listed twice, or one key bound to two agents is an error. A typo in
  `revoked_after_ms` that dropped a revocation would fail open. An entry may carry `kid`, checked
  against the key; `KeyFile::entry` writes it. Points off the curve or of small order are refused.

**R1, continued, 2026-11-24: SG-P3 built.**

- **R1-16.** What §8 left open, settled in the CLI.
  - A signing key file is HJSON like the key file: `{ agent, ed25519_secret: "hex:…", kid }`
    under a comment that says whose key it is. `KeyFile::secret` writes it and
    `KeyFile::parse_secret` reads it, both pure. `keygen` creates it owner-only (0600 on Unix)
    and refuses a file that exists.
  - The signature file is written whole, sorted and without duplicates, through a temporary
    file and a rename. A file that does not read fails `verify` (exit 1): a damaged signature
    is not an absent one. Exit 2 stays for usage and the key file, as §8 says.
  - `sign` signs every attestation by the key's agent that this key has not signed. Other
    agents' attestations are left to their keys.
  - `merge --sign` needs `--staged` and `-o`: it signs the staged batch, and a signature file
    goes beside a named output. A merge to stdout whose inputs carry signatures says it did not
    write them.
  - `retract --sign` needs the key's agent among `--as`. The attestation it records on the
    edge is `op: attested`, `rung: document`, at the wall clock, and is written with the edge
    in one append. A surface store cannot hold an attestation, so it refuses both.
  - `check --verify-sigs` reports each finding as a diagnostic with the code
    `sign/<verdict>` (`sign/unsigned`, `sign/invalid-signature`, …) and fails with exit 3, as
    `check`'s other errors do.
  - `sign` and `verify` take the store positionally or with `-s`. The default key file is
    `.smysl/keys.hjson` beside the store, or inside a library.
