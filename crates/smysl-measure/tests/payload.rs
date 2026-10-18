//! TX-P8 M1's exit (SMYSL-2.6 §6): golden payload bytes for a hand-built value, and the frame
//! a measure will run in — recipe, scope, basis, unit — before any measure exists.
//!
//! The goldens are in `fixtures/measure/golden.tsv`, one `name<TAB>hex` per line, and are
//! language-free on purpose: the payload layout is a format (OQ-56), and a port reproducing it
//! checks itself against the same bytes. `SMYSL_BLESS=1` rewrites the file, which should be a
//! decision — a changed golden here is a changed uid for every result with that shape.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smysl_core::surface::parse_surface;
use smysl_core::{Code, Record, SchemaId, Status};
use smysl_graph::Store;
use smysl_measure::num::{exp, ln, log2, powf, q20};
use smysl_measure::rng::Rng;
use smysl_measure::{
    digest, resolve, run_with, to_payload, to_unit, Ci, Input, Measure, NotPersisted, Num, Outcome,
    Param, Purity, Recipe, Refusal, Resolved, Value,
};
use smysl_query::{Env, PlanOptions, StoreScope};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Every golden this file pins, compared at once, so a change reports all it moved.
fn goldens(actual: &BTreeMap<&str, String>) {
    let path = repo("fixtures/measure/golden.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        let mut out = String::from(
            "# TX-P8 M1 goldens (SMYSL-2.6 §6): name<TAB>lowercase hex. Written by\n\
             # crates/smysl-measure/tests/payload.rs under SMYSL_BLESS=1.\n",
        );
        for (k, v) in actual {
            out.push_str(&format!("{k}\t{v}\n"));
        }
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, out).unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path).expect("fixtures/measure/golden.tsv; bless it");
    let want: BTreeMap<&str, &str> = text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .filter_map(|l| l.split_once('\t'))
        .collect();
    let mut moved = Vec::new();
    for (k, v) in actual {
        match want.get(k) {
            Some(w) if w == v => {}
            Some(w) => moved.push(format!("{k}:\n  want {w}\n  got  {v}")),
            None => moved.push(format!("{k}: not in the golden file")),
        }
    }
    for k in want.keys() {
        if !actual.contains_key(k) {
            moved.push(format!("{k}: in the golden file and no longer produced"));
        }
    }
    assert!(moved.is_empty(), "goldens moved:\n{}", moved.join("\n"));
}

/// The value the RFC's T2 example shows (§3.6), extended to every shape a payload can hold.
fn hand_built() -> (Recipe, Value) {
    let mut r = Recipe::new(
        "T2",
        1,
        "from texts where alias = \"kjv-1769\" and book = \"Ps\"",
    );
    r.scope_digest = digest([[7u8; 32], [3u8; 32]]);
    r.codec = Some("brotli-rs/8.0.4 q11 w24".to_string());
    r.analyzer = Some("en/1:surface".to_string());
    r.min_n = Some(10_000);
    r.params.insert("quality".to_string(), Param::Int(11));
    r.params.insert("smoothing".to_string(), Param::Ratio(1, 2));
    let v = Value::new()
        .with_n("chars", 243_117)
        .with_n("bytes", 243_117)
        .with_value("bits/char", Num::Fixed(2_393_899))
        .with_value("bits/word", Num::Fixed(10_727_114))
        .with_value("parts", Num::Count(150))
        .with_value("ascii", Num::Ratio(243_001, 243_117))
        .with_ci(
            "bits/char",
            Ci {
                lo: Num::Fixed(2_380_000),
                hi: Num::Fixed(2_410_000),
                level: (95, 100),
                b: 1000,
            },
        )
        .with_series(
            "per-part",
            vec![Num::Fixed(2_300_000), Num::Fixed(-1), Num::Count(0)],
        )
        .with_flag("stem-level")
        .with_flag("low-n")
        .with_headline("bits/char", "brotli rate");
    (r, v)
}

const SURFACE: &str = r#"
@claim c/a { status: cited, source: { kind: doc, ref: "a" } }
~ The first claim the scope holds.

@claim c/b { status: speculative }
~ The second claim the scope holds.

@observation o/c { status: cited, source: { kind: doc, ref: "c" } }
~ An observation the scope leaves out.
"#;

fn store() -> Store {
    Store::from_records(parse_surface(SURFACE).expect("parses").records)
}

const SCOPE: &str = "where kind = claim";

/// A measure for the frame's sake: counts the scope, and is the only one this build knows.
struct Census;

impl Measure for Census {
    fn id(&self) -> &'static str {
        "X0"
    }
    fn version(&self) -> u16 {
        1
    }
    fn purity(&self) -> Purity {
        Purity::Pure
    }
    fn needs(&self) -> smysl_measure::Needs {
        smysl_measure::Needs::UNITS
    }
    fn admit(&self, r: &Recipe, _: &Input<'_>) -> Result<(), Refusal> {
        if r.lossy {
            return Err(Refusal::new(Code::E489, "X0 has no lossy reading"));
        }
        Ok(())
    }
    fn run(&self, _: &Recipe, input: &Input<'_>) -> Outcome {
        let n = input.scope.uids.len() as u64;
        Outcome::Value(
            Value::new()
                .with_n("units", n)
                .with_value("units", Num::Count(n))
                .with_headline("units", "census"),
        )
    }
}

#[test]
fn goldens_hold() {
    let (recipe, value) = hand_built();
    let mut actual = BTreeMap::new();
    actual.insert("recipe.cbor", hex(&recipe.to_cbor().unwrap()));
    actual.insert("recipe.hash", hex(&recipe.hash().unwrap()));
    actual.insert("payload.cbor", hex(&to_payload(&recipe, &value).unwrap()));
    actual.insert("scope.digest", hex(&digest([[7u8; 32], [3u8; 32]])));

    // libm, by bit pattern. Pure Rust with `arch` off computes these the same on every target,
    // which is the claim; the platform's `f64::ln` is not part of it and clippy refuses it here.
    let bits = |x: f64| format!("{:016x}", x.to_bits());
    actual.insert("libm.ln(2)", bits(ln(2.0)));
    actual.insert("libm.ln(10)", bits(ln(10.0)));
    actual.insert("libm.ln(0.7)", bits(ln(0.7)));
    actual.insert("libm.log2(3)", bits(log2(3.0)));
    actual.insert("libm.exp(-1.5)", bits(exp(-1.5)));
    actual.insert("libm.pow(1.1,2.5)", bits(powf(1.1, 2.5)));

    // The generator's first draws for an all-zero recipe hash and seed 0 (OQ-57).
    let mut g = Rng::new(&[0; 32], 0);
    let draws: Vec<String> = (0..4).map(|_| format!("{:016x}", g.next_u64())).collect();
    actual.insert("rng.zero.first4", draws.join(""));

    // A whole unit: the hand-built value over a resolved scope, by uid.
    let s = store();
    let scope = StoreScope::new("m", &s);
    let env = Env::new(&s, &scope);
    let resolved = resolve(SCOPE, &env, &PlanOptions::default()).expect("resolves");
    let unit = to_unit(
        &smysl_measure_result(recipe, value),
        resolved.basis.as_ref(),
    )
    .expect("a unit");
    actual.insert("unit.uid", smysl_core::canonical_uid(&unit).canonical());
    actual.insert("unit.gist", hex(unit.gist.as_bytes()));
    goldens(&actual);
}

/// A result as `run` would hand it back, for a value built by hand.
fn smysl_measure_result(recipe: Recipe, value: Value) -> smysl_measure::MeasureResult {
    let (r, v) = (recipe, value);
    let s = store();
    let resolved = Resolved::of_uids("hand", BTreeSet::new());
    let input = Input::new(&s, &resolved);
    struct Given(Value);
    impl Measure for Given {
        fn id(&self) -> &'static str {
            "T2"
        }
        fn version(&self) -> u16 {
            1
        }
        fn purity(&self) -> Purity {
            Purity::Pure
        }
        fn needs(&self) -> smysl_measure::Needs {
            smysl_measure::Needs::NONE
        }
        fn admit(&self, _: &Recipe, _: &Input<'_>) -> Result<(), Refusal> {
            Ok(())
        }
        fn run(&self, _: &Recipe, _: &Input<'_>) -> Outcome {
            Outcome::Value(self.0.clone())
        }
    }
    let m = Given(v);
    let digest = r.scope_digest;
    let mut out = run_with(&[&m], &["T2"], &r, &input).remove(0);
    // `run_with` stamps the scope's digest; this value was built over the hand-made one.
    out.recipe.scope_digest = digest;
    out
}

#[test]
fn a_payload_holds_integers_and_no_floats() {
    let (recipe, value) = hand_built();
    let bytes = to_payload(&recipe, &value).unwrap();
    // CBOR major 7 with additional info 25, 26 or 27 is a float; none may appear.
    let o = smysl_core::surface::payload_to_object(&bytes).expect("decodes");
    fn no_float(v: &smysl_core::surface::hjson::HValue) -> bool {
        use smysl_core::surface::hjson::HValue;
        match v {
            HValue::Float(_) => false,
            HValue::Array(a) => a.iter().all(|i| no_float(&i.value)),
            HValue::Object(o) => o.iter().all(|(_, v)| no_float(&v.value)),
            _ => true,
        }
    }
    assert!(o.iter().all(|(_, v)| no_float(&v.value)));
    assert!(smysl_core::surface::payload::is_lossless(
        &smysl_core::surface::hjson::HValue::Object(o)
    ));
}

#[test]
fn every_field_with_a_value_moves_the_recipe_hash_and_absent_fields_do_not() {
    let base = Recipe::new("T1", 1, SCOPE);
    let h = base.hash().unwrap();
    let mut seeded = base.clone();
    seeded.seed = 1;
    let mut replicates = base.clone();
    replicates.boot_b = 999;
    let mut versioned = base.clone();
    versioned.v = 2;
    for (what, r) in [("seed", seeded), ("boot:b", replicates), ("v", versioned)] {
        assert_ne!(r.hash().unwrap(), h, "{what} must move the hash");
    }
    // An absent optional is not written, so it is not a null that would change with the schema.
    let cbor = base.to_cbor().unwrap();
    let o = smysl_core::surface::payload_to_object(&cbor).unwrap();
    for absent in ["policy", "codec", "lenses", "params", "min:n"] {
        assert!(o.get(absent).is_none(), "{absent} written when absent");
    }
    for present in [
        "seed",
        "boot:b",
        "level",
        "axis",
        "filter",
        "undated:max",
        "lossy",
    ] {
        assert!(
            o.get(present).is_some(),
            "{present}, a default, not written"
        );
    }
    // A seed past a payload integer is refused rather than wrapped.
    let mut big = base;
    big.seed = u64::MAX;
    assert_eq!(big.hash().unwrap_err().diagnostic().code, Code::E489);
}

#[test]
fn the_basis_is_the_saved_scope_query_and_the_unit_rests_on_it_alone() {
    let s = store();
    let scope = StoreScope::new("m", &s);
    let env = Env::new(&s, &scope);
    let resolved = resolve(SCOPE, &env, &PlanOptions::default()).expect("resolves");
    assert_eq!(resolved.uids.len(), 2, "two claims");
    assert_eq!(
        resolved.digest,
        digest(resolved.uids.iter().map(|u| u.to_bytes()))
    );
    let basis = resolved.basis.clone().expect("an sq scope has a basis");
    assert_eq!(
        basis.unit.schema,
        SchemaId::Extension(smysl_core::schemas::query::SAVED.to_string())
    );
    assert_eq!(basis.unit.status, Status::Cited);
    assert_eq!(basis.edges.len(), 2, "one answer edge per unit in scope");

    let input = Input::new(&s, &resolved);
    let base = Recipe::new("X0", 0, SCOPE);
    let result = run_with(&[&Census], &["X0"], &base, &input).remove(0);
    assert_eq!(result.recipe.scope_digest, resolved.digest);
    let unit = to_unit(&result, resolved.basis.as_ref()).expect("persisted");
    assert_eq!(unit.schema, SchemaId::Kernel(smysl_core::KernelType::Data));
    assert_eq!(unit.status, Status::Derived);
    assert_eq!(unit.grounds, BTreeSet::from([basis.uid]));
    assert!(unit.deps.is_empty());
    assert!(unit.source.is_none());
    assert!(
        unit.gist
            .starts_with("X0 census 2 units, over where kind = claim"),
        "{}",
        unit.gist
    );

    // Rule M holds over the result: append the declaration, the basis and the unit, and the
    // store checks clean of E030 and E033 (§8, reused codes never expected on a measure unit).
    let mut records = vec![
        Record::SchemaDecl(smysl_core::schemas::query::decl()),
        Record::Unit(basis.unit.clone()),
        Record::Unit(unit.clone()),
    ];
    records.extend(basis.edges.iter().cloned().map(Record::Relation));
    let mut written = store();
    written.append(&records).expect("appends");
    let report = smysl_check::check(&written, smysl_check::CheckOptions::default());
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|d| d.code == Code::E030 || d.code == Code::E033),
        "{report:?}"
    );

    // The same recipe over the same scope is the same unit; a different seed is another.
    let again = to_unit(
        &run_with(&[&Census], &["X0"], &base, &input).remove(0),
        resolved.basis.as_ref(),
    )
    .unwrap();
    assert_eq!(
        smysl_core::canonical_uid(&again),
        smysl_core::canonical_uid(&unit)
    );
    let mut reseeded = base.clone();
    reseeded.seed = 7;
    let other = to_unit(
        &run_with(&[&Census], &["X0"], &reseeded, &input).remove(0),
        resolved.basis.as_ref(),
    )
    .unwrap();
    assert_ne!(
        smysl_core::canonical_uid(&other),
        smysl_core::canonical_uid(&unit)
    );
}

#[test]
fn a_refusal_an_unknown_id_and_a_missing_basis_are_reported_and_never_units() {
    let s = store();
    let resolved = Resolved::of_uids("a harness's set", s.units().map(|(u, _)| *u).collect());
    let input = Input::new(&s, &resolved);
    let mut base = Recipe::new("X0", 0, "a harness's set");

    let unknown = run_with(&[&Census], &["T9"], &base, &input).remove(0);
    assert!(matches!(
        unknown.outcome,
        Outcome::NotRun {
            code: Some(Code::E489),
            ..
        }
    ));
    assert!(
        unknown.diagnostics[0].message.contains("X0"),
        "names what exists"
    );
    assert!(matches!(to_unit(&unknown, None), Err(NotPersisted::NotRun)));

    let ran = run_with(&[&Census], &["X0"], &base, &input).remove(0);
    match to_unit(&ran, resolved.basis.as_ref()) {
        Err(NotPersisted::NoBasis(d)) => assert_eq!(d.code, Code::W483),
        other => panic!("{other:?}"),
    }

    base.lossy = true;
    let refused = run_with(&[&Census], &["X0"], &base, &input).remove(0);
    assert!(matches!(refused.outcome, Outcome::NotRun { .. }));
    assert_eq!(refused.diagnostics.len(), 1);
}

#[test]
fn the_generator_is_seeded_by_recipe_and_seed_and_its_index_is_the_written_rule() {
    let h = Recipe::new("T4", 1, SCOPE).hash().unwrap();
    let a: Vec<u64> = {
        let mut g = Rng::new(&h, 0);
        (0..8).map(|_| g.next_u64()).collect()
    };
    let b: Vec<u64> = {
        let mut g = Rng::new(&h, 0);
        (0..8).map(|_| g.next_u64()).collect()
    };
    let c: Vec<u64> = {
        let mut g = Rng::new(&h, 1);
        (0..8).map(|_| g.next_u64()).collect()
    };
    assert_eq!(a, b);
    assert_ne!(a, c);
    // `(x · n) >> 64`, against the draw it was computed from.
    let mut g = Rng::new(&h, 0);
    let mut d = Rng::new(&h, 0);
    for n in [1usize, 2, 7, 1000, 1 << 23] {
        let x = d.next_u64();
        let i = g.index(n);
        assert_eq!(i, ((u128::from(x) * n as u128) >> 64) as usize);
        assert!(i < n);
    }
}

#[test]
fn q20_refuses_what_a_failed_measure_would_produce() {
    assert_eq!(q20(f64::NAN), None);
    assert_eq!(q20(ln(0.0)), None, "ln 0 is -inf, not a stored number");
}

/// §3.5 and §4.5, as a scan of this crate's source: no `HashMap` (iteration order is seeded
/// per process), no platform transcendental, no clock, no environment, no unseeded generator.
/// Crude, like the purity grep, and on purpose; clippy's `disallowed-methods` covers the
/// transcendentals by type, this covers them by name and covers what clippy cannot.
#[test]
fn the_source_holds_nothing_section_3_5_forbids() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut todo = vec![dir.clone()];
    let mut seen = 0;
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                todo.push(p);
                continue;
            }
            if p.extension().and_then(|x| x.to_str()) != Some("rs") {
                continue;
            }
            seen += 1;
            let src = std::fs::read_to_string(&p).unwrap();
            let name = p.strip_prefix(&dir).unwrap().display().to_string();
            // Lines that are comments say why a thing is forbidden, and may name it.
            let code: String = src
                .lines()
                .filter(|l| {
                    let t = l.trim_start();
                    !t.starts_with("//")
                })
                .collect::<Vec<_>>()
                .join("\n");
            for bad in [
                "HashMap",
                "HashSet",
                ".ln()",
                ".log2()",
                ".log10()",
                ".exp()",
                ".powf(",
                ".powi(",
                "SystemTime",
                "Instant",
                "std::env",
                "thread_rng",
                "OsRng",
                "from_entropy",
                "getrandom",
            ] {
                assert!(!code.contains(bad), "{name} names `{bad}` (SMYSL-2.6 §3.5)");
            }
        }
    }
    assert!(seen >= 5, "the crate's modules were read");
}
