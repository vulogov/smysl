//! Staging - rule S (§9.1, §22, `stage.rs`).
//!
//! **Model output MUST NOT enter the store directly.** It is parsed, checked, and written to
//! `.smysl/staged.smy`, where a human or a later command decides whether it becomes part of
//! the graph. `smysl merge --staged` is that decision; `ingest` exits 10 without it.
//!
//! Rule M is checked **here**, against the store, rather than inside the repair loop:
//! grounds may reference units the chunk did not contain, so a claim resting on something
//! ingested an hour ago is only checkable once both are in view. A unit violating rule M
//! yields a diagnostic, not a stored unit - §9.1 is explicit that the diagnostic is the
//! outcome, not a downgrade.
//!
//! The staged file is ordinary surface text. That is deliberate: the thing a human is asked
//! to approve should be the thing they can read.

use std::path::{Path, PathBuf};

use smysl_check::{check, CheckOptions};
use smysl_core::surface::{write_surface, WriteContext};
use smysl_core::{
    canonical_uid, Attestation, Diagnostic, Hlc, Label, Op, Record, Report, Rung, Severity, Uid,
    UnitCore,
};
use smysl_graph::Store;

use std::collections::BTreeMap;

/// Where staged output waits for confirmation (§7.3).
pub const PATH: &str = ".smysl/staged.smy";

/// A batch awaiting confirmation.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Staged {
    pub units: Vec<UnitCore>,
    pub attestations: Vec<Attestation>,
    pub labels: BTreeMap<Label, Uid>,
    /// What checking the batch against the store found.
    pub report: Report,
    /// Units rejected by rule M, with why. §9.1: a diagnostic, not a stored unit.
    pub rejected: Vec<(UnitCore, Diagnostic)>,
}

impl Staged {
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Whether anything blocks confirmation.
    pub fn has_errors(&self) -> bool {
        !self.report.is_clean() || !self.rejected.is_empty()
    }

    /// The records a caller would commit.
    pub fn records(&self) -> Vec<Record> {
        let mut out: Vec<Record> = self.units.iter().cloned().map(Record::Unit).collect();
        out.extend(self.attestations.iter().cloned().map(Record::Attestation));
        out
    }

    /// The staged batch as surface text - the thing a human is asked to approve should be
    /// the thing they can read.
    pub fn to_surface(&self) -> String {
        let ctx = WriteContext::from_labels(&self.labels);
        write_surface(None, &self.records(), &ctx)
    }
}

/// Check a batch against the store and prepare it for staging (rule S).
///
/// `now` is supplied rather than read, so a caller replaying an ingest gets the same
/// attestations (guarantee A2).
pub fn prepare(
    store: &Store,
    units: Vec<UnitCore>,
    labels: BTreeMap<Label, Uid>,
    attest: &Attest,
) -> Staged {
    // Rule M needs both the batch and the store in view, so the check runs over their
    // union. The staged units are still only the batch's.
    // A throwaway union, never written: staging must not touch the store, which is exactly
    // what rule S is about.
    let mut records: Vec<Record> = store.iter().cloned().collect();
    records.extend(units.iter().cloned().map(Record::Unit));
    let merged = Store::from_records(records);

    let opts = CheckOptions::default().with_labels(labels.clone());
    let report = check(&merged, opts);

    // §9.1: a unit violating rule M yields a diagnostic, not a stored unit. Which unit a
    // diagnostic is about is what makes that separable.
    let mut rejected = Vec::new();
    let mut kept = Vec::new();
    for u in units {
        let uid = canonical_uid(&u);
        let fatal = report.iter().find(|d| {
            d.severity == Severity::Error
                && d.subject == smysl_core::Subject::Unit(uid)
                && d.code == smysl_core::Code::E030
        });
        match fatal {
            Some(d) => rejected.push((u, d.clone())),
            None => kept.push(u),
        }
    }

    let attestations = kept
        .iter()
        .map(|u| attest.for_unit(canonical_uid(u)))
        .collect();

    Staged {
        units: kept,
        attestations,
        labels,
        report,
        rejected,
    }
}

/// How to attest what was ingested.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Attest {
    pub agent: smysl_core::AgentId,
    pub rung: Rung,
    /// Supplied, never read, so a replayed ingest produces the same attestations.
    pub now: Hlc,
    pub hop: u32,
    pub recipe: Option<([u8; 32], [u8; 32])>,
}

impl Attest {
    pub fn new(agent: smysl_core::AgentId, rung: Rung, now: Hlc) -> Attest {
        Attest {
            agent,
            rung,
            now,
            hop: 0,
            recipe: None,
        }
    }

    pub fn with_recipe(mut self, recipe: [u8; 32], family: [u8; 32]) -> Attest {
        self.recipe = Some((recipe, family));
        self
    }

    pub fn at_hop(mut self, hop: u32) -> Attest {
        self.hop = hop;
        self
    }

    /// The attestation for one ingested unit.
    ///
    /// `op: Imported` rather than `Authored`: ingest transcribes a document, and claiming
    /// authorship would misattribute the content to the tool. Rule T reads the rung from
    /// here, so this is the record that decides what the unit may claim.
    pub fn for_unit(&self, uid: Uid) -> Attestation {
        let a = Attestation::new(
            uid,
            self.agent.clone(),
            Op::Imported,
            self.rung,
            self.now.clone(),
        )
        .at_hop(self.hop);
        match self.recipe {
            Some((r, f)) => a.with_recipe(r, f),
            None => a,
        }
    }
}

/// Write a staged batch to `.smysl/staged.smy`, relative to `root`.
pub fn write(root: impl AsRef<Path>, staged: &Staged) -> Result<PathBuf, std::io::Error> {
    let path = root.as_ref().join(PATH);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, staged.to_surface())?;
    Ok(path)
}

/// Read a staged batch back, as records.
pub fn read(root: impl AsRef<Path>) -> Result<Vec<Record>, String> {
    let path = root.as_ref().join(PATH);
    let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let out = smysl_core::surface::parse_surface(&src).map_err(|e| e.to_string())?;
    Ok(out.records)
}

/// Discard a staged batch.
pub fn discard(root: impl AsRef<Path>) -> Result<(), std::io::Error> {
    match std::fs::remove_file(root.as_ref().join(PATH)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{AgentId, KernelType, SourceKind, SourceRef, Status, UnitCoreBuilder};

    fn agent() -> AgentId {
        AgentId::new("tool:smysl-ingest").unwrap()
    }

    fn attest() -> Attest {
        Attest::new(agent(), Rung::Document, Hlc::zero(agent()))
    }

    fn cited(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Evidence, gist, Status::Cited)
            .source(SourceRef::new(SourceKind::Doc, "postmortem"))
            .build()
            .unwrap()
    }

    fn derived_on(gist: &str, ground: Uid) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Finding, gist, Status::Derived)
            .grounds([ground])
            .build()
            .unwrap()
    }

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("smysl-stage-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn a_clean_batch_stages_with_an_attestation_each() {
        let a = cited("p95 rose to 410ms");
        let b = derived_on("the pool saturated", canonical_uid(&a));
        let out = prepare(&Store::new(), vec![a, b], BTreeMap::new(), &attest());

        assert_eq!(out.len(), 2);
        assert_eq!(out.attestations.len(), 2);
        assert!(!out.has_errors(), "{:?}", out.report);
    }

    /// Ingest transcribes a document; claiming authorship would misattribute the content to
    /// the tool.
    #[test]
    fn attestations_are_imported_not_authored() {
        let out = prepare(&Store::new(), vec![cited("g")], BTreeMap::new(), &attest());
        assert_eq!(out.attestations[0].op, Op::Imported);
        assert_eq!(out.attestations[0].rung, Rung::Document);
    }

    #[test]
    fn a_recipe_reaches_the_attestation() {
        let out = prepare(
            &Store::new(),
            vec![cited("g")],
            BTreeMap::new(),
            &attest().with_recipe([1; 32], [2; 32]),
        );
        assert_eq!(out.attestations[0].recipe, Some([1; 32]));
        assert_eq!(out.attestations[0].family, Some([2; 32]));
    }

    /// §9.1 is explicit: a unit violating rule M yields a diagnostic, not a stored unit.
    #[test]
    fn a_rule_m_violation_is_rejected_rather_than_downgraded() {
        let weak = UnitCoreBuilder::new(KernelType::Claim, "a guess", Status::Speculative)
            .build()
            .unwrap();
        // `derived` resting on `speculative` exceeds its weakest ground.
        let laundered = derived_on("a strong claim on a weak ground", canonical_uid(&weak));

        let out = prepare(
            &Store::new(),
            vec![weak.clone(), laundered.clone()],
            BTreeMap::new(),
            &attest(),
        );

        assert_eq!(out.len(), 1, "only the honest unit stages");
        assert_eq!(out.units[0], weak);
        assert_eq!(out.rejected.len(), 1);
        assert_eq!(out.rejected[0].0, laundered);
        assert_eq!(out.rejected[0].1.code, smysl_core::Code::E030);
        assert!(out.has_errors());
    }

    /// Grounds may reference units the chunk did not contain, so rule M is checked against
    /// the union - a claim resting on something ingested an hour ago is the normal case.
    #[test]
    fn grounds_already_in_the_store_satisfy_rule_m() {
        let ground = cited("already ingested");
        let uid = canonical_uid(&ground);
        let store = Store::from_records(vec![Record::Unit(ground)]);

        let out = prepare(
            &store,
            vec![derived_on("rests on the earlier one", uid)],
            BTreeMap::new(),
            &attest(),
        );
        assert_eq!(out.len(), 1);
        assert!(out.rejected.is_empty(), "{:?}", out.rejected);
    }

    /// The thing a human is asked to approve should be the thing they can read.
    #[test]
    fn the_staged_form_is_readable_surface_text() {
        let out = prepare(
            &Store::new(),
            vec![cited("p95 rose to 410ms")],
            BTreeMap::new(),
            &attest(),
        );
        let text = out.to_surface();
        assert!(text.contains("@evidence"), "{text}");
        assert!(text.contains("p95 rose to 410ms"));
    }

    #[test]
    fn a_staged_batch_round_trips_through_the_file() {
        let root = tmp("roundtrip");
        let out = prepare(
            &Store::new(),
            vec![cited("one"), cited("two")],
            BTreeMap::new(),
            &attest(),
        );
        let path = write(&root, &out).unwrap();
        assert!(path.ends_with(PATH));

        let records = read(&root).unwrap();
        let units: Vec<&UnitCore> = records.iter().filter_map(|r| r.as_unit()).collect();
        assert_eq!(units.len(), 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn discarding_is_idempotent() {
        let root = tmp("discard");
        let out = prepare(&Store::new(), vec![cited("x")], BTreeMap::new(), &attest());
        write(&root, &out).unwrap();
        assert!(root.join(PATH).exists());
        discard(&root).unwrap();
        assert!(!root.join(PATH).exists());
        assert!(discard(&root).is_ok(), "discarding twice is not an error");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reading_an_absent_batch_is_an_error_not_a_panic() {
        assert!(read("/nonexistent-smysl-root").is_err());
    }

    #[test]
    fn an_empty_batch_stages_as_nothing() {
        let out = prepare(&Store::new(), Vec::new(), BTreeMap::new(), &attest());
        assert!(out.is_empty());
        assert!(!out.has_errors());
    }

    /// A replayed ingest must produce the same attestations, or the log would grow a new
    /// record every time somebody re-ran the same command.
    #[test]
    fn attestations_are_a_function_of_their_inputs() {
        let first = prepare(&Store::new(), vec![cited("g")], BTreeMap::new(), &attest());
        let second = prepare(&Store::new(), vec![cited("g")], BTreeMap::new(), &attest());
        assert_eq!(first.attestations, second.attestations);
    }

    #[test]
    fn the_staged_path_is_the_documented_one() {
        assert_eq!(PATH, ".smysl/staged.smy");
    }
}
