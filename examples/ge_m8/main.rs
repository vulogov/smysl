//! GE-M8 (RFC SMYSL-3 §21, §26): do episodes recur detectably? On simulated episodes.
//!
//! As fixed in §26 before the run:
//! - **The corpus** is twelve fault types over one shop's services, ten episodes each. A type
//!   has a core of eight to ten features: anomalies on named series, changes at pointers,
//!   alerts and log templates. Half of each core is shared with its sibling type of the same
//!   service. An occurrence keeps each core feature with probability 0.7 (at least two), and
//!   adds zero to four from a background pool of thirty.
//! - **Each episode is units.** Anomaly, change and alert units are grouped by
//!   `smysl_tel::detect::episode`, which joins them with `x.tel/during`. The log templates of
//!   the episode's window are `x.tel/log-template` units. `smysl_tel::similar::episodes` reads
//!   the features from them, as `tel similar` does.
//! - **Three arms rank the other 119:**
//!   - MinHash, 128 functions;
//!   - exact Jaccard;
//!   - embeddings: Model2Vec over a text rendering of the same units, by cosine. The model is
//!     the directory `SMYSL_EMBED_MODEL` names (`potion-base-8M`); without it this arm is not
//!     run, and the report says so.
//! - **recall@5:** the share of the top five that are of the episode's type, averaged.
//!
//! The stop condition is the RFC's: MinHash below 0.6 where embeddings exceed 0.8 makes
//! similarity model-dependent by default.
//!
//! `cargo run --release --features tel,semantic --example ge_m8 [-- --seed S] [--similarity RECIPE]`
//!
//! `--similarity` reads the features and functions of a `tel/minhash/2` recipe (RFC SMYSL-9
//! SI-P2), as `tel similar --similarity` does; the default is `/1`.

use std::collections::{BTreeMap, BTreeSet};

use smysl::schemas::tel;
use smysl::surface::hjson::{HObject, HValue, Spanned};
use smysl::tel::detect::{self, episode, Direction, Input, Recipe};
use smysl::tel::logs::Template;
use smysl::tel::similar;
use smysl::{
    canonical_uid, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};

/// splitmix64: a fixed, seedable sequence.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// One feature an incident shows, before it is a unit.
#[derive(Clone, Debug)]
enum F {
    Anomaly(String, Direction),
    Change(String, &'static str),
    Alert(String),
    Log(String),
}

impl F {
    /// What the embeddings arm reads for it.
    fn text(&self) -> String {
        match self {
            F::Anomaly(k, d) => format!("anomaly {} on {k}", d.as_str()),
            F::Change(obj, p) => format!("change {p} on {obj}"),
            F::Alert(a) => format!("alert {a}"),
            F::Log(t) => format!("log {t}"),
        }
    }
}

const SERVICES: [&str; 6] = [
    "cart",
    "checkout",
    "payment",
    "catalog",
    "search",
    "inventory",
];

fn lat(s: &str) -> String {
    format!("http_request_duration_seconds{{service=\"{s}\"}}")
}
fn err(s: &str) -> String {
    format!("http_requests_errors_total{{service=\"{s}\"}}")
}
fn deploy(s: &str) -> String {
    format!("apps/v1/Deployment/shop/{s}")
}

/// The twelve fault types: for each service, a bad deploy and an exhausted database. The two
/// share half their core: latency, errors, the timeout their callers log, the error alert.
fn fault_types() -> Vec<(String, Vec<F>)> {
    let mut out = Vec::new();
    for s in SERVICES {
        let shared = vec![
            F::Anomaly(lat(s), Direction::Up),
            F::Anomaly(err(s), Direction::Up),
            F::Log(format!("timeout calling {s} after <NUM> ms")),
            F::Alert(format!("alertmanager HighErrorRate service={s}")),
        ];
        let mut deploy_bad = shared.clone();
        deploy_bad.extend([
            F::Change(deploy(s), "/spec/template/spec/containers/0/image"),
            F::Anomaly(
                format!("process_cpu_seconds_total{{service=\"{s}\"}}"),
                Direction::Up,
            ),
            F::Log(format!("unhandled exception in {s} handler <*>")),
            F::Alert(format!(
                "alertmanager KubeDeploymentRolloutStuck deployment={s}"
            )),
            F::Anomaly(
                format!("container_restarts_total{{container=\"{s}\"}}"),
                Direction::Up,
            ),
        ]);
        let mut db_bad = shared;
        db_bad.extend([
            F::Anomaly(format!("db_pool_in_use{{service=\"{s}\"}}"), Direction::Up),
            F::Log(format!(
                "connection pool exhausted for {s}-db after <NUM> ms"
            )),
            F::Alert(format!("alertmanager DatabaseConnectionsHigh db={s}-db")),
            F::Anomaly(
                format!("http_requests_total{{service=\"{s}\"}}"),
                Direction::Down,
            ),
            F::Change(format!("v1/ConfigMap/shop/{s}-db"), "/data/max_connections"),
        ]);
        out.push((format!("{s}: bad deploy"), deploy_bad));
        out.push((format!("{s}: database exhausted"), db_bad));
    }
    out
}

/// Thirty features any incident may show.
fn background() -> Vec<F> {
    let mut out = Vec::new();
    for s in SERVICES {
        out.push(F::Anomaly(
            format!("go_gc_duration_seconds{{service=\"{s}\"}}"),
            Direction::Up,
        ));
        out.push(F::Anomaly(
            format!("container_memory_working_set_bytes{{container=\"{s}\"}}"),
            Direction::Up,
        ));
    }
    for t in [
        "connection from <IP> closed after <NUM> ms",
        "health check ok",
        "cache miss for key <HEX>",
        "request <UUID> completed in <NUM> ms",
        "retrying in <NUM> ms",
        "slow query took <NUM> ms",
        "gc pause <NUM> ms",
        "tls handshake from <IP> failed",
        "rate limit reached for client <*>",
        "config reloaded",
    ] {
        out.push(F::Log(t.to_string()));
    }
    for a in [
        "alertmanager Watchdog",
        "alertmanager NodeClockSkewDetected node=n1",
        "alertmanager CPUThrottlingHigh namespace=shop",
        "alertmanager KubeHpaMaxedOut hpa=frontend",
        "zabbix Zabbix agent unreachable host=db2",
        "alertmanager TargetDown job=node",
        "alertmanager NodeFilesystemAlmostFull node=n2",
        "alertmanager KubePodNotReady namespace=shop",
    ] {
        out.push(F::Alert(a.to_string()));
    }
    out
}

fn obj(entries: &[(&str, HValue)]) -> Vec<u8> {
    let span = smysl::Span::new(0, 0);
    let mut o = HObject::new();
    for (k, v) in entries {
        o.insert(
            Spanned::new((*k).to_string(), span),
            Spanned::new(v.clone(), span),
        );
    }
    smysl::surface::object_to_payload(&o).expect("a payload")
}

/// The unit of one feature, at `t`; templates are made per episode, not here.
fn unit(f: &F, t: u64, inst: Uid) -> Option<UnitCore> {
    match f {
        F::Anomaly(key, dir) => {
            let s = tel::series("prom", key).expect("a series");
            let input = Input {
                series: canonical_uid(&s),
                scheme: "prom",
                key,
                instrument: inst,
                samples: &[],
            };
            detect::anomaly(
                &input,
                t,
                t + 60_000,
                &Recipe::new("mad/1"),
                6.0,
                Some(*dir),
            )
        }
        F::Change(object, pointer) => {
            let o = canonical_uid(&tel::series("k8s", object).expect("a series"));
            Some(
                UnitCoreBuilder::new(
                    SchemaId::Extension(tel::CHANGE.to_string()),
                    format!("replace {pointer}"),
                    Status::Measured,
                )
                .source(
                    SourceRef::new(SourceKind::Metric, format!("series:k8s:{object}"))
                        .observed_at(t),
                )
                .payload(obj(&[
                    ("tel:op", HValue::Str("replace".into())),
                    ("tel:pointer", HValue::Str((*pointer).into())),
                    ("tel:class", HValue::Str("config".into())),
                    ("tel:vtype", HValue::Str("text".into())),
                    ("tel:object", HValue::Str(o.canonical())),
                ]))
                .build()
                .expect("a change"),
            )
        }
        F::Alert(a) => {
            let (system, key) = a.split_once(' ').expect("system and key");
            Some(
                UnitCoreBuilder::new(
                    SchemaId::Extension(tel::ALERT.to_string()),
                    format!("alert {key}"),
                    Status::Cited,
                )
                .source(SourceRef::new(SourceKind::Tool, system.to_string()).observed_at(t))
                .payload(obj(&[
                    ("tel:system", HValue::Str(system.into())),
                    ("tel:key", HValue::Str(key.into())),
                    ("tel:state", HValue::Str("firing".into())),
                ]))
                .build()
                .expect("an alert"),
            )
        }
        F::Log(_) => None,
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| f64::from(*x) * f64::from(*y))
        .sum();
    let na: f64 = a.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| f64::from(*x).powi(2)).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na * nb)
    }
}

/// recall@5 of a ranking function: for each episode, the share of its top five of its type.
fn recall5(eps: &[Uid], ty: &BTreeMap<Uid, usize>, rank: impl Fn(&Uid) -> Vec<Uid>) -> f64 {
    let mut sum = 0.0;
    for e in eps {
        let top = rank(e);
        let hits = top.iter().take(5).filter(|u| ty[*u] == ty[e]).count();
        sum += hits as f64 / 5.0;
    }
    sum / eps.len() as f64
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let seed = args
        .iter()
        .position(|a| a == "--seed")
        .and_then(|i| args.get(i + 1))
        .and_then(|v| v.parse().ok())
        .unwrap_or(8u64);
    let recipe = match args.iter().position(|a| a == "--similarity") {
        Some(i) => similar::Recipe::parse(args.get(i + 1).expect("--similarity RECIPE"))
            .unwrap_or_else(|e| panic!("--similarity: {e}")),
        None => similar::Recipe::v1(0),
    };
    let started = std::time::Instant::now();
    let mut r = Rng(seed);
    let types = fault_types();
    let bg = background();
    let inst =
        canonical_uid(&tel::instrument("scraper", "prom-scrape:shop").expect("an instrument"));
    let stream = tel::series("journald", r#"{"host":"h1","unit":"shop"}"#).expect("a stream");
    let stream_uid = canonical_uid(&stream);

    // Episodes, six hours apart, types interleaved.
    const T0: u64 = 1_768_557_600_000;
    let mut members: Vec<UnitCore> = Vec::new();
    let mut templates: Vec<UnitCore> = Vec::new();
    let mut slot_type: Vec<(u64, usize)> = Vec::new();
    let mut texts: Vec<(u64, String)> = Vec::new();
    for occ in 0..10 {
        for (ti, (_, core)) in types.iter().enumerate() {
            let slot = (occ * types.len() + ti) as u64;
            let t = T0 + slot * 6 * 3_600_000;
            let mut fs: Vec<F> = core.iter().filter(|_| r.unit() < 0.7).cloned().collect();
            while fs.len() < 2 {
                fs.push(core[r.below(core.len() as u64) as usize].clone());
            }
            // An episode is its members: at least one feature that is not a log line.
            if fs.iter().all(|f| matches!(f, F::Log(_))) {
                fs.push(core[0].clone());
            }
            for _ in 0..r.below(5) {
                fs.push(bg[r.below(bg.len() as u64) as usize].clone());
            }
            let mut logs = BTreeSet::new();
            let mut rendered = Vec::new();
            for (k, f) in fs.iter().enumerate() {
                rendered.push(f.text());
                match f {
                    F::Log(text) => {
                        logs.insert(text.clone());
                    }
                    other => {
                        if let Some(u) = unit(other, t + k as u64 * 30_000, inst) {
                            members.push(u);
                        }
                    }
                }
            }
            rendered.sort();
            rendered.dedup();
            texts.push((slot, rendered.join(". ")));
            for text in logs {
                templates.push(
                    Template {
                        stream: stream_uid,
                        scheme: "journald".into(),
                        key: r#"{"host":"h1","unit":"shop"}"#.into(),
                        inst,
                        from_ms: t,
                        to_ms: t + 600_000,
                        template: text,
                        count: 1 + r.below(40),
                        digest: [slot as u8; 32],
                    }
                    .unit(),
                );
            }
            slot_type.push((slot, ti));
        }
    }
    // Grouping: the episode detector, a five-minute gap.
    let p = episode::Params::default();
    let grouped = episode::run(&members, T0, T0 + 10_000 * 3_600_000, &p);
    let mut units: BTreeMap<Uid, UnitCore> = BTreeMap::new();
    let mut relations = Vec::new();
    for u in members.iter().chain(&templates) {
        units.insert(canonical_uid(u), u.clone());
    }
    let mut ty: BTreeMap<Uid, usize> = BTreeMap::new();
    let mut text_of: BTreeMap<Uid, String> = BTreeMap::new();
    for g in &grouped {
        let uid = canonical_uid(&g.unit);
        let o = smysl::surface::payload_to_object(g.unit.payload.as_deref().expect("a payload"))
            .expect("an object");
        let HValue::Int(from) = o.get("tel:from_ms").expect("from").value else {
            panic!("from_ms")
        };
        let slot = (from as u64 - T0) / (6 * 3_600_000);
        ty.insert(uid, slot_type[slot as usize].1);
        text_of.insert(uid, texts[slot as usize].1.clone());
        units.insert(uid, g.unit.clone());
        relations.extend(g.relations.iter().cloned());
    }
    let eps_feats = similar::episodes_with(units.iter(), relations.iter(), &recipe);
    let eps: Vec<Uid> = eps_feats.keys().copied().collect();
    let feats: Vec<usize> = eps_feats.values().map(BTreeSet::len).collect();

    // Arm MinHash.
    let mh = recall5(&eps, &ty, |e| {
        similar::nearest_with(&eps_feats, e, 5, &recipe)
            .into_iter()
            .map(|n| n.uid)
            .collect()
    });
    // Arm exact Jaccard.
    let jac = recall5(&eps, &ty, |e| {
        let mut v: Vec<(Uid, (usize, usize))> = eps
            .iter()
            .filter(|u| *u != e)
            .map(|u| (*u, similar::jaccard(&eps_feats[e], &eps_feats[u])))
            .collect();
        // a/b > c/d as a·d > c·b, then the smaller uid.
        v.sort_by(|x, y| {
            (y.1 .0 * x.1 .1)
                .cmp(&(x.1 .0 * y.1 .1))
                .then(x.0.cmp(&y.0))
        });
        v.into_iter().map(|(u, _)| u).collect()
    });
    // Arm embeddings.
    let emb = std::env::var_os("SMYSL_EMBED_MODEL").map(|dir| {
        let model = smysl::EmbedModel::from_dir(&dir).expect("a Model2Vec model directory");
        let vecs: BTreeMap<Uid, Vec<f32>> =
            eps.iter().map(|e| (*e, model.embed(&text_of[e]))).collect();
        recall5(&eps, &ty, |e| {
            let mut v: Vec<(Uid, f64)> = eps
                .iter()
                .filter(|u| *u != e)
                .map(|u| (*u, cosine(&vecs[e], &vecs[u])))
                .collect();
            v.sort_by(|x, y| y.1.total_cmp(&x.1).then(x.0.cmp(&y.0)));
            v.into_iter().map(|(u, _)| u).collect()
        })
    });

    println!("GE-M8 (simulated): do episodes recur detectably?");
    println!(
        "  corpus            {} episodes of {} fault types (seed {seed}), {}–{} features each",
        eps.len(),
        types.len(),
        feats.iter().min().unwrap_or(&0),
        feats.iter().max().unwrap_or(&0)
    );
    println!("  recall@5          MinHash (128)   {mh:.3}");
    println!("                    exact Jaccard   {jac:.3}");
    match emb {
        Some(e) => {
            println!("                    embeddings      {e:.3}  (Model2Vec, SMYSL_EMBED_MODEL)")
        }
        None => {
            println!("                    embeddings      not run: SMYSL_EMBED_MODEL is not set")
        }
    }
    let verdict = match emb {
        Some(e) if mh < 0.6 && e > 0.8 => {
            "MinHash < 0.6 where embeddings > 0.8: model-dependent by default"
        }
        Some(_) => "the stop condition does not fire: similarity stays model-independent (MinHash)",
        None => "undecided without the embeddings arm",
    };
    println!("  verdict           {verdict}");
    println!(
        "  run               {:.1} s; simulated episodes: GE-M7's corpus and RCAEval are owed",
        started.elapsed().as_secs_f64()
    );
    assert_eq!(eps.len(), 120, "every occurrence is one episode");
}
