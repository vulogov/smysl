//! Joining what an import brought to what the library holds (SMYSL-2.7 §3.5, TX-P11 step 9).
//!
//! Two pure joins, both writing `x.text/same-as` edges:
//!
//! - **`identical-span`** (TX-P7): the same bytes of the same part, read under two manifests,
//!   with the same gist. Not a guess, which is why S0 accepted it at rung `computed`.
//! - **Lexical entity same-as within a language**: two `x.text/entity` units with one name in
//!   one language, from two expressions. A-2.4 rule 7 makes an entity's identity its name, its
//!   language and its expression, and its source `alias:<expression alias>` with no manifest,
//!   so the same person read from two editions is two units, and this is the edge that says so.
//!
//! An edge is proposed only when one end at least is a unit this import introduced, and only
//! when the library does not already hold it live. The edges and their attestations are **not
//! records of the input**, so they go to their own staged batch, `join:<name>`, under a marker
//! of their own rather than any manifest, and P1, which counts what the import retained,
//! leaves them out (§3.9). The clock is fixed, `Hlc::new(0, 0, tool:smysl-lib)` as §3.2 fixes
//! merge's, so the same imports propose the same records.

use smysl_core::schemas::text;
use smysl_core::{
    AgentId, Attestation, Hlc, Mid, Op, Record, RelKind, Relation, Rung, SchemaId, Uid,
};
use smysl_graph::Store;
use smysl_text::proposition::{Engine, IdenticalSpan, Proposal};
use std::collections::{BTreeMap, BTreeSet};

/// The batch name of an import's joins.
pub(crate) fn batch(name: &str) -> String {
    format!("join:{name}")
}

/// The marker a join batch's `imported` rows stand under, in place of a manifest: a mid nothing
/// else can be, since no manifest's body hashes to it.
pub(crate) fn marker(name: &str) -> Mid {
    Mid::from_bytes(*blake3::hash(format!("smysl/join/1\0{name}").as_bytes()).as_bytes())
}

fn entity_agent() -> AgentId {
    AgentId::new("tool:smysl-same-as-entity").expect("an agent id")
}

/// Entity units with one name in one language from two expressions.
fn entities(store: &Store, scope: &BTreeSet<Uid>) -> Vec<Proposal> {
    let entity = SchemaId::Extension(text::ENTITY.to_string());
    // (language, name) -> (unit, the expression it is from).
    type Named = Vec<(Uid, Option<String>)>;
    let mut by_name: BTreeMap<(String, String), Named> = BTreeMap::new();
    for (u, unit) in store.units() {
        let c = &unit.core;
        if c.schema != entity {
            continue;
        }
        let Some(lang) = &c.lang else {
            continue;
        };
        let expression = c
            .source
            .as_ref()
            .and_then(|s| s.reference.strip_prefix("alias:"))
            .map(str::to_string);
        by_name
            .entry((lang.as_str().to_string(), c.gist.trim().to_string()))
            .or_default()
            .push((*u, expression));
    }
    let mut out = BTreeSet::new();
    for ((lang, gist), units) in &by_name {
        for (i, (a, ma)) in units.iter().enumerate() {
            for (b, mb) in &units[i + 1..] {
                if ma.is_none() || mb.is_none() || ma == mb {
                    continue;
                }
                if !scope.contains(a) && !scope.contains(b) {
                    continue;
                }
                out.extend(Proposal::new(
                    *a,
                    *b,
                    format!("one name, `{gist}`, in {lang}"),
                ));
            }
        }
    }
    out.into_iter().collect()
}

/// The join records over `store`, for the units `introduced`: edges, attestations, and the
/// `x.text/v1` declaration when the store has none. Empty when nothing is new.
pub(crate) fn propose(store: &Store, introduced: &BTreeSet<Uid>) -> Vec<Record> {
    let held: BTreeSet<Uid> = store
        .relations()
        .filter(|r| matches!(&r.kind, RelKind::Extension(n) if n == text::SAME_AS))
        .filter(|r| !store.is_withdrawn(r))
        .map(|r| r.uid())
        .collect();
    let identical = IdenticalSpan;
    let spans: Vec<(AgentId, Proposal)> = identical
        .propose(store, None)
        .into_iter()
        .filter(|p| introduced.contains(&p.a) || introduced.contains(&p.b))
        .map(|p| (identical.agent(), p))
        .collect();
    let names = entities(store, introduced)
        .into_iter()
        .map(|p| (entity_agent(), p));
    let mut out: BTreeMap<Uid, (AgentId, Relation)> = BTreeMap::new();
    for (agent, p) in spans.into_iter().chain(names) {
        let rel = Relation::new(RelKind::Extension(text::SAME_AS.to_string()), p.a, p.b);
        if held.contains(&rel.uid()) {
            continue;
        }
        out.entry(rel.uid()).or_insert((agent, rel));
    }
    if out.is_empty() {
        return Vec::new();
    }
    let declared = store
        .iter()
        .any(|r| matches!(r, Record::SchemaDecl(d) if d.id.as_str() == text::ID));
    let mut records = Vec::new();
    if !declared {
        records.push(Record::SchemaDecl(text::decl()));
    }
    let clock = Hlc::new(0, 0, AgentId::new("tool:smysl-lib").expect("an agent id"));
    for (rid, (agent, rel)) in out {
        records.push(Record::Relation(rel));
        records.push(Record::Attestation(Attestation::new(
            rid,
            agent,
            Op::Imported,
            Rung::Computed,
            clock.clone(),
        )));
    }
    records
}
