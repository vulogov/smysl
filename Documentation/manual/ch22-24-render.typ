#import "design.typ": *

#part(number: "VII", title: "Export for Human Consumption")

#chapter(number: 24, title: "render and Profiles")

#callout(label: "Why")[
  Every command before this chapter produces more graph: more units, a narrower
  selection, a named thread. None of it is prose yet. `render` is the one step
  where a graph plus a thread plus a set of rendering choices becomes text a
  specific person actually reads — and it is still pure. The same thread,
  rendered under the same profile, produces the same bytes every time, on every
  machine, forever. Nothing upstream of `render` cares who reads the output;
  `render` is where that finally matters, and where the tool has to decide how
  much to say, in what voice, and whether to let a disagreement stay visible.
]

#section("What render actually does")

`render` takes three things — a store, a thread inside it, and a profile — and
produces a fourth: an artifact in one of six formats. The store and the thread
say *what* the document claims. The profile says *how* it sounds: how formal,
how much apparatus per block, how deep by role, whether a status is a glyph or
a word. Changing the profile never changes what a claim says or what it rests
on; it only changes how loudly the surrounding apparatus talks about it.

Here is the whole pipeline, run for real against the incident brief from
Chapter 8's fixtures — `fixtures/corpus/F1-incident.smy`, which carries an
authored `t/brief` thread with three steps and one live rebuttal.

#screen(caption: "$ smysl render --thread t/brief --profile plain fixtures/corpus/F1-incident.smy")[```
# Auth p95 tripled in eu-west; pool saturation is leading but contested.

*brief · profile plain*

## bottom-line

≈ Pool saturation is the leading cause but is not consistent with the canary.

## support

≈ The eu-west connection pool is saturated.

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

## risk

⊢ On the other hand, the canary rules out a pure configuration cause.

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

---

**Open contentions:** k/ccm3actwjjti65famnoe6mapo5d
```]

Three things to notice before anything else. First, the title line is the
thread's own gist — `render` never invents a headline. Second, every claim
carries a marker (`≈`, `⊢`) that names its epistemic status; `≈` is
`inferred`, `⊢` is `derived`, and the two are never the same glyph. Third, the
contested block survived. The finding this thread leads with has a live
rebuttal on record, and it appears three times — once against the finding
itself and once against each of the two units the rebuttal names — because the
renderer attaches a contention note to every unit it touches, not only the one
it opens on. Nothing here smoothed the disagreement away, and nothing will,
short of asking for that explicitly (rule V2, a few pages on).

#term("Profile")[
  A named bundle of *rendering* choices, entirely separate from the document
  itself: register (how formal the prose reads), person, verbosity (how much
  apparatus travels with each block), an audience label, how much level-of-detail
  each thread role gets by default and per role, and how status, provenance, and
  contentions are surfaced. A profile never touches what a claim says, what it
  rests on, or what its status is — only how the surrounding text talks about
  those facts. `smysl` ships three built in: `plain`, `exec`, `analyst`. You can
  also write your own (Chapter 26) and pass its file path in place of a name.
]

#whatsnext[
  You've seen one profile render one thread. The rest of this chapter renders
  the *same* thread under all three built-ins side by side, so the difference
  a profile makes is visible rather than asserted — then works through the two
  rules (`V1`, `V2`) that keep a profile honest, `--lod`, and what happens when
  a store holds more than one thread under the name you asked for.
]

#section("The same thread, three voices")

Run `smysl render --profiles` to see the built-ins at a glance before rendering
any of them:

#screen(caption: "$ smysl render --profiles")[```
plain      neutral - · lod L1 · status marker
exec       formal  engineering leadership · lod L1 · status marker
analyst    neutral the person who has to check this · lod L2 · status word
```]

#dtable(
  (auto, 1fr, auto, auto, auto),
  (
    ([Profile], [Audience], [Verbosity], [Depth (`lod`)], [Status shown as]),
    ([`plain`], [none stated], [standard], [L1 for every role], [a marker — `≈`, `⊢`, …]),
    ([`exec`], [engineering leadership], [tight], [L1 default; `L0` for `bottom-line`, `risk`, `support`, `ask`], [a marker]),
    ([`analyst`], [the person who has to check this], [full], [L2 for every role], [the word — `[inferred]`, `[derived]`, …]),
  ),
)

Now the same store, the same thread, the same three blocks — rendered once per
profile. `exec` and `analyst` differ from `plain` in exactly the ways the
table above predicts, and in no others: none of the three changes which units
appear or what they say.

#screen(caption: "$ smysl render --thread t/brief --profile exec fixtures/corpus/F1-incident.smy")[```
# Auth p95 tripled in eu-west; pool saturation is leading but contested.

*brief · profile exec*
*for engineering leadership*

## bottom-line

≈ Pool saturation is the leading cause but is not consistent with the canary.

## support

≈ The eu-west connection pool is saturated.

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

## risk

⊢ On the other hand, the canary rules out a pure configuration cause.

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

---

**Open contentions:** k/ccm3actwjjti65famnoe6mapo5d
```]

#screen(caption: "$ smysl render --thread t/brief --profile analyst fixtures/corpus/F1-incident.smy")[```
# Auth p95 tripled in eu-west; pool saturation is leading but contested.

*brief · profile analyst*
*for the person who has to check this*

## bottom-line

[inferred] Pool saturation is the leading cause but is not consistent with the canary.

*rests on b3:wo4t2c46lq45fnakd6tajlgcac*

## support

[inferred] The eu-west connection pool is saturated.

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

*rests on b3:izyuzlt42mqcvgdfb4nfpllxyq*

## risk

[derived] On the other hand, the canary rules out a pure configuration cause.

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

*rests on b3:xkys7j42mcuyiaxiyh73xddimr*

---

**Open contentions:** k/ccm3actwjjti65famnoe6mapo5d
```]

Read the three side by side and every difference traces to the table. `exec`
prints an audience line the plain profile omits, because `exec` names one and
`plain` does not. `analyst` spells `[inferred]` and `[derived]` out instead of
using `≈`/`⊢`, because its `show.status` is `word` rather than
`inline-marker` — the same fact, a slower read. `analyst` also carries a
`*rests on …*` note under every block: that is `verbosity: full` surfacing
each unit's grounds that this three-step thread does not itself render, so a
checker can walk to them; `plain`'s `standard` verbosity and `exec`'s `tight`
both keep quiet about the same grounds — `tight` in particular budgets exactly
one note per block, which is why `exec`'s `support` block shows the
contention once where `plain`'s shows it twice. None of the three profiles
drops a block, changes a claim's wording, or renders a status two profiles
disagree about identically — that last property is not a coincidence; it is
rule V1, next.

Choosing between them is an audience decision, not a formatting one. Reach
for `plain` by default. Reach for `exec` when the reader is going to spend
four seconds on this and needs the headline and the risk, not the trail.
Reach for `analyst` when the reader's job is to check the brief rather than
act on it — someone who needs to see every status spelled out and know
exactly which unfetched grounds they'd have to pull to verify a claim.

#whatsnext[
  The differences above are all inside what a profile is *allowed* to vary.
  The next two sections are about what it is not: rule V1 says a profile may
  never make two statuses look the same, and rule V2 says a profile may hide a
  disagreement from the rendered text but never from the artifact's own
  record of itself.
]

#section("Rule V1: a profile cannot flatten status")

#term("Rule V1")[
  Every profile must render each of the six kernel statuses
  (`unfounded`, `speculative`, `inferred`, `derived`, `cited`, `measured`)
  distinguishably from every other one. This is checked once, at
  `Profile::load`, before any rendering happens — not per block, not at emit
  time. The practical effect: there is no `Profile` value in the entire
  program that flattens status, so no backend anywhere has to re-check it, and
  none can be talked into skipping the check. A profile that would flatten
  status never becomes a document; it becomes an error, and the error names
  which status collided.
]

You cannot construct this by accident from the CLI — the three built-ins all
pass — but you can construct it deliberately in a profile file, which is
exactly how you'd discover the rule if you tried to write a profile that
hides status to make a report look tidier:

#screen(caption: "$ cat /tmp/flat.profile")[```
profile flat {
  register: neutral, person: third, verbosity: standard
  show: { provenance: none, status: none, contentions: always }
}
```]

#screen(caption: "$ smysl render --thread t/brief --profile /tmp/flat.profile fixtures/corpus/F1-incident.smy")[```
smysl render: /tmp/flat.profile: SMY-E210: profile flat has no distinct rendering for unfounded
```]

Exit code `3` (check errors) — not `1`. `show.status: none` means every
status renders as an empty string, which is indistinguishable from every
other status's empty string; `enforce_v1` catches the first one it walks in
kernel order, `unfounded`, and refuses to produce a `Profile` at all. Nothing
downstream — no IR, no backend, no artifact — ever sees this profile, because
there is nothing downstream of a load that failed. The same rule fires if two
statuses happen to render to the same non-empty marker (`markers: {
speculative: "!", measured: "!" }` fails the same way, naming `measured` as
the one that collided, because it is checked second in kernel order) or if a
single status is blanked while the others are fine. There is exactly one way
past rule V1: give every status a distinct, non-empty rendering. `word` and
`inline-marker` both satisfy it by construction; only a profile that turns one
of them off, or hand-edits `markers`, can violate it.

#whatsnext[
  Rule V1 is about status never disappearing. Rule V2, next, is about
  disagreement — and it is enforced differently: not as a load-time refusal,
  but as a warning that still lets the render through, because suppressing a
  contention is sometimes a legitimate choice and flattening status never is.
]

#section("Rule V2 in practice: contentions")

#term("Rule V2")[
  A profile may choose not to *print* an open contention in the rendered
  text (`show.contentions: suppress`, or `--contentions suppress` on the
  command line) — but the artifact's own metadata must always say that a
  suppression happened and name which contention it was. The distinction
  matters: rule V1 stops a flattening profile from ever producing bytes at
  all, because there is no way to recover from lost status after the fact.
  Rule V2 lets the bytes through, because a reader who has the artifact in
  hand can always ask it whether something was hidden — the suppression is
  never silent, only optional to display.
]

Same thread, same profile, contentions shown (the default) and then
suppressed:

#screen(caption: "$ smysl render --thread t/brief --profile plain --contentions show fixtures/corpus/F1-incident.smy")[```
# Auth p95 tripled in eu-west; pool saturation is leading but contested.

*brief · profile plain*

## bottom-line

≈ Pool saturation is the leading cause but is not consistent with the canary.

## support

≈ The eu-west connection pool is saturated.

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

## risk

⊢ On the other hand, the canary rules out a pure configuration cause.

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

---

**Open contentions:** k/ccm3actwjjti65famnoe6mapo5d
```]

#screen(caption: "$ smysl render --thread t/brief --profile plain --contentions suppress fixtures/corpus/F1-incident.smy")[```
smysl render: SMY-W211: 1 open contention(s) suppressed by profile plain
# Auth p95 tripled in eu-west; pool saturation is leading but contested.

*brief · profile plain*

## bottom-line

≈ Pool saturation is the leading cause but is not consistent with the canary.

## support

≈ The eu-west connection pool is saturated.

## risk

⊢ On the other hand, the canary rules out a pure configuration cause.

---

<!-- SMY-W211: 1 open contention(s) suppressed by profile plain: k/ccm3actwjjti65famnoe6mapo5d -->
```]

The rendered body is genuinely quieter — no `> **contested**` lines anywhere
in it — but two things still say the suppression happened. `stderr` gets
`SMY-W211` the moment the render runs, and the Markdown artifact itself ends
with an HTML comment carrying the same code and the contention's id, invisible
when the file is previewed but present in the bytes on disk. This is not a
markdown-specific courtesy: every backend records `W211` and the contention id
in whatever form its output has room for (a `<meta>` tag in HTML, a `notes: []`-adjacent
field in JSON, a comment in Typst), and it is asserted once, across every
backend at once, in `crates/smysl-render/src/backend/mod.rs`
(`every_backend_records_suppression_in_its_output`) — a new backend cannot
quietly opt out of recording it. This is why suppression is a warning and not
a hard refusal: with `--strict`, the same command exits `3` instead of `0`,
so a pipeline that wants to treat "someone hid a disagreement" as a build
failure can do so on demand, without the tool ever hiding it from the one
place that would notice — its own metadata.

#section("--lod: overriding depth regardless of the profile")

#term("Level of detail (LOD)")[
  `L0` is a unit's one-line gist; `L1` adds its body paragraph if it has one;
  `L2` adds a further detail block if it has one. A profile sets a *default*
  LOD (and can override it per role — `exec` does, dropping `risk`, `support`,
  and `ask` to `L0`), but `--lod` on the command line caps every block at that
  level regardless of what the profile asked for. It is a cap, not a floor: if
  the profile already asks for less than the cap, the cap changes nothing.
]

`t/brief`'s three units carry only a gist — there is no body text to reveal —
so LOD is invisible on it. `t/story`, the postmortem narrative in
`fixtures/corpus/F3-narrative.smy`, has real body paragraphs, and shows the
cap doing something. Rendered under `plain` (default `L1`, so the body
already shows):

#screen(caption: "$ smysl render --thread t/story --profile plain fixtures/corpus/F3-narrative.smy   (excerpt)")[```
## setup

† A routine Thursday rollout of release 4.2 to the eu-west shard.

The 4.2 rollout began at 09:14 on a Thursday and reached the eu-west shard
forty minutes later. Nothing in the release notes touched the connection pool,
and the canary had been clean for six hours, so the on-call engineer approved
the promotion without waiting for the extended soak. ...
```]

Cap the same render at `L0` and the bodies vanish, leaving only the gist per
step — the cap wins over the profile's own `L1` default, which is the whole
point of a cap that applies "regardless of what the profile says":

#screen(caption: "$ smysl render --thread t/story --profile plain --lod L0 fixtures/corpus/F3-narrative.smy   (excerpt)")[```
## setup

† A routine Thursday rollout of release 4.2 to the eu-west shard.
[^1]

## complication

† Next, latency alarms fired ninety minutes later, and nobody believed them.
[^2]
```]

Reach for `--lod` when you want one render at a fixed depth without writing a
whole new profile just to change one number — a quick "give me the headlines
only" pass over a thread you'd normally read at `L1` or `L2`.

#section("Disambiguating which thread")

Threads are keyed by `(id, owner)`, not by id alone — a store can legitimately
hold two threads named `t/brief` if two different agents each derived or
authored their own reading of the same graph under that name. `render
--thread t/brief` with nothing else is then a guess between them, and this
tool does not guess between two agents' readings of the same graph — that
is exactly the kind of flattening the whole format exists to prevent. It
refuses instead.

Construct the situation for real: derive a second `t/brief` from the same
`F1-incident` store, owned by a different agent (`thread --derive`, covered in
full in Chapter 20, always defaults its derived thread's id to
`t/derived-<schema>` specifically so it never collides with an authored one by
accident — colliding here is deliberate, via an explicit `--id`):

#screen(caption: "$ smysl thread --derive brief --id t/brief --as human:priya --format surface fixtures/corpus/F1-incident.smy > /tmp/f1-two-briefs.smy")[```
@thread t/brief { schema: brief, owner: human:vladimir, ts: [0, 0] }
~ Auth p95 tripled in eu-west; pool saturation is leading but contested.
  bottom-line → f/root-cause
  support → c/pool-saturation
  risk → c/canary-clean

@thread t/brief { schema: brief, owner: human:priya, ts: [0, 0] }
~ Pool saturation is the leading cause but is not consistent with the canary.; The canary rules out a pure configuration…
  bottom-line → f/root-cause
  support → d/p95
  support → c/regression
  support → c/pool-saturation
  risk → c/canary-clean
```]

The store now holds two `t/brief` threads with different owners and different
steps. Ask for `t/brief` without saying whose, and `render` refuses rather than
picking one:

#screen(caption: "$ smysl render --thread t/brief --profile plain /tmp/f1-two-briefs.smy")[```
smysl render: /tmp/f1-two-briefs.smy holds 2 matching threads; narrow with --thread or --as
smysl render:   t/brief (human:priya)
smysl render:   t/brief (human:vladimir)
exit 2
```]

`--as` breaks the tie:

#screen(caption: "$ smysl render --thread t/brief --as human:priya --profile plain /tmp/f1-two-briefs.smy   (excerpt)")[```
# Pool saturation is the leading cause but is not consistent with the canary.; The canary rules out a pure configuration…

*brief · profile plain*

## bottom-line

≈ Pool saturation is the leading cause but is not consistent with the canary.

## support

† The 95th percentile of request latency over a one-minute window.
[^1]
```]

Exit `2` is `usage`, not `failure` — the store and profile were both fine; the
command line itself did not say enough to pick one thread over the other. The
fix is always the same shape: add `--as <owner>` (or a more specific
`--thread`, if two different ids happen to share a prefix you were matching
loosely). This is the same category of refusal as rule V1 — a decision the
tool will not make silently on your behalf — expressed as a `clap` usage
error instead of a load-time one, because the ambiguity lives in which thread
you meant, not in what a profile is allowed to render.

#whatsnext[
  You can now render one thread three ways, override its depth, control
  whether a disagreement shows in the text, and pick the right thread out of a
  store that holds more than one. The other axis of `render` is *format*
  rather than *voice* — Chapter 25 runs the same thread through every target
  this build can produce, from a PR-ready Markdown file to a JSON document
  meant for another program to consume.
]

#exercises((
  [Render `F1-incident.smy` under `--profile plain`, then under
   `--profile analyst`. The same finding comes out as `≈ Pool saturation…` in
   one and `[inferred] Pool saturation…` in the other, and only the second
   prints what the claim rests on. Neither is more *correct*. Say which you
   would put in front of an executive and which in front of an auditor, and
   defend the choice.],
  [Run `render --profile plain --contentions suppress`. The tool emits
   `SMY-W211` on stderr *and* leaves a comment in the artifact naming the
   suppressed contention by id. Argue why naming it — rather than counting it
   — is the difference between rule V2 being honoured and merely gestured at.],
  [A profile can hide a contention. It cannot hide a *status*. Explain why the
   format is willing to let a rendering suppress one and not the other.],
))

#answers((
  [`plain` for the executive: the marker is compact and the audience wants the
   shape of the situation, not the apparatus. `analyst` for the auditor: the
   status is spelled as a word that cannot be misread, and each claim carries
   what it rests on, so the reader can walk the argument without the source
   document. The point of profiles is that this is a *presentation* decision
   made once, rather than two summaries that can drift apart.],
  [Because a count is not actionable and an id is. "1 contention suppressed"
   tells a reader something was hidden and gives them no way to go look; the id
   lets them find it in the store and decide for themselves. Rule V2 exists so
   that a rendered artifact can never read unanimous over a store that is not —
   and a reader who cannot locate what was hidden is, for practical purposes,
   reading a unanimous document.],
  [Because a contention is a fact *about the document* and a status is a fact
   *about the claim*. Suppressing a disagreement produces a shorter honest
   document with a marked omission; suppressing a status would produce a
   sentence that asserts more than the document does — the reader would see a
   claim and have no way to tell a measurement from a guess. That is precisely
   the loss the whole format exists to prevent, so no profile is permitted to
   cause it: rule V1 requires every status to render distinctly.],
))

#recap((
  [`render` is a pure function of (store, thread, profile, target): the same
   inputs always produce the same bytes.],
  [A profile controls *how* a thread sounds — register, person, verbosity,
   depth by role, and how status/provenance/contentions are shown — never
   *what* it says.],
  [Rule V1 is enforced at profile load: a profile that would render two
   statuses identically, or any status as nothing, never becomes a `Profile`
   value and never reaches an artifact (`SMY-E210`, exit `3`).],
  [Rule V2 lets a profile suppress contentions from the rendered text, but
   every backend still records the suppression in the artifact's own
   metadata (`SMY-W211`); `--strict` turns that warning into exit `3`.],
  [`--lod` caps depth regardless of the profile's own default; it can only
   lower what a profile asks for, never raise it.],
  [Threads are keyed by `(id, owner)`. A store holding two threads under one
   name is refused rather than guessed at (exit `2`, usage); `--as` names
   the owner and resolves it.],
))

#chapter(number: 25, title: "Every Target Format")

#callout(label: "Why")[
  A brief read by an engineering lead, a brief pasted into a pull request, a
  brief opened in a browser, a brief projected on a screen, and a brief
  consumed by another program are five different jobs, not one job with five
  fonts. `--target` is the axis that answers "who — or what — reads this
  artifact next," independently of the profile axis from Chapter 24, which
  answers "in what voice." The two compose freely: any profile, any target.
]

#section("The six targets")

`Target::parse` accepts six names (`md` is an accepted alias for
`markdown`, not a seventh target):

#dtable(
  (auto, auto, 1fr),
  (
    ([`--target`], [Extension], [Built for]),
    ([`markdown`], [`.md`], [Docs, pull requests, chat — anything that already renders Markdown.]),
    ([`typst`], [`.typ`], [A print-quality document, chained straight into `typst compile` for a PDF.]),
    ([`html`], [`.html`], [A browser view — semantic tags and `data-*` attributes, no styling opinions.]),
    ([`slides`], [`.typ`], [A presentation: one Typst slide per block, via the same Typst pipeline as `typst`.]),
    ([`json`], [`.json`], [Programmatic consumption — the IR, essentially, as a document another program parses.]),
    ([`text`], [`.txt`], [The leanest possible plain rendering: no markup at all, not even Markdown's.]),
  ),
)

Every target is built from the same IR (Chapter 24's `build_ir` step) by a
backend that only ever sees that IR — never the store, never the thread
directly. That is a deliberate seam: a backend that could reach back into the
store could disagree with another backend about what the document says,
which would make "the same thread renders to the same meaning in every
format" false. It is asserted directly in
`crates/smysl-render/src/backend/mod.rs`: every available target renders
every block, keeps every status distinguishable, and records rule V2
suppression identically — the same properties Chapter 24 showed for Markdown
hold for all six.

Markdown is the default and already fully shown in Chapter 24 — the same
`t/brief` / `plain` render from there is what you get with no `--target` flag
at all. The rest of this chapter runs that identical render through the
other five.

#subsection("typst — a print-quality artifact, chained to a real PDF")

#screen(caption: "$ smysl render --thread t/brief --profile plain --target typst fixtures/corpus/F1-incident.smy")[```
#set document(title: "Auth p95 tripled in eu-west; pool saturation is leading but contested.")
#set text(font: "Libertinus Serif", size: 11pt)
#set par(leading: 0.7em, justify: true)

= Auth p95 tripled in eu-west; pool saturation is leading but contested.

#emph[brief · profile plain]

== bottom-line

#strong[≈] Pool saturation is the leading cause but is not consistent with the canary.

== support

#strong[≈] The eu-west connection pool is saturated.

#block(stroke: 1pt, inset: 6pt)[contested — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record]

#block(stroke: 1pt, inset: 6pt)[contested — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record]

== risk

#strong[⊢] On the other hand, the canary rules out a pure configuration cause.

#block(stroke: 1pt, inset: 6pt)[contested — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record]

#strong[Open contentions:] k/ccm3actwjjti65famnoe6mapo5d
```]

This is not a Markdown file wearing a `.typ` extension — it is Typst source,
using `#set`, `#emph`, `#strong`, and `#block` the way the design system in
this very manual does. It compiles for real. Piping it straight through the
`typst` CLI produces an actual PDF, no manual editing in between:

#screen(caption: "$ smysl render --thread t/brief --profile plain --target typst fixtures/corpus/F1-incident.smy > brief.typ && typst compile brief.typ brief.pdf")[```
$ typst compile brief.typ brief.pdf
$ ls -la brief.pdf
-rw-r--r--  1 gandalf  wheel  23856 Jul 28 01:35 brief.pdf
```]

That is the chain this target exists for: a store you can diff and merge like
any other text file, ending in a typeset PDF you can hand to someone who will
never see a terminal.

#subsection("html — a browser view")

`html` is compiled in on this build (default features include neither `html`
nor `render-html` — more on that below), so building it first requires
`--features render-html`:

#screen(caption: "$ cargo build --features render-html && smysl render --thread t/brief --profile plain --target html fixtures/corpus/F1-incident.smy")[```
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>Auth p95 tripled in eu-west; pool saturation is leading but contested.</title>
<meta name="smysl-profile" content="plain">
<meta name="smysl-thread" content="t/brief">
<meta name="smysl-contentions-suppressed" content="false">
<meta name="smysl-open-contentions" content="k/ccm3actwjjti65famnoe6mapo5d">
</head>
<body>
<h1>Auth p95 tripled in eu-west; pool saturation is leading but contested.</h1>
<p class="meta">brief · profile plain</p>
<section class="block" data-role="bottom-line" data-status="inferred" data-uid="b3:js4xzessu5zwjpv2rawtugnuvjuf2o3cys7ko3lsnza3btoldlrq" data-lod="L0">
<h2>bottom-line</h2>
<p><span class="status-marker">≈</span> Pool saturation is the leading cause but is not consistent with the canary.</p>
</section>
<section class="block" data-role="support" data-status="inferred" data-uid="b3:cvhirtgs2mpvli2ethhyeo32ufud72l42pa3zxfmsq7dgg722ada" data-lod="L0">
<h2>support</h2>
<p><span class="status-marker">≈</span> The eu-west connection pool is saturated.</p>
<aside class="contention">k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record</aside>
<aside class="contention">k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record</aside>
</section>
<section class="block" data-role="risk" data-status="derived" data-uid="b3:phsoomklkmlq3sjvbe6cyuqy5v3sl6srqxpimogpgmoxvg2pit6q" data-lod="L0">
<h2>risk</h2>
<p><span class="status-marker">⊢</span> On the other hand, the canary rules out a pure configuration cause.</p>
<aside class="contention">k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record</aside>
</section>
<footer><strong>Open contentions:</strong> k/ccm3actwjjti65famnoe6mapo5d</footer>
</body>
</html>
```]

No inline styling, no framework markup — plain semantic HTML with `data-*`
attributes carrying exactly the information a stylesheet or a script would
need (`data-status`, `data-uid`, `data-lod`), and the metadata the artifact
must always carry (rule V2's suppression flag, the open contention id) landed
as `<meta>` tags rather than dropped. This is a target for a browser or for
whatever renders on top of the browser, not a finished page — bring your own
CSS.

#subsection("slides — one Typst slide per block")

`slides` shares its availability with `typst` (both are Typst-backed; the
source ties `Target::Slides.available()` to the same feature flag as
`Target::Typst`), and the output is a Typst deck rather than a document — one
`#pagebreak()`-separated slide per rendered block, sized for a 25×14 cm
screen rather than a page:

#screen(caption: "$ smysl render --thread t/brief --profile plain --target slides fixtures/corpus/F1-incident.smy")[```
#set page(width: 25cm, height: 14cm, margin: 1.5cm)
#set text(size: 20pt)
#set document(title: "Auth p95 tripled in eu-west; pool saturation is leading but contested.")

#align(center + horizon)[
  #text(size: 28pt)[Auth p95 tripled in eu-west; pool saturation is leading but contested.]

  #text(size: 14pt)[brief · plain]
]
#pagebreak()

// slide 1: b3:js4xzessu5zwjpv2rawtugnuvj
#text(size: 14pt)[bottom-line]

#strong[≈] Pool saturation is the leading cause but is not consistent with the canary.

#pagebreak()

// slide 2: b3:cvhirtgs2mpvli2ethhyeo32uf
#text(size: 14pt)[support]

#strong[≈] The eu-west connection pool is saturated.

#block(stroke: 1pt, inset: 6pt)[contested — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record]

#block(stroke: 1pt, inset: 6pt)[contested — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record]

#pagebreak()

// slide 3: b3:phsoomklkmlq3sjvbe6cyuqy5v
#text(size: 14pt)[risk]

#strong[⊢] On the other hand, the canary rules out a pure configuration cause.

#block(stroke: 1pt, inset: 6pt)[contested — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record]


#pagebreak()
#strong[Open contentions:] k/ccm3actwjjti65famnoe6mapo5d
```]

This chains through `typst compile` exactly like the `typst` target does — the
only differences are the page geometry and the one-block-per-slide layout. If
you need to stand in front of a room with this brief, `slides` is the target,
not `typst`.

#subsection("json — for another program")

#screen(caption: "$ smysl render --thread t/brief --profile plain --target json fixtures/corpus/F1-incident.smy")[```
{
  "gist": "Auth p95 tripled in eu-west; pool saturation is leading but contested.",
  "meta": {
    "profile": "plain",
    "thread": "t/brief",
    "schema": "brief",
    "audience": null,
    "contentions_suppressed": false,
    "open_contentions": ["k/ccm3actwjjti65famnoe6mapo5d"]
  },
  "blocks": [
    {
      "role": "bottom-line",
      "uid": "b3:js4xzessu5zwjpv2rawtugnuvjuf2o3cys7ko3lsnza3btoldlrq",
      "level": "L0",
      "status": "inferred",
      "marker": "≈",
      "connective": null,
      "text": "Pool saturation is the leading cause but is not consistent with the canary.",
      "notes": []
    },
    {
      "role": "support",
      "uid": "b3:cvhirtgs2mpvli2ethhyeo32ufud72l42pa3zxfmsq7dgg722ada",
      "level": "L0",
      "status": "inferred",
      "marker": "≈",
      "connective": null,
      "text": "The eu-west connection pool is saturated.",
      "notes": [
        {"kind": "contention", "text": "k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record"},
        {"kind": "contention", "text": "k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record"}
      ]
    },
    {
      "role": "risk",
      "uid": "b3:phsoomklkmlq3sjvbe6cyuqy5v3sl6srqxpimogpgmoxvg2pit6q",
      "level": "L0",
      "status": "derived",
      "marker": "⊢",
      "connective": "On the other hand, ",
      "text": "The canary rules out a pure configuration cause.",
      "notes": [
        {"kind": "contention", "text": "k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record"}
      ]
    }
  ]
}
```]

This is close to the IR itself, serialized: every field Chapter 24 discussed
by name — `status`, `marker`, `level`, `connective`, `notes` with their
`kind` — appears as a JSON field with the same name. Reach for this target
when the reader is a program: a dashboard that wants to colour by `status`, a
notifier that wants to alert on a non-empty `open_contentions`, a second tool
in your own pipeline that wants the brief without re-deriving it from the
store.

#subsection("text — the leanest possible rendering")

#screen(caption: "$ smysl render --thread t/brief --profile plain --target text fixtures/corpus/F1-incident.smy")[```
Auth p95 tripled in eu-west; pool saturation is leading but contested.
======================================================================

bottom-line [b3:js4xzessu5zwjpv2rawtugnuvj]
  ≈ Pool saturation is the leading cause but is not consistent with the canary.

support [b3:cvhirtgs2mpvli2ethhyeo32uf]
  ≈ The eu-west connection pool is saturated.
      - k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record
      - k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

risk [b3:phsoomklkmlq3sjvbe6cyuqy5v]
  ⊢ On the other hand, the canary rules out a pure configuration cause.
      - k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

open contentions: k/ccm3actwjjti65famnoe6mapo5d
```]

No `#`, no `**`, no HTML tags — a title underlined with `=`, indentation
instead of headings, a short uid in brackets instead of a footnote. This is
the target for a terminal pager, a log line, an email body, or anywhere
Markdown's own syntax would just be visual noise the reader has to mentally
strip out.

#section("Feature gates: what this build can and cannot produce")

`Target::available()` is a compile-time question, and the answer is checked
before a backend ever runs — an unavailable target refuses rather than
silently substituting a different format:

#dtable(
  (auto, auto, 1fr),
  (
    ([Target], [Cargo feature], [In this repo's default build?]),
    ([`markdown`, `json`, `text`], [none — always compiled], [Yes, unconditionally.]),
    ([`typst`, `slides`], [`render-typst` → `smysl-render/typst`], [Yes — `render-typst` is in the workspace's `default` feature list.]),
    ([`html`], [`render-html` → `smysl-render/html`], [No — `render-html` is not a default feature; it must be requested explicitly.]),
  ),
)

The default build (`cargo build`, no flags) genuinely cannot produce `html`
until you ask for it:

#screen(caption: "$ smysl render --thread t/brief --profile plain --target html fixtures/corpus/F1-incident.smy   (default build)")[```
smysl render: render target html is not available in this build
exit 1
```]

Rebuilding with `--features render-html` is what produced the real HTML
sample earlier in this chapter. The same refusal-not-substitution rule runs
the other way too: build with only the bare minimum (`--no-default-features
--features cli`, which drops `render-typst`) and `typst` refuses on that
build the same way `html` refused on the default one, while `markdown` still
works untouched:

#screen(caption: "$ cargo build --no-default-features --features cli && smysl render --thread t/brief --profile plain --target typst fixtures/corpus/F1-incident.smy")[```
smysl render: render target typst is not available in this build
exit 1
```]

#screen(caption: "$ smysl render --thread t/brief --profile plain --target markdown fixtures/corpus/F1-incident.smy   (same minimal build)")[```
# Auth p95 tripled in eu-west; pool saturation is leading but contested.
...
```]

An artifact in the wrong format would be more surprising than an error
naming the format as unavailable — this is a deliberate design choice
(`crates/smysl-render/src/backend/mod.rs`), not an oversight: `emit` checks
`target.available()` before matching on the target at all, so there is no
code path in which a build silently hands you Markdown when you asked for
HTML.

#whatsnext[
  Six targets, one IR, one guarantee: whichever format you pick, it says the
  same thing the others do, with the same statuses, the same contentions,
  and the same suppression record. Pick the target that matches your actual
  downstream reader — a person (`markdown`, `html`, `slides`), a typesetter
  chasing a PDF (`typst`), or another program (`json`) — rather than defaulting
  to Markdown out of habit. Chapter 26 goes the other direction: instead of
  picking a shipped profile, you write your own from scratch.
]

#exercises((
  [Render `F1-incident.smy` to `markdown`, `text`, `json` and `slides` in turn.
   The `slides` output is Typst source, not an image, and `json` is a tree
   rather than prose. Given that all four came from one thread under one
   profile, say what the *target* axis controls that the profile axis does
   not.],
  [Run `render --target html` on a default build. It reports that the target is
   not available. Look back at Chapter 4's feature table and say why a missing
   render backend is a build-time decision rather than a runtime one.],
  [Render the same document twice with the same flags and compare hashes. They
   match. Name the property, and then name one thing you would have to add to
   `render` to lose it.],
))

#answers((
  [The target decides *who or what reads the artifact next* — a person in a
   terminal, a person in a browser, a projector, another program — and
   therefore its syntax and structure. The profile decides *in what voice* the
   same content is put. They are independent on purpose: you can send the
   analyst voice to a slide deck or the executive voice to JSON, and neither
   axis has to know about the other.],
  [Because the backends are separate code with separate dependencies, and
   compiling them in is a choice about what you want in your dependency tree.
   A build without `render-html` genuinely does not contain an HTML renderer,
   which is a stronger and more auditable statement than a runtime flag that
   disables one. The command tells you plainly rather than silently emitting
   something else.],
  [Determinism — rule D. `render` is a pure function of the graph, the thread
   and the profile, so the same inputs give the same bytes forever, and an
   artifact signed off last year can be regenerated and compared today. You
   would lose it by adding almost any of the obvious conveniences: a
   generated-on timestamp in the header, a model call to smooth the prose, or
   anything that consulted the wall clock or the network.],
))

#recap((
  [Six targets: `markdown` (default), `typst`, `html`, `slides`, `json`,
   `text` — `md` is an alias for `markdown`, not a seventh target.],
  [Every target is built from the same IR by a backend that never touches the
   store directly, so no format can disagree with another about what the
   document says.],
  [`typst` and `slides` chain directly into `typst compile` for a real PDF or
   deck; verified end to end in this chapter.],
  [`html` needs `--features render-html`; `typst`/`slides` need
   `render-typst` (in this repo's default feature set already). An
   unavailable target refuses with exit `1` rather than substituting another
   format.],
  [`json` is the closest thing to the IR itself, and is the target to reach
   for when the next reader is a program rather than a person.],
))

#chapter(number: 26, title: "Writing a Custom Profile")

#callout(label: "Why")[
  `plain`, `exec`, and `analyst` cover the common cases — a neutral default,
  a thin brief for people who will act on it, and a full trace for someone who
  has to check it. They do not cover every case. A regulator who needs every
  status spelled out in full words with inline citations, in a register your
  own house style forbids from ever using `exec`'s tight verbosity, is a real
  audience your three built-ins were never written for. A profile file is how
  you describe that audience once and reuse it, without any of it touching the
  graph itself.
]

#section("The profile file grammar")

A profile file is a Hjson-flavoured object, optionally preceded by a
`profile NAME {` header (a bare `{ … }` also loads, with a default name of
`unnamed`). Every field is optional; a missing field takes the same default
`Profile::plain()` uses. Here is the complete field list, straight from
`crates/smysl-render/src/profile.rs`:

#dtable(
  (auto, auto, 1fr),
  (
    ([Field], [Values], [Default]),
    ([`register`], [`formal`, `neutral`, `plain`], [`neutral`]),
    ([`person`], [`first`, `second`, `third`], [`third`]),
    ([`verbosity`], [`tight` (1 note/block), `standard` (3), `full` (unbounded)], [`standard`]),
    ([`audience`], [any string], [none]),
    ([`connectives`], [`from-relations`, `none`], [`from-relations`]),
    ([`lod.default`], [`L0`, `L1`, `L2`], [`L1`]),
    ([`lod.roles.<role>`], [`L0`/`L1`/`L2` per role name (`bottom-line`, `risk`, `support`, `ask`, `setup`, …)], [falls back to `lod.default`]),
    ([`show.provenance`], [`none`, `inline`, `footnote`], [`footnote`]),
    ([`show.status`], [`inline-marker`, `word`, `none` (refused — rule V1)], [`inline-marker`]),
    ([`show.contentions`], [`always`, `on-rendered`, `suppress`], [`always`]),
    ([`markers.<status>`], [a string per kernel status, overriding the default glyph/word], [built from `show.status`]),
  ),
)

The RFC's own worked example — reproduced verbatim as the built-in `exec`
profile — shows the shape end to end:

```
profile exec {
  register: formal, person: third, verbosity: tight
  audience: "engineering leadership"
  lod:  { default: L1, roles: { bottom-line: L1, risk: L0, support: L0, ask: L0 } }
  show: { provenance: footnote, status: inline-marker, contentions: always }
  connectives: from-relations
}
```

Read it field by field: `register: formal` and `verbosity: tight` set the
overall voice; `lod.default: L1` is the fallback depth, overridden per role so
`risk`, `support`, and `ask` sit at `L0` (headline only) while `bottom-line`
stays at `L1`; `show.status: inline-marker` picks glyphs over spelled-out
words; `show.contentions: always` means this profile never hides a
disagreement, only `--contentions suppress` on the command line can. Nothing
here is order-sensitive — Hjson object fields, not a sequence of statements —
and any field left out takes `Profile::plain()`'s value for it.

#section("Authoring one from scratch")

Suppose the audience is an external compliance reviewer: formal register,
full verbosity (every note, every citation inline, nothing held back), status
spelled out as full words rather than glyphs, and grounds pulled up to `L2`
for the two roles a reviewer actually has to verify (`bottom-line` and
`risk`), while `support` stays at `L1`. Connectives off, because a reviewer
should read each claim on its own footing rather than have the prose imply a
narrative flow between them.

#screen(caption: "$ cat /tmp/regulator.profile")[```
profile regulator {
  register: formal, person: third, verbosity: full
  audience: "external compliance reviewer"
  lod:  { default: L1, roles: { bottom-line: L2, risk: L2, support: L1 } }
  show: { provenance: inline, status: word, contentions: always }
  connectives: none
  markers: {
    unfounded: "[UNFOUNDED]", speculative: "[SPECULATIVE]", inferred: "[INFERRED]",
    derived: "[DERIVED]", cited: "[CITED]", measured: "[MEASURED]"
  }
}
```]

A profile file has no special extension the tool requires — `--profile` tries
a built-in name first, and if that lookup fails, treats the value as a
filesystem path and reads it, whatever it's named. Point `--profile` straight
at the path:

#screen(caption: "$ smysl render --thread t/brief --profile /tmp/regulator.profile fixtures/corpus/F1-incident.smy")[```
# Auth p95 tripled in eu-west; pool saturation is leading but contested.

*brief · profile regulator*
*for external compliance reviewer*

## bottom-line

[INFERRED] Pool saturation is the leading cause but is not consistent with the canary.

*rests on b3:wo4t2c46lq45fnakd6tajlgcac*

## support

[INFERRED] The eu-west connection pool is saturated.

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

*rests on b3:izyuzlt42mqcvgdfb4nfpllxyq*

## risk

[DERIVED] The canary rules out a pure configuration cause.

> **contested** — k/ccm3actwjjti65famnoe6mapo5d: contested, 2 position(s) on record

*rests on b3:xkys7j42mcuyiaxiyh73xddimr*

---

**Open contentions:** k/ccm3actwjjti65famnoe6mapo5d
```]

This is a fourth, genuinely distinct voice: `[INFERRED]`/`[DERIVED]` in
brackets (the custom `markers` override, not the built-in `word` display's
`[inferred]`/`[derived]` — you can restyle the word itself, not only choose
word-vs-marker), no connective softening the `risk` block into "on the other
hand" the way `plain` and `exec` both did, and every block carries the
`*rests on …*` grounds note that only `verbosity: full` unlocks. Nothing about
the claims or their status changed from the plain render at the start of
Chapter 24 — only how insistently the apparatus around them talks.

#section("Breaking rule V1 on purpose, one more time")

The same refusal from Chapter 24 applies to every custom profile, built-in or
not — because it is enforced once, in `Profile::load`, before a name is ever
attached to a `Profile` value. Ask this new profile file to also hide status,
and it fails exactly the way the hand-built `flat` profile did:

#screen(caption: "$ cat /tmp/regulator-broken.profile")[```
profile regulator {
  register: formal, person: third, verbosity: full
  audience: "external compliance reviewer"
  show: { provenance: inline, status: none, contentions: always }
}
```]

#screen(caption: "$ smysl render --thread t/brief --profile /tmp/regulator-broken.profile fixtures/corpus/F1-incident.smy")[```
smysl render: /tmp/regulator-broken.profile: SMY-E210: profile regulator has no distinct rendering for unfounded
exit 3
```]

There is no register, verbosity, or audience setting anywhere in the grammar
that buys an exemption from rule V1 — formal, tight, aimed at a regulator, or
otherwise. The check runs against `show.status`/`markers` alone, and it runs
before the rest of the profile's intent is even relevant. If your house style
genuinely wants a quieter status display than `inline-marker` gives you, the
honest move is `word` (spell it out, which reads *more* insistently, not
less) or a custom `markers` block with your own six distinct strings — never
`none`.

#whatsnext[
  You now have full control over every stage this manual has covered so far:
  creation (Chapters 6–7), enrichment (7–10), operation (11–18), verification
  (19–21), and — as of these three chapters — export. Part VIII shows the
  same primitives (`Profile`, `Target`, `build_ir`, `emit`) used as a Rust
  library rather than through this CLI, for a program that wants to render
  without shelling out; Chapter 29 chains everything in this manual, `render`
  included, into one realistic end-to-end scenario.
]

#exercises((
  [`render --profiles` lists three built-ins and their settings: `plain` is
   neutral at L1 with a status *marker*, `analyst` is L2 with a status *word*.
   Before writing any profile of your own, say which of those two axes — level
   of detail, or how status is spelled — you would expect a regulator to care
   about, and why.],
  [Rule V1 says a profile must render every status distinctly. Try to write a
   profile that maps both `inferred` and `derived` to the same marker, and see
   what `Profile::load` says. Then explain why this is enforced at *load* time
   rather than when the offending unit is first encountered.],
  [Your house style forbids the terse register `exec` uses, but you want its
   brevity. Which parts of a profile can you change to get that, and which part
   of the output is not the profile's to decide at all?],
))

#answers((
  [The status spelling. A marker like `≈` is compact and assumes the reader
   knows the convention; the word `[inferred]` cannot be misread by someone
   encountering the document once, under obligation, possibly in a dispute.
   Level of detail matters too, but it trades length against completeness —
   the status axis trades *nothing*, which is why an audience that must not
   misunderstand gets words.],
  [It refuses to load. Enforcing it at load time means a profile is either
   valid for every document or rejected outright — the failure cannot depend
   on which units a particular store happens to contain. The alternative would
   be a profile that renders a thousand documents correctly and then flattens
   two statuses on the one document where the distinction mattered, which is
   the hedge loss from Chapter 1 arriving at the very last step.],
  [You can change the register, the verbosity, the level of detail, the status
   spelling, and whether an audience line is printed. You cannot change what
   the units *say* — a profile selects and frames gists, it never rewrites
   them. That boundary is why rendering stays pure: if a profile could reword
   a claim, the artifact would no longer be a function of the graph, and two
   renderings of one document could disagree about what it asserts.],
))

#recap((
  [A profile file is an optional `profile NAME { … }` header over an Hjson
   object; every field is optional and falls back to `Profile::plain()`'s
   default.],
  [Fields: `register`, `person`, `verbosity`, `audience`, `connectives`, a
   `lod` block (`default` plus per-role overrides), a `show` block
   (`provenance`, `status`, `contentions`), and a `markers` block overriding
   individual status renderings.],
  [`--profile` accepts a built-in name or a filesystem path — no special
   extension required — and falls back to treating the value as a path only
   after the built-in lookup fails.],
  [A hand-authored profile can combine these fields into a voice none of the
   three built-ins provide, while never changing what a claim says or what it
   rests on.],
  [Rule V1 applies identically to every profile, built-in or custom: any
   profile that would render two statuses the same way, including via
   `show.status: none`, fails to load (`SMY-E210`, exit `3`) before it can
   ever produce an artifact.],
))
