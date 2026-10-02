# SMYSL-2 — review and implementation plan

**Written:** 2026-10-02, against the RFC set at `Documentation/SMYSL-2_RFC/` (commit `5563fd5`)
and the tree at `d25ec9e`.
**Status:** response to the set. Not normative. The RFC set is the input; this is what to do
about it.

---

## 1. Verdict on the set

**It is sound, and its claims about today's code hold up.** Eighteen concrete assertions from
SMYSL-2.1 §2 were checked against the tree: seventeen confirmed with line numbers accurate to
±2, one miscited in a way that makes its own conclusion *stronger* rather than weaker (2.1 line
77 cites `thread.rs:385` for an enum cast; that line casts a `u8`, but the real casts are
`self as u8` inside `as_u8` in-crate, so a data-carrying variant would fail to compile inside
`smysl-core` before any downstream cast). A document that marks its unverifiable claims
*(unverified)* in place and survives spot-checking at that rate can be planned against.

Two anchors were re-verified directly rather than taken on report:

- **An unknown enumeration code is fatal.** `DetectionKind::from_u8(…).ok_or_else(|| bad(a))?`
  (`cbor/envelope.rs:852`), and the same shape at twelve other decode sites. Not a warning —
  the store does not open.
- **`converged_with` is blind to commitments.** `state_hash` (`smysl-graph/src/store/mod.rs:907`)
  folds units, attestations, salience, labels, relations, threads, views, withdrawals,
  resolutions and contentions. It never mentions commitments, schema declarations, pack infos or
  unknown records. `converged_with` is `state_hash` equality (`:953`). **Two stores differing
  only in commitments report as converged in shipped 1.8.0.** That is a live defect in the
  feature 1.7 added, not merely a gap the RFC wants closed.

---

## 2. The decision that cannot wait: gate G−1

The set asks the owner to accept four parts of SMYSL-2.3 **before 1.9.0 ships**. They are not
equally urgent, and the set does not rank them. Ranked by irreversibility:

### 1. A-8.1 — open the enumerations. **Irreversible. Decide first.**

Open `ThreadSchema`, `Role`, `SourceKind`, `DetectionKind`; reserve code 255 in those four plus
`admission`. `status`, `lod`, `op`, `rung` and `commitment` stay closed, because rules M, T and L
read them.

If 1.9.0 ships with these closed, spec §8.1's mechanical test ("a reader written against the
older revision must round-trip a document containing the addition, byte for byte") fails for
**every** code allocated afterwards. Each one becomes a **format major bump**, and shipped 1.9.x
readers can never open a store from the 2.4/2.5 era — they fail to open it, not degrade.

Opening the enums in 1.10 does not repair readers already in the field. This is the only one of
the four where the ordering cannot be bought back later.

### 2. A-10 item 2 — the `lang` lint. **The window closes with the release.**

`SMY-W432` must ship a release *before* A-1 activates at TX-P5. The lint is the entire
mitigation: at upgrade, every document that wrote `lang:` as a payload key has that key move
from the hashed payload to hashed core key 9. **The bytes of the core change, the uid changes,
and nothing reports it.** Corpus-wide, undetectable from inside one peer.

### 3. A-4 — the record-set digest. **Retrofittable, but it is the ruler.**

Without it, `converged_with` keeps returning `true` for stores that differ in commitments,
schema declarations, pack infos or unknown records. 1.9.0 simultaneously introduces record types
14–19 and a convergence check structurally blind to them, so every convergence test from TX-P1
onward is measured with a broken instrument and the failures read as library bugs.

### 4. A-9 — the estimator key. **Smallest, and the most mechanical.**

`dec_granularity` collects unknown keys into a local `extra` and drops it on the floor
(`cbor/envelope.rs:730`, verified). A 1.9.x reader silently drops granularity key 5, so its
re-encode is not byte-identical — a plain C-Read failure. Combined with A-4, a dropping peer and
a preserving peer compute **different digests for the same store**, so rule U reports permanent
spurious divergence with no visible cause.

---

## 3. Three things to settle before accepting 2.3, beyond G−1

These are not blockers on the plan, but they should not be folded into the spec as written.

**(a) A-10 item 2 is neither a §8.1 addition nor a §8.3 tightening.** §8.1's test asks whether an
*older reader* can round-trip a *new document*. This is the reverse: a *new reader* reads an
*existing, valid* document differently, moving a key from the hashed payload to the hashed core.
That is "changing the meaning of anything in §4", which §8.2 says requires a new format version.
The RFC's §0 claim that every amendment is an addition or a tightening does not hold here, and
line 482 concedes the substance. **Either accept it explicitly as a one-off out-of-band break
with `W432` as a hard shipping precondition, or pick a spelling that collides with nothing
(`@lang:`).**

**(b) Rule Z and A-4 give rule U two incompatible meanings.** A-4 defines convergence as
record-set-digest equality. Rule Z makes store state the union of records *filtered* by
accumulated redactions, and says explicitly that a pre-amendment peer keeps record 15 as an
unknown record. Two peers behaving exactly as specified then **never converge**. The semilattice
argument is correct about *state*; A-4 measures the *raw record set*. **A-4 must say which, and
it has to be the filtered set, with redactions themselves inside the digest** — so a peer missing
a redaction is reported as not-yet-converged rather than permanently divergent.

**(c) `SourceKind` is the only opened enumeration inside the uid.** A-8.1 says both "preserve an
unknown code and re-encode it unchanged" and "an implementation MAY use 255 internally to
represent unknown". For the other four enums, normalising to 255 corrupts a non-hashed record.
For `SourceKind` it **silently changes the uid**. Add a sentence making the raw code
identity-bearing for `source.kind`, and make the conformance fixture include a unit with an
unknown source kind asserting the uid is unchanged.

---

## 4. The 1.9.0 collision, and why it resolves itself

`dev/1.9.0`'s changelog currently promises five items carried from 1.8: a staleness check, rival
explanations (`contrasts`), a `timeline` thread schema, corroboration, and an incremental
retrieval index. None of the five appears anywhere in the RFC set — `contrasts` has zero hits
across all nine documents.

They are not in conflict. They are the same work, and the RFC says so without knowing it:

- A **`timeline` thread schema** is thread schema code 5 — exactly what A-8.2 allocates, with
  roles 49–63 reserved for it.
- **Rival explanations** need a new `DetectionKind` — exactly A-8.2's codes 4 and 5 territory.

**Both are blocked behind A-8.1.** Shipping either into a 1.9.0 with closed enumerations
produces stores that no 1.9.x reader can open. So the queued work *requires* the G−1 decision
regardless of whether the rest of the RFC is adopted.

Corroboration, the staleness check and the incremental index are independent of the RFC and can
move to 1.10 without cost.

---

## 5. Implementation steps

TX-P0 is sized by the RFC at **4–6 engineer-weeks**, with F-2's calibration and F-12 the long
poles. The order below is the RFC's twelve-step order (2.1 §6.1) with the dependency constraints
made explicit and three corrections folded in.

### Step 0 — before any code

1. ~~**Install `cargo-semver-checks` and establish the baseline.**~~ **Done, 2026-10-02.**
   See §8. The RFC's §2.3 claim that the tool "is not installed in the environment" is false
   here: `cargo-semver-checks 0.50.0` was already present. §6.2's assertion that every change is
   API-additive is therefore checkable rather than assumed, and each TX-P0 item should be
   measured against the recorded baseline rather than asserted against it — F-12 above all,
   where the no-major-bump argument rests entirely on `#[non_exhaustive]` suppressing
   `enum_variant_added`.
2. **Owner decides G−1**, at minimum A-8.1.
3. **Resolve §3(a) and §3(b) above** — they change normative text, not code.

### Step 1 — the free wins, in any order

`H-16` (minutes: a missing `Record::Commit` arm in the merge omitted-count filter),
`H-8` (`-C/--config` is declared and never read), `H-10` (`num_ctx` never sent to Ollama),
`H-12`, `H-11`, `H-5`.

### Step 2 — `H-1` before `F-2`

Granularity `extra` preservation is the precondition for the estimator key. **Correction to the
RFC:** make `estimator` an `Option<TokenEstimator>`, not a defaulted field. As written
(2.1 lines 275–277), a document carrying key 5 = `"smysl/utf8-div4"` decodes and re-encodes
*without* key 5 — a round-trip failure in the very sub-map H-1 exists to protect. The
`source::OBSERVED` precedent works only because `observed` is an `Option`.

### Step 3 — `F-12` (+ `H-4`, `H-7`, `W409`). The long pole.

Opens four enumerations with an `Unknown` unit variant plus a preserved private code. Touches
core types, codec, check, thread, render, the ingest recipe and a four-language conformance
fixture. **Must land before any new thread schema or detection kind**, which means before the
`timeline` and rival-explanation work.

**Correction:** F-12 allows an API-constructed `Unknown` with no code to encode as 255. Since the
preserved code is in the uid preimage, a decoded unknown (code 9) and a constructed one hash
differently. Refuse to encode a code-less `Unknown` rather than documenting the hazard.

### Step 4 — `F-13` + `F-14` + `H-3`: strict surface parsing

**Correction:** the RFC widens this further than its own changelog entry says. Today a failed
`source()` drops the source and keeps the unit; §4.3.7 makes any source error refuse the whole
unit, which silently catches the pre-existing "source needs `kind` and `ref`" path. Documents
with a misspelled `kind` lose the unit entirely in 1.9. Name that case in the §8.3 changelog
entry, loudly.

### Step 5 — `F-18` + `H-2`: surface-form predicates

After F-12 and F-13/F-14.

### Step 6 — `F-17`: the record-set digest

Then **step 7 — `F-16`** (exhaustive `emit`, the keep-set fixpoint, `H-14`, `H-15`), whose exit
test uses the digest. Run the merge-algebra fuzz targets here, not at step 12: F-17 strengthens
what commutativity and idempotence *claim*, and that should be exercised early.

### Step 8 — the remaining F-items

`F-3` (quote normaliser V2), `F-4` (`folding_for`), `F-6` (language policy), `H-6`
(prompt-injection fencing), `H-9`.

### Step 9 — CLI consistency

`H-17` (`--format` honoured or refused across 26 commands), `H-19`, `H-20`, `H-18`
(derive indexing).

**Correction to H-18:** `matches` currently short-circuits on `store.get(uid).is_none()` before
any matcher runs. A pure set lookup loses that guard, so a uid appearing as a relation endpoint
with no `UnitCore` would newly match. The index build must carry the same filter, and the golden
test must include a store with a dangling edge — otherwise "byte-identical by construction" is
false in exactly the case nobody fixtures.

### Deferred, with a recommendation

- **`F-2` may slip to 1.10.0** (the RFC permits this). OQ-31 is open and the calibration, corpus
  licensing and parity test dominate the cost. Nothing else in TX-P0 depends on it.
- **`H-13` should not ship as written.** It turns a read-only invocation into one that writes
  `--store`, justified by help text, in a minor release. This project's own semver discipline
  argues for warning in 1.9 and committing in 1.10.

---

## 6. The spike (S0), in parallel

SMYSL-2.2 is a well-formed experiment: ten passages, five languages, four arms, a decision table
that can only reorder work or change defaults, never kill a phase. It needs no TX-P0 code and can
run alongside.

Two practical snags before it can start:

- **Arm L proposes local `qwen2.5:14b-instruct`, marked *tag unverified*.** This machine has
  `Qwen2.5-Coder:14b` — a coder variant, not the multilingual instruct model the protocol needs.
  Resolve the tag and pull it.
- **Arm R is ~145 hosted DeepSeek calls.** That is live model spend and an owner decision, not
  something to start unprompted.

---

## 7. What was verified, and what was not

**Verified against the tree:** the thirteen `from_u8` decode sites and their fatal `ok_or_else`;
`dec_granularity` dropping `extra`; `source()` ignoring unknown keys and silently dropping a
malformed `captured`/`observed`, with an in-code comment claiming the opposite; `Store::emit`'s
`_ => false` arm; `state_hash`'s coverage and `converged_with`'s dependence on it; `--config`
never read; `--seed-check` declared and unused; `find`/`pack` marked `Purity::Pure`; `--yes`
printing without committing; `SEMVER_BREAKING` empty.

**Not verified:** the behavioural probe *outputs* quoted in 2.1 §2.1 (they need the crates built
and run), H-18's 24.5 s → 0.55 s figure, H-10's server-side truncation, and the `make semver`
outcome. The RFC flags all of these *(unverified)* itself. Every static precondition for them is
confirmed, so they are plausible — but none is reproduced here.


---

## 8. The semver baseline, recorded

**Run 2026-10-02 on `dev/1.9.0` at `99a54fe`, `cargo-semver-checks 0.50.0`, `BASELINE = 1.8.0`.**

| | |
|---|---|
| crates checked | **12** — the full published set, against what is on crates.io |
| `no semver update required` | **12** |
| failures | **0** |
| changes requiring a new major | **0** |
| `make semver` exit | **0** |
| `SEMVER_BREAKING` | empty — no crate is exempted |

This is the clean starting line for TX-P0: the 1.9.0 tree is currently API-identical to published
1.8.0 in every way `cargo-semver-checks` can see, so any report that is not clean from here is
caused by TX-P0 itself and nothing earlier.

Two caveats on what the baseline does and does not prove:

- **`make semver` reports, it does not gate.** That is deliberate, and the Makefile records why:
  until 0.13 a breaking crate was skipped with a one-line SKIP, which left it with *nothing*
  watching it, so a second unintended break in the same crate rode along invisibly for the rest
  of the cycle. Running them all and printing the result makes the question answerable — *are
  these the breaks that were meant?* — rather than silently answered.
- **It sees the Rust API, not the wire.** None of the §2 items — opened enumerations, new record
  types, the digest, granularity key 5 — is visible to it. The wire guarantees are held by
  `make spec-tables`, the conformance corpus and the four-language fixtures, and A-13 adds
  obligations to all three. A clean `make semver` says nothing about §8.1 compatibility.


---

## 9. A-8.1 decided, and the probe that sized it

**Decision, 2026-10-02: Option B.** Open `ThreadSchema`, `Role`, `SourceKind` and
`DetectionKind`; reserve 255 in those four plus `admission`; **and** add the normative sentence
making the raw code identity-bearing for `source.kind`, refusing to encode a code-less
`Unknown`, with a conformance fixture asserting an unknown source kind leaves the uid unchanged.

### Why, in one fact

`v1.6.0`'s `SourceKind::from_u8` returns `None` for code 5. `1.7.0` added `Node = 5`. **A 1.6
reader cannot open any store containing `source.kind: node`** — it fails at decode rather than
degrading. This project has already shipped the break A-8.1 exists to prevent, silently, and on
the one enumeration that sits inside the uid. The risk is a recurrence, not a forecast.

### The probe, and what it measured

A scratch branch added `Unknown = 255` to all four enumerations and nothing else. Torn down
after measuring.

| question | answer |
|---|---|
| Does `#[non_exhaustive]` suppress `enum_variant_added`? | **Yes.** `smysl-core`: 223 checks, 223 pass, `no semver update required` |
| All twelve published crates? | **12 clean, 0 failures, 0 major, `make semver` exit 0** |
| Does the workspace still build? | **Yes**, `--workspace --all-features`, exit 0 |
| Exhaustive matches broken downstream | **Zero** |
| Exhaustive matches broken in-crate | **Six**, all in `smysl-core`: `as_str` ×4, `DetectionKind::code`, `ThreadSchema::roles` |

**What this changes about the plan.** The RFC sizes F-12 at 3–5 days and calls it "the highest
blast radius of the set". On measurement, the *type change* is a morning: four variants and six
match arms, with no downstream breakage and no semver impact. The 3–5 days is the plumbing —
carrying the raw code through decode and re-encode, `W409`, recipe hashing, render skipping
unknowns, and the four-language conformance fixture. That is a better-shaped estimate: the risky
part is not the API, it is identity preservation, which is exactly where the Option B sentence
is aimed.


---

## 10. A-10 item 2 decided

**Decision, 2026-10-02: Option A + D.** Reserve `lang:` as unit core key 9, require quoting for
payload use, ship `SMY-W432` in 1.9.0 — **and** at TX-P5, when A-1 activates, emit a `check`
diagnostic naming every unit whose uid changes, rather than letting the lint be the only warning.

### What was measured, not assumed

- `lang:` on a unit is **accepted today with zero diagnostics**, survives `fmt` unquoted, and is
  **inside the uid**: the same text gives `b3:27ww…` with it and `b3:mwpk…` without. A-1 yields a
  third uid for those same bytes.
- **The project's own corpus is unaffected.** All fifteen `lang:` occurrences under `fixtures/`
  are in `@doc` headers (view key 6), which is correct and untouched. Zero unit-level uses.
- **The mitigation is implementable.** `HObject` stores `Vec<(Spanned<String>, Spanned<HValue>)>`,
  so the key's span survives parsing and `lang:` is distinguishable from `"lang":` by a span
  check. The 2.3 review's doubt on this point was too pessimistic.

### Why A over the alternatives

A sigil spelling (`@lang:`) buys §8.1 cleanliness with a permanent wart — every other core key is
a bare word. Leaving `lang` in `x.text/v1` keeps compatibility but gives up validation, and an
unvalidated language tag is of little use to retrieval or to a five-language library. Both options
make the format permanently worse to avoid a migration one release wide.

### Why D is the part that matters

The defect in A as written is **silence**, not the break. A lint in 1.9.0 warns whoever upgrades
and reads the changelog; it does nothing for a corpus that sits untouched until TX-P5 and then
shifts under its owner. Both preimages are computable at activation, so the change can be
*reported* — which turns a silent identity drift into a migration.

### Conditions

1. **The spec fold states this as an out-of-band break.** The set's §0 claim that every amendment
   is a §8.1 addition or a §8.3 tightening is false here, and the §8.3 obligation — "a changelog
   entry loud enough that somebody with stored documents can check them" — applies.
2. **`W432` ships in 1.9.0.** The entire mitigation is that a release warned first.
3. **Zero in-corpus uses is reassurance, not permission.** It says nothing about corpora nobody
   can survey, which is the population a format specification exists for.
