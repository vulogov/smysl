//! The index, `verify`, `ref` closure and the duplication factor of TX-P11 step 3 (SMYSL-2.7
//! §3.7, §3.9, §4.2, §5.1).
//!
//! The step's exit: `verify` holds on F1–F13 × {single, by-view} × {full, ref}, and a flipped
//! byte gives exit 9.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smysl_core::surface::parse_surface;
use smysl_core::{
    canonical_uid, from_cbor_seq, Code, ExitCode, KernelType, Record, Status, UnitCore,
    UnitCoreBuilder, View, ViewId,
};
use smysl_graph::Store;
use smysl_lib::{
    export_store, verify, verify_collection, verify_files, write_collection, Closure, Collection,
    ExportIndex, ExportOptions, Layout, Property, Selection, With, INDEX_FILE,
};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(rel: &str) -> Vec<Record> {
    let path = repo().join(rel);
    if rel.ends_with(".cbor") {
        from_cbor_seq(&std::fs::read(&path).unwrap()).unwrap().0
    } else {
        parse_surface(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{rel}: {e}"))
            .records
    }
}

/// F1–F13, F10 from the wire, and F14's unknown codes.
fn corpus() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(repo().join("fixtures/corpus"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".smy"))
        .map(|n| format!("fixtures/corpus/{n}"))
        .collect();
    out.push("fixtures/wire/F10-lifecycle.cbor".to_string());
    out.push("fixtures/wire/F14-unknown-codes.cbor".to_string());
    out.sort();
    out
}

fn unit(gist: &str) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .unwrap()
}

fn all(store: &Store, opts: &ExportOptions) -> Collection {
    export_store(store, &Selection::all(store), opts).unwrap()
}

/// A fresh directory under the test target's scratch space.
fn scratch(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&d);
    d
}

/// Every record written, by hash, with how many files write it. Generated views are left out,
/// since each file has its own.
fn writes(c: &Collection) -> BTreeMap<[u8; 32], usize> {
    let mut n = BTreeMap::new();
    for f in &c.files {
        for r in from_cbor_seq(&f.bytes).unwrap().0 {
            if matches!(&r, Record::View(v) if v.id.as_str().starts_with("export/")) {
                continue;
            }
            *n.entry(Store::record_hash(&r)).or_default() += 1;
        }
    }
    n
}

// --- The exit ------------------------------------------------------------------------------

#[test]
fn verify_holds_on_every_fixture_layout_and_closure() {
    for rel in corpus() {
        let store = Store::from_records(fixture(&rel));
        for with in [With::new(), With::new().views(true).packs(true)] {
            for layout in [Layout::Single, Layout::ByView] {
                let full = all(&store, &ExportOptions::new().layout(layout).with(with));
                let refd = all(
                    &store,
                    &ExportOptions::new()
                        .layout(layout)
                        .with(with)
                        .closure(Closure::Ref),
                );
                for (c, mode) in [(&full, "full"), (&refd, "ref")] {
                    let at = format!("{rel} {layout:?} {mode} {with:?}");
                    let r = verify_collection(c);
                    assert_eq!(r.p1, Property::Holds, "{at}: P1");
                    assert_eq!(r.p3, Property::Holds, "{at}: P3");
                    assert!(r.files.iter().all(|f| f.problem.is_none()), "{at}");
                    assert_eq!(r.exit_code(), ExitCode::Success, "{at}");
                }
                // `ref` writes what `full` writes, once each, and the factor is the same.
                assert_eq!(full.rsd(), refd.rsd(), "{rel} {layout:?}");
                assert_eq!(full.duplication(), refd.duplication(), "{rel} {layout:?}");
                assert!(writes(&refd).values().all(|n| *n == 1), "{rel} {layout:?}");
                let w = writes(&full);
                assert_eq!(
                    full.duplication().0,
                    w.values().sum::<usize>() as u64 - unattributed(&full)
                );
                assert_eq!(full.duplication().1, w.len() as u64 - unattributed(&full));
            }
        }
    }
}

/// Records in `unattributed.cbor`, which no key selects and the factor does not count.
fn unattributed(c: &Collection) -> u64 {
    c.file("unattributed.cbor").map_or(0, |f| f.records as u64)
}

#[test]
fn a_flipped_byte_gives_exit_9() {
    let store = Store::from_records(fixture("fixtures/corpus/F13-canon.smy"));
    for closure in [Closure::Full, Closure::Ref] {
        let c = all(&store, &ExportOptions::new().closure(closure));
        let dir = scratch(&format!("flip-{}", closure.as_str()));
        write_collection(&c, &dir).unwrap();
        let index = dir.join(INDEX_FILE);
        assert!(verify(&index).unwrap().holds(), "{closure:?}: as written");

        let file = dir.join(&c.files[0].path);
        let mut bytes = std::fs::read(&file).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0x01;
        std::fs::write(&file, &bytes).unwrap();
        let r = verify(&index).unwrap();
        assert_eq!(r.exit_code(), ExitCode::HashVerification, "{closure:?}");
        assert_eq!(r.exit_code() as i32, 9);
        assert!(r.files[0].problem.is_some());
        assert!(r.report.iter().any(|d| d.code == Code::E494));
        assert!(matches!(r.p1, Property::NotApplicable(_)));

        std::fs::remove_file(&file).unwrap();
        let r = verify(&index).unwrap();
        assert_eq!(
            r.exit_code(),
            ExitCode::HashVerification,
            "{closure:?}: absent"
        );
    }
}

// --- The index (§4.2, §5.1) -----------------------------------------------------------------

#[test]
fn the_index_decodes_to_itself_and_refuses_a_tampered_byte() {
    for rel in corpus() {
        let store = Store::from_records(fixture(&rel));
        for closure in [Closure::Full, Closure::Ref] {
            let c = all(
                &store,
                &ExportOptions::new().layout(Layout::ByView).closure(closure),
            );
            let b = c.index.to_cbor();
            let back = ExportIndex::from_cbor(&b).unwrap_or_else(|e| panic!("{rel}: {e}"));
            assert_eq!(back, c.index, "{rel}");
            assert_eq!(back.to_cbor(), b, "{rel}");
            for i in 0..b.len() {
                let mut t = b.clone();
                t[i] ^= 0x01;
                if let Ok(x) = ExportIndex::from_cbor(&t) {
                    // A flipped bit in a digest or a count is still an index; it must then be
                    // a different one, which verify will hold against the files.
                    assert_ne!(x, c.index, "{rel}: byte {i}");
                }
            }
        }
    }
}

#[test]
fn a_tampered_index_fails_verify() {
    let store = Store::from_records(fixture("fixtures/corpus/F13-canon.smy"));
    let c = all(&store, &ExportOptions::new());
    let dir = scratch("tampered-index");
    write_collection(&c, &dir).unwrap();
    let index = dir.join(INDEX_FILE);
    let mut b = std::fs::read(&index).unwrap();
    b.truncate(b.len() - 1);
    std::fs::write(&index, &b).unwrap();
    assert!(verify(&index).is_err(), "a cut-short index does not decode");
}

#[test]
fn the_index_records_what_the_export_was() {
    let store = Store::from_records(fixture("fixtures/corpus/F11-extension-kinds.smy"));
    let c = all(
        &store,
        &ExportOptions::new()
            .closure(Closure::Ref)
            .with(With::new().views(true))
            .edges(smysl_graph::EdgeSet::support()),
    );
    let x = &c.index;
    assert_eq!(
        (x.layout.as_str(), x.closure.as_str(), x.format.as_str()),
        ("single", "ref", "cbor")
    );
    assert_eq!(x.flags, ["edges=deps,grounds", "with-views"]);
    assert_eq!(x.schemas.len(), 1, "F11's declaration");
    assert_eq!(x.counts.len(), 8);
    assert!(x.complete);
    assert_eq!(x.tool, smysl_lib::TOOL);
    assert_eq!(x.files.len(), c.files.len());
    for (f, i) in c.files.iter().zip(&x.files) {
        assert_eq!(i.blake3, *blake3::hash(&f.bytes).as_bytes());
        assert_eq!((i.rsd, i.records), (f.rsd, f.records as u64));
    }
}

// --- `ref` (§3.7) ----------------------------------------------------------------------------

/// `b` grounds on `a`; `v/alpha` keys `a` and `v/beta` keys `b`. Under `full` the beta file
/// holds `a` too. Under `ref` the alpha file writes it, and the beta file requires the alpha file.
#[test]
fn ref_writes_each_record_once_and_names_where_the_rest_is() {
    let a = unit("a");
    let ua = canonical_uid(&a);
    let b = UnitCoreBuilder::new(KernelType::Claim, "b", Status::Inferred)
        .grounds([ua])
        .build()
        .unwrap();
    let ub = canonical_uid(&b);
    let store = Store::from_records(vec![
        Record::Unit(a.clone()),
        Record::Unit(b),
        Record::View(View::new(ViewId::new("v/alpha").unwrap(), "x").with_roots([ua])),
        Record::View(View::new(ViewId::new("v/beta").unwrap(), "x").with_roots([ub])),
    ]);
    let opts = ExportOptions::new().layout(Layout::ByView);
    let full = all(&store, &opts);
    let refd = all(&store, &opts.clone().closure(Closure::Ref));

    let has_a = |c: &Collection, p: &str| {
        from_cbor_seq(&c.file(p).unwrap().bytes)
            .unwrap()
            .0
            .contains(&Record::Unit(a.clone()))
    };
    assert!(has_a(&full, "v-alpha.cbor") && has_a(&full, "v-beta.cbor"));
    assert!(has_a(&refd, "v-alpha.cbor") && !has_a(&refd, "v-beta.cbor"));
    assert_eq!(refd.file("v-beta.cbor").unwrap().requires, ["v-alpha.cbor"]);
    assert!(refd.file("v-alpha.cbor").unwrap().requires.is_empty());
    assert!(full.files.iter().all(|f| f.requires.is_empty()));
    assert_eq!(
        refd.index.file("v-beta.cbor").unwrap().requires,
        ["v-alpha.cbor"],
        "in the index"
    );
    // alpha: a and its view; beta: b, a and its view. Five selected, four distinct.
    assert_eq!(full.duplication(), (5, 4));
    assert_eq!(refd.duplication(), (5, 4));
    assert!(verify_collection(&refd).holds());

    // A `ref` file is not closed alone: verifying it on its own loses P1's records.
    let alone: BTreeSet<&str> = ["v-beta.cbor"].into();
    let files: BTreeMap<String, Vec<u8>> = refd
        .files
        .iter()
        .filter(|f| alone.contains(f.path.as_str()))
        .map(|f| (f.path.clone(), f.bytes.clone()))
        .collect();
    assert_eq!(
        verify_files(&refd.index, &files).exit_code(),
        ExitCode::HashVerification
    );
}

/// P3 compares bytes, so an index that misstates how the collection was cut fails it, even
/// when every file matches its own entry.
#[test]
fn p3_fails_when_the_index_misstates_the_closure() {
    let a = unit("a");
    let ua = canonical_uid(&a);
    let b = UnitCoreBuilder::new(KernelType::Claim, "b", Status::Inferred)
        .grounds([ua])
        .build()
        .unwrap();
    let ub = canonical_uid(&b);
    let store = Store::from_records(vec![
        Record::Unit(a),
        Record::Unit(b),
        Record::View(View::new(ViewId::new("v/alpha").unwrap(), "x").with_roots([ua])),
        Record::View(View::new(ViewId::new("v/beta").unwrap(), "x").with_roots([ub])),
    ]);
    let c = all(&store, &ExportOptions::new().layout(Layout::ByView));
    let files: BTreeMap<String, Vec<u8>> = c
        .files
        .iter()
        .map(|f| (f.path.clone(), f.bytes.clone()))
        .collect();
    let mut lie = c.index.clone();
    lie.closure = "ref".to_string();
    let r = verify_files(&lie, &files);
    assert_eq!(r.p1, Property::Holds);
    assert!(
        matches!(
            r.p3,
            Property::Fails {
                first_diff: Some(_),
                ..
            }
        ),
        "{:?}",
        r.p3
    );
    assert_eq!(r.exit_code(), ExitCode::HashVerification);
}

/// The view a `by-view` file is keyed by travels with it, `--with-views` or not: without it a
/// recipient could not cut the collection the same way, and P3 would fail.
#[test]
fn by_view_carries_the_view_it_is_keyed_by() {
    let a = unit("a");
    let ua = canonical_uid(&a);
    let v = View::new(ViewId::new("v/alpha").unwrap(), "x").with_roots([ua]);
    let store = Store::from_records(vec![Record::Unit(a), Record::View(v.clone())]);
    let c = all(&store, &ExportOptions::new().layout(Layout::ByView));
    let got = from_cbor_seq(&c.file("v-alpha.cbor").unwrap().bytes)
        .unwrap()
        .0;
    assert!(got.contains(&Record::View(v)));
    assert!(verify_collection(&c).p3.holds());
}
