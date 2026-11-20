# RFC SMYSL-5 — information measures over telemetry

**Author:** Vladimir Ulogov
**Status:** first draft, amended after review (R1, 2026-11-20). TI-P0 is built; no statistic is,
and no number in it is measured. Every threshold is a proposal until the experiment that owns it has
run (§17).
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
| D-I12 (R1) | Which series are admitted | **By `tel:kind`** (§5.2): `gauge` and an absent kind as `value`; `counter` only as `increase`; `state` only with `width:1`; every other kind refused. | Admitting by `tel:class` | The draft named a class. `tel:class` is a field of `x.tel/change`; a series carries `tel:kind`, and it is optional. |
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
| `null`, `seed`, `guard` | `mi/1` | surrogates B, their seed, the least shift as `<a>/<b>` of `n` | 0, 0, `1/10` |
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
- Admission is by the series' `tel:kind` (D-I12, R1-2):

  | `tel:kind` | Admitted |
  |---|---|
  | `gauge`, or no kind | as `value` or `increase` |
  | `counter` | as `increase` only. The level of a counter is the time since it started, which is not what anyone means to measure. |
  | `state` | with `bins=width:1` only, which makes each state code its own symbol |
  | `histogram`, `summary`, `text`, `span`, `log`, `event` | no. A statistic of their samples is not a statistic of what they measure (`tent/1` reads a log stream's templates, not its samples). |

  A series read against this table is refused with `SMY-E701`.

### 5.3 Binnings

| `bins` | Symbol of a value `x` | Admitted by |
|---|---|---|
| `width:w` | `floor(x / w)`, an integer; `w > 0` | `ent/1`, `mi/1`, `jsd/1` |
| `rank:k` | `floor(k · L(x) / n)`, where `L(x)` is the number of values in the window strictly less than `x` and `n` the number present. Tied values share a symbol. | `mi/1` only |
| `baseline:k` | the number of edges strictly less than `x`. The edges are the reference window's values at sorted positions `ceil(i · m / k) − 1`, `i = 1 … k − 1`, `m` the number present there. | `ent/1`, `mi/1`, `jsd/1` |
| `diff` | the sign of the value at `t` minus the value at `t − step`: −1, 0 or 1; missing as `increase` is | `ent/1`, `mi/1`, `jsd/1` |

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
- `n` is the smaller window's count, and `min_n` (default 200) applies to each.
- `rank` bins are refused: ranks within each window would make the two distributions equal.

### 6.3 `mi/1`

Two series over one window, at lag `ℓ` in steps: the pairs are (symbol of `series` at `t`,
symbol of `with` at `t + ℓ·step`), for every `t` where both exist. A positive lag means
`series` leads.

- `value = Σ (c_ab / N) · log₂( c_ab · N / (c_a · c_b) )` over the observed pairs.
- `bias = (K_a − 1)(K_b − 1) / (2 · N · ln 2)`. It is stored and not subtracted: a reader who
  wants the corrected value has both numbers, and a stored difference would hide a negative one.
- `min_n` defaults to `5 · k²` for `rank:k`, and 500 otherwise.
- **A scan** (`lag=-L..L`, `L ≤ 64`) computes the value at each lag. `values` holds them,
  stored as §6.0 says; `value` is their maximum, compared on the stored values; `n` is the
  count at `lag`. Every lag MUST meet `min_n` and `cover` on its own pairs. If any lag does
  not, the whole scan is refused (`SMY-W703`) and the report names the lag (R1-7), so a curve
  never holds a value computed from fewer pairs than its recipe admits.
- **Surrogates** (`null=B`, `B ≤ 1000`). Let `G` be the window's grid length. The second
  series' symbol sequence over the **whole grid**, missing points included, is rotated
  circularly by an offset `o`, with `guard·G ≤ o ≤ G − guard·G`, so that the symbol at grid
  index `i` moves to index `(i + o) mod G` and a missing point stays missing where it lands.
  Pairs are then formed by grid position at the lag, exactly as for `value`, over the indices
  where both are present (R1-6). The statistic is recomputed: at the one lag, or at every lag
  of the scan and then maximised, each stored as §6.0 says before it is compared. A rotation
  whose pairs fall below `min_n` counts toward `null_b` and not toward `null_ge`. `null_ge` is
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
  does.

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
| `SMY-E701` | a statistic was asked of a series whose `tel:kind` does not admit it (§5.2): a `counter` read as `value`, a `state` without `width:1`, or a kind that is not admitted at all |
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

`stat: te` is reserved. Until GE-I5 releases it, a writer MUST NOT emit it. A reader preserves
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
  pair counts. TI-P2 measures it.

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
| **TI-P1** symbols and one series | grid, `of`, the four binnings, ordinal patterns; `ent/1`, `perm/1`; `tel info`; the info test in `measure-platforms` | `fixtures/tel/info/` uids stable on the four CI platforms; **GE-I1** harness runs; **GE-I2 (a)** |
| **TI-P2** two series and two windows | `mi/1` with lag, scan and surrogates; `jsd/1` | **GE-I1** over them; **GE-I2 (b)**; scan cost measured |
| **TI-P3** logs | `tent/1` | **GE-I4** run |
| **TI-P4** check and investigations | rule TI in `check`; `tel-evidence` recomputation; the info key in `SMY-W614`'s incremental state; tests of TQ over info fields (no TQ code changes, §8); the dossier's evidence section; the manual section and the example | the `oko_ledger` example quotes an info value and a planted wrong quote is caught; **GE-I3** run |
| **TI-P5** transfer entropy | `te/1`, only if released | **GE-I5** decides |

TI-P3 needs TI-P1 and not TI-P2. GE-I3 needs TI-P2.

## 15. Experiments

Each has its stop condition stated before it runs. Data is simulated unless a row says
otherwise, and each report says so.

| Id | Question | Design | Decides | Stop / pivot |
|---|---|---|---|---|
| **GE-I1** | Can every statistic be replayed? | Each recipe over `fixtures/tel/info/`, on the four CI platforms, twice each, as GE-M3 does for detectors | whether a statistic ships | Any byte difference keeps that statistic off until fixed |
| **GE-I2** | Is plug-in good enough at telemetry sizes? | **(a)** i.i.d. sources of known entropy over 2 to 64 symbols, and ordinal patterns of white noise, at `n` from 100 to 10⁵. **(b)** Pairs of AR(1) series with known correlation ρ, where the Gaussian mutual information −½·log₂(1 − ρ²) is an upper bound for the binned one; and independent AR(1) pairs with autocorrelation up to 0.99. | D-I6, D-I7, the `min_n` defaults | (a) If `mm` is more than 5 % off at the default `min_n`, the default rises until it is not. (b) If, for independent pairs, `null_ge / null_b ≤ 0.05` occurs in more than 10 % of runs, circular-shift surrogates are not a usable null and D-I7 is redesigned before TI-P2 ships. |
| **GE-I3** | Does this find anything the shipped detectors miss? | GE-M6's simulated faults, extended with three kinds no level detector sees: a variance change at constant median, a change from noise to oscillation, and two series that decouple. Arms: CUSUM, PELT and MAD as shipped; `jsd/1` and `perm/1` against a reference window; `mi/1` before and after. Also on RCAEval (CC BY 4.0), labelled real. | whether these are worth an investigator's attention | If no statistic separates faulty from normal windows on any of the three kinds at the false-alarm rate MAD has on the same data, the statistics stay as described here, a tool for study, and no later RFC proposes a detector from them. |
| **GE-I4** | Is template entropy a property of the logs, or of Drain? | One simulated log stream, normal and faulty. Compute `tent/1` over windows whose boundaries are shifted by a tenth of their length. | `tent/1` | If shifting a boundary moves the value more than the median difference between normal and faulty windows, `tent/1` does not ship and TI-P3 closes with that result. |
| **GE-I5** | Does transfer entropy name the right direction? | Simulated pairs with a known direction and delay; pairs with a common driver and no direct effect; pairs with none. | §10 | Released only if the direction is right in at least 90 % of driven pairs **and** a direction is asserted, at `null_ge / null_b ≤ 0.05`, in at most 10 % of common-driver pairs. Otherwise `te` stays reserved, and the result is recorded. |

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
