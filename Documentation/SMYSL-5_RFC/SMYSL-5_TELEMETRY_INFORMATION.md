# RFC SMYSL-5 — information measures over telemetry

**Author:** Vladimir Ulogov
**Status:** first draft, amended after review (R1, 2026-11-20), while building (R2 to R6) and
after (R7 to R11).
TI-P0 to TI-P4 are built: `ent/1`, `perm/1`, `mi/1`, `jsd/1` and `tent/1`, rule TI in `check`,
and their recomputation. TI-P5 closed with GE-I5's result: transfer entropy is not released,
and `te` stays reserved (§10). GE-I1 to GE-I5 have run on simulated data (§15), and GE-I3 on RCAEval's
RE2-SS, RE2-OB and RE2-TT, real (R7 to R11); every other real-data run is still owed, and every other threshold is a
proposal until the experiment that owns it has run (§17).
**Target version:** not fixed. Phases are named (TI-P0 to TI-P5), not numbered by release.
**Evidence baseline:** `dev/1.18.0` at `3e57160` ("SMYSL-4 BQ-P4"). Every code pointer below
describes that tree. Re-check them against the tree in hand before starting a phase.
**Companions:**
- RFC SMYSL-3 (`Documentation/SMYSL-3_RFC/SMYSL-3_TELEMETRY.md`), cited as 3 §n;
- RFC SMYSL-2.6 (`Documentation/SMYSL-2_RFC/SMYSL-2.6_MEASURES_AND_ASSESSMENT.md`), cited as
  2.6 §n.

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as in RFC 2119, and only in
Part A.

**Amendments.**

| Rev | Date | Change |
|---|---|---|
| R1 | 2026-11-20 | Review against `3e57160`. **R1-1** a stale or no-value sample ends the value in force (§5.1). **R1-2** admission is by the series' `tel:kind`, not a class, and every kind is placed (§5.2, D-I12). **R1-3** a statistic is stored rounded to six places so rule TQ can verify a quote of it (§6, D-I13). **R1-4** a recipe value never holds `?`, `&` or `=`; `tent/1` names Drain's recipe by hash (§4). **R1-5** one observed symbol is exactly 0 and a negative result is clamped to 0 (§6). **R1-6** surrogates rotate over the whole grid and pair by grid position (§6.3). **R1-7** every lag of a scan meets `min_n`, and the maximum is taken on stored values (§6.3). **R1-8** `SMY-E603` and `SMY-E604` are reworded when they extend (§9). **R1-9** recipe ids are bare, as detectors' are (D-I14). **R1-10** the CLI synopsis, the four-platform CI step and TQ's work in TI-P4 are corrected to the tree (§12–§14). |
| R2 | 2026-11-20 | Found while building TI-P1. **R2-1** admission is by the leaf class the substrate records for a series (`SeriesRow::class`), which every reader writes, and not by `tel:kind`, which the schema allows and no reader writes: R1-2 read the schema and not the readers, and the draft's "class" was right. `state` is not a class, so its row goes. **R2-2** `diff` reads the sequence `of` produces, so under `of=increase` it is the sign of the increase's change. |
| R3 | 2026-11-21 | Found while building TI-P2. **R3-1** `mi/1` refuses `baseline` bins: they would read a reference window of each series, and `x.tel/info` has one `ref_digest`, so rule TI could not be met (§5.3). **R3-2** `guard` is a share of the grid length `G`, as §6.3 always said, not of `n` (§4). **R3-3** `mi/1`'s `cover` at lag `ℓ` is over the `G − |ℓ|` points where a pair can form, and `missing` counts those where none did; otherwise a long lag alone would fail it (§6.3). **R3-4** a scan's `n`, `k`, `k_with`, `bias` and `missing` are those at `lag` (§6.3). **R3-5** a scan's surrogate counts toward `null_ge` only when every lag meets `min_n` (§6.3). **R3-6** `jsd/1`'s `k` and `missing` are the window's, `cover` applies to each window, and `H(M)` is a plug-in over integer weights, so no probability is formed (§6.2). |
| R4 | 2026-11-21 | Found while building TI-P3. **R4-1** one window's template units that carry different digests (late events) are separate inputs, and give one `tent/1` unit each, as windows of one key do (§6.4). **R4-2** a metric series given to `tent/1` is `SMY-E701`; a `templates` hash other than the shipped Drain's, with no such templates in the store, is `SMY-W703`, since this build runs one Drain (§6.4). **R4-3** templates `tel info` materialises are committed before the statistic, with their stream's series unit, attested as `tel templates` attests them (§6.4). **R4-4** GE-I4's stop condition failed on 2 of 20 seeds, for a fault that barely moves the entropy, while Drain's share of every boundary move was zero; `tent/1` ships by the author's decision, and the result is recorded in §15. |
| R6 | 2026-11-22 | TI-P5. **R6-1** GE-I5's reading of its stop condition, fixed before the run, and its result: `te/1` is not released, and TI-P5 closes with no code beyond the experiment (§10, §15). |
| R5 | 2026-11-21 | Found while building TI-P4. **R5-1** two info units of one key disagree (`SMY-W614`) when they read different sample sets: their `digest`, `with_digest` and `ref_digest` together (§3). **R5-2** recomputation reads a stored recipe text back into a recipe, and one this build would write otherwise, a parameter respelled or missing, is `SMY-E604`, since recomputing it would compute something else; `SMY-W704` is a recipe id this build does not have (§7). **R5-3** rule TI's checks of the fields: `mi` carries `with`, `with_instrument`, `with_digest` and `lag`; a statistic of one series names no second; `jsd` names a reference window; a reference window carries `ref_from_ms`, `ref_to_ms` and `ref_digest` together; a scan's `lags` and `values` are of one length (§3). **R5-4** a `template-entropy` unit is recomputed from the store's templates of its stream, instrument, window and recipe hash; with none, from the events, for the shipped Drain only; with neither, it is `SMY-W605` (§7). **R5-5** the dossier lists the statistics among the evidence an investigation consulted or grounds on, and its open questions include those not re-derivable (§8). **R5-6** GE-I3's reading of its stop condition, fixed before the run, and its result (§15). |
| R7 | 2026-11-22 | 1.19. **R7-1** GE-I3's real-data arm, on RCAEval RE2-SS: its first reading, held to MAD's false-alarm rate with the detectors at their shipped defaults, was degenerate on one-second data, since those defaults flag 90 to 100 % of normal windows; the reading was amended before the rerun to hold every arm, detectors included, to 10 %. Its result: every fault type is seen by a level detector, and no statistic flags a faulty window none of them does (§15). |
| R8 | 2026-11-22 | 1.19. **R8-1** The detectors are calibrated rather than scored on a grid: `detect::calibrate` (3 §21.1) sets MAD's threshold, CUSUM's `h` and PELT's `penalty` on RE2-SS's normal pairs to 10 %, and the values are run unchanged on RE2-OB, held out. **R8-2** GE-I3's real-data arm is replicated on RE2-OB. Its result: RE2-SS's values hold on RE2-OB (1 to 4 % false alarms) and miss only CUSUM's and PELT's delays there, and RE2-OB gives R7's verdict (§15). |
| R9 | 2026-11-22 | 1.19. **R9-1** The detectors are also calibrated per series (3 §21.1, `calibrate_each`), keyed by metric kind so that values carry between systems, the rate held jointly over the trials. **R9-2** Run on RE2-SS and held out on RE2-OB beside R8's one value a trial. Its result: per series, CUSUM sees every RE2-SS fault and, carried to RE2-OB, its delays (100 % against 13 %); PELT's carried values overshoot their rate there (16.7 %); R7's verdict holds in all four readings (§15). |
| R10 | 2026-11-22 | 1.19. **R10-1** Every calibration is also made under `floor: resolution` (3 §21.2) and at a transfer margin of 2 (3 §21.1). **R10-2** Its result: the floor brings CUSUM's values to sane ranges and with them RE2-OB's delays at one value a trial (100 % against 13 %), and does nothing for PELT on float-valued metrics; a margin of 2 halves an overshoot that is sampling and not one that is the systems differing. R7's verdict holds, with one RE2-OB pair under the floor flagged by a statistic and no detector (§15). |
| R11 | 2026-11-23 | 1.19. **R11-1** Per-series calibration no longer refuses when a series fires at the ladder's top in a fault-free trial: the series takes the top, those trials count against the rate jointly, and only more of them than the rate allows refuse (3 §21.1). **R11-2** Every R10 calibration on RE2-SS is carried to a third system, RE2-TT (Train Ticket). CUSUM under `floor: resolution` at one value a trial is the only detector configuration that held its rate on both systems; per-kind values overshoot on one system or the other, PELT's up to every fault-free window. R7's verdict holds (§15). |

---

## 0. Summary

smysl has two research surfaces that share a store and no code.

- `smysl-measure` (2.6) computes entropy, compression, surprisal and divergence over **text**,
  and structural measures over **units**.
- `smysl-tel` (3) reads telemetry into a series substrate and writes units for what is asserted
  about it: changes, windows, segments, anomalies, episodes, hypotheses.

Nothing computes an information-theoretic quantity on the telemetry itself. `smysl-measure`
never reads the substrate, and every shipped detector (3 §21) looks at the *level* of one
series: CUSUM and PELT find shifts in it, the robust z finds points far from it, and the
BARO-style ranking scores how far it moved. None of them can say that a series became less
predictable at the same level, that two series started or stopped moving together, or that a
log stream's mix of messages changed.

This RFC adds those statements, as evidence:

**1. One new unit type, `x.tel/info`.** It states a statistic of a symbolised sample set: the
entropy of a series over a window, its permutation entropy, the mutual information between two
series at a lag, the divergence between two windows of one series, the entropy of a log
stream's templates. It is `derived`, it carries the digest of the samples it read, and `check`
recomputes it while they are present, exactly as it does a sketched window (3 §9.5).

**2. Symbols are made in exact decimal arithmetic.** A series becomes symbols on an explicit
grid, by rules that compare and subtract `dec` values and nothing else (§5). Floating point
enters once, in the final sum over integer counts, through the same pinned `libm` that
`smysl-measure` uses. So the counts are identical on every machine by construction, and the
value is as replayable as a 2.6 measure.

**3. A number comes with what it would be by chance.** A plug-in mutual information is never
zero on finite data. Each pair statistic records its analytic bias term and, on request, how
many of B seeded surrogates reached it (§6.3). A lag scan is one unit holding the whole curve,
and its surrogates are compared at their own maximum, so choosing the best lag is not hidden.

**4. Nothing here is a cause.** A statistic is evidence. A hypothesis that grounds on one is
`inferred` at most, by rule M, and this RFC ships no recipe that writes a hypothesis. Transfer
entropy, the one directional statistic, is reserved behind an experiment (§10, GE-I5).

**What this RFC adds:**

| Area | Addition |
|---|---|
| Format | No new record type, no kernel change, no new encoding. `x.tel/v1` moves to revision 2, additively. |
| Schema | One unit type, `x.tel/info`. No relation kind. |
| Rules | Rule TI, a paragraph beside rule TW: how an info unit is attested and what it must carry. |
| Code | One module, `smysl_tel::info`. `libm` joins `smysl-tel`, pinned with `smysl-measure`'s. No new crate. |
| CLI | One action, `smysl tel info`. |
| Delivery | Six phases, TI-P0 to TI-P5. Five gated experiments, GE-I1 to GE-I5. |

## 1. Decisions

Each names the option it chose, so that a later reversal is a recorded change.

| # | Question | Decision | Rejected | Why |
|---|---|---|---|---|
| D-I1 | Where the code lives | **`smysl-tel`, as a module beside `detect`.** | `smysl-measure`, as measures S1… over a series scope | A 2.6 measure is a function of a store and an sq scope, its result is grounded on a saved query, and a query never touches the substrate (3 §22.1). Samples are not units, so a series has no scope to resolve. `smysl-measure` also depends on `smysl-query`, `smysl-text` and `smysl-pack`, none of which `smysl-tel` needs. The shape that fits already exists: a recipe, a window, a derived unit grounded on its series. |
| D-I2 | What a result is | **A new type, `x.tel/info`.** | Reusing `x.tel/anomaly` with a score; a `@data` measure unit | An anomaly asserts that something is wrong, and an entropy asserts nothing of the kind. A `@data` measure unit has no `digest`, so `tel-evidence` could not recompute it and retention could not report it (`SMY-W605`). |
| D-I3 | Numbers in the payload | **`dec`, by 3 §5.3**, as every `x.tel` number is, rounded as D-I13 says. | 2.6's `q20` fixed point | Rule TQ (3 §11.4) verifies a quoted number against a `dec` field. An investigator who writes "the mutual information was 0.831294 bits" should be checked the same way as one who quotes a window's maximum. |
| D-I4 | Transcendentals | **`libm`, pinned at the version `smysl-measure` pins, through wrappers in `info::num`.** The `detect` module keeps its rule of none. | Avoiding logarithms (a table, as the DDSketch buckets do) | An entropy is a sum of logarithms. 2.6 §2.9 measured the problem and 2.6 §3.5 solved it; this reuses the solution. The pin becomes a workspace dependency so the two crates cannot drift apart. |
| D-I5 | How a series becomes symbols | **An explicit grid and an explicit binning, both recipe parameters with no default for the grid step** (§5). | Reading raw samples; a default step taken from the chunk span | Raw samples weight a densely scraped hour over a sparse one. A chunk span is a local layout (3 §19.2) and may not enter an identity. |
| D-I6 | Estimators | **Plug-in over counts, with the Miller–Madow term stored beside it.** | k-nearest-neighbour (KSG), kernel, Bayesian (NSB) estimators | A plug-in value is a function of integer counts in a fixed order, which is what makes it replayable and portable. The others are floating-point algorithms a second implementation would have to match bit for bit. GE-I2 decides whether plug-in is good enough at telemetry sample sizes. |
| D-I7 | Chance level | **Seeded circular-shift surrogates, counted, on request.** ChaCha20 keyed by the recipe and seed, as MinHash is (3 §21). | A p-value field; permutation surrogates; nothing | A count of surrogates out of B is an exact integer pair, and a p-value is a claim. Permuting a series destroys its autocorrelation, which makes independent but slowly varying series look related. A circular shift keeps each series as it is and breaks only the alignment. |
| D-I8 | Lag scans | **One unit carrying the curve**, with surrogates compared at their maximum over the same lags. | One unit per lag; writing the best lag only | One unit per lag multiplies units by the scan width. Writing only the best lag hides that it was chosen. |
| D-I9 | Causal statistics | **Transfer entropy reserved**, behind GE-I5. No recipe in this RFC writes a hypothesis. | Shipping transfer entropy and a ranking with the first cut | 3 §21 lists causal discovery as "not planned" because its edges rest on assumptions that do not reliably hold. A common driver makes transfer entropy point between two series that do not affect each other. |
| D-I10 | CLI | **A new action, `smysl tel info`.** | Running these through `tel detect` | A detector finds something and these describe something. The recipes share a file format and nothing else, and `detect`'s report counts "units found". |
| D-I11 | Name | **SMYSL-5.** Phases TI-P0 to TI-P5, experiments GE-I1 to GE-I5, diagnostics `SMY-E7xx` / `SMY-W7xx`. | — | The 7xx range is unused at the baseline. |
| D-I12 (R1, R2) | Which series are admitted | **By the substrate's leaf class** (§5.2, R2-1): `gauge`, or none, as `value` or `increase`; `counter` only as `increase`; every other class refused. | Admitting by `tel:kind` (R1-2) | Every reader records a series' leaf class (3 §4.3) in its substrate row when it first sees it: OTLP's monotonic sum is `counter`, its gauge and non-monotonic sum `gauge`, its histograms and summaries `histogram`. `tel:kind` is an optional field of `x.tel/series` that no reader writes, so admitting by it would admit every counter as a level. |
| D-I13 (R1) | How a statistic is stored | **Rounded to six decimal places, half to even**, after the clamp of §6, then written as a `dec` by 3 §5.3. `bias` and `mm` likewise. | The shortest binary64 spelling (`Decimal::from_f64`); requiring quotes to give it in full | Rule TQ verifies by equality of canonical text: `0.90` does not verify `0.9`. A 16-digit value cannot be quoted by a person. Six places is below every threshold §15 can act on, and a last-bit `libm` difference moves a stored value only when it crosses a rounding boundary, which GE-I1 sees. |
| D-I14 (R1) | Recipe ids | **Bare, as detectors' are**: `ent/1`, `perm/1`, `mi/1`, `jsd/1`, `tent/1`, `te/1`. | `tel/`-prefixed, as Drain's `tel/drain/1` | A statistic's recipe shares the detectors' file format and `Recipe` type, and is read beside them. The unit's type already says which family it is. |

## 2. What this does not change

- **Wire and kernel.** Nothing in `smysl/1.0` or the kernel schema changes. `dec` and `bytes32`
  are used as 3 §5 defines them.
- **The ports.** `python/`, `nodejs/` and `go/` hold C-Tel-Read and already decode `dec`. An
  `x.tel/info` unit is preserved by them under rule X. They learn nothing new.
- **`smysl-measure`.** No measure is added, moved or renumbered. Its unit measures (A1–A10)
  already run over a telemetry library, and an `x.tel/info` unit is a unit like any other to
  them.
- **Detectors.** No shipped detector changes its output. GE-M3's expected uids
  (`fixtures/tel/detect/expected.tsv`) stay as they are.
- **Episode similarity.** `x.tel/info` units are not MinHash features (3 §21), so GE-M8's
  result is not disturbed.
- **Purity.** `smysl-tel` stays on `xtask check-purity`'s list with the same bans. `libm` with
  default features off brings nothing on `NEVER`.

---

# Part A — Normative

## 3. The `x.tel/info` unit

`x.tel/v1` moves to revision 2. The revision adds one unit type and its payload keys, and
changes the meaning of nothing in revision 1. A reader at revision 1 preserves the type under
rule X.

| Field | Type | Required | Meaning |
|---|---|---|---|
| `stat` | enum | yes | `entropy`, `perm-entropy`, `mi`, `jsd`, `template-entropy`; `te` reserved (§10) |
| `series` | uid | yes | the series, or the log stream for `template-entropy` |
| `instrument` | uid | yes | the instrument whose samples were read |
| `from_ms`, `to_ms` | int | yes | the window `[from_ms, to_ms)` |
| `recipe` | text | yes | the recipe text (§4) |
| `n` | int | yes | the number of symbols, or symbol pairs, the value was computed from |
| `value` | dec | yes | the statistic, in bits, as §6.0 stores it |
| `digest` | bytes32 | yes | 3 §9.1's digest of `series` and `instrument` over the window; for `template-entropy`, 3 §19.6's event digest |
| `k` | int | no | distinct symbols observed in `series` |
| `mm` | dec | no | the Miller–Madow value (`entropy`, `perm-entropy`, `template-entropy`) |
| `missing` | int | no | grid points that gave no symbol (§5.1); absent when zero |
| `with`, `with_instrument` | uid | `mi` | the second series and its instrument |
| `with_digest` | bytes32 | `mi` | the second stream's digest over the same window |
| `k_with` | int | no | distinct symbols observed in `with` |
| `ref_from_ms`, `ref_to_ms` | int | `jsd`, and any recipe with `baseline` bins | the reference window |
| `ref_digest` | bytes32 | as above | the series' digest over the reference window |
| `bias` | dec | no | the analytic bias term of `mi` (§6.3). Stored, never subtracted. |
| `lags` | list<int> | scan | the lags scanned, ascending, in grid steps |
| `values` | list<dec> | scan | the statistic at each lag, each stored as §6.0 says; `value` is their maximum |
| `lag` | int | `mi` | the lag `value` belongs to; in a scan, the smallest-magnitude lag at the maximum of the stored `values`, the negative one first |
| `null_b`, `null_ge` | int | no | surrogates drawn, and those whose statistic was at least `value` (§6.3) |

Keys are `tel:`-prefixed in the payload, as every `x.tel` key is.

**Header.**
- Status `derived`.
- `grounds`: the series; for `mi`, both series, in uid order.
- `source`: as a segment's (3 §4.1), `{kind: metric, reference: "series:<scheme>:<key>"}` for
  `series`, with `observed` at `to_ms`.
- An `x.tel/of-series` relation to each series it grounds on.

**Rule TI.**

> - An `x.tel/info` unit is attested with `Op::Transformed` at `Rung::Computed`, and is
>   `derived`. A statistic depends on parameters, as a sketch does (rule TW).
> - It MUST carry the digest of every sample set it read: `digest`, and `with_digest` and
>   `ref_digest` where those sets exist.
> - Its `value`, and every other number it carries, MUST be the function §5 and §6 define of
>   those sample sets and its `recipe`.
> - An implementation MUST NOT attest otherwise (`SMY-E603`).

**Several units over one key.** Late samples change a digest, and so make a second unit beside
the first, as they do for windows (3 §9.2). Neither supersedes the other. The **info key** is
(`stat`, `series`, `instrument`, `with`, `with_instrument`, `from_ms`, `to_ms`, `recipe`); the
preferred unit of a key is the one with the largest `n`, then the smallest `digest`. Units of
one key that disagree are reported as `SMY-W614`.

**What is read.** A statistic reads the samples of its window `[from_ms, to_ms)` and of its
reference window, and no others. A grid point that would need an earlier sample is missing
(§5.1). So an info unit and the exact `x.tel/window` of the same series, instrument and window
carry the same `digest`, and that equality is how a reader sees they describe the same samples.

## 4. Recipes

A recipe is 3 §21's: an id and version, and parameters by name in name order, written
`id?name=value&…`. Every parameter is written, defaults included, so a recipe states everything
its value was computed under (2.6 §3.4's rule). An unknown parameter is refused.

The recipe text is built as a detector's is (`Recipe::text`), which joins with `?`, `&` and `=`
and escapes nothing. So no parameter value may contain `?`, `&` or `=`, and a recipe that would
is refused (`SMY-E702`). Every value in the table below meets that by its grammar (R1-4).

| Parameter | Recipes | Values | Default |
|---|---|---|---|
| `step` | all but `tent/1` | grid step in ms, a positive integer | **none: required** |
| `hold` | all but `tent/1` | how long a sample stays in force, ms | `step` |
| `of` | all but `tent/1` | `value` or `increase` (§5.2) | `value`; a series of kind `counter` requires `increase` |
| `bins` | all but `perm/1`, `tent/1` | `width:<dec>`, `rank:<k>`, `baseline:<k>`, `diff` (§5.3) | none: required |
| `m`, `tau` | `perm/1` | pattern length 2–7, and delay in steps | 3, 1 |
| `lag` | `mi/1` | one lag in steps, or `-L..L` for a scan | 0 |
| `ref` | `jsd/1`, and with `baseline` bins | `<from_ms>-<to_ms>` | none: required |
| `cover` | all but `tent/1` | the least share of grid points that must give a symbol, as `<a>/<b>` | `4/5` |
| `min_n` | all | the least `n` | §6, per statistic |
| `null`, `seed`, `guard` | `mi/1` | surrogates B (0 to 1000), their seed, the least shift as `<a>/<b>` of the grid length `G`, at most `1/2` (R3-2) | 0, 0, `1/10` |
| `templates` | `tent/1` | the Drain recipe whose templates are read (3 §21), as the lowercase hex of `BLAKE3` over its recipe text: Drain's text holds `?`, `&` and `=` | the hash of Drain's shipped recipe |

A statistic that cannot be given is **not a unit**. A window with too few symbols, too little
coverage or a refused `of` produces a diagnostic and a line in the report, as a 2.6 refusal
does (2.6 §3.1, deviation 2).

## 5. From samples to symbols

Every step in this section is exact. It compares `dec` values, subtracts them, divides one by a
`dec` and takes the floor, and counts. No value passes through binary64.

### 5.1 The grid

- The grid of a window is `{ t : from_ms ≤ t < to_ms, t mod step = 0 }`, in ascending order.
- The **sample in force at `t`** is the last sample, in sample order (3 §9.1), whose `ts_ms`
  lies in `(t − hold, t]` and in the window, whatever its flags. The **value at `t`** is its
  value. `t` is **missing** when there is no such sample, or when it carries flag 1 (stale) or
  flag 4 (no recorded value) (3 §9.3): a marker ends the value before it, as it does in
  `Substrate::at`, which returns the marker and leaves its meaning to the caller (R1-1).
- Two samples at one instant with different values (`SMY-W615`) are both in the digest, and the
  later in sample order is the one in force. The unit is still written.
- **Coverage** is the share of grid points that give a symbol. Below `cover`, the statistic is
  refused (`SMY-W703`).

### 5.2 `of`

- `value`: the value at `t`.
- `increase`: the value at `t` minus the value at `t − step`, exactly. It is missing when either
  is missing, when the difference is negative, or when a sample with flag 2 (reset) lies in
  `(t − step, t]`.
- Admission is by the leaf class (3 §4.3) the substrate records for the series, `SeriesRow::class`
  (D-I12, R2-1):

  | class | Admitted |
  |---|---|
  | `gauge`, or none (a series first seen as a stale marker) | as `value` or `increase` |
  | `counter` | as `increase` only. The level of a counter is the time since it started, which is not what anyone means to measure. |
  | `histogram`, `config`, `identity`, `membership`, `event`, `log` | no. A statistic of their samples is not a statistic of what they measure (`tent/1` reads a log stream's templates, not its samples). |

  A series read against this table is refused with `SMY-E701`. A state, a code held as a gauge,
  is read with `bins=width:1`, which makes each code its own symbol.
- `increase` of a gauge is missing wherever the gauge falls, by the rule above. It is meant for
  counters; `diff` is the binning that reads a gauge's direction.

### 5.3 Binnings

| `bins` | Symbol of a value `x` | Admitted by |
|---|---|---|
| `width:w` | `floor(x / w)`, an integer; `w > 0` | `ent/1`, `mi/1`, `jsd/1` |
| `rank:k` | `floor(k · L(x) / n)`, where `L(x)` is the number of values in the window strictly less than `x` and `n` the number present. Tied values share a symbol. | `mi/1` only |
| `baseline:k` | the number of edges strictly less than `x`. The edges are the reference window's values at sorted positions `ceil(i · m / k) − 1`, `i = 1 … k − 1`, `m` the number present there. | `ent/1`, `jsd/1`. Not `mi/1` (R3-1): two series would need two reference digests. |
| `diff` | the sign of `x` at `t` minus `x` at `t − step`, `x` the sequence `of` produces (R2-2): −1, 0 or 1; missing when either is missing | `ent/1`, `mi/1`, `jsd/1` |

- `rank:k` makes every marginal near uniform by construction, so an entropy under it is
  `log₂ k` whatever the series does, and it is refused there (`SMY-E702`). It is the right
  binning for `mi/1`, which it makes invariant under any monotone rescaling of either series.
- For `mi/1`, ranks are taken over the points where **both** series are present at the lag.
- `k` is 2 to 64.

### 5.4 Ordinal patterns

For `perm/1`, the symbol at `t` is the order of the `m` values at `t, t + τ·step, …,
t + (m − 1)·τ·step`: the permutation that sorts them ascending, equal values keeping time order,
written as its index in lexicographic order, 0 to `m! − 1`. It is missing when any of the `m`
is missing or falls outside the window.

## 6. The statistics

Counts are integers. Every sum below runs over symbols in ascending order (pairs in
lexicographic order), accumulates in binary64 by a left fold, and takes `log₂` and `ln` from
`info::num`. `N` is `n`.

### 6.0 Storing a result

Each statistic, `bias` and `mm` included, passes through these steps in order (R1-3, R1-5):

1. **One symbol.** An entropy over one observed symbol is exactly 0, and a mutual information
   where either side has one observed symbol is exactly 0. Neither is computed.
2. **Clamp.** Every statistic here is non-negative in exact arithmetic, and binary64 can still
   give −1e-17 for an independent pair or two equal windows. A negative `value` (or element of
   `values`) is replaced by 0. `bias` and `mm` are never negative and are not clamped.
3. **Round** to six decimal places, half to even (D-I13), and write the result as a `dec` by
   3 §5.3. Rounding the binary64 to a decimal is exact: it reads the value's full binary
   expansion, not a formatted string.

A quote of a stored value under rule TQ is its six-place canonical text, such as `0.831294`.

### 6.1 `ent/1` and `perm/1`

- `value = log₂ N − (1/N) · Σ c · log₂ c`, over the counts `c` of the observed symbols.
- `mm = value + (K − 1) / (2 · N · ln 2)`, `K` the observed symbols (Miller 1955), as T1 stores
  both (2.6 §3.7).
- `k = K`.
- `min_n` defaults to 200 for `ent/1` and to `10 · m!` for `perm/1`.

### 6.2 `jsd/1`

Two windows of one series and instrument: the window and `ref`. Both are symbolised under the
one binning, with `baseline` edges from `ref`.

- `P` and `Q` are the two windows' symbol distributions and `M` their equal mixture.
- `value = H(M) − ½·H(P) − ½·H(Q)`, each `H` as §6.1's plug-in. It lies in `[0, 1]` in exact
  arithmetic, and §6.0's clamp keeps the stored value there.
- `n` is the smaller window's count, and `min_n` (default 200) and `cover` apply to each.
  `k` and `missing` are the window's (R3-6).
- `H(M)` is computed without forming a probability: `M(s) = (c_P(s)·N_Q + c_Q(s)·N_P) /
  (2·N_P·N_Q)`, so `H(M)` is §6.1's plug-in over the integer weights `c_P(s)·N_Q + c_Q(s)·N_P`.
- `rank` bins are refused: ranks within each window would make the two distributions equal.

### 6.3 `mi/1`

Two series over one window, at lag `ℓ` in steps: the pairs are (symbol of `series` at `t`,
symbol of `with` at `t + ℓ·step`), for every `t` where both exist. A positive lag means
`series` leads.

- `value = Σ (c_ab / N) · log₂( c_ab · N / (c_a · c_b) )` over the observed pairs.
- `bias = (K_a − 1)(K_b − 1) / (2 · N · ln 2)`. It is stored and not subtracted: a reader who
  wants the corrected value has both numbers, and a stored difference would hide a negative one.
- `min_n` defaults to `5 · k²` for `rank:k`, and 500 otherwise.
- **Cover** at lag `ℓ` is over the `G − |ℓ|` grid points where a pair can form, and `missing`
  counts those that formed none (R3-3).
- **A scan** (`lag=-L..L`, `L ≤ 64`) computes the value at each lag. `values` holds them,
  stored as §6.0 says; `value` is their maximum, compared on the stored values; `n` is the
  count at `lag`, and `k`, `k_with`, `bias` and `missing` are those at `lag` too (R3-4). Every
  lag MUST meet `min_n` and `cover` on its own pairs. If any lag does
  not, the whole scan is refused (`SMY-W703`) and the report names the lag (R1-7), so a curve
  never holds a value computed from fewer pairs than its recipe admits.
- **Surrogates** (`null=B`, `B ≤ 1000`). Let `G` be the window's grid length. The second
  series' symbol sequence over the **whole grid**, missing points included, is rotated
  circularly by an offset `o`, with `guard·G ≤ o ≤ G − guard·G`, so that the symbol at grid
  index `i` moves to index `(i + o) mod G` and a missing point stays missing where it lands.
  Pairs are then formed by grid position at the lag, exactly as for `value`, over the indices
  where both are present (R1-6). The statistic is recomputed: at the one lag, or at every lag
  of the scan and then maximised, each stored as §6.0 says before it is compared. A rotation
  whose pairs fall below `min_n`, at any lag of a scan (R3-5), counts toward `null_b` and not
  toward `null_ge`. `null_ge` is
  the number of the `B` whose stored statistic is at least `value`.
- **Offsets** are drawn by rejection from ChaCha20 keyed by
  `BLAKE3("smysl/tel-info/1\0" ‖ recipe ‖ seed)`, the seed as eight little-endian bytes, as
  3 §21 keys MinHash.
- Surrogates say how often chance alignment of these two series does as well. They do not say
  that either series affects the other.

### 6.4 `tent/1`

The entropy of a log stream's templates over a window.

- Its inputs are the `x.tel/log-template` units of one stream, instrument, window and Drain
  recipe that share one `digest` (3 §9.7). A template unit's recipe matches when the hash of
  its `tel:recipe` text equals `templates`. The counts are their `count` fields, in ascending
  order of template text.
- `value`, `mm` and `k` as §6.1; `n` is the sum of the counts. `min_n` defaults to 100.
- `digest` is the templates' own.
- It is a function of units, so it stays recomputable after the events are evicted. When the
  template units are not in the store, `tel info` materialises them first, as `tel templates`
  does, and `--yes` commits them before the statistic, with the stream's series unit (R4-3).
- Template units of one window with different digests are separate inputs, one unit each
  (R4-1).
- A metric series is `SMY-E701`. A `templates` hash naming another Drain recipe is read from
  the store only; with none there, `SMY-W703` (R4-2).

## 7. Recomputation and retention

- While the samples are present, the `tel-evidence` pass MAY recompute any `x.tel/info` unit
  from its digest, recipe and windows, and reports a mismatch as `SMY-E604`.
- When they are absent it reports `SMY-W605` and nothing else. The unit's status does not depend
  on the substrate being present (3 §9.5, rule TL).
- A `template-entropy` unit is recomputed from the template units, and reports `SMY-W605` only
  when those are missing and the events are too.
- A unit whose recipe names a version this build does not have cannot be recomputed and is
  reported `SMY-W704`, once per recipe id.

## 8. Investigations and queries

- **Rule TQ** applies unchanged. A hypothesis may quote `value`, `mm`, `bias`, `n`, `k`,
  `null_ge` or `null_b` of an info unit it grounds on, or an element of `values` by index, and
  the quote is verified against the field (`SMY-W606`). TQ's quote form
  `<uid>#<field>[index]=<value>` reads any payload field by name, so no code changes. TQ
  compares canonical text, so a quote gives the stored six places (§6.0): `0.831294`, not
  `0.83`.
- **`x.tel/consulted`** records an info unit an investigator looked at, as it does any unit.
- **`x.tel/during`** MAY join an info unit to an episode or an investigation.
- **The incident dossier** (3 §22.2) lists info units in its evidence section, each with its
  `null_ge` of `null_b` where it has them.
- **`sq/1`** needs no addition: `type x.tel/info` and `observed during` work through the
  filters that exist.

## 9. Diagnostics

| Code | Meaning |
|---|---|
| `SMY-E701` | a statistic was asked of a series whose class does not admit it (§5.2): a `counter` read as `value`, or a class that is not admitted at all |
| `SMY-E702` | a recipe is not admitted: a binning the statistic refuses, an unknown parameter, a parameter out of range, a value holding `?`, `&` or `=`, a missing `step`, `bins` or `ref` |
| `SMY-W703` | not run: fewer than `min_n` symbols, or coverage below `cover`, at the one lag or at any lag of a scan. No unit is written. |
| `SMY-W704` | an info unit's recipe names a version this build does not have; it is kept and not recomputed |

`SMY-E603`, `SMY-E604`, `SMY-W605` and `SMY-W614` extend to `x.tel/info`, as they extended to
spans and templates in TL-P9. The registry goes from 172 to 176.

All four texts still name windows at the baseline, although TL-P9 extended them to spans and
templates (R1-8):
- `SMY-E603` reads "A window unit's status or attestation is not the one rule TW gives its
  basis and fields".
- `SMY-E604` reads "Window evidence recomputed from the substrate does not match the unit".
- `SMY-W605` reads "Window evidence refers to samples the substrate does not hold".
- `SMY-W614` reads "The windows of one window key disagree".

TI-P0 rewords each to name evidence generally ("an evidence unit", "rule TW or TI", "one
evidence key"), keeping each code and its meaning. `SMY-W614`'s state in the incremental check
(`crates/smysl-check/src/incremental.rs`) is keyed by window keys, and TI-P4 adds the info key
beside them.

## 10. Reserved: transfer entropy

`stat: te` is reserved. Until GE-I5 releases it, a writer MUST NOT emit it. GE-I5 has run, and
did not (§15): a common driver makes it name a direction in about nine pairs of ten. A reader preserves
one under rule X, as it does any unknown enum value.

If it is released, `te/1` is the plug-in transfer entropy from `series` to `with` with history
length 1, `T = Σ p(y′, y, x) · log₂( p(y′ | y, x) / p(y′ | y) )` over the same grid and bins,
with §6.3's surrogates. It stays a statistic: `derived`, and never written as a hypothesis by a
recipe.

## 11. Conformance

| Class | Requires |
|---|---|
| **C-Tel-Read** | unchanged. Preserve `x.tel/info` and its fields under rule X. |
| **C-Tel-Info** | reproduce the uids of `fixtures/tel/info/`: each statistic over the fixture series and log stream, with and without surrogates, and the refusals |
| **C-Tel-Check** | additionally rule TI and §9's codes, on `fixtures/tel/check/` |

---

# Part B — Implementation

## 12. Code

```
smysl-tel/src/info/
  mod.rs       the recipe, the unit writer, the registry of statistics
  num.rs       log2, ln through libm; the left-fold sum
  grid.rs      §5.1, §5.2
  symbols.rs   §5.3, §5.4
  entropy.rs   ent/1, perm/1
  jsd.rs       jsd/1
  mi.rs        mi/1, scans, surrogates
  logs.rs      tent/1
  verify.rs    §7's recomputation, for the tel-evidence pass
```

- **`libm`** becomes a workspace dependency at `=0.2.16`, `default-features = false`, used by
  `smysl-measure` and `smysl-tel`. A `clippy.toml` in `smysl-tel` disallows the `f64` methods
  that `smysl-measure`'s does. The `detect` module's doc comment keeps its own stricter rule.
- **Counts live in `BTreeMap`s.** The crate's `HashMap` ban stands.
- **`Decimal` arithmetic.** §5 needs exact comparison, subtraction and `floor(x / w)`.
  `smysl_core::Decimal` has none of them as methods at the baseline, and deliberately derives
  no order. The exact arithmetic that exists is private to window summaries
  (`crates/smysl-tel/src/evidence.rs`, `BigDec` and `cmp_mag`). TI-P0 lifts what §5 needs into
  one place both can call, and adds the floor of a quotient.
- **`check`.** `tel-evidence` gains the info recomputation (§7), in the incremental state AC-8
  keeps (3 §17.1): an info unit's check depends on its own record and on the substrate, as a
  window's does.
- **Four platforms.** At the baseline only the `measure-platforms` CI job runs on all four
  targets (x86_64 Linux gnu and musl, aarch64 macOS, x86_64 Windows), and its `smysl-tel` step
  runs only `--test ge_m3 --test window_cases`. TI-P1 adds the info fixture test to that step,
  which is what "the four CI platforms" means in §14 and GE-I1 (R1-10).
- **Rounding.** §6.0's six places are taken from the binary64's exact value, as Rust's
  `format!("{:.6}")` does (round half to even on the exact expansion), and then parsed as a
  `dec`. GE-I1 pins the cases near a rounding boundary.
- **Cost.** One statistic is O(N log N) in the grid length. A scan with surrogates is
  O(B · L · N): 200 surrogates over 21 lags of a day at one-minute steps is about 6 million
  pair counts. **Measured at TI-P2** (`examples/ge_i2b.rs`, aarch64 macOS, release): that unit
  takes 170 to 185 ms under `rank:4` and 195 to 215 ms under `width:0.25` over two runs.

## 13. CLI

```
smysl tel info RECIPE --series S [--with S2] --during FROM-TO
               [--instrument I] [--with-instrument I2] [--at MS --yes]
```

`tel` takes its action and arguments as positionals, so `info` joins the action list beside
`detect`, and `RECIPE` is its one argument (R1-10). `--series`, `--during`, `--instrument`,
`--at` and `--yes` already exist on `tel`; `--with` and `--with-instrument` are new. `--yes`
needs `--at`, the attestation clock, as it does for `tel detect`.

- The recipe file is HJSON, as a detector's: `{ stat: mi/1, step: 60000, bins: "rank:4",
  lag: "-10..10", null: 200 }`.
- Units are staged without `--yes` and committed with it, as `tel detect` does.
- `mi/1` pairs each `--series` with `--with`, once per instrument of each, or for
  `--instrument` and `--with-instrument` alone. `--with` given to a statistic of one series, or
  missing from `mi/1`, is exit 2.
- The report prints each value with its `n`, its bias or Miller–Madow term, and `null_ge` of
  `null_b`, and lists every refusal.
- `--seed-check`, a global flag at the baseline, computes twice and compares payload bytes.
- Purity: pure.

The manual's telemetry chapter gains a section, and `examples/information_theory` gains nothing:
it is SMYSL-2's example. A telemetry example over `fixtures/tel/detect/` is TI-P4's.

---

# Part C — Delivery

## 14. Phases

| Phase | Content | Exit criteria |
|---|---|---|
| **TI-P0** prerequisites (**done**, 1.18) | `libm` as a workspace pin; `info::num`; the exact decimal operations §5 needs (§12); `x.tel/v1` revision 2 with `x.tel/info` registered; §9's codes, and the four extended codes reworded; the format spec's §5.2 list | the three ports round-trip a fixture holding an `x.tel/info` unit, byte for byte; purity passes |
| **TI-P1** symbols and one series (**done**, 1.18) | grid, `of`, the four binnings, ordinal patterns; `ent/1`, `perm/1`; `tel info`; the info test in `measure-platforms` | `fixtures/tel/info/` uids stable on the four CI platforms; **GE-I1** harness runs; **GE-I2 (a)** |
| **TI-P2** two series and two windows (**done**, 1.18) | `mi/1` with lag, scan and surrogates; `jsd/1` | **GE-I1** over them; **GE-I2 (b)**; scan cost measured |
| **TI-P3** logs (**done**, 1.18) | `tent/1` | **GE-I4** run |
| **TI-P4** check and investigations (**done**, 1.18) | rule TI in `check`; `tel-evidence` recomputation; the info key in `SMY-W614`'s incremental state; tests of TQ over info fields (no TQ code changes, §8); the dossier's evidence section; the manual section and the example | the `oko_ledger` example quotes an info value and a planted wrong quote is caught; **GE-I3** run |
| **TI-P5** transfer entropy (**closed**, 1.18: not released) | `te/1`, only if released | **GE-I5** decides: it did not release it |

TI-P3 needs TI-P1 and not TI-P2. GE-I3 needs TI-P2.

## 15. Experiments

Each has its stop condition stated before it runs. Data is simulated unless a row says
otherwise, and each report says so.

| Id | Question | Design | Decides | Stop / pivot |
|---|---|---|---|---|
| **GE-I1** | Can every statistic be replayed? | Each recipe over `fixtures/tel/info/`, on the four CI platforms, twice each, as GE-M3 does for detectors | whether a statistic ships | Any byte difference keeps that statistic off until fixed |
| **GE-I2** | Is plug-in good enough at telemetry sizes? | **(a)** i.i.d. sources of known entropy over 2 to 64 symbols, and ordinal patterns of white noise, at `n` from 100 to 10⁵. **(b)** Pairs of AR(1) series with known correlation ρ, where the Gaussian mutual information −½·log₂(1 − ρ²) is an upper bound for the binned one; and independent AR(1) pairs with autocorrelation up to 0.99. | D-I6, D-I7, the `min_n` defaults | (a) If `mm` is more than 5 % off at the default `min_n`, the default rises until it is not. (b) If, for independent pairs, `null_ge / null_b ≤ 0.05` occurs in more than 10 % of runs, circular-shift surrogates are not a usable null and D-I7 is redesigned before TI-P2 ships. |
| **GE-I3** | Does this find anything the shipped detectors miss? | GE-M6's simulated faults, extended with three kinds no level detector sees: a variance change at constant median, a change from noise to oscillation, and two series that decouple; and for `tent/1`, a change of log mix that keeps the entropy level, which GE-I4 found (R4-4). Arms: CUSUM, PELT and MAD as shipped; `jsd/1` and `perm/1` against a reference window; `mi/1` before and after. Also on RCAEval (CC BY 4.0), labelled real. | whether these are worth an investigator's attention | If no statistic separates faulty from normal windows on any of the three kinds at the false-alarm rate MAD has on the same data, the statistics stay as described here, a tool for study, and no later RFC proposes a detector from them. |
| **GE-I4** | Is template entropy a property of the logs, or of Drain? | One simulated log stream, normal and faulty. Compute `tent/1` over windows whose boundaries are shifted by a tenth of their length. | `tent/1` | If shifting a boundary moves the value more than the median difference between normal and faulty windows, `tent/1` does not ship and TI-P3 closes with that result. |
| **GE-I5** | Does transfer entropy name the right direction? | Simulated pairs with a known direction and delay; pairs with a common driver and no direct effect; pairs with none. | §10 | Released only if the direction is right in at least 90 % of driven pairs **and** a direction is asserted, at `null_ge / null_b ≤ 0.05`, in at most 10 % of common-driver pairs. Otherwise `te` stays reserved, and the result is recorded. |

**GE-I1, as of TI-P2.** The harness is `crates/smysl-tel/tests/info_cases.rs`: thirteen
recipes over the eight series of `fixtures/tel/info/`, twice in one process and compared byte
for byte, with every unit's uid and numbers, and every refusal's code, pinned in `expected.tsv`
(written on aarch64 macOS). TI-P1's seven are `ent/1` under width, baseline, diff and increase,
`perm/1` at m = 3 and at m = 4, τ = 2, and a strict cover; TI-P2 adds `mi/1` against
`info_driver` at lag 0 under `rank:4`, a scan of `-5..5` with 50 surrogates, one lag of −3
under `width:0.5`, and `diff` with 20 surrogates, and `jsd/1` under width and baseline bins
against the first two hours. The `measure-platforms` CI job runs it on x86_64 Linux (glibc and
musl), aarch64 macOS and x86_64 Windows, and TI-P1's seven passed there unchanged (run
44892919657), as did TI-P2's (run 44928415339). TI-P3 adds `tent/1` over the generated
`journal.json` (`cart.service`, five events a minute for six hours, a timeout template from
hour four) in three windows, over a metric series (`SMY-E701`), and under another Drain's
hash (`SMY-W703`): 1.828088 bits over the first four hours and 2.201365 over the last two, as
the fifth template appears. A further test recomputes the six-hour unit from its template units
alone, against a substrate holding no event, and gets the same uid, which is §6.4's claim. TI-P4
adds §7 over the same units: every one recomputes to itself; a sample arriving late in
`info_osc`'s window makes exactly the units that read it `SMY-E604` on their digest; with the
substrate empty every unit is `SMY-W605` except the template entropies, which recompute from
their templates; and a recipe id this build does not have is one `SMY-W704` for its units.

What the fixture shows, and one thing it was not built to: `info_follower`, the driver three
minutes late, peaks at lag −3 with 1.495574 bits and no surrogate as high; `info_shift`'s
divergence from its first two hours is 0.194237 bits under `width:1`, against 0.0 to 0.026 for
the steady series. `info_noise`, generated independently of the driver, also peaks at −3, at
0.05783 bits with 0 of 50 surrogates as high. A Python reimplementation of §5.3 and §6.3 gives
the same eleven values to the digit, and the two generators' raw streams show no dependence at
any lag over 2·10⁵ draws, so it is a chance alignment: 300 further rotations all fall below it,
p < 1/300 for one pair and one of six pairs in the fixture. `null_ge` counts chance for one
test; it does not correct for running several, which is the second risk of §17.

**GE-I2 (a), run at TI-P1 on simulated data** (`cargo run --release -p smysl-tel --example
ge_i2`, 200 runs a cell, seed 7). The mean Miller–Madow value against the truth:

| Source | n = 100 | n = 200 | n = 1000 | n = 10⁵ |
|---|---|---|---|---|
| uniform, K = 2 / 16 / 64 | −0.07 / −0.14 / −2.83 % | 0.05 / 0.04 / −0.59 % | 0.00 / 0.00 / 0.00 % | 0.00 % |
| geometric K = 8; Zipf K = 16 | −0.31; −0.59 % | −0.71; −0.37 % | 0.06; −0.04 % | 0.03; 0.00 % |
| ordinal patterns, m = 3 / 5 / 7 | 0.35 / −6.71 / −40.36 % | 0.18 / −1.80 / −32.42 % | 0.04 / 0.01 / −15.18 % | 0.00 % |

At the defaults, `ent/1`'s 200 is at worst 0.71 % off (geometric, K = 8) and `perm/1`'s 10·m!
at worst 0.81 % (m = 3, n = 60), under the 5 % stop condition, so **every default holds and §6.1
stands**. The plug-in value alone is up to 4.2 % low at n = 200 (K = 64), which is why `mm` is
stored. The criterion is the mean over runs; the spread of one run is not what it judges.
Ordinal patterns of m ≥ 5 are far off below 10·m!, which is why that default scales with m!.

**GE-I2 (b), run at TI-P2 on simulated data** (`cargo run --release -p smysl-tel --example
ge_i2b`, 1440 one-minute points, 200 runs a cell, seed 7). Every number goes through
`info::compute`.

*Independent AR(1) pairs, `rank:4`, 100 surrogates: the share of runs with `null_ge / null_b ≤
0.05`.* Exact exchangeability would give 6/101 ≈ 5.9 %.

| φ | lag 0 | scan `-5..5` |
|---|---|---|
| 0 | 7.0 % | 6.0 % |
| 0.5 | 7.0 % | 6.5 % |
| 0.9 | 8.5 % | 5.5 % |
| 0.99 | 7.0 % | 10.0 % |

No cell is over 10 %, so **D-I7 stands** and TI-P2 ships with circular-shift surrogates. The
φ = 0.99 scan is at the limit, not under it, which is where a near-unit-root pair puts it: a
day of minutes holds few independent stretches of such a series, and each rotation is one of
them. A recipe over series that slow should not read a scan's `null_ge` at face value.

*Correlated pairs at lag 0, mean stored `value` against the Gaussian bound −½·log₂(1 − ρ²).*
The true binned value is under the bound, as binning requires; the plug-in `value` is over it
in the two cells ρ = 0.3, `rank:8` (0.0848 and 0.1006 against 0.0680), which is its upward bias
at 64 cells. `value − bias` is under the bound in eleven of twelve cells; the twelfth is
φ = 0.9, ρ = 0.3, `rank:8` (0.1006 − 0.0245 > 0.0680). `bias` is the i.i.d. term, and an autocorrelated pair's bias is larger: the stored
`bias` understates it, which is what the surrogates, keeping each series' autocorrelation, are
for. This is reported, not a stop condition.

**GE-I4, run at TI-P3 on simulated data** (`cargo run --release -p smysl-tel --example
ge_i4`, seven days, about ten messages a minute from eight shapes, a 90-minute fault every six
hours alternating two kinds; 30-minute windows, each shifted three minutes both ways; a window
counts only when it and both shifts lie in one regime). The reading of the stop condition was
fixed before the run: the median move of a shift against each fault kind's median difference
from normal. Twenty seeds:

| | median move | `errors` fault: difference | `mix` fault: difference | stop condition |
|---|---|---|---|---|
| range over 20 seeds | 0.0118 to 0.0138 bits | 0.516 to 0.573 bits | 0.0098 to 0.0503 bits | holds on 18; fails for `mix` on seeds 5 and 8 |

- **Drain's share of the move is zero.** A control computes the same plug-in over the shapes the
  simulator wrote each message from; on every seed its median move equals `tent/1`'s to four
  places. Drain recovers the shapes exactly here, and what a boundary moves is which events
  fall inside it. The answer to GE-I4's question is: the logs.
- **The `mix` fault** raises cache misses and retries from a tenth of the messages to half, with
  no new shape, and moves the entropy by about as much as sampling does. No template miner
  could make it visible to `tent/1`; whether a statistic sees a fault at all is GE-I3's
  question, which inherits the case.
- **Decision (R4-4):** the literal stop condition fails on 2 of 20 seeds, for that kind only.
  The author decided that `tent/1` ships: the failure measures the fault, not Drain. It is
  recorded here so that the decision, and what it rests on, can be read and reversed.
- Real logs carry what this simulation does not: shapes Drain splits or merges, and messages
  whose variable fields its masks miss. GE-I4 says nothing about those, and a run on real logs
  could overturn it.

**GE-I3, run at TI-P4 on simulated data** (`cargo run --release -p smysl-tel --example
ge_i3`, 400 incidents of each kind, seeds 1 to 10). An incident is a two-hour reference window
and a one-hour test window at one sample a minute, Gaussian noise about a level. The faults
are GE-M6's level shift (3 to 10 σ), a variance change (σ × 2 to 4, the median unmoved), noise
become a sine of period 8 to 20 minutes of about the same variance, a pair at correlation 0.9
that decouples, and GE-I4's log mix permuted among eight templates. The detectors run as
shipped; a statistic is held, by its threshold, to MAD's false-alarm rate on the kind's normal
incidents. The reading was fixed before the run: a statistic **separates** a kind when it then
flags at least half of it.

| kind | MAD's false alarms | MAD | CUSUM | PELT | `jsd/1` | `perm/1` | `mi/1` | `tent/1` |
|---|---|---|---|---|---|---|---|---|
| level | 5.0 to 7.8 % | 100 % | 74.5 to 79.2 % | 98.8 to 99.8 % | 100 % | 4.8 to 11.0 % | | |
| variance | as above | 99.5 to 100 % | 88.8 to 93.0 % | 92.0 to 95.5 % | 93.0 to 97.8 % | 5.2 to 10.5 % | | |
| oscillation | as above | 0 % | 34.0 to 42.8 % | 51.5 to 60.5 % | 19.5 to 29.5 % | 89.5 to 94.0 % | | |
| decouple | 7.5 to 11.0 % | 9.2 to 14.0 % | 43.0 to 49.2 % | 4.5 to 8.5 % | 7.0 to 12.2 % | 6.0 to 13.0 % | 100 % | |
| mix | (single series') | | | | | | | 2.5 to 12.5 % |

A detector's column is at its own false-alarm rate on normal incidents: MAD's above; CUSUM's
24.8 to 30.2 % on one series and 38.0 to 48.2 % on a pair (either flagged); PELT's 2.2 to
6.2 % and 3.8 to 7.0 %.

- **The stop condition does not hold.** On every seed a statistic separates each of the three
  kinds: variance by `jsd/1`, oscillation by `perm/1`, decoupling by `mi/1`. The statistics
  are not confined to study, and a later RFC may propose a detector from them; §16's non-goal
  stands for this one.
- **Decoupling is the clear case.** `mi/1`'s fall flags every decoupled pair, and no shipped
  detector flags more of them than of coupled ones: each series' distribution is unmoved.
- **Oscillation is half seen already.** MAD sees none of it, but PELT, at a false-alarm rate
  under MAD's, finds a change in about 56 % of incidents, the sine's slow half-periods reading
  as shifts of level. `perm/1` finds about 92 %.
- **Variance is not a kind the shipped detectors miss** at these sizes. A spread doubled or
  more puts points past MAD's 3.5, and MAD flags all of it; `jsd/1`'s 95 % adds nothing. The
  draft's "three kinds no level detector sees" is two here. A smaller change of spread is
  where the two would differ, and this run does not ask it.
- **`tent/1` does not see the permuted mix**, as GE-I4 found and as it cannot by
  construction: the mix changes and its entropy does not.
- **CUSUM at its defaults** raises a false alarm on about 27 % of normal one-hour windows here,
  its ten-point baseline estimating the level from noise. That is a finding about `cusum/1`
  under this setting, reported and not acted on.
- The noise is independent from minute to minute. Real series are autocorrelated, which moves
  `perm/1`'s value on normal windows and, as GE-I2 (b) found, weakens what surrogates say. The
  run on RCAEval, below, is the real-data arm.

**GE-I3 on real incidents, run in 1.19 on RCAEval RE2-SS** (R7; `cargo run --release -p
smysl-tel --example ge_i3_rcaeval -- --data PATH/RE2-SS`). RCAEval (Pham et al., arXiv
2412.17015; Zenodo 14590730, CC BY 4.0) is not in this repository. RE2-SS is Sock Shop: 90
cases, six fault types injected into five services three times each, 24 minutes a case at one
sample a second, the fault at minute 12. Each case gives a **normal** pair of six-minute
windows, both before the injection, and a **faulty** pair, reference before and test after it.
The series are the faulty service's columns of `simple_metrics.csv` (up to seven) and its
container's log lines, through the shipped Drain, for `tent/1`; a pair's score is the largest
over its series, or its pairs for `mi/1`. Every arm is scored, the detectors by their own code:
MAD's largest |robust z| in the test window, and the largest CUSUM `h` and PELT `penalty` on a
grid of 10^(i/8), i = −8 to 160, at which a change point falls in it.

The reading was fixed twice before a run. The first held the statistics to MAD's false-alarm
rate with the detectors at their shipped defaults, as the simulated run does, and was
degenerate: on one-second microservice metrics the defaults flag the normal pairs too (MAD
90.0 %, CUSUM and PELT 100 %), so every type was seen and every threshold sat at the bottom.
The amended reading holds **every arm to 10 % false alarms** on the 90 normal pairs. A type is
**unseen** when MAD, CUSUM and PELT each flag under half of it; a statistic **separates** it when
it flags at least half. (The first rerun's grid stopped at 10⁶, under what metrics in bytes
need, and CUSUM and PELT scored nothing; it was widened, which cannot change the verdict, since
MAD alone flags 93 % or more of every type.)

| fault (15 each) | MAD | CUSUM | PELT | `jsd/1` | `perm/1` | `mi/1` | `tent/1` |
|---|---|---|---|---|---|---|---|
| cpu | 100 % | 93.3 % | 100 % | 86.7 % | 0 % | 6.7 % | 0 % |
| mem | 100 % | 93.3 % | 100 % | 93.3 % | 20.0 % | 33.3 % | 13.3 % |
| disk | 100 % | 86.7 % | 100 % | 86.7 % | 53.3 % | 6.7 % | 26.7 % |
| delay | 93.3 % | 80.0 % | 80.0 % | 80.0 % | 6.7 % | 6.7 % | 13.3 % |
| loss | 100 % | 93.3 % | 100 % | 60.0 % | 33.3 % | 13.3 % | 53.3 % |
| socket | 93.3 % | 93.3 % | 100 % | 80.0 % | 0 % | 20.0 % | 20.0 % |

Normal pairs over each threshold: MAD, the four statistics 9 of 90; CUSUM 6, PELT 7.

- **No type is unseen, and no statistic adds a detection.** RCAEval's faults are resource
  and network faults, and every one moves a level: MAD at 10 % flags 93 % or more of each type.
  No faulty pair is flagged by a statistic and by no detector. `jsd/1` is the strongest
  statistic, at 60 to 93 %, and below MAD on every type.
- **This does not overturn the simulated verdict.** RCAEval labels none of the three kinds it
  rests on: variance at a constant median, oscillation, decoupling. What it shows is that on
  the faults a benchmark injects the statistics are not needed to see them, so a detector RFC
  built on them needs real evidence of those kinds, which GE-M7's incidents may give and
  RCAEval cannot.
- **The detectors' defaults are minute-data defaults.** At one sample a second over about seven
  series, `mad/1` at 3.5 flags 90 % of fault-free windows, and `cusum/1` and `pelt/1` all of
  them. Acted on in R8: the defaults stay, and calibration (3 §21.1) states a rate instead.
- **`mi/1` mostly refused** (362 of its pair computations), most of them pairs with a constant
  series (sockets, disk I/O of an idle service), which limits what it could show here.
- One system of three. Online Boutique (RE2-OB) is replicated in R8; Train-Ticket (RE2-TT) is
  owed only if a reading turns on it.

**Calibration, and GE-I3 on RE2-OB, run in 1.19** (R8; `… --example ge_i3_rcaeval -- --data
PATH/RE2-SS --held PATH/RE2-OB`). The harness now sets the detectors with `detect::calibrate`
(3 §21.1). Each case's series form one trial, and each detector is set on RE2-SS's 90 normal
pairs to at most 10 % of them firing. The statistics keep R7's thresholds. RE2-OB is Online
Boutique, 90 cases of the same six fault types in five other services, in the same layout. It
is **held out**: RE2-SS's values run on it unchanged, which is what a calibrated recipe is for,
and it is then read at its own calibration, as R7 read RE2-SS. The reading is R7's, fixed before
this run, plus one question asked beforehand: does a value calibrated on one system hold its rate
on another?

| | MAD threshold | CUSUM `h` | PELT `penalty` | false alarms, RE2-OB normal |
|---|---|---|---|---|
| shipped default | 3.5 | 5 | 10 | 96.7 % / 100 % / 100 % |
| calibrated on RE2-SS | 40 | 1.5·10¹⁰ | 4·10¹⁹ | 4.4 % / 2.2 % / 1.1 % |
| calibrated on RE2-OB | 20 | 2.5·10⁹ | 8·10¹⁸ | ≤ 10 % each |

| RE2-OB fault (15 each) | MAD, SS's / own | CUSUM, SS's / own | PELT, SS's / own | `jsd/1` | `perm/1` | `mi/1` | `tent/1` |
|---|---|---|---|---|---|---|---|
| cpu | 100 / 100 % | 93.3 / 100 % | 86.7 / 100 % | 93.3 % | 13.3 % | 13.3 % | 20.0 % |
| mem | 100 / 100 % | 93.3 / 93.3 % | 100 / 100 % | 93.3 % | 33.3 % | 20.0 % | 6.7 % |
| disk | 100 / 100 % | 100 / 100 % | 100 / 100 % | 100 % | 66.7 % | 20.0 % | 6.7 % |
| delay | 100 / 100 % | 13.3 / 20.0 % | 26.7 / 93.3 % | 33.3 % | 33.3 % | 20.0 % | 13.3 % |
| loss | 100 / 100 % | 73.3 / 73.3 % | 100 / 100 % | 46.7 % | 73.3 % | 20.0 % | 26.7 % |
| socket | 100 / 100 % | 100 / 100 % | 100 / 100 % | 100 % | 20.0 % | 6.7 % | 6.7 % |

On RE2-SS, calibration gives R7's table within two pairs a cell (CUSUM's cpu and loss at
86.7 %, R7's grid had 93.3 %). Normal pairs over the calibrated values: MAD 8, CUSUM 9, PELT 7.

- **A calibrated value carried to the second system, conservatively.** RE2-SS's values alarm on
  1 to 4 % of RE2-OB's fault-free windows, under the 10 % they were set for. They still see
  every fault type at 73 % or more, except delay under CUSUM (13.3 %) and PELT (26.7 %). RE2-OB's
  own calibration recovers PELT's delays (93.3 %) but not CUSUM's (20.0 %). MAD sees all of
  RE2-OB's faults at either value. This is one transfer, between two systems of one benchmark:
  it shows that a calibrated value can carry, not that it will.
- **The values are large because some series barely move.** An idle service's sockets or disk
  I/O are constant over a reference window, or step-like at one sample a second. CUSUM's `σ`
  and PELT's `σ̂` then fall to their floors (`10⁻⁹`, `10⁻¹⁸` of the level), and any change
  scores astronomically. A trial fires when any of its series does, so its score is set by its
  least-moving series, and the calibrated value rises until those series are quiet. The busy
  series are then read at a value far above their own noise, which is the likely reason
  CUSUM misses delays. Calibrating per series rather than per trial is R9, below; flooring the
  scale at the data's resolution is recorded, not done.
- **R7's verdict replicates.** On RE2-OB at its own calibration no type is unseen (MAD flags
  every one), no faulty pair is flagged by a statistic and by no detector, and no statistic
  separates a type. `jsd/1` is again the strongest statistic (33 to 100 %). The simulated verdict
  still rests on simulated kinds alone.
- **Refused:** `mi/1` 151 and `tent/1` 42 of RE2-OB's computations, the latter windows with
  no log line from the service. The downloads (RE2-SS 246 MB and RE2-OB 1.2 GB zipped) are
  not kept.

**Per-series calibration, run in 1.19** (R9; the same command). Each detector is also
calibrated per series by `calibrate::each_from_scores` (3 §21.1). The rate is held jointly over
the 90 normal pairs, since a pair still fires when any series fires at its own value. The key is
the **metric kind**: the column with the service's prefix taken off (`cpu`, `mem`, `diskio`,
`socket`, `workload`, `latency-50`, `latency-90`, `error`). Keyed that way, a value carries from
one system's services to another's, which a key per service could not do. RE2-OB has no kind
that RE2-SS lacks. The reading is R8's, with the same question asked beforehand.

| detector | RE2-SS, one value / per kind | RE2-SS's values on RE2-OB normal, one / per kind | RE2-SS's values on RE2-OB delay, one / per kind |
|---|---|---|---|
| MAD | 93–100 % / 93–100 % (normal: 8 / 9 of 90) | 4.4 % / 6.7 % | 100 % / 100 % |
| CUSUM | 80–93 % / **100 %** every type (9 / 6) | 2.2 % / 4.4 % | 13.3 % / **100 %** |
| PELT | 80–100 % / 87–100 % (7 / 6) | 1.1 % / **16.7 %** | 26.7 % / 73.3 % |

Carried per kind to RE2-OB, every other fault type is seen at 100 % by all three, except delay
under PELT. At RE2-OB's own per-kind calibration, all three see every type at 93 % or more, with
4, 9 and 8 of 90 normal pairs firing.

- **Per series is the better calibration where it holds.** CUSUM goes from missing most of
  RE2-OB's delays to seeing all of them, at 4.4 % false alarms, and sees every RE2-SS fault.
  The busy series are no longer read at a value set by the idle ones. Delays move latency,
  which is a busy series, and that is where per-trial calibration lost them.
- **Tighter values carry less margin.** PELT's per-kind values alarm on 16.7 % of RE2-OB's
  fault-free windows, over the 10 % they were set for: a calibration holds on the windows it was
  set on, and per series it is fitted closer to them. A value carried to another system needs
  its rate checked there, or it should be set with margin. One value a trial stayed under the
  rate on transfer, at the cost of the delays.
- **Some kinds still sit at the scale floor.** `cpu` takes CUSUM `h` 8·10¹⁶ and PELT `penalty`
  4·10²⁰ on RE2-SS, because a CPU series that is flat over a reference window still meets the
  `10⁻⁹` floor. Per series contains that to the kind instead of letting it set every value, but
  it does not remove it. Flooring the scale at the data's resolution is R10, below.
- **R7's verdict holds in all four readings** (each system, each calibration). No faulty pair is
  flagged by a statistic and by no detector, and no statistic separates a type.

**The scale floor and a transfer margin, run in 1.19** (R10; the same command). Every R9
calibration (one value a trial or one per metric kind) is made twice: under the shipped floor,
and under `floor: resolution` (3 §21.2), where a detector's scale is at least the smallest gap
between the series' distinct values. Each is made at a margin of 1 and of 2 (3 §21.1: at most
`⌊10 % · 90 / margin⌋` normal pairs, so 9 and 4). All are run on RE2-SS and carried unchanged to
RE2-OB. The question was fixed before the run: does the floor bring the values to sane ranges and
keep the detections, and does a margin keep a carried value under its rate?

| calibration on RE2-SS | CUSUM `h` | RE2-OB false alarms, MAD / CUSUM / PELT | RE2-OB delay, MAD / CUSUM / PELT |
|---|---|---|---|
| tiny floor, one a trial, margin 1 (R8) | 1.5·10¹⁰ | 4.4 / 2.2 / 1.1 % | 100 / 13.3 / 26.7 % |
| resolution floor, one a trial, margin 1 | 1.2·10⁵ | 5.6 / 3.3 / 1.1 % | 100 / **100** / 13.3 % |
| tiny floor, per kind, margin 1 (R9) | `cpu` 8·10¹⁶ | 6.7 / 4.4 / 16.7 % | 100 / 100 / 73.3 % |
| tiny floor, per kind, margin 2 | `cpu` 5·10¹⁷ | 2.2 / 3.3 / **16.7 %** | 100 / 100 / 73.3 % |
| resolution floor, per kind, margin 1 | `cpu` 6000 | 6.7 / **11.1** / **24.4 %** | 100 / 100 / 80.0 % |
| resolution floor, per kind, margin 2 | `cpu` 8·10¹⁶ | 2.2 / 6.7 / **12.2 %** | 100 / 100 / 33.3 % |

On RE2-SS itself, every calibration holds its rate (at most 9, or 4, of 90). MAD flags 87 to
100 % of every type under all of them, and CUSUM 80 to 100 %. PELT under the floor at one value a
trial drops to 53 % on `loss`. At RE2-OB's own calibrations, every configuration sees every type
at 87 % or more, except CUSUM under the tiny floor at one value a trial (delay 20 %, loss 73 %).

- **The floor fixes CUSUM.** At one value a trial its `h` falls from 1.5·10¹⁰ to 1.2·10⁵. Carried
  to RE2-OB it sees every delay at 3.3 % false alarms, which R8's value missed (13.3 %). That
  was what per-series calibration was needed for in R9. Per kind, `cpu`'s `h` falls from
  8·10¹⁶ to 6000 at margin 1.
- **The floor does nothing for PELT on float metrics.** Its values stay at 10¹⁷ to 10²⁰.
  RCAEval's metrics are floats whose smallest gap between distinct values is noise-sized, so
  their resolution is far under any real scale and the floor never binds. PELT's `σ̂`, from the
  median first difference, is zero for step-like series anyway. The floor helps quantized
  series (counts, rounded values) and not these. That is a limit of the definition, not a bug,
  and it is recorded in 3 §21.2.
- **A margin halves an overshoot that is sampling, not one that is the systems differing.** At
  margin 2, the per-kind overshoots under the floor fall by about half (CUSUM 11.1 → 6.7 %, PELT
  24.4 → 12.2 %). PELT's under the tiny floor stays at 16.7 %: RE2-OB's fault-free windows score
  above anything RE2-SS's did, and no tightening on RE2-SS can see that. Margins cost
  detections: PELT's delays on RE2-OB fall from 80 % to 33 % at margin 2.
- **What to run, from this data.** CUSUM with `floor: resolution` held its rate when carried, at
  one value a trial (3.3 %) or per kind at margin 2 (6.7 %), and saw at least 86.7 % of every
  type. MAD held its rate in every configuration. PELT's carried values are not to be trusted
  without checking their rate on the new system. Two systems of one benchmark are what this
  rests on.
- **R7's verdict holds.** Under the floor on RE2-OB, one faulty pair is flagged by a statistic and
  by no detector. No statistic separates a type in any configuration on either system.

**A third system, run in 1.19** (R11; `--data PATH/RE2-SS --held PATH/RE2-TT`). R10's advice
rested on two systems of one benchmark. RE2-TT (Train Ticket, Zenodo record 14590730, 90 cases,
five `ts-*-service`s, the same six fault types and metric kinds) tests it: every R10
calibration on RE2-SS is carried unchanged to RE2-TT. The question was fixed before the run:
does a configuration that held its rate on RE2-OB hold it on RE2-TT, and still see the faults?

The first run stopped. On RE2-TT's own data, a `pelt/1` series scored at the ladder's top in a
fault-free window, and per-series calibration refused the whole calibration, although one
value a trial over the same windows did not. That was stricter than 3 §21.1's rule, under which
only a rate the ladder's top cannot meet is refused. Now a series that cannot be brought to the
count takes the top and fires only in those trials. They count against the rate jointly, and the
calibration is refused only when more of them fire than the rate allows (R11-1).

| calibration on RE2-SS | RE2-OB false alarms, MAD / CUSUM / PELT (R10) | RE2-TT false alarms, MAD / CUSUM / PELT |
|---|---|---|
| tiny floor, one a trial, margin 1 | 4.4 / 2.2 / 1.1 % | 13.3 / 15.6 / 12.2 % |
| tiny floor, one a trial, margin 2 | | 8.9 / 6.7 / 2.2 % |
| resolution floor, one a trial, margin 1 | 5.6 / **3.3** / 1.1 % | 15.6 / **6.7** / 12.2 % |
| resolution floor, one a trial, margin 2 | | 7.8 / **6.7** / 2.2 % |
| tiny floor, per kind, margin 1 | 6.7 / 4.4 / 16.7 % | 11.1 / 2.2 / **100** % |
| tiny floor, per kind, margin 2 | 2.2 / 3.3 / 16.7 % | 10.0 / 1.1 / 15.6 % |
| resolution floor, per kind, margin 1 | 6.7 / 11.1 / 24.4 % | 11.1 / **23.3** / **100** % |
| resolution floor, per kind, margin 2 | 2.2 / **6.7** / 12.2 % | 10.0 / **12.2** / 30.0 % |

On RE2-TT at its own calibrations every arm holds its rate. MAD and PELT see `cpu`, `mem`,
`disk` and `socket` faults in full. `loss` is the hardest type: 40 to 100 % depending on the
detector, and 67 % for CUSUM under the floor at one value a trial.

- **One configuration held on both systems: CUSUM, `floor: resolution`, one value a trial.**
  Carried, it alarms on 3.3 % of RE2-OB's fault-free windows and 6.7 % of RE2-TT's, at either
  margin. On RE2-TT it sees `cpu` 87 %, `mem`, `disk` and `socket` 100 %, `delay` 80 % and
  `loss` 67 %. RE2-TT's own calibration of the same configuration sees `loss` at 67 % too, so
  the transfer costs nothing there.
- **Per kind does not carry reliably.** CUSUM per kind under the floor, R10's other candidate
  (6.7 % on RE2-OB at margin 2), overshoots on RE2-TT: 23.3 % at margin 1 and 12.2 % at margin 2.
  Under the shipped floor it holds on RE2-TT (2.2 and 1.1 %) after overshooting nowhere on RE2-OB,
  but its `cpu` value there is `h` 8·10¹⁶. PELT per kind fires on **every** RE2-TT fault-free
  window at margin 1, and on 15.6 to 30 % at margin 2. Per-kind values are fitted closely to
  the calibrating system's kinds, and how a kind behaves differs between systems.
- **A margin of 2 brings MAD under its rate.** At margin 1, MAD's carried values overshoot on
  RE2-TT (11.1 to 15.6 %), so R10's "MAD held in every configuration" does not replicate. At
  margin 2 every MAD configuration holds (7.8 to 10.0 %) and sees every type at 73 % or more.
  PELT at one value a trial holds at margin 2 too (2.2 %), at the cost of `delay` (53 to 67 %)
  and `loss` (27 %).
- **What to run, amended.** For a value carried to a system it was not calibrated on:
  - CUSUM with `floor: resolution` at one value a trial;
  - MAD at margin 2;
  - per kind, only with its rate checked on the new system.

  This rests on three systems of one benchmark, at one sample a second.
- **R7's verdict holds on RE2-TT.** In every configuration, no faulty pair is flagged by a
  statistic and by no detector, and no statistic separates a type. `jsd/1` is the strongest, at
  13 to 67 % per type.

**GE-I5, run at TI-P5 on simulated data** (`cargo run --release -p smysl-tel --example
ge_i5`, 300 pairs of each kind, seeds 1 to 5). The estimator is §10's, written in the example
and not in the library: the plug-in transfer entropy with history length 1 over `rank:4`
symbols, scanned over source lags 1 to 5 and maximised, with 200 surrogates rotating the
source by §6.3's offsets. A pair is a day of one-minute AR(1) points (φ = 0.5): **driven**, the
target reading the source at a delay of 1 to 5 minutes with coupling 0.3 to 0.8, the pair's
order drawn; **common driver**, one AR(1) driving both at delays of 0 to 5 minutes and neither
affecting the other; **none**, independent. The reading was fixed before the run: each way is
significant at `null_ge / null_b ≤ 0.05`; a direction is asserted when one way is, or both are,
then the larger; a driven pair is right when the asserted direction is the true one.

| | seed 1 | 2 | 3 | 4 | 5 |
|---|---|---|---|---|---|
| driven: right | 100 % | 100 % | 100 % | 100 % | 100 % |
| common driver: a direction asserted | 90.7 % | 88.0 % | 91.0 % | 92.3 % | 90.0 % |
| of those with equal delays | 55.1 % | 47.4 % | 62.3 % | 64.0 % | 50.0 % |
| none: a direction asserted | 12.0 % | 10.3 % | 11.3 % | 10.3 % | 13.7 % |

- **`te/1` is not released.** It names the true direction of every driven pair, at every delay,
  and none the wrong way; and it names a direction in about nine common-driver pairs of ten,
  against a bound of one. `te` stays reserved, and TI-P5 closes with this result and no code
  in the library.
- **The reading does not decide it.** Both ways are significant in 6.0 to 9.7 % of
  common-driver pairs; read as no direction, 80.3 to 85.0 % still assert one.
- **What makes it fail is the confound D-I9 named.** The series the driver reaches first
  carries the driver's past, so its past predicts the other's future, and history length 1
  cannot condition that away. Equal delays halve the rate and do not remove it: each series'
  own noise makes one the better proxy. Conditioning on a third series, or on longer histories,
  is a different statistic, causal discovery's, which 3 §21 and §16 leave out.
- Independent pairs assert a direction in 10.3 to 13.7 %: two ways each held to 0.05, each the
  maximum of a scan, as GE-I2 (b) found for `mi/1`'s scans.

**Order.** GE-I1 and GE-I2 first: they are cheap and decide the defaults. GE-I3 is the one that
says whether the work was worth doing. The real-data runs wait on the cluster GE-M1 still
needs (3 §1, I-1).

## 16. Non-goals

- **Causal discovery.** PC, PCMCI and Granger stay where 3 §21 put them.
- **Continuous estimators.** No differential entropy, no KSG, no kernels (D-I6).
- **A detector.** Nothing here writes an anomaly, a segment or a hypothesis. GE-I3 says whether
  that is worth proposing.
- **All pairs.** `tel info` computes the pairs it is given. A fleet's worth of pairwise mutual
  information is a quadratic job and a multiple-comparisons problem, and is not offered.
- **Histograms.** A histogram series is refused (`SMY-E701`). The divergence between two
  histograms' bucket distributions is a natural follow-up and is not in this RFC.
- **Compression measures over series.** An LZ or entropy-rate estimate of a symbol sequence is
  2.6's T2 and T3 in another setting. Not here.

## 17. Risks

| Risk | Where it bites | Mitigation |
|---|---|---|
| A statistic is read as a cause | hypotheses, dossiers | Rule M caps the hypothesis; no recipe writes one; §6.3 says what surrogates do not show; `te` is reserved (D-I9) |
| Parameters are tried until something shows | every recipe | Each unit carries its whole recipe; a scan is one unit with its curve (D-I8); `consulted` records what an investigation looked at. What was staged and never committed leaves no trace: OQ-I2. |
| Plug-in bias at small `n` reads as signal | `mi/1` | `bias` and `null_ge` beside the value; `min_n`; GE-I2 |
| The grid misreads an irregular series | §5.1 | `cover`; `missing` in the unit; `step` has no default |
| `libm` changes a result on upgrade | every value | pinned exactly, one pin for both crates; an upgrade that moves a fixture is a new recipe version, as 2.6 has it |
| Units multiply as agents explore | the store | identity makes a repeat free; a scan is one unit; a refusal is none |
| Every threshold in this draft is a guess | §4, §6, §15 | each is owned by a named experiment, and the draft says so in its header |

## 18. Open questions

- **OQ-I1.** Should `jsd/1` have a chance level? Circular shifts do not apply to two windows of
  one series. Splitting the reference window in two and reporting their divergence beside
  `value` is one candidate. GE-I3's data decides whether a bare value is usable.
- **OQ-I2.** Should `tel info` run inside an investigation always commit, so that the record of
  what was tried is complete? It costs units and answers the second risk above.
- **OQ-I3.** Is adding a unit type a revision of `x.tel/v1`, as §3 assumes, or does 2.3 A-12's
  rule N want it handled another way? TL-P9 added two types before the schema's first release,
  so this is the first addition after one. To be settled in TI-P0 against `smysl schema`'s
  naming check.
  **Decided in TI-P0: a revision.** `tel::decl_at(1)` and `tel::decl()` (revision 2) pass
  `SchemaSet::check_rule_n` as a pair, since revision 2 only appends a type and its keys. A
  store keeps both declarations, and `fixtures/wire/F16-telemetry-info.cbor` holds them.
- **OQ-I4.** Should an info unit ground on the exact `x.tel/window` of its key when one exists,
  rather than on the series? The shared digest already links them, and grounding on the series
  is what segments and anomalies do.
- **OQ-I5.** For `mi/1` across two instruments of one series (two witnesses), is a low value a
  witness conflict that detection kind 7 should see? Not proposed here.

---

## Appendix A — Found while drafting

Two places where 3's text and the baseline differ. Neither blocks this RFC.

- **`SeriesSource`.** 3 §19.4 specifies a `SeriesSource` trait as the AC-10 port. No item of
  that name exists in `crates/` or `src/` at the baseline; detectors and the materialiser read
  `smysl_series::Substrate<B: Backend>` directly. This RFC is written against `Substrate`.
- **What a derived unit grounds on.** 3 §0 says a derived unit grounds on window evidence
  units. `smysl_tel::detect::segment` and `::anomaly` ground on the series
  (`crates/smysl-tel/src/detect/mod.rs`). §3 follows the code, and OQ-I4 asks the question.
