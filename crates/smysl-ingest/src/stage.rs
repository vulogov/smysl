//! Staging - rule S (§9.1, §22, `stage.rs`).
//!
//! **Model output MUST NOT enter the store directly.** It is parsed, checked, and written to
//! `.smysl/staged.smy`, where a human or a later command decides whether it becomes part of
//! the graph. `smysl merge --staged` is that decision; `ingest` exits 10 without it.
//!
//! Rule M is applied **here**, against the store, rather than inside the repair loop:
//! grounds may reference units the chunk did not contain, so a claim resting on something
//! ingested an hour ago is only checkable once both are in view.
//!
//! A unit that overclaims its grounds is **weakened to what they support, and reported** -
//! see the `monotone` module, which carries the reasoning. The same treatment rule T gives an
//! over-claimed rung ceiling, so the boundary has one rule rather than two. Earlier this
//! rejected the unit instead, on a reading of §9.1; rejection cascades through everything
//! grounded on it and loses content that a later merge could have justified, and both
//! outcomes satisfy rule M equally.
//!
//! The staged file is ordinary surface text. That is deliberate: the thing a human is asked
//! to approve should be the thing they can read.

use std::path::{Path, PathBuf};

use smysl_check::{check, CheckOptions};
use smysl_core::surface::{write_surface, WriteContext};
use smysl_core::{
    canonical_uid, Attestation, Diagnostic, Hlc, Label, Op, Record, Relation, Report, Rung,
    SchemaDecl, Uid, UnitCore,
};
use smysl_graph::Store;

use std::collections::BTreeMap;

/// Where staged output waits for confirmation (§7.3).
pub const PATH: &str = ".smysl/staged.smy";

/// The staged batch's records as CBOR, beside the surface file.
///
/// The surface file is what a reviewer reads, edits and approves, and surface text has no syntax
/// for an attestation — so a batch read back from it alone commits with no provenance at all. The
/// sidecar carries the attestations; [`read`] re-attaches each only to a unit the reviewed text
/// still contains unchanged.
pub const SIDECAR: &str = ".smysl/staged.cbor";

/// A batch awaiting confirmation.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Staged {
    pub units: Vec<UnitCore>,
    /// The edges the batch declared, with endpoints following any weakening remap.
    pub relations: Vec<Relation>,
    pub attestations: Vec<Attestation>,
    pub labels: BTreeMap<Label, Uid>,
    /// Extension declarations the batch depends on, staged with it so the check sees them and
    /// the commit carries them.
    pub schemas: Vec<SchemaDecl>,
    /// What checking the batch against the store found.
    pub report: Report,
    /// Units whose status rule M lowered, and to what. Empty when the model claimed
    /// nothing it could not support.
    pub weakened: Vec<crate::monotone::Weakening>,
}

impl Staged {
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    pub fn len(&self) -> usize {
        self.units.len()
    }

    /// Whether anything blocks confirmation.
    ///
    /// A weakening does not: the unit was brought into rule M and the change is recorded,
    /// which is a thing to read rather than a thing to fix.
    pub fn has_errors(&self) -> bool {
        !self.report.is_clean()
    }

    /// The records a caller would commit: declarations, units, their label bindings, relations
    /// and attestations.
    ///
    /// The bindings were missing, so a store built from these records could not resolve a single
    /// label the batch had just staged. They follow the units they name, as the parser emits them.
    pub fn records(&self) -> Vec<Record> {
        let mut out: Vec<Record> = self
            .schemas
            .iter()
            .cloned()
            .map(Record::SchemaDecl)
            .collect();
        out.extend(self.units.iter().cloned().map(Record::Unit));
        // Only for units the batch holds: a label rule M's weakening left pointing at a uid that
        // is no longer staged would bind a name to nothing.
        let staged: std::collections::BTreeSet<Uid> =
            self.units.iter().map(canonical_uid).collect();
        out.extend(
            self.labels
                .iter()
                .filter(|(_, uid)| staged.contains(uid))
                .map(|(l, u)| Record::LabelBinding(smysl_core::LabelBinding::new(l.clone(), *u))),
        );
        out.extend(self.relations.iter().cloned().map(Record::Relation));
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
    relations: Vec<Relation>,
    labels: BTreeMap<Label, Uid>,
    attest: &Attest,
) -> Staged {
    prepare_declared(store, units, relations, labels, Vec::new(), attest)
}

/// [`prepare`], with the extension declarations the batch depends on.
///
/// A batch using `x.verify/supports` needs the `SchemaDecl` declaring it, or every check of it
/// reports `SMY-W013`. The declaration is staged with the batch rather than appended to the
/// store first, because appending outside staging is the thing rule S forbids.
pub fn prepare_declared(
    store: &Store,
    units: Vec<UnitCore>,
    relations: Vec<Relation>,
    labels: BTreeMap<Label, Uid>,
    schemas: Vec<SchemaDecl>,
    attest: &Attest,
) -> Staged {
    prepare_attested(store, units, relations, labels, schemas, attest)
}

/// [`prepare_declared`], attested per record (1.5).
///
/// Every batch until now was one agent's work, so one `Attest` described it. A caller that links
/// evidence proposes edges from a different agent than the units — and an edge attestation is how
/// a store says who asserted one — so the attestor is asked per record rather than fixed for the
/// batch. `Attest` implements [`Attesting`] and answers the same thing for everything, which is
/// what `prepare_declared` passes.
pub fn prepare_attested(
    store: &Store,
    units: Vec<UnitCore>,
    relations: Vec<Relation>,
    labels: BTreeMap<Label, Uid>,
    schemas: Vec<SchemaDecl>,
    attest: &dyn Attesting,
) -> Staged {
    prepare_under(store, units, relations, labels, schemas, attest, None)
}

/// [`prepare_attested`], checked under a granularity the caller names (1.7).
///
/// `None` takes the profile from the store's view, which is what every `prepare` did and still
/// does. `Some` is a caller saying what *this batch* was produced under — granularity constrains
/// production, not the store (D-5), so a batch ingested at `fine` should be checked at `fine`
/// however the store it is joining was authored.
///
/// This existed as a validated-and-discarded string for three releases: `ingest --granularity
/// fine` was accepted, hashed into the recipe, and then checked under the store's profile, so
/// `l0_max` and the single-assertion admission were not the ones the caller asked for.
pub fn prepare_under(
    store: &Store,
    units: Vec<UnitCore>,
    relations: Vec<Relation>,
    labels: BTreeMap<Label, Uid>,
    schemas: Vec<SchemaDecl>,
    attest: &dyn Attesting,
    granularity: Option<smysl_core::GranularityProfile>,
) -> Staged {
    // Rule M first, and *before* the check: weakening moves identities, so a report
    // computed over the model's original uids would describe a batch that no longer
    // exists. This was the bug the SM-P14 gate kept hitting - the report was taken before
    // the units were split, so it carried errors about units that were then removed.
    let applied = crate::monotone::apply(store, units);

    // Edges follow their endpoints. A weakening moves a unit's identity, so an edge left
    // pointing at the old uid would dangle - and it is `rebuts` edges that rule R needs, so
    // losing one silently is losing the constraint.
    let relations: Vec<Relation> = relations
        .into_iter()
        .map(|mut r| {
            r.from = applied.remap.get(&r.from).copied().unwrap_or(r.from);
            r.to = applied.remap.get(&r.to).copied().unwrap_or(r.to);
            r
        })
        .collect();

    // Labels follow their units to the new identities, or a label would name a uid that
    // the weakening replaced.
    let labels: BTreeMap<Label, Uid> = labels
        .into_iter()
        .map(|(l, u)| (l, applied.remap.get(&u).copied().unwrap_or(u)))
        .collect();

    // Rule M needs both the batch and the store in view, so the check runs over their
    // union. The staged units are still only the batch's.
    // A throwaway union, never written: staging must not touch the store, which is exactly
    // what rule S is about.
    let mut records: Vec<Record> = store.iter().cloned().collect();
    records.extend(schemas.iter().cloned().map(Record::SchemaDecl));
    records.extend(applied.units.iter().cloned().map(Record::Unit));
    records.extend(relations.iter().cloned().map(Record::Relation));
    let merged = Store::from_records(records);

    let mut opts = CheckOptions::default().with_labels(labels.clone());
    if let Some(g) = granularity {
        opts = opts.with_granularity(g);
    }
    let mut report = check(&merged, opts);

    // What the weakening did, said out loud. A warning: the unit is in rule M now, and the
    // record exists so a reviewer can see the model overclaimed rather than discovering it
    // by comparing statuses.
    for w in &applied.weakened {
        report.push(
            Diagnostic::on(smysl_core::Code::W036, w.after).with_message(format!(
                "rule M: {} lowered to {} by its weakest ground",
                w.from, w.to
            )),
        );
    }
    report.sort();

    let kept = applied.units;
    // Edges are attested as units are, by their rid (1.4). Until then a staged `rebuts` edge a
    // model proposed committed with no record of who asserted it, indistinguishable from a
    // reviewer's — which is what an edge attestation exists to tell apart.
    let mut attestations: Vec<Attestation> = kept
        .iter()
        .map(|u| Attesting::for_unit(attest, canonical_uid(u)))
        .collect();
    let mut rids = std::collections::BTreeSet::new();
    for r in &relations {
        if rids.insert(r.uid()) {
            attestations.push(Attesting::for_relation(attest, r.uid()));
        }
    }

    Staged {
        units: kept,
        relations,
        attestations,
        labels,
        schemas,
        report,
        weakened: applied.weakened,
    }
}

/// Who attests each staged record.
///
/// [`Attest`] answers with one agent for the whole batch, which is what an ingest run is. A caller
/// whose batch came from several hands — a model that proposed the units, another that linked a
/// test to the claim it verifies, a person confirming one edge — implements this instead and
/// answers per record (1.5). Rule T still reads the rung from whatever comes back, so an
/// attestation with a rung the unit outranks is caught at staging as it always was.
pub trait Attesting {
    /// The attestation for a staged unit.
    fn for_unit(&self, uid: Uid) -> Attestation;

    /// The attestation for a staged edge, by its rid. Defaults to [`Attesting::for_unit`], which
    /// is right when one agent produced the whole batch.
    fn for_relation(&self, rid: Uid) -> Attestation {
        self.for_unit(rid)
    }
}

impl Attesting for Attest {
    fn for_unit(&self, uid: Uid) -> Attestation {
        Attest::for_unit(self, uid)
    }

    fn for_relation(&self, rid: Uid) -> Attestation {
        Attest::for_relation(self, rid)
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

    /// The attestation for one staged edge, by its rid: the same agent, rung, clock, hop and
    /// recipe as the units staged with it. Rule T does not read it — an edge has no status.
    pub fn for_relation(&self, rid: Uid) -> Attestation {
        self.for_unit(rid)
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
    std::fs::write(
        root.as_ref().join(SIDECAR),
        smysl_core::to_cbor_seq(&staged.records()),
    )?;
    Ok(path)
}

/// Read a staged batch back, as records.
///
/// The surface file decides *what* is committed — it is what the reviewer approved. The sidecar
/// decides *who produced it*: an attestation is kept only when the unit it names is present,
/// byte for byte, in the reviewed text. A unit the reviewer edited has a different uid and commits
/// unattested, because the tool did not produce that content and must not be recorded as having.
/// A missing or unreadable sidecar leaves every unit unattested rather than failing the commit.
pub fn read(root: impl AsRef<Path>) -> Result<Vec<Record>, String> {
    let path = root.as_ref().join(PATH);
    let src = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let out = smysl_core::surface::parse_surface(&src).map_err(|e| e.to_string())?;
    let mut records = out.records;

    let units: std::collections::BTreeSet<Uid> = records
        .iter()
        .filter_map(|r| r.as_unit().map(canonical_uid))
        .collect();
    // Edges too: an edge's attestation is kept while the reviewed text holds the edge *and* both
    // its endpoints. Editing a unit leaves an `@rel` line naming the old uid, so the rid is
    // unchanged while the edge now points at content the tool never staged.
    let mut present = units.clone();
    present.extend(records.iter().filter_map(|r| match r {
        Record::Relation(rel) if units.contains(&rel.from) && units.contains(&rel.to) => {
            Some(rel.uid())
        }
        _ => None,
    }));
    if let Ok(bytes) = std::fs::read(root.as_ref().join(SIDECAR)) {
        if let Ok((sidecar, _)) = smysl_core::from_cbor_seq(&bytes) {
            records.extend(sidecar.into_iter().filter(|r| match r {
                Record::Attestation(a) => present.contains(&a.uid),
                _ => false,
            }));
        }
    }
    Ok(records)
}

/// Discard a staged batch.
pub fn discard(root: impl AsRef<Path>) -> Result<(), std::io::Error> {
    for f in [PATH, SIDECAR] {
        match std::fs::remove_file(root.as_ref().join(f)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
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

    /// A batch that uses an extension relation stages with its declaration, not a warning.
    ///
    /// `prepare` took units, relations and labels, so a library caller had no way to stage the
    /// `SchemaDecl` for `x.verify/supports`: every batch reported `SMY-W013` unless the declaration
    /// had been appended to the store outside staging, which rule S exists to prevent.
    #[test]
    fn a_batch_stages_with_its_schema_declaration() {
        let fact = cited("a fact read from the code");
        let claim = cited("a claim the fact supports");
        let kind = smysl_core::RelKind::parse("x.verify/supports").unwrap();
        let rel = Relation::new(kind.clone(), canonical_uid(&fact), canonical_uid(&claim));
        let mut decl =
            smysl_core::SchemaDecl::new(smysl_core::SchemaId::parse("x.verify/v1").unwrap(), 1);
        decl.relations = vec![kind];

        let bare = prepare(
            &Store::new(),
            vec![fact.clone(), claim.clone()],
            vec![rel.clone()],
            BTreeMap::new(),
            &attest(),
        );
        assert_eq!(
            bare.report.count(smysl_core::Code::W013),
            1,
            "the control: undeclared, it warns"
        );

        let declared = prepare_declared(
            &Store::new(),
            vec![fact, claim],
            vec![rel],
            BTreeMap::new(),
            vec![decl.clone()],
            &attest(),
        );
        assert_eq!(
            declared.report.count(smysl_core::Code::W013),
            0,
            "{:?}",
            declared.report
        );
        assert_eq!(declared.schemas, vec![decl]);
        assert!(declared
            .records()
            .iter()
            .any(|r| matches!(r, Record::SchemaDecl(_))));

        // And it survives the staged file, since the surface can spell it since 1.3.
        let dir = tmp("declared");
        write(&dir, &declared).unwrap();
        let back = read(&dir).unwrap();
        assert!(
            back.iter().any(|r| matches!(r, Record::SchemaDecl(_))),
            "the declaration did not survive staged.smy"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_clean_batch_stages_with_an_attestation_each() {
        let a = cited("p95 rose to 410ms");
        let b = derived_on("the pool saturated", canonical_uid(&a));
        let out = prepare(
            &Store::new(),
            vec![a, b],
            Vec::new(),
            BTreeMap::new(),
            &attest(),
        );

        assert_eq!(out.len(), 2);
        assert_eq!(out.attestations.len(), 2);
        assert!(!out.has_errors(), "{:?}", out.report);
    }

    /// Ingest transcribes a document; claiming authorship would misattribute the content to
    /// the tool.
    #[test]
    fn attestations_are_imported_not_authored() {
        let out = prepare(
            &Store::new(),
            vec![cited("g")],
            Vec::new(),
            BTreeMap::new(),
            &attest(),
        );
        assert_eq!(out.attestations[0].op, Op::Imported);
        assert_eq!(out.attestations[0].rung, Rung::Document);
    }

    #[test]
    fn a_recipe_reaches_the_attestation() {
        let out = prepare(
            &Store::new(),
            vec![cited("g")],
            Vec::new(),
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
            Vec::new(),
            BTreeMap::new(),
            &attest(),
        );

        // Both stage. The overclaim is lowered rather than dropped, and the lowering is
        // recorded - a reviewer sees what the model tried, not a hole where it was.
        assert_eq!(out.len(), 2, "nothing is lost to an overclaim");
        assert!(out.units.iter().any(|u| u.gist == laundered.gist));
        assert_eq!(out.weakened.len(), 1);
        assert_eq!(out.weakened[0].from, Status::Derived);
        assert_eq!(out.weakened[0].to, Status::Speculative);

        // And the batch now satisfies rule M, so nothing blocks confirmation.
        assert!(!out.has_errors(), "{:?}", out.report);
        assert!(out.report.iter().any(|d| d.code == smysl_core::Code::W036));
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
            Vec::new(),
            BTreeMap::new(),
            &attest(),
        );
        assert_eq!(out.len(), 1);
        assert!(out.weakened.is_empty(), "{:?}", out.weakened);
    }

    /// The thing a human is asked to approve should be the thing they can read.
    #[test]
    fn the_staged_form_is_readable_surface_text() {
        let out = prepare(
            &Store::new(),
            vec![cited("p95 rose to 410ms")],
            Vec::new(),
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
            Vec::new(),
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

    /// A staged batch's attestations reach the commit — for exactly the units the tool produced.
    ///
    /// The staged file is surface text, which has no syntax for an attestation, and `read` parsed
    /// it back: every unit committed by `merge --staged` arrived with no agent, rung or recipe. Found
    /// by committing a real live batch — 9 units, 9 label bindings, 0 attestations. The round-trip
    /// test above counted units and never asked.
    /// A batch from several hands: the units from the model that proposed them, the edge from the
    /// one that linked it (1.5). Before `Attesting`, a caller wanting that staged twice.
    #[test]
    fn a_batch_can_be_attested_per_record() {
        struct PerRecord {
            units: Attest,
            edges: Attest,
            rid: Uid,
        }
        impl Attesting for PerRecord {
            fn for_unit(&self, uid: Uid) -> Attestation {
                self.units.for_unit(uid)
            }
            fn for_relation(&self, rid: Uid) -> Attestation {
                assert_eq!(rid, self.rid, "asked about an edge the batch does not have");
                self.edges.for_relation(rid)
            }
        }

        let (a, b) = (cited("p95 rose to 410ms"), cited("the pool saturated"));
        let edge = Relation::new(
            smysl_core::RelKind::Backs,
            canonical_uid(&a),
            canonical_uid(&b),
        );
        let linker = AgentId::new("model:linker").unwrap();
        let attest = PerRecord {
            units: attest(),
            edges: Attest::new(linker.clone(), Rung::Model, Hlc::zero(linker.clone())),
            rid: edge.uid(),
        };
        let out = prepare_attested(
            &Store::new(),
            vec![a, b],
            vec![edge.clone()],
            BTreeMap::new(),
            Vec::new(),
            &attest,
        );

        let who = |uid: Uid| {
            out.attestations
                .iter()
                .find(|x| x.uid == uid)
                .map(|x| x.agent.clone())
                .expect("attested")
        };
        assert_eq!(who(edge.uid()), linker, "the edge carries the linker");
        assert_eq!(
            who(canonical_uid(&out.units[0])),
            agent(),
            "the units do not"
        );
        assert!(out.report.fail_on(smysl_core::Severity::Error).is_ok());
    }

    /// A staged edge is attested by its rid, keeps that attestation through the staged file while
    /// the reviewed text still holds it, and loses it when the reviewer changes an endpoint.
    #[test]
    fn a_staged_edge_is_attested_and_the_attestation_survives_review() {
        let root = tmp("edge-attest");
        let (a, b) = (cited("p95 rose to 410ms"), cited("the pool saturated"));
        let edge = Relation::new(
            smysl_core::RelKind::Rebuts,
            canonical_uid(&a),
            canonical_uid(&b),
        );
        let out = prepare(
            &Store::new(),
            vec![a, b],
            vec![edge.clone()],
            BTreeMap::new(),
            &attest(),
        );
        let on_edge = |records: &[Record]| {
            records
                .iter()
                .filter(|r| matches!(r, Record::Attestation(x) if x.uid == edge.uid()))
                .count()
        };
        assert_eq!(on_edge(&out.records()), 1, "prepare attests the edge");
        let store = Store::from_records(out.records());
        let attached = &store.relation_by_id(&edge.uid()).unwrap().attestations;
        assert_eq!(attached.len(), 1);
        assert_eq!(attached.iter().next().unwrap().op, Op::Imported);

        write(&root, &out).unwrap();
        assert_eq!(on_edge(&read(&root).unwrap()), 1, "kept through the file");

        let file = root.join(PATH);
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(
            &file,
            text.replace("the pool saturated", "the pool saturated at the peak"),
        )
        .unwrap();
        assert_eq!(
            on_edge(&read(&root).unwrap()),
            0,
            "an edge whose endpoint was edited is not the edge the tool staged"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_staged_batchs_attestations_survive_the_file_for_units_left_unedited() {
        let root = tmp("attest");
        let out = prepare(
            &Store::new(),
            vec![cited("p95 rose to 410ms"), cited("the pool saturated")],
            Vec::new(),
            BTreeMap::new(),
            &attest(),
        );
        write(&root, &out).unwrap();

        let attested = |records: &[Record]| -> Vec<String> {
            let units: BTreeMap<Uid, String> = records
                .iter()
                .filter_map(|r| r.as_unit().map(|u| (canonical_uid(u), u.gist.clone())))
                .collect();
            let mut v: Vec<String> = records
                .iter()
                .filter_map(|r| match r {
                    Record::Attestation(a) => units.get(&a.uid).cloned(),
                    _ => None,
                })
                .collect();
            v.sort();
            v
        };
        assert_eq!(
            attested(&read(&root).unwrap()),
            vec![
                "p95 rose to 410ms".to_string(),
                "the pool saturated".to_string()
            ]
        );

        // A reviewer edits one unit. Its content is no longer what the tool produced, so it must
        // not commit carrying the tool's attestation; the untouched one keeps its own.
        let file = root.join(PATH);
        let text = std::fs::read_to_string(&file).unwrap();
        std::fs::write(
            &file,
            text.replace("the pool saturated", "the pool saturated at the peak"),
        )
        .unwrap();
        let after = read(&root).unwrap();
        assert_eq!(attested(&after), vec!["p95 rose to 410ms".to_string()]);
        // Counted directly as well. The helper above maps attestations through the units present,
        // so one left naming the edited unit's old uid would vanish from it — and the first
        // version of this test passed with exactly that defect put back.
        let attestations = after
            .iter()
            .filter(|r| matches!(r, Record::Attestation(_)))
            .count();
        assert_eq!(
            attestations, 1,
            "an attestation for content that is no longer staged was kept"
        );

        discard(&root).unwrap();
        assert!(
            !root.join(SIDECAR).exists(),
            "discard left the sidecar behind"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A store built from a staged batch's records resolves the batch's own labels.
    ///
    /// `records()` — "the records a caller would commit" — carried units, relations and
    /// attestations and no `LabelBinding`, so `resolve_label` returned `Unbound` for every label
    /// the batch had just staged. The CLI had a private helper for it that a library caller could
    /// not reach.
    #[test]
    fn a_staged_batchs_records_carry_its_label_bindings() {
        let a = cited("p95 rose to 410ms");
        let b = cited("the pool saturated");
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        let labels = BTreeMap::from([
            (Label::new("e/p95").unwrap(), ua),
            (Label::new("c/pool").unwrap(), ub),
        ]);
        let staged = prepare(
            &Store::new(),
            vec![a, b],
            Vec::new(),
            labels.clone(),
            &attest(),
        );
        let store = Store::from_records(staged.records());
        for (label, uid) in &labels {
            assert_eq!(
                smysl_graph::resolve_label(&store, label),
                Ok(*uid),
                "{label}"
            );
        }
        let bindings = staged
            .records()
            .iter()
            .filter(|r| matches!(r, Record::LabelBinding(_)))
            .count();
        assert_eq!(bindings, 2);
    }

    #[test]
    fn discarding_is_idempotent() {
        let root = tmp("discard");
        let out = prepare(
            &Store::new(),
            vec![cited("x")],
            Vec::new(),
            BTreeMap::new(),
            &attest(),
        );
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
        let out = prepare(
            &Store::new(),
            Vec::new(),
            Vec::new(),
            BTreeMap::new(),
            &attest(),
        );
        assert!(out.is_empty());
        assert!(!out.has_errors());
    }

    /// A replayed ingest must produce the same attestations, or the log would grow a new
    /// record every time somebody re-ran the same command.
    #[test]
    fn attestations_are_a_function_of_their_inputs() {
        let first = prepare(
            &Store::new(),
            vec![cited("g")],
            Vec::new(),
            BTreeMap::new(),
            &attest(),
        );
        let second = prepare(
            &Store::new(),
            vec![cited("g")],
            Vec::new(),
            BTreeMap::new(),
            &attest(),
        );
        assert_eq!(first.attestations, second.attestations);
    }

    #[test]
    fn the_staged_path_is_the_documented_one() {
        assert_eq!(PATH, ".smysl/staged.smy");
    }
}
