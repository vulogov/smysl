#import "design.typ": *

// ═══════════════════════════════════════════════════════════════════════
#part(number: "X", title: "Libraries of Texts (SMYSL-2)")
// ═══════════════════════════════════════════════════════════════════════
//
// Everything before this part is about stores of units: claims, evidence and the edges between
// them. SMYSL-2 adds the texts those units are drawn from. Every terminal block below was
// produced by the smysl 1.14.0 binary. Where a step needs a model or units extracted by one,
// the output is the offline `information_theory` example's, and the block says so.

#chapter(number: 30, title: "Libraries: Texts, Parts and Readings")

Until now a unit's `source` has been a pointer out of the store: a URL, a dashboard panel, a
document reference. Nothing checked that the words a claim quotes are in the thing it cites,
because the thing was not there. A *library* puts the text in.

#section("Why a library, and not a folder of files")

#callout(label: "Why")[
  *A citation is only as good as the bytes it points at.* A claim drawn from chapter 3 of a
  book should name those bytes so exactly that a second copy of the book, under another title,
  is recognised as the same text — and so that a changed edition is recognised as a different
  one. A folder of files cannot say either. A library can, because every text in it has an
  identity computed from its content.
]

Four identities do the work. Each is BLAKE3 over a domain byte and canonical bytes, like a
unit's uid, and each has its own prefix so that one can never be mistaken for another.

#dtable(
  (auto, auto, 1fr),
  (
    ([Prefix], [Names], [Taken over]),
    ([`t3:`], [a *part*], [the part's normalised bytes: UTF-8, NFC, LF line endings. The same paragraph in two books is one tid.]),
    ([`r3:`], [a *reading*], [the reader's segment table over the part: each row's range, level, locator, and what else the reader knew (speaker, time, language).]),
    ([`m3:`], [a *manifest*], [the catalog entry: alias, reader, language, licence, and the list of parts with their readings and structure hashes.]),
    ([`d3:`], [a *dating*], [a statement about when something was said, composed or is about (Chapter 32).]),
  ),
)

#term("structure hash")[
  The part entry's hash of the segment *tree*: each row's start, end, level and locator, and
  nothing else. Since 1.14 (OQ-70) a speaker, an instant or a pseudonym moves the rdid but not
  the structure, so pseudonymising a chat no longer re-identifies its parts.
]

#section("Adding texts")

A text goes in through a *reader*. The reader turns a file format into the normalised text and
the segment table; nothing else in the library interprets a format. Nine readers ship with the
CLI: `txt/1`, `md/1`, `usfm/1`, `osis/1`, `zefania/1`, `json/1`, `telegram/1`, `slack/1` and
`whatsapp/1`.

#screen(caption: "$ smysl text add ruth.usfm --reader usfm/1 --alias kjv/ruth --lang en --licence public-domain --carry text --versification kjv -s lib")[
```
manifest  m3:7gwhq4wwndj3xenah4ujp3ywed7dv7gn446xzj5a7te7a3i3iidq
alias     kjv/ruth
reader    usfm/1
bytes     13215
parts     4
  part 0  t3:qu2osma5u6cmqfwzaryyp6kl3cmfjlmdcnp4vkonykxqw7mx6yla
  part 1  t3:6n6vm3q7goznq7ol2nz34c4kv7fbwwrsi6hrpofvojj7l7hgmb6q
  part 2  t3:ovftalgzwzxdqlpylk7kwbtvomf3jsnhoqkm3atsc3iudp3wixvq
  part 3  t3:o7ywaoqkiq3j7obruk5swzxqtq3q57cehfz3fjlxiodmkuwm2yva
objects   8 written
```
]

The book of Ruth became four parts, one per chapter: the part policy cuts at the top level
the reader reports and keeps each part within its size targets. `--lang` is required when the
source does not say. The library refuses to write `und`, because "nobody knows" is a claim and
"nobody said" is not. `--licence` and `--carry` decide whether the text may travel in an
export (Chapter 36).

A chat export is added the same way. `--pseudonymise` replaces every speaker with a keyed
pseudonym. The key is thirty-two random bytes under `secrets/pseudonym.key`, so the same
person gets the same pseudonym in every later `text append`:

#screen(caption: "$ smysl text add chat.json --reader telegram/1 --alias team-chat --lang en --licence CC0-1.0 --pseudonymise -s lib")[
```
manifest  m3:gxymlmtcksmizaaxqppkdjaatpk3czotgb3ycc6yx74lqempj3tq
alias     team-chat
reader    telegram/1
bytes     349
parts     1
  part 0  t3:2xiicmkhwjfdpacpeu4qhlntfpdosfnw6iapkkaohvbtlidfhgka
objects   2 written
lossy     the reader dropped something the source carried
speakers  pseudonymised under secrets/pseudonym.key
```
]

`lossy` is the reader being honest. A Telegram export carries edits, forwards and media that
the reader does not keep (OQ-19 and OQ-20 decided that it should not), and a measure that
counts text refuses a lossy one unless its recipe admits it (Chapter 35).

#section("Reading it back")

#screen(caption: "$ smysl text ls -s lib")[
```
kjv/ruth
  m3:7gwhq4wwndj3xenah4ujp3ywed7dv7gn446xzj5a7te7a3i3iidq  4 part(s)
notes
  m3:bm7kj77dhl7k4fbbuoudll5pmc7wj4yyx4rlxja45nbtfjbwa2rq  1 part(s)
team-chat
  m3:gxymlmtcksmizaaxqppkdjaatpk3czotgb3ycc6yx74lqempj3tq  1 part(s)
```
]

A passage is addressed as `<alias>#<locator>`, in the locator scheme the reader writes:
canonical references for scripture, line numbers for Markdown, and dated message ids for a chat.

#screen(caption: "$ smysl text show kjv/ruth#Ruth.1.16 -s lib")[
```
manifest  m3:7gwhq4wwndj3xenah4ujp3ywed7dv7gn446xzj5a7te7a3i3iidq
part      t3:qu2osma5u6cmqfwzaryyp6kl3cmfjlmdcnp4vkonykxqw7mx6yla
range     2244..2456

And Ruth said, Intreat me not to leave thee, [or] to return from following after thee:
for whither thou goest, I will go; and where thou lodgest, I will lodge: thy people
[shall be] my people, and thy God my God:
```
]

The `range` is the passage's byte span in the part. It is what a unit drawn from this
verse records as its `source.span`, and what makes the citation checkable: the quote must be
in those bytes.

#section("Growing a text, and taking one out")

A chat grows. `text append <file> --alias team-chat` reads the whole new export as a new
version of the expression: parts whose text did not change hash to the tids they already had,
and the new manifest `supersedes` the old, so the alias keeps one head.

A text can also have to leave. `text redact` removes a part's bytes and readings from the
object store and records that it did. The claims drawn from the part stay, because a redaction is
not a retraction: the claims were made, whether or not the text is still held.

#screen(caption: "$ smysl text redact t3:y2hmm7kwc4xlhqekmjufjyqkvug2ore3q55wodazon53nwt4wulq --as human:editor --at 1730500000000 -s lib")[
```
redacted  t3:y2hmm7kwc4xlhqekmjufjyqkvug2ore3q55wodazon53nwt4wulq
agent     human:editor
objects   2 unlinked
manifests 1 still name this part
```
]

#screen(caption: "$ smysl text show notes#L1 -s lib")[
```
smysl text show: t3:y2hmm7kwc4xlhqekmjufjyqkvug2ore3q55wodazon53nwt4wulq has been redacted;
this library no longer holds its text
```
]

#recap((
  [A library holds texts as content-addressed *parts* (`t3:`), each with a *reading*
   (`r3:`), catalogued by *manifests* (`m3:`) under an alias.],
  [A reader is the only code that interprets a file format; nine ship with the CLI.],
  [`--pseudonymise` keys speakers so that the same person stays the same pseudonym across
   appends. The structure hash ignores speakers, so pseudonyms move no part's identity.],
  [`text redact` removes bytes and keeps claims.],
))

#whatsnext[
  A library of texts holds no claims yet. The next chapter is the one place a model enters:
  `ingest --text` reads an expression window by window and stages units whose quotes are
  attached to the bytes they came from.
]

// ─────────────────────────────────────────────────────────────────────────

#chapter(number: 31, title: "Ingest over Texts: Windows, Consensus and Replies")

Chapter 10's `ingest` turns a document into units. `ingest --text` does the same for an
expression of the library, with three differences that matter:

- the model sees the text in *windows* cut at the reading's own boundaries;
- every quote is attached to its exact byte span;
- each window is extracted *twice*.

#section("Why two passes")

#callout(label: "Why")[
  *A single extraction cannot say how stable it is.* Two passes over the same window, compared
  by `identical-span` (same tid, same span, same gist), give a figure for that: how much of what
  the model found it would find again. Both passes are staged and nothing is deduplicated, because
  two passes that found the same claim produce the same uid anyway.
]

Two passes above temperature 0 are refused before any call is made. Two runs that sample
share no identity, so the second pass would buy nothing. `--single-pass` opts out, and the
recipe records which policy built a corpus.

```
smysl ingest --text team-chat -s lib            # two passes, temperature 0
smysl ingest --text team-chat --single-pass -s lib
smysl ingest --text team-chat --cost -s lib     # what the journal says it cost; no call
```

#section("Whether the provider reproduced")

Temperature 0 is what the caller asked for, not what every provider does. A hosted model sent
`"temperature": 0.0` varied as much within one language as between languages. So since 1.14
each window's *first* answer from each pass is compared byte for byte (OQ-74), and the run
reports two counts beside the agreement figure:

- *Every window reproduced.* The second pass repeated the first, and the figure measures the
  extraction alone.
- *Any window differed.* The figure includes the provider's own variation and is a lower bound.
  The run says so once, with `SMY-W454`.

The `information_theory` example runs this over its study-group chat with a scripted, and
therefore reproducible, provider:

#screen(caption: "cargo run --example information_theory   (§4, offline)")[
```
windows 1  calls 2  passes 2  units 6  spanned 12  reply edges 3
agreement: 6 of 6 place(s) identical-span; 1 of 1 window(s) answered identically by
both passes, so the provider reproduced: Some(true)
  SMY-W448 1 reply names a message `study-group` does not hold (9); no edge was drawn for it
  bruno said: In coding it is the average number of
    replies to ada said: Is entropy the same thing as disorder, or
```
]

#section("Replies are edges")

A chat says which message answers which. Since 1.14, ingest turns each reply id into an
`x.text/replies` edge from the units of the reply to the units of the message it answers,
across windows, once. Rule E reads them as ordering constraints, since an answer comes after
its question. A reply to a message the export does not hold is reported as `SMY-W448` rather
than drawn to nothing.

#callout(label: "Note")[
  An ingest attests its units at a clock (`IngestOptions::with_now` in Rust). Rule E bounds a
  unit's said time by when the store first saw it. An ingest stamped before the chat took place
  therefore makes every message contradict its own attestation (`SMY-W413`). The CLI
  attests at `--at`, or the wall clock. A program has to supply the clock itself, because the
  default is time zero.
]

#recap((
  [`ingest --text <alias>` windows an expression, extracts twice at temperature 0, and attaches
   every quote to its byte span.],
  [The agreement figure comes with a count of windows the provider answered identically; read
   the figure as a lower bound when they differ (`SMY-W454`).],
  [Reply ids become `x.text/replies` edges; a reply to a missing message is `SMY-W448`.],
))

#whatsnext[
  Units now have places and, through replies and readings, the beginnings of an order in time.
  The next chapter makes time explicit: datings, the solver that turns them into intervals, and
  the lock that stops a settled date from moving.
]

// ─────────────────────────────────────────────────────────────────────────

#chapter(number: 32, title: "Time: Datings, Effective Time and Locks")

A claim has three times that are easy to confuse:

- when it was *said*;
- when what it describes was *composed*;
- what time it is *about*.

A chat message has an instant on the first. A history paragraph written in 2024 is about 1948.
SMYSL-2 keeps the three apart as *axes* and lets anyone state a time on any of them as a
*dating*.

#section("Why datings are records, not fields")

#callout(label: "Why")[
  *Two people can disagree about when something happened, and both can be wrong.* A date
  written into a unit would be part of its identity and could not be disputed without changing
  the claim. A dating is a separate record, `@date`, with its own author, its own basis (the
  unit that gives the evidence) and its own identity (`d3:`). Rule E then solves all live datings
  and ordering constraints into each subject's *effective* interval. A contradiction is reported
  as one, and no answer is chosen for it.
]

#section("Setting and ordering")

The store here is the CI-incident document from `examples/simple_demo`. A dating needs a
basis. One without a basis is not applied (`SMY-W412`), because nothing grounds it.

#screen(caption: "$ smysl date set e/lockfile -s ci.smy --value 2026-10-27 --as human:demo --at 1761800000000 --basis e/timings")[
```
ci.smy: e/lockfile said  d3:aqcmuoyjtxgsz7hkwna3aqpmxa
ci.smy:   selects 1 subject(s)
```
]

`date order` is sugar for a relative dating, using an Allen relation (`before`, `after`, `meets`,
`overlaps`, `during`, `contains`, `equals`):

#screen(caption: "$ smysl date order c/cache-cold after e/lockfile -s ci.smy --as human:demo --at 1761800001000 --basis e/cache-misses")[
```
ci.smy: c/cache-cold said  d3:yy4teu2owfbgrbbcvy2xetk5sw
ci.smy:   selects 1 subject(s)
```
]

#screen(caption: "$ smysl date show c/cache-cold -s ci.smy --why")[
```
ci.smy: said
  c/cache-cold  [2026-10-27T00:00:00Z, ..)  lo measured / hi speculative
    lo: dating d3:yy4teu2owfbgrbbcvy2xetk5sw at measured through e/lockfile
    hi: nothing set this bound
```
]

The 2026-10-27 floor crossed the `after` relation. `--why` names the dating that set each bound
and the subject it came through. Each bound carries its own status, taken from the basis.

#section("Locks, and reviewing them")

A `canonical` commitment on a dating *locks* it, and since 1.14 surface text spells it by the
dating's did (OQ-73): `@commit d3:… { level: canonical, … }`. A lock means the value does not
move until somebody has looked. It holds the dating it names, and any later dating of the same
target that would change it, each as a contention a resolution must name:

#screen(caption: "$ smysl commit d3:aqcmuoyjtxgsz7hkwna3aqpmxainmtqzs5td2q542kfeh6bi3qnq ci.smy --level canonical --as human:demo --at 1761800002000")[
```
ci.smy: d3:aqcmuoyjtxgsz7hkwna3aqpmxa  -> canonical
```
]

#screen(caption: "$ smysl check ci.smy   (after a second, conflicting dating by human:other)")[
```
ci.smy: warning: SMY-W412: said: dating d3:aqcmuoyjtxgsz7hkwna3aqpmxa is held by a canonical
  commitment and was not applied; a resolution naming the derived contention
  (k/c5x7hqsm4o5hbti7zny7nzmydro) releases it
ci.smy: warning: SMY-W412: said: dating d3:qgaybho2gx23evscypf4s2rff3 is held by a canonical
  commitment and was not applied; a resolution naming the derived contention
  (k/cyor53zlg4pygqfzcsf5zpqxgmk) releases it
ci.smy: 24 records, 8 units, 2 diagnostic(s)
```
]

Reviewing the locked dating releases it. The conflicting one stays held until it is reviewed in
its turn:

#screen(caption: "$ smysl resolve k/c5x7hqsm4o5hbti7zny7nzmydro ci.smy --as human:demo --at 1761800004000")[
```
ci.smy: k/c5x7hqsm4o5hbti7zny7nzmydro  dating not applied over d3:aqcmuoyjtxgsz7hkwna3aqpmxa
  (held, said)
ci.smy: resolved by human:demo
```
]

#screen(caption: "$ smysl date show e/lockfile -s ci.smy")[
```
ci.smy: said
  e/lockfile  [2026-10-27T00:00:00Z, 2026-10-28T00:00:00Z)  lo measured / hi measured
```
]

#recap((
  [Three axes: `said`, `composed`, `about`. A dating is a record (`@date`, `d3:`) with an
   author and a basis, never a field of the unit.],
  [Rule E solves datings and Allen relations into effective intervals; `date show --why`
   explains each bound; contradictions are reported, not resolved.],
  [A `canonical` commitment on a dating locks it, spelled `@commit d3:…`. Held datings are
   released one at a time by `resolve`.],
))

#whatsnext[
  Time orders claims. The next question is identity across texts: when two units in two books
  say the same thing about the same bytes, are they one proposition?
]

// ─────────────────────────────────────────────────────────────────────────

#chapter(number: 33, title: "Propositions: Same-As, Links and Summaries")

A uid is exact: one changed word is a different unit. That is right for identity and wrong for
counting, since a paragraph reprinted in an anthology is still one proposition. SMYSL-2 adds a
layer above uids: `x.text/same-as` edges, proposed by *engines*, and *classes* derived from
them under a stated policy.

#section("Engines propose, policies count")

#dtable(
  (auto, 1fr),
  (
    ([Engine], [Proposes a pair when]),
    ([`identical-span`], [two units say the same gist about the *same bytes* (one tid, one span) under two different manifests. Exact, and needs no judgement.]),
    ([`lexical`], [two gists are near neighbours by BM25. It is a *candidate* only: staged at rung `computed`, and never auto-accepted (OQ-44).]),
    ([`anchored`], [two units are tied by an anchor the library already holds, such as an alignment of two numberings.]),
  ),
)

`same-as classes --policy strict` counts propositions by the normative rule: only edges whose
evidence is exact. `component` follows every edge; `attested:2` follows edges that two
engines agree on. Every corpus measure that divides by "propositions" names its policy.

The `information_theory` example adds one paragraph twice, as a lecture handout and as an
anthology reprint, and draws a unit from each:

#screen(caption: "cargo run --example information_theory   (§5, offline)")[
```
proposed b3:azxl6db3mbkklcdf4m7l2dwf6f = b3:m2l6nizznxy2gy2t5odndiqz5f
  because the same bytes 0..241 of t3:wtuflz4boo6w7dwc3a6r3fgroczfrddkexx7rnly3epliyw2v65q
  under m3:6dgh7he6pdkxxtnqr3nu3ro4zz and m3:3uc66374mov7omnxnbhzu3jkob
1 class(es) of more than one unit
```
]

```
smysl same-as propose --engine identical-span lib
smysl same-as classes --policy strict lib
```

#section("Links and summaries")

Two further commands work on top of the classes. `link` asks a judge (a model, or a person)
whether candidate pairs *entail* or *contradict* each other, and stages the edges it is given.
`summarise` builds a grounded summary tree over a scope: every summary node cites the units it
summarises, so a summary can be checked the way a claim is. Both stage their output for
`merge --staged`, like `ingest`.

#recap((
  [`same-as` edges are proposed by engines; classes are derived under a named policy
   (`strict` is normative).],
  [`identical-span` is exact and needs no judge; `lexical` proposes candidates only and is never
   auto-accepted.],
  [`link` and `summarise` stage judged edges and grounded summaries.],
))

#whatsnext[
  With units, places, times and classes in the library, the next chapter asks questions of all
  of it at once.
]

// ─────────────────────────────────────────────────────────────────────────

#chapter(number: 34, title: "sq/1: Querying, Saving and Asking")

`find` ranks units against words. sq/1 is the language for everything else: which units, from
where, at what time, reached by which edges, in what order, and whether the answer can be kept
and rerun. A query is a pipeline of stages, one per line, read top to bottom:

```
from <scope>          where to look: store:NAME, texts, library, channels,
                      collection:ALIAS, channel:ALIAS, import:NAME
find "words"          a ranked source (lexical by default)
about entity("…")     units that mention an entity, across same-as
where <predicate>     kind, status, lang, mode, fields; and / or / not
said|observed …       time on an axis: before, after, during, overlaps
expand <edge> [depth N] [only]
order by k [asc|desc], k …
limit N | count | units | spans
```

#section("Why a language, and why this one")

#callout(label: "Why")[
  *A question you can write down is a question you can rerun.* An sq/1 query is text with a
  version (`using sq/1`), planned by a deterministic planner and run by a pure executor. The
  same query over the same store gives the same answer byte for byte. That property is what
  makes a saved query's *data effect*, the units that changed since it last ran, mean anything.
]

#section("Filters, walks and order")

#screen(caption: "$ smysl q 'from store:q\\nwhere (kind = claim or kind = finding) and status >= derived\\norder by status descending, salience descending\\nunits' q.smy")[
```
b3:m6qmtoia6ql2wz45uehrsatfz7  The cache key changed with the lockfile, so every build
                               compiles from scratch.
```
]

`order by` takes several keys since 1.14, compared one after another. A walk follows edges from
the units it was given. With `only`, it keeps only what it reached:

#screen(caption: "$ smysl q 'from store:q\\nwhere kind = finding\\nexpand grounds depth 2 only\\nunits' q.smy")[
```
b3:cxl4265ejbxsmi4dtdqkumz5xd  Median pipeline time rose from 11 to 23 minutes on 2026-10-27.
b3:lrlguu5j2ixqospd5ziinziplr  A lockfile change on 2026-10-27 bumped forty transitive
                               dependencies.
b3:m6qmtoia6ql2wz45uehrsatfz7  The cache key changed with the lockfile, so every build
                               compiles from scratch.
b3:4jwvvfpzpeu7yr67q3dc4xu7ri  The build job's dependency cache missed on every run since
                               2026-10-27.
```
]

#callout(label: "Note")[
  An arrow in `expand` points the way a unit *rests on* others, as grounds do, and not the way
  the edge is written. So `->rebuts` from a claim reaches what rebuts it. `<-supersedes` walks
  from a unit to the versions it replaced.
]

#screen(caption: "$ smysl q 'from store:q\\nwhere kind = claim and status = speculative\\nexpand ->rebuts only\\nunits' q.smy")[
```
b3:qdsoqbq4m3taswnjsowihzwn74  Runner queue time stayed under one minute all week.
```
]

Pipelines combine with `union`, `intersect` and `except`. `--explain` prints the plan
instead of running it:

#screen(caption: "$ smysl q --explain 'from store:q\\nfind \"cache\"\\nlimit 2' q.smy")[
```
plan sq/1
  purity: pure
  1 header: sq/1, no schema pins
  2 resolution: no relation kinds named
  3 scope: store:q via q -> 1 store(s) q, 8 unit(s)
  4 partitions: 1, idf over collection
      (no manifest): 8 unit(s)
  5 source: lexical "cache", 10 per partition, rrf k=60
  - supersession: superseded units give way to their newest head
  9 shape: | limit 2
```
]

Over a library, `--library` makes the library scopes available. `from texts` is every unit
drawn from a text, `from channels` every chat, and `from collection:kjv` the expressions under an
alias prefix. A scope that names nothing is refused (`SMY-E511`) rather than treated as empty.

#section("Saving a query, and rerunning it")

`--save` appends the query, after it has run, as an `x.query/saved` unit with an
`x.query/answers` edge to each result. A saved unit's payload has no surface spelling, so a
saved query lives in a CBOR store (`smysl merge doc.smy -o store.cbor`):

#screen(caption: "$ smysl q --save 'from store:ci\\nfind \"cache\"' ci.cbor")[
```
0.016393  b3:lyfwzxrlfrudcophtv6my62qft  A cold dependency cache, after the lockfile bump, …
0.016129  b3:m6qmtoia6ql2wz45uehrsatfz7  The cache key changed with the lockfile, …
0.015873  b3:4jwvvfpzpeu7yr67q3dc4xu7ri  The build job's dependency cache missed on every run …
smysl q: saved b3:kpsddc75tanenqkfprjnaz2cqq
```
]

After one more claim is merged in, `--rerun` runs the same text pinned to the revisions it
recorded, and reports exactly what changed:

#screen(caption: "$ smysl q --rerun b3:kpsddc75tanenqkfprjnaz2cqq… ci.cbor")[
```
0.016393  b3:zmxbp3rqt5b6xmbhlz3k36baj3  Pinning the cache key to the toolchain would keep …
0.016129  b3:lyfwzxrlfrudcophtv6my62qft  A cold dependency cache, after the lockfile bump, …
0.015873  b3:m6qmtoia6ql2wz45uehrsatfz7  The cache key changed with the lockfile, …
0.015625  b3:4jwvvfpzpeu7yr67q3dc4xu7ri  The build job's dependency cache missed on every run …
smysl q: data effect: 1 added, 0 removed
  + b3:zmxbp3rqt5b6xmbhlz3k36baj3
```
]

#section("Asking a question")

`ask` is experimental and says so on every run. It classifies a question, finds its subject
among the store's entities, and fills a *template* of slots (definition, characteristic,
exemplifies, counter-evidence, open question), each slot a saved query. The result is a dossier:
a view and a thread that can be rendered like any other (Chapter 24). With a provider it can
also write a grounded answer. Over the `information_theory` library:

#screen(caption: "cargo run --example information_theory   (§9, offline)")[
```
subject `Shannon`, 1 entity candidate(s)
  definition       0 unit(s)
  characteristic   1 unit(s)
  exemplifies      0 unit(s)
  counter          0 unit(s)
  open-question    0 unit(s)
```
]

#recap((
  [sq/1 is a staged pipeline: scope, source, filters, time, walks, order, shape. It is
   versioned, planned deterministically, and pure unless a stage needs a model.],
  [`expand` arrows follow what a unit rests on; `only` keeps just what the walk reached;
   `order by` takes several keys.],
  [`--save` keeps a query and its answers; `--rerun` reports the data effect since.],
  [`ask` builds a dossier of saved queries around a question's subject.],
))

#whatsnext[
  A query returns units. Some questions are about the text itself: how predictable it is, how
  much it repeats, how two texts differ. Those are measures.
]

// ─────────────────────────────────────────────────────────────────────────

#chapter(number: 35, title: "Measures")

A measure is a pure function of a store, a scope and a *recipe*, and its result is written back
as a `@data` unit whose identity is that result. Run it twice and you get the same unit, on any
machine. Its payload holds integers, not floats, so that a different libm cannot give a different
last bit.

#section("Why measures are units")

#callout(label: "Why")[
  *A number without its recipe cannot be compared with anything.* The recipe names the scope,
  the level the text was cut at, the analyzer, the seed and the minimum sample size, and it
  hashes to the same 32 bytes everywhere. A measure stored as a unit carries that recipe and is
  grounded on the scope it read, so a figure in a report can be traced to exactly what produced it.
]

#dtable(
  (auto, 1fr),
  (
    ([Family], [Measures]),
    ([Text (T)], [T1 unigram entropy (Miller–Madow), T2 compression rate, T3 match length, T4 Zipf and Heaps, T5 n-gram surprisal (Kneser–Ney, leave one segment out), T6 compression distance, T7 Jensen–Shannon divergence and collocations]),
    ([Graph (A)], [A1–A10 over a scope's units: density, redundancy, novelty, status entropy, grounding, concentration, contention, rate–distortion, entity communities, dating coverage]),
    ([Channel (C)], [C1–C5 between a source and the target it was carried into]),
    ([Dialogue (D)], [D1–D6 over a conversation]),
  ),
)

#screen(caption: "$ smysl measure T1,T2 --scope 'from collection:kjv' --min-n 1000 lib")[
```
T1 v2  b3:yvng2u7vv5zoawgb5qc3ptidsi  new
  T1 unigram entropy, Miller–Madow 7.6403 bits/token, over from collection:kjv
  n.segments = 4
  n.tokens = 2583
  n.types = 523
  H = 7.4945
  H:mm = 7.6403
T2 v1  b3:m5urn2exrdruxw7x6zpwasoynv  new
  T2 brotli rate 2.5767 bits/char, over from collection:kjv
  n.bytes = 13218
  n.chars = 13189
  n.compressed = 4248
  bits/char = 2.5767
  bits/word = 13.1568
```
]

The text measures want ten thousand tokens before they report. Ruth has 2,583, so
`--min-n` lowers the floor and the result carries a `low-n` flag. Over a scope holding a lossy
text, a measure refuses (`SMY-E480`) unless `--allow-lossy` admits it, and then says that it did
(`SMY-W408`). A graph measure that counts propositions needs `--policy`, and one that depends on
how finely the text was cut needs a granularity `--profile`. Each refuses by name until it gets
one.

`assess` is the companion for people: assessment keys, reader scores and knowledge states, with
agreement between readers reported as Krippendorff's α.

#recap((
  [A measure is a pure function of store, scope and recipe; its result is a `@data` unit, the
   same on every machine.],
  [Four families: text (T1–T7), graph (A1–A10), channel (C1–C5), dialogue (D1–D6).],
  [Minimum samples, lossy texts, class policies and granularity profiles are refused by name
   until the recipe states them.],
))

#whatsnext[
  A library is useful to more than its owner. The next chapter takes it apart into files and
  puts it together again elsewhere, with a check that nothing changed on the way.
]

// ─────────────────────────────────────────────────────────────────────────

#chapter(number: 36, title: "Export, Import and Verify")

`lib export` cuts a selection of a library into files by a *layout*: a single file, or one
per view, expression, collection, time window or query. Each file holds a generated view and
everything the selection needs, closed to a fixpoint. An index file records what went where.

#section("Why export is checked, not trusted")

#callout(label: "Why")[
  *An exchange format is only as good as its round trip.* Three properties are checked, by
  default in memory before anything is written. P1: what an import retains has the same
  record-set digest as what was exported. P2: an import exports again. P3: a re-export is byte
  for byte identical. A collection that fails one is not written.
]

#screen(caption: "$ smysl lib export --scope all -s lib -o out --layout by-expression --with-text")[
```
1 file(s), 19 record(s), rsd 8fc402341f1848bb93cbdc1ea56af41dd56a2481160053c02a2742541273db87;
P1 and P3 hold
  unkeyed.cbor  19
```
]

#screen(caption: "$ smysl lib verify out")[
```
1 of 1 file(s) match the index
P1 holds
P3 holds
```
]

#screen(caption: "$ smysl lib import out/export-index.cbor --accept -s lib2")[
```
import  out
  unkeyed.cbor  19 new, 0 already held: 19 accepted, 0 staged, 0 waiting, 0 quarantined
```
]

`--with-text` carries the parts' text where each manifest's `carry` and licence allow it. Without
`--accept`, an import is *staged*: `lib status` lists it, and `lib accept` or `lib discard`
decides. A record that references something not yet present waits; a record that fails its
checks is quarantined and reported, never silently dropped. A redacted part's text is dropped on
import (`SMY-W414`), so a redaction follows the data.

#recap((
  [`lib export` writes files by layout, closed under `full` or `ref`, with an index.],
  [P1 and P3 are checked before writing, and `lib verify` checks them again later.],
  [Import stages by default; records wait or are quarantined, never lost.],
))

#whatsnext[
  The last chapter of this part is about size: what changes when a library holds a hundred
  thousand units rather than a hundred.
]

// ─────────────────────────────────────────────────────────────────────────

#chapter(number: 37, title: "Scale, and the Library from Rust")

#section("Shards, the unit store and the index")

A library's records live in *shards*, one log per expression and one for the catalog, so that
appending to one chat does not rewrite the others. `--shard <alias>` reads only the shards
named, with references into the others *waiting* (`SMY-W500`) instead of dangling.

Two optional backends keep large libraries fast. Both are off by default because a small
library does not need them:

- *`store-redb`* puts the unit store on disk: cores stay in the log until asked for, behind
  the same `StoreRead` every command reads. Select it with `--backend redb`; `lib migrate` moves
  a shard between its log and the unit store.
- *`substrate-redb`* keeps the substrate index and its postings in `index/substrate.redb`. In
  1.14 the index stores a dense document id. At 155k units the file shrank from 540 MB to 85 MB,
  and a view opens in 1.5 ms instead of 17.5. A file written by an older version is rebuilt on open.

`text index` reports the index's tables, and `text index --rebuild` rebuilds them from the
catalog alone:

#screen(caption: "$ smysl text index -s lib")[
```
manifests   3 row(s)
parts       6 row(s)
by-part     6 row(s)
redactions  0 row(s)
segments    6 row(s)
locators    103 row(s)
align       0 row(s)
said        0 row(s)
composed    0 row(s)
observed    0 row(s)
postings    0 term(s) across 0 partition(s), analysed by nothing
journal     0 row(s)
```
]

#section("The same thing from Rust")

Everything in this part is reachable from the `smysl` crate (Chapter 27). Two examples in the
repository show it end to end and run offline:

- *`examples/simple_demo`* covers Parts I–VII in ten steps: parse, identity, CBOR, store
  and `check`, sq/1, salience, pack, render, withdraw, `fmt`.
- *`examples/information_theory`* covers this part in fourteen sections: a library of
  four texts about information theory, a pseudonymised chat, drawn units, a consensus text
  ingest against a scripted provider, same-as, datings and a reviewed lock, queries, a saved
  query, a dossier, eight measures, export and import, redaction, and `check`, which ends clean.

```
cargo run --example simple_demo
cargo run --example information_theory
cargo run --example information_theory --features store-redb
```

#callout(label: "Note")[
  A first program over a library tends to miss three steps, and the `information_theory`
  README names them:
  - declare `x.text/v1` and `x.query/v1` before using their edges (`SMY-W013` otherwise);
  - give `ingest_text` a clock;
  - give each dating a basis.
]

#recap((
  [Shards keep appends local; `--shard` reads a subset with cross-shard references waiting.],
  [`store-redb` and `substrate-redb` are opt-in backends for size; the 1.14 index is a sixth of
   its former size at 155k units.],
  [Both repository examples run offline and are the shortest route from this manual to working
   code.],
))

#whatsnext[
  The appendices list every command and flag (Appendix A), including `text`, `date`,
  `same-as`, `q`, `ask`, `measure`, `assess` and `lib`, and every diagnostic code (Appendix B),
  including the 4xx and 5xx ranges SMYSL-2 added.
]
