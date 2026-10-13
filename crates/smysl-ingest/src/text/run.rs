//! `Ingestor::ingest_text`: one expression, window by window (§3.4, §4.2, §4.3.4).
//!
//! Everything else in this module is a function over data. This is the part with a provider, a
//! library and a journal, and the order it does things in is the whole of what it has to get
//! right:
//!
//! 1. **window** the parts of the manifest at the chosen level;
//! 2. **skip** what the journal says is done under this exact recipe, unless `--reextract`;
//! 3. **digest** what has already been extracted from this expression, under the recipe's budget;
//! 4. **call** the model — twice by default (§3.4), the window fenced and the digest fenced
//!    separately;
//! 5. **convert** under a scope, so a reference outside the batch and the digest is `SMY-E442`
//!    and a tool-owned field is dropped with `SMY-W444`;
//! 6. **attach** a span to every quote the window supports, writing the copy set, the holder and
//!    the mode from structure;
//! 7. **stage** the batch and commit it;
//! 8. **journal** the window — *after* the commit, which is what makes a kill lose at most the
//!    batch in flight.
//!
//! Steps 7 and 8 are in that order and not the other, and the ordering is the resume property.
//! A journal entry written first would record a window as done that no store holds.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::{Label, Mid, Relation, Uid, UnitCore};
use smysl_provider::{ProviderError, Task, Usage};
use smysl_text::limits::Caps;
use smysl_text::reading::Level;
use smysl_text::{Library, Tid};

use super::attach::{self, Opts};
use super::journal::{Entry, Journal, Key, Verdicts};
use super::window::{self, Pane};
use crate::json_ast::Scope;
use crate::{recipe, stage, Attest, IngestPath, Ingestor};

/// What to ingest.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Target {
    /// The head manifest of an alias — what `ingest --text <alias>` means.
    Alias(String),
    /// One manifest by id, for a corpus with a fork or for re-running an older cataloguing.
    Manifest(Mid),
}

/// One text ingest.
#[derive(Debug)]
#[non_exhaustive]
pub struct TextIngest<'l> {
    /// Mutable because the batches are committed as they are staged: the journal entry that
    /// makes a window resumable is written after the commit, so the commit cannot be the
    /// caller's to do later.
    pub lib: &'l mut Library,
    pub target: Target,
    /// The structure level windows are filled with — `verse`, `message`, `paragraph`.
    pub window_level: Level,
    /// Ignore the journal for this expression (`--reextract`).
    pub reextract: bool,
    /// `D`, the ledger digest's budget in tokens. Zero sends no digest.
    pub digest_budget: u32,
    /// The reader resource caps the structure is built under.
    pub caps: Caps,
}

impl<'l> TextIngest<'l> {
    pub fn new(lib: &'l mut Library, target: Target, window_level: Level) -> TextIngest<'l> {
        TextIngest {
            lib,
            target,
            window_level,
            reextract: false,
            digest_budget: 512,
            caps: Caps::default(),
        }
    }

    pub fn reextracting(mut self) -> TextIngest<'l> {
        self.reextract = true;
        self
    }

    pub fn with_digest_budget(mut self, budget: u32) -> TextIngest<'l> {
        self.digest_budget = budget;
        self
    }
}

/// What one text ingest did.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct TextReport {
    pub alias: String,
    pub mid: Option<Mid>,
    /// The language of the expression, as its manifest records it. Here because every figure
    /// below is reported *per language* by the phase exit (§6 TX-P5 step 5) and by GE-T2, and a
    /// report that did not carry one would have to be grouped by the caller remembering.
    pub lang: String,
    /// The path every call of this run took (D-9), or `None` for a run that made none.
    ///
    /// Reported because it is chosen rather than configured, and because §3.9.2 does not treat
    /// the two as equivalent: a figure compared across two corpora that took different paths is
    /// a figure about two pipelines. That is not hypothetical — see `choose_tokens`.
    pub path: Option<crate::IngestPath>,
    /// Windows the run was responsible for.
    pub windows: usize,
    /// Windows the journal said were already done under this recipe.
    pub skipped: usize,
    pub calls: usize,
    pub passes: usize,
    /// Units committed, after rule M and content addressing.
    pub units: usize,
    /// `x.text/entity` units the attributions added (step 3b).
    pub entities: usize,
    pub spanned: usize,
    pub spanless: usize,
    pub degraded: usize,
    pub repaired: usize,
    /// Repair turns, over the whole run: `calls - windows * passes` where nothing was skipped.
    ///
    /// Not [`smysl_provider::Usage::retries`], which counts a transport retrying one call. This
    /// is the repair loop — a call made because the answer before it did not hold up — and it is
    /// §3.7's `r`, the term the cost model multiplies a window's base cost by.
    pub retries: usize,
    /// Units the pivot pass wrote, and source units it did not cover (`SMY-W453`).
    ///
    /// Both zero under `--lang-policy source`, which is the default: there is no pass to run.
    /// Counted as the pass produced them rather than as the store holds them, because what a
    /// reader wants from these two is the pass's coverage — the staged total is `units`.
    pub pivoted: usize,
    pub pivot_missing: usize,
    /// Calls the pivot pass made: one per window that had anything to translate.
    ///
    /// Separate from the repair loop and separate from the passes, because it is neither. See
    /// [`TextReport::retry_rate`].
    pub pivot_calls: usize,
    /// What the passes agreed about, summed over the windows of this run (§3.4).
    ///
    /// This is GE-T2's figure under the consensus policy: `identical-span` agreement, computed
    /// rather than judged, which §5.5 asks to be reported per language — hence [`TextReport::lang`].
    pub agreement: crate::Agreement,
    pub verdicts: Verdicts,
    pub usage: Usage,
    pub recipe: Option<[u8; 32]>,
    pub family: Option<[u8; 32]>,
    pub diagnostics: Vec<Diagnostic>,
}

impl TextReport {
    /// The share of attached units out of those that attributed a quote at all.
    ///
    /// `None` when nothing attributed one, for [`crate::Agreement::rate`]'s reason: an empty
    /// comparison is not perfect coverage, and the phase exit (§6 TX-P5 step 5) reads this
    /// figure against 95%.
    pub fn span_coverage(&self) -> Option<f64> {
        let total = self.spanned + self.spanless;
        (total > 0).then(|| self.spanned as f64 / total as f64)
    }

    /// `identical-span` agreement between the passes, or `None` where nothing was comparable.
    ///
    /// GE-T2's stability figure (§5.5). `None` for a single-pass run, and for a run whose units
    /// all came out spanless, for [`crate::Agreement::rate`]'s reason.
    pub fn stability(&self) -> Option<f64> {
        self.agreement.rate()
    }

    /// §3.7's retry term: repair turns per first attempt, or `None` where nothing was attempted.
    ///
    /// The cost of a window is `base(tokens) * (1 + r)`, so the denominator is the calls that
    /// were *not* repairs — one per pass — and not the total.
    ///
    /// The pivot pass is out of both terms. It is one call per window with no repair loop of its
    /// own, so counting it as a first attempt would make the same corpus report a lower `r` for
    /// having been pivoted — a figure about the policy rather than about the model. Its tokens
    /// are in `usage` and in the journal, where a cost report reads them.
    pub fn retry_rate(&self) -> Option<f64> {
        let attempts = self
            .calls
            .checked_sub(self.retries)?
            .checked_sub(self.pivot_calls)?;
        (attempts > 0).then(|| self.retries as f64 / attempts as f64)
    }
}

#[cfg(feature = "model")]
impl Ingestor<'_> {
    /// Ingest one expression of a library, window by window (§3.4).
    ///
    /// Fails only where [`Ingestor::ingest`] does — a provider the registry could not route
    /// around, or a configuration mistake caught before egress — plus the one thing a library
    /// can refuse: an alias or manifest that is not there. Everything a model can get wrong is
    /// recoverable, which is rule I, and is why a window's failure is a diagnostic rather than a
    /// return.
    pub fn ingest_text(&self, t: &mut TextIngest<'_>) -> Result<TextReport, ProviderError> {
        // Before egress, beside the other configuration mistakes: two passes above temperature
        // 0 buy no agreement (MS-8), and a prompt override that collides with a reserved id is
        // not a thing to discover per window.
        let requested = self
            .opts
            .requested_path()
            .map_err(|e| ProviderError::Config(format!("prompt override {e}")))?;
        self.opts
            .passes
            .admits(self.opts.temperature)
            .map_err(ProviderError::Config)?;
        let profile = self
            .opts
            .granularity_profile()
            .map_err(ProviderError::Config)?;

        let (alias, mid) = resolve(t).map_err(ProviderError::Config)?;
        let manifest =
            t.lib.store().manifest(&mid).cloned().ok_or_else(|| {
                ProviderError::Config(format!("no manifest `{}`", mid.canonical()))
            })?;

        let provider = self.registry.for_task(Task::ContentIngest)?;
        let caps = provider.caps();
        let budget =
            crate::chunk::Window::for_context(caps.context_window, self.opts.output_budget(&caps))
                .budget;

        let mut report = TextReport {
            alias: alias.clone(),
            mid: Some(mid),
            lang: manifest.lang.as_str().to_string(),
            passes: self.opts.passes.count(),
            ..TextReport::default()
        };

        // The parts of this manifest, each with its reading and its structure. Read up front
        // because a window may cross parts and the panes have to outlive the windowing.
        let mut panes: Vec<PaneData> = Vec::new();
        for entry in &manifest.parts {
            match pane_of(t, &mid, &entry.tid) {
                Ok(p) => panes.push(p),
                Err(d) => report.diagnostics.push(d),
            }
        }
        let marks: Vec<Vec<std::ops::Range<u64>>> = panes
            .iter()
            .map(|p| super::attribution::quoted_spans(&p.text))
            .collect();
        let views: Vec<Pane<'_>> = panes
            .iter()
            .zip(marks.iter())
            .map(|(p, m)| {
                Pane::new(p.tid, &p.text, &p.structure, &p.rows).with_reported(m.as_slice())
            })
            .collect();

        let segmenter = smysl_text::Segmenter::new(&manifest.lang);
        let (windows, mut notes) = window::windows(
            &views,
            &t.window_level,
            budget,
            self.opts.estimator,
            &segmenter,
        );
        report.diagnostics.append(&mut notes);
        report.windows = windows.len();

        // The path is chosen from the **largest window**, not from the size of the expression.
        // `path::choose` reads its size argument as the output a call is expected to produce,
        // and for a document that is the document; for a text ingest it is one window, which is
        // bounded by the budget above. Passing the expression's length would send every Bible
        // down the surface path on the strength of its total size, when each call sees a few
        // verses (D-9, §3.9.2's "json-ast path preferred").
        //
        // In **tokens** and not bytes, which is `choose_tokens` rather than `choose`: a window
        // is filled to a token budget, and asking the byte question of it makes the path depend
        // on the script. Two bytes per Cyrillic character sent a Russian expression down the
        // surface path where the same passage in English went down the json-ast one — found by
        // measuring GE-T2 in two languages (§6 TX-P5 step 5), and the figure would otherwise
        // have been about two pipelines.
        let widest = windows
            .iter()
            .map(|w| self.opts.estimator.count(&w.text))
            .max()
            .unwrap_or(0);
        let path = crate::path::choose_tokens(&caps, Task::ContentIngest, widest, requested).path;
        report.path = Some(path);

        // The recipe, for the journal key and for every attestation of this run. One per run and
        // not one per window: that is what lets two extractions of one expression corroborate
        // each other (§3.5).
        let template = self.text_template(path, t.digest_budget > 0);
        let digest_policy =
            (t.digest_budget > 0).then(|| (t.digest_budget, super::digest::RANKING.to_string()));
        let conditions = recipe::Conditions::new(template.id.clone(), template.version)
            .with_provider(provider.id().to_string(), self.model_for(&caps))
            .with_granularity(&self.opts.granularity)
            .with_temperature(self.opts.temperature)
            .with_normaliser(self.opts.normaliser)
            .with_lang_policy(self.opts.lang_policy)
            .with_pivot_lang(self.opts.pivot())
            .with_estimator(self.opts.estimator)
            .with_passes(self.opts.passes)
            .with_framing(crate::prompt::FRAMING)
            .with_schemas([
                "smysl.kernel/0.1".to_string(),
                smysl_core::schemas::text::ID.to_string(),
            ])
            .with_path(path)
            .for_text(
                t.window_level.as_str(),
                digest_policy,
                segmenter.id(),
                reader_of(&manifest),
            );
        report.recipe = Some(conditions.recipe());
        report.family = Some(conditions.family());
        // The attestation, from the options as `Ingestor::ingest` builds it: the agent, the
        // rung, the clock and the hop are the caller's, and the recipe is this run's.
        let attest = Attest::new(
            self.opts.agent.clone(),
            self.opts.rung,
            self.opts.now.clone(),
        )
        .at_hop(self.opts.hop)
        .with_recipe(conditions.recipe(), conditions.family());

        let mut journal = Journal::open(t.lib.root());
        if t.reextract {
            journal.forget(&alias);
        }

        // What the digest may draw on: what the store already holds for this expression, grown
        // by each window as it is committed. Incremental rather than rescanned, because a scan
        // per window is a scan per window.
        let mut scope: BTreeSet<Uid> = extracted(t.lib, &manifest.parts);
        // What this run put in the store, as sets: see the counting comment below.
        let mut staged_units: BTreeSet<Uid> = BTreeSet::new();
        let mut staged_entities: BTreeSet<Uid> = BTreeSet::new();

        for w in &windows {
            let Some(owned) = owned_range(w) else {
                continue;
            };
            let key = Key {
                alias: alias.clone(),
                tid: owned.0,
                owned: (owned.1, owned.2),
                recipe: conditions.recipe(),
            };
            if journal.contains(&key) {
                report.skipped += 1;
                continue;
            }

            // What this window costs is a difference, because `usage` and `calls` are the run's
            // and a journal entry is one window's. Step 4 recorded the running total, which
            // summed over a corpus gives a figure that counts the first window once per window.
            let before = (
                report.usage.input_tokens,
                report.usage.output_tokens,
                report.calls,
            );

            let digest = super::digest::digest(t.lib.store(), &scope, t.digest_budget);
            let scope_for_answer = if digest.is_empty() {
                Scope::for_text(BTreeMap::new(), BTreeSet::new())
            } else {
                Scope::for_text(digest.labels.clone(), digest.allowed.clone())
            };

            // D-10: a window carrying one of this tool's markers is neutralised in the prompt
            // and nowhere else, so the bytes a span names are still the part's own.
            // The window's own source, supplied as the caller's (`SourcePolicy::Override`) and
            // **without a span**. Two rules meet here. A `cited` unit must have a source or the
            // builder refuses it, and a text answer's own `source` is dropped (`SMY-W444`) — so
            // if the tool supplied none, every `cited` unit in a text answer would be refused
            // for a field it was forbidden to write. And the source carries no span, because
            // §3.4 says a unit whose quote the window does not support is staged with *no span*:
            // a window-wide span would be true and useless, and it would make the phase exit's
            // coverage figure meaningless.
            let window_source = (
                smysl_core::SourceRef::new(smysl_core::SourceKind::Doc, owned.0.canonical()),
                smysl_core::SourcePolicy::Override,
            );

            let (sent, neutralised) = crate::prompt::neutralise_fences(&w.rendered());
            if neutralised {
                report.diagnostics.push(
                    Diagnostic::new(Code::W443)
                        .with_message(format!(
                            "a node of `{}` contains a prompt marker; it was neutralised in the \
                             prompt only, and the stored text is unchanged",
                            owned.0.canonical()
                        ))
                        .with_suggestion("check that the passage is the text's own"),
                );
            }

            // One list per pass, kept apart until the agreement is computed: two passes that
            // found one claim produce one uid, so a flattened batch cannot answer GE-T2's
            // question (§3.4, step 3a).
            let mut per_pass: Vec<Vec<UnitCore>> = Vec::new();
            let mut entities: Vec<UnitCore> = Vec::new();
            let mut relations: Vec<Relation> = Vec::new();
            let mut labels: BTreeMap<Label, Uid> = BTreeMap::new();
            let mut verdicts = Verdicts::default();

            for _pass in 0..self.opts.passes.count() {
                let out = self.one_call(
                    provider,
                    path,
                    &template,
                    &sent,
                    &w.text,
                    // Rendered whenever the *template* has a digest section, even for the
                    // first window where the digest is empty: the template is one per run
                    // because the recipe is, so a window that skipped the substitution would
                    // send the literal `<<<SMYSL-DIGEST>>>` and an unfilled `{digest}`. An
                    // empty digest inside its fence says "nothing has been extracted yet",
                    // which is both true and what the first window of every expression means.
                    (t.digest_budget > 0).then_some(digest.text.as_str()),
                    &scope_for_answer,
                    Some(&window_source),
                    &mut report.usage,
                );
                report.calls += out.calls;
                report.degraded += out.degraded;
                report.repaired += out.repaired;
                report.diagnostics.extend(out.diagnostics);
                relations.extend(out.relations);
                for (label, uid) in out.labels {
                    labels.entry(label).or_insert(uid);
                }

                // The span, the copy set, the holder and the mode — all from the window and the
                // reading, none from the answer.
                let attached = attach::attach(
                    out.units,
                    w,
                    &Opts {
                        manifest: &manifest,
                        normaliser: self.opts.normaliser,
                        lang: Some(&manifest.lang),
                    },
                );
                report.spanned += attached.spanned;
                report.spanless += attached.spanless;
                report.diagnostics.extend(attached.diagnostics);
                for a in &attached.attachments {
                    verdicts.count(a.support);
                }
                per_pass.push(attached.units);
                entities.extend(attached.entities);
            }

            // GE-T2 under the consensus policy, this window's share of it. Summed rather than
            // averaged: a window with two comparable places weighs twice a window with one, and
            // a mean over windows would let a window that extracted a single claim outvote a
            // chapter.
            let window_agreement = crate::consensus::agreement(&per_pass);
            report.agreement.passes = window_agreement.passes;
            report.agreement.comparable += window_agreement.comparable;
            report.agreement.agreed += window_agreement.agreed;
            report.agreement.spanless += window_agreement.spanless;
            let units: Vec<UnitCore> = per_pass.into_iter().flatten().collect();

            report.verdicts.present += verdicts.present;
            report.verdicts.loose += verdicts.loose;
            report.verdicts.absent += verdicts.absent;

            let mut batch: Vec<UnitCore> = Vec::with_capacity(units.len() + entities.len());
            batch.extend(entities.iter().cloned());
            batch.extend(units);
            let mut seen = BTreeSet::new();
            batch.retain(|u| seen.insert(smysl_core::canonical_uid(u)));

            // The pivot pass (§4.3.4), in this window and in this batch. One commit holds the
            // units and their translations, which is what keeps the resume property true of the
            // policy as well: a kill between the two commits would otherwise leave a window
            // journalled as done with half of what it owes in the store.
            let mut window_pivot_calls = 0usize;
            if let Some(lang) = self.opts.pivot() {
                let (pivoted, calls) = self.pivot_window(provider, lang, &batch, &mut report.usage);
                window_pivot_calls = calls;
                report.calls += calls;
                report.pivot_calls += calls;
                report.pivoted += pivoted.units.len();
                report.pivot_missing += pivoted.missing.len();
                report.diagnostics.extend(pivoted.diagnostics);
                relations.extend(pivoted.relations);
                for u in pivoted.units {
                    if seen.insert(smysl_core::canonical_uid(&u)) {
                        batch.push(u);
                    }
                }
            }

            // `x.text/v1`, declared with the first batch that uses it (A-11, step 2).
            //
            // Found by the pivot pass, and not caused by it: an `x.text/translates` edge in a
            // store with no declaration is `SMY-W013` on every check of it afterwards — "a
            // warning that was correct and unactionable", which is what `smysl-check`'s own
            // test calls the state this repository was in before the declaration shipped. A
            // text ingest has been writing `x.text/entity` units into undeclaring libraries all
            // along; the edge is simply the first record whose *kind* the check looks up.
            //
            // Staged with the batch rather than appended first, because appending outside
            // staging is what rule S forbids, and once rather than per window, because the
            // store the next window sees already holds it.
            let schemas = if uses_text_schema(&batch, &relations) && !declares_text(t.lib.store()) {
                vec![smysl_core::schemas::text::decl()]
            } else {
                Vec::new()
            };
            let staged = stage::prepare_under(
                t.lib.store(),
                batch,
                relations,
                labels,
                schemas,
                &attest,
                Some(profile.clone()),
            );
            report.diagnostics.extend(staged.report.iter().cloned());
            let records = staged.records();
            // Counted as a **set** over the whole run, not summed per window. The same claim
            // extracted in two windows — which the one-node overlap makes ordinary — is one
            // record in the store, and a sum would report it as two: a corpus of 400 verses
            // would then claim more units than it holds, and the phase exit reads these
            // figures.
            for u in &staged.units {
                let uid = smysl_core::canonical_uid(u);
                if u.schema.as_str() == smysl_core::schemas::text::ENTITY {
                    staged_entities.insert(uid);
                } else {
                    staged_units.insert(uid);
                }
            }

            // Commit, then journal. A window recorded as done that no store holds is the one
            // failure this ordering rules out.
            if let Err(e) = t.lib.append_records(&records) {
                report.diagnostics.push(
                    Diagnostic::new(Code::E001)
                        .with_message(format!("the batch could not be committed: {e}")),
                );
                continue;
            }
            scope.extend(staged.units.iter().map(smysl_core::canonical_uid));

            // This window's calls, and of them the repair turns: one first attempt per pass,
            // everything beyond that is the repair loop — which is the 73% S0 measured and the
            // `r` of §3.7's cost model. The pivot pass's call is subtracted rather than counted
            // as a repair: it is a different request, and a window that was pivoted would
            // otherwise report a repair turn it never made.
            let window_calls = report.calls.saturating_sub(before.2);
            let retries = window_calls
                .saturating_sub(self.opts.passes.count())
                .saturating_sub(window_pivot_calls);
            report.retries += retries;
            let entry = Entry {
                key,
                batch: batch_hash(&staged.units),
                digest: digest.hash,
                input_tokens: report.usage.input_tokens.saturating_sub(before.0),
                output_tokens: report.usage.output_tokens.saturating_sub(before.1),
                lang: report.lang.clone(),
                calls: window_calls as u32,
                retries: retries as u32,
                pivots: window_pivot_calls as u32,
                verdicts,
            };
            if let Err(e) = journal.record(entry) {
                report.diagnostics.push(
                    Diagnostic::new(Code::W304)
                        .with_message(format!(
                            "the window was staged and the journal entry was not written: {e}"
                        ))
                        .with_suggestion("a rerun extracts this window again"),
                );
            }
        }

        report.units = staged_units.len();
        report.entities = staged_entities.len();
        Ok(report)
    }

    /// One pivot call for one window's batch (§4.3.4, `crate::text::pivot`).
    ///
    /// Returns what the pass produced and the calls it cost — one, or none where the window had
    /// nothing to translate. There is **no repair loop** here, and the asymmetry with extraction
    /// is deliberate: a repair turn exists to fix an answer that failed a check this crate can
    /// run, and the only check available to a translation is whether a line came back. Asking
    /// again for a line a model chose not to write doubles the cost of every pivoted window to
    /// buy a line that would be unverifiable if it arrived. A missing line is reported
    /// (`SMY-W453`) and a rerun under `--reextract` is the operator's to choose.
    ///
    /// A provider failure is a diagnostic and not a return, like every other failure in this
    /// run: the extraction is already in the batch, and losing it because a translation could
    /// not be fetched would be a worse outcome than a window with no translations in it.
    fn pivot_window(
        &self,
        provider: &dyn smysl_provider::Provider,
        lang: crate::PivotLang,
        batch: &[UnitCore],
        usage: &mut Usage,
    ) -> (super::pivot::Pivoted, usize) {
        let idx = super::pivot::translatable(batch);
        let sources: Vec<&UnitCore> = idx.iter().map(|&i| &batch[i]).collect();
        if sources.is_empty() {
            return (super::pivot::Pivoted::default(), 0);
        }

        let template = crate::prompt::pivot_gists(lang.as_str());
        let rendered = template.render(&super::pivot::render(&sources));
        let caps = provider.caps();
        let mut request = smysl_provider::Request::new(self.model_for(&caps), rendered)
            .with_system(&template.system)
            .with_max_output(self.opts.output_budget(&caps));
        request.temperature = self.opts.temperature;

        match provider.complete(&request) {
            Ok(c) => {
                usage.input_tokens += c.usage.input_tokens;
                usage.output_tokens += c.usage.output_tokens;
                usage.estimated |= c.usage.estimated;
                usage.retries += c.usage.retries;
                let answer = super::pivot::parse_answer(&c.text, sources.len());
                (super::pivot::units(&sources, &answer, lang.as_str()), 1)
            }
            Err(e) => {
                // Counted as a call on the same rule as `one_call`'s: an error the endpoint
                // returned is a call made, and usually billed.
                let billed = !matches!(
                    e,
                    ProviderError::Unreachable
                        | ProviderError::OfflineViolation
                        | ProviderError::Config(_)
                        | ProviderError::StructuredUnsupported
                );
                let mut out = super::pivot::Pivoted::default();
                out.missing
                    .extend(sources.iter().map(|u| smysl_core::canonical_uid(u)));
                out.diagnostics.push(
                    Diagnostic::new(Code::W453)
                        .with_message(format!(
                            "the pivot pass could not be run for {} unit(s): {e}",
                            sources.len()
                        ))
                        .with_suggestion(
                            "they are staged in the source language only; a rerun asks again",
                        ),
                );
                (out, usize::from(billed))
            }
        }
    }

    /// The text template for this path, with a caller's override applied as elsewhere.
    fn text_template(&self, path: IngestPath, digest: bool) -> crate::prompt::Template {
        let base = match path {
            IngestPath::JsonAst => crate::prompt::ingest_text_json(self.opts.estimator, digest),
            IngestPath::Surface => crate::prompt::ingest_text_surface(self.opts.estimator, digest),
        };
        match self.opts.prompt.as_ref() {
            Some(o) => o.apply(base),
            None => base,
        }
    }
}

/// Whether a batch holds anything the `x.text/v1` declaration is about.
fn uses_text_schema(batch: &[UnitCore], relations: &[Relation]) -> bool {
    const PREFIX: &str = "x.text/";
    batch.iter().any(|u| u.schema.as_str().starts_with(PREFIX))
        || relations
            .iter()
            .any(|r| r.kind.as_str().starts_with(PREFIX))
}

/// Whether the store already carries that declaration, at any revision.
fn declares_text(store: &smysl_graph::Store) -> bool {
    store.iter().any(|r| {
        matches!(r, smysl_core::Record::SchemaDecl(d)
            if d.id.as_str() == smysl_core::schemas::text::ID)
    })
}

/// A part, held so the windowing can borrow it.
struct PaneData {
    tid: Tid,
    text: String,
    rows: Vec<smysl_text::reading::Segment>,
    structure: smysl_text::structure::Structure,
}

fn pane_of(t: &TextIngest<'_>, mid: &Mid, tid: &Tid) -> Result<PaneData, Diagnostic> {
    let part = t.lib.part(tid).map_err(|e| {
        Diagnostic::new(Code::E001).with_message(format!("part {}: {e}", tid.canonical()))
    })?;
    let reading = t.lib.part_reading(mid, tid).map_err(|e| {
        Diagnostic::new(Code::E001).with_message(format!("reading of {}: {e}", tid.canonical()))
    })?;
    let structure = t.lib.structure(mid, tid, &t.caps).map_err(|e| {
        Diagnostic::new(Code::E001).with_message(format!("structure of {}: {e}", tid.canonical()))
    })?;
    let text = String::from_utf8(part.text).map_err(|_| {
        Diagnostic::new(Code::E001).with_message(format!("part {} is not UTF-8", tid.canonical()))
    })?;
    Ok(PaneData {
        tid: *tid,
        text,
        rows: reading.rows,
        structure,
    })
}

/// The alias and manifest a target names.
fn resolve(t: &TextIngest<'_>) -> Result<(String, Mid), String> {
    match &t.target {
        Target::Alias(alias) => {
            let mid = t.lib.head(alias).map_err(|e| format!("`{alias}`: {e}"))?;
            Ok((alias.clone(), mid))
        }
        Target::Manifest(mid) => {
            let manifest = t
                .lib
                .store()
                .manifest(mid)
                .ok_or_else(|| format!("no manifest `{}`", mid.canonical()))?;
            Ok((manifest.alias.clone(), *mid))
        }
    }
}

/// The part and byte range a window **owns**, which is what the journal keys on.
///
/// `None` for a window that owns nothing, which `window::windows` does not produce; the arm is
/// here because a key over an empty range would collide with every other empty range.
fn owned_range(w: &window::TextWindow) -> Option<(Tid, u64, u64)> {
    let owned = w.owned_nodes();
    let first = owned.first()?;
    let last = owned.last()?;
    Some((first.tid, first.part.start, last.part.end))
}

/// What the store already holds for this expression.
fn extracted(lib: &Library, parts: &[smysl_core::types::PartEntry]) -> BTreeSet<Uid> {
    let mut out = BTreeSet::new();
    for p in parts {
        out.extend(
            lib.store()
                .units_with_source_prefix(&format!("{}#", p.tid.canonical())),
        );
        out.extend(lib.store().units_with_source_prefix(&p.tid.canonical()));
    }
    out
}

/// A hash over a staged batch, for the journal: the uids, in order, length-delimited.
fn batch_hash(units: &[UnitCore]) -> [u8; 32] {
    let mut b = Vec::with_capacity(units.len() * 33);
    for u in units {
        b.extend_from_slice(smysl_core::canonical_uid(u).as_bytes());
        b.push(0x1f);
    }
    smysl_core::hash::hash_bytes(&b)
}

/// The reader a manifest was read with, for the recipe.
fn reader_of(manifest: &smysl_core::types::Manifest) -> String {
    manifest.reader.clone()
}
