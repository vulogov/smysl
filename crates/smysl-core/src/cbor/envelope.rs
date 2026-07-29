//! Record encoding and decoding: `[ type_code, payload ]` (§7.2).
//!
//! Two properties are load-bearing here and both are tested rather than assumed:
//!
//! - **Round-trip stability.** `encode(decode(bytes)) == bytes` for every conforming
//!   input, including records carrying keys this build does not know. Without that,
//!   `check --verify-hashes` would report a store written by a later minor version as
//!   entirely corrupt.
//! - **Unknown type codes survive.** They decode to [`Record::Unknown`] with their
//!   payload bytes intact and re-encode identically (`SMY-W014`).

use std::collections::BTreeSet;

use crate::cbor::keys;
use crate::cbor::reader::Dec;
use crate::cbor::writer::{enc, Enc, MapBuilder};
use crate::error::CodecError;
use crate::ids::{AgentId, ContentionId, Label, LangTag, SchemaId, ThreadId, ViewId};
use crate::types::aux::{
    Contention, ContentionStatus, Detected, DetectionKind, DropReason, LabelBinding, Optimality,
    PackInfo, PackMode, SchemaDecl,
};
use crate::types::epistemics::{Date, Lod, SourceKind, SourceRef, Status};
use crate::types::provenance::{Attestation, Hlc, Op, Rung};
use crate::types::record::{code, Record};
use crate::types::relation::{RelKind, Relation};
use crate::types::thread::{Role, Step, Thread, ThreadSchema};
use crate::types::unit::{Extra, UnitCore, UnitCoreBuilder};
use crate::types::view::{Admission, GranularityProfile, View};

type Res<T> = Result<T, CodecError>;

fn bad(at: usize) -> CodecError {
    CodecError::MalformedEnvelope { at }
}

// ===========================================================================
// Encoding
// ===========================================================================

/// The canonical bytes of a `UnitCore` - the payload map alone, which is the hash input
/// of rule P1. Not the envelope: the type code is framing, not content.
pub fn unit_core_bytes(u: &UnitCore) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(keys::unit::SCHEMA, |e| e.text(u.schema.as_str()));
    m.put(keys::unit::GIST, |e| e.text(&u.gist));
    m.put_opt(keys::unit::BODY, u.body.as_ref(), |e, v| e.text(v));
    m.put_opt(keys::unit::DETAIL, u.detail.as_ref(), |e, v| e.text(v));
    m.put_uid_set(keys::unit::DEPS, u.deps.iter());
    m.put_uid_set(keys::unit::GROUNDS, u.grounds.iter());
    m.put(keys::unit::STATUS, |e| e.uint(u.status.as_u8() as u64));
    m.put_opt(keys::unit::SOURCE, u.source.as_ref(), enc_source);
    m.put_opt(keys::unit::PAYLOAD, u.payload.as_ref(), |e, v| e.bytes(v));
    m.put_extra(&u.extra);
    m.into_bytes()
}

fn enc_source(e: &mut Enc, s: &SourceRef) {
    let mut m = MapBuilder::new();
    m.put(keys::source::KIND, |e| e.uint(s.kind.as_u8() as u64));
    m.put(keys::source::REFERENCE, |e| e.text(&s.reference));
    m.put_opt(keys::source::CAPTURED, s.captured.as_ref(), |e, d| {
        e.text(&d.to_string())
    });
    m.finish(e);
}

fn enc_hlc(e: &mut Enc, h: &Hlc) {
    e.array_head(3);
    e.uint(h.wall_ms);
    e.uint(h.counter as u64);
    e.text(h.agent.as_str());
}

fn enc_rel_kind(e: &mut Enc, k: &RelKind) {
    match k.code() {
        Some(c) => e.uint(c as u64),
        None => e.text(k.as_str()),
    }
}

fn attestation_bytes(a: &Attestation) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(keys::attestation::UID, |e| e.uid(&a.uid));
    m.put(keys::attestation::AGENT, |e| e.text(a.agent.as_str()));
    m.put(keys::attestation::HOP, |e| e.uint(a.hop as u64));
    m.put_uid_set(keys::attestation::PARENTS, a.parents.iter());
    m.put(keys::attestation::TS, |e| enc_hlc(e, &a.ts));
    m.put(keys::attestation::OP, |e| e.uint(a.op.as_u8() as u64));
    m.put_opt(keys::attestation::RECIPE, a.recipe.as_ref(), |e, r| {
        e.bytes(r)
    });
    m.put_opt(keys::attestation::SIG, a.sig.as_ref(), |e, s| e.bytes(s));
    m.put(keys::attestation::RUNG, |e| e.uint(a.rung.as_u8() as u64));
    m.put_opt(keys::attestation::FAMILY, a.family.as_ref(), |e, r| {
        e.bytes(r)
    });
    m.put_extra(&a.extra);
    m.into_bytes()
}

fn relation_bytes(r: &Relation) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(keys::relation::KIND, |e| enc_rel_kind(e, &r.kind));
    m.put(keys::relation::FROM, |e| e.uid(&r.from));
    m.put(keys::relation::TO, |e| e.uid(&r.to));
    m.put_opt(keys::relation::WEIGHT, r.weight.as_ref(), |e, w| e.f32q(*w));
    m.put_opt(keys::relation::NOTE, r.note.as_ref(), |e, n| e.uid(n));
    m.put_extra(&r.extra);
    m.into_bytes()
}

fn thread_bytes(t: &Thread) -> Vec<u8> {
    let steps: Vec<Vec<u8>> = t
        .steps
        .iter()
        .map(|s| {
            enc(|e| {
                e.array_head(if s.note.is_some() { 3 } else { 2 });
                e.uint(s.role.as_u8() as u64);
                e.uid(&s.unit);
                if let Some(n) = &s.note {
                    e.text(n);
                }
            })
        })
        .collect();

    let mut m = MapBuilder::new();
    m.put(keys::thread::ID, |e| e.text(t.id.as_str()));
    m.put(keys::thread::SCHEMA, |e| e.uint(t.schema.as_u8() as u64));
    m.put(keys::thread::OWNER, |e| e.text(t.owner.as_str()));
    m.put(keys::thread::GIST, |e| e.text(&t.gist));
    m.put_array(keys::thread::STEPS, steps);
    m.put(keys::thread::TS, |e| enc_hlc(e, &t.ts));
    m.put_extra(&t.extra);
    m.into_bytes()
}

fn view_bytes(v: &View) -> Vec<u8> {
    let threads: Vec<Vec<u8>> = v
        .threads
        .iter()
        .map(|t| enc(|e| e.text(t.as_str())))
        .collect();
    let requires: Vec<Vec<u8>> = v
        .requires
        .iter()
        .map(|s| enc(|e| e.text(s.as_str())))
        .collect();

    let mut m = MapBuilder::new();
    m.put(keys::view::ID, |e| e.text(v.id.as_str()));
    m.put_uid_set(keys::view::ROOTS, v.roots.iter());
    m.put_sorted_set(keys::view::THREADS, threads);
    m.put_sorted_set(keys::view::REQUIRES, requires);
    m.put(keys::view::GRANULARITY, |e| {
        enc_granularity(e, &v.granularity)
    });
    m.put(keys::view::INTENT, |e| e.text(&v.intent));
    m.put(keys::view::LANG, |e| e.text(v.lang.as_str()));
    m.put_extra(&v.extra);
    m.into_bytes()
}

fn enc_granularity(e: &mut Enc, g: &GranularityProfile) {
    let mut m = MapBuilder::new();
    m.put(keys::granularity::PROFILE, |e| e.text(&g.profile));
    m.put(keys::granularity::L0_MAX, |e| e.uint(g.l0_max as u64));
    m.put(keys::granularity::L1_MIN, |e| e.uint(g.l1_min as u64));
    m.put(keys::granularity::L1_MAX, |e| e.uint(g.l1_max as u64));
    m.put(keys::granularity::ADMISSION, |e| {
        e.uint(g.admission.as_u8() as u64)
    });
    m.finish(e);
}

fn contention_bytes(c: &Contention) -> Vec<u8> {
    let positions: Vec<Vec<u8>> = c.positions.iter().map(|u| enc(|e| e.uid(u))).collect();
    let mut m = MapBuilder::new();
    m.put(keys::contention::ID, |e| e.text(c.id.as_str()));
    m.put(keys::contention::OVER, |e| e.uid(&c.over));
    m.put_array(keys::contention::POSITIONS, positions);
    m.put(keys::contention::DETECTED, |e| {
        e.array_head(2);
        e.uint(c.detected.kind.as_u8() as u64);
        enc_hlc(e, &c.detected.ts);
    });
    m.put(keys::contention::STATUS, |e| {
        e.uint(c.status.as_u8() as u64)
    });
    m.put_extra(&c.extra);
    m.into_bytes()
}

fn packinfo_bytes(p: &PackInfo) -> Vec<u8> {
    let dropped: Vec<Vec<u8>> = p
        .dropped
        .iter()
        .map(|(u, r)| {
            enc(|e| {
                e.array_head(2);
                e.uid(u);
                e.uint(r.as_u8() as u64);
            })
        })
        .collect();
    let degraded: Vec<Vec<u8>> = p
        .degraded
        .iter()
        .map(|(u, l)| {
            enc(|e| {
                e.array_head(2);
                e.uid(u);
                e.uint(l.as_u8() as u64);
            })
        })
        .collect();

    let mut m = MapBuilder::new();
    m.put(keys::packinfo::BUDGET, |e| e.uint(p.budget));
    m.put(keys::packinfo::USED, |e| e.uint(p.used));
    m.put_opt(keys::packinfo::THREAD, p.thread.as_ref(), |e, t| {
        e.text(t.as_str())
    });
    m.put_array(keys::packinfo::DROPPED, dropped);
    m.put_array(keys::packinfo::DEGRADED, degraded);
    m.put(keys::packinfo::OPTIMALITY, |e| {
        e.array_head(2);
        e.uint(p.optimality.mode.as_u8() as u64);
        e.f32q(p.optimality.gap);
    });
    m.put(keys::packinfo::ESTIMATOR, |e| e.text(&p.estimator));
    m.put_extra(&p.extra);
    m.into_bytes()
}

fn schema_decl_bytes(d: &SchemaDecl) -> Vec<u8> {
    let types: Vec<Vec<u8>> = d
        .types
        .iter()
        .map(|t| enc(|e| e.text(t.as_str())))
        .collect();
    let relations: Vec<Vec<u8>> = d
        .relations
        .iter()
        .map(|r| enc(|e| enc_rel_kind(e, r)))
        .collect();

    let mut m = MapBuilder::new();
    m.put(keys::schema_decl::ID, |e| e.text(d.id.as_str()));
    m.put(keys::schema_decl::VERSION, |e| e.uint(d.version as u64));
    m.put_array(keys::schema_decl::TYPES, types);
    m.put_array(keys::schema_decl::RELATIONS, relations);
    m.put_opt(
        keys::schema_decl::PAYLOAD_SHAPE,
        d.payload_shape.as_ref(),
        |e, p| e.bytes(p),
    );
    m.put_extra(&d.extra);
    m.into_bytes()
}

/// Two keys, neither optional. `extra` carries anything a later version adds (rule X).
fn label_binding_bytes(b: &LabelBinding) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(keys::label_binding::LABEL, |e| e.text(b.label.as_str()));
    m.put(keys::label_binding::UID, |e| e.uid(&b.uid));
    m.put_extra(&b.extra);
    m.into_bytes()
}

/// Decode a label binding. Both keys are required: a binding missing either half binds
/// nothing, and accepting one would put a half-record in the store.
fn dec_label_binding(d: &mut Dec<'_>) -> Res<LabelBinding> {
    let at = d.position();
    let mut label = None;
    let mut uid = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::label_binding::LABEL => {
            label = Some(Label::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::label_binding::UID => {
            uid = Some(d.uid()?);
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(LabelBinding {
        label: label.ok_or_else(|| bad(at))?,
        uid: uid.ok_or_else(|| bad(at))?,
        extra,
    })
}

/// Encode one record as a complete envelope.
pub fn to_cbor(r: &Record) -> Vec<u8> {
    let payload = match r {
        Record::Unit(u) => unit_core_bytes(u),
        Record::Attestation(a) => attestation_bytes(a),
        Record::Relation(rel) => relation_bytes(rel),
        Record::Thread(t) => thread_bytes(t),
        Record::View(v) => view_bytes(v),
        Record::Contention(c) => contention_bytes(c),
        Record::PackInfo(p) => packinfo_bytes(p),
        Record::SchemaDecl(d) => schema_decl_bytes(d),
        Record::LabelBinding(b) => label_binding_bytes(b),
        Record::Unknown { payload, .. } => payload.clone(),
    };
    let mut e = Enc::with_capacity(payload.len() + 4);
    e.array_head(2);
    e.uint(r.type_code());
    e.raw(&payload);
    e.into_bytes()
}

/// Encode a whole store as a bare CBOR sequence (RFC 8742).
pub fn to_cbor_seq(records: &[Record]) -> Vec<u8> {
    let mut out = Vec::new();
    for r in records {
        out.extend_from_slice(&to_cbor(r));
    }
    out
}

// ===========================================================================
// Decoding
// ===========================================================================

/// Walk a record payload map, handing each known key to `f` and stashing unknown keys
/// verbatim so they survive a round trip (rule X at the record level).
fn read_map<F>(d: &mut Dec<'_>, extra: &mut Extra, mut f: F) -> Res<()>
where
    F: FnMut(&mut Dec<'_>, u16) -> Res<bool>,
{
    let n = d.map_head()?;
    let mut prev = None;
    for _ in 0..n {
        let k = d.map_key(prev)?;
        prev = Some(k);
        d.reject_null()?;
        if !f(d, k)? {
            // Unknown key: preserve the raw bytes rather than dropping them. `f` returns
            // false without consuming, so the decoder is still positioned at the value.
            let raw = d.skip_item()?.to_vec();
            extra.insert(k, raw);
        }
    }
    Ok(())
}

fn dec_source(d: &mut Dec<'_>) -> Res<SourceRef> {
    let at = d.position();
    let mut kind = None;
    let mut reference = None;
    let mut captured = None;
    let mut extra = Extra::new();
    read_map(d, &mut extra, |d, k| match k {
        keys::source::KIND => {
            kind = SourceKind::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::source::REFERENCE => {
            reference = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::source::CAPTURED => {
            captured = Some(Date::parse(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        _ => Ok(false),
    })?;
    Ok(SourceRef {
        kind: kind.ok_or_else(|| bad(at))?,
        reference: reference.ok_or_else(|| bad(at))?,
        captured,
    })
}

fn dec_hlc(d: &mut Dec<'_>) -> Res<Hlc> {
    let at = d.position();
    if d.array_head()? != 3 {
        return Err(bad(at));
    }
    let wall_ms = d.uint()?;
    let counter = u32::try_from(d.uint()?).map_err(|_| bad(at))?;
    let agent = AgentId::new(d.text()?).map_err(|_| bad(at))?;
    Ok(Hlc {
        wall_ms,
        counter,
        agent,
    })
}

fn dec_rel_kind(d: &mut Dec<'_>) -> Res<RelKind> {
    let at = d.position();
    match d.peek_major()? {
        0 => {
            let c = u8::try_from(d.uint()?).map_err(|_| bad(at))?;
            RelKind::from_code(c).ok_or_else(|| bad(at))
        }
        3 => RelKind::parse(d.text()?).map_err(|_| bad(at)),
        _ => Err(bad(at)),
    }
}

fn dec_bytes32(d: &mut Dec<'_>) -> Res<[u8; 32]> {
    let at = d.position();
    let b = d.bytes()?;
    <[u8; 32]>::try_from(b).map_err(|_| bad(at))
}

fn dec_unit(d: &mut Dec<'_>) -> Res<UnitCore> {
    let at = d.position();
    let mut schema = None;
    let mut gist = None;
    let mut body = None;
    let mut detail = None;
    let mut deps = BTreeSet::new();
    let mut grounds = BTreeSet::new();
    let mut status = None;
    let mut source = None;
    let mut payload = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::unit::SCHEMA => {
            schema = Some(SchemaId::parse(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::unit::GIST => {
            gist = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::unit::BODY => {
            body = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::unit::DETAIL => {
            detail = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::unit::DEPS => {
            deps = d.uid_set()?;
            Ok(true)
        }
        keys::unit::GROUNDS => {
            grounds = d.uid_set()?;
            Ok(true)
        }
        keys::unit::STATUS => {
            status = Status::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::unit::SOURCE => {
            source = Some(dec_source(d)?);
            Ok(true)
        }
        keys::unit::PAYLOAD => {
            payload = Some(d.bytes()?.to_vec());
            Ok(true)
        }
        _ => Ok(false),
    })?;

    let mut b = UnitCoreBuilder::new(
        schema.ok_or_else(|| bad(at))?,
        gist.ok_or_else(|| bad(at))?,
        status.ok_or_else(|| bad(at))?,
    );
    b.body = body;
    b.detail = detail;
    b.deps = deps;
    b.grounds = grounds;
    b.source = source;
    b.payload = payload;
    b.extra = extra;
    UnitCore::new(b).map_err(|_| bad(at))
}

fn dec_attestation(d: &mut Dec<'_>) -> Res<Attestation> {
    let at = d.position();
    let mut uid = None;
    let mut agent = None;
    let mut hop = 0u32;
    let mut parents = BTreeSet::new();
    let mut ts = None;
    let mut op = None;
    let mut rung = None;
    let mut recipe = None;
    let mut family = None;
    let mut sig = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::attestation::UID => {
            uid = Some(d.uid()?);
            Ok(true)
        }
        keys::attestation::AGENT => {
            agent = Some(AgentId::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::attestation::HOP => {
            hop = u32::try_from(d.uint()?).map_err(|_| bad(at))?;
            Ok(true)
        }
        keys::attestation::PARENTS => {
            parents = d.uid_set()?;
            Ok(true)
        }
        keys::attestation::TS => {
            ts = Some(dec_hlc(d)?);
            Ok(true)
        }
        keys::attestation::OP => {
            op = Op::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::attestation::RUNG => {
            rung = Rung::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::attestation::RECIPE => {
            recipe = Some(dec_bytes32(d)?);
            Ok(true)
        }
        keys::attestation::FAMILY => {
            family = Some(dec_bytes32(d)?);
            Ok(true)
        }
        keys::attestation::SIG => {
            sig = Some(d.bytes()?.to_vec());
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(Attestation {
        uid: uid.ok_or_else(|| bad(at))?,
        agent: agent.ok_or_else(|| bad(at))?,
        hop,
        parents,
        ts: ts.ok_or_else(|| bad(at))?,
        op: op.ok_or_else(|| bad(at))?,
        rung: rung.ok_or_else(|| bad(at))?,
        recipe,
        family,
        sig,
        extra,
    })
}

fn dec_relation(d: &mut Dec<'_>) -> Res<Relation> {
    let at = d.position();
    let mut kind = None;
    let mut from = None;
    let mut to = None;
    let mut weight = None;
    let mut note = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::relation::KIND => {
            kind = Some(dec_rel_kind(d)?);
            Ok(true)
        }
        keys::relation::FROM => {
            from = Some(d.uid()?);
            Ok(true)
        }
        keys::relation::TO => {
            to = Some(d.uid()?);
            Ok(true)
        }
        keys::relation::WEIGHT => {
            weight = Some(d.f32q()?);
            Ok(true)
        }
        keys::relation::NOTE => {
            note = Some(d.uid()?);
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(Relation {
        kind: kind.ok_or_else(|| bad(at))?,
        from: from.ok_or_else(|| bad(at))?,
        to: to.ok_or_else(|| bad(at))?,
        weight,
        note,
        attestations: BTreeSet::new(),
        extra,
    })
}

fn dec_thread(d: &mut Dec<'_>) -> Res<Thread> {
    let at = d.position();
    let mut id = None;
    let mut schema = None;
    let mut owner = None;
    let mut gist = None;
    let mut steps = Vec::new();
    let mut ts = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::thread::ID => {
            id = Some(ThreadId::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::thread::SCHEMA => {
            schema = ThreadSchema::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::thread::OWNER => {
            owner = Some(AgentId::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::thread::GIST => {
            gist = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::thread::STEPS => {
            steps = d.array(|d| {
                let at = d.position();
                let n = d.array_head()?;
                if !(2..=3).contains(&n) {
                    return Err(bad(at));
                }
                let role = Role::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(at))?)
                    .ok_or_else(|| bad(at))?;
                let unit = d.uid()?;
                let note = if n == 3 {
                    Some(d.text()?.to_string())
                } else {
                    None
                };
                Ok(Step { role, unit, note })
            })?;
            Ok(true)
        }
        keys::thread::TS => {
            ts = Some(dec_hlc(d)?);
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(Thread {
        id: id.ok_or_else(|| bad(at))?,
        schema: schema.ok_or_else(|| bad(at))?,
        owner: owner.ok_or_else(|| bad(at))?,
        gist: gist.ok_or_else(|| bad(at))?,
        steps,
        ts: ts.ok_or_else(|| bad(at))?,
        extra,
    })
}

fn dec_granularity(d: &mut Dec<'_>) -> Res<GranularityProfile> {
    let at = d.position();
    let mut g = GranularityProfile::default();
    let mut extra = Extra::new();
    read_map(d, &mut extra, |d, k| match k {
        keys::granularity::PROFILE => {
            g.profile = d.text()?.to_string();
            Ok(true)
        }
        keys::granularity::L0_MAX => {
            g.l0_max = u32::try_from(d.uint()?).map_err(|_| bad(at))?;
            Ok(true)
        }
        keys::granularity::L1_MIN => {
            g.l1_min = u32::try_from(d.uint()?).map_err(|_| bad(at))?;
            Ok(true)
        }
        keys::granularity::L1_MAX => {
            g.l1_max = u32::try_from(d.uint()?).map_err(|_| bad(at))?;
            Ok(true)
        }
        keys::granularity::ADMISSION => {
            g.admission = Admission::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(at))?)
                .ok_or_else(|| bad(at))?;
            Ok(true)
        }
        _ => Ok(false),
    })?;
    Ok(g)
}

fn dec_view(d: &mut Dec<'_>) -> Res<View> {
    let at = d.position();
    let mut id = None;
    let mut roots = BTreeSet::new();
    let mut threads = BTreeSet::new();
    let mut requires = BTreeSet::new();
    let mut granularity = GranularityProfile::default();
    let mut intent = String::new();
    let mut lang = LangTag::default();
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::view::ID => {
            id = Some(ViewId::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::view::ROOTS => {
            roots = d.uid_set()?;
            Ok(true)
        }
        keys::view::THREADS => {
            threads = d
                .sorted_array(|d| ThreadId::new(d.text()?).map_err(|_| bad(at)))?
                .into_iter()
                .collect();
            Ok(true)
        }
        keys::view::REQUIRES => {
            requires = d
                .sorted_array(|d| SchemaId::parse(d.text()?).map_err(|_| bad(at)))?
                .into_iter()
                .collect();
            Ok(true)
        }
        keys::view::GRANULARITY => {
            granularity = dec_granularity(d)?;
            Ok(true)
        }
        keys::view::INTENT => {
            intent = d.text()?.to_string();
            Ok(true)
        }
        keys::view::LANG => {
            lang = LangTag::new(d.text()?).map_err(|_| bad(at))?;
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(View {
        id: id.ok_or_else(|| bad(at))?,
        roots,
        threads,
        requires,
        granularity,
        intent,
        lang,
        extra,
    })
}

fn dec_contention(d: &mut Dec<'_>) -> Res<Contention> {
    let at = d.position();
    let mut id = None;
    let mut over = None;
    let mut positions = Vec::new();
    let mut detected = None;
    let mut status = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::contention::ID => {
            id = Some(ContentionId::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::contention::OVER => {
            over = Some(d.uid()?);
            Ok(true)
        }
        keys::contention::POSITIONS => {
            positions = d.array(|d| d.uid())?;
            Ok(true)
        }
        keys::contention::DETECTED => {
            let a = d.position();
            if d.array_head()? != 2 {
                return Err(bad(a));
            }
            let kind = DetectionKind::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(a))?)
                .ok_or_else(|| bad(a))?;
            detected = Some(Detected {
                kind,
                ts: dec_hlc(d)?,
            });
            Ok(true)
        }
        keys::contention::STATUS => {
            status = ContentionStatus::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(Contention {
        id: id.ok_or_else(|| bad(at))?,
        over: over.ok_or_else(|| bad(at))?,
        positions,
        detected: detected.ok_or_else(|| bad(at))?,
        status: status.ok_or_else(|| bad(at))?,
        extra,
    })
}

fn dec_packinfo(d: &mut Dec<'_>) -> Res<PackInfo> {
    let at = d.position();
    let mut budget = 0;
    let mut used = 0;
    let mut thread = None;
    let mut dropped = Vec::new();
    let mut degraded = Vec::new();
    let mut optimality = Optimality {
        mode: PackMode::Greedy,
        gap: 0.0,
    };
    let mut estimator = String::new();
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::packinfo::BUDGET => {
            budget = d.uint()?;
            Ok(true)
        }
        keys::packinfo::USED => {
            used = d.uint()?;
            Ok(true)
        }
        keys::packinfo::THREAD => {
            thread = Some(ThreadId::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::packinfo::DROPPED => {
            dropped = d.array(|d| {
                let a = d.position();
                if d.array_head()? != 2 {
                    return Err(bad(a));
                }
                let u = d.uid()?;
                let r = DropReason::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(a))?)
                    .ok_or_else(|| bad(a))?;
                Ok((u, r))
            })?;
            Ok(true)
        }
        keys::packinfo::DEGRADED => {
            degraded = d.array(|d| {
                let a = d.position();
                if d.array_head()? != 2 {
                    return Err(bad(a));
                }
                let u = d.uid()?;
                let l = Lod::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(a))?)
                    .ok_or_else(|| bad(a))?;
                Ok((u, l))
            })?;
            Ok(true)
        }
        keys::packinfo::OPTIMALITY => {
            let a = d.position();
            if d.array_head()? != 2 {
                return Err(bad(a));
            }
            let mode = PackMode::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(a))?)
                .ok_or_else(|| bad(a))?;
            optimality = Optimality {
                mode,
                gap: d.f32q()?,
            };
            Ok(true)
        }
        keys::packinfo::ESTIMATOR => {
            estimator = d.text()?.to_string();
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(PackInfo {
        budget,
        used,
        thread,
        dropped,
        degraded,
        optimality,
        estimator,
        extra,
    })
}

fn dec_schema_decl(d: &mut Dec<'_>) -> Res<SchemaDecl> {
    let at = d.position();
    let mut id = None;
    let mut version = 0u32;
    let mut types = Vec::new();
    let mut relations = Vec::new();
    let mut payload_shape = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::schema_decl::ID => {
            id = Some(SchemaId::parse(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::schema_decl::VERSION => {
            version = u32::try_from(d.uint()?).map_err(|_| bad(at))?;
            Ok(true)
        }
        keys::schema_decl::TYPES => {
            types = d.array(|d| SchemaId::parse(d.text()?).map_err(|_| bad(at)))?;
            Ok(true)
        }
        keys::schema_decl::RELATIONS => {
            relations = d.array(dec_rel_kind)?;
            Ok(true)
        }
        keys::schema_decl::PAYLOAD_SHAPE => {
            payload_shape = Some(d.bytes()?.to_vec());
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(SchemaDecl {
        id: id.ok_or_else(|| bad(at))?,
        version,
        types,
        relations,
        payload_shape,
        extra,
    })
}

/// Decode one record envelope, returning it and the number of bytes consumed.
pub fn from_cbor(bytes: &[u8]) -> Res<(Record, usize)> {
    let mut d = Dec::new(bytes);
    let at = d.position();
    if d.array_head()? != 2 {
        return Err(bad(at));
    }
    let code = d.uint()?;
    let record = match code {
        code::UNIT_CORE => Record::Unit(dec_unit(&mut d)?),
        code::ATTESTATION => Record::Attestation(dec_attestation(&mut d)?),
        code::RELATION => Record::Relation(dec_relation(&mut d)?),
        code::THREAD => Record::Thread(dec_thread(&mut d)?),
        code::VIEW => Record::View(dec_view(&mut d)?),
        code::CONTENTION => Record::Contention(dec_contention(&mut d)?),
        code::PACK_INFO => Record::PackInfo(dec_packinfo(&mut d)?),
        code::SCHEMA_DECL => Record::SchemaDecl(dec_schema_decl(&mut d)?),
        code::LABEL_BINDING => Record::LabelBinding(dec_label_binding(&mut d)?),
        other => {
            // `SMY-W014`: preserved verbatim, skipped semantically. The payload is still
            // parsed strictly, so an unknown record cannot smuggle in a non-deterministic
            // encoding.
            let payload = d.skip_item()?.to_vec();
            Record::Unknown {
                code: other,
                payload,
            }
        }
    };
    Ok((record, d.position()))
}

/// Decode a bare CBOR sequence.
///
/// Returns everything up to the last complete record, plus the byte offset where parsing
/// stopped. A truncated tail is not an error at this layer: the log is append-only and
/// may be read while a writer is mid-append (§7.3).
pub fn from_cbor_seq(bytes: &[u8]) -> Res<(Vec<Record>, usize)> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off < bytes.len() {
        match from_cbor(&bytes[off..]) {
            Ok((r, n)) => {
                out.push(r);
                off += n;
            }
            Err(CodecError::Truncated { .. }) => break,
            Err(e) => return Err(e),
        }
    }
    Ok((out, off))
}
