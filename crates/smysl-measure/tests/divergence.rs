//! TX-P8 M6's exit (SMYSL-2.6 §6): §5.1's T7 rows — JSD 0 and 1 exactly at its two ends, and
//! G² on 2×2 tables against an independent computation — and T7 over a library built from the
//! committed Bible slices.
//!
//! The G² tables and this crate's values for them are `fixtures/measure/g2.tsv`; the Python
//! implementation's `tests/test_measure_g2.py` recomputes each from the table alone, with its
//! own logarithm. The library result is `fixtures/measure/divergence.tsv`. `SMYSL_BLESS=1`
//! rewrites both.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::canonical_uid;
use smysl_core::diag::Code;
use smysl_core::ids::LangTag;
use smysl_core::types::Carry;
use smysl_graph::Store;
use smysl_measure::num::{self, from_q20};
use smysl_measure::text::divergence::{g2, jsd, Pairs};
use smysl_measure::text::{self, Corpus, Doc, Stream};
use smysl_measure::{
    gist, resolve, run, to_unit, Level, MeasureResult, Num, Outcome, Param, Recipe, Resolved, Value,
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

fn one(recipe: &Recipe, c: &Corpus) -> MeasureResult {
    let store = Store::new();
    let scope = Resolved::of_uids(SCOPE, BTreeSet::new());
    let input = smysl_measure::Input::new(&store, &scope).with_text(c);
    run(&["T7"], recipe, &input).remove(0)
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
        other => panic!("{} was not refused: {other:?}", r.recipe.id),
    }
}

fn low() -> Recipe {
    let mut r = Recipe::new("", 0, SCOPE);
    r.min_n = Some(2);
    r
}

/// §5.1, T7: P = Q gives JSD = 0 exactly, and disjoint supports give 1 exactly — as values,
/// not within a tolerance, because the form computed makes both ends exact (`text::divergence`).
/// A text twice the length of another with the same proportions is the same distribution.
#[test]
fn jsd_is_exact_at_both_ends() {
    assert_eq!(jsd(&[3, 1, 4, 0], &[3, 1, 4, 0]), Some(0.0));
    assert_eq!(jsd(&[3, 1, 4, 0], &[6, 2, 8, 0]), Some(0.0));
    assert_eq!(jsd(&[3, 1, 0, 0], &[0, 0, 4, 7]), Some(1.0));
    assert_eq!(jsd(&[0, 0], &[1, 1]), None);
    // Between the ends, the textbook form, H(M) − ½H(P) − ½H(Q), to rounding.
    let (p, q) = ([5u64, 3, 2, 0], [1u64, 3, 4, 2]);
    let h = |c: &[f64]| -> f64 {
        c.iter()
            .filter(|&&x| x > 0.0)
            .map(|&x| -x * num::log2(x))
            .sum()
    };
    let pf: Vec<f64> = p.iter().map(|&c| c as f64 / 10.0).collect();
    let qf: Vec<f64> = q.iter().map(|&c| c as f64 / 10.0).collect();
    let m: Vec<f64> = pf.iter().zip(&qf).map(|(a, b)| (a + b) / 2.0).collect();
    let want = h(&m) - h(&pf) / 2.0 - h(&qf) / 2.0;
    assert!((jsd(&p, &q).unwrap() - want).abs() < 1e-15);

    let same = Corpus::from_docs(vec![
        Doc::new("a", vec!["x y z x".into()]),
        Doc::new("b", vec!["z x y x".into(), "x z y x".into()]),
    ]);
    let v = value(&one(&low(), &same)).clone();
    assert_eq!(v.values["jsd:a|b"], Num::Fixed(0));
    assert_eq!(v.values["jsd:sqrt:a|b"], Num::Fixed(0));
    let apart = Corpus::from_docs(vec![
        Doc::new("a", vec!["x y".into()]),
        Doc::new("b", vec!["z w w".into()]),
    ]);
    let v = value(&one(&low(), &apart)).clone();
    assert_eq!(v.values["jsd:a|b"], Num::Fixed(1 << 20));
    assert_eq!(v.values["jsd:sqrt:a|b"], Num::Fixed(1 << 20));
}

/// The tables `fixtures/measure/g2.tsv` holds: a perfect association, independence (G² = 0
/// exactly, every `O/E` being 1), a zero cell, a pair seen less than chance, Dunning's own
/// shape — a rare pair in a large table — and a large one.
const TABLES: [(u64, u64, u64, u64); 8] = [
    (10, 0, 0, 10),
    (2, 4, 3, 6),
    (5, 0, 3, 12),
    (1, 9, 9, 1),
    (3, 7, 12, 31_415),
    (1, 0, 0, 999_999),
    (110, 2_552, 221, 10_000_000),
    (7, 3, 2, 1),
];

/// §5.1, T7 G²: by hand, then the fixture the Python harness recomputes.
///
/// `(10, 0, 0, 10)`: N = 20, every margin 10, so E₁₁ = E₂₂ = 5 and the two zero cells add 0:
/// G² = 2 (10 ln 2 + 10 ln 2) = 40 ln 2. `(2, 4, 3, 6)` is independence, `2·6 = 4·3`, and every
/// `O·N/(R·C)` is exactly 1.
#[test]
fn g2_by_hand_and_for_the_python_harness() {
    let by_hand = 40.0 * num::ln(2.0);
    assert!((g2(10, 0, 0, 10).unwrap() - by_hand).abs() < 1e-12);
    assert_eq!(g2(2, 4, 3, 6), Some(0.0));
    assert_eq!(g2(0, 0, 0, 0), None);
    // Attraction and repulsion alike: the same four cells, summed in another order.
    let (attract, repel) = (g2(9, 1, 1, 9).unwrap(), g2(1, 9, 9, 1).unwrap());
    assert!((attract - repel).abs() < 1e-12 * attract);

    let mut got = String::from(
        "# TX-P8 M6: Dunning's G² of 2x2 tables, for the Python harness to recompute\n\
         # o11\to12\to21\to22\tg2 (f64, shortest round-trip)\tq20(g2)\n",
    );
    for (a, b, c, d) in TABLES {
        let g = g2(a, b, c, d).unwrap();
        got.push_str(&format!(
            "{a}\t{b}\t{c}\t{d}\t{g:?}\t{}\n",
            num::q20(g).unwrap()
        ));
    }
    bless_or_compare("fixtures/measure/g2.tsv", &got);
}

/// Pairs are adjacent within a segment and never across two.
#[test]
fn pairs_stay_inside_a_segment() {
    let c = Corpus::from_terms(
        "smysl/an/1:surface",
        Stream::Surface,
        Level::Part,
        vec![
            vec!["a".into(), "b".into()],
            vec!["c".into(), "d".into(), "a".into(), "b".into()],
        ],
    );
    let p = Pairs::of(&c);
    let id = |w: &str| c.types.iter().position(|t| t == w).unwrap() as u32;
    assert_eq!(p.n, 4);
    assert_eq!(p.counts.get(&(id("b"), id("c"))), None);
    assert_eq!(p.counts[&(id("a"), id("b"))], 2);
    assert_eq!(p.table(id("a"), id("b")), (2, 0, 0, 2));
}

/// G² ranks attraction only; PMI needs `min:pair`; both are stored by their terms with the
/// pair's count; the defaults are in the recipe.
#[test]
fn collocations_are_ranked_and_stored_by_their_terms() {
    // "new york" six times; "york new" never; "the" everywhere.
    let mut segs = Vec::new();
    for _ in 0..6 {
        segs.push("the new york the city".to_string());
    }
    for _ in 0..4 {
        segs.push("the old city the new road".to_string());
    }
    let c = Corpus::from_docs(vec![Doc::new("t", segs)]);
    let r = one(&low(), &c);
    assert_eq!(r.recipe.params.get("min:pair"), Some(&Param::Int(5)));
    assert_eq!(r.recipe.params.get("top:k"), Some(&Param::Int(20)));
    let v = value(&r);
    assert_eq!(v.n["pair:new york"], 6);
    let p = Pairs::of(&c);
    let id = |w: &str| c.types.iter().position(|t| t == w).unwrap() as u32;
    let (a, b, cc, d) = p.table(id("new"), id("york"));
    assert_eq!(
        v.values["g2:new york"],
        Num::Fixed(num::q20(g2(a, b, cc, d).unwrap()).unwrap())
    );
    let pmi = num::log2(6.0 * p.n as f64 / (10.0 * 6.0));
    assert_eq!(v.values["pmi:new york"], Num::Fixed(num::q20(pmi).unwrap()));
    // "new road" is seen 4 times, under min:pair, so it has a G² and no PMI.
    assert!(v.values.contains_key("g2:new road"));
    assert!(!v.values.contains_key("pmi:new road"));
    // Every G² key is a pair seen more often than chance.
    for k in v.values.keys().filter(|k| k.starts_with("g2:")) {
        let (x, y) = k["g2:".len()..].split_once(' ').unwrap();
        let (o11, o12, o21, _) = p.table(id(x), id(y));
        assert!(
            o11 as u128 * p.n as u128 > ((o11 + o12) * (o11 + o21)) as u128,
            "{k}"
        );
    }
    // One text has no divergence to store.
    assert!(!v.values.keys().any(|k| k.starts_with("jsd:")));

    let mut top = low();
    top.params.insert("top:k".into(), Param::Int(1));
    let v = value(&one(&top, &c)).clone();
    assert_eq!(v.values.keys().filter(|k| k.starts_with("g2:")).count(), 1);
    assert_eq!(v.values.keys().filter(|k| k.starts_with("pmi:")).count(), 1);

    assert_eq!(refusal(&one(&Recipe::new("", 0, SCOPE), &c)), Code::W485);
    for (k, n) in [("min:pair", 0), ("top:k", -1)] {
        let mut bad = low();
        bad.params.insert(k.into(), Param::Int(n));
        assert_eq!(refusal(&one(&bad, &c)), Code::E489, "{k} = {n}");
    }
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

const BOOKS: [&str; 2] = ["08-RUT.usfm", "65-JUD.usfm"];
const BIBLES: [(&str, &str, &str); 2] = [("kjv", "en", "eng-kjv"), ("synodal", "ru", "russyn")];

fn library() -> Built {
    let root = std::env::temp_dir().join(format!("smysl-measure-t7-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut library = Library::create(&root).unwrap();
    for (alias, lang, dir) in BIBLES {
        for book in BOOKS {
            let raw =
                std::fs::read(repo("fixtures/library/bibles/slice").join(dir).join(book)).unwrap();
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

fn bless_or_compare(rel: &str, got: &str) {
    let path = repo(rel);
    if blessing() {
        std::fs::write(&path, got).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; SMYSL_BLESS=1 writes it", path.display()));
    assert_eq!(want, got, "{} changed", path.display());
}

/// T7 over the four slice books at part level, under the plain chain so that the English and
/// the Russian are counted in one vocabulary: the two books of one Bible are close, the two
/// languages share almost nothing, and the collocations are pinned for the ports.
#[test]
fn t7_over_a_library() {
    const PLAIN: &str = "smysl/an/1:surface";
    const SCOPE_TEXT: &str = "from texts";
    let built = library();
    let lib = &built.library;
    let store = lib.store();
    let ls = LibraryScope::new("lib", lib);
    let env = Env::new(store, &ls);
    let texts = text::texts(SCOPE_TEXT, lib).unwrap();
    let c = Corpus::load(lib, &texts, Level::Part, Some(PLAIN)).unwrap();
    let resolved = resolve(SCOPE_TEXT, &env, &PlanOptions::default()).unwrap();
    let input = smysl_measure::Input::new(store, &resolved).with_text(&c);
    let mut recipe = Recipe::new("", 0, SCOPE_TEXT);
    recipe.analyzer = Some(PLAIN.to_string());
    recipe.min_n = Some(2);
    let r = run(&["T7"], &recipe, &input).remove(0);
    let v = value(&r);
    let d = |a: &str, b: &str| match v.values[&format!("jsd:{a}|{b}")] {
        Num::Fixed(i) => from_q20(i),
        _ => unreachable!(),
    };
    assert!(d("kjv/08-rut", "kjv/65-jud") < d("kjv/08-rut", "synodal/08-rut"));
    assert!(d("synodal/08-rut", "synodal/65-jud") < d("kjv/65-jud", "synodal/65-jud"));
    // Latin and Cyrillic words share no term, so every pair across the languages is the
    // disjoint end, 1 exactly, as the payload integer.
    for (a, b) in [
        ("kjv/08-rut", "synodal/08-rut"),
        ("kjv/08-rut", "synodal/65-jud"),
        ("kjv/65-jud", "synodal/08-rut"),
        ("kjv/65-jud", "synodal/65-jud"),
    ] {
        assert_eq!(
            v.values[&format!("jsd:{a}|{b}")],
            Num::Fixed(1 << 20),
            "{a} {b}"
        );
    }
    let unit = to_unit(&r, resolved.basis.as_ref()).expect("a unit");

    let mut got = String::from(
        "# TX-P8 M6: T7 over the four slice books at part level, plain chain, min:n 2\n\
         # key\tvalue\n",
    );
    got.push_str(&format!(
        "recipe.hash\t{}\n",
        hex(&r.recipe.hash().unwrap())
    ));
    for (k, n) in &v.n {
        got.push_str(&format!("n.{k}\t{n}\n"));
    }
    for (k, n) in &v.values {
        got.push_str(&format!("value.{k}\t{n}\n"));
    }
    got.push_str(&format!("gist\t{}\n", gist(&r.recipe, v)));
    got.push_str(&format!("unit.uid\t{}\n", canonical_uid(&unit).canonical()));
    bless_or_compare("fixtures/measure/divergence.tsv", &got);
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
