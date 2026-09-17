//! `smysl-ingest` - the ingest boundary (§9, §22).
//!
//! Three rules meet here. **Rule S**: model output is staged, checked, and confirmed, never
//! written straight to the store. **Rule I**: ingest always makes progress - an unrepairable
//! span degrades to an opaque `prose` unit rather than failing the run. **Rule T**: a model
//! asserting from its own priors is capped at `inferred`, however confidently it phrases the
//! claim.
//!
//! The three are one idea from three directions: a model's output is a proposal, its
//! failures are recoverable, and its confidence is not evidence.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

#[cfg(feature = "model")]
pub mod attest;
pub mod ceiling;
// Splitting a document into model-sized pieces. Internal since 0.13 (§1.2 S4): `Ingestor`
// chunks as part of the path a caller actually takes.
#[cfg(feature = "model")]
pub(crate) mod chunk;
pub mod import;
/// Reachable for `smysl-eval`'s `quoting_live.rs`, which drives real models against the
/// batch shape and needs to parse what comes back the way the ingest path does.
#[doc(hidden)]
pub mod json_ast;
// Rule M's status ceiling arithmetic. Internal since 0.13 (§1.2 S4): `ceiling` is the
// facade-exported way to ask the question this answers.
pub(crate) mod monotone;
#[cfg(feature = "model")]
pub mod path;
/// Reachable for `tests/gate.rs`, which checks the prompt the ingest path actually sends.
///
/// `FENCE` and `content_ingest_json` are what the gate asserts against, and asserting against
/// a copy of them would test the copy. Hidden rather than `pub(crate)` because an integration
/// test is a separate crate; hidden rather than left public because no consumer builds our
/// prompts — `Ingestor` does.
#[doc(hidden)]
pub mod prompt;
/// Reachable for `tests/gate.rs` and the workspace's `tests/interactions.rs`, which check that
/// a quoted body survives the round trip the ingest path puts it through.
#[doc(hidden)]
pub use smysl_core::quote;
pub mod recipe;
/// Reachable for `tests/gate.rs`, which drives `check_local` and the degradation path
/// directly. No consumer runs the re-ask loop itself — `IngestOptions::repair_attempts` is
/// how a caller sets its budget.
#[doc(hidden)]
pub mod repair;
/// Reachable for `smysl-eval`'s `quoting_live.rs`, which sends `batch_schema()` to live
/// providers to measure whether they honour it.
#[doc(hidden)]
pub mod schema;
pub mod stage;

#[cfg(feature = "model")]
use std::collections::BTreeMap;

#[cfg(feature = "model")]
use smysl_core::{Diagnostic, Label, Relation, Uid, UnitCore};
use smysl_core::{Hlc, Rung};
#[cfg(feature = "model")]
use smysl_graph::Store;
#[cfg(feature = "model")]
use smysl_provider::{Provider, ProviderError, Registry, Request, Task, Usage};

#[cfg(feature = "model")]
pub use attest::{attest, AttestOptions, AttestReport, Judgement, What};
pub use smysl_core::SourcePolicy;
pub use stage::{Attest, Staged};

/// The fewest output tokens an ingest call asks for when the caller names no budget.
pub const DEFAULT_MAX_OUTPUT: usize = 2048;

/// Default repair attempts before an unrepairable span degrades to opaque `prose`
/// (rule I, `SMY-W304`).
pub const DEFAULT_REPAIR_ATTEMPTS: u8 = 2;

/// Which ingest path a request takes (D-9). Surface is the default for bulk content: a
/// malformed unit is recoverable, a truncated JSON object is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum IngestPath {
    Surface,
    JsonAst,
}

impl IngestPath {
    pub const fn as_str(self) -> &'static str {
        match self {
            IngestPath::Surface => "surface",
            IngestPath::JsonAst => "json-ast",
        }
    }

    pub fn parse(s: &str) -> Option<IngestPath> {
        match s {
            "surface" => Some(IngestPath::Surface),
            "json-ast" => Some(IngestPath::JsonAst),
            _ => None,
        }
    }
}

impl std::fmt::Display for IngestPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

/// How to ingest.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct IngestOptions {
    pub rung: Rung,
    pub repair_attempts: u8,
    /// `auto` unless the caller insists (D-9).
    pub path: Option<IngestPath>,
    /// A granularity preset: `coarse`, `default` or `fine`, or `standard`, the name this field
    /// has defaulted to since before the presets had names and which means `default`. Hashed
    /// into the recipe as written, so a valid name keeps the recipe it always had; anything
    /// else is refused by [`Ingestor::ingest`] before a call is made. It does not yet choose
    /// the profile units are checked under.
    pub granularity: String,
    pub agent: smysl_core::AgentId,
    /// Supplied, never read, so a replayed ingest produces the same attestations.
    pub now: Hlc,
    /// Which handoff of a pipeline this is.
    ///
    /// Stamped onto every attestation, which is what makes `Store::at_hop` able to answer
    /// "what did the last step add" and what the recency term in `salience` measures
    /// against. Zero for a one-shot ingest; a pipeline increments it per step.
    pub hop: u32,
    pub temperature: f32,
    /// Output tokens to ask for per call. `0`, the default, means the provider's configured
    /// `max_output`, and never less than [`DEFAULT_MAX_OUTPUT`]; see
    /// [`IngestOptions::output_budget`]. Anything else is sent as given.
    pub max_output: usize,
    /// The model to ask for. Empty means the provider's configured default.
    pub model: String,
    /// The caller's own prompt and schema, in place of the built-in ones.
    ///
    /// Everything downstream of the model is unchanged by it: the same conversion, the same
    /// quote check, the same rule T cap, the same staging. That is what distinguishes running an
    /// extraction through `ingest` from running it beside `ingest` in a separate script.
    pub prompt: Option<prompt::PromptOverride>,
    /// A source for the units, and how it combines with one the model wrote.
    ///
    /// For a caller who knows provenance exactly and does not want a model inventing it.
    /// Applied on both paths before any unit is built; see [`SourcePolicy`]. Part of the recipe.
    pub source: Option<(smysl_core::SourceRef, SourcePolicy)>,
}

impl IngestOptions {
    /// Ingest at this trust rung. `#[non_exhaustive]`, so options are built by adjusting a
    /// default rather than by a literal - a caller who writes out every field today would
    /// stop compiling when one is added.
    pub fn at_rung(rung: Rung) -> IngestOptions {
        IngestOptions {
            rung,
            ..IngestOptions::default()
        }
    }

    pub fn with_path(mut self, p: IngestPath) -> IngestOptions {
        self.path = Some(p);
        self
    }

    pub fn with_repair_attempts(mut self, n: u8) -> IngestOptions {
        self.repair_attempts = n;
        self
    }

    pub fn with_model(mut self, m: impl Into<String>) -> IngestOptions {
        self.model = m.into();
        self
    }

    /// Stamp this ingest as a given handoff of a pipeline.
    pub fn at_hop(mut self, hop: u32) -> IngestOptions {
        self.hop = hop;
        self
    }

    pub fn with_max_output(mut self, n: usize) -> IngestOptions {
        self.max_output = n;
        self
    }

    /// The output tokens a call to this provider asks for.
    ///
    /// Until 1.3 every ingest asked for 2,048 whatever the provider was configured for, so a
    /// json-ast answer of about a dozen units was cut off at `MAX_TOKENS` and degraded while
    /// the configuration said 8,192. Now the configured `max_output` is used when the caller
    /// names none — with 2,048 as a floor, so a configuration that never set the field (and
    /// reads 1,024) asks for no less than it did.
    #[cfg(feature = "model")]
    pub fn output_budget(&self, caps: &smysl_provider::Capabilities) -> usize {
        if self.max_output > 0 {
            self.max_output
        } else {
            caps.max_output.max(DEFAULT_MAX_OUTPUT)
        }
    }

    pub fn with_agent(mut self, a: smysl_core::AgentId) -> IngestOptions {
        self.agent = a;
        self
    }

    pub fn with_now(mut self, now: Hlc) -> IngestOptions {
        self.now = now;
        self
    }

    pub fn with_granularity(mut self, g: impl Into<String>) -> IngestOptions {
        self.granularity = g.into();
        self
    }

    /// The preset [`IngestOptions::granularity`] names, or why it names none.
    pub fn granularity_profile(&self) -> Result<smysl_core::GranularityProfile, String> {
        let name = match self.granularity.as_str() {
            "standard" => "default",
            other => other,
        };
        smysl_core::GranularityProfile::preset(name).ok_or_else(|| {
            format!(
                "`{}` is not a granularity preset: coarse, default (or standard), fine",
                self.granularity
            )
        })
    }

    /// Ask with the caller's prompt and, optionally, a narrower schema. Validated by
    /// [`Ingestor::ingest`] before any call is made.
    pub fn with_prompt(mut self, p: prompt::PromptOverride) -> IngestOptions {
        self.prompt = Some(p);
        self
    }

    /// Give units this source, combined with any the model wrote by `policy`.
    pub fn with_source(
        mut self,
        source: smysl_core::SourceRef,
        policy: SourcePolicy,
    ) -> IngestOptions {
        self.source = Some((source, policy));
        self
    }

    /// The path the caller asked for, reconciled with the prompt override.
    ///
    /// A schema only has a channel on the json-ast path, so an override that supplies one
    /// moves `auto` there. Forcing `surface` alongside it is refused rather than quietly
    /// dropping the schema: a caller who wrote one believes answers are held to it.
    ///
    /// Public so that `--dry-run` reports the path the run will take. It computed the path from
    /// `path` directly, which with a schema-bearing override would say `surface` for a run that
    /// goes json-ast — and "what would be sent" is the only question `--dry-run` answers.
    ///
    /// The error is a `String` rather than a `ProviderError` because it is the caller's
    /// configuration that is wrong, not a provider's answer. It was a `ProviderError::Malformed`
    /// first, and the CLI duly printed "malformed provider response" and exited 6, the code for
    /// a provider failure, for a mistake in a file the caller wrote.
    pub fn requested_path(&self) -> Result<Option<IngestPath>, String> {
        let Some(o) = &self.prompt else {
            return Ok(self.path);
        };
        o.validate().map_err(|e| format!("`{}`: {e}", o.id))?;
        match (o.schema.is_some(), self.path) {
            (true, Some(IngestPath::Surface)) => Err(format!(
                "`{}` supplies a schema, which the surface path has no way to send; use the \
                 json-ast path or drop the schema",
                o.id
            )),
            (true, None) => Ok(Some(IngestPath::JsonAst)),
            (_, p) => Ok(p),
        }
    }
}

impl Default for IngestOptions {
    fn default() -> IngestOptions {
        let agent = smysl_core::AgentId::new("tool:smysl-ingest").expect("a valid literal");
        IngestOptions {
            // `document`, not `model`: a caller ingesting a file is transcribing something
            // that already exists, and the ceiling should say so. A caller asking a model
            // to invent content has to say that explicitly.
            rung: Rung::Document,
            repair_attempts: DEFAULT_REPAIR_ATTEMPTS,
            path: None,
            granularity: "standard".into(),
            now: Hlc::zero(agent.clone()),
            hop: 0,
            agent,
            temperature: 0.0,
            max_output: 0,
            model: String::new(),
            prompt: None,
            source: None,
        }
    }
}

#[cfg(feature = "model")]
/// What one ingest run did.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct IngestReport {
    pub chunks: usize,
    /// Model calls made, including repairs.
    pub calls: usize,
    /// Spans that exhausted their repair budget and degraded (`SMY-W304`). A chunk whose only
    /// defects were single units' own (`repair::UNIT_LOCAL`) degrades those units, each
    /// counted here, and keeps the rest.
    pub degraded: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub path: Option<IngestPath>,
    pub recipe: Option<[u8; 32]>,
    pub family: Option<[u8; 32]>,
    pub usage: Usage,
    pub provider: Option<smysl_provider::ProviderId>,
}

#[cfg(feature = "model")]
/// The ingest boundary.
pub struct Ingestor<'a> {
    registry: &'a Registry,
    opts: IngestOptions,
}

#[cfg(feature = "model")]
impl<'a> Ingestor<'a> {
    pub fn new(registry: &'a Registry, opts: IngestOptions) -> Ingestor<'a> {
        Ingestor { registry, opts }
    }

    /// Ingest a document into staged units (rules S, I, T).
    ///
    /// Fails only on a provider error the registry could not route around - an unroutable
    /// task, or `--offline` against a hosted provider. Everything a *model* can get wrong is
    /// recoverable, which is rule I, and is why the chunk loop returns a report rather than
    /// a `Result` per span.
    pub fn ingest(
        &self,
        store: &Store,
        input: &str,
    ) -> Result<(Staged, IngestReport), ProviderError> {
        // A bad override is a configuration mistake, and it is reported before egress rather
        // than spent chunk by chunk as degraded spans.
        let requested = self
            .opts
            .requested_path()
            .map_err(|e| ProviderError::Config(format!("prompt override {e}")))?;
        // Was accepted whatever it said and only hashed: `--granularity bogus` ran, and its
        // recipe differed from every real run's for a word nothing else read.
        self.opts
            .granularity_profile()
            .map_err(ProviderError::Config)?;
        let provider = self.registry.for_task(Task::ContentIngest)?;
        let caps = provider.caps();

        let choice = path::choose(&caps, Task::ContentIngest, input.len(), requested);
        let window =
            chunk::Window::for_context(caps.context_window, self.opts.output_budget(&caps));
        let chunks = chunk::chunk(input, window);

        // The recipe names the template that is actually sent. It used to hardcode the
        // built-in id at version 1, which made the documented way to distinguish a deployment's
        // wording — change the id or the version — a change nothing read.
        let template = self.template_for(choice.path);
        let conditions = recipe::Conditions::new(template.id.clone(), template.version)
            .with_provider(provider.id().to_string(), &self.opts.model)
            .with_granularity(&self.opts.granularity)
            .with_temperature(self.opts.temperature)
            .with_schemas(["smysl.kernel/0.1".to_string()])
            .with_path(choice.path);
        let conditions = match &self.opts.source {
            Some((s, policy)) => conditions.with_source(s, *policy),
            None => conditions,
        };

        let mut report = IngestReport {
            chunks: chunks.len(),
            path: Some(choice.path),
            recipe: Some(conditions.recipe()),
            family: Some(conditions.family()),
            provider: Some(provider.id()),
            ..IngestReport::default()
        };

        let mut units: Vec<UnitCore> = Vec::new();
        let mut relations: Vec<Relation> = Vec::new();
        let mut labels: BTreeMap<Label, Uid> = BTreeMap::new();
        for piece in &chunks {
            let out = self.one_chunk(provider, choice.path, &piece.text, &mut report.usage);
            report.calls += out.calls;
            report.degraded += out.degraded;
            report.diagnostics.extend(out.diagnostics);
            units.extend(out.units);
            relations.extend(out.relations);
            // A model names units per chunk and cannot see the others, so two chunks can use one
            // label for different units. The first keeps it, and the collision is reported
            // rather than resolved by whichever chunk came last.
            for (label, uid) in out.labels {
                match labels.get(&label) {
                    Some(existing) if *existing != uid => {
                        report.diagnostics.push(
                            Diagnostic::on(smysl_core::Code::W054, uid).with_message(format!(
                                "`{label}` was given to a different unit in an earlier chunk; \
                                 this one is staged without a name"
                            )),
                        );
                    }
                    Some(_) => {}
                    None => {
                        labels.insert(label, uid);
                    }
                }
            }
        }

        // Chunk-boundary duplication self-heals: two chunks that produced the same claim
        // produced the same uid, so this is bookkeeping rather than repair.
        let mut seen = std::collections::BTreeSet::new();
        units.retain(|u| seen.insert(smysl_core::canonical_uid(u)));

        let attest = Attest::new(
            self.opts.agent.clone(),
            self.opts.rung,
            self.opts.now.clone(),
        )
        .at_hop(self.opts.hop)
        .with_recipe(conditions.recipe(), conditions.family());

        // Edges duplicated across chunk boundaries collapse the same way units do: the
        // endpoints are content-addressed, so the same edge twice is the same edge.
        relations.sort_by_key(|r| (r.kind.as_str().to_string(), r.from, r.to));
        relations.dedup_by(|a, b| a.kind == b.kind && a.from == b.from && a.to == b.to);

        Ok((
            stage::prepare(store, units, relations, labels, &attest),
            report,
        ))
    }

    /// One chunk, with its repair budget. Always produces units (rule I).
    fn one_chunk(
        &self,
        provider: &dyn Provider,
        path: IngestPath,
        text: &str,
        usage: &mut Usage,
    ) -> ChunkOutcome {
        let template = self.template_for(path);

        let mut request = self.request(provider, &template, text, path);
        let mut calls = 0usize;
        // Every attempt's errors, marked by attempt. A degraded chunk used to report only the
        // last attempt's, which shows what the repair broke rather than what the model first
        // got wrong: flash-lite's `SMY-E032` vanished behind the stray text its own repair
        // introduced, so the reported cause was one smysl's prompt had caused.
        let mut history: Vec<Diagnostic> = Vec::new();
        let attempts = self.opts.repair_attempts as usize + 1;
        let mut last_answer = None;

        for attempt in 0..=self.opts.repair_attempts {
            let completion = match provider.complete(&request) {
                Ok(c) => c,
                // A provider failure is not a model mistake, so it does not spend the repair
                // budget - but rule I still applies, so the span degrades rather than taking
                // the run down.
                Err(e) => {
                    // An error the endpoint returned is a call made, and usually billed. Every
                    // one was reported as no call: a run cut off at the output limit said
                    // `0 call(s), 0 token(s)`. Only a request that never reached a provider is
                    // not counted.
                    if !matches!(
                        e,
                        ProviderError::Unreachable
                            | ProviderError::OfflineViolation
                            | ProviderError::Config(_)
                            | ProviderError::StructuredUnsupported
                    ) {
                        calls += 1;
                    }
                    if let ProviderError::Truncated { used: Some(n), .. } = e {
                        usage.output_tokens += n as u64;
                        usage.estimated = true;
                    }
                    let (core, d) = repair::degrade(text, self.opts.rung, &e.to_string());
                    return ChunkOutcome {
                        units: vec![core],
                        relations: Vec::new(),
                        labels: BTreeMap::new(),
                        calls,
                        degraded: 1,
                        diagnostics: vec![d],
                    };
                }
            };
            calls += 1;
            usage.input_tokens += completion.usage.input_tokens;
            usage.output_tokens += completion.usage.output_tokens;
            usage.estimated |= completion.usage.estimated;
            usage.retries += completion.usage.retries;

            let (units, relations, mut diagnostics, labels) = repair::convert_labelled(
                &completion.text,
                path,
                self.opts.rung,
                self.opts.source.as_ref(),
            );

            // Check every attributed quote against the text this chunk was drawn from.
            // A quote that is not in the source is a fabricated attribution, and an error -
            // so it buys a repair turn, which is the one thing a model can actually fix
            // here. An elided quote is a warning and passes.
            diagnostics.extend(quote::verify(&units, text));
            // §22.3: check what can be checked without the store, so the model still has a
            // turn in which to fix it. Discovering a granularity violation at staging would
            // mean discovering it after the calls were paid for.
            diagnostics.extend(repair::check_local(&units, self.opts.rung).iter().cloned());

            if !repair::needs_repair(&diagnostics) && !units.is_empty() {
                return ChunkOutcome {
                    units,
                    relations,
                    labels,
                    calls,
                    degraded: 0,
                    diagnostics,
                };
            }

            // An answer with no units and no complaint is still a failure - it just has
            // nothing to say about itself, so the repair turn has to.
            let last = if units.is_empty() && diagnostics.is_empty() {
                vec![Diagnostic::new(smysl_core::Code::E001)
                    .with_message("the answer contained no units")]
            } else {
                diagnostics
            };
            for d in last
                .iter()
                .filter(|d| d.severity == smysl_core::Severity::Error)
            {
                let mut marked = d.clone();
                marked.message = format!("attempt {} of {attempts}: {}", attempt + 1, d.message);
                history.push(marked);
            }

            if attempt < self.opts.repair_attempts {
                let t = prompt::repair(
                    &template,
                    &completion.text,
                    &repair::render_diagnostics(&last),
                );
                request = self.request(provider, &t, text, path);
            } else {
                last_answer = Some((units, relations, labels, last));
            }
        }

        // Exhausted, but perhaps only by units whose defect is their own. Then those degrade
        // and their siblings are kept; see `repair::salvage`.
        let why = format!("{} attempt(s)", self.opts.repair_attempts + 1);
        if let Some((units, relations, labels, last)) = &last_answer {
            if let Some(s) = repair::salvage(units, relations, labels, last, self.opts.rung, &why) {
                history.extend(s.diagnostics);
                return ChunkOutcome {
                    units: s.units,
                    relations: s.relations,
                    labels: s.labels,
                    calls,
                    degraded: s.degraded,
                    diagnostics: history,
                };
            }
        }

        // Exhausted. Rule I: degrade, never fail.
        let (core, d) = repair::degrade(text, self.opts.rung, &why);
        history.push(d);
        ChunkOutcome {
            units: vec![core],
            relations: Vec::new(),
            labels: BTreeMap::new(),
            calls,
            degraded: 1,
            diagnostics: history,
        }
    }

    /// The content template for a path, with the caller's override applied.
    fn template_for(&self, path: IngestPath) -> prompt::Template {
        // A caller-supplied source gets the templates that do not ask the model for provenance.
        let sourced = self.opts.source.is_some();
        let base = match (path, sourced) {
            (IngestPath::Surface, false) => prompt::content_ingest_surface(),
            (IngestPath::Surface, true) => prompt::content_ingest_surface_sourced(),
            (IngestPath::JsonAst, false) => prompt::content_ingest_json(),
            (IngestPath::JsonAst, true) => prompt::content_ingest_json_sourced(),
        };
        match &self.opts.prompt {
            Some(o) => o.apply(base),
            None => base,
        }
    }

    /// The batch schema sent on the json-ast path: the caller's, or the kernel's.
    fn batch_schema(&self) -> String {
        self.opts
            .prompt
            .as_ref()
            .and_then(|o| o.schema.clone())
            .unwrap_or_else(|| schema::batch_schema_with(self.opts.source.is_some()))
    }

    fn request(
        &self,
        provider: &dyn Provider,
        template: &prompt::Template,
        text: &str,
        path: IngestPath,
    ) -> Request {
        let caps = provider.caps();
        let mut r = Request::new(&self.opts.model, template.render(text))
            .with_system(&template.system)
            .with_max_output(self.opts.output_budget(&caps));
        r.temperature = self.opts.temperature;

        // A schema is only worth *sending as a schema* where the provider will enforce it;
        // asking for one it ignores would let a caller believe the answer was checked when it
        // was not. That reasoning is right about the structured channel and was wrong about
        // the prompt: the template says "matching the supplied schema exactly", and under a
        // non-enforcing mode nothing supplied one. The model was told to match a document it
        // had never been shown.
        //
        // Observed on DeepSeek under `json-mode`: well-formed JSON, every field invented —
        // `quote` where the schema says `gist`, `grounds` as a sentence rather than an array,
        // `U1` where a label belongs. Sixteen diagnostics and no units, every time, which read
        // as "the provider is useless in json-mode" rather than "nobody told it the fields".
        //
        // So the schema goes in the only channel a non-enforcing provider has. This does not
        // claim enforcement: `Completion::structured` still comes from the provider, and a
        // json-mode answer is still checked by `json_ast::convert` on the way in.
        if path == IngestPath::JsonAst {
            let schema = self.batch_schema();
            if caps.structured.is_enforced() {
                r = r.with_schema(caps.structured, schema);
            } else {
                r = r.with_system(format!(
                    "{}\n\nThe schema your object must match:\n{}",
                    template.system, schema
                ));
            }
        }
        r
    }
}

#[cfg(feature = "model")]
struct ChunkOutcome {
    units: Vec<UnitCore>,
    relations: Vec<Relation>,
    labels: BTreeMap<Label, Uid>,
    calls: usize,
    /// Spans degraded: the whole chunk counts one, a salvaged answer one per unit.
    degraded: usize,
    diagnostics: Vec<Diagnostic>,
}

#[cfg(all(test, feature = "model"))]
mod tests {
    use super::*;

    #[test]
    fn repair_budget_is_two_attempts() {
        assert_eq!(DEFAULT_REPAIR_ATTEMPTS, 2);
    }

    #[test]
    fn ingest_paths_have_stable_names() {
        assert_eq!(IngestPath::Surface.as_str(), "surface");
        assert_eq!(IngestPath::JsonAst.as_str(), "json-ast");
        assert_eq!(IngestPath::parse("surface"), Some(IngestPath::Surface));
        assert_eq!(IngestPath::parse("json-ast"), Some(IngestPath::JsonAst));
        assert_eq!(IngestPath::parse("telepathy"), None);
    }

    /// `document`, not `model`: a caller ingesting a file is transcribing something that
    /// already exists, and the ceiling should say so.
    #[test]
    fn the_default_options_are_conservative() {
        let o = IngestOptions::default();
        assert_eq!(o.rung, Rung::Document);
        assert_eq!(o.repair_attempts, DEFAULT_REPAIR_ATTEMPTS);
        assert_eq!(o.path, None, "auto by default");
        assert_eq!(o.temperature, 0.0);
        assert_eq!(o.max_output, 0, "the provider's, not a constant");
    }

    /// A provider that will not enforce a schema must still be *told* the schema.
    ///
    /// The template says "matching the supplied schema exactly". Until 0.10 nothing supplied
    /// one unless the provider enforced it, so under `json-mode` the model was asked to match
    /// a document it had never seen. Observed on DeepSeek: well-formed JSON with every field
    /// invented, sixteen diagnostics, no units — which reads as a useless provider rather than
    /// a prompt referring to nothing. With the schema in the prompt the same call yields three
    /// units, two relations and no diagnostics.
    mod schema_reaches_the_model {
        use super::*;
        use smysl_provider::{Capabilities, Completion, ProviderId, StructuredMode};

        struct Fake(StructuredMode);

        impl Provider for Fake {
            fn id(&self) -> ProviderId {
                ProviderId::new("fake").unwrap()
            }
            fn caps(&self) -> Capabilities {
                let mut c = Capabilities::default();
                c.structured = self.0;
                c
            }
            fn complete(&self, _: &Request) -> Result<Completion, ProviderError> {
                unreachable!("the request is inspected, never sent")
            }
            fn probe(&self) -> Result<smysl_provider::Probe, ProviderError> {
                unreachable!("not reached; this fake exists to report capabilities")
            }
        }

        fn request_for(mode: StructuredMode) -> Request {
            let reg = Registry::default();
            let ing = Ingestor::new(&reg, IngestOptions::default());
            ing.request(
                &Fake(mode),
                &prompt::content_ingest_json(),
                "some text",
                IngestPath::JsonAst,
            )
        }

        #[test]
        fn an_enforcing_provider_gets_it_as_a_schema() {
            let r = request_for(StructuredMode::JsonSchema);
            assert!(
                r.schema.is_some(),
                "an enforced schema goes in the schema field"
            );
            // A marker of the schema *document*, not of the word "units" — the template
            // itself says `Return one object: {"units": [...]}`, so the obvious check passes
            // vacuously in one direction and fails spuriously in the other.
            assert!(
                !r.system.contains("\"properties\""),
                "and not also in the prompt, which would send it twice"
            );
        }

        #[test]
        fn a_non_enforcing_provider_gets_it_in_the_prompt() {
            let r = request_for(StructuredMode::JsonMode);
            assert!(
                r.schema.is_none(),
                "still not claimed as enforced, because it is not"
            );
            assert!(
                r.system.contains("\"properties\""),
                "but the field names must reach the model somehow; this is the only channel"
            );
        }

        /// The control: the surface path has no schema at all and must not grow one.
        #[test]
        fn the_surface_path_is_untouched() {
            let reg = Registry::default();
            let ing = Ingestor::new(&reg, IngestOptions::default());
            let r = ing.request(
                &Fake(StructuredMode::JsonMode),
                &prompt::content_ingest_surface(),
                "some text",
                IngestPath::Surface,
            );
            assert!(r.schema.is_none());
            assert!(!r.system.contains("\"properties\""));
        }
    }
}
