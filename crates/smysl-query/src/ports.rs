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
//!
//! # Two more ports, in TX-P6 step 6
//!
//! Step 6 lowers the stages that need the library layer, and the layer is in `smysl-text`,
//! which depends on this crate for the traits above — so nothing step 6 enables could be
//! *called* from here. Each capability therefore crosses a port, and the division is the same
//! one every time: this crate owns the question and the graph walk, `smysl-text` owns the fact.
//!
//! - [`Times`] answers what a unit's effective time is and what an EDTF literal means. Rule E,
//!   the locks, the STN and the EDTF grammar are all SMYSL-2.4's, and a second EDTF parser here
//!   would eventually refuse a query the library would accept.
//! - [`Entities`] answers which `x.text/entity` units a name is about, through aliases and
//!   `x.text/same-as`. The `x.text/mentions` walk stays here, because §3.3 lists it as an
//!   existing function of a store this crate already reads.
//! - [`Scope`] gains [`Scope::analyzer`] and [`Scope::restrict_lang`], both defaulted, because
//!   they are facts about a catalog and `StoreScope` has none. Adding them to the existing
//!   trait rather than making a third one keeps the answer and the thing it is an answer about
//!   together: a per-partition analyzer is chosen by the `lang` half of the key that scope
//!   itself produced.

use std::collections::BTreeSet;

use smysl_core::diag::{Code, Diagnostic, Span};
use smysl_core::ids::Uid;
use smysl_retrieve::{Analyze, Postings};

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

    /// The units `stores` admitted. The constructor a `Scope` in another crate needs, since
    /// the struct is `#[non_exhaustive]` and a literal of it is this crate's alone.
    pub fn of(stores: Vec<String>, uids: BTreeSet<Uid>) -> Candidates {
        Candidates { stores, uids }
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
    /// The analyzer chain this partition is searched under (SMYSL-2.4 chain id).
    ///
    /// SMYSL-2.5 §3.4: *"Each partition is searched with its own analyzer (SMYSL-2.4 chain id,
    /// recorded)."* Recorded here rather than looked up when a source runs, because the id is
    /// one of §3.6's eight recipe inputs: a saved query that did not record which analyzer
    /// found its hits cannot be checked against a rerun. `None` is the honest answer for a
    /// scope with no catalog — a plain store does not know what language its units are in, so
    /// it is searched under the plain tokenizer and says so by saying nothing.
    pub chain: Option<String>,
}

impl Partition {
    pub fn new(key: PartitionKey, uids: BTreeSet<Uid>, chain: Option<String>) -> Partition {
        Partition { key, uids, chain }
    }
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
            chain: None,
        }]
    }

    /// The analyzer one partition is searched under (SMYSL-2.5 §3.4).
    ///
    /// `None` means "the plain tokenizer", which is what a scope with no catalog can offer:
    /// an analyzer chain is chosen by language and a plain store records no language. The
    /// method takes the key rather than the whole [`Partition`] because that is what the
    /// choice depends on — §3.2 phase 4 keys partitions on `(manifest, lang)`, and the `lang`
    /// half is the half an analyzer is selected by.
    ///
    /// Returning a borrowed `&dyn Analyze` is deliberate: the chains live in the scope, there
    /// is one per language rather than one per partition, and an owned analyzer per call would
    /// rebuild a stemmer for every query.
    fn analyzer(&self, _key: &PartitionKey) -> Option<&dyn Analyze> {
        None
    }

    /// The postings a lexical source reads over `universe` (OQ-91; SMYSL-2.5 §3.4 as amended at
    /// TX-P13): one table in which **each unit's terms are cut by its own partition's
    /// analyzer**, so its statistics over any set of units are the sum of those units' own.
    ///
    /// A catalogued scope answers from the postings it keeps, which is what makes a query's cost
    /// its candidates' rather than the store's: building a table over the whole store, once per
    /// analyzer, for every query was 13 s over twenty works of a 10⁶-unit library and 150 s over
    /// all of them (SMYSL-2.8 GE-T7's stand-in). One with none kept builds one over `universe`
    /// alone, by the same rule, so the two give the same answer bit for bit.
    ///
    /// `None`, the default, is a scope with no catalog: the source then builds one table per
    /// analyzer over the store, as it always has.
    fn postings(&self, _universe: &BTreeSet<Uid>) -> Option<Box<dyn Postings + '_>> {
        None
    }

    /// `find "…" lang:<code>`: the units of `uids` that are in that language.
    ///
    /// `None` means this scope has no language facts, and the planner then refuses the source
    /// option with `SMY-E461` rather than ranking over everything — a query that asked for one
    /// language and silently got all of them is answered wrongly and says nothing about it.
    ///
    /// Separate from [`Scope::partition`] although both read the catalog's `lang`, because
    /// they are asked at different times and for different reasons: partitioning splits a
    /// candidate set for fusion, and this restricts it before a source ever scores. A query
    /// with `lang:ru` over three languages has one partition to rank, not three with two
    /// empty.
    fn restrict_lang(&self, _code: &str, _uids: &BTreeSet<Uid>) -> Option<BTreeSet<Uid>> {
        None
    }
}

/// A half-open interval of milliseconds since the Unix epoch, `[lo, hi)`.
///
/// `None` on a side is *open* on that side, and both open is **undated** — not early, not late
/// (SMYSL-2.3 A-2.4 rule 5). The distinction is the whole reason `SMY-W415` exists: an undated
/// unit is excluded from a time-scoped result and counted, rather than failing a comparison it
/// was never in.
///
/// Milliseconds and `i64` rather than SMYSL-2.4's `Interval` because this crate must not depend
/// on `smysl-text` — the implementation of [`Times`] lives there and would be a cycle. It is the
/// same choice [`PartitionKey::Catalog`] makes by carrying its manifest as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Bounds {
    pub lo: Option<i64>,
    pub hi: Option<i64>,
}

impl Bounds {
    pub const UNDATED: Bounds = Bounds { lo: None, hi: None };

    pub fn of(lo: Option<i64>, hi: Option<i64>) -> Bounds {
        Bounds { lo, hi }
    }

    pub fn is_undated(&self) -> bool {
        self.lo.is_none() && self.hi.is_none()
    }

    /// `during`: every instant of this interval is inside `range`.
    ///
    /// An open side of `self` is contained only by an open side of `range`: "from 1984 onwards"
    /// is not during 1984, and treating the open bound as the range's own end would turn an
    /// unbounded claim into a bounded one.
    pub fn during(&self, range: &Bounds) -> bool {
        let lo_ok = match (self.lo, range.lo) {
            (_, None) => true,
            (Some(a), Some(b)) => a >= b,
            (None, Some(_)) => false,
        };
        let hi_ok = match (self.hi, range.hi) {
            (_, None) => true,
            (Some(a), Some(b)) => a <= b,
            (None, Some(_)) => false,
        };
        lo_ok && hi_ok
    }

    /// `overlaps`: the two intervals share an instant. Half-open, so touching is not sharing.
    pub fn overlaps(&self, range: &Bounds) -> bool {
        let starts_before_other_ends = match (self.lo, range.hi) {
            (Some(a), Some(b)) => a < b,
            _ => true,
        };
        let ends_after_other_starts = match (self.hi, range.lo) {
            (Some(a), Some(b)) => a > b,
            _ => true,
        };
        starts_before_other_ends && ends_after_other_starts
    }

    /// `before`: this interval ends at or before `range` begins.
    pub fn before(&self, range: &Bounds) -> bool {
        match (self.hi, range.lo) {
            (Some(a), Some(b)) => a <= b,
            _ => false,
        }
    }

    /// `after`: this interval begins at or after `range` ends.
    pub fn after(&self, range: &Bounds) -> bool {
        match (self.lo, range.hi) {
            (Some(a), Some(b)) => a >= b,
            _ => false,
        }
    }
}

/// Effective time, for the `said` and `observed` stages of phase 6 (SMYSL-2.5 §3.3).
///
/// Two methods, because a time stage compares two things this crate can compute neither of.
/// The unit's interval is SMYSL-2.4's rule E engine — datings, locks, contentions, an STN —
/// and the query's is an EDTF literal, whose parser is also SMYSL-2.4's and deliberately not
/// re-implemented here: `SMY-E410` is the library's code for a malformed one, and two parsers
/// of the same grammar would eventually disagree about an edge case and refuse different
/// queries in different builds.
pub trait Times {
    /// The EDTF value or range a query wrote, as bounds.
    ///
    /// `Err` carries the message of a `SMY-E410`; the planner attaches the span, which this
    /// port does not have.
    fn parse(&self, edtf: &str) -> Result<Bounds, String>;

    /// A unit's effective interval on one axis, [`Bounds::UNDATED`] when it has none.
    fn of(&self, uid: &Uid, axis: Axis) -> Bounds;
}

/// Which time a stage is about (SMYSL-2.9 §4.6).
///
/// A copy of [`crate::ast::TimeAxis`] rather than a re-export, so an implementation of [`Times`]
/// in another crate does not take the whole AST with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Axis {
    Said,
    Observed,
}

/// `about entity("…")`: the entity units a name resolves to (SMYSL-2.5 §3.3, §3.5).
///
/// The port answers the half this crate cannot: which `x.text/entity` units *are* the thing the
/// query named, through aliases and `x.text/same-as`. What a caller does with them — the
/// `x.text/mentions` walk — is `Adjacency::edge_kind`, which §3.3 lists as existing and which
/// this crate already reaches, so it stays on the query side rather than crossing the port.
///
/// A name resolving to several entities is not an error. Two people called Mary are two
/// entities and a query naming "Mary" is about both; it is the dossier subject of `SMY-W417`
/// that has to be unambiguous, not a source.
pub trait Entities {
    fn resolve(&self, name: &str) -> Vec<Uid>;

    /// The entities a name could mean, one group per *thing*: the members of one group are
    /// joined by `x.text/same-as`, and two groups are two things that share a spelling.
    ///
    /// This is the question a dossier has to ask and a source does not (TX-P8 step 3): two
    /// groups for one subject is `SMY-W417`. The default is the honest answer for a port
    /// that cannot tell — everything [`Entities::resolve`] returns, as one thing — so an
    /// implementation that knows no classes never reports an ambiguity it cannot see.
    /// [`Entities::candidates`] may also match more loosely than `resolve` (case, for one),
    /// because a question is typed by a person and a query is written by one.
    fn candidates(&self, name: &str) -> Vec<Vec<Uid>> {
        let all = self.resolve(name);
        if all.is_empty() {
            Vec::new()
        } else {
            vec![all]
        }
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
