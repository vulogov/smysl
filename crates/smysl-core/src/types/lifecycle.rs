//! The lifecycle of edges and disagreements: withdrawal and resolution (1.4).
//!
//! A unit can be retracted and superseded; until 1.4 an edge could be neither withdrawn nor
//! reviewed, so a `rebuts` edge a reviewer rejected stayed in the graph and pinned its claim into
//! every pack for good. Both records here name what they act on by identity — a relation by its
//! rid (`Relation::uid`), a contention by its derived id — so they travel independently of it and
//! take effect whichever arrives first (rule U).
//!
//! Neither decides anything. A withdrawal says an edge should not be followed; a resolution says a
//! review happened. What the reviewer concluded is said by the records that already say it:
//! retraction, withdrawal, supersession.

use crate::ids::{AgentId, ContentionId, Uid};
use crate::types::annex::DetectionKind;
use crate::types::provenance::Hlc;
use crate::types::unit::Extra;

/// An edge that should no longer be followed (record type 11).
///
/// A relation is withdrawn in a store iff the store holds at least one withdrawal naming its rid.
/// The relation's own record stays, and round-trips; what changes is that nothing interpreting the
/// graph follows it. `retracts` and `supersedes` edges cannot be withdrawn in 1.4 (`SMY-W056`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct Withdrawal {
    /// The rid of the withdrawn relation.
    pub relation: Uid,
    pub agent: AgentId,
    pub ts: Hlc,
    /// A unit saying why.
    pub reason: Option<Uid>,
    pub extra: Extra,
}

impl Withdrawal {
    pub fn new(relation: Uid, agent: AgentId, ts: Hlc) -> Withdrawal {
        Withdrawal {
            relation,
            agent,
            ts,
            reason: None,
            extra: Extra::new(),
        }
    }

    pub fn with_reason(mut self, reason: Uid) -> Withdrawal {
        self.reason = Some(reason);
        self
    }
}

/// What a resolution reviewed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ResolutionTarget {
    /// A contention, by its derived id.
    Contention(ContentionId),
    /// A `rebuts` edge nobody threaded into a contention, by its rid.
    Relation(Uid),
}

/// A record that a disagreement was reviewed (record type 12).
///
/// It never records an outcome: two resolutions with different outcomes would be a disagreement
/// about the disagreement, which merge could not settle without adjudicating. A contention named
/// by one reads as resolved and stops pinning its positions; a `rebuts` edge named by one stays
/// live — rule R still binds — and leaves the review queue.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct Resolution {
    pub target: ResolutionTarget,
    pub agent: AgentId,
    pub ts: Hlc,
    /// A unit recording the decision.
    pub note: Option<Uid>,
    pub extra: Extra,
}

impl Resolution {
    pub fn new(target: ResolutionTarget, agent: AgentId, ts: Hlc) -> Resolution {
        Resolution {
            target,
            agent,
            ts,
            note: None,
            extra: Extra::new(),
        }
    }

    pub fn with_note(mut self, note: Uid) -> Resolution {
        self.note = Some(note);
        self
    }
}

impl ContentionId {
    /// The id a contention of `kind` over `over` with `positions` has, wherever it is detected.
    ///
    /// `"k/c"` and the first 130 bits of `BLAKE3(kind ‖ over ‖ positions)` in §2.1's base32, with
    /// positions sorted and deduplicated. Normative since 1.4, because a resolution names a
    /// contention by it: two implementations that derived different ids would each read the other's
    /// resolutions as naming nothing. The detection clock is not identity.
    pub fn derive(kind: DetectionKind, over: &Uid, positions: &[Uid]) -> ContentionId {
        let mut sorted = positions.to_vec();
        sorted.sort();
        sorted.dedup();
        let mut bytes = Vec::with_capacity(1 + 32 * (sorted.len() + 1));
        bytes.push(kind.as_u8());
        bytes.extend_from_slice(over.as_bytes());
        for p in &sorted {
            bytes.extend_from_slice(p.as_bytes());
        }
        let digest = Uid::from_bytes(crate::hash::hash_bytes(&bytes));
        // The leading letter keeps the id a well-formed label, whose name must start with one.
        ContentionId::new(format!("k/c{}", &digest.short()[3..]))
            .expect("a derived identifier is always well-formed")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    #[test]
    fn a_contention_id_ignores_position_order_and_duplicates() {
        let a = ContentionId::derive(DetectionKind::LiveRebuttal, &uid(1), &[uid(2), uid(1)]);
        let b = ContentionId::derive(
            DetectionKind::LiveRebuttal,
            &uid(1),
            &[uid(1), uid(2), uid(2)],
        );
        assert_eq!(a, b);
        assert!(a.as_str().starts_with("k/c"));
        assert_eq!(a.as_str().len(), 3 + 26);
    }

    #[test]
    fn a_contention_id_depends_on_kind_over_and_positions() {
        let base = ContentionId::derive(DetectionKind::LiveRebuttal, &uid(1), &[uid(2)]);
        assert_ne!(
            base,
            ContentionId::derive(DetectionKind::SupersessionFork, &uid(1), &[uid(2)])
        );
        assert_ne!(
            base,
            ContentionId::derive(DetectionKind::LiveRebuttal, &uid(3), &[uid(2)])
        );
        assert_ne!(
            base,
            ContentionId::derive(DetectionKind::LiveRebuttal, &uid(1), &[uid(3)])
        );
    }
}

// ---------------------------------------------------------------------------
// Commitment (1.7)
// ---------------------------------------------------------------------------

/// How settled a unit is, as a matter of the author's decision rather than of the world.
///
/// `Status` answers *how true* — `Measured` means an instrument recorded it — and its integer
/// order **is** rule M. That is the wrong quantity for a body of work whose content is settled by
/// fiat rather than confirmed by reality: a plot point is canonical because somebody committed to
/// it. This is a second axis, deliberately independent of `Status`, so neither order constrains
/// the other; a `Floated` idea may be `Measured` and a `Canonical` decision may be `Speculative`.
///
/// Higher is more settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum Commitment {
    /// An idea on the table.
    Floated = 0,
    /// Written down, not load-bearing.
    Drafted = 1,
    /// Other decisions may depend on it.
    Committed = 2,
    /// Settled truth of the work.
    Canonical = 3,
    /// Explicitly overridden, kept as history. Pairs with a `supersedes` edge, which stays
    /// authoritative for traversal — this says how to *read* the unit, not how to walk to it.
    Retconned = 4,
}

impl Commitment {
    pub const ALL: &'static [Commitment] = &[
        Commitment::Floated,
        Commitment::Drafted,
        Commitment::Committed,
        Commitment::Canonical,
        Commitment::Retconned,
    ];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<Commitment> {
        match v {
            0 => Some(Commitment::Floated),
            1 => Some(Commitment::Drafted),
            2 => Some(Commitment::Committed),
            3 => Some(Commitment::Canonical),
            4 => Some(Commitment::Retconned),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Commitment::Floated => "floated",
            Commitment::Drafted => "drafted",
            Commitment::Committed => "committed",
            Commitment::Canonical => "canonical",
            Commitment::Retconned => "retconned",
        }
    }

    pub fn parse(s: &str) -> Option<Commitment> {
        Commitment::ALL.iter().copied().find(|c| c.as_str() == s)
    }
}

impl core::fmt::Display for Commitment {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.pad(self.as_str())
    }
}

/// A commitment to a unit (record type 13, 1.7).
///
/// A record rather than a field on `Unit`, for two reasons that turned out to be the same one.
/// `Unit`'s non-identity fields are assembled from records — `attestations` from record 2,
/// `labels` from record 10 — except `salience`, which has no record and therefore does not
/// survive a store write at all. A commitment that behaved like `salience` would be lost the
/// moment a ledger was written to disk.
///
/// And a record says more than a field can. *Who* settled this, *when*, and what it said before
/// are the questions a development history exists to answer; a field holds only the latest answer
/// and cannot say whose it was.
///
/// Identity is untouched: `commitment` is not in `UnitCore`, so committing to a unit does not
/// move its uid. "The content did not change; my commitment to it did" is the event this records.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct Commit {
    /// The unit being committed to.
    pub unit: Uid,
    pub level: Commitment,
    pub agent: AgentId,
    pub ts: Hlc,
    /// A unit saying why, as a withdrawal's `reason` does.
    pub note: Option<Uid>,
    pub extra: Extra,
}

impl Commit {
    pub fn new(unit: Uid, level: Commitment, agent: AgentId, ts: Hlc) -> Commit {
        Commit {
            unit,
            level,
            agent,
            ts,
            note: None,
            extra: Extra::new(),
        }
    }

    pub fn with_note(mut self, note: Uid) -> Commit {
        self.note = Some(note);
        self
    }
}
