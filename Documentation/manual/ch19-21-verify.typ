#import "design.typ": *

#part(number: "VI", title: "Verification")

#chapter(number: 21, title: "`check`, Pass by Pass")

Chapter 8 used `check` the way you actually use it while drafting — a quick,
cheap loop, run after every change, read for whatever it turns up. This
chapter takes the same command apart. Ten passes are named in
the format's own specification (§17), each with its own diagnostic codes, its
own reasoning, and its own idea of what "wrong" means. Knowing which pass
caught something — and, just as often, which pass *cannot* catch
something — is what turns a diagnostic from a red line you clear into
evidence you can reason about.

#callout(label: "Why")[
  `check` never tells you a claim is *true*. It tells you the document is
  *internally consistent* — that a body cites what it names, that a status
  never outranks what it rests on, that nothing points at a uid the store
  does not have. Those are mechanical properties: they follow from the bytes
  alone, and a machine can decide them without understanding a word of the
  English inside a gist or a body. Whether the eu-west pool was *actually*
  saturated is not one of those properties, and no pass below decides it —
  that question belongs to a person, or to `attest` asking a model a bounded
  question and citing its answer as evidence rather than as fact (Chapter 11).
  The boundary is exact: a mechanical pass can verify a *relationship*
  between things already in the document; it cannot verify a *fact* about
  the world the document describes.
]

#section("Ten passes, and where each one actually runs")

`Pass` is a ten-member enum in `smysl-check`, numbered in the order the
pipeline runs them, and every diagnostic code in this manual traces back to
exactly one of them:

#dtable(
  (auto, auto, 1fr, auto),
  (
    ([No.], [Pass], [What it checks], [Runs inside `check`?]),
    ([1], [`codec`], [The CBOR is deterministically encoded — canonical key order, minimal-length integers, no indefinite-length items.], [No — enforced at read time, before the pipeline starts.]),
    ([2], [`integrity`], [Every reference resolves; no cycle in the support graph.], [Yes.]),
    ([3], [`shape`], [Field-level rules the constructor cannot see, chiefly the gist's token bound.], [Yes.]),
    ([4], [`closure`], [A body only reaches for what `deps`/`grounds` already declared (rule L, structural half).], [Yes.]),
    ([5], [`granularity`], [A body fits the view's admission and length range.], [Yes.]),
    ([6], [`epistemics`], [Rule M — status never outranks the weakest ground.], [Yes.]),
    ([7], [`trust`], [Rule T — status never exceeds the ceiling of its best attested rung.], [Yes.]),
    ([8], [`retraction`], [Retraction authority and orphaning.], [No — enforced by `merge`, not by `check`.]),
    ([9], [`extension`], [An extension never redefines a kernel rule; unknown schemas degrade, never silently.], [Yes.]),
    ([10], [`hashes`], [Recomputed uids match a stored index, entry for entry.], [No — belongs to `Store::verify_against`, surfaced by `reindex --verify` (Chapter 23).]),
  ),
)

That "runs inside `check`?" column is not a simplification — it is
`smysl-check`'s own test suite, asserted directly:
`Pass::IMPLEMENTED` names exactly seven of the ten, and a build that is asked
for one of the other three (`check --pass codec`, `--pass retraction`, or
`--pass hashes`) runs nothing and reports nothing, rather than pretending to
have checked it. The reasons the other three sit outside the pipeline are
different from each other and worth knowing individually:

- `codec` decides whether the bytes are even legal CBOR before a `Store`
  exists to run passes over — a codec defect is not a diagnostic in a
  report, it is the reason `check` never gets that far.
- `retraction`'s codes (`SMY-E050`, `SMY-W052`) are real and enforced today,
  just not by `check` — they fire inside `merge`'s own retraction-integrity
  step instead (Chapter 13). The registry reserved pass 8 for it; the
  implementation landed the *check* in a different command than the one
  this chapter is about.
- `hashes` is the store's business, not a graph-shape question — verifying
  a uid means recomputing a hash over payload bytes and comparing it to what
  a sidecar index recorded, which is what `reindex --verify` does.

Each subsection below builds the smallest real file that trips one pass, and
only that pass where isolation is possible at all, and shows the actual
diagnostic. Where a pass cannot be reached from hand-written surface text —
`trust` needs an attestation, and attestations have no surface syntax at all
(Appendix A's grammar has none; the corpus fixtures' own README says so in
as many words) — the example is built through the library instead and the
gap is named rather than papered over.

#subsection("Pass 1 — `codec`")

#callout(label: "Why")[
  Two encoders of the same document must produce the same bytes, or hashing
  becomes ambiguous and merge stops being commutative. `codec` is what makes
  that promise a checked fact rather than a convention: canonical CBOR fixes
  map-key order, forbids indefinite-length items, and requires the shortest
  legal integer encoding, so there is exactly one way to write any given
  record.
]

You will essentially never trigger this by hand — `fmt` and every command
that writes CBOR already emit canonical bytes. The realistic way to see it
is a store that was hand-edited at the byte level, or damaged in transit.
Take a one-unit CBOR record and swap two of its map entries out of order —
the same corruption a bit-flipping transport bug or a careless byte-level
patch could cause:

#screen(caption: "$ smysl check /tmp/e080.cbor   (map keys 0, 1, 6 reordered to 0, 6, 1)")[
```
smysl check: /tmp/e080.cbor: SMY-E080: map keys not strictly ascending at byte 12
```
]

Exit code 1, not 3 — this never reaches the check pipeline at all, so it is
not counted among "diagnostics found," it is the reason there was nothing
to check. `smysl fmt` cannot repair this either, because `fmt` itself has to
decode the file first. The only fix is to regenerate the store from a
surface file or a known-good log; a canonical encoder never produces this
shape of corruption in the first place, which is the actual guarantee this
pass exists to protect.

#subsection("Pass 2 — `integrity`")

#callout(label: "Why")[
  Content addressing means a unit's identity is a hash of its own content —
  nothing else in the store can ever accidentally collide with it, but
  nothing stops a `deps` or `grounds` list from *naming* a uid, or a label,
  that the store never defines. That single typo class — right shape, wrong
  target — is the most common mistake in hand-authored `.smy`, and it is
  this pass's entire job.
]

```
@definition d/p95 { status: cited, source: { kind: doc, ref: "sre-handbook#latency" } }
~ The 95th percentile of request latency over a one-minute window.

@claim c/regression { status: speculative, deps: [d/p951] }
~ p95 auth latency tripled after the 4.2 rollout.
```

That `d/p951` is one character off from the definition actually declared
just above it.

#screen(caption: "$ smysl check /tmp/e060.smy")[
```
/tmp/e060.smy: error: SMY-E060: unresolved reference `d/p951` (at 206..212)
```
]

The byte span points at the exact six characters that do not resolve, which
is why this pass costs nothing to act on: fix the typo, or add the missing
definition, and the diagnostic disappears. A dangling `b3:` uid literal
(rather than a label) is caught the same way, by the same code, once the
store actually exists as a graph — this instance happens to be caught even
earlier, while the surface parser is still resolving labels into uids, but
it is the identical check and the identical wire code.

#subsection("Pass 3 — `shape`")

#callout(label: "Why")[
  Most shape rules — an empty gist, a `detail` with no `body` above it, an
  authored `unfounded` — are things `UnitCore`'s own constructor already
  refuses, so a well-formed store cannot contain them; this pass is defence
  in depth for exactly that reason. One rule genuinely cannot live in the
  constructor: the gist's token bound is *relative to a granularity
  profile*, and the constructor has no access to one. That is the rule worth
  seeing fail for real.
]

```
@claim c/verbose { status: speculative }
~ The eu-west connection pool saturated during the rollout window and the resulting queueing on every downstream call is almost certainly what tripled p95 latency for the auth service that morning.
```

A gist that reads like a topic sentence for a paragraph, because that is
exactly what it is.

#screen(caption: "$ smysl check /tmp/e022.smy")[
```
/tmp/e022.smy: error: SMY-E022: gist is 49 tokens, default allows 30 (at b3:6cvjx2fjm5bo7ohskil4ty6cli) [try: move the detail into `body` and shorten the gist]
```
]

The suggestion is the fix, verbatim: move everything past the first clause
into `body`, and leave the gist as the one sentence a reader sees at the
document's L0 — the level a summary is built from without ever touching
this unit's `body` at all.

#subsection("Pass 4 — `closure`")

#callout(label: "Why")[
  Rule L says a body should be interpretable from its own declared support —
  the L0 of its `deps` and `grounds` — without the reader having to go
  fishing elsewhere in the store. `check` verifies the *structural* half of
  that: a body must not name a uid it never declared a relationship with.
  (The semantic half — does the body actually say what the gist claims — is
  `attest --what gist-coverage`'s job, because deciding that needs a model,
  not a graph walk.)
]

```
@evidence e/pool-wait { status: measured, source: { kind: metric, ref: "pool.wait_ms{shard=eu-west}" } }
~ Pool acquisition wait rose from 2 ms to 310 ms over the same window.

@claim c/regression { status: speculative }
~ p95 auth latency tripled after the 4.2 rollout.

As e/pool-wait shows, wait time climbing in lockstep with request latency across every
shard that took the rollout early is the strongest single signal pointing at the pool
rather than at the release itself, and it is the reason this claim treats saturation as
the leading explanation.
```

`c/regression`'s body leans on `e/pool-wait` by name, but the claim never
put it in `deps` or `grounds` — the two fields this pass actually reads.

#screen(caption: "$ smysl check /tmp/e020.smy")[
```
/tmp/e020.smy: error: SMY-E020: the body references b3:vnuj3l674thnkkflwjm7kzkjdu, which is neither a dep nor a ground (at b3:fbnkpl2b4lbuk2jnuylypgikbc) [try: add it to `deps` if it is a prerequisite, or `grounds` if it is support]
```
]

The fix is a one-line edit — add `grounds: [e/pool-wait]` to the claim's
header — and it is worth pausing on *why* the checker insists on it rather
than just reading the prose: a reference that is only in the prose is
invisible to `pack`'s closure expansion and `trace`'s walk. Declare it, and
both commands now know this claim's support includes the pool-wait
evidence; leave it only in English, and neither does.

#subsection("Pass 5 — `granularity`")

#callout(label: "Why")[
  Granularity constrains *production* — what a unit is allowed to say
  in one shot — not truth (D-5), and `SMY-E040` is decided *structurally*
  rather than by reading the prose: more than one paragraph, or a list of
  more than one item, is more than one assertion by its own layout, however
  short each part is. Deciding "multi-assertion" any other way would need a
  model, and an error resting on a guess would be worse than no error.
]

```
@claim c/two-things { status: speculative }
~ The eu-west connection pool saturated during the rollout window and the resulting queueing on every downstream call is almost certainly what tripled p95 latency for the auth service.

The connection pool saturated in eu-west, tripling p95 latency for auth.

The canary shard also regressed independently, on a completely unrelated code path.
```

Two assertions in one body — the pool story and the canary story — under
the `default` profile's `single-assertion` admission:

#screen(caption: "$ smysl check /tmp/e040.smy")[
```
/tmp/e040.smy: error: SMY-E022: gist is 46 tokens, default allows 30 (at b3:iekhua3u4l62a363hvd5gzh2r4) [try: move the detail into `body` and shorten the gist]
/tmp/e040.smy: error: SMY-E040: body has more than one paragraph under single-assertion admission (at b3:iekhua3u4l62a363hvd5gzh2r4) [try: split into one unit per assertion, or move to `coarse`]
```
]

Two passes fired on the same unit here (shape's gist bound and
granularity's admission rule), which is a reminder that `check` never
short-circuits — every requested pass runs over the whole store regardless
of what an earlier one found, because the repair loop wants every defect at
once, not one round per pass. The fix for `E040` specifically is either
structural (split into `c/pool-saturated` and `c/canary-regressed`, each
grounding the eventual finding) or a granularity change (`coarse` admits
several paragraphs as one topical unit) — which one is right is an authoring
decision the checker deliberately leaves to you.

#subsection("Pass 6 — `epistemics` (rule M)")

#callout(label: "Why")[
  This is the guarantee that makes hallucination laundering structurally
  impossible inside the graph: prose has no type for *measured* versus
  *guessed*, so a hedge is the first casualty of summarisation, and by the
  third retelling a speculation reads as a fact. Rule M caps that
  mechanically — a claim can never be stronger than the weakest thing it
  rests on — and the diagnostic names the weakest ground directly, because
  that is the actionable part and it is free to compute.
]

```
@hypothesis h/guess { status: speculative }
~ The connection pool is probably the cause, on general principle.

@claim c/promoted { status: derived, grounds: [h/guess] }
~ The connection pool caused the latency regression.
```

A guess, laundered into a `derived` claim by nothing more than restating it
more confidently:

#screen(caption: "$ smysl check /tmp/e030.smy")[
```
/tmp/e030.smy: error: SMY-E030: derived exceeds the cap of speculative set by its weakest ground b3:4p7cfgyytyimdbqyhq7hkuw2sb (at b3:3nqopz6ucqiyui24ladjfkqlmb) [try: weaken this unit to speculative, or strengthen b3:4p7cfgyytyimdbqyhq7hkuw2sb]
```
]

The suggestion offers both directions honestly: weaken `c/promoted` back to
`speculative` (it is still a perfectly good hypothesis, just not a claim),
or go find the measurement that would let `h/guess` itself become
`measured` or `cited` — at which point the exact same `derived` status on
`c/promoted` would be earned rather than laundered. Fixture `F6-adversarial`
in the corpus is built entirely around this rule — three separate laundering
attempts, one of them two hops deep — and its own `.expected` file names
`SMY-E030` as the only code that may appear; a clean run over `F6` would
mean rule M had stopped binding.

#subsection("Pass 7 — `trust` (rule T)")

#callout(label: "Why")[
  Rule M stops laundering *inside* the graph; rule T stops it *at entry*.
  However confidently a model phrases something from its own parametric
  knowledge, that knowledge is capped at `inferred` — never `derived`, and
  never `measured`, which only an instrument recording `op: imported` with a
  checkable source may claim. This is the rule that keeps a model's fluency
  from being mistaken for evidence.
]

This one cannot be triggered from a hand-written `.smy` file at all: an
attestation — the record that carries an agent, an op, and a rung — has no
surface syntax. It is stamped on automatically by whatever tool ingests or
authors a unit (`ingest`, `attest`, `thread --refine`), and the corpus
fixtures' own README says outright that rule T "cannot be exercised from
surface text" for exactly that reason. The honest way to see it is to build
the store through the library directly — the same three records `ingest`
would eventually write — and check the result:

```
// A store no surface file can express directly: a claim resting safely on
// measured evidence (rule M is fine here), then a model's own attestation
// asserting `derived` for it (rule T is not) — via smysl_core directly.
let evidence = UnitCoreBuilder::new(KernelType::Evidence, "seven days of traces", Status::Measured)
    .source(SourceRef::new(SourceKind::Metric, "grafana://board/12"))
    .build()?;
let claim = UnitCoreBuilder::new(KernelType::Claim, "p95 auth latency tripled", Status::Derived)
    .grounds([canonical_uid(&evidence)])
    .build()?;
let agent = AgentId::new("model:openai/gpt-4")?;
let attestation = Attestation::new(canonical_uid(&claim), agent.clone(), Op::Authored, Rung::Model, Hlc::zero(agent));
```

Encoded to CBOR and checked for real:

#screen(caption: "$ smysl check /tmp/trust-violation.cbor")[
```
/tmp/trust-violation.cbor: error: SMY-E033: derived exceeds the inferred ceiling of its best provenance (model rung) (at b3:dpg2vjstd3ah7hw6l66ho2asav) [try: weaken to inferred, or import it with op: imported and a checkable source]
```
]

Notice rule M does not also fire here — `claim`'s ground (`evidence`,
`measured`) is strong enough that `derived` is well within cap, so this is a
clean, single-pass isolation of rule T alone. The fix the diagnostic
suggests is the honest one: either the model's claim gets relabelled
`inferred` (which is all its own knowledge ever entitled it to), or a real
import — an instrument, a tool adapter, something that recorded `op:
imported` against a source a machine could check — has to be the one making
the `derived` claim instead.

#subsection("Pass 8 — `retraction`")

#callout(label: "Why")[
  `retracts` says a unit should no longer be believed, and anything
  resting entirely on a retracted unit is left with no support at all —
  `SMY-E050` is what catches a document that retracted something without
  noticing what it orphaned downstream.
]

The registry reserves pass 8 for this, and the diagnostic codes are real and
enforced — just not by `check`. Selecting it directly proves the gap rather
than hiding it:

#screen(caption: "$ smysl check --pass retraction /tmp/retraction-demo.smy")[
```
/tmp/retraction-demo.smy: 7 records, 3 units, 0 diagnostic(s)
```
]

Nothing fires, on a store that plainly should trip it:

```
@evidence e/old { status: measured, source: { kind: metric, ref: "pool.wait_ms" } }
~ Pool wait rose from 2ms to 310ms over the window.

@claim c/downstream { status: derived, grounds: [e/old] }
~ Pool saturation caused the regression.

@claim c/retractor { status: speculative }
~ The sensor feeding e/old was later found miscalibrated.

@rel c/retractor --retracts--> e/old
```

The actual enforcement lives one command over — retraction integrity runs
as the last step of `merge`'s own fold, and a single-input merge is enough
to trigger it for real:

#screen(caption: "$ smysl merge /tmp/retraction-demo.smy -o /tmp/retraction-demo.cbor")[
```
/tmp/retraction-demo.smy: error: SMY-E050: every ground of this unit has been retracted (at b3:tu45r2ged2aes2iinznh6zimlx) [try: retract this unit too, or re-ground it on something surviving]
```
]

And the gap is worth stating plainly, because it has a real consequence: the
CBOR `merge` just wrote still shows `c/downstream` as plain `derived`
resting on `e/old` — the *declared* status never changes, only the
*effective* one does, and effective status is computed on demand, not
stored. Run `check` on that merged store and it reports clean, because
none of the seven pipeline passes look at `retracts` edges at all:

#screen(caption: "$ smysl check /tmp/retraction-demo.cbor")[
```
/tmp/retraction-demo.cbor: 7 records, 3 units, 0 diagnostic(s)
```
]

So today, orphaning is something `merge` tells you about at the moment it
happens, printed to stderr, not something a later `check` on the same store
will ever rediscover — if you need to know whether a store you did not just
merge has an orphaned unit in it, re-running it through `merge` (even with
itself as the only input) is currently the only way to ask.

#subsection("Pass 9 — `extension`")

#callout(label: "Why")[
  Rule X is why a store written by someone who knew more than you stays
  readable: an extension may only *add* — a new unit type, a new relation
  kind — never redefine a kernel rule, and a reader without that extension
  degrades gracefully rather than refusing. This pass enforces both
  directions: `SMY-E012` if an extension tries to overwrite the kernel,
  `SMY-W013` if a relation kind shows up that nothing declared.
]

```
@claim c/a { status: speculative }
~ The pool saturated.

@claim c/b { status: speculative }
~ We rolled back the release.

@rel c/a --x.sre/mitigated-by--> c/b
```

`x.sre/mitigated-by` is shaped like a legal extension relation, but nothing
in this file declared it with a `SchemaDecl` — which, like an attestation,
has no surface syntax of its own, so an undeclared extension relation is the
realistic case:

#screen(caption: "$ smysl check /tmp/w013.smy")[
```
/tmp/w013.smy: warning: SMY-W013: relation kind `x.sre/mitigated-by` is undeclared; treated as elaborates
/tmp/w013.smy: 5 records, 2 units, 1 diagnostic(s)
```
]

Exit `0` — a warning never blocks a plain `check`, and the store stays
routable: an unknown kind degrades to the weakest kernel relation
(`elaborates`) rather than being dropped, so `pack` and `trace` still see an
edge there, just not the specific meaning `x.sre/mitigated-by` intended.

#subsection("Pass 10 — `hashes`")

#callout(label: "Why")[
  Every other pass reasons about the *graph* — what a unit says about
  itself and what it points at. This one asks a narrower, more mechanical
  question: does the content still hash to the uid a sidecar index says it
  should. That is a property of the store's bytes against its own cached
  index, not of the document's shape, which is why it lives in
  `Store::verify_against` rather than in the graph-shape pipeline.
]

`check --pass hashes` selects nothing, for the same reason `--pass
retraction` did — `Pass::Hashes.is_implemented()` is `false` in this build.
`SMY-E070` ("the log does not match the index") is a real, tested code, but
nothing in the current CLI sets `verify_hashes: true` when opening a store,
so it is not yet reachable from any flag. What *is* reachable, and produces
the practically equivalent result, is `reindex --verify` — a whole-index
byte comparison rather than a per-unit `E070` report, covered in full in
Chapter 23 alongside the rest of what `reindex` does. If your document
checks clean on all seven implemented passes, hash verification is the next
question worth asking, and it belongs to that chapter, not this one.

#section("Narrowing and raising the bar")

#subsection("`--pass` — run only the passes you name")

Ten passes over a large store is real work, and while debugging you often
know which one you care about. `--pass` restricts the run to exactly the
names you give it — pass in a store with two independent defects and narrow
to just one:

```
@claim c/two-things { status: speculative }
~ The eu-west connection pool saturated during the rollout window and the resulting queueing on every downstream call is almost certainly what tripled p95 latency for the auth service.

The connection pool saturated in eu-west, tripling p95 latency for auth.

The canary shard also regressed independently, on a completely unrelated code path.
```

#screen(caption: "$ smysl check --pass shape /tmp/two-defects.smy")[
```
/tmp/two-defects.smy: error: SMY-E022: gist is 46 tokens, default allows 30 (at b3:iekhua3u4l62a363hvd5gzh2r4) [try: move the detail into `body` and shorten the gist]
```
]

#screen(caption: "$ smysl check --pass granularity /tmp/two-defects.smy")[
```
/tmp/two-defects.smy: error: SMY-E040: body has more than one paragraph under single-assertion admission (at b3:iekhua3u4l62a363hvd5gzh2r4) [try: split into one unit per assertion, or move to `coarse`]
```
]

Two independent, single-code reports from the same file — exactly the
narrowing you want once you already know a category and just want it
isolated. One caveat worth knowing: `--pass` only narrows the seven
*pipeline* passes. A diagnostic caught while the surface parser resolves
labels — an unresolved reference, `SMY-E060` — happens before the pipeline
runs at all, so it is included in the report no matter which `--pass` you
name.

#subsection("`--strict` — warnings stop the build too")

A plain `check` treats a warning as informational: it is printed, but exit
stays `0`. `--strict` raises the bar to `Severity::Warn`, so the same
warning now fails:

#screen(caption: "$ smysl check /tmp/w013.smy")[
```
/tmp/w013.smy: warning: SMY-W013: relation kind `x.sre/mitigated-by` is undeclared; treated as elaborates
/tmp/w013.smy: 5 records, 2 units, 1 diagnostic(s)
```
]

#screen(caption: "$ smysl check --strict /tmp/w013.smy")[
```
/tmp/w013.smy: warning: SMY-W013: relation kind `x.sre/mitigated-by` is undeclared; treated as elaborates
```
]

Same single diagnostic, same text — but the second run's exit code is `3`
where the first's was `0`, and the summary line that prints on a passing
run is withheld on a failing one. `--strict` is the flag to reach for in a
pipeline that should refuse to move forward on anything short of clean, not
just on anything outright broken — a `w` code is not wrong, but it may be
exactly the threshold a particular team or a particular document's stakes
call for.

#whatsnext[
  A document that checks clean on all seven implemented passes is
  internally consistent — every reference resolves, every status is earned,
  every extension behaves. That is exactly what makes it safe to hand to
  `merge`, `pack`, or `retract` (Part V) in the first place — every one of
  those operations trusts that the graph it is walking is already this
  sound. But a clean report answers only "is this store safe to touch at
  all." It does not yet answer "is this store safe for the *kind* of use I
  have in mind" — reading only, versus producing new units into it, versus
  merging it with someone else's. Chapter 22 is that second question:
  conformance classes ask what a store is safe for; fidelity asks what one
  particular consumer, implementing one particular set of schemas, can
  actually get out of it.
]

#exercises((
  [`check` runs its passes in a fixed order, and this chapter walks them one
   at a time. Given that a parse failure makes every later pass meaningless,
   predict what `check` does when a file fails to parse partway through — does
   it report the parse error alone, or does it also report what it can see of
   the rest?],
  [Chapter 21 says pass 6 is rule M and pass 7 is rule T. Run `check` on
   `F6-adversarial.smy` (which trips M) and recall the `SMY-E033` example that
   trips T. Why is it useful that these are separate passes with separate
   codes, when both amount to "this status is too strong"?],
  [Write the shortest `.smy` file you can that produces *exactly one*
   diagnostic. Then try to write one that produces exactly two from a single
   mistake. Which was harder, and what does that tell you about reading
   `check` output?],
))

#answers((
  [It reports what it can. The parser is built to keep going past a bad span
   rather than stopping at the first fault, so you get the parse error *and*
   whatever the later passes could establish about the records that did admit.
   The alternative — one error per run — turns fixing a file into a sequence of
   round trips, and this chapter's whole argument is that a checker is a tool
   you use continuously rather than a gate you visit once.],
  [Because the two have different fixes and different meanings. `SMY-E030`
   (rule M) says the claim outran *what it rests on* — the answer is inside the
   document, in its grounds. `SMY-E033` (rule T) says the claim outran *who
   produced it* — the answer is outside the document, in the provenance, and no
   amount of editing grounds will help. Collapsing them into one code would
   leave you guessing which of the two situations you were in.],
  [One is easy: a single unit with `status: unfounded` does it. Two from one
   mistake is easier than it sounds — a broken label in a `grounds` list
   usually gets you an unresolved reference *and* a shape error on the unit
   that failed to admit, and Chapter 5 showed one mistake producing three.
   The lesson is that diagnostic *count* is not fault count. Fix the most
   structural one and re-run before reading the rest.],
))

#recap((
  [`check` verifies consistency, never truth — a mechanical pass reasons
   about relationships already in the document, not about the world.],
  [Ten passes are named in the registry; seven run inside `check` itself.
   `codec` runs at read time and aborts before the pipeline starts;
   `retraction` is enforced by `merge`; `hashes` belongs to
   `Store::verify_against`, reachable today through `reindex --verify`.],
  [Every implemented pass has a real, isolable failure mode: `SMY-E060`
   (integrity), `SMY-E022` (shape), `SMY-E020` (closure), `SMY-E040`
   (granularity), `SMY-E030` (epistemics, rule M), `SMY-E033` (trust, rule
   T), `SMY-W013`/`SMY-E012` (extension).],
  [Rule T's violations have no surface syntax to author by hand —
   attestations are stamped on by tooling, never typed — so seeing one for
   real means building the store through the library, exactly as `ingest`
   eventually will.],
  [`--pass` narrows to the pipeline passes you name, but parse-time
   diagnostics are always included; `--strict` promotes every warning to a
   build-stopping error.],
))

#chapter(number: 22, title: "Conformance and Fidelity")

Chapter 21 answered one question per pass: is this specific relationship in
the document consistent. This chapter asks two broader questions that sit on
top of a clean `check` run. Both start from the same seven-pass report, and
both are ways of reading it — neither adds a new pass of its own.

#term("Conformance class")[
  A named answer to "is this *store* safe for a given *kind* of use" —
  reading it at all, consuming it as input to a decision, producing new
  units into it, merging it with another store. Five classes exist, every
  one of them building on `C-Read`, and each is decided by asking which
  diagnostic codes a store's `check` report contains — a conformance class
  is a lens over a report you already have, not a new thing to compute.
]

#term("Fidelity")[
  A named answer to "what can *this specific consumer*, implementing
  *these particular schemas*, actually do with this store" — `Full` (every
  schema the store `requires` is implemented), `Degraded` (an extension is
  missing, but the kernel is not — payload preserved, interpretation lost),
  or `Refuse` (the kernel major itself is unsupported — the one case where
  silently degrading is forbidden).
]

#callout(label: "Why")[
  Conformance is a property of the *store*: "would any C-Consume-conforming
  implementation be able to act on this." Fidelity is a property of a
  *pairing*: "can this particular reader, who happens to implement
  `x.sre/incident` and nothing else, get full value from this particular
  store." A store can conform at every class and still degrade a given
  consumer — a store that only uses kernel types is `C-Full` for everyone,
  and a store built around an SRE extension is still perfectly conformant
  even though a kernel-only reader loses interpretation on every unit typed
  with it. The two questions are independent on purpose: conformance is
  about what the *format* guarantees; fidelity is about what *you*,
  specifically, implement.
]

#section("The five conformance classes, demonstrated")

#dtable(
  (auto, 1fr),
  (
    ([Class], [Safe for]),
    ([`C-Read`], [Parsing and verifying the store at all — the floor every other class builds on.]),
    ([`C-Consume`], [Reading it and acting on it as a decision input — adds rules M and R.]),
    ([`C-Produce`], [Authoring new units into it — adds the shape rules.]),
    ([`C-Merge`], [Merging it with another store — adds the lifecycle rules (retraction, orphaning).]),
    ([`C-Full`], [All of the above at once.]),
  ),
)

`--conformance` takes exactly these five spellings, case-insensitive — the
bare words `read` or `full` are not accepted, only the hyphenated `C-`
form:

#screen(caption: "$ smysl check --conformance full fixtures/corpus/F1-incident.smy")[
```
smysl check: unknown conformance class
```
]

#screen(caption: "$ smysl check --conformance C-Full fixtures/corpus/F1-incident.smy")[
```
fixtures/corpus/F1-incident.smy: C-Full: pass
fixtures/corpus/F1-incident.smy: 21 records, 8 units, 0 diagnostic(s)
```
]

A clean store — `F1-incident`, the corpus's baseline fixture — passes every
class there is, because there is nothing in its report for any class to
forbid. That is the uninteresting case; the useful one is a store that
passes a *lower* class while failing a *higher* one, because that is
exactly the situation conformance classes exist to describe. The corpus's
`F6-adversarial` fixture is built entirely around rule M violations
(`SMY-E030`, Chapter 21's pass 6), and `SMY-E030` is classified as
*epistemic* — which `C-Read` does not forbid, but every other class does:

#screen(caption: "$ smysl check --conformance C-Read fixtures/corpus/F6-adversarial.smy")[
```
fixtures/corpus/F6-adversarial.smy: error: SMY-E030: derived exceeds the cap of inferred set by its weakest ground b3:2hkacsatxuvcywj6f4w2lkzojx (at b3:hdn4uifpmzzopwuyajmhyf5xzu) [try: weaken this unit to inferred, or strengthen b3:2hkacsatxuvcywj6f4w2lkzojx]
fixtures/corpus/F6-adversarial.smy: C-Read: pass
```
]

#screen(caption: "$ smysl check --conformance C-Consume fixtures/corpus/F6-adversarial.smy")[
```
fixtures/corpus/F6-adversarial.smy: C-Consume: fail (SMY-E030)
```
]

Read this pair the way it is meant to be read: `F6` still *parses*, still
*hashes correctly*, still has no dangling reference — nothing structural is
wrong with it, so a naive reader that only cares about getting bytes back
out could open it all day. But no implementation that promises `C-Consume`
(enforcing rules M and R on your behalf) can honestly hand you a decision
built on this store, because the store itself is laundering a speculation
into a derived claim. `C-Produce`, `C-Merge`, and `C-Full` all inherit that
same failure, for the same reason — the constraint is on what the class
*promises about the store*, not on how careful any one reader happens to
be.

#section("Fidelity — `--as`, `Full`, `Degraded`, and the one `Refuse`")

`--as` names the schemas a particular consumer implements; the kernel is
always implied, so naming nothing at all means "a kernel-only reader."
Build a small store around one genuine extension type:

```
@doc smysl/0.1 {
  id: v/sre-demo
  intent: incident-brief
  requires: ["smysl.kernel/0.1", "x.sre/incident"]
  roots: [i/pool-outage]
}

@x.sre/incident i/pool-outage { status: speculative }
~ eu-west connection pool exhausted during the 09:00 rollout window.
```

A kernel-only reader — the default, and the honest baseline for "a consumer
that has not adopted the SRE extension yet":

#screen(caption: "$ smysl check --as smysl.kernel/0.1 /tmp/fidelity-demo.smy")[
```
/tmp/fidelity-demo.smy: as `smysl.kernel/0.1`: Degraded
/tmp/fidelity-demo.smy:   b3:bwmw4g2dyfmtsyw2vb7gdzamr3 degraded: x.sre/incident not implemented
/tmp/fidelity-demo.smy: warning: SMY-W010: schema x.sre/incident is not implemented by `smysl.kernel/0.1`; payload preserved, interpretation lost (at b3:bwmw4g2dyfmtsyw2vb7gdzamr3)
/tmp/fidelity-demo.smy: 3 records, 1 units, 1 diagnostic(s)
```
]

Nothing is lost or dropped — `Degraded` and rule X's own promise are the
same sentence twice: the payload survives untouched, this reader just can
not interpret it as an incident, only as an opaque, still-mergeable,
still-traceable unit. A reader that has adopted the same extension gets
`Full`, on the identical file:

#screen(caption: "$ smysl check --as x.sre/incident /tmp/fidelity-demo.smy")[
```
/tmp/fidelity-demo.smy: as `x.sre/incident`: Full
/tmp/fidelity-demo.smy: 3 records, 1 units, 0 diagnostic(s)
```
]

#callout(label: "The one case degrading is forbidden")[
  Rule X's negotiation is three-valued on purpose, and the third value is
  the whole point: a missing *extension* degrades, but a missing *kernel
  major* refuses, unconditionally, because a reader that does not implement
  the kernel a store was written against cannot safely interpret *any* of
  it — not even enough to know what it is safe to ignore. `smysl` enforces
  this at the earliest point it possibly can: the surface parser itself
  refuses to load a document declaring an unsupported kernel major, before
  `check` or `--as` ever run.
]

#screen(caption: "$ smysl check --as smysl.kernel/0.1 /tmp/refuse-demo.smy   (requires: [\"smysl.kernel/9\"])")[
```
smysl check: /tmp/refuse-demo.smy: SMY-E002: kernel schema smysl.kernel/9
```
]

Exit code `8` — `UnsupportedVersion`, distinct from a check failure, because
this is not "the document is inconsistent," it is "this build cannot safely
read this document at all." That refusal happens so early that `--as` never
gets a chance to print `Refuse` for a hand-authored file — but the verdict
exists in the library independently of the parser's own gate, and a store
that reaches `check` by another path (a merged CBOR log, say, carrying a
view whose `requires` names a kernel major none of its contributors
actually wrote against) can still surface it as a fidelity result rather
than a load failure:

#screen(caption: "$ smysl check --as smysl.kernel/0.1 /tmp/refuse-demo.cbor   (same requirement, built directly as a view record)")[
```
/tmp/refuse-demo.cbor: as `smysl.kernel/0.1`: Refuse
/tmp/refuse-demo.cbor: 1 records, 0 units, 0 diagnostic(s)
```
]

Two different guards, one rule: whichever point a mismatched kernel major is
discovered at — the parser's own refusal on the way in, or `--as`'s verdict
on a store that got further than that — nothing about it is ever silently
downgraded to "read what you can."

#section("`--granularity` — the distribution, not a verdict")

Mixed granularity in a merged store is legal by design (D-5): different
documents are written for different depths, and a merge does not force them
into agreement. `--granularity` reports what is actually there rather than
passing judgement on it. `F7-mixed-granularity` in the corpus is exactly
that — a real merge of the incident brief (`default` profile) and a
research trace (`fine` profile):

#screen(caption: "$ smysl check --granularity fixtures/corpus/F7-mixed-granularity.smy")[
```
fixtures/corpus/F7-mixed-granularity.smy: 1 view(s) at granularity default
fixtures/corpus/F7-mixed-granularity.smy: warning: SMY-W041: body is 30 tokens, under the default range 40..120 (at b3:s6etp4fjq3gkd7335lzdwjzx54)
fixtures/corpus/F7-mixed-granularity.smy: warning: SMY-W041: body is 26 tokens, under the default range 40..120 (at b3:tfhgiir2fegvfkvq2fplgtvgq7)
fixtures/corpus/F7-mixed-granularity.smy: 22 records, 9 units, 2 diagnostic(s)
```
]

The two `SMY-W041` warnings are ordinary pass-5 length advisories on
individual units — advisory, not fatal, exactly as Chapter 21 described —
and the granularity line above them is not a diagnostic at all, just a
count of how many views this store declares at each profile. A store with
several views at several profiles would print one line per profile, with
no ranking implied between them: this flag exists so a reader can see what
they are looking at, not to tell them they are looking at the wrong thing.

#whatsnext[
  You now know what a store is safe for — structurally (Chapter 21),
  categorically (conformance), and for one specific reader (fidelity). None
  of that has asked anything about the *tool* yet: whether the index
  `smysl` keeps beside a CBOR log actually describes the log it sits next
  to, and whether the same invocation really does produce the same bytes on
  a different machine. Chapter 23 turns to those two guarantees —
  properties of `smysl` itself, not of any document it happens to be
  reading.
]

#exercises((
  [Run `smysl check --conformance C-Consume fixtures/corpus/F1-incident.smy`,
   then again with `C-Full`. Both pass. Explain what you have and have not
   learned about the file from two passes.],
  [Run `smysl check --as x.sre/incident
   fixtures/corpus/F7-mixed-granularity.smy`. It reports fidelity `Full`, and
   *also* two `SMY-W041` granularity warnings. Both are true at once. What is
   each one telling you, and which would block a pipeline?],
  [Conformance is a property of a store; fidelity is a property of a pairing.
   Describe a store that conforms at every class and still gives a particular
   consumer almost nothing.],
))

#answers((
  [You have learned the store is consumable by any conforming implementation,
   at both the minimum and the full class — no exotic record shapes, no version
   the reader could not handle. You have learned nothing about whether the
   claims are true, whether the argument is any good, or whether *your*
   consumer will find it useful. Conformance answers "can this be processed",
   which is a lower bar than it sounds and a necessary one.],
  [Fidelity `Full` says a consumer implementing `x.sre/incident` gets everything
   the store has to offer — nothing in it needs a schema that consumer lacks.
   The `W041` warnings say two bodies are shorter than the document's own
   declared granularity range, which is a drafting observation about prose
   length. Neither blocks by default; under `--strict` the warnings would, and
   fidelity would not, because fidelity is a report rather than a diagnostic.],
  [A store using only kernel types conforms everywhere — every implementation
   understands `@claim` and `@evidence`. Hand it to a consumer that exists to
   process `x.sre/incident` extensions and it finds not one unit it was built
   for. Perfectly conformant, near-zero fidelity for that reader. This is why
   both numbers exist: conformance is about the store alone, fidelity only
   means anything once you name who is reading.],
))

#recap((
  [Conformance and fidelity are both readings of a report you already have,
   not new passes — conformance asks what the *store* is safe for;
   fidelity asks what *one consumer* gets out of it.],
  [Five classes build on `C-Read`; a store can pass a lower class while
   failing a higher one, and the exact codes blocking each class are
   printed, not just a pass/fail bit.],
  [`--as` reports `Full`, `Degraded`, or `Refuse`. Degrading preserves the
   payload and loses only interpretation; refusing — reserved for a
   kernel major mismatch — is enforced as early as the surface parser
   itself, before fidelity is even computed.],
  [`--granularity` reports a distribution across a merged store's views,
   never a verdict — mixed granularity is legal by design.],
))

#chapter(number: 23, title: "Determinism, `reindex`, and the Index")

Everything so far has verified properties of a *document*. This chapter
verifies properties of the *tool*: does the on-disk index next to a CBOR log
actually describe that log, and does running the same command twice really
produce the same bytes. Both questions matter for the same underlying
reason — a store is meant to survive being copied, interrupted, and rebuilt
from nothing but its own log, on any machine, indefinitely.

#callout(label: "Why")[
  A store's index is derived, disposable state: everything it contains can
  be recomputed from the append-only log alone. That split exists for two
  concrete reasons. First, corruption recovery — a log is append-only and
  content-addressed, so it can be replayed from scratch even if the index
  beside it is stale, truncated, or gone entirely; nothing about the log's
  own integrity depends on the index agreeing with it. Second, portability —
  handing someone your log is handing them everything; the index is a local
  performance cache, not a second copy of the truth, and it never needs to
  travel with the log for the recipient to reconstruct an identical one.
]

#section("`reindex` and `--verify`")

Build a real CBOR store the way any pipeline would — `merge` writing to a
file:

#screen(caption: "$ smysl merge fixtures/corpus/F1-incident.smy -o /tmp/incident.cbor")[
```
fixtures/corpus/F1-incident.smy: contention k/ccm3actwjjti65famnoe6mapo5d over b3:cvhirtgs2mpvli2ethhyeo32uf (2 positions, live-rebuttal)
```
]

`reindex` rebuilds the sidecar from the log alone and reports what it
found:

#screen(caption: "$ smysl reindex /tmp/incident.cbor")[
```
/tmp/incident.cbor: 21 records, 8 units, index 1759 bytes
```
]

That write lands beside the log, at `.smysl/index/incident.idx` — a name
derived from the log's own filename, never from anything you pass
explicitly. `--verify` does not overwrite it; it rebuilds a second copy in
memory and compares the two byte for byte:

#screen(caption: "$ smysl reindex --verify /tmp/incident.cbor")[
```
/tmp/incident.cbor: index matches a rebuild (1759 bytes)
```
]

That is the case you want to see in CI: the sidecar genuinely describes the
log it sits next to. The other case — the one worth constructing on
purpose, since it is the whole reason `--verify` exists — is a sidecar that
has drifted from its log. Flip a single byte in the `.idx` file directly,
the way a truncated copy, a disk error, or a careless hand edit might:

#screen(caption: "$ smysl reindex --verify /tmp/incident.cbor   (one byte flipped in the sidecar)")[
```
/tmp/incident.cbor: warning: SMY-W110: index does not describe this log; rebuilding
/tmp/incident.cbor: the sidecar does not match a rebuild from the log
```
]

Exit code `9` — `HashVerification`, the same code the registry reserves for
pass 10's hash-mismatch case. Two things happened in that one run, worth
separating: `Store::open_with` noticed on the way in that the sidecar's own
header no longer matches the log (`SMY-W110`, and it silently rebuilds an
in-memory index so the rest of the command can proceed at all), and then
`--verify`'s own comparison — a raw byte-for-byte diff of the sidecar file
against a fresh rebuild — reported the mismatch explicitly. The fix is
never to hand-edit the sidecar back; it is to run `smysl reindex` (without
`--verify`) and let it overwrite the stale file from the log, which is
always authoritative.

#callout(label: "SMY-E070 exists, but no flag reaches it yet")[
  `Store::verify_against` — the function that recomputes every unit's uid
  and compares it entry-by-entry against a sidecar, reporting `SMY-E070` for
  anything that does not match — is real, tested library code, exposed
  through `StoreOptions::strict()`. As of this build, no CLI flag sets
  `verify_hashes: true` when opening a store, so `SMY-E070` itself is not
  yet something you can trigger from the command line. What `reindex
  --verify` gives you instead — a whole-index byte comparison rather than a
  per-unit report — answers the same practical question ("has this sidecar
  drifted from its log") with a coarser diagnostic. If you need the
  per-unit version today, it is one `Store::open_with(path,
  StoreOptions::strict())` call away in the library.
]

#section("`--seed-check` and `cargo xtask determinism`")

The global `--seed-check` flag is documented as asserting that a specific
invocation is bit-reproducible (rule D) — the same guarantee, on demand,
against your own store. It is worth being exact about what this build
actually does with it: the flag parses on every subcommand (it is declared
`global(true)`), but nothing in `main.rs` reads its value — passing it
changes nothing about the output of any command today.

#screen(caption: "$ smysl --seed-check check fixtures/corpus/F1-incident.smy")[
```
fixtures/corpus/F1-incident.smy: 21 records, 8 units, 0 diagnostic(s)
```
]

Byte-for-byte identical to the same command without the flag. That is not a
bug to route around in this manual — it is the accurate state of the flag,
and the guarantee it is meant to name is enforced today by a different,
coarser mechanism: `cargo xtask determinism`, a CI gate rather than a
runtime flag. It runs every registered pure operation twice, under eight
environment permutations — two locales, two timezones, two hash seeds — and
asserts byte-identical output across all sixteen runs per operation:

#screen(caption: "$ cargo xtask determinism")[
```
xtask determinism (rule D)
  matrix: 8 permutations
  5 of 5 operations registered
  pack: identical across 16 runs
  salience: identical across 16 runs
  merge: identical across 16 runs
  derive_thread: identical across 16 runs
  render: identical across 16 runs
ok
```
]

All five of rule D's named operations are registered and passing in this
build. The permutation matrix targets the four things that quietly break
determinism in practice and nowhere else: locale-dependent collation and
case folding, timezone-dependent date formatting, and hash-seed-dependent
iteration order over an unordered collection — the class of bug that passes
every test on one machine and fails silently on another. `--seed-check`
names the same property `xtask determinism` enforces; today, the gate is
where rule D is actually checked, and the flag is the place that guarantee
is documented to eventually live.

#section("`cargo xtask check-purity`")

A build-time gate rather than anything `smysl` itself runs, and worth
knowing about even though no chapter of this manual will ever tell you to
type it against a document: it verifies that the library core stays
synchronous and network-free, which is a precondition for rule D rather
than rule D itself. A pure operation that could reach a socket or block on a
runtime would not be reliably reproducible even if its logic were —
timing, retries, and network state are exactly the kind of thing determinism
cannot promise anything about.

#screen(caption: "$ cargo xtask check-purity")[
```
xtask check-purity (rules A, B)
  dependency tree (--no-default-features): 25 crates, none forbidden
  pure crates: 6 checked
  source scan: 66 files, 7 symbols
ok
```
]

Two independent checks run, deliberately redundant with each other: a
`cargo tree` walk over the facade's `--no-default-features` build (and over
each pure crate on its own) asserting that no async runtime, HTTP client,
argument parser, or TUI library ever appears in the dependency graph; and a
source-level grep across every pure crate for the literal symbols that would
betray a socket or a runtime even if no crate dependency ever named one — a
thread spawned and a raw `TcpStream` opened by hand would pass the first
check and fail the second. Either alone is escapable; both together are the
actual guarantee.

#whatsnext[
  Verification is complete at this point, in the sense this Part set out to
  cover: a document's own consistency (Chapter 21), what it is safe for and
  for whom (Chapter 22), and the tool's guarantees about its own index and
  its own reproducibility (this chapter). Everything you have built so far —
  a checked, conformant, correctly-fidelity-reported graph — is still a
  graph: units, relations, a thread's ordered walk over a selection of them.
  Part VII turns that graph into something a person actually reads, starting
  with `render` and the profiles that shape it.
]

#exercises((
  [Run `smysl reindex` on a `.smy` surface file. It fails with `SMY-E004:
   malformed envelope at byte 0`. Now `merge -o store.cbor` that same file and
   reindex *that* — it reports the record count and an index size. Why does the
   command only make sense against one of the two forms?],
  [Run any pure command twice with `--seed-check` and confirm it exits `0`.
   Then say what `--seed-check` would have to observe to exit non-zero, and why
   a flag like this exists at all when CI already runs
   `cargo xtask determinism`.],
  [Delete a store's index entirely and reindex it. Nothing is lost. Now
   construct the argument for why the *log* could not be treated the same way
   — what property does the log have that the index does not?],
))

#answers((
  [An index is derived state over an append-only *log*, and a surface file is
   not a log — it is text that parses into records. There is no envelope to
   index, no byte offsets to record, nothing to be stale against. `reindex`
   rebuilds a cache beside a binary store; pointed at surface text it is being
   asked to cache something that has no persistent form to cache.],
  [It would have to find that running the same operation over the same bytes
   produced different output — a clock read, a hash-map iteration order leaking
   through, an uninitialised salt. The CI gate proves determinism for the
   *inputs the gate happens to use*; `--seed-check` lets you assert it for
   *your* input, in your pipeline, on the machine that will actually run it.
   A guarantee you can re-verify locally is worth more than one you have to
   take on trust from someone else's CI.],
  [Everything in the index is recomputable from the log, so losing it costs
   time and nothing else. The log is the only place the content exists — it is
   append-only and content-addressed, which is exactly what makes it
   authoritative: entries are never rewritten, and each one's identity is a
   hash of what it contains, so corruption is detectable rather than silent.
   Derived state can be thrown away because it can be re-derived; the log
   cannot, because there is nothing to re-derive it from.],
))

#recap((
  [An index is derived, disposable state, recoverable from the log alone —
   which is why corruption recovery and portability both depend on the log
   never needing its index to be trustworthy.],
  [`reindex` rebuilds the sidecar; `reindex --verify` compares the existing
   sidecar against a fresh rebuild without overwriting it, and a genuine
   mismatch exits `9` after first warning `SMY-W110` on the way in.],
  [`SMY-E070`, the per-unit hash-mismatch code, is real library code
   (`Store::verify_against` via `StoreOptions::strict()`) not yet wired to
   any CLI flag; `reindex --verify`'s whole-index comparison is what the
   command line actually gives you today.],
  [`--seed-check` is declared globally but not yet read by any command in
   this build; `cargo xtask determinism` is where rule D is actually
   enforced today, across all five registered pure operations and eight
   environment permutations.],
  [`cargo xtask check-purity` is a build-time, not a document-time,
   guarantee — a synchronous, network-free core is a precondition for
   determinism, checked two independent ways so that neither is
   escapable alone.],
))
