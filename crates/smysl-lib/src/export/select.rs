//! The inclusion closure (SMYSL-2.7 §3.6): which units a file needs, and which records.
//!
//! One rule per record type, applied to the snapshot's records and never to its derived maps.
//! A store's `relations` map keeps the first of two records that differ only in weight, and
//! the log keeps both. Every function here is a function of a set of uids and the snapshot,
//! so nothing depends on log order or on the order of a walk.

use super::{ExportOptions, With};
use smysl_core::types::record::code;
use smysl_core::{
    canonical_uid, ContentionId, DatingTarget, Mid, Record, RelKind, ResolutionTarget, SchemaId,
    ThreadId, Tid, Uid,
};
use smysl_graph::merge::contention::detect;
use smysl_graph::{closure, DetectionContext, EdgeSet, Store};
use std::collections::{BTreeMap, BTreeSet};

/// One record of the snapshot, with what selection keeps asking of it computed once.
pub(crate) struct Entry<'s> {
    pub record: &'s Record,
    /// BLAKE3 of the record's canonical bytes: its identity in a file and in the digest.
    pub hash: [u8; 32],
    /// The unit's uid, for a unit record.
    uid: Option<Uid>,
    /// Whether the record names only things the snapshot does not hold (dangling at source).
    ///
    /// No selection can include it by its type's rule, since its rule asks for something no
    /// file can hold. §3.6 exports such records as they are and counts them (`SMY-W495`). Since
    /// no key owns them, every keyed file carries them, so a recipient of any one file holds
    /// them waiting, as it would have in the source store.
    orphan: bool,
}

/// The snapshot's records, each once.
///
/// Deduplicated by record hash, because a store opened from a log keeps a log's repeats and a
/// file written from it must not.
pub(crate) struct Source<'s> {
    pub store: &'s Store,
    pub entries: Vec<Entry<'s>>,
    /// The contentions detection derives over the snapshot, which merge reports and never
    /// records: a resolution may name one by its derived id (row 12).
    detected: Vec<smysl_core::Contention>,
    /// Per schema id, every type and relation kind any of its declarations lists (row 8).
    ///
    /// Row 8 as §3.6 writes it misses a declaration that lists neither, which is F11's: its units
    /// use the schema through payload keys (`"code:kind"` under `x.code/v1`). So a declaration
    /// also travels with a unit whose payload has a key prefixed by the schema's domain.
    schemas: BTreeMap<SchemaId, (BTreeSet<SchemaId>, BTreeSet<RelKind>)>,
}

impl<'s> Source<'s> {
    pub(crate) fn new(store: &'s Store) -> Source<'s> {
        let mut seen = BTreeSet::new();
        let mut schemas: BTreeMap<SchemaId, (BTreeSet<SchemaId>, BTreeSet<RelKind>)> =
            BTreeMap::new();
        let detected = detect(store, &DetectionContext::default());
        let mut rids = BTreeSet::new();
        let mut dids = BTreeSet::new();
        let mut contentions: BTreeSet<ContentionId> =
            detected.iter().map(|k| k.id.clone()).collect();
        for r in store.iter() {
            match r {
                Record::Relation(x) => {
                    rids.insert(x.uid());
                }
                Record::Dating(d) => {
                    dids.insert(d.did().as_uid());
                }
                Record::Contention(k) => {
                    contentions.insert(k.id.clone());
                }
                _ => {}
            }
        }
        let held = |u: &Uid| store.contains_uid(u);
        let orphan = |r: &Record| match r {
            Record::Attestation(a) => {
                !held(&a.uid) && !rids.contains(&a.uid) && !dids.contains(&a.uid)
            }
            Record::LabelBinding(b) => !held(&b.uid) && !rids.contains(&b.uid),
            Record::Relation(x) => !held(&x.from) && !held(&x.to),
            Record::Contention(k) => !held(&k.over),
            Record::Withdrawal(w) => !rids.contains(&w.relation) && !dids.contains(&w.relation),
            Record::Resolution(x) => match &x.target {
                ResolutionTarget::Relation(rid) => !rids.contains(rid),
                ResolutionTarget::Contention(id) => !contentions.contains(id),
                _ => false,
            },
            Record::Commit(c) => !held(&c.unit) && !dids.contains(&c.unit),
            Record::Dating(d) => matches!(&d.target, DatingTarget::Unit(u) if !held(u)),
            _ => false,
        };
        let mut entries = Vec::new();
        for r in store.iter() {
            let hash = Store::record_hash(r);
            if !seen.insert(hash) {
                continue;
            }
            if let Record::SchemaDecl(d) = r {
                let e = schemas.entry(d.id.clone()).or_default();
                e.0.extend(d.types.iter().cloned());
                e.1.extend(d.relations.iter().cloned());
            }
            let uid = r.as_unit().map(canonical_uid);
            entries.push(Entry {
                record: r,
                hash,
                uid,
                orphan: orphan(r),
            });
        }
        Source {
            store,
            entries,
            detected,
            schemas,
        }
    }
}

/// What one file's selection holds.
#[derive(Default)]
pub(crate) struct Picked<'s> {
    /// The selected records by `(type code, record hash)`: the canonical order (§3.7).
    pub records: BTreeMap<(u64, [u8; 32]), &'s Record>,
    /// Records left out because what they name is not all in the file, per type (`SMY-W490`).
    pub left_out: BTreeMap<&'static str, usize>,
    /// The hashes of selected records naming a uid the snapshot does not hold (`SMY-W495`).
    pub dangling: BTreeSet<[u8; 32]>,
    /// The ids of the threads included.
    pub threads: BTreeSet<ThreadId>,
    /// The schema ids whose declarations are included.
    pub schemas: BTreeSet<SchemaId>,
    /// Units named by an included record without an edge: a relation's `note`, a
    /// commitment's `note`, a withdrawal's `reason`, a resolution's `note`, a contention's
    /// positions, a dating's basis and a redaction's reason (§2.3, gap B).
    pub annotations: BTreeSet<Uid>,
}

/// The units a file needs: `sel` closed under rule L, the edges asked for, rule R and the
/// annotation pull-in, to a fixpoint (§3.6).
///
/// Only units the snapshot holds are in the result. One that is named and not held stays
/// named, and the record naming it is counted as dangling when it is selected. Retracted units
/// are kept: export is exact, and a `retracts` edge needs both of its ends.
pub(crate) fn close(src: &Source<'_>, sel: &BTreeSet<Uid>, opts: &ExportOptions) -> BTreeSet<Uid> {
    let store = src.store;
    let g = store.adjacency();
    let kinds = match &opts.edges {
        Some(e) => EdgeSet::support().union(e),
        None => EdgeSet::all(),
    };
    let mut units = sel.clone();
    loop {
        let roots: Vec<_> = units.iter().filter_map(|u| g.id(u)).collect();
        let mut next: BTreeSet<Uid> = closure(g, &roots, &kinds)
            .into_iter()
            .filter_map(|n| g.uid(n))
            .filter(|u| store.contains_uid(u))
            .copied()
            .collect();
        next.extend(units.iter().copied());
        let rebuttals: Vec<Uid> = next.iter().flat_map(|u| store.rebuttals_of(u)).collect();
        next.extend(rebuttals);
        let picked = pick(src, &next, &opts.with);
        next.extend(
            picked
                .annotations
                .into_iter()
                .filter(|u| store.contains_uid(u)),
        );
        if next == units {
            return units;
        }
        units = next;
    }
}

/// The records selected over a closed set of units, one rule per type (§3.6's table).
///
/// Records 15 and 18 need `--with-text` and a manifest's carry and licence, which arrive with
/// TX-P11 step 6; until then they are never selected. Record 9 is never selected, and other
/// unknown records are the caller's: they go to `unattributed.cbor`, never to a keyed file.
pub(crate) fn pick<'s>(src: &Source<'s>, units: &BTreeSet<Uid>, with: &With) -> Picked<'s> {
    let store = src.store;
    let mut p = Picked::default();
    let held = |u: &Uid| store.contains_uid(u);

    // What the rules below test membership in, built first: rids of the relations kept, the
    // parts and manifests the units name, the corpus views kept.
    let mut rel: BTreeSet<Uid> = BTreeSet::new();
    let mut tids: BTreeSet<Tid> = BTreeSet::new();
    let mut mids: BTreeSet<Mid> = BTreeSet::new();
    let mut types: BTreeSet<SchemaId> = BTreeSet::new();
    let mut kinds: BTreeSet<RelKind> = BTreeSet::new();
    let mut requires: BTreeSet<SchemaId> = BTreeSet::new();
    let mut prefixes: BTreeSet<String> = BTreeSet::new();
    for e in &src.entries {
        match e.record {
            Record::Unit(c) if e.uid.as_ref().is_some_and(|u| units.contains(u)) => {
                types.insert(c.schema.clone());
                prefixes.extend(payload_prefixes(c));
                if let Some(s) = &c.source {
                    tids.extend(Store::part_of_reference(&s.reference));
                    mids.extend(s.manifest().copied());
                }
            }
            Record::Relation(r) if spans(r, units, store) => {
                rel.insert(r.uid());
                kinds.insert(r.kind.clone());
            }
            Record::View(v) if keeps_view(v, units, with) => {
                requires.extend(v.requires.iter().cloned());
            }
            _ => {}
        }
    }
    let threads: BTreeSet<ThreadId> = src
        .entries
        .iter()
        .filter_map(|e| match e.record {
            Record::Thread(t) if t.steps.iter().all(|s| units.contains(&s.unit)) => {
                Some(t.id.clone())
            }
            _ => None,
        })
        .collect();
    let datings: BTreeSet<Uid> = src
        .entries
        .iter()
        .filter_map(|e| match e.record {
            Record::Dating(d) if dates(&d.target, units, &tids, &mids) => Some(d.did().as_uid()),
            _ => None,
        })
        .collect();
    let mut contentions: BTreeSet<ContentionId> = src
        .entries
        .iter()
        .filter_map(|e| match e.record {
            Record::Contention(c) if units.contains(&c.over) => Some(c.id.clone()),
            _ => None,
        })
        .collect();
    // A detected contention is in the file as far as row 12 is concerned when its subject is,
    // and its positions are pulled in as a recorded one's are, so the recipient derives the
    // same id from the same units and the resolution still names something.
    for k in src.detected.iter().filter(|k| units.contains(&k.over)) {
        if contentions.insert(k.id.clone()) {
            p.annotations.extend(k.positions.iter().copied());
        }
    }
    let schema_needed = |s: &SchemaId| {
        src.schemas.get(s).is_some_and(|(t, k)| {
            t.iter().any(|x| types.contains(x)) || k.iter().any(|x| kinds.contains(x))
        }) || requires.contains(s)
            || domain(s).is_some_and(|d| prefixes.contains(d))
    };

    for e in &src.entries {
        let r = e.record;
        let mut names: Vec<Uid> = Vec::new();
        let keep = e.orphan
            || match r {
                Record::Unit(c) => {
                    let keep = e.uid.as_ref().is_some_and(|u| units.contains(u));
                    if keep {
                        names.extend(c.references().copied());
                    }
                    keep
                }
                Record::Attestation(a) => units.contains(&a.uid) || rel.contains(&a.uid),
                Record::Relation(x) => {
                    let keep = rel.contains(&x.uid());
                    if keep {
                        p.annotations.extend(x.note);
                        names.extend([x.from, x.to]);
                        names.extend(x.note);
                    }
                    keep
                }
                Record::Thread(t) => {
                    let keep = t.steps.iter().all(|s| units.contains(&s.unit));
                    if keep {
                        p.threads.insert(t.id.clone());
                    } else if t.steps.iter().any(|s| units.contains(&s.unit)) {
                        *p.left_out.entry(r.type_name()).or_default() += 1;
                    }
                    keep
                }
                Record::View(v) => keeps_view(v, units, with),
                Record::Contention(c) => {
                    let keep = units.contains(&c.over);
                    if keep {
                        p.annotations.extend(c.positions.iter().copied());
                        names.extend(c.positions.iter().copied());
                    }
                    keep
                }
                Record::PackInfo(x) => {
                    let named: Vec<&Uid> = x
                        .dropped
                        .iter()
                        .map(|(u, _)| u)
                        .chain(x.degraded.iter().map(|(u, _)| u))
                        .collect();
                    let keep = with.packs
                        && named.iter().all(|u| units.contains(u))
                        && x.thread.as_ref().is_none_or(|t| threads.contains(t));
                    if with.packs && !keep && named.iter().any(|u| units.contains(u)) {
                        *p.left_out.entry(r.type_name()).or_default() += 1;
                    }
                    keep
                }
                Record::SchemaDecl(d) => {
                    let keep = schema_needed(&d.id);
                    if keep {
                        p.schemas.insert(d.id.clone());
                    }
                    keep
                }
                Record::LabelBinding(b) => units.contains(&b.uid),
                Record::Withdrawal(w) => {
                    let keep = rel.contains(&w.relation) || datings.contains(&w.relation);
                    if keep {
                        p.annotations.extend(w.reason);
                        names.extend(w.reason);
                    }
                    keep
                }
                Record::Resolution(x) => {
                    let keep = match &x.target {
                        ResolutionTarget::Relation(rid) => rel.contains(rid),
                        ResolutionTarget::Contention(id) => contentions.contains(id),
                        _ => false,
                    };
                    if keep {
                        p.annotations.extend(x.note);
                        names.extend(x.note);
                    }
                    keep
                }
                Record::Commit(c) => {
                    let keep = units.contains(&c.unit) || datings.contains(&c.unit);
                    if keep {
                        p.annotations.extend(c.note);
                        names.extend(c.note);
                    }
                    keep
                }
                Record::Manifest(m) => mids.contains(&m.mid()),
                Record::Dating(d) => {
                    let keep = datings.contains(&d.did().as_uid());
                    if keep {
                        p.annotations.extend(d.basis);
                        names.extend(d.basis);
                    }
                    keep
                }
                Record::Redaction(x) => {
                    let keep = tids.contains(&x.tid);
                    if keep {
                        p.annotations.extend(x.reason);
                        names.extend(x.reason);
                    }
                    keep
                }
                // Text lives in an object store and needs `--with-text` (step 6); unknown records
                // are the caller's (`unattributed.cbor`); a checkpoint goes nowhere.
                Record::PartText(_) | Record::PartReading(_) | Record::Unknown { .. } => false,
                // A record type added to `smysl-core` after this table was written: never guessed
                // into a file. The exhaustive arms above are the ones the RFC rules on.
                #[allow(unreachable_patterns)]
                _ => false,
            };
        if keep {
            if e.orphan || names.iter().any(|u| !held(u)) {
                p.dangling.insert(e.hash);
            }
            p.records.insert((r.type_code(), e.hash), r);
        }
    }
    p
}

/// The prefixes of a unit's payload keys: `code` for `"code:kind"`.
///
/// A payload this build cannot read as a text-keyed map names no prefix. It is preserved
/// verbatim either way; what is lost is only the declaration travelling with it.
fn payload_prefixes(c: &smysl_core::UnitCore) -> Vec<String> {
    let Some(bytes) = &c.payload else {
        return Vec::new();
    };
    let Ok(o) = smysl_core::surface::payload_to_object(bytes) else {
        return Vec::new();
    };
    o.keys()
        .filter_map(|k| k.split_once(':').map(|(p, _)| p.to_string()))
        .collect()
}

/// An extension schema's domain: `code` for `x.code/v1`.
fn domain(s: &SchemaId) -> Option<&str> {
    match s {
        SchemaId::Extension(id) => id.strip_prefix("x.")?.split_once('/').map(|(d, _)| d),
        _ => None,
    }
}

/// Row 3: both ends are in the file, or one is and the other is held by no one.
///
/// A relation whose far end the snapshot does not hold is dangling at source. It goes where
/// its held end goes, and is counted (`SMY-W495`), rather than nowhere.
fn spans(r: &smysl_core::Relation, units: &BTreeSet<Uid>, store: &Store) -> bool {
    let end = |u: &Uid| units.contains(u) || !store.contains_uid(u);
    (units.contains(&r.from) || units.contains(&r.to)) && end(&r.from) && end(&r.to)
}

/// Row 5: a corpus view, with `--with-views`, when every root is in the file and its id is not
/// an earlier export's (`export/`, OQ-61).
fn keeps_view(v: &smysl_core::View, units: &BTreeSet<Uid>, with: &With) -> bool {
    with.views && !v.id.as_str().starts_with("export/") && v.roots.is_subset(units)
}

/// Row 17: whether a dating's target is in the file.
fn dates(
    t: &DatingTarget,
    units: &BTreeSet<Uid>,
    tids: &BTreeSet<Tid>,
    mids: &BTreeSet<Mid>,
) -> bool {
    match t {
        DatingTarget::Unit(u) => units.contains(u),
        DatingTarget::Part(tid) | DatingTarget::Window { tid, .. } => tids.contains(tid),
        DatingTarget::Manifest(m) => mids.contains(m),
        #[allow(unreachable_patterns)]
        _ => false,
    }
}

/// Whether an unknown record is a checkpoint, which no export file carries (row 9).
pub(crate) fn is_checkpoint(r: &Record) -> bool {
    matches!(r, Record::Unknown { code, .. } if *code == code::CHECKPOINT)
}
