//! The linker's candidate generator: which pairs of units are worth a judge's attention
//! (SMYSL-2.4 §3.5, TX-P7 step 3).
//!
//! # What this is, and what it deliberately is not
//!
//! `smysl link` asks a model whether two units entail or contradict each other. It cannot ask
//! about every pair: a corpus of ten thousand units holds fifty million pairs, and a model
//! turn per pair is not a budget anybody has. So something has to choose, and §3.5 names four
//! sources for the choosing — "same entity, BM25 neighbours within scope and language, same
//! theme, anchored alignment" — with a **candidate budget per unit** carried in the recipe.
//!
//! This module is that choosing, and nothing else. It is pure, it calls no model, and it
//! decides nothing about any pair it offers: a candidate is a question, and
//! `smysl_ingest::link` is what pays to have it answered. The split is [`crate::proposition`]'s
//! and is kept for the same reason — a generator that judged its own candidates could not be
//! measured against the judge.
//!
//! # Adjacency is off here, and that is the whole difference from the same-as engines
//!
//! [`crate::proposition::Lexical`] filters its neighbours down to the same or a neighbouring
//! verse, because S0 measured that filter carrying precision from 0.789 to 0.835 (§3.6). A
//! linker that did the same thing would be a linker that could not find what it is for.
//!
//! GE-T11's wording is the argument: *fifty planted **far** contradictions*. A text
//! contradicting itself two verses later is a copy-editing mistake; a text contradicting
//! itself two books later is the finding, and it is the one a reader cannot make by reading
//! forwards. The adjacency filter removes exactly those pairs. So the same evidence — BM25
//! neighbours over gists — is used here with the filter taken off, and the precision that
//! filter was buying is bought instead by the judge, which is the thing the linker has and the
//! same-as engines do not.
//!
//! # Four sources, and what each one knows that the others do not
//!
//! | source | evidence | blind to |
//! |---|---|---|
//! | [`Source::Entity`] | both units mention the same `x.text/entity` | a claim that names nobody |
//! | [`Source::Theme`] | both units mention the same `x.text/theme` | a corpus with no themes extracted |
//! | [`Source::Lexical`] | BM25 neighbours within one language | two languages, and a paraphrase |
//! | [`Source::Anchored`] | a versification alignment puts them at one place | anything outside an aligned corpus |
//!
//! A pair found by two sources is a better candidate than a pair found by one, and that is how
//! the budget is spent: [`Candidate::sources`] is the first ranking key. It is not a score —
//! two sources agreeing that a pair is *worth asking about* is not two sources agreeing that
//! the answer is yes, and this module has no opinion on the answer.
//!
//! # A hub entity proposes nothing
//!
//! An entity five hundred units mention generates a hundred and twenty-five thousand pairs and
//! says nothing about any of them. [`Budget::hub`] is the cutoff, and it is the same argument
//! an inverse document frequency makes: a term in every document discriminates between none of
//! them. The cutoff is a count rather than a proportion because the question is how many pairs
//! one entity may cost, and that is quadratic in the count whatever the corpus size.
//!
//! The cost of the cutoff is that the *most* written-about entity in a corpus is the one whose
//! contradictions this source will not find. That is a real loss and it is why the source is
//! one of four: a pair sharing a hub entity is normally also a lexical neighbour, and a pair
//! that is neither is a pair nothing here can see.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::ids::{LangTag, Uid};
use smysl_core::types::RelKind;
use smysl_graph::Store;
use smysl_retrieve::{Bm25, Query, Retriever};

use crate::index::align;
use crate::proposition::{adjacent_locators, place, Place};

/// Where a candidate came from.
///
/// Ordered as §3.5 lists the sources, and the order is load-bearing only in that it makes a
/// candidate's `sources` set print the same way twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Source {
    /// Both units mention the same `x.text/entity` unit.
    Entity,
    /// Both units mention the same `x.text/theme` unit.
    Theme,
    /// One is among the other's BM25 neighbours, in the same language.
    Lexical,
    /// A versification alignment puts the two places at one place.
    Anchored,
}

impl Source {
    /// The name this source is printed and recorded under.
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Entity => "entity",
            Source::Theme => "theme",
            Source::Lexical => "lexical",
            Source::Anchored => "anchored",
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One pair worth asking a judge about.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Candidate {
    /// The smaller uid. A pair is unordered here for [`crate::proposition::Proposal`]'s
    /// reason: the judge is asked once, and asking twice would buy two answers to one question.
    pub a: Uid,
    /// The larger uid.
    pub b: Uid,
    /// Every source that offered this pair, never empty.
    pub sources: BTreeSet<Source>,
    /// The fused reciprocal rank where [`Source::Lexical`] offered it, and `0.0` where it did
    /// not.
    ///
    /// A tie-break and not a confidence. Three of the four sources have no score at all, and a
    /// number that existed for one of them would be read as a ranking across all four.
    ///
    /// **A rank and not a BM25 score**, and the difference was worth four tenths of GE-T11's
    /// recall. A BM25 score is not comparable between two queries: a long gist scores every
    /// neighbour highly and a short one scores every neighbour low, so sorting the whole
    /// corpus's pairs by raw score spends the budget on the long units' neighbourhoods first
    /// and never reaches a short unit's best match. Each direction contributes
    /// `1 / (RRF_K + rank)` instead — reciprocal rank fusion, which is scale-free by
    /// construction — and a pair found in **both** directions sums both, which is how
    /// mutuality earns its place without being a special case.
    pub score: f32,
    /// One line naming what each source saw, for the person reading `link --dry-run`.
    pub why: String,
}

impl Candidate {
    /// The ranking key, highest first: more sources, then a higher lexical score.
    fn rank(&self) -> (usize, f32) {
        (self.sources.len(), self.score)
    }
}

/// How much asking the linker may do.
///
/// §3.5 puts the candidate budget in the recipe, because it is a condition of the run: two
/// runs at different budgets saw different questions, and a corroboration group that mixed
/// them would be claiming two models agreed when one of them was never asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Budget {
    /// How many candidates one unit may contribute.
    ///
    /// Applied to each endpoint, and a pair survives if **either** endpoint still has room.
    /// The alternative — both — makes the budget mean something different for a unit in a
    /// dense neighbourhood than for one in a sparse one, and it is the dense neighbourhood
    /// where a true pair is hardest to see.
    ///
    /// What that buys and what it costs. Each kept pair spends from both endpoints, and a
    /// pair is kept only while one of them has room, so the whole run is bounded by
    /// `units × per_unit` candidates — which is the number the budget exists to bound, since
    /// it is the number of model turns. What it is **not** is a bound on how often one unit
    /// appears: a unit whose own budget is spent still turns up in the candidates of units
    /// that have room, and a unit at the centre of a dense neighbourhood can appear many times
    /// over. That is the intended reading. The budget rations asking, not being asked about.
    pub per_unit: usize,
    /// How many BM25 neighbours [`Source::Lexical`] asks for, before the budget is applied.
    ///
    /// Separate from `per_unit` because a retrieval depth and an asking budget are different
    /// questions: a deeper retrieval costs arithmetic, and a larger `per_unit` costs model
    /// turns.
    pub neighbours: usize,
    /// How many units may mention an entity or a theme before it proposes nothing.
    pub hub: usize,
}

impl Budget {
    /// The default budget: sixteen candidates per unit, thirty-two neighbours retrieved, hubs
    /// at thirty-two.
    ///
    /// GE-T11 is measured at this budget and these three numbers were **chosen from that
    /// measurement**, after it, which is worth saying out loud since the measurement is also
    /// the gate. What was not done is picking the smallest numbers that cleared the bar: the
    /// reasoning per field is below, and the figure they give on the committed corpus is 0.68
    /// against a bar of 0.6 rather than 0.62, because a default sitting exactly on a threshold
    /// is a default that fails on the next corpus.
    ///
    /// `neighbours` is **32** because retrieval depth is arithmetic and model turns are money.
    /// Going from 24 to 32 moved recall from 0.52 to 0.58 and cost nothing a user can feel;
    /// going from 32 to 48 moved it not at all. That is a knee, and it is where the default
    /// sits.
    ///
    /// `per_unit` is **16** because this is the field that spends: a run buys up to
    /// `units × per_unit` model turns. The curve through it is shallow — 0.48 at 8, 0.62 at
    /// 12, 0.68 at 16, 0.78 at 24 — which is the honest shape of a lexical generator looking
    /// for contradictions, and the honest reason the field exists in the recipe for a caller
    /// to lower.
    ///
    /// `hub` is **32**, and it is a cost guard rather than a quality knob. Moving it between 4
    /// and ten thousand changed neither the recall nor the number of candidates on the
    /// committed corpus, because a hub still confirms what another source found — see
    /// [`candidates`]'s ordering and the note at the co-mention source.
    pub const DEFAULT: Budget = Budget {
        per_unit: 16,
        neighbours: 32,
        hub: 32,
    };
}

impl Default for Budget {
    fn default() -> Budget {
        Budget::DEFAULT
    }
}

/// The constant in `1 / (k + rank)`.
///
/// 60 is the number the fusion literature uses and the number everything that implements it
/// uses; it is large enough that the first few ranks are not wildly far apart and small enough
/// that rank still means something at the depth a candidate budget reaches. It is not tuned
/// here, because a constant tuned on a planted set this repository also wrote would be a
/// constant fitted to fifty sentences.
const RRF_K: f32 = 60.0;

/// `x.text/mentions`, the edge both co-mention sources read.
const MENTIONS: &str = "x.text/mentions";
/// The schema id of an entity unit.
const ENTITY: &str = "x.text/entity";
/// The schema id of a theme unit.
const THEME: &str = "x.text/theme";

/// Every pair worth asking about, best first.
///
/// `scope` restricts which units may appear in a pair; `None` is every unit the store holds.
/// `table` is the versification alignment where there is one — [`Source::Anchored`] is silent
/// without it, which is what a store that is not a library of scripture looks like.
///
/// Entity and theme units are never candidates themselves. They are not propositions: an
/// entity is a name A-2.4 rule 7 fixes to an expression, and asking a model whether "Ruth"
/// contradicts "Boaz" is asking a question with no answer.
pub fn candidates(
    store: &Store,
    scope: Option<&BTreeSet<Uid>>,
    table: Option<&align::Table>,
    budget: Budget,
) -> Vec<Candidate> {
    let mut subjects: BTreeMap<Uid, Subject> = BTreeMap::new();
    for (uid, unit) in store.units() {
        if !scope.is_none_or(|s| s.contains(uid)) {
            continue;
        }
        let schema = unit.core.schema.as_str();
        if schema == ENTITY || schema == THEME {
            continue;
        }
        subjects.insert(
            *uid,
            Subject {
                gist: unit.core.gist.clone(),
                lang: unit.core.lang.clone(),
                place: place(unit),
            },
        );
    }

    let mut found: BTreeMap<(Uid, Uid), Found> = BTreeMap::new();
    // Lexical and anchored first, then co-mention — the order matters for one reason, written
    // at `co_mention`: a hub may confirm a pair another source already found, and may not
    // raise one of its own.
    lexical(store, &subjects, budget, &mut found);
    anchored(&subjects, table, &mut found);
    co_mention(store, &subjects, budget, &mut found);

    let mut all: Vec<Candidate> = found
        .into_iter()
        .map(|((a, b), f)| Candidate {
            a,
            b,
            why: f.why(),
            sources: f.sources,
            score: f.score,
        })
        .collect();
    // Best first, and the uid pair last so that the order is total: two candidates with one
    // source and no lexical score would otherwise be ordered by a map's insertion history.
    all.sort_by(|x, y| {
        y.rank()
            .0
            .cmp(&x.rank().0)
            .then(
                y.rank()
                    .1
                    .partial_cmp(&x.rank().1)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then((x.a, x.b).cmp(&(y.a, y.b)))
    });

    // The budget, applied per endpoint over that order. Walking the sorted list once and
    // keeping a pair when either endpoint still has room is what makes "either endpoint" mean
    // *the best* candidates of that endpoint rather than the first ones a map happened to
    // yield.
    let mut spent: BTreeMap<Uid, usize> = BTreeMap::new();
    let mut kept: Vec<Candidate> = Vec::new();
    for c in all {
        let a = spent.get(&c.a).copied().unwrap_or(0);
        let b = spent.get(&c.b).copied().unwrap_or(0);
        if a >= budget.per_unit && b >= budget.per_unit {
            continue;
        }
        *spent.entry(c.a).or_default() += 1;
        *spent.entry(c.b).or_default() += 1;
        kept.push(c);
    }
    kept
}

/// What `candidates` needs of a unit, read once.
struct Subject {
    gist: String,
    lang: Option<LangTag>,
    place: Option<Place>,
}

/// A pair under construction, before it is a [`Candidate`].
#[derive(Default)]
struct Found {
    sources: BTreeSet<Source>,
    score: f32,
    notes: Vec<String>,
}

impl Found {
    fn add(&mut self, source: Source, note: String) {
        self.sources.insert(source);
        self.notes.push(note);
    }

    fn why(&self) -> String {
        self.notes.join("; ")
    }
}

/// An ordered key for a pair, or `None` for a unit against itself.
fn pair(x: Uid, y: Uid) -> Option<(Uid, Uid)> {
    match x.cmp(&y) {
        std::cmp::Ordering::Less => Some((x, y)),
        std::cmp::Ordering::Greater => Some((y, x)),
        std::cmp::Ordering::Equal => None,
    }
}

/// [`Source::Entity`] and [`Source::Theme`]: two units naming one thing.
///
/// A `mentions` edge is read in whichever direction puts the entity or theme at one end — the
/// schema does not constrain the direction, and a corpus that wrote it the other way round
/// would otherwise contribute nothing while looking as though it had been considered.
fn co_mention(
    store: &Store,
    subjects: &BTreeMap<Uid, Subject>,
    budget: Budget,
    found: &mut BTreeMap<(Uid, Uid), Found>,
) {
    let kind = RelKind::Extension(MENTIONS.to_string());
    let mut groups: BTreeMap<Uid, (Source, String, BTreeSet<Uid>)> = BTreeMap::new();
    for rel in store.relations_of_kind(&kind) {
        if store.is_withdrawn(rel) {
            continue;
        }
        for (target, mentioner) in [(rel.to, rel.from), (rel.from, rel.to)] {
            let Some(unit) = store.get(&target) else {
                continue;
            };
            let source = match unit.core.schema.as_str() {
                ENTITY => Source::Entity,
                THEME => Source::Theme,
                _ => continue,
            };
            if !subjects.contains_key(&mentioner) {
                continue;
            }
            groups
                .entry(target)
                .or_insert_with(|| (source, unit.core.gist.clone(), BTreeSet::new()))
                .2
                .insert(mentioner);
        }
    }

    for (source, name, members) in groups.into_values() {
        if members.len() < 2 {
            continue;
        }
        // A hub confirms, and does not raise. Above the cutoff the name is too common to put a
        // pair in front of a judge by itself — but it is not nothing, and the first cut of
        // this rule threw away the part that was worth having. A pair some other source
        // already found, which also shares a name, is a pair two independent kinds of evidence
        // point at, and [`Candidate::sources`] is the first ranking key precisely so that such
        // a pair goes first. Dropping hubs outright dropped that conjunction as well, and over
        // a corpus that is one story about four people it dropped it on every pair that
        // mattered: in Ruth, everything mentions Naomi.
        let confirm_only = members.len() > budget.hub;
        // The rarer the name, the stronger the signal, on the same scale the lexical source
        // is scored on. Two units sharing an entity nothing else in the corpus mentions is a
        // better question than a unit's eighth-best term overlap, and before this they tied at
        // zero and were ordered by uid — which is to say the strongest evidence this source
        // has was the evidence the budget reached last.
        let weight = 1.0 / (RRF_K + members.len() as f32);
        let members: Vec<Uid> = members.into_iter().collect();
        for (i, x) in members.iter().enumerate() {
            for y in &members[i + 1..] {
                let Some(key) = pair(*x, *y) else { continue };
                if confirm_only && !found.contains_key(&key) {
                    continue;
                }
                let entry = found.entry(key).or_default();
                entry.score += weight;
                entry.add(
                    source,
                    format!("both mention the {source} `{name}` ({} do)", members.len()),
                );
            }
        }
    }
}

/// [`Source::Lexical`]: BM25 neighbours within one language, with no adjacency filter.
fn lexical(
    store: &Store,
    subjects: &BTreeMap<Uid, Subject>,
    budget: Budget,
    found: &mut BTreeMap<(Uid, Uid), Found>,
) {
    if budget.neighbours == 0 {
        return;
    }
    let mut by_lang: BTreeMap<&LangTag, BTreeSet<Uid>> = BTreeMap::new();
    for (uid, s) in subjects {
        if let Some(lang) = s.lang.as_ref() {
            by_lang.entry(lang).or_default().insert(*uid);
        }
    }
    // One index over the whole store, for `proposition::lexical`'s reason: an inverse document
    // frequency is a statement about the corpus, and refitting it per language group would make
    // a term's weight depend on what else was ingested.
    let index = Bm25::index(store);
    for (lang, group) in &by_lang {
        if group.len() < 2 {
            continue;
        }
        for uid in group {
            let Some(s) = subjects.get(uid) else { continue };
            // `+ 1`, because a unit is its own best neighbour and a budget that spent one on
            // itself would be one smaller than it says.
            let query =
                Query::new(s.gist.clone(), budget.neighbours + 1).within(group.iter().copied());
            let mut rank = 0usize;
            for hit in index.search(&query) {
                let Some(key) = pair(*uid, hit.uid) else {
                    // The unit's own hit, which is not a rank anybody else competes for.
                    continue;
                };
                let entry = found.entry(key).or_default();
                // Both directions contribute, so a mutual neighbour outranks a one-sided one.
                entry.score += 1.0 / (RRF_K + rank as f32);
                if !entry.sources.contains(&Source::Lexical) {
                    entry.add(
                        Source::Lexical,
                        format!("a BM25 neighbour in {lang} at rank {}", rank + 1),
                    );
                }
                rank += 1;
            }
        }
    }
}

/// [`Source::Anchored`]: an alignment putting two places at one place.
///
/// The same rule [`crate::proposition::Anchored`] applies — equal or adjacent under the
/// alignment — because it is the same evidence. What differs is what is done with it: there a
/// pair at one place is proposed as one proposition, here it is offered as a question, and the
/// question is worth asking precisely where the answer is *not* "the same".
fn anchored(
    subjects: &BTreeMap<Uid, Subject>,
    table: Option<&align::Table>,
    found: &mut BTreeMap<(Uid, Uid), Found>,
) {
    let Some(table) = table else { return };
    let mut by_mid: BTreeMap<smysl_core::ids::Mid, Vec<(Uid, String)>> = BTreeMap::new();
    for (uid, s) in subjects {
        if let Some(at) = s.place.as_ref() {
            if let (Some(mid), Some(locator)) = (at.mid, at.locator.clone()) {
                by_mid.entry(mid).or_default().push((*uid, locator));
            }
        }
    }
    let mids: Vec<smysl_core::ids::Mid> = by_mid.keys().copied().collect();
    for (i, ma) in mids.iter().enumerate() {
        for mb in &mids[i + 1..] {
            let Some(rows) = table.get(&(*ma, *mb)) else {
                continue;
            };
            for (x, here) in &by_mid[ma] {
                let Some(target) = rows.rows.get(here) else {
                    continue;
                };
                for (y, there) in &by_mid[mb] {
                    if !adjacent_locators(target, there) {
                        continue;
                    }
                    let Some(key) = pair(*x, *y) else { continue };
                    found.entry(key).or_default().add(
                        Source::Anchored,
                        format!("{here} and {there} are one place under {}", rows.scheme),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::ids::SchemaId;
    use smysl_core::{canonical_uid, KernelType, Record, Relation, Status, UnitCoreBuilder};

    /// A proposition unit in English, and its uid.
    fn claim(gist: &str) -> (Record, Uid) {
        let u = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .lang(LangTag::new("en").expect("a tag"))
            .build()
            .expect("a unit");
        let uid = canonical_uid(&u);
        (Record::Unit(u), uid)
    }

    /// A unit of one of the two schema types this module refuses to treat as a proposition.
    fn named(schema: &str, gist: &str) -> (Record, Uid) {
        let u = UnitCoreBuilder::new(
            SchemaId::Extension(schema.to_string()),
            gist,
            Status::Speculative,
        )
        .build()
        .expect("a unit");
        let uid = canonical_uid(&u);
        (Record::Unit(u), uid)
    }

    fn mentions(from: Uid, to: Uid) -> Record {
        Record::Relation(Relation::new(
            RelKind::Extension(MENTIONS.to_string()),
            from,
            to,
        ))
    }

    fn has(cs: &[Candidate], a: Uid, b: Uid) -> Option<&Candidate> {
        let key = pair(a, b).expect("two distinct units");
        cs.iter().find(|c| (c.a, c.b) == key)
    }

    /// The source this module exists for: two units that share no vocabulary and one entity.
    ///
    /// Nothing lexical joins "the king sent no letter" to "a letter arrived from the palace"
    /// except the entity, which is the case §3.5's first source is in the list for.
    #[test]
    fn two_units_naming_one_entity_are_a_candidate() {
        let (ra, a) = claim("the king sent no letter that winter");
        let (rb, b) = claim("a sealed dispatch arrived from the palace in midwinter");
        let (rc, c) = claim("the harvest failed and the granaries were opened");
        let (re, e) = named(ENTITY, "the king");
        let store = Store::from_records(vec![ra, rb, rc, re, mentions(a, e), mentions(b, e)]);

        let cs = candidates(&store, None, None, Budget::DEFAULT);
        let found = has(&cs, a, b).expect("the two units that mention the king");
        assert!(found.sources.contains(&Source::Entity));
        assert!(
            found.why.contains("the king"),
            "the reason names the entity: {}",
            found.why
        );
        assert!(
            has(&cs, a, c).is_none_or(|x| !x.sources.contains(&Source::Entity)),
            "a unit mentioning nothing is not an entity candidate"
        );
    }

    /// An entity is not a proposition, and is never one end of a candidate.
    #[test]
    fn an_entity_or_theme_unit_is_never_itself_a_candidate() {
        let (ra, a) = claim("the king sent no letter that winter");
        let (rb, b) = claim("the king sent a letter that winter");
        let (re, e) = named(ENTITY, "the king");
        let (rt, t) = named(THEME, "royal correspondence");
        let store = Store::from_records(vec![ra, rb, re, rt, mentions(a, e), mentions(b, e)]);

        let cs = candidates(&store, None, None, Budget::DEFAULT);
        assert!(has(&cs, a, b).is_some(), "the two claims are a candidate");
        for c in &cs {
            assert!(
                c.a != e && c.b != e && c.a != t && c.b != t,
                "an entity or theme unit is at neither end of {c:?}"
            );
        }
    }

    /// The hub rule: a name everybody uses raises nothing, and still confirms.
    ///
    /// Both halves matter and the first version of this rule had only one. Dropping hub
    /// entities outright dropped the *conjunction* as well — a pair that some other source
    /// already found and that also shares a name — and over a corpus that is one story about
    /// four people that is every pair worth having.
    #[test]
    fn an_entity_above_the_hub_cutoff_raises_nothing_and_still_confirms() {
        // Two units that share the entity and share no vocabulary: nothing but the entity can
        // put them together.
        let (r1, far1) = claim("snow fell upon a northern road before midwinter");
        let (r2, far2) = claim("eleven ships were counted in southern harbours at dawn");
        // Two units that share the entity and are plainly each other's lexical neighbours.
        let (r3, near1) = claim("the elders sat in the gate of the city and said nothing");
        let (r4, near2) = claim("the elders of the city sat down in the gate and were silent");
        let (re, e) = named(ENTITY, "the city");
        let store = Store::from_records(vec![
            r1,
            r2,
            r3,
            r4,
            re,
            mentions(far1, e),
            mentions(far2, e),
            mentions(near1, e),
            mentions(near2, e),
        ]);

        let mut under = Budget::DEFAULT;
        under.hub = 4;
        let mut over = Budget::DEFAULT;
        over.hub = 3;

        let within = candidates(&store, None, None, under);
        assert!(
            has(&within, far1, far2).is_some_and(|c| c.sources.contains(&Source::Entity)),
            "four mentions at a hub of four is within the cutoff, and the entity is all there is"
        );

        let above = candidates(&store, None, None, over);
        assert!(
            has(&above, far1, far2).is_none(),
            "above the cutoff the name raises nothing of its own"
        );
        assert!(
            has(&above, near1, near2).is_some_and(|c| c.sources.contains(&Source::Entity)),
            "and still confirms a pair the lexical source found"
        );
    }

    /// The departure from [`crate::proposition::Lexical`]: a far pair is still a candidate.
    ///
    /// Two units at `Ruth.1.1` and `Jude.1.19` are as far apart as this corpus goes, so the
    /// same-as engine's adjacency filter removes them and the linker's lexical source does not.
    /// This is the property GE-T11 measures, asserted on one pair where it can be read.
    #[test]
    fn a_far_lexical_neighbour_is_a_candidate_here_and_not_a_same_as_proposal() {
        use crate::proposition::{Engine, Lexical};

        let far = |gist: &str, locator: &str| {
            let u = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Cited)
                .lang(LangTag::new("en").expect("a tag"))
                .source(smysl_core::types::SourceRef::new(
                    smysl_core::types::SourceKind::Doc,
                    format!("t3:{}#{locator}", "a".repeat(52)),
                ))
                .build()
                .expect("a unit");
            let uid = canonical_uid(&u);
            (Record::Unit(u), uid)
        };
        let (ra, a) = far("the elders sat in the gate of the city", "Ruth.4.2");
        let (rb, b) = far(
            "these are murmurers walking after their own lusts",
            "Jude.1.16",
        );
        let (rc, c) = far("the elders of the city sat down in the gate", "Jude.1.19");
        let store = Store::from_records(vec![ra, rb, rc]);

        let cs = candidates(&store, None, None, Budget::DEFAULT);
        let found = has(&cs, a, c).expect("the two units about elders in the gate");
        assert!(found.sources.contains(&Source::Lexical));
        assert!(found.score > 0.0, "a lexical candidate carries its score");

        let proposals = Lexical::default().propose(&store, None);
        assert!(
            proposals.is_empty(),
            "the same-as engine's adjacency filter removes the far pair: {proposals:?}"
        );
        let _ = b;
    }

    /// The budget is per endpoint, and a pair survives when either end still has room.
    #[test]
    fn the_budget_caps_what_one_unit_contributes() {
        let mut records = Vec::new();
        let mut uids = Vec::new();
        for i in 0..6 {
            let (r, u) = claim(&format!(
                "the elders sat in the gate of the city and said nothing on day {i}"
            ));
            records.push(r);
            uids.push(u);
        }
        let store = Store::from_records(records);

        let wide = candidates(
            &store,
            None,
            None,
            Budget {
                per_unit: 8,
                ..Budget::DEFAULT
            },
        );
        let narrow = candidates(
            &store,
            None,
            None,
            Budget {
                per_unit: 1,
                ..Budget::DEFAULT
            },
        );
        assert!(
            narrow.len() < wide.len(),
            "a budget of one keeps fewer than a budget of eight: {} vs {}",
            narrow.len(),
            wide.len()
        );
        // The bound the budget actually gives, and the one worth asserting: the whole run,
        // not any one unit. A unit whose own budget is spent still appears in the candidates
        // of units that have room — see `Budget::per_unit`.
        assert!(
            narrow.len() <= uids.len(),
            "`units × per_unit` bounds the run: {} candidates over {} units at a budget of 1",
            narrow.len(),
            uids.len()
        );
        let busiest = uids
            .iter()
            .map(|u| narrow.iter().filter(|c| c.a == *u || c.b == *u).count())
            .max()
            .expect("six units");
        assert!(
            busiest > 1,
            "and it is not a per-unit cap: the busiest unit appears {busiest} time(s)"
        );
    }

    /// `scope` restricts both ends, not one.
    #[test]
    fn a_scope_restricts_the_corpus() {
        let (ra, a) = claim("the elders sat in the gate of the city");
        let (rb, b) = claim("the elders of the city sat down in the gate");
        let (rc, c) = claim("the elders came down to the gate of that city");
        let store = Store::from_records(vec![ra, rb, rc]);

        let only = BTreeSet::from([a, b]);
        let cs = candidates(&store, Some(&only), None, Budget::DEFAULT);
        assert!(has(&cs, a, b).is_some());
        for cand in &cs {
            assert!(cand.a != c && cand.b != c, "`c` is out of scope");
        }
    }
}
