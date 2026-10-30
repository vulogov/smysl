//! GE-T15, the round-trip gate of TX-P11 (SMYSL-2.7 §5.2, step 12; gate G2).
//!
//! A table: every corpus × every layout × both closure modes × both formats × two flag sets.
//! Per cell the collection is exported and verified, and the kill criteria of draft 3 §23 are
//! asserted:
//!
//! - **no P1 or P3 failure.** P1 and P3 in memory for every CBOR cell, and P1 again through a
//!   real import into an empty library, retained records against the index's digest. A surface
//!   cell reads back as exactly what it can spell (`SMY-E424` never raised); P3 is CBOR's.
//!   Beside it, the accepted-only digest (OQ-60): equal to the index's whenever the source
//!   checks clean and nothing in it dangles (`SMY-W495`); the cells where it is not are counted.
//! - **no record type lost.** Every record type present in the export is present among what
//!   the import retained, checked per type code and not by digest alone.
//! - **P2** for every corpus file imported raw.
//! - **The duplication factor per layout** is recorded in `fixtures/ge-t15/duplication.tsv`
//!   (OQ-26), and `by-expression` stays under the 3× kill.
//!
//! The corpora are F1–F13 (F8a and F8b merged), every wire fixture, a generated store of F13 and
//! F14's unknown codes, the five Bibles with their text (records 14, 15, 18, 19), and the two
//! chats with a dating, commitments, a withdrawal, a resolution and a redaction (11–13, 17, 19).
//! A cell that cannot apply is skipped by name in the report, never silently: `by-view` over a
//! store with no corpus view, and `--with-text` over a plain store.
//!
//! The scale cells, `--ignored`, export and import generated stores of 10⁵ and 10⁶ records and
//! print throughput and peak RSS against §2.10's baseline.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Instant;

use smysl_check::{check, CheckOptions};
use smysl_core::{Carry, Code, Record};
use smysl_graph::Store;
use smysl_lib::scope::group;
use smysl_lib::{
    accepted, export_library, export_store, import, p2, retained, snapshot, verify_collection,
    write_collection, Closure, Collection, ExportOptions, Format, ImportOptions, Input, Layout,
    Property, Selection, With, INDEX_FILE,
};
use smysl_text::Library;

mod common;

use common::repo;

fn dir(name: &str) -> PathBuf {
    common::dir("ge-t15", name)
}

/// A corpus: a plain store, or a library with what only a library has.
enum Source {
    Store(Box<Store>),
    Library(Box<Library>),
}

fn corpora() -> Vec<(String, Source)> {
    let store = |records: Vec<Record>| Source::Store(Box::new(Store::from_records(records)));
    let mut out = Vec::new();
    let mut f8 = Vec::new();
    for rel in common::corpus() {
        let stem = rel
            .rsplit('/')
            .next()
            .unwrap()
            .split('.')
            .next()
            .unwrap()
            .to_lowercase();
        if stem.starts_with("f8") {
            f8.extend(common::fixture(&rel));
            continue;
        }
        out.push((stem, store(common::fixture(&rel))));
    }
    out.push(("f8-merged".into(), store(f8)));
    for e in std::fs::read_dir(repo().join("fixtures/wire")).unwrap() {
        let name = e.unwrap().file_name().to_string_lossy().to_string();
        if !name.ends_with(".cbor")
            || name == "F10-lifecycle.cbor"
            || name == "F14-unknown-codes.cbor"
        {
            continue;
        }
        let rel = format!("fixtures/wire/{name}");
        let key = format!("wire-{}", name.trim_end_matches(".cbor").to_lowercase());
        out.push((key, store(common::fixture(&rel))));
    }
    let mut generated = common::fixture("fixtures/corpus/F13-canon.smy");
    generated.extend(common::fixture("fixtures/wire/F14-unknown-codes.cbor"));
    out.push(("generated-unknown".into(), store(generated)));
    out.push((
        "bibles".into(),
        Source::Library(Box::new(common::bibles(&dir("bibles"), Carry::Text))),
    ));
    out.push((
        "chats".into(),
        Source::Library(Box::new(common::chats(&dir("chats")).0)),
    ));
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

const LAYOUTS: [Layout; 6] = [
    Layout::Single,
    Layout::ByView,
    Layout::ByExpression,
    Layout::ByCollection,
    Layout::ByTime,
    Layout::ByQuery,
];

fn all_with(library: bool) -> With {
    With::new()
        .views(true)
        .packs(true)
        .text(library)
        .staged(library)
        .quarantine(library)
}

fn export(src: &Source, opts: &ExportOptions) -> Collection {
    let select = |store: &Store| {
        let all = Selection::all(store);
        match opts.layout {
            Layout::ByQuery => group(store, all, "kind").unwrap(),
            _ => all,
        }
    };
    match src {
        Source::Store(s) => export_store(s, &select(s), opts).unwrap(),
        Source::Library(lib) => {
            let snap = snapshot(lib.as_ref(), &opts.with).unwrap();
            export_library(lib.as_ref(), &snap, &select(&snap), opts).unwrap()
        }
    }
}

fn types(rs: &[Record]) -> BTreeSet<u64> {
    rs.iter().map(Record::type_code).collect()
}

#[test]
fn ge_t15_holds_on_the_whole_matrix() {
    let mut failures = Vec::new();
    let mut skipped = BTreeSet::new();
    let mut cells = 0;
    let mut duplication: BTreeMap<(String, &'static str), (u64, u64)> = BTreeMap::new();
    let mut unaccepted: BTreeMap<String, usize> = BTreeMap::new();
    for (name, src) in corpora() {
        let (has_views, library) = match &src {
            Source::Store(s) => (s.views().next().is_some(), false),
            Source::Library(l) => (l.store().views().next().is_some(), true),
        };
        let clean = !match &src {
            Source::Store(s) => check(s.as_ref(), CheckOptions::default()),
            Source::Library(l) => check(l.store(), CheckOptions::default()),
        }
        .has_errors();
        for layout in LAYOUTS {
            if layout == Layout::ByView && !has_views {
                skipped.insert(format!("{name} by-view: the store holds no corpus view"));
                continue;
            }
            for closure in [Closure::Full, Closure::Ref] {
                for format in [Format::Cbor, Format::Surface] {
                    for (flags, with) in [("none", With::new()), ("with-all", all_with(library))] {
                        cells += 1;
                        let cell = format!(
                            "{name} {}/{}/{}/{flags}",
                            layout.as_str(),
                            closure.as_str(),
                            format.as_str()
                        );
                        let opts = ExportOptions::new()
                            .layout(layout)
                            .closure(closure)
                            .format(format)
                            .with(with);
                        let c = export(&src, &opts);
                        if closure == Closure::Full && format == Format::Cbor && flags == "none" {
                            duplication.insert((name.clone(), layout.as_str()), c.duplication());
                        }
                        let v = verify_collection(&c);
                        if !v.holds() || !v.p1.holds() {
                            failures.push(format!("{cell}: P1 {v:?}"));
                            continue;
                        }
                        match format {
                            Format::Cbor if !v.p3.holds() => {
                                failures.push(format!("{cell}: P3 {:?}", v.p3));
                                continue;
                            }
                            Format::Surface if !c.index.complete => {
                                failures.push(format!("{cell}: SMY-E424 {:?}", c.report));
                                continue;
                            }
                            _ => {}
                        }
                        // P1 through a real import, and no record type lost. A collection
                        // with no file (a layout that keys nothing over a store of no unit,
                        // F12's three pack infos) imports as nothing, which is what it holds.
                        if format == Format::Cbor && c.files.is_empty() {
                            if c.rsd() != smysl_graph::record_set_digest(&[]) {
                                failures
                                    .push(format!("{cell}: no file, and a digest of something"));
                            }
                        } else if format == Format::Cbor {
                            let out = dir(&format!("cells/{}", cells));
                            write_collection(&c, &out).unwrap();
                            let mut to = Library::create(&out.join("lib")).unwrap();
                            let r = import(
                                &mut to,
                                &[Input::Index(out.join(INDEX_FILE))],
                                &ImportOptions::new().accept(true).join(false),
                            )
                            .unwrap();
                            let kept = retained(&to, &r.name).unwrap();
                            if smysl_graph::record_set_digest(&kept) != c.rsd() {
                                failures.push(format!("{cell}: P1 through a real import"));
                            }
                            // OQ-60: the accepted-only digest beside P1's. It is the
                            // index's whenever the source checks clean and nothing it holds
                            // dangles (`SMY-W495`); otherwise the difference is counted.
                            let took = accepted(&to, &r.name).unwrap();
                            if smysl_graph::record_set_digest(&took) != c.rsd() {
                                if clean && !c.report.iter().any(|d| d.code == Code::W495) {
                                    failures.push(format!(
                                        "{cell}: accepted-only digest, from a clean source"
                                    ));
                                }
                                *unaccepted.entry(name.clone()).or_default() += 1;
                            }
                            let lost: Vec<u64> = types(&c.records())
                                .difference(&types(&kept))
                                .copied()
                                .collect();
                            if !lost.is_empty() {
                                failures.push(format!("{cell}: record types lost {lost:?}"));
                            }
                            let _ = std::fs::remove_dir_all(&out);
                        }
                    }
                }
            }
        }
        if !library {
            skipped.insert(format!("{name} --with-text, --with-staged, --with-quarantine: a plain store has no library"));
        }
    }
    eprintln!("GE-T15: {cells} cells, {} failure(s)", failures.len());
    for (corpus, n) in &unaccepted {
        eprintln!("GE-T15: {corpus}: {n} cell(s) where P1 holds and not every record is accepted");
    }
    for s in &skipped {
        eprintln!("GE-T15: skipped {s}");
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    // OQ-26: the duplication factor per corpus and layout, `full`, as `summed/once`.
    let mut tsv = String::from("# GE-T15's duplication factor under `full` (OQ-26): records selected summed over\n# files, records selected once, and the factor, per corpus and layout. Rewritten by\n# `SMYSL_BLESS=1 cargo test -p smysl-lib --test ge_t15`.\ncorpus\tlayout\tsummed\tonce\tfactor\n");
    for ((corpus, layout), (summed, once)) in &duplication {
        let factor = if *once == 0 {
            1.0
        } else {
            *summed as f64 / *once as f64
        };
        tsv.push_str(&format!(
            "{corpus}\t{layout}\t{summed}\t{once}\t{factor:.3}\n"
        ));
        if *layout == "by-expression" {
            assert!(
                factor <= 3.0,
                "{corpus}: by-expression duplicates {factor:.3}×, over GE-T15's 3× kill"
            );
        }
    }
    let golden = repo().join("fixtures/ge-t15/duplication.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, &tsv).unwrap();
    } else {
        assert_eq!(
            std::fs::read_to_string(&golden).unwrap_or_default(),
            tsv,
            "the duplication factors moved: rebuild with SMYSL_BLESS=1 if that is intended"
        );
    }
}

#[test]
fn p2_holds_for_every_corpus_file_imported_raw() {
    let mut files: Vec<PathBuf> = common::corpus().iter().map(|r| repo().join(r)).collect();
    for e in std::fs::read_dir(repo().join("fixtures/wire")).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_some_and(|x| x == "cbor") {
            files.push(p);
        }
    }
    files.sort();
    files.dedup();
    let mut failures = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let mut lib = Library::create(&dir(&format!("p2/{i}"))).unwrap();
        let r = import(
            &mut lib,
            &[Input::File(f.clone())],
            &ImportOptions::new().accept(true).join(false),
        )
        .unwrap();
        match p2(&lib, &r.name).unwrap() {
            Property::Holds => {}
            other => failures.push(format!("{}: {other:?}", f.display())),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The §2.10 shape: `@prose`-like units each grounded on the one before, with an attestation.
fn generated(records: usize) -> Store {
    use smysl_core::{canonical_uid, KernelType, Status, UnitCoreBuilder};
    let mut out = Vec::with_capacity(records);
    let mut prev = None;
    for i in 0..records / 2 {
        let mut b = UnitCoreBuilder::new(
            KernelType::Claim,
            format!("generated unit {i}"),
            Status::Speculative,
        );
        if let Some(p) = prev {
            b = b.grounds([p]);
        }
        let core = b.build().unwrap();
        let uid = canonical_uid(&core);
        out.push(Record::Unit(core));
        out.push(common::attest(uid, common::BASE + i as u64));
        prev = Some(uid);
    }
    Store::from_records(out)
}

fn rss_mb() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok();
    out.and_then(|o| {
        String::from_utf8_lossy(&o.stdout)
            .trim()
            .parse::<u64>()
            .ok()
    })
    .map_or(0, |kb| kb / 1024)
}

fn scale(records: usize) {
    let t = Instant::now();
    let store = generated(records);
    let built = t.elapsed();
    let rss_built = rss_mb();
    let t = Instant::now();
    let c = export_store(&store, &Selection::all(&store), &ExportOptions::new()).unwrap();
    let exported = t.elapsed();
    let rss_exported = rss_mb();
    let t = Instant::now();
    let v = verify_collection(&c);
    let verified = t.elapsed();
    let rss_verified = rss_mb();
    assert!(v.holds() && v.p1.holds() && v.p3.holds(), "{v:?}");
    let out = dir(&format!("scale-{records}"));
    write_collection(&c, &out).unwrap();
    let mut lib = Library::create(&out.join("lib")).unwrap();
    let t = Instant::now();
    let r = import(
        &mut lib,
        &[Input::Index(out.join(INDEX_FILE))],
        &ImportOptions::new().accept(true).join(false),
    )
    .unwrap();
    let imported = t.elapsed();
    let rss_imported = rss_mb();
    assert_eq!(
        smysl_graph::record_set_digest(&retained(&lib, &r.name).unwrap()),
        c.rsd()
    );
    let bytes: usize = c.files.iter().map(|f| f.bytes.len()).sum();
    eprintln!(
        "GE-T15 scale {records}: build {built:?} (RSS {rss_built} MB), export {exported:?} ({:.0} records/s, RSS {rss_exported} MB), verify {verified:?} (RSS {rss_verified} MB), import {imported:?} ({:.0} records/s, RSS {rss_imported} MB), {:.1} MB written",
        records as f64 / exported.as_secs_f64(),
        records as f64 / imported.as_secs_f64(),
        bytes as f64 / 1e6,
    );
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
#[ignore = "a scale cell: run in the GE-T15 job, or with --ignored"]
fn scale_cell_at_10e5_records() {
    scale(100_000);
}

#[test]
#[ignore = "a scale cell: run in the GE-T15 job, or with --ignored"]
fn scale_cell_at_10e6_records() {
    scale(1_000_000);
}
