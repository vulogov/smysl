# RFC SMYSL-10 — GE-M7: does smysl make agent RCA better?

**Author:** Vladimir Ulogov
**Status:** draft, R1 (2026-11-25): read against the code; §12 lists what R1 found and how §10's
questions are settled. Nothing built or run.
**Target compatibility:** smysl 1.x, strictly additive; format `smysl/1.0`, unchanged. The
harness is an example and a test crate; no public API is planned to change (§8, Q-6).
**Crates:** none changed by plan. New: `examples/ge_m7` (the harness), fixtures under
`fixtures/ge_m7/`.
**Amends:** RFC SMYSL-3 §11, §25 (TL-P6's exit criterion) and §26 (GE-M7's design), D-T12 and
D-T13. Results land as amendments there; this RFC is the plan and the pre-registered gates.
**License:** MPL-2.0.
**Evidence baseline:** `dev/1.22.0` at `9084112`. Code pointers below were read there in R1.

---

## 1. Summary

GE-M7 is the experiment RFC SMYSL-3 calls "the reason for all of this" (§26, Order): whether an
agent doing root-cause analysis does it better when it writes into smysl. §11's investigation
design (consulted, hypotheses with grounds, rule TQ, kind 6) was built in TL-P6 without it, and
every release since 1.17 has listed it as owed. It has not run for two reasons, both still true:

- **The testbed.** D-T12 chose a live Train-Ticket deployment with scripted deploys and config
  edits. Train Ticket is 64 services. It has not been deployed on the one machine available
  (24 GB), and no cluster exists.
- **The agent.** There is no agent loop in the tree. `smysl-provider` does one completion per
  request, with structured output (`crates/smysl-provider/src/lib.rs`, `Request`,
  `StructuredMode`), and nothing drives tools.

This RFC splits GE-M7 so that the part that decides §11 can run without the testbed:

| Stage | Data | Changes in the corpus | Needs | Decides |
|---|---|---|---|---|
| **M7-R** (replay) | RCAEval RE2, all three systems: 270 labelled faults with metrics, logs and traces (CC BY 4.0) | none: faults only | this machine, local models (Ollama), one hosted model for one arm | §26's stop condition: does arm (b) reduce fabricated evidence relative to (a)? Top-k, unused evidence |
| **M7-T** (testbed) | Train-Ticket, deployed, with scripted deploys and config edits, faults injected after some | deploys, config edits | a cluster (not here), approval of its cost | whether change units help an agent: the part of GE-M7 RCAEval cannot test, since public RCA corpora carry almost no change events (§26, Data) |

M7-R answers the question §26's stop condition asks. M7-T answers the one D-T12 was chosen for.
M7-T is planned here only far enough to state its gate and what it needs; it is gated on a
decision this RFC does not make (§7).

## 2. Invariants

- **One agent, four arms.** Every arm that has an agent uses the same model, prompt skeleton,
  tool set, step budget and stopping rule. Arms differ only in what §4 says they differ in.
- **One answer shape.** Every arm ends with the same structured answer (§4.3), so one oracle
  scores all arms. Fabrication is never measured by smysl's own rule TQ, which is what is under
  test.
- **No labels in reach.** The agent never sees the root-cause service, the fault type or a path
  that names them (§5.2).
- **Pre-registered.** §6's gates, the case sample and the prompts are fixed and committed before
  the first scored run. The pilot (M7-P2) may change prompts and budgets; nothing after it may.
- **Reported as measured.** A gate missed is reported as missed, as SMYSL-9's Part A was.
- **Local first (D-T13).** Every arm runs on a local model. The hosted model runs as one arm pair,
  labelled and reported apart.

## 3. What §11 claims, and what would show it wrong

§11 makes three claims an experiment can test:

1. **Rule TQ catches fabricated numbers** (§11.4). A hypothesis that quotes a number its cited
   window does not hold is capped at `speculative` and named by `SMY-W606`. `oko_ledger` shows
   the mechanism on planted quotes (605 written, 605 raised); no model's quotes have been tried.
2. **Writing into the ledger changes what the agent does.** If the agent sees `check`'s findings
   while it works, it can retract or fix a quote, so fewer fabrications reach its final answer.
   This is the claim §26's stop condition is about.
3. **Consulted makes two failure modes numbers** (§11.2): unused evidence (consulted, never
   cited) and unexplored evidence (in scope, never consulted).

Claim 1 can hold while claim 2 fails: TQ could label every fabrication and the agent could
ignore the labels. §26's stop condition is claim 2. If it fails, §11 is redesigned (§26: "before
TL-P7"; TL-P7 has since shipped, so the redesign would now be an amendment).

## 4. The arms (M7-R)

### 4.1 Arms

| Arm | Agent | Writes into smysl | Sees `check` findings | BARO ranking given |
|---|---|---|---|---|
| **(a)** agent alone | yes | no: tools return text; nothing is recorded | no | no |
| **(b)** agent with smysl | yes | yes: each tool result is a unit (a window, a template set), consulted is written per tool call, each hypothesis is a unit with grounds, `explains` and `quotes` | yes, after each hypothesis it writes: its effective status and any `SMY-W606` | no |
| **(c)** ranking alone | no | the ranking's `derived` hypotheses | — | is the answer |
| **(d)** (b) + (c) | yes | as (b) | as (b) | yes: the ranking's hypotheses are in the store at the start, and listed in the prompt by uid |

- Arm (c) is `smysl_tel::detect::baro::run` (`crates/smysl-tel/src/detect/baro.rs`) over every
  metric the tools expose (`simple_metrics.csv`), with `at_ms` the case's alert instant (§5.3)
  and `pre_ms` and `post_ms` set to six minutes, GE-I3's window length. `Params::new` defaults to
  an hour and ten minutes, so both are set explicitly (R1-2). `top` is the number of series, not
  the default five: five series can be two services. Its answer is the first five distinct
  services in rank order. It makes no numeric claims, so it has no fabrication rate.
- (b) differs from (a) by the ledger **and** the feedback. Separating the two is Q-2.

### 4.2 Tools

The same tools for arms (a), (b) and (d). Each tool reads the case's files through smysl's
readers (`prom/1`-style CSV through the RCAEval loaders the GE-I3 and GE-M8 examples already
use), so the numbers an agent sees in (a) are the numbers (b) records.

| Tool | Returns | In (b) and (d) also |
|---|---|---|
| `services()` | the case's services, anonymised as listed (§5.2) | — |
| `series(service)` | that service's metric names | — |
| `window(series, from, to)` | two summaries of one window: the exact one (`count`, `min`, `max`, `sum`, `mean`, `first`, `last`) and the sketched one (`count`, `q` at `qs` = 0.5, 0.9, 0.99, `alpha` 0.01), each with an id | materialises both window units (§9.3: `evidence::materialise` and `materialise_sketched`), writes consulted for each, returns their uids as the ids |
| `compare(series, at)` | `window` over `[at−6m, at)` and over `[at, at+6m)` | as `window`, for both |
| `logs(service, from, to)` | Drain templates with counts, top 20 by count, each with an id | the `x.tel/log-template` units (§9.7, `logs::materialise`), consulted |
| `traces(service, from, to)` | per operation, top 10 by span count: span count and the sketched duration window | the operation's duration series' sketched window (R1-3), consulted |
| `hypothesise(…)` | in (a): an id. In (b)/(d): the unit's uid, effective status, findings (`SMY-W606`, and a kind 6 contention, `SMY-W053`, when one names this hypothesis) | the hypothesis unit, `explains`, `during`, attested as `model:<provider>/<name>-<digest>` (an `AgentId`, as `oko_ledger` writes it) |
| `answer(…)` | ends the run (§4.3) | — |

A tool result is capped in size (Q-4). Time is in seconds from the case start, never wall-clock,
so the agent cannot read the injection instant from a timestamp. Every number a tool prints is
the unit's own canonical `dec` spelling (§5.3 of SMYSL-3), in every arm, so a value copied
faithfully is a value rule TQ verifies (R1-6). In (a) the ids are the same uids, printed and
never recorded, so the arms see identical text.

### 4.3 The answer

```json
{ "ranking": ["svc-07", "svc-02", "svc-11"],            // up to five services, best first
  "claims": [ { "text": "p99 latency of svc-07 rose to 2.31 s after t",
                "id": "b3:…", "series": "svc-07/latency", "from": 720, "to": 1080,
                "field": "q[2]", "value": "2.31" } ] }
```

Every numeric claim in an answer is a structured claim, in the same shape in every arm. `id` is
the id a tool returned for the window or template the claim reads; `field` is the unit's field
without its `tel:` prefix, as rule TQ spells it (`max`, `q[2]`). In (b) and (d) the claim is
also the quote `<id>#<field>=<value>` on the hypothesis it came from, written **by the agent**:
the harness does not look the id up from `series`, `from` and `to`, because a quote the harness
builds from the right window verifies by construction and TQ would measure nothing (R1-5). In
(a) it is only the claim. Prose outside `claims` is kept in the transcript and not scored.

### 4.4 Models

- **Local:** `qwen2.5:14b-instruct` through the Ollama provider, temperature 0, `JsonSchema`
  structured output (enforced by Ollama's `format`), `context_window` 32768. `Request` has no
  seed and the Ollama mapping sends none (R1-4); temperature 0 is greedy, and G-5 measures what
  nondeterminism is left. A step whose `payload_len` exceeds the window ends the run as a
  protocol failure: Ollama would truncate it silently.
  `llama3.1` (8B) as a second local model if the pilot shows 14B can follow the tool protocol
  and 8B can too (Q-3).
- **Hosted (D-T13):** one model through the Anthropic provider, the one verified against the
  live endpoint, on arms (a) and (b) only: the pair the stop condition compares. Labelled in
  every table.

## 5. The corpus (M7-R)

### 5.1 Cases

RCAEval RE2: three systems (Online Boutique, Sock Shop, Train Ticket), six fault types (`cpu`,
`mem`, `disk`, `delay`, `loss`, `socket`), five services each, three repetitions: 270 cases, the
same download SMYSL-9 used (deleted after it; needed again, about 32 GB extracted).

**The scored sample** is 90 cases: per system, each fault type on each of its five services,
first repetition only (3 × 6 × 5). The other 180 cases are the pilot's pool and the repeat set
(§6, G-5). The sample is listed by case path in `fixtures/ge_m7/sample.txt` and committed before
M7-P3.

### 5.2 Leakage

RCAEval's case directories are named by root-cause service and fault (`<service>_<fault>/<rep>`).
The harness copies nothing from a path into a prompt. Services are renamed `svc-NN` per case by a
seeded permutation, and the mapping is kept out of the agent's reach; series names keep their
metric part (`cpu`, `latency`) and lose the service part. Log lines keep their text, which can
name services: that is data the agent is meant to read, and the renaming is applied to it too
(token replacement of the case's service names **and their variants**: Train Ticket's
`ts-order-service` appears in its own data as `orderservice` and `order-service`, Sock Shop's
`carts` as `cart`; the variant list is per system, committed, and checked by the test, R1-7). A
test checks that no prompt or tool result of any fixture case contains an original service name,
a listed variant, or a fault-type word in a path.

### 5.3 The alert

Each case gives the agent one alert: "something is wrong from t", with `t` the case's
`inject_time` rounded down to the minute, as BARO's trigger and GE-I3's windows use it. The
agent gets the alert, not the fault. Whether to give a fuzzier alert (a detector's, which may be
late) is Q-5.

### 5.4 The oracle

A claim is **fabricated** when the oracle, reading the case's raw files with its own CSV reader
and not smysl's store or substrate, computes the named field over the named series and window
and the claim's value does not match it, or when the series or window does not exist, or when
`id` names a unit of a different series or window. The match:

- **exact fields** (`count`, `min`, `max`, `sum`, `mean`, `first`, `last`): the claim equals the
  oracle's value, or is that value correctly rounded to the digits the claim gives (Q-1);
- **sketched quantiles** (`q[i]`): the claim is within the sketch's `alpha` (relative) of the
  oracle's exact order statistic at that rank, DDSketch's guarantee, or a correct rounding of
  such a value. The oracle does not reimplement the sketch;
- **template counts**: equal.

The oracle is independent of rule TQ by construction: TQ compares a quote with a unit in the
store; the oracle recomputes from the files. Each claim is also scored **strictly** (canonical
text equal to the unit's), which is TQ's rule, and both rates are reported.

## 6. Gates (pre-registered)

Per case and arm: AC@k (the root-cause service among the first k of `ranking`), the fabrication
rate (fabricated claims ÷ claims), unused-evidence rate (units consulted and never cited ÷
consulted; for (a), tool results never referred to by a claim ÷ tool results, from the
transcript), and in (b) and (d) TQ's precision and recall against the oracle. Comparisons
between arms are paired by case, with a case-resampled bootstrap (10,000 draws, seed fixed),
reported as a 95 % interval.

| Gate | Statement | Fails means |
|---|---|---|
| **G-1** (§26's stop condition) | Fabrication rate by §5.4's tolerant match, (b) − (a) < 0, the interval's upper end below 0, on the local model. The tolerant match is used so that (b) is not credited for learning TQ's spelling from its feedback while (a), with none, rounds | §11 is wrong as designed: rule TQ does not change what reaches an answer. Redesign before anything else in SMYSL-3 builds on it |
| **G-2** | TQ precision ≥ 0.9 in (b): an `SMY-W606` is a real fabrication by the oracle's tolerant match. A W606 on a correct rounding counts against it | TQ raises on correct quotes (spelling, rounding), and the agent learns to ignore it |
| **G-3** | TQ recall ≥ 0.8 in (b) over fabricated claims that are quotes | Fabrications go around TQ: wrong numbers with no quote, or quotes of units not in the grounds |
| **G-4** | AC@3 of (d) ≥ the larger of (a)'s and (c)'s, interval's lower end above −0.05 | smysl's ledger and the ranking together cost accuracy |
| **G-5** | On 30 cases rerun three times, AC@3 and the fabrication rate vary by less than G-1's effect | The run is too noisy to decide G-1 at 90 cases; the sample grows to 270 before G-1 is read |

G-1 is the gate §26 names. G-2 to G-5 are what makes G-1 readable. Unused and unexplored evidence
are reported, not gated: §11.2 promised them as numbers, and this is their first real value. The
volume of consulted records per investigation is reported too, which decides OQ-T1.

On the hosted pair, G-1 and G-2 are computed and reported apart; they do not decide §11, since
D-T13 put the decision on local models.

## 7. M7-T: the testbed stage

M7-T is what D-T12 chose: Train-Ticket deployed, with scripted deploys and config edits, faults
injected after some of them, so that a change unit can be the explanation. It adds:

- the readers on the change path (`k8s/1` snapshots and the canonical diff: `snapshot.rs` and
  `diff.rs` in `smysl-tel`);
- a change tool for the agent (`changes(from, to)`), consulted like the others;
- a gate **G-6**: on cases whose fault follows a change, (b) cites the change unit in its
  top-ranked hypothesis more often than (a) names the change, interval above 0.

**Needs, not available here:** a Kubernetes cluster able to run Train Ticket's services with
load generation and fault injection, a few days of it, and the
model runs above repeated. Its size and cost are measured by deploying once (M7-P5) before any
scored run, and put to a decision. M7-R does not wait for it.

## 8. What is built

Nothing in the published API is planned to change. The harness is:

- `examples/ge_m7/`: the case loader (RCAEval RE2: `inject_time.txt`, `simple_metrics.csv` and
  `logs.csv` as the loaders in `examples/ge_m8_rcaeval` and
  `crates/smysl-tel/examples/ge_i3_rcaeval.rs` read them; the traces file, which no loader in the
  tree reads yet, R1-3), anonymisation, the tool set, the agent loop over `smysl-provider` (one
  structured completion per step: the model returns `{tool, args}` and the harness runs it; the
  transcript is `Request::messages`, user and assistant turns), the four arms, the oracle, the
  scoring.
- The substrate is in-memory (`Substrate::new(MemBackend::default())`), one per case, filled at
  load: metric columns as series, `logs.csv` rows as CloudEvents through
  `logs::record_cloudevents` (`logs::materialise` templates `journald` and `cloudevents` streams
  only), and per-operation span durations as series (R1-3). Windows are materialised as
  `examples/ge_m9` does, through the public `evidence` API.
- The ledger in (b) and (d) is written through `smysl_tel::investigate` (`open`, `hypothesis`,
  `consulted`, `during`, `explains`, `view`) and committed through the library, as `oko_ledger`
  does; findings are read with the check passes as `smysl check` reads them.
- `fixtures/ge_m7/`: two synthetic cases small enough to commit, for tests: a scripted agent
  (no model) that writes one true and one fabricated quote, so the oracle, TQ and scoring are
  checked without a model or RCAEval.
- Transcripts, stores and scores of the scored runs are kept outside the repository (they hold
  RCAEval data); the tables and the per-case score file go into the amendment.

R1 found no tool that needs an API the crates do not expose (Q-6, §12). If M7-P1 finds one, that
is an amendment to this RFC before M7-P1 ends.

## 9. Phases

| Phase | Delivers | Exit |
|---|---|---|
| **M7-P1** harness | `examples/ge_m7`: loader, anonymisation, tools, agent loop, arms, oracle, scoring; fixtures; the scripted agent. With the download (needs approval): RE2's file list per system checked, the traces file's format read, the variant lists written | The scripted agent's run on the fixtures gives the expected scores in a test; the leakage test passes on the fixtures and on every RE2 case; no model needed |
| **M7-P2** pilot | 6 cases from the pool (two per system), every arm, local model; tokens, time per step and per case, tool-protocol failures | Prompts, step budget (start: 25 steps), tool caps and the model list frozen and committed with `sample.txt`; the measured cost of M7-P3 and M7-P4 stated |
| **M7-P3** M7-R, local | 90 cases × arms (a), (b), (c), (d); the 30-case triple rerun | G-1 to G-5 read, SMYSL-3 §26 and §11 amended |
| **M7-P4** M7-R, hosted | 90 cases × arms (a), (b), hosted model | Reported apart |
| **M7-P5** M7-T, gated | One deployment of Train-Ticket, measured; then the scored runs if approved | G-6 read, D-T12 closed |

**Cost, before the pilot measures it.** Arms (a), (b) and (d) are 270 agent runs per model on the
sample, plus 270 for the triple rerun's three arms on 30 cases. At a guessed 25 steps of about
6,000 prompt tokens each, that is about 80 M prompt tokens per model. Locally that is time, not
money; the pilot states the hours. Hosted, it is 180 runs (two arms), about 27 M prompt tokens
before caching, priced at the provider's rate on the day; M7-P2 replaces this guess with
measured numbers, and M7-P4 runs only after that figure is approved.

## 10. Questions for R1

All seven are settled in §12; the text below is the draft's, kept so the answers read against it.

- **Q-1.** Rounding: is a claim of `2.3` for a true `2.31` fabricated? Rule TQ says yes (canonical
  text must be equal). The oracle above says no. If they differ, G-2 counts TQ's strictness as
  false positives. Pick one rule for both, or report both.
- **Q-2.** (b) changes two things at once: the ledger and the feedback. An arm (b′), ledger
  without feedback, separates them at a third more cost. Without it, a G-1 pass cannot say
  which helped.
- **Q-3.** Can a 14B local model drive a 7-tool protocol for 25 steps at all? If the pilot shows
  most runs failing on protocol, G-1 measures the protocol, not §11. The pilot's exit states the
  protocol-failure rate, and a rate above 20 % stops M7-P3 for a redesign of the loop.
- **Q-4.** Tool result caps change what an agent can fabricate: a summary it cannot see in full
  invites guessing. Same caps in every arm, but which?
- **Q-5.** The alert at the injection minute is generous. A detector's alert (GE-I3's calibrated
  CUSUM) is realistic and sometimes late or absent. Run both, or one?
- **Q-6.** Does writing a window unit from a tool call need anything the library does not expose
  for an example? `oko_ledger` writes windows from Zabbix trends; RCAEval windows are computed
  from samples, through the substrate (§9.1).
- **Q-7.** D-T13 says "one hosted arm". This RFC reads it as one hosted model over the arm pair
  the stop condition compares. One arm alone cannot read G-1.

## 11. Amendments

None yet. Results land in RFC SMYSL-3 §26 (GE-M7's row), §11 if G-1 fails, §25 (TL-P6's exit
criterion), D-T12 and D-T13, and in SMYSL-3 §29 for OQ-T1.

## 12. R1 (2026-11-25)

R1 read the draft against `dev/1.22.0` at `9084112`. What held: the ledger builders
(`investigate::{open, hypothesis, consulted, during, explains, view}`) are what §8 names, and
`hypothesis` takes quotes as `<uid>#<field>=<value>`; rule TQ is `rule_tq_violations` in
`smysl-core`, read by `smysl check` as `SMY-W606`, a canonical-text equality, as §11.4 of SMYSL-3
says; `Request` carries a message list, so a multi-turn loop needs no new API; the Ollama and
Anthropic mappings exist; `AgentId` accepts `model:<provider>/<name>`; `oko_ledger`'s check found
605 fabricated quotes and raised 605 `SMY-W606` (CHANGELOG, 1.17); `snapshot.rs` and `diff.rs`
are the change path M7-T needs. What did not hold, each fixed in the body:

- **R1-1. Window fields.** An exact window (`tel-window/exact/1`) holds `count`, `min`, `max`,
  `sum`, `mean`, `first`, `last`, and no quantile. Quantiles are only in a sketched window
  (`tel-window/ddsketch/1`, `derived` by rule TW, relative accuracy `alpha`), whose default ranks
  are 0.5, 0.9, 0.95, 0.99, so the draft's `q[2]` was p95, not p99. `window` now materialises
  both units and asks for `qs` 0.5, 0.9, 0.99; the oracle matches a quantile within `alpha` of the
  exact order statistic (§4.2, §5.4).
- **R1-2. BARO's parameters.** `baro::Params::new` gives an hour before and ten minutes after,
  and the top five series. GE-I3 does not run BARO (GE-M6 does); its six-minute windows are the
  ones meant. Arm (c) now sets both windows and ranks every series, and answers with five
  distinct services (§4.1).
- **R1-3. Traces.** `trace::materialise` makes the span units of one trace by id. There is no
  per-operation unit, and the tree has no reader for RCAEval's traces file. The `traces` tool now
  writes each operation's span durations as a series and windows it, so its numbers are window
  quotes like any other. Whether RE2 has a traces file for all three systems, and its columns,
  is read at M7-P1 when the data is downloaded again; a system without one has no `traces` tool,
  in every arm (§4.2, §8).
- **R1-4. No seed.** `Request` has no seed and the Ollama mapping sends none. Dropped rather than
  added: a field on a public, `non_exhaustive` struct is additive, but this RFC changes no API,
  and temperature 0 is greedy. G-5 measures the rest (§4.4).
- **R1-5. Who writes the quote.** The draft did not say. If the harness turned a claim's series
  and window into a quote, it would cite the right unit by construction and TQ would agree with
  the oracle on everything but spelling. The agent writes the id; the claim carries it (§4.3).
- **R1-6. Spelling.** TQ compares canonical text. Tools print canonical text in every arm, so
  the only spelling failures left are the agent's own (§4.2).
- **R1-7. Leakage by variant.** Exact-token renaming misses the spellings services have in their
  own logs and traces (§5.2).
- **R1-8. Context.** The Ollama mapping sends the configured `context_window` as `num_ctx`; the
  provider configuration's built-in default is 4096. A transcript longer than it is truncated by
  Ollama without an error. The loop sets 32768 and ends a step that would not fit (§4.4).

**§10's questions:**

- **Q-1, rounding.** Both rules, for different things. The oracle is tolerant: a correct rounding
  is not a fabrication, and G-1 is read on that, so that (b), which W606 teaches the canonical
  spelling, is not credited against (a), which nothing teaches. TQ stays strict, as SMYSL-3 §11.4
  says. G-2 counts a W606 on a correct rounding as a false positive, because that is what it is
  to an agent. Strict rates are reported beside the tolerant ones (§5.4, §6).
- **Q-2, (b′).** Not in M7-P3. If G-1 passes, (b′), ledger without feedback, runs on the same 90
  cases as a follow-up, pre-registered here, so a pass can say which of the two did it. If G-1
  fails, (b′) cannot rescue it and is not run.
- **Q-3, protocol.** Kept: above 20 % protocol failures in the pilot stops M7-P3. A protocol
  failure is a reply that does not parse against the step schema, an unknown tool or argument,
  a step over the context window, or the step budget spent without `answer`. The step schema
  names the tools as an enum and Ollama enforces it, so most of these should be budget and
  context failures; the pilot says.
- **Q-4, caps.** 2,000 characters per tool result, the lists as §4.2 gives them (logs 20, traces
  10), the same in every arm. The pilot reports how often a cap cut a result and may raise it
  before the freeze.
- **Q-5, the alert.** One: the injection minute. G-1 is about what reaches an answer, not about
  detection, and a late or missing alert adds variance G-5 would have to absorb. A detector's
  alert is a follow-up.
- **Q-6, API.** None needed. Exact and sketched windows are materialised through the public
  `evidence` API from an in-memory substrate, as `examples/ge_m9` does; log templates through
  `logs::materialise` once `logs.csv` rows are recorded as CloudEvents; the ledger through
  `investigate` and a commit as `oko_ledger` does.
- **Q-7, hosted.** As read: one hosted model over arms (a) and (b), reported apart.

**What R1 leaves for M7-P1:** the traces file (R1-3), the variant lists (R1-7), and the RE2 file
list per system, all of which need the download, which needs approval.
