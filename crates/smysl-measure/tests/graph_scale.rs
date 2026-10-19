//! TX-P9 N1's scale exit (SMYSL-2.6 §6): a synthetic scope of 10⁶ units runs A1, A2, A4, A5
//! and A7 in time within 2× of 10× the 10⁵ run — that is, no worse than linear by more than a
//! factor of two. Its timings feed GE-T7.
//!
//! `--ignored`, in release: building a million units takes longer than the rest of the suite.
//!
//! ```text
//! cargo test --release -p smysl-measure --test graph_scale -- --ignored --nocapture
//! ```
//!
//! The store is shaped so that every measure has work to do at every size: units in groups of
//! ten, each grounded on the one before it (A5's chains, depth up to nine); a same-as pair in
//! every hundred (A2's classes); a warrant in every fifty (E6); a contradiction in every
//! thousand (A7); and every unit drawn from the KJV's Ruth (A1's words, and the one text the
//! per-text figures are over). Only the measures are timed, not the building.

use std::path::{Path, PathBuf};
use std::time::Instant;

use smysl_core::ids::LangTag;
use smysl_core::types::{Carry, RelKind, SourceKind, SourceRef};
use smysl_core::{canonical_uid, KernelType, Record, Relation, Status, UnitCoreBuilder};
use smysl_graph::proposition::SAME_AS;
use smysl_graph::Store;
use smysl_measure::{run, ClassPolicy, Input, Outcome, Recipe, Resolved};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input as ReadInput;

const IDS: [&str; 5] = ["A1", "A2", "A4", "A5", "A7"];

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn store(n: u32, library: &Library) -> (Store, Resolved) {
    let mut records: Vec<Record> = Vec::with_capacity(n as usize * 2);
    let mut tid = None;
    for (_, m) in library.store().manifests() {
        records.push(Record::Manifest(m.clone()));
        tid = Some(m.parts[0].tid);
    }
    let reference = tid.expect("Ruth has a part").canonical();
    let mut uids = Vec::with_capacity(n as usize);
    for i in 0..n {
        let mut b = UnitCoreBuilder::new(
            KernelType::Claim,
            format!("unit {i} of a synthetic scope"),
            Status::Speculative,
        )
        .source(SourceRef::new(SourceKind::Doc, reference.clone()));
        if i % 10 != 0 {
            b = b.grounds([uids[i as usize - 1]]);
        }
        let core = b.build().expect("a unit");
        uids.push(canonical_uid(&core));
        records.push(Record::Unit(core));
    }
    for i in 0..n as usize {
        if i % 100 == 0 && i + 1 < uids.len() {
            records.push(Record::Relation(Relation::new(
                RelKind::Extension(SAME_AS.into()),
                uids[i],
                uids[i + 1],
            )));
        }
        if i % 50 == 1 {
            records.push(Record::Relation(Relation::new(
                RelKind::Warrant,
                uids[i - 1],
                uids[i],
            )));
        }
        if i % 1000 == 2 {
            records.push(Record::Relation(Relation::new(
                RelKind::Extension(smysl_core::schemas::text::CONTRADICTS.into()),
                uids[i],
                uids[i - 2],
            )));
        }
    }
    let store = Store::from_records(records);
    let scope = Resolved::of_uids("synthetic", uids.into_iter().collect());
    (store, scope)
}

fn timed(n: u32, library: &Library) -> f64 {
    let (store, scope) = store(n, library);
    let input = Input::new(&store, &scope).with_library(library);
    let mut recipe = Recipe::new("", 0, "synthetic");
    recipe.policy = Some(ClassPolicy::Strict);
    recipe.profile = Some("default".into());
    let start = Instant::now();
    let mut results = Vec::new();
    let mut each = Vec::new();
    for id in IDS {
        let t = Instant::now();
        results.extend(run(&[id], &recipe, &input));
        each.push(format!("{id} {:.3}", t.elapsed().as_secs_f64()));
    }
    let secs = start.elapsed().as_secs_f64();
    eprintln!("{n:>8} units: {}", each.join(", "));
    for r in &results {
        assert!(
            matches!(r.outcome, Outcome::Value(_)),
            "{} at {n}: {:?}",
            r.recipe.id,
            r.outcome
        );
    }
    eprintln!("{n:>8} units: {secs:.3} s for {}", IDS.join(", "));
    secs
}

#[test]
#[ignore = "builds a million units; run in release with --ignored"]
fn a_million_units_take_at_most_twenty_times_a_hundred_thousand() {
    let root = std::env::temp_dir().join(format!("smysl-measure-scale-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut library = Library::create(&root).unwrap();
    let raw = std::fs::read(repo("fixtures/library/bibles/slice/eng-kjv/08-RUT.usfm")).unwrap();
    let spec = AddSpec::new("usfm/1", "kjv/08-rut", "public-domain")
        .with_lang(LangTag::new("en").unwrap())
        .with_carry(Carry::Text);
    library
        .add(&ReadInput::new(&raw), &spec, &Caps::DEFAULT)
        .unwrap();

    let small = timed(100_000, &library);
    let large = timed(1_000_000, &library);
    let _ = std::fs::remove_dir_all(&root);
    eprintln!("ratio {:.2} (the bound is 20)", large / small);
    assert!(
        large <= 20.0 * small,
        "10⁶ units took {large:.3} s and 10⁵ took {small:.3} s: {:.1}×, past 2× of 10×",
        large / small
    );
}
