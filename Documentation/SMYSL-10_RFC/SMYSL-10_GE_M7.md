# RFC SMYSL-10 — GE-M7: does smysl make agent RCA better?

**Author:** Vladimir Ulogov
**Status:** draft, R1 (2026-11-25): read against the code; §12 lists what R1 found and how §10's
questions are settled. M7-P1 to M7-P3 done (§11, A1 to A3); M7-P4 and M7-P5 not run.
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
| `metrics(service)` | that service's metric names (named `series` until M7-P1, A1-4) | — |
| `window(series, from, to)` | two summaries of one window: the exact one (`count`, `min`, `max`, `sum`, `mean`, `first`, `last`) and the sketched one (`count`, `q` at `qs` = 0.5, 0.9, 0.99, `alpha` 0.01), each with an id | materialises both window units (§9.3: `evidence::materialise` and `materialise_sketched`), writes consulted for each, returns their uids as the ids |
| `compare(series, at)` | `window` over `[at−6m, at)` and over `[at, at+6m)` | as `window`, for both |
| `logs(service, from, to)` | Drain templates with counts, top 20 by count, each with an id | the `x.tel/log-template` units (§9.7, `logs::materialise`), consulted |
| `traces(service, from, to)` | per operation, top 10 by span count: two per-second series, calls and mean latency in ms, each as `window` gives it; Online Boutique and Train Ticket only (A1-1) | as `window`, for both (R1-3) |
| `hypothesise(…)` | in (a): an id. In (b)/(d): the unit's uid, effective status, findings (`SMY-W606`, and a kind 6 contention, `SMY-W053`, when one names this hypothesis) | the hypothesis unit, `explains`, `during`, attested as `model:<provider>/<name>-<digest>` (an `AgentId`, as `oko_ledger` writes it) |
| `answer(…)` | ends the run (§4.3) | — |

A tool result is capped in size (Q-4). Time is in seconds from the case start, never wall-clock,
so the agent cannot read the injection instant from a timestamp. Every number a tool prints is
the unit's own canonical `dec` spelling (§5.3 of SMYSL-3), in every arm, so a value copied
faithfully is a value rule TQ verifies (R1-6). In (a) the ids are the same uids, printed and
never recorded, so the arms see identical text.

### 4.3 The answer

```json
{ "tool": "answer", "thought": "…",
  "ranking": ["svc-07", "svc-02", "svc-11"],            // up to five services, best first
  "quotes": [ { "id": "b3:…", "field": "q[2]", "value": "2.31" } ] }   // A1-7
```

Every numeric claim in an answer is a quote, in the same shape in every arm. `id` is
the id a tool returned for the window or template the claim reads; `field` is the unit's field
without its `tel:` prefix, as rule TQ spells it (`max`, `q[2]`). In (b) and (d) the claim is
also the quote `<id>#<field>=<value>` on the hypothesis it came from, written **by the agent**:
the harness does not look the id up from `series`, `from` and `to`, because a quote the harness
builds from the right window verifies by construction and TQ would measure nothing (R1-5). In
(a) it is only the claim. Prose (`thought`, a hypothesis's `claim`) is kept in the transcript and
not scored.

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
`carts` as `cart` and `CartsController`; the variants are generated by one rule for all three
systems, R1-7, A1-2). A test checks the rule on the fixtures, and `ge_m7 check-data` runs every
tool over every RE2 case and fails on a spelling of the root cause in any result.

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
  load: metric columns as series, `logs.csv` rows as one `journald` stream per container through
  `logs::record_journald` (A1-3), and per-operation calls and latency as series (R1-3). Windows are materialised as
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

Results land in RFC SMYSL-3 §26 (GE-M7's row), §11 if G-1 fails, §25 (TL-P6's exit
criterion), D-T12 and D-T13, and in SMYSL-3 §29 for OQ-T1.

### A1. M7-P1 (2026-11-25)

M7-P1 built `examples/ge_m7` and ran it against RE2, downloaded again with approval. Each item
below is fixed in the body or in the code, as it says.

- **A1-1. Files.** All 270 cases have `inject_time.txt`, `simple_metrics.csv` and `logs.csv`.
  `traces.csv` is in every Online Boutique and Train Ticket case and in no Sock Shop case, so
  Sock Shop has no traced operations: `services` says so, and `traces` returns nothing there,
  in every arm. Its columns are `time, traceID, spanID, serviceName, methodName, operationName,
  startTimeMillis, startTime, duration, statusCode, parentSpanID`; `duration` is in µs. Every
  traced service matches a metric or log service once renamed (Online Boutique's traces name
  `frontendservice` where its metrics say `frontend`). The tool reports two per-second series
  per operation, calls and mean latency in ms, windowed like any other (§4.2).
- **A1-2. Variants.** R1-7's per-system list became one rule: a name, the name without `ts-` and
  `service`, its singular or plural and `catalog` for `catalogue`, each with `service` and
  `-service`; a variant two services share is dropped. Tokens are matched whole, as a prefix
  before `-` (pod names), as a camelCase segment (`CartsController`, `setCurrency`), and, for
  six letters or more, inside a token. The first leakage run found nothing because it checked the
  same list the renaming used. A survey of the logs then found Sock Shop's `cart` (7,331 lines in
  one case), `Carts` and `CartResource`, and Online Boutique's `ListRecommendations`, all left
  as they were. The rule above came out of that survey. The check now also lists every token
  that holds a root's stem anywhere, apart from the leak test. On all 270 cases it finds 0 leaks,
  and that list is `username` (Sock Shop, 243 results), `train_types`, `trainfoods`, `foodorder`
  (Train Ticket) and one random hash. They are left: renaming `user` inside `username` would
  rewrite ordinary text.
- **A1-3. Logs as `journald`.** §8 said CloudEvents. A CloudEvents unit's message is its type and
  subject, so Drain would template that and not the log line. The rows go in as one `journald`
  stream per container, the message as the line.
- **A1-4. The step schema.** One object per tool (`anyOf`). Each names its tool as a `const`,
  starts with a required `thought` (a sentence on why this call is next), and requires its
  arguments. The first smoke run, on a flat object with every argument optional, dropped `from`
  from `window` and repeated the call to the end of the budget. Then the provider fix below. The
  tool `series` became `metrics`, so that no two tool names share a first token.
- **A1-5. The Ollama mapping reordered schemas.** `smysl-provider`'s Ollama mapping parsed the
  schema into a `serde_json::Value`, whose keys are sorted, and sent that. Ollama's grammar
  writes properties in the order given, so every caller's schema reached the model alphabetised.
  Here `thought` was first only in `services`' branch, and every reply that began with its
  reasoning could only be `services`: 25 times in a row, against thoughts that named `metrics`.
  Replaying the dumped request against Ollama directly chose `metrics`. The mapping now splices
  the schema text in as written (a fix in a published crate, no API change). `GE_M7_DUMP=DIR`
  writes each step's request for that kind of replay.
- **A1-6. Argument errors and repeats.** Q-3 counted an unknown argument as a protocol failure.
  The schema now makes a missing argument impossible to emit; a bad one (an unknown series, an
  empty window) is answered with the error and the tool's call form, and counted
  (`arg_errors`), not ended. A call identical to an earlier one, `thought` aside, is answered
  "made before" and counted the same way. A protocol failure is now a reply that does not parse,
  an unknown tool, a step over the window, or the budget spent.
- **A1-7. The answer's shape.** §4.3's `claims` became `quotes`, the same `{id, field, value}`
  objects `hypothesise` takes, with the ranking. The id is still the agent's to write (R1-5).
- **A1-8. Sample.** `fixtures/ge_m7/sample.txt` lists the 90 scored cases; `pilot.txt` lists
  M7-P2's six (repetition 2, first and last fault directory per system).
- **A1-9. Smoke.** One pilot-pool case (`RE2-SS/carts_cpu/2`), all four arms, on
  `qwen2.5:14b-instruct`: every arm answered with no protocol failure (a: 24 steps, b: 12, d: 8),
  no fabricated quote; (c) and (d) ranked the root cause first, (a) and (b) did not. One case
  says the loop works, not how well.

### A2. M7-P2, the pilot and the freeze (2026-11-26)

The pilot ran `fixtures/ge_m7/pilot.txt` (six cases, repetition 2) on every arm with
`qwen2.5:14b-instruct`, twice: once as M7-P1 left the harness, and once after the two fixes
below. The numbers are the second run's.

| Arm | AC@1 | AC@5 | Claims | Fabricated (tolerant) | Steps | Input tokens / run | s / run |
|---|---|---|---|---|---|---|---|
| (a) | 0.000 | 0.000 | 18 | 0 | 17.2 | 73,538 | 75.6 |
| (b) | 0.000 | 0.000 | 18 | 0 | 12.0 | 41,702 | 58.0 |
| (c) | 0.500 | 0.667 | — | — | — | — | 0 |
| (d) | 0.500 | 0.500 | 18 | 0 | 12.5 | 46,653 | 59.7 |

Protocol failures: 0 of 18 agent runs (Q-3's stop line is 20 %). Argument errors: 0–1 per run.
Results cut at the cap: 0–2 per run, mostly `logs` and `traces`.

- **A2-1. The prompt chose the first service.** Every example in the system prompt named
  `svc-07`, and in the first run 10 of 12 (a) and (b) runs opened with `svc-07`. Most of them
  answered it, and a (d) run read BARO's ranking as headed by `svc-07` when `svc-06` was first.
  The examples now say `svc-NN` and `svc-MM`, "any of them, not a service to start with". In the
  second run the first service opened is the first or second listed. Names are a seeded
  permutation per case, so that order carries nothing about the root cause.
- **A2-2. The oracle counted repeated lines.** A (d) quote of a template's count, 718, copied
  from the tool, was judged fabricated: the oracle's Drain over `logs.csv` counted 724. The file
  repeats lines at the same millisecond in the same container, and the substrate stores events
  by content, so the tool counts each once. The oracle now does the same, and the quote verifies.
  With it went the first run's TQ recall of 0 in (d): the one "miss" was the oracle's. `runs.jsonl`
  now records every quote the oracle rejects or finds misspelt, with the reason.
- **A2-3. The floor.** No arm fabricated a quote: 0 of 54 claims, tolerant or strict. Tools print
  each number as the unit's canonical text beside its id (R1-6), and at temperature 0 this model
  copies what it reads. G-1 compares (b)'s rate with (a)'s, and (a)'s is 0. A probe (`--digits 4`:
  tools print four significant digits; one case, (a) and (b)) did not move it. Of six claims in
  each arm, none was fabricated under the tolerant oracle. Four of (a)'s were not TQ's spelling.
  In (b) TQ flagged the four correct roundings (G-2's false positives), and after that feedback
  (b)'s answer quoted every value canonically: feedback teaches spelling, not honesty. Rounding
  adds TQ false positives without making fabrication to measure, so printing stays canonical.
  `--digits` is kept as a probe and is not used in scored runs.
- **A2-4. G-1 at the floor, read before M7-P3.** If (a)'s tolerant fabrication rate on the 90
  cases is 0, or too small for G-1's difference to be one, G-1 is reported as **not testable on
  this model**: neither a pass nor a fail, and §26's stop condition is not decided by it.
  G-2 to G-4, the unused-evidence share and AC@k are reported as for any run. M7-P4's hosted model
  is the next place G-1 can be read, at the cost M7-P2 states below, on approval.
- **A2-5. Frozen** for M7-P3, as committed with this amendment: `examples/ge_m7`'s system prompt
  and step schema; 25 steps; context 32,768; 1,024 output tokens per step; results capped at
  2,000 characters, logs 20 templates, traces 10 operations; canonical printing; one local model,
  `qwen2.5:14b-instruct`. `llama3.1` is not added: the pilot gave no reason to need a second model
  for the protocol. The scored sample is `sample.txt`. G-5's repeat set is `repeat.txt`: 10 cases
  per system from the sample, each service twice, the six faults five times each. It runs arms (a),
  (b) and (d) three times.
- **A2-6. Cost.** An agent run is about 60 s and 40,000–75,000 input tokens. M7-P3 is 270 scored
  agent runs (90 × (a), (b), (d)), about 4.5 h, and 270 for the repeat set (30 × 3 arms × 3),
  about 4.5 h. Hosted (M7-P4, (a) and (b) on 90 cases): 180 runs at about 60,000 input tokens is
  about 11 M input tokens before caching, priced on the day and run only on approval.

### A3. M7-P3, M7-R on the local model (2026-11-26)

M7-P3 ran at `c965120` as A2-5 froze it: `sample.txt` on every arm, then `repeat.txt` on (a),
(b) and (d) three times, on `qwen2.5:14b-instruct`. Every case and arm is in
`fixtures/ge_m7/scores-m7p3.tsv` (630 rows; transcripts and stores stay outside the repository,
§8). Intervals are §6's: paired by case, 10,000 case-resampled draws, seed 20261126. A rate is
the ratio of sums over a draw's cases: fabricated claims ÷ claims.

| Arm | AC@1 | AC@3 | AC@5 | Claims | Fabricated (tolerant) | TQ flags (real) | Unused | Budget failures | Steps | Input tokens / run | s / run |
|---|---|---|---|---|---|---|---|---|---|---|---|
| (a) | 0.244 | 0.300 | 0.300 | 293 | 14 (4.8 %) | — | 0.943 | 9 | 18.6 | 83,995 | 86.9 |
| (b) | 0.200 | 0.200 | 0.200 | 199 | 3 (1.5 %) | 25 (25) | 0.943 | 7 | 14.3 | 55,595 | 67.3 |
| (c) | 0.322 | 0.656 | 0.800 | — | — | — | — | 0 | — | — | 0 |
| (d) | 0.411 | 0.489 | 0.533 | 203 | 5 (2.5 %) | 24 (24) | 0.864 | 11 | 12.8 | 46,396 | 57.5 |

The strict rate equals the tolerant one in every arm: at canonical printing no claim was a
correct rounding (A2-3). A budget failure is a run that spent its 25 steps without answering,
and scores as a miss: 27 of 270 agent runs (10 %), under Q-3's 20 %.

| Gate | Measured | Read |
|---|---|---|
| **G-1** | (b) − (a): −0.033, [−0.070, −0.0005] | **Holds.** The upper end is below 0, by a hair |
| **G-2** | (b): 25 of 25 W606 are real (1.00); (d): 24 of 24 | **Holds.** Wilson 95 % lower bound 0.87 on 25 |
| **G-3** | (b): 25 of 25 fabricated quotes raised (1.00); (d): 24 of 24 | **Holds** |
| **G-4** | AC@3 (d) − max((a), (c)) = (d) − (c): −0.167, [−0.256, −0.089] | **Fails** |
| **G-5** | Range over three runs: 0 for AC@3 and the fabrication rate, every arm | **Not measured** (A4-10): the runs replayed one order, so they agreed by construction |

- **A3-1. G-1 is testable.** A2-4's floor did not occur: (a) fabricated 14 claims in 9 cases. (b)
  fabricated 3, all in one case (`RE2-TT/ts-auth-service_mem/1`). (d) − (a), not gated, is
  −0.023, [−0.066, +0.018].
- **A3-2. What TQ did.** In (b) TQ raised 25 W606 during the runs, every one a fabrication by the
  oracle, and the agent re-quoted or dropped all but three before its answer. The fabrication
  that reaches an answer is what G-1 measures, and it is the feedback, not the ledger alone, that
  removed it. There is no arm (b′), ledger without feedback (Q-2), so the split is not measured. (b) also makes a third fewer
  claims than (a), which a rate does not punish.
- **A3-3. Why G-4 fails.** The agent answers with one or two services (mean ranking length: (a)
  1.5, (b) 1.0, (d) 2.2), and BARO with five. Given BARO's ranking, (d) is the best arm at the
  first place, AC@1 (d) − (c) +0.089, [+0.022, +0.156], and (d) − (a) +0.167, [+0.033, +0.289].
  It loses BARO's tail: AC@3 −0.167, AC@5 −0.267, [−0.367, −0.178]. On Train Ticket, where BARO's
  AC@3 is 0.20, (d)'s is 0.03. That the short answer, not the ledger, is the cost is a reading
  after the run, not a gate: an arm whose answer is the agent's choice followed by the rest of
  BARO's ranking would test it, and needs its own pre-registration.
- **A3-4. The ledger and feedback cost accuracy without BARO.** Not gated: AC@3 (b) − (a) −0.100,
  [−0.178, −0.022]; AC@1 −0.044, [−0.111, +0.022]. (b) takes 23 % less time and 34 % fewer input
  tokens a run, and answers sooner (14.3 steps against 18.6).

  | System | (a) AC@1 / @3 | (b) | (c) | (d) |
  |---|---|---|---|---|
  | Online Boutique | 0.33 / 0.33 | 0.30 / 0.30 | 0.53 / 0.90 | 0.63 / 0.73 |
  | Sock Shop | 0.23 / 0.33 | 0.20 / 0.20 | 0.43 / 0.87 | 0.57 / 0.70 |
  | Train Ticket | 0.17 / 0.23 | 0.10 / 0.10 | 0.00 / 0.20 | 0.03 / 0.03 |

- **A3-5. Greedy decoding repeats itself.** The three repeat runs agree on every field but wall
  time, in all 90 runs each: (a) AC@3 0.167 and 5 of 85 claims fabricated, (b) 0.133 and 3 of 75,
  (d) 0.567 and 4 of 62. The scored run agrees with them on 88 of 90; the two that differ are the
  first case of the list, `RE2-OB/checkoutservice_cpu/1`, in (a) and (b), with one step more and
  the same rank and fabrication. At temperature 0 G-5 measures Ollama's run-to-run determinism,
  not sampling: the variance left in G-1 is which cases were drawn, which the bootstrap carries.
  A model sampled above 0 would need G-5 as designed.
- **A3-6. Unused evidence and volume.** Unused: 0.943 in (a) and (b), 0.864 in (d): the agent
  cites one unit in seven to eighteen it reads. Unexplored evidence (§11.2) was not computed by the
  harness, so it is owed. Consulted units per investigation: median 50.5 (a), 38 (b), 20 (d);
  largest 153. A relation per unit is a few hundred records an investigation at most, so OQ-T1
  keeps one relation per unit.
- **A3-7. Cost.** 11 h 12 min of wall time for both parts. The scored agent runs took 5.3 h and
  16.8 M input tokens: (a) 7.6 M, (b) 5.0 M, (d) 4.2 M.
- **A3-8. What this decides.** SMYSL-3 §26's stop condition does not fire: (b) reduces fabricated
  evidence relative to (a), so §11 stands as designed, on this model, with the margin A3-1 gives.
  G-4's failure does not touch §11: it says that an agent handed a ranking should not be scored
  on a shortened answer, and that this agent, alone, ranks below BARO (AC@3 (a) − (c) −0.356,
  [−0.478, −0.233]). M7-P4 (hosted, (a) and (b)) is the second reading of G-1 and runs only on
  approval.

### A4. M7-P3H, the held-out run: a completed answer and a forced answer (2026-11-26, frozen)

M7-P3 left two faults in how an answer is formed, not in §11: (d) answers with about two services
and drops BARO's tail (A3-3), and 27 of 270 agent runs spent the budget without answering and
scored as misses. A4 changes both, states the gates before anything runs, and runs on cases no
earlier phase has seen. It is frozen by the commit that implements it and records this text,
before the run. Drafting it found that M7-P3's repeats measured less than A3-5 says (A4-10).

- **A4-1. Cases: repetition 3.** `fixtures/ge_m7/held-out.txt` is the 90 repetition-3 cases, the
  same 5 services × 6 faults per system as `sample.txt` (all repetition 1). No case of repetition 3
  has been run: the pilot was repetition 2. A completion of M7-P3's own (d) answers was computed
  while A4 was drafted (A4-7), so the sample cannot test it.
- **A4-2. Arm (e), the completed answer.** (e) is (d)'s run, with no run of its own. Its ranking is
  (d)'s answer followed by BARO's ranking, repeats removed, first five kept. The agent is not told,
  so (d) and (e) differ only in scoring. Completion adds no claim and no quote, so (e)'s
  fabrication is (d)'s.
- **A4-3. The forced answer.** When the budget is spent with no answer, the loop makes one more
  call: the message "No tool calls left: answer now, with what you have", and a schema whose only
  branch is `answer`. Its answer is scored like any other, and the run records `forced`. A forced
  step that does not parse, or does not fit the context, is a protocol failure, as now. The note
  on each of the last three calls ("N tool calls left") is unchanged. It applies in (a), (b) and
  (d) alike.
- **A4-4. Frozen otherwise as A2-5.** Prompt, step schema, 25 steps, context, output limit, caps,
  canonical printing, `qwen2.5:14b-instruct`, temperature 0. The repeat set returns, with the
  case order shuffled per run (A4-10): `fixtures/ge_m7/held-out-repeat.txt` is `repeat.txt`'s
  30 service-and-fault pairs at repetition 3, run on (a), (b) and (d) with `--shuffle 1`, `2`
  and `3`. The scored run is `held-out.txt` in its listed order, arms (a) to (e).
- **A4-5. Before the scored run, done.** Tests: the forced step on a scripted agent (its schema is
  `answer`'s alone, its answer is scored, a forced reply that is not an answer is a budget
  failure), completion, and the shuffle. Smoke, unscored, on three M7-P3 sample cases that spent
  the budget in (a), (b) and (d) (`RE2-OB/recommendationservice_mem/1`, `RE2-SS/orders_mem/1`,
  `RE2-TT/ts-train-service_socket/1`): 9 agent runs, no protocol failure, 4 forced answers, each
  parsed and scored, none fabricated. Five of the nine did not reach the budget this time; A4-10
  says why.
- **A4-6. Gates.** Intervals as §6, seed 20261126, paired by case.

  | Gate | Statement | Fails means |
  |---|---|---|
  | **G-7** (G-4 on a completed answer) | AC@3 (e) − max((a), (c)), lower end above −0.05 | The ranking and the ledger cost accuracy even when the agent cannot drop BARO's tail: G-4's failure is not only the short answer |
  | **G-8** (the budget) | Protocol failures of every kind ≤ 2 % of agent runs (at most 5 of 270) | The forced answer does not answer: the loop needs redesign before another scored run |
  | **G-9** (G-1, replicated) | G-1's statement, (b) − (a) < 0 with the upper end below 0, on repetition 3, forced answers included | M7-P3's margin is not replicated. §26's stop condition is reported as undecided on this model, and M7-P4 is the deciding reading, on approval |

  | **G-10** (G-5, with the order shuffled) | On `held-out-repeat.txt`'s three shuffled runs, the range of AC@3 and of the fabrication rate, per arm, is below G-9's measured difference | Which cases ran before a case moves G-1 as much as the arms do: G-9 is reported as undecided whatever its interval, and a larger sample is proposed, on approval |

  Reported, not gated: (d)'s AC@k as frozen; AC@1 (e) − (c); G-1's difference without the forced
  answers, which is comparable with A3; the number of forced answers per arm and their
  fabrication rate; G-2 and G-3, with their thresholds; unused evidence; tokens and time.
- **A4-7. What M7-P3 already says, and why it is not the test.** Completed, M7-P3's (d) answers
  give AC@1 0.411, AC@3 0.656 and AC@5 0.800; AC@3 (e) − (c) is 0.000, [−0.033, +0.033], within
  G-7's margin. Completion keeps (d)'s first place and gains back BARO's tail, but it was computed
  after M7-P3's data was seen, so it is exploratory. G-7 reads it on repetition 3.
- **A4-8. What it decides.** G-7 holds: the answer an agent returns with a derived ranking is
  completed from that ranking, and SMYSL-3 §26 records G-4's failure as the short answer. G-7
  fails: (d)'s cost is reported as the ledger's, against the ranking, and is investigated before
  M7-T. G-9 holds: G-1 has held on two disjoint case sets on this model. G-9 fails: as its row
  says. G-8 decides only the loop.
- **A4-9. Cost.** 270 scored agent runs (90 × (a), (b), (d)) and 270 repeat runs (30 × 3 arms ×
  3 orders): about 11 h local. Repetition 3 needs RCAEval's data on disk until the run is read.
- **A4-10. A run depends on what ran before it.** In the smoke, `RE2-SS/orders_mem/1` (b) took 8
  steps, where M7-P3 spent the budget. M7-P3's binary (`c965120`), run on that case alone, gave
  M7-P3's transcript exactly once, and 8 steps the two times it ran after other requests; the new
  binary gave 2 claims and, once, 3. Both binaries send byte-identical requests (system prompt,
  schema, messages, by `GE_M7_DUMP`). The reply to an identical request at temperature 0 depends
  on the model server's state from earlier requests, most likely its reuse of a cached prompt
  prefix, which evaluates the prompt in different batches. M7-P3's three repeats replayed the
  same requests in the same order from the same start, so they agreed, and the one case that
  differed was the first of the list, the one whose predecessor differed. A3-5's reading is
  wrong: the variance G-5 was meant to measure exists, and M7-P3 did not measure it. M7-P3's
  G-1 interval stands as computed, over case draws only; G-10 measures the rest.

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
