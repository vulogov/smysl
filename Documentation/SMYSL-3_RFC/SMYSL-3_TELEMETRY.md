# RFC SMYSL-3 — smysl for telemetry study and research

**Author:** Vladimir Ulogov
**Status:** final, for implementation. Every hypothesis of the research draft is either decided
here or deferred to a named, gated experiment with its stop condition written down beforehand.
**Target version:** not fixed. The work lands in whichever releases are current when each phase
is taken up. Phases are named (TL-P0 to TL-P9), not numbered by release.
**Evidence baseline:** `main` at `4f5ecb9` ("Merge 1.14.0"). Every code pointer and measurement
below describes that tree. Re-check them against the tree in hand before starting a phase.
**Supersedes:**
- *SMYSL-3 — smysl for telemetry study and research*, research draft 1 (2026-10-06)
- the review of that draft

**Companions:**
- `research_A_datamodels.md` [A], `research_B_methods.md` [B], `research_C_code_and_measure.md` [C]
- the architecture change proposal (`SMYSL_ARCHITECTURE_PROPOSAL.md`, cited as AC-n)
- the SMYSL-2 RFC set (`Documentation/SMYSL-2_RFC/`, cited as 2.n)

The key words MUST, MUST NOT, SHOULD, SHOULD NOT and MAY are used as in RFC 2119, and only in
Part A.

---

## 0. Summary

The question has not changed since before SMYSL-2 existed: **can smysl hold the history of
changes in telemetry, so that people and AI can study what changed, why, and how sure we are?**

Yes. The research draft showed how, and this RFC fixes the details.

**1. Readings are substrate. Units are assertions about readings.**
- One simulated day of a 200-host fleet produced 6.8 million change units even with unchanged
  values suppressed [C §3.1]. Only 6,566 of them were not gauge noise.
- SMYSL-2 learned the same lesson for text: segments are not units. SMYSL-3 applies it to numbers.
- Samples live in a **series substrate** outside the unit log. Object snapshots live as SMYSL-2
  parts. Units are made for what is *asserted* about them: a configuration change, a reset, a
  regime shift, an anomaly, an episode, a hypothesis, an action.

**2. Evidence is materialised on demand.**
- A derived unit grounds on **window evidence** units. Each is a pure function of a series, an
  instrument, a window, a summary recipe and the substrate bytes it read.
- Exact aggregates (count, min, max, correctly rounded sum, first, last) are `measured`.
- Approximate summaries (sketch quantiles) are `derived`.
- The unit count tracks what is asserted, not what is read.

**3. The value is the epistemics of explanations, not storage.** The literature measures how often
explanations go wrong:
- The best agent localises root causes in 11.34 % of OpenRCA cases.
- Agents fabricate data interpretations and leave evidence unused [B §5–6].

smysl's machinery is aimed at exactly those failures:
- **Rule M:** an explanation cannot outrank its evidence.
- **The numeric quote check (rule TQ, §11.4):** a quoted value must equal the evidence it cites.
- **Consulted relations:** record what an investigator looked at, not only what it cited.
- **Derived rival-explanation contentions (§11.5):** keep competing hypotheses open instead of
  silently picking one.

**4. Telemetry joins the SMYSL-2 library.** Postmortems, runbooks, tickets and incident chats are
already text with time, speaker and provenance. Numbers and narrative share one store under one
status ladder. That is the reason SMYSL-3 exists rather than being a telemetry tool.

**What this RFC adds:**

| Area | Addition |
|---|---|
| Format | No new record type and no kernel change. Two payload encodings defined by `x.tel/v1` (`dec`, a canonical decimal, and `bytes32`), proposed for A-7. Role assignments for the `timeline` thread schema. Two derived-only detection kinds. One `source` key number and record 16 kept reserved behind experiments. |
| Schema | `x.tel/v1`: 19 unit types (17, and 2 in TL-P9), 8 relation kinds, a canonical diff, canonical values, series identity as an ordinary uid |
| Rules | Schema-level rules, enforced by implementations that register `x.tel/v1`: TQ (numeric quotes), TW (window evidence), TL (retention is local) |
| Code | Two crates, `smysl-series` (substrate) and `smysl-tel` (readers, profiles, diff, detectors, materialiser). Both sit on the layering the architecture proposal fixes. |
| Delivery | Ten phases, TL-P0 to TL-P9. Nine gated experiments, GE-M1 to GE-M9. |

---

## 1. Decisions

The research draft ended with questions for the owner. The decisions below are made, so that the
RFC can be implemented. Each one names the option it chose, so that a later reversal is a
recorded change and not a silent one.

| # | Question | Decision | Rejected | Why |
|---|---|---|---|---|
| D-T1 | Readings as units, or as substrate (H-1) | **Substrate.** Units only for assertions; evidence materialised on demand. | A unit per reading or per change | 30 GB/day at the 1.9 resident size [C §3.1]; still ~18 GB/day at 1.14's 2.6 KB/unit |
| D-T2 | First-cut sources | **Kubernetes objects, Prometheus text / OpenMetrics 1.0, OTLP metrics (binary and JSON), Zabbix API exports.** Traces and logs come in TL-P9, behind GE-M2 and GE-M8. | Everything at once | Zabbix is OKO's demonstration source. The other three are the draft's proposal [A §1]. |
| D-T3 | Series identity (H-2) | **The uid of an `x.tel/series` unit.** No new identity kind or domain byte. | A `s3:` identity | Entities already collapse across windows by carrying no time (2.3 A-2.4 rule 7), and the series unit does the same. A new id kind costs every port. |
| D-T4 | Canonical numbers (H-5, T-A2) | **`dec`, a canonical decimal defined by `x.tel/v1`** for its own payloads (A-7 is not accepted), and proposed for A-7. The kernel float rule stays as it is. | Changing the kernel float rule; JCS text | The kernel encodes floats as binary32 quantised to 1/1024 and rejects other widths (`smysl-core/src/cbor/mod.rs` rule 4). Telemetry values cannot live there, and changing it would move every uid that has a float. |
| D-T5 | Sub-millisecond time (H-6, T-A1) | **ns kept in the substrate (option c). `source` key 7 reserved, not assigned.** GE-M2 (simulated) keeps it from release: 39.5 % of spans are affected, and the substrate orders them all. **Trigger:** key 7 is assigned only when something needs sub-millisecond order between *units*, not events: a `check` rule that orders spans across hosts, or a unit that must keep its order after its events are evicted. Neither exists (GE-M5 kept the causal constraint out of rule E). | Assigning key 7 now | The surface refuses an unknown `source` key, but CBOR preserves one silently and it enters identity. So a key is assigned only when a reader of it exists, and the number is reserved now so that nothing else takes it. |
| D-T6 | Record 16 (`hold`) | **Stays reserved**, decided by GE-M4. Expected outcome: release. | Deciding now | Holds added 11 % rather than saving anything [C §3.1]. The substrate is the likely answer. |
| D-T7 | Retention (H-10) | **Local eviction of substrate, not rule Z.** Rule TL. | Rule Z redactions | Rule Z propagates through merges and is meant for removal. Retention is a per-store policy: one store keeps 6 months, its peer 30 days. Writing a redaction would delete a peer's data at the next merge. |
| D-T8 | Investigations and OKO's "investigations as views" | **Both.** An `x.tel/investigation` unit is the anchor that `consulted` relations hang from; a View record with that unit as its root is what a TUI opens. | View only | A view cannot be the subject of a relation, and "what was consulted" needs one |
| D-T9 | Rival explanations and witness conflicts | **Detection kinds 6 and 7, derived-only** (like 4 and 5). | Recorded contentions | Derived from the record set, so rule U holds and nothing has to be written for them to appear |
| D-T10 | Instrument independence (H-9) | **Reported in dossiers; not a status rule.** Kept after GE-M6 (simulated, TL-P7). | A rule from the start | The draft's own stop condition. In GE-M6, collapsing duplicates never changed a conclusion. Ranking by the weakest witness changed many conclusions, but did not improve them: it doubled hit@1 where an instrument reported a fault that did not happen, and took it to near zero where an instrument missed a real fault. Lineage cannot tell the two cases apart. The disagreement is the signal, and kind 7 reports it. |
| D-T11 | Signing (`sig`, attestation key 7) | **Out of scope.** It gets its own RFC. SMYSL-3 locks timestamps with canonical commitments (2.3 A-12.2). | Signing in SMYSL-3 | `sig` is encoded but no code verifies it. Signing is a key-management problem as much as a format one, and it serves text as much as telemetry. |
| D-T12 | Testbed for GE-M7 | **Train-Ticket** (RCAEval-comparable, Apache-2.0) with scripted deploys and config edits | DeathStarBench, ITBench | Comparable to published numbers; 64 services |
| D-T13 | Model spend | **Local models by default**, as in the SMYSL-2 spike. One hosted arm in GE-M7, labelled and reported separately. | Hosted only | Reproducibility first |
| D-T14 | Name | **SMYSL-3** | — | — |

**Inputs still owed by the owner.** These are data, not decisions, and the RFC does not invent
them:
- **I-1:** a cluster to observe for 7 days for GE-M1, and the data policy for it. The fallback is
  a `kind` cluster under synthetic load, with results labelled synthetic.
- **I-2:** a Zabbix 6.x/7.x/8.x instance with exportable history for the Zabbix reader fixtures
  (shared with OKO).

---

## 2. Conventions

- **Identities:** `uid` (`b3:`), `tid` (`t3:`), `mid` (`m3:`), `did` (`d3:`), `rdid` (`r3:`) as in
  the format spec. SMYSL-3 adds none.
- **Status ladder:** `measured` > `cited` > `derived` > `inferred` > `speculative` > `unfounded`.
- **The ceilings rule T gives:**
  - `Op::Imported` at `Rung::Computed`: `measured`, and the only path to it
  - `Rung::Computed` otherwise: `derived`
  - `Rung::Document` / `Web`: `cited`
  - `Rung::Model`: `inferred`
- **Diagnostics:** SMYSL-3 takes the range `SMY-E6xx` / `SMY-W6xx`, which no earlier release
  uses (§15).
- **"Substrate"** means bytes that are not records in the unit log:
  - SMYSL-2 part texts (record 15, already refused in the log by `TextInLog`)
  - the series chunks of §19
- **"The baseline"** is the evidence baseline in the header.

---

# Part A — Normative

## 3. The three strata

```
 units (the log)          change · reset · stale · recreate · segment · anomaly · episode ·
                          alert · hypothesis · probe · action · investigation
                          + SMYSL-2 text units (postmortem, ticket, chat)
                                       ▲ grounds, x.tel/explains, x.tel/consulted
 evidence units           x.tel/window: series × instrument × window × recipe → summary
 (materialised on demand)              ▲ source.ref → series substrate digest
 substrate                snapshot parts (SMYSL-2 record 15, manifests chaining a source)
                          series chunks (§19), outside the log
```

**What becomes a unit, by leaf class (§8.2):**

| Class | Becomes a unit when | Type | Status |
|---|---|---|---|
| `config`, `identity`, `membership` | its value changes under the canonical diff (§8) | `change`, `recreate` | `measured` |
| `counter` | first seen, reset, recreated | `reset`, `recreate` | `measured` if the source states a start-time change; `derived` if inferred from a decrease |
| `gauge` | **never per reading.** Only through detectors and evidence. | `segment`, `anomaly`, `window` | `derived` (`window` per §9.4) |
| `histogram` | as `gauge`, per bucket layout | same | same |
| `event` | it happens and is in scope of a profile | `alert`, `change` | `cited` for a platform's own event (a Zabbix problem, a Kubernetes Event); `measured` for an instrument's own record |
| spans (TL-P9) | **never on read.** A trace's spans are materialised as evidence when something asks for them (§9.6) | `span` | `measured` |
| `log` (TL-P9) | **never per record.** Only through templates over a window (§9.7) | `log-template` | `derived` |
| staleness, series end | the source marks it (Prometheus stale NaN; OTLP `FLAG_NO_RECORDED_VALUE`) | `stale` | `measured` |
| episode | a grouping recipe fires (§21) | `episode` | `derived` |
| hypothesis | an agent or person proposes it | `hypothesis` | `inferred` with resolving grounds; `speculative` otherwise; capped by TQ |

An implementation that registers `x.tel/v1` **MUST NOT** write a unit per sample of a `gauge` or
`histogram` leaf, nor a unit per span or log record on read. A reader that is asked to fails with
`SMY-E601`.

## 4. The `x.tel/v1` schema

`x.tel/v1` is a registered extension schema in the sense of 2.3 A-11, with the same standing as
`x.text/v1`. An implementation that does not register it preserves every unit and relation under
rule X, and enforces none of rules TQ, TW and TL.

**Field tables live in code.** A-7, the structure that would put a field map on the wire, was not
accepted at G0 and stays for discussion (`smysl-core/src/schemas.rs`, module comment). So
`x.tel/v1` does what `x.text/v1` does:
- Its fields are a table in code (`smysl_tel::FIELDS`).
- Its schema declaration leaves `payload_shape` absent.
- The two value encodings it needs that the kernel lacks, `dec` (§5) and `bytes32` (a 32-byte
  CBOR byte string that is **not** a reference and is never resolved), are defined here, for
  `x.tel/v1` payloads only.

The A-7 proposal gains both types for the day A-7 is taken up (§5.4).

### 4.1 Unit types

| Type | Meaning | Required payload fields | Optional |
|---|---|---|---|
| `x.tel/series` | A series: what is measured, not when | `scheme` (enum), `key` (text, the canonical identity tuple §6) | `name`, `unit` (text), `kind` (enum) |
| `x.tel/series-meta` | Non-identifying metadata, versioned by `supersedes` | `of` (uid of the series) | `text` (help or description), `buckets` (list<dec>), `native_id` (text: a platform's own id, e.g. a Zabbix `itemid`) |
| `x.tel/instrument` | A thing in a measurement lineage | `role` (enum §7), `key` (text) | — |
| `x.tel/instrument-meta` | Non-identifying instrument metadata | `of` (uid) | `version` (text) |
| `x.tel/change` | One leaf changed between two snapshots | `op` (enum `add`/`remove`/`replace`), `pointer` (text), `class` (enum), `vtype` (enum) | `from`, `to` (text, canonical §5.5), `object` (uid of the object's series unit) |
| `x.tel/recreate` | An object or series was recreated: same name, new identity | `old`, `new` (text) | `object` (uid) |
| `x.tel/reset` | A counter reset | `series` (uid), `at_ms` (int) | `basis` (enum `start-time`/`decrease`) |
| `x.tel/stale` | A series ended or went stale | `series` (uid), `at_ms` (int) | `marker` (enum) |
| `x.tel/window` | Window evidence (§9) | `series`, `instrument` (uid), `from_ms`, `to_ms` (int), `recipe` (text), `basis` (enum `substrate`/`platform`), `count` (int) | `digest` (bytes32; required when `basis` is `substrate`), `min`, `max`, `sum`, `mean`, `first`, `last` (dec); `qs`, `q` (list<dec>), `alpha` (dec) |
| `x.tel/segment` | A regime between two change points | `series`, `from_ms`, `to_ms`, `recipe` | `level`, `slope` (dec) |
| `x.tel/anomaly` | A point or window judged anomalous by a recipe | `series`, `from_ms`, `to_ms`, `recipe`, `score` (dec) | `direction` (enum `up`/`down`) |
| `x.tel/alert` | An alerting system's own statement (Zabbix problem, Alertmanager alert) | `system` (text), `key` (text), `state` (enum `firing`/`resolved`) | `severity` (text) |
| `x.tel/episode` | A deterministic grouping of changes and anomalies in a window | `from_ms`, `to_ms`, `recipe` | `label` (text) |
| `x.tel/hypothesis` | A proposed explanation | `claim` (text) | `quotes` (list<text>, §11.4), `target` (uid) |
| `x.tel/investigation` | The anchor of one investigation | `question` (text), `opened_ms` (int) | `rotation` (text), `closed_ms` (int) |
| `x.tel/probe` | A read-only observation an investigator made: a command, a query, a dashboard read | `what` (text), `at_ms` (int) | `output` (text, a tid) |
| `x.tel/action` | Something done to the system: a restart, a rollback, a scale | `what` (text), `at_ms` (int) | `confirmed_by` (text), `output` (text, a tid) |
| `x.tel/span` | Span evidence (TL-P9, §9.6): one span, materialised from the event substrate | `series` (uid of the span stream), `instrument` (uid), `trace` (text, 32 lowercase hex), `span_id` (text, 16 lowercase hex), `name` (text), `start_ms`, `end_ms` (int), `digest` (bytes32) | `parent` (text, 16 lowercase hex), `span_kind` (enum), `status` (enum), `service` (text) |
| `x.tel/log-template` | Template evidence (TL-P9, §9.7): one Drain template over a window of a log stream | `series` (uid of the log stream), `instrument` (uid), `from_ms`, `to_ms` (int), `recipe` (text), `template` (text), `count` (int), `digest` (bytes32) | — |

Nineteen types. Four describe what is measured and by what (`series`, `series-meta`,
`instrument`, `instrument-meta`); fifteen carry assertions about it. TL-P9 added the last two,
`x.tel/span` and `x.tel/log-template`, additively: a reader that knows seventeen preserves them
under rule X.

**Times.** Times inside payloads (`*_ms`) are epoch milliseconds, the unit `observed` uses. The
unit's own `source.observed` is:
- for `window`, `segment`, `anomaly`, `episode`, `log-template`: the end of the window
- for `span`: its start (`start_ms`), so that span order (§10.4) compares the instants it orders
- for `change`, `recreate`, `reset`, `stale`: the instant of the reading that showed it
- for `probe` and `action`: when it ran
- for `series-meta` and `instrument-meta`: the instant the metadata was read

**Entity-like units (the A-2.4 rule 7 analogue).** `x.tel/series` and `x.tel/instrument`:
- MUST NOT carry `span`, `manifest`, `published`, `observed` or `grounds`
- MUST carry a source of exactly `{kind: metric, reference: "series:<scheme>:<key>"}` (series) or
  `{kind: tool, reference: "instrument:<key>"}` (instrument), and no other `source` key

Their identity is therefore a function of the identity tuple alone, and they collapse across time,
readers and peers. Everything that varies over time goes into the `-meta` units, which point *at*
the entity and never the other way, so there is no cycle and a metadata update never moves a
series uid. A violation is `SMY-E607`.

### 4.2 Relation kinds

| Kind | From → to | Meaning |
|---|---|---|
| `x.tel/of-series` | window, segment, anomaly, reset, stale → series | what it is about (redundant with the payload `series` field, kept for traversal) |
| `x.tel/via` | instrument → instrument | lineage: the source instrument's data passed through the target |
| `x.tel/measured-by` | series → instrument | the instrument that measures this series at its origin |
| `x.tel/same-quantity` | series → series | two series measure one physical quantity, in the same unit (declared by a profile or a person) |
| `x.tel/during` | any → episode or investigation | membership in a window or an investigation |
| `x.tel/explains` | hypothesis → change, anomaly, alert, episode | the hypothesis offers an explanation of the target |
| `x.tel/consulted` | investigation → any unit | the investigator looked at the target, attested by its agent at the time it looked |
| `x.tel/precedes` | alert, probe, action, change, and in TL-P9 span → the same set | a causal order known from the protocol or the tool (§10.4) |

The kernel relations (`supports`, `rebuts`, `supersedes` and the rest) keep their meaning. A
hypothesis cites its evidence through the unit's `grounds`, as any claim does.

### 4.3 Enumerations

All of them are open (Unknown preserved), as every enumeration has been since 1.9.

- **`scheme`:** `otel`, `prom`, `k8s`, `zabbix`, `json` (a generic profile), and in TL-P9
  `journald`, `cloudevents`
- **`class`:** `config`, `identity`, `membership`, `counter`, `gauge`, `histogram`, `event`, `log`
- **`kind`:** `counter`, `gauge`, `histogram`, `summary`, `state`, `text`, and in TL-P9 `span`,
  `log`, `event` (the three event-stream kinds, §19.6)
- **`span_kind`** (TL-P9): `internal`, `server`, `client`, `producer`, `consumer`, OTLP's
  `SPAN_KIND_*` lowercased; `SPAN_KIND_UNSPECIFIED` is the field's absence
- **`status`** (span, TL-P9): `unset`, `ok`, `error`, OTLP's `STATUS_CODE_*` lowercased
- **`role`** (instrument): `exporter`, `agent`, `scraper`, `collector`, `pipeline`, `store`,
  `transform`
- **`vtype`:** `null`, `bool`, `int`, `dec`, `text`, `json`
- **`basis`:** `substrate`, `platform`

## 5. `dec` and canonical values

### 5.1 Why the kernel float cannot carry telemetry

The kernel encodes a float as binary32 after quantising it to 1/1024, and rejects binary16 and
binary64 (`smysl-core/src/cbor/mod.rs`, rule 4). That is right for weights and gaps. It loses a
latency of 0.0004 s and every counter above 2²⁴, so telemetry values do not go through it.

### 5.2 `dec`: wire form

A `dec` is a CBOR array of two integers `[m, e]` with value m × 10^e.

**Canonical form.** An implementation MUST refuse anything else when decoding (`SMY-E602`):
1. `m` and `e` are shortest-form CBOR integers, major type 0 or 1.
2. `m` is in [−(2⁶⁴ − 1), 2⁶⁴ − 1]: the magnitude fits `u64`, with the sign carried by the major
   type. The one CBOR value outside this range, −2⁶⁴, is refused. Implementations need 65 bits
   (`i128` in Rust, `BigInt` in JavaScript, `int` in Python, sign plus `uint64` in Go).
3. `e` is in [−400, 400].
4. If `m ≠ 0`, then `m` is not divisible by 10. Zero is `[0, 0]`.
5. NaN and the infinities are not values. A stale marker is a fact (`x.tel/stale`), not a number.

### 5.3 `dec`: from sources, and its one text spelling

**From a decimal token** (a JSON number, a Prometheus sample value, a Zabbix value string):
- The token is parsed **exactly as written**, never through binary64: its digits become `m` and
  its exponent becomes `e`, then the pair is normalised by rule 4.
- `41`, `41.0` and `4.10e1` are all `[41, 0]`.
- A token whose normalised `m` or `e` falls outside rules 2–3 is not a `dec`. In a change unit it
  becomes `vtype text` with the token verbatim (`SMY-W613`).

**From IEEE-754 binary64** (OTLP `asDouble`, protobuf doubles):
- The value is the decimal that **ECMA-262 `Number::toString` selects**: the fewest significant
  digits that round-trip, and among those the one closest to the exact binary value, with ties to
  even. Ryū produces exactly this. Grisu2 does not, and MUST NOT be used.
- `-0` becomes `[0, 0]`.

**From a typed integer** (`u64`, `i64`, OTLP `asInt`): `[n, 0]`, normalised by rule 4, and never
routed through a double.

**The text spelling** (surface, and `from`/`to` in change units) is a plain decimal with no
exponent and no `+` sign:
- `-` if negative
- if `e ≥ 0`: the digits of |m| followed by `e` zeros
- if `e < 0`: |m| with a point inserted |e| digits from the right, left-padded with zeros so that
  at least one digit precedes the point

So `[412, -1]` is `41.2`, `[1, 3]` is `1000`, `[15, -10]` is `0.0000000015`, and `[0, 0]` is `0`.
Every canonical `dec` has exactly one spelling, and parsing the spelling back gives the same pair.

### 5.4 For A-7, when it is taken up

The A-7 field-type table gains `dec` (wire form §5.2, surface §5.3) and `bytes32` (a 32-byte byte
string, never resolved as a reference). A reader that knows neither preserves the field under
rule X.

### 5.5 Canonical leaf values in `x.tel/change`

`from` and `to` are text, read through `vtype`:

| `vtype` | Text | When |
|---|---|---|
| `null`, `bool` | `null`, `true`, `false` | JSON literals |
| `int` | decimal integer, `-` only if negative, no leading zeros | values typed as integers by the source format: protobuf integer fields, OTLP `asInt`, Kubernetes fields whose OpenAPI type is `integer` |
| `dec` | the spelling of §5.3 | **every JSON number** not covered by the row above |
| `text` | the string as NFC (the kernel normalises all text to NFC) | strings, and out-of-range tokens (`SMY-W613`) |
| `json` | an unkeyed array or opaque object as RFC 8785 JCS text, **except** that every number is written by the `dec` spelling (JCS turns large integers into doubles [A §5]) | §8.1 step 2 |

Because a JSON number is `dec` unless the source *types* it as an integer, `41` → `41.0` in a
snapshot is no change at all. `41.2` and `41.20` from two exporters are one value and one uid.

## 6. Series identity

A series is identified by the uid of its `x.tel/series` unit (D-T3). `key` is the canonical
identity tuple, as text so that it is inspectable:

| `scheme` | `key` |
|---|---|
| `otel` | the §5.5-number JCS text of `{resource, scope:{name,version}, name, kind, unit, temporality, monotonic, attributes}`, with attribute sets sorted by key [A §1]. `description` and bucket bounds are **not** identity; they go to `x.tel/series-meta`. |
| `prom` | the label set including `__name__`, sorted by label name, as `name{l1="v1",…}` with Prometheus escaping |
| `k8s` | `apiVersion/kind/namespace/name#uid` + the JSON Pointer of the leaf (§8.1 pointer form). The object's own series has an empty pointer. |
| `zabbix` | `host:<host>/item:<key_>` with the item key verbatim. The `itemid` goes to `series-meta.native_id`, because it changes when templates are re-imported. |
| `json` | the profile's declared key fields, then the pointer |
| `otel` span stream (TL-P9) | the §5.5-number JCS text of `{signal:"spans", resource, scope:{name,version}}`. One stream per (resource, scope), as OTLP groups them. |
| `journald` (TL-P9) | the JCS text of `{host, unit}`: `_HOSTNAME`, and `_SYSTEMD_UNIT`, else `SYSLOG_IDENTIFIER`, else `""` |
| `cloudevents` (TL-P9) | the JCS text of `{source, type}`, the two attributes CloudEvents 1.0 says identify a kind of event from one producer |

**Renames and recreation:**
- A Kubernetes object recreated with a new `uid` is a new series, linked to the old one by an
  `x.tel/recreate` unit.
- A Prometheus label rename is a new series. A profile MAY declare a lens (2.5 §3.12) that maps old
  labels to new. The lens applies at query time and never changes a uid.

**Exporter upgrades** that change only metadata add an `x.tel/series-meta` unit that supersedes
the previous one. The series uid stands.

## 7. Instruments, lineage and witnesses

An instrument is an `x.tel/instrument` unit whose `key` is its own stable identity, for example
`prom-scrape:<job>/<instance>`, `otel-collector:<service.instance.id>` or `zabbix-server:<name>`.
Lineage is the `x.tel/via` edges from where a value is measured to where smysl read it, and each
series names its origin by `x.tel/measured-by`.

**Witnesses of one series.** Every window over one series shares that series' origin, so evidence
about one series is always **one** witness, however many scrapers carried it. Several transport
paths for one origin are duplicates, and are reported as such.

**Witnesses of one quantity.** Two series are evidence about one quantity only when an
`x.tel/same-quantity` relation joins them. Their evidence counts as **independent witnesses**
when:
- their origin instruments differ, and
- no instrument with role `transform` lies on both lineages

Instruments with roles `scraper`, `collector`, `pipeline` and `store` that appear on both lineages
are **common transport**. Common transport does not merge witnesses, but it is reported (D-T10).
Some cases:
- Two scrapes of one node_exporter: one series, one witness.
- node_exporter disk usage and an application's own disk metric, joined by `same-quantity`: two
  witnesses.
- Two series both produced by one recording rule: one witness.

Witness counts are reported in dossiers (§22.2) and in detection kind 7 (§11.5). They change no
status: GE-M6 kept D-T10.

## 8. Change units: the canonical diff

### 8.1 Procedure (normative)

Given two consecutive snapshots S₀, S₁ of one object under one profile P:

1. **Parse.** Both are parsed as JSON. Leaves become canonical leaf values (§5.5).
2. **Key arrays.** For each array whose pattern P declares a merge key (`containers` → `name`,
   `conditions` → `type`, `env` → `name`; for Kubernetes taken from the published
   `x-kubernetes-patch-merge-key`):
   - If **every** element is an object whose key field is a `text`, `int` or `dec` scalar, and the
     key texts (§5.5) are pairwise distinct, the array is replaced by an object keyed by those
     texts.
   - Otherwise (a missing key, a non-scalar key, or a duplicated key value, as `env` sometimes has)
     the whole array is one leaf of `vtype json`, reported `SMY-W608`.
3. **Filter.** Remove the leaves matched by P's noise patterns (§8.3). Patterns therefore name
   keyed pointers.
4. **Classify.** Each remaining leaf takes the class of the **longest** class pattern of P that
   matches its pointer or a prefix of it. A class declared on `/metadata/labels` therefore covers
   every label. A leaf with no matching pattern is `gauge`.
5. **Diff per leaf.** For every pointer present in either side:
   - in S₁ only: `add`
   - in S₀ only: `remove`
   - in both, with different `vtype` or text: `replace`
6. **Emit.** One `x.tel/change` per diffed leaf of class `config` or `membership`, with:
   - `source.reference` = S₁'s tid and the pointer
   - `source.manifest` = S₁'s mid
   - `source.observed` = S₁'s instant

   A change to a leaf of class `identity` emits `x.tel/recreate` instead. `gauge` and `counter`
   leaves emit no unit; they go to the series substrate if P routes them there (§20.1).

**Pointers.** Pointers are RFC 6901. A keyed element's segment is its key text, escaped by RFC
6901 (`~0`, `~1`): `/spec/containers/web/image`. All pointers and patterns begin with `/`.

**Patterns.** Profiles use pointer patterns: RFC 6901 pointers in which a segment written `*`
matches exactly one segment, and a literal key `*` is written `~2`. `~2` is the only extension to
RFC 6901, and it appears only in profiles, never in units.

The procedure is a function of (S₀, S₁, P), so the same inputs give the same uids on every
implementation, as the fixtures of §16 check. Change units are attested with `Op::Imported` at
`Rung::Computed`, a deterministic transcription of two measured snapshots, so they are
`measured`.

### 8.2 Leaf classes

The class is declared per pattern in a reader profile (§20.2), and the profile hash is recorded in
the recipe, so a misclassification changes the recipe hash and is visible. An undeclared leaf
defaults to `gauge` and emits nothing until someone declares it. A profile MAY promote a leaf once
observation, of the GE-M1 kind, shows that it is quiet.

### 8.3 Noise filters

A profile's noise patterns remove leaves before the part is stored, so a snapshot that changed
only in filtered leaves has the same tid as its predecessor and costs nothing.

The `k8s/1` profile MUST filter:
- `/metadata/managedFields`
- `/metadata/resourceVersion`
- `/metadata/annotations/kubectl.kubernetes.io~1last-applied-configuration`
- `/status/conditions/*/lastHeartbeatTime`
- `/status/conditions/*/lastProbeTime`
- Lease objects' `/spec/renewTime`

It MUST NOT read the `data` or `stringData` of `Secret` objects at all.

The part stores the **filtered, keyed** object, serialised as canonical JSON (JCS with §5.5
numbers). The unfiltered object is not kept: D-T1 applies to bytes as well as to units. The profile
hash is in the manifest's reader recipe.

## 9. Window evidence

### 9.1 Samples and the substrate digest

A **sample** is the tuple (series uid, instrument uid, `ts_ms`, `ns`, value, flags), where:
- `ns` is the sub-millisecond remainder 0–999,999, or absent
- `value` is a `dec`
- `flags` is a bit set: 1 stale, 2 reset (start-time change), 4 no-recorded-value, 8 `ns` present

A **stale** or no-recorded-value sample has value `[0, 0]`.

**Sample order:** `ts_ms`, then `ns` (absent sorts as 0), then the CBOR encoding of the value,
then flags.

**The substrate is a set of samples.** Two equal tuples are one sample. Two samples that differ
only in value or flags at the same (series, instrument, `ts_ms`, `ns`) are both kept: a data error,
reported `SMY-W615`, and both enter the digest.

**Digest of a window** `[from, to)`:

```
digest = BLAKE3-256( "smysl/tel-window/1" ‖ 0x00 ‖ cbor_array( s₁, s₂, … ) )
sᵢ     = cbor_array( ts_ms, ns_or_0, value, flags )
```

The samples are those of the series and instrument whose `ts_ms` falls in the window, in sample
order, encoded by the kernel's deterministic CBOR. This is the only definition of the digest. It
depends on the samples, never on how they are chunked, stored or exchanged (§19).

### 9.2 Identity, late data, and several windows over one key

An `x.tel/window` unit's core is a function of its payload (§4.1), including `digest`.
Materialising the same evidence over the same samples twice therefore gives the same uid, and
merging the two is idempotent.

**Late data does not supersede.**
- A window materialised before late samples arrived and one materialised after are **both true**:
  each states what a given sample set contained, and its digest says which set.
- Neither supersedes the other. A store can therefore hold several windows with one **window key**
  (series, instrument, `from_ms`, `to_ms`, `recipe`), and any two peers converge to the same set
  of them (rule U).
- Where a consumer needs one, it takes the **preferred window** of the key: the largest `count`,
  then the lexicographically smallest `digest`. That is a function of the set, not of arrival.
- When the windows of a key disagree, `SMY-W614` reports it. It is not a contention: they describe
  different substrate states, and a reviewer has nothing to resolve.

### 9.3 Summaries

| Field | Definition | Exact? |
|---|---|---|
| `count` | number of samples without flag 1 or 4 | exact |
| `min`, `max` | over the counted samples | exact |
| `first`, `last` | the earliest and latest counted samples in sample order | exact |
| `sum` | **integer series** (every counted value has `e ≥ 0`): the exact integer sum, as a `dec`, absent with `SMY-W616` if it exceeds §5.2's range. **Otherwise**: the exact sum of the counted values (they are decimals, so the sum is a decimal), rounded once to binary64 by round-half-even and converted by §5.3 | exact (the rounding is a fixed function of the set, independent of order) |
| `mean` | the exact sum ÷ `count`, rounded once to binary64 by round-half-even, then §5.3. Never computed from the rounded `sum`. | exact in the same sense |
| `qs`, `q`, `alpha` | quantiles `q[i]` at ranks `qs[i]` from a DDSketch with relative accuracy `alpha` | approximate |

### 9.4 Status (rule TW)

> **Rule TW.**
> - A window with `basis: substrate` that carries only exact fields is attested with
>   `Op::Imported` at `Rung::Computed`, and is `measured`.
> - A window with `basis: substrate` that carries any approximate field is attested with
>   `Op::Transformed` at `Rung::Computed`, and is `derived`.
> - A window with `basis: platform` (a platform's own aggregate, such as a Zabbix trend) carries
>   no `digest`, and is attested with `Op::Imported` at `Rung::Document`, which makes it `cited`:
>   we transcribed it, but we did not compute it.
> - An `x.tel/span` (§9.6) is attested as an exact window, and an `x.tel/log-template` (§9.7) as
>   a sketched one.
> - An implementation MUST NOT attest otherwise (`SMY-E603`).

The usual way to have both kinds is two units, one exact and one sketched. DDSketch merges are
bit-exact in any order [B §8, probe]. t-digest is not, and MUST NOT be used for evidence.

### 9.5 Recomputation

While its substrate is present, evidence with `basis: substrate` is recomputable from the samples,
digest and every field. The `tel-evidence` check pass MAY recompute any window and reports a
mismatch as `SMY-E604`. When the samples are absent (evicted, or never received), it reports
`SMY-W605`, "evidence not re-derivable", and nothing else: the unit's status does not depend on
the substrate being present. That is the honest cost of retention, and it keeps rule TL (§13) from
touching rule U.

### 9.6 Span evidence (TL-P9)

A span is a reading, as a sample is (D-T1). OTLP traces are read into the **event substrate**
(§19.6), not into units. An `x.tel/span` unit is materialised from it when something asks for a
trace: an investigation consulting it, a hypothesis grounding on it, `tel trace` (§23).

- **Fields.** Copied from the span's event body (§19.6): ids as lowercase hex, `start_ms` and
  `end_ms` as `floor(unix_nano / 10⁶)`, `service` from the resource's `service.name`. The
  sub-millisecond remainders stay in the body (§10.2), and `digest` commits to them.
- **`digest`** is the event digest (§19.6) of the one event, so the unit is a function of the
  substrate and materialising twice gives one uid.
- **Order.** Materialising a trace also writes `x.tel/precedes` from each parent span unit to each
  child span unit whose `parent` names it. A parent that is not in the substrate gets no edge.
- **Status.** As an exact window: `Op::Imported` at `Rung::Computed`, `measured` (rule TW, §9.4).

### 9.7 Template evidence (TL-P9)

A log record is a reading too. An `x.tel/log-template` unit states that, over the events of one
log stream and instrument in `[from, to)`, Drain (§21) with the recipe `recipe` produced this
template `count` times.

- **`digest`** is the event digest of the window, every template of one run sharing it. Drain's
  templates depend on every record it read and on their order, so a template is evidence about
  the window, not about the records it matched.
- **Order** is event order (§19.6), so the run is a function of the event set.
- **Status.** `Op::Transformed` at `Rung::Computed`, `derived`: a template is a summary that
  depends on parameters, as a sketch is (rule TW). Its grounds are its stream, as a sketched
  window's are its series.

### 9.8 Recomputation of span and template evidence

As §9.5. While the events are present, `tel-evidence` recomputes a span (one event's digest and
fields) or a template (the window's digest, then Drain) and reports a mismatch as `SMY-E604`.
When they are absent it reports `SMY-W605` and nothing else.

## 10. Time

### 10.1 The index stays `observed`

`observed` keeps its type (u64 epoch ms) and its meaning. Every SMYSL-2 decision and every store
since 1.8 relies on it. SMYSL-3 MUST NOT change it.

### 10.2 Sub-millisecond time

- Nanosecond timestamps (OTel, gNMI) are kept exactly in the series substrate (§19.2), as a `u32`
  remainder column.
- Units do not carry them. Ordering below the millisecond across hosts is `inferred`, never
  `measured`.
- `source` key 7 is **reserved** for an optional sub-millisecond remainder. GE-M2 decides only
  whether it is released (under 1 % of the spans that matter collapse); it is assigned on D-T5's
  trigger, when a rule or a unit needs sub-millisecond order between units. Until then a writer
  MUST NOT emit it (`SMY-E612`). Note what reserving does and does not do: the surface parser refuses an
  unknown `source` key, but the CBOR decoder preserves one silently, and it enters identity. A
  foreign producer's key 7 is therefore carried, not refused. A `check` pass in an implementation
  that registers `x.tel/v1` reports it as `SMY-E612`.

### 10.3 Time statuses (an extension of 2.3 A-12.2's table)

| As-recorded value | Time status |
|---|---|
| `observed` of a change, reset or stale unit from an instrument transcription (`Op::Imported`, `Computed`) | measured |
| `observed` of window evidence | measured (exact) or derived (approximate), the unit's own status |
| a platform's own timestamp exported through its API (Zabbix trend hour, Kubernetes Event `lastTimestamp`, Alertmanager `startsAt`) | cited |
| a human's statement of when something happened (chat, postmortem) | as SMYSL-2: the unit's status |

**Locking.** A deploy time that came from the CD system's own record is locked with a `canonical`
commitment naming its dating, exactly as 2.3 A-12.2 specifies for truly observed time. A later
human dating that disagrees then forms a detection-kind-4 contention instead of moving it.

### 10.4 Causal-order free constraints (TL-P9, gated by GE-M5)

If GE-M5 passes, rule E gains one free constraint, at the status of the trace data:

| Constraint | Rule | Status |
|---|---|---|
| span order | if A `x.tel/precedes` B (parent before child, request before response), said(A) ≤ said(B) | cited (exporter-recorded) |

Only materialised spans take part: the constraint reads units and relations, never substrate, so
rule U holds.

**GE-M5 on simulated traces failed it (TL-P9), so rule E is unchanged** and `x.tel/precedes`
stays an ordinary relation. What the run found, for whoever takes this up with real data:
- **A start-only constraint is blind to some hosts.** A callee whose clock runs ahead never
  starts too early, and a root whose clock runs behind has no caller to start before. That was 8
  of the 22 host-and-direction cases here.
- **The response constraint sees every skew of 2 ms or more.** That is a child's end at most its
  parent's. It needs a second instant per span, which said(·) does not have.
- **A violated edge names a pair of clocks, not the wrong one.**
- **Ordinary NTP residuals trip it.** At millisecond resolution, offsets of a quarter of a
  millisecond violate about 1 % of cross-host edges, so a rule would need a tolerance or the
  sub-millisecond part.

What follows describes the constraint as it would work if adopted. Skew would show up as a
detection-kind-5 contention (an empty interval), and would be corrected by a window dating
with a measured offset, which SMYSL-2 already specifies. Until GE-M5 passes,
`x.tel/precedes` is an ordinary relation and rule E ignores it.

## 11. Investigations

### 11.1 Shape

```
 x.tel/investigation  ◄── View (root = the investigation; what a TUI opens)
    │ x.tel/consulted ──► window, change, alert, chat units …   (what was looked at)
    │ x.tel/during    ◄── hypothesis, probe, action             (what happened in it)
 x.tel/hypothesis ── grounds ──► evidence units
                  ── x.tel/explains ──► change / anomaly / alert / episode
```

One store can hold many investigations. OKO's "one ledger store per on-call rotation, with
investigations as views" is this shape, with `rotation` set on each investigation.

### 11.2 Consulted

- An investigator (human handle or model agent) records `x.tel/consulted` when it reads a unit,
  attested at the instant of reading.
- Relations are records, so they cost a record each. They are written per unit looked at, not per
  screen refresh.
- An implementation MAY batch them per probe.

From consulted, cited and explained, a dossier reports:
- **unused evidence:** units consulted but not cited by any hypothesis in the investigation
- **unexplored evidence:** window or change units in the investigation's window, about series that
  a hypothesis targets, that were never consulted

These are the two failure modes the literature measures [B §6]. They become numbers here.

### 11.3 Probes and actions

A probe or action's output is text: a SMYSL-2 part, redacted by the producer before storing. OKO
keeps "full redacted command output". The unit's `output` names the part's tid.

An action records `confirmed_by` when a human confirmed an unsafe command. Probes and actions are
attested with `Op::Imported` at `Rung::Computed` by the tool that ran them: that the command ran,
and what it printed, is `measured`. What it *means* is a hypothesis.

### 11.4 Rule TQ — numeric quotes

A hypothesis that quotes numbers MUST list each quote in `quotes`, as text:

```
<uid>#<field>=<value>              e.g.  b3:k2…#max=0.931
<uid>#<field>[<index>]=<value>     e.g.  b3:k2…#q[2]=0.9      (0-based, into a list field)
```

`<value>` is the `dec` spelling of §5.3, or an `int`.

> **Rule TQ.**
> - A quote **verifies** when the cited unit:
>   - is among the hypothesis's grounds
>   - resolves
>   - has that field, and that index for a list
>   - holds a value whose canonical text equals `<value>`
> - A hypothesis with a quote that does not verify has an effective status of at most
>   `speculative`, whatever it was authored as. `SMY-W606` names the quote.
> - Like rule M, rule TQ is a cap. It reads only records, never substrate, so it is a function of
>   the record set, it is monotone, and it is safe under merge.

This is SMYSL-2's text quote check applied to numbers, and it catches the fabricated
interpretation of [B §6] mechanically. A hypothesis that says "p99 went to 2.3 s", quotes
`#q[2]=2.3`, and cites a window whose `q[2]` is `0.9` cannot rise above `speculative`.

### 11.5 Rival explanations and witness conflicts (detection kinds 6 and 7)

Both kinds are **derived-only**, like 4 and 5. They are detected from the record set by
implementations that register `x.tel/v1`, and are never written. Their ids follow spec §6.2:

```
digest = BLAKE3-256( kind ‖ over ‖ positions ),  id = "k/c" ‖ base32(first 130 bits)
```

| Kind | Name | Detected when | `over` | `positions` |
|---|---|---|---|---|
| 6 | `RivalExplanation` | two or more live hypotheses `x.tel/explains` one target; no one of them `rebuts` or `supersedes` another; each has effective status ≥ `inferred` after rules M and TQ | the target's uid | all such hypotheses' uids, sorted and deduplicated: **one contention per target** |
| 7 | `WitnessConflict` | two series joined by `x.tel/same-quantity`, with independent origins (§7), have preferred windows (§9.2) with the same `from_ms`, `to_ms` and `recipe` that disagree: an exact field differs, or a quantile differs by more than 2 × the larger `alpha` (relative) | the smaller of the two series uids | the two preferred windows' uids |

- **Determinism.** Each kind is a function of the record set and the registered schemas. An
  implementation that does not register `x.tel/v1` detects neither, and preserves everything. The
  same holds for every extension-level derivation, and C-Tel-Check (§16) fixes the expected ids.
- **Refusal.** TL-P1 adds both kinds to `DetectionKind::DERIVED_ONLY` before any `x.tel` writer
  exists. A store then refuses a record 6 of either kind (`SMY-E611`), as it refuses 4 and 5.
  A store that predates TL-P1 decodes such a record as `Unknown` and keeps it. That is the same
  exposure 4 and 5 have had with pre-1.10 peers since 1.10, and it arises only from a
  non-conforming writer.
- **Resolution.** A resolution (record 12) names the derived id, which is the record a reviewer
  writes. Kind 6 is the "rival explanations" item carried since 1.9.

## 12. The `timeline` thread schema

SMYSL-2.3 A-8.2 allocated code 5 to `timeline` and roles 49–63 to it. SMYSL-3 assigns them. Role
names are unique across all schemas, because `Role::parse` returns the first match, and none of
these collides with an existing name:

| Role | Name | Typical members |
|---|---|---|
| 49 | `trigger` | the change, deploy or action believed to start it |
| 50 | `onset` | first anomalous evidence |
| 51 | `detection` | first alert or human notice |
| 52 | `escalation` | paging, severity raised |
| 53 | `diagnosis` | probes and hypotheses |
| 54 | `mitigation` | actions taken to reduce impact |
| 55 | `recovery` | evidence of return to baseline |
| 56 | `closure` | the investigation closed and the accepted explanation (named so because `resolution` is role 10) |
| 57 | `impact` | evidence of user-facing effect |
| 58 | `communication` | status updates, chat |
| 59 | `follow-up` | tickets and postmortem actions |
| 60–63 | reserved | — |

**Order.** A `timeline` thread is ordered by the unit's effective said-time (rule E):
1. dated units, by the interval's earliest bound, then its latest bound, then uid
2. contested units, by their as-recorded instant, then uid, each marked contested
3. undated units, by uid

Thread derivation (`smysl-thread`) proposes roles from unit types. Humans and agents author them
with an override, as for every thread schema.

## 13. Retention (rule TL)

> **Rule TL.** Retention is the local eviction of telemetry substrate.
> - An implementation MAY delete samples (§9.1), events (§19.6), and the part texts of manifests
>   whose reader is an `x.tel` reader (§20.1), when they are older than a local policy bound.
> - It MUST NOT evict the parts of any other manifest. Postmortems, chats and tickets are SMYSL-2
>   texts, governed by SMYSL-2 alone.
> - Eviction writes no record, does not propagate through merge, and MUST NOT be expressed as a
>   rule Z redaction.
> - Units are never evicted.

**Consequences:**
- **Rule U holds.** Nothing derived from records depends on substrate. Window status is fixed at
  attestation (§9.5). Change units carry their values. Rule TQ reads records only. `x.tel`
  snapshot parts are not quote-checked, because their units are self-contained.
- **Merging can bring substrate back.** A merge with a peer that kept the substrate restores it, if
  both peers exchange substrate (§19.5). Eviction is a cache decision, not a statement about the
  world.
- **Round trip.** P1–P3 (2.7) are stated over records. Substrate travels only with
  `--with-substrate`, and its listing names what is absent. Exporting an evicted store is lossless
  over what the store holds.
- **Missing substrate is a warning.** References to evicted substrate are reported as `SMY-W605`,
  never as errors.
- **Rule Z keeps its job:** removing text that must not be held, for legal or privacy reasons.

## 14. Reserved and gated

| Item | State | Decided by |
|---|---|---|
| Record 16 (`hold`) | reserved; MUST NOT be written | GE-M4. If the substrate answers "value in force at t" correctly and under 50 ms p95 over 6 months, release the number to the general pool. |
| `source` key 7 (sub-ms remainder) | reserved | Release: GE-M2 (simulated: 39.5 % affected, not released). Assignment: D-T5's trigger, sub-ms order needed between units; none yet |
| `SourceKind` value for an instrument | not allocated; instruments are units (§7) | GE-M6: the unit model was sufficient, so no kind is added. |
| Causal free constraint | not in rule E | GE-M5. Simulated: fails (§10.4) |
| Witness count in status | not a rule | GE-M6: kept as a report (D-T10) |

## 15. Diagnostics

| Code | Meaning |
|---|---|
| `SMY-E601` | a reader was asked to emit units per sample of a `gauge`/`histogram` leaf |
| `SMY-E602` | a non-canonical `dec`: trailing-zero mantissa, `-0`, out of range, non-integer parts, or −2⁶⁴ |
| `SMY-E603` | a window, span or template attested contrary to rule TW |
| `SMY-E604` | recomputed window, span or template evidence does not match the unit |
| `SMY-W605` | window, span or template evidence refers to substrate that is absent (evicted or never received). A probe output is a SMYSL-2 part that names no samples and rule TL never evicts, so it is not reported. |
| `SMY-W606` | a numeric quote does not verify (rule TQ); the hypothesis is capped at `speculative` |
| `SMY-E607` | a `series` or `instrument` unit breaks §4.1's entity-like rule |
| `SMY-W608` | a declared keyed array could not be keyed (missing, non-scalar or duplicated key); diffed as one `json` leaf |
| `SMY-W609` | a series has no `x.tel/measured-by` (witness counting treats it as its own origin) |
| `SMY-W610` | the profile changed between two snapshots of one manifest chain; the boundary is reported as a profile change, not diffed |
| `SMY-E611` | a record 6 of kind 6 or 7 (derived-only) |
| `SMY-E612` | a writer emitted reserved `source` key 7 or record 16 |
| `SMY-W613` | a number token outside `dec`'s range, kept as `vtype text` |
| `SMY-W614` | the windows of one window key disagree (§9.2) |
| `SMY-W615` | two samples at one (series, instrument, instant) with different values or flags |
| `SMY-W616` | an integer `sum` outside `dec`'s range; the field is omitted |
| `SMY-W617` | two spans with one (trace, span id) in one stream whose events differ; both are kept and both materialise (TL-P9) |

## 16. Conformance

New classes extend 2.0's C-Read … C-Library:

| Class | Requires |
|---|---|
| **C-Tel-Read** | decode `dec` canonically and refuse non-canonical forms; preserve every `x.tel` type, relation and field under rule X |
| **C-Tel-Produce** | reproduce the uids of `fixtures/tel/`: the diff fixtures (§8), the `dec` fixtures, the series-key fixtures (§6), and window evidence from the substrate fixture |
| **C-Tel-Check** | rule TQ, rule TW, derived kinds 6 and 7, and §15's codes, on `fixtures/tel/check/` |

The three ports (`python/`, `nodejs/`, `go/`) target C-Tel-Read, as they hold C-Read today.
`dec` is the only thing they must learn.

---

# Part B — Implementation

## 17. Prerequisites from the architecture proposal

SMYSL-3 needs four changes from the architecture proposal before its own code starts. They are
TL-P0.

| Need | Change | Why SMYSL-3 cannot do without it |
|---|---|---|
| Rule E without the text library | **AC-4**, `smysl-time` | The time engine is in `smysl-text/src/time/` (3.1 k lines), and `check` reaches it through an optional dependency on `smysl-text`. Telemetry needs rule E and nothing else from text. |
| Queries over a paged store | **AC-1**, `Env` over `&AnyStore` | `query::plan::Env` holds `&Store`, so `sq`, dossiers and measures load everything into memory. A telemetry library is the first that will not fit. |
| A series port | **AC-10**, `Env::series: Option<&dyn SeriesSource>` | Stages that need the substrate are refused by name without it, as `~sem` is without `semantic` |
| A live ledger | **AC-8**, incremental derived state (**done in 1.17**, §17.1); **AC-9**, concurrent readers | OKO appends one unit at a time and reads immediately. At the baseline the next read rebuilds the adjacency in O(store), and a second process waits 2 s for redb's lock. |

Also part of TL-P0:

- **The PRNG.** Move `smysl-measure/src/rng.rs` (ChaCha20 per RFC 8439, keyed from the recipe
  hash, already cross-checked by `nodejs/test/measure_rng.test.js`) into `smysl-core::rng`, behind
  a feature, so that `smysl-tel` does not depend on `smysl-measure`'s text and query stack.
- **The PRNG ban (H-7).** `xtask purity` fails a pure-tier crate that names `thread_rng`,
  `rand::random`, `StdRng`, `SmallRng`, or a `HashMap` / `HashSet` with the default `RandomState`
  on a path that produces records. `BTreeMap` or a fixed hasher replaces them.
- **Cross-architecture replay.** `xtask determinism` gains a run of the `fixtures/tel/` pipeline
  on x86_64 and aarch64, compared byte for byte (GE-M3).

### 17.1 AC-8: a store's check kept as derived state

The architecture proposal that names AC-8 is not in the repository, so this section is its
design, as implemented in 1.17.

**What it measured.** `make oko-ledger` commits one ledger operation at a time, staged and
checked against the whole store, as §24 says OKO will. At 6 × 10³ units a commit took 43 ms at
p50 against 5.8 ms at the start, 30 ms of it staging. A profile of the late commits found every
cost O(store):

| Where | Share | Why it grows |
|---|---|---|
| rule E's solve, in `check` and again in the range index's append | ~40 % | the whole network is solved: `findings` per axis, and `axis_on_append` for every batch that bears on time, as `x.tel/*` edges do |
| the ten passes' loops | ~25 % | each pass iterates `units()` |
| `Overlay::new` and its drop | ~15 % | the interpretive layer and the library are cloned for every batch |
| the adjacency | ~5 % | rebuilt over the union |

The log's `fsync` is per commit and constant; AC-8 leaves it.

**The contract is exact.** `Checked` holds the report of the store as it stands. For a batch
`B`, `checked.with_batch(S, B, opts)` equals `check(&Overlay::new(S, B), opts)`: the same
diagnostics, messages and counts. `crates/smysl-check/tests/incremental.rs` holds it over every
fixture store the check is pinned on and over generated stores, each dealt at random into a base
and up to five batches. `examples/oko_ledger --verify` holds it after every operation. A batch
the recomputation cannot bound is checked in full: a fallback costs time, never correctness. A
weaker contract (report only what the batch touches) was rejected: it would change what staging
reports, and could not be tested against anything.

**What is checked is what is committed.** `stage::prepare_checked` checks `Staged::records()`,
attestations included, and the caller's `extra` records (OKO's investigation View). The other
`prepare` functions check the units, relations and declarations only, as they did. The cache
advances by exactly the records appended.

**The design.** A sub-store "view" of the batch's neighbourhood was ruled out: dangling
references, `b3:` prefix resolution and transitive grounds all need the whole store, and a
restricted adjacency would report spurious `SMY-E060`. The passes instead run over the whole
union, restricted in what they iterate. The cached report is kept by rule family:

| Family | Codes | Recomputed for |
|---|---|---|
| per unit | shape, granularity, closure, E030, trust, W010, E603, E607, E612 (key 7), W606, W609, W057, unit E403, E404, W405, W411, E410/W449 on `published` | the dirty units: the batch's; those grounding on a new unit or a new commit's subject; the endpoints of new relations; the subjects of new attestations; units whose body has a `b3:` token that is a prefix of a new uid |
| dangling | E060 (subject: the missing uid) | uids the batch adds or references |
| per record | W432, W409, W014, E612, E611, E452, E012, manifest E403, E410/W449 | the batch's new records, added |
| aggregates | W013, W614, W418, W111 | the keys the batch touches: a relation kind, a window key, an alias |
| cycles | E061, W062 | the strongly connected components a new edge can close |
| time | rule E | the constraint components the batch touches (below) |

**Rule E by component.** Two subjects are in one component when a live dating or a free
constraint names both. The engine solves each component as its own network, in the full check
too, with its subjects, datings and free constraints in their original order: no edge joins two
components, and a tie between causes is broken by their relative order, so the answer is the
one network's. The one input that reached across components was the search's cutoff on a
negative cycle, which counts the network's variables. `tests/time_algebra.rs` adds unrelated
units around 40 stores whose strict datings close contradictory cycles and finds rule E's answer
about each unchanged, under the old single network as under the split. The cutoff was not seen
to change a finding.

In a store with no datings and no manifests, only `replies` and `quotes` relations tie two units,
and every rule E finding is a unit's. There `engine::findings_within` solves a region: the
closure, over those relations, of the batch's units, the subjects of its attestations and the
ends of its new ties. The kept check keeps the relations under each end and re-solves only that
region. The range index's `axis_on_append` does the same and rewrites only the region's rows. A
store with datings or manifests is solved whole, as before: that is the text library's shape,
where appends are few and large, and not the ledger's.

**Fallbacks**, each recorded in the report's `path`: options other than the cache's; a View
whose granularity profile the store does not have; a schema declaration the store does not hold;
a withdrawal, retraction, commitment or resolution that can change a dating's liveness, which is
transitive through rule R.

**Steps.** (1) This design, `Checked` with the full check on every batch, `prepare_checked`, the
harness and `--verify`. (2) Per-unit, dangling, per-record and aggregate families. (3) Rule E
by component, in `check` and in the range index. (4) `Overlay` without cloning the store's
interpretive layer and library; cycles from the new edges. (5) Measured at 10⁴ and 10⁵ units.

| `make oko-ledger`, 1,000 investigations, late commits, p50 | stage | commit | run |
|---|---|---|---|
| before AC-8 | 30.4 ms | 43.2 ms | 167 s |
| step 2 | 14.6 ms | 27.2 ms | 107 s |
| step 3 | 2.9 ms | 6.9 ms | 39 s |
| step 4 | 0.21 ms | 4.0 ms | 28 s |

Step 4's `Overlay` reads through its base's interpretive layer and holds only the batch's
entries, each merged with the base's entry for its key; the merged layer is built whole only for
a caller that asks for the layer itself. `SMY-W062` is recomputed only when a new causal edge
closes a cycle or points at a node on or upstream of one. What remains of the commit is the
append's `fsync`.

**Measured at §24's sizes (step 5).** At 10⁴ units, 11,661 operations with `--verify`: every
batch the kept check took equals the full check; commit 3.3 → 4.5 ms. At 10⁵ units, 116,224
operations: staging 0.15 → 0.63 ms, commit 4.0 → 7.7 ms, against the several hundred
milliseconds the trend before AC-8 gave. Of the late commits at 10⁵, 43 % is the `fsync`, 41 %
the range index's tables, which `axis_on_append` and `observed_on_append` rewrite whole on every
append (a sorted row vector and a segment tree, O(rows) in copies), and 8 % staging, which
splices a report of O(findings). The range index's tables are the text library's derived state
and need another structure to grow by less than their size; that is left open, outside AC-8.

## 18. Crates

```
core ─ graph ─┬─ time (AC-4) ──────────────┐
              ├─ retrieve (AC-6) ──────────┤
              ├─ series   ← new            │
              └─ tel      ← new: depends on core, graph, time, series; readers behind features
query ── (Env.series port) ── series
text  ── (unchanged; postmortems and chats keep using it)
facade: `smysl tel …` behind feature `tel`
```

| Crate | Tier | Holds | External dependencies |
|---|---|---|---|
| `smysl-series` | pure | the chunk format, the series store (redb tables), window reads, eviction, `SeriesSource` impl | `redb` (already in the workspace), `blake3` |
| `smysl-tel` | pure core; readers behind features | `x.tel/v1` registration and helpers, `dec`, profiles, canonical diff, readers, materialiser, detectors, investigation helpers | `serde_json` (reader-json), `prost` and `opentelemetry-proto` (types only, no serde) for `reader-otlp`; `sketches-ddsketch`; no C |

`dec` itself (the field type, codec and parser) lives in `smysl-core`, because it is a format
type.

## 19. The series substrate

### 19.1 What it is not

It is not a time-series database (§27). It stores samples well enough to materialise evidence,
answer "value in force at t", and re-derive detectors. It does not serve dashboards.

### 19.2 Chunks: a local layout, not an identity

Samples (§9.1) are stored in chunks. A chunk holds one (series uid, instrument uid) stream over one
**epoch-aligned** span: `floor(ts_ms / span) × span`. The span is chosen per series from
{1 min, 15 min, 2 h} by its sample rate, and recorded in the `series` table.

| Column | Encoding |
|---|---|
| timestamp ms | delta-of-delta, zig-zag varint |
| ns remainder | optional; `u32` varint; present iff any sample in the chunk has flag 8 |
| value | `i64` delta varint while every value is an integral `dec` within `i64`; otherwise each `dec` as its CBOR pair, with a mantissa XOR stream when exponents repeat |
| flags | run-length |

Chunks are **local**. Their bytes depend on span choice and on encoder version, so they are not
part of any identity and are never compared across peers. What is compared is samples, through the
window digest (§9.1).

### 19.3 Tables (redb, in `<library>/index/series.redb`)

| Table | Key → value |
|---|---|
| `chunks` | (series uid, instrument uid, span start ms) → chunk bytes |
| `series` | series uid → (span, first ms, last ms, sample count, class) |
| `evicted` | (series uid, instrument uid, span start ms) → eviction instant |
| `meta` | format version, profile hashes seen |

- **Adding samples** merges them into the target chunk as a set (§9.1), and rewrites the chunk.
- **"Value in force at t"** costs one key lookup and one chunk decode. GE-M4 measures it against
  holds.
- **Window reads** decode the overlapping chunks, take the samples in `[from, to)`, and compute
  the digest of §9.1.

### 19.4 `SeriesSource` (the AC-10 port)

```rust
pub trait SeriesSource {
    fn series(&self, sel: &SeriesSelector) -> Result<Vec<Uid>, Error>;          // via x.tel/series units
    fn window(&self, s: &Uid, inst: &Uid, from_ms: u64, to_ms: u64)
        -> Result<WindowRead, Error>;                                           // samples in order + digest
    fn at(&self, s: &Uid, inst: &Uid, t_ms: u64) -> Result<Option<Sample>, Error>;
    fn present(&self, s: &Uid, inst: &Uid, from_ms: u64, to_ms: u64) -> Coverage; // for W605
}
```

The trait is object-safe on purpose, unlike `StoreRead`, because a query holds it optionally as a
port.

### 19.5 Exchange

- `lib export --with-substrate` writes samples, not chunks, under `substrate/`. Each file holds
  one (series, instrument, UTC day) as a CBOR sequence of §9.1 sample arrays in sample order, and
  is named by the BLAKE3 of its bytes.
- Import adds the samples as a set, so substrate exchange is commutative, associative and
  idempotent.
- A substrate listing records the (series, instrument, day) triples the export holds and those it
  knows are evicted.
- `bundle` carries no substrate. Evidence units are self-contained, which is why they are
  materialised.
- Events (§19.6) travel the same way (TL-P9): one file per (stream, instrument, UTC day), a CBOR
  sequence of event arrays in event order, named by its BLAKE3. The listing gains three lists
  after the sample days: the streams `[stream, scheme, key, kind]`, the event days held, and
  those evicted. Import re-indexes each span under its trace id.

### 19.6 The event substrate (TL-P9)

Spans and log records are readings, but not numbers. They live beside the samples, in the same
redb file, and follow the same rules.

An **event** is the tuple (stream uid, instrument uid, `ts_ms`, `ns`, body), where:
- the **stream** is an `x.tel/series` unit of kind `span`, `log` or `event`, keyed by §6
- `ts_ms` and `ns` are the record's instant (a span's start), `ns` the remainder 0–999,999
- the **body** is the reader's canonical record (§20.1), encoded by the kernel's deterministic
  CBOR. It holds everything the reader keeps, the full-resolution times among it.

**Event order:** `ts_ms`, then `ns`, then the body bytes. **The substrate is a set of events**:
two equal tuples are one event, so reading an export twice, or the same trace from two
collectors' files, adds nothing.

**Digest of events** (one event, for a span; a window `[from, to)`, for templates):

```
digest = BLAKE3-256( "smysl/tel-events/1" ‖ 0x00 ‖ cbor_array( e₁, e₂, … ) )
eᵢ     = cbor_array( ts_ms, ns, body )        body as a CBOR byte string
```

| Table | Key → value |
|---|---|
| `events` | (stream uid, instrument uid, hour start ms) → the hour's events, a CBOR sequence in event order |
| `evicted_events` | (stream uid, instrument uid, hour start ms) → eviction instant |
| `streams` | stream uid → (kind, scheme, key, first ms, last ms, event count) |
| `tags` | (trace id, stream uid, instrument uid, hour start ms) → (), so a trace is read without reading every stream. An entry outlives its hour's eviction, which is how a reader tells an evicted trace from one never received. |

A stream's row is its own, not a `series` row, because a stream has no chunk span. Hours are a local layout, as chunks
are (§19.2): what is compared is events, through the digest.

## 20. Readers and profiles

### 20.1 Readers (first cut, D-T2)

| Reader | Input | Goes to |
|---|---|---|
| `k8s/1` | `kubectl get -o json` lists or watch-event JSON lines | object snapshots → parts + manifests; changes per §8; `status` gauges routed to series if the profile says so |
| `prom/1` | Prometheus text 0.0.4 and OpenMetrics 1.0 text, one scrape per file, scrape instant given | series substrate; `stale` from the stale NaN `0x7ff0000000000002`; `reset` from `_created` / start timestamps when present, else from decreases (`derived`) |
| `otlp/1` | OTLP metrics protobuf (`ExportMetricsServiceRequest`) | series substrate; `reset` from `start_time_unix_nano` changes (`measured`); `stale` from `FLAG_NO_RECORDED_VALUE` |
| `otlp-json/1` | OTLP/JSON | as `otlp/1`. **Our own JSON reader**: `opentelemetry-proto`'s `with-serde` drops spec-compliant string `asInt` values [A §5, probe]. |
| `zabbix/1` | Zabbix API JSON exports: `host.get`, `item.get`, `history.get`, `trend.get`, `problem.get`, `event.get`. The caller records the API version it saw (`apiinfo.version`: 6.x, 7.x, 8.x) beside the export, and the reader refuses an export without it. | hosts and items → parts (config changes per §8); history → series substrate; **trends → `x.tel/window` with `basis: platform`, `cited` by rule TW** (Zabbix computed them, not us); problems and events → `x.tel/alert` at `cited` |

**Trace and log readers (TL-P9).** They write the event substrate (§19.6) and an `x.tel/series`
unit per stream, and no other unit.

| Reader | Input | Goes to |
|---|---|---|
| `otlp-traces/1` | OTLP traces protobuf (`ExportTraceServiceRequest`) | span streams (§6), one event per span. Body: `[trace_id, span_id, parent_span_id, name, kind, start_unix_nano, end_unix_nano, status_code, status_message, attributes]`, the ids as byte strings (`parent` empty for a root), the attributes as §5.5 JCS text. Span events and links are not kept in the first cut. |
| `otlp-traces-json/1` | OTLP/JSON traces | as `otlp-traces/1`, the ids decoded from OTLP/JSON's hex |
| `journald/1` | `journalctl -o json`, one entry per line | log streams (§6), origin `journald:<host>`. Instant: `__REALTIME_TIMESTAMP` (µs), refused when absent. Body: `[MESSAGE, PRIORITY or null, fields]`, `fields` the JCS text of every field not starting `__`, so the cursor and monotonic clock, which differ between reads of one entry, are not identity. A `MESSAGE` that is not UTF-8 (an array of bytes) is read lossily as the message and kept exactly in `fields`. |
| `cloudevents/1` | CloudEvents 1.0 JSON in structured mode: one event, a batch (a JSON array), or JSON lines | event streams (§6), origin `cloudevents:<source>`. `specversion` 1.0, `id`, `source` and `type` required. Instant: `time` (RFC 3339), refused when absent. Body: `[JCS text of the event]`, so a producer's retry is one event. |

- Readers read files and never the network. Collecting is the caller's job: OKO for Zabbix, a
  script or `kubectl` otherwise.
- This keeps the pure tier free of HTTP, and keeps the readers replayable from a fixture.
- An OpenMetrics 2.0 reader waits until 2.0 is final.

### 20.2 Profiles

A profile is an HJSON file, the same configuration format smysl uses elsewhere:

```hjson
{
  profile: "k8s-deployments/1"
  scheme:  k8s
  select:  { apiVersion: "apps/v1", kind: "Deployment" }
  noise:   [ "/metadata/managedFields", "/metadata/resourceVersion", … ]
  keys:    { "/spec/template/spec/containers": "name", "/spec/template/spec/containers/*/env": "name" }
  classes: {
    "/spec/replicas": config
    "/spec/template/spec/containers/*/image": config
    "/metadata/labels": membership
    "/metadata/uid": identity
    "/status/readyReplicas": gauge
  }
  route:   { gauge: series }          // gauges go to the substrate, never to units
}
```

- The profile's hash (canonical CBOR of the parsed profile) is part of every recipe that uses it.
- Profiles ship for:
  - Kubernetes Deployments, StatefulSets, DaemonSets, ConfigMaps (keys only by default), Services,
    Nodes, Pods (spec only)
  - Zabbix hosts and items
  - a generic `json/1` profile
- Pseudonymisation (2.4 §3.2) applies to declared pointers, for example user ids in labels.

## 21. Detectors and recipes

Every analysis output records a recipe:
- detector id and version
- parameters
- window
- the classes in scope
- the PRNG name and seed (when used)
- the reduction order

The recipe hash goes in the unit's `recipe` field and in its attestation's `recipe`, as SMYSL-2.6
measures already do.

| Detector | Output | Status | Phase | Notes |
|---|---|---|---|---|
| exact window summary | `x.tel/window` (exact) | measured | TL-P4 | §9 |
| DDSketch window summary | `x.tel/window` (sketched) | derived | TL-P4 | `sketches-ddsketch`; α recorded |
| CUSUM | `x.tel/segment` boundaries | derived | TL-P5 | own implementation (~80 lines) |
| PELT | `x.tel/segment` | derived | TL-P5 | `pelt` (MPL-2.0, the same licence as smysl) or own implementation; penalty recorded |
| robust z (MAD) | `x.tel/anomaly` | derived | TL-P5 | median and MAD over a declared baseline window |
| BARO-style scorer | `x.tel/hypothesis` ranking units, `Rung::Computed` | derived | TL-P5 | own implementation (under 300 lines) [B §5] |
| episode grouping | `x.tel/episode` | derived | TL-P5 | changes and anomalies within a gap bound, ordered by (time, uid) |
| Drain over canonical order | `x.tel/log-template` | derived | TL-P9 | GE-M3 for replay. Parameters below. |
| MinHash (ChaCha-seeded) | episode similarity, a ranking `tel similar` prints | derived | TL-P9 | GE-M8 decides whether it beats embeddings, and whether a label is ever proposed from it |
| causal discovery (PC, PCMCI, Granger) | hypotheses | **inferred** even though deterministic | not planned | edges rest on assumptions that do not reliably hold [B §5] |
| LLM agent | hypotheses | inferred / speculative, capped by TQ | TL-P6 | model hash recorded, as the provider layer does |

- BOCPD stays out until a crate without `SmallRng` exists, or until we write one.
- **Drain** (He et al. 2017, as Drain3 runs it) is our own implementation, recipe
  `tel/drain/1?depth=4&sim=0.4&children=100&masks=uuid,ip,hex,num`:
  - **Tokens** split on ASCII whitespace.
  - **Masks** are applied to whole tokens before parsing, in order: a UUID, an IPv4 address
    (with an optional port), a run of 8 or more hex digits (with an optional `0x`), and a
    decimal number become `<UUID>`, `<IP>`, `<HEX>`, `<NUM>`. Whole tokens, not substrings, so
    `id=42` stays a word.
  - **Depth 4** counts as Drain3 does: the root, a layer by token count, one layer by the first
    token, and the leaves. A token with a digit is routed as `<*>`, as is any token past 100
    children of a node.
  - **Similarity 0.4.** A message joins the cluster at its leaf with the most positions equal
    to its template, if that is at least 0.4 of its length, in integer arithmetic. Ties go to
    the cluster made first, which event order fixes.
  - **Equal templates** are one: two clusters whose templates end equal are one template, their
    counts summed.
  - **The message** is journald's `MESSAGE`, and for a CloudEvent its `type` followed by its
    `subject`.
- **MinHash** (`tel/minhash/1?k=128&seed=S`) is over an episode's typed features. Its members
  are the units joined to it by `x.tel/during`:
  - `series:<uid>` per member's series (a change's or recreate's `object`);
  - `change:<pointer>`;
  - `anomaly:<series uid>:<direction>` (`-` without one);
  - `alert:<system>:<key>`;
  - `template:<text>` for each template whose window overlaps the episode.

  It uses 128 functions `hᵢ(x) = (aᵢ·x + bᵢ) mod (2⁶¹ − 1)`. `x` is a feature's BLAKE3, its
  first eight bytes little-endian, reduced mod 2⁶¹ − 1. `(aᵢ, bᵢ)` are drawn by rejection from
  ChaCha20, keyed by `BLAKE3("smysl/tel-minhash/1\0" ‖ recipe ‖ seed)` with the seed as eight
  little-endian bytes. Similarity is the number of equal minima out of 128. `tel similar` prints
  it beside the exact Jaccard index and the neighbours' labels, and writes nothing. GE-M8 found
  it model-independent on simulated episodes, so a label shown from similar episodes is a
  reading of the store, never a unit.
- A detector that fails GE-M3 stays off the pure path until fixed.

### 21.1 Calibration

The level detectors' defaults (robust z's 3.5, CUSUM's `h = 5`, PELT's `penalty = 10`) are
textbook values for i.i.d. noise. On RCAEval's one-second telemetry they flagged 90 % (MAD) and
100 % (CUSUM, PELT) of fault-free windows (SMYSL-5 §15, R7). A default therefore says nothing
about how often a recipe will alarm on a given system. **Calibration** makes that a stated number
instead: given windows the caller holds fault-free, it sets the parameter to the least value that
at most a stated fraction of them alarm at. The value then goes into the recipe like any other
parameter, so the recipe hash still says exactly what ran. The defaults are unchanged.

- **Trial.** A trial is a reference window and the test window after it, read over each series
  in scope and per instrument. A trial **fires** at a value when any of its streams does:
  - `mad/1` at threshold θ fires when some test point's |robust z| against the reference
    exceeds θ;
  - `cusum/1` at `h` and `pelt/1` at `penalty` (their other parameters as given) fire when,
    run over the reference and test points together, they put a change point in the test window.
- **Ladder.** Values come from a fixed decimal ladder of ten steps a decade (1, 1.2, 1.5, 2, 2.5,
  3, 4, 5, 6, 8 × 10^e), from 10⁻² to 8·10²⁰. They are written as decimals, so a calibrated
  recipe reads as written and no libm `pow` is involved. A trial's **score** is the largest ladder
  value at which it fires.
- **Value.** The calibrated value is the least ladder value `v` such that at most `⌊rate · n⌋` of
  the `n` trials score `v` or above. `rate` is a decimal in (0, 1), and the floor is exact. Any
  trial that fires at or above `v` scores at least `v`, so on the given trials the bound holds at
  `v` and at every value above it, even where CUSUM's and PELT's alarms are not monotone in their
  parameter. A rate the ladder's top cannot meet is refused.
- **Per series.** One value per trial is set by the trial's least-moving series. A series that
  is constant over its reference window scores at the ladder's top for any change at all
  (SMYSL-5 §15, R8). Calibrating **per series** gives each `(series, instrument)` its own value,
  with the rate still held jointly over the trials, since a trial fires when any series fires at
  its own value:
  - for a per-series count `q`, each series takes the least value that at most `q` of its trials
    score at or above;
  - `q` is the largest count at which at most `⌊rate · n⌋` trials fire;
  - a series' count is capped one below the number of trials it scored in, so its value is never
    at or below all of its scores (the ladder's foot, a hair trigger no evidence supports);
  - a series that fired in no trial keeps the recipe's value and is reported as uncalibrated.

  None of these can add an alarm on the given trials.
- **Scope.** Calibration applies to `mad/1`, `cusum/1` and `pelt/1`. `episode/1` and `baro/1`
  have no threshold to calibrate, and are refused.

The library API is `smysl_tel::detect::calibrate` and `detect::calibrate_on` (over a substrate),
with `calibrate_each` and `calibrate_on_each` per series. `smysl tel calibrate` prints the
calibrated recipe, or one per series with `--each`, and writes nothing. The rate holds on the
trials given; whether it carries to another system is an empirical question (SMYSL-5 §15, R8).

## 22. Query and dossiers

### 22.1 `sq/1` additions

These are additions to SMYSL-2.9's grammar, through the same `x.query` vocabulary path that
`channels` took:
- `series("<selector>")` as a scope term. It resolves to series uids through `x.tel/series` units:
  `about series(prom:http_request_duration_seconds{service="checkout"})`.
- `type x.tel/change`, `type x.tel/anomaly`, … work today, through type filters.
- `observed during <range>` works today.
- `in investigation(<uid>)` follows `x.tel/during`.

A query never materialises evidence. Materialisation writes units, so it is a command (§23), and
dossiers ask for it explicitly (`--materialise`). With an absent `series` port, `series(…)` is
refused by name.

### 22.2 The incident dossier

A dossier template for "What happened to X during W":

| Section | Content |
|---|---|
| definition | the series units, with metadata |
| timeline | the `timeline` thread, by role |
| changes | `x.tel/change` and `recreate` in W, by effective time |
| first sign | earliest `anomaly` or `segment` boundary |
| evidence | window units, with witness counts (§7) and common transport |
| hypotheses | with effective status, TQ results, kind-6 contentions |
| counter-evidence | `rebuts` and conflicting witnesses (kind 7) |
| reported views | chat and postmortem units (SMYSL-2 speaker, holder, mode) |
| exploration | unused and unexplored evidence (§11.2) |
| open questions | unresolved contentions, `SMY-W605` gaps |

This also fills the fourth dossier term that SMYSL-2.5 §3.13 named and the baseline does not
compute: the independent-witness count (`smysl-query/src/dossier/mod.rs`, module comment).

## 23. CLI

The facade gains feature `tel` and one command group:

| Command | Does |
|---|---|
| `smysl tel read <reader> <files…> --profile P [--at T]` | read into parts, series and change units (staged, as `ingest` is) |
| `smysl tel window <series> <from> <to> [--sketch α] [--instrument I]` | materialise evidence |
| `smysl tel detect <recipe.hjson> --series … --during …` | run detectors, stage their units |
| `smysl tel calibrate <recipe.hjson> --series … --window REF-FROM-TO… --rate R [--each]` | a level detector's parameter from fault-free windows (§21.1), or one per series; writes nothing |
| `smysl tel at <series> <t>` | value in force at t, from the substrate |
| `smysl tel investigate open\|consult\|hypothesis\|probe\|action\|close` | the ledger operations, for scripts and OKO |
| `smysl tel evict --older-than <dur>` | rule TL eviction |
| `smysl tel trace <trace-id>` | materialise a trace's spans and their `x.tel/precedes` (TL-P9) |
| `smysl tel templates <stream> <from> <to>` | materialise a window's log templates (TL-P9) |
| `smysl tel similar <episode>` | rank episodes by MinHash similarity (TL-P9) |
| `smysl tel profile check <file>` | validate a profile and print its hash |

Every command takes `--at` for its attestation clock, as `ingest` does since 1.14. Nothing reads a
wall clock implicitly.

## 24. The OKO ledger profile

OKO uses smysl as a library, not through the CLI:

| OKO decision | SMYSL-3 mechanism |
|---|---|
| one ledger store per on-call rotation | one library; `rotation` on each investigation |
| investigations as views | `x.tel/investigation` + a View rooted at it (D-T8) |
| humans by configured handle | `AgentId` from the handle; model agents by provider and model hash |
| full redacted command output kept | probe and action `output` parts. Redaction before storing is OKO's; rule Z is for later removal. |
| confirmation on unsafe commands | `x.tel/action.confirmed_by` |
| Zabbix 6/7/8 demo source | OKO calls the API and hands exports to `zabbix/1` |
| alerts, investigations, telemetry API | `SeriesSource`, `StoreRead` / `AnyStore`, the investigation helpers in `smysl-tel` |

Expected size: a rotation's ledger is ~10⁴–10⁵ units. That fits the in-memory store, which is why
AC-8 (incremental derived state) is on OKO's critical path and AC-9 (concurrent readers) matters
when OKO's TUI and CLI or agent share a store.

The identity-stability policy of the architecture proposal (§5 there) has to be in force before
OKO's first persistent ledger.

---

# Part C — Delivery

## 25. Phases

| Phase | Content | Exit criteria |
|---|---|---|
| **TL-P0** prerequisites | AC-4, AC-1 (query part), AC-10 seam, AC-8 property test harness, rng move, PRNG ban, cross-arch replay job | `check` and `q` outputs byte-identical on all fixtures; purity passes with `smysl-time` |
| **TL-P1** format | `dec` in `smysl-core` and the three ports; `x.tel/v1` registered; timeline roles 49–59; kinds 6 and 7 derived-only; reserved key 7 and record 16 refused on write; the codes of §15 | C-Tel-Read on all four implementations; format spec updated for timeline roles and kinds 6–7; the A-7 proposal amended (§5.4) |
| **TL-P2** object snapshots | `k8s/1`, `zabbix/1` (hosts, items), `json/1` profile; canonical diff; change and recreate units | `fixtures/tel/diff/` uids stable on x86_64 and aarch64; **GE-M1 (a)** run |
| **TL-P3** series substrate | `smysl-series`; `prom/1`, `otlp/1`, `otlp-json/1`, `zabbix/1` history; reset and stale units | substrate fixture round trip; `tel at` p95 measured; **GE-M4** run |
| **TL-P4** evidence | materialiser; exact and sketched windows; rule TW; `tel-evidence` check; Zabbix trends as cited evidence | window uids reproduced by a second run and a sharded run |
| **TL-P5** detectors | CUSUM, PELT, MAD, BARO-style, episodes | **GE-M3** passes for each shipped detector |
| **TL-P6** investigations | investigation, hypothesis, probe, action, consulted; rule TQ; kind 6; timeline thread; incident dossier; OKO helpers | OKO POC writes a rotation ledger; **GE-M7** arms (a) and (b) run. Arms (c) and (d) run once TL-P5 has shipped the BARO-style ranking. |
| **TL-P7** instruments | instrument units, `via`, `measured-by`; witness counts; kind 7 | **GE-M6** run; D-T10 confirmed or reversed |
| **TL-P8** retention and exchange | rule TL eviction; `--with-substrate` export and import | **GE-M9** passes (simulated, `make ge-m9`: held) |
| **TL-P9** traces and logs | OTLP traces, `x.tel/precedes`; Drain; MinHash; journald and CloudEvents readers | **GE-M2**, **GE-M5**, **GE-M8** decide key 7, the causal constraint and similarity labelling (simulated, `make ge-m2`, `make ge-m5`, `make ge-m8`: key 7 not released; the constraint fails, rule E unchanged; similarity model-independent) |

TL-P2 and TL-P3 can run in parallel after TL-P1. TL-P6 needs TL-P4 but not TL-P5 to be built,
because an agent can hypothesise over evidence before any detector exists. GE-M7 is complete only
after both.

## 26. Experiments

Each experiment has a stop condition, stated before it runs.

| Id | Question | Design | Decides | Stop / pivot |
|---|---|---|---|---|
| **GE-M1** | Does D-T1 hold on real data? | I-1's cluster (kube-state-metrics, node_exporter, Kubernetes snapshots, Zabbix where available), 7 days. Count units per active series per day under change-only and class-based encodings. Check `dec` canonicalisation across exporters. | D-T1, H-3 defaults, D-T4 | **Normalised per series:** if class-based encoding exceeds a median of 1 unit per active series per day, or a p99 of 50, the class defaults or the noise filters are wrong. Revisit before TL-P3. |
| **GE-M2** | Is ms enough? | OTLP traces from DeathStarBench at production-like load; count sub-ms spans and same-ms siblings whose order matters | D-T5 (key 7) | If < 1 % are affected, key 7 is released to the general pool. **Run on simulated traces (TL-P9, `make ge-m2`): not released.** 5,000 traces, 72,177 spans: 66.7 % under 1 ms, **39.5 % affected**, nearly all store calls (memcached, redis) made in sequence by one handler. The substrate's nanoseconds ordered all 25,311 collapsed pairs as they ran. Key 7 is not assigned either: assignment waits on D-T5's trigger, and no rule reads sub-ms order between units (GE-M5). |
| **GE-M3** | Can the analysis layer be replayed? | Every shipped detector over `fixtures/tel/`, on x86_64 and aarch64, twice each | H-7 | Any byte difference in a `derived` unit keeps that detector off the pure path until fixed |
| **GE-M4** | Is a hold record needed? | "Value in force at t" from the substrate against simulated holds, on GE-M1 data extended to 6 months | D-T6 | If the substrate is correct and ≤ 50 ms p95 at 6 months, release record 16 |
| **GE-M5** | Does causal order catch skew? | Inject 1–500 ms skew on chosen hosts; derive constraints from spans | §10.4 | Detection below 90 % at ≥ 5 ms skew means the constraint is not worth a rule E change. **Run on simulated traces (TL-P9, `make ge-m5`): fails; rule E unchanged.** Detection as fixed (the skewed host named): 20.1 % at ≥ 5 ms, and 26.0 % with the response constraint, because two clocks that disagree do not say which is wrong. Seen at all (post hoc: an edge of the host violated by ≥ 2 ms): 63.6 % for §10.4's start-only constraint, which cannot see a leaf service running ahead or the root running behind; 100 % with the response constraint at ≥ 2 ms. With no skew, ±0.25 ms offsets violate 1.3 % of edges at ms resolution (18 of 20 runs). |
| **GE-M6** | Do independent instruments change conclusions? | The same faults seen through duplicated scrapers and through independent sources | D-T10 | If witness counting never changes a ranking, it stays a report |
| **GE-M7** | **Does smysl make agent RCA better?** | Train-Ticket (D-T12) with scripted deploys and config edits, faults injected after some of them. Arms: (a) agent alone; (b) agent writing into smysl with TQ, consulted and kind 6; (c) BARO-style derived ranking; (d) (b)+(c). Local models; one hosted arm, labelled (D-T13). | the design of §11 | Measured on top-k accuracy, **fabrications caught** (TQ failures that were real errors), and **unused-evidence rate**. If (b) does not reduce fabricated evidence relative to (a), §11 is wrong and is redesigned before TL-P7. |
| **GE-M8** | Do episodes recur detectably? | MinHash over typed feature sets against embeddings, on GE-M7's corpus and RCAEval | similarity labelling | If MinHash recall@5 < 0.6 where embeddings exceed 0.8, similarity is model-dependent by default. **Run on simulated episodes (TL-P9, `make ge-m8`): does not fire; similarity is model-independent.** 120 episodes of twelve fault types, two per service sharing half their core. recall@5: MinHash **0.970**, exact Jaccard 0.982, Model2Vec `potion-base-8M` 0.898. Over five other seeds: 0.968–0.987, 0.972–0.985 and 0.880–0.945. The corpus is made of shared identifiers, which favours sets by construction: renamed series and paraphrased alerts are what GE-M7's corpus and RCAEval would test. |
| **GE-M9** | Does retention keep the round trip? | 6 months of GE-M1-scale data, compressed into a test; evict per rule TL; export, import, merge with an unevicted peer | rule TL | Any P1 failure over records, or any substrate resurrection that changes a unit, blocks TL-P8. **Run on simulated data (TL-P8): held.** 20 gauges, 182 days, 5.2 M samples, 152 days evicted. No record changed at any step, and every window recomputed to the unit it is after the merge. A control peer with one sample more was caught as `SMY-E604`. |

**GE-M2 and GE-M5 on simulated traces (TL-P9 step 4).** The measures, fixed before the runs:
- **The traces.** A social-network service graph after DeathStarBench's: eleven instrumented
  services, one per host, with a client span on the caller and a server span on the callee for
  every call between them. Calls to memcached, redis and MongoDB are client spans only, as their
  drivers record them. Calls are made in stages: parallel within a stage, sequential across
  stages. Latencies are log-normal with published-order medians (a cache call ≈ 0.12 ms, MongoDB
  ≈ 1.2 ms, one network hop ≈ 0.08 ms). Each host's clock has a residual offset uniform in
  ±0.25 ms, as NTP within a data centre gives.
- **GE-M2.** The trace data goes through `otlp-traces/1`'s path into the substrate, and its spans
  are materialised. *A span is affected* when it has a sibling (same parent) that ran strictly
  before or after it (one ended before the other started) and the two units' `start_ms` are
  equal: their order is in the substrate's nanoseconds and not in any unit. Reported: the share
  of spans under 1 ms, and the share affected, over all spans and over server spans alone. The
  stop condition is the table's: under 1 % affected releases key 7.
- **GE-M5.** One host's clock is skewed by ±1, 2, 5, 10, 20, 50, 100, 200 or 500 ms, in turn
  for every host, over 300 traces each; and twenty runs with no skew. A run is checked at the
  unit level, as rule E would: an `x.tel/precedes` edge is violated when the child's `start_ms`
  is below the parent's. *The skew is detected* when the skewed host is the one host touching
  the most violated edges. *A false alarm* is a no-skew run with any violated edge. A second arm
  adds the response constraint, a child's `end_ms` at most its parent's, which a span unit can
  state but rule E's single instant cannot. The stop condition is the table's: detection below
  90 % at ≥ 5 ms keeps the constraint out of rule E.

**GE-M8 on simulated episodes (TL-P9 step 5).** The measures, fixed before the run:
- **The corpus.** Twelve fault types over one shop's services, ten episodes each, 120 in all.
  A fault type has a core of eight to ten features (anomalies on named series, changes at
  pointers, alerts, log templates). Half of each core is shared with one other fault type, so
  that two faults of one service look alike. An occurrence keeps each core feature with
  probability 0.7, and adds zero to four features from a background pool of thirty that any
  incident shows.
- **Each episode is a store's units:** its members joined by `x.tel/during`, and its templates
  overlapping it. Features are read from them by `smysl_tel::similar`, as `tel similar` reads
  them.
- **Three arms rank the other 119 episodes for each one:**
  - MinHash, 128 functions;
  - exact Jaccard, the quantity MinHash estimates;
  - embeddings: Model2Vec `potion-base-8M` over a text rendering of the same units, by cosine.
- **recall@5** is the share of the top five that are of the episode's fault type, averaged over
  the 120. Ties go to the smaller uid.
- **The stop condition** is the table's: MinHash below 0.6 where embeddings exceed 0.8 makes
  similarity model-dependent by default.

**Order:**
1. GE-M1, and GE-M3's harness. The harness exists from TL-P0, runs first on TL-P4's summaries, and
   then on each detector as TL-P5 ships it. Both are cheap, and they decide the shape.
2. GE-M7: expensive, and the reason for all of this.
3. The rest as their phases arrive.

**Data:**
- Public RCA corpora carry almost no change events [B §7], so the testbed corpus is ours.
- RCAEval (CC BY 4.0) is usable for the methods comparison.
- OpenRCA and the AIOps Challenge data are non-commercial and do not leave the lab.

## 27. Non-goals

- **A time-series database.** The substrate is for evidence, not dashboards or ad-hoc PromQL.
- **Collection.** No scrapers, no OTLP receiver, no Zabbix API client in smysl. Readers read
  files.
- **Real-time alerting.** SMYSL-3 is for study, history and investigation, not paging.
- **Truth.** A `derived` root-cause ranking is a computation, not a verdict.
- **Signing** (D-T11). It gets its own RFC.
- **Re-implementing OpenTelemetry.** We consume its data and emit none.

## 28. Risks

| Risk | Where it bites | Mitigation |
|---|---|---|
| Profiles become the real schema, and drift | §8, §20.2 | profile hash in every recipe; `SMY-W610` at a profile boundary; profiles versioned in the repo with fixtures |
| `consulted` volume in long investigations | §11.2 | batch per probe; it is a record per unit looked at, not per view refresh; measure in GE-M7 |
| Evidence units multiply when agents materialise freely | §9 | identity makes repeats free; the materialiser refuses windows narrower than the series' scrape interval |
| AC-1 or AC-8 slips, and TL-P0 blocks everything | §17 | TL-P1 (format) and the GE-M1/GE-M3 scripts do not need TL-P0, and start in parallel |
| redb lock (AC-9) unresolved when OKO needs two processes | §24 | OKO's POC runs the TUI as the single writer; readers use the log fallback (`SMY-W504`) until AC-9 lands |
| Detector crates pull non-portable RNGs | §21 | PRNG ban in `xtask purity`; own implementations for the first cut |

## 29. Remaining open questions

The research draft's questions are closed by §1. These are new, and none blocks TL-P0–TL-P2:

- **OQ-T1.** Should `x.tel/consulted` be a relation per unit, or one relation per probe pointing to
  a saved query whose result set is pinned by its record-set digest? The second is smaller, but
  makes "was unit X consulted" a query evaluation instead of a lookup. Decide by GE-M7's volume.
- **OQ-T2.** Do Zabbix trends deserve `cited`, or `derived` because their hourly aggregation is
  documented and reproducible? `cited` is chosen because we cannot re-derive them once history is
  housekept on the Zabbix side.
- **OQ-T3.** Should the substrate exchange (§19.5) have its own digest, a substrate-set digest
  beside `rsd/1`, so that two peers can tell cheaply whether their substrates agree?

---

## Appendix A — Changes from research draft 1

| Draft | This RFC | Reason |
|---|---|---|
| H-1: window evidence `measured` | exact `measured`, approximate `derived`, platform aggregates `cited` (rule TW) | a sketch is a transformation and a platform's aggregate is a document; rule T |
| Window evidence superseded by late data | windows over different sample sets coexist; a preferred window per key; digest over samples, never chunks | supersession by late data forks across peers |
| Series metadata reachable from the series | `-meta` units point at the entity; the entity points at nothing | the other direction is a hash cycle and moves the series uid |
| Witnesses among series of one metric | witnesses among series joined by `x.tel/same-quantity` | evidence about one series has one origin |
| H-5: canonical floats, maybe a format rule (T-A2) | `dec`, an exact decimal defined by `x.tel/v1`, with one text spelling; JSON numbers parsed exactly, never through binary64; kernel float untouched | the kernel float is binary32 quantised to 1/1024 and rejects binary64; A-7 is not accepted |
| H-8: consulted units recorded | `x.tel/consulted` relations from an `x.tel/investigation` unit | a relation needs a subject; OKO's views root at it |
| H-8: rival explanations need a `DetectionKind` | kinds 6 and 7, **derived-only** | 4 and 5 took the codes the draft might have used; derived keeps rule U trivial |
| H-10: retention as rule Z | rule TL, local eviction | rule Z propagates through merges |
| GE-M1: 1 M units/week for 50 nodes | per active series: median 1/day, p99 50/day | a fleet-size threshold does not transfer between fleets |
| §2 "what exists": `dev/1.9.0` | the baseline: rule E, library, `sq`, dossiers, redb, open enums, `ingest --at` all present | SMYSL-2 landed |
| Instrument as a `SourceKind` (T-A3) | instruments are units; a kind only if GE-M6 forces it | no format change needed |
| — | Zabbix reader, `x.tel/alert`, `probe`, `action`, `investigation`, OKO profile | OKO decisions |
| — | dependency on architecture changes AC-1, AC-4, AC-8, AC-9, AC-10 | measured state of the baseline |

## Appendix B — Corrections to SMYSL-2 found by this work, as of the baseline

| # | Item | State at baseline |
|---|---|---|
| 1 | H-17, `fmt --format cbor` writes surface text | **fixed** in 1.9 (CHANGELOG 1.9.0) |
| 2 | CLI `import` stamps a wall clock (`Hlc::now` in `cmd_import`) and puts the row number into identity, so it is not replayable | **open.** The telemetry readers do not use it; fix by giving `import` `--at` as `ingest` got. |
| 3 | `impl Display for SourceRef` omits `observed` | **open** (the impl does not mention it) |
| 4 | Surface `observed` capped at `i64::MAX` while CBOR accepts `u64` | **verify** against the current surface parser |
| 5 | Draft 3's telemetry table (§20) assumed change units and holds | superseded by D-T1 and D-T6 |
| 6 | The independent-witness dossier term (2.5 §3.13) is not computed | delivered by §22.2 |

## Appendix C — Sources

Primary sources are cited with URLs in [A] and [B]. The ones this RFC relies on:

- OpenTelemetry specification and OTLP: https://opentelemetry.io/docs/specs/otel/ ,
  https://opentelemetry.io/docs/specs/otlp/
- Prometheus exposition formats and OpenMetrics:
  https://prometheus.io/docs/instrumenting/exposition_formats/
- Kubernetes API concepts, resource versions and strategic merge keys:
  https://kubernetes.io/docs/reference/using-api/api-concepts/
- Zabbix API reference: https://www.zabbix.com/documentation/current/en/manual/api
- RFC 8785 (JCS), RFC 6902 (JSON Patch), RFC 6901 (JSON Pointer), RFC 8949 §4.2 (deterministic
  CBOR), RFC 8439 (ChaCha20)
- RCAEval: https://arxiv.org/abs/2412.17015
- OpenRCA (ICLR 2025):
  https://proceedings.iclr.cc/paper_files/paper/2025/file/d29b8d53678015079e1d245c023e49d2-Paper-Conference.pdf
- Kim et al., "Why Do AI Agents Systematically Fail at Cloud Root Cause Analysis?":
  https://arxiv.org/abs/2602.09937
- BARO: https://arxiv.org/abs/2405.09330
- DDSketch: https://arxiv.org/abs/1908.10693 ; PELT: https://arxiv.org/abs/1101.1438
- Gorilla (time-series compression): Pelkonen et al., VLDB 2015
- Shewchuk, "Adaptive Precision Floating-Point Arithmetic", 1997 (exact summation)
