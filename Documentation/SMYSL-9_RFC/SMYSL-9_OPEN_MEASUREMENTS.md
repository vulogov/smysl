# RFC SMYSL-9 — three measurements 1.20 left open

**Author:** Vladimir Ulogov
**Status:** draft (2026-11-25). Every phase built and run. Part C (RT-P1): gates met. Part B
(CC-P1, CC-P2): gates met, CC-1 at its edge. Part A (SI-P1, SI-P2): built, and none of SI-1 to
SI-3 met; the cause found on RE2-SS is not the one §3.1 named (R2-A2). 1.22: `min=` added for
RE2-SS's cause, and RE2-TT's cause found; no template reading rescues RE2-TT (R3-A).
**Target compatibility:** smysl 1.x, strictly additive; format `smysl/1.0`, unchanged. Recipe
texts without the new keys give the same bytes as before.
**Crates:** `smysl-tel` (similarity features, recipe keys), `smysl-series` (resampling),
`smysl-text` (the range tables).
**Amends:** RFC SMYSL-3 §21 and §26, SMYSL-5's GE-I3 arm, SMYSL-2.4 §3.8. Results land as
amendments there; this RFC is the plan and the pre-registered gates.
**License:** MPL-2.0.
**Evidence baseline:** `dev/1.21.0` at `7d6ba97`. Every code pointer below was read there.

---

## 1. Summary

Three items 1.20's changelog lists as open. Each one is a measured weakness with a named cause,
so each part states the cause, the change, and a gate written before the run.

| Part | What was measured | Cause | Change |
|---|---|---|---|
| **A. Similarity on real incidents** (§3) | GE-M8 on RCAEval: recurrence recall@5 0.24 / 0.50 / 0.25 (SS / OB / TT, MinHash), against 0.97 on simulated episodes | "New" Drain templates are mostly drift (median 369 a case on SS); features name series, so two faults of one type on two services share nothing | Template novelty against a reference window; series-independent `kind` features; both as recipe keys under `tel/minhash/2` |
| **B. Calibration at coarser cadence** (§4) | Calibration tested at one sample a second only | No resampling exists anywhere in `smysl-series`, `smysl-tel` or the CLI | A pure `resample` and `step=`/`agg=` recipe keys; GE-I3's calibration rerun at 5 s and 15 s |
| **C. Range tables that grow by less than their size** (§5) | At 10⁵ units, 41 % of a late commit (7.7 ms) is the range index's tables | `axis_on_append` and `observed_on_append` rebuild the sorted row vector and its max-`hi` tree on every append, O(rows) in copies | Blocked rows, each block with its own max `hi`; O(√rows) per append, same dump bytes |

## 2. Invariants

| | Requirement | How this RFC meets it |
|---|---|---|
| **G1. Semver and format** | `smysl/1.0`; `cargo-semver-checks` | No record or wire change. New recipe keys are absent by default, and a recipe without them hashes as before. `tel/minhash/1` stays available and unchanged. The range tables' dump bytes are unchanged (§5). |
| **G2. Purity** | Rule B | Features, resampling and the range structure are pure functions of their inputs. |
| **G3. Determinism** | Rule D | Features are a `BTreeSet<String>`; resampling is defined on sample times, not on arrival; the blocked rows iterate in the same order as the vector they replace. |
| **G4. Epistemic safety** | Rule M; no label from similarity | `tel similar` still writes nothing and proposes no label (SMYSL-3 §21). A resampled series is a read of the series, not a new one: units record the recipe that resampled. |
| **G5. Honest gates** | Pre-registered | Each gate is fixed here, before the run. A result below its gate is reported as measured and amends the RFC; it is not tuned away by changing the gate after seeing the data. |

**Data.** Parts A and B rerun on RCAEval RE2 (SS, OB, TT; CC BY 4.0). It is not in the
repository, and the local copy was deleted after the 1.20 runs (only Zenodo metadata remains).
The runs need it downloaded again, which needs the user's approval each time.

## 3. Part A: episode similarity on real incidents (SMYSL-3 §21, §26)

### 3.1 What is measured today

An episode's MinHash signature is over typed identifier strings (`crates/smysl-tel/src/similar.rs`
`features`): `series:<uid>`, `change:<pointer>`, `anomaly:<series>:<direction>`,
`alert:<system>:<key>`, and `template:<text>` for every log template whose window overlaps the
episode. `K = 128`, recipe `tel/minhash/1?k=128&seed=S`.

`examples/ge_m8_rcaeval` builds one episode a case from `mad/1` anomalies calibrated to 10 % and
the templates Drain finds in the faulty window. SMYSL-3 §26 records:

| recall@5, MinHash | SS | OB | TT | chance |
|---|---|---|---|---|
| recurrence | 0.239 | 0.500 | 0.250 | 0.056 |
| recurrence, no templates | 0.561 | 0.539 | 0.356 | 0.056 |
| fault type | 0.260 | 0.284 | 0.256 | 0.157 |
| fault type, no templates | 0.304 | 0.300 | 0.344 | 0.157 |

Two causes are visible in those numbers:

1. **Templates add noise, not signal.** Removing them doubles recurrence on SS. A median case on
   SS has 369 templates "new in the window"; most are Drain's drift (a variable token read as a
   new template), present before the fault as well as after. §26 already says the open question
   is "which templates count as new, not the measure".
2. **Features name places, not kinds.** A CPU hog on `carts` and a CPU hog on `orders` share no
   feature, because `anomaly:` names the series. The fault-type reading cannot rise much while
   every feature carries a service in it.

### 3.2 The change

Two feature families, chosen by recipe keys, under a new recipe version so `/1` results stay
reproducible:

```
tel/minhash/2?k=128&seed=S&templates=novel&kinds=on
```

- **`templates=novel`** (default `overlap`, which is `/1`'s behaviour). A template is a feature
  only if it is **absent from the reference window** `[start − w, start)`, where `w` is the
  episode's own length. Drift templates appear on both sides of the start and drop out. A
  template seen in both windows whose count rises by more than a factor `r` (recipe key
  `rise=`, default 4) is kept as `template-rise:<text>`, so a known message that bursts is
  still evidence. Both checks read the substrate's log-template counts over the two windows.
- **`kinds=on`** adds one series-independent feature per anomaly: `kind:<metric>:<direction>`,
  where `<metric>` is the series' metric name without its identifying labels (`tel:metric` on the
  series descriptor; for RCAEval's columns, the suffix after the service name, as
  `cpu`, `mem`, `latency`). The `series:` and `anomaly:` features stay, so recurrence still sees
  the place, and type sees the kind.

Nothing is weighted, because MinHash estimates set Jaccard and weights would need another
estimator. Features stay a set.

### 3.3 Gate (pre-registered)

`examples/ge_m8_rcaeval --recipe tel/minhash/2?…` on the same 270 cases, the same episodes and
the same calibration:

- **SI-1.** With `templates=novel`, recurrence recall@5 is at least the no-templates figure on
  every system (0.561, 0.539, 0.356). Novel templates must not cost what removing all templates
  gains.
- **SI-2.** With `kinds=on`, fault-type recall@5 is at least 0.40 on at least two of the three
  systems.
- **SI-3.** Simulated GE-M8 (`make ge-m8`, six seeds) stays within its 1.20 range under `/2`:
  recurrence 0.968–0.987. A change that helps real data by breaking the simulated case is not
  accepted.
- **Stop condition, kept from §26.** If MinHash recall@5 is under 0.6 on a reading where
  embeddings (Model2Vec) exceed 0.8, similarity becomes model-dependent and that is reported. It
  did not fire in 1.20.

## 4. Part B: calibration at coarser cadence (SMYSL-3 §21.1–21.2, SMYSL-5 GE-I3)

### 4.1 What is measured today

`tel calibrate` sets MAD's `threshold`, CUSUM's `h` or PELT's `penalty` at a stated false-alarm
rate over (reference, test) window pairs (`crates/smysl-tel/src/detect/calibrate.rs`).
`crates/smysl-tel/examples/ge_i3_rcaeval.rs` calibrated on RE2-SS and carried to OB and TT at one
sample a second, with six-minute windows (n = 360 points a window) and rate 0.10. Production
metrics are scraped at 15 s to 60 s. Whether a value set at one cadence holds at another, or
whether calibration works at all at a coarser one, is unmeasured. No code resamples a series:
`downsample`, `rollup`, `resample`, `aggregate` and `step_ms` appear nowhere in `smysl-series`,
`smysl-tel` or the CLI. Window summaries (`x.tel/window`) summarise a window; they do not produce
a coarser series.

### 4.2 The change

```rust
// smysl-series
#[non_exhaustive]
pub enum Agg { Last, Mean, Max }
pub fn resample(samples: &[Sample], from_ms: i64, step_ms: i64, agg: Agg) -> Vec<Sample>;
```

Buckets are `[from + k·step, from + (k+1)·step)`, aligned to `from`, never to the first sample.
`Last` emulates a scrape: the last sample at or before each bucket's end, the reading a scraper
at that cadence would have taken. `Mean` and `Max` emulate a rollup. An empty bucket yields no
sample, so gaps stay gaps.

Detector recipes gain `step=` (a duration) and `agg=` (default `last`). With them, `tel detect`
and `tel calibrate` read the resampled series, and the recipe text, and so every unit's recorded
recipe, says so. A recipe without them is byte-identical to today's.

**A minimum n.** A detector needs points. With RCAEval's six-minute windows, 15 s gives n = 24,
and 60 s gives n = 6. `tel calibrate` refuses a trial whose window holds fewer than
`min_n = 20` points after resampling, with a message naming the window. RCAEval's cases are 24
minutes long, so 60 s is not testable on it, and this RFC says so rather than running it.

### 4.3 Gate (pre-registered)

`ge_i3_rcaeval --step 5s` and `--step 15s`, `agg=last`, at rate 0.10, margins 1 and 2, the
1.20 settings otherwise (`floor: resolution` for MAD and CUSUM, `floor: spread` for PELT):

- **CC-1. Within a cadence.** Calibrated on SS and carried to OB and TT at the same cadence, the
  arm that carried at 1 s (CUSUM under `floor: resolution`, one value a window) holds at most
  10 % fault-free alarms on both held-out systems at 5 s. At 15 s the result is reported either
  way.
- **CC-2. Across cadence.** A value calibrated at 1 s and used at 15 s on the same system is
  reported with its realised rate. No gate: this is the measurement production users need before
  they copy a value from one scrape interval to another, and the RFC predicts nothing about it.
- **CC-3.** Recall on RCAEval's faults at 10 % is reported per cadence beside 1.20's 1 s figures.

## 5. Part C: range tables that grow by less than their size (SMYSL-2.4 §3.8; SMYSL-3 §17.1)

### 5.1 What is measured today

The text library's `said`, `composed` and `observed` range tables (`crates/smysl-text/src/index/ranges.rs`)
are `Ranges { rows: Vec<Row>, maxhi: Vec<Bound>, … }`: rows sorted by `lo`, and a max-`hi`
segment tree derived from them and never persisted. Every append that bears on time goes through
`axis_on_append`, whose splice (`spliced`) merges the kept rows and the new ones into a fresh
vector and rebuilds the tree; `observed_on_append` copies every row (`rows().to_vec()`), edits,
and rebuilds. Since AC-8 only the affected rows are re-solved, but every append still copies all
of them. SMYSL-3 §17.1 measured it at 10⁵ units: commit 4.0 → 7.7 ms, of which 41 % is these
tables, 43 % the `fsync`.

### 5.2 The change

Rows in **blocks**: a `Vec<Block>`, each block a sorted `Vec<Row>` of at most `2B` rows and its own
`max_hi`, blocks ordered by their first row. Insert finds its block by binary search on first
rows and inserts in place; a block over `2B` splits in two; a block under `B/2` merges with a
neighbour. Delete is the same. A stabbing query (rows whose `[lo, hi]` holds an instant) skips
any block whose first `lo` is past the instant or whose `max_hi` is before it, then scans.

With `B = 512`, an append costs O(B + rows/B) moves instead of O(rows): at 10⁵ rows, about
700 instead of 100,000. A plain array of blocks needs no balancing and no unsafe code, and its
iteration order is the vector's order, so `Ranges::rows` (the dump, and §3.8's byte comparison)
is unchanged.

`spliced` becomes a sequence of block deletes and inserts for the dropped and added subjects. The
`danglers` path, which recomputes the whole axis while an unresolved dating is present, is
unchanged: its cost is the solve, not the copy.

### 5.3 Gate (pre-registered)

- **RT-1.** §3.8's comparison holds: after any sequence of appends, the incrementally kept tables
  equal the tables built from scratch, byte for byte (a property test over generated append
  sequences, plus the existing goldens).
- **RT-2.** At §24's 10⁵ units, the range tables' share of a late commit is under 10 %, and the
  commit is under 5.0 ms on the machine that measured 7.7.
- **RT-3.** Per-append time in the range tables grows by at most 2× from 10⁴ to 10⁵ units, where
  it grew about 10× before.

## 6. Phases

- **SI-P1 — features.** `templates=novel`, `rise=`, `kinds=on`, `tel/minhash/2`; `tel similar`
  reads them. **Gate:** `/1` recipes give 1.20's signatures bit for bit; unit tests on drift
  templates (present in both windows, dropped) and a burst (kept as `template-rise:`).
- **SI-P2 — the run.** GE-M8 on RCAEval and simulated under `/2`. **Gate:** SI-1 to SI-3,
  reported in SMYSL-3 §26 as an amendment. Needs RCAEval downloaded with approval.
- **CC-P1 — resampling.** `resample`, `Agg`, `step=`, `agg=`, `min_n`. **Gate:** a series at
  1 s resampled to 15 s by `Last` equals the series a 15 s scraper would have recorded, on
  generated series; a recipe without `step=` hashes as before.
- **CC-P2 — the run.** GE-I3 calibration at 5 s and 15 s. **Gate:** CC-1, reported with CC-2 and
  CC-3 as a SMYSL-5 amendment (R13). Needs RCAEval.
- **RT-P1 — blocked rows.** **Gate:** RT-1 to RT-3, with the measurements in SMYSL-3 §17.1.

The three parts are independent and may land in any order. RT-P1 needs no data download.

## 7. Questions for R1

1. Whether a series descriptor carries a metric name separate from its labels (`tel:metric` or
   equivalent), for `kind:`; and what the Prometheus, OTLP and Zabbix readers put there.
2. Whether the substrate can count a template over an arbitrary window without decoding every
   log line, for `templates=novel`.
3. Whether `tel detect` reads a series through one function that `step=` can wrap, or through
   several.
4. Whether the range tables are written to the sidecar whole at every commit. If so, the write
   is O(rows) too and RT-2 may need the sidecar written by block.
5. Whether `min_n = 20` agrees with the minimum each detector already assumes.

## 8. Amendments

**R1 for Part C, 2026-11-24: §5 read against the code at `869bffb`.**

- **R1-C1. Question 4: the sidecar is not written at a commit.** `Library::append_records`
  updates the index in memory (`Index::on_append`) and does not call `write_index`; the CLI's
  writing commands call it once per command, and `oko_ledger` never does. So the O(rows) in §17.1
  is the in-memory tables alone, and RT-2 needs no sidecar written by block. The dump format is
  untouched.
- **R1-C2. `Ranges::rows` is public and returns `&[Row]`.** Blocks cannot lend one slice, and
  changing the return type would break `smysl-text`'s API. `rows` keeps its signature over a
  flattened copy built on the first call after a change. Only the dump and tests call it, and the
  dump is O(rows) anyway.
- **R1-C3. Removing a subject's row needs to find it.** `spliced` drops rows by subject, and rows
  are sorted by `(lo, hi, subject)`, so a block delete needs the row. The table keeps a
  subject-to-row index, built by its first splice, never persisted, so a table that is only read
  never pays for it. `Ranges::of` accepts two rows for one subject, which no builder in the crate
  makes; such a table falls back to splicing whole, as every splice did before.
- **R1-C4. The query's "rows read" bound stays.** `tests/ranges.rs` asserts that a day's window
  over 155,510 rows reads fewer than 200 (TX-P4 step 3), so a block cannot simply be scanned. Each
  block keeps a max-`hi` segment tree over its rows, rebuilt in O(B) when the block changes. A block
  is skipped on its first row and its `max_hi`, and neither check reads a row as the query counts
  them.

**R2, 2026-11-24: RT-P1 as built.**

- **R2-C1. The structure.** `Ranges` holds blocks of 256 to 1,024 rows (`B = 512`), each with
  its own max-`hi` tree. `spliced` became an in-place `splice`: each dropped subject's row is
  removed from its block, each new row is inserted where it sorts, and a block over `2B` splits
  while one under `B/2` merges with a neighbour. `observed_on_append` splices too, instead of
  copying every row. `rows`, equality and the dump are unchanged (R1-C2).
- **R2-C2. RT-1 holds.** The property test (`ranges::tests`, 25 generated cases of 60 splices
  over up to 3,000 starting rows, with batches of 1,500 so that blocks split and merge) compares
  every splice with `Ranges::of` over the same rows, checks the layout invariants, and compares
  four windows each against `scan`. A subject with two rows splices whole and stays correct.
  `smysl-text`'s existing goldens, which compare tables kept incrementally with a rebuild, pass
  unchanged.
- **R2-C3. RT-2 holds.** `oko_ledger` at 17,000 investigations, which is 116,229 operations and
  99,249 units, §17.1's size, on the machine that measured 7.7. Late-commit p50 is **4.75 ms**
  (gate 5.0) and p95 5.79 ms. The range tables are **1.9 %** of the late commits' samples (gate
  10 %; it was 41 %). That share comes from macOS `sample`, 10 s every 20 s over the last two
  minutes, 39,134 samples in the commit. Most of the rest of a late commit is the append's
  `F_FULLFSYNC`, 56 % of the samples.
- **R2-C4. RT-3 holds.** Range-table time per append is the share times the late-commit p50:
  1.4 % of 3.99 ms at 10⁴ units (1,500 investigations, 8,835 units) and 1.9 % of 4.75 ms at 10⁵,
  0.056 → 0.090 ms. That is **1.6×** for 11× the units, where it was about 10× before. The 10⁴
  share comes from three samples across a 42 s run, not only its last tenth.
- **R2-C5. Not caused by this change, and left open.** Staging's late p50 at 10⁵ is 1.11 ms,
  where §17.1 recorded 0.63 ms over the same 116k operations. Staging does not touch the range
  tables. Since AC-8 it has grown with something else, and finding what is owed. The worst
  commit of the clean run was 365 ms, a single outlier §17.1 has no figure to compare with.

**R1 for Parts A and B, 2026-11-25: §3 and §4 read against the code at `9468ca7`.**

- **R1-A1. 1.20's "new" templates were already novel by absence.** `examples/ge_m8_rcaeval` fed
  one Drain per container the reference window's lines, then the faulty window's, and counted
  the templates the second pass added, by text. That is §3.2's `templates=novel` as first
  written. What made a median of 369 a case is not absence from the reference: Drain only ever
  generalises a cluster (`drain.rs`, `add`), so a template the reference window held as
  `GET /cart user alice` is `GET /cart user <*>` after one more user, and its text is new. Text
  equality cannot see that. `templates=novel` is therefore judged by **family**: two templates
  are one family when they have as many tokens, no position where both are words and differ,
  and at least one where they are the same word.
- **R1-A2. Question 2: the reference is read from units, not the substrate.** `tel similar` reads
  only the library's store (`src/main.rs`, `tel_similar`); counting a template over an arbitrary
  window would decode every event in it (`logs::materialise`). So a template's reference is the
  `x.tel/log-template` units of the same stream and instrument in the **latest window that ends
  at or before its own begins**, not `[start − w, start)`. The episode's own length plays no
  part: an episode of a few seconds would otherwise have a reference of a few seconds. A
  stream with no earlier window has no reference, and its templates are all new. `rise=`
  compares rates, count over window length, in integers.
- **R1-A3. Question 1: no series carries a metric name apart from its key.** `tel::series` writes
  `tel:scheme` and `tel:key` only, by design (§4.1: `name` is not part of a series' identity).
  `kind:` reads the name from the key, per scheme: Prometheus' name before `{`, OTLP's `name`
  member of its JCS key, a Zabbix item key before `[`. Other schemes give no `kind:`; nor does an
  anomaly whose series unit is not in the store. RCAEval's columns (`carts_cpu`) carry no labels,
  so SI-P2 keys them as Prometheus series with the service as a label (`cpu{service="carts"}`).
- **R1-A4. `/2` keys MinHash by its full recipe text.** `MinHash::new(seed)` hashed
  `tel/minhash/1?k=128&seed=S`; `/2` hashes its own text with every key written, so `/2` with
  its defaults reads `/1`'s features under different functions and is never mistaken for it.
- **R1-B1. Question 3: one read per path, two paths.** `detect::run_on_with` reads windows through
  one closure for CUSUM, PELT and MAD (and MAD's baseline through the same one), and
  `calibrate_on` and `calibrate_on_each` through `read_trials`. `step` wraps both; BARO and
  episodes are not level detectors and do not take it.
- **R1-B2. `step` and `agg` are `detect::Options`, as `floor` is.** That is what `stamp` writes
  into each output's recipe and what `Spec::hjson` writes back, so a recipe without them is
  byte-identical by construction. `step` is in ms, an integer, as `gap` and `baseline` are.
- **R1-B3. Question 5: no detector assumes 20 points.** CUSUM estimates each regime from
  `baseline` points (default 10), PELT needs `2 · min_size` (4), MAD a non-empty baseline.
  `min_n = 20` (`detect::MIN_N`) is twice CUSUM's baseline: a floor for a calibration worth
  reading, not a detector's minimum. It applies only under `step`, so calibration without one
  behaves as before.
- **R1-B4. `Last` keeps the sample.** Each bucket's last counted sample is returned as it is,
  its own instant and flags, which is what a scraper at bucket ends that keeps an exporter's
  timestamps and never records one sample twice would store. `Mean` and `Max` stamp the bucket's
  start. Each window is aligned to its own start, the reference's to its start and the test's to
  its start.

**R2-A, 2026-11-25: SI-P1 as built.** `similar::Recipe` (`v1`, `v2`, `parse`, `text`),
`Templates`, `episodes_with`, `nearest_with`, `MinHash::of` and `metric_name`; `tel similar
--similarity RECIPE`. The gate holds: `/1` signatures equal those of `9468ca7` on a fixed set
(`v1_signatures_are_the_ones_1_20_gave`), a drift template of the reference's family is
dropped, a burst of 2 → 40 is kept as `template-rise:` and dropped at `rise=20`, and two CPU
anomalies on two services share `kind:cpu:up` and nothing else.

**R2-B, 2026-11-25: CC-P1 as built.** `smysl_series::resample` and `Agg`; `detect::Resample`,
`Options::resample`, `Options::read`, `MIN_N`. The gate holds: on 40 generated one-second series
with jitter, missed samples, gaps longer than a step and stale markers, `Last` at 5 s and 15 s
(and at 15 s from an offset start) equals a simulated scraper's record; a recipe without `step`
stamps the same text as before. `tel calibrate` with `step: 60000` refuses the GE-M3 fixture's
ten-minute windows (ten points) naming the first, and calibrates twenty-minute ones.

**R2-A2, 2026-11-25: SI-P2, the run. No gate is met.** `examples/ge_m8_rcaeval --similarity
RECIPE` and `examples/ge_m8 --similarity RECIPE`; the figures and their reading are SMYSL-3 §26's
1.21 amendment. In brief:

- **SI-1 fails on every system.** `templates=novel`, recurrence by MinHash: 0.239, 0.528, 0.328
  against 0.561, 0.539, 0.356 (exact Jaccard 0.350, 0.589, 0.339).
- **SI-2 fails.** `kinds=on`, type by MinHash: 0.258, 0.293, 0.316, none at 0.40; 0.329, 0.289,
  0.376 without templates.
- **SI-3 is not met as worded.** Both keys give 0.965–0.983 over the six seeds against
  0.968–0.987; `/2` with neither key gives 0.955–0.987, so the miss is MinHash's functions, not
  the features (exact Jaccard 0.972–0.992 against 0.972–0.985).
- **§3.1's first cause was wrong, and R1-A1's correction was not the whole of it.** On RE2-SS the
  "new" templates are singletons: lines whose own identifiers (a user name, an e-mail) leave two
  of six tokens in common, under Drain's 0.4, so each line is its own template in whichever
  window it falls. Family cannot match them. Dropping a window's count-1 templates, a diagnosis
  run after the gates had failed, brings RE2-SS to 0.550 / 0.567, the no-templates figure: once
  singletons go, the templates left neither help nor hurt. On RE2-TT, singletons are about a
  third of the templates, and its cause is not found.
- **A gate on one MinHash reading is finer than MinHash.** Renaming the series (for `kinds:`)
  moves recurrence by up to 0.04 with exact Jaccard unchanged. A successor should gate on exact
  Jaccard, or on MinHash over several seeds.
- **What ships.** `/2` and its keys stay, as readings whose gain is not shown; `/1` stays the
  default. No recipe key was added after the run: `--drop-singletons` is the example's.

**R2-B2, 2026-11-25: CC-P2, the run. CC-1 is met, at its edge.** `ge_i3_rcaeval --step 5000` and
`--step 15000`, `--held` RE2-OB and then RE2-TT; the figures and their reading are SMYSL-5 R13
and §15's 1.21 paragraph. In brief:

- **CC-1.** CUSUM under `floor: resolution`, one value a trial, calibrated on RE2-SS at 5 s and
  carried at margin 1: 0.0 % fault-free alarms on RE2-OB and 10.0 % on RE2-TT (9 of 90, the
  rate). At 15 s, reported either way: the same.
- **CC-2.** A value calibrated at 1 s and used at 5 s or 15 s alarms less on CUSUM and PELT (0
  to 2.2 %), and MAD stays near the rate (up to 14.4 % per kind at 15 s). CUSUM's recall at its
  1 s value falls to 0–53 % at 15 s.
- **CC-3.** At its own cadence's calibration, each level detector sees every RE2-SS fault type
  at 87 to 100 %.
- **`min_n`.** At 15 s, 43 of RE2-TT's 1,968 (series, window) hold fewer than 20 points, against
  1 at 1 s; the example reads pairs directly and does not refuse them, so the CLI's refusal is
  not exercised by this run.

**R3-A, 2026-11-25 (1.22): singletons as a recipe key, and RE2-TT's cause.** Both follow-ups
1.21's changelog named. Rerun on the same 270 cases, the same episodes and calibration, with the
series keyed as in R2-A2. Within this run, the no-templates figures are 0.539 / 0.533 / 0.417
(MinHash; SS / OB / TT) and 0.561 / 0.561 / 0.411 (exact Jaccard). TT's is higher than the 0.356
SI-1 used, which was read under 1.20's series keys.

- **`min=N`**, a `/2` key (`similar::Recipe::with_min`): a template unit counted fewer than `N`
  times in its window is read as absent, as a feature and in a reference. At 1 it is not
  written, so every 1.21 `/2` text, and its signatures, is unchanged. `examples/ge_m8_rcaeval`
  drops `--drop-singletons` for it.
- **`templates=novel&min=2`** gives the diagnosis's figures exactly in exact Jaccard: 0.567 /
  0.556 / 0.328 (MinHash 0.556 / 0.550 / 0.289; the functions differ with the recipe text).
  RE2-SS recovers from 0.239 to its no-templates figure, as R2-A2 found; RE2-OB is unchanged;
  RE2-TT stays below its no-templates figure. With `kinds=on` added: 0.561 / 0.561 / 0.344.
- **RE2-TT's cause.** Each episode keeps about 90 templates after `min=2` (median 91), and 6,234
  of its 6,617 distinct template features are in one episode only. Only 11 of RE2-TT's 90
  repetition pairs share any template (none for `disk` or `socket`). A case's best repetition
  is no closer by exact Jaccard (median 0.118) than its fifth-nearest other case (0.117); on
  RE2-SS the two are 0.458 and 0.319. The templates are Train Ticket's request payloads: Java
  objects printed whole (`LeftTicketInfo{soldTickets=[Ticket(seatNo=1118025005, …`, median 568
  tokens, up to 4,904) by `ts-seat-service`, `ts-order-service` and `ts-order-other-service`,
  which carry 95 % of them. They are new in every window because every window's tickets differ,
  and they come from the load generator, not the fault, so they recur nowhere and dilute the
  series and anomaly features that do.
- **What was tried, in the example only, and not kept.** Every Train Ticket line starts with its
  own date (`2024-01-23 09:56:25.001 INFO 1 --- [thread]`); the date stays a literal word in a
  template, and 45 of 76 repetition pairs with templates were recorded on different days. Masking
  it changes nothing (exact 0.328): the payloads differ anyway. Masking digit runs inside tokens
  makes RE2-TT worse (0.261), because the payloads also differ in length, and Drain's first
  layer is token count; it also costs RE2-SS (0.511). Truncating messages to 12 tokens, or to
  40 after the digit mask, gives 0.361 and 0.372. Leaving out templates longer than 10, 20, 40
  or 80 tokens, computed from the episodes' features, gives 0.361 to 0.372. None reaches 0.411.
- **What ships.** `min=` only. `/1` stays the default, and the reading for systems that log
  payloads is no templates (`--no-templates` in the example). A Drain mask for dates or for
  digits inside tokens would be a new `tel/drain` recipe, since the masks are in its text, and
  it is not proposed: it did not help.

