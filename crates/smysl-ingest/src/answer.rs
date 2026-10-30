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
//! second chance to ground somewhere else.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::surface::parse_surface;
use smysl_core::{Code, Diagnostic, Label, Record, Relation, Status, Uid, UnitCore};
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

impl Ingestor<'_> {
    /// Ask a model to answer the dossier's question from its members, and stage what it says.
    ///
    /// `store` must hold every member — the dossier's records appended — because rule M reads
    /// each ground's status there. `members[0]` is the question.
    pub fn answer(&self, store: &Store, members: &[Member]) -> Result<AnswerReport, ProviderError> {
        let mut report = AnswerReport::default();
        let provider = self.registry.for_task(Task::ContentIngest)?;
        let caps = provider.caps();
        let template = crate::prompt::answer_dossier();
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

        let staged_uids: BTreeSet<Uid> = units.iter().map(smysl_core::canonical_uid).collect();
        let labels: BTreeMap<Label, Uid> = parsed
            .labels
            .into_iter()
            .filter(|(_, u)| staged_uids.contains(u))
            .collect();
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
