//! TX-P8 M4's exit (SMYSL-2.6 §6): T3's Λ against its definition on 10⁴ random inputs, the
//! calibration goldens, and T3 over a library built from the committed Bible slices.
//!
//! The inputs are generated here from the crate's own generator (§5.1). The calibration table
//! is `fixtures/measure/t3-calibration.tsv`, written by `examples/t3_calibration`; the rows up
//! to 10⁵ symbols are recomputed here bit for bit, and the rest by `--ignored`, in release.

#[path = "support/calibration.rs"]
mod calibration;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use calibration::{iid, row};

use smysl_core::canonical_uid;
use smysl_core::diag::Code;
use smysl_core::ids::LangTag;
use smysl_core::types::Carry;
use smysl_graph::Store;
use smysl_measure::num::from_q20;
use smysl_measure::rng::Rng;
use smysl_measure::text::lz_match::{estimate, lcp, lpf, ranks, suffix_array};
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
    run(&["T3"], recipe, &input).remove(0)
}

fn value(r: &MeasureResult) -> &Value {
    match &r.outcome {
        Outcome::Value(v) => v,
        other => panic!("{} did not run: {other:?}", r.recipe.id),
    }
}

fn q(v: &Value, k: &str) -> i64 {
    match v.values.get(k) {
        Some(Num::Fixed(i)) => *i,
        other => panic!("`{k}` is {other:?}"),
    }
}

fn refusal(r: &MeasureResult) -> Code {
    match &r.outcome {
        Outcome::NotRun { code: Some(c), .. } => *c,
        other => panic!("{} was not refused: {other:?}", r.recipe.id),
    }
}

/// `lpf[i]` by the definition: the longest common prefix of `s[i..]` with any `s[j..]`, `j <
/// i`, each common prefix by its own recurrence `lcp(i, j) = [sᵢ = sⱼ] · (1 + lcp(i+1, j+1))`.
/// O(n²), and nothing in it knows what a suffix array is.
fn naive_lpf(s: &[u32]) -> Vec<u32> {
    let n = s.len();
    let mut out = vec![0; n];
    let (mut next, mut row) = (vec![0u32; n + 1], vec![0u32; n + 1]);
    for i in (0..n).rev() {
        for j in 0..n {
            row[j] = if s[i] == s[j] { 1 + next[j + 1] } else { 0 };
        }
        out[i] = row[..i].iter().copied().max().unwrap_or(0);
        std::mem::swap(&mut next, &mut row);
    }
    out
}

/// One random input of at most 2,000 symbols: a length drawn log-uniformly, so that short
/// inputs, where the edge cases are, are most of the cases; and one of four shapes, since an
/// i.i.d. input over a large alphabet has almost no matches to get wrong.
fn input(rng: &mut Rng) -> Vec<u32> {
    let bits = 1 + rng.index(11) as u32;
    let n = 1 + rng.index((1usize << bits).min(2_000));
    let k = [1, 2, 3, 4, 16, 256][rng.index(6)];
    match rng.index(4) {
        // i.i.d.
        0 | 1 => (0..n).map(|_| rng.index(k) as u32).collect(),
        // A random block repeated, with the occasional symbol changed.
        2 => {
            let p = 1 + rng.index(n.min(40));
            let block: Vec<u32> = (0..p).map(|_| rng.index(k) as u32).collect();
            (0..n)
                .map(|i| {
                    if rng.index(50) == 0 {
                        rng.index(k) as u32
                    } else {
                        block[i % p]
                    }
                })
                .collect()
        }
        // A Fibonacci word, whose repeats overlap at every scale.
        _ => {
            let (mut a, mut b) = (vec![0u32], vec![0u32, 1]);
            while b.len() < n {
                let c = [b.clone(), a].concat();
                a = b;
                b = c;
            }
            b.truncate(n);
            b
        }
    }
}

/// §5.1, T3 reference: LPF-based Λᵢ equal to the naive definition, element by element, on 10⁴
/// random inputs of at most 2,000 symbols. The suffix array is checked against a comparison
/// sort and the LCP against a direct count where the input is short enough for both to be
/// quick.
#[test]
fn lpf_equals_its_definition_on_random_inputs() {
    let mut rng = Rng::new(&[3; 32], 0);
    for case in 0..10_000 {
        let s = input(&mut rng);
        let k = s.iter().max().map_or(1, |&m| m + 1);
        let got = lpf(&s, k);
        assert_eq!(got, naive_lpf(&s), "case {case}: {s:?}");
        if s.len() <= 300 {
            let sa = suffix_array(&s, k);
            let mut want: Vec<u32> = (0..s.len() as u32).collect();
            want.sort_by(|&a, &b| s[a as usize..].cmp(&s[b as usize..]));
            assert_eq!(sa, want, "case {case}: suffix array of {s:?}");
            let l = lcp(&s, &sa);
            for i in 1..s.len() {
                let (a, b) = (&s[sa[i - 1] as usize..], &s[sa[i] as usize..]);
                let direct = a.iter().zip(b).take_while(|(x, y)| x == y).count();
                assert_eq!(l[i] as usize, direct, "case {case}: lcp[{i}] of {s:?}");
            }
        }
    }
}

/// The edges: nothing, one symbol, a run of one symbol, which is all overlap.
#[test]
fn lpf_at_the_edges() {
    assert!(lpf(&[], 1).is_empty());
    assert_eq!(lpf(&[0], 1), [0]);
    assert_eq!(lpf(&[0; 6], 1), [0, 5, 4, 3, 2, 1]);
    assert_eq!(lpf(&[0, 1, 0, 1, 0], 2), [0, 0, 3, 2, 1]);
    assert_eq!(estimate(&[0]), None);
    // abab…: Λ₁ = 1 and Λᵢ = n − i + 1 after it.
    let (h, mr) = estimate(&lpf(&[0, 1, 0, 1, 0], 2)).unwrap();
    let lg = |x: f64| smysl_measure::num::log2(x);
    let want_h = 4.0 / (1.0 / lg(2.0) + 4.0 / lg(3.0) + 3.0 / lg(4.0) + 2.0 / lg(5.0));
    let want_mr = (lg(2.0) + lg(3.0) / 4.0 + lg(4.0) / 3.0 + lg(5.0) / 2.0) / 4.0;
    assert_eq!((h, mr), (want_h, want_mr));
}

fn t3_of(text: String, min_n: u64) -> MeasureResult {
    calibration::t3(text, min_n)
}

/// The calibration fixture's rows, as `examples/t3_calibration` writes them, with their n.
fn calibration_rows() -> Vec<(usize, String)> {
    let path = repo("fixtures/measure/t3-calibration.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "{} is missing; `cargo run --release --example t3_calibration` writes it",
            path.display()
        )
    });
    text.lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| {
            (
                l.split('\t').nth(1).unwrap().parse().unwrap(),
                l.to_string(),
            )
        })
        .collect()
}

fn check_rows(keep: impl Fn(usize) -> bool) {
    let rows = calibration_rows();
    assert_eq!(
        rows.len(),
        calibration::BITS.len() * calibration::LENGTHS.len()
    );
    for (n, want) in rows.into_iter().filter(|r| keep(r.0)) {
        let f: Vec<&str> = want.split('\t').collect();
        let got = row(f[0].parse().unwrap(), n, f[2].parse().unwrap());
        assert_eq!(got, want, "a calibration row changed");
    }
}

/// §5.1, T3 calibration — H = 1, 4, 6 at 2k, 8k and 32k are among them — and every row of the
/// fixture up to 10⁵ symbols, bit for bit: both values and the payload's hash.
#[test]
fn calibration_goldens() {
    check_rows(|n| n <= 100_000);
}

/// The fixture's rows past 10⁵ symbols. `cargo test --release -- --ignored`.
#[test]
#[ignore = "10⁶ and 10⁷ symbols; run in release"]
fn calibration_goldens_long() {
    check_rows(|n| n > 100_000);
}

/// §5.1 asks for §2.9's table to 3 decimals. §2.9's probe drew its sources from another
/// generator, so its numbers are one draw each and no other generator reproduces them to the
/// digit; what can be checked is that they are draws of the same estimator. Over 20 seeds of
/// this crate's sources, each of §2.9's values is within three of a single draw's standard
/// deviations of the mean, plus the half-unit its printed rounding hides.
#[test]
fn the_probe_table_is_a_draw_of_this_estimator() {
    // (bits, n, H at §2.9's printed places, H:mr where §2.9 gives it)
    let probe = [
        (1, 2_000, 0.869, 0.0005, None),
        (1, 8_000, 0.896, 0.0005, None),
        (1, 32_000, 0.910, 0.0005, Some(0.925)),
        (4, 2_000, 3.08, 0.005, None),
        (4, 8_000, 3.21, 0.005, None),
        (4, 32_000, 3.30, 0.005, Some(3.36)),
        (6, 2_000, 4.32, 0.005, None),
        (6, 8_000, 4.48, 0.005, None),
        (6, 32_000, 4.60, 0.005, Some(4.68)),
    ];
    for (bits, n, h, half, mr) in probe {
        let (mut hs, mut mrs) = (Vec::new(), Vec::new());
        for seed in 0..20 {
            let r = t3_of(iid(bits, n, seed), 2);
            let v = value(&r);
            hs.push(from_q20(q(v, "H")));
            mrs.push(from_q20(q(v, "H:mr")));
        }
        let stats = |xs: &[f64]| {
            let m = xs.iter().sum::<f64>() / xs.len() as f64;
            let var = xs.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / (xs.len() - 1) as f64;
            (m, var.sqrt())
        };
        let (m, sd) = stats(&hs);
        assert!(
            (m - h).abs() <= 3.0 * sd + half,
            "H = {bits}, n = {n}: mean {m:.4}, sd {sd:.4}, §2.9 {h}"
        );
        if let Some(want) = mr {
            let (m, sd) = stats(&mrs);
            assert!(
                (m - want).abs() <= 3.0 * sd + 0.0005,
                "H:mr, H = {bits}, n = {n}: mean {m:.4}, sd {sd:.4}, §2.9 {want}"
            );
        }
    }
}

/// A scope of two texts is two `x`s: no match crosses between them, the scope's value is the
/// harmonic aggregate of the texts', and each text's values are stored beside it.
#[test]
fn texts_are_measured_apart_and_aggregated() {
    let (a, b) = (iid(2, 3_000, 1), iid(4, 5_000, 2));
    let single = |s: &str| {
        let v = value(&t3_of(s.to_string(), 2)).clone();
        (from_q20(q(&v, "H")), from_q20(q(&v, "H:mr")))
    };
    let (ha, ma) = single(&a);
    let (hb, mb) = single(&b);
    let c = Corpus::from_docs(vec![
        Doc::new("a", vec![a.clone()]),
        Doc::new("b", vec![b.clone()]),
    ]);
    let mut recipe = Recipe::new("", 0, SCOPE);
    recipe.min_n = Some(2);
    let r = one(&recipe, &c);
    let v = value(&r);
    assert_eq!(v.n["symbols"], 8_000);
    assert_eq!(v.n["symbols:a"], 3_000);
    assert_eq!(from_q20(q(v, "H:a")), ha);
    assert_eq!(from_q20(q(v, "H:mr:b")), mb);
    let agg = from_q20(q(v, "H"));
    let want = 8_000.0 / (3_000.0 / ha + 5_000.0 / hb);
    assert!((agg - want).abs() < 2e-6, "{agg} against {want}");
    let mr = from_q20(q(v, "H:mr"));
    let want = (3_000.0 * ma + 5_000.0 * mb) / 8_000.0;
    assert!((mr - want).abs() < 2e-6, "{mr} against {want}");
    // The same two texts as one would let b's matches reach into a.
    let joined = value(&t3_of(format!("{a}\n{b}"), 2)).clone();
    assert_ne!(q(&joined, "H"), q(v, "H"));
}

/// The defaults are in the recipe; `min:n` holds for each text and names the shortest; the
/// symbols are chars or terms, and terms are the analyzer stream's.
#[test]
fn a_t3_recipe_states_its_symbols_and_its_minimum() {
    let c = Corpus::from_docs(vec![
        Doc::new("long", vec![iid(1, 60_000, 0)]),
        Doc::new("short", vec![iid(1, 49_999, 0)]),
    ]);
    let r = one(&Recipe::new("", 0, SCOPE), &c);
    assert_eq!(r.recipe.min_n, Some(50_000));
    assert_eq!(
        r.recipe.params.get("symbols"),
        Some(&Param::Text("chars".into()))
    );
    assert_eq!(refusal(&r), Code::W485);
    match &r.outcome {
        Outcome::NotRun { why, .. } => assert!(why.contains("`short` has 49999"), "{why}"),
        _ => unreachable!(),
    }

    let mut bad = Recipe::new("", 0, SCOPE);
    bad.params
        .insert("symbols".into(), Param::Text("bytes".into()));
    assert_eq!(refusal(&one(&bad, &c)), Code::E489);

    // Terms: "a b a b a" is the symbols 0 1 0 1 0, whatever the words' characters are.
    let words = Corpus::from_terms(
        "smysl/an/1:surface",
        Stream::Surface,
        Level::Part,
        vec![
            vec!["ab".into(), "c".into()],
            vec!["ab".into(), "c".into(), "ab".into()],
        ],
    );
    let mut terms = Recipe::new("", 0, SCOPE);
    terms.min_n = Some(2);
    terms
        .params
        .insert("symbols".into(), Param::Text("terms".into()));
    let r = one(&terms, &words);
    let v = value(&r);
    assert_eq!(v.n["symbols"], 5);
    let (h, mr) = estimate(&lpf(&[0, 1, 0, 1, 0], 2)).unwrap();
    assert_eq!(
        (q(v, "H"), q(v, "H:mr")),
        (
            smysl_measure::num::q20(h).unwrap(),
            smysl_measure::num::q20(mr).unwrap()
        )
    );
    assert_eq!(
        v.headline.as_ref().map(|h| h.unit.as_str()),
        Some("bits/token")
    );
}

/// The ranks keep the order and only the order.
#[test]
fn ranks_are_the_alphabet_in_order() {
    assert_eq!(ranks(&[900, 5, 900, 70]), (vec![2, 0, 2, 1], 3));
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
    let root = std::env::temp_dir().join(format!("smysl-measure-lz-{}", std::process::id()));
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

/// T3 over the KJV's Ruth and Jude, characters and terms, at part level: the scope resolved
/// to texts, the basis saved, the units written and the results pinned for the ports. The two
/// books are 14k and 4k characters, under the default `min:n`, which refuses them; the pinned
/// runs set it to 2 and say so in their recipe.
#[test]
fn t3_over_a_library() {
    const PLAIN: &str = "smysl/an/1:surface";
    const SCOPE_TEXT: &str = "from collection:kjv";
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

    let r = run(&["T3"], &recipe, &input).remove(0);
    assert_eq!(
        refusal(&r),
        Code::W485,
        "the default min:n refuses two short books"
    );

    let mut got = String::from(
        "# TX-P8 M4: T3 over the KJV's Ruth and Jude at part level, characters and terms,\n\
         # min:n 2\n",
    );
    got.push_str("# symbols\tkey\tvalue\n");
    recipe.min_n = Some(2);
    for which in ["chars", "terms"] {
        let mut recipe = recipe.clone();
        recipe
            .params
            .insert("symbols".into(), Param::Text(which.into()));
        let r = run(&["T3"], &recipe, &input).remove(0);
        let v = value(&r);
        assert_eq!(v.n["texts"], 2);
        let unit = to_unit(&r, resolved.basis.as_ref()).expect("a unit");
        got.push_str(&format!(
            "{which}\trecipe.hash\t{}\n",
            hex(&r.recipe.hash().unwrap())
        ));
        for (k, n) in &v.n {
            got.push_str(&format!("{which}\tn.{k}\t{n}\n"));
        }
        for (k, n) in &v.values {
            got.push_str(&format!("{which}\tvalue.{k}\t{n}\n"));
        }
        got.push_str(&format!("{which}\tgist\t{}\n", gist(&r.recipe, v)));
        got.push_str(&format!(
            "{which}\tunit.uid\t{}\n",
            canonical_uid(&unit).canonical()
        ));
    }
    let path = repo("fixtures/measure/lz_match.tsv");
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
