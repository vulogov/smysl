//! Predicate resolution and evaluation (SMYSL-2.5 §3.3, TX-P6 step 4, planner phase 6).
//!
//! # What this build can read, and what it cannot
//!
//! The step's own clause is *"filters (kernel fields, raw payload — lenses come in TX-P12)"*,
//! and the three words in brackets are the whole boundary. A kernel field has a type this
//! crate knows: `kind` is a `KernelType`, `status` is an ordered enum, `commitment` is another,
//! `lang` is a tag. A payload key does not — a declaration's `payload_shape` is opaque CBOR
//! until FC-9 decoding lands, which is why planner phase 2 records `fields_checked: false`.
//!
//! So a payload key is read **raw**: the value is whatever the payload object holds under that
//! name, compared as text. That is honest and it is also limited, and the limit is visible in
//! one place — an ordering comparison on a raw key is refused rather than guessed, because
//! `>` on text whose type nobody has declared would answer a question the query did not ask.
//!
//! A **qualified** reference (`x.text/v1@1:mode`) is refused outright. It means *the field as
//! that revision named it, compiled forward* (SMYSL-2.5 §3.9, OQ-77), and compile-forward is
//! TX-P12 step 3's. Refusing is the only safe answer: reading it unlensed would silently give
//! the current revision's field under an older revision's name.

use smysl_core::diag::{Code, Diagnostic, Span};
use smysl_core::ids::{SchemaId, Uid};
use smysl_core::surface::{payload_to_object, HValue, Spanned};
use smysl_core::types::{Commitment, Status};
use smysl_graph::Store;

use crate::ast::{Atom, CmpOp, FieldRef, Pred, Value};

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
    /// A payload key, read raw and compared as text (lenses are TX-P12's).
    Payload(String),
}

impl Field {
    /// Whether `<`, `<=`, `>` and `>=` mean anything for this field.
    ///
    /// Two fields are ordered and the order is declared in the kernel, not inferred here.
    /// Everything else is not: `kind` and `schema` are names, `lang` is a tag, and a raw
    /// payload value has no declared type at all until FC-9.
    pub fn is_ordered(&self) -> bool {
        matches!(self, Field::Status | Field::Commitment)
    }

    pub fn name(&self) -> &str {
        match self {
            Field::Kind => "kind",
            Field::Schema => "schema",
            Field::Status => "status",
            Field::Commitment => "commitment",
            Field::Lang => "lang",
            Field::Payload(k) => k,
        }
    }
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
}

/// Phase 6 for one predicate.
pub fn lower(pred: &Pred, diags: &mut Vec<Diagnostic>) -> Option<Lowered> {
    match pred {
        Pred::And(items) => lower_all(items, diags).map(Lowered::And),
        Pred::Or(items) => lower_all(items, diags).map(Lowered::Or),
        Pred::Not(inner) => lower(inner, diags).map(|l| Lowered::Not(Box::new(l))),
        Pred::Matches(text) => Some(Lowered::Matches {
            text: text.clone(),
            span: Span::new(0, 0),
        }),
        Pred::Near { .. } => {
            // Cosine against an embedding. Not a later phase of this planner: a different tier.
            diags.push(
                Diagnostic::at(Code::E461, Span::new(0, 0)).with_message(
                    "`near(…)` compares against an embedding, which this pure build has no port \
                 bound for"
                        .to_string(),
                ),
            );
            None
        }
        Pred::Cmp { field, op, value } => {
            let resolved = resolve(field, diags)?;
            if matches!(op, CmpOp::Lt | CmpOp::Le | CmpOp::Gt | CmpOp::Ge) && !resolved.is_ordered()
            {
                diags.push(Diagnostic::at(Code::E512, field.span).with_message(format!(
                    "`{}` has no order, so `{}` cannot be asked of it{}",
                    resolved.name(),
                    op_text(*op),
                    match resolved {
                        Field::Payload(_) =>
                            ": a payload key's type is not declared to this build until FC-9, \
                             and ordering undeclared text would answer a different question",
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

fn lower_all(items: &[Pred], diags: &mut Vec<Diagnostic>) -> Option<Vec<Lowered>> {
    let mut out = Vec::with_capacity(items.len());
    let mut ok = true;
    for item in items {
        match lower(item, diags) {
            Some(l) => out.push(l),
            None => ok = false,
        }
    }
    // Every branch is lowered before returning, so one query reports every unusable field
    // rather than the first: a person fixing a predicate should see the whole list.
    ok.then_some(out)
}

/// Resolve a name to a field, refusing what this build cannot read.
///
/// Public because phase 9 resolves fields too: `order by`, `group by` and `compare by` each
/// name one, and a field that is unreadable in a filter is unreadable in a sort.
pub fn resolve(field: &FieldRef, diags: &mut Vec<Diagnostic>) -> Option<Field> {
    if let Some(pin) = &field.pin {
        diags.push(Diagnostic::at(Code::E461, field.span).with_message(format!(
            "`{}@{}:{}` is the field as that revision named it, compiled forward \
             (SMYSL-2.5 §3.9), and compile-forward is TX-P12's; this build has no lens engine",
            pin.id, pin.revision, field.name
        )));
        return None;
    }
    Some(match field.name.as_str() {
        "kind" => Field::Kind,
        "schema" => Field::Schema,
        "status" => Field::Status,
        "commitment" => Field::Commitment,
        "lang" => Field::Lang,
        // Anything else is a payload key. Not refused as unknown: `SMY-E421` is "no schema in
        // scope declares this field", and this build cannot tell a declared key from an
        // undeclared one because it cannot read a declaration's shape. Treating the name as a
        // raw key is the reading that does not claim knowledge it lacks — and the one that
        // makes `where holder = "…"` work on a store whose payloads say so.
        other => Field::Payload(other.to_string()),
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
/// unequal either — see [`holds`].
fn read(store: &Store, uid: &Uid, field: &Field) -> Option<String> {
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
            object.get(key).map(|v| text_of(&v.value))?
        }
    })
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

/// Where an ordered field's value sits on its ladder.
fn rank(field: &Field, text: &str) -> Option<u8> {
    match field {
        Field::Status => Status::parse(text).map(Status::as_u8),
        Field::Commitment => Commitment::parse(text).map(Commitment::as_u8),
        _ => None,
    }
}

/// Does this unit satisfy the predicate?
///
/// A missing field makes a comparison **false**, including `!=`. That is deliberate and it is
/// the only reading that keeps `where x = "a"` and `where x != "a"` from partitioning the
/// store: a unit with no `x` is not a unit whose `x` differs. `not (x = "a")` is how a caller
/// asks for the complement, and it says so.
pub fn holds(
    store: &Store,
    uid: &Uid,
    pred: &Lowered,
    params: &std::collections::BTreeMap<String, String>,
) -> bool {
    match pred {
        Lowered::And(items) => items.iter().all(|p| holds(store, uid, p, params)),
        Lowered::Or(items) => items.iter().any(|p| holds(store, uid, p, params)),
        Lowered::Not(inner) => !holds(store, uid, inner, params),
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
            let Some(have) = read(store, uid, field) else {
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
    read(store, uid, field)
}

/// A sort key: `(ladder position, text)`.
///
/// One comparator for both kinds of field. An ordered field fills the position and leaves the
/// text empty; an unordered one does the reverse, so comparing the pairs sorts a status by its
/// ladder and a schema by its name without the caller choosing which.
pub fn sort_key(store: &Store, uid: &Uid, field: &Field) -> Option<(Option<u8>, String)> {
    let text = read(store, uid, field)?;
    Some(if field.is_ordered() {
        (rank(field, &text), String::new())
    } else {
        (None, text)
    })
}
