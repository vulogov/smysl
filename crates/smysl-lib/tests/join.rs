//! Span re-verification and joins, of TX-P11 step 9 (SMYSL-2.7 §3.2 step 4, §3.5).
//!
//! The step's exit: an imported unit with an out-of-range span is quarantined (`SMY-E404`);
//! same-span units from two imports gain one same-as edge; P1 is unaffected, with `--no-join`
//! and with joins. The text is Ruth in the KJV, added twice under two aliases (`kjv/08-rut` and
//! `reprint/08-rut`), so its parts are one set of tids under two manifests: the reprint case
//! `identical-span` exists for.

use std::collections::BTreeSet;
use std::path::PathBuf;

use smysl_core::schemas::text;
use smysl_core::{
    canonical_uid, Carry, Code, KernelType, LangTag, Mid, Record, RelKind, SchemaId, SourceKind,
    SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};
use smysl_lib::{
    accept, export_library, import, retained, snapshot, status, write_collection, Closure,
    Collection, ExportOptions, ImportOptions, ImportReport, Input, Selection, With, INDEX_FILE,
};
use smysl_text::limits::Caps;
use smysl_text::readers::Input as Bytes;
use smysl_text::{AddSpec, Library};

mod common;

use common::{attest, repo, BASE};

fn dir(name: &str) -> PathBuf {
    common::dir("join", name)
}

/// Ruth twice, and one unit from the first part of each copy with one gist and one span.
fn reprint(name: &str) -> (Library, Uid, Uid, Mid) {
    let mut lib = Library::create(&dir(name)).unwrap();
    let bytes =
        std::fs::read(repo().join("fixtures/library/bibles/slice/eng-kjv/08-RUT.usfm")).unwrap();
    let mut mids = Vec::new();
    for alias in ["kjv/08-rut", "reprint/08-rut"] {
        let spec = AddSpec::new("usfm/1", alias, "public-domain")
            .with_lang(LangTag::new("en").unwrap())
            .with_carry(Carry::Text);
        mids.push(
            lib.add(&Bytes::new(&bytes), &spec, &Caps::DEFAULT)
                .unwrap()
                .mid,
        );
    }
    let mut records = Vec::new();
    let mut uids = Vec::new();
    for mid in &mids {
        let e = lib.store().manifest(mid).unwrap().parts[0].clone();
        let core = UnitCoreBuilder::new(
            KernelType::Claim,
            "Naomi returns to Bethlehem",
            Status::Speculative,
        )
        .lang(LangTag::new("en").unwrap())
        .source(
            SourceRef::new(SourceKind::Doc, e.tid.canonical())
                .in_part(0, 64, *mid)
                .unwrap(),
        )
        .build()
        .unwrap();
        let uid = canonical_uid(&core);
        records.push(Record::Unit(core));
        records.push(attest(uid, BASE));
        uids.push(uid);
    }
    lib.append_records(&records).unwrap();
    (lib, uids[0], uids[1], mids[0])
}

/// One unit's collection, with its text.
fn collection(lib: &Library, u: Uid) -> Collection {
    let snap = snapshot(lib, &With::new()).unwrap();
    export_library(
        lib,
        &snap,
        &Selection::of([u]),
        &ExportOptions::new()
            .closure(Closure::Full)
            .with(With::new().text(true)),
    )
    .unwrap()
}

fn import_collection(
    to: &mut Library,
    c: &Collection,
    name: &str,
    opts: ImportOptions,
) -> ImportReport {
    let out = dir(&format!(
        "coll-{name}-{}",
        to.root().display().to_string().len()
    ));
    write_collection(c, &out).unwrap();
    import(
        to,
        &[Input::Index(out.join(INDEX_FILE))],
        &opts.name(name).accept(true),
    )
    .unwrap()
}

fn same_as(lib: &Library) -> Vec<(Uid, Uid)> {
    lib.store()
        .relations()
        .filter(|r| matches!(&r.kind, RelKind::Extension(n) if n == text::SAME_AS))
        .map(|r| (r.from, r.to))
        .collect()
}

#[test]
fn same_span_units_from_two_imports_gain_one_same_as_edge() {
    let (src, a, b, _) = reprint("edge-src");
    let (ca, cb) = (collection(&src, a), collection(&src, b));
    for (order, first, second) in [("ab", &ca, &cb), ("ba", &cb, &ca)] {
        let mut to = Library::create(&dir(&format!("edge-to-{order}"))).unwrap();
        let r1 = import_collection(&mut to, first, "one", ImportOptions::new());
        assert_eq!(r1.joins, 0, "{order}: nothing to join to yet");
        let r2 = import_collection(&mut to, second, "two", ImportOptions::new());
        assert_eq!(r2.joins, 1, "{order}: one edge");
        // Staged under its own batch, and merged by `lib accept`.
        let batches = status(to.root(), Some("join:two")).unwrap();
        // The edge, its attestation, and the `x.text/v1` declaration the library did not hold.
        assert_eq!(batches[0].staged, 3, "{order}");
        assert!(same_as(&to).is_empty());
        accept(&mut to, "join:two", false).unwrap();
        let edges = same_as(&to);
        let (lo, hi) = if a < b { (a, b) } else { (b, a) };
        assert_eq!(edges, vec![(lo, hi)], "{order}");
        // P1 is unaffected: what each import retained is its collection, joins excluded.
        assert_eq!(
            smysl_graph::record_set_digest(&retained(&to, "two").unwrap()),
            second.rsd()
        );
        assert_eq!(
            smysl_graph::record_set_digest(&retained(&to, "one").unwrap()),
            first.rsd()
        );
    }
}

#[test]
fn no_join_proposes_nothing_and_p1_is_the_same() {
    let (src, a, b, _) = reprint("nojoin-src");
    let (ca, cb) = (collection(&src, a), collection(&src, b));
    let mut to = Library::create(&dir("nojoin-to")).unwrap();
    import_collection(&mut to, &ca, "one", ImportOptions::new().join(false));
    let r = import_collection(&mut to, &cb, "two", ImportOptions::new().join(false));
    assert_eq!(r.joins, 0);
    assert!(status(to.root(), Some("join:two")).is_err());
    assert_eq!(
        smysl_graph::record_set_digest(&retained(&to, "two").unwrap()),
        cb.rsd()
    );
}

#[test]
fn an_imported_unit_with_an_out_of_range_span_is_quarantined() {
    let (src, a, _, _) = reprint("span-src");
    let mut to = Library::create(&dir("span-to")).unwrap();
    import_collection(&mut to, &collection(&src, a), "ruth", ImportOptions::new());
    // A unit naming the part the library now holds, past its end.
    let (mid, e) = to
        .store()
        .manifests()
        .find_map(|(m, x)| x.parts.first().map(|e| (*m, e.clone())))
        .unwrap();
    let bad = UnitCoreBuilder::new(KernelType::Claim, "past the end", Status::Speculative)
        .source(
            SourceRef::new(SourceKind::Doc, e.tid.canonical())
                .in_part(0, e.length + 1000, mid)
                .unwrap(),
        )
        .build()
        .unwrap();
    let uid = canonical_uid(&bad);
    let mut bytes = smysl_core::to_cbor(&Record::Unit(bad));
    bytes.extend(smysl_core::to_cbor(&attest(uid, BASE)));
    let r = import(
        &mut to,
        &[Input::Bytes {
            name: "bad-span.cbor".into(),
            bytes,
        }],
        &ImportOptions::new().accept(true),
    )
    .unwrap();
    assert_eq!(r.files[0].quarantined, 2, "the unit and its attestation");
    assert!(
        r.report
            .iter()
            .any(|d| d.code == Code::W423 && d.message.contains("SMY-E404")),
        "{:?}",
        r.report
    );
    assert!(!to.store().contains_uid(&uid));

    // The same unit in the same file as its part and manifest is checked against that part.
    let (fresh_src, a2, _, _) = reprint("span-src2");
    let c = collection(&fresh_src, a2);
    let mut records = c.records();
    let m = records
        .iter()
        .find_map(|r| match r {
            Record::Manifest(m) => Some((m.mid(), m.parts[0].clone())),
            _ => None,
        })
        .unwrap();
    let past = UnitCoreBuilder::new(KernelType::Claim, "also past the end", Status::Speculative)
        .source(
            SourceRef::new(SourceKind::Doc, m.1.tid.canonical())
                .in_part(0, m.1.length + 1, m.0)
                .unwrap(),
        )
        .build()
        .unwrap();
    let past_uid = canonical_uid(&past);
    records.push(Record::Unit(past));
    let bytes: Vec<u8> = records.iter().flat_map(smysl_core::to_cbor).collect();
    let mut fresh = Library::create(&dir("span-fresh")).unwrap();
    let r = import(
        &mut fresh,
        &[Input::Bytes {
            name: "with-text.cbor".into(),
            bytes,
        }],
        &ImportOptions::new().accept(true),
    )
    .unwrap();
    assert!(
        r.report.iter().any(|d| d.message.contains("SMY-E404")),
        "{:?}",
        r.report
    );
    assert!(!fresh.store().contains_uid(&past_uid));
    assert!(
        fresh.store().contains_uid(&a2),
        "the rest of the file is accepted"
    );
}

fn entity(name: &str, expression: &str) -> UnitCore {
    UnitCoreBuilder::new(
        SchemaId::Extension(text::ENTITY.into()),
        name,
        Status::Speculative,
    )
    .lang(LangTag::new("en").unwrap())
    .source(SourceRef::new(
        SourceKind::Doc,
        format!("alias:{expression}"),
    ))
    .build()
    .unwrap()
}

#[test]
fn one_name_in_one_language_from_two_expressions_is_joined() {
    let naomi_kjv = entity("Naomi", "kjv/08-rut");
    let naomi_reprint = entity("Naomi", "reprint/08-rut");
    let naomi_fr = UnitCoreBuilder::new(
        SchemaId::Extension(text::ENTITY.into()),
        "Naomi",
        Status::Speculative,
    )
    .lang(LangTag::new("fr").unwrap())
    .source(SourceRef::new(SourceKind::Doc, "alias:fralsg/08-rut"))
    .build()
    .unwrap();
    let ids: Vec<Uid> = [&naomi_kjv, &naomi_reprint, &naomi_fr]
        .iter()
        .map(|c| canonical_uid(c))
        .collect();
    let file = |c: &UnitCore, at: u64| -> Vec<u8> {
        let u = canonical_uid(c);
        [Record::Unit(c.clone()), attest(u, at)]
            .iter()
            .flat_map(smysl_core::to_cbor)
            .collect()
    };
    let mut lib = Library::create(&dir("entities")).unwrap();
    for (i, (c, name)) in [
        (&naomi_kjv, "kjv"),
        (&naomi_reprint, "reprint"),
        (&naomi_fr, "fr"),
    ]
    .iter()
    .enumerate()
    {
        let r = import(
            &mut lib,
            &[Input::Bytes {
                name: format!("{name}.cbor"),
                bytes: file(c, BASE + i as u64),
            }],
            &ImportOptions::new().accept(true),
        )
        .unwrap();
        // The first has nothing to join; the reprint joins the KJV; the French is another
        // language and joins nothing.
        assert_eq!(r.joins, usize::from(i == 1), "{name}");
        if r.joins > 0 {
            accept(&mut lib, &format!("join:{}", r.name), false).unwrap();
        }
    }
    let edges: BTreeSet<Uid> = same_as(&lib)
        .into_iter()
        .flat_map(|(a, b)| [a, b])
        .collect();
    assert_eq!(edges, [ids[0], ids[1]].into_iter().collect());
}
