//! `fixtures/tel/check/` (RFC SMYSL-3 §16, C-Tel-Check): rule TQ, rule TW, the derived detection
//! kinds 6 and 7, and §15's record-set codes, pinned on six stores.
//!
//! Each case is a CBOR record sequence, `<case>.cbor`, that this file builds. `expected.tsv`
//! holds what an implementation must derive from each one, and nothing else:
//!
//! - `code`: one row per diagnostic, its code and the unit it names (`-` for a store-level
//!   finding). Exact in both directions, as `fixtures/conformance/check/` is: an extra code fails
//!   as a missing one does.
//! - `status`: each hypothesis's effective status after rule TQ's cap (§11.4).
//! - `contention`: each derived contention of kind 6 or 7, its id (spec §6.2), `over` and
//!   `positions`.
//! - `unit`: the label each uid was built under, so that a person reading a row can tell which
//!   unit it is. Not part of what an implementation derives.
//!
//! The cases:
//!
//! - `clean`: one series, its instrument, one exact window and one hypothesis quoting it
//!   correctly. Nothing at all.
//! - `tw`: rule TW (§9.4). Exact, sketched and platform windows attested as the rule asks, and
//!   one of each attested otherwise (`SMY-E603`). The exact one, `measured` but attested
//!   `Transformed`, also breaks rule T: `SMY-E033` (above its attestation's ceiling) and
//!   `SMY-W035` (`measured` but not `Imported`), the kernel's own findings on the same fault.
//!   Two exact windows of one key with different digests (`SMY-W614`).
//! - `tq`: rule TQ (§11.4). One quote that verifies, and one for each way a quote fails
//!   (`SMY-W606`): the wrong value, a non-canonical spelling, a unit outside the grounds, a
//!   missing field, an index past the list.
//! - `rivals`: kind 6 (§11.5). Two hypotheses of one target; three of another, as one
//!   contention; and four targets with no contention: one rebuts the other, one supersedes the
//!   other, one is only `speculative`, and one is capped there by rule TQ.
//! - `witnesses`: kind 7 (§11.5). Independent witnesses whose exact windows disagree; the
//!   preferred window chosen by `count` across instruments; one origin under two series; a
//!   transform on both lineages; sketches within and beyond 2·alpha; witnesses that agree; and
//!   a series with no origin (`SMY-W609`).
//! - `events`: rule TW over TL-P9's evidence (§9.6, §9.7). A parent and a child span, `measured`
//!   and ordered by `x.tel/precedes`, and a log template, `derived` and grounded on its stream,
//!   all attested as the rule asks; then a span attested `Transformed`, a span with no digest,
//!   and a template stated `measured` (`SMY-E603` each). The first also breaks rule T, as the
//!   `tw` case's exact window does (`SMY-E033`, `SMY-W035`).
//! - `info`: rule TI (RFC SMYSL-5 §3, TI-P4), on a store revision 2 declares. An entropy and a
//!   mutual-information scan attested as the rule asks, and a hypothesis quoting their `value`,
//!   `mm`, an element of `values` and `null_ge` at the stored six places, which verifies; a
//!   second quoting `value` to two places and with a trailing zero (`SMY-W606` each, so it is
//!   capped at `speculative`). Then an entropy stated `measured`, and a mutual information with
//!   no `with_digest` (`SMY-E603` each), each by a recipe of its own so that it shares no info
//!   key; the first, `measured` but attested `Transformed`, also breaks rule T (`SMY-E033`,
//!   `SMY-W035`), as the `tw` case's exact window does. Last, a second entropy of the first's
//!   info key over another sample set (`SMY-W614`).
//! - `reserved`: what no writer may emit, carried and reported: records 6 of kinds 6 and 7
//!   (`SMY-E611`), record 16 and `source` key 7 (`SMY-E612`), and a series that breaks §4.1's
//!   entity-like rule (`SMY-E607`).
//!
//! Recomputing window evidence (`SMY-E604`, `SMY-W605`) needs the substrate, which no record set
//! holds; `fixtures/tel/window/` pins that. The readers' codes are pinned by the reader fixtures.
//!
//! `SMYSL_BLESS=1` rewrites the files. That is a decision: C-Tel-Check is defined by them.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_check::{check, CheckOptions};
use smysl_core::cbor::major;
use smysl_core::cbor::writer::Enc;
use smysl_core::schemas::tel;
use smysl_core::types::{Contention, Detected, DetectionKind};
use smysl_core::{
    canonical_uid, from_cbor_seq, to_cbor_seq, AgentId, Attestation, ContentionId, Decimal, Hlc,
    Op, Record, RelKind, Relation, Rung, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCore,
    UnitCoreBuilder,
};
use smysl_graph::{effective_status, RetractionPolicy, Store, StoreRead};

/// 2026-01-16T10:00:00Z, and a minute after it.
const FROM: u64 = 1_768_557_600_000;
const TO: u64 = FROM + 60_000;

const CASES: &[&str] = &[
    "clean",
    "tw",
    "tq",
    "rivals",
    "witnesses",
    "events",
    "info",
    "reserved",
];

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/check")
}

/// One payload value, by its CBOR.
enum V {
    Text(String),
    Int(u64),
    Uid(Uid),
    Dec(&'static str),
    Decs(Vec<&'static str>),
    Texts(Vec<String>),
    Ints(Vec<i64>),
    Bytes([u8; 32]),
}

/// A payload map: text keys, sorted by their encoded bytes.
fn payload(entries: Vec<(&str, V)>) -> Vec<u8> {
    let dec = |t: &str| Decimal::parse_token(t).expect("a dec");
    let mut kv: Vec<(Vec<u8>, Vec<u8>)> = entries
        .into_iter()
        .map(|(k, v)| {
            let mut ke = Enc::new();
            ke.text(k);
            let mut ve = Enc::new();
            match v {
                V::Text(s) => ve.text(&s),
                V::Int(n) => ve.uint(n),
                V::Uid(u) => ve.text(&u.canonical()),
                V::Dec(t) => dec(t).encode(&mut ve),
                V::Decs(ts) => {
                    ve.array_head(ts.len());
                    for t in ts {
                        dec(t).encode(&mut ve);
                    }
                }
                V::Texts(ts) => {
                    ve.array_head(ts.len());
                    for t in ts {
                        ve.text(&t);
                    }
                }
                V::Ints(ns) => {
                    ve.array_head(ns.len());
                    for n in ns {
                        ve.int(n);
                    }
                }
                V::Bytes(b) => ve.bytes(&b),
            }
            (ke.into_bytes(), ve.into_bytes())
        })
        .collect();
    kv.sort();
    let mut e = Enc::new();
    e.head(major::MAP, kv.len() as u64);
    for (k, v) in kv {
        e.raw(&k);
        e.raw(&v);
    }
    e.into_bytes()
}

/// A quote of `unit` (§11.4): `<uid>#<rest>`, the uid in full.
fn quote(unit: Uid, rest: &str) -> String {
    format!("{}#{rest}", unit.canonical())
}

fn ty(t: &str) -> SchemaId {
    SchemaId::Extension(t.to_string())
}

fn agent() -> AgentId {
    AgentId::new("tool:smysl-tel").unwrap()
}

/// A window's shape: which rule TW row it is, and its numbers.
#[derive(Clone, Copy)]
enum Kind {
    Exact,
    Sketch,
    Platform,
}

/// One store under construction, with a label for every unit.
struct Case {
    records: Vec<Record>,
    labels: BTreeMap<Uid, String>,
    series_keys: BTreeMap<Uid, String>,
}

impl Case {
    fn new() -> Case {
        Case {
            // A store a revision-1 build wrote: C-Tel-Check is pinned on these bytes, and
            // revision 2 (SMYSL-5) changes nothing a revision-1 store holds.
            records: vec![Record::SchemaDecl(tel::decl_at(1).expect("revision 1"))],
            labels: BTreeMap::new(),
            series_keys: BTreeMap::new(),
        }
    }

    fn unit(&mut self, label: &str, u: UnitCore) -> Uid {
        let id = canonical_uid(&u);
        self.records.push(Record::Unit(u));
        self.labels.insert(id, label.to_string());
        id
    }

    fn rel(&mut self, kind: &str, from: Uid, to: Uid) {
        let k = RelKind::parse(kind).unwrap_or_else(|e| panic!("{kind}: {e:?}"));
        self.records
            .push(Record::Relation(Relation::new(k, from, to)));
    }

    fn kernel_rel(&mut self, kind: RelKind, from: Uid, to: Uid) {
        self.records
            .push(Record::Relation(Relation::new(kind, from, to)));
    }

    fn attest(&mut self, uid: Uid, op: Op, rung: Rung) {
        self.records.push(Record::Attestation(Attestation::new(
            uid,
            agent(),
            op,
            rung,
            Hlc::new(TO, 0, agent()),
        )));
    }

    fn instrument(&mut self, role: &str, key: &str) -> Uid {
        self.unit(key, tel::instrument(role, key).unwrap())
    }

    fn series(&mut self, key: &str) -> Uid {
        let s = self.unit(key, tel::series("prom", key).unwrap());
        self.series_keys.insert(s, key.to_string());
        s
    }

    /// A series measured at `origin`.
    fn measured(&mut self, key: &str, origin: Uid) -> Uid {
        let s = self.series(key);
        self.rel(tel::MEASURED_BY, s, origin);
        s
    }

    /// A window of `series` from `instrument`, at the status rule TW gives `kind` unless
    /// `status` overrides it, attested as rule TW asks unless `att` overrides it. `fields` are
    /// the window's numbers; `digest` its sample-set digest (a platform window has none unless
    /// one is given).
    #[allow(clippy::too_many_arguments)]
    fn window(
        &mut self,
        label: &str,
        series: Uid,
        instrument: Uid,
        kind: Kind,
        fields: Vec<(&str, V)>,
        digest: Option<u8>,
        status: Option<Status>,
        att: Option<(Op, Rung)>,
    ) -> Uid {
        let (recipe, basis, tw_status, tw_att) = match kind {
            Kind::Exact => (
                "tel/window-exact/1",
                "substrate",
                Status::Measured,
                (Op::Imported, Rung::Computed),
            ),
            Kind::Sketch => (
                "tel/window-ddsketch/1",
                "substrate",
                Status::Derived,
                (Op::Transformed, Rung::Computed),
            ),
            Kind::Platform => (
                "zabbix/trend/1",
                "platform",
                Status::Cited,
                (Op::Imported, Rung::Document),
            ),
        };
        let key = self.series_keys[&series].clone();
        let mut source = SourceRef::new(SourceKind::Metric, format!("series:prom:{key}"));
        source.observed = Some(TO);
        let mut p = vec![
            ("tel:series", V::Uid(series)),
            ("tel:instrument", V::Uid(instrument)),
            ("tel:from_ms", V::Int(FROM)),
            ("tel:to_ms", V::Int(TO)),
            ("tel:recipe", V::Text(recipe.into())),
            ("tel:basis", V::Text(basis.into())),
        ];
        if let Some(d) = digest {
            p.push(("tel:digest", V::Bytes([d; 32])));
        }
        p.extend(fields);
        let u = UnitCoreBuilder::new(ty(tel::WINDOW), label, status.unwrap_or(tw_status))
            .source(source)
            .grounds([series])
            .payload(payload(p))
            .build()
            .unwrap();
        let w = self.unit(label, u);
        let (op, rung) = att.unwrap_or(tw_att);
        self.attest(w, op, rung);
        self.rel(tel::OF_SERIES, w, series);
        w
    }

    /// An exact window with `count`, `min` and `max`.
    #[allow(clippy::too_many_arguments)]
    fn exact(
        &mut self,
        label: &str,
        series: Uid,
        instrument: Uid,
        count: u64,
        min: &'static str,
        max: &'static str,
        digest: u8,
    ) -> Uid {
        self.window(
            label,
            series,
            instrument,
            Kind::Exact,
            vec![
                ("tel:count", V::Int(count)),
                ("tel:min", V::Dec(min)),
                ("tel:max", V::Dec(max)),
            ],
            Some(digest),
            None,
            None,
        )
    }

    /// A sketched window: `count`, the quantiles 0.5, 0.9, 0.99 at `q`, and `alpha`.
    fn sketch(
        &mut self,
        label: &str,
        series: Uid,
        instrument: Uid,
        q: Vec<&'static str>,
        alpha: &'static str,
        digest: u8,
    ) -> Uid {
        self.window(
            label,
            series,
            instrument,
            Kind::Sketch,
            vec![
                ("tel:count", V::Int(60)),
                ("tel:qs", V::Decs(vec!["0.5", "0.9", "0.99"])),
                ("tel:q", V::Decs(q)),
                ("tel:alpha", V::Dec(alpha)),
            ],
            Some(digest),
            None,
            None,
        )
    }

    /// An anomaly on `series`, the target hypotheses explain.
    fn anomaly(&mut self, label: &str, series: Uid, w: Uid) -> Uid {
        let mut source = SourceRef::new(
            SourceKind::Metric,
            format!("series:prom:{}", self.series_keys[&series]),
        );
        source.observed = Some(TO);
        let u = UnitCoreBuilder::new(ty(tel::ANOMALY), label, Status::Derived)
            .source(source)
            .grounds([w])
            .payload(payload(vec![
                ("tel:series", V::Uid(series)),
                ("tel:from_ms", V::Int(FROM)),
                ("tel:to_ms", V::Int(TO)),
                ("tel:recipe", V::Text("tel/mad/1".into())),
                ("tel:score", V::Dec("7.25")),
                ("tel:direction", V::Text("up".into())),
            ]))
            .build()
            .unwrap();
        self.unit(label, u)
    }

    /// A hypothesis that `explains` `target`, grounded on `grounds`, quoting `quotes`.
    fn hypothesis(
        &mut self,
        label: &str,
        status: Status,
        target: Uid,
        grounds: &[Uid],
        quotes: Vec<String>,
    ) -> Uid {
        let mut p = vec![
            ("tel:claim", V::Text(label.to_string())),
            ("tel:target", V::Uid(target)),
        ];
        if !quotes.is_empty() {
            p.push(("tel:quotes", V::Texts(quotes)));
        }
        let u = UnitCoreBuilder::new(ty(tel::HYPOTHESIS), label, status)
            .grounds(grounds.iter().copied())
            .payload(payload(p))
            .build()
            .unwrap();
        let h = self.unit(label, u);
        self.rel(tel::EXPLAINS, h, target);
        h
    }
}

/// One exporter, its scraper, one series and one exact window: the ground most cases stand on.
struct Base {
    exporter: Uid,
    scraper: Uid,
    series: Uid,
    window: Uid,
}

fn base(c: &mut Case, name: &str) -> Base {
    let exporter = c.instrument("exporter", &format!("prom-target:{name}/h1:9100"));
    let scraper = c.instrument("scraper", &format!("prom-scrape:{name}/h1:9100"));
    c.rel(tel::VIA, exporter, scraper);
    let series = c.measured(&format!("{name}_seconds"), exporter);
    let window = c.exact("w", series, scraper, 60, "0.0004", "2.31", 0xa1);
    Base {
        exporter,
        scraper,
        series,
        window,
    }
}

fn clean() -> Case {
    let mut c = Case::new();
    let b = base(&mut c, "latency");
    let a = c.anomaly("latency jumped", b.series, b.window);
    let _ = b.exporter;
    c.hypothesis(
        "h: the deploy made latency jump",
        Status::Inferred,
        a,
        &[b.window],
        vec![quote(b.window, "max=2.31")],
    );
    c
}

fn tw() -> Case {
    let mut c = Case::new();
    let b = base(&mut c, "tw");
    let _ = b.window;
    let sk = |q: &'static str| vec![q, "0.9", "2.3"];
    // As rule TW asks: sketched and platform (the exact one is the base's).
    c.sketch("w-sketch", b.series, b.scraper, sk("0.012"), "0.01", 0xb1);
    let zbx = c.instrument("collector", "zabbix-server:zbx");
    c.window(
        "w-platform",
        b.series,
        zbx,
        Kind::Platform,
        vec![("tel:count", V::Int(60)), ("tel:max", V::Dec("2.31"))],
        None,
        None,
        None,
    );
    // Against it, once each, from a second scraper so that no key is shared with the above.
    let other = c.instrument("scraper", "prom-scrape:tw-b/h1:9100");
    c.rel(tel::VIA, b.exporter, other);
    c.window(
        "e603: exact, attested transformed",
        b.series,
        other,
        Kind::Exact,
        vec![("tel:count", V::Int(60)), ("tel:max", V::Dec("2.31"))],
        Some(0xa2),
        None,
        Some((Op::Transformed, Rung::Computed)),
    );
    c.window(
        "e603: sketched, measured",
        b.series,
        other,
        Kind::Sketch,
        vec![
            ("tel:count", V::Int(60)),
            ("tel:qs", V::Decs(vec!["0.5"])),
            ("tel:q", V::Decs(vec!["0.013"])),
            ("tel:alpha", V::Dec("0.01")),
        ],
        Some(0xb2),
        Some(Status::Measured),
        Some((Op::Imported, Rung::Computed)),
    );
    c.window(
        "e603: platform, with a digest",
        b.series,
        zbx,
        Kind::Platform,
        vec![("tel:count", V::Int(59)), ("tel:max", V::Dec("2.31"))],
        Some(0xc1),
        None,
        None,
    );
    // W614: a second exact window of the base's key, over another sample set.
    c.exact(
        "w614: same key, another digest",
        b.series,
        b.scraper,
        59,
        "0.0004",
        "2.31",
        0xa3,
    );
    c
}

fn tq() -> Case {
    let mut c = Case::new();
    let b = base(&mut c, "tq");
    let s = c.sketch(
        "w-sketch",
        b.series,
        b.scraper,
        vec!["0.012", "0.9", "2.3"],
        "0.01",
        0xb1,
    );
    // A window the hypotheses do not ground on.
    let other = c.exact("w-other", b.series, b.exporter, 60, "0.0004", "2.31", 0xa9);
    let a = c.anomaly("latency jumped", b.series, b.window);
    let w = b.window;
    let g = [w, s];
    c.hypothesis(
        "h-verifies",
        Status::Inferred,
        a,
        &g,
        vec![quote(w, "max=2.31"), quote(s, "q[2]=2.3")],
    );
    c.hypothesis(
        "h-wrong-value",
        Status::Inferred,
        a,
        &g,
        vec![quote(w, "max=2.3")],
    );
    c.hypothesis(
        "h-not-canonical",
        Status::Inferred,
        a,
        &g,
        vec![quote(w, "max=2.310")],
    );
    c.hypothesis(
        "h-outside-grounds",
        Status::Inferred,
        a,
        &g,
        vec![quote(other, "max=2.31")],
    );
    c.hypothesis(
        "h-no-field",
        Status::Inferred,
        a,
        &g,
        vec![quote(w, "mean=1.2")],
    );
    c.hypothesis(
        "h-past-the-list",
        Status::Inferred,
        a,
        &g,
        vec![quote(s, "q[3]=2.3")],
    );
    c
}

fn rivals() -> Case {
    let mut c = Case::new();
    let b = base(&mut c, "rivals");
    let w = b.window;
    let target = |c: &mut Case, n: &str| c.anomaly(&format!("target {n}"), b.series, w);
    let inf = Status::Inferred;
    // Two of one target.
    let t = target(&mut c, "two");
    c.hypothesis("two/a: the deploy", inf, t, &[w], vec![]);
    c.hypothesis("two/b: the database", inf, t, &[w], vec![]);
    // Three of one target: one contention.
    let t = target(&mut c, "three");
    c.hypothesis("three/a: the deploy", inf, t, &[w], vec![]);
    c.hypothesis("three/b: the database", inf, t, &[w], vec![]);
    c.hypothesis("three/c: the network", inf, t, &[w], vec![]);
    // One rebuts the other.
    let t = target(&mut c, "rebutted");
    let x = c.hypothesis("rebutted/a: the deploy", inf, t, &[w], vec![]);
    let y = c.hypothesis("rebutted/b: the database", inf, t, &[w], vec![]);
    c.kernel_rel(RelKind::Rebuts, y, x);
    // One supersedes the other.
    let t = target(&mut c, "superseded");
    let x = c.hypothesis("superseded/a: the deploy", inf, t, &[w], vec![]);
    let y = c.hypothesis("superseded/b: the deploy at 10:00", inf, t, &[w], vec![]);
    c.kernel_rel(RelKind::Supersedes, y, x);
    // One is only speculative.
    let t = target(&mut c, "speculative");
    c.hypothesis("speculative/a: the deploy", inf, t, &[w], vec![]);
    c.hypothesis(
        "speculative/b: cosmic rays",
        Status::Speculative,
        t,
        &[w],
        vec![],
    );
    // One is capped at speculative by rule TQ.
    let t = target(&mut c, "capped");
    c.hypothesis("capped/a: the deploy", inf, t, &[w], vec![]);
    c.hypothesis("capped/b: max hit 9", inf, t, &[w], vec![quote(w, "max=9")]);
    c
}

fn witnesses() -> Case {
    let mut c = Case::new();
    let scraper = c.instrument("scraper", "prom-scrape:central");
    let pair = |c: &mut Case, name: &str| {
        let a = c.instrument("exporter", &format!("prom-target:node/{name}"));
        let b = c.instrument("exporter", &format!("prom-target:app/{name}"));
        c.rel(tel::VIA, a, scraper);
        c.rel(tel::VIA, b, scraper);
        let sa = c.measured(&format!("node_{name}"), a);
        let sb = c.measured(&format!("app_{name}"), b);
        c.rel(tel::SAME_QUANTITY, sa.min(sb), sa.max(sb));
        (a, b, sa, sb)
    };
    // Independent, exact, max differs: one conflict.
    let (_, _, sa, sb) = pair(&mut c, "disk");
    c.exact("disk/node", sa, scraper, 60, "10", "90", 0x11);
    c.exact("disk/app", sb, scraper, 60, "10", "91", 0x12);
    // Preferred by count: the app's 60-sample window disagrees, its 59-sample one would not.
    let (_, b, sa, sb) = pair(&mut c, "mem");
    c.exact("mem/node", sa, scraper, 60, "10", "90", 0x21);
    c.exact("mem/app, preferred", sb, b, 60, "10", "95", 0x22);
    c.exact("mem/app, fewer samples", sb, scraper, 59, "10", "90", 0x23);
    // Agree: none.
    let (_, _, sa, sb) = pair(&mut c, "cpu");
    c.exact("cpu/node", sa, scraper, 60, "1", "9", 0x31);
    c.exact("cpu/app", sb, scraper, 60, "1", "9", 0x32);
    // One origin under two series: one witness, none.
    let origin = c.instrument("exporter", "prom-target:node/net");
    c.rel(tel::VIA, origin, scraper);
    let sa = c.measured("node_net", origin);
    let sb = c.measured("node_net{replica=\"b\"}", origin);
    c.rel(tel::SAME_QUANTITY, sa.min(sb), sa.max(sb));
    c.exact("net/a", sa, scraper, 60, "1", "9", 0x41);
    c.exact("net/b", sb, scraper, 60, "1", "8", 0x42);
    // A transform on both lineages: one witness, none.
    let (a, b, sa, sb) = pair(&mut c, "qps");
    let rule = c.instrument("transform", "rule:qps");
    c.rel(tel::VIA, a, rule);
    c.rel(tel::VIA, b, rule);
    c.exact("qps/node", sa, scraper, 60, "1", "9", 0x51);
    c.exact("qps/app", sb, scraper, 60, "1", "8", 0x52);
    // Sketches 1 % apart at alpha 0.01: within 2·alpha, none.
    let (_, _, sa, sb) = pair(&mut c, "p99-near");
    c.sketch(
        "p99-near/node",
        sa,
        scraper,
        vec!["1", "2", "100"],
        "0.01",
        0x61,
    );
    c.sketch(
        "p99-near/app",
        sb,
        scraper,
        vec!["1", "2", "99"],
        "0.01",
        0x62,
    );
    // Sketches 3 % apart at alpha 0.01: beyond it, one conflict on q[2].
    let (_, _, sa, sb) = pair(&mut c, "p99-far");
    c.sketch(
        "p99-far/node",
        sa,
        scraper,
        vec!["1", "2", "100"],
        "0.01",
        0x71,
    );
    c.sketch(
        "p99-far/app",
        sb,
        scraper,
        vec!["1", "2", "97"],
        "0.01",
        0x72,
    );
    // No origin: SMY-W609.
    c.series("orphan_seconds");
    c
}

fn reserved() -> Case {
    let mut c = Case::new();
    let b = base(&mut c, "reserved");
    let _ = (b.exporter, b.window);
    // Records 6 of kinds 6 and 7, as a writer that predates TL-P1 might have left them.
    for (kind, n) in [
        (DetectionKind::RivalExplanation, 1u8),
        (DetectionKind::WitnessConflict, 2u8),
    ] {
        let (p, q) = (Uid::from_bytes([n; 32]), Uid::from_bytes([n + 10; 32]));
        c.records.push(Record::Contention(Contention::new(
            ContentionId::derive(kind, &b.series, &[p, q]),
            b.series,
            vec![p, q],
            Detected::new(kind, Hlc::new(TO, 0, agent())),
        )));
    }
    // Record 16, `hold`.
    c.records.push(Record::Unknown {
        code: smysl_core::types::record::code::HOLD,
        payload: vec![0xa0],
    });
    // `source` key 7, a sub-millisecond remainder.
    let mut s = SourceRef::new(SourceKind::Metric, "series:prom:reserved_seconds");
    s.observed = Some(TO);
    s.extra.insert(7, vec![0x19, 0x01, 0xf4]);
    let u = UnitCoreBuilder::new(
        smysl_core::KernelType::Observation,
        "reserved_seconds read 2.31 with a remainder",
        Status::Measured,
    )
    .source(s)
    .build()
    .unwrap();
    c.unit("e612: source key 7", u);
    // A series with an observed time: §4.1's entity-like rule.
    let mut u = tel::series("prom", "timed_seconds").unwrap();
    if let Some(src) = u.source.as_mut() {
        src.observed = Some(FROM);
    }
    let timed = c.unit("e607: a series with a time", u);
    c.series_keys.insert(timed, "timed_seconds".into());
    c.rel(tel::MEASURED_BY, timed, b.exporter);
    c
}

/// A span stream, as `otlp-traces/1` keys one (§6).
const SPANS: &str = r#"{"resource":{"service.name":"cart"},"scope":{"name":"io.opentelemetry.http","version":"1.2.0"},"signal":"spans"}"#;
/// A journald stream (§6).
const JOURNAL: &str = r#"{"host":"h1","unit":"cart.service"}"#;

impl Case {
    /// An event-stream series and its collector, measured there.
    fn stream(&mut self, scheme: &str, key: &str, collector: &str) -> (Uid, Uid, String) {
        let s = self.unit(key, tel::series(scheme, key).unwrap());
        let i = self.instrument("collector", collector);
        self.rel(tel::MEASURED_BY, s, i);
        (s, i, format!("series:{scheme}:{key}"))
    }

    /// A span of `stream`, measured unless `status` says otherwise, attested as rule TW asks
    /// unless `att` does.
    #[allow(clippy::too_many_arguments)]
    fn span(
        &mut self,
        label: &str,
        (stream, inst, reference): (Uid, Uid, &str),
        id: &str,
        parent: Option<&str>,
        digest: Option<u8>,
        att: Option<(Op, Rung)>,
    ) -> Uid {
        let mut source = SourceRef::new(SourceKind::Metric, reference);
        source.observed = Some(FROM);
        let mut p = vec![
            ("tel:series", V::Uid(stream)),
            ("tel:instrument", V::Uid(inst)),
            (
                "tel:trace",
                V::Text("4bf92f3577b34da6a3ce929d0e0e4736".into()),
            ),
            ("tel:span_id", V::Text(id.into())),
            ("tel:name", V::Text("GET /cart".into())),
            ("tel:start_ms", V::Int(FROM)),
            ("tel:end_ms", V::Int(FROM + 12)),
            ("tel:status", V::Text("ok".into())),
        ];
        if let Some(pa) = parent {
            p.push(("tel:parent", V::Text(pa.into())));
        }
        if let Some(d) = digest {
            p.push(("tel:digest", V::Bytes([d; 32])));
        }
        let u = UnitCoreBuilder::new(ty(tel::SPAN), label, Status::Measured)
            .source(source)
            .payload(payload(p))
            .build()
            .unwrap();
        let id = self.unit(label, u);
        let (op, rung) = att.unwrap_or((Op::Imported, Rung::Computed));
        self.attest(id, op, rung);
        id
    }
}

fn events() -> Case {
    let mut c = Case::new();
    let (s, i, r) = c.stream("otel", SPANS, "otel-collector:c1");
    let parent = c.span(
        "span: parent",
        (s, i, &r),
        "00f067aa0ba902b7",
        None,
        Some(0xd1),
        None,
    );
    let child = c.span(
        "span: child",
        (s, i, &r),
        "53995c3f42cd8ad8",
        Some("00f067aa0ba902b7"),
        Some(0xd2),
        None,
    );
    c.rel(tel::PRECEDES, parent, child);
    c.span(
        "e603: span, attested transformed",
        (s, i, &r),
        "b7ad6b7169203331",
        None,
        Some(0xd3),
        Some((Op::Transformed, Rung::Computed)),
    );
    c.span(
        "e603: span, no digest",
        (s, i, &r),
        "c0ffee0000000001",
        None,
        None,
        None,
    );
    let (log, li, lr) = c.stream("journald", JOURNAL, "journald:h1");
    for (label, status, op, d) in [
        ("template", Status::Derived, Op::Transformed, 0xe1),
        (
            "e603: template, measured",
            Status::Measured,
            Op::Imported,
            0xe2,
        ),
    ] {
        let mut source = SourceRef::new(SourceKind::Metric, lr.as_str());
        source.observed = Some(TO);
        let p = vec![
            ("tel:series", V::Uid(log)),
            ("tel:instrument", V::Uid(li)),
            ("tel:from_ms", V::Int(FROM)),
            ("tel:to_ms", V::Int(TO)),
            ("tel:recipe", V::Text("tel/drain/1".into())),
            (
                "tel:template",
                V::Text("connection from <IP> closed after <NUM> ms".into()),
            ),
            ("tel:count", V::Int(17)),
            ("tel:digest", V::Bytes([d; 32])),
        ];
        let u = UnitCoreBuilder::new(ty(tel::LOG_TEMPLATE), label, status)
            .source(source)
            .grounds([log])
            .payload(payload(p))
            .build()
            .unwrap();
        let t = c.unit(label, u);
        c.attest(t, op, Rung::Computed);
    }
    c
}

/// RFC SMYSL-5 §3 and §8: rule TI, the info key, and rule TQ over a statistic's fields.
fn info() -> Case {
    let mut c = Case::new();
    // `x.tel/info` is revision 2's.
    c.records[0] = Record::SchemaDecl(tel::decl());
    let b = base(&mut c, "load");
    let peer = c.measured("load_peer_seconds", b.exporter);
    let ent_recipe = "ent/1?bins=width:0.5&cover=4/5&hold=1000&min_n=50&of=value&step=1000";
    let mi_recipe = "mi/1?bins=rank:4&cover=4/5&guard=1/10&hold=1000&lag=-3..3&min_n=50\
                     &null=200&of=value&seed=7&step=1000";
    let info = |c: &mut Case,
                label: &str,
                status: Status,
                stat: &str,
                fields: Vec<(&'static str, V)>,
                digest: u8| {
        let mut source = SourceRef::new(SourceKind::Metric, "series:prom:load_seconds");
        source.observed = Some(TO);
        let mut p = vec![
            ("tel:stat", V::Text(stat.into())),
            ("tel:series", V::Uid(b.series)),
            ("tel:instrument", V::Uid(b.scraper)),
            ("tel:from_ms", V::Int(FROM)),
            ("tel:to_ms", V::Int(TO)),
            ("tel:digest", V::Bytes([digest; 32])),
        ];
        p.extend(fields);
        let mut grounds = vec![b.series];
        if stat == "mi" {
            grounds.push(peer);
            grounds.sort();
        }
        let u = UnitCoreBuilder::new(ty(tel::INFO), label, status)
            .source(source)
            .grounds(grounds.clone())
            .payload(payload(p))
            .build()
            .unwrap();
        let id = c.unit(label, u);
        c.attest(id, Op::Transformed, Rung::Computed);
        for g in grounds {
            c.rel(tel::OF_SERIES, id, g);
        }
        id
    };
    let entropy = |n: u64, value: &'static str, recipe: &str| {
        vec![
            ("tel:recipe", V::Text(recipe.into())),
            ("tel:n", V::Int(n)),
            ("tel:value", V::Dec(value)),
            ("tel:k", V::Int(6)),
            ("tel:mm", V::Dec("1.84211")),
        ]
    };
    let mi = |with_digest: bool, recipe: &str| {
        let mut f = vec![
            ("tel:recipe", V::Text(recipe.into())),
            ("tel:n", V::Int(57)),
            ("tel:value", V::Dec("0.412")),
            ("tel:with", V::Uid(peer)),
            ("tel:with_instrument", V::Uid(b.scraper)),
            ("tel:k", V::Int(4)),
            ("tel:k_with", V::Int(4)),
            ("tel:bias", V::Dec("0.11389")),
            ("tel:lag", V::Int(0)),
            ("tel:lags", V::Ints(vec![-3, -2, -1, 0, 1, 2, 3])),
            (
                "tel:values",
                V::Decs(vec![
                    "0.021", "0.05", "0.190116", "0.412", "0.18", "0.033", "0.02",
                ]),
            ),
            ("tel:null_b", V::Int(200)),
            ("tel:null_ge", V::Int(0)),
        ];
        if with_digest {
            f.push(("tel:with_digest", V::Bytes([0xc2; 32])));
        }
        f
    };
    let ent = info(
        &mut c,
        "entropy",
        Status::Derived,
        "entropy",
        entropy(60, "1.831294", ent_recipe),
        0xb1,
    );
    let scan = info(
        &mut c,
        "mi scan",
        Status::Derived,
        "mi",
        mi(true, mi_recipe),
        0xb1,
    );
    let a = c.anomaly("load jumped", b.series, b.window);
    c.hypothesis(
        "h: the load and its peer move together",
        Status::Inferred,
        a,
        &[ent, scan],
        vec![
            quote(ent, "value=1.831294"),
            quote(ent, "mm=1.84211"),
            quote(scan, "values[2]=0.190116"),
            quote(scan, "null_ge=0"),
        ],
    );
    c.hypothesis(
        "w606: h quoting the entropy to two places, and with a trailing zero",
        Status::Inferred,
        a,
        &[ent],
        vec![quote(ent, "value=1.83"), quote(ent, "value=1.8312940")],
    );
    info(
        &mut c,
        "e603: entropy, measured",
        Status::Measured,
        "entropy",
        entropy(59, "1.8", &ent_recipe.replace("min_n=50", "min_n=40")),
        0xb3,
    );
    let other = mi_recipe.replace("seed=7", "seed=8");
    info(
        &mut c,
        "e603: mi, no with_digest",
        Status::Derived,
        "mi",
        mi(false, &other),
        0xb4,
    );
    info(
        &mut c,
        "w614: entropy, another sample set",
        Status::Derived,
        "entropy",
        entropy(61, "1.83", ent_recipe),
        0xb2,
    );
    c
}

fn build(name: &str) -> Case {
    match name {
        "events" => events(),
        "info" => info(),
        "clean" => clean(),
        "tw" => tw(),
        "tq" => tq(),
        "rivals" => rivals(),
        "witnesses" => witnesses(),
        "reserved" => reserved(),
        other => panic!("{other}"),
    }
}

/// What an implementation derives from one case's records: its rows of `expected.tsv`.
fn derive(name: &str, records: Vec<Record>, labels: &BTreeMap<Uid, String>) -> Vec<String> {
    let store = Store::from_records(records);
    let mut rows = Vec::new();
    for (uid, label) in labels {
        rows.push(format!("{name}\tunit\t{}\t{label}", uid.canonical()));
    }
    let mut codes: Vec<String> = check(&store, CheckOptions::default())
        .iter()
        .map(|d| {
            let on = d.uid().map_or("-".to_string(), Uid::canonical);
            format!("{name}\tcode\t{}\t{on}", d.code)
        })
        .collect();
    codes.sort();
    rows.extend(codes);
    let effective = effective_status(&store, RetractionPolicy::default());
    let payload_of =
        |u: &Uid| StoreRead::get(&store, u).map(|c| c.core.payload.clone().unwrap_or_default());
    for (uid, u) in StoreRead::units(&store) {
        if u.core.schema.as_str() != tel::HYPOTHESIS {
            continue;
        }
        let failures = tel::rule_tq_violations(&u.core, payload_of).len();
        let status = tel::tq_capped(effective.get(uid).unwrap_or(Status::Unfounded), failures);
        rows.push(format!(
            "{name}\tstatus\t{}\t{}",
            uid.canonical(),
            status.as_str()
        ));
    }
    let now = Hlc::new(0, 0, agent());
    let mut contentions = tel::rival_explanations(
        StoreRead::units(&store).map(|(uid, u)| (uid, &u.core)),
        store.relations(),
        |u| effective.get(u).unwrap_or(Status::Unfounded),
        payload_of,
        now.clone(),
    );
    contentions.extend(tel::witness_conflicts(
        StoreRead::units(&store).map(|(uid, u)| (uid, &u.core)),
        store.relations(),
        now,
    ));
    for c in contentions {
        let positions: Vec<String> = c.positions.iter().map(Uid::canonical).collect();
        rows.push(format!(
            "{name}\tcontention\t{}\tkind={} over={} positions={}",
            c.id.as_str(),
            c.detected.kind.as_u8(),
            c.over.canonical(),
            positions.join(",")
        ));
    }
    rows
}

const HEADER: &str = "\
# RFC SMYSL-3 §16 C-Tel-Check: what each fixtures/tel/check/<case>.cbor derives.
# case\tunit\t<uid>\t<label it was built under: for reading, not derived>
# case\tcode\t<SMY code>\t<the unit it names, or ->
# case\tstatus\t<hypothesis uid>\t<effective status after rule TQ's cap>
# case\tcontention\t<derived id>\tkind=<6|7> over=<uid> positions=<uid>,<uid>...
# Every code, status and contention, exactly: an extra row fails as a missing one does.
";

#[test]
fn the_check_fixtures_are_current_and_derive_what_they_pin() {
    let bless = std::env::var_os("SMYSL_BLESS").is_some();
    let mut tsv = HEADER.to_string();
    for name in CASES {
        let c = build(name);
        let bytes = to_cbor_seq(&c.records);
        let path = dir().join(format!("{name}.cbor"));
        if bless {
            std::fs::create_dir_all(dir()).unwrap();
            std::fs::write(&path, &bytes).unwrap();
        }
        let on_disk = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        assert_eq!(
            on_disk, bytes,
            "{name}.cbor no longer matches its source; SMYSL_BLESS=1 rewrites it, and that \
             redefines C-Tel-Check"
        );
        // Derived from the bytes on disk, as a second implementation would.
        let (records, rest) = from_cbor_seq(&on_disk).expect("the fixture decodes");
        assert_eq!(rest, on_disk.len(), "{name}.cbor: every byte decodes");
        for row in derive(name, records, &c.labels) {
            writeln!(tsv, "{row}").unwrap();
        }
    }
    let expected = dir().join("expected.tsv");
    if bless {
        std::fs::write(&expected, &tsv).unwrap();
    }
    let on_disk = std::fs::read_to_string(&expected).expect("fixtures/tel/check/expected.tsv");
    assert_eq!(on_disk, tsv, "SMYSL_BLESS=1 rewrites expected.tsv");
}
