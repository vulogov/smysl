#import "design.typ": *

// ═══════════════════════════════════════════════════════════════════════
#part(number: "XI", title: "Telemetry (SMYSL-3)")
// ═══════════════════════════════════════════════════════════════════════
//
// SMYSL-3 is the telemetry RFC. 1.17 builds its format phase, TL-P1, its object snapshots,
// TL-P2, its series substrate, TL-P3, its window evidence, TL-P4, its detectors, TL-P5, and
// its investigations, TL-P6. This part says what exists, so that a reader of the format knows it, and what
// does not. Every terminal block below was produced by the smysl 1.17 binary.

#chapter(number: 38, title: "What 1.17 has of SMYSL-3")

RFC SMYSL-3 (`Documentation/SMYSL-3_RFC/SMYSL-3_TELEMETRY.md`) asks whether smysl can hold the
history of changes in telemetry, so that people and models can study what changed, why, and how
sure anyone is. Its answer starts with a refusal. *Readings are not units.* One simulated day of
a 200-host fleet was 6.8 million change units, and all but 6,566 were gauge noise. Samples live
outside the log, and units are made for what is *asserted* about them: a configuration change, a
reset, an anomaly, a hypothesis, an action.

The RFC is delivered in ten phases. 1.17 builds the first six:

- TL-P1, the format;
- TL-P2, which reads object snapshots: Kubernetes objects and Zabbix hosts and items, and what
  changed between two reads of one;
- TL-P3, the series substrate: samples from Prometheus, OTLP and Zabbix history, kept outside
  the log, and the few facts about them that are units.
- TL-P4, window evidence: what a span of samples contained, as a unit anyone can recompute.
- TL-P5, detectors: regimes, anomalies, episodes and a ranking, each one replayable to the byte.
- TL-P6, investigations: the ledger an on-call rotation keeps, and the checks that hold its
  hypotheses to the evidence.

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

#section("x.tel/v1: nineteen types, eight relation kinds")

`x.tel/v1` is registered as `x.text/v1` is: a store that does not know it keeps every unit and
relation and enforces none of its rules. The format spec's §5.2 lists the names. Four types
describe what is measured and by what: `x.tel/series`, `x.tel/instrument`, and their `-meta`
units. Fifteen carry assertions about it, from `x.tel/change` and `x.tel/window` to
`x.tel/hypothesis` and `x.tel/action`, and the two TL-P9 adds for traces and logs, `x.tel/span`
and `x.tel/log-template`. Payload keys are prefixed `tel:`.

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
  `thread --derive timeline` proposes six of them from telemetry types, in time order:
  - a change or recreate is the `trigger`;
  - an anomaly, reset or stale is the `onset`;
  - an alert, or an investigation being opened, is the `detection`;
  - a hypothesis or probe is `diagnosis`;
  - an action is `mitigation`;
  - a closed investigation is `closure`.

  `escalation`, `recovery`, `impact`, `communication` and `follow-up` are authored. Units with
  no telemetry role, such as series, windows and kernel claims, are not steps of an incident.

- *`source` key 7 and record 16* stay reserved, behind experiments that decide whether they are
  ever needed. Neither is refused, because key 7 is inside a uid, but `check` names either one
  as `SMY-E612`.

#section("Reading a Kubernetes object, twice")

TL-P2 reads object snapshots. A *profile* says what an object's leaves are: which are
configuration, which are membership (labels), which name the object, and which are gauges that
belong to the series substrate rather than to units. It also lists the noise that changes on
every reconcile and means nothing. Eight profiles ship, and a profile's hash is recorded in
every manifest read under it:

#screen(caption: "$ smysl tel profile check k8s-deployments/1")[
```
profile   k8s-deployments/1
scheme    k8s
hash      b3:rtinj5g5oln36jym3u6ivceawl7jqms5e54ekmyd5yoy4xpi7kla
patterns  8 noise, 8 keyed, 11 classed, 9 int
```
]

The first read of a Deployment writes its snapshot and its series unit. There is nothing yet
to compare it with, so there are no changes:

#screen(caption: "$ smysl tel read k8s/1 deploy-1000.json --profile k8s-deployments/1 --at 1768557600000 --licence unknown -s lib --yes")[
```
manifest  m3:vlqtix6ad4h6tbm6dbhhmzj4cnrpalu3yqpjg2xeyyrxsniuchpq  k8s/apps/v1/deployment/shop/checkout
read      1 object(s), 0 not selected; 1 new snapshot(s), 0 unchanged; 0 change(s)
1 record(s) committed
```
]

Five minutes later the image has been bumped. Without `--yes`, nothing is written: the change
is staged for review and the manifest that would be written is printed.

#screen(caption: "$ smysl tel read k8s/1 deploy-1005.json --profile k8s-deployments/1 --at 1768557900000 --licence unknown -s lib")[
```
would add m3:7jnvv6zpcthn4eris7iiijqul3rynihsm6wayzrdhdjrl3akp7eq  k8s/apps/v1/deployment/shop/checkout
read      1 object(s), 0 not selected; 1 new snapshot(s), 0 unchanged; 1 change(s)
2 record(s) staged in ./.smysl/staged.smy for review; nothing was written to the library -
  run again with --yes to write the snapshots and commit them
```
]

The staged change says exactly what moved, from what, to what. Its source is the new
snapshot's part, with the leaf's pointer as the locator:

#screen(caption: "$ sed -n 3,4p .smysl/staged.smy")[
```
@x.tel/change { status: measured, source: { kind: doc,
  ref: t3:tuqciktgcusfxqvvyzai3y5teaxjq7gdma3oyzvsgs6vrgsumyiq#/spec/template/spec/containers/checkout/image,
  observed: 1768557900000, manifest: m3:7jnvv6zpcthn4eris7iiijqul3rynihsm6wayzrdhdjrl3akp7eq },
  "tel:op": replace, "tel:to": registry.example/checkout:2.14.1,
  "tel:from": registry.example/checkout:2.14.0, "tel:class": config, "tel:vtype": text, …
~ replace /spec/template/spec/containers/checkout/image
```
]

The same command with `--yes` writes that manifest, by the same id, and commits the change.
Three things the reading does not do:

- *It does not notice noise.* A snapshot whose `resourceVersion`, generation and condition
  times moved, and nothing else, has its predecessor's part and is nothing at all.
- *It does not count spellings.* `replicas: 3` and `replicas: 3.0` are one value, and keyed
  arrays in another order are one object.
- *It does not read secrets.* A Secret's `data` and `stringData` are removed before anything,
  the profile included, sees the object.

A Deployment deleted and recreated under the same name continues its manifest chain, because
the alias leaves out the uid. The snapshot after the gap therefore records an
`x.tel/recreate`, and the object's series, whose key has the uid in it, is a new one.

#section("Samples, and the facts about them")

TL-P3 reads *series*: Prometheus scrapes (`prom/1`, text 0.0.4 and OpenMetrics), OTLP metrics
exports (`otlp/1` protobuf and `otlp-json/1`), and Zabbix history (`zabbix/1` with `--items`).
Their samples go to the *series substrate*, `<library>/index/series.redb`, and never become
units. What does become a unit is what a sample shows about its series:

- `x.tel/reset`, when a counter starts again. It is `measured` when the source states a new
  start time (a moved `_created`, an OTLP start time) and `derived` when only a decrease shows
  it.
- `x.tel/stale`, `measured`, when the source says a series ended: a Prometheus series gone from
  the target, an OTLP point flagged `FLAG_NO_RECORDED_VALUE`.
- `x.tel/series-meta`, `cited`, for a Zabbix item's `itemid`, which is metadata, never identity.

Each reset and stale is joined to its series by `x.tel/of-series`. A gauge reading is never a
unit.

A scrape has no instant of its own, so `--at` gives it, and the target's `job` and `instance`
are part of every series key. The first scrape is samples and no facts. The NaN in it is kept
as a sample with no value, and counted:

#screen(caption: "$ smysl tel read prom/1 s0.prom --job node --instance h:9100 --at 1768557600000 -s lib --yes")[
```
smysl tel read: s0.prom: 1 sample(s) not a value (NaN, ±Inf or out of range), kept with flag 4
read      s0.prom: 10 series, 10 sample(s), 0 fact(s)
series    10 sample(s) added to the substrate, 0 already there
11 record(s) committed
```
]

Fifteen seconds later one temperature sensor is gone. Without `--yes` nothing is written, and
the fact is staged:

#screen(caption: "$ smysl tel read prom/1 s1.prom --job node --instance h:9100 --at 1768557615000 -s lib")[
```
read      s1.prom: 10 series, 10 sample(s), 1 fact(s)
series    10 sample(s) would be added; 1 fact(s)
12 record(s) staged in ./.smysl/staged.smy for review; nothing was written to the library -
  run again with --yes to add the samples and commit them
```
]

#screen(caption: "$ grep -A1 x.tel/stale .smysl/staged.smy")[
```
@x.tel/stale { status: measured, source: { kind: metric, ref: "series:prom:node_temperature_celsius{…}",
  observed: 1768557615000 }, "tel:at_ms": 1768557615000, "tel:marker": stale-nan,
  "tel:series": b3:inw6b5ighd2lrkgvmnrhygno6xsqg35k6zud2gquvx5uwu2mn3pq }
```
]

Committed, and followed by a scrape in which every counter fell, it gives three `derived`
resets. `tel at` gives the value in force at an instant: the last sample at or before it, read
from the substrate. A series is named by its uid, or by its scheme and key:

#screen(caption: "$ smysl tel at 'prom:http_requests_total{code=\"200\",instance=\"h:9100\",job=\"node\",method=\"get\"}' 1768557630000 -s lib")[
```
series    b3:y43mgnsmlhmxzker7l2bpvp5kb7k342dh5wi3xi7i5ydzohfmgbq  prom:http_requests_total{…}
value     b3:qmeztdgxzzr5sg7fowwamvyf5vl3vdmcbmrjmnshvclsjoreesfq  1768557630000  12
```
]

The second uid is the instrument, the scraper `prom-scrape:node/h:9100`. A series that went
away is not its last number. What is in force past a stale marker is for you to decide:

#screen(caption: "$ smysl tel at 'prom:node_temperature_celsius{instance=\"h:9100\",job=\"node\",sensor=\"b\"}' 1768557620000 -s lib")[
```
series    b3:inw6b5ighd2lrkgvmnrhygno6xsqg35k6zud2gquvx5uwu2mn3pq  prom:node_temperature_celsius{…}
value     b3:qmeztdgxzzr5sg7fowwamvyf5vl3vdmcbmrjmnshvclsjoreesfq  1768557615000  stale
```
]

Samples are a set, so reading one scrape or export twice adds nothing. The same OTLP export as
protobuf and as JSON is the same samples. OTLP and Zabbix name what they were read from with
`--instrument` (`otel-collector:c1`, `zabbix-server:main`).

*Is a "hold" record needed?* Record 16 was reserved for "value v held from t1 to t2", in case
the substrate could not answer "value in force at t" well enough (GE-M4). `make ge-m4` asks it:

- It writes six months of 20 gauges scraped every minute, 5.2 million samples.
- It keeps each run of one value as a hold would, and asks both 237,694 times: at random, and
  at and just before every hold's start.
- Every answer agreed. The substrate's p95 was 4 µs in process, and 50 ms is the bar.
- `tel at` as a command took 21 ms at p95 over 200 runs against that file. That includes
  starting the process and opening the 68 MB file.

The data is simulated, and the report says so. The RFC's run is on GE-M1's data, a real cluster
extended to six months, which 1.17 does not have.

#section("Evidence: what a window held")

Samples are not units, but an assertion about them can be. `tel window` materialises
*window evidence*: one `x.tel/window` unit saying what `[from, to)` of one series contained, as
one instrument recorded it.

#screen(caption: "$ smysl tel window 'otel:{…\"part\":\"sum\"…}' 1768557600000 1768557660000 -s lib")[
```
window    b3:vny6wgxqxqxzx6h4we4qj2z5ripqovilz2cwy6oc65zdoxlfcw2q  b3:fy3jdo…  [1768557600000,
  1768557660000)  substrate measured  count 3  min 0.05 max 1.4 mean 0.7833333333333333 sum 2.35
1 window(s); nothing was written - run again with --yes --at MS to commit them
```
]

Its payload carries the BLAKE3 digest of the samples it was computed from, so the same window
over the same samples is one uid, wherever and whenever it is computed. The summary is exact:

- The sum is an exact decimal, so `0.1 + 0.2` is `0.3`.
- The mean is the exact quotient, rounded once to the nearest binary64.
- The order the samples arrived in cannot change either.

`--sketch 0.01` asks instead for quantiles from a DDSketch accurate to 1 %. A sketch is a
transformation, so its window is `derived`, where the exact one is `measured`:

#screen(caption: "$ smysl tel window 'otel:{…}' 1768557600000 1768557660000 -s lib --sketch 0.01 --at 1768557660000 --yes")[
```
window    b3:aaagnggqhfq5efawqhpuks26xfsdylc7aqvcdjbksrgp2pqf3kwa  b3:fy3jdo…  [1768557600000,
  1768557660000)  substrate derived  count 3  q0.5=0.8967553887478583 q0.9=0.8967553887478583 …
2 record(s) committed
```
]

The sketch's buckets are computed without a logarithm, from a value's own binary exponent and a
table built by multiplication. So a value lands in the same bucket on every machine, and
sketches of shards merge to the sketch of the whole.

*Rule TW* fixes what a window may claim:

#dtable(
  (auto, auto, 1fr),
  (
    ([Window], [Status], [Because]),
    ([exact, from the substrate], [`measured`], [Computed from samples, recomputable from them.]),
    ([sketched, from the substrate], [`derived`], [An approximation of what was measured.]),
    ([a platform's aggregate], [`cited`], [Transcribed: a Zabbix trend (`tel read zabbix/1 --trends`) was computed by Zabbix, from samples smysl may never have had.]),
  ),
)

`tel check` holds a library's windows to rule TW and recomputes each from the substrate. A
window materialised before late samples arrived is still true of the samples it saw, and its
digest says which those were. The substrate no longer agrees with it, and that is what
`SMY-E604` reports. Here a window over three Prometheus scrapes was committed, and a fourth
scrape inside it was read afterwards:

#screen(caption: "$ smysl tel check -s lib")[
```
error: SMY-E604: recomputed from the substrate, the window's digest, count differ(s) (at b3:6yklrb74zgteqpsvijg3qvpvja)
checked   1 window(s): 1 error(s), 0 warning(s)
```
]

A window whose samples are gone is `SMY-W605`, "not re-derivable", and nothing more. Its status
does not depend on the substrate being there.

A window has no text form, because its digest is 32 bytes and the surface syntax has no byte
string. So there is nothing to stage for review: without `--yes`, `tel window` prints the uid it
would commit and writes nothing.

*A sample is its source's alone.* A reset is a fact about two readings, so it is a unit,
`x.tel/reset`, found when both are known. It is never a mark on the later sample. So the same
exports read in any order give the same samples and the same window uids. Prometheus staleness
is the one exception: a series that disappeared can only be seen by reading scrapes in order.

#section("Detectors")

A detector reads samples and writes what it finds as `derived` units. Each unit records its
*recipe*: the detector and its version, its parameters, and the window it ran over. A recipe is
an HJSON file:

#screen(caption: "$ cat pelt.hjson")[
```
{ detector: pelt/1, penalty: 10, min_size: 3 }
```
]

Five detectors ship:

#dtable(
  (auto, auto, 1fr),
  (
    ([Recipe], [Writes], [Finds]),
    ([`cusum/1`], [`x.tel/segment`], [Shifts in level, by Page's cumulative sum.]),
    ([`pelt/1`], [`x.tel/segment`], [The best segmentation of the level, exactly, for a recorded penalty per change.]),
    ([`mad/1`], [`x.tel/anomaly`], [Points far from a declared baseline window, by the modified z-score.]),
    ([`episode/1`], [`x.tel/episode`], [Changes and anomalies close in time, grouped.]),
    ([`baro/1`], [`x.tel/hypothesis`], [The series that moved most after an instant, ranked.]),
  ),
)

Over a cpu series whose level rises at 10:30, PELT finds two regimes:

#screen(caption: "$ smysl tel detect pelt.hjson --series 'otel:{…ge_m3_cpu…}' --during 1768550400000-1768564800000 -s lib --at 1768564800000 --yes")[
```
x.tel/segment     b3:zwvcmgpwvzffxudokb5a5nyuqrytxvzu24hxghm43mieiiz2et4a  segment {…ge_m3_cpu…} [1768550400000, …
x.tel/segment     b3:lxu2c3lg2ymkytaqs4zjabkfoei6rts5or2r2dfnghe6mddwpl6q  segment {…ge_m3_cpu…} [1768559400000, …
detect    pelt/1: 2 unit(s), 2 edge(s)
2 record(s) committed
```
]

The robust z, scored against the two hours before, flags the cpu shift and a latency spike, and
`episode/1` groups the two anomalies. The ranking then asks, at the spike, which series moved
most, and grounds each answer on the episode:

#screen(caption: "$ smysl tel detect baro.hjson --series 'otel:{…cpu…}' --series 'otel:{…latency…}' --trigger b3:dskbng2mxmjznghzwcfq6minyfpomz6n3ug6fybgqzg4rqk4zswq -s lib --at 1768564800000 --yes")[
```
x.tel/hypothesis  b3:jinfgmntif56t6catscbv4np4ud3swtk4ze6hojymtwirwq2ys3q  rank 1 of 2: {…ge_m3_latency…} moved …
x.tel/hypothesis  b3:ovwbgge6bkoyviht2n475fe2mjjn3crudzmgjlmqddowjjuhvqba  rank 2 of 2: {…ge_m3_cpu…} moved …
detect    baro/1: 2 unit(s), 0 edge(s)
2 record(s) committed
```
]

Without `--yes`, a detector's units are staged for review in `.smysl/staged.smy`, as `tel read`
stages, and nothing is written.

*Calibrated thresholds.* The defaults (MAD's 3.5, CUSUM's `h = 5`, PELT's `penalty = 10`) are
textbook values for independent noise, and on real one-second telemetry they alarm on almost
every quiet window: 90 to 100 % of them on RCAEval's (RFC SMYSL-5 §15). `tel calibrate` replaces a
default with a stated false-alarm rate. Give it windows you hold fault-free, each as a reference
and the test window after it (`--window REF-FROM-TO`). It returns the least value on a fixed
decimal ladder at which at most that share of them alarm, as a recipe `tel detect` takes. It
writes nothing. Here ten quiet half-hours before the 10:30 shift are used, at 10 %:

#screen(caption: "$ smysl tel calibrate cusum.hjson --series 'otel:{…ge_m3_cpu…}' --window 1768550400000-1768551600000-1768552200000 --window … --rate 0.1 -s lib")[
```
{ detector: cusum/1, k: "0.5", h: "3", baseline: 20 }
calibrate cusum/1: 1 of 10 trial(s) fire at 3 (at most 1 at rate 0.1)
```
]

With several series, one value is set by whichever barely moves, since a near-constant series
alarms on any change at all. `--each` gives each series its own value instead, and still holds
the 10 % over the windows as a whole. Adding `floor: resolution` to the recipe goes further: it
keeps a flat window's scale at the series' own step instead of near zero, so the value stays in a
sane range. `--margin 2` holds the windows to half the rate, leaving room for when the values are
used somewhere they were not set. The calibrated value goes into the recipe, so the recipe
hash still records what ran. The rate
holds on the windows given. Whether it carries to another system has to be measured; RFC SMYSL-3
§21.1 has the rule.

*Replayed to the byte.* A detector here uses only addition, subtraction, multiplication,
division and square roots, which every machine rounds the same way. It never uses a logarithm or
an exponential from the platform's maths library, and it draws no randomness. GE-M3 is the check:

- every detector runs twice over `fixtures/tel/detect/`;
- each output is pinned by uid;
- the pins are compared on x86_64 Linux, Windows and aarch64 macOS.

A detector whose bytes differ anywhere does not ship.

#section("Investigations")

An investigation is a ledger:

- the question that was asked, and when;
- what was looked at;
- what was proposed, probed and done.

Each operation is a record, written as it happens. `--as` names who did it, a person by handle
or a model agent, and `--at` says when:

#screen(caption: "$ smysl tel investigate open 'why did requests drop?' --rotation sre-eu -s lib --as human:alice --at 1768557670000")[
```
x.tel/investigation  b3:22pvtdrvtq4mhpllu7rqcw3gd2huxnt7cvpjfvlenq3okanje7oq  why did requests drop?
committed 1 unit(s), 0 edge(s)
```
]

`consult INV UNIT…` records each unit looked at. `hypothesis INV CLAIM` proposes an
explanation:

- `--explains` names what it explains;
- `--ground` names the evidence it rests on;
- `--quote` lists each number it quotes, as `<uid>#<field>=<value>`.

*Rule TQ* holds a quote to the evidence. A quote verifies only when the cited unit is among the
hypothesis's grounds and holds that value, spelt as it holds it. A hypothesis whose quote does
not verify is at most `speculative`, whoever wrote it. Here a model agent claimed the window's
maximum was 4000, and it was 40:

#screen(caption: "$ smysl tel investigate show b3:22pvtdrvtq4mhpllu7rqcw3gd2huxnt7cvpjfvlenq3okanje7oq -s lib")[
```
investigation b3:22pvtdrvtq4mhpllu7rqcw3gd2huxnt7cvpjfvlenq3okanje7oq  why did requests drop?
opened        1768557670000  closed (open)
hypotheses    3
  b3:jzm73arfdlfutd3mikbo54tijix6nbyfudlklkiixxcdxqtaneeq  inferred  a deploy at 10:00
  b3:ux5lu7gz4ra2btug355cy72gvqlpd2ywbaqakrrpuohrximxflxa  inferred  the counter reset: a restart
  b3:6fwg72yj4thjfrnrpafkktbmrlnjwgbddor4kg2c76hchrsvuvpa  speculative  traffic moved to another pod
    SMY-W606 b3:lecz7w2nblrwjtlnmxawgdocl2mgnsfqqwjp5f2qvlx5ueglmnna#max=4000: the cited unit holds 40, not 4000
probes        0  -
actions       0  -
consulted     1
unused        0  -
unexplored    0  -  (from 3600000 ms before the opening)
rivals        1
  k/cdhlgou2wwnbrmmjqgznatugrdu  over b3:lecz7w2nblrwjtlnmxawgdocl2mgnsfqqwjp5f2qvlx5ueglmnna  (Open)
```
]

The two hypotheses that stand explain the same window, and neither rebuts the other, so they
are a *rival explanation*. That is a contention like any other: derived from the records,
named by its content, and resolved by a person. The speculative one is not a rival. The report
also counts the two failure modes an investigation can be measured by:

- *unused* evidence: looked at, and cited by no hypothesis;
- *unexplored* evidence: windows and changes about a series a hypothesis targets (`--target`),
  from shortly before the question was asked, that nobody looked at.

`probe` and `action` record a command that was run, and what it printed, by the part that holds
it. `close` closes the investigation. `thread --derive timeline` lays the ledger out as a
timeline (Chapter 18).

#section("A ledger that grows one operation at a time")

A rotation's ledger is written as it happens: open, consult, probe, hypothesise, act, close, each
committed alone and read back at once. Staging checks each operation against everything already
in the store, and until 1.17 that meant running every check pass over the whole
ledger every time. The cost grew with the ledger: 5.8 ms per commit at the start of a rotation,
43 ms by its six-thousandth unit.

A program that appends this way keeps the store's check and stages against it:

```rust
let mut checked = Checked::new(lib.store(), &CheckOptions::default());
// for each operation:
let mut staged = stage::prepare_checked(
    lib.store(), &checked, units, relations, labels, schemas, &attest, extra,
);
let batch = staged.checked.take().expect("prepare_checked checks");
lib.append_records(&staged.records())?;
checked.advance(batch);
```

`prepare_checked` checks what will be committed: the batch's units and relations, their
attestations, and `extra`, the records the caller commits with them (an investigation's View,
say). Its report is the one the full check of the store with those records would give, exactly.
It reaches it by recomputing only what the batch can change: the units it adds and those that
rest on them, the subjects of its attestations, the ends of its edges, rule E over the components
its units are tied into. A batch that changes something global (a new schema declaration, a View
under a profile the store has not seen, a withdrawal, a manifest) is checked in full, and the
batch report says why. `advance` is the caller's word that exactly those records were appended.

`examples/oko_ledger` writes a rotation this way, and `make oko-ledger` runs it. With
`--verify` it also checks every operation in full and stops if the two reports differ; at
10#super[4] units, all 11,659 operations agreed. Commit times, at the median, over the first and
the last tenth of a run:

#table(
  columns: 4,
  [*Ledger*], [*Staging*], [*Commit*], [*Run*],
  [6 × 10#super[3] units, full check (1.16)], [1.4 → 30.4 ms], [5.8 → 43.2 ms], [167 s],
  [6 × 10#super[3] units, kept check], [0.10 → 0.21 ms], [3.9 → 4.0 ms], [28 s],
  [10#super[5] units, kept check], [0.15 → 0.63 ms], [4.0 → 7.7 ms], [789 s],
)

Most of what is left is the append's `fsync`, which every commit pays once. The check of a
batch still looks up what it needs in the store, but no longer walks the whole of it.

#section("Instruments: what measured a series")

A series is what is measured. An *instrument* is a thing on the way from where it was measured
to where smysl read it. Each reader says what it knows of that path, as relations:

- `x.tel/measured-by`, from a series to the instrument that measured it at its *origin*;
- `x.tel/via`, from an instrument to the next one the data passed through.

#table(
  columns: 3,
  [*Reader*], [*Origin* (`measured-by`)], [*Carried by* (`via`)],
  [`prom/1`], [the target, `prom-target:<job>/<instance>`, an exporter], [the scraper, `prom-scrape:<job>/<instance>`],
  [`otlp/1`], [the resource's process, `otel-sdk:<namespace>/<name>/<instance>`, an agent], [the collector named by `--instrument`],
  [`zabbix/1`], [the item's host, `zabbix-agent:<host>`, an agent], [the server named by `--instrument`],
)

An OpenTelemetry resource without a `service.name` does not say what measured it, and the
reader does not guess: two unrelated processes would become one origin. Such a series has no
`measured-by`, and `check` says so with `SMY-W609`. Counting witnesses takes it as its own
origin, so it counts as independent of every other series. That can overcount, which is why
`check` warns.

What no export says, a person declares with `tel declare`, authored by `--as` at `--at`.
That covers a further hop, an origin the reader could not see, an instrument's version, and
above all that two series measure one quantity:

#screen(caption: "$ smysl tel declare same-quantity 'prom:http_requests_total{…}' 'zabbix:host:DB 01/item:mysql.qps' -s lib --as human:alice --at 1768557700002")[
```
x.tel/same-quantity  b3:lutau6eld2ljjo4jsgthf3uaqu4qn4t7sx6atevzxck3uyybzxpq -> b3:y43mgnsmlhmxzker7l2bpvp5kb7k342dh5wi3xi7i5ydzohfmgbq
committed 0 unit(s), 1 edge(s)
```
]

`same-quantity` has no direction, so either order writes the same relation. `instrument ROLE
KEY VERSION` adds an instrument, and its version as an `x.tel/instrument-meta` read at `--at`.
`via` and `measured-by` name instruments by key. A key that two roles share is refused, and
you name the instrument by its uid instead.

*How many witnesses?* Two series are evidence about one quantity only when `same-quantity`
joins them. They are *independent witnesses* when two things hold:

- their origins differ;
- no `transform`, such as a recording rule, lies on both lineages.

Series that are not independent are merged, transitively, and each class counts as one
witness. A scraper, collector, pipeline or store that both lineages pass through is *common
transport*: it is reported, and merges nothing.

#screen(caption: "$ smysl tel witnesses 'prom:http_requests_total{…}' -s lib")[
```
quantity      2 series
witnesses     2
  1   b3:lutau6…  host:DB 01/item:mysql.qps  (measured by zabbix-agent:DB 01)
  2   b3:y43mgn…  http_requests_total{…}  (measured by prom-target:node/h:9100)
common        1
  b3:qzvqax…  store thanos:eu
duplicated    0
unmeasured    0
```
]

`duplicated` lists an origin that reached smysl by more than one path, such as one target
scraped by two Prometheus servers: one witness, carried twice. The count is a report, and it
changes no status (D-T10). GE-M6 tested making it a rule and kept it a report: lineage tells
which series are one witness, but not which of two disagreeing instruments is wrong. When they
disagree, that is a witness conflict, below.

*When witnesses disagree.* Take two independent witnesses of one quantity, each with a window
over one span by one recipe. When those windows disagree, `tel check` reports a *witness
conflict* (detection kind 7). Like a rival explanation, it is derived from the records, named
by its content, and resolved by a person. The two windows disagree when:

- an exact field that both carry differs (`count`, `min`, `max`, `first`, `last`, `sum`,
  `mean`); or
- a quantile differs by more than twice the larger accuracy, relative to the larger estimate.
  Two honest sketches of one value, each within `alpha` of it, can be no further apart.

Each series is represented by its *preferred* window for the span: the largest count, then the
smallest digest, over every instrument that carried it. A series is one witness however many
scrapers carried it, so one window speaks for it. Two processes reporting one gauge 1% apart
conflict on their exact windows. Their sketches at `alpha` 0.01 do not:

#screen(caption: "$ smysl tel check -s lib")[
```
contention k/c…  witness conflict over b3:dhhz3f…: b3:…, b3:… disagree on min  (Open)
checked   4 window(s): 0 error(s), 0 warning(s); 0 rival explanation(s), 1 witness conflict(s)
```
]

Declare a recording rule both pass through (`tel declare via … rule:disk`) and they become one
witness, which cannot conflict with itself. `tel investigate show` lists, for each series its
evidence is about, the quantity's witness count and common transport, and then the conflicts
on that evidence.

== Retention: evicting old samples

Samples pile up. Retention is a local decision: this library keeps six months, a peer keeps
thirty days. `tel evict` drops samples older than a bound, and nothing else.

```
smysl tel evict --older-than 30d --at 1771149600000 -s lib          # what would go
smysl tel evict --older-than 30d --at 1771149600000 -s lib --yes    # evict it
```

The substrate is cut into chunks by span (a minute, fifteen minutes or two hours, chosen per
series). A chunk goes when its whole span ends before the bound, so no sample younger than the
bound is ever removed. Each evicted chunk is recorded as evicted, locally. Spans, which a later
section reads, go the same way by the hour.

Object snapshots go the same way. The part text of a Kubernetes or Zabbix snapshot taken before
the bound is deleted, with two exceptions:

- each object's latest snapshot, however old, because the next `tel read` of that object diffs
  against it;
- any part another manifest names. A postmortem, a chat or a ticket is never evicted.

The snapshot's manifest and its change units stay, and they carry the values that changed.

What eviction does not do matters more:

- It writes no record. A unit, a relation and an attestation are never evicted, and a merge
  carries nothing about it to a peer.
- It is not a redaction. Rule Z removes text that must not be held. Eviction is a cache
  decision.
- Window evidence keeps its status. It was attested when it was computed, and it says what it
  says. `tel check` can no longer recompute it, and reports `SMY-W605`, "not re-derivable": a
  warning, never an error.
- An investigation that used such a window keeps its hypotheses as they were. `tel investigate
  show` ends with its *gaps*: each window it consulted or cited that can no longer be
  recomputed, as `SMY-W605`. These are the dossier's open questions.

Reading the same exports again, or merging the samples back from a peer that kept them, adds
them as a set. The windows recompute exactly as before.

== Exchanging samples

A collection carries records, and a window carries its own summary, so a peer can read the
evidence without the samples. To give the peer the samples too, export with the substrate:

```
smysl lib export --scope all --with-substrate -o exp -s lib
smysl lib import exp --accept -s peer
```

The samples go under `exp/substrate/`. That is one file per series, instrument and UTC day,
holding that day's samples in order, and named by the BLAKE3 of its bytes. Chunks are not
exchanged: their layout is local. The same samples therefore always write the same file,
whoever writes it. `substrate/listing.cbor` lists the days held, and the days this library
knows it has evicted. A day partly evicted is listed in both.

`lib import` of such an export adds the samples as a set, after its records. It checks every
day file against its name, its day and the listing first. If any file fails, nothing is
imported, records included, and the import exits 1. Importing the same export twice adds
nothing. An export from a peer that kept what this library evicted brings those samples back,
and the windows over them check clean again. No unit changes. The listing's evicted days evict
nothing here.

The substrate exported is the library's whole substrate, whatever `--scope` selects. It sits
beside the collection, outside its index, so `lib verify` checks the records as before.
`bundle` carries no substrate: evidence units are self-contained, which is why windows are
materialised.

*Does retention keep the round trip?* That is GE-M9, and `make ge-m9` asks it:

- It writes six months of 20 gauges scraped every minute, 5.2 million samples, and one
  window per series per day, 3,640 window units.
- Library A keeps 30 days and evicts the other 152, and peer B is a copy that keeps everything.
- A exports with its substrate, and a fresh library C imports it. C then merges B's export,
  A merges it too, and a fourth library imports both in the other order.
- At every step the records were A's, byte for byte, and P1 and P3 held. After the merges every
  window recomputed to the unit it already was, and neither `SMY-W605` nor `SMY-E604` remained.
- As a control, a peer holding one sample more was caught as exactly one `SMY-E604`, and still
  changed no record.

It took 90 seconds. The data is simulated, as GE-M4's is.

#section("Traces: spans as readings")

A span is a reading, as a sample is, so reading a trace export makes no span unit. Each span is
kept as an *event* in the substrate, beside the samples: its trace and span ids, its parent,
name, kind, status, attributes, and its start and end to the nanosecond. One stream holds the
spans of one OTLP resource and scope, and is an `x.tel/series` like any other.

```
smysl tel read otlp-traces/1 t0.pb t1.pb --instrument otel-collector:c1 \
    --at 1768557700000 -s lib --yes
```

`otlp-traces-json/1` reads the same exports as OTLP/JSON, and gives the same events. Span events,
links and `trace_state` are not kept yet. The substrate is a set, so reading an export twice,
or one trace from two collectors' files, adds nothing.

A span becomes evidence when something asks for its trace:

#screen(caption: "$ smysl tel trace 4bf92f3577b34da6a3ce929d0e0e4736 -s lib")[
```
span      b3:h4an2g46dgfzbayrs7zcbcbdoeihfxomu2sra4l45czdsh52ammq  at 1768557600000  cart GET /cart
span      b3:w7eb4g7j2ld7363u7zeld73zm3dio3q7rmvp4nuvhfg46ievy7oa  at 1768557600001    cart POST /pay
span      b3:gtmeeujhyhoiuj6fcdvmvneceqilu3wojudmu7y5kzxjp2d4e32q  at 1768557600001      payment POST /pay
span      b3:tzdj25qhvuovs5sqrlrrbpyme46dghltezcsv2myigy727glmm6a  at 1768557600001        payment SELECT cards
4 span(s), 3 precedes edge(s); nothing was written - run again with --yes --at MS to commit them
```
]

Each line is an `x.tel/span` unit, `measured`, under its parent. Three spans start in the same
millisecond, and the tree still orders them, because the order comes from who called whom and
not from the clocks. With `--yes --at MS` the units are committed with an `x.tel/precedes` edge
from each parent to each child. A unit's digest is its one event's, so committing twice changes
nothing, and `tel check` recomputes each span from the substrate as it does a window:
`SMY-E604` if the span there is another one, and `SMY-W605` if it was evicted. A span id held
twice with different contents, because two exports disagree about it, is `SMY-W617`. Both are
kept.

The sub-millisecond part of a span's times stays in the substrate, and units hold milliseconds.
*Is a millisecond enough?* That is GE-M2, and `make ge-m2` asks it of 5,000 simulated traces of
a social-network service. Two thirds of the spans are under a millisecond. In 39.5 % of them, a
span and a sibling that ran before or after it share a `start_ms`, mostly cache calls made one
after another by one handler. The substrate's nanoseconds put every such pair in the order it
ran. So `source` key 7 is not released. It is not assigned either. It will be only when something
needs sub-millisecond order between units rather than events: a `check` rule that orders spans
across hosts, or a unit that must keep its order after its events are evicted. Nothing does yet.

*Could the order of calls catch a clock that is wrong?* That is GE-M5, and `make ge-m5` skews
one host at a time by 1 to 500 ms. A child span that starts before its parent says some clock
is off, but not which one, and some clocks it never catches:
- a service that calls nobody, running ahead;
- the entry point, running behind.

Adding "a child ends before its parent ends" catches every skew of 2 ms or more. Ordinary
quarter-millisecond clock error already breaks about 1 % of cross-host edges at millisecond
resolution. The experiment failed its bar, so `x.tel/precedes` stays an ordinary relation and
rule E does not read it.

#section("Logs: templates as evidence")

A log line is a reading too. `journald/1` reads `journalctl -o json`, and `cloudevents/1` reads
CloudEvents, such as the CDEvents a deploy pipeline sends. Each entry or event is kept in the
substrate as spans are, in one stream per host and unit, or per source and type:

```
smysl tel read journald/1 j0.json --instrument journald:h1 --at 1768557700000 -s lib --yes
```

The cursor and the monotonic clock, which change between two reads of one entry, are not kept,
so reading a journal twice adds nothing. What a person asks of a log is what it said, and Drain
answers that: it groups messages of one shape into a template.

#screen(caption: "$ smysl tel templates 'journald:{\"host\":\"h1\",\"unit\":\"cart.service\"}' 1768557600000 1768557660000 -s lib")[
```
template  b3:vtlizy6u73…  b3:xxomdilkiq…  count 4  connection from <IP> closed after <NUM> ms
template  b3:2civxfk47i…  b3:xxomdilkiq…  count 2  payment provider acme returned <NUM>
template  b3:ph7pk6xzun…  b3:xxomdilkiq…  count 2  retrying payment <UUID> in <NUM> ms
3 template(s); nothing was written - run again with --yes --at MS to commit them
```
]

Addresses, UUIDs, hex runs and numbers are masked before Drain reads a line, so `returned 502`
and `returned 503` are one template. With `--yes --at MS` each template is committed as an
`x.tel/log-template`, `derived`, with its count and the digest of every record in the window.
Drain's result depends on every line it read and their order, so the template is evidence
about the window, and `tel check` recomputes it from the window: `SMY-E604` if the records or
the templates have changed since, and `SMY-W605` once they are evicted.

Spans and log records travel with `lib export --with-substrate` as samples do, one file per
stream and day, and an import finds each trace again by its id.

#section("Episodes that look alike")

An on-call engineer's first question about an incident is often "have we seen this before?"
`tel similar EPISODE` answers it from the library. An episode's *features* are the series its
members are about, the pointers its changes touched, its anomalies with their direction, its
alerts, and the log templates over its window. Two episodes are alike as far as those sets
overlap:

```
smysl tel similar b3:… -s lib
```

Each line is another episode, with its MinHash similarity out of 128, the exact share of
features the two have in common, and its label if it has one. When the nearest carry labels,
the most common is printed as what this episode is labelled like. That is a reading, and
nothing is written.

*Does that find recurrences?* That is GE-M8, and `make ge-m8` asks it of 120 simulated
episodes of twelve faults. Each service has two faults that share half their signs. Of each
episode's five nearest, 97 % were the same fault by MinHash and 90 % by a small embedding
model reading a description of the same units. So similarity does not depend on a model.
The simulated incidents name things by the same identifiers each time, which favours sets.
Renamed series and alerts in other words are what real incident data would test.

#section("Information measures (1.18)")

A detector says *where* a series changed. An information measure says how much a window's
values are spread out, how far two windows' distributions are apart, or how much one series
tells about another. RFC SMYSL-5 adds five, each stated as an `x.tel/info` unit, `derived`,
from a recipe like a detector's:

#dtable(
  (auto, auto, 1fr),
  (
    ([Recipe], [`tel:stat`], [Says]),
    ([`ent/1`], [`entropy`], [How spread out a window's binned values are, in bits.]),
    ([`perm/1`], [`perm-entropy`], [How varied the order of successive values is: noise is near log#sub[2] m!, a constant 0.]),
    ([`jsd/1`], [`jsd`], [How far a window's distribution is from a reference window's: 0 when they are one, 1 bit at most.]),
    ([`mi/1`], [`mi`], [How much one series tells about another at a lag, or the best of a scan of lags, with surrogates to say whether chance would do as well.]),
    ([`tent/1`], [`template-entropy`], [How spread out a log stream's templates are over a window.]),
  ),
)

A series is first put on a grid of one value per `step`, and its values are binned into
symbols, all in exact decimal arithmetic. Only the final sum over counts uses floating point,
through one pinned maths library, and the result is stored at six places. So `tel info` gives
the same unit on every machine, which GE-I1 checks on the four CI platforms.

Over GE-M3's series, the last two hours of cpu are far from the first two, binned at the first
two hours' quartiles, and latency's are not:

#screen(caption: "$ smysl tel info jsd.hjson --series 'otel:{…ge_m3_cpu…}' --series 'otel:{…ge_m3_latency…}' --during 1768557600000-1768564800000 -s lib --at 1768564800000 --yes")[
```
x.tel/info        b3:uzewadeticaiijqgvpdg2ozygzxbhhfysb54grrbaio2ffjepevq  jsd {…ge_m3_cpu…}
    value 0.284594 bits, n 120, k 4
x.tel/info        b3:4vhizgivrwzfqgny3nf2cuc6rpxvmcp2ap25kwarlcpipvegg5aa  jsd {…ge_m3_latency…}
    value 0.012115 bits, n 120, k 4
info      jsd/1?bins=baseline:4&cover=4/5&hold=60000&min_n=100&of=value&ref=1768550400000-…: 2 unit(s), 0 refused
2 record(s) committed
```
]

The recipe text the unit stores writes every parameter, the defaults included, so it says
exactly what was computed. Here the file was
`{ stat: jsd/1, step: 60000, bins: "baseline:4", ref: "1768550400000-1768557600000", min_n: 100 }`.

A mutual information is easy to over-read. Two series of plain noise share some, by chance,
and more the more symbols there are. So `mi/1` stores a bias term beside its value, and with
`null: B` it rotates the second series by `B` seeded offsets and counts how many did as well.
cpu and latency are independent here, and the scan says so:

#screen(caption: "$ smysl tel info mi.hjson --series 'otel:{…ge_m3_cpu…}' --with 'otel:{…ge_m3_latency…}' --during 1768550400000-1768564800000 -s lib --at 1768564800000 --yes")[
```
x.tel/info        b3:zjwwj5yjj7eqbtch6dvn7tak3iii5e5dxlogw5apl2xakwomwiuq  mi {…ge_m3_cpu…} ~ {…ge_m3_latency…}
    value 0.050135 bits, at lag -1, bias 0.027164, n 239, k 4, k_with 4, 114 of 200 surrogates at least as high
    by lag -5:0.022187 -4:0.018284 -3:0.038701 -2:0.042784 -1:0.050135 0:0.020149 1:0.02754 …
info      mi/1?bins=rank:4&cover=4/5&guard=1/10&hold=60000&lag=-5..5&min_n=80&null=200&of=value&seed=7&step=60000: 1 unit(s), 0 refused
1 record(s) committed
```
]

*Which way?* A mutual information has no direction. Transfer entropy has one, and it was
tried and not released. GE-I5 simulated pairs where one series drives the other, and pairs
that one hidden series drives together. Transfer entropy named the right direction in every
driven pair. It also named a direction in about nine of ten pairs with a common driver, where
there is none to name. So `te/1` stays reserved, and `tel info` refuses it.

A window with too few values for its recipe is refused (`SMY-W703`) and writes nothing. A
counter read as a level, or a log stream read as a metric, is `SMY-E701`.

*Rule TI.* A statistic is `derived`, attested as transformed at the computed rung, and carries
the digest of every sample set it read: its window's, the second series' for `mi/1`, the
reference window's for `jsd/1`. `tel check` holds each statistic to the rule (`SMY-E603`) and
recomputes it from the substrate, as it does windows:

#screen(caption: "$ smysl tel check -s lib")[
```
checked   0 window(s) and 3 statistic(s): 0 error(s), 0 warning(s); 0 rival explanation(s), 0 witness conflict(s)
```
]

A sample that arrives late, inside a statistic's window, makes it `SMY-E604` on its digest. It
is still true of the samples it read, and a second unit over the new set stands beside it;
units of one key over different sample sets are `SMY-W614`. Once the samples are evicted, a
statistic is `SMY-W605` and nothing worse. A template entropy is recomputed from its template
units, so it outlives its log records for as long as its templates are kept.

*Quoting a statistic.* A hypothesis may quote a statistic's `value`, `mm`, `bias`, `n`, `k`,
`null_ge` or `null_b`, or one lag's value as `values[i]`, and rule TQ verifies the quote against
the unit. A quote gives the stored six places: `…#value=0.284594` verifies, and `…#value=0.28`
is `SMY-W606`. The dossier that `tel investigate show` prints lists each statistic among the
evidence with its value, its `n`, and its surrogates.

`cargo run --features tel --example tel_info` runs all of this over GE-M3's four series,
and `make oko-ledger` quotes two statistics in a rotation's ledger, once rightly and once
rounded.

#callout(label: "Not yet")[
  This part covers object snapshots, the series substrate, window evidence, detectors,
  investigations, instrument lineage, witness counting, witness conflicts, eviction, exchanging
  the substrate, traces, logs, similar episodes and information measures. The RFC's experiments on real data are
  still to run. So are:
  - ConfigMaps, whose values a profile cannot yet hide;
  - a generic JSON object profile;
  - what a deleted object becomes as a unit;
  - exponential-histogram buckets as series;
  - Zabbix problems and events;
  - the incident dossier as a `dossier` template (`tel investigate show` reports its numbers).
]
