//! The conformance tree is data, not code, so nothing checks it at compile time. These
//! tests keep it honest: every fixture must be paired with an expected-diagnostic file,
//! and every code named there must exist in the registry.
//!
//! SM-P0 validated the *shape* of the suite. SM-P1 adds the decoding half: every fixture
//! is fed to the reader and the exact diagnostic set is asserted.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl::{from_cbor, Code, Record, Severity};

fn codec_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/conformance/codec")
}

fn fixtures(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("fixture directory is missing")
        .map(|e| e.expect("unreadable fixture entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    out.sort();
    out
}

fn expected_codes(fixture: &Path) -> BTreeSet<Code> {
    let path = fixture.with_extension("expected");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            Code::parse(l).unwrap_or_else(|| {
                panic!(
                    "{}: `{l}` is not a registered diagnostic code",
                    path.display()
                )
            })
        })
        .collect()
}

#[test]
fn every_codec_fixture_has_an_expected_set() {
    let dir = codec_dir();
    let files = fixtures(&dir, "cbor");
    assert!(!files.is_empty(), "the codec conformance tree is empty");
    for f in &files {
        assert!(
            f.with_extension("expected").is_file(),
            "{} has no .expected sibling",
            f.display()
        );
    }
}

#[test]
fn every_expected_code_is_registered() {
    for f in fixtures(&codec_dir(), "cbor") {
        // Panics inside `expected_codes` if a code is unknown.
        let _ = expected_codes(&f);
    }
}

#[test]
fn no_orphaned_expectation_files() {
    let dir = codec_dir();
    for e in fixtures(&dir, "expected") {
        assert!(
            e.with_extension("cbor").is_file(),
            "{} has no fixture",
            e.display()
        );
    }
}

/// The §15.4 table names six distinct ways to be non-deterministic and one way to be a
/// bad float. All of them must be represented, or the reader could pass the suite while
/// silently normalising one of them.
#[test]
fn codec_tree_covers_the_section_15_4_table() {
    let dir = codec_dir();
    let names: BTreeSet<String> = fixtures(&dir, "cbor")
        .iter()
        .map(|p| p.file_stem().unwrap().to_string_lossy().into_owned())
        .collect();

    for required in [
        "nonshortest-int",
        "indefinite-length-map",
        "indefinite-length-text",
        "unsorted-map-keys",
        "duplicate-map-key",
        "null-optional",
        "non-nfc-text",
        "float-f64",
        "float-unquantised",
        "unknown-envelope-code",
    ] {
        assert!(names.contains(required), "missing fixture `{required}`");
    }
}

/// A control that decodes cleanly is what distinguishes "the reader rejects everything"
/// from "the reader rejects the right things".
#[test]
fn the_tree_contains_clean_controls() {
    let clean: Vec<PathBuf> = fixtures(&codec_dir(), "cbor")
        .into_iter()
        .filter(|f| expected_codes(f).is_empty())
        .collect();
    assert!(
        clean.len() >= 2,
        "expected at least two fixtures that must decode cleanly, found {}",
        clean.len()
    );
}

/// An unknown envelope code is forward compatibility, not corruption: it MUST be a
/// warning, so a store written by a later minor version stays readable (rule X).
#[test]
fn unknown_envelope_code_is_a_warning_not_an_error() {
    let f = codec_dir().join("unknown-envelope-code.cbor");
    let codes = expected_codes(&f);
    assert_eq!(codes, BTreeSet::from([Code::W014]));
    assert_eq!(Code::W014.severity(), Severity::Warn);
}

/// Everything else in the tree is an error. A codec fixture that expected only warnings
/// would mean the reader is allowed to accept a non-deterministic encoding.
#[test]
fn every_defective_fixture_expects_at_least_one_error() {
    for f in fixtures(&codec_dir(), "cbor") {
        let codes = expected_codes(&f);
        let name = f.file_stem().unwrap().to_string_lossy().into_owned();
        if codes.is_empty() || name == "unknown-envelope-code" {
            continue;
        }
        assert!(
            codes.iter().any(|c| c.severity() == Severity::Error),
            "{name} expects only warnings; a non-deterministic encoding must be rejected"
        );
    }
}

/// What the reader actually reports for a fixture, expressed as diagnostic codes.
///
/// A clean decode is the empty set. An unknown record type is `SMY-W014` - a warning,
/// because the record survives - and everything else is the error the reader raised.
fn observed_codes(bytes: &[u8]) -> BTreeSet<Code> {
    match from_cbor(bytes) {
        Ok((Record::Unknown { .. }, _)) => BTreeSet::from([Code::W014]),
        Ok(_) => BTreeSet::new(),
        Err(e) => BTreeSet::from([e.code()]),
    }
}

/// The SM-P1 gate: every non-conforming fixture is rejected with exactly the expected
/// code set - no more, no fewer.
#[test]
fn every_codec_fixture_produces_its_expected_diagnostics() {
    for f in fixtures(&codec_dir(), "cbor") {
        let bytes = std::fs::read(&f).unwrap();
        let name = f.file_stem().unwrap().to_string_lossy().into_owned();
        assert_eq!(
            observed_codes(&bytes),
            expected_codes(&f),
            "fixture `{name}` did not produce its expected diagnostics"
        );
    }
}

/// The controls must decode *and* re-encode to the same bytes. Without that half, a
/// reader could pass by accepting the control and silently rewriting it.
#[test]
fn clean_fixtures_re_encode_to_the_same_bytes() {
    for f in fixtures(&codec_dir(), "cbor") {
        if !expected_codes(&f).is_empty() {
            continue;
        }
        let bytes = std::fs::read(&f).unwrap();
        let (record, n) = from_cbor(&bytes).unwrap();
        assert_eq!(n, bytes.len(), "{}: trailing bytes", f.display());
        assert_eq!(
            smysl::to_cbor(&record),
            bytes,
            "{}: re-encoding changed the bytes",
            f.display()
        );
    }
}

/// The unknown-type fixture is the one that must survive rather than fail, and its
/// payload must come back byte-identical.
#[test]
fn the_unknown_type_fixture_survives_verbatim() {
    let bytes = std::fs::read(codec_dir().join("unknown-envelope-code.cbor")).unwrap();
    let (record, _) = from_cbor(&bytes).unwrap();
    assert!(record.is_unknown());
    assert_eq!(smysl::to_cbor(&record), bytes);
}

// ---------------------------------------------------------------------------
// The corpus (§27.2)
// ---------------------------------------------------------------------------

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/corpus")
}

#[test]
fn every_corpus_fixture_has_an_expected_set() {
    let files = fixtures(&corpus_dir(), "smy");
    assert!(!files.is_empty(), "the corpus is empty");
    for f in &files {
        assert!(
            f.with_extension("expected").is_file(),
            "{} has no .expected sibling",
            f.display()
        );
    }
}

#[test]
fn every_corpus_fixture_produces_its_expected_diagnostics() {
    for f in fixtures(&corpus_dir(), "smy") {
        let src = std::fs::read_to_string(&f).unwrap();
        assert_eq!(
            all_codes(&src),
            expected_codes(&f),
            "{} did not produce its expected diagnostics",
            f.display()
        );
    }
}

/// The corpus is what every later phase is measured against, so it has to survive the
/// full surface -> records -> surface -> records loop unchanged.
#[test]
fn every_corpus_fixture_round_trips() {
    use smysl::{parse_surface, write_surface, WriteContext};
    for f in fixtures(&corpus_dir(), "smy") {
        if !expected_codes(&f).is_empty() {
            continue;
        }
        let src = std::fs::read_to_string(&f).unwrap();
        let a = parse_surface(&src).unwrap();
        let ctx = WriteContext::from_labels(&a.labels).with_salience(a.salience.clone());
        let text = write_surface(a.view.as_ref(), &a.records, &ctx);
        let b = parse_surface(&text).unwrap();
        assert_eq!(b.records, a.records, "{} lost content", f.display());
        assert_eq!(b.labels, a.labels, "{} moved a uid", f.display());

        let bytes = smysl::to_cbor_seq(&a.records);
        let (back, _) = smysl::from_cbor_seq(&bytes).unwrap();
        assert_eq!(back, a.records, "{} lost content over CBOR", f.display());
    }
}

/// F1 exercises the shapes rules M and R are about: a grounds chain deep enough for the
/// monotonicity check to bind, and a rebuttal for rule R to pin.
#[test]
fn f1_carries_a_rebuttal_and_a_grounds_chain() {
    let src = std::fs::read_to_string(corpus_dir().join("F1-incident.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    assert!(out
        .records
        .iter()
        .any(|r| matches!(r, Record::Relation(rel) if rel.kind == smysl::RelKind::Rebuts)));
    assert!(out
        .records
        .iter()
        .any(|r| matches!(r, Record::Relation(rel) if rel.kind == smysl::RelKind::Warrant)));
    let grounded = out
        .records
        .iter()
        .filter_map(Record::as_unit)
        .filter(|u| !u.grounds.is_empty())
        .count();
    assert!(grounded >= 4, "only {grounded} units carry grounds");
}

/// F3 is the design's most likely falsifier (GE-2): narrative carried on a claim graph.
/// Until that experiment runs, the least it must do is survive the format intact.
#[test]
fn f3_is_coarse_and_ordered() {
    let src = std::fs::read_to_string(corpus_dir().join("F3-narrative.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    let v = out.view.as_ref().unwrap();
    assert_eq!(v.granularity.profile, "coarse");
    assert_eq!(v.granularity.admission, smysl::Admission::Topical);

    let thread = out
        .records
        .iter()
        .find_map(|r| match r {
            Record::Thread(t) => Some(t),
            _ => None,
        })
        .unwrap();
    assert_eq!(thread.schema, smysl::ThreadSchema::Narrative);
    assert_eq!(thread.steps.len(), 5);
    assert!(thread.foreign_roles().is_empty());
}

/// F2 is where rule M has room to cascade: a ground chain deep enough that weakening the
/// bottom would have to travel, and a `backs` edge for corroboration. A shallow fixture
/// satisfies rule M by having nothing to violate, which is not the same as exercising it.
#[test]
fn f2_is_fine_grained_with_a_deep_chain_and_corroboration() {
    use smysl::Store;
    let src = std::fs::read_to_string(corpus_dir().join("F2-research.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    let v = out.view.as_ref().unwrap();
    assert_eq!(v.granularity.profile, "fine");
    assert_eq!(v.granularity.admission, smysl::Admission::SingleAssertion);

    assert!(
        out.records
            .iter()
            .any(|r| matches!(r, Record::Relation(rel) if rel.kind == smysl::RelKind::Backs)),
        "corroboration is what F2 is for"
    );

    // Depth, walked rather than assumed: the longest grounds chain must reach three hops,
    // which is the shortest chain on which a cascade is distinguishable from a local cap.
    let store = Store::from_records(out.records.clone());
    let by_uid: std::collections::BTreeMap<smysl::Uid, &smysl::UnitCore> =
        store.units().map(|(u, unit)| (*u, &unit.core)).collect();
    fn depth(
        uid: &smysl::Uid,
        by_uid: &std::collections::BTreeMap<smysl::Uid, &smysl::UnitCore>,
        seen: &mut BTreeSet<smysl::Uid>,
    ) -> usize {
        if !seen.insert(*uid) {
            return 0;
        }
        let d = by_uid
            .get(uid)
            .map(|u| {
                u.grounds
                    .iter()
                    .map(|g| 1 + depth(g, by_uid, seen))
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        seen.remove(uid);
        d
    }
    let deepest = by_uid
        .keys()
        .map(|u| depth(u, &by_uid, &mut BTreeSet::new()))
        .max()
        .unwrap_or(0);
    assert!(
        deepest >= 3,
        "deepest grounds chain is only {deepest} hop(s)"
    );
}

/// F4 exists to drive the `qa` schema, so the assertion is that derivation actually fills
/// its roles. A fixture that merely contains a question would pass a weaker test and teach
/// nothing about whether `answers` reaches the answer slot.
#[test]
fn f4_derives_a_qa_thread_that_fills_every_role() {
    use smysl::Store;
    let src = std::fs::read_to_string(corpus_dir().join("F4-qa.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    assert!(
        out.records
            .iter()
            .any(|r| matches!(r, Record::Relation(rel) if rel.kind == smysl::RelKind::Answers)),
        "F4 without an `answers` edge is not a Q&A fixture"
    );

    let store = Store::from_records(out.records.clone());
    let (thread, _) = smysl::derive_thread(
        &store,
        smysl::ThreadSchema::Qa,
        &smysl::DeriveOptions::default(),
    );
    let roles: BTreeSet<_> = thread.steps.iter().map(|s| s.role).collect();
    for required in [
        smysl::Role::Question,
        smysl::Role::Evidence,
        smysl::Role::Answer,
        smysl::Role::Caveat,
    ] {
        assert!(roles.contains(&required), "qa left {required} unfilled");
    }
}

/// F5 carries the two types nothing else in the corpus does, and the unknown header keys
/// rule X is about. The payload assertion is the load-bearing one: a fixture whose
/// extension keys were silently dropped would still parse, check and round-trip.
#[test]
fn f5_carries_data_artifact_refs_and_extension_payloads() {
    let src = std::fs::read_to_string(corpus_dir().join("F5-dataset.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();

    let types: BTreeSet<_> = out.units().map(|u| u.schema.clone()).collect();
    for required in [smysl::KernelType::Data, smysl::KernelType::ArtifactRef] {
        assert!(
            types.contains(&smysl::SchemaId::Kernel(required)),
            "{required} is absent"
        );
    }

    let with_payload = out.units().filter(|u| u.payload.is_some()).count();
    assert!(
        with_payload >= 4,
        "only {with_payload} unit(s) carry a payload"
    );
}

/// **D-5, on the real thing.** F7 is what a merged store looks like, but the property is
/// about merging, so this asserts it by merging the actual F1 and F2 stores rather than by
/// reading a file that represents one. Mixed granularity must survive `check` with warnings
/// and no errors: a merged store that failed would make merge unusable across teams, which
/// is the whole of D-5.
#[test]
fn merging_two_granularities_is_legal_not_an_error() {
    use smysl::{merge, MergeOptions, Severity, Store};

    let f1 = smysl::parse_surface(
        &std::fs::read_to_string(corpus_dir().join("F1-incident.smy")).unwrap(),
    )
    .unwrap();
    let f2 = smysl::parse_surface(
        &std::fs::read_to_string(corpus_dir().join("F2-research.smy")).unwrap(),
    )
    .unwrap();
    assert_eq!(f1.view.as_ref().unwrap().granularity.profile, "default");
    assert_eq!(f2.view.as_ref().unwrap().granularity.profile, "fine");

    let mut store = Store::from_records(f1.records.clone());
    merge(
        &mut store,
        &Store::from_records(f2.records.clone()),
        MergeOptions::default(),
    )
    .expect("merging two granularities");

    let report = smysl::check(&store, smysl::CheckOptions::default());
    let errors: Vec<_> = report
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(
        errors.is_empty(),
        "a merged store must still check: {errors:?}"
    );
}

/// F7 itself: the shape of that merged store, with the two body bands side by side. The
/// expected set is `SMY-W041` and nothing else - a warning because a body outside the
/// declared band is worth remarking on, not an error because the store is legal.
#[test]
fn f7_carries_two_body_bands_and_only_warns() {
    let f = corpus_dir().join("F7-mixed-granularity.smy");
    assert_eq!(expected_codes(&f), BTreeSet::from([Code::W041]));
    assert!(
        all_codes(&std::fs::read_to_string(&f).unwrap())
            .iter()
            .all(|c| c.severity() != Severity::Error),
        "F7 must not error"
    );
}

/// **F8: multi-agent contention.** Two agents triage the same incident, each store clean on
/// its own, and merging them raises all three detections of §5.4 at once - the label both
/// agents bound to their own conclusion, the fork alpha left by superseding one claim
/// twice, and beta's rebuttal of a claim its own thread also presents.
///
/// Detections are *reported, not recorded* (§5.4), so this asserts the report rather than
/// the store: a contention written into the log would be a stale finding the moment a third
/// store supplied the edge that orders it.
#[test]
fn f8_merging_two_agents_raises_every_contention_kind() {
    use smysl::{merge, DetectionKind, MergeOptions, Store};

    let load = |name: &str| {
        let src = std::fs::read_to_string(corpus_dir().join(name)).unwrap();
        smysl::parse_surface(&src).unwrap()
    };
    let alpha = load("F8a-agent-alpha.smy");
    let beta = load("F8b-agent-beta.smy");

    // Each half is clean alone. The disagreement is created by putting them together,
    // which is what makes this a merge fixture rather than a defective one.
    for out in [&alpha, &beta] {
        let store = Store::from_records(out.records.clone());
        let report = smysl::check(&store, smysl::CheckOptions::default());
        assert!(
            report.fail_on(smysl::Severity::Error).is_ok(),
            "an agent's own store must be clean"
        );
    }

    let mut store = Store::from_records(alpha.records.clone());
    let mut opts = MergeOptions::default();
    opts.labels = vec![alpha.labels.clone(), beta.labels.clone()];
    let report = merge(&mut store, &Store::from_records(beta.records.clone()), opts)
        .expect("merging two agents");

    let kinds: BTreeSet<DetectionKind> =
        report.contentions.iter().map(|c| c.detected.kind).collect();
    for required in [
        DetectionKind::LabelCollision,
        DetectionKind::SupersessionFork,
        DetectionKind::LiveRebuttal,
    ] {
        assert!(kinds.contains(&required), "{required:?} was not detected");
    }

    // Reported, not recorded: the store must not have grown a contention record.
    assert!(
        store.contentions().is_empty(),
        "a detection was written into the log"
    );
}

// ---------------------------------------------------------------------------
// The corpus, loaded into a store
// ---------------------------------------------------------------------------

/// Parsing and storing are separate phases, so this is the first place they meet: every
/// fixture must load into a store with nothing dangling.
#[test]
fn every_corpus_fixture_loads_into_a_store_with_no_dangling_references() {
    use smysl::Store;
    for f in fixtures(&corpus_dir(), "smy") {
        if !expected_codes(&f).is_empty() {
            continue;
        }
        let src = std::fs::read_to_string(&f).unwrap();
        let out = smysl::parse_surface(&src).unwrap();
        let store = Store::from_records(out.records.clone());

        let mut report = smysl::Report::new();
        store.report_dangling(&mut report);
        assert!(report.is_empty(), "{}: {report}", f.display());
        assert_eq!(store.units().count(), out.units().count());
    }
}

/// A view is a root set, not a container: membership is computed. F1's root reaches its
/// grounds without anything having been copied.
#[test]
fn f1_reaches_its_evidence_from_the_view_root() {
    use smysl::{EdgeSet, Store};
    let src = std::fs::read_to_string(corpus_dir().join("F1-incident.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    let store = Store::from_records(out.records.clone());

    let view = out.view.as_ref().unwrap();
    let g = store.adjacency();
    let roots: Vec<_> = view.roots.iter().filter_map(|u| g.id(u)).collect();
    assert_eq!(roots.len(), 1);

    let reachable = smysl::closure(g, &roots, &EdgeSet::support());
    let trace = out.uid_of(&smysl::Label::new("e/trace").unwrap()).unwrap();
    assert!(
        reachable.contains(&g.id(&trace).unwrap()),
        "the root does not reach its evidence"
    );
}

/// Rule R pins rebuttals into any pack touching the claim, so the store has to be able to
/// find them by uid before packing can enforce anything.
#[test]
fn f1_rebuttals_are_reachable_from_the_store() {
    use smysl::Store;
    let src = std::fs::read_to_string(corpus_dir().join("F1-incident.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    let store = Store::from_records(out.records.clone());

    let pool = out
        .uid_of(&smysl::Label::new("c/pool-saturation").unwrap())
        .unwrap();
    let canary = out
        .uid_of(&smysl::Label::new("c/canary-clean").unwrap())
        .unwrap();
    assert_eq!(store.rebuttals_of(&pool), vec![canary]);
}

/// F3's narrative is ordered by `sequences`, so a topological walk over the ordering
/// edges must reproduce the story rather than shuffle it.
#[test]
fn f3_orders_deterministically_over_its_sequence_edges() {
    use smysl::{EdgeSet, Store};
    let src = std::fs::read_to_string(corpus_dir().join("F3-narrative.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    let store = Store::from_records(out.records.clone());

    let t = smysl::topo(store.adjacency(), &EdgeSet::ordering());
    assert!(t.is_acyclic(), "the narrative must not loop");
    assert_eq!(t.order.len(), store.adjacency().len());

    let g = store.adjacency();
    let at = |l: &str| {
        let uid = out.uid_of(&smysl::Label::new(l).unwrap()).unwrap();
        t.order
            .iter()
            .position(|&n| n == g.id(&uid).unwrap())
            .unwrap()
    };
    for (earlier, later) in [
        ("p/setup", "p/complication"),
        ("p/complication", "p/turn"),
        ("p/turn", "p/resolution"),
        ("p/resolution", "p/coda"),
    ] {
        assert!(at(earlier) < at(later), "{earlier} must precede {later}");
    }
}

// ---------------------------------------------------------------------------
// Structural checks (§17 passes 2-5)
// ---------------------------------------------------------------------------

fn check_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/conformance/check")
}

/// Every code a fixture produces, from parsing and checking together.
///
/// Which layer catches a defect is an implementation detail - a unit that violates a
/// shape rule never reaches a store, so the constructor catches it at parse time. What
/// the fixture pins is the code set, not the layer.
fn all_codes(src: &str) -> BTreeSet<Code> {
    use smysl::{check, CheckOptions, Store};
    let out = match smysl::parse_surface(src) {
        Ok(o) => o,
        Err(e) => return BTreeSet::from([e.code()]),
    };
    let mut codes: BTreeSet<Code> = out.diagnostics.iter().map(|d| d.code).collect();

    let store = Store::from_records(out.records.clone());
    let opts = CheckOptions::default().with_labels(out.labels.clone());
    codes.extend(check(&store, opts).iter().map(|d| d.code));
    codes
}

#[test]
fn every_check_fixture_has_an_expected_set() {
    let files = fixtures(&check_dir(), "smy");
    assert!(!files.is_empty(), "the check conformance tree is empty");
    for f in &files {
        assert!(
            f.with_extension("expected").is_file(),
            "{} has no .expected sibling",
            f.display()
        );
    }
}

/// The SM-P4 gate: exactly the expected codes, no more and no fewer.
#[test]
fn every_check_fixture_produces_exactly_its_expected_codes() {
    for f in fixtures(&check_dir(), "smy") {
        let src = std::fs::read_to_string(&f).unwrap();
        let observed = all_codes(&src);
        let expected = expected_codes(&f);
        assert_eq!(
            observed,
            expected,
            "{}: observed {observed:?}, expected {expected:?}",
            f.file_name().unwrap().to_string_lossy()
        );
    }
}

/// A control that produces nothing distinguishes "the checker works" from "the checker
/// complains about everything".
#[test]
fn the_check_tree_contains_a_clean_control() {
    let src = std::fs::read_to_string(check_dir().join("clean-control.smy")).unwrap();
    assert!(all_codes(&src).is_empty());
}

#[test]
fn the_check_tree_covers_every_pass_the_build_implements() {
    use smysl::Pass;
    let mut seen: BTreeSet<Code> = BTreeSet::new();
    for f in fixtures(&check_dir(), "smy") {
        seen.extend(expected_codes(&f));
    }
    for (required, pass) in [
        (Code::E060, Pass::Integrity),
        (Code::W062, Pass::Integrity),
        (Code::E022, Pass::Shape),
        (Code::W024, Pass::Closure),
        (Code::E040, Pass::Granularity),
        (Code::W041, Pass::Granularity),
        (Code::E030, Pass::Epistemics),
        (Code::W013, Pass::Extension),
    ] {
        assert!(
            seen.contains(&required),
            "no fixture exercises {required} ({pass})"
        );
    }
    // Rule T has no surface syntax to exercise it: attestations are not in Appendix A's
    // grammar, so provenance can only be authored programmatically until `ingest` lands.
    assert!(Pass::Trust.is_implemented());
    assert!(!seen.contains(&Code::E033));
}

/// F6 exists to be caught. A run that reports nothing means rule M has stopped binding,
/// which is the failure mode the whole design is built to prevent.
#[test]
fn f6_is_caught_by_rule_m_at_every_hop() {
    use smysl::{check, CheckOptions, Store};
    let src = std::fs::read_to_string(corpus_dir().join("F6-adversarial.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    assert!(
        out.diagnostics.is_empty(),
        "F6 must parse cleanly - it launders epistemics, it is not malformed"
    );

    let store = Store::from_records(out.records.clone());
    let report = check(&store, CheckOptions::default());
    assert_eq!(
        report.count(smysl::Code::E030),
        3,
        "each promotion in the chain must be caught: {report}"
    );

    // Every diagnostic names the ground responsible, which is the actionable part.
    for d in report.iter().filter(|d| d.code == smysl::Code::E030) {
        assert!(d.suggestion.is_some(), "{d}");
        assert!(d.message.contains("weakest ground"), "{d}");
    }
}

/// A laundering store parses and reads, but cannot be consumed - a consumer at C-Consume
/// promises rules M and R, and this store makes that promise unkeepable.
#[test]
fn f6_reads_but_does_not_conform_at_c_consume() {
    use smysl::{check, conformance, CheckOptions, ConformanceClass, Store};
    let src = std::fs::read_to_string(corpus_dir().join("F6-adversarial.smy")).unwrap();
    let out = smysl::parse_surface(&src).unwrap();
    let store = Store::from_records(out.records.clone());
    let report = check(&store, CheckOptions::default());

    assert!(conformance(&report, ConformanceClass::Read).passed);
    let consume = conformance(&report, ConformanceClass::Consume);
    assert!(!consume.passed);
    assert_eq!(consume.blocking, vec![smysl::Code::E030]);
    assert!(!conformance(&report, ConformanceClass::Full).passed);
}

/// The corpus is what later phases are measured against, so it has to survive the checks
/// that exist today.
#[test]
fn every_corpus_fixture_passes_the_structural_checks() {
    use smysl::{check, CheckOptions, Store};
    for f in fixtures(&corpus_dir(), "smy") {
        if !expected_codes(&f).is_empty() {
            continue;
        }
        let src = std::fs::read_to_string(&f).unwrap();
        let out = smysl::parse_surface(&src).unwrap();
        let store = Store::from_records(out.records.clone());
        let report = check(
            &store,
            CheckOptions::default().with_labels(out.labels.clone()),
        );
        assert!(
            report.is_empty(),
            "the baseline corpus should be free of warnings too - {}: {report}",
            f.file_name().unwrap().to_string_lossy()
        );
    }
}
