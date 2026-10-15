//! The grounded summary tree (SMYSL-2.4 §3.5, TX-P7 step 3).
//!
//! §3.5 in one sentence: *levels from the structure (`chapter`, `book`, …); each summary is
//! grounded on its children with `x.text/summarises` edges; rule M caps it, applied by
//! [`crate::stage::prepare_under`] as for any batch.* Everything below is what building that
//! sentence turned out to require.
//!
//! # The tree is the reading's tree, not a tree this pass invents
//!
//! A summary of a chapter has to know which units are in that chapter, and a unit knows only
//! the locator it was drawn at. The relation between `Rut.1.5` and `Rut.1` is not a string
//! operation — it is the reading's, which is why [`crate::text::window`] reads a
//! [`smysl_text::structure::Structure`] and why this does too. A pass that split locators on
//! dots would agree with the reading for USFM and disagree with it for everything else, and
//! the disagreement would be silent.
//!
//! That is the departure from §4.2's sketch, `summarise(&self, levels, scope)`. There is no
//! way to turn `chapter` into a set of units from a scope alone, so this takes the library and
//! the expression as well, and the scope narrows what may be summarised rather than defining
//! it.
//!
//! # Rule M is the point, and it is not applied here
//!
//! A summary is `derived` and is grounded on its children. Rule M says a unit may not be
//! stronger than its weakest ground, so a chapter summary over one `speculative` verse unit is
//! `speculative` however confident the sentence sounds. This module does **not** implement
//! that: [`crate::stage::prepare_under`] does, through `monotone::apply`, exactly as it does
//! for an ingest batch — and that is the whole reason the cap can be trusted. A second
//! implementation of rule M, in the one pass whose output is most likely to overclaim, is the
//! implementation that would be wrong.
//!
//! The visible consequence is that a summary's uid is not known until it is staged: weakening
//! changes the core and therefore the identity, and `prepare_under` re-points the
//! `x.text/summarises` edges onto the new uids.
//! [`SummaryReport::capped`](crate::summarise::SummaryReport::capped) counts how often it
//! happened, because a tree where every node was lowered is a tree built over material that
//! does not support it.
//!
//! # Level by level, bottom first
//!
//! The levels are summarised in the order given, and each level after the first is grounded on
//! the level below it — the chapter summaries, not the verses again. A book summary grounded
//! on five hundred verses would be a prompt nobody can afford and a claim nothing can check;
//! grounded on four chapter summaries it is four sentences in and one sentence out, and the
//! chain of grounds reaches the verses through the tree.
//!
//! # What is not summarised
//!
//! A node with one child, because a summary of one claim is that claim with a model's wording
//! in front of it. A node with no units in scope. And a node whose children's gists do not fit
//! a turn — reported rather than truncated, since a summary of the first half of a chapter
//! that claims to be a summary of the chapter is the one failure this pass can produce that a
//! reader cannot see.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::ids::Uid;
use smysl_core::types::RelKind;
use smysl_core::{Diagnostic, KernelType, Relation, Status, UnitCore, UnitCoreBuilder};
use smysl_provider::{ProviderError, Task, Usage};
use smysl_text::library::Library;
use smysl_text::reading::Level;
use smysl_text::structure::Structure;

use crate::stage::{self, Attest, Staged};
use crate::Ingestor;

/// `x.text/summarises`, the edge a summary carries to each of its children.
pub const SUMMARISES: &str = smysl_core::schemas::text::SUMMARISES;

/// What one `summarise` run did.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct SummaryReport {
    /// Nodes summarised, per level, in the order the levels were given.
    pub levels: Vec<(String, usize)>,
    /// Model calls made, including ones that failed.
    pub turns: usize,
    /// Summaries written.
    pub summaries: usize,
    /// Summaries rule M lowered, as `prepare_under` reports them.
    pub capped: usize,
    /// Nodes skipped because they held fewer than two children in scope.
    pub thin: usize,
    /// The batch, or `None` when nothing was summarised.
    pub staged: Option<Staged>,
    pub diagnostics: Vec<Diagnostic>,
    pub usage: Usage,
    pub provider: Option<smysl_provider::ProviderId>,
    pub model: Option<String>,
    pub recipe: Option<[u8; 32]>,
    pub family: Option<[u8; 32]>,
}

/// One node's children, as the template's input: a number, a tab, a gist.
pub fn render(gists: &[&str]) -> String {
    let flat = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    gists
        .iter()
        .enumerate()
        .map(|(i, g)| format!("{}\t{}\n", i + 1, flat(g)))
        .collect()
}

/// The one sentence a turn came back with, or `None`.
///
/// The first non-empty line that is not an echo of a marker, with no further parsing: the
/// prompt asks for one sentence and a model that writes two has written a summary that is too
/// long rather than an answer that is wrong.
pub fn parse_answer(answer: &str) -> Option<String> {
    crate::prompt::strip_echo(answer)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| l.to_string())
}

/// A node of the tree this pass walks: what it is, and which units hang under it.
struct Group {
    locator: String,
    members: Vec<Uid>,
}

/// The groups at `level`, over the units in `by_locator`, using the expression's readings.
///
/// A unit is in a group when the node its own locator resolves to is a descendant of that
/// group's node. Resolved through the tree and not by prefix, for the reason in the module
/// header.
fn groups(
    structures: &[Structure],
    level: &Level,
    by_locator: &BTreeMap<String, Vec<Uid>>,
) -> Vec<Group> {
    let mut out: BTreeMap<String, Vec<Uid>> = BTreeMap::new();
    for s in structures {
        // The node each locator names, and the ancestor of that node at `level`.
        for node in s.nodes() {
            let Some(members) = by_locator.get(&node.locator.to_string()) else {
                continue;
            };
            let mut at = Some(node);
            while let Some(n) = at {
                if &n.level == level {
                    out.entry(n.locator.to_string())
                        .or_default()
                        .extend(members.iter().copied());
                    break;
                }
                at = n.parent.and_then(|p| s.node(p));
            }
        }
    }
    out.into_iter()
        .map(|(locator, mut members)| {
            members.sort();
            members.dedup();
            Group { locator, members }
        })
        .collect()
}

impl Ingestor<'_> {
    /// Summarise an expression level by level, grounded, and stage it (rules S and M).
    ///
    /// `levels` are summarised in the order given and each is grounded on the one before it.
    /// `scope` narrows which of the expression's units may be summarised; `None` is all of
    /// them.
    #[cfg(feature = "text")]
    pub fn summarise(
        &self,
        lib: &Library,
        alias: &str,
        levels: &[Level],
        scope: Option<&BTreeSet<Uid>>,
    ) -> Result<SummaryReport, ProviderError> {
        let mut report = SummaryReport::default();
        if levels.is_empty() {
            return Ok(report);
        }
        let mid = lib
            .head(alias)
            .map_err(|e| ProviderError::Config(format!("--scope {alias}: {e}")))?;
        let manifest = lib
            .store()
            .manifest(&mid)
            .ok_or_else(|| ProviderError::Config(format!("{alias} has no manifest")))?
            .clone();

        // The readings, as trees. One per part, because a part is what a reading covers.
        let mut structures = Vec::new();
        for tid in &manifest.parts {
            let Ok(reading) = lib.part_reading(&mid, &tid.tid) else {
                continue;
            };
            let Ok(mut budget) =
                smysl_text::limits::Budget::new(smysl_text::limits::Caps::DEFAULT, 0)
            else {
                continue;
            };
            if let Ok(s) = Structure::build(&reading.rows, u64::MAX, &mut budget) {
                structures.push(s);
            }
        }
        if structures.is_empty() {
            return Err(ProviderError::Config(format!(
                "{alias} has no reading to take its levels from"
            )));
        }

        // The units in play, by the locator each was drawn at.
        let store = lib.store();
        let mut by_locator: BTreeMap<String, Vec<Uid>> = BTreeMap::new();
        for (uid, unit) in store.units() {
            if !scope.is_none_or(|s| s.contains(uid)) {
                continue;
            }
            let Some(source) = unit.core.source.as_ref() else {
                continue;
            };
            if source.manifest() != Some(&mid) {
                continue;
            }
            let Some((_, locator)) = source.reference.split_once('#') else {
                continue;
            };
            by_locator
                .entry(locator.to_string())
                .or_default()
                .push(*uid);
        }
        if by_locator.is_empty() {
            return Ok(report);
        }

        let provider = self.registry.for_task(Task::ContentIngest)?;
        let caps = provider.caps();
        let template = crate::prompt::summarise_children();
        let conditions = crate::recipe::Conditions::new(template.id.clone(), template.version)
            .with_provider(provider.id().to_string(), self.model_for(&caps))
            .with_temperature(self.opts.temperature)
            .with_framing(crate::prompt::FRAMING)
            .with_schemas([
                "smysl.kernel/0.1".to_string(),
                smysl_core::schemas::text::ID.to_string(),
            ])
            .for_text(
                levels
                    .iter()
                    .map(Level::as_str)
                    .collect::<Vec<&str>>()
                    .join(","),
                None,
                "none",
                manifest.reader.clone(),
            );
        report.provider = Some(provider.id());
        report.model = Some(self.model_for(&caps).to_string());
        report.recipe = Some(conditions.recipe());
        report.family = Some(conditions.family());

        let gist_of = |uid: &Uid, written: &BTreeMap<Uid, UnitCore>| -> Option<String> {
            written
                .get(uid)
                .map(|u| u.gist.clone())
                .or_else(|| store.get(uid).map(|u| u.core.gist.clone()))
        };

        let mut units: Vec<UnitCore> = Vec::new();
        let mut relations: Vec<Relation> = Vec::new();
        let mut written: BTreeMap<Uid, UnitCore> = BTreeMap::new();
        // What the next level up is grounded on: the level below's summaries, by locator.
        let mut lower = by_locator.clone();

        for level in levels {
            let mut made = 0usize;
            let mut next: BTreeMap<String, Vec<Uid>> = BTreeMap::new();
            for group in groups(&structures, level, &lower) {
                if group.members.len() < 2 {
                    report.thin += 1;
                    continue;
                }
                let gists: Vec<String> = group
                    .members
                    .iter()
                    .filter_map(|u| gist_of(u, &written))
                    .collect();
                if gists.len() < 2 {
                    report.thin += 1;
                    continue;
                }
                let borrowed: Vec<&str> = gists.iter().map(String::as_str).collect();
                let rendered = template.render(&render(&borrowed));
                let mut request = smysl_provider::Request::new(self.model_for(&caps), rendered)
                    .with_system(&template.system)
                    .with_max_output(self.opts.output_budget(&caps));
                request.temperature = self.opts.temperature;
                report.turns += 1;

                let sentence = match provider.complete(&request) {
                    Ok(c) => {
                        report.usage.input_tokens += c.usage.input_tokens;
                        report.usage.output_tokens += c.usage.output_tokens;
                        report.usage.estimated |= c.usage.estimated;
                        report.usage.retries += c.usage.retries;
                        parse_answer(&c.text)
                    }
                    Err(e) => {
                        report.diagnostics.push(
                            Diagnostic::new(smysl_core::Code::W453)
                                .with_message(format!(
                                    "no summary for {} at {}: {e}",
                                    group.locator,
                                    level.as_str()
                                ))
                                .with_suggestion("a rerun asks again; the level below is intact"),
                        );
                        None
                    }
                };
                let Some(sentence) = sentence.filter(|s| !s.is_empty()) else {
                    continue;
                };

                // `derived` and grounded on its children, which is what makes rule M bite.
                // The status is a claim about where the sentence came from; `prepare_under`
                // is what decides whether it may keep it.
                let mut b = UnitCoreBuilder::new(KernelType::Claim, sentence, Status::Derived);
                b.grounds = group.members.iter().copied().collect();
                if let Some(lang) = Some(manifest.lang.clone()) {
                    b = b.lang(lang);
                }
                let Ok(unit) = b.build() else {
                    continue;
                };
                let uid = smysl_core::canonical_uid(&unit);
                for child in &group.members {
                    relations.push(Relation::new(
                        RelKind::Extension(SUMMARISES.to_string()),
                        uid,
                        *child,
                    ));
                }
                next.entry(group.locator.clone()).or_default().push(uid);
                written.insert(uid, unit.clone());
                units.push(unit);
                made += 1;
            }
            report.levels.push((level.as_str().to_string(), made));
            report.summaries += made;
            if next.is_empty() {
                break;
            }
            lower = next;
        }

        if units.is_empty() {
            return Ok(report);
        }

        let schemas = if store.iter().any(|r| {
            matches!(r, smysl_core::Record::SchemaDecl(d)
                if d.id.as_str() == smysl_core::schemas::text::ID)
        }) {
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
        let staged = stage::prepare_under(
            store,
            units,
            relations,
            Default::default(),
            schemas,
            &attest,
            None,
        );
        report.capped = staged
            .report
            .diagnostics
            .iter()
            .filter(|d| d.code == smysl_core::Code::W036)
            .count();
        report.staged = Some(staged);
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_render_as_one_numbered_line_each() {
        let text = render(&["the first\tclaim", "the second\nclaim"]);
        assert_eq!(text, "1\tthe first claim\n2\tthe second claim\n");
    }

    #[test]
    fn the_answer_is_the_first_sentence_that_is_not_empty() {
        assert_eq!(
            parse_answer("\n\n  The chapter tells of a return.  \nand more\n").as_deref(),
            Some("The chapter tells of a return.")
        );
        assert_eq!(parse_answer("   \n\n").as_deref(), None);
    }
}
