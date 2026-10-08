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
use crate::ids::{
    AgentId, ContentionId, Label, LangTag, Mid, Rdid, SchemaId, ThreadId, Tid, ViewId,
};
use crate::types::annex::{
    Contention, ContentionStatus, Detected, DetectionKind, DropReason, LabelBinding, Optimality,
    PackInfo, PackMode, SchemaDecl,
};
use crate::types::epistemics::{Date, Lod, SourceKind, SourceRef, Status};
use crate::types::estimate::ProfileEstimator;
use crate::types::library::{
    Calendar, Carry, Manifest, ParentKind, PartEntry, PartReading, PartText,
};
use crate::types::lifecycle::{Commit, Commitment, Resolution, ResolutionTarget, Withdrawal};
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
    m.put(keys::source::KIND, |e| e.uint(s.kind_code() as u64));
    m.put(keys::source::REFERENCE, |e| e.text(&s.reference));
    m.put_opt(keys::source::CAPTURED, s.captured.as_ref(), |e, d| {
        e.text(&d.to_string())
    });
    m.put_opt(keys::source::OBSERVED, s.observed.as_ref(), |e, ms| {
        e.uint(*ms)
    });
    // Rule X inside the sub-map, as every record body has done since 0.2. Without it a reader
    // that met a key it did not know re-encoded the unit without it and changed its uid.
    m.put_extra(&s.extra);
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
                e.uint(s.role_code() as u64);
                e.uid(&s.unit);
                if let Some(n) = &s.note {
                    e.text(n);
                }
            })
        })
        .collect();

    let mut m = MapBuilder::new();
    m.put(keys::thread::ID, |e| e.text(t.id.as_str()));
    m.put(keys::thread::SCHEMA, |e| e.uint(t.schema_code() as u64));
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
        e.uint(g.admission_code() as u64)
    });
    // Absent for the pre-F-2 default: writing it would change the bytes of every view that
    // predates F-2, which §8.1 forbids (A-9).
    if let Some(id) = g.estimator.wire_id() {
        let id = id.to_string();
        m.put(keys::granularity::ESTIMATOR, move |e| e.text(&id));
    }
    m.put_extra(&g.extra);
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
        e.uint(c.detected.kind_code() as u64);
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
    // Only when non-zero: a pack that reserved nothing must encode to the bytes it encoded to
    // before this key existed, or every fixture's uid moves for a field nobody set.
    if p.reserved != 0 {
        m.put(keys::packinfo::RESERVED, |e| e.uint(p.reserved));
    }
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

fn withdrawal_bytes(w: &Withdrawal) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(keys::withdrawal::RELATION, |e| e.uid(&w.relation));
    m.put(keys::withdrawal::AGENT, |e| e.text(w.agent.as_str()));
    m.put(keys::withdrawal::TS, |e| enc_hlc(e, &w.ts));
    m.put_opt(keys::withdrawal::REASON, w.reason.as_ref(), |e, u| e.uid(u));
    m.put_extra(&w.extra);
    m.into_bytes()
}

fn resolution_bytes(r: &Resolution) -> Vec<u8> {
    let mut m = MapBuilder::new();
    match &r.target {
        ResolutionTarget::Contention(id) => {
            m.put(keys::resolution::CONTENTION, |e| e.text(id.as_str()));
        }
        ResolutionTarget::Relation(rid) => {
            m.put(keys::resolution::RELATION, |e| e.uid(rid));
        }
    }
    m.put(keys::resolution::AGENT, |e| e.text(r.agent.as_str()));
    m.put(keys::resolution::TS, |e| enc_hlc(e, &r.ts));
    m.put_opt(keys::resolution::NOTE, r.note.as_ref(), |e, u| e.uid(u));
    m.put_extra(&r.extra);
    m.into_bytes()
}

fn commit_bytes(c: &Commit) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(keys::commit::UNIT, |e| e.uid(&c.unit));
    m.put(keys::commit::LEVEL, |e| e.uint(c.level.as_u8() as u64));
    m.put(keys::commit::AGENT, |e| e.text(c.agent.as_str()));
    m.put(keys::commit::TS, |e| enc_hlc(e, &c.ts));
    m.put_opt(keys::commit::NOTE, c.note.as_ref(), |e, u| e.uid(u));
    m.put_extra(&c.extra);
    m.into_bytes()
}

/// `true` as a CBOR simple value.
///
/// A bool appears in exactly one kernel field — a manifest's `lossy` — and it is written only
/// when true, so `false` has no encoding at all. That is what keeps one manifest to one byte
/// string: admitting `0xF4` would give a lossless manifest two spellings.
fn enc_true(e: &mut Enc) {
    e.head(crate::cbor::major::SIMPLE, 21);
}

/// Write a pre-encoded CBOR item that this layer carries without interpreting.
///
/// `empty` is the item to write when the carried bytes are empty, and it is the reason this
/// is a function. A reading's segment table is required and may be empty, and "empty" on the
/// wire is an empty array, not zero bytes; writing nothing would emit a map with a key and no
/// value and corrupt every record that followed it in the log. A caller that builds an empty
/// table with `Vec::new()` is making an ordinary statement, not a mistake, so the encoder
/// spells it rather than refusing it.
fn enc_opaque(e: &mut Enc, bytes: &[u8], empty: u8) {
    if bytes.is_empty() {
        e.raw(&[empty]);
    } else {
        e.raw(bytes);
    }
}

/// A CBOR map with text keys, sorted by encoded key bytes.
///
/// Not by the strings: constraint 4 orders a map by its encoded keys, and a text head carries
/// its length first, so `"a" < "doi" < "isbn"` by bytes while `"doi" < "isbn" < "a"`… is what
/// a `BTreeMap` iterates. Sorting the encoded pairs is the only order that agrees with what
/// `skip_item` enforces when it meets the same map under an unknown key.
fn enc_text_map<'a>(e: &mut Enc, entries: impl Iterator<Item = (&'a str, &'a str)>) {
    let mut rows: Vec<(Vec<u8>, Vec<u8>)> = entries
        .map(|(k, v)| (enc(|e| e.text(k)), enc(|e| e.text(v))))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    debug_assert!(
        rows.windows(2).all(|w| w[0].0 < w[1].0),
        "duplicate text key in encoder output"
    );
    e.head(crate::cbor::major::MAP, rows.len() as u64);
    for (k, v) in rows {
        e.raw(&k);
        e.raw(&v);
    }
}

fn enc_part_entry(e: &mut Enc, p: &PartEntry) {
    let mut m = MapBuilder::new();
    m.put(keys::part_entry::TID, |e| e.bytes(p.tid.as_bytes()));
    m.put(keys::part_entry::LENGTH, |e| e.uint(p.length));
    m.put(keys::part_entry::STRUCTURE, |e| e.bytes(&p.structure));
    m.put(keys::part_entry::RDID, |e| e.bytes(p.rdid.as_bytes()));
    m.put_opt(keys::part_entry::LANG, p.lang.as_ref(), |e, l| {
        e.text(l.as_str())
    });
    m.put_extra(&p.extra);
    m.finish(e);
}

/// The canonical bytes of a manifest body — the mid's preimage after its domain byte (A-3).
///
/// Public for the same reason [`unit_core_bytes`] is: a second implementation derives a mid
/// from these bytes, and a disagreement has to be attributable to the encoding or to the
/// hashing rather than to the pair of them together.
pub fn manifest_bytes(m: &Manifest) -> Vec<u8> {
    let mut b = MapBuilder::new();
    b.put(keys::manifest::ALIAS, |e| e.text(&m.alias));
    // Not `put_array`: `parts` is required and may be empty. An import manifest records an
    // expression whose text was never carried, and it has to encode to a key with an empty
    // array rather than to no key at all.
    b.put(keys::manifest::PARTS, |e| {
        e.array(
            m.parts
                .iter()
                .map(|p| enc(|e| enc_part_entry(e, p)))
                .collect(),
        )
    });
    b.put(keys::manifest::LANG, |e| e.text(m.lang.as_str()));
    b.put(keys::manifest::READER, |e| e.text(&m.reader));
    b.put(keys::manifest::LICENCE, |e| e.text(&m.licence));
    b.put(keys::manifest::CARRY, |e| e.uint(m.carry.as_u8() as u64));
    b.put_opt(keys::manifest::TITLE, m.title.as_ref(), |e, t| e.text(t));
    b.put_array(
        keys::manifest::CREATORS,
        m.creators.iter().map(|c| enc(|e| e.text(c))).collect(),
    );
    b.put_opt(keys::manifest::PUBLISHED, m.published.as_ref(), |e, p| {
        e.text(p)
    });
    if !m.identifiers.is_empty() {
        b.put(keys::manifest::IDENTIFIERS, |e| {
            enc_text_map(
                e,
                m.identifiers.iter().map(|(k, v)| (k.as_str(), v.as_str())),
            )
        });
    }
    b.put_opt(keys::manifest::ORIGIN, m.origin.as_ref(), enc_source);
    if let Some((mid, kind)) = &m.parent {
        b.put(keys::manifest::PARENT, |e| e.bytes(mid.as_bytes()));
        b.put(keys::manifest::PARENT_KIND, |e| e.text(kind.as_str()));
    }
    b.put_opt(keys::manifest::SUPERSEDES, m.supersedes.as_ref(), |e, s| {
        e.bytes(s.as_bytes())
    });
    b.put_opt(
        keys::manifest::VERSIFICATION,
        m.versification.as_ref(),
        |e, v| e.text(v),
    );
    if m.lossy {
        b.put(keys::manifest::LOSSY, enc_true);
    }
    b.put_opt(keys::manifest::RAW, m.raw.as_ref(), |e, r| {
        enc_opaque(e, r, 0xA0)
    });
    b.put(keys::manifest::PART_POLICY, |e| e.text(&m.part_policy));
    b.put_opt(keys::manifest::CALENDAR, m.calendar.as_ref(), |e, c| {
        e.text(c.as_str())
    });
    b.put_extra(&m.extra);
    b.into_bytes()
}

fn part_text_bytes(p: &PartText) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(keys::part_text::TID, |e| e.bytes(p.tid.as_bytes()));
    m.put(keys::part_text::TEXT, |e| e.bytes(&p.text));
    m.put_extra(&p.extra);
    m.into_bytes()
}

/// The canonical bytes of a reading body — the rdid's preimage after its domain byte (A-3).
pub fn part_reading_bytes(r: &PartReading) -> Vec<u8> {
    let mut m = MapBuilder::new();
    m.put(keys::part_reading::TID, |e| e.bytes(r.tid.as_bytes()));
    m.put(keys::part_reading::READER, |e| e.text(&r.reader));
    m.put(keys::part_reading::SEGMENTS, |e| {
        enc_opaque(e, &r.segments, 0x80)
    });
    m.put_opt(keys::part_reading::RAW, r.raw.as_ref(), |e, raw| {
        enc_opaque(e, raw, 0xA0)
    });
    m.put_extra(&r.extra);
    m.into_bytes()
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
        Record::Withdrawal(w) => withdrawal_bytes(w),
        Record::Resolution(r) => resolution_bytes(r),
        Record::Commit(c) => commit_bytes(c),
        Record::Manifest(m) => manifest_bytes(m),
        Record::PartText(p) => part_text_bytes(p),
        Record::PartReading(r) => part_reading_bytes(r),
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
    let mut observed = None;
    let mut raw_kind = None;
    let mut extra = Extra::new();
    read_map(d, &mut extra, |d, k| match k {
        keys::source::KIND => {
            // Open from 1.9: a code this build cannot name is kept, not refused. Before, an
            // unrecognised kind made the whole store unopenable.
            let c = u8::try_from(d.uint()?).map_err(|_| bad(at))?;
            raw_kind = Some(c);
            kind = Some(SourceKind::from_u8(c).unwrap_or(SourceKind::Unknown));
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
        keys::source::OBSERVED => {
            observed = Some(d.uint()?);
            Ok(true)
        }
        _ => Ok(false),
    })?;
    let kind = kind.ok_or_else(|| bad(at))?;
    let mut s = SourceRef::new(kind, reference.ok_or_else(|| bad(at))?);
    s.captured = captured;
    s.observed = observed;
    s.extra = extra;
    if kind == SourceKind::Unknown {
        // `source.kind` is inside the uid, so the code has to leave again exactly as it
        // arrived. `with_unknown_kind` refuses 255 and refuses a code we do know.
        s = s
            .with_unknown_kind(raw_kind.ok_or_else(|| bad(at))?)
            .ok_or_else(|| bad(at))?;
    }
    Ok(s)
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
            // `parse_forward`, not `parse`: a type this build does not know is a later
            // version's kernel type, and refusing it would fail the whole record. It
            // decodes to `SchemaId::UnknownKernel`, re-encodes to the same bytes, and is
            // reported as `SMY-W014` by a caller that wants to say so.
            schema = Some(SchemaId::parse_forward(d.text()?).map_err(|_| bad(at))?);
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
    let mut raw_schema = None;
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
            // Open from 1.9: an unrecognised schema is kept, not refused.
            let c = u8::try_from(d.uint()?).map_err(|_| bad(at))?;
            raw_schema = Some(c);
            schema = Some(ThreadSchema::from_u8(c).unwrap_or(ThreadSchema::Unknown));
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
                // Open from 1.9: an unrecognised role is kept, not refused.
                let rc = u8::try_from(d.uint()?).map_err(|_| bad(at))?;
                let role = Role::from_u8(rc).unwrap_or(Role::Unknown);
                let unit = d.uid()?;
                let note = if n == 3 {
                    Some(d.text()?.to_string())
                } else {
                    None
                };
                let mut s = Step::new(role, unit);
                s.note = note;
                if role == Role::Unknown {
                    s = s.with_unknown_role(rc).ok_or_else(|| bad(at))?;
                }
                Ok(s)
            })?;
            Ok(true)
        }
        keys::thread::TS => {
            ts = Some(dec_hlc(d)?);
            Ok(true)
        }
        _ => Ok(false),
    })?;

    let schema = schema.ok_or_else(|| bad(at))?;
    let mut th = Thread::new(
        id.ok_or_else(|| bad(at))?,
        schema,
        owner.ok_or_else(|| bad(at))?,
        gist.ok_or_else(|| bad(at))?,
        ts.ok_or_else(|| bad(at))?,
    );
    th.steps = steps;
    th.extra = extra;
    if schema == ThreadSchema::Unknown {
        th = th
            .with_unknown_schema(raw_schema.ok_or_else(|| bad(at))?)
            .ok_or_else(|| bad(at))?;
    }
    Ok(th)
}

fn dec_granularity(d: &mut Dec<'_>) -> Res<GranularityProfile> {
    let at = d.position();
    let mut g = GranularityProfile::default();
    // Collected into the profile, not a local: a key this build does not know has to leave
    // again in the bytes it arrived in (§8.1).
    let mut extra = Extra::new();
    let raw_admission = &mut None;
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
            // Open from 1.10 (A-8.1): a code this build cannot name is kept and treated as
            // unknown, where until 1.9 it failed the whole decode. The raw byte goes back out
            // through `with_unknown_admission`, below, once the profile is built.
            let c = u8::try_from(d.uint()?).map_err(|_| bad(at))?;
            *raw_admission = Some(c);
            g.admission = Admission::from_u8(c).unwrap_or(Admission::Unknown);
            Ok(true)
        }
        keys::granularity::ESTIMATOR => {
            // An id this build does not know is kept as it arrived, not dropped and not
            // silently replaced by the default: it has to re-encode byte for byte (§8.1), and
            // it makes the bounds unevaluable rather than evaluable under another count.
            g.estimator = ProfileEstimator::from_wire(d.text()?);
            Ok(true)
        }
        _ => Ok(false),
    })?;
    g.extra = extra;
    if g.admission == Admission::Unknown {
        // The code has to leave again exactly as it arrived, or two peers compute different
        // record-set digests for one store. `with_unknown_admission` refuses 255 and refuses
        // a code we do know, so a profile that claims `Unknown` for a named code is rejected
        // rather than re-encoded as something else.
        g = g
            .with_unknown_admission(raw_admission.ok_or_else(|| bad(at))?)
            .ok_or_else(|| bad(at))?;
    }
    Ok(g)
}

fn dec_view(d: &mut Dec<'_>) -> Res<View> {
    let at = d.position();
    let mut id = None;
    let mut roots = BTreeSet::new();
    let mut threads = BTreeSet::new();
    let mut requires = BTreeSet::new();
    // Required, not defaulted — the encoder writes all three unconditionally, so a decoder
    // that filled a default for a missing one accepted bytes that re-encoded into different
    // bytes. Same defect as `dec_packinfo`; both found by fuzzing.
    let mut granularity = None;
    let mut intent = None;
    let mut lang = None;
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
            granularity = Some(dec_granularity(d)?);
            Ok(true)
        }
        keys::view::INTENT => {
            intent = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::view::LANG => {
            lang = Some(LangTag::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(View {
        id: id.ok_or_else(|| bad(at))?,
        roots,
        threads,
        requires,
        granularity: granularity.ok_or_else(|| bad(at))?,
        intent: intent.ok_or_else(|| bad(at))?,
        lang: lang.ok_or_else(|| bad(at))?,
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
            // Open from 1.9: an unrecognised detection kind is kept, not refused.
            let c = u8::try_from(d.uint()?).map_err(|_| bad(a))?;
            let kind = DetectionKind::from_u8(c).unwrap_or(DetectionKind::Unknown);
            let mut det = Detected::new(kind, dec_hlc(d)?);
            if kind == DetectionKind::Unknown {
                det = det.with_unknown_kind(c).ok_or_else(|| bad(a))?;
            }
            detected = Some(det);
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
    // Required rather than defaulted. The encoder writes all four of these unconditionally,
    // so a decoder that supplied a default for a missing one accepted an encoding that did
    // not re-encode to itself — two distinct byte strings mapping to one record, which is
    // exactly what makes a uid stop being an identity. Found by fuzzing on `[7, {0: 0}]`,
    // which came back as a four-key map.
    let mut budget = None;
    let mut used = None;
    let mut thread = None;
    let mut dropped = Vec::new();
    let mut degraded = Vec::new();
    let mut optimality = None;
    let mut estimator = None;
    // Defaulted rather than required, unlike the four above: the encoder omits it when it is
    // zero, so a missing key and a zero mean the same thing and re-encode identically.
    let mut reserved = 0u64;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::packinfo::BUDGET => {
            budget = Some(d.uint()?);
            Ok(true)
        }
        keys::packinfo::USED => {
            used = Some(d.uint()?);
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
            optimality = Some(Optimality::new(mode, d.f32q()?));
            Ok(true)
        }
        keys::packinfo::ESTIMATOR => {
            estimator = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::packinfo::RESERVED => {
            reserved = d.uint()?;
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(PackInfo {
        budget: budget.ok_or_else(|| bad(at))?,
        used: used.ok_or_else(|| bad(at))?,
        thread,
        dropped,
        degraded,
        optimality: optimality.ok_or_else(|| bad(at))?,
        estimator: estimator.ok_or_else(|| bad(at))?,
        reserved,
        extra,
    })
}

fn dec_schema_decl(d: &mut Dec<'_>) -> Res<SchemaDecl> {
    let at = d.position();
    let mut id = None;
    // Required, like `dec_packinfo`'s and `dec_view`'s fields before it. The encoder writes
    // `version` unconditionally, so defaulting it here accepted `[8, {0: "smysl.kernel/x"}]`
    // and re-encoded it as a two-key map. Third instance of this defect; see the sweep in
    // `tests/roundtrip.rs`, which missed it because none of its probe values parsed as a
    // `SchemaId` and so never entered this decoder at all.
    let mut version = None;
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
            version = Some(u32::try_from(d.uint()?).map_err(|_| bad(at))?);
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
        version: version.ok_or_else(|| bad(at))?,
        types,
        relations,
        payload_shape,
        extra,
    })
}

fn dec_withdrawal(d: &mut Dec<'_>) -> Res<Withdrawal> {
    let at = d.position();
    let mut relation = None;
    let mut agent = None;
    let mut ts = None;
    let mut reason = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::withdrawal::RELATION => {
            relation = Some(d.uid()?);
            Ok(true)
        }
        keys::withdrawal::AGENT => {
            agent = Some(AgentId::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::withdrawal::TS => {
            ts = Some(dec_hlc(d)?);
            Ok(true)
        }
        keys::withdrawal::REASON => {
            reason = Some(d.uid()?);
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(Withdrawal {
        relation: relation.ok_or_else(|| bad(at))?,
        agent: agent.ok_or_else(|| bad(at))?,
        ts: ts.ok_or_else(|| bad(at))?,
        reason,
        extra,
    })
}

fn dec_commit(d: &mut Dec<'_>) -> Res<Commit> {
    let at = d.position();
    let mut unit = None;
    let mut level = None;
    let mut agent = None;
    let mut ts = None;
    let mut note = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::commit::UNIT => {
            unit = Some(d.uid()?);
            Ok(true)
        }
        keys::commit::LEVEL => {
            // An unknown level fails the record rather than defaulting: the whole point of the
            // axis is *how settled*, and guessing at it would put words in an author's mouth.
            level = Some(
                Commitment::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(at))?)
                    .ok_or_else(|| bad(at))?,
            );
            Ok(true)
        }
        keys::commit::AGENT => {
            agent = Some(AgentId::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::commit::TS => {
            ts = Some(dec_hlc(d)?);
            Ok(true)
        }
        keys::commit::NOTE => {
            note = Some(d.uid()?);
            Ok(true)
        }
        _ => Ok(false),
    })?;

    Ok(Commit {
        unit: unit.ok_or_else(|| bad(at))?,
        level: level.ok_or_else(|| bad(at))?,
        agent: agent.ok_or_else(|| bad(at))?,
        ts: ts.ok_or_else(|| bad(at))?,
        note,
        extra,
    })
}

fn dec_resolution(d: &mut Dec<'_>) -> Res<Resolution> {
    let at = d.position();
    let mut contention = None;
    let mut relation = None;
    let mut agent = None;
    let mut ts = None;
    let mut note = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::resolution::CONTENTION => {
            contention = Some(ContentionId::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::resolution::RELATION => {
            relation = Some(d.uid()?);
            Ok(true)
        }
        keys::resolution::AGENT => {
            agent = Some(AgentId::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::resolution::TS => {
            ts = Some(dec_hlc(d)?);
            Ok(true)
        }
        keys::resolution::NOTE => {
            note = Some(d.uid()?);
            Ok(true)
        }
        _ => Ok(false),
    })?;

    // Exactly one target. Both or neither cannot be re-encoded to what was read as a resolution
    // of one thing, so it is rejected rather than defaulted.
    let target = match (contention, relation) {
        (Some(c), None) => ResolutionTarget::Contention(c),
        (None, Some(r)) => ResolutionTarget::Relation(r),
        _ => return Err(bad(at)),
    };
    Ok(Resolution {
        target,
        agent: agent.ok_or_else(|| bad(at))?,
        ts: ts.ok_or_else(|| bad(at))?,
        note,
        extra,
    })
}

/// Read a 32-byte string into a fixed array.
fn dec_32(d: &mut Dec<'_>) -> Res<[u8; 32]> {
    let at = d.position();
    d.bytes()?.try_into().map_err(|_| bad(at))
}

/// Read `true`, and only `true`.
///
/// `false` is not an encoding of anything here: the one bool in the format is written only
/// when true (A-5, manifest key 15). Accepting `0xF4` would admit a second spelling of a
/// lossless manifest, which is a different mid for the same facts.
fn dec_true(d: &mut Dec<'_>) -> Res<bool> {
    let at = d.position();
    if d.peek_byte()? != 0xF5 {
        return Err(bad(at));
    }
    d.advance(1);
    Ok(true)
}

fn dec_text_map(d: &mut Dec<'_>) -> Res<std::collections::BTreeMap<String, String>> {
    let at = d.position();
    let n = d.map_head()?;
    let mut out = std::collections::BTreeMap::new();
    let mut prev: Option<Vec<u8>> = None;
    for _ in 0..n {
        let kstart = d.position();
        // The key's own bytes, so the order check below compares what constraint 4 orders by.
        // `skip_item` is what validates them — shortest-form head, valid UTF-8, NFC — and the
        // text is read back out of them rather than decoded twice from the stream.
        let kbytes = d.skip_item()?.to_vec();
        let k = Dec::new(&kbytes)
            .text()
            .map_err(|_| bad(kstart))?
            .to_string();
        // Encoded-key order, which is what `skip_item` enforces for the same map when it
        // arrives under a key this build does not know. Comparing the decoded strings instead
        // would accept a map that a reader one version older rejects.
        if let Some(p) = &prev {
            if *p >= kbytes {
                return Err(bad(kstart));
            }
        }
        prev = Some(kbytes);
        d.reject_null()?;
        if out.insert(k, d.text()?.to_string()).is_some() {
            return Err(bad(at));
        }
    }
    Ok(out)
}

fn dec_part_entry(d: &mut Dec<'_>) -> Res<PartEntry> {
    let at = d.position();
    let mut tid = None;
    let mut length = None;
    let mut structure = None;
    let mut rdid = None;
    let mut lang = None;
    let mut extra = Extra::new();
    read_map(d, &mut extra, |d, k| match k {
        keys::part_entry::TID => {
            tid = Some(Tid::from_bytes(dec_32(d)?));
            Ok(true)
        }
        keys::part_entry::LENGTH => {
            length = Some(d.uint()?);
            Ok(true)
        }
        keys::part_entry::STRUCTURE => {
            structure = Some(dec_32(d)?);
            Ok(true)
        }
        keys::part_entry::RDID => {
            rdid = Some(Rdid::from_bytes(dec_32(d)?));
            Ok(true)
        }
        keys::part_entry::LANG => {
            lang = Some(LangTag::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        _ => Ok(false),
    })?;
    let mut p = PartEntry::new(
        tid.ok_or_else(|| bad(at))?,
        length.ok_or_else(|| bad(at))?,
        structure.ok_or_else(|| bad(at))?,
        rdid.ok_or_else(|| bad(at))?,
    );
    p.lang = lang;
    p.extra = extra;
    Ok(p)
}

fn dec_manifest(d: &mut Dec<'_>) -> Res<Manifest> {
    let at = d.position();
    let mut alias = None;
    let mut parts = None;
    let mut lang = None;
    let mut reader = None;
    let mut licence = None;
    let mut carry = None;
    let mut title = None;
    let mut creators = Vec::new();
    let mut published = None;
    let mut identifiers = std::collections::BTreeMap::new();
    let mut origin = None;
    let mut parent = None;
    let mut parent_kind = None;
    let mut supersedes = None;
    let mut versification = None;
    let mut lossy = false;
    let mut raw = None;
    let mut part_policy = None;
    let mut calendar = None;
    let mut extra = Extra::new();

    read_map(d, &mut extra, |d, k| match k {
        keys::manifest::ALIAS => {
            alias = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::manifest::PARTS => {
            parts = Some(d.array(dec_part_entry)?);
            Ok(true)
        }
        keys::manifest::LANG => {
            lang = Some(LangTag::new(d.text()?).map_err(|_| bad(at))?);
            Ok(true)
        }
        keys::manifest::READER => {
            reader = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::manifest::LICENCE => {
            licence = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::manifest::CARRY => {
            // Closed, unlike `source.kind`: `bundle` reads it to decide whether text may
            // travel (`SMY-E402`), and a code it had to guess at would be a permission
            // granted or refused on a guess.
            carry = Some(
                Carry::from_u8(u8::try_from(d.uint()?).map_err(|_| bad(at))?)
                    .ok_or_else(|| bad(at))?,
            );
            Ok(true)
        }
        keys::manifest::TITLE => {
            title = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::manifest::CREATORS => {
            creators = d.array(|d| Ok(d.text()?.to_string()))?;
            Ok(true)
        }
        keys::manifest::PUBLISHED => {
            published = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::manifest::IDENTIFIERS => {
            identifiers = dec_text_map(d)?;
            Ok(true)
        }
        keys::manifest::ORIGIN => {
            origin = Some(dec_source(d)?);
            Ok(true)
        }
        keys::manifest::PARENT => {
            parent = Some(Mid::from_bytes(dec_32(d)?));
            Ok(true)
        }
        keys::manifest::PARENT_KIND => {
            parent_kind = Some(ParentKind::parse(d.text()?).ok_or_else(|| bad(at))?);
            Ok(true)
        }
        keys::manifest::SUPERSEDES => {
            supersedes = Some(Mid::from_bytes(dec_32(d)?));
            Ok(true)
        }
        keys::manifest::VERSIFICATION => {
            versification = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::manifest::LOSSY => {
            lossy = dec_true(d)?;
            Ok(true)
        }
        keys::manifest::RAW => {
            raw = Some(d.skip_item()?.to_vec());
            Ok(true)
        }
        keys::manifest::PART_POLICY => {
            part_policy = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::manifest::CALENDAR => {
            calendar = Some(Calendar::parse(d.text()?).ok_or_else(|| bad(at))?);
            Ok(true)
        }
        _ => Ok(false),
    })?;

    // Key 12 is required with key 11 and meaningless without it (A-5). Either half alone
    // cannot be re-encoded as what was read, so it is refused rather than dropped.
    let parent = match (parent, parent_kind) {
        (Some(m), Some(k)) => Some((m, k)),
        (None, None) => None,
        _ => return Err(bad(at)),
    };

    let mut m = Manifest::new(
        alias.ok_or_else(|| bad(at))?,
        lang.ok_or_else(|| bad(at))?,
        reader.ok_or_else(|| bad(at))?,
        licence.ok_or_else(|| bad(at))?,
        part_policy.ok_or_else(|| bad(at))?,
    );
    m.parts = parts.ok_or_else(|| bad(at))?;
    m.carry = carry.ok_or_else(|| bad(at))?;
    m.title = title;
    m.creators = creators;
    m.published = published;
    m.identifiers = identifiers;
    m.origin = origin;
    m.parent = parent;
    m.supersedes = supersedes;
    m.versification = versification;
    m.lossy = lossy;
    m.raw = raw;
    m.calendar = calendar;
    m.extra = extra;
    Ok(m)
}

fn dec_part_text(d: &mut Dec<'_>) -> Res<PartText> {
    let at = d.position();
    let mut tid = None;
    let mut text = None;
    let mut extra = Extra::new();
    read_map(d, &mut extra, |d, k| match k {
        keys::part_text::TID => {
            tid = Some(Tid::from_bytes(dec_32(d)?));
            Ok(true)
        }
        keys::part_text::TEXT => {
            text = Some(d.bytes()?.to_vec());
            Ok(true)
        }
        _ => Ok(false),
    })?;
    // The tid is **not** verified here, and that is deliberate (SMYSL-2.4 §4.3.1). A record
    // whose bytes do not hash to its tid has to decode, so that the layer above can report
    // `SMY-E446` and name the part. Failing the decode would let one bad record stop
    // `Store::open` — which is F-12, and once was enough.
    let mut p =
        PartText::with_claimed_tid(tid.ok_or_else(|| bad(at))?, text.ok_or_else(|| bad(at))?);
    p.extra = extra;
    Ok(p)
}

fn dec_part_reading(d: &mut Dec<'_>) -> Res<PartReading> {
    let at = d.position();
    let mut tid = None;
    let mut reader = None;
    let mut segments = None;
    let mut raw = None;
    let mut extra = Extra::new();
    read_map(d, &mut extra, |d, k| match k {
        keys::part_reading::TID => {
            tid = Some(Tid::from_bytes(dec_32(d)?));
            Ok(true)
        }
        keys::part_reading::READER => {
            reader = Some(d.text()?.to_string());
            Ok(true)
        }
        keys::part_reading::SEGMENTS => {
            segments = Some(d.skip_item()?.to_vec());
            Ok(true)
        }
        keys::part_reading::RAW => {
            raw = Some(d.skip_item()?.to_vec());
            Ok(true)
        }
        _ => Ok(false),
    })?;
    let mut r = PartReading::new(
        tid.ok_or_else(|| bad(at))?,
        reader.ok_or_else(|| bad(at))?,
        segments.ok_or_else(|| bad(at))?,
    );
    r.raw = raw;
    r.extra = extra;
    Ok(r)
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
        code::WITHDRAWAL => Record::Withdrawal(dec_withdrawal(&mut d)?),
        code::RESOLUTION => Record::Resolution(dec_resolution(&mut d)?),
        code::COMMIT => Record::Commit(dec_commit(&mut d)?),
        code::MANIFEST => Record::Manifest(dec_manifest(&mut d)?),
        code::PART_TEXT => Record::PartText(dec_part_text(&mut d)?),
        code::PART_READING => Record::PartReading(dec_part_reading(&mut d)?),
        other => {
            // `SMY-W014`: preserved verbatim, skipped semantically. The payload is parsed
            // strictly, so an unknown record cannot smuggle in a non-deterministic encoding.
            //
            // That sentence was here before it was true. Until 0.10 `skip_item` checked
            // shortest form, definite lengths, nulls, tags and depth, but not map key order,
            // NFC, UTF-8 validity or float quantisation — and this payload is stored in
            // `Extra`, which `unit_core_bytes` hashes. Two orderings of one extension map
            // therefore gave one unit two uids. See `tests/invalid_corpus.rs`.
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
