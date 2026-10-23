//! Every revision of every schema id in a store (§3.8, §3.10).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::lens::{validate, LensChain, LensError};
use super::{decode, LensStep, Revision};
use crate::diag::{Code, Diagnostic, Report};
use crate::ids::SchemaId;
use crate::types::{Record, SchemaDecl};

/// The declarations of a store, decoded and grouped by id.
///
/// **Current** is the highest version (A-12.1). Two declarations of one `(id, version)` with
/// different bytes are a fork (`SMY-W429`), and that id has no current revision until a
/// resolution names the fork; two with the same bytes are one declaration seen twice, as a
/// merge of two stores that both hold it produces.
#[derive(Debug, Clone, Default)]
pub struct SchemaSet {
    revisions: BTreeMap<String, Vec<Revision>>,
    forks: BTreeSet<(String, u32)>,
}

impl SchemaSet {
    pub fn new() -> SchemaSet {
        SchemaSet::default()
    }

    pub fn from_records<'a>(
        records: impl IntoIterator<Item = &'a Record>,
        report: &mut Report,
    ) -> SchemaSet {
        SchemaSet::from_decls(
            records.into_iter().filter_map(|r| match r {
                Record::SchemaDecl(d) => Some(d),
                _ => None,
            }),
            report,
        )
    }

    pub fn from_decls<'a>(
        decls: impl IntoIterator<Item = &'a SchemaDecl>,
        report: &mut Report,
    ) -> SchemaSet {
        let mut set = SchemaSet::new();
        for d in decls {
            set.insert(decode(d, report));
        }
        for (id, v) in &set.forks {
            report.push(Diagnostic::new(Code::W429).with_message(format!(
                "{id}@{v}: two declarations of one version with different content; {id} has \
                 no current revision until the fork is resolved"
            )));
        }
        set
    }

    fn insert(&mut self, r: Revision) {
        let list = self.revisions.entry(r.id.as_str().to_string()).or_default();
        if list.iter().any(|x| x.decl_hash == r.decl_hash) {
            return;
        }
        if list.iter().any(|x| x.version == r.version) {
            self.forks.insert((r.id.as_str().to_string(), r.version));
        }
        list.push(r);
        list.sort_by_key(|a| (a.version, a.decl_hash));
    }

    /// The ids declared, in byte order.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.revisions.keys().map(String::as_str)
    }

    /// Every revision of an id, by version (forked versions appear once per declaration).
    pub fn revisions(&self, id: &SchemaId) -> &[Revision] {
        self.revisions
            .get(id.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The revision at a version, `None` when there is none or it is forked.
    pub fn revision(&self, id: &SchemaId, version: u32) -> Option<&Revision> {
        if self.forks.contains(&(id.as_str().to_string(), version)) {
            return None;
        }
        self.revisions(id).iter().find(|r| r.version == version)
    }

    /// The highest revision; `None` while any version of the id is forked (A-12.1 item 4).
    pub fn current(&self, id: &SchemaId) -> Option<&Revision> {
        if self.forks.iter().any(|(i, _)| i == id.as_str()) {
            return None;
        }
        self.revisions(id).last()
    }

    pub fn forks(&self) -> Vec<(SchemaId, u32)> {
        self.forks
            .iter()
            .filter_map(|(i, v)| SchemaId::parse(i).ok().map(|id| (id, *v)))
            .collect()
    }

    /// The schema id whose revisions declare a unit type, with the revisions that do. A type
    /// declared by two ids is ambiguous and governed by neither.
    pub fn governing(&self, ty: &SchemaId) -> Option<&SchemaId> {
        let mut found = None;
        for list in self.revisions.values() {
            if list.iter().any(|r| r.declares_type(ty)) {
                if found.is_some() {
                    return None;
                }
                found = Some(&list[0].id);
            }
        }
        found
    }

    /// Every distinct step any revision of the id carries, in `(from, to, bytes)` order.
    pub fn steps(&self, id: &SchemaId) -> Vec<&LensStep> {
        let mut v: Vec<&LensStep> = self
            .revisions(id)
            .iter()
            .flat_map(|r| r.lenses.iter())
            .collect();
        v.sort_by(|a, b| (a.from, a.to, a.bytes()).cmp(&(b.from, b.to, b.bytes())));
        v.dedup_by(|a, b| a.bytes() == b.bytes());
        v
    }

    /// The step path from `from` up to `to` (§3.10). Steps only go up, so a path is a chain
    /// of rising revisions; the shortest one is taken, the first in step order on a tie, so
    /// that every reader of one store takes the same path. No path is `SMY-E470`, and a step
    /// on it that is ill-formed against the revision it leaves is `SMY-E471`.
    pub fn chain(&self, id: &SchemaId, from: u32, to: u32) -> Result<LensChain, LensError> {
        if from > to {
            return Err(LensError::NoPath {
                id: id.clone(),
                from,
                to,
            });
        }
        let fields = |v: u32| {
            self.revision(id, v)
                .map(|r| r.fields.clone())
                .unwrap_or_default()
        };
        if from == to {
            return Ok(LensChain::build(
                id.clone(),
                from,
                to,
                Vec::new(),
                Vec::new(),
                fields(to),
            ));
        }
        let steps = self.steps(id);
        // Breadth-first over revisions, steps in their sorted order: shortest path, and the
        // first such in that order.
        let mut prev: BTreeMap<u32, usize> = BTreeMap::new();
        let mut queue = VecDeque::from([from]);
        let mut seen = BTreeSet::from([from]);
        while let Some(at) = queue.pop_front() {
            if at == to {
                break;
            }
            for (i, s) in steps.iter().enumerate() {
                if s.from == at && s.to <= to && seen.insert(s.to) {
                    prev.insert(s.to, i);
                    queue.push_back(s.to);
                }
            }
        }
        if !seen.contains(&to) {
            return Err(LensError::NoPath {
                id: id.clone(),
                from,
                to,
            });
        }
        let mut path = Vec::new();
        let mut at = to;
        while at != from {
            let s = steps[prev[&at]];
            path.push(s.clone());
            at = s.from;
        }
        path.reverse();
        for s in &path {
            validate(id, s, &fields(s.from))?;
        }
        let sources = path.iter().map(|s| fields(s.from)).collect();
        Ok(LensChain::build(
            id.clone(),
            from,
            to,
            path,
            sources,
            fields(to),
        ))
    }
}
