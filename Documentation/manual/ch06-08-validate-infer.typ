#import "design.typ": *

#part(number: "III", title: "Validating While You Write")

#chapter(number: 6, title: "check in the Authoring Loop")

You have written a handful of units by hand, or ingested a handful and staged
them — either way, you now have a `.smy` file and a belief that it says what
you meant. `check` is how that belief stops being a belief. This chapter is
about the loop you actually run while a document is still moving: write a
little, check it, read what comes back, fix it, write more. The full ten-pass
pipeline — what each pass does, in what order, and why that order is fixed —
is Chapter 19. Here, `check` is a tool you reach for mid-sentence, not a
report you generate at the end.

#section("Why check is a step, and not a formality")

A parser has to be forgiving. `smysl`'s surface parser never hard-fails on a
malformed unit (§15.3) — it records what went wrong and keeps reading, because
stopping at the first bad span would throw away every well-formed unit after
it. That tolerance is exactly why parsing cannot be the place you learn
whether a document is *right*. It is the place you learn whether a document
is *readable*, which is a much weaker thing.

#callout(label: "Why")[
  `check` verifies *consistency*, never *correctness* — a distinction the
  source states plainly: it can tell you a body reaches for a unit it never
  declared, not whether the claim is true. A parser that kept going past a bad
  span, and a checker that only judged what the parser handed it, would
  between them let a document *look* fine and *be* broken — a claim resting
  on a typo'd id, a `measured` status with no source behind it, a status the
  document is not entitled to. `check` is the step where those become visible,
  every time, on demand, rather than discovered later by someone reading the
  prose and noticing something does not add up.
]

Running `check` is cheap and has no side effect — it reads a store and prints
a report. There is no reason to wait until a document feels finished to run
it once. The rest of this chapter is one document, checked after every
change, exactly the way you would work on it yourself.

#section("A document, checked three times")

#subsection("Round zero: a small, clean brief")

Start with two units — one piece of evidence and one claim resting on it —
the smallest thing worth calling a document.

```
@doc smysl/0.1 {
  id: v/pool-brief
  intent: incident-brief
  lang: en
  requires: ["smysl.kernel/0.1"]
}

@evidence e/pool-wait { status: measured, source: { kind: metric, ref: "pool.wait_ms", captured: 2026-07-09 } }
~ Pool acquisition wait rose from 2 ms to 310 ms over the incident window.

@claim c/pool-saturation { status: inferred, grounds: [e/pool-wait] }
~ The eu-west connection pool is saturated.
```

#screen(caption: "$ smysl check brief.smy")[
```
brief.smy: 3 records, 2 units, 0 diagnostic(s)
```
]

Clean, and cheaply so — this ran in the time it takes to read the summary
line. That summary line is worth reading closely: 3 *records* (the `@doc`
header counts) and 2 *units*, with 0 diagnostics. Every round below reports
the same three numbers, so the diagnostic count is the thing to watch move.

#subsection("Round one: a reference that does not resolve")

You keep drafting. The next claim is meant to ground on the pool-wait
evidence, but you mistype the id — an easy thing to do once a document has a
handful of units and you are typing from memory rather than looking each one
up:

```
@claim c/pool-saturation { status: inferred, grounds: [e/missing-evidence] }
~ The eu-west connection pool is saturated.
```

#screen(caption: "$ smysl check mistake1.smy")[
```
mistake1.smy: error: SMY-E060: unresolved reference `e/missing-evidence` (at 351..369)
mistake1.smy: error: SMY-E031: SMY-E031: derived/inferred with empty grounds (at 296..416)
```
]

Two diagnostics from one typo, and both are worth reading rather than just
fixing on reflex. `SMY-E060` is the direct hit: nothing in this document is
named `e/missing-evidence`. `SMY-E031` follows from it — with the only named
ground unresolved, the claim's `grounds` list is, as far as the checker can
tell, empty, and `inferred` requires at least one. Fix the id and both
diagnostics disappear together, because the second one was never really a
separate problem — it was the first one's consequence, reported so you would
not have to work that out yourself.

#subsection("Round two: a status the unit cannot back up")

Next round, you add a second piece of evidence quickly, meaning to fill in
the source in a moment, and then don't:

```
@evidence e/canary { status: measured }
~ The 4.2 canary ran the same pool configuration without the regression.
```

#screen(caption: "$ smysl check mistake2.smy")[
```
mistake2.smy: error: SMY-E032: SMY-E032: measured/cited without source (at 108..225)
```
]

This is a shape rule, not a content judgement: `measured` is the strongest
rung on the trust ladder, reserved for something an instrument recorded, and
the document's own grammar requires that claim to point at a `source` field
naming what recorded it. No source, no `measured` — the checker will not
take your word for it, which is the entire point of writing status as a
field instead of a tone of voice. Add the `source: { kind: metric, ref: ... }`
back and the diagnostic clears.

#subsection("Round three: a warning, not an error")

One more round. You add a supporting detail to the saturation claim, but
you're moving fast and the paragraph you type is three words long:

```
@claim c/pool-saturation { status: inferred, grounds: [e/pool-wait] }
~ The eu-west connection pool is saturated.

Saturated.
```

#screen(caption: "$ smysl check mistake3.smy")[
```
mistake3.smy: warning: SMY-W041: body is 3 tokens, under the default range 40..120 (at b3:rdmccg53fegdhoyvxzgwwm34gf)
mistake3.smy: 3 records, 2 units, 1 diagnostic(s)
```
]

Exit code `0`. This is the first diagnostic in this chapter that did not
stop anything: the summary line still prints, and the process still succeeds.
`SMY-W041` is a *granularity* remark (Chapter 19 covers the full pass) — the
default profile expects an `l1` body of 40 to 120 tokens, and three tokens is
under that, but nothing here is factually wrong. A three-word elaboration is
usable; it just is not doing the elaborating it claims to be there for. A
warning is the checker's way of saying "this is fine to ship, and also here
is something you might want to know."

#subsection("--strict: deciding who gets to ignore that")

Whether "fine to ship" is actually true depends entirely on who is asking.
Mid-draft, at your own keyboard, a three-word body is a note to yourself to
come back to — stopping the process over it would be the tool getting in the
way of the thing it exists to help with. In a pipeline that gates a merge or
a release, the same warning is exactly the kind of thing that should never
reach main unremarked. `--strict` is how you tell `check` which situation
you're in, and it works by promoting the failure threshold rather than adding
a new rule:

#screen(caption: "$ smysl check --strict mistake3.smy")[
```
mistake3.smy: warning: SMY-W041: body is 3 tokens, under the default range 40..120 (at b3:rdmccg53fegdhoyvxzgwwm34gf)
```
]

Exit code `3` this time — the same diagnostic, unchanged, but now it is fatal.
Nothing about the document changed between the two runs; only the threshold
did. `check`'s own source states the rule directly: it exits `3` on any
error-severity diagnostic, and `--strict` promotes warnings to that
threshold. That single flag is the difference between "a personal draft in
progress" and "a document a CI job is about to let through":

#dtable(
  (1fr, 1fr, 1fr),
  (
    ([Setting], [Threshold], [When]),
    ([default], [errors only], [drafting at your own keyboard — warnings are notes, not blockers]),
    ([`--strict`], [errors *and* warnings], [CI, a merge gate, anything unattended]),
  ),
)

#whatsnext[
  A document that `check` reports clean is trustworthy in the narrow sense
  `check` promises — internally consistent, shape-valid, nothing dangling —
  and that is now the fork. If the next thing you want is more content, an
  outside model can propose units at the boundary Chapter 7 sets up, and
  Chapter 8 shows exactly how that proposal gets reviewed before it counts.
  If the document is already complete and you want to *operate* on it —
  merge it with another author's work, diff two versions, trace a claim back
  to its grounds — that is Part V. If it is ready to leave your machine as a
  budgeted excerpt or a rendered document, that is Part V and Part VII. All
  three forks assume the same thing: that you ran `check` before taking them,
  because none of the commands on the other side of this fork re-derive
  consistency for you.
]

#recap((
  [`check` verifies consistency, not correctness — it catches a dangling
   reference or an unsupported status, never a false claim.],
  [Diagnostics are cheap to generate and safe to run constantly: check after
   every meaningful change, not just at the end.],
  [A `Warn`-severity diagnostic (like `SMY-W041`) reports and continues; an
   `Error`-severity one (`SMY-E031`, `SMY-E032`, `SMY-E060`, …) exits `3`.],
  [`--strict` promotes warnings to the error threshold — the same rule for
   your own drafting pass and for a CI gate, just a different setting.],
))

#part(number: "IV", title: "Infer and Enrich")

#chapter(number: 7, title: "The Model Boundary")

Everything in Part III happens on your machine, deterministically, from
files you already have. This chapter is about the three places that stops
being true.

#term("Model boundary")[
  The complete set of commands allowed to call an external model, and
  therefore the complete set of commands allowed to send anything off the
  machine. There are exactly three: `ingest`, `attest`, and `thread --refine`
  (the one flag on `thread` that touches a model — plain `thread` does not).
  Every other command in `smysl` is a pure function of its inputs: same
  bytes in, same bytes out, on any machine, forever.
]

#callout(label: "Why")[
  The source states the rule this chapter is built on directly: *only*
  `ingest`, `attest`, and `thread --refine` may depend on a model — every
  other command's purity is checked and asserted in the binary's own test
  suite, not left to documentation to promise. The reason to draw the
  boundary this narrowly, rather than letting convenience creep it wider, is
  that a determinism guarantee is worthless with exceptions. If `check` or
  `merge` were ever allowed a hidden network call or a hidden read of the
  wall clock, "run it twice, get the same bytes" would need a footnote every
  time, and a footnoted guarantee is not a guarantee a pipeline can build on.
  Concentrating every model dependency into three named commands means the
  other twenty-some are something you can audit once and stop worrying about.
]

You do not have to take that on faith. `smysl providers --tasks` reports,
without contacting anything, exactly which command sends what to where:

#screen(caption: "$ smysl providers --tasks")[
```
task                 provider       egress     command
content-ingest       ollama         local      ingest
relation-extraction  ollama         local      ingest
gist-rewrite         ollama         local      thread --refine
thread-refine        ollama         local      thread --refine
attest               ollama         local      attest
```
]

Five tasks, and the `command` column names exactly the three commands the
boundary permits — `ingest`, `attest`, `thread --refine` — nothing else.
`egress` reads `local` here because this environment's only configured
provider is a default Ollama entry at `127.0.0.1:11434` — `smysl` ships with
that as its out-of-the-box configuration precisely so a first run cannot
egress content nobody asked to send. Route a task to a hosted provider
instead — an Anthropic or OpenAI-shaped endpoint that is not a loopback
address — and the same column reports `LEAVES`, before anything is sent,
because `egress` here is read from configuration, not from a probe.
Chapter 10 covers configuring providers in depth; what matters here is that
the report is unconditionally safe to run — asking the question makes no
network call itself.

#subsection("--offline: a refusal decided before any socket opens")

`--offline` turns the "would leave the machine" column into a hard stop.
Locality is decided from the endpoint alone — `127.0.0.1`, `localhost`, and a
handful of loopback forms count as local; anything else does not — so
`--offline` is never a promise a config file makes about itself, it is a
fact read off the address. Against the default all-local setup, `--offline`
changes nothing, because nothing here is hosted to begin with:

#screen(caption: "$ smysl --offline ingest --dry-run --rung document note.txt")[
```
provider     ollama
egress       no - local
path         json-ast (default for small enforced ingest)
rung         document (ceiling cited)
input        139 bytes, 35 token(s)
```
]

To see the refusal fire, `content-ingest` has to actually route to a
non-loopback address. Routing it at a provider entry whose endpoint is not
`127.0.0.1` — this environment ships no compiled hosted mapper, so a
temporary config that reused the local mapper against a non-loopback host
was enough to make the endpoint genuinely non-local — reproduces the
condition `--offline` exists to catch:

#screen(caption: "$ smysl providers --tasks   (content-ingest routed to a non-local endpoint)")[
```
task                 provider       egress     command
content-ingest       remote         LEAVES     ingest
```
]

#screen(caption: "$ smysl --offline ingest --dry-run --rung document note.txt")[
```
smysl ingest: operation would leave the machine while --offline is set
```
]

Exit code `7`. Nothing was sent — not even the dry-run's own report ran,
because the refusal happens inside the provider lookup, before `ingest`
decides what it would have said. That ordering is the guarantee: `--offline`
is checked before any I/O is attempted, so the refusal cannot depend on
whether the network happens to be up, and it costs nothing to check.

#whatsnext[
  You now know which three commands can reach outside this machine, and how
  to prove it (`--tasks`) and forbid it (`--offline`) without trusting either
  claim. Chapter 8 walks through the first of the three in full: `ingest`,
  which is where a model gets to *propose* new units, and the whole chapter
  is about why "propose" is the right word and what has to happen before a
  proposal counts as part of your document.
]

#recap((
  [Exactly three commands may depend on a model: `ingest`, `attest`, and
   `thread --refine`. Everything else is a pure function of its inputs,
   asserted in the binary's own tests, not just documented.],
  [`smysl providers --tasks` reports, with no network call, which task
   routes to which provider and whether that provider is local or would
   leave the machine.],
  [A provider's locality is read from its endpoint, not declared by its
   configuration — a loopback address is a fact, not a promise.],
  [`--offline` refuses a hosted route before any socket opens, exit `7`;
   against an all-local configuration it is a no-op, because there is
   nothing hosted to refuse.],
))

#chapter(number: 8, title: "ingest: From Prose to Staged Units")

`ingest` is the command that turns prose, or any document you did not write
in `.smy` yourself, into units a model proposed on your behalf. It is also
the command every question in this manual's introduction was really about:
staged — why? What next? This chapter answers both, in order, with a real
attempt run against this environment's actual provider and a real failure
along the way, because the failure turns out to be exactly as instructive as
a success would have been.

#callout(label: "Why")[
  A model's output is a *proposal*, not a fact, until a human — or a later,
  accountable step — confirms it. This is why `ingest` is structurally unable
  to write into your real store: there is no code path from a model's answer
  to a committed unit that does not pass through a file you can open, read,
  edit, and either accept or throw away. The source states this as a rule,
  not a preference — model output *must not* enter the store directly — and
  the rest of this chapter is what that rule looks like from the outside.
]

#section("--dry-run: what would be sent, and to whom")

Before anything leaves the machine, `--dry-run` answers the only question
worth asking first: what would this call have done? It makes no request at
all — no socket, no DNS lookup — which is what makes it safe to run even
when you are unsure a provider is configured correctly, or unsure you want
to spend the call yet.

```
$ smysl ingest --dry-run --rung document note.txt
```

#screen(caption: "note.txt is a short paragraph about a pool-saturation incident")[
```
provider     ollama
egress       no - local
path         json-ast (default for small enforced ingest)
rung         document (ceiling cited)
input        139 bytes, 35 token(s)
```
]

Every line here is a decision `ingest` made and is willing to show you before
committing to it. `path` and `rung` are the two worth understanding well,
because both cap what the eventual units are allowed to claim.

#subsection("rung: what kind of source this is")

`--rung` tells `ingest` what *kind* of thing it is reading, and that answer
sets a hard ceiling on the status any resulting unit may carry — never a
suggestion, a cap the checker enforces at staging time regardless of how
confidently the model writes. Run the same file at each of the four rungs and
only the ceiling changes:

#screen(caption: "$ smysl ingest --dry-run --rung <R> note.txt, R in {document, model, web, computed}")[
```
rung         document (ceiling cited)
rung         model    (ceiling inferred)
rung         web      (ceiling cited)
rung         computed (ceiling derived)
```
]

#dtable(
  (1fr, 2fr, 1fr, 2fr),
  (
    ([Rung], [Origin], [Ceiling], [Notes]),
    ([`computed`], [A deterministic tool, calculation, or parser produced this], [`derived`], [never requires a source, but needs `grounds`]),
    ([`document`], [You supplied an existing document or dataset], [`cited`], [requires a `source`]),
    ([`web`], [Fetched content, gated], [`cited`], [requires a `source` *and* a captured timestamp]),
    ([`model`], [The model's own parametric knowledge — it is guessing], [`inferred`], [requires `grounds`, not a source]),
  ),
)

`ingest`'s own default is `document`, not `model` — deliberately. Ingesting a
file is transcribing something that already exists; asking a model to invent
content from what it already knows is a different, riskier act, and has to
be asked for explicitly with `--rung model`. Nothing here is advisory: a
model that writes `measured` anyway is downgraded unconditionally and told
so, via `SMY-E033`, because rule T's whole purpose is to stop a model's
confidence from laundering itself into a status only an instrument earns —
however the sentence is phrased. `ingest` *never* assigns `measured` at any
rung; that status is reserved for `op: Imported` records with a
machine-checkable source, and a model reasoning from its own priors is
never that.

#subsection("path: surface text, or a JSON shape")

`--path` picks the wire shape the model is asked to answer in. `auto` (the
default) reasons from the job: extracting a handful of relations or
rewriting one gist takes the `json-ast` path unconditionally, because a
schema the provider enforces is worth more than robustness for something
that small. Bulk content ingestion goes the other way by default — the
surface path — for a reason worth keeping in mind whenever a large document
is involved: a malformed unit in surface text is *recoverable*, since the
parser reports a span and moves on, while a truncated JSON object is not —
the closing brace is load-bearing for the whole answer. Past roughly 4
kilobytes of expected output, `ingest` takes the surface path automatically
for exactly that reason:

#screen(caption: "$ smysl ingest --dry-run --rung document bignote.txt   (a ~7 KB paragraph, repeated)")[
```
provider     ollama
egress       no - local
path         surface (output too large to risk truncation)
rung         document (ceiling cited)
input        7300 bytes, 1825 token(s)
```
]

`--path surface` or `--path json-ast` overrides the choice outright, and the
dry-run report always says which reason applied — `caller override`,
`a structured operation`, `the provider enforces no schema`,
`output too large to risk truncation`, or the plain default — so you are
never left guessing why a run took the path it took.

#section("A real attempt, and what actually happened")

`--dry-run` never contacts anything, which means it cannot tell you whether
the provider it named is actually *there*. Dropping `--dry-run` does:

#screen(caption: "$ smysl ingest --rung document note.txt")[
```
smysl ingest: warning: SMY-W304: span degraded to opaque prose after provider unreachable (at b3:olmqbgjpyhetxghtootiil6nmr)
smysl ingest: 1 chunk(s), 0 call(s), 1 unit(s), 1 degraded, 0 token(s)
1 unit(s) staged in ./.smysl/staged.smy; review, then `smysl merge --staged`
```
]

Exit code `10`. This environment has no Ollama server actually running at
`127.0.0.1:11434` — `smysl providers --probe` confirms the same thing
independently, reporting `ollama down no server at http://127.0.0.1:11434`,
exit `6` — so the call genuinely could not be made. This is the real,
unedited result of that failure, and it is worth reading carefully rather
than treating as a dead end, because what happened next is the reason rule I
exists.

#callout(label: "Why")[
  `ingest` *must always make progress.* An input the model never actually
  saw did not vanish and did not abort the run — it degraded to an opaque
  `prose` unit, carrying the original text verbatim in its body, tagged
  `SMY-W304`, and staged like anything else. Rule I's reasoning is blunt and
  correct: a corpus with some opaque units in it is usable — you, a later
  pass, or a better-behaved provider can come back to exactly that span later
  — and a failed ingest run is not usable at all. The same degrade path fires
  whether the *provider* was unreachable, as here, or whether the *model
  answered but never produced a repairable unit* after exhausting its repair
  budget (covered just below) — either way, the guarantee is the same: every
  path out of the ingest boundary produces units, never a bare failure.
]

Look at what actually landed in `.smysl/staged.smy`:

#screen(caption: "$ cat .smysl/staged.smy")[
```
@prose { status: speculative, ingest:unrepaired: true }
~ The eu-west connection pool looked saturated during the incident window

The eu-west connection pool looked saturated during the incident window. Wait times rose sharply after the 4.2 rollout reached that shard.
```
]

Nothing was lost — the input paragraph is right there, verbatim, as the
body — but nothing was *understood* either. `status: speculative` is the
honest ceiling for an opaque span: rule I degrades rather than fails, but it
never pretends a span it could not process is worth more than a guess. That
`ingest:unrepaired` marker is how a later pass, or you, can find every unit
that came through this way and decide what to do about it — split it by
hand, re-run ingest once a provider is actually reachable, or leave it as
searchable prose. This is a completely real result of a completely real
failure, produced by this exact binary in this exact environment, and it is
the truest possible answer to "what does staged output look like" — but it
is not what a *successful* ingest looks like, and that is worth seeing too.

#term("Staging")[
  `ingest` never writes to your real store. It writes to exactly one place —
  `.smysl/staged.smy`, relative to the project root — and that file is
  ordinary surface text: the same syntax you write by hand, readable in any
  editor, checkable with `smysl check` like any other `.smy` file, and
  editable before you commit to it. Nothing about staging is a special
  binary format standing between you and the model's output. The file *is*
  the review.
]

Because it was never reachable, that run could not demonstrate what a
*normal* multi-unit staged batch — several proposed units, mixed statuses,
maybe one rejected — reads like. So here is one, written by hand rather than
produced by a live call, to make that concrete. *This block is
hand-authored, not real model output* — a stand-in for what a successful
`ingest --rung model` against a working provider would plausibly stage, so
the shape of a review is visible even though this environment could not
produce it live:

```
@finding f/pool-root-cause { status: inferred, grounds: [c/pool-saturation] }
~ Pool saturation is the most likely cause of the eu-west latency regression.

@claim c/config-drift { status: speculative }
~ A configuration difference between shards may also be contributing.
```

This is what you actually review: two units, one resting on a real ground
already in your store (`inferred`, because that is what `--rung model`'s
ceiling permits and the unit has `grounds` to justify it) and one offered
with nothing behind it at all (`speculative` — the honest floor for a bare
guess). You would open this file the same way you'd open any draft, and you
have three ordinary options at this point, not a mysterious ritual:

- *Edit it.* Delete the `c/config-drift` line if the guess is not worth
  keeping, correct a `gist`, tighten a status you think the model was
  generous with. It is text; edit it like text.
- *Check it first.* `smysl check .smysl/staged.smy` runs the same pipeline
  Chapter 6 walked through, against the staged file directly — catching a
  shape problem before it ever touches your real store.
- *Merge it, or don't.* Confirmed units become part of your document only
  when you say so, which is the next section.

#section("The exit-10 contract")

Exit `10` is not an error in the ordinary sense — `ingest` did its job, the
units were staged, the file is right there on disk. It exists because a
*pipeline* — a script, a CI step, another program driving `smysl` — needs a
machine-checkable way to learn "a decision is waiting for a human" without
parsing prose off stdout. Three things can happen next, and all three are
first-class, not workarounds:

#dtable(
  (1.3fr, 2.6fr, 2.3fr),
  (
    ([Action], [What it does], [Why you'd choose it]),
    ([Do nothing yet], [Staged file sits at `.smysl/staged.smy`; exit stays `10`], [You want to read it, maybe in an editor, before deciding — the default, and the safest one]),
    ([`smysl merge --staged`], [Commits the staged records into a real store, via the ordinary merge join (Chapter 11)], [You've reviewed it — by eye, by `check`, or both — and it's ready to become part of the document]),
    ([`ingest --yes`], [Same staging happens, but `ingest` itself exits `0` — "staged and confirmed" — instead of `10`], [You've decided in advance that this class of ingest doesn't need a pause — a script that already trusts this recipe and provider]),
    ([`rm .smysl/staged.smy`], [Discards the batch outright; nothing was ever in your real store to undo], [The proposal isn't worth keeping — maybe the whole run degraded, maybe you changed your mind]),
  ),
)

One nuance worth being exact about: `--yes` changes `ingest`'s *exit code and
message*, not what happens to the file. The batch is still written to
`.smysl/staged.smy` either way — `--yes` only tells a calling script "don't
treat this run as a pause," because whoever is running it already decided,
ahead of time, that this recipe's output does not need a human in the loop
every time. It is pre-approval, not a shortcut around staging itself — rule S
is not something a flag can opt out of. And merging does not delete the
staged file for you afterward; it is ordinary practice to `rm` it once you're
confident it is committed, though re-merging it again would cost nothing
worse than redundancy — the join underneath is idempotent.

Confirming this end to end, against the earlier degraded unit and a small
existing store:

#screen(caption: "$ smysl check .smysl/staged.smy")[
```
.smysl/staged.smy: warning: SMY-W041: body is 35 tokens, under the default range 40..120 (at b3:wmixdny4fa7xm4j6pprsiy4r3r)
.smysl/staged.smy: 1 records, 1 units, 1 diagnostic(s)
```
]

#screen(caption: "$ smysl merge --staged brief.smy -o combined.cbor")[
```
smysl merge: committed 1 staged record(s)
```
]

#screen(caption: "$ smysl check combined.cbor")[
```
combined.cbor: warning: SMY-W041: body is 35 tokens, under the default range 40..120 (at b3:wmixdny4fa7xm4j6pprsiy4r3r)
combined.cbor: 4 records, 3 units, 1 diagnostic(s)
```
]

The staged unit is now indistinguishable from any other unit in the store —
same checks apply, same warning still shows because merging does not fix
content, it only decides what counts. That is rule S working exactly as
intended: the model proposed, the file made the proposal legible, and a
deliberate command — not a side effect of `ingest` itself — is what made it
real.

#section("--granularity, --repair, and what happens when repair runs out")

Two more flags shape what a successful call would have produced.
`--granularity` names the body-length profile (`fine`, `default`, `coarse`)
the model is asked to write to, and it becomes part of the ingest *recipe* —
a hash of everything that decided what the model was asked to do, which is
what later lets tooling tell two runs of "the same" ingest apart from two
runs that were never really comparable. `--repair` sets how many times
`ingest` will show the model its own mistake and ask again before giving up
on a span — `2` by default.

The repair loop only spends a turn on genuine errors. A span the model wrote
well is accepted immediately; a span with a real structural problem — a
reference to nothing, a body that reads as two assertions instead of one —
gets shown the diagnostic and one more chance, up to the repair budget. When
that budget is exhausted without a clean answer, the span does not fail the
run: it degrades to exactly the same kind of opaque `prose` unit an
unreachable provider produces, `SMY-W304`, verbatim body, `speculative`
ceiling. This environment could not reach a provider at all, so it could not
exercise the "N failed repair attempts" path specifically — the unreachable
path pre-empts it — but it is the same function in the source that both
failure modes call, and the source's own test suite exercises it directly:
an unrepairable span becomes an opaque `prose` unit whatever put it there,
never a failed run.

#whatsnext[
  A staged batch is just a store that happens to live at
  `.smysl/staged.smy` instead of wherever your real document lives — which
  is why every tool that works on a store already works on it. Before
  merging anything, run `smysl check .smysl/staged.smy` the way Chapter 6
  showed, on the batch alone, so you're reading the model's proposal against
  the same bar your own hand-written units have to clear. Once it's clean —
  or you've edited it until it is — `smysl merge --staged` is where it stops
  being a proposal, and Chapter 11 picks up exactly there: what a join
  actually does, how a contention is recorded rather than silently resolved,
  and what "merge" means when two independent batches disagree.
]

#recap((
  [`ingest` never writes to your real store; it writes readable, editable
   surface text to `.smysl/staged.smy` and exits `10` to say a decision is
   waiting.],
  [`--dry-run` reports the provider, the egress, the chosen path, the rung's
   ceiling, and the input size — and makes no network call at all.],
  [`--rung` caps what a proposed unit may claim (`computed`→`derived`,
   `document`/`web`→`cited`, `model`→`inferred`); a model claiming more is
   downgraded unconditionally and told so, never silently accepted.],
  [Rule I guarantees an ingest run always produces units: an unreachable
   provider, or a span that exhausts its repair budget, degrades to an
   opaque `prose` unit rather than failing the whole run.],
  [Exit `10` has three legitimate responses: leave it staged and look at it,
   `smysl merge --staged` once it's reviewed, or `rm .smysl/staged.smy` to
   discard it — `--yes` only changes which of the first two happens by
   default, not whether staging itself happened.],
))
