//! `--scope` as a query, `by-query`, and `from import:<…>`, of TX-P11 step 8 (SMYSL-2.7 §3.6,
//! §3.9).
//!
//! The step's exit: P3 with scope `from import:<collection>` is byte-identical on all landed
//! cells. A landed cell is a corpus fixture, the five Bibles or the two chats, under every
//! layout this build has (`by-view` where there are views, `by-query` by `status`) and both
//! closure modes. Each is exported, written, imported with `--accept` into an empty library
//! under a name, and exported again from that library with scope `from import:<name>`. Every
//! file comes back byte for byte, with the index's `files` and `rsd`.

use std::path::PathBuf;

use smysl_core::{Carry, KernelType, Record};
use smysl_graph::Store;
use smysl_lib::scope::{group, import_scope, query, reexport_import};
use smysl_lib::{
    export_library, export_store, import, snapshot, verify_collection, write_collection, Closure,
    Collection, ExportOptions, ImportOptions, Input, Layout, Selection, With, INDEX_FILE,
};
use smysl_query::{Env, StoreScope};
use smysl_text::Library;

mod common;

fn dir(name: &str) -> PathBuf {
    common::dir("scope", name)
}

fn layouts() -> Vec<Layout> {
    vec![
        Layout::Single,
        Layout::ByView,
        Layout::ByExpression,
        Layout::ByCollection,
        Layout::ByTime,
        Layout::ByQuery,
    ]
}

/// Export, write, import under `name` with `--accept`, re-export from `import:<name>`, compare.
fn round_trip(cell: &str, c: &Collection) {
    let v = verify_collection(c);
    assert!(v.holds() && v.p1.holds(), "{cell}: {v:?}");
    let out = dir(&format!("{cell}-out"));
    write_collection(c, &out).unwrap();
    let mut to = Library::create(&dir(&format!("{cell}-to"))).unwrap();
    let name = "coll";
    import(
        &mut to,
        &[Input::Index(out.join(INDEX_FILE))],
        &ImportOptions::new().name(name).accept(true),
    )
    .unwrap();
    let again = reexport_import(&to, name, &c.index).unwrap();
    assert_eq!(
        again.index.files, c.index.files,
        "{cell}: the index's files"
    );
    assert_eq!(again.rsd(), c.rsd(), "{cell}: the index's rsd");
    for (a, b) in again.files.iter().zip(&c.files) {
        assert_eq!(a.path, b.path, "{cell}");
        assert!(a.bytes == b.bytes, "{cell}: {} differs", a.path);
    }
    assert_eq!(again.index.scope, format!("from import:{name}"));
}

/// The selection a layout needs: `by-query` groups by `status` over the snapshot.
fn select(store: &Store, layout: Layout) -> Selection {
    let all = Selection::all(store);
    match layout {
        Layout::ByQuery => group(store, all, "status").unwrap(),
        _ => all,
    }
}

#[test]
fn p3_from_import_is_byte_identical_on_the_corpus() {
    for rel in common::corpus() {
        let stem = rel
            .rsplit('/')
            .next()
            .unwrap()
            .split('.')
            .next()
            .unwrap()
            .to_lowercase();
        let store = Store::from_records(common::fixture(&rel));
        for layout in layouts() {
            if layout == Layout::ByView && store.views().next().is_none() {
                continue;
            }
            for closure in [Closure::Full, Closure::Ref] {
                let cell = format!("{stem}-{}-{}", layout.as_str(), closure.as_str());
                let opts = ExportOptions::new().layout(layout).closure(closure);
                let c = export_store(&store, &select(&store, layout), &opts).unwrap();
                round_trip(&cell, &c);
            }
        }
    }
}

#[test]
fn p3_from_import_is_byte_identical_on_the_bibles_and_the_chats() {
    let bibles = common::bibles(&dir("bibles"), Carry::Text);
    let (chats, _) = common::chats(&dir("chats"));
    for (name, lib, with) in [
        ("bibles", &bibles, With::new().text(true)),
        ("chats", &chats, With::new()),
    ] {
        let snap = snapshot(lib, &with).unwrap();
        for layout in layouts() {
            if layout == Layout::ByView {
                continue;
            }
            for closure in [Closure::Full, Closure::Ref] {
                let cell = format!("{name}-{}-{}", layout.as_str(), closure.as_str());
                let opts = ExportOptions::new()
                    .layout(layout)
                    .closure(closure)
                    .with(with);
                let c = export_library(lib, &snap, &select(&snap, layout), &opts).unwrap();
                round_trip(&cell, &c);
            }
        }
    }
}

#[test]
fn a_scope_query_selects_and_is_recorded() {
    let store = Store::from_records(common::fixture("fixtures/corpus/F13-canon.smy"));
    let scope = StoreScope::new("f13", &store);
    let env = Env::new(&store, &scope);
    let all = query(&env, "from store:f13").unwrap();
    assert_eq!(all.units(), Selection::all(&store).units());
    let claims = query(&env, "from store:f13\nwhere kind = claim").unwrap();
    assert!(!claims.is_empty() && claims.len() < all.len());
    for u in claims.units() {
        assert_eq!(
            store.get(u).unwrap().core.schema.kernel(),
            Some(KernelType::Claim)
        );
    }
    let c = export_store(&store, &claims, &ExportOptions::new()).unwrap();
    assert_eq!(c.index.scope, "from store:f13\nwhere kind = claim");
    assert_eq!(c.index.sq_version.as_deref(), Some("sq/1"));
    assert!(verify_collection(&c).holds());

    // A count names no units, and a scope that does not parse is refused.
    assert!(query(&env, "from store:f13\ncount").is_err());
    assert!(query(&env, "from nowhere at all").is_err());
}

#[test]
fn by_query_files_by_the_group_value() {
    let store = Store::from_records(common::fixture("fixtures/corpus/F13-canon.smy"));
    let sel = group(&store, Selection::all(&store), "kind").unwrap();
    let c = export_store(&store, &sel, &ExportOptions::new().layout(Layout::ByQuery)).unwrap();
    assert!(c.files.len() > 1, "F13 holds several kinds");
    assert!(c.index.flags.contains(&"group-by=kind".to_string()));
    for f in &c.files {
        let kind = f.path.trim_end_matches(".cbor");
        for r in smysl_core::from_cbor_seq(&f.bytes).unwrap().0 {
            if let Record::View(v) = r {
                for u in &v.roots {
                    let k = store.get(u).unwrap().core.schema.kernel().unwrap();
                    assert_eq!(k.as_str(), kind);
                }
            }
        }
    }
    let v = verify_collection(&c);
    assert!(v.holds() && v.p3.holds(), "{v:?}");
    // Without a field there is nothing to key by.
    assert!(export_store(
        &store,
        &Selection::all(&store),
        &ExportOptions::new().layout(Layout::ByQuery)
    )
    .is_err());
}

#[test]
fn from_import_resolves_to_the_export_roots_not_to_everything_that_arrived() {
    let store = Store::from_records(common::fixture("fixtures/corpus/F13-canon.smy"));
    let some: Vec<_> = store
        .units()
        .filter(|(_, u)| u.core.references().next().is_some())
        .map(|(u, _)| *u)
        .take(1)
        .collect();
    let c = export_store(&store, &Selection::of(some.clone()), &ExportOptions::new()).unwrap();
    let out = dir("roots-out");
    write_collection(&c, &out).unwrap();
    let mut to = Library::create(&dir("roots-to")).unwrap();
    import(
        &mut to,
        &[Input::Index(out.join(INDEX_FILE))],
        &ImportOptions::new().name("part").accept(true),
    )
    .unwrap();
    let sel = import_scope(&to, "part").unwrap().unwrap();
    assert_eq!(sel.units().iter().copied().collect::<Vec<_>>(), some);
    assert!(
        c.records()
            .iter()
            .filter(|r| matches!(r, Record::Unit(_)))
            .count()
            > some.len(),
        "the closure brought more units than were selected"
    );
}
