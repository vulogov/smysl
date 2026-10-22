# Fixtures

Eleven trees, with different jobs.

| Tree | Contents | Consumed by |
|---|---|---|
| `corpus/` | F1–F8 (§27.2): whole stores exercising a design property each | unit, property, and integration tests |
| `conformance/` | `.smy` / `.cbor` inputs plus the exact diagnostic set each must produce | the external conformance suite |
| `golden/` | canonical encodings, rendered artifacts, `--explain` output | golden tests |
| `library/same-as/` | `anthology.smy` — a paragraph reprinted under two expressions, and one that is not | `same-as propose`/`classes` in the determinism matrix (`xtask determinism`) |
| `sq/` | `accept/*.sq` with the tree and the canonical text each parses to, `reject/*.sq` with the one diagnostic each must produce, `plans/*.explain` with the plan a store yields, `plans/*.why` with the plan and the fused ranking together, `library/*.sq` with the `.golden` each yields over a library built from the Bible slices and chat exports | the `sq/1` grammar goldens (`crates/smysl-query/tests/grammar.rs`), the planner goldens (`tests/planner.rs`) and the library-layer goldens (`crates/smysl-text/tests/sq_library.rs`) |

`library/same-as/anthology.smy` holds three units and no part: a same-as engine reads which part,
which manifest and which bytes a unit *says* it came from, which is all A-2.4 lets a unit carry
about its source, so the ids are canonical and nothing behind them is in this repository. The
third unit is at other bytes of the same part, and it is there so that an engine proposing every
pair fails against the file rather than passing.

`wire/rsd/` holds the record-set digest's vectors (SMYSL-2.7 §5.4): `empty.cbor`, `f13.cbor` (F13
as CBOR) and `repeats.cbor` (F13 reversed, then again), with `cases.json` naming each one's record
count and digest. The last two have one digest, which is the property: order and repeats do not
count. The Rust writes them (`crates/smysl-lib/tests/cross_impl.rs`, `SMYSL_BLESS=1` to rewrite)
and `python/`, `nodejs/` and `go/` each recompute them in their own suite.

Six trees hold **gate material** rather than fixtures, and the difference is who they are
addressed to: a fixture is an instrument, and these are the things being measured.

| Tree | Contents | Consumed by |
|---|---|---|
| `chronology/` | 45 pairs of public dates whose order is a matter of record, `earlier`/`later`/relation/note | GE-T13's fourth clause (`crates/smysl-text/tests/ge_t13.rs`) |
| `library/bibles/` | `expected.tsv` and `slice.tsv`, the identities five scripture libraries build to; `slice/`, Ruth and Jude in five editions; `wire/`, one expression the three ports recompute | GE-T1's corpus clause (`crates/smysl-text/tests/ge_t1_bibles.rs`, `gen_bible_wire.rs`, and the three ports' library suites) |
| `library/link/` | `planted.tsv`, fifty pairs of claims that cannot both be true, each half cited in a different book | GE-T11 (`crates/smysl-text/tests/ge_t11.rs`) |
| `proposition/ge-t5/` | `paraphrase.tsv`, a second English expression of Ruth — forty restatements and ten near-misses; `sheet.tsv`, three scripted coders over the pool | GE-T5's within-language arm (`crates/smysl-text/tests/ge_t5.rs`) |
| `attribution/probes/` | Mark 14-15 in two editions of scripture, and `probes.tsv`: every utterance of Mark 15 in both, with the verse, the expected mode and how the source marks it | TX-P5 step 3b's probe-set gate (`crates/smysl-ingest/tests/probe_set.rs`) |
| `stability/corpus/` | `<lang>.txt` for de, en, es, fr and ru — the same three passages in each, 59 lines apiece | GE-T2's rerun under the consensus policy (`crates/smysl-ingest/tests/ge_t2.rs`) |
| `dossier/` | `subjects.smy`, people in Ruth and two women called Mary, each unit annotated with the slot that should find it; `gold/`, GE-T10's corpus (ten subjects in five languages), 150 questions and the gold for 118 (question, slot) pairs | `ask`'s unit tests (`crates/smysl-query/tests/dossier.rs`, `tests/cmd_ask.rs`) and GE-T10 (`crates/smysl-query/tests/ge_t10.rs`) |
| `measure/` | `golden.tsv`, `name<TAB>hex`: a recipe's canonical CBOR and hash, the payload of a hand-built value holding every number shape, a whole measure unit's uid and gist, six `libm` results by bit pattern and the measure generator's first draws | the measure frame (`crates/smysl-measure/tests/payload.rs`, TX-P8 M1); language-free, for the ports. `text.tsv`, `measure<TAB>key<TAB>value`: T1 and T4 over the KJV's Ruth and Jude at verse level — recipe hashes, sample sizes, values and intervals rendered from their stored integers, flags, gists and unit uids (`tests/text.rs`, TX-P8 M2). `compress.tsv`, the same layout: the codec's id and its compressed lengths for the four slice files, T2 over the KJV's Ruth and Jude at verse level and T6 over the four slice books with its linkage series (`tests/compress.rs`, TX-P8 M3) — a build whose `brotli` gained `std` fails it. `lz_match.tsv`, the same layout: T3 over the KJV's Ruth and Jude at part level, characters and terms, `min:n` 2 (`tests/lz_match.rs`, TX-P8 M4). `t3-calibration.tsv`, `bits<TAB>n<TAB>seed<TAB>q20(H)<TAB>q20(H:mr)<TAB>H<TAB>H:mr<TAB>blake3(payload)`: T3 on i.i.d. sources of 1 to 8 bits at 10³ to 10⁷ symbols, written by `examples/t3_calibration` and recomputed by `tests/lz_match.rs` — rows to 10⁵ by default, the rest `--ignored` in release. `kneser_ney.tsv`, `key<TAB>value`: T5 over the KJV's Ruth and Jude at verse level, order 3, `min:n` 2, its top ten verses by locator (`tests/kneser_ney.rs`, TX-P8 M5). `divergence.tsv`, the same layout: T7 over the four slice books at part level, JSD between each two and the top collocations by G² and PMI (`tests/divergence.rs`, TX-P8 M6). `g2.tsv`, `o11<TAB>o12<TAB>o21<TAB>o22<TAB>g2<TAB>q20(g2)`: Dunning's G² of eight 2×2 tables, written by `tests/divergence.rs` and recomputed by `python/tests/test_measure_g2.py`. `genres.tsv`, `code<TAB>genre`: GE-T6's labels for the 66 books, from SMYSL-2.6 §5.4's list, `-` for the four it does not place. `ge-t6.tsv`, `edition<TAB>measure<TAB>key<TAB>value`: GE-T6's pure arm over GE-T1's five Bibles — per edition and measure, the books kept and excluded, Kruskal–Wallis H, ε², p, the variances the kill criterion compares, and the genre medians with intervals (`crates/smysl-measure/tests/ge_t6.rs` under `SMYSL_GE_T6`, TX-P8 M7) |

`library/bibles/slice/` is 112 KiB of public-domain scripture, carried because a two-platform
comparison that needs three megabytes mounted first is a comparison nobody runs. The whole corpus
stays outside: `SMYSL_GE_T1=<dir>` points the harness at it, and `SMYSL_CHRONOLOGY=<file>` replaces
the chronology with somebody else's.

`stability/corpus/` is GE-T2's, and it answers the development-set objection the way the probe
set does: the passages are **not chosen here**. They are the S0 spike's (SMYSL-2.2), fixed before
this code existed, and the rule that takes three of its eleven is mechanical — every reference all
five tier-1 languages carry. What that buys is a corpus identical in shape across the five:
26 verses of narrative, 26 of law, 7 of poetry, so the only thing varying between five reported
figures is the language. It needs a credential as well as material, so `SMYSL_INGEST_LIVE=1` is
required and the harness reports the clause as owed without one; `SMYSL_GE_T2=<dir>` and
`SMYSL_GE_T2_MODEL=<name>` replace the corpus and the model. See that directory's README for
editions, URLs and hashes.

`proposition/ge-t5/` is two files with opposite standing, which is why they are in one directory.
`paraphrase.tsv` is **material**: forty rows restate one verse of the King James Ruth each in
modern English and ten sit at a locator one of those forty already uses, saying something else in
much the same words. The `lexical` engine exists for two expressions of one language and the
bibles slice holds one edition per language, so without this file there is no within-language pair
in this repository to measure. Its gold is of record by construction — a row's `relation` is what
its author wrote it to be — and `SMYSL_GE_T5_PARAPHRASE=<file>` replaces it.
`sheet.tsv` is **not** material: it is three scripted coder columns, the pool's constructed truth
with disagreement written in at fixed intervals chosen so that α comes out at S0's own 0.600. It
exists so the instrument runs on every `make test`, and the harness reports GE-T5's clause as
*owed* rather than met whenever it is the sheet in use. Three coders judging independently is what
is owed; `SMYSL_GE_T5_SHEET=<file>` is how somebody else's sheet decides the clause.

`attribution/probes/` is the probe set TX-P5 step 3b exits on, and it is **material** with the
authored part removed rather than defended. Two editions of Mark 14-15 came from eBible's USFM
distributions, which is not where `library/bibles/slice/` came from: that slice was built out of
eBible's *verse-per-line* files, which carry no markup at all, which is why its `eng-kjv/` has no
`\wj` in it and why a probe set could not be drawn from it. The two editions carry the same 47
verses, one marking the words of Jesus and punctuating nothing (the 1769 text) and one marking them
and punctuating every quotation (the World English Bible), so the two arms of the gate differ in
nothing but typography. **Every** utterance of Mark 15 is probed in both, one probe each and at the
verse it begins in, so no verse was chosen — the chapter was, for the reason that the Passion
narrative is where ten speakers take their turn, from Pilate to the centurion. A probe's `expected` is read off the verse's frame (a verb of saying attributing the clause
to somebody, or nothing) and its `marking` off the source file, so both labels are checkable by
opening the file; its `locator` is load-bearing, because a short quote matched loosely attaches to
the wrong verse. Chapter 14 is carried and not probed: it is what makes chapter 15's probes read
out of a part that does not begin at byte zero, which is how the set found that a row's marks were
not being shifted with the row. `SMYSL_PROBE_SET=<dir>` replaces the whole directory.

`library/link/planted.tsv` is the other file here a person wrote from nothing, and its header
says so before it says anything else. GE-T11 asks for fifty *far* contradictions, which no corpus
supplies ready-made — a text that contradicts itself two books later does so by accident and not
on request — so they are planted, in the idiom of `library/bibles/slice/eng-kjv/` and at locators
in two different books. A recall measured over a set its own author wrote is a statement about the
instrument; `SMYSL_GE_T11_PLANTED=<file>` and `SMYSL_GE_T11_CORPUS=<dir>` are how it becomes a
statement about the linker.

`dossier/gold/` is the third, and the largest. GE-T10 asks for expert-filled slots, and the
project has none to ask, so one hand wrote the corpus, the questions and the gold, after the
templates they measure. That is said at the head of each file, and the criteria the gold follows
are written at the head of `slots.tsv`. Units are placed in a slot by what they *say*, not by
the kind or edge a template selects on, and the cases where those come apart are planted and
marked `PLANT` with the failure each probes. The questions were written as a person would ask
them and were not rephrased after a run, so a question whose subject Russian inflects away from
its entity's spelling stays inflected and is counted. `SMYSL_GE_T10_{CORPUS,QUESTIONS,GOLD}`
take somebody else's set.

`sq/` is the grammar's conformance suite (RFC SMYSL-2.9 §5) and it is **external** for the
reason the conformance tree is: three more implementations of this grammar are coming — the
Python, Go and JavaScript ports SMYSL-2.5 §3.1 writes the parser for — and they validate against
these artifacts rather than against a Rust test. Each accept case carries two expectations, which
fail for different reasons: `.ast` is the tree, so a parser change shows up there, and `.sqc` is
the canonical text, so a printer change that leaves the tree alone is still a reviewable diff.
The `.ast` dump leaves spans out, because a span is a byte offset and every expectation would
churn on an edit to the whitespace above it; spans are asserted in `reject/*.diag`, which carry
one each along with the code and the message. The reject set holds a near-miss of every accept
case on purpose: in a hand-written recursive-descent parser the risk is never ambiguity, it is
unintended acceptance, which no accept case can detect. `SMYSL_BLESS=1` rewrites every
expectation and should be a decision rather than a habit.

`sq/library/` is TX-P6 step 6's exit: one query per capability the library layer enables —
the six scope items, catalogued partitions with their chain ids, `lang:<code>`, the four time
predicates on both axes, and the entity bridge — each with the `.golden` it yields. There is
no `.smy` beside them because no committed store holds units a library can place: the test
builds the library from `library/bibles/slice/` and `library/readers/`, draws one unit per
verse and per message with the provenance an extraction would stamp, and plants two entities
with the `mentions` and `same-as` edges one would write. The golden is therefore a function of
the readers, which GE-T1 pins, and `SMYSL_BLESS=1` rewrites it.

`sq/plans/` is the planner's goldens (TX-P6 steps 2 and 3), and each `.explain` is the text for
`from store:<fixture>` over the matching file in `corpus/`. The snapshot is the explain output
rather than a field-by-field assertion because SMYSL-2.5 §3.2 makes each planner phase record
what it did *so that* `--explain` can print it: what is worth pinning is the whole record, and
a golden a person can read is a golden a person will notice is wrong.

`sq/plans/supersession.smy` is the second input here that is not in `corpus/`, and for the
same reason: none of F1 to F13 supersedes anything, so the supersession pass of SMYSL-2.5 §3.2
had nothing to act on. This file is the smallest store that exercises both of its arms at once
— a chain of three, a fork where two units revise one target with nothing ordering them, and a
claim nobody revised, so the pass can be seen to resolve, to decline to resolve (`SMY-W464`),
and to leave alone.

`sq/plans/find-lexical.why` is the one golden here that is not a plan alone. Step 3's
`--explain` gains a phase-5 line, and none of the `.explain` snapshots can show it: they are
`from store:<fixture>` with no source, so there is nothing for phase 5 to lower. This one is
`find "connection pool" ~lex` over F1, and it pins the `--why` text beside the plan — the
partition ranks, the fused score and the per-term contributions behind each — because those
numbers are what makes a ranking auditable and a change in any of them is a change in the
answer.

`sq/plans/catalog.smy` is the one input here that is not in `corpus/`. None of F1 to F13
carries a manifest, so every snapshot taken over them exercises only the uncatalogued branch of
phase 4; this file is one manifest with two parts, the second declaring `lang: ru`, and three
units — one per part and one sourcing no part — so a single snapshot shows all three outcomes
phase 4 can produce. It is here rather than in `corpus/` because that tree is F1 to F13 with an
`.expected` beside each and four other crates iterate it, so a fourteenth file would change
what their tests measure.

The conformance tree is deliberately **external** — fixture files plus expected
diagnostics, no Rust — so a second implementation in another language validates against
the same artifacts.

## Expected-diagnostic format

Each fixture `NAME.<ext>` has a sibling `NAME.expected` listing one diagnostic code per
line, in no particular order. An empty file means the input MUST be accepted with no
diagnostics at all. The set is exact: producing an extra code fails the fixture just as
producing too few does (§26, SM-P4 gate).

## Regenerating

`conformance/codec/` is machine-generated by `generate.py` in that directory, and the
outputs are committed so the suite is usable with no Python in the loop.

`library/bibles/` is generated too, and in two steps. `vpl2usfm.py` turns eBible's
verse-per-line distribution into USFM, one file per book — the mapping of the older Paratext
book codes (`JOH`, `MAR`, `JAM`, `SOL`) is the only judgement in it, and it carries the 66-book
canon. The expectations and the port bundle are then written by the harnesses themselves under
`SMYSL_BLESS=1`, which should be a decision rather than a habit: it changes what two platforms
and three ports are compared against.

Everything else is authored by hand.
