//! TX-P8 M2's exit (SMYSL-2.6 §6): T1 and T4 against §5.1's rows, the bootstrap seed rules of
//! §5.3, and both measures over a library built from the committed Bible slices.
//!
//! The synthetic texts are generated here from the crate's own generator, so the fixtures are
//! code and not files (§5.1). The library results are pinned in `fixtures/measure/text.tsv`,
//! language-free so the ports can check themselves against it; `SMYSL_BLESS=1` rewrites it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::diag::Code;
use smysl_core::ids::LangTag;
use smysl_core::types::Carry;
use smysl_core::{canonical_uid, KernelType, Status, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_measure::num::{from_q20, ln, log2, powf};
use smysl_measure::rng::Rng;
use smysl_measure::text::{self, Corpus, Stream};
use smysl_measure::{
    gist, resolve, run, to_payload, to_unit, Basis, Ci, Level, MeasureResult, Num, Outcome, Param,
    Recipe, Resolved, Value,
};
use smysl_query::{Env, PlanOptions};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input as ReadInput;
use smysl_text::scope::LibraryScope;

const SCOPE: &str = "synthetic";

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// A uniform draw in [0, 1) from 53 bits of the generator.
fn uniform(rng: &mut Rng) -> f64 {
    (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64
}

/// `tokens` cut into segments of `size`.
fn segments(tokens: Vec<String>, size: usize) -> Vec<Vec<String>> {
    tokens.chunks(size).map(|c| c.to_vec()).collect()
}

fn corpus(segs: Vec<Vec<String>>) -> Corpus {
    Corpus::from_terms("smysl/an/1:surface", Stream::Surface, Level::Part, segs)
}

fn base(boot_b: u32, seed: u64) -> Recipe {
    let mut r = Recipe::new("", 0, SCOPE);
    r.boot_b = boot_b;
    r.seed = seed;
    r
}

fn one(id: &str, recipe: &Recipe, c: &Corpus) -> MeasureResult {
    let store = Store::new();
    let scope = Resolved::of_uids(SCOPE, BTreeSet::new());
    let input = smysl_measure::Input::new(&store, &scope).with_text(c);
    run(&[id], recipe, &input).remove(0)
}

fn value(r: &MeasureResult) -> &Value {
    match &r.outcome {
        Outcome::Value(v) => v,
        other => panic!("{} did not run: {other:?}", r.recipe.id),
    }
}

fn fixed(v: &Value, k: &str) -> f64 {
    match v.values.get(k) {
        Some(Num::Fixed(i)) => from_q20(*i),
        other => panic!("`{k}` is {other:?}"),
    }
}

fn ci(v: &Value, k: &str) -> (f64, f64) {
    match v.ci.get(k) {
        Some(Ci {
            lo: Num::Fixed(lo),
            hi: Num::Fixed(hi),
            ..
        }) => (from_q20(*lo), from_q20(*hi)),
        other => panic!("`{k}` has no interval: {other:?}"),
    }
}

/// §5.1, T1 i.i.d.: uniform over k ∈ {2, 16, 256}, n = 10⁶. Miller–Madow within 10⁻³ of
/// log₂ k, and the plug-in below it by the correction, which is (k − 1)/(2n ln 2) exactly
/// when every type is seen.
#[test]
fn t1_on_a_uniform_source_is_log2_k() {
    const N: usize = 1_000_000;
    for k in [2usize, 16, 256] {
        let mut rng = Rng::new(&[1; 32], k as u64);
        let tokens: Vec<String> = (0..N).map(|_| format!("w{}", rng.index(k))).collect();
        let c = corpus(segments(tokens, 1000));
        let r = one("T1", &base(0, 0), &c);
        let v = value(&r);
        let (h, mm) = (fixed(v, "H"), fixed(v, "H:mm"));
        let want = log2(k as f64);
        assert!(
            (mm - want).abs() < 1e-3,
            "k = {k}: Ĥ_MM = {mm}, log₂k = {want}"
        );
        assert!(h < want, "k = {k}: the plug-in is low");
        let correction = (k as f64 - 1.0) / (2.0 * N as f64 * ln(2.0));
        // Two quanta: each of the two values was rounded once.
        assert!(
            ((mm - h) - correction).abs() <= 2.0 / 1_048_576.0,
            "k = {k}: Ĥ_MM − Ĥ = {}, (k−1)/(2n ln 2) = {correction}",
            mm - h
        );
        assert_eq!(v.n["tokens"], N as u64);
        assert_eq!(v.n["types"], k as u64);
        assert!(v.ci.is_empty(), "boot:b = 0 draws no interval");
    }
}

/// §5.1, T1 degenerate: one repeated term. Ĥ = 0 exactly, and Miller–Madow 0, since V̂ − 1 = 0.
#[test]
fn t1_on_one_repeated_term_is_zero_exactly() {
    let c = corpus(segments(vec!["amen".to_string(); 20_000], 1000));
    let r = one("T1", &base(50, 0), &c);
    let v = value(&r);
    assert_eq!(v.values["H"], Num::Fixed(0));
    assert_eq!(v.values["H:mm"], Num::Fixed(0));
    assert_eq!(v.ci["H"].lo, Num::Fixed(0));
    assert_eq!(v.ci["H"].hi, Num::Fixed(0));
}

/// A text whose type frequencies are drawn exactly from a discrete power law with exponent
/// `alpha` and `f_min = 1`, by inversion of `P(X ≥ x) = ζ(α, x) / ζ(α, 1)`, its tokens
/// shuffled and cut into segments of 500.
fn power_law_text(alpha: f64, types: usize, seed: u64) -> Corpus {
    use smysl_measure::text::zipf_heaps::hurwitz_zeta;
    let mut rng = Rng::new(&[4; 32], seed);
    let z1 = hurwitz_zeta(alpha, 1.0);
    let survival = |x: u64| hurwitz_zeta(alpha, x as f64) / z1;
    let mut tokens = Vec::new();
    for t in 0..types {
        let u = uniform(&mut rng);
        // The largest x with P(X ≥ x) > u: double, then bisect.
        let mut hi = 2u64;
        while survival(hi) > u {
            hi *= 2;
        }
        let mut lo = hi / 2;
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if survival(mid) > u {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        for _ in 0..lo {
            tokens.push(format!("t{t}"));
        }
    }
    for i in (1..tokens.len()).rev() {
        tokens.swap(i, rng.index(i + 1));
    }
    corpus(segments(tokens, 500))
}

/// §5.1, T4 Zipf: α̂ within its bootstrap interval, and near α, for α = 2.0 and 2.5. The
/// coverage half of the row — the interval covers α in at least 90 of 100 seeds — is
/// `tests/coverage.rs`, `--ignored`.
#[test]
fn t4_recovers_the_exponent_of_a_power_law() {
    for alpha in [2.0, 2.5] {
        let c = power_law_text(alpha, 3000, 0);
        let mut recipe = base(100, 0);
        recipe.min_n = Some(1000);
        let r = one("T4", &recipe, &c);
        let v = value(&r);
        let a = fixed(v, "alpha");
        let (lo, hi) = ci(v, "alpha");
        assert!(
            lo <= a && a <= hi,
            "α = {alpha}: α̂ = {a} outside [{lo}, {hi}]"
        );
        assert!((a - alpha).abs() < 0.2, "α = {alpha}: α̂ = {a}");
        let s = fixed(v, "s");
        assert!((s - 1.0 / (a - 1.0)).abs() < 1e-5, "s = 1/(α − 1)");
        assert!(matches!(v.values["f:min"], Num::Count(f) if f >= 1));
        assert!(v.n["tail"] >= 50);
    }
}

/// §5.1, T4 Heaps: a text whose vocabulary is `⌊10·n^0.6⌋` after n tokens, by construction —
/// a new type whenever the formula says one is due, and an old one otherwise. β̂ = 0.6 ± 0.01.
#[test]
fn t4_recovers_a_heaps_exponent_built_in() {
    const N: usize = 1_000_000;
    let mut tokens = Vec::with_capacity(N);
    let mut v = 0usize;
    for i in 1..=N {
        let due = ((10.0 * powf(i as f64, 0.6)).floor() as usize).min(i);
        if due > v {
            tokens.push(format!("t{v}"));
            v += 1;
        } else {
            tokens.push(format!("t{}", i % v));
        }
    }
    let c = corpus(segments(tokens, 1000));
    let r = one("T4", &base(0, 0), &c);
    let v = value(&r);
    let beta = fixed(v, "heaps:beta");
    assert!((beta - 0.6).abs() < 0.01, "β̂ = {beta}");
    let k = fixed(v, "heaps:K");
    assert!((k - 10.0).abs() < 1.0, "K̂ = {k}");
}

/// A short i.i.d. text for the bootstrap rules, in 40 segments.
fn bootstrap_text() -> Corpus {
    let mut rng = Rng::new(&[9; 32], 0);
    let tokens: Vec<String> = (0..20_000).map(|_| format!("w{}", rng.index(64))).collect();
    corpus(segments(tokens, 500))
}

/// 20,000 tokens over 2,000 types drawn with weights `r^−1.1`, in 40 segments: a text with a
/// tail for T4's α to fit, where `bootstrap_text`'s 64 equal types put α at its bound.
fn zipf_text() -> Corpus {
    let mut rng = Rng::new(&[10; 32], 0);
    let w: Vec<f64> = (1..=2000)
        .map(|r| smysl_measure::num::powf(r as f64, -1.1))
        .collect();
    let total: f64 = w.iter().sum();
    let mut acc = 0.0;
    let cdf: Vec<f64> = w
        .iter()
        .map(|x| {
            acc += x / total;
            acc
        })
        .collect();
    let tokens: Vec<String> = (0..20_000)
        .map(|_| {
            let u = (rng.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
            format!("w{}", cdf.partition_point(|&c| c < u).min(1999))
        })
        .collect();
    corpus(segments(tokens, 500))
}

fn a_basis() -> Basis {
    let unit = UnitCoreBuilder::new(KernelType::Claim, "a saved scope", Status::Speculative)
        .build()
        .unwrap();
    Basis {
        uid: canonical_uid(&unit),
        unit,
        edges: Vec::new(),
    }
}

/// §5.3, bootstrap seeds, over the one interval that still bootstraps — T4's α, which resamples
/// types: the same seed gives the same interval bytes; seeds 0 and 1 give different replicates,
/// and intervals within twice each other's width; and `boot:b` is in the recipe, so changing it
/// changes the hash and the uid.
#[test]
fn bootstrap_seeds_follow_section_5_3() {
    let c = zipf_text();
    let t4 = |b: u32, seed: u64| one("T4", &base(b, seed), &c);
    let a = t4(200, 0);
    let again = t4(200, 0);
    let payload = |r: &MeasureResult| to_payload(&r.recipe, value(r)).unwrap();
    assert_eq!(payload(&a), payload(&again), "one seed, one payload");

    let b = t4(200, 1);
    let (alo, ahi) = ci(value(&a), "alpha");
    let (blo, bhi) = ci(value(&b), "alpha");
    assert_ne!((alo, ahi), (blo, bhi), "two seeds, two sets of replicates");
    let w = (ahi - alo).max(bhi - blo);
    assert!(w > 0.0);
    assert!(
        (alo - blo).abs() <= 2.0 * w && (ahi - bhi).abs() <= 2.0 * w,
        "[{alo}, {ahi}] and [{blo}, {bhi}]"
    );
    assert_eq!(
        value(&a).values,
        value(&b).values,
        "the seed moves the interval only"
    );

    let more = t4(201, 0);
    assert_ne!(a.recipe.hash().unwrap(), more.recipe.hash().unwrap());
    let basis = a_basis();
    assert_ne!(
        canonical_uid(&to_unit(&a, Some(&basis)).unwrap()),
        canonical_uid(&to_unit(&more, Some(&basis)).unwrap())
    );
    assert_eq!(value(&more).ci["alpha"].b, 201);
}

/// OQ-89: T1's interval and T4's β are a grouped jackknife, which draws nothing — so no seed and
/// no `b` moves them, `b` records the twenty groups, and `boot:b = 0` still asks for none.
#[test]
fn the_jackknife_intervals_take_nothing_from_the_seed() {
    let c = bootstrap_text();
    let a = value(&one("T1", &base(200, 0), &c)).clone();
    let b = value(&one("T1", &base(7, 1), &c)).clone();
    assert_eq!(a.ci, b.ci);
    assert_eq!(a.ci["H:mm"].b, smysl_measure::boot::GROUPS as u32);
    let (lo, hi) = ci(&a, "H:mm");
    let est = match a.values["H:mm"] {
        Num::Fixed(q) => from_q20(q),
        _ => unreachable!(),
    };
    assert!(lo <= est && est <= hi, "[{lo}, {hi}] around {est}");
    assert!(value(&one("T1", &base(0, 0), &c)).ci.is_empty());

    let z = zipf_text();
    let t4 = |b: u32, seed: u64| value(&one("T4", &base(b, seed), &z)).ci["heaps:beta"];
    assert_eq!(t4(200, 0), t4(9, 3));
}

/// What `run` writes into a text measure's recipe, and the refusals it makes before a
/// measure computes anything.
#[test]
fn a_text_recipe_states_its_defaults_and_refuses_what_it_cannot_count() {
    let c = bootstrap_text();
    let t1 = one("T1", &base(10, 0), &c);
    assert_eq!(t1.recipe.min_n, Some(10_000));
    assert_eq!(t1.recipe.analyzer.as_deref(), Some("smysl/an/1:surface"));
    assert_eq!(t1.recipe.scope_digest, c.digest);
    let t4 = one("T4", &base(10, 0), &c);
    assert_eq!(t4.recipe.params.get("tail:min"), Some(&Param::Int(50)));
    assert!(
        !t1.recipe.params.contains_key("tail:min"),
        "T4's own parameter"
    );

    // Below min:n: W485, not run.
    let short = corpus(segments(vec!["a".to_string(); 9_999], 100));
    let r = one("T1", &base(10, 0), &short);
    assert!(matches!(
        r.outcome,
        Outcome::NotRun {
            code: Some(Code::W485),
            ..
        }
    ));
    assert_eq!(r.diagnostics[0].code, Code::W485);
    assert!(
        to_unit(&r, Some(&a_basis())).is_err(),
        "not run is not a unit"
    );

    // T4 with too few types for a tail: W485 too.
    let r = one("T4", &base(10, 0), &short);
    assert!(matches!(
        r.outcome,
        Outcome::NotRun {
            code: Some(Code::W485),
            ..
        }
    ));

    // Another analyzer, another level: E489.
    let mut other = base(10, 0);
    other.analyzer = Some("smysl/an/1:stem".into());
    let r = one("T1", &other, &c);
    assert!(matches!(
        r.outcome,
        Outcome::NotRun {
            code: Some(Code::E489),
            ..
        }
    ));
    let mut other = base(10, 0);
    other.level = Level::Verse;
    let r = one("T1", &other, &c);
    assert!(matches!(
        r.outcome,
        Outcome::NotRun {
            code: Some(Code::E489),
            ..
        }
    ));

    // A lossy text: E480 unless admitted; admitted, W408 and the flag.
    let mut lossy = c.clone();
    lossy.lossy = true;
    let r = one("T1", &base(10, 0), &lossy);
    assert!(matches!(
        r.outcome,
        Outcome::NotRun {
            code: Some(Code::E480),
            ..
        }
    ));
    let mut admit = base(10, 0);
    admit.lossy = true;
    let r = one("T1", &admit, &lossy);
    assert!(value(&r).flags.contains(&text::LOSSY_ADMITTED.to_string()));
    assert_eq!(r.diagnostics[0].code, Code::W408);

    // The stem stream is flagged.
    let stems = Corpus::from_terms(
        "smysl/an/1:stem",
        Stream::Stem,
        Level::Part,
        c.segments
            .iter()
            .map(|s| s.iter().map(|&t| c.types[t as usize].clone()).collect())
            .collect(),
    );
    let r = one("T1", &base(10, 0), &stems);
    assert!(value(&r).flags.contains(&text::STEM_LEVEL.to_string()));

    // No text at all.
    let store = Store::new();
    let scope = Resolved::of_uids(SCOPE, BTreeSet::new());
    let input = smysl_measure::Input::new(&store, &scope);
    let r = run(&["T1"], &base(10, 0), &input).remove(0);
    assert!(matches!(
        r.outcome,
        Outcome::NotRun {
            code: Some(Code::E489),
            ..
        }
    ));
}

struct Built {
    root: PathBuf,
    library: Library,
}

impl Drop for Built {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The KJV's Ruth and Jude, and the Synodal's, as `kjv/08-rut` and the like.
fn library() -> Built {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "smysl-measure-text-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut library = Library::create(&root).unwrap();
    for (alias, lang, dir) in [("kjv", "en", "eng-kjv"), ("synodal", "ru", "russyn")] {
        for book in ["08-RUT.usfm", "65-JUD.usfm"] {
            let path = repo("fixtures/library/bibles/slice").join(dir).join(book);
            let raw = std::fs::read(&path).unwrap();
            let alias = format!("{alias}/{}", book.trim_end_matches(".usfm").to_lowercase());
            let mut spec = AddSpec::new("usfm/1", &alias, "public-domain")
                .with_lang(LangTag::new(lang).unwrap())
                .with_carry(Carry::Text);
            spec.versification = Some("kjv".to_string());
            library
                .add(&ReadInput::new(&raw), &spec, &Caps::DEFAULT)
                .unwrap_or_else(|e| panic!("{alias}: {e}"));
        }
    }
    Built { root, library }
}

fn blessing() -> bool {
    std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1")
}

/// T1 and T4 over the KJV's Ruth and Jude at verse level: the scope resolved to texts, the
/// basis saved, both units written, and the results pinned for the ports. The two books are
/// 3,191 tokens between them, below either measure's `min:n`, so the default recipe refuses
/// them (`SMY-W485`) — the real text is short — and a recipe with `min:n` 1000 runs them.
#[test]
fn t1_and_t4_over_a_library() {
    let built = library();
    let lib = &built.library;
    let scope_text = "from collection:kjv";
    let texts = text::texts(scope_text, lib).unwrap();
    assert_eq!(
        texts.iter().map(|t| t.alias.as_str()).collect::<Vec<_>>(),
        ["kjv/08-rut", "kjv/65-jud"]
    );
    // The plain chain, named: the golden is then the same in a build with `stem` and one
    // without, where the language's own chain would be `smysl/an-en/1` in one and
    // `smysl/an/1` in the other.
    const PLAIN: &str = "smysl/an/1:surface";
    let c = Corpus::load(lib, &texts, Level::Verse, Some(PLAIN)).unwrap();
    assert_eq!(c.texts, ["kjv/08-rut", "kjv/65-jud"]);
    assert_eq!(c.segments.len(), 85 + 25, "Ruth's 85 verses and Jude's 25");

    let store = lib.store();
    let ls = LibraryScope::new("lib", lib);
    let env = Env::new(store, &ls);
    let resolved = resolve(scope_text, &env, &PlanOptions::default()).unwrap();
    let input = smysl_measure::Input::new(store, &resolved).with_text(&c);

    let mut recipe = Recipe::new("", 0, scope_text);
    recipe.level = Level::Verse;
    recipe.analyzer = Some(PLAIN.to_string());
    recipe.boot_b = 200;
    for r in run(&["T1", "T4"], &recipe, &input) {
        assert!(
            matches!(
                r.outcome,
                Outcome::NotRun {
                    code: Some(Code::W485),
                    ..
                }
            ),
            "{}: {:?}",
            r.recipe.id,
            r.outcome
        );
    }

    recipe.min_n = Some(1000);
    let mut got =
        String::from("# TX-P8 M2: T1 and T4 over the KJV's Ruth and Jude at verse level\n");
    got.push_str("# measure\tkey\tvalue\n");
    for r in run(&["T1", "T4"], &recipe, &input) {
        let v = value(&r);
        assert_eq!(
            r.recipe.scope_digest, c.digest,
            "a text measure's digest is over tids"
        );
        let unit = to_unit(&r, resolved.basis.as_ref()).expect("a unit");
        assert_eq!(
            unit.grounds.iter().copied().collect::<Vec<_>>(),
            [resolved.basis().unwrap()]
        );
        let id = &r.recipe.id;
        got.push_str(&format!(
            "{id}\trecipe.hash\t{}\n",
            hex(&r.recipe.hash().unwrap())
        ));
        for (k, n) in &v.n {
            got.push_str(&format!("{id}\tn.{k}\t{n}\n"));
        }
        for (k, n) in &v.values {
            got.push_str(&format!("{id}\tvalue.{k}\t{n}\n"));
        }
        for (k, c) in &v.ci {
            got.push_str(&format!("{id}\tci.{k}\t{} {} {}\n", c.lo, c.hi, c.b));
        }
        got.push_str(&format!("{id}\tflags\t{}\n", v.flags.join(" ")));
        got.push_str(&format!("{id}\tgist\t{}\n", gist(&r.recipe, v)));
        got.push_str(&format!(
            "{id}\tunit.uid\t{}\n",
            canonical_uid(&unit).canonical()
        ));
    }
    let path = repo("fixtures/measure/text.tsv");
    if blessing() {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; SMYSL_BLESS=1 writes it", path.display()));
    assert_eq!(want, got, "{} changed", path.display());
}

/// What a text scope accepts: the `from` and nothing after it, and an alias that exists.
#[test]
fn a_text_scope_is_a_from_and_nothing_else() {
    let built = library();
    let lib = &built.library;
    let code = |q: &str| text::texts(q, lib).unwrap_err().code;
    assert_eq!(code("from collection:kjv\nwhere kind = claim"), Code::E461);
    assert_eq!(code("from collection:kjv\nexpand ->grounds"), Code::E461);
    assert_eq!(code("find \"boaz\""), Code::E461);
    assert_eq!(code("from collection:vulgate"), Code::E511);
    assert_eq!(code("from channels"), Code::E461);
    assert_eq!(text::texts("from texts", lib).unwrap().len(), 4);
    assert_eq!(
        text::texts("from collection:\"kjv/08-rut\"", lib)
            .unwrap()
            .len(),
        1
    );
}

/// Two languages under two chains are two vocabularies: refused without an analyzer, read
/// under the one named with one.
#[test]
#[cfg(feature = "stem")]
fn a_scope_of_two_chains_needs_an_analyzer_named() {
    let built = library();
    let lib = &built.library;
    let texts = text::texts("from texts", lib).unwrap();
    match Corpus::load(lib, &texts, Level::Part, None) {
        Err(text::TextError::Refused(d)) => assert_eq!(d.code, Code::E489),
        other => panic!("{other:?}"),
    }
    let c = Corpus::load(lib, &texts, Level::Part, Some("smysl/an/1:surface")).unwrap();
    assert_eq!(c.analyzer, "smysl/an/1:surface");
    let ru = text::texts("from collection:synodal", lib).unwrap();
    let c = Corpus::load(lib, &ru, Level::Part, Some("smysl/an-ru/1:stem")).unwrap();
    assert_eq!(c.stream, Stream::Stem);
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
