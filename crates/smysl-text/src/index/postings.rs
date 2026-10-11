//! The `postings` table of RFC SMYSL-2.4 §3.8, partitioned (TX-P4 step 2).
//!
//! # What it is for
//!
//! `find` spends its time building an index rather than searching one: 7.06 s of 7.59 s at 155k
//! units (SMYSL-2.8 M-5, F-10). The terms of a unit are a fact about the unit, so that work
//! belongs in a table beside the other eleven, with the two builders every table here has.
//!
//! The shape is `smysl_retrieve::MemPostings` per partition, not a shape of this module's own.
//! That is deliberate: the seam's in-memory implementation is what `find` compares against, and
//! a persistent table of a *different* shape would have to be converted into it to be scored,
//! which is the conversion that quietly disagrees. Here the persistent table **is** the
//! in-memory one, written down.
//!
//! # The partition, and what it is derivable from today
//!
//! §3.8 partitions by manifest × language. A unit says neither. What a unit says is a
//! `source.reference`, which names a **part** (`Store::part_of_reference`), and the part is
//! recorded by one or more manifests, each of which has exactly one `lang`. So the partition of
//! a unit is found by going through the `by_part` and `manifests` tables, and where several
//! manifests record the part the **head** one decides — the current reading of the text is the
//! one its units belong to.
//!
//! The language is in the key although it is a function of the mid today, and that is not
//! redundancy waiting to be cleaned up: FC-1 (TX-P5 step 1) gives a unit its own `lang`, and a
//! manifest then holds units of two languages at once — a pivot translation is exactly that. The
//! key is the one the corpus will need, filled from the manifest for as long as the manifest is
//! the only thing that knows.
//!
//! A unit whose source names no part, or names one no manifest records, is in [`Partition::Loose`].
//! Nothing is left out of the table: a library's store holds whatever was appended to it, and a
//! retrieval index that silently skipped half a corpus would look exactly like a query that
//! matched nothing.
//!
//! # Why partitions, when the scoring sums over all of them anyway
//!
//! Document frequency, document count and total length are **additive**, so summing a term's
//! `df` across every partition recovers the whole-corpus statistic exactly — which is what makes
//! "results identical to the in-memory path" a reachable claim rather than an approximate one.
//! The partition is a unit of *storage and maintenance*, not of scoring: re-ingesting one text
//! replaces one partition, and a per-language analyzer applies to exactly one.
//!
//! # Which analyzer, and the one place the two paths part company
//!
//! A partition's terms are produced by [`crate::analyze::Chain`] for its language — the whole
//! point of partitioning by language — and the id is recorded per partition. The in-memory path
//! `find` uses today analyses the *query and every document* with one tokeniser. So the two
//! paths agree exactly when the table is analysed under one chain and the query is split by the
//! same one, and they are **meant** to differ when they are not: per-document routing is the
//! improvement, not a divergence. `find --analyzer` is how a caller asks for the comparable
//! thing, and `text index` prints the ids a table actually holds.

use std::collections::BTreeMap;

use smysl_core::ids::{LangTag, Mid, Tid};
use smysl_core::types::Record;
use smysl_core::Uid;
use smysl_graph::Store;
use smysl_retrieve::{MemPostings, Postings};

use super::ManifestRow;
use crate::analyze::Chain;
use crate::library::Library;

/// One partition of the postings table.
///
/// `Ord` is what the dump is written in, so the derive order — `Text` before `Loose`, mid then
/// lang — is the file order. It has no meaning beyond being total.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Partition {
    /// Units extracted from the parts of one manifest, in one language.
    Text { mid: Mid, lang: String },
    /// Units whose source names no part of this library.
    ///
    /// Not a fallback to be ashamed of: a library's store takes any record, and until TX-P5
    /// every unit in one got there some other way. It is one partition rather than none so that
    /// the corpus-wide statistics are over the corpus.
    Loose,
}

impl Partition {
    /// The analyzer for a partition: the chain of its language, the plain chain for loose units.
    pub fn chain(&self) -> Chain {
        match self {
            Partition::Text { lang, .. } => match LangTag::new(lang) {
                Ok(tag) => Chain::new(&tag),
                // A manifest cannot hold an unparseable tag — `Manifest::lang` is a `LangTag` —
                // so this is reachable only from a sidecar somebody edited. The plain chain is
                // the honest answer to "which language", not a guess at one.
                Err(_) => Chain::plain(),
            },
            Partition::Loose => Chain::plain(),
        }
    }
}

impl core::fmt::Display for Partition {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Partition::Text { mid, lang } => write!(f, "{} {}", mid.short(), lang),
            Partition::Loose => f.write_str("(loose)"),
        }
    }
}

/// The table: one postings list set per partition.
pub type Table = BTreeMap<Partition, MemPostings>;

/// The partition a unit belongs to, from the two tables that know.
///
/// A function of the final state of `by_part` and `manifests`, never of arrival order — which is
/// what lets the two builders agree. Where several manifests record the part, the first **head**
/// one in mid order decides; where none of them is a head (every manifest of the part
/// superseded, which a chain of corrections can produce), the first in mid order does, because
/// answering "no partition" would drop the unit out of the corpus.
pub fn partition_of(
    store: &Store,
    manifests: &BTreeMap<Mid, ManifestRow>,
    by_part: &BTreeMap<Tid, Vec<Mid>>,
    uid: &Uid,
) -> Partition {
    let Some(unit) = store.get(uid) else {
        return Partition::Loose;
    };
    let Some(source) = unit.core.source.as_ref() else {
        return Partition::Loose;
    };
    let Some(tid) = Store::part_of_reference(&source.reference) else {
        return Partition::Loose;
    };
    let Some(mids) = by_part.get(&tid) else {
        return Partition::Loose;
    };
    let chosen = mids
        .iter()
        .find(|mid| manifests.get(mid).is_some_and(|row| row.head))
        .or_else(|| mids.first());
    match chosen.and_then(|mid| manifests.get(mid).map(|row| (mid, row))) {
        Some((mid, row)) => Partition::Text {
            mid: *mid,
            lang: row.lang.clone(),
        },
        None => Partition::Loose,
    }
}

/// Build the whole table from the library's units.
///
/// `O(units)` analyses, which is the cost this table exists to pay once. It reads no object: a
/// unit's text is in the catalog, in the record that declared it.
pub fn rebuild(
    lib: &Library,
    manifests: &BTreeMap<Mid, ManifestRow>,
    by_part: &BTreeMap<Tid, Vec<Mid>>,
) -> Table {
    let store = lib.store();
    let mut out = Table::new();
    for (uid, _) in store.units() {
        index_one(store, manifests, by_part, &mut out, uid);
    }
    out
}

/// Bring the table forward over records that were just appended.
///
/// Two arms, for two different events:
///
/// - a **unit** is analysed and added to its partition, which is `O(batch)`;
/// - a **manifest** can move units that are already indexed, because it may record a part
///   nothing recorded before (its units were loose) or supersede the manifest that was deciding
///   a partition. That is the one step here that reaches backwards, for the same reason
///   `manifests_on_append` does: `head` is a fact about other manifests.
///
/// There is no redaction arm. Rule Z unlinks a part's *objects*; the units extracted from it are
/// still units of the corpus and still retrievable, and a redaction that silently emptied them
/// out of the index would make `find` answer a question nobody asked.
pub fn on_append(
    table: &mut Table,
    lib: &Library,
    manifests: &BTreeMap<Mid, ManifestRow>,
    by_part: &BTreeMap<Tid, Vec<Mid>>,
    records: &[Record],
) {
    let store = lib.store();
    for record in records {
        match record {
            Record::Unit(core) => {
                let uid = smysl_core::canonical_uid(core);
                forget(table, &uid);
                index_one(store, manifests, by_part, table, &uid);
            }
            Record::Manifest(m) => {
                for entry in &m.parts {
                    for uid in store.units_with_tid(&entry.tid) {
                        forget(table, &uid);
                        index_one(store, manifests, by_part, table, &uid);
                    }
                }
            }
            _ => {}
        }
    }
    table.retain(|_, row| row.docs() > 0);
}

/// Analyse one unit into its partition.
fn index_one(
    store: &Store,
    manifests: &BTreeMap<Mid, ManifestRow>,
    by_part: &BTreeMap<Tid, Vec<Mid>>,
    table: &mut Table,
    uid: &Uid,
) {
    let Some(text) = smysl_retrieve::postings::document_text(store, uid) else {
        return;
    };
    let partition = partition_of(store, manifests, by_part, uid);
    let chain = partition.chain();
    let terms = smysl_retrieve::Analyze::terms(&chain, &text);
    table
        .entry(partition)
        .or_insert_with(|| MemPostings::new(&chain))
        .add(*uid, &terms);
}

/// Drop a uid from whichever partition holds it.
///
/// A scan of the partitions rather than a reverse map, and that is a measured trade rather than
/// a shortcut: the reverse map would be a second structure to persist and to keep in agreement,
/// for a lookup that happens once per appended unit over a partition count in the thousands at
/// worst. Each probe is a `BTreeMap` hit on the length table and costs nothing.
fn forget(table: &mut Table, uid: &Uid) {
    for row in table.values_mut() {
        if row.doc_len(uid).is_some() {
            row.remove(uid);
        }
    }
}

/// Every analyzer id the table holds, in order, without repetition.
///
/// What `text index` prints and what a caller choosing `--analyzer` needs: a table under one
/// chain can be compared with the in-memory path, and a table under several is answering a
/// mixed-language corpus the way it should be answered.
pub fn analyzers(table: &Table) -> Vec<&str> {
    let mut out: Vec<&str> = table.values().filter_map(|p| p.analyzer()).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The whole table as one [`Postings`], summing the partitions.
///
/// The sums are exact: `df`, `docs` and `total_len` are counts over disjoint document sets, and a
/// unit is in exactly one partition. So a view over every partition answers precisely what a
/// single unpartitioned table would have, which is the property the equality with the in-memory
/// path rests on.
pub struct View<'t> {
    parts: Vec<&'t MemPostings>,
}

impl<'t> View<'t> {
    /// Every partition.
    pub fn all(table: &'t Table) -> View<'t> {
        View {
            parts: table.values().collect(),
        }
    }

    /// Only the partitions analysed under `analyzer`.
    ///
    /// For the comparison with the in-memory path, and for a caller that wants one language's
    /// text out of a mixed library. An empty view is a possible and honest answer: it means the
    /// library holds no partition under that chain, which is a different sentence from "nothing
    /// matched".
    pub fn under(table: &'t Table, analyzer: &str) -> View<'t> {
        View {
            parts: table
                .values()
                .filter(|p| p.analyzer() == Some(analyzer))
                .collect(),
        }
    }

    /// How many partitions this view spans.
    pub fn partitions(&self) -> usize {
        self.parts.len()
    }
}

impl Postings for View<'_> {
    fn df(&self, term: &str) -> u32 {
        self.parts.iter().map(|p| p.df(term)).sum()
    }

    fn postings(&self, term: &str) -> Vec<(Uid, u32)> {
        let mut out: Vec<(Uid, u32)> = self.parts.iter().flat_map(|p| p.postings(term)).collect();
        // Uid order across partitions, as the seam requires. Within a partition the list is
        // already sorted; between them it is not, and a merge would be the same sort with more
        // code.
        out.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        out
    }

    fn doc_len(&self, uid: &Uid) -> Option<u32> {
        self.parts.iter().find_map(|p| p.doc_len(uid))
    }

    /// Asked of each partition in turn, not of the merged list.
    ///
    /// The seam's default answer materialises a postings list, and a term in every document has
    /// a posting in every document — so leaving it to the default would make decomposing ten
    /// hits' scores copy the corpus twenty times. A unit is in exactly one partition, so the
    /// first answer is the answer.
    fn tf(&self, term: &str, uid: &Uid) -> u32 {
        self.parts
            .iter()
            .map(|p| p.tf(term, uid))
            .find(|tf| *tf > 0)
            .unwrap_or(0)
    }

    fn docs(&self) -> u32 {
        self.parts.iter().map(|p| p.docs()).sum()
    }

    fn total_len(&self) -> u64 {
        self.parts.iter().map(|p| p.total_len()).sum()
    }

    /// The id every partition agrees on, or `None` when they do not.
    ///
    /// `None` is "this view does not say", which is what the seam documents it to mean. A mixed
    /// view claiming one of its ids would make a query split by that chain look like a match for
    /// the whole table.
    fn analyzer(&self) -> Option<&str> {
        let first = self.parts.first()?.analyzer()?;
        self.parts
            .iter()
            .all(|p| p.analyzer() == Some(first))
            .then_some(first)
    }
}
