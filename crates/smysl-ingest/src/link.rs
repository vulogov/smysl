//! The linker: asking a model about the pairs the generator chose (SMYSL-2.4 §3.5, TX-P7
//! step 3).
//!
//! [`smysl_text::link`] decides *which* pairs are worth asking about and decides nothing else;
//! this is where a pair becomes a question, an answer and — if the answer is one of the two
//! that mean something — a staged `x.text/entails` or `x.text/contradicts` edge.
//!
//! # Why the two halves are in two crates
//!
//! The generator is pure and this is not. That split is [`crate::same_as`]'s and it is kept
//! here for a sharper reason: GE-T11 measures the generator's recall and nothing else, and a
//! measurement that had to buy model turns to produce a figure would be a measurement nobody
//! runs. `make test` runs it.
//!
//! # What is sent, and what a model may say
//!
//! One turn carries several pairs, numbered, each as two gists. The model answers one line per
//! pair with one of four words:
//!
//! | answer | what is staged |
//! |---|---|
//! | `contradicts` | one `x.text/contradicts` edge |
//! | `entails-ab` | one `x.text/entails` edge from the first to the second |
//! | `entails-ba` | one `x.text/entails` edge from the second to the first |
//! | `unrelated` | nothing |
//!
//! `unrelated` is not a throwaway. It is the answer most pairs get — a generator with a recall
//! of 0.68 at a budget of sixteen is offering a great many pairs that are merely near each
//! other — and a model that could not say it would say something else instead. Nothing is
//! staged for it, and the count is reported, because the ratio of answers to edges is the only
//! cheap read on whether the budget is buying anything.
//!
//! Entailment is **directional** and contradiction is not, which is why there are two spellings
//! of one and one of the other. A pair whose answer does not parse is counted and dropped:
//! rule I, as everywhere in this crate — a model that writes a fifth word has not invalidated
//! the four other pairs in its turn.
//!
//! # Rung, and the reason this is not `computed`
//!
//! [`crate::same_as`] attests at `computed` because an engine is arithmetic. A judgement is
//! not: it is a model's reading of two sentences, and the rung is the caller's
//! ([`crate::IngestOptions::at_rung`]), as it is for every other thing a model says in this
//! crate. The agent is the caller's too. What this module fixes is the *recipe*, which records
//! the template, the provider, the model and the candidate budget — because two runs at
//! different budgets saw different questions, and a corroboration group that mixed them would
//! be claiming two models agreed when one of them was never asked.
//!
//! # Edges the store already holds
//!
//! Skipped and counted, exactly as [`crate::same_as`] skips them and for the same reason: a
//! relation's rid is a function of its kind and its endpoints, so re-running the linker over a
//! store it has already run over would stage byte-identical records and claim to have found
//! something. A **withdrawn** edge is asked about again — a person judging that two units do
//! not contradict each other, and a model proposing it a second time, is a disagreement worth
//! seeing.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::ids::Uid;
use smysl_core::types::RelKind;
use smysl_core::{Diagnostic, Relation};
use smysl_graph::Store;
use smysl_provider::{ProviderError, Task, Usage};
use smysl_text::link::{self, Budget, Candidate};

use crate::stage::{self, Attest, Staged};
use crate::{Ingestor, Rung};

/// `x.text/entails`, staged for `entails-ab` and `entails-ba`.
pub const ENTAILS: &str = smysl_core::schemas::text::ENTAILS;
/// `x.text/contradicts`, staged for `contradicts`.
pub const CONTRADICTS: &str = smysl_core::schemas::text::CONTRADICTS;

/// How many pairs go in one turn.
///
/// Ten rather than one, because a turn has a fixed cost in system prompt and latency and the
/// pairs are independent; and ten rather than a hundred, because a model asked about a hundred
/// pairs at once answers the later ones worse and a parse failure costs the whole turn. It is
/// a field so a caller whose provider has a small context can lower it.
pub const PAIRS_PER_TURN: usize = 10;

/// What one `link` run did.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct LinkReport {
    /// Pairs the generator offered.
    pub candidates: usize,
    /// Pairs sent to a model: the candidates, less the ones the store already relates.
    pub asked: usize,
    /// Pairs the store already relates, which were not asked about again.
    pub already: usize,
    /// Model calls made, including ones that failed.
    pub turns: usize,
    /// Answers by verdict.
    pub contradicts: usize,
    /// Answers by verdict.
    pub entails: usize,
    /// Answers by verdict.
    pub unrelated: usize,
    /// Pairs whose answer did not come back, or did not parse.
    pub unanswered: usize,
    /// The batch, or `None` when nothing was found to stage.
    pub staged: Option<Staged>,
    pub diagnostics: Vec<Diagnostic>,
    pub usage: Usage,
    pub provider: Option<smysl_provider::ProviderId>,
    pub model: Option<String>,
    pub recipe: Option<[u8; 32]>,
    pub family: Option<[u8; 32]>,
}

impl LinkReport {
    /// Edges this run would add.
    pub fn new_edges(&self) -> usize {
        self.staged.as_ref().map_or(0, |s| s.relations.len())
    }
}

/// One answer about one pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The two cannot both be true.
    Contradicts,
    /// The first entails the second.
    EntailsAb,
    /// The second entails the first.
    EntailsBa,
    /// Neither, which is most of them.
    Unrelated,
}

impl Verdict {
    /// The word a model writes, or `None` for anything else.
    pub fn parse(s: &str) -> Option<Verdict> {
        match s.trim().to_ascii_lowercase().as_str() {
            "contradicts" => Some(Verdict::Contradicts),
            "entails-ab" => Some(Verdict::EntailsAb),
            "entails-ba" => Some(Verdict::EntailsBa),
            "unrelated" => Some(Verdict::Unrelated),
            _ => None,
        }
    }
}

/// The numbered pairs of one turn, as the template's input.
///
/// `n<TAB>A<TAB>gist` and `n<TAB>B<TAB>gist`, which is the shape [`crate::text::pivot`] uses
/// and for the same reason: a line-oriented answer can be matched back to its question by a
/// number, and a model that drops a line drops one pair instead of shifting every later one.
/// Tabs and newlines inside a gist are replaced by spaces, because the number is only a key
/// while the format holds.
pub fn render(pairs: &[(usize, &str, &str)]) -> String {
    let flat = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    let mut out = String::new();
    for (n, a, b) in pairs {
        out.push_str(&format!("{n}\tA\t{}\n{n}\tB\t{}\n", flat(a), flat(b)));
    }
    out
}

/// The verdicts a turn came back with, by pair number.
///
/// Anything that is not `number<TAB>word` is skipped rather than failing the turn, and a number
/// that was not asked about is skipped too — a model inventing pair 94 must not be able to
/// stage an edge between two units nobody offered it.
pub fn parse_answer(answer: &str, asked: &BTreeSet<usize>) -> BTreeMap<usize, Verdict> {
    let mut out = BTreeMap::new();
    for line in crate::prompt::strip_echo(answer).lines() {
        let Some((n, rest)) = line.split_once('\t') else {
            continue;
        };
        let Ok(n) = n.trim().parse::<usize>() else {
            continue;
        };
        if !asked.contains(&n) {
            continue;
        }
        if let Some(v) = Verdict::parse(rest) {
            out.insert(n, v);
        }
    }
    out
}

/// The relation one verdict stages, or `None` for `unrelated`.
fn edge(v: Verdict, a: Uid, b: Uid) -> Option<Relation> {
    let (kind, from, to) = match v {
        Verdict::Contradicts => (CONTRADICTS, a, b),
        Verdict::EntailsAb => (ENTAILS, a, b),
        Verdict::EntailsBa => (ENTAILS, b, a),
        Verdict::Unrelated => return None,
    };
    Some(Relation::new(
        RelKind::Extension(kind.to_string()),
        from,
        to,
    ))
}

/// Every live `entails` or `contradicts` edge the store already holds, by rid.
fn held(store: &Store) -> BTreeSet<Uid> {
    let mut out = BTreeSet::new();
    for kind in [ENTAILS, CONTRADICTS] {
        for rel in store.relations_of_kind(&RelKind::Extension(kind.to_string())) {
            if !store.is_withdrawn(rel) {
                out.insert(rel.uid());
            }
        }
    }
    out
}

/// Whether the store already declares `x.text/v1`.
fn declares_text(store: &Store) -> bool {
    store.iter().any(|r| {
        matches!(r, smysl_core::Record::SchemaDecl(d)
            if d.id.as_str() == smysl_core::schemas::text::ID)
    })
}

impl Ingestor<'_> {
    /// Ask a model about the pairs the generator chose, and stage what it says (rule S).
    ///
    /// `scope` restricts the units; `table` supplies the versification alignment where there
    /// is one. Fails only where the registry cannot route a turn at all — every failure a
    /// *model* can produce is a diagnostic, because the other pairs in the run are still
    /// answerable.
    #[cfg(feature = "text")]
    pub fn link(
        &self,
        store: &Store,
        scope: Option<&BTreeSet<Uid>>,
        table: Option<&smysl_text::index::align::Table>,
        budget: Budget,
        pairs_per_turn: usize,
    ) -> Result<LinkReport, ProviderError> {
        let mut report = LinkReport::default();
        let candidates = link::candidates(store, scope, table, budget);
        report.candidates = candidates.len();
        if candidates.is_empty() {
            return Ok(report);
        }

        let provider = self.registry.for_task(Task::ContentIngest)?;
        let caps = provider.caps();
        let template = crate::prompt::link_pairs();
        let conditions = crate::recipe::Conditions::new(template.id.clone(), template.version)
            .with_provider(provider.id().to_string(), self.model_for(&caps))
            .with_temperature(self.opts.temperature)
            .with_framing(crate::prompt::FRAMING)
            .with_schemas([
                "smysl.kernel/0.1".to_string(),
                smysl_core::schemas::text::ID.to_string(),
            ])
            .for_link(budget.per_unit, budget.neighbours, budget.hub);
        report.provider = Some(provider.id());
        report.model = Some(self.model_for(&caps).to_string());
        report.recipe = Some(conditions.recipe());
        report.family = Some(conditions.family());

        let seen = held(store);
        // The pairs worth a turn: a candidate whose edge the store already carries is not one.
        // Both kinds are checked, because "already answered" is the question, and a pair the
        // store says entails is a pair somebody has already judged.
        let mut asking: Vec<&Candidate> = Vec::new();
        for c in &candidates {
            let known = [
                Relation::new(RelKind::Extension(CONTRADICTS.to_string()), c.a, c.b),
                Relation::new(RelKind::Extension(ENTAILS.to_string()), c.a, c.b),
                Relation::new(RelKind::Extension(ENTAILS.to_string()), c.b, c.a),
            ]
            .iter()
            .any(|r| seen.contains(&r.uid()));
            if known {
                report.already += 1;
            } else {
                asking.push(c);
            }
        }
        report.asked = asking.len();
        if asking.is_empty() {
            return Ok(report);
        }

        let per_turn = pairs_per_turn.max(1);
        let mut relations: Vec<Relation> = Vec::new();
        for chunk in asking.chunks(per_turn) {
            let mut lines: Vec<(usize, &str, &str)> = Vec::new();
            let mut numbers: BTreeSet<usize> = BTreeSet::new();
            for (i, c) in chunk.iter().enumerate() {
                let (Some(ua), Some(ub)) = (store.get(&c.a), store.get(&c.b)) else {
                    continue;
                };
                lines.push((i + 1, ua.core.gist.as_str(), ub.core.gist.as_str()));
                numbers.insert(i + 1);
            }
            if lines.is_empty() {
                continue;
            }

            let rendered = template.render(&render(&lines));
            let mut request = smysl_provider::Request::new(self.model_for(&caps), rendered)
                .with_system(&template.system)
                .with_max_output(self.opts.output_budget(&caps));
            request.temperature = self.opts.temperature;
            report.turns += 1;

            let answers = match provider.complete(&request) {
                Ok(c) => {
                    report.usage.input_tokens += c.usage.input_tokens;
                    report.usage.output_tokens += c.usage.output_tokens;
                    report.usage.estimated |= c.usage.estimated;
                    report.usage.retries += c.usage.retries;
                    parse_answer(&c.text, &numbers)
                }
                Err(e) => {
                    // A turn that did not come back costs its pairs and not the run: the other
                    // chunks are still answerable, which is rule I at the granularity this
                    // pass has.
                    report.diagnostics.push(
                        Diagnostic::new(smysl_core::Code::W453)
                            .with_message(format!(
                                "the linker could not judge {} pair(s): {e}",
                                lines.len()
                            ))
                            .with_suggestion("a rerun asks about them again"),
                    );
                    BTreeMap::new()
                }
            };

            for (i, c) in chunk.iter().enumerate() {
                match answers.get(&(i + 1)) {
                    Some(Verdict::Unrelated) => report.unrelated += 1,
                    Some(v) => {
                        match v {
                            Verdict::Contradicts => report.contradicts += 1,
                            _ => report.entails += 1,
                        }
                        if let Some(r) = edge(*v, c.a, c.b) {
                            relations.push(r);
                        }
                    }
                    None => report.unanswered += 1,
                }
            }
        }

        if relations.is_empty() {
            return Ok(report);
        }

        let schemas = if declares_text(store) {
            Vec::new()
        } else {
            vec![smysl_core::schemas::text::decl()]
        };
        let attest = Attest::new(
            self.opts.agent.clone(),
            self.opts.rung,
            self.opts.now.clone(),
        )
        .at_hop(self.opts.hop)
        .with_recipe(conditions.recipe(), conditions.family());
        report.staged = Some(stage::prepare_attested(
            store,
            Vec::new(),
            relations,
            Default::default(),
            schemas,
            &attest,
        ));
        Ok(report)
    }
}

/// The rung a judgement belongs at when a caller names none.
///
/// `model`, and the name says it: the evidence for an entailment here is a model's reading of
/// two sentences and nothing else. Not `computed`, which is what [`crate::same_as`]'s engines
/// attest at, because arithmetic over records and a judgement about meaning are the two things
/// A-9's ladder exists to tell apart.
pub const DEFAULT_RUNG: Rung = Rung::Model;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_words_are_the_four_words() {
        assert_eq!(Verdict::parse("contradicts"), Some(Verdict::Contradicts));
        assert_eq!(Verdict::parse(" ENTAILS-AB "), Some(Verdict::EntailsAb));
        assert_eq!(Verdict::parse("entails-ba"), Some(Verdict::EntailsBa));
        assert_eq!(Verdict::parse("unrelated"), Some(Verdict::Unrelated));
        assert_eq!(
            Verdict::parse("maybe"),
            None,
            "a fifth word is not a verdict"
        );
        assert_eq!(Verdict::parse(""), None);
    }

    #[test]
    fn a_pair_renders_as_two_numbered_lines_with_no_tab_inside_a_gist() {
        let text = render(&[(1, "a gist\twith a tab", "another\ngist")]);
        assert_eq!(text, "1\tA\ta gist with a tab\n1\tB\tanother gist\n");
    }

    #[test]
    fn an_answer_about_a_pair_nobody_asked_about_is_dropped() {
        let asked = BTreeSet::from([1usize, 2]);
        let got = parse_answer("1\tcontradicts\n2\tunrelated\n94\tcontradicts\n", &asked);
        assert_eq!(got.len(), 2, "pair 94 was never offered: {got:?}");
        assert_eq!(got.get(&1), Some(&Verdict::Contradicts));
        assert_eq!(got.get(&2), Some(&Verdict::Unrelated));
    }

    #[test]
    fn a_line_that_does_not_parse_costs_its_pair_and_no_other() {
        let asked = BTreeSet::from([1usize, 2, 3]);
        let got = parse_answer(
            "1\tcontradicts\nI think 2 is unrelated\n3\tentails-ab\n",
            &asked,
        );
        assert_eq!(got.len(), 2);
        assert!(!got.contains_key(&2));
    }

    #[test]
    fn contradiction_is_symmetric_and_entailment_is_not() {
        let a = Uid::from_bytes([1; 32]);
        let b = Uid::from_bytes([2; 32]);
        let ab = edge(Verdict::EntailsAb, a, b).expect("an edge");
        let ba = edge(Verdict::EntailsBa, a, b).expect("an edge");
        assert_eq!((ab.from, ab.to), (a, b));
        assert_eq!((ba.from, ba.to), (b, a));
        assert_ne!(ab.uid(), ba.uid(), "two directions are two edges");
        assert!(edge(Verdict::Unrelated, a, b).is_none());
    }
}
