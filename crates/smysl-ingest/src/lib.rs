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

pub mod attest;
pub mod ceiling;
pub mod chunk;
pub mod json_ast;
pub mod monotone;
pub mod path;
pub mod prompt;
pub mod recipe;
pub mod repair;
pub mod schema;
pub mod stage;

use std::collections::BTreeMap;

use smysl_core::{Diagnostic, Hlc, Label, Relation, Rung, Uid, UnitCore};
use smysl_graph::Store;
use smysl_provider::{Provider, ProviderError, Registry, Request, Task, Usage};

pub use attest::{attest, AttestOptions, AttestReport, Judgement, What};
pub use stage::{Attest, Staged};

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
    pub granularity: String,
    pub agent: smysl_core::AgentId,
    /// Supplied, never read, so a replayed ingest produces the same attestations.
    pub now: Hlc,
    pub temperature: f32,
    pub max_output: usize,
    /// The model to ask for. Empty means the provider's configured default.
    pub model: String,
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

    pub fn with_max_output(mut self, n: usize) -> IngestOptions {
        self.max_output = n;
        self
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
            agent,
            temperature: 0.0,
            max_output: 2048,
            model: String::new(),
        }
    }
}

/// What one ingest run did.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct IngestReport {
    pub chunks: usize,
    /// Model calls made, including repairs.
    pub calls: usize,
    /// Spans that exhausted their repair budget and degraded (`SMY-W304`).
    pub degraded: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub path: Option<IngestPath>,
    pub recipe: Option<[u8; 32]>,
    pub family: Option<[u8; 32]>,
    pub usage: Usage,
    pub provider: Option<smysl_provider::ProviderId>,
}

/// The ingest boundary.
pub struct Ingestor<'a> {
    registry: &'a Registry,
    opts: IngestOptions,
}

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
        let provider = self.registry.for_task(Task::ContentIngest)?;
        let caps = provider.caps();

        let choice = path::choose(&caps, Task::ContentIngest, input.len(), self.opts.path);
        let window = chunk::Window::for_context(caps.context_window, self.opts.max_output);
        let chunks = chunk::chunk(input, window);

        let conditions = recipe::Conditions::new(
            match choice.path {
                IngestPath::Surface => "ingest.content.surface",
                IngestPath::JsonAst => "ingest.content.json",
            },
            1,
        )
        .with_provider(provider.id().to_string(), &self.opts.model)
        .with_granularity(&self.opts.granularity)
        .with_temperature(self.opts.temperature)
        .with_schemas(["smysl.kernel/0.1".to_string()])
        .with_path(choice.path);

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
        for piece in &chunks {
            let out = self.one_chunk(provider, choice.path, &piece.text, &mut report.usage);
            report.calls += out.calls;
            report.degraded += usize::from(out.degraded);
            report.diagnostics.extend(out.diagnostics);
            units.extend(out.units);
            relations.extend(out.relations);
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
        .with_recipe(conditions.recipe(), conditions.family());

        let labels: BTreeMap<Label, Uid> = BTreeMap::new();
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
        let template = prompt::resolve_prompt(match path {
            IngestPath::Surface => prompt::content_ingest_surface(),
            IngestPath::JsonAst => prompt::content_ingest_json(),
        });

        let mut request = self.request(provider, &template, text, path);
        let mut calls = 0usize;
        let mut last: Vec<Diagnostic> = Vec::new();

        for attempt in 0..=self.opts.repair_attempts {
            let completion = match provider.complete(&request) {
                Ok(c) => c,
                // A provider failure is not a model mistake, so it does not spend the repair
                // budget - but rule I still applies, so the span degrades rather than taking
                // the run down.
                Err(e) => {
                    let (core, d) = repair::degrade(text, self.opts.rung, &e.to_string());
                    return ChunkOutcome {
                        units: vec![core],
                        relations: Vec::new(),
                        calls,
                        degraded: true,
                        diagnostics: vec![d],
                    };
                }
            };
            calls += 1;
            usage.input_tokens += completion.usage.input_tokens;
            usage.output_tokens += completion.usage.output_tokens;
            usage.estimated |= completion.usage.estimated;
            usage.retries += completion.usage.retries;

            let (units, relations, mut diagnostics) =
                repair::convert(&completion.text, path, self.opts.rung);
            // §22.3: check what can be checked without the store, so the model still has a
            // turn in which to fix it. Discovering a granularity violation at staging would
            // mean discovering it after the calls were paid for.
            diagnostics.extend(repair::check_local(&units, self.opts.rung).iter().cloned());

            if !repair::needs_repair(&diagnostics) && !units.is_empty() {
                return ChunkOutcome {
                    units,
                    relations,
                    calls,
                    degraded: false,
                    diagnostics,
                };
            }

            // An answer with no units and no complaint is still a failure - it just has
            // nothing to say about itself, so the repair turn has to.
            last = if units.is_empty() && diagnostics.is_empty() {
                vec![Diagnostic::new(smysl_core::Code::E001)
                    .with_message("the answer contained no units")]
            } else {
                diagnostics
            };

            if attempt < self.opts.repair_attempts {
                let t = prompt::resolve_prompt(prompt::repair(
                    &completion.text,
                    &repair::render_diagnostics(&last),
                ));
                request = self.request(provider, &t, text, path);
            }
        }

        // Exhausted. Rule I: degrade, never fail.
        let (core, d) = repair::degrade(
            text,
            self.opts.rung,
            &format!("{} attempt(s)", self.opts.repair_attempts + 1),
        );
        last.push(d);
        ChunkOutcome {
            units: vec![core],
            relations: Vec::new(),
            calls,
            degraded: true,
            diagnostics: last,
        }
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
            .with_max_output(self.opts.max_output);
        r.temperature = self.opts.temperature;

        // A schema is only worth sending where the provider will enforce it; asking for one
        // it ignores would let a caller believe the answer was checked when it was not.
        if path == IngestPath::JsonAst && caps.structured.is_enforced() {
            r = r.with_schema(caps.structured, schema::batch_schema());
        }
        r
    }
}

struct ChunkOutcome {
    units: Vec<UnitCore>,
    relations: Vec<Relation>,
    calls: usize,
    degraded: bool,
    diagnostics: Vec<Diagnostic>,
}

#[cfg(test)]
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
    }
}
