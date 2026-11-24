# RFC SMYSL-9 — three measurements 1.20 left open

**Author:** Vladimir Ulogov
**Status:** draft (2026-11-24). Not yet read against the code as built (R1 is owed before the first
phase of each part).
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

None yet.
