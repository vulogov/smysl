# S0 — extraction and proposition spike: report

**Protocol:** RFC SMYSL-2.2 (draft 1). **Run against:** crate 1.9.0, TX-P0 complete
except F-2. **Corpus:** 29 inputs, 5 languages, public domain or CC0.
**Arms:** H 29 runs, L 145 runs, P 16 runs, R 145 runs, T 30 runs.
**Models:** arm L `qwen2.5:14b-instruct` (local, Apache 2.0), arm R `deepseek-chat` (hosted).

The protocol was written against `d25ec9e`, before TX-P0. Six of its observations
(O-1 to O-6) are now resolved and were re-verified live; several of its stated facts no
longer hold. `FINDINGS.md` records every departure. Labels for same-as and holder/mode
come from models rather than two people, so every row resting on them is **provisional**.

## 1. What the spike answers

| q | question | answer |
|---|---|---|
| Q1 | Does one model, rerun, extract the same propositions? | J_class within model: L 1.000, R 0.327 |
| Q2 | Do two models extract the same propositions? | **no, not at uid level**: zero shared uids between the two models anywhere in the corpus; only judged classes can answer this |
| Q3 | Can same-as be judged reliably? | model-model alpha only; the human ceiling GE-T9 asks for is still unmeasured |
| Q4 | How often is reported speech extracted as asserted? | arm L deepseek **24.5%** of 147; arm L gemini **11.1%** of 126; arm R deepseek **20.6%** of 247; arm R gemini **16.3%** of 257, every cell above the 10% threshold |
| Q5 | How fair is attribution across languages under V1? | 0 false-`Absent` in 4034 quotes = 0.00%; every `Absent` is a genuine paraphrase, not a normalisation artefact |
| Q6 | What does extraction cost, and does density depend on language? | hosted Russian costs **4.0x** Section 6's per-chapter figure (3.0x against it including its x1.3 repair allowance), driven by repair retries; English is inside the plan on either reading; density differences are swamped by run-to-run variance |

## 2. The result that matters: Russian ingest is degraded, and F-2 is why

Counts are corpus-wide over all five arms. Arm R carries 554 of the 579 `E022`; arm L
carries none at all, and arms H and P carry the rest (MS-11). The bytes/char table below
is arm R only, for the reason stated there.

| | en | ru | es | fr | de |
|---|---|---|---|---|---|
| `SMY-E022` gist exceeds l0_max | 18 | 552 | 3 | 3 | 3 |
| `SMY-W304` unit degraded to prose | 8 | 161 | 2 | 2 | 1 |
| units degraded, as a share of all units | 0.26% | 8.26% | 0.34% | 0.32% | 0.18% |
| runs degrading at least one unit | 5% | 39% | 6% | 6% | 3% |

The chain is closed: a Russian gist exceeds the bound, `E022` fires, three repair
attempts fail on the same unit, and the proposition becomes one opaque `@prose` unit.

The cause is the byte-based estimator, not the prompt. The enforced limit is 30
estimator tokens = 120 bytes. Measured bytes per character in gists:

Arm R only: arm L writes most of its Russian gists in Latin script (MS-5), which would
pull the Russian figure towards English and misstate the ceiling.

| | bytes/char | effective limit | prompt says | mean gist | headroom used |
|---|---|---|---|---|---|
| en | 1.03 | 116 chars | 120 | 60.8 chars | 52% |
| ru | 1.83 | 65 chars | 60 | 49.1 chars | 75% |

H-5's per-script bound is correctly calibrated against the limit. What differs is
headroom: an English gist uses about half its allowance and a Russian gist three
quarters, so ordinary variance pushes Russian over. A 65-character Russian gist also
carries less content than a 116-character English one, which is the inequity F-2 removes.
This is a measured cost of deferring F-2 to 1.10.0 and it depends on no judged label.

## 3. Stability (M1)

Zero uids are shared between the two models anywhere in the corpus. Since a uid covers
label, gist, status and quote, cross-model uid agreement is vacuous by construction, and
the protocol's between-model J_uid row cannot inform Q2. Only judged classes can.

| arm | lang | inputs | J_uid median | J_gist median |
|---|---|---|---|---|
| L | de | 3 | 1.000 | 1.000 |
| L | en | 10 | 1.000 | 1.000 |
| L | es | 3 | 1.000 | 1.000 |
| L | fr | 3 | 1.000 | 1.000 |
| L | ru | 10 | 1.000 | 1.000 |
| R | de | 3 | 0.182 | 0.182 |
| R | en | 10 | 0.182 | 0.282 |
| R | es | 3 | 0.333 | 0.402 |
| R | fr | 3 | 0.547 | 0.583 |
| R | ru | 10 | 0.186 | 0.219 |
| T | en | 6 | 0.000 | 0.000 |

## 4. Decision table

Rows resting on same-as or holder labels are **provisional**: the labels are a
model's, not two people's, so the human ceiling GE-T9 asks for is still unmeasured.

| # | measure | threshold | measured | state | plan change |
|---|---|---|---|---|---|
| **T1** | alpha (same/not), en and ru | < 0.667 after one revision | 0.600, 95% CI [0.550, 0.646], 3 model coders (deepseek, gemini, ollama) | FIRES | gold unreliable; restate GE-T5/GE-T2 against alpha |
| **T2** | J_class within model, median over judged inputs | >= 0.6 for both models | L 1.000, R 0.327 | does not hold | - |
| **T3** | same | < 0.6 for either model | L 1.000, R 0.327 | FIRES | pivot to consensus extraction (attested:2) |
| **T4** | same | < 0.4 for both models | L 1.000, R 0.327 | clear | - |
| **T5** | J_class narrative (P3) vs law/epistle (P1, P7) | narrative < 0.6 while others >= 0.6, or RA on narrative under H > 25% | narrative 0.747, law/epistle 0.811, RA(H,narr) not measured | clear on J_class; the RA disjunct is unmeasured (arm H produced no probe unit in Gen.4) | - |
| **T6** | precision at recall 0.7, within language | < 0.9 | C2 cosine 0.789, C3 and C2 cosine 0.835 | FIRES | class measures ship as exploration only; TX-P7 defaults to attested:2 |
| **T7** | precision at recall 0.7, cross-lingual | < 0.9 | C2 cosine 0.427, C3 and C2 cosine 0.712 | FIRES | cross-lingual classes wait for TX-P10 S1 embeddings; anchored only |
| **T8** | RA rate, arm L | > 10% | deepseek 24.5% of 147, gemini 11.1% of 126 | FIRES | holder/mode structural from TX-P5; probe gate in its exit tests |
| **T9** | RA rate, arm H | <= 5% where L > 10% | deepseek 23.8% of 21, gemini 16.7% of 24 | does not hold (arm H is above 5% on every coder) | - |
| **T10** | V1 false-Absent, ru (M3b) | > 1% of quotes | 0 of 1575 = 0.00% | clear | - |
| **T11** | E022 degradations, ru | > 5% of units | 161 degraded units (W304) against 1785 = 9.0%, from 552 E022 events | FIRES | confirms FC-6 as a precondition; ru A1 not comparable until F-2 lands |
| **T12** | GE-T3 pilot | between-language variance > run variance | L between 51.9 / within 239.7; R between 60.5 / within 249.3 | clear | - |
| **T13** | ru/en model-token ratio vs estimator ratio | differ by > 20% | L regression 3.53 vs 2.03 (74%); L totals 2.07 vs 1.96 (6%); R regression 1.99 vs 2.03 (2%); R totals 3.94 vs 1.96 (101%) | FIRES (R); the regression disagrees on L, R | calibrate FC-6 weights on M5 data |
| **T14** | measured tokens per chapter | > 2x the planning figure | H-de 1.24x, H-en 1.28x, H-es 1.22x, H-fr 1.92x, H-ru 1.61x, L-de 0.71x, L-en 1.11x, L-es 1.00x, L-fr 1.59x, L-ru 1.69x, P-en 1.62x, P-ru 3.19x, R-de 2.17x, R-en 1.42x, R-es 1.27x, R-fr 1.34x, R-ru 3.96x, T-en 1.24x | FIRES (P-ru, R-de, R-ru) | revise the cost model before TX-P5 is sized |

- fires: T1, T3, T6, T7, T8, T11, T13, T14
- inconclusive: none
- not measured: none

## 5. Deviations from the protocol

| # | protocol | what was done | why |
|---|---|---|---|
| 1 | arm L on `qwen2.5:14b-instruct`, tag unverified | the tag exists and was used | confirmed; Apache 2.0, which also eases OQ-34 for arm L |
| 2 | Russian essay from Project Gutenberg | taken from Russian Wikisource | Gutenberg carries only the English translation; the original is public domain |
| 3 | P10 written by the owner, translated by a person | written by the assistant in both languages | no human translator was available; P10 carries no cross-lingual pairs, so the channel cannot reach a cross-lingual measure |
| 4 | V2-emu, a hand-rolled fold | the real normaliser V2 | F-3 shipped in 1.9.0, so T10 tests FC-7 directly instead of estimating it |
| 5 | two human annotators, ~1,800 pairs | three models as coders, two hosted and one local | the human ceiling GE-T9 asks for is unmeasured; every dependent row is provisional |
| 6 | essay probes H10 to H12 | H10 only | Maude's paragraph split diverges from the Russian after about p12, so only the Baumgarten block is locatable in both |
| 7 | one working directory per run, because `-C` was ignored | kept | H-8 fixed `-C`, but each uid still carries the input path as typed, so the layout is what makes uids comparable across arms |
| 8 | `smysl find` per candidate | one batched process per pool | `find` reloads the 512 MB embedding model on every call, which does not scale to this corpus |

| 9 | every en, ru and en-ru pair judged (about 1,800 estimated) | one stratified sample of 1,500 of the 11,602 the corpus actually yields, fixed seed, shared by every coder | the five-run corpus is 6.4x the protocol's estimate; the draw is proportional over (set, rule, cross-language) and well above Section 3.6's 300-pair floor |
| 10 | arm T optional, only if arm L is bit-identical at T=0 | run, 5 x 6 judged English inputs at T=0.7 | arm L is bit-identical on all 145 runs, which is the condition Section 3.3 states |

## 6. The resulting plan state

Section 3.7: the spike never kills a phase, it reorders work or changes defaults. In one
line: **continue, with a stated pivot** - TX-P7's same-as core and F-2 move ahead of
further ingest work, class measures ship as exploration only, and holder/mode becomes
structural rather than prompt-borne.

What each fired row changes (T1, T3, T6, T7, T8, T11, T13, T14):

- **F-2 moves ahead of TX-P1** (T11, T14, and Section 2). It is the one change that
  removes a measured, reproducible loss of propositions, and it needs no judged label.
- **Extraction becomes consensus by default** (T3): two runs or two models per window,
  class measures default to `attested:2`, and GE-T2 reruns under that policy.
- **Class measures ship as exploration only** (T6, T7). Cross-lingual classes wait for
  TX-P10's S1 embeddings; `anchored` alone is used for the Bibles.
- **Holder/mode becomes structural from TX-P5's first ingest** (T8), with a probe-set
  gate in its exit tests. The holder paragraph does **not** go into the TX-P0 prompt:
  T9 does not hold, and MS-9 and MS-11 both argue against it.
- **GE-T5 and GE-T2 thresholds are restated relative to alpha** (T1), and TX-P7's exit
  test changes before TX-P7 starts. The alpha here is model-model, so the human
  measurement GE-T9 asks for is still owed.
- **FC-6 weights are calibrated on this M5 data** (T13), and the cost model is revised
  for non-English before TX-P5 is sized (T14).

Two findings outside the table bear on the plan. `temperature` is correctness-relevant,
not a quality knob: arm L is bit-identical at T=0 and shares no uid at all at T=0.7
(MS-8). And the language policy F-6 added is an instruction with no verification, which
both models violate in opposite directions (MS-5); a character-range count is cheap
enough to be a diagnostic.

## 7. Answers to open questions

- **F-11 resolved.** `potion-multilingual-128M` loads in `model2vec-rs 0.2` and ranks;
  no Python fallback was needed. It bridges en to ru, but at a markedly lower score than
  within one language, so a single global threshold would not behave the same both ways.
- **OQ-3 resolved.** The chosen Synodal module contains **one** `ё` in the whole Bible.
  The Russian F-3 measurement therefore rests on model-introduced `ё`, not the source.
- **OQ-34, partly.** Arm L's model is Apache 2.0, so its outputs are usable as fixtures
  on the same footing as the inputs. Arm R's hosted output still needs the decision.
- **OQ-7.** `strict` and `component` are both reported, with component diameter.

