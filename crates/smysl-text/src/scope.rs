//! The query ports a library answers (SMYSL-2.5 §3.3, TX-P6 step 6).
//!
//! `smysl-query` plans and runs a query through three traits it cannot implement for a
//! catalogue it cannot see — [`Scope`], [`Times`] and [`Entities`] — and this file implements
//! all three over one open [`Library`]. The division is the one SMYSL-2.5 §3.3's table draws
//! in its last column: the query crate owns the question and every graph walk, and the library
//! owns the facts a plain store does not record — which manifest a unit came from and in what
//! language, when it was said, which entity a name is.
//!
//! # What the six scope items mean here
//!
//! SMYSL-2.5 names `texts`, `library`, `collection:`, `channels`, `channel:` and `import:` and
//! defines none of them, so each is defined below in terms of something the catalogue already
//! records, and the definitions are recorded in §3.3 beside the table that names them.
//!
//! - **`texts` and `library`** are the whole library. Two spellings of one scope, kept because
//!   SMYSL-2.6's examples write the first and SMYSL-2.5's the second.
//! - **A collection is an alias namespace.** `collection:bible` is every expression whose
//!   alias is `bible` or begins `bible/`. The alias grammar (`smysl_core::is_alias`) is
//!   segmented on `/` and `:` for exactly this use — every library in this repository already
//!   names its editions `kjv/08-rut`, `kjv/65-jud` — and a collection defined any other way
//!   would need a record type the format does not have.
//! - **`import:<name>` is the reserved namespace** SMYSL-2.3 A-5 keeps for imported manifests
//!   and SMYSL-2.7 writes as `import:<collection>/<stem>`. It is the collection rule applied to
//!   one prefix, which is why it is the same code path: a collection named `import:x` *is*
//!   `import:x`.
//! - **A channel is a conversation.** `channels` is every expression a chat reader produced.
//!   `channel:<name>` names one, and the name is found in two places because the readers put
//!   it in two: Telegram and WhatsApp export one conversation per file, so the expression's
//!   alias is the channel; Slack exports an archive of many, and the reader makes each
//!   conversation a `channel` level whose locator head is its name (`general.20240115.7`). A
//!   name is looked up both ways and the two answers are unioned, because a library with a
//!   Telegram chat aliased `general` and a Slack channel called `general` holds two
//!   conversations of that name, not one.
//! - **`store:<name>`** is answered when the name is this scope's own, as `StoreScope` answers
//!   it, so a query written against a plain store runs unchanged against the library that
//!   grew from it.
//!
//! # Partitions and analyzers
//!
//! §3.2 phase 4 keys a partition on `(manifest, lang)` and §3.4 says each is *"searched with
//! its own analyzer (SMYSL-2.4 chain id, recorded)"*. The chain is chosen by the `lang` half
//! of the key through [`Chain::new`], one per language rather than one per partition, and its
//! id goes into the [`Partition`] so the plan records which analyzer found the hits — one of
//! the eight recipe inputs of §3.6.
//!
//! # Time
//!
//! The `said` axis is rule E's effective time, computed once per scope over the whole store by
//! [`engine::effective`] and cached, because the engine is a network relaxation and a query
//! over a thousand units must not run it a thousand times. The `observed` axis is
//! `Store::observed_at`, which §3.3 lists as the existing function for exactly that axis.
//!
//! # Entities
//!
//! A name resolves to every `x.text/entity` unit whose gist is that name, then to every unit
//! in the same `x.text/same-as` class as one of those. The first hop is what "aliases" means
//! under A-2.4 rule 7 — an entity's identity is its name *in one expression*, so the same
//! person across three expressions is three units with one gist — and the second is the edge
//! the format provides for saying two differently named things are one.

use std::cell::OnceCell;
use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Diagnostic, Span};
use smysl_core::ids::{LangTag, Uid};
use smysl_core::types::Axis as CoreAxis;
use smysl_query::ast::{Name, ScopeItem};
use smysl_query::ports::{
    unresolvable, Axis, Bounds, Candidates, Entities, Partition, PartitionKey, Scope, Times,
};
use smysl_retrieve::{Analyze, Postings};

use crate::analyze::Chain;
use crate::library::Library;
use crate::locator::{self, Locator};
use crate::time::engine::{self, Effective};
use crate::time::{Bound, Interval, Subject};

/// The reader ids that read a conversation. One place, so a fourth chat reader is one line.
const CHAT_READERS: &[&str] = &["telegram/", "whatsapp/", "slack/"];

/// What the catalogue says about one unit: the expression it was drawn from and where in it.
#[derive(Debug, Clone)]
struct Placed {
    mid: String,
    alias: String,
    reader: String,
    lang: String,
    /// The head of a canonical locator, which for a Slack reading is the channel name.
    head: Option<String>,
}

/// The three ports over one library.
pub struct LibraryScope<'a> {
    name: String,
    library: &'a Library,
    placed: BTreeMap<Uid, Placed>,
    chains: BTreeMap<String, Chain>,
    /// The chain units the catalogue does not place are cut with.
    plain: Chain,
    /// Build the postings for each query rather than read the kept table: the comparison the
    /// two paths are held to.
    build_postings: bool,
    said: OnceCell<Effective>,
}

impl<'a> LibraryScope<'a> {
    /// This scope, building the postings each query reads rather than reading the table the
    /// library keeps: what shows the two give one answer (OQ-91), and nothing else.
    pub fn building_postings(mut self) -> LibraryScope<'a> {
        self.build_postings = true;
        self
    }

    /// A scope named `name` over `library`.
    ///
    /// The name is what `from store:<name>` must match, given rather than taken from the
    /// library's path for the reason `StoreScope` gives: a query names an identity the caller
    /// chose, not a directory.
    pub fn new(name: impl Into<String>, library: &'a Library) -> LibraryScope<'a> {
        let store = library.store();
        let mut placed: BTreeMap<Uid, Placed> = BTreeMap::new();
        let mut chains: BTreeMap<String, Chain> = BTreeMap::new();
        for (mid, manifest) in store.manifests() {
            for part in &manifest.parts {
                let lang = part.lang.as_ref().unwrap_or(&manifest.lang);
                chains
                    .entry(lang.to_string())
                    .or_insert_with(|| Chain::new(lang));
                for uid in store.units_with_tid(&part.tid) {
                    // A part two manifests share — an import of an edition already held —
                    // answers `units_with_tid` for both, and the unit's own source says which
                    // manifest it was drawn under. Placing it by that and not by the first
                    // manifest to claim the tid is what keeps a unit in one partition.
                    let drawn_here = store
                        .get(&uid)
                        .and_then(|u| u.core.source.as_ref())
                        .and_then(|s| s.manifest().cloned())
                        .is_none_or(|m| &m == mid);
                    if !drawn_here {
                        continue;
                    }
                    let head = store
                        .get(&uid)
                        .and_then(|u| u.core.source.as_ref())
                        .and_then(|s| s.reference.split_once('#'))
                        .and_then(|(_, l)| locator::parse(l).ok())
                        .and_then(|l| match l {
                            Locator::Canonical { head, .. } => Some(head),
                            _ => None,
                        });
                    placed.insert(
                        uid,
                        Placed {
                            mid: mid.canonical(),
                            alias: manifest.alias.clone(),
                            reader: manifest.reader.clone(),
                            lang: lang.to_string(),
                            head,
                        },
                    );
                }
            }
        }
        LibraryScope {
            name: name.into(),
            library,
            placed,
            chains,
            plain: Chain::plain(),
            build_postings: false,
            said: OnceCell::new(),
        }
    }

    fn all_uids(&self) -> BTreeSet<Uid> {
        self.library.store().units().map(|(uid, _)| *uid).collect()
    }

    fn candidates(&self, uids: BTreeSet<Uid>) -> Candidates {
        Candidates::of(vec![self.name.clone()], uids)
    }

    /// The units whose expression's alias is `name` or lies under `name/`.
    fn in_namespace(&self, name: &str) -> BTreeSet<Uid> {
        self.placed
            .iter()
            .filter(|(_, p)| {
                p.alias == name
                    || p.alias
                        .strip_prefix(name)
                        .is_some_and(|r| r.starts_with('/'))
            })
            .map(|(uid, _)| *uid)
            .collect()
    }

    fn is_chat(reader: &str) -> bool {
        CHAT_READERS.iter().any(|r| reader.starts_with(r))
    }

    fn refuse(&self, item: &ScopeItem, span: Span, why: &str) -> Diagnostic {
        unresolvable(item, span, &self.name, why)
    }

    fn said(&self) -> &Effective {
        self.said
            .get_or_init(|| engine::effective(self.library.store(), CoreAxis::Said))
    }
}

fn name_of(n: &Name) -> &str {
    match n {
        Name::Ident(s) | Name::Str(s) => s,
        // `Name` is `#[non_exhaustive]` in another crate. A spelling this build does not know
        // is a name nothing in the catalogue can match, and an empty one matches nothing.
        _ => "",
    }
}

fn bounds_of(i: Interval) -> Bounds {
    let at = |b: Bound| match b {
        Bound::At(instant) => Some(instant.0),
        Bound::Open => None,
    };
    Bounds::of(at(i.lo), at(i.hi))
}

impl Scope for LibraryScope<'_> {
    fn name(&self) -> &str {
        &self.name
    }

    fn resolve(&self, item: &ScopeItem, span: Span) -> Result<Candidates, Diagnostic> {
        match item {
            ScopeItem::Texts | ScopeItem::Library => Ok(self.everything()),
            ScopeItem::Store(n) => {
                if name_of(n) != self.name {
                    return Err(self.refuse(
                        item,
                        span,
                        &format!("the open library is `{}`", self.name),
                    ));
                }
                Ok(self.everything())
            }
            ScopeItem::Collection(n) => {
                let name = name_of(n);
                let known = self.library.catalog().keys().any(|alias| {
                    alias == name || alias.strip_prefix(name).is_some_and(|r| r.starts_with('/'))
                });
                if !known {
                    return Err(self.refuse(
                        item,
                        span,
                        "no expression in the catalogue has that alias or lies under it",
                    ));
                }
                Ok(self.candidates(self.in_namespace(name)))
            }
            ScopeItem::Import(n) => {
                let name = format!("import:{}", name_of(n));
                let known = self.library.catalog().keys().any(|alias| {
                    *alias == name
                        || alias
                            .strip_prefix(&name)
                            .is_some_and(|r| r.starts_with('/'))
                });
                if !known {
                    return Err(self.refuse(
                        item,
                        span,
                        "no imported expression in the catalogue has that collection name",
                    ));
                }
                Ok(self.candidates(self.in_namespace(&name)))
            }
            ScopeItem::Channels => {
                let any_chat = self
                    .library
                    .store()
                    .manifests()
                    .any(|(_, m)| LibraryScope::is_chat(&m.reader));
                if !any_chat {
                    return Err(self.refuse(
                        item,
                        span,
                        "no expression in the catalogue was read by a chat reader",
                    ));
                }
                Ok(self.candidates(
                    self.placed
                        .iter()
                        .filter(|(_, p)| LibraryScope::is_chat(&p.reader))
                        .map(|(uid, _)| *uid)
                        .collect(),
                ))
            }
            ScopeItem::Channel(n) => {
                let name = name_of(n);
                let uids: BTreeSet<Uid> = self
                    .placed
                    .iter()
                    .filter(|(_, p)| {
                        LibraryScope::is_chat(&p.reader)
                            && (p.alias == name || p.head.as_deref() == Some(name))
                    })
                    .map(|(uid, _)| *uid)
                    .collect();
                // Known by name, not by having units: a channel whose every message nobody
                // has drawn a unit from is still a channel, and an empty answer is the truth
                // about it. Unknown is a different fact.
                let known = self
                    .library
                    .store()
                    .manifests()
                    .any(|(_, m)| LibraryScope::is_chat(&m.reader) && m.alias == name)
                    || self.placed.values().any(|p| {
                        LibraryScope::is_chat(&p.reader) && p.head.as_deref() == Some(name)
                    });
                if !known {
                    return Err(self.refuse(
                        item,
                        span,
                        "no conversation in the catalogue has that alias, and no chat reading \
                         has a channel of that name",
                    ));
                }
                Ok(self.candidates(uids))
            }
            _ => Err(self.refuse(
                item,
                span,
                "a scope item this library does not know how to answer",
            )),
        }
    }

    fn everything(&self) -> Candidates {
        self.candidates(self.all_uids())
    }

    /// Phase 4, by `(manifest, lang)` from the catalogue, chain id recorded.
    fn partition(&self, uids: &BTreeSet<Uid>) -> Vec<Partition> {
        let mut out: BTreeMap<PartitionKey, BTreeSet<Uid>> = BTreeMap::new();
        let mut rest: BTreeSet<Uid> = BTreeSet::new();
        for uid in uids {
            match self.placed.get(uid) {
                Some(p) => {
                    out.entry(PartitionKey::Catalog {
                        manifest: p.mid.clone(),
                        lang: p.lang.clone(),
                    })
                    .or_default()
                    .insert(*uid);
                }
                None => {
                    rest.insert(*uid);
                }
            }
        }
        if !rest.is_empty() {
            out.insert(PartitionKey::Uncatalogued, rest);
        }
        out.into_iter()
            .map(|(key, uids)| {
                let chain = match &key {
                    PartitionKey::Catalog { lang, .. } => {
                        self.chains.get(lang).map(|c| c.id().to_string())
                    }
                    _ => None,
                };
                Partition::new(key, uids, chain)
            })
            .collect()
    }

    /// A partition's chain; for units the catalogue does not place, the plain chain, which is
    /// what the kept postings cut them with (OQ-91), so a query and its documents agree.
    fn analyzer(&self, key: &PartitionKey) -> Option<&dyn Analyze> {
        match key {
            PartitionKey::Catalog { lang, .. } => self.chains.get(lang).map(|c| c as &dyn Analyze),
            _ => Some(&self.plain),
        }
    }

    /// The library's postings, each unit cut by its own partition's analyzer (OQ-91): the kept
    /// table when the library has one current, or one built over `universe` alone by the same
    /// rule — the same answer either way, which `tests/sq_library.rs` holds the two paths to.
    fn postings(&self, universe: &BTreeSet<Uid>) -> Option<Box<dyn Postings + '_>> {
        if !self.build_postings {
            if let Some(kept) = self.library.postings() {
                return Some(Box::new(kept));
            }
        }
        Some(Box::new(self.library.postings_over(universe)))
    }

    /// `lang:<code>`: the catalogue's language for a placed unit, the unit's own for one the
    /// catalogue does not place. Compared on the primary subtag, so `lang:en` finds `en-GB`.
    fn restrict_lang(&self, code: &str, uids: &BTreeSet<Uid>) -> Option<BTreeSet<Uid>> {
        let primary = |tag: &str| tag.split('-').next().unwrap_or("").to_ascii_lowercase();
        let want = primary(code);
        let store = self.library.store();
        Some(
            uids.iter()
                .filter(|uid| {
                    let tag = match self.placed.get(uid) {
                        Some(p) => Some(p.lang.clone()),
                        None => store
                            .get(uid)
                            .and_then(|u| u.core.lang.as_ref())
                            .map(LangTag::to_string),
                    };
                    tag.is_some_and(|t| primary(&t) == want)
                })
                .copied()
                .collect(),
        )
    }
}

impl Times for LibraryScope<'_> {
    fn parse(&self, edtf: &str) -> Result<Bounds, String> {
        smysl_core::edtf::parse(edtf)
            .map(|v| bounds_of(crate::time::edtf::to_interval(&v)))
            .map_err(|e| e.to_string())
    }

    fn of(&self, uid: &Uid, axis: Axis) -> Bounds {
        match axis {
            Axis::Said => bounds_of(self.said().interval(&Subject::Unit(*uid))),
            Axis::Observed => match self.library.store().observed_at(uid) {
                Some(ms) => match i64::try_from(ms) {
                    Ok(ms) => bounds_of(Interval::at(crate::time::Instant(ms))),
                    Err(_) => Bounds::UNDATED,
                },
                None => Bounds::UNDATED,
            },
            // An axis this build does not know has no time on it for any unit.
            _ => Bounds::UNDATED,
        }
    }
}

impl Entities for LibraryScope<'_> {
    fn resolve(&self, name: &str) -> Vec<Uid> {
        smysl_query::store_scope::resolve_entity(self.library.store(), name)
    }

    /// One group per thing the name could mean, case-insensitively and merged across
    /// `same-as` — the store's records answer it, so the library answers it as a store does.
    fn candidates(&self, name: &str) -> Vec<Vec<Uid>> {
        smysl_query::store_scope::entity_candidates(self.library.store(), name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_collection_is_an_alias_namespace_and_not_a_prefix() {
        // `kjv/08-rut` lies under `kjv`; `kjvx` does not, and neither does `kj`.
        let under = |alias: &str, name: &str| {
            alias == name || alias.strip_prefix(name).is_some_and(|r| r.starts_with('/'))
        };
        assert!(under("kjv/08-rut", "kjv"));
        assert!(under("kjv", "kjv"));
        assert!(!under("kjvx/08-rut", "kjv"));
        assert!(!under("kjv/08-rut", "kj"));
    }

    #[test]
    fn bounds_keep_open_sides_open() {
        let b = bounds_of(Interval::UNDATED);
        assert!(b.is_undated());
        let b = bounds_of(Interval::of(10, 20));
        assert_eq!((b.lo, b.hi), (Some(10), Some(20)));
    }
}
