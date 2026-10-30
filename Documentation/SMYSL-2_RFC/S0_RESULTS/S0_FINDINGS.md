# S0 findings log

Written as they are found, against crate 1.9.0 (`50c79bf`, TX-P0 complete except F-2).
RFC SMYSL-2.2 was written against `d25ec9e`, before TX-P0, so some of its observations are
already resolved and some of its stated facts no longer hold. Both kinds are recorded.

## 0. The headline: Russian ingest is substantially degraded at 1.9.0, and F-2 is why

Measured, and the mechanism is closed end to end:

All figures are **arm R** (`deepseek-chat`), which is where the degradation is; §0-bis explains
why arm L's zero is not virtue. Complete corpus, 50 runs per language.

| | English | Russian |
|---|---|---|
| `SMY-E022` "gist exceeds l0_max" | 12 | **533** |
| `SMY-W304` unit degraded to opaque prose | 4 | **152** |
| units lost to opaque prose | 0.29% | **12.67%** |
| runs degrading at least one unit | 8% | **80%** |
| repair calls above one per chunk | 13.8% | **64.0%** |
| tokens per 1,000 source words | 5,892 | **19,925** |

The chain: a Russian gist exceeds the bound, `E022` fires, three repair attempts fail on the
same unit, and the proposition is replaced by one opaque `@prose` unit. One Russian proposition
in eight is lost this way, **forty-four times** the English rate, and the retries are what makes a
Russian chapter cost 3.4x an English one.

T14 compares measured tokens with a planning figure. The threshold text points at draft 3 §7.7,
which is not in the repository, but §6 of this RFC states its own: "≈ 1.6k in (≈ 600 prompt +
text) + ≈ 1.2k out, ×1.3 for repairs" per **English** chapter, and says outright that "the
measured figures replace them in `REPORT.md`". T14 is decided against that, with the scope stated:
the figure was only ever given for English.

| arm-lang | tokens/run | against 2.8k | against 3.64k (incl. the ×1.3 repair allowance) |
|---|---|---|---|
| L-en | 3,120 | 1.11x | 0.86x |
| R-en | 3,982 | 1.42x | 1.09x |
| R-de | 6,063 | 2.17x | 1.67x |
| P-ru | 8,930 | 3.19x | 2.45x |
| R-ru | 11,098 | **3.96x** | **3.05x** |

English is inside the plan on both readings. **T14 fires on Russian either way** — the only cell
whose verdict depends on the repair allowance is R-de. So the row's conclusion is robust to the
ambiguity, and the cost model needs revising for non-English before TX-P5 is sized.

What M5 asks for separately is also decidable, and is a clean FC-6 result:

| ru/en cost ratio | arm L | arm R |
|---|---|---|
| from the model tokenizer | 1.84x | **3.38x** |
| from `ceil(bytes/4)` | 1.96x | 1.96x |
| estimator error | 6% over | **73% under** |

The estimator is well calibrated against a model that does not enter the repair loop and
under-states the hosted model's Russian cost by 73%. The difference is the repair loop itself,
not tokenization.

**The cause is the byte-based estimator, not the prompt.** H-5's per-script gist bound is
correctly calibrated — the enforced limit is 30 estimator tokens = 120 bytes, and measured
bytes-per-character give the effective character limits below:

| | bytes/char in gists | effective limit | prompt says | mean gist | headroom used |
|---|---|---|---|---|---|
| English | 1.03 | 116 chars | 120 | 60.8 chars | 52% |
| Russian | 1.83 | **65 chars** | 60 | 49.1 chars | **76%** |

So the instruction matches the limit in both scripts. What differs is the headroom: an English
gist uses half its allowance, a Russian gist three quarters, and ordinary variance then pushes
Russian over. 14.9% of the Russian gists that *survived* already exceed 60 characters, against
54.3% of English gists that exceed 60 with 116 available. A
65-character Russian gist also carries less content than a 117-character English one, which is
the inequity F-2 exists to remove.

This is a measured cost of deferring F-2 to 1.10.0, and it is the spike's main result for
SMYSL-2.1. It does not depend on any judged label.

## 0-bis. The two TX-P0 features are in tension, and F-2's absence is the tension

Every one of the 533 `E022` and 152 `W304` above is **arm R**. Arm L has none — and the reason
is not that qwen2.5 writes shorter Russian gists. It is that **qwen2.5 does not write Russian**:

| | Russian gists | in Cyrillic | in Latin | mean bytes/char |
|---|---|---|---|---|
| arm L, `qwen2.5:14b-instruct` | 585 | 125 | **460 (78.6%)** | 1.13 |
| arm R, `deepseek-chat` | 1,200 | **1,200 (100%)** | 0 | 1.83 |

Arm L's Russian gists read `The release plan is to proceed with the build completed at 06:10.`
against Russian input. That violates the `LANG_RULE` F-6 and D-10 added — "Write each gist and
body in the language of the passage it comes from" — in 78.6% of cases.

So the byte-based bound **punishes exactly the model that obeys the language rule**. Writing
Cyrillic costs 1.84 bytes per character against a 120-byte ceiling; translating to English costs
1.13 and always fits. Obeying F-6 causes `E022`; violating it avoids `E022`.

That is a direct conflict between two things TX-P0 shipped (F-6's language policy, H-5's
per-script bound) and one it deferred (F-2's script-aware estimator). It is an argument for
moving F-2 ahead of further ingest work rather than leaving it in 1.10.0: until it lands, the
tree rewards the wrong behaviour.

### 0-ter. Both models break the language rule, in opposite directions
`deepseek-chat` obeys it on Russian and breaks it on English: for `en-kjv1769-Jas.2` it wrote
**every gist in Chinese** in runs 2, 3, 4 and 5 — 80 units, 100% of those runs — while run 1 was
English. 80 of 1389 English gists (5.8%), all from that one input.

A CJK gist is bounded at 40 characters rather than 120, so these pass the length check while
being in entirely the wrong language. The script-aware bound H-5 added makes a wrong-language
gist *easier* to fit, not harder.

A language-policy check belongs in the tree, not only in the prompt: `lang-policy` is currently
an instruction with no verification, and nothing in `smysl ingest` notices that a Russian
passage produced English records or an English passage produced Chinese ones. The spike's
`attrib.py` detects it with a character-range count, which is cheap enough to be a diagnostic.

### 0-quater. `deepseek-chat` is not deterministic at temperature 0
Five identical invocations of `en-kjv1769-Jas.2`, same recipe, `temperature: 0.0`:

| pair | J_uid | shared units |
|---|---|---|
| r1 vs r2-r5 | 0.000 | 0 (run 1 English, the rest Chinese) |
| r2 vs r3 | 0.333 | 10 of 20 |
| r2 vs r4 | 0.053 | 2 |
| r4 vs r5 | 0.333 | 10 |
| r3 vs r5 | 0.026 | 1 |

The protocol expected the opposite problem — §7 warns that "Local T = 0 is bit-identical, so J
within L = 1 means nothing. Arm R carries Q1." Arm R does carry Q1, and the answer is that a
hosted model at T = 0 does not reproduce its own output. Anything resting on hosted
reproducibility — rule D's `--seed-check` promise for a `Model`-purity command is already
honest about this (H-19 refuses) — should keep assuming it cannot.

## A. Defect candidates found by the spike

### A-1. The ingest system prompt names a fence the user message does not use
`content_ingest_surface_sourced` v3 opens: "Everything between the two `<<<SMYSL-INPUT>>>`
markers is material to describe." D-10 made the real marker per-input — the user message
carries `<<<SMYSL-INPUT-110e3cc0d868d422>>>` — but `Template::render` substitutes `FENCE` in
`self.user` only, so the system prompt still describes the pre-D-10 literal. The model is told
to find a delimiter that is not present.

Observed live: arm L, first call, system prompt 2468 bytes naming `<<<SMYSL-INPUT>>>`, user
message opening and closing with `<<<SMYSL-INPUT-110e3cc0d868d422>>>`.

Impact: the derived fence still works, because the model reads the delimiters structurally and
the `<<<SMYSL-INPUT` prefix matches. But the instruction is false as written, and D-10's whole
purpose is that the model should trust the marker rather than the shape. A forged
`<<<SMYSL-INPUT>>>` inside a document now matches the *described* marker exactly while the
*real* one differs, which is the confusion D-10 set out to remove.

Fix: render the system prompt through the same substitution, so both name the derived marker.

### A-2. V2 `Present` spans can include trailing deleted typography
`support_span_with(V2, "нет Бога", <Psalm 13:1>)` returns `99..117`, which slices to
`нет Бога”` — the closing curly quote is inside the span. V1 returns `99..114`, exactly the
quote. The end sentinel is `at(i + q.len())`, the source offset of the next *retained*
character, so every character V2 deletes between the match end and the next kept character is
swallowed.

Measured over the corpus: **4 occurrences in 3066 comparable `Present` spans**, and in every
one the overshoot is a single trailing `”` (3 bytes). The head is never wider. So the defect is
exactly "trailing characters the fold deletes are included", and it is rare because it needs a
quote ending immediately before deleted typography.

Impact: cosmetic. A caller highlighting a quote highlights one character too many. It is an
asymmetry V1 does not have and the span contract does not mention.

### A-3. F-3's span fix holds at corpus scale (a positive result)
Every span the library returned over the whole corpus was checked for character-boundary
validity and sliced: **3066 V1 spans and 3066 V2 spans, zero invalid, zero panics**. The defect
F-3 fixed — a span pointing inside a character — does not recur on real model output in five
languages, including the Cyrillic text where multi-byte characters are the rule rather than the
exception. `bin/spancheck.py` is the check and it is cheap enough to keep.

## B. RFC SMYSL-2.2 statements that do not hold against 1.9.0

### B-1. E-10's `ё` example is inverted for the chosen edition
E-10 states: "a quote `…в сердце своем` against source `…в сердце своём` is **`Absent`**". In
the eBible `russyn` module the source reads `своем`, without `ё` — so that quote is `Present`.
Verified: `v1=present span=Some(68..94)`.

The live direction is the opposite one, and it does occur: the model writes standard modern
orthography, `в сердце своём`, against a source spelling `своем`. That gives `v1=absent`,
`v2=present` — which is the T10 case, reached from the model side rather than the source side.

### B-2. OQ-3 resolved: the Synodal module is effectively `ё`-less
The whole `russyn` Bible contains **one** `ё` and no `Ё`. Across the spike's ten Russian inputs
there are 11, all of them in P9 (Wikisource) and P10 (written for the spike). So the F-3 Russian
measurement rests almost entirely on model-introduced `ё`, not on source orthography.

### B-3. The module uses curly quotes, not guillemets
`russyn` uses `“ ”` (936 occurrences) and no `« »` at all. V1 already folds `“ ”`, so the
guillemet half of V2's table — and of the protocol's V2-emu list — is never exercised by this
corpus. `нет Бога` inside `“нет Бога”` is `Present` under both normalisers, as E-10 says.

### B-3-bis. T10 does not fire, and V2 rescues nothing on this corpus
M3b over every quote of every attempt behind the proxy: **769 quotes, 9 `Absent`, 0
false-`Absent`** — a V1 false-`Absent` rate of 0.00% against T10's 1% threshold. Not one
`Absent` quote becomes `Present` under the real V2.

Inspecting all of them shows why. Every `Absent` verdict is a genuine misquote, and each is a
small lexical or grammatical substitution rather than a normalisation difference:

| input | source reads | model wrote | kind |
|---|---|---|---|
| Gen 4:10 ru | `голос крови брата твоего` | `братья твоего` | wrong case and number |
| Lev 19:4 ru | `Не обращайтесь` | `Не обращайся` | plural imperative made singular |
| Jas 2:6 ru | `Не богатые ли притесняют` | `Не они ли притесняют` | "the rich" replaced by a pronoun |
| Ecc 1:18 ru | `потому что` | `поскольку` | synonym substituted |
| 1 Kgs 3:20 en | `And she arose at midnight` | `And I rose at midnight` | **person changed** |

So on the A1 scale of §3.5 M3c this corpus is 5 of 5 **paraphrase**, with no normalisation, no
translated quote, no cross-verse join and no fabrication. The quote check is behaving precisely:
its `Absent` verdicts are all true positives.

The last row is worth keeping. The model turned one woman's testimony about the other woman
into a first-person claim, inverting the holder *inside the quote* — and the quote check caught
it as a side effect of byte comparison, which is the only mechanism in the tree that noticed.

Read with B-2, the conclusion for F-3 is narrow and should be stated plainly: V2 is sound and
its spans are valid (A-3), but **this corpus does not exercise it**, because the chosen Synodal
edition has no `ё` and already uses quote marks V1 folds. V2's value remains argued rather than
measured, and a corpus that would measure it needs an edition with `ё` in it.

### B-4. V2-emu is unnecessary
§3.5 M3b specifies a hand-rolled "V2-emu" that pre-folds strings and reruns V1, because
normaliser V2 did not exist. F-3 shipped it in 1.9.0, so the harness calls
`quote_support_span_with(QuoteNormaliser::V2, …)` and M3b reports the real V2. T10 therefore
tests FC-7 directly instead of estimating it.

### B-5. Nothing in the corpus reached the whole-chunk degradation path
E-9 describes a chunk degrading to one `@prose` unit carrying `"ingest:unrepaired": true`
(`SMY-W304`). Over 178 collected runs **no unit carries that flag**: all 170 prose units come
from the *unit-local* `E022` path, and `W304`'s message in these cases reads "unit degraded to
opaque prose: its own error after 3 attempt(s)". So `W304` is emitted per degraded unit here,
not per degraded chunk, and the chunk-wide path E-9 describes was never triggered.

Consequence for anyone reusing the protocol: excluding degraded units by the `ingest:unrepaired`
payload key alone would exclude nothing. The spike filters on `schema == prose` as well.

### B-6. P9 carries no paragraph alignment
§3.2 implies the essay aligns like the Bibles. It does not: Maude splits paragraphs the Russian
original keeps whole, and the two diverge after about paragraph 12. §3.5 draws cross-lingual
pairs only from P1, P3 and P5, so P9 needs none; each language is taken to the same word budget
independently.

## B-bis. Observations that are not defects but matter to whoever sizes TX-P5

### O-7. Degradation is invisible in the exit code
47 of the collected runs degraded a chunk to one opaque `@prose` unit (`SMY-W304`) — the whole
chapter's propositions thrown away — and **every one exited 10**, identical to a clean run.

This is correct per the contract: `StagedWithCorrections` (11) is documented as "staged, **and
rule M lowered at least one unit** (`SMY-W036`)", and `weakened` is 0 throughout the corpus. So
the exit code reports status-lowering, not content loss.

It is still worth stating, because the reasoning recorded for exit 11 — that a corrected batch
was "the one outcome most worth knowing about" being "indistinguishable from nothing having
happened" — applies at least as strongly to a degraded one. A pipeline that routes on the exit
code sees nothing. Today the signal is on stderr (`N degraded`) and in `--json`.

Not filed as a defect: the contract is explicit, and changing it is a compatibility decision,
not a bug fix.

## C. RFC observations confirmed resolved by TX-P0, live

| obs | what it said | state at 1.9.0 |
|---|---|---|
| O-1 | `-C/--config` declared and never read | resolved (H-8); the per-run layout is kept anyway, because the *typed input path* is what every uid carries |
| O-2 | recipe hashes an empty model | resolved (H-9); `usage.log` records `"model":"deepseek-chat"`, where E-12 documented `"model":""` |
| O-3 | Ollama never receives `num_ctx` | resolved (H-10); observed live: `options: {num_ctx: 32768, num_predict: 8192, temperature: 0.0}` |
| O-4 | repair diagnostics dropped on success | resolved (H-11); `SMY-W435` reports each repaired attempt, so the proxy is now corroboration rather than the only record |
| O-5 | no CLI temperature | resolved (H-12); arm T needs no patched build |
| O-6 | `--yes` help overstates what it does | resolved (H-13) |

## D. Protocol assumptions that checked out

- Exodus 20 is 26 verses in all five editions, Synodal included (§3.2 flagged this unverified).
- Synodal Psalm 13 has exactly the 7 verses of KJV Psalm 14, so the P5 versification pair holds.
- Every input is one chunk: the largest is 4913 estimator tokens against a 15,360 budget.
- `--path surface` and the sourced template are selected as §3.3 pins them.

## MS — the measured set, on the complete corpus

365 runs: arm L 145, arm R 145, arm H 29, arm P 16 (the 335 the protocol specifies)
and arm T 30, which §3.3 makes optional and which MS-2 qualified us for. 6,728 units,
5,560 quotes, 1,118 diagnostics.

### MS-1 No model coder, hosted or local, reaches the protocol's reliability floor

§3.6 sets α ≥ 0.667 and makes α "the ceiling every model figure is reported against (GE-T9)".
The sample is one shared stratified draw of 1,500 pairs from the 11,602 the corpus yields,
seed 20261007, so every coder judged the same pairs.

Three coders in the end: two hosted (`gemini-3.5-flash`, `deepseek-chat`) and one local
(`qwen2.5:14b-instruct`, the arm L model), all on the same 1,500 pairs.

| coders | binary same / not | six labels |
|---|---|---|
| **all three** | **0.6004** [0.5500, 0.6461] | **0.3297** [0.3008, 0.3563] |
| Gemini vs DeepSeek | 0.5441 [0.4820, 0.6050] | 0.3633 |
| Gemini vs Ollama | 0.6236 [0.5612, 0.6832] | 0.2370 |
| DeepSeek vs Ollama | 0.6362 [0.5760, 0.6868] | 0.3607 |

**No coder is the outlier.** The local 14B model agrees with each hosted model slightly better
than the two hosted models agree with each other, and all four binary figures sit in a narrow
0.54–0.64 band below the floor. The ceiling is the task, not a provider.

The three-coder CI's upper bound is 0.6461, below 0.667, so by §3.7 T1 is a point decision and
not *inconclusive*. One sensitivity is worth stating: taken alone, the two best-agreeing coders
(DeepSeek and Ollama, 0.6362) have a CI that does straddle 0.667, so a two-coder study that
happened to pick that pair would have had to report T1 inconclusive. The three-coder estimate is
the one reported, and `data/alpha-pairwise.json` keeps the rest.

Raw agreement between the hosted pair is 61.7% (922/1494). The disagreement is concentrated where
it matters least — 277 cells are *different* vs *related*, a boundary the same-as question never
asks about — but 91 cells are DeepSeek *same* against Gemini *related*, which does bear on it.

The coders behave validly against the pool's own structure, so this is a reliability result and
not a broken instrument: *same* runs 7.1% on random within-input controls, 18.6% on candidates and
23.0% on cross-lingual pairs, the last being highest exactly as "translation equivalents are same"
predicts.

**Consequence.** An LLM judge does not substitute for the two human annotators of §3.6. Every M2
figure below is provisional and carries this ceiling.

### MS-1b A single model coder does not agree with itself at the floor either

A second DeepSeek pass over the same 1,500 pairs in a different order (seed 99), which is
within-coder reliability rather than between-coder:

| comparison | six labels | binary same / not |
|---|---|---|
| between coders, Gemini vs DeepSeek | 0.3633 [0.3245, 0.4018] | 0.5441 [0.4820, 0.6050] |
| within one coder, DeepSeek reordered | 0.4679 [0.4303, 0.5023] | 0.5838 [0.5314, 0.6341] |

Within-coder is only marginally above between-coder and is itself below 0.667 on both scales. The
shortfall in MS-1 is therefore not two providers placing the *same* boundary differently: the
same-as judgment is intrinsically unstable for these models. §3.6's two human annotators are
load-bearing and MS-1's consequence stands at full strength.

Stored separately because `alpha.py` writes fixed filenames: `data/alpha{,-six}.json` hold the
three-coder figures, `data/alpha-pairwise.json` the three pairings, `data/alpha{,-six}-within.json`
the within-coder ones and `data/alpha-m4.json` the M4 labels.

### MS-2 M1/M2 split the two arms completely

| measure | arm L (`qwen2.5:14b-instruct`) | arm R (`deepseek-chat`) |
|---|---|---|
| M1 J_uid, median over inputs | **1.000** every language, every genre | 0.135 – 0.547 |
| M2 J_class, median over 12 judged inputs | **1.000** | **0.314** |
| decision row T2/T3/T4 | continue (≥ 0.6) | **reorder phases** (< 0.4) |

Arm L is bit-identical at T=0 — 145 runs, every input, J_uid exactly 1.000 — which is what §3.3
gates the optional arm T on, so arm T was added (5 runs × 6 judged English inputs at T=0.7).
Arm R at temperature 0 does **not** reproduce its own output. §7 expected the opposite: that local
T=0 would be trivially perfect and "arm R carries Q1". Q1's answer is therefore poor, not perfect,
and it is poor for the hosted arm specifically.

### MS-3 Semantic cosine cannot propose same-as classes (T6/T7)

Precision at recall 0.7, under two provisional golds: majority of the two hosted coders, and
majority of all three. The second is the better gold and is the one reported.

| set | proposer | n | precision, 2-coder gold | precision, 3-coder gold | bar |
|---|---|---|---|---|---|
| candidate | C2 cosine | 385 | 0.634 | **0.789** | 0.9 |
| candidate | C3 ∧ C2 cosine | 198 | 0.638 | **0.835** | 0.9 |
| cross | C2 cosine | 187 | 0.352 | **0.427** | 0.9 |
| cross | C3 ∧ C2 cosine | 85 | 0.596 | **0.712** | 0.9 |

A better gold raises every cell, by up to 0.2 — which is itself evidence that the two-coder gold
was adding noise rather than signal — and **not one cell reaches 0.9 under either gold**. T6 and
T7 therefore fire robustly, independent of how the provisional gold is formed. J_class moves
equally little: arm R 0.314 under the two-coder gold, 0.327 under three, against a 0.6 threshold.

Adjacency (C3) is worth more than it first appeared: under the three-coder gold it lifts
same-language precision 0.789 → 0.835 and cross-lingual 0.427 → 0.712, the latter by shrinking a
candidate set that cosine alone ranks badly across languages.

### MS-4 F-3 holds at corpus scale; the V2 overshoot is confirmed and narrow

5,560 spans under each normaliser: **5,560 valid under V1 and 5,560 under V2, zero invalid**. The
regression F-3 fixed does not recur across 290 runs. V2's Present span is wider than V1's in 10 of
5,560 comparable cases, all of them `ru-syn1876-Ps.13`, and all the same shape: V2 swallows the
trailing `”`. This is finding A-2, now with a corpus-scale rate (0.18%).

### MS-5 The language rule is broken in both arms, in opposite directions

Confirmed at full scale (gist script against input language):

| arm | lang | wrong script | gists | rate |
|---|---|---|---|---|
| L | ru | 460 | 585 | **78.63%** |
| R | en | 80 | 1,389 | 5.76% |
| both | all others | 0 | — | 0.00% |

`qwen2.5:14b-instruct` answers Russian input in English; `deepseek-chat` wrote every gist for
English James 2 in Chinese on runs 2–5 (80 units, 100% of those runs; run 1 was English). The
`lang-policy` D-10 added is an instruction with no verification: nothing in `smysl ingest` notices
either case. A character-range count is cheap enough to be a diagnostic.

### MS-6 The byte ceiling punishes the model that obeys the language rule

Of the corpus's 579 `SMY-E022`, 554 are arm R and 533 of those are Russian; arm L has **none**.
Arm L's zero is not virtue: it is MS-5's 78.6% English-for-Russian, which costs 1.13 bytes/char
against the 120-byte `l0_max` where Cyrillic costs 1.84 and overflows. Obeying F-6 causes `E022`;
violating it avoids `E022`. Conversely a CJK gist is bounded at ~40 characters, so H-5's
script-aware bound makes a *wrong-language* gist easier to fit.

This is the F-6 / H-5 / F-2 tension. It argues for moving F-2 ahead of further ingest work rather
than leaving it in 1.10.0 behind A-9 and OQ-31.

Arm R Russian also shows repair at 178% of chunks — more repair calls than chunks — against
13–16% for every other arm/language cell.

### MS-7 T10 is clear, and V2 changes no verdict anywhere in the corpus

FC-7 treats V1's false-`Absent` on Russian as a precondition, and T10 fires above 1% of quotes.
Measured over the complete corpus:

| lang | quotes | V1 `Absent` while V2 finds it | rate |
|---|---|---|---|
| en | 2,265 | 0 | 0.00% |
| ru | 1,623 | 0 | 0.00% |
| de / es / fr | 515 / 552 / 605 | 0 / 0 / 0 | 0.00% |

**T10 does not fire.** V1 returns `Absent` not once in 5,560 quotes: the verdicts are 5,506
`Present`, 54 `Loose` (`SMY-W308`) and 171 units with no quote at all. The V1 false-`Absent`
behaviour FC-7 was written against does not reproduce at 1.9.0, which is consistent with F-11
and OQ-3 having been resolved in TX-P0.

The stronger observation is about V2 itself. Across all 5,560 quotes **V1 and V2 never return a
different verdict** — the disagreement set is empty. Their only corpus-wide difference is span
width, in the 10 `ru-syn1876-Ps.13` cases of MS-4 (0.18%), and in those V2 is the *wrong* one: it
swallows the trailing `”`. On this corpus the second normaliser buys no verdict and costs one
span defect. That is evidence for the narrower reading of A-8.1 and against making V2 a default
before the overshoot is fixed.

### MS-8 Arm T: the local model's determinism is entirely an artifact of T=0

§3.3 makes arm T optional and gates it on arm L being bit-identical, which MS-2 established. Five
runs of each of the six judged English inputs at temperature 0.7, same build, same prompt
(H-12, so no patched binary):

| input | J_uid at T=0 (arm L) | J_uid at T=0.7 (arm T) | units per run |
|---|---|---|---|
| en-cc0chat-Chat.1 | 1.000 | **0.000** | 4 – 19 |
| en-kjv1769-Ex.20 | 1.000 | **0.000** | 10 – 26 |
| en-kjv1769-Gen.4 | 1.000 | **0.000** | 1 – 28 |
| en-kjv1769-Jas.2 | 1.000 | **0.000** | 9 – 25 |
| en-kjv1769-Ps.14 | 1.000 | 0.036 | 10 – 15 |
| en-maude1899-Art.3 | 1.000 | **0.000** | 1 – 12 |

Not one uid in common between runs on five of six inputs, and the unit count swings by up to 28x
on one input. Arm L's perfect M1 is therefore a property of greedy decoding and not of the local
model, the local prompt or the pipeline. Any claim that local extraction is "stable" holds only
at temperature 0, and `temperature` is consequently a correctness-relevant setting rather than a
quality knob — it belongs in the recipe and in the uid's provenance, not in user configuration.

### MS-9 M4: T8 fires, and the holder paragraph does not fix holder attribution

831 probe units labelled by both hosted coders. α (five M4 labels) = **0.6042**
[0.5646, 0.6473], raw agreement 71.7% — better than the same-as task (MS-1) but still below
0.667, so M4 figures carry a ceiling too.

The RA rate's denominator is the units where attribution is at stake, `RA + RC + RN`; `MI` and
`X` carry no attribution to get wrong. The two coders split `MI`/`X` differently, which is why
their denominators differ on the same units:

| arm | RA rate, Gemini | RA rate, DeepSeek |
|---|---|---|
| L (local, builtin prompt) | 11.1% of 126 | **24.5% of 147** |
| H (local, holder paragraph) | 16.7% of 24 | 23.8% of 21 |
| R (hosted, builtin prompt) | 16.3% of 257 | 20.6% of 247 |
| P (packed) | 0.0% of 11 | 14.3% of 7 |
| T (T=0.7) | 11.1% of 36 | 15.2% of 33 |

**T8 fires.** Arm L exceeds 10% on both coders, so the threshold is not straddled and the row is
a decision rather than an *inconclusive*: holder/mode must be structural from TX-P5's first
ingest, with a probe-set gate in its exit tests.

**T9 does not hold.** It asks whether the holder paragraph brings arm H to ≤ 5% where arm L
exceeds 10%. Arm H is above 5% on both coders, and on Gemini it is *worse* than arm L
(11.1% → 16.7%). T9's plan change was to move the holder paragraph into the TX-P0 ingest prompt
beside F-6; this measurement is evidence against doing that, because the paragraph does not buy
the reduction it was supposed to buy. Arm H is one run per input, so n is 21–24 and this is a
direction, not a rate.

Arm P reads lowest on Gemini and mid on DeepSeek at n = 7–11, which carries nothing either way.

### MS-10 Arm P: more preceding context halves the language-rule violation

Packing the preceding chapter ahead of the passage (arm P, same basenames so the typed path and
therefore the uid are comparable) changes MS-5's violation rate:

| arm | ru gists | wrong script | rate |
|---|---|---|---|
| L (passage alone) | 585 | 460 | **78.63%** |
| P (passage + preceding chapter) | 81 | 46 | **56.79%** |

More Russian context in the window makes the local model substantially more likely to answer in
Russian, without any prompt change. That points at the violation being a decoding-prior effect
rather than an instruction-comprehension failure, and it is a second argument that the fix for
MS-5 has to be verification rather than better wording.

### MS-11 The holder paragraph introduces the degradation that arm L never has

Arm L and arm H run the same local model on the same inputs. The only difference is the holder
paragraph arm H adds to the system prompt — the very change T9 was written to justify.

| arm | prompt | gists | `E022` | `W304` | degraded share |
|---|---|---|---|---|---|
| L | builtin | 2,070 | **0** | **0** | 0.00% |
| H | builtin + holder paragraph | 336 | 18 | 9 | **2.68%** |
| P | builtin, passage + preceding chapter | 261 | 7 | 4 | 1.53% |
| T | builtin, T=0.7 | 400 | 0 | 2 | 0.50% |

Mean gist length barely moves (53.4 → 54.9 characters), so the holder paragraph is not lengthening
gists uniformly; it is fattening the tail, and the tail is what the byte ceiling cuts. On Russian
specifically arm H degrades 5 of 82 units (6.1%) where arm L degrades 0 of 585.

Arm P tells the same story from the other side. MS-10 showed packing makes the model write Cyrillic
more often — the Cyrillic share of its Russian gists rises from 21.4% (arm L) to 43.2% — and its
degradation rate rises with it, from 0.00% to 1.53%. Writing Russian *is* what triggers the
overflow, which is MS-6's mechanism observed a second time under a different intervention.

**Consequence.** This is a second, independent argument against T9's plan change. MS-9 showed the
holder paragraph does not reduce the RA rate; MS-11 shows it introduces `E022` degradations in a
configuration that previously had none. Putting it into the TX-P0 ingest prompt before F-2 lands
would trade an unmeasured attribution gain for a measured loss of propositions.
