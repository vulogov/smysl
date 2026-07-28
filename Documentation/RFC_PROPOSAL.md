# RFC SMYSL-1 — proposed amendments

**Status:** proposal, awaiting application to the RFC.
**Covers:** SM-P0 through SM-P15 (2026-07-24 → 28).
**Format version affected:** `smysl/0.1`, kernel `smysl.kernel/0.1`.

The RFC states that it has zero open design decisions. Implementing it in full produced the
58 items below: places where the specification is silent, self-contradictory, or contradicted
outright by a live endpoint.

**Each one is already resolved in code and pinned by a test.** That is what makes this a
proposal rather than a bug list — the implementation had to choose something in order to
exist, and these are the choices it made. The RFC should either ratify them or overrule them,
and where it overrules, the code changes.

Ordered by RFC section so it can be worked through in one pass. Each entry states what the
RFC says, what the implementation does, and why.

Two markers:

- **[live]** — found by running code against a real endpoint, not by reading. These are the
  items a careful re-reading of the RFC could not have caught, and the ones most likely to
  recur in a second implementation.
- **Needs a decision** — where the implementation picked a side that the RFC could reasonably
  settle the other way, and the choice is visible to users.

Two items are *known contradictions of the RFC's stated text* rather than gaps, and are the
ones to read first: **§9.1 (item 4)** and **Appendix D (item 5)**.

---

## §1, §6 — the kernel and the worked example

**1. The §6 worked example is shape-invalid.** `c/auth-p95-regression` is `measured` with no
`source`, which §1.4 forbids. Pinned by a test; the corpus carries a corrected copy.

**2. `SchemaId` has three shapes, not two.** `requires` names `smysl.kernel/0.1`, so
`SchemaId::KernelSchema` exists beside `Kernel(KernelType)` and `Extension`.

**3. §11 writes the kernel schema as `smysl.kernel/MAJOR`** but every instance in the RFC is
`smysl.kernel/0.1`. `kernel_major()` parses `MAJOR[.MINOR]` and ignores the minor.

---

## §5.4, §9 — epistemics, merge, and rule M

**4. Rule M at ingest: weaken, do not reject.** *(SM-P14, settled 2026-07-28.)* §9.1 is read
in the code as mandating that a unit violating rule M "yields a diagnostic, not a stored
unit". Rejection turned out to be the wrong operational choice and the RFC should say so:

- Both outcomes satisfy rule M identically — a capped unit sits at or below its weakest
  ground by construction — so the guarantee never preferred rejection.
- Rejection **cascades**: units grounded on a rejected one dangle, and rule I's
  degrade-to-prose path is not on this route, so the text is simply lost.
- It is **irreversible** against the normal flow of the system, where the evidence that
  would have justified the claim arrives in a later merge.

`monotone::apply` now lowers the unit to what its grounds support, exactly as rule T caps an
over-claimed rung ceiling. **[live]** — the defect showed as an intermittent `SMY-E060` in the
provider gate, about one run in four.

**5. `SMY-W036` is an addition to Appendix D.** A weakening has to be reportable and Appendix
D has no code for one. The registry is now 50 codes; `diag::tests::registry_matches_appendix_d_size`
states the reason rather than merely counting higher.

**6. Weakening moves an identity.** Status is hashed into the uid, so lowering a unit changes
its uid and every unit grounded on it points at something that no longer exists. The RFC
does not discuss this consequence anywhere. The pass sweeps the batch topologically,
rewriting references as it goes; labels follow the remap.

**7. Capping `measured` can make a unit unbuildable.** `inferred` requires grounds and
`measured` requires a source, so a model claiming `measured` with a source and no grounds
cannot become `inferred`. `ceiling::attainable` walks down to the strongest status the
*shape* supports, floored at `speculative` and never `unfounded`. §9.3's "`inferred` —
typically `speculative`" anticipates this without saying it.

**8. `SMY-E061` is worded "cycle in deps"** but `grounds` belongs there too: rule M's check is
a topological pass over grounds, and a cycle leaves those units unverifiable. Both are fatal.

**9. Rule V2 is vacuous as literally specified.** §10 requires open contentions to be
surfaced, but §5.4's merge *reports* detections rather than recording them — a decision
needed for associativity — so a store almost never holds a `Contention` record. `ir::build`
runs `contention::detect` itself, unioned with recorded ones, deduped by id, under a fixed
clock.

---

## §7 — files and wire format

**10. Nested key tables are missing from Appendix B.** `SourceRef`, `GranularityProfile`,
`Hlc`, thread steps, `detected`, `dropped`/`degraded`/`optimality` are all wire format with
no documented keys. Defined in `crates/smysl-core/src/cbor/keys.rs`.

**11. Unknown map keys have no specified behaviour.** Every record struct gained
`extra: BTreeMap<u16, Vec<u8>>`, preserved verbatim and re-emitted in key order.

**12. Payload maps use text keys**, sorted by encoded bytes. §7.1 mandates integer keys, which
cannot hold author-chosen names. Kernel records are unaffected.

**13. Relation attestations have no Appendix B key.** They travel as separate records.

**14. Appendix B key 0 (`schema`) vs Appendix C `type`.** Settled 2026-07-24: key 0 carries the
type directly; there is no separate `type` key.

**15. §7.3 names `.smysl/usage.log` but not its format.** One JSON object per line,
append-only, unknown keys ignored on read. A corrupt line is skipped rather than fatal —
losing a row of cost accounting must not stop the work that generates it.

**16. Labels have no wire record.** `Unit.labels` and the index `labels` table exist, but no
record type carries a binding, so labels survive a parse and not a store round trip.
`CheckOptions.labels` is how a caller supplies them. **Needs a decision:** either a record
type, or an explicit statement that labels are document-scoped.

---

## Appendix A, §13 — surface syntax

**17. Appendix A's grammar rejects the RFC's own example.** `ext-type = "x." ident "/" ident`
cannot parse `x.sre/1` (§6). Relaxed to allow a leading digit and dots after the slash.

**18. The surface has no comment syntax.** HJSON headers take `#` and `//`, but a `#` line
*between records* lexes as body text and is absorbed into the preceding unit — it reports
`SMY-E001: stray Text outside a record`. Confirmed again while writing fixture F7.

**19. Attestations have no surface syntax.** `record = unit | relation | thread` cannot
express provenance, so rule T is unexercisable from a `.smy` file.

**20. Surface syntax has no place for `salience`, thread `ts`, or a view `id`.**
`ParseOutcome` carries `labels` and `salience`; `ts` and `id` have documented defaults.

**21. §13 nominates `deser-hjson`, which cannot work.** Headers need an untyped value model
(unknown keys → `payload`) and byte spans (for the repair loop). Parsed in-crate instead.

**22. Thread owners take `model`/`human`/`tool`.** `agent:` is not a valid `AgentId` kind and
fails with `SMY-E001`. Worth stating where §19 discusses ownership.

---

## §10, §20 — rendering

**23. Rule V1 has no operational definition.** §10 says a profile "MUST define a distinct
rendering for each epistemic status", but the example profile in §10 defines none — it only
sets `show.status: inline-marker`. Implemented as: the resolved marker per status must be
non-empty and pairwise distinct over `Status::ALL`, checked in `Profile::load`. So
`show: { status: none }` fails at load with `SMY-E210`.

**24. §20's connective table covers 6 of the 14 kernel relation kinds.** The other eight are
supplied in `connective.rs`. An *extension* kind deliberately has none: rule X treats an
unknown kind as `elaborates` for closure, which is not a licence to invent prose for it.

**25. §20 does not say how a connective joins its text.** "As a result, The pool saturated" is
not English. `Block::joined` lowers an ordinary leading capital and leaves anything else —
`IEEE`, `SLO`, `p99`, `eu-west` — exactly as written.

**26. `RenderMeta` in §20 carries only profile, suppression flag, and contention ids.**
Extended with `thread`, `schema` and `audience`, without which an artifact cannot say which
reading of which graph it is.

---

## §19 — threads

**27. §19's role-assignment comment has the rebuttal edge backwards.** The RFC writes
`// target of rebuts -> rebuttal`, which puts the *rebutted* claim in the rebuttal slot — the
opposite of what a reader needs. Every table matches `SourceOf(Rebuts)` instead.

**28. §19 gives no rule table, only role lists**, leaving two roles unreachable by
construction: `narrative` could only assign setup/complication/coda, and `analysis` could
never reach `next`. Fixed with `Position::Band(i, n)` and by mapping `decision → next` /
`claim → implication`.

**29. §19 does not say what a derived thread's `gist` is.** Built from the two
*heaviest-weighted* roles rather than the opening ones — an analysis summarised by its
definitions says nothing about what it found.

**30. Coherence repair can exceed a role's declared arity**, and must: an incoherent thread is
worse than a long one. Arity is asserted over *selected* steps only.

**31. §23 does not say what `thread --derive` emits.** Appendix G pipes it into `render`, which
needs the units as well as the thread record, so it emits the store followed by the derived
thread; `--only` gives the record alone. Default id is `t/derived-<schema>`, because threads
are keyed by (id, owner) and `t/brief` would collide with an authored one.

---

## §21 — the provider layer

**32. §21.1's trait and §21.3's registry cannot both hold.** The trait is written with
`async fn complete`; the registry stores `Box<dyn Provider>`. `async fn` in a trait is not
dyn-compatible. The trait is synchronous and the runtime lives behind it — which is also what
guarantee A3 promises callers. All five mappers confirm it.

**33. §21.2 says "each mapper is one file"** but `openai` and `deepseek` differ only in
endpoint, auth and structured mode. `openai_compat.rs` holds the shared shape; duplicating
200 lines to honour a file count would mean two places to fix.

**34. The `Provider` trait was uninhabitable from outside the crate.** `Completion`, `Usage`
and `Probe` are `#[non_exhaustive]` with no constructors, so no external crate could
implement the public trait. Added `Completion::new`/`enforced`, `Usage::reported`/`estimated`,
`Probe::reachable`.

**35. §21 has no configuration format.** Added as `.smysl/config.hjson` (§7.3 names the path
but not the contents). Locality is read off the endpoint rather than declared: a config that
could *claim* to be local would make `--offline` a promise the file makes to itself.

**36. §29 says keys are never stored but nothing enforces it.** `ProviderConfig` has no field
a key could go in, and a config containing `api_key`/`key`/`token`/`secret`/`password` is
refused at load rather than warned about.

**37. A status code is data, not an error. [live]** The HTTP wrapper returned `Err` for any
4xx, discarding the body — and Ollama answers a missing model with 404 *plus* the
explanation. Responsibility 5 of the §21.2 mapper contract cannot be met by a mapper that
cannot see the body. Only transport failures are `Err`.

**38. Backpressure is a class, not a status code. [live]** §21.4 specifies retry on rate
limiting; only 429 was retried. Gemini signals overload with **503** and Anthropic with
**529**, and both arrived as non-retryable. `http::is_backpressure` covers all three. 500 is
deliberately excluded — waiting does not fix a bug on the far side. The *status* must also
decide backpressure ahead of the vendor envelope, or a 503 that explains itself is classified
from its body and loses its retry.

**39. D-12's tokio runtime does almost nothing** while the HTTP client is blocking.
Implemented as specified anyway, because the observable contract (`std::sync::mpsc` +
`try_recv`) is what callers depend on.

---

## Appendix C — the JSON schema

**40. Appendix C cannot be an intersection of vendor dialects, because there is no
intersection. [live]** Written as the intersection of OpenAI strict and Gemini. A live Gemini
call falsified it: `responseSchema` is an OpenAPI 3.0 `Schema` proto, not a subset of draft
2020-12, and it has **no `additionalProperties` field at all** — while OpenAI strict
**requires** `additionalProperties: false`. No schema satisfies both.

Resolved by making translation a mapper responsibility (§21.2, resp. 2): Appendix C stays
conservative draft 2020-12, and `gemini::dialect()` translates on the way out via an
allow-list of the proto's fields. Dropped for Gemini: `$schema`, `additionalProperties`,
`if`/`then`, and the `allOf` left empty by their removal.

**Consequence to record in the RFC:** the conditional halves of rules M and T go unenforced at
Gemini's endpoint and are decided by `check` after conversion, costing a repair turn.

**41. The OpenAI mapper is still unverified against this.** Strict mode also requires every
key in `properties` to appear in `required`; Appendix C lists 3 of 11. It likely needs its
own translation — flagged in the mapper's header, not yet proven either way.

---

## §22 — the ingest boundary

**42. Rule I and single-assertion granularity contradicted each other. [live]** Rule I degrades
an unrepairable span to a `prose` unit carrying the raw span verbatim; a raw span is often
several paragraphs; `SMY-E040` forbids that under single-assertion admission. So a conformant
ingest could not also make progress. Resolved by exempting `prose` from E040 — requiring the
opaque-text type to be one assertion is requiring it not to be prose. *Found by a live
DeepSeek run.*

**43. Rule T's cap must not spend the repair budget.** `convert` applies the ceiling
unconditionally and emits `SMY-E033`, which is an error, so the repair loop bought a turn to
fix something already fixed — and a model confident enough to claim `measured` claims it
again, so the budget always exhausted and rule I degraded the whole chunk, discarding
correctly capped units. **[live]** — degraded 2 runs in 3 on Gemini. `needs_repair` now
exempts E033 alone. This contradicted the RFC's own "downgraded and told so".

**44. `SMY-E040` cannot be decided semantically without a model**, and it is an error. Decided
structurally instead: two paragraphs or a multi-item list. One paragraph never trips it,
however long.

**45. Thinking models break output accounting and budget sizing. [live]** Reasoning tokens are
billed as output but reported apart from it, so a ledger reading `candidatesTokenCount`
understates every call. They are also spent against `maxOutputTokens`, so a budget sized for
the answer can never finish. Measured: `gemini-3.5-flash` spent **1468 thought tokens for a
234-token answer** on a three-sentence input. §21.4 should say that an output budget covers
both halves.

**46. A model list is a catalogue, not an entitlement. [live]** `GET /v1beta/models` lists
`gemini-2.5-flash` and `-2.5-pro`; calling either returns "no longer available to new users",
on both free and paid keys. `providers --probe` reports availability from the list alone,
which overstates what it knows.

---

## §24, §26 — the terminal UI

**47. The pack simulator needs a pin control that §24 does not mention.** With nothing
focused, a budget of zero selects nothing and is *feasible* — the mandatory floor exists only
once a unit is forced in, because only then are its rebuttals mandatory. Without a pin the
pane can never reach `INFEASIBLE`, which is the state it exists to show. `f` pins the unit
under the cursor (C5).

**48. §24 and §26 are cited inconsistently in the code** for the same seven-pane UI
(`smysl-tui/src/lib.rs` says §24; the `ui` command said §26). One of them is wrong.

---

## §27.2 — the corpus

**49. The corpus specified F1–F8; only F1, F3 and F6 existed.** F2 and F5 were due in SM-P5,
F4 in SM-P11, F7 and F8 in SM-P6 — all phases marked done. All eight now exist.

**50. F7 and F8 cannot be shaped like the others.** F7's property is about *merging* two
stores, so its D-5 assertion merges the real F1 and F2 rather than reading one file. F8 ships
as **two files**, because a label collision is unwritable in one document: labels are
document-local, so two agents binding `c/cause` differently only collide once merged.

**51. The corpus README promised a `.cbor` per fixture.** None ever existed, and nothing
missed them, because the round-trip test derives the encoding in memory.

---

## §28 — the evaluation harness

**52. The chain shape is not specified.** Implemented as: a hop is one `pack` of what the
previous hop passed on; five hops; the smysl arm is a pure function of store and budget.

**53. The budget must be a fraction of the input, not an absolute.** An absolute budget large
enough for one fixture does not bind on a smaller one, and a chain whose budget never binds
reports E1 = 1.0 and E2 = 1.0 on every input — the exact output of a harness measuring
nothing, and indistinguishable from a triumph. A fraction binds by construction.

**54. E1 must be measured at the selected level of detail.** Counting gist-plus-body
regardless charges the chain for bodies the pack deliberately dropped to `L0`, and reports no
saving on a corpus where the saving *is* those bodies.

**55. E4 is taken at the worst hop, not the last.** Rule R holding at the end after an
intermediate hop orphaned a rebuttal would be rule R not holding.

**56. Claim survival cannot be asserted as a flat threshold.** Whether units must be dropped
is arithmetic: if every gist fits the budget, nothing needs dropping; if the gists overrun
it, units must go and no format prevents it.

**57. The prose baseline needs two things the RFC does not mention**, and without either it
measures nothing:

- **The baseline prose must carry its hedges in words.** A store keeps confidence in a field;
  prose has no fields. A renderer that drops the status hands the baseline a passage with no
  hedges at all, so every one is "lost" before the first hop and the experiment measures the
  renderer.
- **The judge must be controlled.** A model asked how confidently a passage states something
  may simply lean toward "measured", and a biased instrument is indistinguishable from a
  devastating result. The same judge reads the *unsummarised* prose first; a post-chain
  figure the control does not clear is not reportable.

**58. First two-arm result** (F1, 5 hops, `gemini-3.5-flash-lite`, 4 runs — a data point, not
a benchmark): control 8/8 claims and **0 of 8 hedges lost**; prose 7–8/8 claims and **3–5 of 8
hedges lost**; smysl 8/8 and 0. Compression is a wash at roughly half the tokens either way.
The difference is epistemic, not economic.

---

## Watch item, not yet a divergence

**`SchemaId` decoding is strict**, so a kernel type added in a later 0.x minor fails to decode
rather than degrading. Consistent with §2.1 declaring the type set closed — but if kernel
types may be added within 0.x, this must be revisited **before any store exists**, because it
is a decode-side compatibility break.
