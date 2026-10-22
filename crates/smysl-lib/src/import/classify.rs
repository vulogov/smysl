//! Classification (SMYSL-2.7 §3.3): which incoming records are accepted, which wait, which are
//! quarantined.
//!
//! Three layers, because the check names units and not records (§2.6):
//!
//! - **Waiting.** A record waits while something it names ([`names`]) is not held, by the
//!   library or by an incoming record that is not itself waiting or quarantined. A fixpoint:
//!   a waiting record stops providing what it would have provided ([`provides`]), so `held`
//!   only shrinks and the loop ends.
//! - **Quarantine.** The check runs over the library and the records that do not wait. An
//!   error on a unit or a record this import introduced quarantines it, with every incoming
//!   record that names that unit directly. An error about the whole store that the library
//!   alone does not raise quarantines everything incoming: conservative, until every pass
//!   attributes its findings to records. Warnings never quarantine.
//! - **Accept.** The rest.
//!
//! The check runs over what does not wait, not over everything, because a missing ground is
//! `SMY-E060`, an error. Checked whole, a unit whose ground has not arrived would be
//! quarantined, when §3.3 says it waits, and a reviewer would have to release what was only
//! early.
//!
//! The layers repeat until quarantine stops growing: a quarantined unit is no longer held, so
//! what rests on it now waits, and what waits is no longer checked. Each round quarantines
//! only the units whose findings rest on no other unit with one, which is what makes the
//! outcome the same whatever order the records arrive in.

use smysl_check::{check, CheckOptions};
use smysl_core::{
    ContentionId, DatingTarget, Diagnostic, Mid, Record, ResolutionTarget, Severity, Subject, Tid,
    Uid,
};
use smysl_graph::merge::contention::detect;
use smysl_graph::{DetectionContext, Store};
use std::collections::{BTreeMap, BTreeSet};

/// Something a record can name and a library can hold.
///
/// Units, relations and datings share the uid space, as they do on the wire: an attestation's
/// `uid` may be any of them, and nothing in the reference says which. So one tag serves the
/// three, where §4.1's table gives relations a second. A contention is named by its label id,
/// which the table has no tag for; it gets one, since a resolution of a contention nobody
/// holds waits as an attestation of an absent unit does.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Id {
    /// A unit, a relation (its rid) or a dating (its did). Tag `0x01`.
    Uid(Uid),
    /// A contention, by id. Tag `0x03`.
    Contention(ContentionId),
    /// A part. Tag `0x0F`.
    Tid(Tid),
    /// A manifest. Tag `0x0E`.
    Mid(Mid),
}

impl Id {
    /// The tag a table stores it under (§4.1).
    pub const fn tag(&self) -> u8 {
        match self {
            Id::Uid(_) => 0x01,
            Id::Contention(_) => 0x03,
            Id::Tid(_) => 0x0F,
            Id::Mid(_) => 0x0E,
        }
    }
}

/// What a record refers to and must find held before it is accepted (§3.3's attribution table).
///
/// Pack infos, schema declarations and records this build cannot read name nothing and never
/// wait. Records 15 and 18 never reach here: a log does not hold them (OQ-39).
pub fn names(r: &Record) -> Vec<Id> {
    let uid = |u: &Uid| Id::Uid(*u);
    match r {
        Record::Unit(c) => {
            let mut out: Vec<Id> = c.references().map(uid).collect();
            if let Some(s) = &c.source {
                out.extend(Store::part_of_reference(&s.reference).map(Id::Tid));
                out.extend(s.manifest().copied().map(Id::Mid));
            }
            out
        }
        Record::Attestation(a) => vec![uid(&a.uid)],
        Record::Relation(x) => {
            let mut out = vec![uid(&x.from), uid(&x.to)];
            out.extend(x.note.as_ref().map(uid));
            out
        }
        Record::Thread(t) => t.steps.iter().map(|s| uid(&s.unit)).collect(),
        Record::View(v) => v.roots.iter().map(uid).collect(),
        Record::Contention(k) => std::iter::once(&k.over)
            .chain(k.positions.iter())
            .map(uid)
            .collect(),
        Record::LabelBinding(b) => vec![uid(&b.uid)],
        Record::Withdrawal(w) => {
            let mut out = vec![uid(&w.relation)];
            out.extend(w.reason.as_ref().map(uid));
            out
        }
        Record::Resolution(x) => {
            let mut out = match &x.target {
                ResolutionTarget::Relation(rid) => vec![uid(rid)],
                ResolutionTarget::Contention(id) => vec![Id::Contention(id.clone())],
                _ => Vec::new(),
            };
            out.extend(x.note.as_ref().map(uid));
            out
        }
        Record::Commit(c) => {
            let mut out = vec![uid(&c.unit)];
            out.extend(c.note.as_ref().map(uid));
            out
        }
        Record::Manifest(m) => m
            .parent
            .iter()
            .map(|(p, _)| Id::Mid(*p))
            .chain(m.supersedes.map(Id::Mid))
            .collect(),
        Record::Dating(d) => {
            let mut out = match &d.target {
                DatingTarget::Unit(u) => vec![uid(u)],
                DatingTarget::Part(t) | DatingTarget::Window { tid: t, .. } => vec![Id::Tid(*t)],
                DatingTarget::Manifest(m) => vec![Id::Mid(*m)],
                #[allow(unreachable_patterns)]
                _ => Vec::new(),
            };
            out.extend(d.basis.as_ref().map(uid));
            out
        }
        Record::Redaction(x) => {
            let mut out = vec![Id::Tid(x.tid)];
            out.extend(x.reason.as_ref().map(uid));
            out
        }
        _ => Vec::new(),
    }
}

/// What a record makes held: a unit its uid, a relation its rid, a dating its did, a
/// contention its id, and a manifest its mid and its parts' tids.
pub fn provides(r: &Record) -> Vec<Id> {
    match r {
        Record::Unit(c) => vec![Id::Uid(smysl_core::canonical_uid(c))],
        Record::Relation(x) => vec![Id::Uid(x.uid())],
        Record::Dating(d) => vec![Id::Uid(d.did().as_uid())],
        Record::Contention(k) => vec![Id::Contention(k.id.clone())],
        Record::Manifest(m) => std::iter::once(Id::Mid(m.mid()))
            .chain(m.parts.iter().map(|p| Id::Tid(p.tid)))
            .collect(),
        _ => Vec::new(),
    }
}

/// What became of one incoming record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class {
    Accepted,
    /// Waiting for these.
    Waiting(BTreeSet<Id>),
    /// Quarantined, by these findings: the check's own, or the one naming the unit it rests on.
    Quarantined(Vec<Diagnostic>),
}

/// Classify `incoming` against `library`: every incoming record gets one class.
///
/// `incoming` holds records the library does not, each once. `extra` is held beside the
/// library: a staged import's own records, when what is promoted belongs to it.
pub fn classify(
    library: &Store,
    incoming: &BTreeMap<[u8; 32], Record>,
    extra: &BTreeSet<Id>,
) -> BTreeMap<[u8; 32], Class> {
    let base: BTreeSet<Id> = library
        .iter()
        .flat_map(provides)
        .chain(extra.iter().cloned())
        .collect();
    let lib_store_errors: BTreeSet<(smysl_core::Code, String)> =
        check(library, CheckOptions::default())
            .iter()
            .filter(|d| d.severity == Severity::Error && d.subject == Subject::Store)
            .map(|d| (d.code, d.message.clone()))
            .collect();
    let introduced: BTreeMap<Uid, [u8; 32]> = incoming
        .iter()
        .filter_map(|(h, r)| r.as_unit().map(|c| (smysl_core::canonical_uid(c), *h)))
        .filter(|(u, _)| !library.contains_uid(u))
        .collect();

    let mut quarantined: BTreeMap<[u8; 32], Vec<Diagnostic>> = BTreeMap::new();
    loop {
        let live = |h: &[u8; 32]| !quarantined.contains_key(h);

        // Contentions that detection derives are held as well as recorded ones: a resolution
        // may name either, as export's row 12 does.
        let candidate = Store::from_records(
            library
                .iter()
                .cloned()
                .chain(
                    incoming
                        .iter()
                        .filter(|(h, _)| live(h))
                        .map(|(_, r)| r.clone()),
                )
                .collect(),
        );
        let mut held = base.clone();
        held.extend(
            incoming
                .iter()
                .filter(|(h, _)| live(h))
                .flat_map(|(_, r)| provides(r)),
        );
        held.extend(
            detect(&candidate, &DetectionContext::default())
                .into_iter()
                .map(|k| Id::Contention(k.id)),
        );

        let mut waiting: BTreeMap<[u8; 32], BTreeSet<Id>> = BTreeMap::new();
        loop {
            let mut changed = false;
            for (h, r) in incoming {
                if !live(h) || waiting.contains_key(h) {
                    continue;
                }
                let missing: BTreeSet<Id> =
                    names(r).into_iter().filter(|i| !held.contains(i)).collect();
                if !missing.is_empty() {
                    for i in provides(r) {
                        held.remove(&i);
                    }
                    waiting.insert(*h, missing);
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }

        let checked = Store::from_records(
            library
                .iter()
                .cloned()
                .chain(
                    incoming
                        .iter()
                        .filter(|(h, _)| live(h) && !waiting.contains_key(*h))
                        .map(|(_, r)| r.clone()),
                )
                .collect(),
        );
        let mut found: BTreeMap<[u8; 32], Vec<Diagnostic>> = BTreeMap::new();
        for d in check(&checked, CheckOptions::default()).iter() {
            if d.severity != Severity::Error {
                continue;
            }
            match &d.subject {
                Subject::Unit(u) => {
                    if let Some(h) = introduced.get(u).filter(|h| !waiting.contains_key(*h)) {
                        found.entry(*h).or_default().push(d.clone());
                    }
                }
                Subject::Record(h) if incoming.contains_key(h) => {
                    found.entry(*h).or_default().push(d.clone());
                }
                Subject::Store if !lib_store_errors.contains(&(d.code, d.message.clone())) => {
                    for h in incoming.keys().filter(|h| live(h)) {
                        found.entry(*h).or_default().push(d.clone());
                    }
                }
                _ => {}
            }
        }
        found.retain(|h, _| live(h));
        // A unit with a finding that rests on another unit with one waits for it rather than
        // being quarantined beside it. Its finding may be the other's consequence, and either
        // way the outcome must not depend on arrival order: had it come first it would have
        // waited, and had the other come first it would wait for a unit that is quarantined.
        // Only when every finding rests on another (a cycle) are they all quarantined.
        let flagged: BTreeSet<Uid> = found
            .keys()
            .filter_map(|h| incoming[h].as_unit().map(smysl_core::canonical_uid))
            .collect();
        let minimal: BTreeMap<[u8; 32], Vec<Diagnostic>> = found
            .iter()
            .filter(|(h, _)| match incoming[*h].as_unit() {
                Some(c) => !c.references().any(|u| flagged.contains(u)),
                None => true,
            })
            .map(|(h, ds)| (*h, ds.clone()))
            .collect();
        if !minimal.is_empty() {
            found = minimal;
        }
        if found.is_empty() {
            return incoming
                .keys()
                .map(|h| {
                    let class = match (quarantined.remove(h), waiting.remove(h)) {
                        (Some(ds), _) => Class::Quarantined(ds),
                        (None, Some(m)) => Class::Waiting(m),
                        (None, None) => Class::Accepted,
                    };
                    (*h, class)
                })
                .collect();
        }

        // What names a newly quarantined unit directly goes with it: its attestations, bindings,
        // commitments, edges and contentions (§3.3). What rests on one (its `deps` and
        // `grounds`), and a view, thread or other record naming one, does not: it names a unit
        // the library does not hold, so it waits, and a reviewer who releases the unit
        // releases it too.
        let gone: BTreeMap<Uid, smysl_core::Code> = found
            .iter()
            .filter_map(|(h, ds)| {
                let c = incoming[h].as_unit()?;
                Some((smysl_core::canonical_uid(c), ds[0].code))
            })
            .collect();
        for (h, r) in incoming {
            let direct = matches!(
                r,
                Record::Attestation(_)
                    | Record::LabelBinding(_)
                    | Record::Commit(_)
                    | Record::Relation(_)
                    | Record::Contention(_)
            );
            if !live(h) || found.contains_key(h) || !direct {
                continue;
            }
            if let Some((u, code)) = names(r).iter().find_map(|i| match i {
                Id::Uid(u) => gone.get_key_value(u),
                _ => None,
            }) {
                found.entry(*h).or_default().push(
                    Diagnostic::new(*code)
                        .with_message(format!("names {}, which is quarantined", u.short())),
                );
            }
        }
        quarantined.extend(found);
    }
}
