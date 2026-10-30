//! TX-P8 M3's exit (SMYSL-2.6 §6): T2 and T6 against §5.1's rows, the window guard, the codec
//! pinned, and both measures over a library built from the committed Bible slices.
//!
//! The synthetic texts are generated here from the crate's own generator (§5.1). The library
//! results and the codec's lengths for the slices are pinned in `fixtures/measure/compress.tsv`,
//! language-free, so a port — or a build of this crate whose brotli gained its `std` feature —
//! is checked against them; `SMYSL_BLESS=1` rewrites it.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::canonical_uid;
use smysl_core::diag::Code;
use smysl_core::ids::LangTag;
use smysl_core::types::Carry;
use smysl_graph::Store;
use smysl_measure::num::from_q20;
use smysl_measure::rng::Rng;
use smysl_measure::text::compress::{compress_into, compressed_len, pieces, Heap, CODEC, WINDOW};
use smysl_measure::text::ncd::{average_linkage, ncd, Merge};
use smysl_measure::text::{self, Corpus, Doc, Stream};
use smysl_measure::{
    gist, resolve, run, to_unit, Level, MeasureResult, Num, Outcome, Recipe, Resolved, Value,
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

fn refusal(r: &MeasureResult) -> Code {
    match &r.outcome {
        Outcome::NotRun { code: Some(c), .. } => *c,
        other => panic!("{} was not refused: {other:?}", r.recipe.id),
    }
}

/// `n` characters, each drawn from `alphabet` by `pick`.
fn text_of(n: usize, mut pick: impl FnMut() -> char) -> String {
    (0..n).map(|_| pick()).collect()
}

fn t2_rate(s: String) -> f64 {
    let c = Corpus::from_docs(vec![Doc::new("synthetic", vec![s])]);
    fixed(
        value(&one("T2", &Recipe::new("", 0, SCOPE), &c)),
        "bits/char",
    )
}

/// §5.1, T2 i.i.d.: uniform over k ∈ {2, 16, 256}, n = 10⁶, within 0.5% of log₂ k. The 256
/// symbols are U+0100 to U+01FF, two bytes each in UTF-8, so the row is over characters as T2
/// counts them and not over bytes.
#[test]
fn t2_on_a_uniform_source_is_log2_k() {
    for (k, first, want) in [(2u32, 'a', 1.0), (16, 'a', 4.0), (256, '\u{100}', 8.0)] {
        let mut rng = Rng::new(&[2; 32], u64::from(k));
        let s = text_of(1_000_000, || {
            char::from_u32(first as u32 + rng.index(k as usize) as u32).unwrap()
        });
        let got = t2_rate(s);
        assert!(
            (got - want).abs() < 0.005 * want,
            "k = {k}: {got} bits/char, log₂ k = {want}"
        );
    }
}

/// §5.1, T2 Markov: a first-order chain with a known rate, within 2%.
///
/// **A chain brotli's model can see.** Brotli codes a literal under a context, and for UTF-8
/// text the context is the *class* of the previous byte — lowercase vowel, lowercase consonant,
/// uppercase vowel, uppercase consonant, digit, `=`, `.`, `,` and so on — not the byte. A chain
/// over letters of one class is invisible to it: over sixteen letters, each moving to one of
/// eight with dyadic probabilities at 2.75 bits, T2 read 3.75. So the chain's eight states are eight characters in eight classes,
/// `a b A B 0 = . ,`, and each moves to one of the next eight with probabilities ¼ ¼ ⅛ ⅛ and
/// four of ¹⁄₁₆: a rate of 2.75 bits, and dyadic, because a prefix code reaches a rate only
/// when the probabilities are powers of ½. Measured: 2.7517. (A skewed row is another matter
/// even under distinct contexts: ½ ¼ ⅛ … at 1.98 bits read 2.31, LZ copies winning the parse
/// at rates that low. Both are in SMYSL-2.6 §3.7.)
#[test]
fn t2_on_a_chain_its_codec_can_model_is_its_rate() {
    const STATES: [char; 8] = ['a', 'b', 'A', 'B', '0', '=', '.', ','];
    // In sixteenths, so the draw is an integer and the rate is exact.
    const ROW: [usize; 8] = [4, 4, 2, 2, 1, 1, 1, 1];
    let mut rng = Rng::new(&[9; 32], 1);
    let mut cur = 0;
    let s = text_of(1_000_000, || {
        let mut u = rng.index(16);
        let mut step = 0;
        while u >= ROW[step] {
            u -= ROW[step];
            step += 1;
        }
        cur = (cur + step + 1) % STATES.len();
        STATES[cur]
    });
    let rate = 2.75;
    let got = t2_rate(s);
    assert!(
        (got - rate).abs() < 0.02 * rate,
        "{got} bits/char, the chain's rate is {rate}"
    );
}

/// What T2 counts is a brotli stream: decoded by the decompressor in the same crate, it is the
/// input, byte for byte.
#[test]
fn the_stream_t2_counts_decodes_to_its_input() {
    let input = std::fs::read(repo("fixtures/library/bibles/slice/russyn/08-RUT.usfm")).unwrap();
    let mut stream = Vec::new();
    compress_into(&input, &mut stream).unwrap();
    assert!(stream.len() < input.len() / 2);

    let mut state = brotli::BrotliState::new(Heap, Heap, Heap);
    let mut out = vec![0u8; input.len() + 1];
    let (mut avail_in, mut in_off) = (stream.len(), 0);
    let (mut avail_out, mut out_off, mut total) = (out.len(), 0, 0);
    let r = brotli::BrotliDecompressStream(
        &mut avail_in,
        &mut in_off,
        &stream,
        &mut avail_out,
        &mut out_off,
        &mut out,
        &mut total,
        &mut state,
    );
    assert!(matches!(r, brotli::BrotliResult::ResultSuccess), "{r:?}");
    assert_eq!(&out[..out_off], &input[..]);
}

/// A text longer than the window is read in pieces at segment boundaries; a segment longer
/// than the window is cut at a character boundary.
#[test]
fn a_text_past_the_window_is_read_in_pieces() {
    let short = vec!["one".to_string(), "two".to_string()];
    assert_eq!(pieces(&short), ["one\ntwo"]);

    let half = "x".repeat(WINDOW / 2 + 1);
    let three = vec![half.clone(), half.clone(), "tail".to_string()];
    let p = pieces(&three);
    assert_eq!(p.len(), 2);
    assert_eq!(p[0], half);
    assert_eq!(p[1], format!("{half}\ntail"));

    // Two-byte characters, one more than the window holds.
    let long = "é".repeat(WINDOW / 2 + 1);
    let p = pieces(std::slice::from_ref(&long));
    assert_eq!(p.len(), 2);
    assert!(p.iter().all(|s| s.len() <= WINDOW));
    assert_eq!(p.concat(), long);

    let c = Corpus::from_docs(vec![Doc::new("long", vec![long])]);
    let v = value(&one("T2", &Recipe::new("", 0, SCOPE), &c)).clone();
    assert_eq!(v.n["pieces"], 2);
    assert!(v.flags.iter().any(|f| f == "windowed"), "{:?}", v.flags);
}

/// `n` characters of word-like text: words of two to nine letters from a vocabulary of 2,000,
/// skewed towards the first.
fn words(n: usize, seed: u64) -> String {
    let mut rng = Rng::new(&[5; 32], seed);
    let vocab: Vec<String> = (0..2000)
        .map(|_| {
            let len = 2 + rng.index(8);
            (0..len)
                .map(|_| char::from(b'a' + rng.index(26) as u8))
                .collect()
        })
        .collect();
    let mut s = String::new();
    while s.len() < n {
        let top = rng.index(vocab.len()) + 1;
        s.push_str(&vocab[rng.index(top)]);
        s.push(' ');
    }
    s.truncate(n);
    s
}

/// §5.1, T6: NCD(x, x) < 0.1, NCD(x, noise) > 0.9, and symmetric by construction — one value
/// per pair, the same with the orders swapped. The two copies of x merge first.
#[test]
fn t6_tells_a_text_from_itself_and_from_noise() {
    let x = words(60_000, 1);
    let mut rng = Rng::new(&[6; 32], 0);
    let noise = text_of(60_000, || char::from(b'a' + rng.index(26) as u8));
    let c = Corpus::from_docs(vec![
        Doc::new("a-x", vec![x.clone()]),
        Doc::new("b-x", vec![x]),
        Doc::new("c-noise", vec![noise]),
    ]);
    let v = value(&one("T6", &Recipe::new("", 0, SCOPE), &c)).clone();
    assert_eq!(
        v.values.len(),
        3,
        "one value per pair: {:?}",
        v.values.keys()
    );
    let same = fixed(&v, "ncd:a-x|b-x");
    assert!(same < 0.1, "NCD(x, x) = {same}");
    for k in ["ncd:a-x|c-noise", "ncd:b-x|c-noise"] {
        let d = fixed(&v, k);
        assert!(d > 0.9, "{k} = {d}");
    }
    assert_eq!(v.n["texts"], 3);
    assert_eq!(v.n["pairs"], 3);
    let l = &v.series["linkage"];
    assert_eq!(l.len(), 8, "two merges of four numbers");
    assert_eq!(&l[..2], &[Num::Count(0), Num::Count(1)], "the copies first");
    assert_eq!(l[3], Num::Count(2));
    assert_eq!(&l[4..6], &[Num::Count(2), Num::Count(3)]);
    assert_eq!(l[7], Num::Count(3));

    for (cx, cy, cxy, cyx) in [(10, 20, 25, 27), (100, 40, 130, 120), (7, 7, 9, 8)] {
        assert_eq!(ncd(cx, cy, cxy, cyx), ncd(cy, cx, cyx, cxy));
    }
}

/// §5.1, T6 window: two 9 MiB texts are refused (`SMY-E484`) before either is compressed.
#[test]
fn t6_refuses_a_pair_past_the_window() {
    let nine = "word ".repeat((9 << 20) / 5);
    assert!(nine.len() * 2 > WINDOW && nine.len() < WINDOW);
    let c = Corpus::from_docs(vec![
        Doc::new("left", vec![nine.clone()]),
        Doc::new("right", vec![nine]),
    ]);
    assert_eq!(
        refusal(&one("T6", &Recipe::new("", 0, SCOPE), &c)),
        Code::E484
    );
}

/// Average linkage on four points in two tied pairs: the tie goes to the pair whose labels come
/// first, the merged pairs are nodes 4 and 5, and they join at the mean of their four distances.
#[test]
fn average_linkage_breaks_ties_by_label() {
    let n = 4;
    let mut d = vec![0.0; n * n];
    let mut put = |i: usize, j: usize, v: f64| {
        d[i * n + j] = v;
        d[j * n + i] = v;
    };
    put(0, 1, 0.25);
    put(2, 3, 0.25);
    put(0, 2, 0.5);
    put(0, 3, 0.75);
    put(1, 2, 0.75);
    put(1, 3, 1.0);
    let m = average_linkage(&d, n);
    let at = |a, b, height, size| Merge { a, b, height, size };
    assert_eq!(
        m,
        [
            at(0, 1, 0.25, 2),
            at(2, 3, 0.25, 2),
            at(4, 5, (0.5 + 0.75 + 0.75 + 1.0) / 4.0, 4)
        ]
    );
}

/// The recipe a compressing measure carries names its codec and its `min:n`; a recipe naming
/// another codec is not this build's (`SMY-E489`), and too little to measure is not run
/// (`SMY-W485`).
#[test]
fn a_compression_recipe_names_its_codec() {
    let c = Corpus::from_docs(vec![Doc::new("one", vec![words(5_000, 3)])]);
    let r = one("T2", &Recipe::new("", 0, SCOPE), &c);
    assert_eq!(r.recipe.codec.as_deref(), Some(CODEC));
    assert_eq!(r.recipe.min_n, Some(1));
    value(&r);

    let mut other = Recipe::new("", 0, SCOPE);
    other.codec = Some("brotli-rs/8.0.4 q11 w24".into());
    assert_eq!(refusal(&one("T2", &other, &c)), Code::E489);
    assert_eq!(refusal(&one("T6", &other, &c)), Code::E489);

    let t6 = one("T6", &Recipe::new("", 0, SCOPE), &c);
    assert_eq!(t6.recipe.min_n, Some(2));
    assert_eq!(refusal(&t6), Code::W485, "one text is no distance");

    let terms = Corpus::from_terms(
        "smysl/an/1:surface",
        Stream::Surface,
        Level::Part,
        vec![vec!["a".into()]],
    );
    assert_eq!(
        refusal(&one("T2", &Recipe::new("", 0, SCOPE), &terms)),
        Code::W485,
        "a corpus of terms has no characters"
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

const BOOKS: [&str; 2] = ["08-RUT.usfm", "65-JUD.usfm"];
const BIBLES: [(&str, &str, &str); 2] = [("kjv", "en", "eng-kjv"), ("synodal", "ru", "russyn")];

/// The KJV's Ruth and Jude, and the Synodal's, as `kjv/08-rut` and the like.
fn library() -> Built {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "smysl-measure-compress-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
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

/// T2 over the KJV's Ruth and Jude at verse level, and T6 over all four books at part level:
/// the scope resolved to texts, the basis saved, the units written, and the results pinned for
/// the ports beside the codec's own lengths for the four slice files.
#[test]
fn t2_and_t6_over_a_library() {
    const PLAIN: &str = "smysl/an/1:surface";
    let built = library();
    let lib = &built.library;
    let store = lib.store();
    let ls = LibraryScope::new("lib", lib);
    let env = Env::new(store, &ls);

    let mut got = String::from(
        "# TX-P8 M3: the codec over the slice files, T2 over the KJV's Ruth and Jude at verse\n\
         # level, and T6 over the four books at part level\n",
    );
    got.push_str("# measure\tkey\tvalue\n");
    got.push_str(&format!("codec\tid\t{CODEC}\n"));
    for (_, _, dir) in BIBLES {
        for book in BOOKS {
            let raw =
                std::fs::read(repo("fixtures/library/bibles/slice").join(dir).join(book)).unwrap();
            got.push_str(&format!(
                "codec\t{dir}/{book}\t{} {}\n",
                raw.len(),
                compressed_len(&raw).unwrap()
            ));
        }
    }

    for (id, scope_text, level) in [
        ("T2", "from collection:kjv", Level::Verse),
        ("T6", "from texts", Level::Part),
    ] {
        let texts = text::texts(scope_text, lib).unwrap();
        let c = Corpus::load(lib, &texts, level, Some(PLAIN)).unwrap();
        let resolved = resolve(scope_text, &env, &PlanOptions::default()).unwrap();
        let input = smysl_measure::Input::new(store, &resolved).with_text(&c);
        let mut recipe = Recipe::new("", 0, scope_text);
        recipe.level = level;
        recipe.analyzer = Some(PLAIN.to_string());
        let r = run(&[id], &recipe, &input).remove(0);
        let v = value(&r);
        assert_eq!(r.recipe.scope_digest, c.digest);
        let unit = to_unit(&r, resolved.basis.as_ref()).expect("a unit");
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
        for (k, s) in &v.series {
            let s: Vec<String> = s.iter().map(Num::render).collect();
            got.push_str(&format!("{id}\tseries.{k}\t{}\n", s.join(" ")));
        }
        got.push_str(&format!("{id}\tflags\t{}\n", v.flags.join(" ")));
        got.push_str(&format!("{id}\tgist\t{}\n", gist(&r.recipe, v)));
        got.push_str(&format!(
            "{id}\tunit.uid\t{}\n",
            canonical_uid(&unit).canonical()
        ));
    }
    let path = repo("fixtures/measure/compress.tsv");
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
