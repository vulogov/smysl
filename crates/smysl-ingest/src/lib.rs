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
/// Consensus extraction: how many passes a window gets, and what their agreement means
/// (§3.4). Ungated — the agreement is computed over units and consults no model, so a build
/// without the `model` feature can still read a corpus's stability.
pub mod consensus;
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
// Payload surgery for the two callers that edit one: lossless, sorted, and `pub(crate)` because
// no consumer edits a payload — a producer builds one. Ungated: `repair` is in every build and
// `text::attach` is not, so the module cannot belong to either side's feature.
pub(crate) mod payload;
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
pub use smysl_core::lang;
pub use smysl_core::quote;
/// The answer step of `ask --answer` (SMYSL-2.5 §3.13, TX-P8 step 3): a model reads a dossier
/// and answers its question, grounded only on the dossier and capped by rule M. Behind `model`,
/// since an answer is something a model writes; it takes the dossier as lines rather than as
/// `smysl-query`'s type, so this crate does not link the query layer to say what a line is.
#[cfg(feature = "model")]
pub mod answer;
/// The linker: judging the pairs [`smysl_text::link`] chose, and staging the edges (TX-P7
/// step 3).
///
/// Behind `text` for the reason the module below it is: the candidate generator lives in
/// `smysl-text`, and a default build of this crate links no library layer. Behind `model` as
/// well, which `same_as` is not: proposing a same-as is an engine's job and judging a pair is
/// a model's, so this module has no form that compiles without a provider.
#[cfg(all(feature = "text", feature = "model"))]
pub mod link;
pub mod recipe;
/// Reachable for `tests/gate.rs`, which drives `check_local` and the degradation path
/// directly. No consumer runs the re-ask loop itself — `IngestOptions::repair_attempts` is
/// how a caller sets its budget.
#[doc(hidden)]
pub mod repair;
/// Staging same-as proposals as edges (TX-P7 step 2).
///
/// Behind `text` for the reason every other text module is: the engines live in
/// `smysl-text`, and a default build of this crate links no library layer.
#[cfg(feature = "text")]
pub mod same_as;
/// Reachable for `smysl-eval`'s `quoting_live.rs`, which sends `batch_schema()` to live
/// providers to measure whether they honour it.
#[doc(hidden)]
pub mod schema;
pub mod stage;
/// The grounded summary tree (TX-P7 step 3).
///
/// Behind `text` like the two modules above it: the levels come from a reading, and a reading
/// is the library layer's. Behind `model` too, like [`link`] and for the same reason — a
/// summary is a sentence a model writes.
#[cfg(all(feature = "text", feature = "model"))]
pub mod summarise;
/// Text ingest: windows over a reading, and the span attachment that anchors an answer in a
/// part (SMYSL-2.4 §3.4, §4.3.4). Behind `text`, which links the library layer — a caller
/// ingesting a document has no structure, no part bytes and no manifest, and should not pay for
/// a reader tier to say so.
#[cfg(feature = "text")]
pub mod text;

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

pub use consensus::{Agreement, Passes};

/// A language tag small enough to copy.
///
/// [`LangPolicy`] has to carry the pivot language — a recipe that recorded `pivot` without
/// saying *which* language would give two incomparable corpora one identity — and `LangPolicy`
/// is `Copy`, which it has been since 1.9 and which cannot be taken back without a major
/// version. A `LangTag` is a `String`, so it cannot go in the variant; the grammar is the only
/// thing worth sharing, and [`LangTag::new`](smysl_core::LangTag) is what defines it.
///
/// So this is that grammar with the bound it already states — a tag is at most 35 bytes — held
/// inline. Ordering is the byte ordering of the tag, because the unused bytes are zero and a
/// shorter tag is a prefix of the longer ones it sorts before.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PivotLang {
    tag: [u8; PivotLang::CAP],
    len: u8,
}

impl PivotLang {
    /// The longest tag `LangTag::new` admits.
    pub const CAP: usize = 35;

    /// Parse a tag, with `LangTag`'s grammar and no second opinion about it.
    ///
    /// The error names the tag rather than the grammar, because the caller that reaches this is
    /// `LangPolicy::parse` and what it prefixes is a policy name.
    pub fn new(s: &str) -> Result<PivotLang, String> {
        let tag =
            smysl_core::LangTag::new(s).map_err(|_| format!("`{s}` is not a language tag"))?;
        let b = tag.as_str().as_bytes();
        // Unreachable while `CAP` is `LangTag`'s own bound, and checked rather than asserted so
        // that a later widening there is a refusal here instead of a panic.
        if b.len() > PivotLang::CAP {
            return Err(format!("`{s}` is longer than {} bytes", PivotLang::CAP));
        }
        let mut buf = [0u8; PivotLang::CAP];
        buf[..b.len()].copy_from_slice(b);
        Ok(PivotLang {
            tag: buf,
            len: b.len() as u8,
        })
    }

    pub fn as_str(&self) -> &str {
        // Valid UTF-8 by construction: `LangTag` admits ASCII alphanumerics and `-` only.
        std::str::from_utf8(&self.tag[..self.len as usize]).unwrap_or("")
    }

    /// The tag as the core carries it, for the `lang` of a unit this pass writes.
    pub fn tag(&self) -> smysl_core::LangTag {
        smysl_core::LangTag::new(self.as_str()).unwrap_or_default()
    }
}

impl std::fmt::Display for PivotLang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

impl std::fmt::Debug for PivotLang {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PivotLang({})", self.as_str())
    }
}

/// What language the model writes in (F-6).
///
/// `Source` writes each gist in the language of the passage it came from. It is the policy that
/// keeps the quote check honest without any help, since a translated quote cannot be `Present`
/// against the text it came from.
///
/// `Pivot` extracts under `Source` and then **translates the gist** of each unit into one
/// working language, as a second pass whose output carries the source unit in `deps`
/// (SMYSL-2.4 §4.3.4). It is not a different extraction: the quote, the span and the place are
/// the source unit's, and what moves language is the one field the pass asked for. Only
/// `ingest --text` implements it — a document ingest has no second pass to run, and is refused
/// rather than quietly given `Source`.
///
/// # Why the language is not in the variant
///
/// `Pivot` is a unit variant, and the tag it writes in lives beside it — in
/// [`IngestOptions::pivot_lang`] and [`Conditions`](crate::recipe::Conditions) — rather than
/// inside it. `LangPolicy` has been an all-unit enum since 1.9, so `LangPolicy::Source as isize`
/// compiles against it downstream; a variant carrying data takes those discriminants away, which
/// `cargo-semver-checks` calls `enum_discriminants_undefined_non_unit_variant` and the project
/// calls a 2.0. Naming a language is not worth a major version, so the policy stayed the family
/// and the language became its parameter. The pairing is read in one place —
/// [`IngestOptions::pivot`] — which is what keeps `Pivot` with no language from meaning anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[non_exhaustive]
pub enum LangPolicy {
    #[default]
    Source,
    Pivot,
}

impl LangPolicy {
    /// The policy's name, without its language.
    pub const fn id(self) -> &'static str {
        match self {
            LangPolicy::Source => "source",
            LangPolicy::Pivot => "pivot",
        }
    }

    /// Parse a policy name, keeping the family and dropping the language.
    ///
    /// The signature 1.9 published, and what a caller that only has to *record* the policy
    /// wants. A caller that has to run the pass needs the tag, which is what
    /// [`LangPolicy::parse_with_lang`] returns; the grammar is one grammar, so everything this
    /// accepts that one accepts and everything one refuses the other refuses.
    ///
    /// The error is a `String` rather than SMYSL-2.1's proposed `ProviderError::Config`:
    /// `ProviderError` arrives with the `model` feature, and `Conditions` has to hold a policy
    /// without the provider layer — `recipe` is reachable from `stage` alone. A conditional
    /// error type for one message is worse than the message. The caller prefixes it, so what a
    /// user sees is unchanged.
    pub fn parse(s: &str) -> Result<LangPolicy, String> {
        LangPolicy::parse_with_lang(s).map(|(policy, _)| policy)
    }

    /// Parse a policy name together with the language it writes in, where it names one.
    pub fn parse_with_lang(s: &str) -> Result<(LangPolicy, Option<PivotLang>), String> {
        match s {
            "source" => Ok((LangPolicy::Source, None)),
            p if p.starts_with("pivot:") => {
                let lang = &p["pivot:".len()..];
                if lang.is_empty() {
                    return Err("`pivot:` names no language; expected `pivot:<lang>`".to_string());
                }
                // `und` is a real answer for a unit whose language nobody knows, and not a
                // language anything can be translated *into*. Refused by name for the same
                // reason the whole policy is: a pass told to write in "undetermined" would
                // write in whatever the model prefers, which is the defect F-6 exists for.
                if lang == "und" || lang == "mul" {
                    return Err(format!(
                        "`{lang}` is not a language to write in; name one language"
                    ));
                }
                Ok((LangPolicy::Pivot, Some(PivotLang::new(lang)?)))
            }
            other => Err(format!(
                "`{other}` is not a language policy; expected `source` or `pivot:<lang>`"
            )),
        }
    }
}

impl std::fmt::Display for LangPolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.id())
    }
}
pub use smysl_core::SourcePolicy;
pub use stage::{Attest, Attesting, Staged};

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
    /// else is refused by [`Ingestor::ingest`] before a call is made. Since 1.7 it also chooses
    /// the profile the staged units are checked under — see [`IngestOptions::granularity_named`]
    /// for when.
    pub granularity: String,
    /// Whether the caller *named* a granularity, rather than leaving the default (1.7).
    ///
    /// The preset was validated and then discarded for three releases: `--granularity fine` was
    /// accepted, hashed into the recipe, and the batch was checked under whatever the store's
    /// view declared, so the flag did not do what its name says. It does now — but only when
    /// asked. The field defaults to `standard`, and a run that never mentions granularity must
    /// keep taking the profile from the store's view (D-5), or upgrading would silently
    /// re-check every batch against a different `l0_max`.
    ///
    /// Set by [`IngestOptions::with_granularity`]. A caller assigning the field directly is
    /// saying the same thing the string says, and should set this too.
    pub granularity_named: bool,
    /// How the granularity bounds are counted (1.9, F-2).
    ///
    /// A preset names bounds; it does not say what counts against them. The default is the
    /// pre-F-2 byte count, so an ingest that does not ask is checked and prompted exactly as
    /// before. `smysl/content/1` gives a Russian gist about 100 characters where the byte count
    /// gives 67, which is what S0 measured as 12.67% of hosted Russian units lost to `SMY-E022`
    /// rather than shortened.
    ///
    /// It changes both halves of the ingest: the gist bound stated in the prompt, and the bound
    /// the staged units are checked against. Recorded in the recipe, because a run asked for a
    /// different gist length is a different run.
    pub estimator: smysl_core::TokenEstimator,
    /// Whether the caller *named* an estimator, rather than leaving the default (1.9, F-2).
    ///
    /// The same trap `granularity_named` exists for, and worse. Without this the profile would
    /// not be applied, so the prompt would state the new estimator's bound — about 100
    /// characters of Cyrillic — while the check still enforced the old one, 67. A caller asking
    /// for a fairer bound would have got *more* `SMY-E022`, not fewer.
    ///
    /// Set by [`IngestOptions::with_estimator`].
    pub estimator_named: bool,
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
    /// What language the model writes in (1.9).
    ///
    /// `Source` always, and the only reason the field exists is that the recipe has to record
    /// it: a run under a policy and a run under none are different runs, and before 1.9 there
    /// was no policy at all, so nothing was recorded and nothing was asked for.
    pub lang_policy: LangPolicy,
    /// The language [`LangPolicy::Pivot`] writes in (1.10).
    ///
    /// Beside the policy rather than inside it, for the reason [`LangPolicy`] gives. Read
    /// through [`IngestOptions::pivot`], which is where the two fields become one answer:
    /// a tag set under `Source` is a leftover and not an instruction, and `Pivot` with no
    /// tag is a policy that never says what to write in. Set by
    /// [`IngestOptions::with_pivot`], which cannot produce either.
    pub pivot_lang: Option<PivotLang>,
    /// Which comparison form the quote check uses (1.9).
    ///
    /// `V1` by default, which is what every release up to 1.8 checked with. `V2` answers the
    /// five SMYSL-2.1 §2.1 probes — French guillemets with spaces inside, German low-high
    /// quotation marks, `ё` against `е`, `ß` against `ss`, a modifier-letter apostrophe — each of
    /// which is a quote a reader would call verbatim and the checker called `Loose` or `Absent`.
    /// `E307` on an honest quote is the expensive direction of that error, since it refuses a
    /// correct attribution. SMYSL-2.4 makes `V2` the default for text ingest.
    pub normaliser: smysl_core::quote::Normaliser,
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
    /// How many times each input is extracted (1.10, §3.4).
    ///
    /// [`Passes::Single`] by default, which is what every release up to 1.10 did and what
    /// document ingest keeps: the doubling §3.4 states belongs to `ingest --text`, and a
    /// default of two would double the cost and the call count of every `ingest` that never
    /// asked for consensus. The text path takes [`Passes::TEXT`].
    ///
    /// Recorded in the recipe, because a corpus built one way is not comparable with one built
    /// the other, and refused above temperature 0 — see [`Passes::admits`].
    pub passes: Passes,
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

    /// What language the model writes in.
    ///
    /// Sets the family only. A policy of [`LangPolicy::Pivot`] needs a language too, which is
    /// what [`IngestOptions::with_pivot`] sets; this one clears any tag a previous call left,
    /// so `with_lang_policy(Source)` is `source` and not `source` with a forgotten pivot in it.
    pub fn with_lang_policy(mut self, p: LangPolicy) -> IngestOptions {
        self.lang_policy = p;
        if p != LangPolicy::Pivot {
            self.pivot_lang = None;
        }
        self
    }

    /// Translate each gist into `lang` after extracting under `Source` (F-6, §4.3.4).
    pub fn with_pivot(mut self, lang: PivotLang) -> IngestOptions {
        self.lang_policy = LangPolicy::Pivot;
        self.pivot_lang = Some(lang);
        self
    }

    /// The language the pivot pass writes in, or `None` under any other policy.
    ///
    /// The one reader of the two fields together: `Some` only when the policy asked for a
    /// pivot *and* named the language, so a caller that assigned the fields directly cannot
    /// get a pass that runs without knowing what to write in.
    pub fn pivot(&self) -> Option<PivotLang> {
        match self.lang_policy {
            LangPolicy::Pivot => self.pivot_lang,
            _ => None,
        }
    }

    /// Which comparison form the quote check uses.
    pub fn with_normaliser(mut self, n: smysl_core::quote::Normaliser) -> IngestOptions {
        self.normaliser = n;
        self
    }

    /// The sampling temperature, 0.0 to 2.0. Not range-checked here: the field is public and
    /// has been since the type existed, so the check belongs where the value is accepted from
    /// outside — `Config::load` and `--temperature`, both of which refuse before any call.
    pub fn with_temperature(mut self, t: f32) -> IngestOptions {
        self.temperature = t;
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
        self.granularity_named = true;
        self
    }

    /// The preset [`IngestOptions::granularity`] names, or why it names none.
    pub fn granularity_profile(&self) -> Result<smysl_core::GranularityProfile, String> {
        let name = match self.granularity.as_str() {
            "standard" => "default",
            other => other,
        };
        smysl_core::GranularityProfile::preset(name)
            .map(|mut p| {
                // The preset supplies the bounds, the option supplies the count. A preset
                // cannot carry the estimator: all three are shared by every caller, and this
                // is per run.
                //
                // The default is left `Unset` rather than named, which is the same precedent
                // as `Conditions::with_estimator` and `with_normaliser`: absent is what the
                // default has always meant, and a profile that names it would compare unequal
                // to the preset it otherwise is.
                p.estimator = match self.estimator {
                    smysl_core::TokenEstimator::Utf8Div4 => smysl_core::ProfileEstimator::Unset,
                    other => smysl_core::ProfileEstimator::Known(other),
                };
                p
            })
            .ok_or_else(|| {
                format!(
                    "`{}` is not a granularity preset: coarse, default (or standard), fine",
                    self.granularity
                )
            })
    }

    /// Count the granularity bounds with `e`, and state its budget in the prompt.
    pub fn with_estimator(mut self, e: smysl_core::TokenEstimator) -> IngestOptions {
        self.estimator = e;
        self.estimator_named = true;
        self
    }

    /// Ask with the caller's prompt and, optionally, a narrower schema. Validated by
    /// [`Ingestor::ingest`] before any call is made.
    pub fn with_prompt(mut self, p: prompt::PromptOverride) -> IngestOptions {
        self.prompt = Some(p);
        self
    }

    /// How many times to extract each input (§3.4).
    pub fn with_passes(mut self, p: Passes) -> IngestOptions {
        self.passes = p;
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
            granularity_named: false,
            // The pre-F-2 count, so an ingest that does not ask behaves exactly as before.
            estimator: smysl_core::TokenEstimator::Utf8Div4,
            estimator_named: false,
            now: Hlc::zero(agent.clone()),
            hop: 0,
            agent,
            temperature: 0.0,
            passes: Passes::default(),
            normaliser: smysl_core::quote::Normaliser::V1,
            lang_policy: LangPolicy::Source,
            pivot_lang: None,
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
    /// Chunks that failed at least one attempt and then succeeded (`SMY-W435`).
    ///
    /// `degraded` counts the repairs that did not work. This counts the ones that did, which
    /// used to be invisible: the chunk returned clean units and the attempt that got it wrong
    /// was dropped on the floor. A run of 40 chunks where 30 needed a second turn and a run
    /// where none did reported the same thing.
    pub repaired: usize,
    pub diagnostics: Vec<Diagnostic>,
    pub path: Option<IngestPath>,
    pub recipe: Option<[u8; 32]>,
    pub family: Option<[u8; 32]>,
    pub usage: Usage,
    /// How many times each chunk was extracted (1.10, §3.4). One unless the caller asked for
    /// consensus; `chunks` counts the pieces of the input, not the extractions, so the calls a
    /// consensus run makes are `chunks * passes` before any repair.
    pub passes: usize,
    /// What the passes agreed about, by `identical-span` (§3.4).
    ///
    /// `None` for a single pass: one extraction agrees with itself, and reporting that as
    /// stability would be reporting the policy rather than a measurement. It is recorded here
    /// rather than left to be recomputed later because the staged batch cannot answer it — two
    /// passes that produced one unit produced one uid, so a count of staged units cannot tell
    /// "both passes said this" from "one did".
    pub agreement: Option<Agreement>,
    pub provider: Option<smysl_provider::ProviderId>,
    /// The model that ran, resolved: what the caller named, else the provider's own.
    ///
    /// The caller knows what they passed and cannot know what an empty `--model` resolved to,
    /// which is why `cmd_ingest` wrote the empty string into the ledger and `usage --by model`
    /// grouped every default run under no model at all.
    pub model: Option<String>,
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
        // A second pass above temperature 0 buys no agreement (MS-8), and the refusal belongs
        // here with the other configuration mistakes: before egress, rather than after paying
        // for two extractions that cannot be compared.
        self.opts
            .passes
            .admits(self.opts.temperature)
            .map_err(ProviderError::Config)?;
        // A pivot over a document is refused here rather than ignored. The pass that writes the
        // other language is `text::pivot`, and it needs units with a place in a part to hang a
        // translation on — which is what a document ingest does not have. Accepting it would
        // hand back gists in the document's own language while the recipe said `pivot:L`, and
        // that is the one failure F-6's policy exists to rule out.
        if self.opts.lang_policy == LangPolicy::Pivot {
            let named = match self.opts.pivot_lang {
                Some(l) => format!("pivot:{l}"),
                None => "pivot".to_string(),
            };
            return Err(ProviderError::Config(format!(
                "`{named}` is implemented for `ingest --text`, which has a reading to attach \
                 the translation to; document ingest takes `source`"
            )));
        }
        // Was accepted whatever it said and only hashed: `--granularity bogus` ran, and its
        // recipe differed from every real run's for a word nothing else read.
        let profile = self
            .opts
            .granularity_profile()
            .map_err(ProviderError::Config)?;
        // Named, or not applied: see `IngestOptions::granularity_named`. Naming an estimator
        // counts as naming the profile, because the bound the units are checked against has to
        // be the bound the prompt stated — otherwise asking for a fairer count enforces the
        // old one and the run is worse off than if it had not asked.
        let profile = (self.opts.granularity_named || self.opts.estimator_named).then_some(profile);
        let provider = self.registry.for_task(Task::ContentIngest)?;
        let caps = provider.caps();

        let choice = path::choose(&caps, Task::ContentIngest, input.len(), requested);
        let window =
            chunk::Window::for_context(caps.context_window, self.opts.output_budget(&caps));
        let chunks = chunk::chunk(input, window);

        // D-10. The fence is derived from the input, so a document carrying a marker cannot
        // close it — but it is still a document trying to speak in this tool's voice, and a
        // reader wants to know. Reported once per input with the offset, not per chunk: the
        // caller is looking for a place in their document.
        let mut marker_notes: Vec<Diagnostic> = Vec::new();
        if let Some(at) = input.find(prompt::MARKER_PREFIX) {
            marker_notes.push(
                Diagnostic::new(smysl_core::Code::W433)
                    .with_message(format!(
                        "input contains `{}` at byte {at}; it is sent inside a fence derived \
                         from the input, so it cannot close one",
                        prompt::MARKER_PREFIX
                    ))
                    .with_suggestion("check that the passage is the document's own text"),
            );
        }

        // The recipe names the template that is actually sent. It used to hardcode the
        // built-in id at version 1, which made the documented way to distinguish a deployment's
        // wording — change the id or the version — a change nothing read.
        let template = self.template_for(choice.path);
        let conditions = recipe::Conditions::new(template.id.clone(), template.version)
            .with_provider(provider.id().to_string(), self.model_for(&caps))
            .with_granularity(&self.opts.granularity)
            .with_temperature(self.opts.temperature)
            .with_normaliser(self.opts.normaliser)
            .with_lang_policy(self.opts.lang_policy)
            .with_estimator(self.opts.estimator)
            .with_passes(self.opts.passes)
            .with_framing(prompt::FRAMING)
            .with_schemas(["smysl.kernel/0.1".to_string()])
            .with_path(choice.path);
        let conditions = match &self.opts.source {
            Some((s, policy)) => conditions.with_source(s, *policy),
            None => conditions,
        };

        let mut report = IngestReport {
            chunks: chunks.len(),
            passes: self.opts.passes.count(),
            path: Some(choice.path),
            recipe: Some(conditions.recipe()),
            family: Some(conditions.family()),
            provider: Some(provider.id()),
            model: Some(self.model_for(&caps).to_string()),
            ..IngestReport::default()
        };

        report.diagnostics.append(&mut marker_notes);

        let mut units: Vec<UnitCore> = Vec::new();
        let mut relations: Vec<Relation> = Vec::new();
        let mut labels: BTreeMap<Label, Uid> = BTreeMap::new();
        // One list per pass, so the agreement can be read afterwards. Keeping it is the whole
        // point of consensus extraction: the staged batch merges what both passes found,
        // because identical content is one unit, and a merged batch cannot be asked which pass
        // produced what.
        let mut per_pass: Vec<Vec<UnitCore>> = Vec::with_capacity(self.opts.passes.count());
        for _pass in 0..self.opts.passes.count() {
            let mut pass_units: Vec<UnitCore> = Vec::new();
            for piece in &chunks {
                let out = self.one_chunk(provider, choice.path, &piece.text, &mut report.usage);
                report.calls += out.calls;
                report.degraded += out.degraded;
                report.repaired += out.repaired;
                report.diagnostics.extend(out.diagnostics);
                pass_units.extend(out.units);
                relations.extend(out.relations);
                // A model names units per chunk and cannot see the others, so two chunks can use
                // one label for different units. The first keeps it, and the collision is
                // reported rather than resolved by whichever chunk came last.
                //
                // A second pass is not a collision: it answers the same chunk, so it offers the
                // same label for the same unit, and the `Some(existing) if *existing != uid`
                // arm is what tells the two cases apart. A pass that named a *different* unit
                // `c/one` is a real disagreement and is reported as one.
                for (label, uid) in out.labels {
                    match labels.get(&label) {
                        Some(existing) if *existing != uid => {
                            report.diagnostics.push(
                                Diagnostic::on(smysl_core::Code::W054, uid).with_message(format!(
                                    "`{label}` was given to a different unit in an earlier \
                                     chunk; this one is staged without a name"
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
            units.extend(pass_units.iter().cloned());
            per_pass.push(pass_units);
        }

        // Recorded for a consensus run and left absent for a single pass: see
        // `IngestReport::agreement`. Nothing here deduplicates, merges or votes — §3.4 reads
        // the agreement, it does not act on it.
        if self.opts.passes.count() > 1 {
            report.agreement = Some(consensus::agreement(&per_pass));
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
            stage::prepare_under(
                store,
                units,
                relations,
                labels,
                Vec::new(),
                &attest,
                profile,
            ),
            report,
        ))
    }

    /// One chunk of a document, with its repair budget. Always produces units (rule I).
    fn one_chunk(
        &self,
        provider: &dyn Provider,
        path: IngestPath,
        text: &str,
        usage: &mut Usage,
    ) -> ChunkOutcome {
        let template = self.template_for(path);
        self.one_call(
            provider,
            path,
            &template,
            text,
            text,
            None,
            &json_ast::Scope::default(),
            self.opts.source.as_ref(),
            usage,
        )
    }

    /// One model call and its repair turns, whatever the call is about.
    ///
    /// Shared by document chunks and text windows, because the repair loop is the same loop and
    /// two copies of it would drift in exactly the place that is hardest to notice: which
    /// diagnostics buy another turn.
    ///
    /// `sent` is what the model is shown; `checked` is what a quote is verified against. For a
    /// chunk they are one string. For a **window** they are not — the model sees each node
    /// prefixed with its locator (§3.9.2) and a quote is checked against the nodes' own text, so
    /// that a quote is not required to contain `[v.3]` to be `Present`.
    #[allow(clippy::too_many_arguments)]
    fn one_call(
        &self,
        provider: &dyn Provider,
        path: IngestPath,
        template: &prompt::Template,
        sent: &str,
        checked: &str,
        digest: Option<&str>,
        scope: &json_ast::Scope,
        source: Option<&(smysl_core::SourceRef, SourcePolicy)>,
        usage: &mut Usage,
    ) -> ChunkOutcome {
        let template = template.clone();
        let text = checked;

        let mut request = self.request_with(provider, &template, sent, digest, path);
        let mut calls = 0usize;
        // Every attempt's errors, marked by attempt. A degraded chunk used to report only the
        // last attempt's, which shows what the repair broke rather than what the model first
        // got wrong: flash-lite's `SMY-E032` vanished behind the stray text its own repair
        // introduced, so the reported cause was one smysl's prompt had caused.
        let mut history: Vec<Diagnostic> = Vec::new();
        let attempts = self.opts.repair_attempts as usize + 1;
        let mut last_answer = None;
        // The last attempt's warnings, so a refused chunk still reports them: see the
        // assignment below.
        let mut last_warnings: Vec<Diagnostic> = Vec::new();

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
                        repaired: 0,
                        diagnostics: vec![d],
                    };
                }
            };
            calls += 1;
            usage.input_tokens += completion.usage.input_tokens;
            usage.output_tokens += completion.usage.output_tokens;
            usage.estimated |= completion.usage.estimated;
            usage.retries += completion.usage.retries;

            let (units, relations, mut diagnostics, labels) =
                repair::convert_scoped(&completion.text, path, self.opts.rung, source, scope);

            // Check every attributed quote against the text this chunk was drawn from.
            // A quote that is not in the source is a fabricated attribution, and an error -
            // so it buys a repair turn, which is the one thing a model can actually fix
            // here. An elided quote is a warning and passes.
            diagnostics.extend(quote::verify_with(self.opts.normaliser, &units, text));
            // And every unit against the *script* of that text (F-6, `SMY-W436`). Until 1.10
            // nothing checked the language policy at all, so a model answering a Russian
            // passage in English staged without remark. A warning, not an error: it must not
            // buy a repair turn on the strength of a code-point table.
            //
            // Matched on the policy rather than run unconditionally, even though `Source` is
            // the only variant there is. `pivot:<lang>` extracts into one working language, so
            // the script to expect under it is the pivot's and not the passage's — and a new
            // variant falling through to this call would inherit a comparison that is wrong
            // for it. The match makes that a compile error instead.
            match self.opts.lang_policy {
                // The extraction pass is a `Source` pass under either policy, which is what
                // makes this comparison the right one for both: `Pivot` translates the gists
                // afterwards (`text::pivot`), and the units it writes are checked against the
                // language they were asked for rather than against the passage's.
                LangPolicy::Source | LangPolicy::Pivot => {
                    diagnostics.extend(smysl_core::lang::verify(&units, text));
                }
            }
            // §22.3: check what can be checked without the store, so the model still has a
            // turn in which to fix it. Discovering a granularity violation at staging would
            // mean discovering it after the calls were paid for.
            diagnostics.extend(repair::check_local(&units, self.opts.rung).iter().cloned());

            if !repair::needs_repair(&diagnostics) && !units.is_empty() {
                // A repair that worked used to erase what it repaired. The chunk returned
                // clean units, `history` went out of scope, and nothing in the run said the
                // model had got it wrong first — so a corpus where most chunks needed a second
                // turn and one where none did reported the same thing, and the prompt that
                // caused it could not be found from the output. Kept as warnings, so no exit
                // code moves: the units really are clean.
                for h in &history {
                    diagnostics.push(
                        Diagnostic::new(smysl_core::Code::W435)
                            .with_message(format!("repaired: {}: {}", h.code, h.message)),
                    );
                }
                return ChunkOutcome {
                    units,
                    relations,
                    labels,
                    calls,
                    degraded: 0,
                    repaired: usize::from(!history.is_empty()),
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
            // The *last* attempt's warnings, kept for the degrade path below. A chunk that is
            // refused used to report only its errors, which lost everything a warning had to
            // say about the same answer: a text answer that supplied a speaker and cited a unit
            // it was never shown reported the citation (an error) and not the speaker (a
            // warning), so a reader saw a window fail for one reason and never learned the
            // other. Replaced rather than appended, because the same warning from three
            // attempts is one fact about the answer.
            last_warnings = last
                .iter()
                .filter(|d| d.severity != smysl_core::Severity::Error)
                .cloned()
                .collect();

            if attempt < self.opts.repair_attempts {
                let t = prompt::repair(
                    &template,
                    &completion.text,
                    &repair::render_diagnostics(&last),
                );
                // The repair template's user message holds the previous answer and the
                // diagnostics, and no `{input}`: a correction turn is about the answer. So the
                // window and its digest are not re-sent, which is what the document path has
                // always done with its chunk.
                request = self.request_with(provider, &t, sent, None, path);
            } else {
                last_answer = Some((units, relations, labels, last));
            }
        }

        // Exhausted, but perhaps only by units whose defect is their own. Then those degrade
        // and their siblings are kept; see `repair::salvage`.
        let why = format!("{} attempt(s)", self.opts.repair_attempts + 1);
        history.extend(last_warnings.iter().cloned());
        if let Some((units, relations, labels, last)) = &last_answer {
            if let Some(s) = repair::salvage(units, relations, labels, last, self.opts.rung, &why) {
                history.extend(s.diagnostics);
                return ChunkOutcome {
                    units: s.units,
                    relations: s.relations,
                    labels: s.labels,
                    calls,
                    degraded: s.degraded,
                    repaired: 0,
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
            repaired: 0,
            diagnostics: history,
        }
    }

    /// The content template for a path, with the caller's override applied.
    fn template_for(&self, path: IngestPath) -> prompt::Template {
        // A caller-supplied source gets the templates that do not ask the model for provenance.
        let sourced = self.opts.source.is_some();
        let base = match (path, sourced) {
            (IngestPath::Surface, false) => {
                prompt::content_ingest_surface_with(self.opts.estimator)
            }
            (IngestPath::Surface, true) => {
                prompt::content_ingest_surface_sourced_with(self.opts.estimator)
            }
            (IngestPath::JsonAst, false) => prompt::content_ingest_json_with(self.opts.estimator),
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

    /// The model this run will actually use: the one the caller named, else the provider's own.
    ///
    /// Every mapper already fell back to its configured model for an empty `Request.model`, so
    /// a run with no `--model` did use a specific model — and recorded an empty string for it.
    /// The recipe exists to say what produced a unit, and that is the field it was silent on.
    /// Resolving it here rather than in each mapper keeps the name in the recipe and the name
    /// in the request the same by construction.
    fn model_for<'c>(&'c self, caps: &'c smysl_provider::Capabilities) -> &'c str {
        if self.opts.model.is_empty() {
            &caps.model
        } else {
            &self.opts.model
        }
    }

    /// The request for one call, with a ledger digest in its own fence where there is one
    /// (§3.9.2).
    fn request_with(
        &self,
        provider: &dyn Provider,
        template: &prompt::Template,
        text: &str,
        digest: Option<&str>,
        path: IngestPath,
    ) -> Request {
        let caps = provider.caps();
        let rendered = match digest {
            Some(d) => template.render_with_digest(text, d),
            None => template.render(text),
        };
        let mut r = Request::new(self.model_for(&caps), rendered)
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
    /// 1 when this chunk succeeded after an attempt that did not.
    repaired: usize,
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
            ing.request_with(
                &Fake(mode),
                &prompt::content_ingest_json(),
                "some text",
                None,
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
            let r = ing.request_with(
                &Fake(StructuredMode::JsonMode),
                &prompt::content_ingest_surface(),
                "some text",
                None,
                IngestPath::Surface,
            );
            assert!(r.schema.is_none());
            assert!(!r.system.contains("\"properties\""));
        }
    }
}
