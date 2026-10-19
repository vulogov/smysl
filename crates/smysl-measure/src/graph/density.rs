//! A1, extraction density (SMYSL-2.6 §3.8): units per thousand words of the text they were
//! drawn from, overall and by kind, holder and mode.
//!
//! **The words are the parts'.** §3.8 counts words "from the analyzer over the spans in
//! scope". A unit's span is where it was attached, and spans overlap and leave gaps, so words
//! over spans would count a verse twice where two claims share it and not at all where nothing
//! was extracted — the opposite of what a density is for. The denominator is therefore the
//! analyzer's words over **each part the scope's units are drawn from, once**: how many units
//! a thousand words of that text gave. A unit drawn from no part is counted under `n.unplaced`
//! and not in the density, which is about text.
//!
//! The analyzer is the recipe's, or the one the parts' language chooses; parts that choose
//! two are refused unless the recipe names one (`SMY-E489`), as a text measure refuses them.
//!
//! **By kind, holder and mode**: `kind:<schema>` per schema, `holder:text` for a unit whose
//! `text:holder` is the literal `text` or absent — the text's own voice — and `holder:named`
//! for one with a holder; `mode:<mode>` per `text:mode`, and `mode:none` where the structure
//! established none, which SMYSL-2.4 §3.4 makes the common case.
//!
//! A recipe without a granularity `profile` is refused (`SMY-E489`): a density depends on how
//! finely a text was cut into units, and two densities under two profiles are two questions.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::Code;
use smysl_core::ids::Tid;
use smysl_core::surface::payload_to_object;
use smysl_core::Uid;
use smysl_graph::Store;
use smysl_text::analyze::Chain;
use smysl_text::Library;

use crate::graph;
use crate::text::{analyzer_id, parse_analyzer, Stream};
use crate::{Input, Measure, Needs, Num, Outcome, Purity, Recipe, Refusal, Value};

/// A1.
#[derive(Debug, Clone, Copy, Default)]
pub struct A1;

fn library<'a>(input: &Input<'a>) -> Result<&'a Library, Refusal> {
    input.library.ok_or_else(|| {
        Refusal::new(
            Code::E489,
            "A1 counts the words of the text its units were drawn from, and was given no library",
        )
    })
}

/// The parts the units are drawn from, with each part's language.
fn parts_of(
    store: &Store,
    library: &Library,
    uids: &[Uid],
) -> (BTreeMap<Tid, smysl_core::ids::LangTag>, u64) {
    let mut lang: BTreeMap<Tid, smysl_core::ids::LangTag> = BTreeMap::new();
    for (mid, m) in library.store().manifests() {
        if library.store().is_superseded(mid) {
            continue;
        }
        for p in &m.parts {
            lang.entry(p.tid)
                .or_insert_with(|| p.lang.clone().unwrap_or_else(|| m.lang.clone()));
        }
    }
    let mut out = BTreeMap::new();
    let mut unplaced = 0;
    for u in uids {
        match graph::part_of(store, u).and_then(|t| lang.get(&t).map(|l| (t, l))) {
            Some((t, l)) => {
                out.insert(t, l.clone());
            }
            None => unplaced += 1,
        }
    }
    (out, unplaced)
}

/// The analyzer A1 counts words under: the recipe's, or the one its parts choose.
pub(crate) fn stamp(recipe: &mut Recipe, input: &Input<'_>) -> Result<(), Refusal> {
    if recipe.analyzer.is_some() {
        return Ok(());
    }
    let library = library(input)?;
    let uids = graph::units(recipe, input)?;
    let (parts, _) = parts_of(input.store, library, &uids);
    let chains: BTreeSet<&'static str> = parts.values().map(|l| Chain::new(l).id()).collect();
    if chains.len() > 1 {
        return Err(Refusal::new(
            Code::E489,
            format!(
                "the scope's units are drawn from texts analysed by {}; name one in the \
                 recipe's `analyzer` to count their words in one vocabulary",
                chains.into_iter().collect::<Vec<_>>().join(" and ")
            ),
        ));
    }
    let chain = parts.values().next().map_or_else(Chain::plain, Chain::new);
    recipe.analyzer = Some(analyzer_id(chain.id(), Stream::Surface));
    Ok(())
}

fn text_field(store: &Store, uid: &Uid, key: &str) -> Option<String> {
    let payload = store.get(uid)?.core.payload.as_ref()?;
    let o = payload_to_object(payload).ok()?;
    o.get(key)
        .and_then(|v| v.value.as_str().map(str::to_string))
}

impl Measure for A1 {
    fn id(&self) -> &'static str {
        "A1"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::UNITS.union(Needs::LIBRARY)
    }

    fn complete(&self, recipe: &mut Recipe) {
        graph::complete(recipe);
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        if recipe.profile.is_none() {
            return Err(Refusal::new(
                Code::E489,
                "A1 is a density of units, which depends on how finely the text was cut; its \
                 recipe names no granularity `profile`",
            ));
        }
        library(input)?;
        graph::admitted(recipe, input).map(|_| ())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let run = || -> Result<Value, Refusal> {
            let library = library(input)?;
            let uids = graph::admitted(recipe, input)?;
            let store = input.store;
            let (parts, unplaced) = parts_of(store, library, &uids);
            let (chain, stream) = match &recipe.analyzer {
                Some(a) => parse_analyzer(a).map_err(|d| Refusal::new(d.code, d.message))?,
                None => (Chain::plain(), Stream::Surface),
            };
            let mut words = 0u64;
            for tid in parts.keys() {
                let part = library.part(tid).map_err(|e| {
                    Refusal::new(Code::E489, format!("part {}: {e}", tid.canonical()))
                })?;
                let text = String::from_utf8_lossy(&part.text);
                words += match stream {
                    Stream::Surface => chain.surface(&text).len(),
                    Stream::Stem => chain.stems(&text).len(),
                } as u64;
            }
            if words == 0 {
                return Err(Refusal::new(
                    Code::W485,
                    format!(
                        "A1 found no words under the units of `{}`: none is drawn from a part \
                         the library holds",
                        recipe.scope.trim()
                    ),
                ));
            }
            let placed: Vec<&Uid> = uids
                .iter()
                .filter(|u| graph::part_of(store, u).is_some_and(|t| parts.contains_key(&t)))
                .collect();
            let per_k = |n: u64| graph::scaled(n, 1000, words).map(Num::Fixed);
            let mut by: BTreeMap<String, u64> = BTreeMap::new();
            for u in &placed {
                let unit = store.get(u).expect("placed units are present");
                *by.entry(format!("kind:{}", unit.core.schema)).or_default() += 1;
                let holder = match text_field(store, u, "text:holder").as_deref() {
                    None | Some("text") => "holder:text",
                    Some(_) => "holder:named",
                };
                *by.entry(holder.to_string()).or_default() += 1;
                let mode = text_field(store, u, "text:mode").unwrap_or_else(|| "none".into());
                *by.entry(format!("mode:{mode}")).or_default() += 1;
            }
            let Some(d) = per_k(placed.len() as u64) else {
                return Err(Refusal::new(
                    Code::E489,
                    "the density does not fit a payload number",
                ));
            };
            let mut v = Value::new()
                .with_n("units", uids.len() as u64)
                .with_n("placed", placed.len() as u64)
                .with_n("unplaced", unplaced)
                .with_n("parts", parts.len() as u64)
                .with_n("words", words)
                .with_value("density", d)
                .with_headline_in("density", "extraction density", "units per 1000 words");
            for (k, n) in by {
                if let Some(x) = per_k(n) {
                    v = v.with_value(k, x);
                }
            }
            Ok(v)
        };
        match run() {
            Ok(v) => Outcome::Value(v),
            Err(r) => Outcome::refused(r),
        }
    }
}
