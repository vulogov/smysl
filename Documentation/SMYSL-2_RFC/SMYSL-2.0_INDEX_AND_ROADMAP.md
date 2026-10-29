# RFC SMYSL-2.0 — Index and roadmap

**Status:** draft 1, for discussion. Non-normative: this document organises the set.
**Author:** Vladimir Ulogov
**Date:** 2026-10-02
**For:** crate `1.9.0-dev` (`d25ec9e`), format `smysl/1.0`, kernel `smysl.kernel/0.1`.
**Supersedes:** RFC SMYSL-2 draft 3 (`RFC_SMYSL-2_TEXT_CORPORA_DRAFT3.md`) as the working
document. Draft 3 remains the design record. Where this set and draft 3 differ, the set wins, and
§7 lists every difference.

---

## 0. What this set is

Draft 3 described using smysl as an instrument for text research:
- a library of books, articles and chats in five languages
- time and dating
- a query language and discovery dossiers
- information-theoretic and knowledge-assessment measures
- schema evolution
- import and export

It was one 2,700-line document. The evaluation of draft 3 recommended splitting it, running the
riskiest experiment first, and settling the decisions that fix identity before writing units at
scale. This set does that. It is **ten documents, one normative**:

| id | document | kind | phases | lines |
|---|---|---|---|---:|
| **2.0** | `SMYSL-2.0_INDEX_AND_ROADMAP.md` (this) | index | all | — |
| **2.1** | `SMYSL-2.1_HARDENING.md`: fixes to today's tree | implementation | TX-P0 | 1,149 |
| **2.2** | `SMYSL-2.2_EXTRACTION_SPIKE.md`: extraction and proposition spike | experiment protocol | S0 | 595 |
| **2.3** | `SMYSL-2.3_FORMAT_AMENDMENT.md`: library, time, schema evolution, round trip | **normative** | per amendment | 966 |
| **2.4** | `SMYSL-2.4_LIBRARY_AND_INGEST.md`: library, readers, time engine, ingest, linking, propositions, languages | implementation | TX-P1–P5, TX-P7 | 3,838 |
| **2.5** | `SMYSL-2.5_QUERY_AND_DOSSIERS.md`: `sq`, dossiers, thread schemas, schema-evolution tooling, embeddings | implementation | TX-P6, TX-P8 (dossiers), TX-P10, TX-P12 | 1,389 |
| **2.6** | `SMYSL-2.6_MEASURES_AND_ASSESSMENT.md`: measures, assessment | implementation | TX-P8 (T1–T7), TX-P9 | 909 |
| **2.7** | `SMYSL-2.7_IMPORT_EXPORT.md`: `lib import` / `lib export`, round trip | implementation | TX-P11 | 998 |
| **2.8** | `SMYSL-2.8_SCALE.md`: `StoreRead`, shards, disk-backed store | implementation | TX-P13 | 1,097 |
| **2.9** | `SMYSL-2.9_QUERY_GRAMMAR.md`: the `sq/1` grammar — lexer, productions, determinism clauses, canonical printing | implementation | TX-P6 (steps 1, 3) | 652 |

The line counts are as of 2026-10-16 and are a sense of weight, not a checked figure —
nothing reads them, and every one of them was stale before that date, because a count recorded
when a document is written is wrong by its next edit.

**Rules of the set:**
- **Only 2.3 is normative.** Implementation RFCs refer to it and do not redefine records, keys,
  identities, rules or enumeration codes. When 2.3 is accepted, its amendments are folded into
  `SMYSL_FORMAT_SPEC.md` one at a time, as their phases land (2.3 §1.2).
- **Every claim about today's code was checked against `d25ec9e`** by the document that makes it,
  with file and function cited. Items that could not be checked are marked *(unverified)* in
  place.
- **Numbering continues draft 3's.** FC-1 to FC-9, records 14–19, GE-T1 to GE-T16, OQ-1 to OQ-28,
  and TX-P0 to TX-P13 mean what they meant there. New diagnostics and open questions take
  per-document ranges (§6).
- **No AI attribution** appears in any document of the set. Authorship is the owner's.

**Reading order:**
- Read 2.0, then 2.3, then the RFC for whatever is being built.
- 2.1 and 2.2 can be read alone. They are the first things to do.

---

## 1. The path

```
   owner accepts 2.3 A-4, A-8.1, A-9, A-10 (items 2–3)   ← done 2026-10-02, 1.9.0 releasable
            ┌─────────────── weeks 1–2 ───────────────┐
            │  TX-P0 done   ‖  S0 spike done (2.2)     │   rest of 2.3 accepted 2026-10-07
            └──────────────┬──────────────┬────────────┘
                           │              │
              gate G0 PASSED 2026-10-07: decision table = continue + pivot; 2.3 accepted
                           │
            TX-P1 → TX-P2 → TX-P3 → TX-P4 → TX-P5          (2.4)   library, time, index, ingest
                                              │
                         ┌────────────────────┼───────────────────┐
                       TX-P6 (2.5)          TX-P7 (2.4)           TX-P13a (2.8)*
                       sq core             propositions,          StoreRead in memory
                         │                 linker, summaries      (needs only TX-P0)
                         ├──────────┬─────────┘
                       TX-P8      TX-P11 (2.7)    TX-P10 (2.5)
                       dossiers   import/export   embeddings, NL front end
                       (2.5), T1–T7 (2.6)
                         │           │
                       TX-P9 (2.6)  gate G2: GE-T15 (round trip exact)
                       TX-P12 (2.5)  │
                       schema       TX-P13d (2.8): disk-primary
                       evolution

   TX-P13b (2.8) redb as an index: after TX-P4 (redb, MSRV)
   TX-P13c (2.8) shards and union: after TX-P5 (library shards) and TX-P13b
```

\* SMYSL-2.8 splits TX-P13, and its gating is the one this set adopts:
- **13a** (fixes, the `StoreRead` trait, consumers ported in memory) needs only TX-P0. It can run
  in parallel with the library work.
- **13b** (redb as an index) and **13c** (shards and the virtual union) need the library's redb
  and shards (TX-P4, TX-P5).
- **Only 13d** (redb as the primary store) is gated on GE-T15.

**Gates:**

| gate | after | passes when | if it fails |
|---|---|---|---|
| **G−1** | before 1.9.0 is released | the owner accepts the parts of 2.3 that TX-P0 activates: A-4 (digest), A-8.1 (four enumerations opened, 255 reserved in five), A-9 (estimator key), A-10 items 2–3 (`lang` lint, strict `source`). Otherwise 1.9.0 ships wire behaviour no normative text covers. | TX-P0 ships without those items. |
| **G0** **passed 2026-10-07** | S0 + TX-P0 | ~~2.2's decision table (§3.7) returns *continue* or a stated pivot, and the owner accepts the rest of 2.3 that TX-P1 to TX-P5 activate~~ **both met.** S0 ran in full (365 runs) and its table returns *continue with a stated pivot*; the owner accepted A-1, A-2, A-3, A-5, A-6, the `admission` opening in A-8.1, A-11 `x.text/v1`, A-12 rules E and Z, A-13 on 2026-10-07. Evidence: `S0_RESULTS/`. | n/a — passed. The pivot it names is §1.1 below, and 2.4 was re-planned against it on 2026-10-08 (draft 2, §0.1) before TX-P1. |
| ~~**G1**~~ **passed 2026-10-14** | TX-P5 | GE-T1, GE-T4, GE-T13 and GE-T14 pass inside 2.4's phases, and the TX-P5 cost report replaces the planning figures. **All four pass and the report is measured.** GE-T14 passed in TX-P2 (the part minimum moved from 64 KiB to 1 KiB on the measured curve); **GE-T13 passes** over `fixtures/chronology/public.tsv`, 45 known orders of which 43 are decidable and 39 claim disjointness; **GE-T1 passes over the five Bibles** on two platforms with the three ports recomputing every identity, its JSON series outstanding and not blocking, Linux x86-64 belonging in CI; **GE-T4 measures 0.0000 false-`Absent` in all five tier-1 languages** under both normalisers, against a bar of two units in 200, with all 29 refusals ruled by hand and none by rule. The gate's own stake is therefore discharged: identity is fixed, and units may be written at scale | units are not written at scale until fixed. This is the last cheap point to change identity (A-1, A-2). |
| **G2** **passed 2026-10-22** | TX-P11 | GE-T15: P1, P2 and P3 hold on the whole matrix — **met**: 1,056 cells over F1–F13, the wire fixtures, unknown codes, the five Bibles with text and two chats, every layout, both closures, both formats and both flag sets; P2 for every fixture imported raw; every export verified by Python, JavaScript and Go; scale cells at 10⁵ and 10⁶ (SMYSL-2.7 §6 step 12) | TX-P13d does not start: export is the way out of a disk store and must be exact |
| **G3** | TX-P13d | GE-T7 at library scale | the CBOR log stays primary, and redb stays an index |


### 1.1 G0's outcome: continue, with a stated pivot

S0 ran the full protocol — 365 runs (the 335 of 2.2 §3 plus 30 of the optional arm T), 29 inputs,
five languages, two models — and its decision table fires eight rows: T1, T3, T6, T7, T8, T11,
T13, T14. None is inconclusive and none is unmeasured. `S0_RESULTS/` holds the report, the table
and the findings log.

§3.7 binds the spike to reordering work or changing defaults, never to killing a phase. The pivot
it names, and what each item changes before TX-P1:

| # | change | from | why |
|---|---|---|---|
| 1 | ~~F-2 comes before TX-P1~~ **done 2026-10-07.** It left 1.10.0, OQ-31 was answered, and it is built. | T11, T14, report §2 | The one change that removes a measured, reproducible loss of propositions, and it rested on no judged label. Hosted Russian degraded 12.67% of units; 579 `E022` across the corpus. `smysl/content/1` closes draft 3 §22's en/ru gap from 68.5% to 3.2% relative, with the English budget unmoved. |
| 2 | **Extraction is consensus by default.** TX-P5 extracts twice per window; class measures default to `attested:2`; GE-T2 reruns under that policy. ~~**Landed 2026-10-08**~~, *narrowed*: the agreement is `identical-span` only, because the broader form needs the judge item 5 found unreliable. 2.4 §0.1, §3.4. | T3 | J_class within model is 1.000 local but 0.327 hosted, against a 0.6 threshold. |
| 3 | **Class measures ship as exploration only.** Cross-lingual classes wait for TX-P10's S1 embeddings; `anchored` alone is used for the Bibles. ~~**Landed 2026-10-08**~~ as written; adjacency (C3) is kept as load-bearing rather than a filter. 2.4 §3.6. | T6, T7 | Precision at recall 0.7 reaches 0.835 same-language and 0.712 cross-lingual against a 0.9 bar, and that is under the better of two provisional golds. |
| 4 | **Holder and mode become structural from TX-P5's first ingest**, with a probe-set gate in its exit tests. The holder paragraph does **not** go into the TX-P0 prompt. ~~**Landed 2026-10-08**~~ as written; the gate is a new phase-exit clause at below 10%. 2.4 §3.4, §6. | T8, T9 | Reported speech is extracted as asserted in 11.1–24.5% of attribution-bearing units. The holder paragraph does not reduce that, and it introduces `E022` degradations in a local configuration that had none (findings MS-9, MS-11). |
| 5 | **GE-T5 and GE-T2 thresholds are restated relative to α**, and TX-P7's exit test changes before TX-P7 starts. ~~**Landed 2026-10-08**~~: an engine passes by joining the coder pool, not by beating it, and the 0.9 bar turned out to be unattainable rather than failed. 2.4 §5.5, §6. | T1 | α is 0.600 [0.550, 0.646] binary and 0.330 six-label over three model coders — below 0.667 on every pairing. |
| 6 | **FC-6 weights are calibrated on S0's M5 data**, and the cost model is revised for non-English before TX-P5 is sized. ~~**Landed 2026-10-08**~~, *corrected*: no weight changes — the gap is the repair loop, not tokenization — and the model gains a retry term, re-measured after F-2. 2.4 §0.1, §3.7. | T13, T14 | `ceil(bytes/4)` predicts a 1.96x ru/en cost; measured is 1.84x local and 3.38x hosted. Hosted Russian runs 4.0x §6's per-chapter figure, English 1.4x. |

Two further findings bear on 2.4 without being table rows:

- **`temperature` is correctness-relevant, not a quality knob.** Arm L is bit-identical across all
  145 runs at T=0, and shares no uid at all at T=0.7 on five of six inputs (MS-8). It belongs in
  the recipe and in a uid's provenance, not in user configuration. It is already in the recipe
  (`recipe.rs`); what the re-plan adds is that it is now a *precondition of item 2* — two passes
  at T>0 cannot agree by `identical-span` except by accident, so `ingest --text` refuses a
  non-zero temperature unless `--single-pass` is given. 2.4 §3.4.
- **The language policy F-6 added is an instruction with no verification**, and both models
  violate it in opposite directions — the local model writes 78.6% of Russian gists in Latin
  script, the hosted one wrote every gist of one English chapter in Chinese (MS-5). A
  character-range count is cheap enough to be a diagnostic, and the byte ceiling currently
  *rewards* the violation (MS-6).

**What G0 does not settle.** The same-as and holder labels are three models', not two people's,
so every row resting on them is provisional and GE-T9's human ceiling is still owed. A-7, A-10's
remainder, A-11 `x.query/v1`, A-12 rule N and the proposition classes, and A-14 stay for
discussion; OQ-7, OQ-23, OQ-35 and OQ-49 stay open with them.

---

## 2. Phases and owners

| phase | RFC | delivers | exit (summary) | depends on |
|---|---|---|---|---|
| S0 | 2.2 | extraction stability and proposition-trust spike with today's CLI | 2.2 decision table | — |
| TX-P0 **complete** | 2.1 | Every item, including **F-2** — which G0's pivot pulled out of 1.10.0 and which landed 2026-10-07 with OQ-31 answered (§1.1 item 1) — and F-16's `bundle --unknown keep\|drop` flag, which had been separated because only the closure was a defect | met: F-16/F-17 reproductions fixed, every probe reversed, and suite, `make semver`, `api-check`, `doc-output` and `spec-tables` clean on every commit. Departures from plan: 2.1 §10 | G−1 accepted in full |
| TX-P1 **complete** | 2.4 | parts, readings, manifests, object store, first readers, structure, locators; `admission` opened (2.3 A-8.1) | met but for the corpus at the time; **the five Bibles are paid in 1.10.0** — five libraries built through `usfm/1`, every identity rolled into one digest per edition, the same rolls on macOS and on Linux, 112 KiB of it committed so CI compares on every push, and the three ports recomputing over a reader's own Cyrillic output. GE-T1's **JSON series** is what remains, recorded at 2.4 §5.5 rather than counted as passed | TX-P0, G0 **passed**; F-2 **done** (§1.1) |
| TX-P2 **complete** | 2.4 | segmenter, analyzers, `lingua`, chat readers, `text append`, redaction (rule Z), **and the class core** (`strict`, `component`, `attested:n`) moved here from TX-P7 so TX-P5's consensus output can be read | met: GE-T1's chat half is pinned in `fixtures/library/readers/` and recomputed by all three ports, rule Z's algebra survives a stale peer by property, the strict class counts agree with an independent Python reading of A-12.4, and **GE-T14 ran** — the part minimum moved from 64 KiB to 1 KiB on the measured curve. Owed: the per-language boundary gold set | TX-P1 |
| TX-P3 **complete** | 2.4 | EDTF, `published`, datings, rule E engine, locks, `date` commands | GE-T13 ran; 3 clauses of 4 met at the time, and the fourth — a public chronology — paid in 1.10.0 | TX-P2 |
| TX-P4 **complete** | 2.4 | substrate index, persistent postings, `text` commands | met, with one clause attributed: **all twelve** of §3.8's tables, both builders each, the byte-comparison gate — which caught two of the five shapes it had to, measured by breaking each in turn — the `Analyze`/`Postings` seams, `find --lang --analyzer --library`, `text align` and `text index`. `find`'s p95 clause is met for the **query** (2,034 ms → 34 ms at 155k) and not for the command (2.6 s → 659 ms), because what is left is the catalog load: SMYSL-2.8's M-7, and TX-P13's. Step 3's range clause is met with 24 rows read for 24 answers out of 155,510. OQ-4 was not needed and stays open. The redb backing and a dense document id are **owed** (2.4 §6) | TX-P3 |
| TX-P5 **complete** | 2.4 | ingest over texts, owned-range spans, ledger digest, journal, FC-1/FC-3, entities without span, **consensus extraction** and **structural holder/mode** | G1, plus the probe-set gate and GE-T2 rerun the pivot adds (2.4 §6). Step 1 done: FC-1 and FC-3 activated, the spec's §2.2 amended in place, all three ports, and `SMY-E404`/`SMY-W405` registered and raised. Step 2 done: `x.text/v1` declared. Step 3a done: two passes, both staged, the `identical-span` agreement recorded by the run, the recipe field, and the temperature refusal before egress. Step 3 done: `smysl_ingest::text` behind a `text` feature — windows of whole nodes with one node of overlap and owned ranges that partition the text, the four-scope search that turns a quote into a span, the `Absent` cap, and `ingest:quote` removed only from units that got a span, with `SMY-W419`, `SMY-W441` and `SMY-W447` registered. Step 3b built: `text:mode`, `text:speaker` and `text:holder` from the node a span landed in, the holder paragraph kept out of the prompt by an assertion, and a probe-set harness — whose **exit is owed**, and the reason was a finding: no reader in this build marked reported speech, so a punctuated edition was handled and a 1769 Bible was not, which located the gap in the readers (`\wj`, `<q who>`) rather than in the converter. **The readers closed it 2026-10-15:** `usfm/1` reads `\wj … \wj*`, `osis/1` reads `<q who="…">` in both the container and the milestone form, and both write reading **key 9** — marks of `{start, end, who?}` on the row whose range holds them, with the `who` carried and deliberately not read, since §3.4 says a mark establishes `reported` and no holder. `reported_speech.rs` asserts the path with no model in it, and the identity cost on this corpus is nil: a part with no marks encodes as it always did, so every reader expectation and all five GE-T1 editions are byte-identical. **The probe set followed it the same day and the gate is met at 0.0%** over the 17 probes whose own edition marks or punctuates the clause: `fixtures/attribution/probes/` is Mark 14-15 in two editions from eBible's USFM distributions — the 1769 KJV, which marks the words of Jesus and punctuates nothing, and the World English Bible, which marks them and punctuates every quotation — with **every** speech act of Mark 15 probed in both, which is how the development-set objection is answered: the choice of verses is removed rather than defended, as GE-T13's chronology removed the choice of date pairs. The 13 probes whose edition punctuates nothing come out at **100%** and are counted and reported rather than passed, GE-T13's rule for a pair open on the side compared, so the typography finding is now a measured figure instead of a prediction. It cost one decision and found two defects. The decision: `marks_of` **joins** the reader's marks with the punctuation scan instead of preferring them, because `\wj` marks the words of *Jesus* rather than reported speech — over the same two chapters the reader finds 30 marks and the scan 54, so ranking the suppliers lost Pilate, the high priest, the centurion and the crowd precisely because the edition had marked anything at all (2.4 §6 step 3b decision 4, reversed with the measurement). The defects, both hidden by every reader fixture in the tree having exactly one part: `part::local_rows` shifted a row to its part's offsets and left the **marks inside the row** at the whole text's, so chapter 15's marks pointed past chapter 15; and a probe needs a **locator** as well as a quote, since the four-scope search matched `Thou sayest it.` at 15:2 against Peter's `what thou sayest` at 14:68 and scored the mode off the wrong verse, which is how the gate first reported 11.8% instead of 0.0%. Step 4 done: the ledger digest, the journal, `SMY-E442`/`SMY-W443`/`SMY-W444`, four recipe fields, the text prompt templates with two fences, and `ingest --text <alias>` with `--window-level`, `--digest-budget`, `--reextract` and `--single-pass` — both exit clauses met offline (`text_gate.rs`, `redteam.rs`). Step 5 measured: the phase exit's own figures (`phase_exit.rs`, `ge_t2.rs`), the per-window cost fields in the journal, `smysl_ingest::text::cost` and `ingest --text <who> --cost`. Three of the six clauses are **met** — resume re-sent 1 window of 8 after a kill, span coverage 1.000 against a control at 0.857, entity duplication 0.000 for 3 speakers across 9 windows — the cost report is **built and measured** per language, and two were **owed as material**: 3b's probe set and GE-T2's corpus, **both paid 2026-10-15** — both taken by an environment variable and both reported as owed without one. GE-T2's corpus is `fixtures/stability/corpus/`, the S0 spike's own passages with the choice removed mechanically (every reference all five tier-1 languages carry, so each file is 26 verses of narrative, 26 of law and 7 of poetry, 59 lines), and its rerun **ran**: three runs of all five languages against hosted `deepseek-chat` at temperature 0, span coverage **1.000** throughout, `identical-span` de 0.024/0.412/0.078, en 0.452/0.769/0.132, es 0.000/0.333/0.047, fr 0.047/0.024/0.391, ru 0.265/0.481/0.030. The finding is the spread rather than the figures: re-running one language moves it by 0.333 to 0.637 while changing the language moves it by 0.361 to 0.745, the same size, and the ranking does not survive a rerun — so **the per-language figure §5.5 asks for is not resolved by one run per language**, and it is recorded as a spread over three runs rather than as five numbers of sampling noise. Temperature was checked rather than assumed and is not the cause (`"temperature": 0.0` reaches the wire); the result reproduces S0's local-against-hosted split under a metric that needs no judge, and **OQ-74** records that `Passes::admits` guards the temperature a caller configured and not whether the provider honours it. It cost one addition and found two defects. The addition: `identical-span` includes the **gist**, so a pass that found the same bytes and re-worded its sentence counts as disagreement — French run B is 0.024 with the gist and **0.680** without, the two passes having found the same 17 of 25 places — so `Agreement` gained `comparable_places`, `agreed_places` and `place_rate`, reported beside the figure and never instead of it. The defects: the live arm set `max_output` to **800**, carried over from the scripted instrument where an answer is one line of JSON, and never set `context_window` at all, so it kept `ProviderConfig::default()`'s 4096 — against any real corpus every answer was cut off (`SMY-W304`), every span degraded to opaque prose and `comparable` was 0, so the arm would have reported *nothing was comparable* against any provider; and the per-window agreements were summed **field by field**, which silently dropped the two fields added to measure the corpus, now `Agreement::merge` in one place. Both were invisible to four passing instrument tests, because a scripted provider's answers fit any budget and its two passes agree on the wording by construction. The Ollama arm is **not measurable here**, measured: `llama3.2` and `qwen2.5:14b-instruct` both return one opaque unit per pass and nothing comparable over this corpus while every call answers 200, which is the first measurement of that mapper over a real corpus and narrows SMYSL-2.1 H-10's "unverified: no Ollama here". Three defects found by measuring: the journal recorded the run's running token total against each window, `Agreement::rate` divided for a single pass that agrees with itself, and the ingest **path** was chosen in bytes — so the same corpus in Russian was extracted through the surface parser and in English through json-ast, F-2's lesson in a second place (`path::choose_tokens`). `pivot:<lang>` landed after the exit, deliberately — GE-T2's figures are measured under `source`, and a policy adding a call per window while they were being taken would have changed the thing being measured: extraction under `source`, then one translated unit per unit with the source in `deps` and an `x.text/translates` edge, the span and the quote the source's, `body` and `detail` left behind because one `lang` covers every text field, entities left to TX-P7 because rule 7 would make a translation a second entity, and no repair loop because the only check available to a translation is whether a line came back (`SMY-W453`). Its call is out of §3.7's retry denominator and in the token counts. It found two more defects: nothing had ever declared `x.text/v1` in a library, so every check of a text-ingest store reported `SMY-W013` — correct and unactionable — and `--lang-policy` carried a closed list of one value. The phase's **precondition GE-T4** has its harness (`tests/ge_t4.rs`) and its corpus scored: 200 units in each of five languages out of the S0 spike's 730 staged documents, with the adjudication worklist down to **five units** — only an `Absent` can be a false `Absent`, a language the check never refused is at zero, and 24 of the 29 refusals are decidable with evidence, every one of them a model quoting past the window it was shown. de, es, fr and ru measured **0.0000** immediately; en was **provisional** on five rulings. **Ruled 2026-10-14: 0.0000 in all five**, all 29 refusals ruled by hand and none by rule, so the precondition is met and **G1 passes**. The five that needed a reader did not turn on the brackets after all — each is real KJV text from the chapter *next to* the one its unit cites, and a `source` is exactly one chapter, so the ruling was the definition the worklist states and the brackets were a second difference on three of the five rows. **The phase closes 2026-10-15**: all five steps are done, and all six of step 5's exit clauses are settled — three met, the probe-set gate met at 0.0%, GE-T2's rerun measured over three runs, and the cost report built and measured per language. Nothing is owed. What the phase leaves behind is recorded rather than carried: **OQ-74** on whether a provider honours the temperature `Passes::admits` guards, the finding that one run per language does not resolve GE-T2's per-language figure, and the Ollama mapper measured as not usable over a real corpus here (SMYSL-2.1 H-10 narrowed, not resolved) | TX-P4, TX-P2's class core |
| TX-P6 **complete** | 2.5 | `sq` core (pure) | `--seed-check` on every pure query — met 2026-10-17: `q --seed-check` runs a pure plan twice and compares; plans stable against the step-2 goldens byte for byte | TX-P5 | **Step 1 done 2026-10-15** and it needed a grammar first: draft 3's Appendix C, which the step cites for "every production", is not in this repository, so the productions were recovered from SMYSL-2.5 §3.3's operator-to-API table and written as **RFC SMYSL-2.9** before any code. `crates/smysl-query` is the lexer, parser, AST and canonical printer, with 23 accept and 17 reject fixtures in `fixtures/sq`, and the fuzz exit took four runs because the first three each found a real defect (2.9 §5). Two questions it raised against 2.5 are settled there: OQ-77 (qualified access is compiled forward, §3.9) and OQ-82 (a query is saved only after it has run, §3.6). **Step 2 done 2026-10-16:** the `Scope` port — whose signature SMYSL-2.5 names four times and never gives, so it was written from those four constraints and recorded in §4.2 — with `StoreScope` over a plain store, planner phases 1 to 4, and `--explain` as the snapshot the exit asks for, over all thirteen corpus fixtures. Two findings shaped it. **Phase 2 is half-buildable and the other half is TX-P12's:** relation kinds resolve today, so `SMY-E462` is raised, but a declaration's `payload_shape` is opaque CBOR until FC-9, so no field can be refused and `SMY-E421` would be a code with nothing behind it — TX-P12 step 3 names `E421`/`E422` and compile-forward as its own, and the plan records that the check did not run rather than reading as though it passed. **A stage phases 5 to 9 own is refused, not ignored,** with `SMY-E461` naming the phase that will lower it, which is also what makes SMYSL-2.5 §3.6's save precondition safe. `E461`, `E462` and `E466` were allocated by 2.5 and withheld until something could raise them; `E511` is new, for a scope naming a store that is not open, and is filed in 2.9's range with the reasoning in its §8. The registry is 95 codes. **Step 6 was blocked and is not any more (2026-10-16):** its exit cited "§11.1 and §11.5 examples", and SMYSL-2.5 has no §11 — those are draft 3 sections, absent here, the same gap that blocked step 1. Settled before the step rather than during it, and not the way step 1's was: a grammar had to exist before a parser could be written, which is what justified writing SMYSL-2.9, but examples do not have to exist before the feature they exercise. So the exit is now a capability checklist drawn from §3.3's own table — the six scope items `StoreScope` refuses today, catalogued partitions with per-partition analyzers, `lang:<code>` (but not `lang:any`, which is TX-P10's bridge), the four time predicates on both axes with `W415`, and the entity bridge — and the examples arrive during the step as `fixtures/sq/` goldens, executable and shared with the three language ports. **A second citation of the same missing sections was stale today:** §5.1 asked the grammar goldens to cover them, so that clause had been unsatisfiable since step 1 built those goldens from §4's productions; it is dropped with the reason recorded. **And SMYSL-2.5 §2's probe is corrected** while the document was open: it recorded `--seed-check` as "read nowhere" on a probe that could not distinguish an inert flag from a working one on a pure invocation, and `src/main.rs` reads it — so part of step 7's exit is already met and that step is smaller than its line suggests. **Step 3 done 2026-10-16:** the lexical source, RRF and `--why`, and it opened by correcting its own exit clause — the step read "lexical source with `Bm25::index_subset`", and §3.4, two sections from the step list, says that function is not needed and was not added. Four places named it; `index_subset` is in no crate. What the pure tier needed was the other shape: `postings::View::of` sums *disjoint partition tables* and lives in `smysl-text`, which step 3 cannot reach, while a plain `Store` is one corpus partitioned after the fact. So `smysl_retrieve::Subset` — a `Postings` restricted to a uid set — and the claim that makes it a replacement rather than an approximation is asserted bit-for-bit: scoring through a `Subset` equals scoring a table built from those units alone. **The partition and the IDF scope are two restrictions and not one** — the partition decides what may be returned, the IDF scope what the statistics are counted over — and `smysl-retrieve` already separated them, so `Query::within` carries the first and the view carries the second. RRF is §3.4's to the letter, `k = 60` recorded in the plan, and the determinism test is the one that can actually fail: the fused order must not depend on the order the partitions arrive in. The exit's first half passes over the whole shared query set — every query in `fixtures/retrieval/queries.tsv` ranks through grammar, planner, `Subset` and fusion exactly as `find` ranks it, scores and per-term contributions included. **OQ-83** was raised and decided on the way. **Step 4 done 2026-10-16:** filters through outputs — phases 6 to 9, which turned `Plan` into a tree, because `Expr::Set` is one and a set expression resolves a scope, a partitioning and a source per branch. §3.3's table is the exit and the test walks it row by row: every row either runs or is refused by name, so a row quietly forgotten fails rather than being absent. Two graph functions, both the shape §3.3 asked for: `dependents_trace`, the depth-bounded `(depth, uid)`-sorted mirror of `trace_via` that `dependents_via` cannot express, and **`trace_both`**, which the RFC did not foresee as a function — `expand -[k]-` is not the union of the two one-way walks, because a unit reached by going out one step and back one is in neither, and that is the common case for a relation two units both point at. Asserted, not argued. **Filters read kernel fields typed and payload keys raw,** which is the step's own clause, and the limit shows in one place: an ordering comparison on a raw key is refused rather than guessed, because `>` on text whose type nobody has declared answers a different question. That needed **`SMY-E512`** (2.9's range, 2.9 §8 has the reasoning) and it is deliberately not `E421`/`E422`, which are about a field nothing declares and a key retired without a mapping — here the field is declared and readable and the *question* is wrong. **`SMY-W463` came in from 2.5's allocation,** withheld until something walked a graph with a depth: a traversal that stopped while the graph went on, reported beside a successful answer, which is why `run` now returns warnings alongside the result. The registry is 97 codes. A defect the goldens could not see: **phase 2 refused `expand ->grounds` with `SMY-E462`** — `grounds` and `deps` are the two kernel support fields and no schema declares them, and nothing reached the walk until traversals were lowered, so the refusal had been invisible since step 2. Four things are refused for being a different tier rather than a later phase: a write (`as view`, which is the CLI's and so step 7's), the lens engine (`with holders`, a qualified field), the library layer (time stages, reading order, `with spans`, `spans`) and TX-P7's classes (`align by class`). The exit's second half is the sharper one: **every stage answers the same over a log appended in reverse**, including the set expressions, because a store is a log and a stage that read the log rather than the graph would pass everything else in the file. **Step 5 done 2026-10-16:** the supersession pass, and most of it was already written in the wrong place — `relink`'s private `fn successors` is now `smysl-graph`'s public `supersession_heads`, which is what §4.2 asks for and for the reason it gives in six words, *"so query and relink cannot disagree"*: two implementations of "which unit is the newest head" would answer a forked history differently in a query and in a repair. The exit's first half guards exactly that — **all nine `relink` tests pass unchanged** — and the second needed a fixture, because none of F1 to F13 supersedes anything, so `fixtures/sq/plans/supersession.smy` is a chain of three, a fork, and a claim nobody revised. **A fork replaces rather than reverts:** the single case replaces a unit by its head, so the fork case replaces it by *all* its successors, because choosing one would adjudicate a contention the store recorded and did not settle — and `SMY-W464` says so, which brings the registry to 98. A ranked answer loses its ranking to the pass and has to: a head that replaced two different hits has no one score, and inventing one would be inventing a ranking. `--all-versions` turns the pass off and the plan records that it did, because "this reader asked for every version" is a fact about the answer and silence would read like a pass that found nothing to do. **Step 6 done 2026-10-16:** the library layer's stages, and the first thing it needed was the thing step 2 had needed — a definition. SMYSL-2.5 names `texts`, `library`, `collection:`, `channels`, `channel:` and `import:` and neither it nor SMYSL-2.4 says what any of them is, so each is now defined (2.5 §3.3) by something the catalogue already records: **a collection is an alias namespace**, which is what the alias grammar's `/` was for and how every library here already names its editions; `import:` is that rule on the reserved prefix; **a channel is a conversation**, found by alias for the readers that export one per file and by `channel`-level locator head for Slack, which exports many. Everything crosses a port, because `smysl-text` depends on `smysl-query` for the traits and nothing in the planner may call back: `Times` (2.5's `TimeIndex`, renamed for what it does) answers a unit's effective time and parses the query's EDTF literal, so `SMY-E410` comes from SMYSL-2.4's one parser; `Entities` resolves a name through aliases and same-as and the query crate walks `mentions`; `Scope` gained `analyzer` and `restrict_lang`, so a partition is searched under its own chain — one postings table **per analyzer**, not per partition, and the chain id printed in the plan. **`SMY-W415` is registered** (99 codes), the oldest withheld allocation in the registry — draft 3's — and it is a count beside the answer rather than a refusal because an undated unit did not fail the comparison; A-2.4 rule 5 says it was never in it. The exit is sixteen goldens over a library the test builds from the committed Bible slices and chat exports, with `W415`'s count checked against an independent one, and one finding the import fixture exists to show: a part two manifests share places a unit in two partitions unless the unit's own `source.manifest` decides. **Step 7 done 2026-10-17, and the phase closes:** `smysl q`, the thirty-second command, a thin dispatcher over the facade's `query` module with `--explain`, `--why`, `--param`, `--all-versions`, `--idf`, `--per-partition`, `--library`, and `--view` for the one stage the pure crate refused because it writes. Both halves of the exit are asserted against things that already existed: `q --explain` over every corpus fixture prints step 2's planner golden byte for byte, and `--seed-check` now performs 2.5 §3.2's stronger assertion — the plan runs twice and the answers are compared — instead of the label check the flag made before. `SMY-E416` stays withheld: the planner refuses every model-dependent stage with `E461` before purity is asked, so no plan this build makes can reach a model, and the code is registered the day TX-P10 lowers one. Owed from the phase and recorded rather than carried: the multi-key `order by` comparator, the qualified-field refusal TX-P12 step 3 lifts, `lang:any` (TX-P10), and `--save`/`--rerun` (TX-P8). |
| TX-P7 **built, exit owed** | 2.4 | same-as engines, linker, summary tree (`strict` classes moved to TX-P2) | GE-T5 **under the α-relative threshold** (2.4 §5.5), GE-T11. Step 2 done: the three engines behind one `Engine` trait in `smysl_text::proposition`, `smysl_ingest::same_as::propose` as the only thing that turns a proposal into a record, and `same-as propose\|classes`. Its exit — the anthology test — is met in a form the sentence does not require: two paragraphs, each reprinted, so the assertion is *two classes of two* rather than one class of four, with a unit carrying one pair's gist over the other's bytes and a second extraction under one manifest sitting beside them as the two things that must not be proposed. A proposal carries a `why` because a pair cannot say why and GE-T5's sheet needs one; adjacency is on by default and switchable, so S0's 0.789 stays reproducible; an edge the store holds is counted and not staged again, and a **withdrawn** one is proposed again, because an engine disagreeing with a person is the thing that should be visible. Step 3 done: `smysl_text::link` chooses the pairs and `smysl_ingest::{link, summarise}` buy the answers, with `link --candidates` as the pure half so the generator can be read — and registered in the determinism matrix — without paying for a judgement. **GE-T11 passes at 0.6800** (34 of 50 planted far contradictions, default budget), measured with no model because the clause is about the generator: if the true pair is not in the candidate set no judge will find the contradiction. The linker turns the adjacency filter **off**, which is its one real difference from the same-as engines — GE-T11's word is *far*, and adjacency removes exactly the pairs a reader cannot find by reading forwards. A hub entity confirms and does not raise, because dropping hubs outright dropped the conjunction as well, and in Ruth everything mentions Naomi. The rule M clause is asserted and rule M is **not** reimplemented: `prepare_under` caps the summary and re-points the `x.text/summarises` edges onto the uid the cap produced. The three budget numbers were chosen from the measurement, after it, and §6 step 3 says so along with the curve. Step 4 measured: **GE-T5's instrument is met and its sheet is owed** (`ge_t5.rs`), which is GE-T2's shape and for GE-T2's reason. The within-language pair the engine exists for did not exist in this repository — one edition per language in the slice — so a second English expression of Ruth was written by hand, forty restatements and ten near-misses, and over it the engine reaches **precision 0.6724 at recall 0.9750** against no bar. The finding reverses draft 1's expectation: recall is nearly free, because a restatement sits at the same locator and keeps the names, and precision is the whole difficulty — from a neighbour count of two the engine proposes **all ten** near-misses, which a term overlap has no access to and TX-P10's embeddings are the answer to. The restated clause is implemented whole (α nominal, pooled gold by majority with ties dropped, the engine as one more coder, a paired bootstrap on the difference, one-sided) and reads δ 95% [−0.1278, 0.2189] over a *scripted* sheet — what `Met` looks like, reported as **owed**, because three independent coders are material and `SMYSL_GE_T5_SHEET` is how somebody else's arrive. Owed: the sheet, and with it the phase's exit | TX-P5 |
| TX-P8 **complete, exit applied** (2026-10-19) | 2.5 + 2.6 | dossiers, saved queries, thread schemas 6–8; measures T1–T7 | GE-T10 | TX-P6, TX-P7 | **Step 1 done 2026-10-17:** saved queries. `x.query/v1` is declared at last — A-11 left it open and nothing wrote one before — and `smysl-query/src/saved.rs` makes OQ-82 a signature: `to_records` takes a plan *and a run*, so a query that did not run to completion cannot be saved. The result edge is `x.query/answers` and the proof that it is inert is on the scores, bit for bit over every unit that existed before the save, because OQ-49 was a mistake about a label. `q --save` and `q --rerun` are the CLI; a rerun is pinned and reports the data effect, and `--current` is `SMY-E461` until TX-P12's lens engine can lower a query at two revisions. Two findings recorded as **OQ-84** (the per-partition limit is a recipe input 2.5 §3.6's table omits, hashed but uncarried) and **OQ-85** (a saved query ranks for its own terms, since its gist is its text; a rerun excludes it, a fresh `find` does not). **Step 2 done 2026-10-17:** thread schemas 6–8 and roles 24–48, the codes SMYSL-2.3 A-8.2 assigned ten days earlier, with `exposition` and `dialogue` derivable and `dossier` refused by `thread --derive` because only `ask` makes one. Three findings fixed where found: derivation grouped steps by role, which lays out an argument and runs a conversation backwards, so the two new schemas follow the units' own order (`SchemaDef::in_order`, with **span order** a new ordering key); a text with no ordering edges had no first or last unit; and **nothing writes `x.text/replies`** — the readers record `reply` ids that no path turns into edges, so the dialogue test draws them and the producer is owed. The five existing schemas derive byte for byte what they did, shown by pinning the H-18 golden to them by name. Older binaries read 6–8 and 24–48 as unknown, keep the codes and re-encode them unchanged (F-12); the F14 wire fixture moved its unknown codes from 7 and 30, which this step made known, to 9 and 64. **Step 3 done 2026-10-17:** dossiers. `smysl ask` is the thirty-third command and `smysl_query::dossier` its pure pipeline — a closed intent table for the five tier-1 languages in which the longest match wins, a subject found as the longest run of the question's words the entity port knows, eighteen slot templates compiled in as `smysl/dossier/1`, every slot planned, run and saved as an `x.query/saved` unit, and a `dossier` thread whose empty slots are `gap` steps on their saved queries — with `SMY-W417` for a name that means two things, `SMY-E468` for a question no rule reads and `SMY-E477` for an unknown template set. `ask --save --answer` is the one model-dependent invocation: the dossier is fenced as data, any provider the configuration routes `content-ingest` to answers, a unit grounding outside the dossier is dropped (`SMY-W478`) and rule M caps the rest at staging. Render profiles gained headings for roles 24–48 and only those. The registry is 103 codes. Five findings fixed where found, the first being that 2.5 §3.13's own template sketch had the traversal arrow backwards for this build; one raised as **OQ-86** — sq/1 cannot ask for only what a walk reached, so an edge-defined slot is written with `except` and loses a rebuttal that itself names the subject, which GE-T10 (step 4) is placed to measure. **Step 4 done 2026-10-17:** the GE-T10 run, and the exit of the phase's SMYSL-2.5 half; SMYSL-2.6's half — M1–M7, the text-level measures `T1`–`T7` and `measure` — is not started. `crates/smysl-query/tests/ge_t10.rs` over `fixtures/dossier/gold/` — ten subjects, thirty questions over all eight intents each asked in the five tier-1 languages, gold for 118 (question, slot) pairs — measures the slots alone, the front, and the two together. **The gold is single-hand**, not the expert-filled set GE-T10 names: one hand wrote corpus, questions and gold to criteria stated in the file, and the harness takes an expert's set by environment variable when one exists. Slots alone: *definition* 1.000 / 1.000, **counter-evidence 0.750 / 0.750**, all slots 0.774 / 0.890. **The kill criterion fires on counter-evidence**, and is applied: `ask` is labelled *Experimental* and each run says why on stderr, stdout unchanged. What fires it is not OQ-86, which costs recall, but a slot binding the subject and nothing of what was asked of it (**OQ-88**, four of six false positives) and a walk reaching the rebuttal of a superseded account (**OQ-87**, two; `revision` is empty for every question for the same reason). The front lost 22 of 150 to four kinds of gap in the intent table and the subject finder — a comma after the verb, a contraction, a plural verb, an inflected subject — recorded and not fixed, since a fix scored on these questions is scored in-sample. Two findings fixed where found: the example slot walked `<-exemplifies`, the wrong way, and was empty for every question; and a star of `same-as` edges is not a strict class (A-12.4 is a clique partition), so the corpus asserts every pair. OQ-52 is not decided: set metrics cannot see a ranking that only reorders what a template's limit already chose. **M1 done 2026-10-18:** the measure frame, SMYSL-2.6's half begun. `crates/smysl-measure` holds no measure yet and everything one needs: a recipe with every §3.4 field, hashed from canonical CBOR; payload numbers as integers — counts, unreduced ratios, `q20` fixed point — because a payload float is binary32 to 1/1024; `ln` and `exp` through `libm`, enforced by a crate-local clippy rule; a ChaCha20 generator keyed by recipe and seed; and the unit writer. Pinned in `fixtures/measure/golden.tsv`, language-free. It joins the purity gate's list with a scan of its own for a clock, the environment and unseeded randomness. **OQ-55 is decided**: a measure grounds on its scope saved as an `x.query/saved` unit, because the catalog units SMYSL-2.6 named do not exist — a manifest is a record, and a record cannot be a ground. Two of 2.6's ten codes are registered, `SMY-W483` and `SMY-E489`, in an eleventh group; the registry is 105. **M2 done 2026-10-18:** T1 (plug-in and Miller–Madow entropy) and T4 (Zipf by discrete power-law MLE with `f_min` by KS distance, Heaps by OLS), the segment bootstrap, and the text they read — a scope's `from` resolved to texts and read as one term stream, on `smysl_text::Chain`'s new `surface` and `stems` streams. §5.1's T1 and T4 rows and §5.3's seed rules pass, the slow coverage row at 96 and 95 seeds of 100, and both measures over the KJV's Ruth and Jude are pinned in `fixtures/measure/text.tsv`. Three findings recorded in 2.6 §3.7: a text scope is the query's `from`, since an unextracted text has no units for the rest of a query to select; α's interval resamples types, Clauset et al.'s bootstrap, because duplicated segments add no types and the segment interval missed its own estimate; Heaps' fit starts at √N, because `V(n) ≤ n` binds first. **OQ-89** raised: the same bias in T1's and β's segment intervals, flagged `ci-off-estimate` where an interval excludes its estimate. `SMY-W408`, `SMY-E480` and `SMY-W485` registered; the registry is 108. **M3 done 2026-10-18:** T2 (brotli rate, bits per character and per word) and T6 (normalised compression distance, with average-linkage clustering), over the characters a corpus now keeps beside its terms. §5.1's T2 and T6 rows pass, the window guard refuses two 9 MiB texts (`SMY-E484`, registered; the registry is 109), and the peak memory of one q11 lgwin 24 compression is recorded: 597 MB at a full window, three times 2.6's estimate. T2 over the KJV's Ruth and Jude, T6 over the four slice books, and the codec's lengths for the slice files are pinned in `fixtures/measure/compress.tsv`. Three findings recorded in 2.6 §3.7: the codec is brotli **without `std`**, because with it the encoder's cost model reads the platform's `log2` and 24 of 29 files changed length between the two builds; T2 is the codec's rate, brotli's UTF-8 contexts being classes of characters, so §5.1's Markov row is a chain the codec can model; and the peak follows the text as well as its length. **M4 done 2026-10-18:** T3 (match-length entropy rate, the increasing-window and mean-ratio forms) over a hand-rolled SA-IS, Kasai's LCP and the longest-previous-factor array, per text and aggregated harmonically, over characters or terms. LPF equals its O(n²) definition on 10⁴ random inputs, and the calibration table (H = 1…8 bits, 10³…10⁷ symbols) ships as `fixtures/measure/t3-calibration.tsv`, its rows to 10⁵ checked bit for bit. T3 over the KJV's Ruth and Jude is pinned in `fixtures/measure/lz_match.tsv`. Three findings recorded in 2.6 §3.7: §2.9's probe is one draw of another generator, so its table is checked in distribution; the estimator steps in n on large alphabets, which bears on OQ-59; and the peak is 23–29 bytes a symbol, about 100 MB for a 4.3 M-character text. **M5 done 2026-10-18:** T5 (n-gram surprisal, interpolated Kneser–Ney, order 3 by default), each segment scored by a model of every other segment, the removal exact because the model is integers and its discounts and vocabulary are recomputed from them. A 30-token corpus worked by hand and 2,000 random corpora against retraining from scratch pass; T5 over the KJV's Ruth and Jude is pinned in `fixtures/measure/kneser_ney.tsv`, its top verses by locator. 155 MB for 800,000 tokens. **M6 done 2026-10-18:** T7 (Jensen–Shannon between every two texts in scope, with its root; collocations by Dunning's G² and by PMI over adjacent pairs). JSD is computed over integer products, so equal distributions give 0 and disjoint supports 1 exactly — every cross-language pair of the slice books is 1 — and G² ranks attraction only. Its G² tables are recomputed by the Python harness (`python/tests/test_measure_g2.py`); T7 over the four slice books is pinned in `fixtures/measure/divergence.tsv`. The text-level measures T1–T7 are complete; M7 is the `measure` command. **M7 built 2026-10-18:** `smysl measure`, the thirty-fourth command, one recipe per id, `--seed-check`, `--dry-run`, `--json`; over the five-Bible slice a second run of T1–T4 adds no unit. GE-T6's pure arm over the five full Bibles keeps both T2 and T3 in all five languages (Kruskal–Wallis p ≤ 0.02, ε² 0.34–0.56), law the lowest genre under both in every one; `fixtures/measure/ge-t6.tsv`. A four-platform matrix job (`measure-platforms`: Linux glibc and musl, macOS arm64, Windows) is added and runs first on the push that carries it. CI had been red since 2026-10-10 for three reasons the local sweep could not see — a newer rustdoc lint, two ungated CLI test files, a shallow checkout for `semver` — all fixed in M7. **M7 done 2026-10-19:** the matrix job's first run (CI 39193002211) is green on all four targets; the same run found `tests/cmd_q.rs` running the binary without `cli`, hidden locally by a stale binary, now gated and checked from the sources by `tests/dispatch.rs`. **Exit applied:** every step of SMYSL-2.5's (1–4) and SMYSL-2.6's (M1–M7) is done, and draft 3's exit asks for GE-T10's numbers recorded and its kill criterion applied, not passed — it fired on counter-evidence, 0.750 against 0.8, so `ask` ships *Experimental* (2.5 §6 step 4). Lifting the label is OQ-87's and OQ-88's, measured on fresh questions. **2026-10-28:** both decided as template set 2 (2.5 OQ-87, OQ-88), and the criterion is met — counter-evidence 1.000 over set 1's questions and over seventeen fresh ones (`fixtures/dossier/fresh/`), against 0.478 for set 1 on the fresh ones. The label stays by decision: single-hand gold, and counter's recall on the fresh questions is 0.471. |
| TX-P9 **exit met** | 2.6 | assertion, channel, dialogue and temporal measures; `assess` | GE-T3, GE-T6 | TX-P8 | **N1 done 2026-10-19:** A1–A7 and A10 over a scope's units, in `smysl_measure::graph`, and E6's count moved into A5. A synthetic 10⁶-unit scope runs A1, A2, A4, A5 and A7 in 12.5× the time of 10⁵ (bound 20×), after the strict class partition in `smysl-graph` was found quadratic and made O(V·d) with the same classes. Draft 3's default filter selects no unit this build writes, so a unit measure's default is the scope itself. **N2 done 2026-10-19:** A8 and `full_tokens`/`floor_tokens` in `smysl_measure::graph::rd`, `SMY-E407` on a scope in two languages. The greedy pack's loss is not monotone in budget on 12 of 534 curves, so the curve is the least loss within each budget, as a rate–distortion function is; A8 is 35.4 s at 10⁶ units, 11.6× per decade. **N3 done 2026-10-19:** A9 over the entities a scope's units mention; two planted cliques recovered exactly, with and without a bridge; centrality over `A + I` and an entity's own label in propagation, so neither oscillates on one edge. **N4 done 2026-10-19:** C1–C5 over a source and the target `--channel` names, since sq's `channel:` is a conversation; planted ratios exact, and on the Psalms fixture KJV → Synodal is survival 6/7 and support 6/8. Accepting every `anchored` candidate pairs Psalm 51 with the wrong verses and leaves C1–C4 unchanged to the digit, so a channel measure is as good as its `same-as`. **N5 done 2026-10-19:** D1–D6 over a conversation, each on its own planted chat, exact. A response counts only with both ends in the scope, so a reply from another conversation is not one; an answer said before its question is counted and given no time; D5 compares full windows only. GE-T8's instrument is built and its measurement owed: it needs extracted chat edges and a person's labels. **N6 done 2026-10-20:** `assess key|score|state|diff`, the thirty-fifth command, and α; a reader scored against seven planted items exactly, a moved class id kept by the diff, and Krippendorff's worked example 113/152 (his 0.743). `SMY-W487` and `SMY-W488` registered; the registry is 114. A reader is `human:` or `model:`, not `reader:`; ingest has no reader flags, so K2 arrives with the model arm; SMYSL-2.4's engines rank without a threshold, so the judge is asked across the whole key. GE-T9's annotators are owed. **GE-T3 measured 2026-10-20:** 59 verses × 5 languages × 5 DeepSeek runs; run nested in language, not crossed with it, and language exceeds run for units (74.90 against 21.76) and classes (24.34 against 9.20), so cross-lingual `A*` carries the bias. **N7 built 2026-10-20:** GE-T6's A8 arm, with the statistics both arms read moved into one module and the pure arm re-run on it byte-identical. A book's value is A8's area over the units drawn from its text, the median of its runs, and the noise the kill criterion reads is the variance across runs; A8 has no length control, because its budgets are fractions of the book's own cost. GE-T2's runs are not its material — 59 lines per language in three runs, in a scratch library the harness removes — so the arm owes its own 5 × 5 runs of the canon. Every step's exit test is met; **GE-T6's A8 arm measured 2026-10-20:** 5 Bibles × 5 DeepSeek runs of 66 books at chapter windows; A8 kept in 5 of 5 languages (H 19.4–28.3, p ≤ 0.0005), and kept without the 43 books a window was cut off in. A first set at verse leaves and 32k windows had 564 of 1,650 book-runs cut off at the 8,192-token output limit and read A8 dropped — the truncation's variance. The recipe does not record the context window. Both draft-3 exit tests are measured; GE-T8's measurement and GE-T9's human α stay owed. |
| TX-P10 **complete** (2026-10-27) | 2.5 | S0/S1 embeddings, semantic sources and bridges, NL front end | GE-T5 cross-lingual arm, GE-T12 | TX-P6 | **Step 1 done 2026-10-26:** S1's dependencies verified — candle 0.9.2, since from 0.10 `candle-core` takes `tokenizers` with Oniguruma's C unconditionally; XLM-R present; 1.88, the facade's floor — and declared in `smysl-embed-candle` behind `semantic-candle`; `xtask check-native` in `gates` and CI, clean on all four feature sets after S0 moved to `model2vec-rs` 0.3 `wasm` (step 2's pin, taken here: 0.2.1 compiled `esaxx-rs`'s C++). **Step 2 done 2026-10-26:** `potion-multilingual-128M` loads through `from_dir` on 0.3; its smoke test finds each of four claims across the five tier-1 languages in 80 of 80 searches against a bar of 40 set before the run; `semantic.rs`'s English rankings are `potion-base-8M`'s, which passes them on 0.3 and the multilingual model does not. **Step 3 done 2026-10-27:** `smysl-embed-candle`'s `Encoder` (BERT for `e5-small`, whose config says so, and XLM-R), `Pooling` in plain loops, the model hash over the files, pooling and prefixes, and `VectorCache` in `<store>.vec-<hash12>` with `SMY-W476`; one text a forward pass so a cached vector is the encoder's to the bit; ten S1 tests on generated models, and `multilingual-e5-small` meets step 2's cross-language bar at 80 of 80. **Step 4 done 2026-10-27:** the `SemanticSource` port and one `Retriever` adapter for S0 and S1; `~sem` per partition with RRF, `lang:any` as the embedding bridge (one ranking), `~hybrid` by the measured routing (an identifier plans pure), `near` with a threshold in (0, 1]; purity reasons and `query:models` on every plan, `SMY-E416` under `--seed-check` and `SMY-W467` on a rerun under another model, both registered (148 codes); `q --engine-model`/`--candle-model`. **GE-T5's cross-lingual arm ran** over the stability corpus: top-1 0.9322 and precision 0.8568 at recall 0.7 for S1, 0.6814 and 0.2012 for S0. **Step 5 done 2026-10-27:** `Translator`, `TranslateContext` (the store's vocabulary as used) and `accept`, which refuses what does not parse, binds a parameter, writes or dispatches, or does not plan — `SMY-E474`, registered (149 codes); `smysl::translate::ProviderTranslator`; `q --from-question "…" [--yes]`, the query printed first and run on `--yes` or a `y`, else exit 10. **GE-T12 met: 82 of 100** equivalent results against local `qwen2.5:14b-instruct`, bar 70 — over a hundred questions on F1–F5 written by one hand before any model saw them; the misses are vocabulary, direction, invented stages (refused) and over-specific terms. Owed: GE-T5's coder clause (2.5 §6) |
| TX-P11 **complete** (step 13, TX-P12's, done 2026-10-22) | 2.7 | `lib import/status/accept/discard/export/verify` | G2 (GE-T15) | TX-P6, TX-P7 (TX-P12 for `--target-revision`) | **Step 1 done 2026-10-21:** `Subject::Record`, the writer's one `has_surface_form` predicate (now what `merge` and `bundle` count omissions by), `Store::record_hash`/`contains_record`/`pending_attestations`, and `record_set_digest` over records not in a store. The count `merge` kept beside the writer still called every redaction and dating omitted — 4 on the manifest fixture — over output holding them all. **Step 2 done 2026-10-21:** crate `smysl-lib` with `export_store` (`single`, `by-view`, `full`): §3.6's table and fixpoint, canonical files, in-memory P1 over F1–F13 and F14; `SMY-W425`/`W490`/`W495`/`W497` registered. The exit completed the table three times: a declaration used only by payload keys (F11), a resolution of a detected contention, and records dangling at source (F10's resolution), which no row could place. **Step 3 done 2026-10-21:** `ref` closure, `export-index.cbor` (strict, canonical), `verify` (file checks with `SMY-E494` and exit 9, P1, P3) and the duplication factor; `verify` holds on F1–F13, F10 and F14 × {`single`, `by-view`} × {`full`, `ref`}. P3 found a fourth completion: a `by-view` file carries the view it is keyed by. **Step 4 done 2026-10-21:** import into `smysl_text::Library` behind `LibraryWrite`: decode (`SMY-E493`), one manifest per file, accept/stage/wait/quarantine (`SMY-W423`, `SMY-W496`), the tables in their CBOR fallback, promotion, `accept`/`discard`/`status`; P1 now holds through a real import. Two findings: the check runs only over what does not wait (a missing ground is `E060`), and a unit whose finding rests on another's waits rather than being quarantined, which is what makes imports commute. **Step 5 done 2026-10-22:** `smysl lib` with six actions (`import`, `status`, `accept`, `discard`, `export`, `verify`) as values of `<ACTION>`; `--format` widened to `pack`, refused everywhere but `lib export`; `lib export … && lib verify` exits 0 on F1–F13 under both layouts and both closures; 36 commands. **Steps 6–12 done 2026-10-22:** text in collections (`--with-text` under the manifest's carry and licence, rule Z at the door with `SMY-W414`, `--keep-original`), `by-expression`/`by-collection`, `by-time` and `--as-of` (`SMY-W498`), `--scope` through `sq` with `by-query` and `from import:<name>` resolving to the export roots, span re-verification (`E404`) and the two joins into `join:<name>`, the `surface` (`SMY-W428`/`SMY-E424`) and `pack` formats, the `lib_roundtrip` and `lib_import` fuzz targets (an hour each, no finding), `verify_export` in Python, JavaScript and Go with the rsd vectors, P2 (`lib verify --p2`), and GE-T15: 1,056 cells, P1/P2/P3 hold, no record type lost, `by-expression` duplication 1.000, 10⁶ records exported in 4.8 s and imported in 21.4 s. Findings: P3 through a library must re-export what the import retained, not the log alone; under `ref` a label binding whose unit another file declares has no surface form; `single` writes its file even when nothing is keyed (F12); import held two copies of the library at scale. **Step 13 done 2026-10-22** (with TX-P12 steps 1 and 2, below): `lib export --target-revision ID@N`, with `--agent` and `--clock`, rewrites the selection through the schema's lenses as a store the normal pipeline exports; a `split-value` exports, a `retire` on the way back refuses (`SMY-E422`), and a two-level grounds chain gets new uids dependencies first with every rewritten core referencing only mapped uids. `SMY-W491` and `SMY-E492` registered (135 codes). |
| TX-P12 **complete** (2026-10-23) | 2.5 | FC-9 lenses, rule N checks, pinned queries, `migrate`, pointer lenses, `--target-revision` | GE-T16 | TX-P8 | Begun out of order on 2026-10-22, because `--target-revision` (SMYSL-2.7 step 13) needs the lens engine and nothing else of the phase; steps 3–9 followed on 2026-10-22 and 23. **Step 1 done 2026-10-22:** FC-9 in `smysl-core` (`smysl_core::schema`): a declaration's field table and lens steps decoded, the steps kept as received so every declaration re-encodes to its own bytes (L9 on `fixtures/schema/fc9/`, which Python, JavaScript and Go hold too), `SchemaSet` with current revisions and forks, `Store::schema_decls`; `SMY-W430`, `SMY-W429` and `SMY-W469` in a thirteenth group. Found: a step with no ops lost its required key 2 to the envelope's omit-if-empty array writer. **Step 2 done 2026-10-22:** the lens engine — chains, their hash, composition, forward and backward application and revision inference — with `SMY-E470`, `SMY-E471`, `SMY-E422` and `SMY-W420`; laws L1–L7 hold at 10,000 seeded cases each over generated rule-N histories, and the `lens_laws` fuzz target holds L3 and L9 over arbitrary bytes. Inference is one rule, *a unit a step would change was written before that step*, which is what makes L5 hold. **Step 13** is SMYSL-2.7's, recorded in the TX-P11 row. **Steps 3–9 done 2026-10-22/23:** field resolution and compile-forward in `sq` (§19.5's rewrite printed as §4.4 shows it; `SMY-E421`); the schema/data effect split (`q --rerun --current`, `--current --save`); `smysl schema show|check` (rule N, `SMY-E426`, the `schema/rule-n/` fixtures); `smysl migrate` (staged, `Transformed` with the chain hash, `supersedes`, commitments and datings proposed; `SMY-W472`, `SMY-E473`); `fields:` and `lenses:` at the surface (a `fmt` round trip of Appendix C); pointer lenses over `json/1` (`SMY-W475`); and GE-T16 — the lens path equals the migrate path on 100 saved queries and 30 dossiers, the effect split reproduces the injected quotations, 0 of 254 units unmappable. A unit is governed by its type's schema or by the one whose tables declare its payload keys, which is what lets kernel claims carrying `text:mode` be lensed. 38 commands, 140 codes. |
| TX-P13 **complete** (13a done 2026-10-23: thread fix re-measured, single hashing, lazy adjacency, `StoreRead`/`StoreWrite` with the conformance suite, every §4.3 consumer ported, `Overlay` under ingest staging, `AnyStore` under the CLI; staging's < 50 ms exit not met — `check` is the floor; **13b.1 done 2026-10-24**: the redb unit store in index mode, `store-redb`, on redb 2.6.4 — `check`'s peak RSS on graph-155510 35% of the in-memory store's, after the `Time` pass stopped solving rule E with nothing to solve; OQ-67 and OQ-69's tiering answered, OQ-68 half; **13b.2 done 2026-10-25**: `RedbShard`, the writer, behind the shard's lock file; one database per file per process; `--backend redb` and `reindex --backend redb`; §5.3 whole; the ladder at 10⁶ completes on redb in 3.3 GB, `check` at 24% of memory; found and fixed, a retraction plan that wrote to the log; **13c.1 done 2026-10-25**: `Union` of shards, `UidShards`, `SMY-W500`, `--shard`; library layout 2, one log per expression with writers routed by alias; conformance implementation 6, `store_union` fuzzed 10 min clean; **13c.2 done 2026-10-26**: contentions over the union, lazy and kept (OQ-16 decided); GE-T7 run on a stand-in of 1,000 works and 10⁶ units, four thresholds of seven met — not the single-record append (5.01 ms against 5, the disk's `fsync` about 4 of it), the scoped query (OQ-91, a 2.5 step) or `check` on redb (OQ-90, step 13c.3: one redb database per library, before 13d); three quadratic costs in the library's write path fixed; **13c.3 done 2026-10-26**: one redb unit store per library, `index/units.redb`, every row naming its shard's log (OQ-90, narrowed by measuring first: one store, not one file of per-shard stores); `check` on the stand-in on redb 2.69 → 1.69 GB, open 1.19 GB → 277 MB, 3.5 GB → 540 MB on disk; ≤ 1 GB not met — the `time` pass alone peaks at 1.41 GB, recorded for 2.4; **rule E at scale, 2026-10-26** (with 2.4 §3.3): a contradiction no longer contests every dated subject, the network holds only what anything bears on, and the axis tables rewrite only those rows — `check` on redb 885 MB (≤ 1 GB met), building the stand-in 528 → 68 s; six of GE-T7's seven thresholds met on the stand-in, the scoped query (OQ-91) left; **13d done 2026-10-26**: primary mode per library shard — `lib migrate --primary/--index`, `lib verify --primary-shards`, `SMY-E505`; the stand-in rerun on primary shards: `check` on redb 889 MB, the single-record append 5.91 ms against 5 (one redb commit), not met; **GE-T7's scoped query met 2026-10-26** by 2.5's OQ-91 step — every GE-T7 threshold met on the stand-in in index mode; **the single-record append met on primary shards 2026-10-26** — routing looks up only the parts a batch cites, the store's metadata written only when it changes: 4.0–4.7 ms, and every GE-T7 threshold met on the stand-in in both modes; 2.8 §6.1) | 2.8 | 13a trait in memory; 13b redb index; 13c shards and union; 13d disk-primary | G3 (GE-T7) | 13a: TX-P0. 13b: TX-P4. 13c: TX-P5, 13b. 13d: G2. |

---

## 3. Decisions taken for the set: owner checklist

These are **proposals**, applied consistently across the set. Each can be overruled. Each row
names where a change would land.

| # | decision | resolves | where it lives | if overruled, change |
|---|---|---|---|---|
| D-1 | the copy set on units: ref, span, manifest, published or observed, lang, speaker | OQ-17 | 2.3 A-2.4 | 2.3, 2.4 §3.4 |
| D-2 | `published`, `span`, `manifest` inside `source`; unknown source keys rejected; bare `lang:` reserved; lint W432 first | OQ-1 | 2.3 A-10, 2.1 | 2.3, 2.1 |
| D-3 | statuses of free constraints | OQ-10 | 2.3 A-12.2 | 2.3, 2.4 §3.3 |
| D-4 | a scholarly dating takes its basis's status; equal-status conflicts become contentions | OQ-12 | 2.3 A-12.2 | 2.3 |
| D-5 | closed lens op set, open tag space | OQ-23 | 2.3 A-12.1, App. B | 2.3, 2.5 |
| D-6 | holder, mode and speaker stay in `x.text/v1` | OQ-6 | 2.3 A-11 | 2.3 |
| D-7 | `strict` classes normative (seed-and-grow greedy partition) | OQ-7 | 2.3 A-12.4 | 2.3, 2.4 §3.6 |
| D-8 | conformance: C-Read additions, new **C-Library**, lenses not obligatory | — | 2.3 A-13 | 2.3 |
| D-9 | new rules N, E, Z | — | 2.3 A-12 | 2.3 |
| D-10 | security owners: prompt injection (2.1, 2.4), reader limits (2.4), concurrency (2.4, 2.8) | — | §5 | — |
| D-11 | property and fuzz harnesses in the same phase as each complex feature | — | §5 | — |
| D-12 | spike S0 before TX-P1; TX-P0 in parallel | — | §1 | — |
| — | open enumerations as a unit `Unknown` variant, code 255 reserved, not `Other(u8)` (keeps `as` casts compiling: no 2.0) | OQ-30 (part) | 2.3 A-8.1, 2.1 §4.3.6 | 2.1, 2.3 |
| — | estimator is a wire field (granularity key 5) | OQ-29 | 2.3 A-9 | 2.3, 2.1 |
| — | time contentions derived, not recorded; detection kinds 4 and 5 | OQ-35 | 2.3 A-8.2 | 2.3, 2.4 |
| — | a chain of constraints tightens at its weakest link's status | OQ-42 | 2.3 A-12.2 | 2.3, 2.4 |
| — | saved-query results by `x.query/answers`, not kernel `answers` | OQ-49 | 2.3 A-11 | 2.3, 2.5 |
| — | `converged_with` compares the record-set digest and `state_hash` | OQ-27 | 2.1 §4.3.10 | 2.1, 2.7 |
| — | the lens engine lives in `smysl-core` | OQ-48 | 2.5 | 2.5 |
| — | export inclusion table and P1–P3 are tool-level, owned by 2.7; 2.3 reserves `import:` (manifest aliases) and `export/` (view ids) | OQ-61 | 2.7, 2.3 A-5 | 2.7, 2.3 |
| — | one lens-chain hash, prefixed `smysl/lens-chain/1` | — | 2.3 App. B | 2.3, 2.5 |
| — | part bytes that do not match the tid: `SMY-E446`; `SMY-E401` keeps its draft-3 meaning | — | 2.3 A-3 | 2.3, 2.4 |
| — | `ingest --temperature` and the model in the recipe ship in TX-P0 | OQ-32, OQ-33 | 2.1 H-12, H-9 | 2.1, 2.2 |
| — | `find` and `pack` relabelled `Mixed`; `--seed-check` enforced | OQ-54 | 2.1 H-19, H-20 | 2.1, 2.5 |

---

## 4. Open questions, by the phase they block

Everything not listed here is either resolved (§3) or does not block anything.

| must be answered before | open questions | owner |
|---|---|---|
| ~~**S0 / TX-P0**~~ **both done** | OQ-34 licensing of model outputs as fixtures — **partly answered by S0**: arm L's model is Apache 2.0, so its outputs are usable as fixtures; the hosted model's still needs the decision | 2.1, 2.2 |
| **TX-P1** | ~~OQ-31~~ **answered, and F-2 is built** (2026-10-07): faithful for cost, content-fair for a bound — 2.1 §4.3.2. · ~~**MSRV: OQ-40 = OQ-66**~~ **answered 1.10.0 by measurement**: 1.79 was already false everywhere — a 1.79 Cargo cannot parse this tree, because `blake3` pulls an edition-2024 `constant_time_eq`. Floors declared per crate (1.85 base, 1.86 provider and ingest, 1.88 tui and facade), `make msrv` gates both directions, and the CI job exists. redb 4.x's 1.90 is an ordinary bump when `store-redb` lands. · ~~OQ-36~~ **answered** (2026-10-07): `create_new`, not advisory locks — `File::lock` is unstable at 1.88, and a kernel lock names no holder and dies with the crash it was evidence of. · ~~OQ-37~~ **answered**: `serde_json` behind `reader-json`; a strict HJSON mode is new code, not a narrowing, and the purity gate's single list was two claims with only the weaker enforced. · ~~OQ-39~~ **answered**: refuse 15/18 in a log (`E452`); a log that can be rewritten is not append-only, and nothing depends on the bytes yet. **No planning blockers left here.** | 2.4, 2.8 |
| **TX-P3** | OQ-13 speaker and device selectors for datings (window targets are already in) | 2.4 |
| ~~**TX-P4**~~ **both answered** | OQ-4 source and licence of versification maps — **not needed**: a map installed in a library is a mechanism, a map distributed with the crate is a dataset, and step 4 shipped the first. The licence question stays open with `data/versification/` empty · OQ-14 default IDF scope — **answered: the query's declared scope, default collection** (2.5 §3.2, §3.4). The cost argument that made it provisional is gone: `df`, `docs` and total length are additive over disjoint partitions, so every scope is exact from one table with no rebuild. Partition as a default has a structural defect — a one-document partition weighs every term alike and takes rank 1 under RRF — and library inflates `N` with corpora the query did not ask about | 2.4, 2.5 |
| **TX-P5 (G1)** | OQ-41 `ingest:quote` removed from span-carrying units changes their uids · OQ-43 strictness of the digest-scoped reference check | 2.4 |
| **TX-P6** | ~~OQ-45 RRF constant~~ (decided 2026-10-28: fixed `k = 60`, recorded, not a parameter — over partition fusion it cannot reorder a unit one partition ranks) · ~~OQ-46 fuse or route `~hybrid`~~ (decided 2026-10-28: route — RRF measured over three corpora and both S0 models ties or loses, by 0.14 MRR across languages) · ~~OQ-47 is `question` interpretive~~ (decided 2026-10-28: `sq` routes on the text's shape alone; `Question` stays interpretive in `Hybrid`) · ~~OQ-50 `match` subset~~ (decided 2026-10-28: none in SMYSL-2; `SMY-E461` and the capture stay) · ~~OQ-21 the name `sq`~~ (decided 2026-10-28: kept — `using sq/1` is in every saved query) | 2.5 |
| **TX-P7** | OQ-44 auto-accept lexical same-as · OQ-15 partial dependence between witnesses | 2.4, 2.5 |
| **TX-P8 / P9** | ~~OQ-52~~ dossier slot ranking (decided 2026-10-28: the product, kept, after GE-T10 compared it with a lexicographic order and RRF) · OQ-22 intent classifier · ~~OQ-55~~ status and grounds of measure units (decided in TX-P8 M1: the saved scope query) · OQ-56 payload number layout · OQ-57 bootstrap generator · OQ-58 class identity across time · OQ-59 T3 bias correction · ~~OQ-86~~ sq/1 cannot ask for only what a walk reached · ~~OQ-87~~ a walk reaching a superseded account's rebuttal · ~~OQ-88~~ a slot binding the subject and nothing of what was asked (all three decided 2026-10-28: `expand … only`, template sets 2 and 3) · ~~OQ-84~~ the per-partition limit, ~~OQ-85~~ saved queries ranking for their own terms (decided 2026-10-28) · ~~OQ-89 segment-bootstrap bias for statistics of rare types~~ (decided 2026-10-28: a grouped jackknife for T1 and β) | 2.5, 2.6 |
| **TX-P11** | OQ-60 what P1 compares · OQ-62 `--format pack` and `-o DIR` · OQ-63 clock of exported attestations · OQ-64 threads spanning files · OQ-25, OQ-26, OQ-28 | 2.7 |
| **TX-P12** | OQ-53 nested payload keys in lenses · OQ-24 always state the lens chain | 2.5 |
| **TX-P13** | OQ-65 generic entry points vs `_in` twins · ~~OQ-67 cross-process readers~~ (wait, then the log; kept at 13b.2) · ~~OQ-68 home of library-wide tables~~ (a uid list per shard, `index/shards/<alias>.uids`; answered at 13c.1) · OQ-69 tiering policy (tiering answered; GE-T7's build definition open) · ~~OQ-16 lazy contentions~~ (lazy; decided at 13c.2) · ~~OQ-90 one redb unit store per library~~ (done at 13c.3) | 2.8 |
| **not blocking** | OQ-2 (stems only, settled in practice) · ~~OQ-3~~ **answered by S0**: the chosen Synodal module contains one `ё` in the whole Bible, so the Russian F-3 measurement rests on model-introduced `ё`, not the source · OQ-5 · OQ-8 (2.4 stores 15 and 18 as objects) · OQ-9 (hand-rolled EDTF) · OQ-11 · OQ-18, OQ-19, OQ-20 · OQ-30 (rest) | various |

---

## 5. Cross-cutting concerns

### 5.1 Conformance and the other implementations

| class | what it gains | implementations |
|---|---|---|
| C-Read | records 14–19, key 9, source keys 4–6, schema-declaration and granularity key 5, unknown enumeration codes, tid/mid/did/rdid, record-set digest | Rust; then Python, JavaScript and Go: first target, about 15 lines each for the digest, plus records |
| C-Consume | the *said* axis; unknown enumeration values | Rust |
| C-Produce | copy and omission, `lang`, EDTF, rule N, canonical uids in payload | Rust |
| C-Merge | did-targeted withdrawals and commitments, rule Z, convergence by digest | Rust |
| **C-Library** | part and reading verification, rule Z, rule E, `strict` classes | Rust. A brute-force rule-E reference on small networks (2.4 §5.3, P-E6) is the test oracle. |

### 5.2 Security

| concern | owner | summary |
|---|---|---|
| prompt injection | 2.1 (today's ingest), 2.4 (text ingest, ledger digest, linker) | fences around untrusted text, explicit instructions, model references limited to the batch and digest (`E442`), model-supplied metadata dropped (`W444`), red-team fixtures |
| reader resource limits | 2.4 §3.9 | caps on decompressed size, entry count, nesting, part bytes, and a deterministic work budget. Exceeding a cap is an error, never a partial read. |
| concurrent writers | 2.4 (lock file, one writer per shard), 2.8 (redb, cross-process readers) | readers are lock-free on immutable objects |
| erasure | 2.3 rule Z, 2.4 | redaction survives merge. Older peers can still pass text on: stated. |
| licences | 2.4, 2.7 | `carry` and licence gate every export of text |

### 5.3 Property and fuzz harnesses (D-11)

| subject | harness | RFC |
|---|---|---|
| effective time (rule E) | P-E1–P-E8, with a brute-force reference on small networks | 2.4 |
| redaction merge (rule Z) | P-Z1–P-Z4 | 2.4 |
| readers | cargo-fuzz targets per reader | 2.4 |
| lens laws | forward and backward on invertible ops, composition, query compilation equivalent to migration (GE-T16) | 2.5 |
| inclusion rules and round trip | random stores → export → import → digest equality; fuzzed import inputs | 2.7 |
| `StoreRead` | one conformance suite run over every implementation, which must give identical bytes | 2.8 |
| bundle closure | no bundled record names an absent unit | 2.1 |

The repository has no `proptest` today. The RFCs use its existing convention (seeded generators
plus `cargo fuzz` targets under `fuzz/`) rather than adding a dependency.

---

## 6. Registries

### 6.1 Diagnostics

| range | owner | allocated |
|---|---|---|
| E401–W431 | draft 3 / 2.3 | as in draft 3 Appendix D (table below). 2.3 uses E401, E403, E404, W405, W409, E410–W414, W420, E426, W429, W430, and 2.4's E446. |
| W432–W439 | 2.1 | W432 bare `lang:` lint · W433 fence marker in input · W434 bundle unknown-record count · W435 repaired attempt history |
| E440–W459 | 2.4 | E440, W441, E442, W443, W444, E445, E446, W447, W448, W449, E450, W451 (452–459 free) |
| E460–W479 | 2.5 | E460, E461, E462, W463, W464, W465, E466, W467, E468, W469, E470, E471, W472, E473, E474, W475, W476, E477, W478, W479 |
| E480–E489 | 2.6 | E480, E481, E482, W483, E484, W485, W487, W488, E489 (E486 withdrawn: use E416) |
| W490–W499 | 2.7 | W490, W491, E492, E493, E494, W495, W496, W497, W498 (W499 reserved) |
| W500–W509 | 2.8 | W500, W501, E503, W504, E505 (E502 withdrawn: use E445) |
| W510–W519 | 2.9 | W510, E511, E512 (W513–W519 free). Both errors are planner refusals filed under 2.9 because 2.5's range is full and because each refuses something the grammar accepted: `E511` a name (`from store:ledger` parses), `E512` an operator the grammar cannot type-check (`where kind < claim` is the same production as `where status < cited`). 2.9 §8 records the reasoning for both |

Each RFC's §8 has the meanings of its own range. The draft-3 codes, which every document uses,
are:

| code | meaning |
|---|---|
| E401 | structure hash or rdid mismatch on re-read (part bytes that do not match the tid are `E446`, 2.4) |
| E402 | `carry: text` refused by licence |
| E403 | malformed tid, mid, did or rdid, in a record or in `source.manifest` |
| E404 | span out of range |
| W405 | locator and span disagree (span wins) |
| W406 | holder or speaker not in `deps` |
| E407 | byte estimator on a `mul` scope |
| W408 | measure on a lossy text, when allowed |
| W409 | unknown enumeration value or estimator id preserved (thread schema, role, source kind, detection kind, admission; estimator) |
| E410 | malformed EDTF |
| W411 | `observed` outside the `published` interval |
| W412 | dating not applied (lower status than the value it would override, or held by a curated lock); contention opened |
| W413 | temporal inconsistency; contention opened |
| W414 | source redacted |
| W415 | units excluded from a time-scoped result for lack of a time on the requested axis (undated units; units with no known time under `--as-of`); count reported |
| E416 | `--seed-check` on a model-dependent query |
| W417 | ambiguous dossier subject |
| W418 | expression fork: two manifest heads |
| W419 | ambiguous attribution: the quote occurs more than once in its node |
| W420 | a lens cannot map a value; the unit is excluded from predicates on that field and counted |
| E421 | a query names a field that no schema in scope declares |
| E422 | a pinned query names a key or value retired without a mapping, or `lib export --target-revision` meets a `retire` on its lens path |
| W423 | an imported record failed a check and was quarantined |
| E424 | round-trip property violated (P1, P2 or P3); the export index is not written as complete |
| W425 | unknown records exported to `unattributed.cbor`, or dropped with `--unknown drop`; count reported |
| E426 | the naming rule is violated (§15.2): a name gets a different meaning without a paired widening, or a retired name is reintroduced |
| E427 | unsupported `sq` version |
| W428 | a surface export omitted records surface text cannot spell (F-18); count reported |
| W429 | schema fork: two declarations of one id and version with different content; contention opened |
| W430 | a `payload_shape` that does not match FC-9's structure; it is treated as opaque, as today |
| W431 | an extracted unit lacks a key required by the revision its recipe names |

**One condition, one code.** Two duplicates found in review were withdrawn: 2.6's E486 (use
E416) and 2.8's E502 (use E445).

**This registry is the set's.** The format spec does not carry a diagnostics table, and its
`spec-tables` gate checks format constants, not diagnostics. Codes enter the `Code` enum in
`smysl-core/src/diag.rs` (`#[non_exhaustive]`, so additions are additive) as their phase
lands.

### 6.2 Open questions

| range | owner |
|---|---|
| OQ-1–OQ-28 | draft 3. Status per question in §3 and §4. |
| OQ-29–OQ-31 | 2.1 |
| OQ-32–OQ-34 | 2.2 |
| OQ-35–OQ-44 | 2.4 |
| OQ-45–OQ-54 | 2.5 |
| OQ-55–OQ-59 | 2.6 (OQ-55 decided in TX-P8 M1, 2026-10-18: a measure grounds on its scope saved as an `x.query/saved` unit, since the catalog units 2.6 named do not exist) |
| OQ-60–OQ-64 | 2.7 |
| OQ-65–OQ-69 | 2.8 |
| OQ-70 | 2.4 (found in TX-P2 step 2; the decision is an SMYSL-2.3 A-5 amendment) |
| OQ-71 | 2.4 (found in TX-P3 step 2): **what does a dating's arithmetic mean?** Three sub-questions SMYSL-2.3 names words for and gives no arithmetic. (a) *A qualifier is not a width*: A-2.3 says widening for `?` and `~` "is SMYSL-2.3's rule" and A-12 states no such rule, so the engine gives `1920~` the extent of `1920` — any width would be invented, and an invented width propagates. What a qualifier plausibly *should* affect is time status. (b) *A season is its year*: EDTF numbers the seasons 21–24 and fixes no hemisphere, so narrowing one to three months assumes one, and northern winter reaches into the next year. (c) *Four Allen relations collapse*: with a subject that is an instant with uncertainty rather than a duration, `during`, `overlaps`, `contains` and `equals` all read as "at the same time as". Each implemented reading is **weaker** than the relation it stands for, so none can exclude a truth the strict reading admits — the collapse costs precision, never soundness. |
| OQ-73 | 2.4 (found in TX-P3 step 3): **the surface form cannot spell a did.** A-6 lets a withdrawal and a commitment name a dating in key 0, and `@withdraw`/`@commit` accept a uid or a label — so a held dating cannot be written in surface text, and a store that holds one has it written back as `b3:…`, which claims to be a uid. The round trip is byte-exact, so this is legibility rather than data: a person reading the document goes looking for a unit. Accepting `d3:` on input is small; emitting it needs the writer to know which 32 bytes are a dating, which `WriteContext` cannot tell without the store. **Narrowed in TX-P3 step 4:** `smysl commit <did>` accepts the spelling, so a lock can now be written — into a CBOR store, because `persist` still correctly refuses a commitment surface text cannot read back. What remains open is the *document*, which is the grammar question and therefore the amendment. |
| OQ-72 | 2.4 (found in TX-P3 step 2): **nothing dates a manifest.** A-5 gives dating target kind 2 the meaning "every unit drawn under one manifest", so a mid in a target scopes to units and the catalog entry's own `published` can be corrected only by a new manifest version. The engine follows the wording. If the intent was to date the manifest, A-5's kind 2 needs a sentence — and the two readings are not interchangeable: constraining both would let a dating meant for the units contradict the catalog entry's seed and report an inconsistency nobody meant. |
| OQ-74 | 2.4 (found in TX-P5 step 5, measuring GE-T2's corpus): **a configured temperature of 0 is not a reproducible provider.** `Passes::admits` refuses two passes above temperature 0 on MS-8's measurement, but it checks what a caller configured and nothing checks whether the provider honours it. Hosted `deepseek-chat` with `"temperature": 0.0` on the wire gave `identical-span` rates spread 0.333–0.637 *within* a single language over three runs — the same size as the spread *between* languages — so the second pass behaved much as MS-8 says a T>0 run behaves, at a configured T of 0. S0 had the split from the other side: `J_class` 1.000 local against 0.327 hosted, and MS-8's bit-identical runs were the local arm. The consensus policy's premise is a property of the provider rather than of the configuration, and this build cannot distinguish them. Closing it is cheap — the same request twice at T=0 before a run, recorded beside the figure — or §3.4 says `identical-span` is defined for providers reproducible at T=0 and is a lower bound elsewhere. |
| OQ-75–OQ-83 | 2.9 (OQ-75–OQ-82 raised and decided in TX-P6 step 1, 2026-10-15; **OQ-83** raised and decided in step 3, 2026-10-16 — a bare `find` with no engine is `~lex`, because the CLI of the same name already defaults to lexical and because it is the only default that keeps the commonest query in the language pure; §9 carries each with its rejected alternatives). Two of them asked a question of SMYSL-2.5 rather than of the grammar and were settled there the same day: **OQ-77**, whether qualified field access is lensed at all (2.5 §3.1 and §3.9 disagreed; §3.9's "reads revision 1 raw, no lens" was stale, because raw at an old revision needs the backward lensing §3.9 itself forbids), and **OQ-82**, whether saving a query requires a successful plan (2.5 §3.6: it requires a successful *run*, which three of its payload keys and its result edges already implied) |
| OQ-84–OQ-85 | 2.5 (raised by TX-P8 step 1, 2026-10-17: the per-partition limit as a ninth recipe input the payload cannot carry until FC-5 gains a key; saved queries ranking for their own terms. **Both decided 2026-10-28 as recommended:** `x.query/v1` gains `query:per-partition`, an `int`, at revision 1 since A-11 is still for discussion, and a rerun runs under the recorded limit unless the caller names one; a pipeline whose source is `find` leaves `x.query/saved` units out of scope unless a `schema` predicate names them) |
| OQ-86–OQ-88 | 2.5 (raised by TX-P8 steps 3 and 4, 2026-10-17: sq/1 cannot ask for only what a walk reached; a walk reaching the rebuttal of a superseded account; a slot binding the subject and nothing of what was asked of it — the last two are what fired GE-T10's kill criterion on counter-evidence. **OQ-87 and OQ-88 decided 2026-10-28** as template set `smysl/dossier/2`: seeds superseded before the walk, `counter` and `reported` narrowed to the question's topic, `revision` withdrawn from `trace`; counter-evidence precision 1.000 over set 1's questions and over seventeen fresh ones, the criterion met, `ask` kept experimental. **OQ-86 decided the same day** as SMYSL-2.9's `expand … only`, and OQ-87's part (a) by the walk into history (`expand <-supersedes only`), both taken by template set `smysl/dossier/3`: counter-evidence 1.000 / 0.917 over set 1's questions and 1.000 / 0.706 over the fresh ones) |
| ~~OQ-89~~ | 2.6 (raised by TX-P8 M2, 2026-10-18; **decided 2026-10-28**: a grouped jackknife for T1 and Heaps' β, measure version 2 — the percentile interval excluded its estimate in all 60 cases over five Bibles up to their whole length, and the jackknife covered β in 90–97.5% of simulated seeds; T1's interval is not claimed to cover the source's entropy on a short text) |
| OQ-90 | 2.8 (raised and decided at TX-P13c step 2, 2026-10-26, from GE-T7's stand-in: one redb unit store per library, a set of tables per shard, as step 13c.3 before 13d — a store per shard paid its fixed cost a thousand times) |
| OQ-91 | 2.5 (raised at TX-P13c step 2 and decided 2026-10-26 as a step of its own: `find` builds its postings per query and per analyzer over the whole store, and should read the library's; **done 2026-10-26** — each unit cut by its own partition's analyzer, p95 under 1 s at every scope on GE-T7's stand-in) |

### 6.3 Format registry

2.3 Appendix A is authoritative: records, keys, identities, open enumerations, new codes,
estimator ids and rules.

---

## 7. Differences from draft 3 (errata and refinements)

Found while writing the set. **The set is authoritative where it differs.**

| # | draft 3 said | the set says | found by |
|---|---|---|---|
| E-1 | §2.2 scale probe: "surface → CBOR" times and "store" sizes | **`fmt --format cbor` writes surface text** (the flag is ignored, H-17), so those figures measured surface parsing and surface files. Re-measured in 2.8 §2: about 2.6 KB per unit resident, every unit core held twice, and loading about 90% of `check`'s time. | 2.8, 2.1 |
| E-2 | export views `v/export/<key>` | that is not a legal view id (surface parsing silently turns it into `v/doc`; CBOR rejects it). Use `export/<slug>`. | 2.7 |
| E-3 | GE-T15 uses fixtures F1–F13 in `fixtures/corpus` | there is no F10 there. It is `fixtures/wire/F10-lifecycle.cbor`. | 2.7 |
| E-4 | open enumerations as `Other(u8)` | a unit variant `Unknown = 255`. `Other(u8)` breaks downstream `as` casts, which the project treats as a 2.0. | 2.1 |
| E-5 | an `Absent` quote caps the unit at `inferred` | `inferred` needs grounds. Without them, the cap is `speculative` (`W441`). | 2.4 |
| E-6 | saved-query results linked by kernel `answers` | `x.query/answers`: kernel edges feed salience | 2.5 → 2.3 |
| E-7 | time contentions materialised as contentions | derived and reported, not recorded (2.3 A-8.2) | 2.4 → 2.3 |
| E-8 | "no `cc`" for the pure-Rust stack | `blake3`'s `pure` feature still lists `cc` as a build dependency *(whether it compiles C is unverified)*. Today the `remote` feature already needs a C compiler and `semantic` a C++ compiler. The claim is narrowed to "no new C/C++ in the default build", with a CI check. | 2.5, 2.2 |
| E-9 | `bundle` loses commitments, schema declarations and unknown records (F-16) | it also loses attestations on relations, and units reachable only as notes, reasons or contention positions (H-14, H-15) | 2.7 → 2.1 |
| E-10 | `--seed-check` asserts reproducibility | it is accepted but read by nothing. `find` and `pack` are labelled pure even with model-dependent engines (H-19, H-20). | 2.5, 2.6 → 2.1 |
| E-11 | ingest staging reused for imports | ingest staging rewrites units and loses records, so import staging is CBOR-only | 2.7 |
| E-12 | `strict` classes informally specified | a seed-and-grow greedy partition in uid order (2.3 A-12.4) | 2.4, 2.3 |
| E-13 | the toolchain was not considered | ~~MSRV: redb 4.x needs 1.90, quick-xml 0.42 needs 1.86, unicode-segmentation 1.13 needs 1.85. The workspace declares 1.79 and does not test it.~~ **Closed 1.10.0**, and the gap was larger than the omission suggested: the declared 1.79 was false for every crate already, and the named three were not why. Measured floors declared per crate, gated by `make msrv` and an MSRV CI job (OQ-40 = OQ-66). | 2.4, 2.8 |
| E-14 | `derive_thread` scales | it is quadratic: 24.5 s at 171k units, 0.55 s after a one-arm fix with byte-identical output (H-18) | 2.8 → 2.1 |

**Defects in today's tree** that the set documents and 2.1 fixes:
- F-2, F-3, F-4, F-6, F-12, F-13, F-14, F-16, F-17 and F-18
- H-1 to H-20, including the six operational issues the spike found:
  - `-C/--config` is ignored
  - the model is not in the recipe
  - Ollama's `num_ctx` is not sent
  - repaired failures leave no trace
  - there is no temperature control
  - `--yes` does not commit

**2.1 is worth shipping on its own merit.**

---

## 8. Size and effort

These figures are rough, extrapolated from the current tree: about 69k lines of Rust including
tests. Releases 0.6 (31 July) to 1.8 (22 September) were built at roughly 35k lines a month.

| RFC | new and changed code, incl. tests | notes |
|---|---:|---|
| 2.1 | 4–6k | about 30 fixes, most of them small, with tests |
| 2.2 | throwaway scripts | about 7 days, about 6 hours per annotator, 1.3–1.9M tokens (2.2 §6) |
| 2.4 | 18–25k | the largest: readers, time engine, ingest, propositions |
| 2.5 | 12–18k | parser, planner, lenses, dossiers, embeddings |
| 2.6 | 7–10k | estimators, graph measures, assessment |
| 2.7 | 5–8k | selection, layouts, index, verify, matrix tests |
| 2.8 | 6–9k | trait, consumer ports, redb store, union |
| **total** | **≈ 52–76k** | roughly doubles the codebase |

**Calendar:** at the historical pace, 1.5–2.5 months of coding. The binding constraint is the
experiments, not the coding:
- gold sets and annotators (GE-T9, the spike's same-as judging)
- multi-model extraction runs (GE-T2)
- the 1,000-work scale test (GE-T7)

Coding speed cannot compress those.

---

## 9. Next actions

1. ~~**Finish TX-P0**~~ **Done, in full**, as of 2026-10-07: F-2 and `bundle --unknown
   keep|drop` were the last two. 2.1 §10 records every departure from the plan. **1.9.0 is
   releasable**: G−1 is satisfied and the phase's exit conditions are met.
2. ~~**Run S0**~~ **Done 2026-10-07.** The full protocol ran — 365 runs, 29 inputs, five
   languages, two models — and its decision table returns *continue with a stated pivot* (§1.1).
   `S0_RESULTS/` holds the report, the table and the findings log.
3. ~~**Owner review of 2.3 Appendix E and §3 of this document.**~~ **Done 2026-10-07.** The nine
   amendments G0 needs are accepted: A-1, A-2, A-3, A-5, A-6, the `admission` opening in A-8.1,
   A-11 `x.text/v1`, A-12 rules E and Z, A-13. 2.3's header records what that carries and what it
   leaves open.
4. ~~**G0**~~ **Passed 2026-10-07.** Both halves met.
5. ~~**Do F-2 first.**~~ **Done 2026-10-07.** OQ-31 is answered — faithful for cost,
   content-fair for a bound — and F-2 is built with `smysl/content/1`: Russian gains 51% of its
   gist budget, English moves 2%, and draft 3 §22's en/ru gap closes from 68.5% to 3.2%
   relative. TX-P0 is now complete except `bundle --unknown keep|drop`. 2.1 §4.3.2 and §10.
6. ~~**Re-plan 2.4 against §1.1.**~~ **done 2026-10-08.** 2.4 is draft 2, with the pivot landed in
   §0.1 and in §§3.4, 3.6, 3.7, 5.5 and 6. It was a re-plan rather than an edit for more than the
   two exit tests: a step moved between phases, and **two of the six items did not survive contact
   with the evidence in the form §1.1 states them**.

   *Item 2 is circular as written.* "Extract twice per window and keep what both produce" needs a
   relation that says two units are the same proposition. At uid level there is none — S0 found
   zero shared uids between the models anywhere, and the hosted model does not reproduce itself at
   T=0. At class level it is same-as, which items 3 and 5 have just declared unreliable at α 0.600.
   So TX-P5's consensus gate is stated on `identical-span` alone, which is computed rather than
   judged; the broader claim waits for TX-P7 and GE-T9. Both passes are staged and nothing is
   deduplicated during ingest, which doubles the unit count — a cost item 6 then has to carry.

   *Item 6 names the wrong remedy.* FC-6's weights are not what is miscalibrated: S0 says in as
   many words that the 73% hosted gap is "the repair loop itself, not tokenization", and against
   the model that never enters that loop the estimator is 6% over. Calibrating weights on pre-F-2
   Russian would have fitted the estimator to a defect 1.10.0 removed and then measured it as
   correct. So no weight changes; the cost model gains an explicit per-language retry term read
   from the journal, and the non-English figures are re-measured after F-2 before TX-P5 is sized.

   Two consequences worth naming. **TX-P7 step 1 moves to TX-P2 step 5** — `proposition::classes`
   is pure, needs only records, and TX-P5's consensus output cannot be read before it exists; this
   is the smallest form of the reordering S0's report §6 asks for and §1.1's item 2 omits, and T4,
   the row that would have mandated a full reorder, did not fire. And **GE-T5's 0.9 bar could not
   have been passed**: a gold whose own binary α is 0.600 cannot support a 0.9 precision bar, so
   the pilot's 0.835 and 0.712 measure the gold, not the engines. The restatement is not a
   loosening. 2.4 §0.1.
7. **Answer the remaining TX-P1 blockers** in §4. ~~MSRV first: OQ-40 = OQ-66~~ **done
   2026-10-07.** It was the one with a dependency outside this repository, and measuring it
   turned the question inside out: there was no 1.79 build to preserve, and redb was not the
   reason. The floors are 1.85 for the nine pure-path crates, 1.86 for `smysl-provider` and
   `smysl-ingest`, 1.88 for `smysl-tui` and the facade; each is declared on its own crate,
   `make msrv` fails in both directions, and a CI job compiles the base tier at 1.85.
   ~~OQ-36, OQ-37, OQ-39~~ **done 2026-10-07**, and each turned out to be a question about
   something already written rather than about the thing being planned. The lock: `create_new`,
   because `File::lock` is unstable at 1.88 — the MSRV work above priced the parenthesis — and
   because a kernel lock cannot name its holder and vanishes on the crash it was the only record
   of. JSON: `serde_json` behind `reader-json`, because a strict mode of the HJSON parser is new
   code and not a narrowing (it refuses a surrogate pair, turns an id past i64 into a float, and
   misparses malformed JSON silently) — and because the purity gate's one list was carrying two
   claims and enforcing only the weaker, which TX-P1's feature-per-reader crate would have
   widened into a hole. Erasure: refuse 15/18 in a log, because the rewrite that honours a
   redaction resets the same hash chain that would have shown an edit, and nothing yet depends on
   the bytes — the record enum stops at 13. **§4's TX-P1 column is now empty**, so the planning
   blockers for TX-P1 were step 6 alone — and that is done too (step 6 above). 2.4 §9.
8. **Decide whether to buy the human same-as measurement.** Every provisional row (T1, T3, T6,
   T7, T8) rests on model coders, and no model coder reaches α 0.667 — the local 14B model agrees
   with each hosted model better than the two hosted models agree with each other, and a single
   coder does not agree with itself at the floor. GE-T9's ceiling cannot be established without
   people, and TX-P7's exit test depends on it.
9. **Fold the accepted amendments into the spec** as their phases land, extending
   `scripts/verify-spec-tables.py` for each (A-14). A fold is not complete until that gate covers
   its tables.

**Where the work is:** **TX-P1, TX-P2 and TX-P3 are complete** (2.4 §6 records each step and
its departures) and **TX-P4 is under way**. TX-P3's step 1 (`smysl-core`'s EDTF,
`source.published`, record 17), step 2 (the time engine, rule E, the copy and omission rules),
step 3 (the `Time` check pass), step 4 (`date set/order/show --why`, and §19.2's scenario) and
step 5 (**GE-T13**) are all done, with `P-E1`–`P-E8` green and the diagnostic registry at 78.
Step 4 found four defects in
what was already shipped and none in the command it added, which is recorded in 2.4 §6 and
worth the summary here: a dating could not be written to a surface document at all, an
unevidenced offset was moving a `measured` instant, a held dating named no contention to
resolve and `resolve` refused every derived id, and a `canonical` commitment could not name a
dating by did. Step 5 found nothing, which is itself informative: the four steps before it
were each the first caller of what the step before had built, and every defect was at that
seam. GE-T13 drives the same `effective` two callers already drive, over different data.

**TX-P4 step 1** is the substrate index: §3.8's tables, a `rebuild` and an `on_append` for each
of the seven this build fills, and a deterministic dump so the two can be compared byte for
byte. Three findings are worth the summary. The exit's own gate — rebuild ≡ incremental over a
grown corpus — is *necessary and not sufficient*, which was measured by breaking each of the
five order-dependent table shapes in turn: it caught two, and the other three each need a
corpus chosen to produce the disagreement. `on_append(&[Record])` as §3.8 writes it cannot
build three of the tables at all, because they are built from records 15 and 18 and OQ-39's
answer is that a log never holds one. And `index.redb` is **owed rather than built**: `redb`
4.3 needs Rust 1.90, Cargo gives `rust-version` per package and not per feature, and
`smysl-text` is a base-tier crate at 1.85 that `smysl-check` and the facade depend on — so the
feature as specified would move the floor for every build of the substrate to buy a backing
nobody had turned on. The tables, the builders and the gate are what the step owed; a sidecar
checked against the catalog's length and hash carries them, and costs one rebuild rather than
one per open.

**TX-P4 step 2** is the postings table and the two seams that make it scorable, and it ends with
the exit clause half met and the other half attributed. The two scoring paths agree **bit for
bit** — uid, `f32` score and per-term decomposition, over twelve query shapes — which is the
strongest reading of "results identical to the in-memory path" and the only one worth asserting:
a scorer that agreed about the ranking and disagreed in the fourth decimal would still have
changed every `--why` line `doc-output` compares. On the timing, a query over the table is
**sixty times** faster at 155k units and inside the 200 ms the clause names; the command is four
times faster and outside it, because what remains is parsing the catalog — a floor SMYSL-2.8
already named as M-7 and assigned to TX-P13. The clause also names a corpus that is a *store*,
and a partition is a manifest and a language, so there was no honest way to time the thing it
asked for; what was timed instead is a library of the same size and the two quantities the clause
is really comparing.

The step found one defect in the step before it, and the finding is methodological as much as
technical: step 1's `journal` rebuild inserted each row in order into a `Vec`, which is quadratic
in bytes moved — **4.53 s** of a 4.96 s open at 155k units, in the one table step 1 had filed
under "arithmetic over a log the open has already parsed". It is 58 ms sorted once. A fixture of
nine lines cannot tell a cost class from a cost claim, and step 1's was a claim.

**TX-P4 steps 3 and 4** finish the phase, and between them they found the thing worth taking out
of it. Step 3's three range tables answer a window by descending a max-`hi` tree over rows in
`lo` order, and the clause — "answers without sorting the store" — is asserted as a **count of
rows read** rather than as a time, because a timing says the machine was fast where a count says
the answer did not touch the corpus: 24 rows read for 24 answers out of 155,510. Step 4's `align`
table maps a locator of one expression to a locator of another through a map *installed in the
library*, which is what keeps a derived table derivable; the KJV ↔ Synodal Ps 14 ↔ Ps 13 clause is
met, the numbering being a published convention while the verse text is synthetic and said to be.
OQ-4 — the licence of *shipped* maps — turned out not to block the step at all.

What the phase will be remembered for is the measuring. Three defects were found by building a
corpus of the size the phase is sized for, and not one of them was visible on any fixture in the
repository: step 1's `journal` rebuild was quadratic (4.53 s of a 4.96 s open), the time engine's
`seed` scanned the whole store per subject (64.4 s for one axis, against 51 ms after a one-line
fix), and the first postings query loop allocated a map per candidate (270 ms p95 against a
clause asking for 200). Two of the three were in code that had already shipped a step with its
tests passing. A cost class asserted in a doc comment is a claim; the measurement is the thing.

TX-P3 added one corpus item to the standing debt, and **1.10.0 paid it**: a public chronology
with known relative orders, GE-T13's fourth clause, is `fixtures/chronology/public.tsv` — 45
pairs of public dates, every line carrying the events it names so the order is checkable without
this repository, and `SMYSL_CHRONOLOGY=<file>` still replaces it with somebody else's. The reason
it is not a development set is that it was written to be awkward rather than to pass: 43 pairs
are decidable and 39 of those claim `disjoint`, the stronger relation a widened extent fails,
and writing it found two things before it passed — spring and autumn of 1918 are *not* ordered
here (OQ-71(b) working as decided: a season is its year), and a `Y`-year of 13.8 billion is
refused, this build bounding that year to 32 bits. **GE-T1's five Bibles are also paid**: see its
row in 2.4 §5.5, and **GE-T2's corpus is paid** as of 2026-10-15: `fixtures/stability/corpus/`,
five languages of the S0 spike's own passages, with its rerun measured over three runs. What
remains owed is TX-P2 step 1's 500-sentence boundary gold set per tier-1 language, and GE-T1's
JSON series. Neither blocks TX-P3.

What the re-plan leaves downstream rather than blocking: TX-P5 is sized from a measurement taken
after F-2 rather than from a planning figure, and GE-T5's absolute bar waits on step 8's human α.
TX-P13a remains independent of all of it — the `StoreRead` trait and its consumers
ported in memory (2.8 §§3–4), which §2's note marks as needing only TX-P0.
