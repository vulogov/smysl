//! Field resolution, pins and compile-forward (SMYSL-2.5 §3.2 phase 2, §3.9; TX-P12 step 3).
//!
//! Phase 2's typed half. Until FC-9 a declaration's field table was opaque and every payload
//! name was read raw; now [`smysl_core::schema`] decodes the table, so a name resolves:
//!
//! - **Unqualified** names resolve against the **current** revision of every schema that
//!   declares a typed field table, by the key as declared or by the part after its namespace
//!   (`mode` for `text:mode`), which is how §4.4's explain example spells it. Two schemas
//!   declaring it, or two keys of one schema sharing the short name, is `SMY-E421` with the
//!   candidates listed. A name no table declares is `SMY-E421` too — **unless** some unit in the
//!   store has a type no typed table governs: kernel types are never declared, nor are the
//!   schemas of stores written before FC-9, and refusing a key such a unit may carry would
//!   refuse every query written before this step. Such a name is still read raw, and the plan
//!   says so.
//! - **Qualified** `id@r:key`, and every name a `using id@r` pin's revision declares, is the
//!   field as revision `r` named it, **compiled forward** through the chain `r → current`
//!   ([`Compiler::compile`]): renames rename, a split value becomes its parts, a widened value gains its
//!   paired default, and a predicate on a retired key or a value no op carries is `SMY-E422`.
//!   Lenses never run backwards for a query.
//! - **`with lens none`** keeps a qualified reference literal: read raw, and only from units
//!   whose inferred revision interval holds the revision named; every other unit governed by
//!   that schema is unmappable on the field (`SMY-W420`), counted.
//!
//! The data side is [`crate::pred::Reader`]: a unit governed by a typed schema is read through
//! the chain from its inferred revision to the current one, so a compiled predicate and a
//! lensed unit meet at the current revision (law L8).
//!
//! **Scope is the store.** §3.2 resolves against "the schemas in scope", and phase 2 runs
//! before phase 3 has a scope, so the schemas here are every declaration the store holds.
//! A scope narrower than its store can only have fewer names, never different ones, so this
//! refuses nothing a narrower reading would accept — it can only accept, as raw, a name a
//! narrower scope would have refused.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic, Report, Span};
use smysl_core::ids::SchemaId;
use smysl_core::schema::{Field as Declared, LensChain, LensOp, SchemaSet};
use smysl_core::surface::HValue;
use smysl_graph::Store;

use crate::ast::{CmpOp, FieldRef, Pred, Value};
use crate::plan::Pin;

/// Every declaration in a store, decoded, with what resolution needs to know about the units.
#[derive(Debug, Clone, Default)]
pub struct Schemas {
    pub set: SchemaSet,
    /// Some unit's type is governed by no schema with a typed field table.
    loose: bool,
    /// Some schema has a typed field table at its current revision.
    typed: bool,
}

impl Schemas {
    /// The declarations of a store. Decoding diagnostics (`W430`, `W429`, `W469`) are the
    /// store's, reported by `check`; resolution does not repeat them.
    pub fn of(store: &Store) -> Schemas {
        let mut report = Report::new();
        let set = SchemaSet::from_decls(store.schema_decls(), &mut report);
        let typed = set.ids().any(|id| {
            SchemaId::parse(id)
                .ok()
                .and_then(|id| set.current(&id).map(|r| r.fields.typed().is_some()))
                .unwrap_or(false)
        });
        // A unit is governed by the schema declaring its type or, failing that, the one whose
        // tables declare its payload keys (`SchemaSet::governing_unit`); one governed by
        // neither, or by a schema with no typed table, makes the store loose.
        let mut loose = false;
        for (_, u) in store.units() {
            let governed = set
                .governing_unit(&u.core)
                .and_then(|id| set.current(&id).map(|r| r.fields.typed().is_some()))
                .unwrap_or(false);
            if !governed {
                loose = true;
                break;
            }
        }
        Schemas { set, loose, typed }
    }

    /// Whether any declaration types a field: when none does, resolution is phase 2's first
    /// half alone and every payload name is raw, as before FC-9.
    pub fn typed(&self) -> bool {
        self.typed
    }

    /// Whether a name no table declares may still be a payload key some unit carries.
    pub fn loose(&self) -> bool {
        self.loose
    }

    fn ids(&self) -> Vec<SchemaId> {
        self.set
            .ids()
            .filter_map(|i| SchemaId::parse(i).ok())
            .collect()
    }
}

/// A key in a table, by its declared name or by the part after its namespace.
fn lookup(table: &BTreeMap<String, Declared>, name: &str) -> Vec<String> {
    if table.contains_key(name) {
        return vec![name.to_string()];
    }
    let suffix = format!(":{name}");
    table
        .keys()
        .filter(|k| k.ends_with(&suffix))
        .cloned()
        .collect()
}

/// How a name resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolved {
    /// A key the current revision of `id` declares.
    Declared { id: SchemaId, key: String },
    /// No table declares it and some unit's type is undeclared: read raw.
    Raw,
}

/// Resolve an unqualified payload name against every current revision.
pub fn resolve_name(schemas: &Schemas, name: &str, span: Span) -> Result<Resolved, Diagnostic> {
    if !schemas.typed {
        return Ok(Resolved::Raw);
    }
    let mut found: Vec<(SchemaId, String)> = Vec::new();
    for id in schemas.ids() {
        let Some(table) = schemas.set.current(&id).and_then(|r| r.fields.typed()) else {
            continue;
        };
        for key in lookup(table, name) {
            found.push((id.clone(), key));
        }
    }
    match found.len() {
        1 => {
            let (id, key) = found.pop().expect("one");
            Ok(Resolved::Declared { id, key })
        }
        0 if schemas.loose => Ok(Resolved::Raw),
        0 => Err(Diagnostic::at(Code::E421, span)
            .with_message(format!("no schema in scope declares a field `{name}`"))),
        _ => Err(Diagnostic::at(Code::E421, span).with_message(format!(
            "`{name}` is ambiguous: {}; qualify it",
            found
                .iter()
                .map(|(id, k)| format!("{id}:{k}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// A key as revision `rev` of `id` declares it.
pub fn resolve_at(
    schemas: &Schemas,
    id: &SchemaId,
    rev: u32,
    name: &str,
    span: Span,
) -> Result<String, Diagnostic> {
    let Some(r) = schemas.set.revision(id, rev) else {
        return Err(Diagnostic::at(Code::E466, span).with_message(format!(
            "no declaration of `{id}` at revision {rev} in this store"
        )));
    };
    let Some(table) = r.fields.typed() else {
        return Err(Diagnostic::at(Code::E421, span).with_message(format!(
            "`{id}@{rev}` declares no field table, so it declares no field `{name}`"
        )));
    };
    let mut keys = lookup(table, name);
    match keys.len() {
        1 => Ok(keys.pop().expect("one")),
        0 => Err(Diagnostic::at(Code::E421, span)
            .with_message(format!("`{id}@{rev}` declares no field `{name}`"))),
        _ => Err(Diagnostic::at(Code::E421, span).with_message(format!(
            "`{name}` is ambiguous in `{id}@{rev}`: {}; write the key in full",
            keys.join(", ")
        ))),
    }
}

/// One predicate rewritten by compile-forward, for `--explain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rewrite {
    pub before: String,
    pub after: String,
    /// What did it, in the order the ops ran: `renamed`, `mapped`, `split`, `widening, paired
    /// key`.
    pub why: Vec<&'static str>,
}

/// A pin as phase 2 resolved it: the chain a pinned query is compiled through.
#[derive(Debug, Clone, PartialEq)]
pub struct PinChain {
    pub id: SchemaId,
    pub from: u32,
    pub to: u32,
    pub hash: [u8; 32],
}

/// What compile-forward needs: the declarations, and the query's `using` pins.
pub struct Compiler<'a> {
    pub schemas: &'a Schemas,
    pub pins: &'a [Pin],
    pub rewrites: Vec<Rewrite>,
    /// Every chain a predicate was compiled through.
    pub chains: BTreeMap<(String, u32), PinChain>,
}

impl<'a> Compiler<'a> {
    pub fn new(schemas: &'a Schemas, pins: &'a [Pin]) -> Compiler<'a> {
        Compiler {
            schemas,
            pins,
            rewrites: Vec::new(),
            chains: BTreeMap::new(),
        }
    }

    /// The chain a pin compiles through, recorded for `--explain` even when no predicate used
    /// it, because "pinned and compiled forward" is a fact about the plan.
    pub fn pin_chain(
        &mut self,
        id: &SchemaId,
        from: u32,
        span: Span,
    ) -> Result<PinChain, Diagnostic> {
        let Some(cur) = self.schemas.set.current(id) else {
            return Err(Diagnostic::at(Code::E466, span).with_message(format!(
                "`{id}` has no current revision (a fork, SMY-W429), so nothing can be compiled \
                 forward to it"
            )));
        };
        let to = cur.version;
        if let Some(c) = self.chains.get(&(id.as_str().to_string(), from)) {
            return Ok(c.clone());
        }
        let chain = self
            .schemas
            .set
            .chain(id, from, to)
            .map_err(|e| Diagnostic::at(e.code(), span).with_message(e.to_string()))?;
        let pc = PinChain {
            id: id.clone(),
            from,
            to,
            hash: chain.hash,
        };
        self.chains
            .insert((id.as_str().to_string(), from), pc.clone());
        Ok(pc)
    }

    /// Compile one predicate forward. `lens_none` is the stage's `with lens none`: qualified
    /// references stay literal, and lowering reads them at the revision they name.
    pub fn compile(&mut self, pred: &Pred, lens_none: bool) -> Result<Pred, Vec<Diagnostic>> {
        match pred {
            Pred::And(items) => self.all(items, lens_none).map(Pred::And),
            Pred::Or(items) => self.all(items, lens_none).map(Pred::Or),
            Pred::Not(inner) => Ok(Pred::Not(Box::new(self.compile(inner, lens_none)?))),
            Pred::Cmp { field, op, value } => {
                self.atom(field, *op, value, lens_none).map_err(|d| vec![d])
            }
            other => Ok(other.clone()),
        }
    }

    /// A field named by `order by`, `group by` or `compare by`, compiled forward: renamed to the
    /// current revision's key, refused (`SMY-E422`) if the chain retires it. There is no value
    /// to map or split, so a rename is all a sort key can take.
    pub fn field(&mut self, field: &FieldRef, lens_none: bool) -> Result<FieldRef, Diagnostic> {
        let Some((id, from, key)) = self.pinned(field)? else {
            return Ok(field.clone());
        };
        if lens_none && field.pin.is_some() {
            return Ok(field.clone());
        }
        let pc = self.pin_chain(&id, from, field.span)?;
        let chain = self
            .schemas
            .set
            .chain(&id, pc.from, pc.to)
            .map_err(|e| Diagnostic::at(e.code(), field.span).with_message(e.to_string()))?;
        let mut key = key;
        let mut renamed = false;
        for step in chain.steps() {
            for op in &step.ops {
                match op {
                    LensOp::RenameKey { from, to } if from == &key => {
                        key = to.clone();
                        renamed = true;
                    }
                    LensOp::Retire { key: k } if k == &key => {
                        return Err(Diagnostic::at(Code::E422, field.span).with_message(format!(
                            "`{}` is retired at {id}@{}, and has no forward reading",
                            field.name, step.to
                        )));
                    }
                    _ => {}
                }
            }
        }
        Ok(FieldRef {
            pin: None,
            name: if renamed || field.pin.is_some() {
                key
            } else {
                field.name.clone()
            },
            span: field.span,
        })
    }

    fn all(&mut self, items: &[Pred], lens_none: bool) -> Result<Vec<Pred>, Vec<Diagnostic>> {
        let mut out = Vec::new();
        let mut diags = Vec::new();
        for i in items {
            match self.compile(i, lens_none) {
                Ok(p) => out.push(p),
                Err(d) => diags.extend(d),
            }
        }
        if diags.is_empty() {
            Ok(out)
        } else {
            Err(diags)
        }
    }

    /// Which revision a reference names, if any: its own qualifier, or a `using` pin whose
    /// revision declares the name.
    fn pinned(&self, field: &FieldRef) -> Result<Option<(SchemaId, u32, String)>, Diagnostic> {
        if let Some(pin) = &field.pin {
            let id = SchemaId::parse(&pin.id).map_err(|_| {
                Diagnostic::at(Code::E466, field.span)
                    .with_message(format!("`{}` is not a schema id", pin.id))
            })?;
            let key = resolve_at(self.schemas, &id, pin.revision, &field.name, field.span)?;
            return Ok(Some((id, pin.revision, key)));
        }
        if crate::pred::is_kernel(&field.name) {
            return Ok(None);
        }
        for p in self.pins {
            let Ok(id) = SchemaId::parse(&p.id) else {
                continue;
            };
            let Some(table) = self
                .schemas
                .set
                .revision(&id, p.revision)
                .and_then(|r| r.fields.typed())
            else {
                continue;
            };
            let keys = lookup(table, &field.name);
            if keys.len() == 1 {
                return Ok(Some((id, p.revision, keys[0].clone())));
            }
        }
        Ok(None)
    }

    fn atom(
        &mut self,
        field: &FieldRef,
        op: CmpOp,
        value: &Value,
        lens_none: bool,
    ) -> Result<Pred, Diagnostic> {
        let original = Pred::Cmp {
            field: field.clone(),
            op,
            value: value.clone(),
        };
        let Some((id, from, key)) = self.pinned(field)? else {
            return Ok(original);
        };
        if lens_none && field.pin.is_some() {
            return Ok(original);
        }
        let pc = self.pin_chain(&id, from, field.span)?;
        // A qualified name is spelled as declared from here on: the pin is gone, and an
        // unqualified spelling resolves against the current revision, where the short form
        // might mean something else.
        let spell = if field.pin.is_some() {
            key.clone()
        } else {
            field.name.clone()
        };
        let mut tree = Tree::Atom(Atom {
            key: key.clone(),
            spell,
            op,
            value: value.clone(),
            span: field.span,
        });
        if pc.from == pc.to {
            respell(&mut tree, self.schemas, &id);
            return Ok(tree.into_pred());
        }
        let chain = self
            .schemas
            .set
            .chain(&id, pc.from, pc.to)
            .map_err(|e| Diagnostic::at(e.code(), field.span).with_message(e.to_string()))?;
        let mut why = Vec::new();
        forward(&mut tree, &chain, self.schemas, &id, &mut why)?;
        respell(&mut tree, self.schemas, &id);
        let after = tree.into_pred();
        if !why.is_empty() {
            self.rewrites.push(Rewrite {
                before: crate::print::pred_text(&original),
                after: crate::print::pred_text(&after),
                why,
            });
        }
        Ok(after)
    }
}

/// A short spelling survives only where it still means its key at the current revision: in a
/// store where another schema declares the same short name, `mode` at revision 1 of one schema
/// must not come out ambiguous against the other's (found by GE-T16, TX-P12 step 9).
fn respell(tree: &mut Tree, schemas: &Schemas, id: &SchemaId) {
    tree.respell(&mut |a: &mut Atom| {
        if a.spell != a.key {
            let same = matches!(
                resolve_name(schemas, &a.spell, a.span),
                Ok(Resolved::Declared { id: ref i, key: ref k }) if i == id && *k == a.key
            );
            if !same {
                a.spell = a.key.clone();
            }
        }
    });
}

/// A predicate under rewrite: atoms carry the key they compare, which a rename moves and a
/// spelling does not.
#[derive(Debug, Clone)]
enum Tree {
    And(Vec<Tree>),
    Or(Vec<Tree>),
    Not(Box<Tree>),
    Atom(Atom),
}

#[derive(Debug, Clone)]
struct Atom {
    key: String,
    spell: String,
    op: CmpOp,
    value: Value,
    span: Span,
}

impl Atom {
    fn equals(key: &str, value: Value, span: Span) -> Atom {
        Atom {
            key: key.to_string(),
            spell: key.to_string(),
            op: CmpOp::Eq,
            value,
            span,
        }
    }
}

impl Tree {
    fn into_pred(self) -> Pred {
        match self {
            Tree::And(v) => Pred::And(v.into_iter().map(Tree::into_pred).collect()),
            Tree::Or(v) => Pred::Or(v.into_iter().map(Tree::into_pred).collect()),
            Tree::Not(t) => Pred::Not(Box::new(t.into_pred())),
            Tree::Atom(a) => Pred::Cmp {
                field: FieldRef {
                    pin: None,
                    name: a.spell,
                    span: a.span,
                },
                op: a.op,
                value: a.value,
            },
        }
    }

    /// Apply `f` to every atom present before this pass, replacing each by what it returns.
    /// Atoms an op creates are not visited by the same op, so a split does not re-split the
    /// value it produced.
    fn map(self, f: &mut dyn FnMut(Atom) -> Result<Tree, Diagnostic>) -> Result<Tree, Diagnostic> {
        Ok(match self {
            Tree::And(v) => Tree::And(v.into_iter().map(|t| t.map(f)).collect::<Result<_, _>>()?),
            Tree::Or(v) => Tree::Or(v.into_iter().map(|t| t.map(f)).collect::<Result<_, _>>()?),
            Tree::Not(t) => Tree::Not(Box::new(t.map(f)?)),
            Tree::Atom(a) => f(a)?,
        })
    }

    fn respell(&mut self, f: &mut dyn FnMut(&mut Atom)) {
        match self {
            Tree::And(v) | Tree::Or(v) => v.iter_mut().for_each(|t| t.respell(f)),
            Tree::Not(t) => t.respell(f),
            Tree::Atom(a) => f(a),
        }
    }

    fn atoms(&self, out: &mut Vec<Atom>) {
        match self {
            Tree::And(v) | Tree::Or(v) => v.iter().for_each(|t| t.atoms(out)),
            Tree::Not(t) => t.atoms(out),
            Tree::Atom(a) => out.push(a.clone()),
        }
    }
}

/// A written value as text, when it has one a lens can compare.
fn text(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) | Value::Word(s) | Value::Uid(s) => Some(s.clone()),
        Value::Int(i) => Some(i.to_string()),
        _ => None,
    }
}

/// A value an op names, spelled as a query would write it: a bare word when the lexer reads
/// one, else quoted.
fn spelled(s: &str) -> Value {
    let word = s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if word {
        Value::Word(s.to_string())
    } else {
        Value::Str(s.to_string())
    }
}

/// A payload value as a query literal.
fn literal(v: &HValue) -> Value {
    match v {
        HValue::Str(s) => spelled(s),
        HValue::Int(i) => Value::Int(*i),
        HValue::Bool(b) => Value::Word(b.to_string()),
        HValue::Null => Value::Word("null".to_string()),
        other => Value::Str(format!("{other:?}")),
    }
}

/// Re-spell a value the way the original was spelled, when it was quoted.
fn like(orig: &Value, new: &str) -> Value {
    match orig {
        Value::Str(_) => Value::Str(new.to_string()),
        _ => spelled(new),
    }
}

fn note(why: &mut Vec<&'static str>, w: &'static str) {
    if !why.contains(&w) {
        why.push(w);
    }
}

/// Run the chain's steps over a predicate, ops in declared order (§3.9's compile-forward).
fn forward(
    tree: &mut Tree,
    chain: &LensChain,
    schemas: &Schemas,
    id: &SchemaId,
    why: &mut Vec<&'static str>,
) -> Result<(), Diagnostic> {
    let widenings = chain.widenings();
    for step in chain.steps() {
        for op in &step.ops {
            let t = std::mem::replace(tree, Tree::And(Vec::new()));
            *tree = t.map(&mut |a: Atom| -> Result<Tree, Diagnostic> {
                match op {
                    LensOp::RenameKey { from, to } if &a.key == from => {
                        note(why, "renamed");
                        Ok(Tree::Atom(Atom {
                            key: to.clone(),
                            spell: to.clone(),
                            ..a
                        }))
                    }
                    LensOp::Retire { key } if &a.key == key => {
                        Err(Diagnostic::at(Code::E422, a.span).with_message(format!(
                            "`{}` is retired at {id}@{}, and a predicate on it has no forward \
                             reading",
                            a.spell, step.to
                        )))
                    }
                    LensOp::MapValue { key, map } if &a.key == key => {
                        let mut a = a;
                        let mut hit = false;
                        a.value = match &a.value {
                            Value::List(items) => Value::List(
                                items
                                    .iter()
                                    .map(|v| match text(v).and_then(|t| map.get(&t)) {
                                        Some(n) => {
                                            hit = true;
                                            like(v, n)
                                        }
                                        None => v.clone(),
                                    })
                                    .collect(),
                            ),
                            v => match text(v).and_then(|t| map.get(&t)) {
                                Some(n) => {
                                    hit = true;
                                    like(v, n)
                                }
                                None => v.clone(),
                            },
                        };
                        if hit {
                            note(why, "mapped");
                        }
                        Ok(Tree::Atom(a))
                    }
                    LensOp::SplitValue { key, from, to, add } if &a.key == key => {
                        let widened = widenings
                            .iter()
                            .find(|w| w.from == step.from && &w.key == key && &w.value == to);
                        split(a, step, from, to, add, widened.is_some(), why)
                    }
                    _ => Ok(Tree::Atom(a)),
                }
            })?;
        }
    }
    // A value the chain did not carry into the current revision's values is a retired value
    // with no mapping (§3.9): comparing against it would ask about something no unit at the
    // current revision can hold.
    let Some(table) = schemas.set.current(id).and_then(|r| r.fields.typed()) else {
        return Ok(());
    };
    let mut atoms = Vec::new();
    tree.atoms(&mut atoms);
    for a in atoms {
        let Some(f) = table.get(&a.key) else {
            continue;
        };
        if f.retired_at(chain.to) {
            return Err(Diagnostic::at(Code::E422, a.span).with_message(format!(
                "`{}` is retired at {id}@{}, and a predicate on it has no forward reading",
                a.spell, chain.to
            )));
        }
        let values: Vec<String> = match &a.value {
            Value::List(items) => items.iter().filter_map(text).collect(),
            v => text(v).into_iter().collect(),
        };
        for v in values {
            if !f.admits(&HValue::Str(v.clone())) {
                return Err(Diagnostic::at(Code::E422, a.span).with_message(format!(
                    "`{v}` is not a value of `{}` at {id}@{}, and no lens on the path maps it",
                    a.key, chain.to
                )));
            }
        }
    }
    Ok(())
}

/// `split-value k = v → {k: v', k2: x…}` over one atom: `k = v` becomes `k = v' and k2 = x`,
/// and when the split is a widening, `k = v'` becomes `k = v' and k2 = d` — the old, narrow
/// meaning, `d` from the step's paired `default k2 = d when k = v'`.
fn split(
    a: Atom,
    step: &smysl_core::schema::LensStep,
    from: &str,
    to: &str,
    add: &BTreeMap<String, HValue>,
    widening: bool,
    why: &mut Vec<&'static str>,
) -> Result<Tree, Diagnostic> {
    let parts = |a: &Atom| -> Tree {
        let mut v = vec![Tree::Atom(Atom {
            op: CmpOp::Eq,
            value: like(&a.value, to),
            ..a.clone()
        })];
        for (k, x) in add {
            v.push(Tree::Atom(Atom::equals(k, literal(x), a.span)));
        }
        Tree::And(v)
    };
    let narrow = |a: &Atom| -> Result<Tree, Diagnostic> {
        let mut v = vec![Tree::Atom(Atom {
            op: CmpOp::Eq,
            ..a.clone()
        })];
        let paired: BTreeSet<&String> = add.keys().collect();
        for k2 in paired {
            let d = step.ops.iter().find_map(|o| match o {
                LensOp::Default {
                    key,
                    value,
                    when: Some(c),
                } if key == k2 && c.key == a.key && c.value == HValue::Str(to.to_string()) => {
                    Some(value)
                }
                _ => None,
            });
            let Some(d) = d else {
                return Err(Diagnostic::at(Code::E471, a.span).with_message(format!(
                    "the widening of `{}` = {to} at {}→{} has no `default {k2} when {} = {to}`",
                    a.key, step.from, step.to, a.key
                )));
            };
            v.push(Tree::Atom(Atom::equals(k2, literal(d), a.span)));
        }
        Ok(Tree::And(v))
    };
    let is = |v: &Value, s: &str| text(v).is_some_and(|t| t == s);
    match a.op {
        CmpOp::Eq if is(&a.value, from) => {
            note(why, "split");
            Ok(parts(&a))
        }
        CmpOp::Eq if widening && is(&a.value, to) => {
            note(why, "widening, paired key");
            narrow(&a)
        }
        CmpOp::Ne if is(&a.value, from) => {
            note(why, "split");
            Ok(Tree::Not(Box::new(parts(&a))))
        }
        CmpOp::Ne if widening && is(&a.value, to) => {
            note(why, "widening, paired key");
            Ok(Tree::Not(Box::new(narrow(&a)?)))
        }
        CmpOp::In => {
            let Value::List(items) = &a.value else {
                return Ok(Tree::Atom(a));
            };
            let touched = items.iter().any(|v| is(v, from) || (widening && is(v, to)));
            if !touched {
                return Ok(Tree::Atom(a));
            }
            let mut arms = Vec::new();
            let rest: Vec<Value> = items
                .iter()
                .filter(|v| !is(v, from) && !(widening && is(v, to)))
                .cloned()
                .collect();
            if !rest.is_empty() {
                arms.push(Tree::Atom(Atom {
                    value: Value::List(rest),
                    ..a.clone()
                }));
            }
            for v in items {
                let one = Atom {
                    op: CmpOp::Eq,
                    value: v.clone(),
                    ..a.clone()
                };
                if is(v, from) {
                    note(why, "split");
                    arms.push(parts(&one));
                } else if widening && is(v, to) {
                    note(why, "widening, paired key");
                    arms.push(narrow(&one)?);
                }
            }
            Ok(Tree::Or(arms))
        }
        _ => Ok(Tree::Atom(a)),
    }
}
