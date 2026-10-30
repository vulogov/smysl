//! The `Scope` implementation over a plain `Store` (TX-P6 step 2).
//!
//! SMYSL-2.5 §3.2 phase 3: *"Plain `Store`: `store:` and the whole store. `smysl-text`
//! (SMYSL-2.4) adds `texts`, `library`, `collection:`, `channel` and `import:`."* So this type
//! answers one of the grammar's seven scope items — `store:` — and refuses the other six by
//! name with `SMY-E461`, "not available in this build or phase", whose own definition in
//! SMYSL-2.5 §8 gives `text(…)` without `smysl-text` as an example of exactly this. The
//! eighth case is no `from` at all, which [`Scope::everything`] answers.
//!
//! §4.1's file table gained a row for this file in step 2; the traits stay in `src/ports.rs`.
//! The implementation is split out because it is the only thing here that depends on
//! `smysl-graph`, and a reader looking for "what does the port require of a backend" should
//! find the trait on its own.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic, Span};
use smysl_core::ids::Uid;
use smysl_graph::Store;

use crate::ast::{Name, ScopeItem};
use crate::ports::{item_text, unresolvable, Candidates, Entities, Partition, PartitionKey, Scope};

/// A scope over one open store.
///
/// The name is the store's, and it is what `from store:<name>` must match. It is given rather
/// than derived from the path: a store opened from `incidents.smy` and a store assembled in
/// memory should both be nameable, and a query that names a store is asking about an identity
/// the caller chose, not about a filename.
pub struct StoreScope<'a> {
    name: String,
    store: &'a Store,
}

impl<'a> StoreScope<'a> {
    pub fn new(name: impl Into<String>, store: &'a Store) -> StoreScope<'a> {
        StoreScope {
            name: name.into(),
            store,
        }
    }

    fn all_uids(&self) -> BTreeSet<Uid> {
        self.store.units().map(|(uid, _)| *uid).collect()
    }

    fn needs_text(item: &ScopeItem, span: Span) -> Diagnostic {
        Diagnostic::at(Code::E461, span).with_message(format!(
            "`{}` needs the library layer; this build has a plain store, which answers \
             `store:` and nothing else",
            item_text(item)
        ))
    }
}

impl Scope for StoreScope<'_> {
    fn name(&self) -> &str {
        &self.name
    }

    fn resolve(&self, item: &ScopeItem, span: Span) -> Result<Candidates, Diagnostic> {
        match item {
            ScopeItem::Store(n) => {
                let asked = match n {
                    Name::Ident(s) | Name::Str(s) => s.as_str(),
                };
                if asked != self.name {
                    return Err(unresolvable(
                        item,
                        span,
                        &self.name,
                        &format!("the open store is `{}`", self.name),
                    ));
                }
                Ok(Candidates {
                    stores: vec![self.name.clone()],
                    uids: self.all_uids(),
                })
            }
            _ => Err(StoreScope::needs_text(item, span)),
        }
    }

    fn everything(&self) -> Candidates {
        Candidates {
            stores: vec![self.name.clone()],
            uids: self.all_uids(),
        }
    }

    /// Phase 4, by `(manifest, lang)`.
    ///
    /// A part may override its manifest's language (`PartEntry::lang`, written only when they
    /// differ), so the key is taken per part and not per manifest — a bilingual expression
    /// partitions into two, which is the point of keying on the pair.
    ///
    /// A unit that no manifest's part claims lands in [`PartitionKey::Uncatalogued`]. A store
    /// with no manifests at all therefore gives exactly one partition holding everything,
    /// which is §3.2 phase 4's "else one partition" reached by the same code path rather than
    /// by a special case.
    fn partition(&self, uids: &BTreeSet<Uid>) -> Vec<Partition> {
        let mut out: BTreeMap<PartitionKey, BTreeSet<Uid>> = BTreeMap::new();
        let mut claimed: BTreeSet<Uid> = BTreeSet::new();

        for (mid, manifest) in self.store.manifests() {
            for part in &manifest.parts {
                let lang = part.lang.as_ref().unwrap_or(&manifest.lang);
                // The canonical mid, not `to_string`'s abbreviation. A partition key is part
                // of what a saved query records about its scope (SMYSL-2.9 OQ-79), and an
                // abbreviation is a prefix two manifests can share.
                let key = PartitionKey::Catalog {
                    manifest: mid.canonical(),
                    lang: lang.to_string(),
                };
                for uid in self.store.units_with_tid(&part.tid) {
                    if uids.contains(&uid) {
                        claimed.insert(uid);
                        out.entry(key.clone()).or_default().insert(uid);
                    }
                }
            }
        }

        let rest: BTreeSet<Uid> = uids.difference(&claimed).copied().collect();
        if !rest.is_empty() {
            out.insert(PartitionKey::Uncatalogued, rest);
        }

        out.into_iter()
            .map(|(key, uids)| Partition {
                key,
                uids,
                // A plain store is searched under the plain tokenizer. The chain id is left
                // unset rather than filled with `smysl/an/1`, because the chains are
                // `smysl-text`'s and a scope that cannot reach them should not name one.
                chain: None,
            })
            .collect()
    }
}

/// The entity port over a plain store (TX-P8 step 3): what a store's own records say, which is
/// everything entity resolution reads — `x.text/entity` units and the strict classes
/// `x.text/same-as` joins them into. A library adds a catalogue to a store and nothing to this,
/// so `smysl_text::scope::LibraryScope` answers the port with the same two functions.
impl Entities for StoreScope<'_> {
    fn resolve(&self, name: &str) -> Vec<Uid> {
        resolve_entity(self.store, name)
    }

    fn candidates(&self, name: &str) -> Vec<Vec<Uid>> {
        entity_candidates(self.store, name)
    }
}

fn is_entity(store: &Store, uid: &Uid) -> bool {
    store
        .get(uid)
        .is_some_and(|u| u.core.schema.as_str() == smysl_core::schemas::text::ENTITY)
}

/// `about entity(name)`: the entities spelled `name` exactly, and every entity their strict
/// classes hold (SMYSL-2.5 §3.3).
pub fn resolve_entity(store: &Store, name: &str) -> Vec<Uid> {
    use smysl_graph::proposition::{classes, Policy};
    let entity = smysl_core::schemas::text::ENTITY;
    let named: BTreeSet<Uid> = store
        .units()
        .filter(|(_, u)| u.core.schema.as_str() == entity && u.core.gist == name)
        .map(|(uid, _)| *uid)
        .collect();
    if named.is_empty() {
        return Vec::new();
    }
    let mut out = named.clone();
    for class in classes(store, Policy::Strict, None) {
        if class.members.iter().any(|m| named.contains(m)) {
            out.extend(class.members.iter().filter(|m| is_entity(store, m)));
        }
    }
    out.into_iter().collect()
}

/// What a dossier subject could mean: one group per thing (SMYSL-2.5 §3.13 step 1).
///
/// Each entity spelled `name` — case-insensitively by the pinned [`crate::fold`], since a question is typed by a person —
/// with every entity its strict class joins it to; groups that share a member are one thing and
/// are merged, so a same-as chain is never reported as two candidates.
pub fn entity_candidates(store: &Store, name: &str) -> Vec<Vec<Uid>> {
    let want = crate::fold::fold(name);
    entity_candidates_where(store, |gist| crate::fold::fold(gist) == want)
}

/// [`entity_candidates`] for the entities whose spelling `named` accepts: one group per thing,
/// merged across strict `same-as` classes. The seam a port that matches more loosely than a
/// spelling — by stems, say — builds its candidates on, so its groups are formed the same way.
pub fn entity_candidates_where(store: &Store, named: impl Fn(&str) -> bool) -> Vec<Vec<Uid>> {
    use smysl_graph::proposition::{classes, Policy};
    let entity = smysl_core::schemas::text::ENTITY;
    let named: Vec<Uid> = store
        .units()
        .filter(|(_, u)| u.core.schema.as_str() == entity && named(&u.core.gist))
        .map(|(uid, _)| *uid)
        .collect();
    if named.is_empty() {
        return Vec::new();
    }
    let strict = classes(store, Policy::Strict, None);
    let mut groups: Vec<BTreeSet<Uid>> = Vec::new();
    for uid in named {
        let mut group: BTreeSet<Uid> = BTreeSet::from([uid]);
        for class in &strict {
            if class.members.contains(&uid) {
                group.extend(class.members.iter().filter(|m| is_entity(store, m)));
            }
        }
        let (touching, mut rest): (Vec<_>, Vec<_>) =
            groups.into_iter().partition(|g| !g.is_disjoint(&group));
        for g in touching {
            group.extend(g);
        }
        rest.push(group);
        groups = rest;
    }
    let mut out: Vec<Vec<Uid>> = groups
        .into_iter()
        .map(|g| g.into_iter().collect())
        .collect();
    out.sort();
    out
}
