//! Contentions, pack manifests, and schema declarations (§5.4, §8, §2.2).
//!
//! Named `annex` rather than `aux`, which it was until 0.6.0. `aux` is a reserved device
//! name on Windows, so a crate containing `src/types/aux.rs` cannot be unpacked there —
//! `cargo publish` warns about it and the file simply does not survive extraction. Found by
//! a dry run before the first publish rather than by the first Windows user.

use core::fmt;

use crate::ids::{ContentionId, Label, SchemaId, ThreadId, Uid};
use crate::types::epistemics::Lod;
use crate::types::provenance::Hlc;
use crate::types::relation::RelKind;
use crate::types::unit::Extra;

// ---------------------------------------------------------------------------
// Contentions
// ---------------------------------------------------------------------------

/// Why a contention was raised (§5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum DetectionKind {
    /// Two distinct uids supersede the same target and neither supersedes the other.
    SupersessionFork = 0,
    /// A `rebuts` edge between two units both selected in a common thread.
    LiveRebuttal = 1,
    /// One label bound to different uids across views in scope.
    LabelCollision = 2,
}

impl DetectionKind {
    pub const ALL: &'static [DetectionKind] = &[
        DetectionKind::SupersessionFork,
        DetectionKind::LiveRebuttal,
        DetectionKind::LabelCollision,
    ];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<DetectionKind> {
        match v {
            0 => Some(DetectionKind::SupersessionFork),
            1 => Some(DetectionKind::LiveRebuttal),
            2 => Some(DetectionKind::LabelCollision),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            DetectionKind::SupersessionFork => "supersession-fork",
            DetectionKind::LiveRebuttal => "live-rebuttal",
            DetectionKind::LabelCollision => "label-collision",
        }
    }

    /// The diagnostic merge emits when this fires.
    pub const fn code(self) -> crate::diag::Code {
        match self {
            DetectionKind::SupersessionFork => crate::diag::Code::W053,
            DetectionKind::LiveRebuttal => crate::diag::Code::W053,
            DetectionKind::LabelCollision => crate::diag::Code::W054,
        }
    }
}

impl fmt::Display for DetectionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Detected {
    pub kind: DetectionKind,
    pub ts: Hlc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum ContentionStatus {
    Open = 0,
    Resolved = 1,
    Stale = 2,
}

impl ContentionStatus {
    pub const ALL: &'static [ContentionStatus] = &[
        ContentionStatus::Open,
        ContentionStatus::Resolved,
        ContentionStatus::Stale,
    ];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<ContentionStatus> {
        match v {
            0 => Some(ContentionStatus::Open),
            1 => Some(ContentionStatus::Resolved),
            2 => Some(ContentionStatus::Stale),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            ContentionStatus::Open => "open",
            ContentionStatus::Resolved => "resolved",
            ContentionStatus::Stale => "stale",
        }
    }
}

impl fmt::Display for ContentionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// Materialised disagreement (rule C).
///
/// Merge emits these and never adjudicates. A resolver emits a superseding unit listing
/// both positions as grounds; the renderer surfaces them; rule R pins them into any pack
/// touching either. Convergence without consensus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contention {
    pub id: ContentionId,
    pub over: Uid,
    pub positions: Vec<Uid>,
    pub detected: Detected,
    pub status: ContentionStatus,
    pub extra: Extra,
}

impl Contention {
    pub fn new(id: ContentionId, over: Uid, positions: Vec<Uid>, detected: Detected) -> Contention {
        Contention {
            id,
            over,
            positions,
            detected,
            status: ContentionStatus::Open,
            extra: Extra::new(),
        }
    }

    pub const fn is_open(&self) -> bool {
        matches!(self.status, ContentionStatus::Open)
    }

    /// Whether this contention pins `uid` into a pack (constraint C4).
    pub fn pins(&self, uid: &Uid) -> bool {
        self.is_open() && (self.over == *uid || self.positions.contains(uid))
    }
}

// ---------------------------------------------------------------------------
// Pack manifest
// ---------------------------------------------------------------------------

/// Why a unit did not make it into a pack. Reported as a histogram, so truncation is
/// self-describing (§8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum DropReason {
    /// Ran out of budget before its density came up.
    Budget = 0,
    /// Its closure cost more than it was worth.
    ClosureCost = 1,
    /// Below the value floor at any level.
    LowValue = 2,
    /// Not reachable from the active thread or focus set.
    OutOfScope = 3,
}

impl DropReason {
    pub const ALL: &'static [DropReason] = &[
        DropReason::Budget,
        DropReason::ClosureCost,
        DropReason::LowValue,
        DropReason::OutOfScope,
    ];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<DropReason> {
        match v {
            0 => Some(DropReason::Budget),
            1 => Some(DropReason::ClosureCost),
            2 => Some(DropReason::LowValue),
            3 => Some(DropReason::OutOfScope),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            DropReason::Budget => "budget",
            DropReason::ClosureCost => "closure-cost",
            DropReason::LowValue => "low-value",
            DropReason::OutOfScope => "out-of-scope",
        }
    }
}

impl fmt::Display for DropReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
#[non_exhaustive]
pub enum PackMode {
    Greedy = 0,
    Exact = 1,
}

impl PackMode {
    pub const ALL: &'static [PackMode] = &[PackMode::Greedy, PackMode::Exact];

    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    pub const fn from_u8(v: u8) -> Option<PackMode> {
        match v {
            0 => Some(PackMode::Greedy),
            1 => Some(PackMode::Exact),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            PackMode::Greedy => "greedy",
            PackMode::Exact => "exact",
        }
    }
}

impl fmt::Display for PackMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// What a pack knows about its own optimality.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Optimality {
    pub mode: PackMode,
    /// Upper bound on the value left on the table, in `[0,1]`. Zero means proven optimal.
    pub gap: f32,
}

/// The self-description every pack emits (§8).
///
/// A pack without a `packinfo` MAY be assumed complete; a pack with one says exactly what
/// it dropped and why. That is what makes budget truncation auditable rather than silent.
#[derive(Debug, Clone, PartialEq)]
pub struct PackInfo {
    pub budget: u64,
    pub used: u64,
    pub thread: Option<ThreadId>,
    pub dropped: Vec<(Uid, DropReason)>,
    pub degraded: Vec<(Uid, Lod)>,
    pub optimality: Optimality,
    /// The cost model this pack was built under (D-2). Budgets are approximate against
    /// any specific model by design, and this is where they say so.
    pub estimator: String,
    pub extra: Extra,
}

impl PackInfo {
    pub fn new(budget: u64, used: u64, estimator: impl Into<String>) -> PackInfo {
        PackInfo {
            budget,
            used,
            thread: None,
            dropped: Vec::new(),
            degraded: Vec::new(),
            optimality: Optimality {
                mode: PackMode::Greedy,
                gap: 0.0,
            },
            estimator: estimator.into(),
            extra: Extra::new(),
        }
    }

    /// Count of each drop reason, in reason order.
    pub fn drop_histogram(&self) -> Vec<(DropReason, usize)> {
        DropReason::ALL
            .iter()
            .map(|&r| (r, self.dropped.iter().filter(|(_, dr)| *dr == r).count()))
            .filter(|(_, n)| *n > 0)
            .collect()
    }

    pub fn is_complete(&self) -> bool {
        self.dropped.is_empty() && self.degraded.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Schema declarations
// ---------------------------------------------------------------------------

/// An extension schema declaration (§2.2).
///
/// A declaration MUST NOT alter kernel field semantics, weaken rules M/T/L, or introduce
/// mutability (`SMY-E012`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaDecl {
    pub id: SchemaId,
    pub version: u32,
    pub types: Vec<SchemaId>,
    pub relations: Vec<RelKind>,
    /// Opaque description of the payload shape, as deterministic CBOR.
    pub payload_shape: Option<Vec<u8>>,
    pub extra: Extra,
}

impl SchemaDecl {
    pub fn new(id: SchemaId, version: u32) -> SchemaDecl {
        SchemaDecl {
            id,
            version,
            types: Vec::new(),
            relations: Vec::new(),
            payload_shape: None,
            extra: Extra::new(),
        }
    }

    /// A declaration that redefines a kernel type or kernel relation kind is `SMY-E012`:
    /// an extension may add, never redefine.
    pub fn redefines_kernel(&self) -> bool {
        self.types
            .iter()
            .any(|t| t.is_kernel() || t.is_kernel_schema())
            || self.relations.iter().any(|r| r.is_kernel())
    }
}

/// A binding of a human-readable label to the uid it names. Not hashed, not identity (§1.2).
///
/// Labels being outside identity is exactly why this is its own record rather than a field
/// on a unit: putting a label inside hashed content would make renaming one produce a
/// different unit.
///
/// The type was declared in 0.1 and wired to nothing. Until 0.2 it had no envelope code and
/// no codec, so labels survived a parse and not a store round trip — a document that had
/// been through `merge` came back with every reference spelled as a canonical uid. It
/// re-checked clean and no reader could follow it, which quietly broke the format's central
/// claim that the bytes a machine reads are the bytes a person opens. The claim held for
/// hand-written files and failed for precisely the multi-agent case the format exists for.
///
/// Scope is the store the record lives in. Two stores binding the same label to different
/// uids are a `label-collision` contention on merge — machinery that already existed and
/// could not fire between two CBOR stores, because labels arrived out of band and a CBOR
/// store had none to offer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LabelBinding {
    pub label: Label,
    pub uid: Uid,
    /// Rule X: keys a later version adds, preserved verbatim.
    pub extra: Extra,
}

impl LabelBinding {
    pub fn new(label: Label, uid: Uid) -> LabelBinding {
        LabelBinding {
            label,
            uid,
            extra: Extra::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{AgentId, KernelType};

    fn uid(n: u8) -> Uid {
        Uid::from_bytes([n; 32])
    }

    fn detected(kind: DetectionKind) -> Detected {
        Detected {
            kind,
            ts: Hlc::zero(AgentId::new("human:v").unwrap()),
        }
    }

    #[test]
    fn detection_kinds_round_trip_and_map_to_codes() {
        assert_eq!(DetectionKind::ALL.len(), 3);
        for &k in DetectionKind::ALL {
            assert_eq!(DetectionKind::from_u8(k.as_u8()), Some(k));
        }
        assert_eq!(
            DetectionKind::SupersessionFork.code(),
            crate::diag::Code::W053
        );
        assert_eq!(
            DetectionKind::LabelCollision.code(),
            crate::diag::Code::W054
        );
    }

    #[test]
    fn contentions_start_open() {
        let c = Contention::new(
            ContentionId::new("k/pool-vs-index").unwrap(),
            uid(1),
            vec![uid(2), uid(3)],
            detected(DetectionKind::SupersessionFork),
        );
        assert!(c.is_open());
        assert_eq!(c.status, ContentionStatus::Open);
    }

    /// Constraint C4: an open contention pins every position, and the unit it is over.
    #[test]
    fn an_open_contention_pins_all_of_its_positions() {
        let c = Contention::new(
            ContentionId::new("k/x").unwrap(),
            uid(1),
            vec![uid(2), uid(3)],
            detected(DetectionKind::LiveRebuttal),
        );
        assert!(c.pins(&uid(1)));
        assert!(c.pins(&uid(2)));
        assert!(c.pins(&uid(3)));
        assert!(!c.pins(&uid(9)));
    }

    #[test]
    fn a_resolved_contention_pins_nothing() {
        let mut c = Contention::new(
            ContentionId::new("k/x").unwrap(),
            uid(1),
            vec![uid(2)],
            detected(DetectionKind::LiveRebuttal),
        );
        c.status = ContentionStatus::Resolved;
        assert!(!c.is_open());
        assert!(!c.pins(&uid(2)));
    }

    #[test]
    fn contention_status_round_trips() {
        for &s in ContentionStatus::ALL {
            assert_eq!(ContentionStatus::from_u8(s.as_u8()), Some(s));
        }
        assert_eq!(ContentionStatus::from_u8(3), None);
    }

    #[test]
    fn drop_reasons_and_pack_modes_round_trip() {
        for &r in DropReason::ALL {
            assert_eq!(DropReason::from_u8(r.as_u8()), Some(r));
        }
        assert_eq!(DropReason::from_u8(4), None);
        for &m in PackMode::ALL {
            assert_eq!(PackMode::from_u8(m.as_u8()), Some(m));
        }
        assert_eq!(PackMode::from_u8(2), None);
    }

    #[test]
    fn a_pack_with_nothing_dropped_is_complete() {
        let p = PackInfo::new(8000, 6000, "smysl/utf8-div4");
        assert!(p.is_complete());
        assert!(p.drop_histogram().is_empty());
        assert_eq!(p.optimality.mode, PackMode::Greedy);
    }

    #[test]
    fn the_drop_histogram_counts_by_reason_in_reason_order() {
        let mut p = PackInfo::new(100, 100, "smysl/utf8-div4");
        p.dropped = vec![
            (uid(1), DropReason::Budget),
            (uid(2), DropReason::LowValue),
            (uid(3), DropReason::Budget),
        ];
        assert_eq!(
            p.drop_histogram(),
            [(DropReason::Budget, 2), (DropReason::LowValue, 1)]
        );
        assert!(!p.is_complete());
    }

    #[test]
    fn degraded_levels_also_make_a_pack_incomplete() {
        let mut p = PackInfo::new(100, 100, "smysl/utf8-div4");
        p.degraded = vec![(uid(1), Lod::L0)];
        assert!(!p.is_complete());
    }

    #[test]
    fn an_extension_may_add_types_but_not_redefine_kernel_ones() {
        let mut d = SchemaDecl::new(SchemaId::parse("x.sre/1").unwrap(), 1);
        d.types = vec![SchemaId::parse("x.sre/incident").unwrap()];
        d.relations = vec![RelKind::parse("x.sre/mitigates").unwrap()];
        assert!(!d.redefines_kernel());

        d.types.push(SchemaId::from(KernelType::Claim));
        assert!(d.redefines_kernel(), "redefining `claim` is SMY-E012");
    }

    #[test]
    fn redefining_a_kernel_relation_is_also_a_violation() {
        let mut d = SchemaDecl::new(SchemaId::parse("x.sre/1").unwrap(), 1);
        d.relations = vec![RelKind::Rebuts];
        assert!(d.redefines_kernel());
    }
}
