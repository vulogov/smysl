//! FC-9: record 8 keys 4 and 5, read into types and written back (SMYSL-2.3 A-7, Appendix B).
//!
//! Two rules shape every function here. **Nothing is misread:** a key 4 or key 5 that does not
//! match its structure leaves that half of the revision opaque and says so (`SMY-W430`), and an
//! op this build cannot read in full — an unknown tag, or a known tag carrying a key it does
//! not know — is [`LensOp::Unknown`], whose step is unusable (`SMY-W469`). **Nothing is
//! re-encoded on the way through:** a step keeps the bytes it arrived as, so the declaration
//! round-trips byte for byte and the chain hash is over what was received (L9).

use std::collections::BTreeMap;

use super::{Cond, Field, FieldType, Fields, LensOp, LensStep, Revision};
use crate::cbor::envelope::to_cbor;
use crate::cbor::major;
use crate::cbor::reader::Dec;
use crate::cbor::writer::{enc, Enc, MapBuilder};
use crate::diag::{Code, Diagnostic, Report};
use crate::hash::hash_bytes;
use crate::ids::SchemaId;
use crate::surface::payload::{dec_object, dec_value, object_bytes, value_bytes};
use crate::surface::{HObject, HValue, Spanned};
use crate::types::{Record, RelKind, SchemaDecl};

/// The structure did not match. Carries nothing: the caller reports `SMY-W430` once for the
/// key, which is the granularity A-7 gives the warning.
struct Mismatch;

type R<T> = Result<T, Mismatch>;

impl From<crate::error::CodecError> for Mismatch {
    fn from(_: crate::error::CodecError) -> Mismatch {
        Mismatch
    }
}

/// Decode one declaration (§3.8). Reports `SMY-W430` for a key 4 or key 5 that does not match
/// its structure and `SMY-W469` once per step carrying an op this build does not know.
pub fn decode(decl: &SchemaDecl, report: &mut Report) -> Revision {
    let at = || format!("{}@{}", decl.id.as_str(), decl.version);
    let fields = match &decl.payload_shape {
        None => Fields::Absent,
        Some(b) => match shape(b) {
            Ok(m) => Fields::Typed(m),
            Err(Mismatch) => {
                report.push(Diagnostic::new(Code::W430).with_message(format!(
                    "{}: payload_shape does not match FC-9's structure; read as opaque",
                    at()
                )));
                Fields::Opaque
            }
        },
    };
    let (lenses, lenses_opaque) = match &decl.lenses {
        None => (Vec::new(), false),
        Some(b) => match steps(b) {
            Ok(v) => (v, false),
            Err(Mismatch) => {
                report.push(Diagnostic::new(Code::W430).with_message(format!(
                    "{}: lenses (key 5) do not match FC-9's structure; no step is usable",
                    at()
                )));
                (Vec::new(), true)
            }
        },
    };
    for s in &lenses {
        for op in &s.ops {
            if let LensOp::Unknown { tag, .. } = op {
                report.push(Diagnostic::new(Code::W469).with_message(format!(
                    "{}: step {}→{} carries op `{tag}`, which this build does not know; the \
                     step is unusable",
                    at(),
                    s.from,
                    s.to
                )));
                break;
            }
        }
    }
    Revision {
        id: decl.id.clone(),
        version: decl.version,
        decl_hash: hash_bytes(&to_cbor(&Record::SchemaDecl(decl.clone()))),
        types: decl.types.clone(),
        relations: decl.relations.clone(),
        fields,
        lenses,
        lenses_opaque,
    }
}

/// Key 5's bytes for a list of steps: each step's own bytes, in order. Byte-identical to what
/// was received for decoded steps (L9).
pub fn encode_lenses(steps: &[LensStep]) -> Vec<u8> {
    enc(|e| {
        e.head(major::ARRAY, steps.len() as u64);
        for s in steps {
            e.raw(s.bytes());
        }
    })
}

/// Key 4's bytes for a field table, `{0: fields}`, canonical.
pub fn encode_shape(fields: &BTreeMap<String, Field>) -> Vec<u8> {
    let table = text_map(fields.iter().map(|(k, f)| (k.as_str(), field_bytes(f))));
    let mut m = MapBuilder::new();
    m.put(0, |e| e.raw(&table));
    m.into_bytes()
}

// --- reading -------------------------------------------------------------------------------

fn shape(b: &[u8]) -> R<BTreeMap<String, Field>> {
    let mut d = Dec::new(b);
    let n = d.map_head()?;
    let mut prev = None;
    let mut fields = None;
    for _ in 0..n {
        let k = d.map_key(prev)?;
        prev = Some(k);
        match k {
            0 => fields = Some(field_table(&mut d)?),
            // A later revision of A-7 may add keys beside the table; the bytes are kept, so
            // skipping one loses nothing and reads nothing into it.
            _ => {
                d.skip_item()?;
            }
        }
    }
    if !d.is_done() {
        return Err(Mismatch);
    }
    fields.ok_or(Mismatch)
}

fn field_table(d: &mut Dec<'_>) -> R<BTreeMap<String, Field>> {
    let n = d.map_head()?;
    let mut out = BTreeMap::new();
    let mut prev: Option<String> = None;
    for _ in 0..n {
        let key = d.text()?.to_string();
        // Text keys sort by encoded bytes, length first (the payload's rule).
        if let Some(p) = &prev {
            if (p.len(), p.as_str()) >= (key.len(), key.as_str()) {
                return Err(Mismatch);
            }
        }
        prev = Some(key.clone());
        out.insert(key, field(d)?);
    }
    Ok(out)
}

fn field(d: &mut Dec<'_>) -> R<Field> {
    let n = d.map_head()?;
    let mut prev = None;
    let mut f = Field::new(FieldType::Text);
    let mut ty = None;
    for _ in 0..n {
        let k = d.map_key(prev)?;
        prev = Some(k);
        match k {
            0 => ty = Some(FieldType::parse(d.text()?)),
            1 => f.values = texts(d)?,
            2 => f.retired_in = Some(rev(d)?),
            3 => f.note = Some(d.text()?.to_string()),
            4 => f.required_from = Some(rev(d)?),
            5 => f.required_when = Some(cond(d)?),
            6 => f.literals = texts(d)?,
            _ => {
                d.skip_item()?;
            }
        }
    }
    f.ty = ty.ok_or(Mismatch)?;
    Ok(f)
}

fn steps(b: &[u8]) -> R<Vec<LensStep>> {
    let mut d = Dec::new(b);
    let n = d.array_head()?;
    let mut out = Vec::with_capacity(n.min(1024));
    for _ in 0..n {
        let raw = d.skip_item()?;
        out.push(step(raw)?);
    }
    if !d.is_done() {
        return Err(Mismatch);
    }
    Ok(out)
}

fn step(raw: &[u8]) -> R<LensStep> {
    let mut d = Dec::new(raw);
    let n = d.map_head()?;
    let (mut from, mut to, mut ops) = (None, None, None);
    let mut prev = None;
    for _ in 0..n {
        let k = d.map_key(prev)?;
        prev = Some(k);
        match k {
            0 => from = Some(rev(&mut d)?),
            1 => to = Some(rev(&mut d)?),
            2 => {
                let m = d.array_head()?;
                let mut v = Vec::with_capacity(m.min(1024));
                for _ in 0..m {
                    let raw = d.skip_item()?;
                    v.push(op(raw)?);
                }
                ops = Some(v);
            }
            // A step key this build does not know could change what every op in it means, so
            // the step is not read at all.
            _ => return Err(Mismatch),
        }
    }
    let (from, to, ops) = (
        from.ok_or(Mismatch)?,
        to.ok_or(Mismatch)?,
        ops.ok_or(Mismatch)?,
    );
    // Steps go up (§3.10): a step that does not is not a step.
    if to <= from {
        return Err(Mismatch);
    }
    Ok(LensStep {
        from,
        to,
        ops,
        bytes: raw.to_vec(),
    })
}

/// One op. Keys are read into a table first, because which keys an op has depends on its tag
/// and the tag is key 0.
fn op(raw: &[u8]) -> R<LensOp> {
    let mut d = Dec::new(raw);
    let n = d.map_head()?;
    let mut prev = None;
    let mut keys: BTreeMap<u16, &[u8]> = BTreeMap::new();
    for _ in 0..n {
        let k = d.map_key(prev)?;
        prev = Some(k);
        keys.insert(k, d.skip_item()?);
    }
    let tag = match keys.get(&0) {
        Some(b) => Dec::new(b).text()?.to_string(),
        None => return Err(Mismatch),
    };
    let allowed: &[u16] = match tag.as_str() {
        "rename-type" | "rename-relation" | "rename-key" | "map-value" => &[0, 1, 2],
        "split-value" => &[0, 1, 2, 3, 4],
        "require" => &[0, 1, 2],
        "default" => &[0, 1, 2, 3],
        "retire" => &[0, 1],
        _ => {
            return Ok(LensOp::Unknown {
                tag,
                bytes: raw.to_vec(),
            })
        }
    };
    // A known tag with a key this build does not know is read as unknown: the key may change
    // what the op does, and D-5 says an op not understood is not applied.
    if keys.keys().any(|k| !allowed.contains(k)) {
        return Ok(LensOp::Unknown {
            tag,
            bytes: raw.to_vec(),
        });
    }
    let text_at = |k: u16| -> R<String> {
        let b = keys.get(&k).ok_or(Mismatch)?;
        let mut d = Dec::new(b);
        Ok(d.text()?.to_string())
    };
    let opt_cond = |k: u16| -> R<Option<Cond>> {
        match keys.get(&k) {
            None => Ok(None),
            Some(b) => Ok(Some(cond(&mut Dec::new(b))?)),
        }
    };
    Ok(match tag.as_str() {
        "rename-type" => LensOp::RenameType {
            from: SchemaId::parse(&text_at(1)?).map_err(|_| Mismatch)?,
            to: SchemaId::parse(&text_at(2)?).map_err(|_| Mismatch)?,
        },
        "rename-relation" => LensOp::RenameRelation {
            from: RelKind::parse(&text_at(1)?).map_err(|_| Mismatch)?,
            to: RelKind::parse(&text_at(2)?).map_err(|_| Mismatch)?,
        },
        "rename-key" => LensOp::RenameKey {
            from: text_at(1)?,
            to: text_at(2)?,
        },
        "map-value" => {
            let o = dec_object(&mut Dec::new(keys.get(&2).ok_or(Mismatch)?))?;
            let mut map = BTreeMap::new();
            for (k, v) in o.iter() {
                map.insert(
                    k.value.clone(),
                    v.value.as_str().ok_or(Mismatch)?.to_string(),
                );
            }
            LensOp::MapValue {
                key: text_at(1)?,
                map,
            }
        }
        "split-value" => {
            let o = dec_object(&mut Dec::new(keys.get(&4).ok_or(Mismatch)?))?;
            LensOp::SplitValue {
                key: text_at(1)?,
                from: text_at(2)?,
                to: text_at(3)?,
                add: o
                    .iter()
                    .map(|(k, v)| (k.value.clone(), v.value.clone()))
                    .collect(),
            }
        }
        "require" => LensOp::Require {
            key: text_at(1)?,
            when: opt_cond(2)?,
        },
        "default" => LensOp::Default {
            key: text_at(1)?,
            value: dec_value(&mut Dec::new(keys.get(&2).ok_or(Mismatch)?))?,
            when: opt_cond(3)?,
        },
        "retire" => LensOp::Retire { key: text_at(1)? },
        _ => unreachable!("tags without a table returned above"),
    })
}

fn rev(d: &mut Dec<'_>) -> R<u32> {
    u32::try_from(d.uint()?).map_err(|_| Mismatch)
}

fn texts(d: &mut Dec<'_>) -> R<Vec<String>> {
    Ok(d.array(|d| d.text().map(str::to_string))?)
}

fn cond(d: &mut Dec<'_>) -> R<Cond> {
    let o = dec_object(d)?;
    let mut it = o.iter();
    match (it.next(), it.next()) {
        (Some((k, v)), None) => Ok(Cond::new(k.value.clone(), v.value.clone())),
        _ => Err(Mismatch),
    }
}

// --- writing -------------------------------------------------------------------------------

/// A step's canonical bytes (`LensStep::new`).
pub(super) fn step_bytes(from: u32, to: u32, ops: &[LensOp]) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(0, |e| e.uint(from as u64));
    m.put(1, |e| e.uint(to as u64));
    // Key 2 is required, so an empty op list is written as `[]`: `put_array` omits an empty
    // sequence, as every optional array in the envelope wants, and a step without its key 2
    // does not decode.
    let ops: Vec<Vec<u8>> = ops.iter().map(op_bytes).collect();
    m.put(2, |e| e.array(ops));
    m.into_bytes()
}

fn op_bytes(op: &LensOp) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(0, |e| e.text(op.tag()));
    match op {
        LensOp::RenameType { from, to } => {
            m.put(1, |e| e.text(from.as_str()));
            m.put(2, |e| e.text(to.as_str()));
        }
        LensOp::RenameRelation { from, to } => {
            m.put(1, |e| e.text(from.as_str()));
            m.put(2, |e| e.text(to.as_str()));
        }
        LensOp::RenameKey { from, to } => {
            m.put(1, |e| e.text(from));
            m.put(2, |e| e.text(to));
        }
        LensOp::MapValue { key, map } => {
            m.put(1, |e| e.text(key));
            let o = object(
                map.iter()
                    .map(|(k, v)| (k.as_str(), HValue::Str(v.clone()))),
            );
            m.put(2, |e| e.raw(&object_bytes(&o)));
        }
        LensOp::SplitValue { key, from, to, add } => {
            m.put(1, |e| e.text(key));
            m.put(2, |e| e.text(from));
            m.put(3, |e| e.text(to));
            let o = object(add.iter().map(|(k, v)| (k.as_str(), v.clone())));
            m.put(4, |e| e.raw(&object_bytes(&o)));
        }
        LensOp::Require { key, when } => {
            m.put(1, |e| e.text(key));
            if let Some(c) = when {
                m.put(2, |e| e.raw(&cond_bytes(c)));
            }
        }
        LensOp::Default { key, value, when } => {
            m.put(1, |e| e.text(key));
            m.put(2, |e| e.raw(&value_bytes(value)));
            if let Some(c) = when {
                m.put(3, |e| e.raw(&cond_bytes(c)));
            }
        }
        LensOp::Retire { key } => {
            m.put(1, |e| e.text(key));
        }
        LensOp::Unknown { bytes, .. } => return bytes.clone(),
    }
    m.into_bytes()
}

fn field_bytes(f: &Field) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(0, |e| e.text(&f.ty.as_string()));
    if !f.values.is_empty() {
        m.put_array(1, f.values.iter().map(|v| enc(|e| e.text(v))).collect());
    }
    if let Some(r) = f.retired_in {
        m.put(2, |e| e.uint(r as u64));
    }
    if let Some(n) = &f.note {
        m.put(3, |e| e.text(n));
    }
    if let Some(r) = f.required_from {
        m.put(4, |e| e.uint(r as u64));
    }
    if let Some(c) = &f.required_when {
        m.put(5, |e| e.raw(&cond_bytes(c)));
    }
    if !f.literals.is_empty() {
        m.put_array(6, f.literals.iter().map(|v| enc(|e| e.text(v))).collect());
    }
    m.into_bytes()
}

fn cond_bytes(c: &Cond) -> Vec<u8> {
    object_bytes(&object([(c.key.as_str(), c.value.clone())]))
}

fn object<'a>(entries: impl IntoIterator<Item = (&'a str, HValue)>) -> HObject {
    let span = crate::diag::Span::new(0, 0);
    let mut o = HObject::new();
    for (k, v) in entries {
        o.insert(Spanned::new(k.to_string(), span), Spanned::new(v, span));
    }
    o
}

/// A text-keyed map of already-encoded values, keys in encoded-byte order.
fn text_map<'a>(entries: impl Iterator<Item = (&'a str, Vec<u8>)>) -> Vec<u8> {
    let mut v: Vec<(Vec<u8>, Vec<u8>)> = entries.map(|(k, b)| (enc(|e| e.text(k)), b)).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    let mut e = Enc::new();
    e.head(major::MAP, v.len() as u64);
    for (k, b) in v {
        e.raw(&k);
        e.raw(&b);
    }
    e.into_bytes()
}
