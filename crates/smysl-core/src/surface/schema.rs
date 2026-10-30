//! `@schema`'s `fields:` and `lenses:` (SMYSL-2.3 A-10 item 4; SMYSL-2.5 TX-P12 step 7).
//!
//! A-7's field map and Appendix B's lens steps, spelled as HJSON with their keys written as
//! names — `type`, `values`, `retired-in`, `note`, `required-from`, `required-when`, `literals`
//! for a field; `from`, `to`, `ops` for a step; `tag`, then `from`/`to`, `key`, `map`, `add`,
//! `value`, `when` for an op:
//!
//! ```text
//! @schema x.text/v1 {
//!   version: 2,
//!   types: [x.text/event],
//!   fields: {
//!     "text:mode": { type: enum, values: [asserted, reported] },
//!     "text:verbatim": { type: bool, required-from: 2, required-when: { "text:mode": reported } },
//!   },
//!   lenses: [
//!     { from: 1, to: 2, ops: [
//!       { tag: split-value, key: "text:mode", from: quoted, to: reported, add: { "text:verbatim": true } },
//!       { tag: require, key: "text:verbatim", when: { "text:mode": reported } },
//!       { tag: default, key: "text:verbatim", value: false, when: { "text:mode": reported } },
//!     ] },
//!   ],
//! }
//! ```
//!
//! **Spelled only when nothing is lost.** The CBOR is canonical by construction when this module
//! writes it, but a declaration read off the wire may carry key 4 or 5 in another encoding, an
//! op this build does not know, or a shape that does not match (`SMY-W430`). Such a declaration
//! stays CBOR-only and is counted (F-18): a surface form exists exactly when re-encoding what
//! was decoded gives back the bytes that arrived, so `fmt` round-trips it byte for byte.

use std::collections::BTreeMap;

use crate::diag::{Report, Span};
use crate::schema::{
    decode, encode_lenses, encode_shape, Cond, Field, FieldType, Fields, LensOp, LensStep,
};
use crate::surface::hjson::{HObject, HValue, Spanned};
use crate::types::annex::SchemaDecl;
use crate::types::RelKind;
use crate::SchemaId;

/// Whether keys 4 and 5 can be spelled and read back to the same bytes.
pub(crate) fn spellable(d: &SchemaDecl) -> bool {
    if d.payload_shape.is_none() && d.lenses.is_none() {
        return true;
    }
    let mut report = Report::new();
    let rev = decode(d, &mut report);
    if let Some(bytes) = &d.payload_shape {
        match &rev.fields {
            Fields::Typed(t) if &encode_shape(t) == bytes => {}
            _ => return false,
        }
    }
    if let Some(bytes) = &d.lenses {
        if rev.lenses_opaque || rev.lenses.iter().any(|s| !s.is_usable()) {
            return false;
        }
        let canonical: Vec<LensStep> = rev
            .lenses
            .iter()
            .map(|s| LensStep::new(s.from, s.to, s.ops.clone()))
            .collect();
        if &encode_lenses(&canonical) != bytes {
            return false;
        }
    }
    true
}

/// The `fields:` and `lenses:` lines of a spellable declaration, each with its trailing comma,
/// indented for a multi-line header.
pub(crate) fn write(
    out: &mut String,
    d: &SchemaDecl,
    value: impl Fn(&HValue) -> String,
    key: impl Fn(&str) -> String,
) {
    let mut report = Report::new();
    let rev = decode(d, &mut report);
    let cond = |c: &Cond| format!("{{ {}: {} }}", key(&c.key), value(&c.value));
    let list = |v: &[String]| {
        format!(
            "[{}]",
            v.iter()
                .map(|s| value(&HValue::Str(s.clone())))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    if let Fields::Typed(table) = &rev.fields {
        out.push_str("  fields: {\n");
        for (k, f) in table {
            let mut parts = vec![format!("type: {}", value(&HValue::Str(f.ty.as_string())))];
            if !f.values.is_empty() {
                parts.push(format!("values: {}", list(&f.values)));
            }
            if let Some(r) = f.retired_in {
                parts.push(format!("retired-in: {r}"));
            }
            if let Some(n) = &f.note {
                parts.push(format!("note: {}", value(&HValue::Str(n.clone()))));
            }
            if let Some(r) = f.required_from {
                parts.push(format!("required-from: {r}"));
            }
            if let Some(c) = &f.required_when {
                parts.push(format!("required-when: {}", cond(c)));
            }
            if !f.literals.is_empty() {
                parts.push(format!("literals: {}", list(&f.literals)));
            }
            out.push_str(&format!("    {}: {{ {} }},\n", key(k), parts.join(", ")));
        }
        out.push_str("  },\n");
    }
    if d.lenses.is_some() {
        out.push_str("  lenses: [\n");
        for s in &rev.lenses {
            out.push_str(&format!("    {{ from: {}, to: {}, ops: [\n", s.from, s.to));
            for op in &s.ops {
                let text = |s: &str| value(&HValue::Str(s.to_string()));
                let body = match op {
                    LensOp::RenameType { from, to } => {
                        format!("from: {}, to: {}", text(from.as_str()), text(to.as_str()))
                    }
                    LensOp::RenameRelation { from, to } => {
                        format!("from: {}, to: {}", text(from.as_str()), text(to.as_str()))
                    }
                    LensOp::RenameKey { from, to } => {
                        format!("from: {}, to: {}", text(from), text(to))
                    }
                    LensOp::MapValue { key: k, map } => format!(
                        "key: {}, map: {{ {} }}",
                        text(k),
                        map.iter()
                            .map(|(a, b)| format!("{}: {}", key(a), text(b)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    LensOp::SplitValue {
                        key: k,
                        from,
                        to,
                        add,
                    } => format!(
                        "key: {}, from: {}, to: {}, add: {{ {} }}",
                        text(k),
                        text(from),
                        text(to),
                        add.iter()
                            .map(|(a, b)| format!("{}: {}", key(a), value(b)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    LensOp::Require { key: k, when } => match when {
                        Some(c) => format!("key: {}, when: {}", text(k), cond(c)),
                        None => format!("key: {}", text(k)),
                    },
                    LensOp::Default {
                        key: k,
                        value: v,
                        when,
                    } => match when {
                        Some(c) => {
                            format!("key: {}, value: {}, when: {}", text(k), value(v), cond(c))
                        }
                        None => format!("key: {}, value: {}", text(k), value(v)),
                    },
                    LensOp::Retire { key: k } => format!("key: {}", text(k)),
                    // Unreachable: a declaration with an unknown op is not spellable.
                    LensOp::Unknown { .. } => String::new(),
                };
                out.push_str(&format!("      {{ tag: {}, {body} }},\n", op.tag()));
            }
            out.push_str("    ] },\n");
        }
        out.push_str("  ],\n");
    }
}

/// Why a `fields:` or `lenses:` value did not read, and where.
pub(crate) struct Bad(pub Span, pub String);

fn obj<'a>(v: &'a Spanned<HValue>, what: &str) -> Result<&'a HObject, Bad> {
    match &v.value {
        HValue::Object(o) => Ok(o),
        _ => Err(Bad(v.span, format!("{what} must be a map"))),
    }
}

fn text_of(v: &Spanned<HValue>, what: &str) -> Result<String, Bad> {
    v.value
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| Bad(v.span, format!("{what} must be text")))
}

fn rev_of(v: &Spanned<HValue>, what: &str) -> Result<u32, Bad> {
    v.value
        .as_int()
        .and_then(|n| u32::try_from(n).ok())
        .ok_or_else(|| Bad(v.span, format!("{what} must be a revision number")))
}

fn texts_of(v: &Spanned<HValue>, what: &str) -> Result<Vec<String>, Bad> {
    match &v.value {
        HValue::Array(items) => items.iter().map(|i| text_of(i, what)).collect(),
        _ => Err(Bad(v.span, format!("{what} must be an array of text"))),
    }
}

/// A payload value as A-7 stores it: spans are not part of a value.
fn plain(v: &HValue) -> HValue {
    match v {
        HValue::Array(items) => HValue::Array(
            items
                .iter()
                .map(|i| Spanned::new(plain(&i.value), Span::new(0, 0)))
                .collect(),
        ),
        HValue::Object(o) => {
            let mut out = HObject::new();
            for (k, v) in o.iter() {
                out.insert(
                    Spanned::new(k.value.clone(), Span::new(0, 0)),
                    Spanned::new(plain(&v.value), Span::new(0, 0)),
                );
            }
            HValue::Object(out)
        }
        other => other.clone(),
    }
}

fn cond_of(v: &Spanned<HValue>, what: &str) -> Result<Cond, Bad> {
    let o = obj(v, what)?;
    let mut it = o.iter();
    match (it.next(), it.next()) {
        (Some((k, val)), None) => Ok(Cond::new(k.value.clone(), plain(&val.value))),
        _ => Err(Bad(v.span, format!("{what} is a map with exactly one key"))),
    }
}

/// Refuse a key the map does not take: a misspelt `required_from` passed over would leave the
/// key unrequired under a file that visibly requires it.
fn only(o: &HObject, allowed: &[&str], what: &str) -> Result<(), Bad> {
    for (k, _) in o.iter() {
        if !allowed.contains(&k.value.as_str()) {
            return Err(Bad(
                k.span,
                format!(
                    "{what} has no key `{}`; it takes {}",
                    k.value,
                    allowed.join(", ")
                ),
            ));
        }
    }
    Ok(())
}

/// `fields:`, as key 4's bytes.
pub(crate) fn parse_fields(v: &Spanned<HValue>) -> Result<Vec<u8>, Bad> {
    let o = obj(v, "`fields`")?;
    let mut table = BTreeMap::new();
    for (k, fv) in o.iter() {
        let fo = obj(fv, &format!("field `{}`", k.value))?;
        only(
            fo,
            &[
                "type",
                "values",
                "retired-in",
                "note",
                "required-from",
                "required-when",
                "literals",
            ],
            &format!("field `{}`", k.value),
        )?;
        let ty = fo
            .get("type")
            .ok_or_else(|| Bad(fv.span, format!("field `{}` needs a `type`", k.value)))?;
        let mut f = Field::new(FieldType::parse(&text_of(ty, "`type`")?));
        if let Some(x) = fo.get("values") {
            f.values = texts_of(x, "`values`")?;
        }
        if let Some(x) = fo.get("retired-in") {
            f.retired_in = Some(rev_of(x, "`retired-in`")?);
        }
        if let Some(x) = fo.get("note") {
            f.note = Some(text_of(x, "`note`")?);
        }
        if let Some(x) = fo.get("required-from") {
            f.required_from = Some(rev_of(x, "`required-from`")?);
        }
        if let Some(x) = fo.get("required-when") {
            f.required_when = Some(cond_of(x, "`required-when`")?);
        }
        if let Some(x) = fo.get("literals") {
            f.literals = texts_of(x, "`literals`")?;
        }
        table.insert(k.value.clone(), f);
    }
    Ok(encode_shape(&table))
}

/// `lenses:`, as key 5's bytes.
pub(crate) fn parse_lenses(v: &Spanned<HValue>) -> Result<Vec<u8>, Bad> {
    let HValue::Array(items) = &v.value else {
        return Err(Bad(v.span, "`lenses` must be an array of steps".into()));
    };
    let mut steps = Vec::new();
    for item in items {
        let so = obj(item, "a lens step")?;
        only(so, &["from", "to", "ops"], "a lens step")?;
        let from = rev_of(
            so.get("from")
                .ok_or_else(|| Bad(item.span, "a lens step needs `from`".into()))?,
            "`from`",
        )?;
        let to = rev_of(
            so.get("to")
                .ok_or_else(|| Bad(item.span, "a lens step needs `to`".into()))?,
            "`to`",
        )?;
        if to <= from {
            return Err(Bad(
                item.span,
                "a lens step goes up: `to` is above `from`".into(),
            ));
        }
        let mut ops = Vec::new();
        if let Some(list) = so.get("ops") {
            let HValue::Array(list) = &list.value else {
                return Err(Bad(list.span, "`ops` must be an array".into()));
            };
            for o in list {
                ops.push(op_of(o)?);
            }
        }
        steps.push(LensStep::new(from, to, ops));
    }
    Ok(encode_lenses(&steps))
}

fn op_of(v: &Spanned<HValue>) -> Result<LensOp, Bad> {
    let o = obj(v, "a lens op")?;
    let tag = text_of(
        o.get("tag")
            .ok_or_else(|| Bad(v.span, "a lens op needs a `tag`".into()))?,
        "`tag`",
    )?;
    let need = |k: &str| {
        o.get(k)
            .ok_or_else(|| Bad(v.span, format!("`{tag}` needs `{k}`")))
    };
    let sid = |k: &str| -> Result<SchemaId, Bad> {
        let x = need(k)?;
        SchemaId::parse(&text_of(x, k)?).map_err(|_| Bad(x.span, format!("`{k}` is a schema id")))
    };
    let rel = |k: &str| -> Result<RelKind, Bad> {
        let x = need(k)?;
        RelKind::parse(&text_of(x, k)?)
            .map_err(|_| Bad(x.span, format!("`{k}` is a relation kind")))
    };
    let when =
        || -> Result<Option<Cond>, Bad> { o.get("when").map(|w| cond_of(w, "`when`")).transpose() };
    let op = match tag.as_str() {
        "rename-type" => {
            only(o, &["tag", "from", "to"], "`rename-type`")?;
            LensOp::RenameType {
                from: sid("from")?,
                to: sid("to")?,
            }
        }
        "rename-relation" => {
            only(o, &["tag", "from", "to"], "`rename-relation`")?;
            LensOp::RenameRelation {
                from: rel("from")?,
                to: rel("to")?,
            }
        }
        "rename-key" => {
            only(o, &["tag", "from", "to"], "`rename-key`")?;
            LensOp::RenameKey {
                from: text_of(need("from")?, "`from`")?,
                to: text_of(need("to")?, "`to`")?,
            }
        }
        "map-value" => {
            only(o, &["tag", "key", "map"], "`map-value`")?;
            let m = obj(need("map")?, "`map`")?;
            let mut map = BTreeMap::new();
            for (a, b) in m.iter() {
                map.insert(a.value.clone(), text_of(b, "a `map` value")?);
            }
            LensOp::MapValue {
                key: text_of(need("key")?, "`key`")?,
                map,
            }
        }
        "split-value" => {
            only(o, &["tag", "key", "from", "to", "add"], "`split-value`")?;
            let a = obj(need("add")?, "`add`")?;
            let mut add = BTreeMap::new();
            for (k, x) in a.iter() {
                add.insert(k.value.clone(), plain(&x.value));
            }
            LensOp::SplitValue {
                key: text_of(need("key")?, "`key`")?,
                from: text_of(need("from")?, "`from`")?,
                to: text_of(need("to")?, "`to`")?,
                add,
            }
        }
        "require" => {
            only(o, &["tag", "key", "when"], "`require`")?;
            LensOp::Require {
                key: text_of(need("key")?, "`key`")?,
                when: when()?,
            }
        }
        "default" => {
            only(o, &["tag", "key", "value", "when"], "`default`")?;
            LensOp::Default {
                key: text_of(need("key")?, "`key`")?,
                value: plain(&need("value")?.value),
                when: when()?,
            }
        }
        "retire" => {
            only(o, &["tag", "key"], "`retire`")?;
            LensOp::Retire {
                key: text_of(need("key")?, "`key`")?,
            }
        }
        other => {
            return Err(Bad(
                v.span,
                format!(
                    "`{other}` is not a lens op this build can spell; a declaration carrying one \
                     travels as CBOR"
                ),
            ))
        }
    };
    Ok(op)
}
