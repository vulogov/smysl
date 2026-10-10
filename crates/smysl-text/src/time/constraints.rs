//! Free constraints: the orderings a store implies without anybody writing a dating (D-3).
//!
//! Five, each with a fixed status, and the statuses are the point. A dating carries the status
//! of whatever evidence it names; these carry a status decided once, here, because they are not
//! claims anybody made — they are consequences of records already in the store. A quotation
//! cannot be better evidenced than the unit doing the quoting; a derivation and a supersession
//! are `cited`, because a manifest said so; a first-seen bound is `derived`, because this
//! implementation computed it from attestation clocks.
//!
//! | constraint | rule | status |
//! |---|---|---|
//! | quotation | if B quotes A then said(A) ≤ said(B) | the quoting unit's |
//! | derivation | a manifest is no earlier than its parent (key 11) | cited |
//! | supersession | a manifest is no earlier than the one it supersedes (key 13) | cited |
//! | reply | a reply is no earlier than the message it answers | cited |
//! | first seen | nothing is published after the earliest attestation of it | derived |
//!
//! **Two of the five have no data in this release and are not silently absent.** A *reply*
//! needs the reply ids a chat reader writes into a segment row (record 18, key 7), and a
//! segment row reaches a unit only through ingest (TX-P5); until then no unit in any store is
//! known to answer another. The *manifest* half of quotation and derivation works, because
//! manifests are records a store holds — but a derivation bound reaches the **units** under a
//! manifest only through `source.manifest`, which is FC-3 and also TX-P5. So this release
//! computes manifest orderings among manifests and unit orderings among units, and the bridge
//! between the two arrives with the field that names it. [`free`] reports what it could not
//! build rather than returning a shorter list.

use smysl_core::types::{RelKind, Status};
use smysl_graph::Store;

use super::{Instant, Subject, TimeStatus};

/// The relation an `x.text/v1` quotation is written as (A-11).
pub const QUOTES: &str = "x.text/quotes";

/// Which rule produced a constraint, for a why-chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FreeKind {
    Quotation,
    Derivation,
    Supersession,
    Reply,
    FirstSeen,
}

impl FreeKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            FreeKind::Quotation => "quotation",
            FreeKind::Derivation => "derivation",
            FreeKind::Supersession => "supersession",
            FreeKind::Reply => "reply",
            FreeKind::FirstSeen => "first-seen",
        }
    }
}

impl std::fmt::Display for FreeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

/// One free constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Free {
    /// `earlier` is no later than `later`.
    Order {
        kind: FreeKind,
        earlier: Subject,
        later: Subject,
        status: TimeStatus,
    },
    /// `subject` is no later than an instant the store already holds.
    NoLater {
        kind: FreeKind,
        subject: Subject,
        than: Instant,
        status: TimeStatus,
    },
}

impl Free {
    pub const fn kind(&self) -> FreeKind {
        match self {
            Free::Order { kind, .. } | Free::NoLater { kind, .. } => *kind,
        }
    }

    pub const fn status(&self) -> TimeStatus {
        match self {
            Free::Order { status, .. } | Free::NoLater { status, .. } => *status,
        }
    }
}

/// What [`free`] could not build, so that a caller can say so rather than infer it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Unbuilt {
    /// Manifests whose parent or superseded mid the store does not hold.
    ///
    /// Not an error: a bundle may carry a manifest and not its parent, and the ordering simply
    /// has one end missing. Counted because a corpus where this is common is a corpus whose
    /// chains were split, which is worth knowing before reading the dates.
    pub dangling_manifests: usize,
    /// Whether a reply constraint could have existed. Always true in this release: the ids a
    /// reply would be built from live in a reading, and no unit carries one yet (TX-P5).
    pub reply_needs_ingest: bool,
}

/// Every free constraint a store implies, in a deterministic order.
///
/// Sorted by kind and then by subject, so that two runs over the same store produce the same
/// list and therefore the same constraint ids — which is what makes a why-chain reproducible
/// (rule U).
pub fn free(store: &Store) -> (Vec<Free>, Unbuilt) {
    let mut out = Vec::new();
    let mut unbuilt = Unbuilt {
        dangling_manifests: 0,
        reply_needs_ingest: true,
    };

    // Quotation. The status is the quoting unit's, mapped from its epistemic status: a claim
    // somebody speculated about cannot order two publication dates as firmly as a measurement.
    for r in store.relations() {
        if r.kind != RelKind::Extension(QUOTES.to_string()) {
            continue;
        }
        let Some(quoting) = store.units().find(|(u, _)| **u == r.from).map(|(_, u)| u) else {
            continue;
        };
        out.push(Free::Order {
            kind: FreeKind::Quotation,
            earlier: Subject::Unit(r.to),
            later: Subject::Unit(r.from),
            status: of_status(quoting.core.status),
        });
    }

    // Derivation and supersession, between manifests.
    for (mid, m) in store.manifests() {
        for (kind, other) in [
            (FreeKind::Derivation, m.parent.map(|(mid, _)| mid)),
            (FreeKind::Supersession, m.supersedes),
        ] {
            let Some(other) = other else { continue };
            if store.manifest(&other).is_none() {
                unbuilt.dangling_manifests += 1;
                continue;
            }
            out.push(Free::Order {
                kind,
                earlier: Subject::Manifest(other),
                later: Subject::Manifest(*mid),
                status: TimeStatus::Cited,
            });
        }
    }

    // First seen. A unit was not said after the store first heard of it, and the store's own
    // record of first hearing is the earliest `ts` among its attestations — which is the
    // *known* axis, the one rule E never corrects, used here as a ceiling on *said*.
    for (uid, _) in store.units() {
        let earliest = store
            .attestations_of(uid)
            .iter()
            .map(|a| a.ts.wall_ms)
            .min();
        let Some(ms) = earliest else { continue };
        let Ok(ms) = i64::try_from(ms) else { continue };
        out.push(Free::NoLater {
            kind: FreeKind::FirstSeen,
            subject: Subject::Unit(*uid),
            // The *end* of the millisecond, because a unit said in the same millisecond it was
            // attested is not a contradiction and a half-open bound at `ms` would make it one.
            than: Instant(ms.saturating_add(1)),
            status: TimeStatus::Derived,
        });
    }

    // And the same ceiling for a manifest, through the parts it lists: a catalog entry was not
    // published after the store first heard of a unit drawn from one of its parts. This one
    // *does* work without FC-3, because a unit's `source.reference` names the part and the
    // manifest lists the part.
    for (mid, m) in store.manifests() {
        let earliest = m
            .parts
            .iter()
            .flat_map(|p| store.units_with_tid(&p.tid))
            .filter_map(|u| store.attestations_of(&u).iter().map(|a| a.ts.wall_ms).min())
            .min();
        let Some(ms) = earliest else { continue };
        let Ok(ms) = i64::try_from(ms) else { continue };
        out.push(Free::NoLater {
            kind: FreeKind::FirstSeen,
            subject: Subject::Manifest(*mid),
            than: Instant(ms.saturating_add(1)),
            status: TimeStatus::Derived,
        });
    }

    out.sort_by_key(|f| match f {
        Free::Order {
            kind,
            earlier,
            later,
            ..
        } => (*kind, *earlier, Some(*later), None),
        Free::NoLater {
            kind,
            subject,
            than,
            ..
        } => (*kind, *subject, None, Some(*than)),
    });
    out.dedup();
    (out, unbuilt)
}

/// A unit's epistemic status as a time status.
///
/// The same four names, and deliberately not the same enumeration — a unit's status is about
/// the claim and a time status is about the clock. The mapping is needed in exactly one place:
/// D-3 says a quotation constraint carries "the status of the quoting unit", which is the only
/// rule in the format that reads one as the other.
///
/// `inferred` and below map to `speculative`, because an inferred claim's *date* has nothing
/// behind it even when the claim has grounds.
pub fn of_status(s: Status) -> TimeStatus {
    match s {
        Status::Measured => TimeStatus::Measured,
        Status::Cited => TimeStatus::Cited,
        Status::Derived => TimeStatus::Derived,
        _ => TimeStatus::Speculative,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_units_status_maps_onto_a_time_status() {
        assert_eq!(of_status(Status::Measured), TimeStatus::Measured);
        assert_eq!(of_status(Status::Cited), TimeStatus::Cited);
        assert_eq!(of_status(Status::Derived), TimeStatus::Derived);
        assert_eq!(of_status(Status::Inferred), TimeStatus::Speculative);
        assert_eq!(of_status(Status::Speculative), TimeStatus::Speculative);
        assert_eq!(of_status(Status::Unfounded), TimeStatus::Speculative);
    }

    #[test]
    fn an_empty_store_implies_nothing_and_says_what_it_could_not_build() {
        let store = Store::new();
        let (free, unbuilt) = free(&store);
        assert!(free.is_empty());
        assert_eq!(unbuilt.dangling_manifests, 0);
        assert!(
            unbuilt.reply_needs_ingest,
            "the reply rule has no data until a unit can carry a reply id"
        );
    }
}
