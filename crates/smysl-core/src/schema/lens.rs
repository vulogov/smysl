//! The lens engine (§3.9, §3.10, Appendix B): chains of steps, their hash, composition, and
//! forward and backward application over a unit's reading.

use std::collections::{BTreeMap, BTreeSet};

use super::{Cond, Field, Fields, LensOp, LensStep};
use crate::cbor::major;
use crate::cbor::writer::Enc;
use crate::diag::{Code, Diagnostic};
use crate::ids::SchemaId;
use crate::surface::{object_to_payload, payload_to_object, HObject, HValue, Spanned};
use crate::types::{RelKind, UnitCore};

/// The domain separator of the lens-chain hash (Appendix B).
const CHAIN_DOMAIN: &[u8] = b"smysl/lens-chain/1\0";

/// A lensed reading of a unit: its type and payload, by name (§3.9). Edges are renamed where
/// they are followed ([`LensChain::relation_forward`]), not carried here.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitView {
    pub ty: SchemaId,
    pub payload: BTreeMap<String, HValue>,
}

impl UnitView {
    /// The unit as written. A payload that does not decode reads as empty, as it does for
    /// every other payload reader (rule I).
    pub fn of(core: &UnitCore) -> UnitView {
        let payload = core
            .payload
            .as_deref()
            .and_then(|b| payload_to_object(b).ok())
            .map(|o| {
                o.iter()
                    .map(|(k, v)| (k.value.clone(), v.value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        UnitView {
            ty: core.schema.clone(),
            payload,
        }
    }

    /// The payload as canonical bytes, `None` when empty (as `object_to_payload` has it).
    pub fn payload_bytes(&self) -> Option<Vec<u8>> {
        let span = crate::diag::Span::new(0, 0);
        let mut o = HObject::new();
        for (k, v) in &self.payload {
            o.insert(Spanned::new(k.clone(), span), Spanned::new(v.clone(), span));
        }
        object_to_payload(&o)
    }
}

/// What a lens did with one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Mapped {
    /// Every field reads at the target revision.
    Whole,
    /// These keys hold a name or value the chain could not carry (`SMY-W420`): the unit is
    /// excluded from predicates on them and counted; other fields still read.
    Partial { unmappable: BTreeSet<String> },
    /// A step on the path is unusable (an op this build does not know, `SMY-W469`): the unit
    /// cannot be read through it at all, and is counted under `SMY-W420`.
    Unusable,
}

impl Mapped {
    pub fn is_whole(&self) -> bool {
        matches!(self, Mapped::Whole)
    }
}

/// Why a chain could not be built or run.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum LensError {
    /// No step path between the revisions (`SMY-E470`).
    NoPath { id: SchemaId, from: u32, to: u32 },
    /// A step is ill-formed (`SMY-E471`).
    IllFormed {
        id: SchemaId,
        from: u32,
        to: u32,
        reason: String,
    },
    /// Backward through a `retire` (`SMY-E422`).
    Retire {
        id: SchemaId,
        from: u32,
        to: u32,
        key: String,
    },
    /// Backward through an op with no inverse — a `map-value` that is not one-to-one
    /// (`SMY-E422`, which SMYSL-2.7 §3.11 gives every step without an inverse).
    NotInvertible {
        id: SchemaId,
        from: u32,
        to: u32,
        op: String,
    },
    /// Backward through a step carrying an op this build does not know (`SMY-W469`).
    Unusable {
        id: SchemaId,
        from: u32,
        to: u32,
        tag: String,
    },
}

impl LensError {
    pub fn code(&self) -> Code {
        match self {
            LensError::NoPath { .. } => Code::E470,
            LensError::IllFormed { .. } => Code::E471,
            LensError::Retire { .. } | LensError::NotInvertible { .. } => Code::E422,
            LensError::Unusable { .. } => Code::W469,
        }
    }

    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic::new(self.code()).with_message(self.to_string())
    }
}

impl std::fmt::Display for LensError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LensError::NoPath { id, from, to } => {
                write!(f, "no lens path for {} from {from} to {to}", id.as_str())
            }
            LensError::IllFormed {
                id,
                from,
                to,
                reason,
            } => write!(
                f,
                "{} step {from}→{to} is ill-formed: {reason}",
                id.as_str()
            ),
            LensError::Retire { id, from, to, key } => write!(
                f,
                "{} step {from}→{to} retires `{key}`, which has no backward reading",
                id.as_str()
            ),
            LensError::NotInvertible { id, from, to, op } => write!(
                f,
                "{} step {from}→{to} has a `{op}` with no inverse",
                id.as_str()
            ),
            LensError::Unusable { id, from, to, tag } => write!(
                f,
                "{} step {from}→{to} carries op `{tag}`, which this build does not know",
                id.as_str()
            ),
        }
    }
}

impl std::error::Error for LensError {}

/// A declared widening (A-12.1 item 2): a `split-value` whose target already exists in the
/// revision it leaves, with the keys paired with it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Widening {
    pub from: u32,
    pub to: u32,
    pub key: String,
    /// The value widened: it existed before, and now means more.
    pub value: String,
    pub paired: Vec<String>,
}

/// A path of lens steps from one revision of a schema id up to another (§3.10).
///
/// Each step travels with the field table of the revision it leaves, which is what makes a
/// `split-value` a widening and what backward application checks its result against; the
/// chain's own end carries the target's table, for forward's.
#[derive(Debug, Clone, PartialEq)]
pub struct LensChain {
    pub id: SchemaId,
    pub from: u32,
    pub to: u32,
    steps: Vec<LensStep>,
    sources: Vec<Fields>,
    target: Fields,
    pub hash: [u8; 32],
}

impl LensChain {
    /// The empty chain at one revision. `identity ∘ c == c` (L4).
    pub fn identity(id: SchemaId, rev: u32) -> LensChain {
        LensChain::build(id, rev, rev, Vec::new(), Vec::new(), Fields::Absent)
    }

    /// A chain from steps alone, with no field tables (so no unmappable checks against them).
    /// The steps must be contiguous from `from` to `to`.
    pub fn from_steps(
        id: SchemaId,
        from: u32,
        to: u32,
        steps: Vec<LensStep>,
    ) -> Result<LensChain, LensError> {
        let mut at = from;
        for s in &steps {
            if s.from != at {
                return Err(LensError::NoPath { id, from, to });
            }
            at = s.to;
        }
        if at != to {
            return Err(LensError::NoPath { id, from, to });
        }
        let sources = vec![Fields::Absent; steps.len()];
        Ok(LensChain::build(
            id,
            from,
            to,
            steps,
            sources,
            Fields::Absent,
        ))
    }

    pub(super) fn build(
        id: SchemaId,
        from: u32,
        to: u32,
        steps: Vec<LensStep>,
        sources: Vec<Fields>,
        target: Fields,
    ) -> LensChain {
        let hash = chain_hash(&id, from, to, &steps);
        LensChain {
            id,
            from,
            to,
            steps,
            sources,
            target,
            hash,
        }
    }

    pub fn steps(&self) -> &[LensStep] {
        &self.steps
    }

    /// The field table of the revision the chain starts at.
    pub fn source_fields(&self) -> &Fields {
        self.sources.first().unwrap_or(&self.target)
    }

    pub fn target_fields(&self) -> &Fields {
        &self.target
    }

    /// `self` then `next`. Concatenation, on application and on the hash (L3).
    pub fn compose(self, next: LensChain) -> Result<LensChain, LensError> {
        if self.id != next.id || self.to != next.from {
            return Err(LensError::NoPath {
                id: self.id,
                from: self.from,
                to: next.to,
            });
        }
        // Composing with an identity keeps the other side's tables: an identity knows none.
        let target = if next.steps.is_empty() && matches!(next.target, Fields::Absent) {
            self.target
        } else {
            next.target
        };
        let mut steps = self.steps;
        steps.extend(next.steps);
        let mut sources = self.sources;
        sources.extend(next.sources);
        Ok(LensChain::build(
            self.id, self.from, next.to, steps, sources, target,
        ))
    }

    pub fn is_usable(&self) -> bool {
        self.steps.iter().all(LensStep::is_usable)
    }

    pub fn is_invertible(&self) -> bool {
        self.steps
            .iter()
            .all(|s| s.is_usable() && s.is_invertible())
    }

    pub fn has_retire(&self) -> bool {
        self.steps.iter().any(LensStep::has_retire)
    }

    /// The declared widenings on the path.
    pub fn widenings(&self) -> Vec<Widening> {
        let mut out = Vec::new();
        for (s, src) in self.steps.iter().zip(&self.sources) {
            for op in &s.ops {
                if let LensOp::SplitValue { key, to, add, .. } = op {
                    if is_widening(src.get(key), to) {
                        out.push(Widening {
                            from: s.from,
                            to: s.to,
                            key: key.clone(),
                            value: to.clone(),
                            paired: add.keys().cloned().collect(),
                        });
                    }
                }
            }
        }
        out
    }

    /// An edge kind as the target revision names it.
    pub fn relation_forward(&self, kind: &RelKind) -> RelKind {
        let mut k = kind.clone();
        for s in &self.steps {
            for op in &s.ops {
                if let LensOp::RenameRelation { from, to } = op {
                    if &k == from {
                        k = to.clone();
                    }
                }
            }
        }
        k
    }

    /// An edge kind as the source revision names it.
    pub fn relation_backward(&self, kind: &RelKind) -> RelKind {
        let mut k = kind.clone();
        for s in self.steps.iter().rev() {
            for op in s.ops.iter().rev() {
                if let LensOp::RenameRelation { from, to } = op {
                    if &k == to {
                        k = from.clone();
                    }
                }
            }
        }
        k
    }

    /// Read a view at the source revision as the target revision names it (§3.9, steps in
    /// order, ops in declared order). Never fails: what the chain cannot carry is reported
    /// in the result (L7), and an unusable step leaves the view as it was.
    pub fn forward(&self, view: &mut UnitView) -> Mapped {
        if !self.is_usable() {
            return Mapped::Unusable;
        }
        for s in &self.steps {
            for op in &s.ops {
                forward_op(op, view);
            }
        }
        unmappable(view, &self.target, self.to, false)
    }

    /// Read a view at the target revision as the source revision names it: steps reversed,
    /// ops reversed. Refused outright if any step has no inverse (`retire` above all: L6), so
    /// that nothing is half-applied; after it, a key or value no op took back is unmappable.
    pub fn backward(&self, view: &mut UnitView) -> Result<Mapped, LensError> {
        for s in &self.steps {
            for op in &s.ops {
                let at = |id: &SchemaId| (id.clone(), s.from, s.to);
                match op {
                    LensOp::Unknown { tag, .. } => {
                        let (id, from, to) = at(&self.id);
                        return Err(LensError::Unusable {
                            id,
                            from,
                            to,
                            tag: tag.clone(),
                        });
                    }
                    LensOp::Retire { key } => {
                        let (id, from, to) = at(&self.id);
                        return Err(LensError::Retire {
                            id,
                            from,
                            to,
                            key: key.clone(),
                        });
                    }
                    o if !o.is_invertible() => {
                        let (id, from, to) = at(&self.id);
                        return Err(LensError::NotInvertible {
                            id,
                            from,
                            to,
                            op: o.tag().to_string(),
                        });
                    }
                    _ => {}
                }
            }
        }
        for s in self.steps.iter().rev() {
            for op in s.ops.iter().rev() {
                backward_op(op, view);
            }
        }
        Ok(unmappable(view, self.source_fields(), self.from, true))
    }
}

fn forward_op(op: &LensOp, v: &mut UnitView) {
    match op {
        LensOp::RenameType { from, to } => {
            if &v.ty == from {
                v.ty = to.clone();
            }
        }
        LensOp::RenameRelation { .. } | LensOp::Require { .. } | LensOp::Unknown { .. } => {}
        LensOp::RenameKey { from, to } => {
            if let Some(x) = v.payload.remove(from) {
                v.payload.insert(to.clone(), x);
            }
        }
        LensOp::MapValue { key, map } => {
            if let Some(HValue::Str(s)) = v.payload.get(key) {
                if let Some(n) = map.get(s) {
                    v.payload.insert(key.clone(), HValue::Str(n.clone()));
                }
            }
        }
        LensOp::SplitValue { key, from, to, add } => {
            if v.payload.get(key) == Some(&HValue::Str(from.clone())) {
                v.payload.insert(key.clone(), HValue::Str(to.clone()));
                for (k, x) in add {
                    v.payload.insert(k.clone(), x.clone());
                }
            }
        }
        LensOp::Default { key, value, when } => {
            if !v.payload.contains_key(key) && holds(when, &v.payload) {
                v.payload.insert(key.clone(), value.clone());
            }
        }
        LensOp::Retire { key } => {
            v.payload.remove(key);
        }
    }
}

fn backward_op(op: &LensOp, v: &mut UnitView) {
    match op {
        LensOp::RenameType { from, to } => {
            if &v.ty == to {
                v.ty = from.clone();
            }
        }
        LensOp::RenameRelation { .. }
        | LensOp::Require { .. }
        | LensOp::Unknown { .. }
        | LensOp::Retire { .. } => {}
        LensOp::RenameKey { from, to } => {
            if let Some(x) = v.payload.remove(to) {
                v.payload.insert(from.clone(), x);
            }
        }
        LensOp::MapValue { key, map } => {
            if let Some(HValue::Str(s)) = v.payload.get(key) {
                if let Some((old, _)) = map.iter().find(|(_, n)| *n == s) {
                    v.payload.insert(key.clone(), HValue::Str(old.clone()));
                }
            }
        }
        LensOp::SplitValue { key, from, to, add } => {
            let matches = v.payload.get(key) == Some(&HValue::Str(to.clone()))
                && add.iter().all(|(k, x)| v.payload.get(k) == Some(x));
            if matches {
                v.payload.insert(key.clone(), HValue::Str(from.clone()));
                for k in add.keys() {
                    v.payload.remove(k);
                }
            }
        }
        LensOp::Default { key, value, when } => {
            if v.payload.get(key) == Some(value) && holds(when, &v.payload) {
                v.payload.remove(key);
            }
        }
    }
}

fn holds(when: &Option<Cond>, payload: &BTreeMap<String, HValue>) -> bool {
    when.as_ref().is_none_or(|c| c.holds(payload))
}

/// The keys of a view that do not read at `rev` under `fields`. An enum value outside the
/// revision's values is unmappable either way (L7); after a backward application, so is a
/// key the revision does not declare, or one it has retired, since no op took it back.
fn unmappable(view: &UnitView, fields: &Fields, rev: u32, backward: bool) -> Mapped {
    let Some(table) = fields.typed() else {
        return Mapped::Whole;
    };
    let mut bad = BTreeSet::new();
    for (k, v) in &view.payload {
        match table.get(k) {
            Some(f) if f.retired_at(rev) => {
                if backward {
                    bad.insert(k.clone());
                }
            }
            Some(f) => {
                if !f.admits(v) {
                    bad.insert(k.clone());
                }
            }
            None => {
                if backward {
                    bad.insert(k.clone());
                }
            }
        }
    }
    if bad.is_empty() {
        Mapped::Whole
    } else {
        Mapped::Partial { unmappable: bad }
    }
}

pub(super) fn is_widening(source: Option<&Field>, to: &str) -> bool {
    source.is_some_and(|f| f.values.iter().any(|v| v == to))
}

/// A step that is ill-formed against the revision it leaves is `SMY-E471` (§3.9): a `default`
/// with no `require` of the same key under the same condition, an op on a key the source
/// revision does not declare, or a widening without its paired `require` and `default`.
pub(super) fn validate(id: &SchemaId, s: &LensStep, source: &Fields) -> Result<(), LensError> {
    let bad = |reason: String| LensError::IllFormed {
        id: id.clone(),
        from: s.from,
        to: s.to,
        reason,
    };
    let requires = |key: &str, when: &Option<Cond>| {
        s.ops
            .iter()
            .any(|o| matches!(o, LensOp::Require { key: k, when: w } if k == key && w == when))
    };
    let defaults = |key: &str, when: &Option<Cond>| {
        s.ops
            .iter()
            .any(|o| matches!(o, LensOp::Default { key: k, when: w, .. } if k == key && w == when))
    };
    for op in &s.ops {
        if let LensOp::Default { key, when, .. } = op {
            if !requires(key, when) {
                return Err(bad(format!(
                    "`default {key}` has no `require {key}` under the same condition"
                )));
            }
        }
        if let Some(table) = source.typed() {
            let named = match op {
                LensOp::RenameKey { from, .. } => Some(from),
                LensOp::MapValue { key, .. }
                | LensOp::SplitValue { key, .. }
                | LensOp::Retire { key } => Some(key),
                _ => None,
            };
            if let Some(k) = named {
                if table.get(k).is_none_or(|f| f.retired_at(s.from)) {
                    return Err(bad(format!(
                        "`{}` names `{k}`, which revision {} does not declare",
                        op.tag(),
                        s.from
                    )));
                }
            }
        }
        if let LensOp::SplitValue { key, to, add, .. } = op {
            if is_widening(source.get(key), to) {
                let when = Some(Cond::new(key.clone(), HValue::Str(to.clone())));
                if add.is_empty() {
                    return Err(bad(format!(
                        "`split-value {key}` widens `{to}` and pairs no key with it"
                    )));
                }
                for k2 in add.keys() {
                    if !requires(k2, &when) || !defaults(k2, &when) {
                        return Err(bad(format!(
                            "`split-value {key}` widens `{to}`; `{k2}` needs a `require` and \
                             a `default` when {key} = {to}"
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

/// BLAKE3-256("smysl/lens-chain/1" ‖ 0x00 ‖ canonical_cbor([id, from, to, [step…]])), each step
/// as received (Appendix B).
fn chain_hash(id: &SchemaId, from: u32, to: u32, steps: &[LensStep]) -> [u8; 32] {
    let mut e = Enc::new();
    e.head(major::ARRAY, 4);
    e.text(id.as_str());
    e.uint(from as u64);
    e.uint(to as u64);
    e.head(major::ARRAY, steps.len() as u64);
    for s in steps {
        e.raw(s.bytes());
    }
    let mut h = blake3::Hasher::new();
    h.update(CHAIN_DOMAIN);
    h.update(e.as_bytes());
    *h.finalize().as_bytes()
}
