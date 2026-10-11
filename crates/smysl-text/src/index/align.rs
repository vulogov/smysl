//! The `align` table of RFC SMYSL-2.4 §3.8: which locator of one expression is which of
//! another (TX-P4 step 4).
//!
//! # What a row is
//!
//! A pair of head manifests and, for every locator of the first that the scheme covers and the
//! second holds, the locator it becomes. That is the table TX-P7's `anchored` same-as engine
//! grounds on: "this verse and that verse are the same place" is what makes a proposed
//! `x.text/same-as` between two translations exact rather than judged.
//!
//! # Derived from the library, which is what makes the gate possible
//!
//! Three things decide a row, and all three are in the library: the two manifests' `versification`
//! declarations (key 14), the maps installed at `versification/*.tsv`, and the `locators` table.
//! Nothing comes from a command line. That is deliberate — see [`crate::versification`] — because
//! §3.8's gate is that a rebuild from the library alone reproduces the table, and a table that
//! depended on an argument would be reproducible only by whoever still had the argument.
//!
//! # Why only head manifests, and both directions
//!
//! A superseded manifest is not what the library says any more, and aligning it would double
//! every row for no question anybody asks. Both directions are recorded, because `align(a, b)`
//! and `align(b, a)` are different lookups and a caller holding one mid should not have to know
//! which way the map was written; the reverse comes from [`crate::versification::Scheme::reversed`],
//! which refuses a map that is not injective rather than dropping a row.
//!
//! # Uncovered locators are counted, not dropped quietly
//!
//! `SMY-W451`. A locator the scheme does not cover, or whose target the other manifest does not
//! hold, is not aligned — and the count is in the table, so `text align` can say "1,189 aligned,
//! 42 not" instead of reporting a success that silently covered half the book. A guessed
//! alignment would put a claim about one verse onto another, which is the one failure this table
//! must not have.

use std::collections::BTreeMap;

use smysl_core::ids::Mid;

use super::{LocatorTable, ManifestRow};
use crate::library::Library;
use crate::versification::{self, Scheme};

/// What the table holds for one ordered pair of manifests.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rows {
    /// The map this was built from, by its installed name.
    pub scheme: String,
    /// Locator in the first manifest → locator in the second.
    pub rows: BTreeMap<String, String>,
    /// How many of the first manifest's locators were not aligned (`SMY-W451`).
    pub uncovered: u32,
}

/// The table: rows per ordered pair of head manifests.
pub type Table = BTreeMap<(Mid, Mid), Rows>;

/// Build the whole table from the library's manifests, maps and locator tables.
///
/// `locators` is passed in rather than read again: it is the one input that costs object reads,
/// the caller has just built it, and reading it twice would mean a table built against a
/// different leniency than the one the index records.
pub fn rebuild(
    lib: &Library,
    manifests: &BTreeMap<Mid, ManifestRow>,
    locators: &BTreeMap<Mid, LocatorTable>,
) -> Table {
    let schemes = versification::installed(lib.root());
    if schemes.is_empty() {
        return Table::new();
    }
    // Head manifests with a declared numbering, in mid order. The numbering comes from the
    // manifest itself rather than from the `manifests` table, which does not carry key 14 — a
    // summary of what a catalog listing prints, and a listing does not print it.
    let mut declared: Vec<(Mid, String)> = Vec::new();
    for (mid, row) in manifests {
        if !row.head {
            continue;
        }
        if let Some(scheme) = lib
            .store()
            .manifest(mid)
            .and_then(|m| m.versification.clone())
        {
            declared.push((*mid, scheme));
        }
    }

    let mut out = Table::new();
    for (a, a_scheme) in &declared {
        for (b, b_scheme) in &declared {
            if a == b || a_scheme == b_scheme {
                continue;
            }
            let Some((name, map)) = pick(&schemes, a_scheme, b_scheme) else {
                continue;
            };
            let (Some(from), Some(to)) = (locators.get(a), locators.get(b)) else {
                // A manifest with no locator table has none for the reason the module on partial
                // indexes gives: one of its parts did not resolve. Aligning against half a
                // manifest would produce rows that look complete.
                continue;
            };
            out.insert((*a, *b), pair(&name, &map, from, to));
        }
    }
    out
}

/// The map that connects two numberings, forward or reversed.
///
/// The forward direction first, so that a library holding both `kjv→synodal` and
/// `synodal→kjv` uses the one written in the direction asked. Among several candidates the
/// first in name order wins, which is arbitrary and total — and `Rows::scheme` records which,
/// so an alignment that looks wrong can be traced to the map that produced it.
fn pick(schemes: &BTreeMap<String, Scheme>, from: &str, to: &str) -> Option<(String, Scheme)> {
    if let Some((name, s)) = schemes.iter().find(|(_, s)| s.connects(from, to)) {
        return Some((name.clone(), s.clone()));
    }
    schemes
        .iter()
        .find(|(_, s)| s.connects(to, from))
        .and_then(|(name, s)| s.reversed().map(|r| (name.clone(), r)))
}

/// One pair's rows: every locator of `from` the map covers and `to` holds.
fn pair(name: &str, map: &Scheme, from: &LocatorTable, to: &LocatorTable) -> Rows {
    let mut rows = BTreeMap::new();
    let mut uncovered = 0u32;
    for locator in from.keys() {
        match map.map(locator) {
            Some(target) if to.contains_key(&target) => {
                rows.insert(locator.clone(), target);
            }
            // Two different misses, one answer: a scheme that says nothing about this locator
            // and a scheme that maps it somewhere the other expression does not have. Both mean
            // *not aligned*, and `text align` reports the count rather than the reason — the
            // reason is in the map, which the caller is holding.
            _ => uncovered = uncovered.saturating_add(1),
        }
    }
    Rows {
        scheme: name.to_string(),
        rows,
        uncovered,
    }
}

/// Where a locator of `a` points in `b`, when the table can say.
pub fn locate(table: &Table, a: &Mid, b: &Mid, locator: &str) -> Option<String> {
    table.get(&(*a, *b))?.rows.get(locator).cloned()
}

/// How many rows the whole table holds.
pub fn rows(table: &Table) -> usize {
    table.values().map(|r| r.rows.len()).sum()
}

/// How many locators were left unaligned across the whole table (`SMY-W451`).
pub fn uncovered(table: &Table) -> u32 {
    table.values().map(|r| r.uncovered).sum()
}
