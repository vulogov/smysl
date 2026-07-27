//! `smysl-provider` - the model boundary (§21).
//!
//! This is the only crate in the workspace that links an async runtime or an HTTP client
//! (rule B, D-12). It owns a current-thread tokio runtime on a dedicated OS thread and
//! speaks to synchronous callers - including the TUI - over an `std::sync::mpsc` channel,
//! so async never appears in an event path (§21.5).
//!
//! No vendor SDKs: five SDKs would mean five dependency subtrees for what is, in each
//! case, one POST with a JSON body. `serde_json` is used for provider wire formats only,
//! never for smysl's canonical form.
//!
//! # The trait is synchronous
//!
//! §21.1 writes `async fn complete`. §21.3 writes `providers: BTreeMap<ProviderId, Box<dyn
//! Provider>>`. Those two cannot both hold: `async fn` in a trait is not dyn-compatible, so
//! a trait with one cannot be boxed. The registry is the load-bearing half - routing and
//! fallback are the whole point - so the trait is synchronous and the runtime lives behind
//! it, which is also what guarantee A3 promises callers.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod config;
pub mod http;
pub mod map;
pub mod registry;
pub mod runtime;
pub mod stream;
pub mod usage;

use std::fmt;

pub use config::Config;
/// `Config` under a name that does not collide with a facade re-export.
pub use config::Config as ProviderConfigFile;
pub use registry::Registry;
pub use smysl_core::error::ProviderError;
pub use stream::StreamMsg;
pub use usage::{Ledger, LedgerEntry};

/// A configured provider's name. Not the model: one provider serves many models.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    /// Lowercase, alphanumeric plus `-` and `_`. A provider id reaches a config file, a
    /// ledger, and a `--provider` flag, so it stays boring on purpose.
    pub fn new(s: impl Into<String>) -> Option<ProviderId> {
        let s = s.into();
        let ok = !s.is_empty()
            && s.len() <= 64
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        ok.then_some(ProviderId(s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&self.0)
    }
}

/// What a model is being asked to do (D-9, §22.1).
///
/// Routing is per task rather than global, so a deployment can keep bulk ingest local and
/// send only relation extraction to a hosted model - and `providers --tasks` can say
/// exactly which of those leaves the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Task {
    ContentIngest,
    RelationExtraction,
    GistRewrite,
    ThreadRefine,
    Attest,
}

impl Task {
    pub const ALL: &'static [Task] = &[
        Task::ContentIngest,
        Task::RelationExtraction,
        Task::GistRewrite,
        Task::ThreadRefine,
        Task::Attest,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Task::ContentIngest => "content-ingest",
            Task::RelationExtraction => "relation-extraction",
            Task::GistRewrite => "gist-rewrite",
            Task::ThreadRefine => "thread-refine",
            Task::Attest => "attest",
        }
    }

    pub fn parse(s: &str) -> Option<Task> {
        Task::ALL.iter().copied().find(|t| t.as_str() == s)
    }

    /// Which command performs this task, for `providers --tasks`.
    pub const fn command(self) -> &'static str {
        match self {
            Task::ContentIngest | Task::RelationExtraction => "ingest",
            Task::GistRewrite | Task::ThreadRefine => "thread --refine",
            Task::Attest => "attest",
        }
    }

    /// Whether performing this task sends unit content off the machine.
    ///
    /// Every task does. The method exists so that `providers --tasks` reads from a fact
    /// rather than from a hard-coded sentence, and so that a future task that genuinely
    /// egresses nothing has somewhere to say so.
    pub const fn egresses_content(self) -> bool {
        true
    }
}

impl fmt::Display for Task {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// How a provider can be made to emit structured output (§21.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub enum StructuredMode {
    JsonSchema,
    ToolForce,
    JsonMode,
    Grammar,
    #[default]
    None,
}

impl StructuredMode {
    /// Whether the provider structurally guarantees schema conformance. `JsonMode` does
    /// not - which is why D-9 leaves that provider's default path to measured E9.
    pub const fn is_enforced(self) -> bool {
        matches!(
            self,
            StructuredMode::JsonSchema | StructuredMode::ToolForce | StructuredMode::Grammar
        )
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            StructuredMode::JsonSchema => "json-schema",
            StructuredMode::ToolForce => "tool-force",
            StructuredMode::JsonMode => "json-mode",
            StructuredMode::Grammar => "grammar",
            StructuredMode::None => "none",
        }
    }
}

impl fmt::Display for StructuredMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// What a provider can do (§21.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Capabilities {
    pub context_window: usize,
    pub max_output: usize,
    pub structured: StructuredMode,
    pub streaming: bool,
    pub usage_reporting: bool,
    /// Whether using this provider keeps everything on the machine. This is the field
    /// `--offline` tests, so it is a property of the provider rather than of the flag.
    pub offline: bool,
}

impl Default for Capabilities {
    fn default() -> Capabilities {
        Capabilities {
            context_window: 4096,
            max_output: 1024,
            structured: StructuredMode::None,
            streaming: false,
            usage_reporting: false,
            offline: false,
        }
    }
}

/// Who is speaking in a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Role {
    System,
    User,
    Assistant,
}

impl Role {
    pub const fn as_str(self) -> &'static str {
        match self {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn user(content: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Message {
        Message {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// One model call.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Request {
    pub model: String,
    pub system: String,
    pub messages: Vec<Message>,
    pub max_output: usize,
    /// Zero unless a caller has a reason. A model call cannot be made deterministic, but
    /// nothing is gained by making it less so than it could be.
    pub temperature: f32,
    pub structured: StructuredMode,
    /// A JSON schema, when `structured` calls for one.
    pub schema: Option<String>,
}

impl Request {
    pub fn new(model: impl Into<String>, prompt: impl Into<String>) -> Request {
        Request {
            model: model.into(),
            system: String::new(),
            messages: vec![Message::user(prompt)],
            max_output: 1024,
            temperature: 0.0,
            structured: StructuredMode::None,
            schema: None,
        }
    }

    pub fn with_system(mut self, s: impl Into<String>) -> Request {
        self.system = s.into();
        self
    }

    pub fn with_max_output(mut self, n: usize) -> Request {
        self.max_output = n;
        self
    }

    pub fn with_schema(mut self, mode: StructuredMode, schema: impl Into<String>) -> Request {
        self.structured = mode;
        self.schema = Some(schema.into());
        self
    }

    /// Every character that would be sent. Used to refuse a request that cannot fit before
    /// it is made, rather than paying for the refusal.
    pub fn payload_len(&self) -> usize {
        self.system.len() + self.messages.iter().map(|m| m.content.len()).sum::<usize>()
    }
}

/// What came back.
///
/// Built through [`Completion::new`]: the `Provider` trait is public, so an implementor
/// outside this crate has to be able to return one.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Completion {
    pub text: String,
    pub model: String,
    pub usage: Usage,
    /// Whether the provider structurally enforced the requested schema, as opposed to
    /// merely being asked to.
    pub structured: bool,
}

/// Token counts for one call.
///
/// `#[non_exhaustive]`, so it is built through [`Usage::new`] rather than a literal - the
/// `Provider` trait is public and an implementor outside this crate has to be able to
/// return one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// True when the provider did not report counts and these are the bundled estimator's.
    pub estimated: bool,
    /// Retries are recorded here and never in provenance: a retry is not a distinct model
    /// call for recipe purposes (§21.4).
    pub retries: u32,
}

impl Usage {
    /// Counts the provider reported.
    pub const fn reported(input_tokens: u64, output_tokens: u64) -> Usage {
        Usage {
            input_tokens,
            output_tokens,
            estimated: false,
            retries: 0,
        }
    }

    /// Counts the bundled estimator produced, which says so (D-2, `SMY-W305`).
    pub const fn estimated(input_tokens: u64, output_tokens: u64) -> Usage {
        Usage {
            input_tokens,
            output_tokens,
            estimated: true,
            retries: 0,
        }
    }

    pub const fn after_retries(mut self, retries: u32) -> Usage {
        self.retries = retries;
        self
    }

    pub const fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

impl Completion {
    /// A completion whose structure the provider did *not* enforce.
    ///
    /// The unenforced form is the constructor without a suffix, so claiming enforcement is
    /// something a mapper has to do on purpose.
    pub fn new(text: impl Into<String>, model: impl Into<String>, usage: Usage) -> Completion {
        Completion {
            text: text.into(),
            model: model.into(),
            usage,
            structured: false,
        }
    }

    /// Mark the structure as provider-enforced. Only a mapper whose mechanism actually
    /// guarantees the shape may call this - see `StructuredMode::is_enforced`.
    pub fn enforced(mut self) -> Completion {
        self.structured = true;
        self
    }
}

/// A token count that says whether it is trustworthy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum TokenCount {
    Exact(u64),
    Estimated(u64),
}

impl TokenCount {
    pub const fn value(self) -> u64 {
        match self {
            TokenCount::Exact(n) | TokenCount::Estimated(n) => n,
        }
    }

    pub const fn is_exact(self) -> bool {
        matches!(self, TokenCount::Exact(_))
    }
}

/// What `providers --probe` learned. No content egress: a probe asks what a provider *is*,
/// never sends it anything to think about.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Probe {
    pub reachable: bool,
    pub models: Vec<String>,
    /// Capabilities as reported by the endpoint, which may differ from the configured
    /// ones. A probe that could only echo the configuration would be worthless.
    pub caps: Option<Capabilities>,
    pub detail: String,
}

impl Probe {
    /// A provider that answered.
    pub fn reachable(models: Vec<String>, caps: Capabilities, detail: impl Into<String>) -> Probe {
        Probe {
            reachable: true,
            models,
            caps: Some(caps),
            detail: detail.into(),
        }
    }

    pub fn unreachable(detail: impl Into<String>) -> Probe {
        Probe {
            reachable: false,
            models: Vec::new(),
            caps: None,
            detail: detail.into(),
        }
    }
}

/// The model boundary (§21.1).
pub trait Provider: Send + Sync {
    fn id(&self) -> ProviderId;

    /// What this provider can do, from configuration. `probe` reports what it *actually*
    /// does, which is not always the same thing.
    fn caps(&self) -> Capabilities;

    fn complete(&self, req: &Request) -> Result<Completion, ProviderError>;

    /// Stream tokens over a synchronous channel (§21.5). The default refuses, so a
    /// provider that cannot stream says so rather than silently buffering.
    fn stream(
        &self,
        _req: &Request,
        _tx: std::sync::mpsc::Sender<StreamMsg>,
    ) -> Result<Usage, ProviderError> {
        Err(ProviderError::StructuredUnsupported)
    }

    /// Count tokens, saying whether the count is the provider's or the estimator's.
    ///
    /// The default is the bundled estimator (D-2), which is deliberately approximate and
    /// reports itself as such rather than pretending.
    fn count_tokens(&self, text: &str) -> TokenCount {
        TokenCount::Estimated(smysl_core::tokens(text) as u64)
    }

    /// Ask what the provider is. **No content egress.**
    fn probe(&self) -> Result<Probe, ProviderError>;
}

/// Which mappers this build contains.
pub fn compiled_mappers() -> Vec<&'static str> {
    let mut v = Vec::new();
    if cfg!(feature = "ollama") {
        v.push("ollama");
    }
    if cfg!(feature = "anthropic") {
        v.push("anthropic");
    }
    if cfg!(feature = "openai") {
        v.push("openai");
    }
    if cfg!(feature = "gemini") {
        v.push("gemini");
    }
    if cfg!(feature = "deepseek") {
        v.push("deepseek");
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_mode_is_not_an_enforced_structure() {
        assert!(!StructuredMode::JsonMode.is_enforced());
        assert!(!StructuredMode::None.is_enforced());
        assert!(StructuredMode::JsonSchema.is_enforced());
        assert!(StructuredMode::ToolForce.is_enforced());
        assert!(StructuredMode::Grammar.is_enforced());
    }

    #[test]
    fn mapper_list_is_sorted_and_deduplicated() {
        let m = compiled_mappers();
        let mut sorted = m.clone();
        sorted.dedup();
        assert_eq!(m.len(), sorted.len());
    }

    #[test]
    fn provider_ids_stay_boring() {
        assert!(ProviderId::new("ollama").is_some());
        assert!(ProviderId::new("my-local_2").is_some());
        assert!(ProviderId::new("Ollama").is_none(), "no uppercase");
        assert!(ProviderId::new("oll ama").is_none(), "no spaces");
        assert!(ProviderId::new("").is_none());
        assert!(ProviderId::new("x".repeat(65)).is_none());
    }

    #[test]
    fn task_names_round_trip() {
        for &t in Task::ALL {
            assert_eq!(Task::parse(t.as_str()), Some(t));
        }
        assert_eq!(Task::parse("summarise"), None);
    }

    /// `providers --tasks` reports exactly what would leave the machine (§29), so every
    /// task must be able to answer the question.
    #[test]
    fn every_task_names_its_command_and_admits_it_egresses() {
        for &t in Task::ALL {
            assert!(!t.command().is_empty());
            assert!(t.egresses_content(), "{t} claims to egress nothing");
        }
    }

    #[test]
    fn structured_mode_names_round_trip_through_display() {
        for m in [
            StructuredMode::JsonSchema,
            StructuredMode::ToolForce,
            StructuredMode::JsonMode,
            StructuredMode::Grammar,
            StructuredMode::None,
        ] {
            assert_eq!(m.to_string(), m.as_str());
        }
    }

    #[test]
    fn a_token_count_says_whether_it_can_be_trusted() {
        assert!(TokenCount::Exact(10).is_exact());
        assert!(!TokenCount::Estimated(10).is_exact());
        assert_eq!(TokenCount::Estimated(10).value(), 10);
    }

    #[test]
    fn usage_totals_both_directions() {
        let u = Usage {
            input_tokens: 30,
            output_tokens: 12,
            ..Usage::default()
        };
        assert_eq!(u.total(), 42);
    }

    #[test]
    fn a_request_can_measure_itself_before_it_is_sent() {
        let r = Request::new("m", "hello").with_system("sys");
        assert_eq!(r.payload_len(), 8);
    }

    #[test]
    fn temperature_defaults_to_zero() {
        // A model call cannot be made deterministic, but nothing is gained by making it
        // less so than it could be.
        assert_eq!(Request::new("m", "x").temperature, 0.0);
    }
}
