//! Time in an export, of TX-P11 step 7 (SMYSL-2.7 §3.4, §3.10).
//!
//! The step's exit: the chat cells of GE-T15 pass, and `--as-of` excludes exactly the planted
//! late records, with `SMY-W415` and `SMY-W498` counted. The chats are the WhatsApp and
//! Telegram reader fixtures, added as `chat/wa` and `chat/tg`. Each chat is one part, and six
//! units are drawn from successive spans of it, each observed six hours after the one before,
//! so a day's window holds four; and beside them
//! the records GE-T15's chat axis names: a dating (a clock correction), an edge, its withdrawal
//! and a resolution of it, a commitment, and a redaction.

use std::collections::BTreeSet;
use std::path::PathBuf;

use smysl_core::{
    canonical_uid, Code, KernelType, Record, RelKind, Relation, Status, Uid, UnitCore,
    UnitCoreBuilder,
};
use smysl_graph::Store;
use smysl_lib::{
    export_library, export_store, import, retained, snapshot, verify_collection, write_collection,
    Closure, ExportOptions, ImportOptions, Input, Layout, Selection, With, INDEX_FILE,
};
use smysl_text::Library;

mod common;

use common::{attest, BASE, HOUR};

fn dir(name: &str) -> PathBuf {
    common::dir("time", name)
}

fn chats(name: &str) -> (Library, Vec<Uid>) {
    common::chats(&dir(name))
}

fn claim(gist: &str) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .build()
        .unwrap()
}

#[test]
fn the_chat_cells_hold_p1_and_p3() {
    let (lib, uids) = chats("cells");
    assert_eq!(uids.len(), 12, "six units from each chat");
    let snap = snapshot(&lib, &With::new()).unwrap();
    let sel = Selection::all(&snap);
    for layout in [
        Layout::Single,
        Layout::ByExpression,
        Layout::ByCollection,
        Layout::ByTime,
    ] {
        for closure in [Closure::Full, Closure::Ref] {
            let cell = format!("{}/{}", layout.as_str(), closure.as_str());
            let opts = ExportOptions::new().layout(layout).closure(closure);
            let c = export_library(&lib, &snap, &sel, &opts).unwrap();
            let v = verify_collection(&c);
            assert!(v.holds() && v.p1.holds() && v.p3.holds(), "{cell}: {v:?}");
            let records = c.records();
            for t in [3, 11, 12, 13, 17, 19] {
                assert!(
                    records.iter().any(|r| r.type_code() == t),
                    "{cell}: record type {t} travels"
                );
            }

            let out = dir(&format!("out-{}-{}", layout.as_str(), closure.as_str()));
            write_collection(&c, &out).unwrap();
            let mut to = Library::create(&dir(&format!(
                "to-{}-{}",
                layout.as_str(),
                closure.as_str()
            )))
            .unwrap();
            let r = import(
                &mut to,
                &[Input::Index(out.join(INDEX_FILE))],
                &ImportOptions::new().accept(true),
            )
            .unwrap();
            let kept = retained(&to, &r.name).unwrap();
            assert_eq!(smysl_graph::record_set_digest(&kept), c.rsd(), "{cell}: P1");
        }
    }
}

#[test]
fn by_time_files_a_unit_by_its_day() {
    let (lib, uids) = chats("days");
    let snap = snapshot(&lib, &With::new()).unwrap();
    let c = export_library(
        &lib,
        &snap,
        &Selection::all(&snap),
        &ExportOptions::new().layout(Layout::ByTime),
    )
    .unwrap();
    // Four units a day, six hours apart from 2024-01-15T00:00Z.
    let days = uids.len().div_ceil(4);
    assert_eq!(
        c.files.len(),
        days,
        "{:?}",
        c.files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
    assert_eq!(c.files[0].path, "t20240115t000000z.cbor");
    assert_eq!(
        c.index.counts["SMY-W497"], 0,
        "a window key needs no suffix"
    );
    assert!(c.index.flags.contains(&"window=86400000".to_string()));

    // A six-hour window puts each unit in its own file.
    let c = export_library(
        &lib,
        &snap,
        &Selection::all(&snap),
        &ExportOptions::new().layout(Layout::ByTime).window(6 * HOUR),
    )
    .unwrap();
    assert_eq!(c.files.len(), uids.len());
    assert!(verify_collection(&c).holds());
}

#[test]
fn as_of_excludes_exactly_the_planted_late_records() {
    let (lib, uids) = chats("as-of");
    let snap = snapshot(&lib, &With::new()).unwrap();
    // The cut: the end of 2024-01-15. Every chat unit after the first four is late.
    let cut = BASE + 24 * HOUR;

    // Planted beside the chats: a unit attested after the cut; one never attested (W415); one
    // attested before the cut whose ground is the late one (W498); a late attestation of an
    // early unit; and an edge between early units attested after the cut.
    let late = claim("said on the sixteenth");
    let late_uid = canonical_uid(&late);
    let silent = claim("never attested");
    let resting = UnitCoreBuilder::new(
        KernelType::Claim,
        "rests on the late one",
        Status::Speculative,
    )
    .grounds([late_uid])
    .build()
    .unwrap();
    let resting_uid = canonical_uid(&resting);
    let early_edge = Relation::new(RelKind::Contrasts, uids[0], uids[2]);
    let planted_late: Vec<Record> = vec![
        Record::Unit(late),
        attest(late_uid, cut + HOUR),
        Record::Unit(silent),
        Record::Unit(resting),
        attest(resting_uid, BASE + 2 * HOUR),
        attest(uids[0], cut + 2 * HOUR),
        attest(early_edge.uid(), cut + 3 * HOUR),
        Record::Relation(early_edge),
    ];
    let full = Store::from_records(
        snap.iter()
            .cloned()
            .chain(planted_late.iter().cloned())
            .collect(),
    );
    let c = export_store(
        &full,
        &Selection::all(&full),
        &ExportOptions::new().as_of("2024-01-15"),
    )
    .unwrap();
    assert_eq!(c.index.as_of.as_deref(), Some("2024-01-15"));
    assert_eq!(
        c.index.counts["SMY-W415"], 1,
        "the unit no attestation dates"
    );
    assert_eq!(
        c.index.counts["SMY-W498"], 1,
        "the unit resting on a late one"
    );
    assert!(c.report.iter().any(|d| d.code == Code::W415));
    assert!(c.report.iter().any(|d| d.code == Code::W498));

    let exported: BTreeSet<[u8; 32]> = c.records().iter().map(Store::record_hash).collect();
    for r in &planted_late {
        assert!(
            !exported.contains(&Store::record_hash(r)),
            "{} was planted late",
            r.type_name()
        );
    }
    // And the chat units after the cut, with their attestations.
    let late_chat: Vec<&Uid> = uids.iter().skip(4).collect();
    for r in full.iter() {
        let named_late = match r {
            Record::Unit(c) => late_chat.contains(&&canonical_uid(c)),
            Record::Attestation(a) => late_chat.contains(&&a.uid),
            _ => false,
        };
        if named_late {
            assert!(!exported.contains(&Store::record_hash(r)));
        }
    }
    // Exactly those: the cut export is the plain export of the store without them, record
    // for record and digest for digest. What travelled only with a late unit, such as the
    // Telegram chat's manifest once all its units are late, goes with it, as it would from a
    // store that never held them.
    let mut late: BTreeSet<[u8; 32]> = planted_late.iter().map(Store::record_hash).collect();
    for r in full.iter() {
        let named_late = match r {
            Record::Unit(c) => late_chat.contains(&&canonical_uid(c)),
            Record::Attestation(a) => late_chat.contains(&&a.uid),
            _ => false,
        };
        if named_late {
            late.insert(Store::record_hash(r));
        }
    }
    let early = Store::from_records(
        full.iter()
            .filter(|r| !late.contains(&Store::record_hash(r)))
            .cloned()
            .collect(),
    );
    let plain = export_store(&early, &Selection::all(&early), &ExportOptions::new()).unwrap();
    assert_eq!(
        c.rsd(),
        plain.rsd(),
        "the cut removes exactly the late records"
    );
    assert_eq!(c.files[0].bytes, plain.files[0].bytes);

    // P3 holds under the cut: the re-export applies it again and finds nothing more to cut.
    let v = verify_collection(&c);
    assert!(v.holds() && v.p3.holds(), "{v:?}");
}

#[test]
fn as_of_refuses_a_value_with_no_end() {
    let (lib, _) = chats("no-end");
    let snap = snapshot(&lib, &With::new()).unwrap();
    for bad in ["2024-01-15/..", "not a date"] {
        assert!(
            export_store(
                &snap,
                &Selection::all(&snap),
                &ExportOptions::new().as_of(bad)
            )
            .is_err(),
            "{bad}"
        );
    }
}
