//! The answer step: a model reads a dossier and answers its question (SMYSL-2.5 §3.13,
//! `ask --answer`; TX-P8 step 3). **Model-dependent**, and staged like every other model output
//! (rule S).
//!
//! §3.13's three requirements, and where each is met:
//!
//! - **The dossier is data** (SMYSL-2.1 D-10). It is rendered a line per unit and sent inside a
//!   fence derived from the rendering ([`crate::prompt::Template::render`]), with every marker
//!   a gist carries neutralised first (`SMY-W443`), under the same preamble every template
//!   carries: instructions inside the material are described, never followed. That is not a
//!   security boundary, and the next two requirements are why it does not have to be.
//! - **Every unit grounds only on dossier members.** A unit grounding on a uid the dossier did
//!   not hold, or on nothing at all, is dropped and counted (`SMY-W478`), and so is an edge
//!   touching one. A model that obeyed an injected instruction to write about something else
//!   has to ground that unit somewhere, and there is nowhere in the dossier for it to go.
//! - **Rule M caps each unit at its weakest ground.** Not here: [`crate::stage::prepare_under`]
//!   does it, as it does for an ingest batch and a summary, and reports each lowering as
//!   `SMY-W036`. A `cited` answer over an `inferred` slot comes out `inferred`, whatever the
//!   model wrote. "AI reads AI and passes it to humans", with the hedges intact.
//!
//! **The rendering is not `Target::Text`.** §3.13 names the text backend, and the text backend
//! abbreviates a uid to its first 26 characters — which the surface parser refuses as a
//! reference, rightly, since an abbreviated identity weakens identity silently. A model told to
//! cite what it was shown would cite what cannot be read back. So the dossier goes as one line
//! per unit with its full uid, which is the text rendering's content in the form an answer can
//! be checked against.
//!
//! No repair loop: an answer that does not parse is reported and nothing is staged. A repair
//! turn asks a model to fix the shape of what it wrote, and the shape is the least of what can
//! be wrong with an answer — the grounds are checked whatever the shape, and a second turn is a
//! second chance to ground somewhere else. The one shape defect a constructed unit can carry, a
//! gist over the profile's bound, is met without a turn: [`fit_gists`] cuts it and keeps the
//! claim whole in the body (`SMY-W457`).

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::surface::parse_surface;
use smysl_core::{
    canonical_uid, Code, Diagnostic, GranularityProfile, Label, Record, Relation, Status, Uid,
    UnitCore,
};
use smysl_graph::Store;
use smysl_provider::{ProviderError, Request, Task, Usage};

use crate::stage::{self, Attest, Staged};
use crate::Ingestor;

/// One line of the dossier as the model reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The role it answers under, by name: `question`, `definition`, `counter`, …
    pub role: String,
    pub uid: Uid,
    pub status: Status,
    pub gist: String,
}

/// What one answer turn did.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct AnswerReport {
    /// Records the answer proposed, counting any the surface parser had to skip.
    pub proposed: usize,
    /// Of those, dropped for grounding outside the dossier (`SMY-W478`).
    pub dropped: usize,
    /// Of the rest, gists over the profile's bound, cut to it (`SMY-W457`).
    pub shortened: usize,
    /// Of the rest, lowered by rule M (`SMY-W036`).
    pub lowered: usize,
    /// Gists a fence marker was neutralised in (`SMY-W443`).
    pub neutralised: usize,
    /// The batch, or `None` when nothing survived.
    pub staged: Option<Staged>,
    pub diagnostics: Vec<Diagnostic>,
    pub usage: Usage,
    pub provider: Option<smysl_provider::ProviderId>,
    pub model: Option<String>,
    pub recipe: Option<[u8; 32]>,
    pub family: Option<[u8; 32]>,
}

/// The dossier as the model reads it: `role`, tab, uid, tab, status, tab, gist, one per line.
///
/// A tab or a newline in a gist would be a column or a line the reader did not write, so both
/// become spaces.
pub fn render(members: &[Member]) -> String {
    let flat = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    members
        .iter()
        // `canonical`, not `Display`: a uid displays abbreviated, and the parser refuses an
        // abbreviated reference — so a model shown the short form could cite nothing.
        .map(|m| {
            format!(
                "{}\t{}\t{}\t{}\n",
                m.role,
                m.uid.canonical(),
                m.status,
                flat(&m.gist)
            )
        })
        .collect()
}

/// What an answer may keep: the units grounded wholly inside `members`, and the edges between
/// what is kept and the dossier. Everything dropped is a `SMY-W478`.
///
/// A unit's `deps` may name another unit of the same answer — an answer can build on itself —
/// and so may its grounds; both are allowed when that unit is itself kept. Decided to a fixed
/// point, because dropping one unit can strand another grounded on it.
pub fn admit(
    members: &BTreeSet<Uid>,
    units: Vec<UnitCore>,
    relations: Vec<Relation>,
) -> (Vec<UnitCore>, Vec<Relation>, Vec<Diagnostic>) {
    let mut kept: Vec<(Uid, UnitCore)> = units
        .into_iter()
        .map(|u| (smysl_core::canonical_uid(&u), u))
        .collect();
    let mut diagnostics = Vec::new();
    loop {
        let inside: BTreeSet<Uid> = members
            .iter()
            .copied()
            .chain(kept.iter().map(|(uid, _)| *uid))
            .collect();
        let before = kept.len();
        kept.retain(|(uid, u)| {
            let outside: Vec<&Uid> = u
                .grounds
                .iter()
                .chain(u.deps.iter())
                .filter(|g| !inside.contains(g))
                .collect();
            let grounded_in_dossier = u.grounds.iter().any(|g| members.contains(g))
                || u.grounds.iter().any(|g| inside.contains(g) && g != uid);
            if outside.is_empty() && grounded_in_dossier {
                return true;
            }
            let why = if u.grounds.is_empty() {
                "grounds on nothing".to_string()
            } else if outside.is_empty() {
                "grounds only on itself".to_string()
            } else {
                format!(
                    "grounds on {} outside the dossier",
                    outside
                        .iter()
                        .map(|g| g.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            diagnostics.push(
                Diagnostic::new(Code::W478)
                    .with_message(format!("dropped `{}`: it {why}", u.gist))
                    .with_suggestion(
                        "an answer unit may rest only on the dossier it was given; ask again, \
                         or widen the dossier with --scope",
                    ),
            );
            false
        });
        if kept.len() == before {
            break;
        }
    }
    let inside: BTreeSet<Uid> = members
        .iter()
        .copied()
        .chain(kept.iter().map(|(uid, _)| *uid))
        .collect();
    let mut edges = Vec::new();
    for r in relations {
        if inside.contains(&r.from) && inside.contains(&r.to) {
            edges.push(r);
        } else {
            diagnostics.push(Diagnostic::new(Code::W478).with_message(format!(
                "dropped a `{}` edge from {} to {}: an end of it is outside the dossier",
                r.kind, r.from, r.to
            )));
        }
    }
    (
        kept.into_iter().map(|(_, u)| u).collect(),
        edges,
        diagnostics,
    )
}

/// The one shape defect a constructed unit can carry, met without a model turn.
///
/// A gist over the profile's bound (`SMY-E022`) is cut to it, ending in an ellipsis, and the
/// claim it carried moves whole into the body, above whatever body the unit had — the
/// diagnostic's own suggestion, done mechanically. Nothing is invented and nothing is lost: the
/// gist is a prefix of the model's sentence, and the sentence is in the body. Identities move
/// with the text, so grounds, deps, edges and labels follow, in the topological order rule M's
/// remap uses. Each cut is reported as `SMY-W457` on the unit's new uid.
///
/// Measured before it existed (1.16): with the bound stated in the template, nine answers of
/// twelve at `low` still carried a gist two to eight tokens over, and each lost the whole answer
/// to that one gist. A model cannot count the way the estimator does; the cut is where that is
/// settled. A profile naming an estimator this build lacks cuts nothing: the bound is then
/// unevaluable, and the check says so (`SMY-W025`).
pub fn fit_gists(
    units: Vec<UnitCore>,
    relations: Vec<Relation>,
    labels: BTreeMap<Label, Uid>,
    granularity: &GranularityProfile,
) -> (
    Vec<UnitCore>,
    Vec<Relation>,
    BTreeMap<Label, Uid>,
    Vec<Diagnostic>,
) {
    fn follow(set: &BTreeSet<Uid>, remap: &BTreeMap<Uid, Uid>) -> BTreeSet<Uid> {
        set.iter()
            .map(|u| remap.get(u).copied().unwrap_or(*u))
            .collect()
    }
    let mut remap: BTreeMap<Uid, Uid> = BTreeMap::new();
    let mut diagnostics = Vec::new();
    // Placed back by index, so the answer keeps its order however the sweep visited it.
    let mut placed: Vec<Option<UnitCore>> = vec![None; units.len()];
    for i in smysl_graph::relink::topological(&units) {
        let before = canonical_uid(&units[i]);
        let mut core = units[i].clone();
        core.grounds = follow(&core.grounds, &remap);
        core.deps = follow(&core.deps, &remap);
        if let Some(cut) = granularity.shorten_gist(&core.gist) {
            let n = granularity.tokens(&core.gist).unwrap_or(0);
            let body = match &core.body {
                Some(b) => format!("{}\n\n{b}", core.gist),
                None => core.gist.clone(),
            };
            core.gist = cut;
            core.body = Some(body);
            diagnostics.push(
                Diagnostic::on(Code::W457, canonical_uid(&core)).with_message(format!(
                    "gist of {n} tokens cut to the {} the profile allows; the claim stands \
                     whole in the body",
                    granularity.l0_max
                )),
            );
        }
        let after = canonical_uid(&core);
        if after != before {
            remap.insert(before, after);
        }
        placed[i] = Some(core);
    }
    let units: Vec<UnitCore> = placed
        .into_iter()
        .map(|u| u.expect("the topological sweep visits every unit"))
        .collect();
    let relations = relations
        .into_iter()
        .map(|mut r| {
            r.from = remap.get(&r.from).copied().unwrap_or(r.from);
            r.to = remap.get(&r.to).copied().unwrap_or(r.to);
            r
        })
        .collect();
    let labels = labels
        .into_iter()
        .map(|(l, u)| (l, remap.get(&u).copied().unwrap_or(u)))
        .collect();
    (units, relations, labels, diagnostics)
}

impl Ingestor<'_> {
    /// Ask a model to answer the dossier's question from its members, and stage what it says.
    ///
    /// `store` must hold every member — the dossier's records appended — because rule M reads
    /// each ground's status there. `members[0]` is the question.
    pub fn answer(&self, store: &Store, members: &[Member]) -> Result<AnswerReport, ProviderError> {
        let mut report = AnswerReport::default();
        let provider = self.registry.for_task(Task::ContentIngest)?;
        let caps = provider.caps();
        let template = crate::prompt::answer_dossier_with(self.opts.estimator);
        let conditions = crate::recipe::Conditions::new(template.id.clone(), template.version)
            .with_provider(provider.id().to_string(), self.model_for(&caps))
            .with_temperature(self.opts.temperature)
            .with_framing(crate::prompt::FRAMING)
            .with_schemas(["smysl.kernel/0.1".to_string()]);
        report.provider = Some(provider.id());
        report.model = Some(self.model_for(&caps).to_string());
        report.recipe = Some(conditions.recipe());
        report.family = Some(conditions.family());

        // Neutralised in what is sent, never in what is stored (§3.9.2).
        let sent: Vec<Member> = members
            .iter()
            .map(|m| {
                let (gist, changed) = crate::prompt::neutralise_fences(&m.gist);
                if changed {
                    report.neutralised += 1;
                }
                Member { gist, ..m.clone() }
            })
            .collect();
        if report.neutralised > 0 {
            report
                .diagnostics
                .push(Diagnostic::new(Code::W443).with_message(format!(
                    "{} dossier line(s) carried a prompt fence marker; neutralised in the \
                     prompt only",
                    report.neutralised
                )));
        }

        let mut request = Request::new(self.model_for(&caps), template.render(&render(&sent)))
            .with_system(&template.system)
            .with_max_output(self.opts.output_budget(&caps));
        request.temperature = self.opts.temperature;
        let completion = provider.complete(&request)?;
        report.usage = completion.usage;

        let text = crate::prompt::strip_echo(&completion.text);
        let parsed = match parse_surface(text) {
            Ok(p) => p,
            Err(e) => {
                report.diagnostics.push(
                    Diagnostic::new(Code::E001)
                        .with_message(format!("the answer is not surface records: {e}")),
                );
                return Ok(report);
            }
        };
        report
            .diagnostics
            .extend(parsed.diagnostics.iter().cloned());
        let mut units = Vec::new();
        let mut relations = Vec::new();
        for r in parsed.records {
            match r {
                Record::Unit(u) => units.push(u),
                Record::Relation(rel) => relations.push(rel),
                // An answer is claims and the edges between them. Anything else a model
                // writes — a thread, a view, a schema — is not an answer to a question.
                _ => {}
            }
        }
        // A record the parser skipped was proposed and is not kept; its diagnostic says why.
        report.proposed = units.len() + parsed.recovered;
        let members_set: BTreeSet<Uid> = members.iter().map(|m| m.uid).collect();
        let (units, relations, dropped) = admit(&members_set, units, relations);
        report.dropped = dropped
            .iter()
            .filter(|d| d.message.starts_with("dropped `"))
            .count();
        report.diagnostics.extend(dropped);
        if units.is_empty() {
            return Ok(report);
        }

        let staged_uids: BTreeSet<Uid> = units.iter().map(canonical_uid).collect();
        let labels: BTreeMap<Label, Uid> = parsed
            .labels
            .into_iter()
            .filter(|(_, u)| staged_uids.contains(u))
            .collect();
        // Under the profile the check below runs under — the store's own, since `prepare_under`
        // is given none — because a bound and the count it is compared against have to come
        // from one estimator.
        let granularity = smysl_check::store_granularity(store);
        let (units, relations, labels, cut) = fit_gists(units, relations, labels, &granularity);
        report.shortened = cut.len();
        report.diagnostics.extend(cut);
        let attest = Attest::new(
            self.opts.agent.clone(),
            self.opts.rung,
            self.opts.now.clone(),
        )
        .at_hop(self.opts.hop)
        .with_recipe(conditions.recipe(), conditions.family());
        let staged =
            stage::prepare_under(store, units, relations, labels, Vec::new(), &attest, None);
        report.lowered = staged
            .report
            .diagnostics
            .iter()
            .filter(|d| d.code == Code::W036)
            .count();
        report.staged = Some(staged);
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, UnitCoreBuilder};

    fn claim(gist: &str, grounds: &[Uid]) -> UnitCore {
        // `derived` needs grounds to be built at all; an ungrounded answer is `speculative`.
        let status = if grounds.is_empty() {
            Status::Speculative
        } else {
            Status::Derived
        };
        let mut b = UnitCoreBuilder::new(KernelType::Claim, gist, status);
        b.grounds = grounds.iter().copied().collect();
        b.build().unwrap()
    }

    #[test]
    fn the_example_record_parses_as_one_grounded_claim() {
        let out = parse_surface(crate::prompt::ANSWER_EXAMPLE).expect("parses");
        let units: Vec<&UnitCore> = out
            .records
            .iter()
            .filter_map(|r| match r {
                Record::Unit(u) => Some(u),
                _ => None,
            })
            .collect();
        assert_eq!(units.len(), 1);
        assert_eq!(units[0].grounds.len(), 1);
        assert!(crate::prompt::answer_dossier()
            .system
            .contains(crate::prompt::ANSWER_EXAMPLE));
    }

    #[test]
    fn a_unit_grounded_outside_or_on_nothing_is_dropped_and_what_rests_on_it_follows() {
        let inside = Uid::from_bytes([1; 32]);
        let outside = Uid::from_bytes([2; 32]);
        let members = BTreeSet::from([inside]);
        let good = claim("rests on the dossier", &[inside]);
        let bad = claim("rests elsewhere", &[outside]);
        let none = claim("rests on nothing", &[]);
        let stranded = claim("rests on the bad one", &[smysl_core::canonical_uid(&bad)]);
        let built_on = claim("rests on the good one", &[smysl_core::canonical_uid(&good)]);
        let (kept, _, diags) = admit(
            &members,
            vec![good.clone(), bad, none, stranded, built_on.clone()],
            Vec::new(),
        );
        assert_eq!(kept, vec![good, built_on]);
        assert_eq!(diags.len(), 3);
        assert!(diags.iter().all(|d| d.code == Code::W478));
    }

    #[test]
    fn an_over_cap_gist_is_cut_to_the_bound_and_the_claim_moves_whole_into_the_body() {
        let inside = Uid::from_bytes([1; 32]);
        let long = "Naomi was bitter because the hand of the Lord had gone out against her, \
                    taking her husband and both her sons in Moab and leaving her with nothing";
        assert!(
            long.len() > 120,
            "the fixture must be over the default bound"
        );
        let fine = claim("Naomi left Moab for Bethlehem", &[inside]);
        let over = claim(long, &[inside]);
        let over_uid = canonical_uid(&over);
        let rests = claim("so she returned empty", &[over_uid]);
        let rests_uid = canonical_uid(&rests);
        let edge = Relation::new(smysl_core::RelKind::Elaborates, rests_uid, over_uid);
        let labels = BTreeMap::from([(Label::new("a/why").unwrap(), over_uid)]);
        let g = GranularityProfile::default();

        let (units, relations, labels, diags) =
            fit_gists(vec![fine.clone(), over, rests], vec![edge], labels, &g);

        assert_eq!(units[0], fine, "a gist within the bound is left as written");
        let cut = &units[1];
        assert!(g.gist_within_bound(g.tokens(&cut.gist).unwrap()));
        assert!(cut.gist.ends_with('\u{2026}'));
        assert!(long.starts_with(cut.gist.trim_end_matches('\u{2026}')));
        assert_eq!(
            cut.body.as_deref(),
            Some(long),
            "the claim stands whole in the body"
        );
        assert_eq!(
            cut.grounds,
            BTreeSet::from([inside]),
            "the grounds are kept"
        );
        let cut_uid = canonical_uid(cut);
        assert_ne!(cut_uid, over_uid);
        assert_eq!(
            units[2].grounds,
            BTreeSet::from([cut_uid]),
            "what rests on it follows its new identity"
        );
        assert_eq!(relations[0].to, cut_uid);
        assert_eq!(relations[0].from, canonical_uid(&units[2]));
        assert_eq!(labels.values().copied().collect::<Vec<_>>(), vec![cut_uid]);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, Code::W457);
    }

    #[test]
    fn an_over_cap_gist_with_a_body_keeps_the_body_below_the_claim() {
        let inside = Uid::from_bytes([1; 32]);
        let long = "a".repeat(60) + " " + &"b".repeat(60) + " " + &"c".repeat(20);
        let mut b = UnitCoreBuilder::new(KernelType::Claim, &long, Status::Derived).body("detail");
        b.grounds = BTreeSet::from([inside]);
        let (units, _, _, diags) = fit_gists(
            vec![b.build().unwrap()],
            Vec::new(),
            BTreeMap::new(),
            &GranularityProfile::default(),
        );
        assert_eq!(
            units[0].body.as_deref(),
            Some(format!("{long}\n\ndetail").as_str())
        );
        assert_eq!(units[0].gist, format!("{}\u{2026}", "a".repeat(60)));
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn the_rendering_is_a_line_per_member_with_the_full_uid() {
        let uid = Uid::from_bytes([7; 32]);
        let text = render(&[Member {
            role: "question".into(),
            uid,
            status: Status::Cited,
            gist: "what\tis\nit".into(),
        }]);
        assert_eq!(
            text,
            format!("question\t{}\tcited\twhat is it\n", uid.canonical())
        );
    }
}
