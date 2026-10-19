//! TX-P9 N1's exit (SMYSL-2.6 §6), the graph half: A1–A7 and A10 on synthetic stores with
//! planted structure (§5.2) — k self-repetitions, a known status mix, chains of known grounds
//! depth, salience all equal, a known class sequence — and A1 and the per-text figures over a
//! library built from the committed Bible slices.
//!
//! Every expectation is a count planted by the test, so each assertion is exact: a ratio is
//! compared as its two integers and a fixed-point number as its integer.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::diag::Code;
use smysl_core::ids::{AgentId, LangTag, Uid};
use smysl_core::types::{
    Carry, Contention, Detected, DetectionKind, Hlc, RelKind, SourceKind, SourceRef,
};
use smysl_core::ContentionId;
use smysl_core::{canonical_uid, KernelType, Record, Relation, Status, UnitCore, UnitCoreBuilder};
use smysl_graph::lineage::{trace, TraceKind};
use smysl_graph::proposition::SAME_AS;
use smysl_graph::Store;
use smysl_measure::graph::{concentration, grounding};
use smysl_measure::{
    run, to_unit, Basis, ClassPolicy, Input, MeasureResult, Num, Outcome, Param, Recipe, Resolved,
    Value,
};
use smysl_text::analyze::Chain;
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input as ReadInput;

const SCOPE: &str = "from texts";

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// A unit numbered `i`, so its uid is a function of `i` and nothing else in the test.
fn unit(i: u32, kind: KernelType, status: Status) -> UnitCoreBuilder {
    UnitCoreBuilder::new(kind, format!("planted unit {i}"), status)
}

fn claim(i: u32) -> UnitCore {
    unit(i, KernelType::Claim, Status::Speculative)
        .build()
        .unwrap()
}

fn cited(i: u32, status: Status) -> UnitCore {
    unit(i, KernelType::Claim, status)
        .source(SourceRef::new(SourceKind::Doc, format!("doc-{i}")))
        .build()
        .unwrap()
}

fn same_as(a: Uid, b: Uid) -> Record {
    Record::Relation(Relation::new(RelKind::Extension(SAME_AS.into()), a, b))
}

fn store(records: Vec<Record>) -> Store {
    let mut s = Store::new();
    s.append(&records).expect("appends");
    s
}

fn uids_of(cores: &[UnitCore]) -> Vec<Uid> {
    cores.iter().map(canonical_uid).collect()
}

fn scope(uids: &[Uid]) -> Resolved {
    Resolved::of_uids(SCOPE, uids.iter().copied().collect())
}

fn base() -> Recipe {
    Recipe::new("", 0, SCOPE)
}

fn one(id: &str, recipe: &Recipe, input: &Input<'_>) -> MeasureResult {
    run(&[id], recipe, input).remove(0)
}

fn value(r: &MeasureResult) -> &Value {
    match &r.outcome {
        Outcome::Value(v) => v,
        other => panic!("{} did not run: {other:?}", r.recipe.id),
    }
}

fn refusal(r: &MeasureResult) -> Code {
    match &r.outcome {
        Outcome::NotRun { code: Some(c), .. } => *c,
        other => panic!("{} was expected to refuse: {other:?}", r.recipe.id),
    }
}

fn agent() -> AgentId {
    AgentId::new("human:tester").unwrap()
}

/// §5.2: k self-repetitions give A2 = k / units. Four units claimed pairwise the same are one
/// class, so ten units with that clique among them are seven classes and `[3, 10]`; with no
/// edges at all the scope is `[0, 10]` exactly. Without a policy A2 is refused (`SMY-E481`).
#[test]
fn a2_counts_planted_repetitions() {
    let cores: Vec<UnitCore> = (0..10).map(claim).collect();
    let u = uids_of(&cores);
    let mut records: Vec<Record> = cores.iter().cloned().map(Record::Unit).collect();
    for a in 0..4 {
        for b in a + 1..4 {
            records.push(same_as(u[a], u[b]));
        }
    }
    let s = store(records);
    let sc = scope(&u);
    let input = Input::new(&s, &sc);
    let mut r = base();
    r.policy = Some(ClassPolicy::Strict);
    let a2 = one("A2", &r, &input);
    let v = value(&a2);
    assert_eq!(v.values["redundancy"], Num::Ratio(3, 10));
    assert_eq!(v.n["classes"], 7);

    let bare = store(cores.iter().cloned().map(Record::Unit).collect());
    let input = Input::new(&bare, &sc);
    assert_eq!(
        value(&one("A2", &r, &input)).values["redundancy"],
        Num::Ratio(0, 10)
    );

    assert_eq!(refusal(&one("A2", &base(), &input)), Code::E481);
}

/// The three policies give three numbers on a path `a—b—c`: `strict` two classes, `component`
/// one. The policy is in the recipe, so they are three units.
#[test]
fn a2_follows_the_policy_in_its_recipe() {
    let cores: Vec<UnitCore> = (0..3).map(claim).collect();
    let mut u = uids_of(&cores);
    u.sort();
    let mut records: Vec<Record> = cores.iter().cloned().map(Record::Unit).collect();
    records.push(same_as(u[0], u[1]));
    records.push(same_as(u[1], u[2]));
    let s = store(records);
    let sc = scope(&u);
    let input = Input::new(&s, &sc);
    let at = |p: ClassPolicy| {
        let mut r = base();
        r.policy = Some(p);
        value(&one("A2", &r, &input)).values["redundancy"]
    };
    assert_eq!(at(ClassPolicy::Strict), Num::Ratio(1, 3));
    assert_eq!(at(ClassPolicy::Component), Num::Ratio(2, 3));
    // Nobody attested either edge, so under `attested:1` there are none.
    assert_eq!(at(ClassPolicy::Attested(1)), Num::Ratio(0, 3));
}

/// §5.2, A3: a store whose class sequence is known. A hundred units observed one millisecond
/// apart, unit i claimed the same as unit i − 50 for the second half: the first fifty are new
/// and the rest are not, so the integral is `[50, 100]`, the curve's first 32 bins are all
/// new and its last 32 none, and the curve's new units add up to the classes.
#[test]
fn a3_integral_is_classes_over_units() {
    let cores: Vec<UnitCore> = (0..100u32)
        .map(|i| {
            unit(i, KernelType::Claim, Status::Speculative)
                .source(
                    SourceRef::new(SourceKind::Doc, format!("chat#{i}"))
                        .observed_at(1_700_000_000_000 + u64::from(i)),
                )
                .build()
                .unwrap()
        })
        .collect();
    let u = uids_of(&cores);
    let mut records: Vec<Record> = cores.iter().cloned().map(Record::Unit).collect();
    for i in 50..100 {
        records.push(same_as(u[i - 50], u[i]));
    }
    let s = store(records.clone());
    let sc = scope(&u);
    let input = Input::new(&s, &sc);
    let mut r = base();
    r.policy = Some(ClassPolicy::Strict);
    r.params.insert("order".into(), Param::Text("time".into()));
    let a3 = one("A3", &r, &input);
    let v = value(&a3);
    assert_eq!(v.values["novelty"], Num::Ratio(50, 100));
    let curve = &v.series["curve"];
    assert_eq!(curve.len(), 64);
    let new: u64 = curve
        .iter()
        .map(|n| match n {
            Num::Ratio(k, _) => *k,
            _ => panic!("a bin is a ratio"),
        })
        .sum();
    assert_eq!(new, 50, "every class is new exactly once");
    for (j, bin) in curve.iter().enumerate() {
        let Num::Ratio(k, n) = *bin else {
            unreachable!()
        };
        if j < 32 {
            assert_eq!(k, n, "bin {j} is all new");
        } else {
            assert_eq!(k, 0, "bin {j} has nothing new");
        }
    }
    assert!(a3.diagnostics.is_empty(), "{:?}", a3.diagnostics);

    // Five undated units: left out, counted, and said (`SMY-W415`); 5 of 105 is under 1/10.
    let undated: Vec<UnitCore> = (100..105).map(claim).collect();
    let mut with: Vec<Uid> = u.clone();
    with.extend(uids_of(&undated));
    records.extend(undated.iter().cloned().map(Record::Unit));
    let s = store(records.clone());
    let sc = scope(&with);
    let input = Input::new(&s, &sc);
    let a3 = one("A3", &r, &input);
    let v = value(&a3);
    assert_eq!(v.n["undated"], 5);
    assert_eq!(v.values["novelty"], Num::Ratio(50, 100));
    assert!(v.flags.iter().any(|f| f == "undated-excluded"));
    assert!(
        a3.diagnostics.iter().any(|d| d.code == Code::W415),
        "{:?}",
        a3.diagnostics
    );

    // Twenty more, 25 of 125, is above it: refused.
    let more: Vec<UnitCore> = (105..125).map(claim).collect();
    with.extend(uids_of(&more));
    records.extend(more.iter().cloned().map(Record::Unit));
    let s = store(records);
    let sc = scope(&with);
    let input = Input::new(&s, &sc);
    assert_eq!(refusal(&one("A3", &r, &input)), Code::E482);
    let mut lax = r.clone();
    lax.undated_max = (1, 4);
    assert_eq!(
        value(&one("A3", &lax, &input)).values["novelty"],
        Num::Ratio(50, 100)
    );

    // `known` is not an axis rule E derives, and `order` is one of two words.
    let mut known = r.clone();
    known.axis = smysl_measure::Axis::Known;
    assert_eq!(refusal(&one("A3", &known, &input)), Code::E489);
    let mut odd = r.clone();
    odd.params
        .insert("order".into(), Param::Text("alphabetical".into()));
    assert_eq!(refusal(&one("A3", &odd, &input)), Code::E489);
}

/// §5.2, A4: a known status mix. Two speculative, two cited and four measured is
/// `H = 1.5` bits exactly and a hedging rate of `[2, 8]`; a retraction moves a cited unit to
/// unfounded, which is where it is counted.
#[test]
fn a4_reads_a_planted_status_mix() {
    let mut cores: Vec<UnitCore> = (0..2).map(claim).collect();
    cores.extend((2..4).map(|i| cited(i, Status::Cited)));
    cores.extend((4..8).map(|i| cited(i, Status::Measured)));
    let u = uids_of(&cores);
    let records: Vec<Record> = cores.iter().cloned().map(Record::Unit).collect();
    let s = store(records.clone());
    let sc = scope(&u);
    let input = Input::new(&s, &sc);
    let a4 = one("A4", &base(), &input);
    let v = value(&a4);
    assert_eq!(v.values["H:status"], Num::Fixed(3 << 19));
    assert_eq!(v.values["hedging"], Num::Ratio(2, 8));
    assert_eq!(v.values["status:measured"], Num::Count(4));

    // The retracting unit is outside the scope; what it retracts is in it.
    let retractor = claim(99);
    let mut records = records;
    records.push(Record::Unit(retractor.clone()));
    records.push(Record::Relation(Relation::new(
        RelKind::Retracts,
        canonical_uid(&retractor),
        u[2],
    )));
    let s = store(records);
    let input = Input::new(&s, &sc);
    let v = value(&one("A4", &base(), &input)).clone();
    assert_eq!(v.values["status:unfounded"], Num::Count(1));
    assert_eq!(v.values["status:cited"], Num::Count(1));
    assert_eq!(v.values["hedging"], Num::Ratio(3, 8));
    // −(1/8 log 1/8 + 2/8 log 2/8 + 1/8 log 1/8 + 4/8 log 4/8) = 3/8 + 4/8 + 3/8 + 4/8 = 1.75.
    assert_eq!(v.values["H:status"], Num::Fixed(7 << 18));
}

/// §5.2, A5: chains of known grounds depth. `c0 ← c1 ← c2 ← c3` is depth 0 to 3, which is
/// what `trace` says on a chain; the shortcut `a` on `{b, c}`, `b` on `c`, is depth 2 here and
/// 1 to `trace`, which walks breadth first (see the module). A warrant on c2 is E6's `[1, 3]`.
#[test]
fn a5_measures_planted_chains() {
    let c0 = claim(0);
    let mut chain = vec![c0];
    for i in 1..4 {
        let below = canonical_uid(chain.last().unwrap());
        chain.push(
            unit(i, KernelType::Claim, Status::Speculative)
                .grounds([below])
                .build()
                .unwrap(),
        );
    }
    let u = uids_of(&chain);
    let mut records: Vec<Record> = chain.iter().cloned().map(Record::Unit).collect();
    let warrant = claim(50);
    records.push(Record::Unit(warrant.clone()));
    records.push(Record::Relation(Relation::new(
        RelKind::Warrant,
        canonical_uid(&warrant),
        u[2],
    )));
    let s = store(records);
    let depths = grounding::depths(&s, &u);
    for (k, uid) in u.iter().enumerate() {
        assert_eq!(depths[uid], k as u64);
        assert_eq!(
            u64::from(trace(&s, *uid, TraceKind::Grounds, None).max_depth()),
            k as u64,
            "a chain is a tree, where the longest chain is trace's depth"
        );
    }
    let sc = scope(&u);
    let input = Input::new(&s, &sc);
    let a5 = one("A5", &base(), &input);
    let v = value(&a5);
    assert_eq!(v.values["grounded"], Num::Ratio(3, 4));
    assert_eq!(v.values["warranted"], Num::Ratio(1, 3));
    assert_eq!(v.values["depth:max"], Num::Count(3));
    // Depths 0, 1, 2, 3: the lower median is the second, 1.
    assert_eq!(v.values["depth:median"], Num::Count(1));
    assert_eq!(v.values["unfounded"], Num::Ratio(0, 4));
    // E6 over the same set, as the harness counted it before the move.
    assert_eq!(grounding::warrant_counts(&s, &u), (1, 3));

    let c = claim(10);
    let b = unit(11, KernelType::Claim, Status::Speculative)
        .grounds([canonical_uid(&c)])
        .build()
        .unwrap();
    let a = unit(12, KernelType::Claim, Status::Speculative)
        .grounds([canonical_uid(&b), canonical_uid(&c)])
        .build()
        .unwrap();
    let s = store(vec![
        Record::Unit(c.clone()),
        Record::Unit(b.clone()),
        Record::Unit(a.clone()),
    ]);
    let ua = canonical_uid(&a);
    assert_eq!(grounding::depths(&s, &[ua])[&ua], 2);
    assert_eq!(trace(&s, ua, TraceKind::Grounds, None).max_depth(), 1);
}

/// §5.2, A6: salience all equal gives 0 exactly, and the Gini of `[0, 0, 1]` is 2/3 rounded
/// once.
#[test]
fn a6_is_zero_when_salience_is_equal() {
    let cores: Vec<UnitCore> = (0..6).map(claim).collect();
    let u = uids_of(&cores);
    let s = store(cores.iter().cloned().map(Record::Unit).collect());
    let report = smysl_graph::salience::salience(&s, &Default::default());
    let first = report.get(&u[0]);
    assert!(
        u.iter().all(|x| report.get(x) == first),
        "six unconnected units of one status are equally salient"
    );
    let sc = scope(&u);
    let input = Input::new(&s, &sc);
    assert_eq!(
        value(&one("A6", &base(), &input)).values["gini"],
        Num::Fixed(0)
    );
    assert_eq!(
        concentration::gini(&mut [1024, 0, 0]),
        Some(((2u64 << 20) as f64 / 3.0).round() as i64)
    );
    assert_eq!(concentration::gini(&mut [0, 0]), None);
}

/// A7: one open contention and one live contradiction touch the scope; a withdrawn
/// contradiction and one wholly outside it do not. Two in ten units is 20 per hundred.
#[test]
fn a7_counts_what_touches_the_scope() {
    let cores: Vec<UnitCore> = (0..12).map(claim).collect();
    let u = uids_of(&cores);
    let mut records: Vec<Record> = cores.iter().cloned().map(Record::Unit).collect();
    // A live-rebuttal contention is open while its rebuttal is live, so the edge is planted
    // with it; without the edge the store reads it as stale.
    let kind = DetectionKind::LiveRebuttal;
    records.push(Record::Relation(Relation::new(RelKind::Rebuts, u[1], u[0])));
    records.push(Record::Contention(Contention::new(
        ContentionId::derive(kind, &u[0], &[u[1]]),
        u[0],
        vec![u[1]],
        Detected::new(kind, Hlc::new(1, 0, agent())),
    )));
    let contradicts = |a: Uid, b: Uid| {
        Relation::new(
            RelKind::Extension(smysl_core::schemas::text::CONTRADICTS.into()),
            a,
            b,
        )
    };
    records.push(Record::Relation(contradicts(u[2], u[3])));
    let withdrawn = contradicts(u[4], u[5]);
    records.push(Record::Withdrawal(smysl_core::Withdrawal::new(
        withdrawn.uid(),
        agent(),
        Hlc::new(2, 0, agent()),
    )));
    records.push(Record::Relation(withdrawn));
    records.push(Record::Relation(contradicts(u[10], u[11])));
    let s = store(records);
    let sc = scope(&u[..10]);
    let input = Input::new(&s, &sc);
    let v = value(&one("A7", &base(), &input)).clone();
    assert_eq!(v.values["contentions"], Num::Count(1));
    assert_eq!(v.values["contradictions"], Num::Count(1));
    assert_eq!(v.values["per100"], Num::Fixed(20 << 20));
}

/// A10: four units observed at an instant are dated by their record, width 0; one published
/// in 2020 is a leap year wide; two have no time at all.
#[test]
fn a10_sorts_units_by_what_dated_them() {
    let mut cores: Vec<UnitCore> = (0..4u32)
        .map(|i| {
            unit(i, KernelType::Claim, Status::Speculative)
                .source(
                    SourceRef::new(SourceKind::Doc, format!("log#{i}"))
                        .observed_at(1_600_000_000_000 + u64::from(i)),
                )
                .build()
                .unwrap()
        })
        .collect();
    let mut src = SourceRef::new(SourceKind::Doc, "book");
    src.published = Some("2020".into());
    cores.push(
        unit(4, KernelType::Claim, Status::Speculative)
            .source(src)
            .build()
            .unwrap(),
    );
    cores.extend((5..7).map(claim));
    let u = uids_of(&cores);
    let s = store(cores.iter().cloned().map(Record::Unit).collect());
    let sc = scope(&u);
    let input = Input::new(&s, &sc);
    let v = value(&one("A10", &base(), &input)).clone();
    assert_eq!(v.values["recorded"], Num::Ratio(5, 7));
    assert_eq!(v.values["undated"], Num::Ratio(2, 7));
    assert_eq!(v.values["dating"], Num::Ratio(0, 7));
    assert_eq!(v.values["contested"], Num::Ratio(0, 7));
    assert_eq!(v.values["width:median"], Num::Fixed(0));
    let Num::Fixed(p90) = v.values["width:p90"] else {
        panic!("a width is fixed-point")
    };
    let days = smysl_measure::num::from_q20(p90);
    assert!((days - 366.0).abs() < 1e-3, "2020 is {days} days wide");
}

/// A unit measure's recipe states what it read: no filter when it read every unit of its
/// scope, and the filter when it was given one, as an sq predicate over the same fields a
/// scope's `where` reads. A text measure's recipe keeps draft 3's default, unchanged, so no
/// text result's uid moves.
#[test]
fn a_unit_measure_writes_the_filter_it_applied() {
    let mut cores: Vec<UnitCore> = (0..3).map(claim).collect();
    cores.push(cited(3, Status::Cited));
    let u = uids_of(&cores);
    let s = store(cores.iter().cloned().map(Record::Unit).collect());
    let sc = scope(&u);
    let input = Input::new(&s, &sc);
    let all = one("A4", &base(), &input);
    assert_eq!(all.recipe.filter, "");
    assert!(!all.recipe.to_object().unwrap().contains("filter"));
    assert_eq!(value(&all).n["units"], 4);

    let mut r = base();
    r.filter = "status = cited".into();
    let narrowed = one("A4", &r, &input);
    assert_eq!(value(&narrowed).n["units"], 1);
    assert_eq!(narrowed.recipe.filter, "status = cited");
    assert_ne!(narrowed.recipe.hash(), all.recipe.hash());

    r.filter = "status <".into();
    assert_eq!(refusal(&one("A4", &r, &input)), Code::E489);

    let mut t = Recipe::new("T1", 1, SCOPE);
    smysl_measure::find(smysl_measure::ALL, "T1", None)
        .unwrap()
        .complete(&mut t);
    assert_eq!(t.filter, smysl_measure::recipe::DEFAULT_FILTER);
}

/// A unit measure over nothing is not run (`SMY-W485`), and a result is a unit rule M accepts.
#[test]
fn empty_scopes_are_not_run_and_results_check_clean() {
    let s = store(Vec::new());
    let sc = scope(&[]);
    let input = Input::new(&s, &sc);
    for id in ["A2", "A3", "A4", "A5", "A6", "A7", "A10"] {
        let mut r = base();
        r.policy = Some(ClassPolicy::Strict);
        assert_eq!(refusal(&one(id, &r, &input)), Code::W485, "{id}");
    }

    let cores: Vec<UnitCore> = (0..4).map(claim).collect();
    let u = uids_of(&cores);
    let mut records: Vec<Record> = cores.iter().cloned().map(Record::Unit).collect();
    let s = store(records.clone());
    let sc = scope(&u);
    let input = Input::new(&s, &sc);
    let basis_unit = UnitCoreBuilder::new(KernelType::Claim, "a saved scope", Status::Cited)
        .source(SourceRef::new(SourceKind::Tool, "smysl/q"))
        .build()
        .unwrap();
    let basis = Basis {
        uid: canonical_uid(&basis_unit),
        unit: basis_unit.clone(),
        edges: Vec::new(),
    };
    let mut r = base();
    r.policy = Some(ClassPolicy::Strict);
    records.push(Record::Unit(basis_unit));
    for id in ["A2", "A4", "A5", "A7", "A10"] {
        let unit = to_unit(&one(id, &r, &input), Some(&basis)).expect("a unit");
        assert_eq!(unit.status, Status::Derived);
        assert_eq!(unit.grounds, BTreeSet::from([basis.uid]));
        records.push(Record::Unit(unit));
    }
    let written = store(records);
    let report = smysl_check::check(&written, smysl_check::CheckOptions::default());
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.code == Code::E030 || d.code == Code::E033),
        "{report:?}"
    );
}

// ---- over a library ------------------------------------------------------------------------

struct Built {
    root: PathBuf,
    library: Library,
}

impl Drop for Built {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The KJV's Ruth and Jude as `kjv/08-rut` and `kjv/65-jud`.
fn library() -> Built {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "smysl-measure-graph-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut library = Library::create(&root).unwrap();
    for book in ["08-RUT", "65-JUD"] {
        let raw = std::fs::read(repo(&format!(
            "fixtures/library/bibles/slice/eng-kjv/{book}.usfm"
        )))
        .unwrap();
        let alias = format!("kjv/{}", book.to_lowercase());
        let spec = AddSpec::new("usfm/1", &alias, "public-domain")
            .with_lang(LangTag::new("en").unwrap())
            .with_carry(Carry::Text);
        library
            .add(&ReadInput::new(&raw), &spec, &Caps::DEFAULT)
            .unwrap_or_else(|e| panic!("{alias}: {e}"));
    }
    Built { root, library }
}

/// Units drawn from each book's first part: `n` from Ruth and `m` from Jude, with the
/// manifests beside them so a unit is in its text.
fn drawn(lib: &Library, n: u32, m: u32) -> (Store, Vec<Uid>, Vec<(String, smysl_core::ids::Tid)>) {
    let mut records: Vec<Record> = Vec::new();
    let mut parts = Vec::new();
    for (mid, man) in lib.store().manifests() {
        records.push(Record::Manifest(man.clone()));
        let _ = mid;
        parts.push((man.alias.clone(), man.parts[0].tid));
    }
    parts.sort();
    let mut uids = Vec::new();
    for (k, (alias, tid)) in parts.iter().enumerate() {
        let count = if k == 0 { n } else { m };
        for i in 0..count {
            let core = unit(1000 * k as u32 + i, KernelType::Claim, Status::Speculative)
                .source(SourceRef::new(SourceKind::Doc, tid.canonical()))
                .build()
                .unwrap();
            uids.push(canonical_uid(&core));
            records.push(Record::Unit(core));
        }
        let _ = alias;
    }
    (store(records), uids, parts)
}

/// A1 over Ruth and Jude: units per thousand of the analyzer's words over the two parts they
/// are drawn from, counted here independently; refused without a profile.
#[test]
fn a1_is_units_per_thousand_words_of_their_parts() {
    let built = library();
    let lib = &built.library;
    let (s, u, parts) = drawn(lib, 30, 12);
    let chain = Chain::new(&LangTag::new("en").unwrap());
    let words: u64 = parts
        .iter()
        .map(|(_, tid)| {
            let text = String::from_utf8(lib.part(tid).unwrap().text).unwrap();
            chain.surface(&text).len() as u64
        })
        .sum();
    let sc = scope(&u);
    let input = Input::new(&s, &sc).with_library(lib);
    assert_eq!(refusal(&one("A1", &base(), &input)), Code::E489);
    let mut r = base();
    r.profile = Some("default".into());
    let a1 = one("A1", &r, &input);
    let v = value(&a1);
    assert_eq!(v.n["words"], words);
    assert_eq!(v.n["placed"], 42);
    let want = ((42u128 * 1000) << 20) as f64 / words as f64;
    assert_eq!(
        v.values["density"],
        Num::Fixed(want.round_ties_even() as i64)
    );
    assert_eq!(v.values["mode:none"], v.values["density"]);
    assert_eq!(v.values["holder:text"], v.values["density"]);
    // The chain English chooses, whichever this build has: `smysl/an-en/1` with stems,
    // `smysl/an/1` without.
    assert_eq!(
        a1.recipe.analyzer.as_deref(),
        Some(
            smysl_measure::text::analyzer_id(chain.id(), smysl_measure::text::Stream::Surface)
                .as_str()
        )
    );
}

/// Per text: A2 and A7 report each book beside the scope. Two units of Ruth claimed the same
/// as each other make Ruth redundant and Jude not.
#[test]
fn per_text_figures_follow_the_units_texts() {
    let built = library();
    let (s, u, _) = drawn(&built.library, 4, 3);
    let mut records: Vec<Record> = s.iter().cloned().collect();
    records.push(same_as(u[0], u[1]));
    let s = store(records);
    let sc = scope(&u);
    let input = Input::new(&s, &sc);
    let mut r = base();
    r.policy = Some(ClassPolicy::Strict);
    let v = value(&one("A2", &r, &input)).clone();
    assert_eq!(v.values["redundancy"], Num::Ratio(1, 7));
    assert_eq!(v.values["redundancy:kjv/08-rut"], Num::Ratio(1, 4));
    assert_eq!(v.values["redundancy:kjv/65-jud"], Num::Ratio(0, 3));
    let v = value(&one("A7", &r, &input)).clone();
    assert_eq!(v.values["per100:kjv/08-rut"], Num::Fixed(0));

    // Reading order: every unit names a part, so none is unplaced.
    r.params
        .insert("order".into(), Param::Text("reading".into()));
    let v = value(&one("A3", &r, &input)).clone();
    assert_eq!(v.n["unplaced"], 0);
    assert_eq!(v.values["novelty"], Num::Ratio(6, 7));
}

/// Registry order is report order: the text measures, then the unit measures.
#[test]
fn the_registry_lists_the_unit_measures() {
    let ids: Vec<&str> = smysl_measure::ALL.iter().map(|m| m.id()).collect();
    assert_eq!(
        ids,
        [
            "T1", "T2", "T3", "T4", "T5", "T6", "T7", "A1", "A2", "A3", "A4", "A5", "A6", "A7",
            "A10"
        ]
    );
}
