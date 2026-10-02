//! `SMY-W409`: an enumeration code this build does not know is reported, not merely survived.
//!
//! 1.9 opened four enumerations so an unrecognised code no longer fails the decode. Preserving
//! one in silence would repeat the mistake `SMY-W014` was added to fix — a reader handed a
//! document it cannot fully interpret, and told nothing about it. Preservation is rule X
//! working; saying nothing is how a reader comes to believe it has seen the whole document.

use smysl_check::{check, CheckOptions};
use smysl_core::Code;
use smysl_core::{
    AgentId, Hlc, KernelType, Record, SourceKind, SourceRef, Status, Thread, ThreadId,
    ThreadSchema, UnitCoreBuilder,
};
use smysl_graph::Store;

fn agent() -> AgentId {
    AgentId::new("human:vu").unwrap()
}

/// An unknown source kind is reported, and the message names the code rather than the word
/// "unknown" — the raw byte is the only thing that says which future kind was meant.
#[test]
fn an_unknown_source_kind_is_reported() {
    let s = SourceRef::new(SourceKind::Doc, "x://y")
        .with_unknown_kind(9)
        .expect("9 is neither 255 nor a kind this build knows");
    let u = UnitCoreBuilder::new(
        KernelType::Claim,
        "a claim from a source we cannot name",
        Status::Cited,
    )
    .source(s)
    .build()
    .unwrap();

    let store = Store::from_records(vec![Record::Unit(u)]);
    let report = check(&store, CheckOptions::strict());

    let w409: Vec<_> = report.iter().filter(|d| d.code == Code::W409).collect();
    assert_eq!(
        w409.len(),
        1,
        "exactly one unknown code, exactly one report"
    );
    assert!(
        w409[0].message.contains('9'),
        "the message must name the code, got: {}",
        w409[0].message
    );
}

/// An unknown thread schema is reported, and says what it cost — the thread defines no roles.
#[test]
fn an_unknown_thread_schema_is_reported() {
    let th = Thread::new(
        ThreadId::new("t/x").unwrap(),
        ThreadSchema::Analysis,
        agent(),
        "a thread whose schema is from a later release",
        Hlc::new(1, 0, agent()),
    )
    .with_unknown_schema(7)
    .expect("7 is neither 255 nor a schema this build knows");

    let store = Store::from_records(vec![Record::Thread(th)]);
    let report = check(&store, CheckOptions::strict());

    assert!(
        report.iter().any(|d| d.code == Code::W409),
        "an unknown schema must be reported"
    );
}

/// A store with nothing unknown in it says nothing. A warning that always fires is noise.
#[test]
fn a_store_with_no_unknown_codes_is_quiet() {
    let u = UnitCoreBuilder::new(
        KernelType::Claim,
        "an ordinary claim with a known source",
        Status::Cited,
    )
    .source(SourceRef::new(SourceKind::Doc, "x://y"))
    .build()
    .unwrap();
    let store = Store::from_records(vec![Record::Unit(u)]);
    let report = check(&store, CheckOptions::strict());

    assert!(
        !report.iter().any(|d| d.code == Code::W409),
        "nothing unknown, nothing to say"
    );
}
