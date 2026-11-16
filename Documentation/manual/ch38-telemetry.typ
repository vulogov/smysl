#import "design.typ": *

// ═══════════════════════════════════════════════════════════════════════
#part(number: "XI", title: "Telemetry (SMYSL-3)")
// ═══════════════════════════════════════════════════════════════════════
//
// SMYSL-3 is the telemetry RFC. 1.17 builds its format phase, TL-P1, and nothing that reads
// telemetry yet. This part says what exists, so that a reader of the format knows it, and what
// does not. Every terminal block below was produced by the smysl 1.17 binary.

#chapter(number: 38, title: "What 1.17 has of SMYSL-3")

RFC SMYSL-3 (`Documentation/SMYSL-3_RFC/SMYSL-3_TELEMETRY.md`) asks whether smysl can hold the
history of changes in telemetry, so that people and models can study what changed, why, and how
sure anyone is. Its answer starts with a refusal. *Readings are not units.* One simulated day of
a 200-host fleet was 6.8 million change units, and all but 6,566 were gauge noise. Samples live
outside the log, and units are made for what is *asserted* about them: a configuration change, a
reset, an anomaly, a hypothesis, an action.

The RFC is delivered in ten phases. 1.17 builds the first, TL-P1, which is the format and
nothing that writes it. There are no readers, substrate, detectors or `smysl tel` commands yet.

#section("dec: a number that is one value")

The kernel float is binary32 quantised to 1/1024, which is right for weights and wrong for a
latency of 0.0004 s or a counter above 2#super[24]. Telemetry gets its own number. A `dec` is
m × 10#super[e], written as a CBOR array of two integers, and one value has one encoding: no
trailing zero in the mantissa, zero as `[0, 0]`, the mantissa's magnitude at most
2#super[64] − 1, and the exponent between −400 and 400. Anything else is refused as `SMY-E602`.

#dtable(
  (auto, auto, 1fr),
  (
    ([Source], [Read as], [Why]),
    ([`41`, `41.0`, `4.10e1`], [`[41, 0]`], [A token is read exactly as written, never through binary64, so the spellings exporters disagree on are one value and one uid.]),
    ([`0.1 + 0.2` as a double], [`[30000000000000004, -17]`], [A binary64 is the decimal JavaScript's `String(x)` gives: the shortest that round-trips, and the closest of those.]),
    ([`NaN`, `+Inf`], [refused], [Not values. A stale series is a fact, recorded as one.]),
  ),
)

Every `dec` has exactly one text spelling: `[412, -1]` is `41.2`, `[15, -10]` is
`0.0000000015`. Rust, Python, JavaScript and Go agree on 2,030 doubles and on every token,
spelling and refusal in `fixtures/tel/dec/cases.json`.

#section("x.tel/v1: seventeen types, eight relation kinds")

`x.tel/v1` is registered as `x.text/v1` is: a store that does not know it keeps every unit and
relation and enforces none of its rules. The format spec's §5.2 lists the names. Four types
describe what is measured and by what: `x.tel/series`, `x.tel/instrument`, and their `-meta`
units. Thirteen carry assertions about it, from `x.tel/change` and `x.tel/window` to
`x.tel/hypothesis` and `x.tel/action`. Payload keys are prefixed `tel:`.

*A series is what is measured, not when.* Its identity is its scheme and its key and nothing
else, so the same series seen by fifty scrapes is one unit. A series with a time in it is not:

#screen(caption: "$ smysl check series.smy")[
```
series.smy: error: SMY-E607: an `x.tel/series` carries what its identity
  forbids: observed (at b3:yc6woq56lwmrstpndeshfyaaur) [try: a series or
  instrument has no span, manifest, published or observed time, and no
  grounds, and its source is exactly `series:<scheme>:<key>` (metric) or
  `instrument:<key>` (tool); what varies over time belongs in its `-meta`
  unit]
```
]

The same document without the timed series is clean:

#screen(caption: "$ smysl check one.smy")[
```
one.smy: 3 records, 1 units, 0 diagnostic(s)
```
]

#section("Four numbers the wire already had room for")

- *Detection kinds 6 and 7*, `rival-explanation` and `witness-conflict`, are derived from the
  record set and never written, as rule E's 4 and 5 are. A store refuses a record 6 of either
  kind, and `check` reports one that arrived some other way as `SMY-E611`.
- *Thread schema 5, `timeline`*, has eleven roles, from `trigger` to `follow-up` (Chapter 18).
  It has no derivation rules yet:

#screen(caption: "$ smysl thread --derive timeline one.smy")[
```
smysl thread: `timeline` cannot be derived from the graph; a timeline thread is
  authored; its derivation is not built yet
```
]

- *`source` key 7 and record 16* stay reserved, behind experiments that decide whether they are
  ever needed. Neither is refused, because key 7 is inside a uid, but `check` names either one
  as `SMY-E612`.

#callout(label: "Not yet")[
  None of this reads telemetry. The readers (Kubernetes, Prometheus, OTLP, Zabbix), the series
  substrate, window evidence, detectors, investigations and the `smysl tel` commands are the
  RFC's later phases, TL-P2 to TL-P9. What 1.17 fixes is what they will write, so that a store
  written by them is one that every reader from 1.17 on can carry.
]
