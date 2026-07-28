#import "design.typ": *

#chapter(number: 9, title: "attest — Semantic Judgement Without Mutation")

Chapter 6 ran `check` over a store and got back a verdict on every pass — shape,
closure, granularity, trust. That verdict is mechanical: `check` looks at what a
unit's `deps` and `grounds` actually name, compares it against what the unit's
body and detail actually reach for, and reports where the two disagree. It never
reads the prose and asks whether it is *any good*.

That is not an oversight. It is the boundary this chapter is about.

#section("Why `check` cannot answer this")

Take a concrete case. A unit's gist reads:

```
gist: p95 auth latency tripled after the 4.2 rollout
```

and its body is three paragraphs about a canary that never regressed, a
connection pool whose wait time rose two orders of magnitude, and a rollout
that reached one shard a full day before the rest of the fleet. `check` will
pass this unit without hesitation — the gist is present, the body is present,
nothing in the body cites a `dep` or `ground` that is not declared. Every
mechanical rule is satisfied.

What `check` cannot tell you is whether that one-line gist actually
*represents* those three paragraphs, or whether it quietly drops the canary
evidence that complicates the story, or whether the body is really making two
separate claims wearing a single gist because the eu-west/rollout coincidence
and the pool-saturation theory are two different assertions that happened to
get merged. Those are judgements about meaning, not structure — and a
mechanical pass has no opinion about meaning. It cannot, because it never reads
for content, only for consistency.

#callout(label: "Why")[
  `check` verifies *consistency*: does a body cite what it names, does a
  status respect the trust ladder, does a unit fit its declared granularity
  shape as a matter of counting. Those are yes/no questions a parser can
  answer without understanding a single sentence.

  Whether a gist *actually summarises* its body, whether a warrant is a
  *plausible* reason for a claim to follow from its grounds, whether a unit
  is *too coarse* — those are judgement calls. Judgement calls need something
  that can read prose and reason about it: a model. `attest` is the command
  that asks one.

  And a model's judgement about a unit that already exists must never
  silently change that unit. If asking "is this gist honest?" could rewrite
  the gist, a single disputed opinion would permanently alter the record
  everyone else is working from — with no trace that anything happened, no
  way to disagree, no way to go back. So `attest` never touches the unit it
  judges. It writes its answer down as its own, separate record instead. That
  is the entire reason `attest` is a command of its own rather than a flag on
  `check`.
]

#term("Attestation")[
  One agent's assertion *about* a unit — never a change to it. An attestation
  names the uid it is about, the agent that made the judgement, an operation
  (`Op::Attested`), a rung (`Rung::Model` for anything a model decided), and a
  timestamp. It carries no content of its own beyond that. Attestations are
  not hashed and do not affect a unit's uid: they *accrete* — a unit's
  identity is fixed by its content, and the evidence for or against it grows
  as agents attest to it, without the unit itself ever moving.
]

#section("The three questions `attest` can ask")

`attest` does not ask open-ended questions. Every invocation asks exactly one
of three, chosen with `--what`, because a model given a vague brief gives a
vague answer, and a vague answer is not evidence. Each question is phrased in
the tool's own source as a single yes/no with a reason:

#dtable(
  (auto, 1fr, auto),
  (
    ([`--what`], [The question actually asked], [Needs a body]),
    ([`gist-coverage`], [Does the gist summarise the body faithfully — no claim
      in the gist the body does not support, and nothing central to the body
      that the gist omits?], [yes]),
    ([`warrant-plausibility`], [Is the stated warrant a plausible reason for
      the claim to follow from its grounds? Judging the connection, not
      whether the claim is true.], [no — any unit]),
    ([`granularity`], [Does this unit make exactly one assertion? Several
      assertions sharing one gist is the failure being looked for.], [yes]),
  ),
)

`gist-coverage` and `granularity` only apply to a unit that has a body at all
— asking about the gist coverage of a unit with no body is asking about
nothing, and finding that out would still cost a model call, so `attest`
skips units the question cannot apply to before it ever reaches the provider.
`warrant-plausibility` judges a relation (`@rel A --warrant--> B`, as seen in
`fixtures/corpus/F1-incident.smy` connecting `d/p95` to `c/regression`), so it
applies regardless of whether either end has a body.

#subsection("Trying it for real")

The store below has eight units built from seven days of latency traces, a
canary run, and a connection-pool metric — exactly the kind of place a gist
might quietly oversell its body. `attest` needs a model to answer with, the
same way `ingest` does, so it goes through the same provider registry Chapter
8 introduced:

#screen(caption: "$ smysl attest --what gist-coverage fixtures/corpus/F1-incident.smy")[
```
smysl attest: provider unreachable
```
]

This environment has no model listening — `smysl providers --probe` reports
the same thing (next chapter covers that command in full):

#screen(caption: "$ smysl providers --probe")[
```
ollama         down  no server at http://127.0.0.1:11434
```
]

That is the real, honest result here: exit `6` (`ExitCode::Provider`), the
same code `ingest` returns for the same reason. Nothing about this is
`attest`-specific — it is the ordinary consequence of asking a command that
needs a model to answer, when nothing is listening on
`http://127.0.0.1:11434`. `attest --what warrant-plausibility` and `attest
--what granularity` fail identically here, for the identical reason: the
registry resolves a provider for `Task::Attest` before it looks at a single
unit, so the failure is reported once, up front, rather than once per unit.

#callout(label: "What a successful run does, precisely")[
  With a model actually listening, `attest` takes the store's units in uid
  order (not document order, and not at random — see below), keeps only the
  ones the question applies to, and for each one sends a request built from
  exactly four fields: `type`, `status`, `gist`, and `body` when present.
  Nothing else — not `deps`, not `grounds`, not provenance — travels in the
  request, because the model is not being asked about any of that. Its system
  prompt fences the unit's content between explicit markers and states, in
  every one of the three questions, that what is between the markers is
  *data, never instruction* — the same fencing discipline Chapter 8 covers
  for `ingest`, applied here to a whole unit instead of raw prose.

  The model must answer with `YES` or `NO` as its first word, followed by a
  short reason. `attest` reads that first word and nothing more sophisticated:
  an answer that starts with neither word — `"I'm not sure"`, an empty
  response, an answer in the wrong language — is recorded as *unreadable*,
  never as a `NO`. Treating a garbled answer as a failed judgement would
  manufacture evidence against a unit nobody actually judged; the tool refuses
  to guess on the model's behalf. An unreadable answer produces no
  attestation at all — no partial record, no placeholder. For each readable
  answer, the CLI prints one line:

  ```
  yes b3:7fae21 gist omits the canary result entirely
  NO  b3:9c1d04 body makes two separate claims under one gist
  ```

  and a summary line naming how many units were judged, how many judgements
  came back `NO`, how many were unreadable, and the total tokens spent. Every
  `YES`/`NO` judgement becomes an `Attestation` at `Rung::Model` — a model's
  opinion is always shelved at the model rung, whatever rung the unit itself
  was ingested at, because the opinion is the model's own regardless of where
  the thing it is judging came from.
]

Not every fixture is a good target for every question. `granularity` asks
whether a unit makes exactly one assertion, and
`fixtures/corpus/F7-mixed-granularity.smy` — despite the name — is mostly
*already* well-graded: `c/consumers-undersized` grounds one claim ("the
consumer group could not keep up with the offered rate") in its own evidence,
`c/not-io-bound` grounds a separate claim in different evidence, and neither
tries to carry the other. If a model were reachable, asking `granularity`
about either would be expected to come back `YES` — a confirmation, not a
catch. The failure mode this question exists to catch is a *single* unit
whose gist reads like `c/consumers-undersized`'s but whose body quietly also
argues `c/not-io-bound`'s point underneath it, sharing one gist between two
claims that this fixture, correctly, keeps apart. That distinction — a fixture
that passes a check versus one that would fail it — only means something once
a model can actually answer; until then, the most honest statement about any
of the three questions here is the sample name and the fact that nothing
answered it.

Every exit code `attest` and `providers` can return traces back to the same
handful of causes:

#dtable(
  (auto, auto, 1fr),
  (
    ([Code], [Name], [When you see it here]),
    ([`0`], [success], [Judgements were made (some may be `NO`) — a `NO` is
      not a tool failure, it is the answer you asked for.]),
    ([`6`], [provider], [`ExitCode::Provider` — no provider could be
      reached, or one answered with an error. This is the code every example
      in this chapter actually returned.]),
    ([`7`], [offline], [`ExitCode::Offline` — `--offline` refused to fall
      back to a hosted provider. Chapter 10 covers exactly which providers
      that can happen to.]),
  ),
)

#whatsnext[
  You now know precisely what each of the three questions asks and what a
  real run costs when nothing answers. Before running `attest` against a
  store you actually care about, check what `smysl providers` reports is
  configured and reachable — Chapter 10 is that check in full, including how
  to see what would egress *before* you spend a call finding out the hard
  way.
]

#section("Sampling: a call per unit is not free")

Every unit `attest` judges is one model call. A store with a few thousand
units and no sampling would mean a few thousand calls before you learn
anything — that is real time and, for a hosted provider, real cost. `--sample`
exists so that "attest everything, every time" is the exception you opt into,
not the default you pay for by accident:

```
--sample 20      ask about 20 candidate units
--sample all     ask about every candidate unit — no cap
```

Left unset, `attest` asks about 10 units. That default is not arbitrary
caution dressed up as a number — it is small enough that running `attest`
after every `ingest` is cheap enough to actually do, and large enough that ten
independent judgements about the same kind of unit tell you something about
the batch, not just about one lucky or unlucky example.

Candidates are taken in *uid order*, never document order and never at
random. That matters the moment you run `attest` twice: a random sample would
ask about a different ten units on the second run, and the two reports would
not be comparable — you would not be able to tell whether a problem got fixed
or whether the sample simply moved. Uid order is deterministic and has
nothing to do with where a unit sits in the file, so the same `--sample 10`
against the same store asks about the same ten units every time, and a
before/after comparison of two `attest` runs is actually a comparison of the
same evidence.

#whatsnext[
  Sampling is why `attest` is affordable to run routinely rather than once at
  the end of a project. Chapter 10's usage ledger is where you would see the
  actual bill for that routine: `usage --by task` groups every call `attest`
  made separately from everything `ingest` made, so a rising `attest` line is
  visible on its own rather than buried in a single total.
]

#section("The non-mutation guarantee, made concrete")

A unit's uid is not assigned, stored, or looked up — it is *computed*, as a
BLAKE3 hash over the unit's canonical bytes (Chapter 2 covers the canonical
form in full). Two units with identical content always compute the same uid,
and a unit whose content changes by even one byte computes a different one.
That is what "content-addressed" means in this tool: identity *is* a
function of content, with nothing else in the loop.

Follow that one step further and the reason `attest` cannot touch a unit
becomes a fact about hashing, not a policy choice. If judging a unit's gist
coverage rewrote so much as its status field, the unit's bytes would change,
its uid would change with them, and every `dep`, every `ground`, every
`@rel` edge anywhere in the store that names the old uid would now point at
nothing. A single semantic opinion — one that might itself be wrong, one
that another attestation might later contradict — would have silently broken
every reference to the thing it was commenting on.

`attest` avoids that by construction, not by discipline: a `Judgement`
becomes an `Attestation`, and an `Attestation` names a uid, it is never
folded into the bytes a uid is computed from. The unit is read; it is never
opened for writing. The tool's own test suite asserts exactly this — compute
a unit's uid, run a granularity judgement against it that comes back `NO`,
then recompute the uid from the same unit and check it is bit-identical to
what it was before the judgement existed.

#callout(label: "Why this is not just a promise")[
  Because an attestation carries a uid rather than being merged into one, a
  judgement can be *wrong*. It can be *superseded* by a later attestation
  that disagrees. It can be *disputed* by a human who read the same unit and
  reached a different conclusion. None of that requires touching the claim
  under judgement, because the claim was never the thing that changed — the
  set of opinions about it grew, and growing a set of records is exactly what
  an append-only log is for. A `check` failure and an `attest` `NO` are
  answered differently for the same reason: `check` found something *wrong
  with the unit itself*, which means the unit needs fixing; `attest` found
  someone's *opinion about* the unit, which the record now holds alongside
  the unit, unresolved, until someone acts on it.
]

#whatsnext[
  A judgement that comes back `NO` — "this gist omits the canary result",
  "this unit makes two claims" — is not something `attest` fixes for you, and
  it is not supposed to be: fixing a gist or splitting a unit is authoring,
  and authoring belongs to a human or to `ingest`'s staged-and-confirmed path,
  never to a command whose entire contract is that it does not write units.
  Go back to Chapter 4 (writing units by hand) or Chapter 6 (`check` and
  repair) to act on what the judgement told you — rewrite the gist, split the
  unit, or record that you disagree and move on. `attest` has done its job
  once the judgement exists; deciding what to do about it is yours.
]

#recap((
  [`check` verifies consistency mechanically; `attest` asks a model a
    semantic question `check` cannot answer, which is why they are two
    commands rather than one command with a flag.],
  [Three questions, chosen with `--what`: `gist-coverage` (does the gist
    represent the body), `warrant-plausibility` (is the warrant a plausible
    connection), `granularity` (is this one assertion or several).],
  [`attest` needs a reachable provider exactly as `ingest` does; with nothing
    listening it fails once, up front, with `ExitCode::Provider` (6) — it
    does not try each unit before discovering the provider is down.],
  [`--sample N` (10 by default) bounds the calls spent; candidates are taken
    in uid order so two runs over the same store are directly comparable.],
  [An attestation is its own record: a unit's uid is a hash of its content,
    so mutating the unit to record a judgement would break every reference to
    it. Attestations accrete instead of overwriting.],
  [An unreadable model answer produces no attestation at all — never a
    manufactured `NO`.],
  [`attest` never fixes what it finds; acting on a judgement means going back
    to authoring — by hand or through `ingest`.],
))

#chapter(number: 10, title: "Providers and Usage")

`attest` and `ingest` both need to reach a model, and both go through the
same registry to do it. This chapter is that registry from the outside: what
it reports before you run anything, what it costs after you do, and where
both are recorded so neither is a surprise.

#section("What would leave this machine, and when to check")

#callout(label: "Why")[
  A model call to a hosted provider sends unit content somewhere outside this
  machine. Knowing *that will happen* before it happens is the entire point
  of `--offline` (Chapter 7) — and `--offline` is only as trustworthy as your
  own knowledge of what is routed where. `providers` is how you get that
  knowledge without spending a call to find it out: every mode below either
  reports configuration that is already known, or makes exactly one round
  trip whose purpose is to answer the question you asked and nothing else.
]

Plain `providers`, no flags, reports what is configured without contacting
anything:

#screen(caption: "$ smysl providers")[
```
ollama         ctx 8192     out 2048   json-schema  local
(--probe contacts them; --tasks reports what would egress)
```
]

This is the default that ships when no `.smysl/config.hjson` exists at all —
a single local Ollama entry, every task routed to it. That default is
deliberate: a first run that reached a hosted provider before anyone
configured one would mean content left the machine before anyone decided it
should.

`--probe` is the one mode that actually contacts every configured provider —
one round trip each, to find out what is really there rather than what the
config file merely claims:

#screen(caption: "$ smysl providers --probe")[
```
ollama         down  no server at http://127.0.0.1:11434
```
]

That is a live, real result from this machine: nothing is listening on
`127.0.0.1:11434`, so the probe reports `down` and the process exits `6`
(`ExitCode::Provider`) — the same failure `attest` hit in the previous
chapter, from the same cause, reported here without needing to run a whole
`attest` batch first to discover it. `--models` behaves the same way and for
the same reason: a provider's installed model list only exists on the far
end of a live connection, so asking for it probes exactly as `--probe` does,
whether or not you asked for `--probe` explicitly:

#screen(caption: "$ smysl providers --models")[
```
ollama         down  no server at http://127.0.0.1:11434
```
]

`--tasks` is different in kind, not degree: it never opens a socket. It
answers "which commands would send content off this machine, and to what",
purely by reading routing configuration:

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

Every task here reads `local`, because every task in this configuration
routes to the same loopback Ollama entry. A configuration with a hosted
fallback would show `LEAVES` in that column for whichever tasks route there
— the word chosen specifically to be impossible to skim past. Locality is
never declared by the config file; it is read directly off the endpoint (a
loopback host is local, anything else is not), so a config that wanted to
claim to be safe could not simply say so.

#subsection("`--offline` only refuses what is actually hosted")

`--offline` hard-fails a request rather than letting it fall back to a hosted
provider — but it only refuses providers that are not local to begin with.
Running it here changes nothing, because Ollama on loopback was never going
to leave the machine in the first place:

#screen(caption: "$ smysl providers --probe --offline")[
```
ollama         down  no server at http://127.0.0.1:11434
```
]

Same failure, same exit code — `--offline` adds nothing to check here because
there was nothing hosted to refuse. Against a configuration that *did* route
a task to a remote provider, `--tasks --offline` says so up front:

```
--offline: any task marked LEAVES will exit 7 rather than run
```

That is `ExitCode::Offline`, and it is decided from configuration alone,
before a socket is ever opened — refusing a call that would have been
harmless anyway is still the price of certainty that `--offline` means what
it says.

#callout(label: "Why fallback does not paper over a wrong key")[
  A `fallback` list (see the config file, next) exists for the case where a
  provider is simply down — the registry tries the next one rather than
  failing the whole call. But it only does that on `Unreachable`. An
  `Unauthorized` response, a context-window overrun, or a malformed request
  never triggers fallback: silently trying a different provider after an
  authorization failure would hand you an answer from a model you did not
  choose, and you would never learn that your key was wrong in the first
  place. Fallback covers outages; it deliberately does not cover
  misconfiguration, because the two failures need different reactions and
  collapsing them into one retry would hide which one you were having.
]

#whatsnext[
  `providers --tasks` is the check to run before `ingest` or `attest` against
  anything you have not routed yourself: it costs nothing, contacts nothing,
  and tells you exactly which of the two commands would leave the machine and
  through which provider. Run it once per project, not once per call.
]

#section("The provider configuration file")

Providers are described in `.smysl/config.hjson`, read by the same HJSON
reader the kernel itself uses for its own surface format — one configuration
syntax for the whole project rather than two. A configuration with two
providers, one local and one hosted, and routing that sends most work local
but relation extraction to the hosted one, looks like this:

```
{
  providers: {
    ollama: {
      kind: ollama
      endpoint: "http://127.0.0.1:11434"
      model: "llama3.2"
      context_window: 8192
      max_output: 2048
      structured: json-schema
    }
    remote: {
      kind: anthropic
      endpoint: "https://api.anthropic.com"
      model: "claude-sonnet-5"
      api_key_env: ANTHROPIC_API_KEY
      structured: tool-force
    }
  }
  routing: {
    content-ingest: ollama
    relation-extraction: remote
  }
  fallback: [ollama]
}
```

Every route and every fallback name must resolve to a provider actually
defined in the same file — a typo in a task name or a provider id is a hard
parse error, caught when the file is loaded, not discovered mid-run after a
model has already been asked to do something.

#term("Provider")[
  One configured endpoint: an id, which mapper drives it (`ollama`,
  `anthropic`, and so on), an endpoint URL, a model name, its context window
  and maximum output, and how it accepts structured output. Locality
  (`is_local`) is computed from the endpoint's host, never declared, so a
  config file cannot claim to be safe without actually being so.
]

Notice what is missing from the `remote` entry above: no key. `api_key_env`
names an *environment variable* that holds the key at call time, and
`api_key_cmd` — not shown here — would name a *command* that prints one.
`ProviderConfig` simply has no field a literal key could be written into,
which is a guarantee about the shape of the data rather than a rule asking
someone not to. A config file that does contain a field named `api_key`,
`key`, `token`, `secret`, or `password` is refused outright at load time,
with an error rather than a warning a hurried reader could scroll past:

```
provider remote has a `key` field; use api_key_env or api_key_cmd -
a config file must be safe to commit
```

#callout(label: "Why the indirection, not just a convention")[
  A config file is meant to sit in version control next to the rest of a
  project's settings. A literal key in it would be a credential in your
  git history the moment it was committed, recoverable long after the key
  was rotated. An environment variable name or a command to run for the key
  is not a secret at all — it is a pointer to wherever the secret actually
  lives (a shell profile, a keychain, a secrets manager), so the file itself
  stays exactly as safe to commit, read, and share as any other piece of
  project configuration.
]

#whatsnext[
  With providers and routing declared, `providers --tasks` (previous
  section) is how you confirm the file says what you think it says before
  trusting it with real content through `ingest` or `attest`.
]

#section("The usage ledger")

#term("Recipe")[
  A hash of the *full conditions* of one model call — provider, model,
  prompt template and its version, everything that could change the answer.
  Two calls with the same recipe are, as far as the tool can tell,
  interchangeable; two calls that differ in provider or model but ask
  logically the same question share the same underlying prompt and differ
  only in who answered.
]

#term("Ledger")[
  A local, append-only log of what was called: a timestamp, which provider
  and model answered, which task it was for, how many input and output
  tokens it cost, whether that count was estimated rather than reported, how
  many retries it took, and — when the caller has one — a recipe hash and a
  run identifier. One JSON object per line, in a fixed key order, so a diff
  over the ledger is a diff over what actually happened. *Never the prompt,
  never the completion text* — putting content in the ledger would create a
  second copy of everything the store already holds, outside the log's own
  integrity guarantees and outside `retract`.
]

An empty ledger is the normal starting state, and it reads back as one plain
sentence rather than an empty table:

#screen(caption: "$ smysl usage")[
```
./.smysl/usage.log: no model calls recorded
```
]

A missing ledger file is treated exactly like an empty one — reading it
never fails just because nothing has been recorded yet, since a ledger is
informational and losing the ability to read cost history must never itself
be an error.

#subsection("A real entry, and an honest caveat about attest")

To see a populated ledger without a reachable model, run `ingest` against a
scrap of prose in this same offline environment. `ingest` degrades a chunk to
opaque prose when the provider it needs cannot be reached rather than
failing the whole run — Chapter 8 covers that path in full — and it still
records the attempt:

#screen(caption: "$ echo \"The auth service saw a latency regression after the 4.2 release.\" | smysl ingest --yes")[
```
smysl ingest: warning: SMY-W304: span degraded to opaque prose after provider unreachable (at b3:gnpwyiyujyn6x32m6u36euxjqh)
smysl ingest: 1 chunk(s), 0 call(s), 1 unit(s), 1 degraded, 0 token(s)
1 unit(s) staged and confirmed
```
]

```
$ smysl usage
ollama                        1 call(s)          0 in          0 out
------------------------      1 call(s)          0 tokens
```

```
$ cat .smysl/usage.log
{"at":1785224107272,"provider":"ollama","model":"","task":"content-ingest",
 "in":0,"out":0,"estimated":false,"retries":0,
 "recipe":"b3:livso2vmtfwpz3qwrjvmn3ni32"}
```

The row shows one call, zero tokens, an empty model name: the entry records
that a call was *attempted* against `ollama` for `content-ingest`, not that a
model actually answered — no tokens were spent because no model ever
received the request. That is the ledger doing exactly what it promises:
counts, never content, and it errs toward recording an attempt rather than
silently discarding one.

Read this honestly rather than generalising it: in this build, `ingest` is
the command that writes the ledger from the CLI. `attest`'s own report
carries a token total too (the `N token(s)` in its final summary line from
Chapter 9), but that total is not currently appended to
`.smysl/usage.log` the way an `ingest` call's is. If you are reconciling
`usage` totals against a batch of `attest` runs, the ledger will
under-report them — the number to trust for `attest`'s own cost, today, is
the summary line `attest` prints for itself, not `usage --by task`.

#subsection("Grouping, filtering, and clearing")

#dtable(
  (auto, 1fr),
  (
    ([`--by`], [What it does]),
    ([`provider`], [Default. One row per configured provider — which one is
      actually taking your calls.]),
    ([`task`], [One row per task (`content-ingest`, `attest`, and so on) —
      which *kind* of work is costing you.]),
    ([`model`], [One row per model name, using the recipe's provider- and
      model-free identity where relevant so the same logical request
      aggregates across vendors rather than reading as unrelated calls.]),
    ([`run`], [One row per run identifier, when calls were tagged with one —
      groups everything one command invocation did.]),
  ),
)

```
$ smysl usage --by task
content-ingest                1 call(s)          0 in          0 out
------------------------      1 call(s)          0 tokens
```

`--since MS` restricts the ledger to calls at or after an epoch-millisecond
timestamp, for "what has this cost since I last checked." `--reset` discards
the whole ledger — every entry, and the file itself:

```
$ smysl usage --reset
./.smysl/usage.log: discarded 1 entr(ies)
```

A ledger is informational rather than authoritative — it never blocks a
command and never gates anything the way a staged `ingest` batch does. It
exists so cost is visible, not so it can be enforced; resetting it discards
history, never permission.

#whatsnext[
  You can now see, before a call, what would leave the machine and to whom
  (`providers`), and after a call, exactly what it cost and under what recipe
  (`usage`). That is the whole cost and egress discipline this manual asks of
  you — the rest of the book is about the documents themselves. Part V opens
  with Chapter 11, `merge`: the first command that actually changes what a
  store contains by joining two of them, now that you know what it costs to
  get material into one in the first place.
]

#recap((
  [`providers` (no flags) reports configuration without contacting anything;
    `--probe` and `--models` each make one real round trip per provider;
    `--tasks` reports what would egress by reading routing alone, never by
    dialing out.],
  [The default configuration — no `.smysl/config.hjson` at all — is a single
    local Ollama entry with every task routed to it, so a first run never
    egresses before anyone has decided it should.],
  [`--offline` refuses only providers that are not local; a loopback
    provider is unaffected by it, because there was never anything to
    refuse.],
  [`.smysl/config.hjson` never holds a literal key — only `api_key_env` or
    `api_key_cmd` — and a file that tries to hold one is refused at load, so
    a config file is always safe to commit.],
  [The usage ledger records counts, provider, model, task, recipe, and run —
    never prompt or completion content — one JSON line per call, append-only.],
  [In this build, only `ingest` writes to the ledger from the CLI; `attest`'s
    token total lives in its own run summary, not (yet) in `usage`.],
  [`usage --by provider|task|model|run`, `--since MS`, and `--reset` are how
    you read and clear that record; none of it ever blocks a command from
    running.],
))
