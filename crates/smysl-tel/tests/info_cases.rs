//! RFC SMYSL-5 GE-I1 and C-Tel-Info (TI-P1 to TI-P3): can every statistic be replayed?
//!
//! Every recipe in `fixtures/tel/info/` over every series of its `series.tsv` (a pair recipe
//! against `info_driver`, every series as its first), and `tent/1` over the log stream of
//! `journal.json` in three windows, **twice** in one
//! process and compared byte for byte, and every outcome pinned in `expected.tsv`: the unit's
//! uid and the numbers it states, or the refusal's code. The file was written on aarch64 macOS;
//! CI runs this test on x86_64 Linux (glibc and musl), x86_64 Windows and aarch64 macOS, so a
//! statistic whose bytes differ on any of them fails there, which is GE-I1's stop condition:
//! "any byte difference keeps that statistic off until fixed".
//!
//! `SMYSL_BLESS=1` rewrites `expected.tsv`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::schemas::tel;
use smysl_core::{canonical_uid, to_cbor_seq, Decimal, Record, Uid};
use smysl_series::{MemBackend, Sample, SeriesInfo, Substrate};
use smysl_tel::info::{logs, run_on, Outcome, Spec, With};
use smysl_tel::readers::journald;

const FROM: u64 = 1_768_550_400_000;
const TO: u64 = FROM + 360 * 60_000;

const RECIPES: &[&str] = &[
    "ent-width",
    "ent-baseline",
    "ent-diff",
    "ent-increase",
    "perm-3",
    "perm-4-tau2",
    "ent-strict",
    "mi-rank",
    "mi-scan",
    "mi-width-lag",
    "mi-diff",
    "jsd-width",
    "jsd-baseline",
];

/// The second series of every pair recipe.
const WITH: &str = "info_driver";

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/info")
}

/// The fixture's log stream added to `sub`: its uid and the stream's series unit.
fn journal(sub: &mut Substrate<MemBackend>) -> Uid {
    let text = std::fs::read_to_string(dir().join("journal.json")).unwrap();
    let entries = journald::read(&text).unwrap();
    let r = smysl_tel::logs::record_journald(&entries, "collector", "journald:h1").unwrap();
    assert_eq!(r.streams.len(), 1);
    let st = &r.streams[0];
    sub.add_events(
        &st.uid,
        &r.inst,
        &st.info,
        &st.events,
        &smysl_tel::trace::tag,
    )
    .unwrap();
    st.uid
}

/// `tent/1`'s windows: the six hours, the four before the timeouts start, the two after.
const TENT_WINDOWS: &[(&str, u64, u64)] = &[
    ("tent", FROM, TO),
    ("tent-normal", FROM, FROM + 240 * 60_000),
    ("tent-faulty", FROM + 240 * 60_000, TO),
];

fn substrate() -> (Substrate<MemBackend>, Vec<Uid>) {
    let text = std::fs::read_to_string(dir().join("series.tsv")).unwrap();
    let inst = canonical_uid(&tel::instrument("scraper", "prom-scrape:ge-i1/sim:9100").unwrap());
    let mut by_key: BTreeMap<(String, String), Vec<Sample>> = BTreeMap::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let f: Vec<&str> = line.split('\t').collect();
        let s = Sample::new(
            f[2].parse().unwrap(),
            None,
            Decimal::parse_token(f[3]).unwrap(),
            f[4].parse().unwrap(),
        )
        .unwrap();
        by_key
            .entry((f[0].to_string(), f[1].to_string()))
            .or_default()
            .push(s);
    }
    let mut sub = Substrate::new(MemBackend::default());
    let mut series = Vec::new();
    for ((key, class), samples) in by_key {
        let uid = canonical_uid(&tel::series("prom", &key).unwrap());
        let info = SeriesInfo {
            scheme: "prom".into(),
            key,
            class,
        };
        sub.add(&uid, &inst, &info, &samples).unwrap();
        series.push(uid);
    }
    (sub, series)
}

fn spec(name: &str) -> Spec {
    Spec::parse(&std::fs::read_to_string(dir().join(format!("{name}.hjson"))).unwrap()).unwrap()
}

fn run_all() -> Vec<(&'static str, Vec<Outcome>)> {
    let (sub, series) = substrate();
    let driver = series
        .iter()
        .copied()
        .find(|s| sub.row(s).unwrap().unwrap().key == WITH)
        .unwrap();
    let mut out: Vec<(&'static str, Vec<Outcome>)> = RECIPES
        .iter()
        .map(|name| {
            let spec = spec(name);
            let with = spec.pairs().then_some(With {
                series: driver,
                instrument: None,
            });
            (
                *name,
                run_on(&spec, &sub, &series, None, with, FROM, TO).unwrap(),
            )
        })
        .collect();
    out.extend(run_tent(&sub, &series));
    out
}

/// `tent/1` over the journal in each window, and over one metric series (refused); then a
/// recipe naming another Drain's templates, which none are stored of.
fn run_tent(sub: &Substrate<MemBackend>, series: &[Uid]) -> Vec<(&'static str, Vec<Outcome>)> {
    let mut sub = sub.clone();
    let stream = journal(&mut sub);
    let osc = series
        .iter()
        .copied()
        .find(|s| sub.row(s).unwrap().unwrap().key == "info_osc")
        .unwrap();
    let mut out: Vec<(&'static str, Vec<Outcome>)> = TENT_WINDOWS
        .iter()
        .map(|(name, from, to)| {
            let r = logs::run_on(&spec("tent"), &sub, [], &[stream, osc], None, *from, *to);
            (*name, r.unwrap().outcomes)
        })
        .collect();
    let other = logs::run_on(&spec("tent-other"), &sub, [], &[stream], None, FROM, TO);
    out.push(("tent-other", other.unwrap().outcomes));
    out
}

fn report(runs: &[(&str, Vec<Outcome>)]) -> String {
    let mut out = String::from(
        "# RFC SMYSL-5 GE-I1 / C-Tel-Info: every recipe over fixtures/tel/info/, by uid.\n\
         # Written by crates/smysl-tel/tests/info_cases.rs; SMYSL_BLESS=1 rewrites it.\n\
         # case\tseries\tuid or `refused`\tvalue\tn\tk\tmissing\tthe rest, or the refusal\n\
         # the rest: mm (ent, perm); lag, bias, k_with, null_ge/null_b and the curve (mi)\n",
    );
    for (name, outcomes) in runs {
        for o in outcomes {
            match &o.result {
                Ok(c) => {
                    let mut rest = Vec::new();
                    if let Some(mm) = &c.mm {
                        rest.push(format!("mm={mm}"));
                    }
                    if let Some(lag) = c.lag {
                        rest.push(format!("lag={lag}"));
                    }
                    if let Some(b) = &c.bias {
                        rest.push(format!("bias={b}"));
                    }
                    if let Some(k) = c.k_with {
                        rest.push(format!("k_with={k}"));
                    }
                    if let Some((ge, b)) = c.null {
                        rest.push(format!("null={ge}/{b}"));
                    }
                    if !c.values.is_empty() {
                        let v: Vec<String> = c.values.iter().map(|(_, v)| v.to_string()).collect();
                        rest.push(format!("values={}", v.join(",")));
                    }
                    writeln!(
                        out,
                        "{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                        o.key,
                        canonical_uid(&c.unit).canonical(),
                        c.value,
                        c.n,
                        c.k,
                        c.missing,
                        rest.join(" ")
                    )
                }
                Err(r) => writeln!(out, "{name}\t{}\trefused\t{}\t{}", o.key, r.code, r.message),
            }
            .unwrap();
        }
    }
    out
}

/// The bytes of every unit and edge, for comparing two runs.
fn bytes(runs: &[(&str, Vec<Outcome>)]) -> Vec<u8> {
    let records: Vec<Record> = runs
        .iter()
        .flat_map(|(_, os)| os.iter())
        .filter_map(|o| o.result.as_ref().ok())
        .flat_map(|c| {
            std::iter::once(Record::Unit(c.unit.clone()))
                .chain(c.relations.iter().cloned().map(Record::Relation))
        })
        .collect();
    to_cbor_seq(&records)
}

#[test]
fn every_statistic_replays_to_the_byte_and_matches_its_pins() {
    let (first, second) = (run_all(), run_all());
    assert_eq!(bytes(&first), bytes(&second), "two runs, two byte strings");
    let got = report(&first);
    let path = dir().join("expected.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path).expect("fixtures/tel/info/expected.tsv");
    assert_eq!(
        got, want,
        "an info unit's bytes moved; SMYSL_BLESS=1 rewrites expected.tsv, which C-Tel-Info pins"
    );
}

/// The refusals the fixture is built to show, each by its code.
#[test]
fn the_refusals_are_the_ones_the_fixture_plants() {
    let runs = run_all();
    let code = |case: &str, key: &str| {
        let (_, os) = runs.iter().find(|(n, _)| *n == case).unwrap();
        let o = os.iter().find(|o| o.key == key).unwrap();
        o.result.as_ref().err().map(|r| r.code.as_str())
    };
    // A counter's level and a histogram are not read (SMY-E701); a counter's increase is.
    assert_eq!(code("ent-width", "info_requests"), Some("SMY-E701"));
    assert_eq!(code("ent-width", "info_hist"), Some("SMY-E701"));
    assert_eq!(code("ent-increase", "info_requests"), None);
    // The noise's gap and stale marker leave points without a symbol: under cover 1/1, refused.
    assert_eq!(code("ent-strict", "info_noise"), Some("SMY-W703"));
    assert_eq!(code("ent-strict", "info_osc"), None);
    // A gauge's increase is missing wherever it falls (§5.2): the noise and the oscillation
    // fall half the time, and are refused for coverage.
    assert_eq!(code("ent-increase", "info_noise"), Some("SMY-W703"));
    assert_eq!(code("ent-increase", "info_osc"), Some("SMY-W703"));
    // m = 4 at τ = 2 wants 240 patterns; the noise's gaps leave 316, enough.
    assert_eq!(code("perm-4-tau2", "info_noise"), None);
    // Ranks are refused before any series is read.
    let rank = std::fs::read_to_string(dir().join("ent-rank.hjson")).unwrap();
    assert_eq!(Spec::parse(&rank).unwrap_err().code.as_str(), "SMY-E702");
    // tent/1 reads a log stream, not a metric series; and only the shipped Drain is run.
    assert_eq!(code("tent", "info_osc"), Some("SMY-E701"));
    assert_eq!(
        code("tent-other", r#"{"host":"h1","unit":"cart.service"}"#),
        Some("SMY-W703")
    );
    // So are baseline bins over two series (R3-1).
    let base = std::fs::read_to_string(dir().join("mi-baseline.hjson")).unwrap();
    assert_eq!(Spec::parse(&base).unwrap_err().code.as_str(), "SMY-E702");
    // A pair statistic refuses a counter's level on either side, as one series does.
    assert_eq!(code("mi-rank", "info_requests"), Some("SMY-E701"));
    // The noise's gap leaves 336 of 360 pairs, enough under 4/5; its cover holds at every lag.
    assert_eq!(code("mi-scan", "info_noise"), None);
}

/// What the pair and window statistics are built to show.
#[test]
fn the_follower_peaks_three_minutes_behind_and_the_shift_diverges() {
    let runs = run_all();
    let got = |case: &str, key: &str| {
        let (_, os) = runs.iter().find(|(n, _)| *n == case).unwrap();
        os.iter()
            .find(|o| o.key == key)
            .unwrap()
            .result
            .clone()
            .unwrap()
    };
    let f = got("mi-scan", "info_follower");
    assert_eq!(
        f.lag,
        Some(-3),
        "the follower is the driver three minutes late"
    );
    assert_eq!(f.values.len(), 11);
    assert_eq!(
        f.null,
        Some((0, 50)),
        "no rotation does as well as the true alignment"
    );
    assert_eq!(got("mi-scan", "info_driver").lag, Some(0));
    // An unrelated series is matched by chance alignments.
    let n = got("mi-scan", "info_osc");
    assert!(n.null.unwrap().0 > 0, "{:?}", n.null);
    // The shifted series differs from its first two hours far more than the noise does.
    let shift = got("jsd-width", "info_shift").value.to_string();
    let osc = got("jsd-width", "info_osc").value.to_string();
    assert!(
        shift.parse::<f64>().unwrap() > 10.0 * osc.parse::<f64>().unwrap().max(1e-3),
        "shift {shift}, osc {osc}"
    );
    // A unit over two series grounds on both, with an edge to each.
    assert_eq!(f.relations.len(), 2);
}

/// A sample the window does not hold changes the digest and so the unit; one outside it does
/// not (§3, "What is read").
#[test]
fn only_the_windows_samples_enter_the_unit() {
    let (sub, series) = substrate();
    let osc = series
        .iter()
        .copied()
        .find(|s| sub.row(s).unwrap().unwrap().key == "info_osc")
        .unwrap();
    let s = spec("ent-width");
    let a = run_on(&s, &sub, &[osc], None, None, FROM, FROM + 300 * 60_000).unwrap();
    let mut sub2 = sub;
    let inst = sub2.instruments(&osc).unwrap()[0];
    let row = sub2.row(&osc).unwrap().unwrap();
    let info = SeriesInfo {
        scheme: row.scheme,
        key: row.key,
        class: row.class,
    };
    let late = Sample::new(TO + 60_000, None, Decimal::from_i64(99), 0).unwrap();
    sub2.add(&osc, &inst, &info, &[late]).unwrap();
    let b = run_on(&s, &sub2, &[osc], None, None, FROM, FROM + 300 * 60_000).unwrap();
    let uid = |o: &[Outcome]| canonical_uid(&o[0].result.as_ref().unwrap().unit);
    assert_eq!(uid(&a), uid(&b));
}

/// §6.4: a template entropy is a function of its template units. Recomputed from them alone,
/// with the events gone from the substrate, it is the same unit; and the timeouts of the last
/// two hours raise it.
#[test]
fn a_template_entropy_is_its_templates_and_outlives_the_events() {
    let (base, _) = substrate();
    let mut sub = base.clone();
    let stream = journal(&mut sub);
    let tent = spec("tent");
    let first = logs::run_on(&tent, &sub, [], &[stream], None, FROM, TO).unwrap();
    assert!(!first.materialised.is_empty());
    let units: Vec<_> = first.materialised.iter().map(|t| t.unit()).collect();
    // The events are gone: only the template units are read, and nothing is materialised.
    let again = logs::run_on(&tent, &base, &units, &[stream], None, FROM, TO).unwrap();
    assert!(again.materialised.is_empty());
    let uid = |r: &logs::Run| canonical_uid(&r.outcomes[0].result.as_ref().unwrap().unit);
    assert_eq!(uid(&first), uid(&again));

    let value = |from, to| {
        let r = logs::run_on(&tent, &sub, [], &[stream], None, from, to).unwrap();
        let c = r.outcomes[0].result.clone().unwrap();
        (c.value.to_string().parse::<f64>().unwrap(), c.k)
    };
    let (normal, k_normal) = value(FROM, FROM + 240 * 60_000);
    let (faulty, k_faulty) = value(FROM + 240 * 60_000, TO);
    assert!(faulty > normal, "faulty {faulty}, normal {normal}");
    assert!(k_faulty > k_normal, "a timeout template appears");
}

/// Every unit the fixture writes, with the substrate it was computed from, the journal added,
/// and the journal's stream.
fn units_and_substrate() -> (Vec<smysl_core::UnitCore>, Substrate<MemBackend>, Uid) {
    let (mut sub, _) = substrate();
    let stream = journal(&mut sub);
    let units = run_all()
        .into_iter()
        .flat_map(|(_, o)| o)
        .filter_map(|o| o.result.ok().map(|c| c.unit))
        .collect();
    (units, sub, stream)
}

/// RFC SMYSL-5 §7 (TI-P4): `tel-evidence` recomputes every unit the fixture writes, and each is
/// itself; a late sample in a window makes its units `SMY-E604` on the digest.
#[test]
fn every_statistic_recomputes_from_what_it_read() {
    use smysl_core::diag::Code;
    use smysl_tel::info::verify::verify;
    let (units, mut sub, _) = units_and_substrate();
    assert!(units.len() > 20, "{} units", units.len());
    let clean = verify(&units, &[], &sub);
    assert!(clean.is_empty(), "{clean:?}");

    // A sample arrives late, inside the window of every unit of info_osc.
    let osc = canonical_uid(&tel::series("prom", "info_osc").unwrap());
    let inst = sub.instruments(&osc).unwrap()[0];
    let row = sub.row(&osc).unwrap().unwrap();
    let info = SeriesInfo {
        scheme: row.scheme,
        key: row.key,
        class: row.class,
    };
    let late = Sample::new(FROM + 90 * 60_000 + 30_000, None, Decimal::from_i64(99), 7).unwrap();
    sub.add(&osc, &inst, &info, &[late]).unwrap();
    let late = verify(&units, &[], &sub);
    let reads_osc = |u: &smysl_core::UnitCore| {
        let f = tel::info_fields(u.payload.as_deref().unwrap()).unwrap();
        f.series.as_deref() == Some(&osc.canonical()[..]) || f.with == Some(osc.canonical())
    };
    let expect = units.iter().filter(|u| reads_osc(u)).count();
    assert!(expect > 0);
    assert_eq!(late.len(), expect, "{late:?}");
    assert!(late
        .iter()
        .all(|d| d.code == Code::E604 && d.message.contains("digest")));
}

/// §7: with the samples gone a unit is `SMY-W605` and nothing else, except a template entropy,
/// which is recomputed from its templates; a recipe id this build does not have is `SMY-W704`,
/// once for all its units.
#[test]
fn a_statistic_without_its_samples_is_w605_and_tent_needs_only_its_templates() {
    use smysl_core::diag::Code;
    use smysl_tel::info::verify::verify;
    let (units, sub, stream) = units_and_substrate();
    let tents: Vec<_> = units
        .iter()
        .filter(|u| {
            tel::info_fields(u.payload.as_deref().unwrap())
                .unwrap()
                .stat
                .as_deref()
                == Some("template-entropy")
        })
        .cloned()
        .collect();
    assert_eq!(tents.len(), TENT_WINDOWS.len());
    let mut templates = Vec::new();
    for (_, from, to) in TENT_WINDOWS {
        let r = logs::run_on(&spec("tent"), &sub, [], &[stream], None, *from, *to).unwrap();
        templates.extend(r.materialised.iter().map(|t| t.unit()));
    }
    let refs: Vec<&smysl_core::UnitCore> = templates.iter().collect();
    let empty = Substrate::new(MemBackend::default());
    let d = verify(&units, &refs, &empty);
    assert_eq!(d.len(), units.len() - tents.len(), "{d:?}");
    assert!(d.iter().all(|d| d.code == Code::W605));
    // Without the templates either, the template entropies are W605 too.
    assert_eq!(verify(&tents, &[], &empty).len(), tents.len());
    assert!(verify(&tents, &[], &empty)
        .iter()
        .all(|d| d.code == Code::W605));

    // `ent/9`, a version this build does not have: one W704 for both units.
    let renamed: Vec<_> = units
        .iter()
        .filter(|u| {
            tel::info_fields(u.payload.as_deref().unwrap())
                .unwrap()
                .stat
                .as_deref()
                == Some("entropy")
        })
        .take(2)
        .map(|u| {
            let mut u = u.clone();
            let mut p = u.payload.take().unwrap();
            let i = p.windows(6).position(|w| w == b"ent/1?").unwrap();
            p[i + 4] = b'9';
            u.payload = Some(p);
            u
        })
        .collect();
    assert_eq!(renamed.len(), 2);
    let d = verify(&renamed, &[], &sub);
    assert_eq!(d.len(), 1, "{d:?}");
    assert_eq!(d[0].code, Code::W704);
    assert!(
        d[0].message.contains("2 info unit(s) by `ent/9`"),
        "{}",
        d[0].message
    );
}
