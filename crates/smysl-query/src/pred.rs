//! Predicate resolution and evaluation (SMYSL-2.5 §3.3, §3.9; TX-P6 step 4, TX-P12 step 3;
//! planner phase 6).
//!
//! # What a field reads as
//!
//! A kernel field has a type this crate knows: `kind` is a `KernelType`, `status` is an ordered
//! enum, `commitment` is another, `lang` is a tag. A payload key has the type its declaration
//! gives it, since FC-9 decodes a declaration's field table (TX-P12 step 1), and phase 2
//! resolves the name against those tables ([`crate::resolve`]):
//!
//! - a key a typed table declares is [`Field::Declared`], read through the lens chain from each
//!   unit's inferred revision to the current one ([`Reader`]), so a unit written under an older
//!   revision answers under the current names (§3.9);
//! - a key no table declares, in a store where some unit's type is undeclared, is
//!   [`Field::Payload`], read **raw** and compared as text, as every payload key was before FC-9;
//! - a qualified reference under `with lens none` is [`Field::Literal`]: read raw, and only from
//!   units whose inferred revision is the one named.
//!
//! An ordering comparison needs a declared order: `status`, `commitment`, or a payload key
//! declared `int`. On anything else it is refused rather than guessed, because `>` on text
//! whose type nobody has declared answers a different question.
//!
//! A unit a lens cannot carry on a field (`SMY-W420`) is excluded from predicates on that field
//! and counted, by the [`Reader`], for the run to report.

use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic, Span};
use smysl_core::ids::{SchemaId, Uid};
use smysl_core::schema::{FieldType, LensChain, Mapped, UnitView};
use smysl_core::surface::{payload_to_object, HValue, Spanned};
use smysl_core::types::{Commitment, Status};
use smysl_graph::Store;

use crate::ast::{Atom, CmpOp, Decimal, FieldRef, Pred, Value};
use crate::resolve::{resolve_at, resolve_name, Resolved, Schemas};

/// A resolved field reference.
///
/// Resolution happens once, at plan time, so a filter over ten thousand units does not re-read
/// the same name ten thousand times — and so that an unusable field is refused before anything
/// runs, which is what phase 6 is for.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Field {
    /// The kernel type of the unit's schema. Unordered.
    Kind,
    /// The schema id as written. Unordered.
    Schema,
    /// Ordered, by the ladder `Status::as_u8` fixes.
    Status,
    /// Ordered, by `Commitment::as_u8`.
    Commitment,
    /// Core key 9 (FC-1). Unordered.
    Lang,
    /// A payload key no typed declaration names, read raw and compared as text.
    Payload(String),
    /// A payload key the current revision of schema `id` declares, read through each unit's
    /// lens chain. `int` is whether it is declared `int`, which is what gives it an order.
    Declared { id: String, key: String, int: bool },
    /// A qualified key under `with lens none`: read raw, from units whose inferred revision of
    /// `id` holds `revision`; every other unit that schema governs is unmappable on it.
    Literal {
        id: String,
        revision: u32,
        key: String,
    },
}

impl Field {
    /// Whether `<`, `<=`, `>` and `>=` mean anything for this field.
    ///
    /// Two kernel fields are ordered, by a ladder the kernel declares, and a payload key is
    /// when its declaration types it `int`. Everything else is not: `kind` and `schema` are
    /// names, `lang` is a tag, and a raw payload value has no declared type.
    pub fn is_ordered(&self) -> bool {
        matches!(
            self,
            Field::Status | Field::Commitment | Field::Declared { int: true, .. }
        )
    }

    pub fn name(&self) -> &str {
        match self {
            Field::Kind => "kind",
            Field::Schema => "schema",
            Field::Status => "status",
            Field::Commitment => "commitment",
            Field::Lang => "lang",
            Field::Payload(k) | Field::Declared { key: k, .. } | Field::Literal { key: k, .. } => k,
        }
    }
}

/// Whether a name is one of the kernel fields, which no declaration can shadow.
pub fn is_kernel(name: &str) -> bool {
    matches!(name, "kind" | "schema" | "status" | "commitment" | "lang")
}

/// A lowered predicate: every field resolved, every operator checked against its field's type.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Lowered {
    And(Vec<Lowered>),
    Or(Vec<Lowered>),
    Not(Box<Lowered>),
    Cmp {
        field: Field,
        op: CmpOp,
        value: Value,
        span: Span,
    },
    /// `where matches "…"`: a term-presence test over the unit's indexable text.
    Matches {
        text: Atom,
        span: Span,
    },
    /// `where near("…", t)`: the unit's cosine with the text is at least `t` (TX-P10 step 4).
    /// Model-dependent; it lowers only when the environment binds a semantic source, and is
    /// evaluated against that source's scores by the executor ([`holds_near`]).
    Near {
        text: Atom,
        threshold: Decimal,
        span: Span,
    },
}

/// A `near(…)` text as bound: the string, or the parameter's binding.
pub fn atom_text(a: &Atom, params: &BTreeMap<String, String>) -> Option<String> {
    match a {
        Atom::Str(s) => Some(s.clone()),
        Atom::Param(n) => params.get(n).cloned(),
    }
}

/// Every `near(…)` text in a lowered predicate, bound.
pub fn near_texts(pred: &Lowered, params: &BTreeMap<String, String>, out: &mut BTreeSet<String>) {
    match pred {
        Lowered::And(items) | Lowered::Or(items) => {
            items.iter().for_each(|p| near_texts(p, params, out))
        }
        Lowered::Not(inner) => near_texts(inner, params, out),
        Lowered::Near { text, .. } => {
            if let Some(t) = atom_text(text, params) {
                out.insert(t);
            }
        }
        Lowered::Cmp { .. } | Lowered::Matches { .. } => {}
    }
}

/// Whether a lowered predicate consults a model anywhere in it.
pub fn uses_near(pred: &Lowered) -> bool {
    match pred {
        Lowered::And(items) | Lowered::Or(items) => items.iter().any(uses_near),
        Lowered::Not(inner) => uses_near(inner),
        Lowered::Near { .. } => true,
        Lowered::Cmp { .. } | Lowered::Matches { .. } => false,
    }
}

/// Phase 6 for one predicate, with no declarations: every payload key is raw and a qualified
/// reference is refused. [`lower_in`] is the planner's.
pub fn lower(pred: &Pred, diags: &mut Vec<Diagnostic>) -> Option<Lowered> {
    lower_in(pred, None, false, diags)
}

/// Phase 6 for one predicate already compiled forward ([`crate::resolve::Compiler`]), against
/// a store's declarations. `lens_none` is the stage's `with lens none`.
pub fn lower_in(
    pred: &Pred,
    schemas: Option<&Schemas>,
    lens_none: bool,
    diags: &mut Vec<Diagnostic>,
) -> Option<Lowered> {
    lower_with(pred, schemas, lens_none, false, diags)
}

/// [`lower_in`], and `semantic` says whether the environment binds a semantic source, which is
/// what lets `near(…)` lower (TX-P10 step 4).
pub fn lower_with(
    pred: &Pred,
    schemas: Option<&Schemas>,
    lens_none: bool,
    semantic: bool,
    diags: &mut Vec<Diagnostic>,
) -> Option<Lowered> {
    match pred {
        Pred::And(items) => lower_all(items, schemas, lens_none, semantic, diags).map(Lowered::And),
        Pred::Or(items) => lower_all(items, schemas, lens_none, semantic, diags).map(Lowered::Or),
        Pred::Not(inner) => lower_with(inner, schemas, lens_none, semantic, diags)
            .map(|l| Lowered::Not(Box::new(l))),
        Pred::Matches(text) => Some(Lowered::Matches {
            text: text.clone(),
            span: Span::new(0, 0),
        }),
        Pred::Near { text, threshold } => {
            // Cosine against an embedding: a different tier, so it needs the port.
            if !semantic {
                diags.push(
                    Diagnostic::at(Code::E461, Span::new(0, 0)).with_message(
                        "`near(…)` compares against an embedding, and this environment binds no \
                         semantic source (a build with `semantic` or `semantic-candle` and a \
                         model: `q --engine-model` or `--candle-model`)"
                            .to_string(),
                    ),
                );
                return None;
            }
            // A cosine in (0, 1]. The source returns no unit at or below zero — that is not a
            // hit — so a threshold there would admit units it could never see, and one above
            // one admits none.
            if threshold.scaled <= 0 || threshold.scaled > Decimal::SCALE {
                diags.push(
                    Diagnostic::at(Code::E460, Span::new(0, 0)).with_message(format!(
                        "`near(…)`'s threshold is a cosine above 0 and at most 1, not {}",
                        crate::print::decimal(*threshold)
                    )),
                );
                return None;
            }
            Some(Lowered::Near {
                text: text.clone(),
                threshold: *threshold,
                span: Span::new(0, 0),
            })
        }
        Pred::Cmp { field, op, value } => {
            let resolved = resolve_in(field, schemas, lens_none, diags)?;
            if matches!(op, CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge) && !resolved.is_ordered()
            {
                diags.push(Diagnostic::at(Code::E512, field.span).with_message(format!(
                    "`{}` has no order, so `{}` cannot be asked of it{}",
                    resolved.name(),
                    op_text(*op),
                    match resolved {
                        Field::Payload(_) | Field::Literal { .. } =>
                            ": no declaration gives this payload key a type, and ordering \
                             undeclared text would answer a different question",
                        Field::Declared { .. } => ": its declaration does not type it `int`",
                        _ => ": it is a name, not a quantity",
                    }
                )));
                return None;
            }
            if matches!(op, CmpOp::In) && !matches!(value, Value::List(_)) {
                diags.push(
                    Diagnostic::at(Code::E512, field.span)
                        .with_message("`in` takes a list; a single value is `=`".to_string()),
                );
                return None;
            }
            Some(Lowered::Cmp {
                field: resolved,
                op: *op,
                value: value.clone(),
                span: field.span,
            })
        }
    }
}

fn lower_all(
    items: &[Pred],
    schemas: Option<&Schemas>,
    lens_none: bool,
    semantic: bool,
    diags: &mut Vec<Diagnostic>,
) -> Option<Vec<Lowered>> {
    let mut out = Vec::with_capacity(items.len());
    let mut ok = true;
    for item in items {
        match lower_with(item, schemas, lens_none, semantic, diags) {
            Some(l) => out.push(l),
            None => ok = false,
        }
    }
    // Every branch is lowered before returning, so one query reports every unusable field
    // rather than the first: a person fixing a predicate should see the whole list.
    ok.then_some(out)
}

/// Resolve a name to a field with no declarations (see [`lower`]).
///
/// Public because phase 9 resolves fields too: `order by`, `group by` and `compare by` each
/// name one, and a field that is unreadable in a filter is unreadable in a sort.
pub fn resolve(field: &FieldRef, diags: &mut Vec<Diagnostic>) -> Option<Field> {
    resolve_in(field, None, false, diags)
}

/// Resolve a name to a field against a store's declarations (phase 2's typed half).
///
/// A qualified reference reaching here is one compile-forward left literal (`with lens
/// none`); any other was rewritten to the current revision's names before lowering. Without
/// declarations there is nothing to resolve a qualifier against, and it is refused.
pub fn resolve_in(
    field: &FieldRef,
    schemas: Option<&Schemas>,
    lens_none: bool,
    diags: &mut Vec<Diagnostic>,
) -> Option<Field> {
    if let Some(pin) = &field.pin {
        let (Some(schemas), true) = (schemas, lens_none) else {
            diags.push(Diagnostic::at(Code::E461, field.span).with_message(format!(
                "`{}@{}:{}` is the field as that revision named it, compiled forward \
                 (SMYSL-2.5 §3.9), which needs the store's declarations",
                pin.id, pin.revision, field.name
            )));
            return None;
        };
        let id = match SchemaId::parse(&pin.id) {
            Ok(id) => id,
            Err(_) => {
                diags.push(
                    Diagnostic::at(Code::E466, field.span)
                        .with_message(format!("`{}` is not a schema id", pin.id)),
                );
                return None;
            }
        };
        return match resolve_at(schemas, &id, pin.revision, &field.name, field.span) {
            Ok(key) => Some(Field::Literal {
                id: pin.id.clone(),
                revision: pin.revision,
                key,
            }),
            Err(d) => {
                diags.push(d);
                None
            }
        };
    }
    Some(match field.name.as_str() {
        "kind" => Field::Kind,
        "schema" => Field::Schema,
        "status" => Field::Status,
        "commitment" => Field::Commitment,
        "lang" => Field::Lang,
        other => match schemas {
            None => Field::Payload(other.to_string()),
            Some(schemas) => match resolve_name(schemas, other, field.span) {
                Ok(Resolved::Raw) => Field::Payload(other.to_string()),
                Ok(Resolved::Declared { id, key }) => {
                    let int = schemas
                        .set
                        .current(&id)
                        .and_then(|r| r.fields.get(&key))
                        .is_some_and(|f| f.ty == FieldType::Int);
                    Field::Declared {
                        id: id.as_str().to_string(),
                        key,
                        int,
                    }
                }
                Err(d) => {
                    diags.push(d);
                    return None;
                }
            },
        },
    })
}

fn op_text(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Eq => "=",
        CmpOp::Ne => "!=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
        CmpOp::In => "in",
    }
}

/// What a unit's field reads as, for comparison.
///
/// `None` is "this unit has no such field", which is never equal to anything and never
/// unequal either — see [`holds`]. A unit a lens could not carry on the field reads as `None`
/// too, and the [`Reader`] counts it.
fn read(r: &Reader<'_>, uid: &Uid, field: &Field) -> Option<String> {
    let store = r.store;
    let unit = store.get(uid)?;
    Some(match field {
        Field::Kind => match &unit.core.schema {
            SchemaId::Kernel(k) => k.as_str().to_string(),
            // A unit under an extension schema has no kernel type. Not "unknown": it has none,
            // so `kind = anything` is false for it rather than a comparison against a guess.
            _ => return None,
        },
        Field::Schema => unit.core.schema.to_string(),
        Field::Status => unit.core.status.as_str().to_string(),
        Field::Commitment => store.commitment_of(uid)?.as_str().to_string(),
        Field::Lang => unit.core.lang.as_ref()?.as_str().to_string(),
        Field::Payload(key) => {
            let bytes = unit.core.payload.as_ref()?;
            let object = payload_to_object(bytes).ok()?;
            raw_at(&object, key).map(text_of)?
        }
        Field::Declared { key, .. } => match r.lensed(uid) {
            // No typed schema governs this unit's type: its payload is as written.
            None => {
                let bytes = unit.core.payload.as_ref()?;
                let object = payload_to_object(bytes).ok()?;
                object.get(key).map(|v| text_of(&v.value))?
            }
            Some(l) => {
                if !l.carries(key) {
                    r.unmappable(key, uid);
                    return None;
                }
                l.payload.get(key).map(text_of)?
            }
        },
        Field::Literal { id, revision, key } => {
            let l = r.lensed(uid)?;
            if l.id.as_str() != id {
                return None;
            }
            if !(l.lo..=l.hi).contains(revision) {
                r.unmappable(key, uid);
                return None;
            }
            let bytes = unit.core.payload.as_ref()?;
            let object = payload_to_object(bytes).ok()?;
            object.get(key).map(|v| text_of(&v.value))?
        }
    })
}

/// A unit as the current revision of its schema names it.
#[derive(Debug, Clone)]
pub struct Lensed {
    pub id: SchemaId,
    /// The revisions the unit is consistent with (§3.9's inference).
    pub lo: u32,
    pub hi: u32,
    pub payload: BTreeMap<String, HValue>,
    /// What the chain could not carry: `None` when it carried everything, an empty set when
    /// the chain was unusable or missing and nothing can be read.
    pub lost: Option<BTreeSet<String>>,
}

impl Lensed {
    /// Whether the key reads at the current revision.
    pub fn carries(&self, key: &str) -> bool {
        match &self.lost {
            None => true,
            Some(lost) => !lost.is_empty() && !lost.contains(key),
        }
    }
}

/// Reads units for predicates, through the store's declarations (§3.9).
///
/// One per environment: the declarations are decoded once, each unit is lensed once however
/// many predicates read it, and units a lens could not carry are collected for the run to
/// report as `SMY-W420`.
pub struct Reader<'a> {
    store: &'a Store,
    schemas: OnceCell<Schemas>,
    views: RefCell<BTreeMap<Uid, Option<Lensed>>>,
    chains: RefCell<BTreeMap<(String, u32), Option<LensChain>>>,
    lost: RefCell<BTreeMap<String, BTreeSet<Uid>>>,
}

impl<'a> Reader<'a> {
    pub fn new(store: &'a Store) -> Reader<'a> {
        Reader {
            store,
            schemas: OnceCell::new(),
            views: RefCell::new(BTreeMap::new()),
            chains: RefCell::new(BTreeMap::new()),
            lost: RefCell::new(BTreeMap::new()),
        }
    }

    pub fn store(&self) -> &'a Store {
        self.store
    }

    /// The store's declarations, decoded on first use.
    pub fn schemas(&self) -> &Schemas {
        self.schemas.get_or_init(|| Schemas::of(self.store))
    }

    /// The chain from `from` to the current revision of `id`, built once.
    pub fn chain(&self, id: &SchemaId, from: u32) -> Option<LensChain> {
        let k = (id.as_str().to_string(), from);
        if let Some(c) = self.chains.borrow().get(&k) {
            return c.clone();
        }
        let set = &self.schemas().set;
        let c = set
            .current(id)
            .and_then(|cur| set.chain(id, from, cur.version).ok());
        self.chains.borrow_mut().insert(k, c.clone());
        c
    }

    /// A unit read through its schema's lens chain, or `None` when no typed schema governs
    /// its type (its payload is then read as written).
    pub fn lensed(&self, uid: &Uid) -> Option<Lensed> {
        if let Some(v) = self.views.borrow().get(uid) {
            return v.clone();
        }
        let v = self.lens(uid);
        self.views.borrow_mut().insert(*uid, v.clone());
        v
    }

    fn lens(&self, uid: &Uid) -> Option<Lensed> {
        let core = &self.store.get(uid)?.core;
        let set = &self.schemas().set;
        let id = set.governing_unit(core)?;
        let typed = set
            .revisions(&id)
            .iter()
            .any(|r| r.fields.typed().is_some());
        if !typed {
            return None;
        }
        let view = UnitView::of(core);
        // A fork has no current revision, and a unit with no consistent revision cannot be
        // placed on a chain: either way nothing reads, and every field is unmappable.
        let Some(guess) = set.infer_revision(core) else {
            return Some(Lensed {
                id,
                lo: 0,
                hi: 0,
                payload: view.payload,
                lost: Some(BTreeSet::new()),
            });
        };
        let mut view = view;
        let lost = match self.chain(&id, guess.hi) {
            None => Some(BTreeSet::new()),
            Some(chain) => match chain.forward(&mut view) {
                Mapped::Whole => None,
                Mapped::Partial { unmappable } => Some(unmappable).filter(|u| !u.is_empty()),
                Mapped::Unusable => Some(BTreeSet::new()),
                _ => Some(BTreeSet::new()),
            },
        };
        Some(Lensed {
            id,
            lo: guess.lo,
            hi: guess.hi,
            payload: view.payload,
            lost,
        })
    }

    fn unmappable(&self, key: &str, uid: &Uid) {
        self.lost
            .borrow_mut()
            .entry(key.to_string())
            .or_default()
            .insert(*uid);
    }

    /// The units each key could not be read on since the last call, emptied.
    pub fn take_unmappable(&self) -> BTreeMap<String, BTreeSet<Uid>> {
        std::mem::take(&mut *self.lost.borrow_mut())
    }
}

/// A raw key's value: the top-level key spelled `key`, else the nested value its `.`-separated
/// path names — the same reading as `payload_strings`, which keys `{a: {b: …}}` and a literal
/// `"a.b"` alike, so a filter sees what `find` indexes. The literal key wins when both exist.
fn raw_at<'o>(object: &'o smysl_core::surface::HObject, key: &str) -> Option<&'o HValue> {
    if let Some(v) = object.get(key) {
        return Some(&v.value);
    }
    let mut parts = key.split('.');
    let mut here = &object.get(parts.next()?)?.value;
    for part in parts {
        match here {
            HValue::Object(o) => here = &o.get(part)?.value,
            _ => return None,
        }
    }
    Some(here)
}

/// A payload value as text.
///
/// Raw, which is the point: a value this build cannot type is a value it should not reshape.
/// A list renders as its elements joined by `, ` so that `where tags = "a, b"` is at least
/// answerable; a caller wanting membership writes `in`, which compares element-wise.
fn text_of(v: &HValue) -> String {
    match v {
        HValue::Str(s) => s.clone(),
        HValue::Int(i) => i.to_string(),
        HValue::Bool(b) => b.to_string(),
        HValue::Null => String::new(),
        // Canonical rather than `{f}`: a float's shortest representation has differed between
        // the runtimes this grammar is implemented in, and a filter that compared text would
        // then accept different units in each.
        HValue::Float(f) => format!("{f:?}"),
        HValue::Array(items) => items
            .iter()
            .map(|i: &Spanned<HValue>| text_of(&i.value))
            .collect::<Vec<_>>()
            .join(", "),
        // An object has no text a query could have meant. Compared as nothing, so a predicate
        // naming a nested key is false rather than accidentally true against a debug format.
        HValue::Object(_) => String::new(),
    }
}

/// The written value as text, with `$name` bound.
fn value_text(
    value: &Value,
    params: &std::collections::BTreeMap<String, String>,
) -> Option<String> {
    Some(match value {
        Value::Str(s) | Value::Word(s) | Value::Uid(s) => s.clone(),
        Value::Int(i) => i.to_string(),
        Value::Dec(d) => crate::print::decimal(*d),
        Value::Param(name) => params.get(name)?.clone(),
        // A list has no single text. `in` reads the elements; anything else was refused when
        // the predicate was lowered.
        Value::List(_) => return None,
    })
}

/// Where an ordered field's value sits on its ladder, or its number for a key declared `int`.
fn rank(field: &Field, text: &str) -> Option<i64> {
    match field {
        Field::Status => Status::parse(text).map(|s| i64::from(s.as_u8())),
        Field::Commitment => Commitment::parse(text).map(|c| i64::from(c.as_u8())),
        Field::Declared { int: true, .. } => text.parse().ok(),
        _ => None,
    }
}

/// Does this unit satisfy the predicate?
///
/// A missing field makes a comparison **false**, including `!=`. That is deliberate and it is
/// the only reading that keeps `where x = "a"` and `where x != "a"` from partitioning the
/// store: a unit with no `x` is not a unit whose `x` differs. `not (x = "a")` is how a caller
/// asks for the complement, and it says so.
///
/// This builds a [`Reader`] for the one call, so it decodes the store's declarations each
/// time; a caller testing many units holds one reader and calls [`holds_in`].
pub fn holds(
    store: &Store,
    uid: &Uid,
    pred: &Lowered,
    params: &std::collections::BTreeMap<String, String>,
) -> bool {
    holds_in(&Reader::new(store), uid, pred, params)
}

/// [`holds`], through a reader that lenses each unit once.
pub fn holds_in(
    r: &Reader<'_>,
    uid: &Uid,
    pred: &Lowered,
    params: &std::collections::BTreeMap<String, String>,
) -> bool {
    holds_near(r, uid, pred, params, &Near::default())
}

/// The cosines a run's `near(…)` predicates read: for each text, every candidate's score, as
/// the semantic source gave it. Computed once per text by the executor, so a predicate over a
/// thousand units embeds its text once.
#[derive(Debug, Default)]
pub struct Near {
    pub scores: BTreeMap<String, BTreeMap<Uid, f32>>,
}

/// [`holds_in`], with the cosines `near(…)` is judged by. A `near` whose text has no scores
/// here holds for no unit.
pub fn holds_near(
    r: &Reader<'_>,
    uid: &Uid,
    pred: &Lowered,
    params: &std::collections::BTreeMap<String, String>,
    near: &Near,
) -> bool {
    let store = r.store;
    match pred {
        Lowered::And(items) => items.iter().all(|p| holds_near(r, uid, p, params, near)),
        Lowered::Or(items) => items.iter().any(|p| holds_near(r, uid, p, params, near)),
        Lowered::Not(inner) => !holds_near(r, uid, inner, params, near),
        Lowered::Near {
            text, threshold, ..
        } => {
            let Some(text) = atom_text(text, params) else {
                return false;
            };
            let t = threshold.scaled as f32 / Decimal::SCALE as f32;
            near.scores
                .get(&text)
                .and_then(|m| m.get(uid))
                .is_some_and(|score| *score >= t)
        }
        Lowered::Matches { text, .. } => {
            let Some(query) = (match text {
                Atom::Str(s) => Some(s.clone()),
                Atom::Param(n) => params.get(n).cloned(),
            }) else {
                return false;
            };
            matches_terms(store, uid, &query)
        }
        Lowered::Cmp {
            field, op, value, ..
        } => {
            let Some(have) = read(r, uid, field) else {
                return false;
            };
            if let CmpOp::In = op {
                let Value::List(items) = value else {
                    return false;
                };
                return items
                    .iter()
                    .filter_map(|v| value_text(v, params))
                    .any(|want| want == have);
            }
            let Some(want) = value_text(value, params) else {
                return false;
            };
            if field.is_ordered() {
                let (Some(a), Some(b)) = (rank(field, &have), rank(field, &want)) else {
                    // A value that is not on the ladder cannot be compared to one that is.
                    // False rather than an error, because the predicate was legal and this
                    // unit simply does not answer it.
                    return false;
                };
                return match op {
                    CmpOp::Eq => a == b,
                    CmpOp::Ne => a != b,
                    CmpOp::Lt => a < b,
                    CmpOp::Le => a <= b,
                    CmpOp::Gt => a > b,
                    CmpOp::Ge => a >= b,
                    CmpOp::In => unreachable!("handled above"),
                };
            }
            match op {
                CmpOp::Eq => have == want,
                CmpOp::Ne => have != want,
                _ => false,
            }
        }
    }
}

/// `where matches "…"`: does the unit's indexable text hold every term of the query?
///
/// Term presence and not a score, which is what §3.3 asks for (*"term presence test"*). It
/// goes through `smysl-retrieve`'s tokeniser rather than a second one, because a filter that
/// split words differently from the index would accept units `find` cannot retrieve and
/// reject units it can.
fn matches_terms(store: &Store, uid: &Uid, query: &str) -> bool {
    let tok = smysl_retrieve::Tokenizer::plain();
    let Some(text) = smysl_retrieve::postings::document_text(store, uid) else {
        return false;
    };
    let have: std::collections::BTreeSet<String> = tok.terms(&text).into_iter().collect();
    let want = tok.terms(query);
    !want.is_empty() && want.iter().all(|t| have.contains(t))
}

/// A unit's field as text, for grouping (phase 9).
///
/// The same reader the filters use, exported because `group by` and `order by` ask the same
/// question of a unit that `where` does — and a field that read one way in a filter and
/// another in a sort would make `where x = "a" | order by x` incoherent.
pub fn field_text(store: &Store, uid: &Uid, field: &Field) -> Option<String> {
    read(&Reader::new(store), uid, field)
}

/// [`field_text`] through a reader.
pub fn field_text_in(r: &Reader<'_>, uid: &Uid, field: &Field) -> Option<String> {
    read(r, uid, field)
}

/// A sort key: `(ladder position, text)`.
///
/// One comparator for both kinds of field. An ordered field fills the position and leaves the
/// text empty; an unordered one does the reverse, so comparing the pairs sorts a status by its
/// ladder and a schema by its name without the caller choosing which.
pub fn sort_key(store: &Store, uid: &Uid, field: &Field) -> Option<(Option<i64>, String)> {
    sort_key_in(&Reader::new(store), uid, field)
}

/// [`sort_key`] through a reader.
pub fn sort_key_in(r: &Reader<'_>, uid: &Uid, field: &Field) -> Option<(Option<i64>, String)> {
    let text = read(r, uid, field)?;
    Some(if field.is_ordered() {
        (rank(field, &text), String::new())
    } else {
        (None, text)
    })
}
