//! Import, staging, waiting, quarantine and the tables of TX-P11 step 4 (SMYSL-2.7 §3.1–§3.3,
//! §5.1).
//!
//! The step's exit: idempotence and commutativity property tests, F6's quarantine, and an
//! early attestation promoted when its unit arrives. Beside it, §5.1's `classify.rs` and
//! `manifest.rs` rows, and P1 through a real import, replacing step 3's stand-in.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::surface::parse_surface;
use smysl_core::{
    canonical_uid, from_cbor_seq, to_cbor, AgentId, Attestation, Code, Hlc, KernelType, Op, Record,
    Rung, Status, UnitCoreBuilder,
};
use smysl_graph::{record_set_digest, Store};
use smysl_lib::{
    accept, discard, export_store, import, retained, status, write_collection, Closure,
    ExportOptions, ImportOptions, Input, Layout, LibraryRead, Selection, State, With, INDEX_FILE,
};
use smysl_text::Library;

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

/// An empty library in a fresh directory.
fn library(name: &str) -> Library {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("import")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    Library::create(&d).unwrap()
}

fn cbor(records: &[Record]) -> Vec<u8> {
    records.iter().flat_map(to_cbor).collect()
}

fn bytes(name: &str, records: &[Record]) -> Input {
    Input::Bytes {
        name: name.to_string(),
        bytes: cbor(records),
    }
}

fn accepting(name: &str) -> ImportOptions {
    ImportOptions::new().name(name).accept(true)
}

/// The library's records less its manifests: what imports put there of what they read.
fn content(lib: &Library) -> [u8; 32] {
    record_set_digest(
        lib.snapshot()
            .iter()
            .filter(|r| !matches!(r, Record::Manifest(_))),
    )
}

/// A small seeded generator, so the property tests are the same on every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// `records` cut into `k` files at random, each record in exactly one.
fn split(records: &[Record], k: usize, rng: &mut Rng) -> Vec<Vec<Record>> {
    let mut out = vec![Vec::new(); k];
    for r in records {
        out[(rng.next() % k as u64) as usize].push(r.clone());
    }
    out
}

/// What the corpus imports, without records 15 and 18 (step 6's) and checkpoints.
fn importable(rel: &str) -> Vec<Record> {
    fixture(rel)
        .into_iter()
        .filter(|r| !matches!(r, Record::PartText(_) | Record::PartReading(_)))
        .collect()
}

// --- The exit: idempotence and commutativity --------------------------------------------------

/// Every fixture, cut into three files at random, imported in all six orders: the library
/// ends the same, and so do its waiting and quarantined sets.
#[test]
fn imports_commute() {
    let mut rng = Rng(0x5eed_0001);
    for rel in corpus() {
        let records = importable(&rel);
        for round in 0..2 {
            let parts = split(&records, 3, &mut rng);
            let mut seen: Option<([u8; 32], u64, u64)> = None;
            for order in [
                [0, 1, 2],
                [0, 2, 1],
                [1, 0, 2],
                [1, 2, 0],
                [2, 0, 1],
                [2, 1, 0],
            ] {
                let mut lib = library(&format!("commute-{round}"));
                for i in order {
                    import(
                        &mut lib,
                        &[bytes(&format!("p{i}.cbor"), &parts[i])],
                        &accepting(&format!("p{i}")),
                    )
                    .unwrap();
                }
                let batches = status(lib.root(), None).unwrap();
                assert!(batches.iter().all(|b| b.state == State::Accepted));
                let got = (
                    content(&lib),
                    batches.iter().map(|b| b.waiting).sum(),
                    batches.iter().map(|b| b.quarantined).sum(),
                );
                match &seen {
                    None => seen = Some(got),
                    Some(s) => assert_eq!(*s, got, "{rel} round {round}: order {order:?}"),
                }
            }
        }
    }
}

/// The same files again, under the same names and options, change nothing.
#[test]
fn imports_are_idempotent() {
    let mut rng = Rng(0x5eed_0002);
    for rel in corpus() {
        let parts = split(&importable(&rel), 2, &mut rng);
        let mut lib = library("idempotent");
        for (i, p) in parts.iter().enumerate() {
            import(
                &mut lib,
                &[bytes(&format!("p{i}.cbor"), p)],
                &accepting(&format!("p{i}")),
            )
            .unwrap();
        }
        let once = (
            lib.snapshot().record_set_digest(),
            status(lib.root(), None).unwrap(),
        );
        for (i, p) in parts.iter().enumerate() {
            let r = import(
                &mut lib,
                &[bytes(&format!("p{i}.cbor"), p)],
                &accepting(&format!("p{i}")),
            )
            .unwrap();
            assert!(r.files.iter().all(|f| f.already_imported), "{rel}");
            assert!(r.manifests.is_empty(), "{rel}");
        }
        let twice = (
            lib.snapshot().record_set_digest(),
            status(lib.root(), None).unwrap(),
        );
        assert_eq!(once, twice, "{rel}");
    }
}

/// Imported whole or in pieces, a fixture's library is the same: what waits for a later
/// piece is promoted when it arrives.
#[test]
fn pieces_promote_to_the_whole() {
    let mut rng = Rng(0x5eed_0003);
    for rel in corpus() {
        let records = importable(&rel);
        let mut whole = library("whole");
        import(
            &mut whole,
            &[bytes("all.cbor", &records)],
            &accepting("all"),
        )
        .unwrap();
        let mut pieces = library("pieces");
        for (i, p) in split(&records, 4, &mut rng).iter().enumerate() {
            import(
                &mut pieces,
                &[bytes(&format!("p{i}.cbor"), p)],
                &accepting(&format!("p{i}")),
            )
            .unwrap();
        }
        assert_eq!(content(&whole), content(&pieces), "{rel}");
    }
}

// --- F6 and the quarantine (§3.3) ------------------------------------------------------------

/// The two claims that break rule M are quarantined. The finding breaks it too, and rests on
/// them, so it waits for them, and so does the view rooted on it. Nothing else is touched.
#[test]
fn f6_quarantines_the_rule_m_units_and_their_dependents_wait() {
    let records = importable("fixtures/corpus/F6-adversarial.smy");
    let whole = Store::from_records(records.clone());
    let e030: BTreeSet<_> = smysl_check::check(&whole, smysl_check::CheckOptions::default())
        .iter()
        .filter(|d| d.code == Code::E030)
        .filter_map(|d| match d.subject {
            smysl_core::Subject::Unit(u) => Some(u),
            _ => None,
        })
        .collect();
    let mut lib = library("f6");
    let r = import(
        &mut lib,
        &[Input::File(
            repo().join("fixtures/corpus/F6-adversarial.smy"),
        )],
        &accepting("f6"),
    )
    .unwrap();
    let f = &r.files[0];
    let unit = |gist: &str| {
        records
            .iter()
            .find_map(|r| match r {
                Record::Unit(c) if c.gist.starts_with(gist) => Some(canonical_uid(c)),
                _ => None,
            })
            .unwrap()
    };
    let once = unit("Pool saturation");
    let twice = unit("The eu-west shard");
    let root = unit("The eu-west capacity");
    let quarantined: BTreeSet<_> = std::fs::read_dir(lib.root().join("lib/quarantine/f6"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "cbor"))
        .flat_map(|p| from_cbor_seq(&std::fs::read(p).unwrap()).unwrap().0)
        .filter_map(|r| r.as_unit().map(canonical_uid))
        .collect();
    assert_eq!(quarantined, BTreeSet::from([once, twice]));
    assert!(
        quarantined.is_subset(&e030),
        "only units the check finds E030 on"
    );
    // The finding breaks rule M too, and rests on both: it waits for them rather than being
    // quarantined beside them, as it would have had it arrived first.
    assert!(e030.contains(&root));
    assert!(!lib.snapshot().contains_uid(&root), "the finding waits");
    assert!(lib.snapshot().contains_uid(&unit("The connection pool")));
    assert!(
        f.waiting >= 2,
        "the finding and the view rooted on it: {f:?}"
    );
    assert!(r.report.iter().any(|d| d.code == Code::W423));
    assert!(r.report.iter().any(|d| d.code == Code::W496));
    let report =
        std::fs::read_to_string(lib.root().join("lib/quarantine/f6/f6-adversarial.report"))
            .unwrap();
    assert!(report.contains("SMY-E030"), "{report}");
    // Retained is everything the file held, whatever became of it.
    assert_eq!(
        record_set_digest(&retained(&lib, "f6").unwrap()),
        record_set_digest(&records)
    );
}

// --- Waiting and promotion -------------------------------------------------------------------

fn agent() -> AgentId {
    AgentId::new("tool:test").unwrap()
}

/// An attestation that arrives before its unit waits, in the table and in `lib/waiting/`, and is
/// appended when a second import brings the unit.
#[test]
fn an_early_attestation_waits_then_is_promoted() {
    let u = UnitCoreBuilder::new(KernelType::Claim, "late", Status::Speculative)
        .build()
        .unwrap();
    let uid = canonical_uid(&u);
    let a = Record::Attestation(Attestation::new(
        uid,
        agent(),
        Op::Authored,
        Rung::Document,
        Hlc::new(1, 0, agent()),
    ));
    let mut lib = library("early");
    let r = import(
        &mut lib,
        &[bytes("a.cbor", std::slice::from_ref(&a))],
        &accepting("first"),
    )
    .unwrap();
    assert_eq!(r.files[0].waiting, 1);
    assert!(!lib.snapshot().contains_record(&Store::record_hash(&a)));
    assert!(lib.root().join("lib/waiting/first.cbor").is_file());
    assert_eq!(status(lib.root(), Some("first")).unwrap()[0].waiting, 1);

    import(
        &mut lib,
        &[bytes("u.cbor", &[Record::Unit(u)])],
        &accepting("second"),
    )
    .unwrap();
    assert!(
        lib.snapshot().contains_record(&Store::record_hash(&a)),
        "promoted"
    );
    let first = &status(lib.root(), Some("first")).unwrap()[0];
    assert_eq!((first.waiting, first.accepted), (0, 1));
    assert!(!lib.root().join("lib/waiting/first.cbor").exists());
}

// --- Staging (§3.3) --------------------------------------------------------------------------

#[test]
fn a_staged_import_is_accepted_or_discarded() {
    let records = importable("fixtures/corpus/F1-incident.smy");
    let mut direct = library("direct");
    import(&mut direct, &[bytes("f1.cbor", &records)], &accepting("f1")).unwrap();

    let mut lib = library("staged");
    let r = import(
        &mut lib,
        &[bytes("f1.cbor", &records)],
        &ImportOptions::new().name("f1"),
    )
    .unwrap();
    assert!(r.files[0].staged > 0);
    assert!(
        lib.snapshot()
            .iter()
            .all(|r| matches!(r, Record::Manifest(_))),
        "only the manifest"
    );
    assert_eq!(
        status(lib.root(), Some("f1")).unwrap()[0].state,
        State::Staged
    );
    accept(&mut lib, "f1", false).unwrap();
    assert_eq!(content(&lib), content(&direct));
    assert_eq!(
        lib.snapshot().record_set_digest(),
        direct.snapshot().record_set_digest()
    );
    assert!(
        accept(&mut lib, "f1", false).is_err(),
        "nothing left to accept"
    );

    let mut gone = library("discarded");
    import(
        &mut gone,
        &[bytes("f1.cbor", &records)],
        &ImportOptions::new().name("f1"),
    )
    .unwrap();
    assert!(discard(&mut gone, "f1").unwrap() > 0);
    assert_eq!(
        status(gone.root(), Some("f1")).unwrap()[0].state,
        State::Discarded
    );
    assert!(!gone.root().join("lib/staged/f1.cbor").exists());
    assert!(retained(&gone, "f1").unwrap().is_empty());
}

// --- Decoding and manifests (§3.1, §5.1) -----------------------------------------------------

#[test]
fn truncated_cbor_is_e493_and_imports_nothing() {
    let records = importable("fixtures/corpus/F13-canon.smy");
    let mut b = cbor(&records);
    b.truncate(b.len() - 3);
    let mut lib = library("truncated");
    let r = import(
        &mut lib,
        &[Input::Bytes {
            name: "cut.cbor".into(),
            bytes: b,
        }],
        &accepting("cut"),
    )
    .unwrap();
    assert!(r.files[0].rejected);
    assert!(r.report.iter().any(|d| d.code == Code::E493));
    assert_eq!(lib.snapshot().iter().count(), 0, "not even a manifest");
}

#[test]
fn one_file_one_mid_until_the_options_change() {
    let records = importable("fixtures/corpus/F2-research.smy");
    let mut lib = library("mids");
    let first = import(&mut lib, &[bytes("f2.cbor", &records)], &accepting("f2")).unwrap();
    let again = import(&mut lib, &[bytes("f2.cbor", &records)], &accepting("f2")).unwrap();
    assert_eq!(first.files[0].mid, again.files[0].mid);
    assert!(again.files[0].already_imported);
    assert_eq!(again.files[0].added, 0);

    let other = import(
        &mut lib,
        &[bytes("f2.cbor", &records)],
        &accepting("f2-cc0").licence("CC0-1.0"),
    )
    .unwrap();
    assert_ne!(
        other.files[0].mid, first.files[0].mid,
        "a new licence is a new manifest"
    );
    assert_eq!(other.files[0].added, 0, "and no new content");
    assert_eq!(
        other.files[0].duplicates,
        records
            .iter()
            .map(Store::record_hash)
            .collect::<BTreeSet<_>>()
            .len()
    );

    // The manifest names the file, never its directory.
    let m = lib
        .snapshot()
        .manifest(&first.files[0].mid.unwrap())
        .unwrap()
        .clone();
    assert_eq!(m.alias, "import:f2");
    assert_eq!(m.origin.unwrap().reference, "f2.cbor");
}

// --- P1 through a real import (§3.9) ---------------------------------------------------------

/// Every fixture exported, written, and imported whole into an empty library: what the import
/// retained is what the index says was exported, under every layout and closure.
#[test]
fn p1_holds_through_import() {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("import")
        .join("p1");
    for rel in corpus() {
        let store = Store::from_records(fixture(&rel));
        for layout in [Layout::Single, Layout::ByView] {
            for closure in [Closure::Full, Closure::Ref] {
                let opts = ExportOptions::new()
                    .layout(layout)
                    .closure(closure)
                    .with(With::new().views(true).packs(true));
                let c = export_store(&store, &Selection::all(&store), &opts).unwrap();
                let dir = d.join("collection");
                let _ = std::fs::remove_dir_all(&dir);
                write_collection(&c, &dir).unwrap();
                let mut lib = library("p1-lib");
                let r = import(
                    &mut lib,
                    &[Input::Index(dir.join(INDEX_FILE))],
                    &accepting("x"),
                )
                .unwrap();
                assert!(r.files.iter().all(|f| !f.rejected), "{rel}");
                let got = record_set_digest(&retained(&lib, "x").unwrap());
                assert_eq!(got, c.rsd(), "{rel} {layout:?} {closure:?}: P1");
            }
        }
    }
}

/// A file that does not match its index refuses the whole collection (`SMY-E494`).
#[test]
fn a_collection_with_a_bad_file_imports_nothing() {
    let store = Store::from_records(fixture("fixtures/corpus/F13-canon.smy"));
    let c = export_store(&store, &Selection::all(&store), &ExportOptions::new()).unwrap();
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("import")
        .join("bad-collection");
    let _ = std::fs::remove_dir_all(&dir);
    write_collection(&c, &dir).unwrap();
    let f = dir.join(&c.files[0].path);
    let mut b = std::fs::read(&f).unwrap();
    let n = b.len() / 2;
    b[n] ^= 1;
    std::fs::write(&f, b).unwrap();
    let mut lib = library("bad-collection-lib");
    let r = import(
        &mut lib,
        &[Input::Index(dir.join(INDEX_FILE))],
        &accepting("x"),
    )
    .unwrap();
    assert!(r.report.iter().any(|d| d.code == Code::E494));
    assert_eq!(lib.snapshot().iter().count(), 0);
}
