# RFC SMYSL-8 — trust that reads signatures

**Author:** Vladimir Ulogov
**Status:** amended in review against the code (R1, 2026-11-24). TR-P1 to TR-P4 built (1.21): complete.
**Target compatibility:** smysl 1.x, strictly additive; format `smysl/1.0`, unchanged.
**Crates:** `smysl-graph` and `smysl-thread` (an admission filter, additive), `smysl-sign` (an
admission function, key-file editing), the CLI (`--trust-sigs`, `smysl keys`, `--sign` at commit,
`merge --require-sigs`).
**Depends on:** RFC SMYSL-7 (`smysl-sign`), built to SG-P4 in 1.20.
**License:** MPL-2.0.
**Evidence baseline:** `dev/1.21.0` at `a65932d`. Every code pointer below was read there.

---

## 1. Summary and motivation

RFC SMYSL-7 made signatures and stopped at the report. `smysl verify` says which attestations
verify, and `verified_attesters` gives each unit its verified agents, but nothing that decides
anything reads either. Its §1 named three gaps, and after 1.20 all three are still open:

1. **Corroboration can still be inflated.** `corroboration` in
   `crates/smysl-graph/src/salience.rs` groups every attestation on a unit by
   `corroboration_key()`, and counts a group whoever wrote it. Two attestations typed under two
   made-up agent ids are two groups. The only defence is `SalienceWeights::untrusted()`, which sets
   the corroboration weight to 0 and discards the honest signal with the dishonest one.
2. **Origin authority is still a string comparison.** `plan_retraction`
   (`crates/smysl-graph/src/merge/retraction.rs`) authorises `RetractionAuthority::Origin` when a
   requesting agent's id is among the agents on the unit's attestations. `retract --as` takes any
   id. `Quorum(n)` counts distinct `--as` strings.
3. **Nothing enforces a trust boundary.** `merge` accepts any input. Signatures are made only by
   `smysl sign` after the fact, or by `merge --staged --sign`; the commands that create
   attestations at a trust boundary (`ingest --yes`, `ingest --text`, `tel read`) cannot sign what
   they commit. Keys are edited by hand in `.smysl/keys.hjson`.

This RFC closes them with one rule and four uses of it:

- **Admission (§3).** Under a verification mode, an attestation is *admitted* when that mode
  would not fail it. Admission is computed once, in `smysl-sign`, and handed down as a plain
  set of excluded attestation hashes, because `smysl-graph` sits below `smysl-sign` and cannot
  call it (SMYSL-7 R1-8).
- **A1, trust (§4).** Salience counts corroboration only from admitted attestations, and Origin
  authority looks only at admitted attesters. The CLI proves requesting agents with their keys.
- **A2, `smysl keys` (§6).** List, add, revoke, rotate and check keys, so `.smysl/keys.hjson` is
  no longer hand-edited.
- **A3, signing at the commit (§7).** `--sign --key K` on `ingest --yes`, `ingest --text` and
  `tel read`.
- **A4, a merge policy (§8).** `merge --require-sigs MODE` refuses inputs that fail the mode.

With no mode chosen, every result is bit-for-bit what it is today.

## 2. Invariants

| | Requirement | How this RFC meets it |
|---|---|---|
| **G1. Semver and format** | `smysl/1.0`; `cargo-semver-checks` on every published crate | No record, key or wire change. New fields go on `#[non_exhaustive]` structs (`SalienceRequest`, the thread's options); new functions sit beside the existing ones, which keep their behaviour. |
| **G2. Purity** | Rule B | `Admission` is data. `smysl_sign::admission` and the key-file editing are pure: text in, text out. The CLI reads and writes the files. |
| **G3. Determinism** | Rule D | Admission is a function of the store, the signature list, the key file's text and the mode. Excluded hashes are a `BTreeSet`; groups keep canonical key order. |
| **G4. Epistemic safety** | A signature never raises a status | Admission removes attestations from *counts*: corroboration and origin authority. It never changes a declared or effective status, never removes a record, and never adds evidence. An excluded attestation stays in the store and in every view. |
| **G5. Library first** | Rule A | Admission is computed in `smysl-sign` and read in `smysl-graph` and `smysl-thread`; the commands are thin dispatch over `smysl::sign`. |

## 3. Admission

SMYSL-7 §6 already decides, for each attestation and mode, whether the mode fails it:
`VerificationMode::fails(verdict, in_scope)` is `verdict.is_forged() || (in_scope &&
!verdict.is_verified())` (`crates/smysl-sign/src/verify.rs`). Admission is its complement:

> **An attestation is admitted under mode M when M does not fail it.**

| Attestation's verdict | `Advisory` | `Selective`, in scope | `Selective`, out of scope | `Strict` |
|---|---|---|---|---|
| Verified | admitted | admitted | admitted | admitted |
| Unsigned | admitted | **excluded** | admitted | **excluded** |
| Unknown key | admitted | **excluded** | admitted | **excluded** |
| Forged (invalid, revoked, agent mismatch, over cap) | **excluded** | **excluded** | **excluded** | **excluded** |

So `Advisory` drops only what is provably wrong, `Strict` keeps only what is provably right, and
`Selective` is `Strict` where the deployment says it matters. There is no fourth table to learn:
whatever `smysl verify --mode M` reports as an error is exactly what `--trust-sigs M` stops
counting.

```rust
// smysl-graph: what the layers below the signature crate read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Admission {
    /// Attestations judged, by record hash (`Store::record_hash`, the `target_hash` SMYSL-7
    /// signs), and whether each is admitted.
    pub judged: BTreeMap<[u8; 32], bool>,
    /// The answer for an attestation not judged: `true` under no policy and `Advisory`,
    /// `false` under `Selective` and `Strict` (R1-2).
    pub unjudged: bool,
}
impl Admission {
    /// No policy: everything admitted. The `Default`.
    pub fn all() -> Admission;
    pub fn admits(&self, a: &Attestation) -> bool;
}

// smysl-sign: the one place admission is decided.
pub fn admission<R: KeyResolver>(
    store: &Store,
    sigs: &[DetachedSignature],
    resolver: &R,
    mode: &VerificationMode,
) -> Admission;
```

`admission` runs the same `judge()` as `verify_store`, including SMYSL-7 R1-14's matching of a
changed attestation to its leftover signature, so an attestation edited after signing is forged
here too. It judges every attestation the store's units and edges hold, not only those with a
record in the log, and anything it still did not see is decided by `unjudged`: a policy fails
closed (R1-2). `admits` hashes `Record::Attestation(a)` with `Store::record_hash`, the function
`judge` already uses, so the two crates cannot disagree on identity (R1-1).

`verified_attesters` stays as it is. It is the `Strict` view keyed by agent, and a caller that
wants names rather than counts still uses it.

## 4. A1: what admission changes

### 4.1 Corroboration

`SalienceRequest` gains `admission: Admission` (default: all). `corroboration` builds its groups
from admitted attestations only; a group whose every attestation is excluded is not formed.
`SalienceTerms` (`#[non_exhaustive]`, beside `groups` and `dependent_groups`) gains
`excluded_groups: Vec<String>`, the group keys that would have been formed but for admission, so
`smysl salience --explain` shows what a signature policy removed and why a unit fell (R1-3).

Ancestry collapse (D-8) runs after admission: an excluded attestation's parents claim nothing.
Otherwise a forged attestation could still knock an honest group out by claiming its ancestry.

`smysl-thread`'s `derive_thread` builds its own `SalienceRequest`
(`crates/smysl-thread/src/derive.rs:111`); `DeriveOptions` (`#[non_exhaustive]`) gains the same
`admission`, passed through. `smysl-pack` takes a `SalienceReport` from its caller and needs
nothing. `smysl-query`'s executor and dossiers (`exec.rs`, `dossier/mod.rs`) and the TUI build
default requests; they take an `admission` from their callers the same way (R1-4). The graph
measures in `smysl-measure` stay unadmitted: they measure the graph as written.

### 4.2 Origin authority

```rust
pub fn plan_retraction_admitted(
    store: &(impl StoreRead + ?Sized),
    target: Uid,
    agents: &[AgentId],
    policy: RetractionPolicy,
    authority: RetractionAuthority,
    admission: &Admission,
) -> RetractionPlan;
```

`plan_retraction` becomes `plan_retraction_admitted(…, &Admission::all())`. Under Origin, the
attestors compared with the requesting agents are those of the unit's *admitted* attestations, so
an origin claimed by a forged or, under Strict, an unsigned attestation does not authorise. The
refusal says so: "origin authority: the requesting agent's attestation on this unit is not
admitted under strict".

**Proving the requester.** The library takes `agents` as given; the CLI is where an id has to be
backed. Under `--trust-sigs M`, each `--as A` that M puts in scope needs a `--key` whose agent
is A (`--key` becomes repeatable), and `Quorum(n)` needs n such keys. With the keys in hand,
`retract --sign` (SMYSL-7 SG-P3) already writes and signs each agent's attestation on the
retraction edge, so the proof is recorded, not only checked. Under `Advisory`, as today, an
unproven `--as` is accepted with a warning.

`withdraw` takes `--as` and `--authority` the same way, but its origin check is written in the
CLI, against the edge's attestations (`cmd_withdraw`, `src/main.rs`). It moves to `smysl-graph`
as `withdrawal_authorised(edge, agents, authority, &Admission)`, so retraction and withdrawal
read one rule and the CLI stays dispatch (R1-5).

### 4.3 What admission does not touch

- **Statuses.** Rule M, rule T, effective status and every check pass read attestations as they
  are. A `measured` unit with an unsigned import is still `measured`. Making a status depend on a
  signature would let a key policy raise or lower what a unit claims, which G4 forbids. A
  deployment that wants unsigned measurements refused uses `Selective` with
  `status_at_least: measured` in `verify` or `merge --require-sigs`, where it refuses rather than
  rewrites.
- **Merge.** Merge never adjudicates. Admission filters counts at read time and never removes a
  record from a store.
- **Views and rendering** show every attestation. Neither reads salience (R1-4), so neither
  takes the policy.

## 5. Choosing a policy

A flag, or the project's configuration, both resolved once per command:

```
--trust-sigs MODE         advisory | selective | strict; absent: no admission, today's results
--keys FILE               default: .smysl/keys.hjson beside the store, or in the library
--agent-prefix P          Selective's scope (repeatable)
--status-at-least S       Selective's scope
--sigs FILE               default: SMYSL-7's signature path for the store
```

These are SMYSL-7's `verify` options under one more name, since `--verify-sigs` on `check` *reports*
and `--trust-sigs` *acts*. They apply to the commands that read salience or authority:
`salience`, `pack`, `thread`, `q`, the dossier commands, `ui`, `retract` and `withdraw` (R1-4).

```hjson
# .smysl/config.hjson
trust: {
  signatures: "selective"
  agent_prefixes: ["model:", "tool:"]
  status_at_least: "measured"
}
```

The provider configuration (`crates/smysl-provider/src/config.rs`) reads its keys by name, so
`trust:` is read beside it by a pure `TrustConfig::parse` in `smysl-sign`, from the same file's
text (R1-8). A flag overrides the configuration. When a mode is chosen and the key file is missing or
malformed, the command exits 2 before reading the store, as `verify` does: a policy that cannot
be applied is not silently dropped.

## 6. A2: `smysl keys`

```
smysl keys list   [--keys FILE] [--json]
smysl keys add    --agent A --ed25519 hex:…|base64:… [--max-status S] [--not-before MS --not-after MS]
smysl keys revoke --kid KID (--after MS | --compromised) [--reason TEXT]
smysl keys rotate --agent A --out NEW.key [--max-status S]
smysl keys check  STORE [--sigs FILE]
```

- **`list`** prints each entry: agent, kid, status, cap, bounds.
- **`add`** takes a public key in either SMYSL-7 encoding, or `keygen`'s printed entry on stdin.
- **`revoke`** sets `status: revoked`. `--after MS` refuses signatures whose attestation clock
  reads `MS` or later. `--compromised` sets no time, so every signature the key made is refused,
  because a stolen key can backdate (SMYSL-7 §9).
- **`rotate`** is `keygen` for the agent, `add` of the new entry, and `revoke --after now` of the
  agent's active keys, in one write. Signatures made before the rotation stay valid.
- **`check`** reads a store's signatures against the file: signatures per key, signatures naming
  an unknown kid, and keys that signed nothing.

The file is rewritten whole, through `.tmp` and a rename, keeping its permissions. Its text comes
from a new pure `KeyFile::render(&MemoryKeyResolver) -> String`, which `KeyFile::parse` reads back
to an equal resolver (a property test). HJSON comments do not survive a render, so a file with
comments other than the leading `#` block is refused unless `--rewrite` is given, and the refusal
names the first comment's line.

`MemoryKeyResolver` already has `insert` and `keys`; it gains `set_status`, so editing is a
library operation and the command only reads and writes the file (R1-9).

## 7. A3: signing at the commit

`--sign --key K` on the commands that commit attestations directly:

| Command | Commits | Attestation agent |
|---|---|---|
| `ingest … --yes` | the staged batch | `tool:smysl-ingest` (`IngestOptions`' default, `crates/smysl-ingest/src/lib.rs:636`) |
| `ingest --text` | each window as it is staged | `tool:smysl-ingest` (`text/run.rs:331`) |
| `tel read` | snapshots and series units | `tool:smysl-tel` |

The key's agent must be the attestation agent. A key for `human:alice` cannot sign attestations
by `tool:smysl-ingest` (SMYSL-7 would give `AgentMismatch`). That is checked before anything is
read or any provider is called, and fails with exit 2, so a wrong key costs no tokens.

Ingest attests as the tool, not as the model, whichever provider ran; the provider, model and
prompt are in the recipe, which is what corroboration groups by (R1-6). So a key for
`tool:smysl-ingest` belongs to whoever operates that installation, and its signature says that
this attestation came from their ingest, and nothing about whether the model was right. One key
covers every model the operator runs.

`attest` is not on the list: it prints its judgements and writes nothing (`cmd_attest`,
`src/main.rs`), so there is nothing to sign (R1-7).

Signatures are written after the commit, with SMYSL-7's writer (sorted, deduplicated, `.tmp` and
rename). If the commit succeeds and signing fails, the command exits 1 and names the store, which
is then unsigned rather than wrongly signed; `smysl sign` can finish the job. The staged path,
`ingest` without `--yes` followed by `merge --staged --sign`, is unchanged.

`same-as`, `link`, `summarise`, `migrate` and `ask --answer` also commit attestations. They take
`--sign` in a later phase, once these four have shown the pattern.

## 8. A4: `merge --require-sigs`

```
smysl merge A.cbor B.cbor -o OUT.cbor --require-sigs strict [--keys FILE] [--agent-prefix P] …
```

The merged store, with the merged signature list, is judged under the mode **before** the output
is written. If any attestation fails, merge exits 1, writes nothing, and prints `verify`'s report
(or its JSON with `--json`), each finding naming the inputs that carried the record. With
`--staged`, the staged batch is part of what is judged, after `--sign` has signed it, so `merge
--staged --sign --key K --require-sigs strict` commits only a batch that verifies.

The merged result is judged, not each input, because a record signed in one input and present
unsigned in another is verified once merged (SMYSL-7 §6: a record's verdict is its best valid
signature unless one is forged), and refusing that merge would refuse the very union that
supplies the proof.

## 9. API summary

| Crate | Addition |
|---|---|
| `smysl-graph` | `Admission`; `SalienceRequest::admission`; `SalienceTerms::excluded_groups`; `plan_retraction_admitted`; `withdrawal_authorised` |
| `smysl-thread` | `DeriveOptions::admission` |
| `smysl-query` | an `admission` on the executor's and dossiers' salience |
| `smysl-sign` | `admission`; `TrustConfig::parse`; `KeyFile::render`; `MemoryKeyResolver::set_status` |
| facade | re-exports under `smysl::sign` and the existing paths |
| CLI | `--trust-sigs` and its options; `smysl keys` (45 commands); repeatable `--key` on `retract` and `withdraw`; `--sign --key` on `ingest --yes`, `ingest --text`, `tel read`; `merge --require-sigs` |

## 10. Compatibility and risks

- **Defaults.** Without `--trust-sigs` or `trust:` in the configuration, every salience,
  thread, pack and retraction result is unchanged. TR-P1's gate checks that bit-for-bit.
- **A strict policy over unsigned history** gives corroboration 0 and refuses Origin retractions
  until the history is signed. That is the intended reading of `Strict`. `smysl sign` signs an
  existing store, and `Selective` gives a path between.
- **Cost.** Admission hashes every attestation once and verifies every signature once per
  command. For the library sizes measured in SMYSL-2.8 that is linear and small beside salience's
  PageRank; TR-P1 measures it.
- **Key-file comments** are lost on a rewrite, which is why a commented file needs `--rewrite`.
- **Clocks** are as SMYSL-7 §9 states. `--compromised` is the answer to a stolen key.

## 11. Phases

- **TR-P1 — admission.** `Admission`, `smysl_sign::admission`, corroboration and thread derivation
  under admission, `plan_retraction_admitted`. **Gate:** with `Admission::all()` every existing
  salience, thread and retraction test and determinism golden is unchanged; a store where four
  invented agents corroborate a unit has corroboration 1.0 under no policy and the signed groups'
  share under `Strict`; a forged attestation claiming an honest group's ancestry collapses nothing;
  Origin refuses an agent whose only attestation is forged; under `Strict`, an edge attestation
held in memory with no log record is excluded, and under `Advisory` admitted.
- **TR-P2 — the policy in the CLI.** `--trust-sigs` and `trust:` on `salience`, `pack`, `thread`,
  `q`, the dossier commands, `ui`, `retract`, `withdraw`; `withdrawal_authorised`; repeatable `--key` proving `--as`; `--explain` shows
  exclusions. **Gate:** the same store packed under no policy and under `Strict` differs only in
  units whose corroboration came from unsigned attestations, and `retract --as human:mallory`
  with origin claimed by an unsigned attestation is refused under `Strict`.
- **TR-P3 — keys and signing at the commit.** `smysl keys`, `KeyFile::render`, `--sign` on
  `ingest --yes`, `ingest --text`, `tel read`. **Gate:** `render` then `parse` gives an
  equal resolver over generated key sets; `rotate` leaves earlier signatures valid and later ones
  by the old key refused; `tel read --sign` gives a store that passes `verify --mode strict`; a key
  for another agent exits 2 before any provider call (asserted with a provider that records calls).
- **TR-P4 — `merge --require-sigs`, the gates and the manual.** **Gate:** merging an unsigned
  input under `Strict` writes nothing and exits 1; merging a signed and an unsigned copy of one
  attestation passes; `check-purity`, `cargo semver-checks`, `doc-output` on a new manual section.

## 12. Questions for R1

All five were answered by reading the code; the answers are R1-1 to R1-9 below.

## 13. Amendments

**R1, 2026-11-24: the draft read against the code.**

- **R1-1. Identity.** A unit's attestations are the logged `Attestation` values, attached
  unchanged (`attach`, `crates/smysl-graph/src/store/mod.rs:1236`), and `Store::record_hash`
  (`:1124`), which `judge` already uses, lives in `smysl-graph`. So `Admission::admits` hashes
  with the same function and needs no shared helper. The draft's `admission` signature is
  corrected to what `verify_store` takes: `&Store` and `&[DetachedSignature]`. `load_memory`
  gives a `Store` for a library too, as `verify` does.
- **R1-2. Fail closed.** `judge` walks the log, and an edge can hold attestations in memory that
  have no record of their own (`store/mod.rs:480`). An exclusion set would admit any attestation
  it never saw, so `Strict` would fail open. `Admission` now records what it judged and decides
  the rest by `unjudged`: admitted under `Advisory`, excluded under `Selective` and `Strict`.
  `admission` also judges the attestations the units and edges hold, not only the log's.
- **R1-3.** `SalienceTerms` is `#[non_exhaustive]` and already has `groups` and
  `dependent_groups`; the new field is `excluded_groups`, beside them.
- **R1-4. Who reads salience.** Not `render` or `view`. Callers are `salience` and `pack` in the
  CLI, `derive_thread`, `smysl-query`'s executor and dossiers, the TUI, and `smysl-measure`'s
  graph measures. The policy reaches all but the measures, which measure the graph as written.
  `DeriveOptions` is `#[non_exhaustive]`, so its `admission` is additive.
- **R1-5. Withdrawal.** `withdraw`'s origin check is in the CLI, not the library. It moves to
  `smysl-graph` as `withdrawal_authorised`, so one rule reads admission for both.
- **R1-6. Ingest's agent.** `ingest` and `ingest --text` attest as `tool:smysl-ingest`, the
  `IngestOptions` default, which the CLI never overrides; the draft's "model agent" was wrong.
  The model is in the recipe. One operator key signs every model's ingest.
- **R1-7. `attest` writes nothing.** It prints judgements and exits, so `--sign` on it is
  dropped. Should `attest` ever commit its attestations, it takes `--sign` then.
- **R1-8. Configuration.** `.smysl/config.hjson` is parsed by `smysl-provider`, by key. `trust:`
  is read beside it by `TrustConfig::parse` in `smysl-sign`, from the same text, rather than by
  adding signature policy to the provider crate.
- **R1-9.** `MemoryKeyResolver` already has `insert` and `keys`; only `set_status` is new.

**R2, 2026-11-24: TR-P1 as built.**

- **R2-1. The name.** The type is `AttestationAdmission`, not `Admission`:
  `smysl_core::Admission` is a view's granularity (`SingleAssertion`, `Topical`), and the facade
  flattens `smysl-core` and `smysl-graph` into one namespace. §3's `Admission` reads as
  `AttestationAdmission` throughout; the module is `smysl_graph::admission`.
- **R2-2. `verify` sees held attestations too.** R1-2's judging of attestations a unit or edge
  holds with no log record of their own is in `judge`, which `verify_store` and
  `verified_attesters` share with `admission`. So `smysl verify --mode strict` now fails a store
  whose edge carries an unsigned attestation held in memory, where before it did not see it; a
  store read from disk has none, since its edges' attestations come from records.
- **R2-3. The gate** is `crates/smysl-sign/tests/admission.rs`: no policy gives an identical
  report; four attesters, two invented, give corroboration 1.0 with no policy and 0.5 under
  `Strict` and under `Selective` over `model:`; what `verify_store` fails is exactly what
  `admission` excludes, in each mode; a forged `human:aaron` that displaced an honest
  `human:alice` group with no policy is excluded under `Advisory`, and alice's group counts;
  origin authority refuses an agent whose only attestation is forged; an edge attestation with no
  log record is excluded under `Strict` and admitted under `Advisory`.

**R2, continued: TR-P2 as built.**

- **R2-4. `admission` over any store.** Most commands hold an `AnyStore`, a library on disk
  among them, and copying it into memory to judge it would cost a read of the whole log. Counts
  read only what units and edges hold, so `admission` takes any `StoreRead` and judges those
  attestations; `verify_store` still takes a `Store` and judges its log as well. `judge` now takes
  the records to judge and a status lookup.
- **R2-5. A key proves an agent only if it verifies.** §4.2 said "a `--key` whose agent is A".
  Holding a key file proves nothing — anyone can `keygen` a key for any id — so the proof is
  the attestation `retract --sign` would record, signed with the key and verified against the
  key file now: an unknown, revoked, out-of-bounds or over-cap key fails it, with the verdict
  in the message. An `--as` in the mode's scope without such a key exits 1; under `advisory`
  every agent is out of scope, and an unproven one is a warning.
- **R2-6. Which commands.** `--trust-sigs` is on `salience`, `pack`, `thread`, `q`, `ask` (the
  dossier command), `ui`, `retract` and `withdraw`. `ask --recheck` re-runs saved dossiers with
  no policy, as before; `measure` and `assess` stay unadmitted (R1-4).
- **R2-7. A build without `sign`** cannot apply a policy, so `--trust-sigs`, or a `trust:` in
  the configuration, is a usage error there rather than counting everything.
- **R2-8. The gate** is `tests/cmd_trust.rs`: ranked under `strict`, only the unit an invented
  agent corroborated moves, and `advisory` ranks as no policy does; a fully signed store ranks
  and packs identically under every policy; `trust:` applies without the flag, and a bad one
  exits 2; `retract --as human:mallory`, proven by her key, is refused under `strict` because
  her origin attestation is unsigned, while alice's signed one authorises; a `keygen`ed key the
  key file does not hold proves nothing; a quorum of two signs with both keys; `withdraw` reads
  the same rule.

**R2, continued: TR-P3 as built.**

- **R2-9. `smysl keys`.** Its key file is `--keys`, else `.smysl/keys.hjson` where it runs; for
  `check`, beside the store, as `verify` reads it. `--kid` takes the id or a prefix of at least
  eight digits naming one key. `rotate` revokes the agent's active keys with `revoked_after_ms`
  at the wall clock and the reason "rotated to KID", and refuses a commented file before it makes
  the new key, so a refusal leaves no orphaned secret. `KeyFile::comment_line` finds the first
  comment past the leading `#` lines, outside strings. A key id already in the file is refused
  by `add`.
- **R2-10. The signature path is chosen after the commit.** `tel read --yes` can create the
  library it writes; checked before the run, `sig_path` took the not-yet-library for a file and
  the signatures went to `lib.sig` beside it. Found by the gate's first run.
- **R2-11. What is signed.** The attestations by the key's agent that the store gained during the
  run: their record hashes are taken before and compared after. `ingest --sign` needs `--yes`;
  a staged batch is signed by `merge --staged --sign` when it is committed. `tel read --sign`
  wraps all four readers, which all attest as `tool:smysl-tel`. `ingest --text` signs what its
  windows committed even when a later window failed.
- **R2-12. The gate** is `crates/smysl-sign/tests/keyfile_render.rs` (render and parse are
  inverses over 300 generated key sets, with quotes, backslashes, `#` and `//` in reasons),
  `tests/cmd_keys.rs` (a signature before a rotation verifies under `strict`, the old key's after
  it fails even `advisory`, the new key's verifies; `add`, `list`, `revoke --compromised`,
  `check`; a commented file refused) and `tests/cmd_commit_sign.rs` (`tel read --sign` gives a
  library that passes `strict` with its signatures inside it; `ingest --text --sign` signs every
  attestation it made; a key for `human:alice` exits 2 on `ingest --text`, `ingest FILE` and
  `tel read` with the loopback provider's connection count unchanged).

**R2, continued: TR-P4 as built.**

- **R2-13. When the policy is read.** `--require-sigs`'s mode and key file are resolved before
  any input is read, so a bad mode or a missing key file exits 2 having read nothing. The key
  file defaults to `.smysl/keys.hjson` beside the output, or the working directory's when the
  output is stdout. `--keys`, `--agent-prefix` or `--status-at-least` without `--require-sigs`
  is a usage error rather than ignored. `--sigs` is not taken: a merge's signatures are its
  inputs'.
- **R2-14. Naming the inputs.** Each finding names every input that carried its record, in the
  order given, and `staged` for the staged batch: ` (in a.cbor, b.cbor)` in text, an `inputs`
  list in `--json`. The report's store is the output's name, or `merged` for stdout. A pass is
  one line on stderr, since stdout may be the merged store.
- **R2-15. The gate** is `tests/cmd_merge_require_sigs.rs`: an unsigned input under `strict`
  writes neither the output nor its `.sig` and exits 1; a signed and an unsigned copy of the
  same attestations pass in either order, to identical bytes; two inputs carrying one unsigned
  record are both named; a staged batch fails unsigned and passes once `--sign` signs it; a
  policy that cannot be applied exits 2 before the absent input is read. The manual's new
  section (chapter 21, "Signatures that act") is replayed by `doc-output` against fixtures
  committed under `fixtures/tutorial/ch19-21-verify/`; no secret key is committed.
