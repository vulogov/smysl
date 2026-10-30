//! TX-P8 M5's exit (SMYSL-2.6 §6): T5's probabilities on a corpus worked by hand, leave-one-
//! segment-out against retraining from scratch, and T5 over a library built from the committed
//! Bible slices, pinned in `fixtures/measure/kneser_ney.tsv` (`SMYSL_BLESS=1` rewrites it).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::canonical_uid;
use smysl_core::diag::Code;
use smysl_core::ids::LangTag;
use smysl_core::types::Carry;
use smysl_graph::Store;
use smysl_measure::num::{self, from_q20};
use smysl_measure::rng::Rng;
use smysl_measure::text::kneser_ney::{Model, BOS};
use smysl_measure::text::{self, Corpus, Stream};
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
    run(&["T5"], recipe, &input).remove(0)
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

fn refusal(r: &MeasureResult) -> Code {
    match &r.outcome {
        Outcome::NotRun { code: Some(c), .. } => *c,
        other => panic!("{} was not refused: {other:?}", r.recipe.id),
    }
}

fn terms(segments: &[&str]) -> Corpus {
    Corpus::from_terms(
        "smysl/an/1:surface",
        Stream::Surface,
        Level::Part,
        segments
            .iter()
            .map(|s| s.split(' ').map(String::from).collect())
            .collect(),
    )
}

const HAND: [&str; 3] = [
    "the cat sat on the mat and the cat ran",
    "a dog sat on a mat and the dog ran",
    "the cat and the dog sat on the big mat",
];

/// §5.1, T5: thirty tokens in three segments, order 3, worked by hand.
///
/// Each segment is `<s> <s>` and its ten words. Ten word types (`V` = 10). The trigram counts
/// have 18 ones and 6 twos — `<s> <s> the`, `<s> the cat`, `sat on the`, `mat and the`,
/// `dog sat on` and `and the dog` — so D₃ = 18 / (18 + 12) = 3/5. The bigram continuations `N₁₊(• v w)` count distinct left neighbours
/// of the 19 distinct bigrams, 14 of them once and 5 twice: D₂ = 14 / (14 + 10) = 7/12. The
/// unigram continuations `N₁₊(• w)` count distinct left neighbours of each word, summing to
/// 19, with three ones (`cat`, `on`, `big`) and five twos (`sat`, `and`, `ran`, `a`, `dog`):
/// D₁ = 3 / (3 + 10) = 3/13.
///
/// `P(sat | the cat)`:
/// - order 1: `sat` follows `cat` and `dog`, so `a` = 2 of A = 19, T = 10 types:
///   `(2 − 3/13)/19 + (3/13)(10/19)(1/11)` = `253/2717 + 30/2717` = **283/2717**;
/// - order 2, context `cat`: `cat` is followed by `sat`, `ran` and `and`, each after one
///   distinct word (`the`), so a = 1, A = 3, T = 3:
///   `(1 − 7/12)/3 + (7/12)(3/3)(283/2717)` = **4882/24453**;
/// - order 3, context `the cat`: `the cat sat`, `the cat ran`, `the cat and` once each, so
///   a = 1, A = 3, T = 3: `(1 − 3/5)/3 + (3/5)(4882/24453)` = **10316/40755**.
///
/// The others below are the same three steps. `zebra` is unknown, so every order gives it only
/// its back-off share, ending in the `1/(V + 1)` = 1/11 of order 0: `7/2717` after three
/// discounts. A context the model has never seen (`big red`) backs off whole to order 1.
#[test]
fn t5_probabilities_worked_by_hand() {
    let c = terms(&HAND);
    let id = |w: &str| {
        c.types
            .iter()
            .position(|t| t == w)
            .map_or(1_000, |i| i as u32)
    };
    let m = Model::train(3, c.segments.iter().map(Vec::as_slice));
    assert_eq!(m.vocabulary(), 10);
    assert_eq!(c.tokens(), 30);
    for (k, want) in [(1, 3.0 / 13.0), (2, 7.0 / 12.0), (3, 3.0 / 5.0)] {
        assert_eq!(m.discount(k), want, "D{k}");
    }
    let p = |a: &str, b: &str, w: &str| {
        let h = |x: &str| if x == "<s>" { BOS } else { id(x) };
        m.prob(&[h(a), h(b)], id(w))
    };
    for (a, b, w, num, den) in [
        ("the", "cat", "sat", 10316.0, 40755.0),
        ("on", "the", "mat", 79853.0, 271700.0),
        ("on", "the", "cat", 50109.0, 271700.0),
        ("<s>", "<s>", "the", 6375.0, 10868.0),
        ("the", "dog", "zebra", 7.0, 2717.0),
        ("big", "red", "mat", 426.0, 2717.0),
    ] {
        let got = p(a, b, w);
        assert!(
            (got - num / den).abs() < 1e-12,
            "P({w} | {a} {b}) = {got}, by hand {num}/{den}"
        );
    }
}

/// A small random corpus: 2 to 8 segments of 0 to 12 words over 1 to 6 types.
fn corpus(rng: &mut Rng) -> Vec<Vec<u32>> {
    let k = 1 + rng.index(6);
    let segments = 2 + rng.index(7);
    (0..segments)
        .map(|_| {
            let n = rng.index(13);
            (0..n).map(|_| rng.index(k) as u32).collect()
        })
        .collect()
}

/// §5.1, T5 leave-one-out: on 2,000 random corpora at orders 1 to 5, a model with one segment
/// removed is the model trained without it — every count, context and count of counts — and so
/// scores the segment to the same bit; putting the segment back gives the full model again.
#[test]
fn removing_a_segment_is_retraining_without_it() {
    let mut rng = Rng::new(&[5; 32], 0);
    for case in 0..2_000 {
        let segs = corpus(&mut rng);
        let order = 1 + rng.index(5);
        let full = Model::train(order, segs.iter().map(Vec::as_slice));
        for (i, s) in segs.iter().enumerate() {
            let mut m = full.clone();
            m.remove(s);
            let fresh = Model::train(
                order,
                segs.iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, s)| s.as_slice()),
            );
            assert_eq!(
                m, fresh,
                "case {case}, order {order}, segment {i}: {segs:?}"
            );
            assert_eq!(
                m.surprisal(s).map(f64::to_bits),
                fresh.surprisal(s).map(f64::to_bits),
                "case {case}"
            );
            m.add(s);
            assert_eq!(m, full, "case {case}: restoring segment {i}");
        }
    }
}

/// The measure: H is every segment's leave-one-out bits over every token; the top-k are by
/// locator and in the series in rank order; the defaults are in the recipe.
#[test]
fn t5_scores_each_segment_by_the_others() {
    let c = terms(&HAND);
    let mut recipe = Recipe::new("", 0, SCOPE);
    recipe.min_n = Some(2);
    recipe.params.insert("top:k".into(), Param::Int(2));
    let r = one(&recipe, &c);
    assert_eq!(r.recipe.params.get("order"), Some(&Param::Int(3)));
    let v = value(&r);

    let full = Model::train(3, c.segments.iter().map(Vec::as_slice));
    let mut bits = Vec::new();
    for (i, s) in c.segments.iter().enumerate() {
        let others = Model::train(
            3,
            c.segments
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(_, s)| s.as_slice()),
        );
        bits.push(others.surprisal(s).unwrap());
    }
    let h = num::sum(bits.iter().copied()) / 30.0;
    assert_eq!(fixed(v, "H"), from_q20(num::q20(h).unwrap()));
    assert_eq!(
        fixed(v, "D:3"),
        from_q20(num::q20(full.discount(3)).unwrap())
    );
    assert_eq!(v.n["tokens"], 30);
    assert_eq!(v.n["segments"], 3);

    let mut means: Vec<(usize, i64)> = bits
        .iter()
        .enumerate()
        .map(|(i, b)| (i, num::q20(b / 10.0).unwrap()))
        .collect();
    means.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let series = &v.series["top"];
    assert_eq!(series.len(), 6, "two segments, three numbers each");
    for (rank, &(i, m)) in means.iter().take(2).enumerate() {
        assert_eq!(series[3 * rank], Num::Count(i as u64));
        assert_eq!(series[3 * rank + 2], Num::Fixed(m));
        let at = format!("#{}", i + 1);
        assert_eq!(v.values[&format!("top:{at}")], Num::Fixed(m));
        assert_eq!(v.n[&format!("top:{at}")], 10);
    }
    let third = format!("top:#{}", means[2].0 + 1);
    assert!(!v.values.contains_key(&third), "only the top two are kept");
}

#[test]
fn a_t5_recipe_is_checked_before_it_runs() {
    let c = terms(&HAND);
    let r = one(&Recipe::new("", 0, SCOPE), &c);
    assert_eq!(r.recipe.min_n, Some(10_000));
    assert_eq!(
        refusal(&r),
        Code::W485,
        "thirty tokens is short of the default"
    );

    for (k, v) in [("order", 0), ("order", 6), ("top:k", -1)] {
        let mut bad = Recipe::new("", 0, SCOPE);
        bad.min_n = Some(2);
        bad.params.insert(k.into(), Param::Int(v));
        assert_eq!(refusal(&one(&bad, &c)), Code::E489, "{k} = {v}");
    }

    let mut lone = Recipe::new("", 0, SCOPE);
    lone.min_n = Some(2);
    assert_eq!(
        refusal(&one(&lone, &terms(&HAND[..1]))),
        Code::W485,
        "one segment has no others to be scored by"
    );
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

/// The KJV's Ruth and Jude as `kjv/08-rut` and `kjv/65-jud`.
fn library() -> Built {
    let root = std::env::temp_dir().join(format!("smysl-measure-kn-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut library = Library::create(&root).unwrap();
    for book in ["08-RUT.usfm", "65-JUD.usfm"] {
        let raw = std::fs::read(repo("fixtures/library/bibles/slice/eng-kjv").join(book)).unwrap();
        let alias = format!("kjv/{}", book.trim_end_matches(".usfm").to_lowercase());
        let mut spec = AddSpec::new("usfm/1", &alias, "public-domain")
            .with_lang(LangTag::new("en").unwrap())
            .with_carry(Carry::Text);
        spec.versification = Some("kjv".to_string());
        library
            .add(&ReadInput::new(&raw), &spec, &Caps::DEFAULT)
            .unwrap_or_else(|e| panic!("{alias}: {e}"));
    }
    Built { root, library }
}

fn blessing() -> bool {
    std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1")
}

/// T5 over the KJV's Ruth and Jude at verse level, each verse scored by the other 109: the
/// scope resolved to texts, the basis saved, the unit written and the result pinned. 3,191
/// tokens is under the default `min:n`, which refuses them; the pinned run sets it to 2.
#[test]
fn t5_over_a_library() {
    const PLAIN: &str = "smysl/an/1:surface";
    const SCOPE_TEXT: &str = "from collection:kjv";
    let built = library();
    let lib = &built.library;
    let store = lib.store();
    let ls = LibraryScope::new("lib", lib);
    let env = Env::new(store, &ls);
    let texts = text::texts(SCOPE_TEXT, lib).unwrap();
    let c = Corpus::load(lib, &texts, Level::Verse, Some(PLAIN)).unwrap();
    assert_eq!(c.locators.len(), c.segments.len());
    let resolved = resolve(SCOPE_TEXT, &env, &PlanOptions::default()).unwrap();
    let input = smysl_measure::Input::new(store, &resolved).with_text(&c);
    let mut recipe = Recipe::new("", 0, SCOPE_TEXT);
    recipe.analyzer = Some(PLAIN.to_string());
    recipe.level = Level::Verse;
    assert_eq!(
        refusal(&run(&["T5"], &recipe, &input).remove(0)),
        Code::W485
    );

    recipe.min_n = Some(2);
    let r = run(&["T5"], &recipe, &input).remove(0);
    let v = value(&r);
    let unit = to_unit(&r, resolved.basis.as_ref()).expect("a unit");
    let mut got = String::from(
        "# TX-P8 M5: T5 over the KJV's Ruth and Jude at verse level, order 3, min:n 2\n",
    );
    got.push_str("# key\tvalue\n");
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
    for (k, s) in &v.series {
        let s: Vec<String> = s.iter().map(Num::render).collect();
        got.push_str(&format!("series.{k}\t{}\n", s.join(" ")));
    }
    got.push_str(&format!("gist\t{}\n", gist(&r.recipe, v)));
    got.push_str(&format!("unit.uid\t{}\n", canonical_uid(&unit).canonical()));
    let path = repo("fixtures/measure/kneser_ney.tsv");
    if blessing() {
        std::fs::write(&path, &got).unwrap();
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("{} is missing; SMYSL_BLESS=1 writes it", path.display()));
    assert_eq!(want, got, "{} changed", path.display());
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
