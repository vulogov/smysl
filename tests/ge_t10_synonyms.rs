//! Rule 34's calibration, measured (1.13.0): `fixtures/dossier/synonyms/calibrate.tsv` scored
//! with a model, as its header says. The rule was not adopted — no threshold separated the
//! wanted rows from the guards — and this keeps the figure that decided it, so the question can
//! be asked again of another model by setting the variable to it.
//!
//! ```text
//! SMYSL_EMBED_MODEL_MULTILINGUAL=/path/to/potion-multilingual-128M cargo test --release \
//!     --features semantic --test ge_t10_synonyms -- --nocapture
//! ```
//!
//! The scoring is rule 34's as declared in `questions.tsv`: the question's topic, whole, put to
//! the store's semantic source, and each entity scored by the best cosine of its own unit or of
//! any unit that mentions it, groups formed as the subject search forms them. It is here and not
//! in `smysl-query` because nothing there uses it. Skips without a model, as the other model
//! tests do. A measurement, not a gate: with `potion-multilingual-128M` it records that the rows
//! do not separate, and a model under which they do is the occasion to declare the rule again.

#![cfg(all(feature = "semantic", feature = "text"))]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl::query::dossier::classify;
use smysl::query::ports::SemanticSource;
use smysl::query::RetrieverSource;
use smysl::{parse_surface, EmbedModel, Semantic, Store, Uid};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn model() -> Option<EmbedModel> {
    let path = std::env::var("SMYSL_EMBED_MODEL_MULTILINGUAL").ok()?;
    match EmbedModel::from_dir(&path) {
        Ok(m) => Some(m),
        Err(e) => {
            panic!("SMYSL_EMBED_MODEL_MULTILINGUAL is {path} and the model would not load: {e}")
        }
    }
}

/// Each thing the store's entities name, scored by how near `topic` is to it: best first.
fn meaning_scores(
    topic: &str,
    store: &Store,
    semantic: &dyn SemanticSource,
) -> Vec<(Vec<Uid>, f32)> {
    let groups = smysl::query::store_scope::entity_candidates_where(store, |_| true);
    let members: BTreeSet<Uid> = groups.iter().flatten().copied().collect();
    let kind = smysl::RelKind::parse(smysl::schemas::text::MENTIONS).unwrap();
    let mut mentions: BTreeMap<Uid, BTreeSet<Uid>> = BTreeMap::new();
    for r in store.relations_of_kind(&kind) {
        if members.contains(&r.to) {
            mentions.entry(r.from).or_default().insert(r.to);
        }
    }
    let within: BTreeSet<Uid> = members.iter().chain(mentions.keys()).copied().collect();
    let mut best: BTreeMap<Uid, f32> = BTreeMap::new();
    for hit in semantic.search(topic, &within, within.len()) {
        let own = members.contains(&hit.uid).then_some(hit.uid);
        let named = mentions.get(&hit.uid).into_iter().flatten().copied();
        for e in own.into_iter().chain(named) {
            let b = best.entry(e).or_insert(hit.score);
            *b = b.max(hit.score);
        }
    }
    let mut out: Vec<(Vec<Uid>, f32)> = groups
        .into_iter()
        .filter_map(|g| {
            let s = g
                .iter()
                .filter_map(|u| best.get(u))
                .copied()
                .reduce(f32::max)?;
            Some((g, s))
        })
        .collect();
    out.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    out
}

#[test]
fn rule_34_calibration() {
    let Some(m) = model() else {
        eprintln!("skipped: set SMYSL_EMBED_MODEL_MULTILINGUAL to a model directory");
        return;
    };
    let src = std::fs::read_to_string(repo("fixtures/dossier/gold/corpus.smy")).unwrap();
    let parsed = parse_surface(&src).unwrap();
    let labels: BTreeMap<String, Uid> = parsed
        .labels
        .iter()
        .map(|(l, u)| (l.to_string(), *u))
        .collect();
    let store = Store::from_records(parsed.records);
    let index = Semantic::index(&store, m);
    let hash = index.model_hash();
    let source = RetrieverSource::new(index, hash);

    let text = std::fs::read_to_string(repo("fixtures/dossier/synonyms/calibrate.tsv")).unwrap();
    let mut out = "rule 34 calibration (calibrate.tsv)\n".to_string();
    let (mut wanted_low, mut guard_high, mut gap_low) = (f32::INFINITY, 0.0f32, f32::INFINITY);
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
    {
        let r: Vec<&str> = line.split('\t').collect();
        let (qid, want, lang, question) = (r[0], r[1], r[2], r[3]);
        let topic = classify(question, None).unwrap().topic;
        let scored = meaning_scores(&topic, &store, &source);
        let top = scored.first().map_or(0.0, |(_, s)| *s);
        if want == "none" {
            guard_high = guard_high.max(top);
            let _ = writeln!(
                out,
                "  {qid:<6} {lang}  guard        top {top:.3}: {question}"
            );
            continue;
        }
        let uid = labels[want];
        let own = scored
            .iter()
            .find(|(g, _)| g.contains(&uid))
            .map_or(0.0, |(_, s)| *s);
        let next = scored
            .iter()
            .find(|(g, _)| !g.contains(&uid))
            .map_or(0.0, |(_, s)| *s);
        wanted_low = wanted_low.min(own);
        gap_low = gap_low.min(own - next);
        let _ = writeln!(
            out,
            "  {qid:<6} {lang}  {want:<12} {own:.3}, next {next:.3}: {question}"
        );
    }
    let _ = writeln!(
        out,
        "  lowest wanted {wanted_low:.3}, highest guard {guard_high:.3}, smallest gap {gap_low:.3}: {}",
        if wanted_low > guard_high {
            "a T separates them"
        } else {
            "no T separates them"
        }
    );
    println!("{out}");
    // What 1.13.0 decided on. A model that separates them fails this, which is the point: the
    // rule is then worth declaring again, on a set written for it.
    assert!(wanted_low <= guard_high, "{out}");
}
