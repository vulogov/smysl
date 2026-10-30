//! TX-P9 N2's exit (SMYSL-2.6 §6): A8's distortion is non-increasing in budget over random
//! stores drawn from `fixtures/corpus` (§5.2); its curve over every fixture is pinned in
//! `fixtures/measure/a8.tsv`, which the matrix compares byte for byte on every target; and a
//! scope in two languages, or tagged `mul`, is refused under `smysl/utf8-div4` (`SMY-E407`).
//!
//! `SMYSL_BLESS=1` rewrites the golden.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::diag::Code;
use smysl_core::ids::{LangTag, Uid};
use smysl_core::types::{Carry, SourceKind, SourceRef};
use smysl_core::{canonical_uid, KernelType, Record, Status, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_measure::graph::rd;
use smysl_measure::{
    run, to_payload, Input, MeasureResult, Num, Outcome, Param, Recipe, Resolved, Value,
};
use smysl_pack::Estimator;
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input as ReadInput;

const SCOPE: &str = "from texts";

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn corpus() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(repo("fixtures/corpus"))
        .expect("corpus directory")
        .map(|e| e.expect("corpus entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "smy"))
        .collect();
    out.sort();
    out
}

fn store_of(path: &Path) -> Store {
    let src = std::fs::read_to_string(path).unwrap();
    let out = smysl_core::surface::parse_surface(&src).unwrap();
    Store::from_records(out.records)
}

fn name(path: &Path) -> String {
    path.file_stem().unwrap().to_string_lossy().into_owned()
}

fn a8(recipe: &Recipe, store: &Store, uids: &BTreeSet<Uid>) -> MeasureResult {
    let scope = Resolved::of_uids(SCOPE, uids.clone());
    let input = Input::new(store, &scope);
    run(&["A8"], recipe, &input).remove(0)
}

fn value(r: &MeasureResult) -> &Value {
    match &r.outcome {
        Outcome::Value(v) => v,
        other => panic!("A8 did not run: {other:?}"),
    }
}

fn fixed(n: &Num) -> i64 {
    match n {
        Num::Fixed(x) => *x,
        other => panic!("not a fixed-point number: {other:?}"),
    }
}

/// splitmix64, so the subsets are the same on every machine and need no crate.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// §5.2: D is non-increasing in budget. Every fixture whole, and twenty random halves of each
/// at two resolutions. The greedy pack's own loss is not, on a few halves (see `graph::rd`):
/// the curve is their running minimum, which this checks against the packs point by point,
/// and the test requires both that some half's packs rise and that no whole fixture's do. A whole store is closed under rule R, so at its full budget nothing is
/// left out and the curve ends at 0; a half can hold a claim without its rebuttal, which the
/// pack will not ship alone and which costs more than the half's own full budget, so a half's
/// curve may end above 0 — and does, on some of them, which the test also requires.
#[test]
fn distortion_never_rises_with_the_budget_though_a_pack_may() {
    let mut rng = Rng(0x005e_eda8);
    let mut checked = 0;
    let mut open = 0;
    let mut rises = 0;
    for path in corpus() {
        let store = store_of(&path);
        let all: BTreeSet<Uid> = store.units().map(|(u, _)| *u).collect();
        let mut scopes = vec![all.clone()];
        for _ in 0..20 {
            let half: BTreeSet<Uid> = all
                .iter()
                .filter(|_| rng.next() % 2 == 0)
                .copied()
                .collect();
            if !half.is_empty() {
                scopes.push(half);
            }
        }
        for steps in [20, 50] {
            let mut recipe = Recipe::new("", 0, SCOPE);
            recipe.params.insert("steps".into(), Param::Int(steps));
            for s in &scopes {
                let r = a8(&recipe, &store, s);
                let v = value(&r);
                let d: Vec<i64> = v.series["D"].iter().map(fixed).collect();
                assert_eq!(
                    d.len() as i64,
                    steps,
                    "{}: every point admitted",
                    name(&path)
                );
                for w in d.windows(2) {
                    assert!(w[1] <= w[0], "{}: D rose: {d:?}", name(&path));
                }
                let packed: Vec<i64> = v.series["D:packed"].iter().map(fixed).collect();
                let raised = packed.windows(2).filter(|w| w[1] > w[0]).count() as u64;
                // The curve is the packs' running minimum: equal where no pack lost more
                // than an earlier one, below where one did, and a rise is counted.
                for (p, e) in packed.iter().zip(&d) {
                    assert!(e <= p, "{}: {d:?} above {packed:?}", name(&path));
                }
                assert_eq!(
                    v.n["rises"],
                    packed.iter().zip(&d).filter(|(p, e)| p > e).count() as u64
                );
                if raised > 0 {
                    assert!(
                        s != &all,
                        "{}: a whole fixture's packs rose: {packed:?}",
                        name(&path)
                    );
                    rises += 1;
                } else {
                    assert_eq!(packed, d, "{}", name(&path));
                }
                if s == &all {
                    assert_eq!(
                        d.last(),
                        Some(&0),
                        "{}: the full budget loses nothing",
                        name(&path)
                    );
                } else if d.last() > Some(&0) {
                    open += 1;
                }
                assert_eq!(v.n["refused"], 0);
                checked += 1;
            }
        }
    }
    assert!(checked > 400, "{checked} curves");
    assert!(open > 0, "no half cut a claim from its rebuttal");
    assert!(
        rises > 0,
        "no half's packs rose: the running minimum is then untested"
    );
    eprintln!("{checked} curves, {open} ending above 0, {rises} with a pack that rose");
}

/// The two token counts are the ones E1's chain used, and A8's first budget is the rounded
/// twentieth of the second.
#[test]
fn the_token_counts_moved_with_their_definitions() {
    let est = Estimator::default();
    let store = store_of(&repo("fixtures/corpus/F2-research.smy"));
    let all: BTreeSet<Uid> = store.units().map(|(u, _)| *u).collect();
    let full = rd::full_tokens(&store, &all, &est);
    let floor = rd::floor_tokens(&store, &all, &est);
    assert!(floor > 0 && floor <= full, "{floor} ≤ {full}");
    let r = a8(&Recipe::new("", 0, SCOPE), &store, &all);
    let v = value(&r);
    assert_eq!(v.n["full:tokens"], full);
    assert_eq!(v.n["floor:tokens"], floor);
    let first = (u128::from(full) * 2 + 20) / 40; // half to even only matters at .5
    let Num::Count(b0) = v.series["budget"][0] else {
        panic!("a budget is a count")
    };
    assert!(b0.abs_diff(first as u64) <= 1, "{b0} against {first}");
    assert_eq!(v.series["budget"].last(), Some(&Num::Count(full)));
    assert_eq!(
        r.recipe.estimator.as_deref(),
        Some("smysl/utf8-div4"),
        "the estimator is written into the recipe"
    );
}

/// What a recipe A8 cannot read is refused before any pack.
#[test]
fn a8_refuses_a_recipe_it_cannot_read() {
    let store = store_of(&repo("fixtures/corpus/F2-research.smy"));
    let all: BTreeSet<Uid> = store.units().map(|(u, _)| *u).collect();
    let refused = |r: &Recipe| match a8(r, &store, &all).outcome {
        Outcome::NotRun { code: Some(c), .. } => c,
        other => panic!("expected a refusal: {other:?}"),
    };
    let mut r = Recipe::new("", 0, SCOPE);
    r.estimator = Some("smysl/tiktoken".into());
    assert_eq!(refused(&r), Code::E489);
    let mut r = Recipe::new("", 0, SCOPE);
    r.params.insert("steps".into(), Param::Int(0));
    assert_eq!(refused(&r), Code::E489);
    let mut r = Recipe::new("", 0, SCOPE);
    r.params.insert("lambda".into(), Param::Ratio(1, 0));
    assert_eq!(refused(&r), Code::E489);
    let mut r = Recipe::new("", 0, SCOPE);
    r.params.insert("steps".into(), Param::Int(1));
    assert_eq!(refused(&r), Code::W485, "one point is not a curve");
}

fn blessing() -> bool {
    std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1")
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Bit identity across the matrix: A8 over every fixture whole, its recipe hash, counts, curve,
/// area and payload bytes, compared with the committed golden on every target CI builds.
#[test]
fn a8_over_the_corpus_is_the_golden() {
    let mut got = String::from("# TX-P9 N2: A8 over each fixtures/corpus store, whole\n");
    got.push_str("# fixture\tkey\tvalue\n");
    let recipe = Recipe::new("", 0, SCOPE);
    for path in corpus() {
        let store = store_of(&path);
        let all: BTreeSet<Uid> = store.units().map(|(u, _)| *u).collect();
        let r = a8(&recipe, &store, &all);
        let v = value(&r);
        let f = name(&path);
        got.push_str(&format!(
            "{f}\trecipe.hash\t{}\n",
            hex(&r.recipe.hash().unwrap())
        ));
        for (k, n) in &v.n {
            got.push_str(&format!("{f}\tn.{k}\t{n}\n"));
        }
        for (k, n) in &v.values {
            got.push_str(&format!("{f}\tvalue.{k}\t{n}\n"));
        }
        for (k, s) in &v.series {
            let s: Vec<String> = s.iter().map(ToString::to_string).collect();
            got.push_str(&format!("{f}\tseries.{k}\t{}\n", s.join(" ")));
        }
        got.push_str(&format!(
            "{f}\tpayload\t{}\n",
            hex(&to_payload(&r.recipe, v).unwrap())
        ));
    }
    let path = repo("fixtures/measure/a8.tsv");
    if blessing() {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; SMYSL_BLESS=1 writes it", path.display()));
    assert_eq!(want, got, "{} changed", path.display());
}

// ---- languages -----------------------------------------------------------------------------

struct Built {
    root: PathBuf,
    library: Library,
}

impl Drop for Built {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Ruth in the KJV and the Synodal, as `kjv/08-rut` and `synodal/08-rut`.
fn library() -> Built {
    let root = std::env::temp_dir().join(format!("smysl-measure-rd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut library = Library::create(&root).unwrap();
    for (alias, lang, dir) in [("kjv", "en", "eng-kjv"), ("synodal", "ru", "russyn")] {
        let raw = std::fs::read(repo(&format!(
            "fixtures/library/bibles/slice/{dir}/08-RUT.usfm"
        )))
        .unwrap();
        let spec = AddSpec::new("usfm/1", format!("{alias}/08-rut"), "public-domain")
            .with_lang(LangTag::new(lang).unwrap())
            .with_carry(Carry::Text);
        library
            .add(&ReadInput::new(&raw), &spec, &Caps::DEFAULT)
            .unwrap_or_else(|e| panic!("{alias}: {e}"));
    }
    Built { root, library }
}

/// Three claims from each translation's first part, with the manifests beside them; `lang`
/// rewrites every manifest's language, for a `mul` one.
fn drawn(lib: &Library, lang: Option<&str>) -> (Store, Vec<(String, BTreeSet<Uid>)>) {
    let mut records = Vec::new();
    let mut by = Vec::new();
    for (k, (_, m)) in lib.store().manifests().enumerate() {
        let mut m = m.clone();
        if let Some(l) = lang {
            m.lang = LangTag::new(l).unwrap();
            for p in &mut m.parts {
                p.lang = None;
            }
        }
        let tid = m.parts[0].tid;
        let mut uids = BTreeSet::new();
        for i in 0..3 {
            let core = UnitCoreBuilder::new(
                KernelType::Claim,
                format!("claim {i} of text {k}"),
                Status::Cited,
            )
            .source(SourceRef::new(SourceKind::Doc, tid.canonical()))
            .build()
            .unwrap();
            uids.insert(canonical_uid(&core));
            records.push(Record::Unit(core));
        }
        by.push((m.alias.clone(), uids));
        records.push(Record::Manifest(m));
    }
    by.sort();
    (Store::from_records(records), by)
}

/// `utf8-div4` over English and Russian together is refused (`SMY-E407`), and over either
/// alone it runs; a scope whose text is tagged `mul` is refused on its own.
#[test]
fn a_byte_estimator_is_refused_across_languages() {
    let built = library();
    let (store, by) = drawn(&built.library, None);
    let recipe = Recipe::new("", 0, SCOPE);
    let both: BTreeSet<Uid> = by.iter().flat_map(|(_, u)| u.iter().copied()).collect();
    match a8(&recipe, &store, &both).outcome {
        Outcome::NotRun {
            code: Some(Code::E407),
            why,
        } => {
            assert!(why.contains("en, ru"), "{why}");
        }
        other => panic!("expected SMY-E407: {other:?}"),
    }
    for (alias, uids) in &by {
        let r = a8(&recipe, &store, uids);
        assert!(
            matches!(r.outcome, Outcome::Value(_)),
            "{alias}: {:?}",
            r.outcome
        );
    }

    let (store, by) = drawn(&built.library, Some("mul"));
    let r = a8(&recipe, &store, &by[0].1);
    assert!(
        matches!(
            r.outcome,
            Outcome::NotRun {
                code: Some(Code::E407),
                ..
            }
        ),
        "{:?}",
        r.outcome
    );
}
