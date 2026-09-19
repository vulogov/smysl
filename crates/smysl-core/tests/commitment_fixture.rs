//! `fixtures/wire/F13-commitment.cbor` (1.7): a store carrying record type 13.
//!
//! §8.1 permits a new record type because an older reader preserves it verbatim and reports
//! `SMY-W014` — an addition rather than a break. This fixture is what the Python, JavaScript and
//! Go suites check that against: none of them decodes a commitment's body, and what they have to
//! prove is that a ledger written by 1.7 survives a trip through a build that predates it.
//!
//! It is built from surface text so the same bytes exercise `@commit` as well.
//!
//! `SMYSL_BLESS=1` rewrites the file. That should be a decision: it changes what three other
//! implementations are checked against.

use std::path::Path;

use smysl_core::{surface::parse_surface, to_cbor_seq, Commitment, Record};

const SRC: &str = "\
@claim c/premise { status: speculative }
~ The villain lost a child before the story opens.

@decision d/motive { status: speculative, grounds: [c/premise] }
~ The villain's motive is grief rather than revenge.

@commit c/premise { level: drafted, agent: human:vu, ts: [1726500000000, 0] }

@commit d/motive { level: canonical, agent: human:vu, ts: [1726500000001, 0], note: c/premise }

@commit d/motive { level: retconned, agent: \"model:gemini/flash-lite\", ts: [1726500000002, 3] }
";

#[test]
fn the_commitment_wire_fixture_matches_its_source() {
    let out = parse_surface(SRC).unwrap();
    assert!(out.diagnostics.is_empty(), "{:?}", out.diagnostics);
    let bytes = to_cbor_seq(&out.records);

    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/wire/F13-commitment.cbor");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &bytes).unwrap();
        return;
    }
    assert_eq!(
        std::fs::read(&path).unwrap(),
        bytes,
        "fixtures/wire/F13-commitment.cbor no longer matches its source"
    );

    let commits: Vec<&Record> = out
        .records
        .iter()
        .filter(|r| matches!(r, Record::Commit(_)))
        .collect();
    assert_eq!(
        commits.len(),
        3,
        "three commitments, two of them on one unit"
    );
}

/// `@commit` round-trips through surface text, which is how a ledger is hand-edited.
#[test]
fn a_commitment_round_trips_through_surface() {
    use smysl_core::surface::{write_surface, WriteContext};

    let out = parse_surface(SRC).unwrap();
    let ctx = WriteContext::from_labels(&out.labels);
    let written = write_surface(None, &out.records, &ctx);
    assert!(
        written.contains("@commit") && written.contains("level: canonical"),
        "{written}"
    );

    let again = parse_surface(&written).unwrap();
    assert!(again.diagnostics.is_empty(), "{:?}", again.diagnostics);
    assert_eq!(
        again.records, out.records,
        "parse → write → parse is a fixed point (§4)"
    );
}

/// Every level survives the wire, and an unknown one is refused rather than guessed at.
#[test]
fn every_level_round_trips_and_an_unknown_one_is_refused() {
    use smysl_core::{from_cbor, to_cbor, AgentId, Commit, Hlc, Uid};

    let agent = AgentId::new("human:vu").unwrap();
    for level in Commitment::ALL {
        let c = Commit::new(
            Uid::from_bytes([7; 32]),
            *level,
            agent.clone(),
            Hlc::new(1, 0, agent.clone()),
        );
        let bytes = to_cbor(&Record::Commit(c.clone()));
        let (back, _) = from_cbor(&bytes).unwrap();
        assert_eq!(back, Record::Commit(c));
        assert_eq!(to_cbor(&back), bytes);
    }

    // Level 9 is not a level. Guessing would put a word in an author's mouth.
    let c = Commit::new(
        Uid::from_bytes([7; 32]),
        Commitment::Floated,
        agent.clone(),
        Hlc::new(1, 0, agent),
    );
    let mut bytes = to_cbor(&Record::Commit(c));
    let at = bytes
        .windows(2)
        .position(|w| w == [0x01, 0x00])
        .expect("key 1 carrying level 0");
    bytes[at + 1] = 0x09;
    assert!(
        from_cbor(&bytes).is_err(),
        "an unknown commitment level must fail the record"
    );
}
