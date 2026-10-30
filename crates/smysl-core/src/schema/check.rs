//! `schema check` (SMYSL-2.5 §3.11, SMYSL-2.3 A-12.1; TX-P12 step 5): rule N over every
//! revision of a schema id against **every** earlier one, the lens steps' well-formedness, and
//! totality over a store's units.
//!
//! Rule N is checked from what the declarations say, not from what a lens does to a unit: a
//! name is a type, a relation kind, a payload key or an enum value, and a revision may add
//! them, retire them, or make a key required (A-12.1 item 1). What it may not do:
//!
//! - **give an existing name a new meaning** (item 2): a key whose type changes; a rename
//!   whose target already exists in the revision it leaves (two names collapsed into one, the
//!   survivor now meaning both); a `map-value` onto a value the source already has; a
//!   `split-value` onto an existing value that is not a **declared widening** — paired with a
//!   key required from the widening revision under a condition covering the widened value,
//!   with a `default` under the same condition;
//! - **reintroduce a retired name** (item 3), in a later revision's names or as a lens target.
//!
//! Each is `SMY-E426`, naming the two revisions. A fork is `SMY-W429` (item 4); an ill-formed
//! step `SMY-E471`; an op this build does not know `SMY-W469`, which decoding raises.

use std::collections::{BTreeMap, BTreeSet};

use super::lens::{is_widening, validate};
use super::{Cond, Field, LensOp, LensStep, Revision, SchemaSet, UnitView};
use crate::diag::{Code, Diagnostic, Report};
use crate::ids::{SchemaId, Uid};
use crate::surface::HValue;
use crate::types::UnitCore;

/// What a unit's totality check found (rule 5 of §3.11).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Totality {
    /// Units of the schema, lensed to current.
    pub units: usize,
    /// Units that read whole at the current revision.
    pub whole: usize,
    /// Units a lens could not carry, with what it could not carry (empty: nothing reads).
    pub unmappable: Vec<(Uid, BTreeSet<String>)>,
}

/// Every name a revision holds, by kind, with the retired ones apart.
#[derive(Default)]
struct Names {
    types: BTreeSet<String>,
    relations: BTreeSet<String>,
    keys: BTreeSet<String>,
    retired_keys: BTreeSet<String>,
    values: BTreeMap<String, BTreeSet<String>>,
}

impl Names {
    fn of(r: &Revision) -> Names {
        let mut n = Names {
            types: r.types.iter().map(|t| t.as_str().to_string()).collect(),
            relations: r.relations.iter().map(|k| k.as_str().to_string()).collect(),
            ..Names::default()
        };
        if let Some(table) = r.fields.typed() {
            for (k, f) in table {
                if f.retired_at(r.version) {
                    n.retired_keys.insert(k.clone());
                } else {
                    n.keys.insert(k.clone());
                    n.values
                        .insert(k.clone(), f.values.iter().cloned().collect());
                }
            }
        }
        n
    }
}

fn e426(id: &SchemaId, a: u32, b: u32, what: String) -> Diagnostic {
    Diagnostic::new(Code::E426).with_message(format!("{id}@{b} against @{a}: {what}"))
}

impl SchemaSet {
    /// Rule N and step well-formedness over every revision of `id` (§3.11 items 1–4). Returns
    /// the number of errors pushed.
    pub fn check_rule_n(&self, id: &SchemaId, report: &mut Report) -> usize {
        let errors = |r: &Report| r.iter().filter(|d| d.is_error()).count();
        let before = errors(report);
        if self.forks().iter().any(|(i, _)| i == id) {
            for (i, v) in self.forks() {
                if &i == id {
                    report.push(Diagnostic::new(Code::W429).with_message(format!(
                        "{id}@{v}: two declarations of one version with different content"
                    )));
                }
            }
        }
        let revs: Vec<&Revision> = {
            let mut seen = BTreeSet::new();
            self.revisions(id)
                .iter()
                .filter(|r| seen.insert(r.version))
                .collect()
        };
        let names: Vec<Names> = revs.iter().map(|r| Names::of(r)).collect();

        // Item 3, and item 2's type changes: every revision against every earlier one.
        for (j, b) in revs.iter().enumerate() {
            for (i, a) in revs.iter().enumerate().take(j) {
                let (na, nb) = (&names[i], &names[j]);
                for k in na.retired_keys.intersection(&nb.keys) {
                    report.push(e426(
                        id,
                        a.version,
                        b.version,
                        format!("key `{k}` is retired at @{} and reintroduced", a.version),
                    ));
                }
                // A name a later revision dropped and a still later one brings back.
                for (m, mid) in revs.iter().enumerate().take(j).skip(i + 1) {
                    let nm = &names[m];
                    for t in na.types.difference(&nm.types) {
                        if nb.types.contains(t) {
                            report.push(e426(
                                id,
                                mid.version,
                                b.version,
                                format!(
                                    "type `{t}` was dropped at @{} and reintroduced",
                                    mid.version
                                ),
                            ));
                        }
                    }
                    for r in na.relations.difference(&nm.relations) {
                        if nb.relations.contains(r) {
                            report.push(e426(
                                id,
                                mid.version,
                                b.version,
                                format!(
                                    "relation kind `{r}` was dropped at @{} and reintroduced",
                                    mid.version
                                ),
                            ));
                        }
                    }
                    for (k, vals) in &na.values {
                        let (Some(vm), Some(vb)) = (nm.values.get(k), nb.values.get(k)) else {
                            continue;
                        };
                        for v in vals.difference(vm) {
                            if vb.contains(v) {
                                report.push(e426(
                                    id,
                                    mid.version,
                                    b.version,
                                    format!(
                                        "value `{v}` of `{k}` was dropped at @{} and reintroduced",
                                        mid.version
                                    ),
                                ));
                            }
                        }
                    }
                }
                let (Some(ta), Some(tb)) = (a.fields.typed(), b.fields.typed()) else {
                    continue;
                };
                for (k, fa) in ta {
                    if let Some(fb) = tb.get(k) {
                        if fa.ty != fb.ty {
                            report.push(e426(
                                id,
                                a.version,
                                b.version,
                                format!(
                                    "key `{k}` changes type, `{}` to `{}`",
                                    fa.ty.as_string(),
                                    fb.ty.as_string()
                                ),
                            ));
                        }
                    }
                }
            }
        }

        // Item 2's lens half, item 3's lens targets, and item 4 (`E471`), step by step against
        // the revision each leaves and every name retired before it.
        for step in self.steps(id) {
            let source = self.revision(id, step.from);
            let fields = source.map(|r| r.fields.clone()).unwrap_or_default();
            if let Err(e) = validate(id, step, &fields) {
                report.push(e.diagnostic());
            }
            let Some(src) = source else {
                continue;
            };
            let ns = Names::of(src);
            let mut retired: BTreeSet<String> = BTreeSet::new();
            for (r, n) in revs.iter().zip(&names) {
                if r.version <= step.from {
                    retired.extend(n.retired_keys.iter().cloned());
                }
            }
            self.check_step(id, step, src, &ns, &retired, report);
        }
        errors(report) - before
    }

    fn check_step(
        &self,
        id: &SchemaId,
        s: &LensStep,
        src: &Revision,
        ns: &Names,
        retired: &BTreeSet<String>,
        report: &mut Report,
    ) {
        let (a, b) = (s.from, s.to);
        let target = |k: &str, how: &str, report: &mut Report| {
            if retired.contains(k) {
                report.push(e426(
                    id,
                    a,
                    b,
                    format!("`{how}` targets `{k}`, a key retired by @{a}"),
                ));
            }
        };
        for op in &s.ops {
            match op {
                LensOp::RenameType { from, to } if ns.types.contains(to.as_str()) => {
                    report.push(e426(
                        id,
                        a,
                        b,
                        format!("`rename-type {from} → {to}`: `{to}` already names a type at @{a}"),
                    ));
                }
                LensOp::RenameRelation { from, to } if ns.relations.contains(to.as_str()) => {
                    report.push(e426(
                        id,
                        a,
                        b,
                        format!(
                            "`rename-relation {from} → {to}`: `{to}` already names a relation \
                             kind at @{a}"
                        ),
                    ));
                }
                LensOp::RenameKey { from, to } => {
                    if ns.keys.contains(to) {
                        report.push(e426(
                            id,
                            a,
                            b,
                            format!("`rename-key {from} → {to}`: `{to}` is already a key at @{a}"),
                        ));
                    }
                    target(to, "rename-key", report);
                }
                LensOp::MapValue { key, map } => {
                    let have = ns.values.get(key);
                    for (old, new) in map {
                        if old != new && have.is_some_and(|v| v.contains(new)) {
                            report.push(e426(
                                id,
                                a,
                                b,
                                format!(
                                    "`map-value {key}: {old} → {new}`: `{new}` is already a \
                                     value at @{a}, which now means `{old}` too"
                                ),
                            ));
                        }
                    }
                }
                LensOp::SplitValue { key, to, add, .. } => {
                    for k in add.keys() {
                        target(k, "split-value", report);
                    }
                    if is_widening(src.fields.get(key), to)
                        && !self.widening_is_declared(id, s, key, to, add)
                    {
                        report.push(e426(
                            id,
                            a,
                            b,
                            format!(
                                "`split-value {key} → {to}` widens `{to}`, which @{a} already \
                                 has, without a paired key required from @{b} when {key} = \
                                 {to} and defaulted under the same condition"
                            ),
                        ));
                    }
                }
                LensOp::Default { key, .. } | LensOp::Require { key, .. } => {
                    target(key, op.tag(), report);
                }
                _ => {}
            }
        }
    }

    /// A widening is declared when each paired key is required from the widening revision
    /// under a condition covering the widened value, with a `default` under the same
    /// condition in the step (A-12.1 item 2).
    fn widening_is_declared(
        &self,
        id: &SchemaId,
        s: &LensStep,
        key: &str,
        to: &str,
        add: &BTreeMap<String, HValue>,
    ) -> bool {
        let cond = Cond::new(key, HValue::Str(to.to_string()));
        let table = self.revision(id, s.to).and_then(|r| r.fields.typed());
        !add.is_empty()
            && add.keys().all(|k2| {
                let required = table
                    .and_then(|t| t.get(k2))
                    .is_some_and(|f: &Field| {
                        f.required_from == Some(s.to)
                            && f.required_when.as_ref().is_none_or(|c| c == &cond)
                    });
                let defaulted = s.ops.iter().any(|o| {
                    matches!(o, LensOp::Default { key: k, when: Some(c), .. } if k == k2 && c == &cond)
                });
                required && defaulted
            })
    }

    /// Totality over a store's units (§3.11 item 5): every unit of the schema lensed to its
    /// current revision; what the lens could not carry is listed (`SMY-W420`).
    pub fn check_units<'a>(
        &self,
        id: &SchemaId,
        units: impl IntoIterator<Item = (&'a Uid, &'a UnitCore)>,
        report: &mut Report,
    ) -> Totality {
        let mut t = Totality::default();
        let Some(cur) = self.current(id) else {
            return t;
        };
        for (uid, core) in units {
            if self.governing_unit(core).as_ref() != Some(id) {
                continue;
            }
            t.units += 1;
            let lost = match self.infer_revision(core) {
                None => Some(BTreeSet::new()),
                Some(g) => match self.chain(id, g.hi, cur.version) {
                    Err(_) => Some(BTreeSet::new()),
                    Ok(chain) => {
                        let mut view = UnitView::of(core);
                        match chain.forward(&mut view) {
                            super::Mapped::Whole => None,
                            super::Mapped::Partial { unmappable } => Some(unmappable),
                            super::Mapped::Unusable => Some(BTreeSet::new()),
                        }
                    }
                },
            };
            match lost {
                None => t.whole += 1,
                Some(keys) => t.unmappable.push((*uid, keys)),
            }
        }
        if !t.unmappable.is_empty() {
            report.push(Diagnostic::new(Code::W420).with_message(format!(
                "{id}: {} of {} unit(s) do not read whole at @{}",
                t.unmappable.len(),
                t.units,
                cur.version
            )));
        }
        t
    }
}
