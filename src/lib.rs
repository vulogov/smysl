//! `smysl` - an AI<->AI<->Human data interchange format, library, and CLI.
//!
//! The library is the product; the CLI is its first consumer (principle P8). Rule A
//! holds throughout: no CLI capability may be unreachable from here, and no code path may
//! be CLI-only.
//!
//! # Feature shape
//!
//! With `default-features = false` this crate is a fully synchronous library with no
//! async runtime, no HTTP client, and no argument parser in its dependency tree
//! (rule B, verified by `xtask check-purity` in CI). Model access is opt-in behind
//! `local` or `remote`.
//!
//! # Guarantees to embedders (§12.3)
//!
//! - **A1** No panics on untrusted input.
//! - **A2** No global state, no implicit I/O.
//! - **A3** No hidden async - only `smysl-provider` needs a runtime.
//! - **A4** Typed, `#[non_exhaustive]` errors.
//! - **A5** Determinism is part of the API: making `pack`, `merge`, `derive_thread`,
//!   `salience`, or `render` non-reproducible is a breaking change regardless of
//!   signature.
//! - **A6** No hidden allocation cliffs.
//!
//! # Build status
//!
//! This is SM-P1 (deterministic codec). The public surface below grows phase by phase
//! toward the full contract of §12.2; each phase stabilises its library API before the
//! matching CLI subcommand is wired to it.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

// ---- core: identifiers, kernel types, codec, identity ----------------------
/// Every error this crate can raise, in one enum, with the CLI's exit code attached.
///
/// Exported as `AnyError` rather than `Error`, which is what it is called in `smysl-core`.
/// Inside a crate that name is idiomatic; through a facade that flattens eleven error types
/// into one namespace it is not — `Error` sitting beside `CodecError` and `ParseError` reads
/// as a twelfth sibling rather than as the enum that wraps the other eleven.
pub use smysl_core::Error as AnyError;
// What this crate promises, and what it merely exposes.
//
// `Documentation/API_CONTRACT.md` has the reasoning; the short version belongs here, because a
// consumer reads the crate and not the repository.
//
// **Contract.** The kernel vocabulary — `Uid`, `UnitCore`, `Status`, `RelKind`, `Record` and
// their neighbours — and the twelve operations over it: `parse_surface`, `write_surface`,
// `to_cbor`, `from_cbor`, `canonical_uid`, `check`, `pack`, `merge`, `salience`,
// `derive_thread`, `trace`, `compact`. Their errors, inputs and outputs go with them. These
// are the format, three other implementations model them, and guarantee A5 already promises
// something stronger than semver about the operations: making one non-reproducible is
// breaking whatever the signature says.
//
// **A seam, not a promise.** The provider machinery (`ProviderConfig`, `Request`,
// `Completion`, `Usage`, `StructuredMode`, `Capabilities`), and render and retrieval (`Bm25`,
// `Hybrid`, `Semantic`, `EmbedModel`, `Query`, `Hit`, `Retriever`). These are public because
// the contract needs them, not because anyone designed them to be built on — `Hybrid` changed
// shape twice inside 0.7 alone. Build on them by all means; expect them to move, and pin a
// version if that matters.
//
// The distinction is not enforced by anything. It is stated so that a consumer who relies on
// the second group has been told, rather than finding out at an upgrade.

pub use smysl_core::surface;
pub use smysl_core::surface::{parse_surface, write_surface, ParseOutcome, WriteContext};
pub use smysl_core::{
    canonical_uid, format_version_supported, from_cbor, from_cbor_seq, hash_bytes, json_escape,
    kernel_major, quantise, to_cbor, to_cbor_seq, tokens, unit_core_bytes, verify, Admission,
    AgentId, AgentKind, Attestation, Code, CodecError, Commit, Commitment, Contention,
    ContentionId, ContentionStatus, Date, Detected, DetectionKind, Diagnostic, DropReason,
    ExitCode, Extra, Fidelity, GranularityProfile, Group, Hlc, IdError, IntegrityError, KernelType,
    Label, LabelBinding, LangTag, Lod, NonDetReason, Op, Optimality, PackInfo, PackMode,
    ParseError, ProfileEstimator, Record, RelKind, Relation, Report, Resolution, ResolutionTarget,
    Role, Rung, SchemaDecl, SchemaId, Severity, ShapeError, SourceKind, SourcePolicy, SourceRef,
    Span, Status, Step, Subject, Thread, ThreadId, ThreadSchema, TokenEstimator, Uid, UidPrefix,
    Unit, UnitCore, UnitCoreBuilder, View, ViewId, Withdrawal, FORMAT_VERSIONS_SUPPORTED,
    FORMAT_VERSION_DEFAULT, KERNEL_MAJOR, KERNEL_SCHEMA,
};

/// The library records, and the four identities over them (RFC SMYSL-2.4, TX-P1).
///
/// Unconditional, where [`text`] is behind a feature, and the line between them is the line
/// the RFC draws: a manifest, a part text and a reading are *the format*, decoded by the
/// envelope and held by a `Store` in any build, while reading a file into them needs a reader.
///
/// Step 1 added these types and nothing re-exported them, which made `Record::Manifest`'s
/// payload unnameable through this crate: a consumer could match the variant and not write
/// down what it had. Found in step 5, when a test in this repository had to reach past the
/// facade to the crate that defines them — the clearest possible statement that rule A was
/// not being met.
///
/// `Redaction` (TX-P2 step 4) and the dating types (TX-P3 step 1) are here for the same
/// reason, and the second omission was found by looking rather than by tripping over it: every
/// variant of `Record` has a payload, and a variant whose payload cannot be named is a variant
/// a consumer can only count.
pub use smysl_core::ids::{Did, Mid, Rdid, Tid};
pub use smysl_core::types::{
    Allen, Axis, Calendar, Carry, Dating, DatingTarget, DatingValue, Manifest, ParentKind,
    PartEntry, PartReading, PartResolver, PartText, Redaction, Resolved,
};

/// EDTF — the date syntax `source.published` and a dating's absolute value carry (A-2.3).
///
/// A module rather than a handful of names, because it is a parser with an AST: `Edtf`, the
/// `Date` and `DateTime` it holds, `Year`, `Part`, `Qualifier`, `Zone` and `End`. `Date` would
/// collide with the kernel's own `Date` — the `YYYY-MM-DD` of `source.captured` — if it were
/// re-exported flat, and the two are genuinely different things: one is a calendar day, the
/// other is what a source said about when something happened.
pub use smysl_core::edtf;

/// The registered extension schemas (spec §5.1, RFC SMYSL-2.3 A-11).
///
/// A module rather than flat names, for the reason `edtf` is one: `text::ID`, `text::FIELDS` and
/// `text::entity` are only unambiguous under the schema they belong to, and `Field` flat beside
/// the kernel's types would read as a kernel concept, which is the one thing A-11 says a
/// registered schema is not.
pub use smysl_core::schemas;

// ---- check ----------------------------------------------------------------
pub use smysl_check::{
    check, check_and_fail_on, conformance, fidelity, granularity_distribution, CheckOptions,
    ConformanceClass, ConformanceVerdict, ConsumerProfile, FidelityReport, Pass,
};

// ---- pack -----------------------------------------------------------------
pub use smysl_pack::{
    pack, verify as verify_pack, Constraints, Estimator, Pack, PackError, PackRequest, Reason,
    Selection, Violation,
};

// ---- thread ---------------------------------------------------------------
// `Role` and `ThreadSchema` are wire format and come from the kernel above.
pub use smysl_thread::definition as schema_definition;
pub use smysl_thread::{
    derivable, derive_thread, role_weight, role_weights, salience_seed, satisfies_rule_l,
    DeriveOptions, DeriveReport, Matcher, SchemaDef,
};

// ---- render ---------------------------------------------------------------
pub use smysl_render::{
    build as build_ir, emit as emit_artifact, Artifact, Backend, Block, BuildOptions, Connectives,
    Contentions, Ir, LodPlan, Note, NoteKind, Person, Profile, Provenance, Register, RenderError,
    RenderMeta, Show, StatusDisplay, Target, Verbosity,
};

// ---- graph ----------------------------------------------------------------
pub use smysl_graph::compact::{compact, Compacted};
pub use smysl_graph::relink::{relink, Relinked};
pub use smysl_graph::{
    closure, cycles, dependents, dependents_via, diff, effective_status, hop_diff, label_bindings,
    label_index, labels_of, membership, merge, plan_retraction, rebuttals_of, resolve_label,
    rests_on, reverse_closure, review, review_with, salience, topo, trace, trace_via, view_roots,
    Adjacency, AgentActivity, AppendReport, BundleReport, Cached, DetectionContext, Edge, EdgeKind,
    EdgeSet, EffectiveStatus, Entry, HopDiff, Index, IndexError, LabelError, Lineage, LineageNode,
    MergeError, MergeOptions, MergeReport, NodeId, OpenReport, RecipeChange, RecipeChangeKind,
    RetractionAuthority, RetractionPlan, RetractionPolicy, ReviewItem, ReviewOptions,
    ReviewSubject, SalienceReport, SalienceRequest, SalienceTerms, SalienceWeights, Scratch, Store,
    StoreDiff, StoreOptions, SupersessionPolicy, TopoOrder, TraceKind, UnknownRecords, Via,
};

/// Proposition classes (SMYSL-2.3 A-12.4, SMYSL-2.4 §3.6).
///
/// `strict` is the one policy whose answer is normative, because the number of classes is the
/// number of propositions, and every corpus measure is divided by it. Re-exported as a module
/// rather than flattened: `Policy` and `Class` are common words, and `proposition::Policy` beside
/// `part::Policy` is the pair that would otherwise need renaming.
///
/// It lives in `smysl-graph` and not in `smysl-text`, where SMYSL-2.4's tree puts the engines
/// that propose the edges — counting the classes needs only a `Store`, and `smysl-check` has to
/// reach it: `C-Library` is obliged to derive `strict` classes and may not depend on
/// `smysl-text`.
pub use smysl_graph::proposition;

// ---- retrieve -------------------------------------------------------------
// Pure, and deliberately so: the default engine is BM25 with one transitive dependency, no
// model and no runtime, so retrieval is a bit-reproducible function of the store and the
// query. `Retriever` is the seam an impure semantic backend would sit behind.
/// The analyzer and postings seams (1.10, SMYSL-2.4 §4.3.5).
///
/// Re-exported because `find --analyzer` is the command that names an analyzer and `find
/// --library` is the command that scores a table it did not build — rule A says both reach them
/// through here. `analyze` comes as a module: its `ID_PLAIN` would be a second `ID_PLAIN` beside
/// `smysl_text::analyze`'s, and the two name different analyses.
pub use smysl_retrieve::{analyze as retrieve_analyze, Analyze, Bm25View, MemPostings, Postings};
pub use smysl_retrieve::{
    fold_suffix, tokenize as retrieve_tokenize, Bm25, Hit, PayloadFilter, Query, Retriever,
    Tokenizer,
};

// ---- semantic retrieval (feature-gated) -----------------------------------
// Impure, and outside the pure crates on purpose: it needs a model, and a model is something
// outside the format deciding the answer. Same tier as the providers, for the same reason.
#[cfg(feature = "semantic")]
pub use smysl_embed::{Hybrid, Model as EmbedModel, Semantic};

// ---- ingest / providers (feature-gated) -----------------------------------
#[cfg(feature = "stage")]
pub use smysl_ingest::ceiling::ceiling;
#[cfg(feature = "ingest")]
pub use smysl_ingest::path::choose as choose_ingest_path;
#[cfg(feature = "stage")]
pub use smysl_ingest::recipe::short as recipe_short;
#[cfg(feature = "ingest")]
pub use smysl_ingest::{
    attest, AttestOptions, AttestReport, IngestOptions, IngestPath, IngestReport, Ingestor,
    Judgement, LangPolicy, PivotLang, What, DEFAULT_REPAIR_ATTEMPTS,
};
// Consensus extraction (§3.4). `Passes` is `IngestOptions::with_passes`'s argument, so without
// it the option is set-able only by a caller who depends on `smysl-ingest` directly — the rule A
// violation the `import` block below records. `Agreement` is what `IngestReport` hands back,
// and the module itself for `consensus::agreement`, which a caller measuring GE-T2 over a
// corpus it already extracted needs and no ingest run can give it.
#[cfg(feature = "ingest")]
pub use smysl_ingest::{consensus, Agreement, Passes};
// Windowing and span attachment (§3.4, TX-P5 step 3). The module rather than its items: a
// caller does not pick one of these out — windowing produces the `TextWindow` that attachment
// consumes, and the two are used together or not at all. Gated on both features because that
// is what the code needs: `ingest` for the crate and `text` for the library layer a window is
// cut from.
#[cfg(all(feature = "ingest", feature = "text"))]
pub use smysl_ingest::text as ingest_text;
// The text ingest itself (§4.2, TX-P5 step 4). Rule A: `smysl ingest --text` is built out of
// these four names and nothing else, so a library consumer can do what the CLI does.
#[cfg(all(feature = "ingest", feature = "text"))]
pub use smysl_ingest::text::{Target as TextTarget, TextIngest, TextReport};
// Staging same-as proposals (§3.6, TX-P7 step 2). The module rather than its items, for the
// same reason `ingest_text` is: `propose` returns a `Proposals` whose fields are the thing a
// caller reads, and the engines it is given live in `text::proposition`.
#[cfg(all(feature = "ingest", feature = "text"))]
pub use smysl_ingest::same_as;
#[cfg(feature = "stage")]
pub use smysl_ingest::{stage, Attesting, Staged};
// The answer step of `ask --answer` (TX-P8 step 3). The module, for `Member`, `render` and
// `admit` beside `Ingestor::answer`: a caller builds the lines from its own dossier, and rule A
// says the CLI may not reach past the facade to do it.
#[cfg(feature = "ingest")]
pub use smysl_ingest::answer;
// `smysl import` is the only producer of `measured` units and the only unit-producing command
// that consults no model. Until 0.13 `cmd_import` reached into `smysl_ingest::import` directly
// and none of these three names was re-exported, so a consumer holding the facade could not do
// what the command does — a rule A violation that stood because nothing checked rule A.
// `Imported` is here because it is `from_csv`'s return type: without it the function is
// callable and its result unnameable.
#[cfg(feature = "stage")]
pub use smysl_ingest::import::{from_csv, ImportOptions, Imported};
// A caller's own extraction prompt and schema. Here rather than left in the hidden `prompt`
// module because the point of it is library use: a pipeline that wants its own question but
// the quote check, rule T and staging that `ingest` applies to the answer. `resolve_prompt`,
// which this replaces, was documented as a hook to override and could not be.
#[cfg(feature = "ingest")]
pub use smysl_ingest::prompt::PromptOverride;
// The quote check, for a caller that builds units itself and sends them to `stage::prepare`
// rather than through `Ingestor`. Without it the choice was reimplementing it — a second
// definition of "loose" and "absent" drifting from the first. Its normalisation is stated in
// full on `smysl_core::quote`, because it is now part of what a minor version may not change.
// Ungated since 1.3: it does no I/O, so it lives in core and needs no feature.
pub use smysl_core::quote::{
    support as quote_support, support_in as quote_support_in,
    support_in_span as quote_support_in_span, support_span as quote_support_span,
    support_span_with as quote_support_span_with, support_with as quote_support_with,
    Normaliser as QuoteNormaliser, Support as QuoteSupport, QUOTE_KEY,
};
// The script check, on the same reasoning as the quote check above and for the same caller: a
// pipeline that builds its own units still wants to know whether the model answered in the
// language it was asked in. Ungated — it is a table of code points and does no I/O.
pub use smysl_core::lang::{
    script_of, script_of_lang, verify as lang_verify, Script, Scripts, ABSENT_PERCENT, MIN_LETTERS,
};
/// The string-valued entries of a unit's payload (1.6). What `Query::with_payload` filters on, and
/// what a caller reads to answer "what kind of thing does this extension schema say this is".
pub use smysl_core::surface::payload::payload_strings;
#[cfg(feature = "providers")]
pub use smysl_provider::usage::{GroupBy, Totals};
#[cfg(feature = "providers")]
pub use smysl_provider::{
    config::ProviderConfig, map::build as build_provider, Capabilities, Completion, Ledger,
    LedgerEntry, Message, Probe, Provider, ProviderConfigFile, ProviderError, ProviderId, Registry,
    Request, StreamMsg, StructuredMode, Task, TokenCount, Usage,
};

/// The terminal browser, whole, under the feature that brings it in.
///
/// Re-exported as a module rather than name by name because the browser is one capability with
/// several entry points — `run`, `App`, `render_to_string` for testing a frame without a
/// terminal — and picking a few of them would be the same rule A gap in a smaller form. The
/// `ratatui` and `crossterm` cost stays behind `--features tui`, exactly as before.
#[cfg(feature = "tui")]
pub use smysl_tui as tui;

/// Measures as derived data units (RFC SMYSL-2.6): recipes, the text-level measures T1–T7, and
/// the unit writer.
///
/// Rule A: `measure` reaches the crate through this name and no other. A module for the reason
/// `query` is one — the command needs the registry, the recipe, the text substrate and the
/// unit writer, which is most of the crate. Behind `text`: every measure this build has reads a
/// library's text.
#[cfg(feature = "text")]
pub use smysl_measure as measure;
/// The query language (RFC SMYSL-2.9) and its planner (SMYSL-2.5 §3.2), whole.
///
/// Rule A: `src/main.rs` reaches every crate through this file, and `q` needs the parser, the
/// planner, the executor, both scope implementations and the explain writers — most of the
/// crate's surface. Re-exported as a module rather than name by name for the reason `text` is.
pub use smysl_query as query;
/// The library layer: a catalog and an object store, the readers, and everything that
/// *interprets* the records [`Manifest`] and friends describe (RFC SMYSL-2.4 §4.3.6).
///
/// A module rather than a list of names, for the reason `tui` is one: a library is one
/// capability with several entry points, and `Library` without `Locator`, `Structure` or the
/// reader registry would be rule A's gap in a smaller form.
///
/// Behind `text`, which `cli` turns on. Not in the library's default tree, deliberately: the
/// readers bring `quick-xml`, `pulldown-cmark` and `serde_json`, and the claim that a
/// `--no-default-features` build of this crate carries no serde stack is one the purity gate
/// checks. An embedder who wants to read files asks for it.
#[cfg(feature = "text")]
pub use smysl_text as text;

/// The crate version, as a convenience for embedders recording provenance.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facade_reexports_the_diagnostic_registry() {
        // 51 as of 0.6.0: `SMY-W306` was deleted rather than emitted. It described a usage
        // threshold that does not exist and never did, and had sat "documented as
        // unreachable" for two releases — which is a holding pattern, not a decision. A code
        // nobody can trigger is worse than a missing one, because a reader waits for it.
        // 52 as of 1.3.0, with `SMY-W309`; 53 as of 1.4.0, with `SMY-W056`; 54 as of 1.6.0,
        // with `SMY-E203`; 55, 56 and 57 as of 1.7.0, with `SMY-W111`, `SMY-W057` and
        // `SMY-W058`; 58 as of 1.9.0, with `SMY-W409` — the four enumerations opened in 1.9
        // survive an unknown code, and this is how a reader hears that they met one. 1.10
        // opened the fifth, `admission`, and needed no new code for it: the warning already
        // said what it means, which is what an open enumeration costs when the first four were
        // done properly. 59 with
        // `SMY-W434`: a bundle keeps records it cannot interpret, and the sender is told. 60
        // with `SMY-W432`, the warning that has to ship a release before `lang` becomes a core
        // key, because the lint is the whole migration. 61 with `SMY-W435`: a chunk that
        // succeeded on a later attempt used to throw the earlier error away, which is the one
        // a reader needs to know the model got it wrong first. 62 with `SMY-W433`: an input
        // carrying a marker this crate sends is reported rather than assumed innocent — the
        // fence it would collide with is derived per input, so the collision is survivable, and
        // a reader still wants to know the document tried. 63 with `SMY-W025`: a granularity
        // profile may now name the estimator its bounds are counted with (F-2), and one this
        // build does not have makes `l0_max` unevaluable rather than evaluable under some
        // other count. 64 as of 1.10.0, with `SMY-W436`: `LangPolicy::Source` says a gist is
        // written in the language of its passage, and nothing checked it — the S0 spike found a
        // model answering Russian passages in English in 78.6% of its gists, staged without a
        // remark. The check is by script rather than by language, and by the passage rather than
        // by a declared tag, because `View::lang` defaults to `en` and ingest never sets it.
        // 69, also 1.10.0, with the library's first five: `SMY-E401`, `SMY-E402`, `SMY-E440`,
        // `SMY-E445` and `SMY-E446`, raised by `smysl-text` (RFC SMYSL-2.4, TX-P1 step 2).
        // Five of that RFC's twelve allocations, because these are the five this build can
        // trigger — the rest arrive with the readers, the ingest path and the check pass that
        // raise them. 70 with `SMY-E452`, which TX-P1 step 4 made raisable: a log offered a
        // record 15 or 18 refuses it, because text lives in the object store (OQ-39). 72 with
        // `SMY-E403` and `SMY-W418`, raised by step 5's `Library` pass. Step 5's plan named
        // four more and two of those read `SourceRef.span`, which arrives in TX-P5, so
        // `SMY-E404` and `SMY-W405` are not registered here: a code nothing can raise is the
        // one thing this count is kept for. 73 with `SMY-E450`, which TX-P2 step 3 made
        // raisable — `text append` to a pseudonymised expression with no key — and which step 2
        // deliberately left out for exactly this reason: `--pseudonymise` existed, and nothing
        // could refuse an append until there was an append. 74 with `SMY-E410`, which TX-P3
        // step 1 made raisable: `@date` and `source { published: … }` both carry EDTF text, so
        // the surface parser now has two places to meet a malformed value. The three other
        // codes that step allocates — `SMY-W411`, `SMY-W412`, `SMY-W413` — and `SMY-W449` all
        // need an interval computed from that text, and nothing computes one before step 2.
        // 76 as of step 2, with `SMY-W412` and `SMY-W413`: the time engine exists, so a live
        // dating that did not move a bound and a set of records that cannot all be true are
        // both things something can now say. They are also A-8.2's two `DetectionKind` values,
        // and `DetectionKind::code()` is total, so kind and code had to land together.
        // 78 as of step 3, with `SMY-W411` and `SMY-W449`, which the `Time` check pass raises:
        // an `observed` outside the `published` interval beside it, and an EDTF value outside
        // the millisecond range, indexed as an open bound rather than clamped. That is all of
        // RFC SMYSL-2.4's range but `SMY-E404` and `SMY-W405`, which read `source.span`.
        // 79 as of TX-P4 step 4, with `SMY-W451`: `text align` exists, so a locator an
        // alignment scheme does not cover is a finding something can make. Step 1 could not
        // have made it — there was no command that aligned two expressions, and the code would
        // have sat in the registry unraisable, which is what this count is kept for.
        // 81 as of TX-P5 step 1, with `SMY-E404` and `SMY-W405`: FC-3 gives a unit a `span` and
        // the manifest it is an offset into, so the library pass can now compare a span against
        // the part it names and a locator against the span beside it. Both were allocated in
        // TX-P1 step 5 and left unregistered on purpose — the field they read did not exist —
        // which is what this count is for.
        // 84 as of TX-P5 step 3, with `SMY-W419`, `SMY-W441` and `SMY-W447`: windowing a
        // reading and attaching a span to a quote exist, so a node named twice over, a quote
        // the window does not hold and a node too large to show at once are all findings
        // something can make. `W419` had been allocated since draft 3 with nothing behind it.
        // 87 as of TX-P5 step 4, with `SMY-E442`, `SMY-W443` and `SMY-W444`: a text ingest
        // sends a ledger digest, so a reference has a scope to be outside of; it fences the
        // digest as well as the window, so a marker inside one can be neutralised; and it
        // writes the provenance itself, so a field the answer had no business filling is
        // dropped rather than believed.
        // 88 with `SMY-W453`, raised by the pivot policy's pass: a model asked for one line per
        // unit and one line does not come back, so a unit stays in the language of its passage
        // under a recipe that says otherwise. Reported rather than left to a count, because the
        // silent version is the failure F-6's policy exists to rule out. The wrong-*language*
        // case takes no number of its own — it is `SMY-W436` measured against the language that
        // was asked for rather than the passage's.
        // 91 with `SMY-E427`, `SMY-E460` and `SMY-W510`, raised by the `sq/1` parser (TX-P6
        // step 1, RFC SMYSL-2.9): a `using` clause naming a version this build does not speak,
        // a syntax error or an unbound parameter, and a string literal that is not in NFC. The
        // last is a warning rather than an error because the grammar keeps a literal's bytes
        // exactly as written — a quote has to match a corpus byte for byte — and the hazard
        // that creates is a quote that looks right and matches nothing, which is worth saying
        // out loud rather than failing on.
        // 98 with `SMY-W464` (TX-P6 step 5): a supersession fork in scope, where the pass
        // declines to choose a head and keeps every successor.
        // 99 with `SMY-W415` (TX-P6 step 6): a unit excluded from a time-scoped result for
        // having no time on the axis asked. Draft 3 allocated it and nothing could raise it
        // until a query could name a time, and it is a count beside the answer rather than a
        // refusal because an undated unit did not fail the test — A-2.4 rule 5 makes it
        // neither early nor late, so it was never in the comparison.
        // 97 with `SMY-W463` and `SMY-E512` (TX-P6 step 4). `W463` is SMYSL-2.5's, withheld
        // until something walked a graph with a depth: a traversal that stopped at the depth
        // asked while the graph went on. `E512` is new: an operator that does not apply to the type of
        // the field it names, such as `>` on `kind`, which the grammar cannot refuse because a
        // grammar does not know a field's type.
        // 95 with `SMY-E461`, `SMY-E462`, `SMY-E466` and `SMY-E511`, raised by the planner
        // (TX-P6 step 2): a stage this build does not lower, a traversal naming a relation
        // kind nothing in scope declares, a `using` pin with no matching declaration in the
        // store, and a scope naming something the environment does not hold. The first three
        // were allocated by SMYSL-2.5 and withheld until a planner could raise them. `E421`
        // and `E422` are still absent for that same reason: field resolution needs declared
        // field types, which arrive with FC-9 in TX-P12.
        // 103 with `SMY-W417`, `SMY-E468`, `SMY-E477` and `SMY-W478` (TX-P8 step 3), raised by
        // `ask`: a subject naming two entities, a question no intent rule matches, a template
        // set this build does not carry, and an answer unit grounding outside its dossier.
        // 105 with `SMY-W483` and `SMY-E489` (TX-P8 M1), raised by `smysl-measure`'s unit writer
        // and registry, which the facade does not expose until the `measure` command (M7).
        // 108 with `SMY-W408`, `SMY-E480` and `SMY-W485` (TX-P8 M2), raised by T1 and T4.
        // 109 with `SMY-E484` (TX-P8 M3), raised by T6.
        // 111 with `SMY-E481` and `SMY-E482` (TX-P9 N1), raised by A2 and A3; 112 with
        // `SMY-E407` (TX-P9 N2), raised by A8; 114 with `SMY-W487` and `SMY-W488` (TX-P9
        // N6), raised by `assess score`; 118 with `SMY-W425`, `SMY-W490`, `SMY-W495` and
        // `SMY-W497` (TX-P11 step 2), raised by `smysl-lib`'s exporter; 119 with `SMY-E494`
        // (TX-P11 step 3), raised by its `verify`.
        assert_eq!(Code::ALL.len(), 119);
        assert_eq!(Code::E030.severity(), Severity::Error);
    }

    /// Rule A for `smysl import`, expressed as behaviour rather than as a grep.
    ///
    /// `cargo xtask check-purity` proves the CLI names no sibling crate. It cannot prove the
    /// facade can do what the CLI does — a `cmd_import` that stopped importing would satisfy
    /// it just as well. So this does the import through facade names only, which is what rule
    /// A actually promises a library consumer.
    ///
    /// Written when the gate found `cmd_import` reaching into `smysl_ingest::import` for the
    /// CSV reader. `Imported` is named deliberately: it is `from_csv`'s return type, and
    /// re-exporting the function without it would leave the result unnameable.
    #[cfg(feature = "stage")]
    #[test]
    fn the_import_capability_is_reachable_from_the_facade() {
        let agent = AgentId::new("tool:test").unwrap();
        let opts = ImportOptions::new("latency.csv", agent.clone(), Hlc::zero(agent));
        let out: Imported = from_csv("host,ms\nweb-1,12\nweb-2,31\n", &opts);

        assert_eq!(out.units.len(), 2, "one unit per row");
        assert!(!out.is_empty());
        // The attestation is the licence for `measured`, not a decoration — a unit without
        // one would not be permitted the status at all.
        assert_eq!(
            out.attestations.len(),
            out.units.len(),
            "every imported unit carries its `op: Imported` attestation"
        );
        assert!(
            out.units.iter().all(|u| u.status == Status::Measured),
            "import is the only producer of `measured`; that is what it is for"
        );
    }

    #[test]
    fn facade_reexports_version_constants() {
        // Every declared version must be accepted, not merely the first. The list grew to two
        // in 0.14 and an assertion about `[0]` would have kept passing while the second went
        // unchecked — which is the shape of gap this repository keeps finding.
        for v in FORMAT_VERSIONS_SUPPORTED {
            assert!(
                format_version_supported(v),
                "`{v}` is declared but not accepted"
            );
        }
        assert!(
            FORMAT_VERSIONS_SUPPORTED.contains(&FORMAT_VERSION_DEFAULT),
            "the version we write must be one we can read"
        );
        assert_eq!(kernel_major(KERNEL_SCHEMA), Some(KERNEL_MAJOR));
    }

    /// A tripwire, not a tautology. `VERSION` is `env!("CARGO_PKG_VERSION")`, so it cannot
    /// fail to be the cargo version — what the literal pins is *which release this is
    /// meant to be*, so a bump has to be an edit somebody made on purpose rather than
    /// something that drifted in with a dependency update. Update it when you bump the
    /// manifest, and the diff will say what you decided.
    #[test]
    fn the_crate_version_is_the_one_we_intend_to_ship() {
        assert_eq!(VERSION, "1.10.0");
    }

    /// A crate major bump MUST NOT imply a format break, and vice versa (§11). The two
    /// axes are independent, so they are asserted independently.
    #[test]
    fn crate_and_format_versions_are_independent_axes() {
        assert_ne!(VERSION, FORMAT_VERSIONS_SUPPORTED[0]);
    }

    /// The canonical consumer shape of §12.2, as far as SM-P1 reaches: build a core,
    /// hash it, encode it, read it back.
    #[test]
    fn a_unit_round_trips_through_the_facade_alone() {
        let core = UnitCoreBuilder::new(
            KernelType::Claim,
            "p95 auth latency tripled",
            Status::Speculative,
        )
        .build()
        .unwrap();
        let uid = canonical_uid(&core);
        let bytes = to_cbor(&Record::Unit(core.clone()));
        let (decoded, n) = from_cbor(&bytes).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(decoded.as_unit(), Some(&core));
        assert!(verify(decoded.as_unit().unwrap(), &uid).is_ok());
    }
}
