//! The ports the planner reaches the world through (SMYSL-2.5 §4.1, §4.2).
//!
//! **SMYSL-2.5 names `Scope` four times and never gives its signature.** §3.2 phase 3 says
//! `from` "resolves to a set of stores and a candidate uid set, through the `Scope` trait
//! (§4.2)", §4.1 puts it in this file, §4.2 lists it in `Env` as `&dyn Scope`, and §3.3's
//! table says `smysl-text` supplies the implementation for `texts`, `library`, `collection:`,
//! `channel` and `import:`. That is the whole specification. The trait below is written from
//! those four constraints and the shape is recorded here rather than inferred later; it is the
//! same gap, smaller, as the missing Appendix C that RFC SMYSL-2.9 was written to close.
//!
//! Three things the constraints decide:
//!
//! 1. **It is expressed in uids, never in a store.** A `Scope` answers *which units*, and
//!    `smysl-text`'s implementation answers from the catalog rather than from the log. Putting
//!    `&Store` in the signature would make every implementor take one it may not want.
//! 2. **It is object-safe**, because §4.2's `Env` holds `&dyn Scope`.
//! 3. **Phases 3 and 4 are separate methods**, because SMYSL-2.5 §3.2 makes them separate
//!    phases: scope resolution yields candidates, partitioning splits them. [`Scope::partition`]
//!    has a default returning one partition, which is §3.2 phase 4's own "else one partition"
//!    — so a scope with no catalog leaves it alone.

use std::collections::BTreeSet;

use smysl_core::diag::{Code, Diagnostic, Span};
use smysl_core::ids::Uid;

use crate::ast::{Name, ScopeItem};

/// What a scope item admitted: the stores it named and the units they hold.
///
/// `stores` is a list of names rather than open stores. SMYSL-2.5 §3.3 unions several stores
/// "into a scratch store until SMYSL-2.8's virtual union", which is TX-P6 step 3's work and
/// TX-P13's after it; what phase 3 owes is the record of *which* were named, so `--explain`
/// can print it and a saved query can record it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Candidates {
    pub stores: Vec<String>,
    pub uids: BTreeSet<Uid>,
}

impl Candidates {
    pub fn new() -> Candidates {
        Candidates::default()
    }

    /// Union, which is what a `from` with several items means (SMYSL-2.5 §3.3).
    pub fn absorb(&mut self, other: Candidates) {
        for s in other.stores {
            if !self.stores.contains(&s) {
                self.stores.push(s);
            }
        }
        self.uids.extend(other.uids);
    }
}

/// How a candidate set was split for phase 4.
///
/// `Uncatalogued` is not a degenerate case to apologise for: SMYSL-2.5 §3.2 phase 4 says the
/// split is by `(manifest, lang)` "when the catalog provides them, **else one partition**",
/// and a store with no manifests provides none. It is also where units a catalogued store
/// holds but no manifest claims end up, which is why it is named for what it is rather than
/// for being the whole of anything.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum PartitionKey {
    Uncatalogued,
    /// `(manifest, lang)`, both as text so this type needs no library dependency.
    Catalog {
        manifest: String,
        lang: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Partition {
    pub key: PartitionKey,
    pub uids: BTreeSet<Uid>,
}

/// What `df` and `N` are counted over (SMYSL-2.5 §3.2 phase 4).
///
/// The default is `Collection`, and the reasoning is §3.2's own: it is not a compromise
/// between the other two but the only one without a measured defect — per-partition IDF
/// favours small texts, and a library-wide `N` compresses the ratio between two terms' `idf`
/// toward 1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum IdfScope {
    Partition,
    #[default]
    Collection,
    Library,
}

impl IdfScope {
    pub fn as_str(self) -> &'static str {
        match self {
            IdfScope::Partition => "partition",
            IdfScope::Collection => "collection",
            IdfScope::Library => "library",
        }
    }
}

/// Phase 3 and phase 4 of the planner (SMYSL-2.5 §3.2).
pub trait Scope {
    /// What to call this scope in `--explain`.
    fn name(&self) -> &str;

    /// Which units one `from` item admits.
    ///
    /// An item this scope does not know is an error, never an empty set: a question asked of a
    /// corpus that is not here should say so rather than answer "nothing found", which is the
    /// failure that cannot be told from a true negative.
    fn resolve(&self, item: &ScopeItem, span: Span) -> Result<Candidates, Diagnostic>;

    /// Every unit this scope holds, for a query that wrote no `from` at all.
    ///
    /// `from` is optional (SMYSL-2.9 OQ-79) and a saved query records the scope resolved when
    /// it was saved, so the planner needs a name for "all of it" that is not a scope item.
    fn everything(&self) -> Candidates;

    /// Phase 4. The default is one partition, which is what a scope with no catalog has.
    fn partition(&self, uids: &BTreeSet<Uid>) -> Vec<Partition> {
        if uids.is_empty() {
            return Vec::new();
        }
        vec![Partition {
            key: PartitionKey::Uncatalogued,
            uids: uids.clone(),
        }]
    }
}

/// The spelling of a scope item, for a diagnostic and for `--explain`.
pub fn item_text(item: &ScopeItem) -> String {
    fn name(n: &Name) -> &str {
        match n {
            Name::Ident(s) | Name::Str(s) => s,
        }
    }
    match item {
        ScopeItem::Store(n) => format!("store:{}", name(n)),
        ScopeItem::Collection(n) => format!("collection:{}", name(n)),
        ScopeItem::Import(n) => format!("import:{}", name(n)),
        ScopeItem::Channel(n) => format!("channel:{}", name(n)),
        ScopeItem::Channels => "channels".to_string(),
        ScopeItem::Library => "library".to_string(),
        ScopeItem::Texts => "texts".to_string(),
    }
}

/// `SMY-E511` for a scope item this environment cannot answer.
pub fn unresolvable(item: &ScopeItem, span: Span, scope: &str, why: &str) -> Diagnostic {
    Diagnostic::at(Code::E511, span).with_message(format!(
        "`{}` is not something `{scope}` holds: {why}",
        item_text(item)
    ))
}
