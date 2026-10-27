//! The `smysl` binary - a thin shell over the library (rule A, principle P8).
//!
//! Every subcommand here dispatches into `smysl::*`. Nothing is implemented in the
//! binary that an embedder cannot reach from the library, which is the cheapest way to
//! keep the API honest.
//!
//! SM-P0 declares the full command surface of §23 and reports, per command, the phase
//! that wires it up. Commands are wired only once their library API has stabilised.

mod progress;

use std::io::{Read, Write};
use std::process::ExitCode as ProcExitCode;

use clap::{Arg, ArgAction, ArgMatches, Command};

use progress::{Bar, Spinner, Style};
use smysl::ExitCode;
use smysl::{
    check, conformance, effective_status, fidelity, granularity_distribution, merge, parse_surface,
    plan_retraction, write_surface, AgentId, AnyStore, BuildOptions, CheckOptions, Code,
    ConformanceClass, ConsumerProfile, Contentions, DeriveOptions, Estimator, Hlc, Lod,
    MergeOptions, PackRequest, Pass, Profile, ProfileEstimator, Record, RelKind, Relation,
    RetractionAuthority, RetractionPolicy, Role, SalienceRequest, SalienceWeights, SchemaId,
    Severity, Store, StoreOptions, StoreRead, SupersessionPolicy, Target, TokenEstimator,
    TraceKind, Uid, UidPrefix, View, ViewId, WriteContext,
};

/// Purity classification of a command (§23). `Pure` commands are bit-reproducible
/// functions of their inputs (rule D); `Model` commands are the only egress points.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purity {
    Pure,
    Mixed,
    Model,
}

impl Purity {
    fn tag(self) -> &'static str {
        match self {
            Purity::Pure => "pure",
            Purity::Mixed => "mixed",
            Purity::Model => "model-dependent",
        }
    }
}

/// Is *this* invocation bit-reproducible, in rule D's sense?
///
/// `--seed-check` was declared global, advertised on all 26 commands, and read by nothing: a
/// caller asserting reproducibility got the assertion accepted and never checked, which is the
/// worst shape for a flag whose entire job is to be a check.
///
/// The table's label is not the answer on its own, because a `Mixed` command is pure in most of
/// its invocations — `find` is lexical unless asked otherwise. So the label is refined by the
/// flags, and the refinement lives next to `impure_when`, the prose that tells a reader the
/// same thing.
///
/// `None` for an unrecognised `Mixed` command: a command that is mixed and whose impure
/// invocations this function cannot name must be refused, not assumed pure. The census test
/// asserts no such command exists, which is what keeps the two lists together.
fn reproducible(cmd: &Cmd, sub: &ArgMatches) -> Option<bool> {
    match cmd.purity {
        Purity::Pure => Some(true),
        // `ingest` and `attest` ask a model for a judgement. Nothing refines that.
        Purity::Model => Some(false),
        Purity::Mixed => match cmd.name {
            // Pure in every invocation *today*: `thread` is classified mixed as a reservation
            // for `--refine`, and that flag is not wired, so no argument can reach a model.
            // The manual states the reasoning — the label describes what the command is
            // permitted to become, so nothing downstream is re-audited the day the flag lands —
            // and a reservation is exactly the thing `--seed-check` must not treat as a fact
            // about this run. The arm changes when the flag arrives.
            "thread" => Some(true),
            // Lexical ranking is a function of the store. Semantic and hybrid are functions of
            // a model file outside it, and float kernels are not promised bit-identical across
            // platforms, so rule D does not hold for them.
            "find" => Some(!embeds(sub)),
            // `pack` reads `--engine` only inside its `--query` branch, so a pack with no query
            // cannot reach an embedding however the engine is set.
            "pack" => Some(!(sub.get_one::<String>("query").is_some() && embeds(sub))),
            // `link --candidates` lists what the generator chose and stops there, which is a
            // function of the store. Every other invocation asks a model about those pairs.
            "link" => Some(sub.get_flag("candidates")),
            // Mixed, and judged on the plan rather than the flags. SMYSL-2.5 §3.2 makes a plan
            // model-dependent when any step is — `~sem`, a `~hybrid` routed semantic, `near`, the
            // embedding bridge — and since TX-P10 step 4 those lower when `--engine-model` or
            // `--candle-model` binds a source. A model flag does not make a lexical query
            // impure, and a semantic stage is refused without one, so the flags cannot decide
            // it: `cmd_q` asks `Plan::seed_check` after planning and refuses with `SMY-E416`
            // before anything runs.
            "q" => Some(true),
            // The dossier is a function of the question and the store: a rule table, a fixed
            // template set and pure plans. `--answer` is the one invocation that asks a model.
            "ask" => Some(!sub.get_flag("answer")),
            // Mixed for what the registry may come to hold — T8 and D5's embeddings call a
            // model (SMYSL-2.6 §4.4) — and pure for what it holds: every measure this build
            // registers is `Purity::Pure`, which `cmd_measure` checks per id before it runs one.
            // `SMY-E416` is the refusal `q` raises for a model-dependent plan (TX-P10 step 4); a
            // model-dependent id would take it too, and the registry holds none yet.
            "measure" => Some(true),
            // Mixed for what SMYSL-2.6 §4.4 gives it — `key --generate` writes questions with a
            // model and `score --judge model` asks one — and pure for what this build has: a
            // key from a view, the accepted, lexical and anchored judges, and the diff.
            "assess" => Some(true),
            _ => None,
        },
    }
}

/// Does this invocation's engine consult an embedding model? `--engine` defaults to lexical.
fn embeds(sub: &ArgMatches) -> bool {
    matches!(
        sub.get_one::<String>("engine").map(String::as_str),
        Some("semantic") | Some("hybrid")
    )
}

struct Cmd {
    name: &'static str,
    about: &'static str,
    purity: Purity,
    /// The delivery phase that wires this command to its library API.
    phase: &'static str,
    /// Which invocations of a `Mixed` command are the impure ones.
    ///
    /// `mixed` alone is true and too coarse: a reader cannot tell whether the command they are
    /// about to run calls a model. The dispatcher decides per invocation under `--seed-check`;
    /// this is so the table a user actually reads says the same thing.
    impure_when: Option<&'static str>,
    /// The output forms this command can produce, in `--format`'s vocabulary.
    ///
    /// Empty means it does not emit a document at all: a report, a store written in place, an
    /// artifact with its own `--target`. `--format` is global, so all 26 commands advertised
    /// it and three read it; the other 23 accepted it and wrote whatever they were going to.
    /// A caller who asked for CBOR and got prose has no way to learn the flag was ignored
    /// until their parser fails, so the dispatcher refuses what a command cannot do.
    forms: &'static [&'static str],
}

/// Both forms a store can be written in.
const BOTH: &[&str] = &["surface", "cbor"];
/// A store log, which has no surface spelling: `compact`, `relink` and `import` write one.
const CBOR_ONLY: &[&str] = &["cbor"];
/// Not a document: a report, a store written in place, or an artifact with its own target.
const NO_DOCUMENT: &[&str] = &[];
/// Every form `lib export` names. The table cannot say which of `lib`'s actions writes a
/// document, so `cmd_lib` refuses `--format` for the other five, and the forms this build does
/// not write yet, itself.
const LIB_FORMS: &[&str] = &["surface", "cbor", "pack"];

/// The command table of §23, in table order.
#[rustfmt::skip]
const COMMANDS: &[Cmd] = &[
    Cmd { name: "fmt",       about: "Canonicalise surface text and verify the round-trip", purity: Purity::Pure,  phase: "SM-P2"  , impure_when: None , forms: BOTH },
    Cmd { name: "check",     about: "Run the check pipeline over a store",                 purity: Purity::Pure,  phase: "SM-P4"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "pack",      about: "Budget-bounded, closure-complete selection",          purity: Purity::Mixed, phase: "SM-P9"  , impure_when: Some("--query with --engine semantic|hybrid") , forms: BOTH },
    Cmd { name: "merge",     about: "Join-semilattice union; materialise contentions",     purity: Purity::Pure,  phase: "SM-P6"  , impure_when: None , forms: BOTH },
    Cmd { name: "diff",      about: "Partition uids across stores or hops",                purity: Purity::Pure,  phase: "SM-P7"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "trace",     about: "Walk provenance or evidential support",               purity: Purity::Pure,  phase: "SM-P7"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "view",      about: "Define or print a view",                              purity: Purity::Pure,  phase: "SM-P7"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "bundle",    about: "Emit the reachable closure of a view",                purity: Purity::Pure,  phase: "SM-P7"  , impure_when: None , forms: BOTH },
    Cmd { name: "thread",    about: "Derive, list, or show threads",                        purity: Purity::Mixed, phase: "SM-P11" , impure_when: Some("--refine, which is not yet wired") , forms: BOTH },
    Cmd { name: "salience",  about: "Report derived salience with per-term breakdown",     purity: Purity::Pure,  phase: "SM-P8"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "find",      about: "Rank units against a query, lexically",                purity: Purity::Mixed, phase: "0.5.0"  , impure_when: Some("--engine semantic|hybrid") , forms: NO_DOCUMENT },
    Cmd { name: "retract",   about: "Retract a unit; report the blast radius first",       purity: Purity::Pure,  phase: "SM-P6"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "withdraw",  about: "Withdraw an edge: kept, and no longer followed",       purity: Purity::Pure,  phase: "1.4.0"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "resolve",   about: "Record that a disagreement was reviewed",              purity: Purity::Pure,  phase: "1.4.0"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "review",    about: "List the disagreements open for review",               purity: Purity::Pure,  phase: "1.4.0"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "commit",    about: "Record how settled a unit is",                         purity: Purity::Pure,  phase: "1.7.0"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "render",    about: "Thread plus profile to artifact",                     purity: Purity::Pure,  phase: "SM-P12" , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "import",    about: "Tabular readings to measured units, without a model",  purity: Purity::Pure,  phase: "SM-P15" , impure_when: None , forms: CBOR_ONLY },
    Cmd { name: "relink",    about: "Re-point references onto superseded units",             purity: Purity::Pure,  phase: "SM-P15" , impure_when: None , forms: CBOR_ONLY },
    Cmd { name: "compact",   about: "Drop superseded units nothing needs; never in place",   purity: Purity::Pure,  phase: "SM-P15" , impure_when: None , forms: CBOR_ONLY },
    Cmd { name: "ingest",    about: "Prose or data to staged units",                       purity: Purity::Model, phase: "SM-P14" , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "attest",    about: "Semantic checks that require a model",                purity: Purity::Model, phase: "SM-P14" , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "providers", about: "List providers, capabilities, and what would egress", purity: Purity::Pure,  phase: "SM-P13" , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "usage",     about: "Token and cost ledger",                               purity: Purity::Pure,  phase: "SM-P13" , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "reindex",   about: "Rebuild the derived index from the log alone",        purity: Purity::Pure,  phase: "SM-P3"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "ui",        about: "Terminal UI",                                         purity: Purity::Pure,  phase: "SM-P15" , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "text",      about: "Library: add, append, list and show texts",           purity: Purity::Pure,  phase: "TX-P1"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "date",      about: "Write datings; show effective time and why",          purity: Purity::Pure,  phase: "TX-P3"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "same-as",    about: "Propose same-as edges; derive classes",               purity: Purity::Pure,  phase: "TX-P7"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "link",      about: "Judge candidate pairs; stage entails and contradicts", purity: Purity::Mixed, phase: "TX-P7"  , impure_when: Some("every action but --candidates, which only lists them") , forms: NO_DOCUMENT },
    Cmd { name: "summarise", about: "Grounded summary tree over a scope; staged",           purity: Purity::Model, phase: "TX-P7"  , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "q",         about: "Run an sq/1 query; explain it, or seed-check it",      purity: Purity::Mixed, phase: "TX-P6"  , impure_when: Some("a model-dependent plan (~sem, ~hybrid routed semantic, near, lang:any), with --engine-model or --candle-model") , forms: NO_DOCUMENT },
    Cmd { name: "ask",       about: "Experimental: a dossier for a question; optionally a model's grounded answer", purity: Purity::Mixed, phase: "TX-P8"  , impure_when: Some("--answer, which asks a model to answer from the dossier") , forms: NO_DOCUMENT },
    Cmd { name: "measure",   about: "Text, graph, channel and dialogue measures as derived data units", purity: Purity::Mixed, phase: "TX-P8"  , impure_when: Some("a model-dependent id (T8, D5's embeddings), none of which this build registers") , forms: NO_DOCUMENT },
    Cmd { name: "assess",    about: "Assessment keys, reader scores and knowledge states",  purity: Purity::Mixed, phase: "TX-P9"  , impure_when: Some("a generated key or a model judge (--generate, --judge model), neither of which this build has") , forms: NO_DOCUMENT },
    Cmd { name: "lib",       about: "Import smysl files into the library; export it as files", purity: Purity::Pure,  phase: "TX-P11" , impure_when: None , forms: LIB_FORMS },
    Cmd { name: "schema",    about: "Show schema revisions; check the naming rule and lens totality", purity: Purity::Pure,  phase: "TX-P12" , impure_when: None , forms: NO_DOCUMENT },
    Cmd { name: "migrate",   about: "Re-derive a schema's units under a newer revision; staged", purity: Purity::Pure,  phase: "TX-P12" , impure_when: None , forms: NO_DOCUMENT },
];

fn cli() -> Command {
    let globals = [
        Arg::new("config")
            .short('C')
            .long("config")
            .global(true)
            .help("Configuration file; must exist (default: <store dir>/.smysl/config.hjson)")
            .value_name("FILE"),
        Arg::new("store")
            .short('s')
            .long("store")
            .global(true)
            .help("Store path; `-` reads stdin (rule P)")
            .value_name("PATH"),
        Arg::new("output")
            .short('o')
            .long("output")
            .global(true)
            .help("Output path; defaults to stdout")
            .value_name("PATH"),
        Arg::new("format")
            .long("format")
            .global(true)
            .help("Output form; defaults to cbor on a non-TTY stdout (rule P)")
            .value_name("FORM")
            // `pack` is `lib export`'s alone (SMYSL-2.7 §4.4, OQ-62): a pack is a lossy
            // selection, and no other command's output can be one. The dispatcher refuses it
            // everywhere else, as it refuses any form a command does not write.
            .value_parser(["surface", "cbor", "pack"]),
        Arg::new("strict")
            .long("strict")
            .global(true)
            .help("Treat warnings as errors")
            .action(ArgAction::SetTrue),
        Arg::new("offline")
            .long("offline")
            .global(true)
            .help("Hard-fail rather than send anything off the machine")
            .action(ArgAction::SetTrue),
        Arg::new("no-color")
            .long("no-color")
            .global(true)
            .help("Disable colour")
            .action(ArgAction::SetTrue),
        Arg::new("noprogress")
            .long("noprogress")
            .global(true)
            .help("Disable progress bars, whatever the terminal is")
            .action(ArgAction::SetTrue),
        Arg::new("json")
            .long("json")
            .global(true)
            .help("Machine-readable output")
            .action(ArgAction::SetTrue),
        Arg::new("quiet")
            .short('q')
            .long("quiet")
            .global(true)
            .help("Suppress non-error output")
            .action(ArgAction::SetTrue),
        Arg::new("verbose")
            .short('v')
            .long("verbose")
            .global(true)
            .help("Increase verbosity")
            .action(ArgAction::Count),
        Arg::new("backend")
            .long("backend")
            .global(true)
            .help("Which store a CBOR log is read through; `redb` needs --features store-redb")
            .value_name("BACKEND")
            .value_parser(["memory", "redb"])
            .default_value("memory"),
        Arg::new("shard")
            .long("shard")
            .global(true)
            .help("Read only this shard of a library, with the others' references waiting (repeatable; `all` for every shard)")
            .value_name("ALIAS")
            .action(ArgAction::Append),
        Arg::new("seed-check")
            .long("seed-check")
            .global(true)
            .help("Assert this invocation is bit-reproducible (rule D)")
            .action(ArgAction::SetTrue),
    ];

    let mut cmd = Command::new("smysl")
        .version(smysl::VERSION)
        .about("An AI<->AI<->Human data interchange format, library, and CLI")
        .long_about(format!(
            "smysl {}  -  format {}, kernel {}",
            smysl::VERSION,
            smysl::FORMAT_VERSIONS_SUPPORTED.join(", "),
            smysl::KERNEL_SCHEMA
        ))
        .subcommand_required(true)
        .arg_required_else_help(true)
        .args(globals);

    for c in COMMANDS {
        let mut sub = Command::new(c.name)
            .about(c.about)
            .after_help(match c.impure_when {
                // `mixed` alone makes a reader assume the worst of every invocation, which is
                // its own kind of wrong: `find` is pure unless asked for a model.
                Some(w) => format!("Purity: {} (pure except {w})", c.purity.tag()),
                None => format!("Purity: {}", c.purity.tag()),
            });
        sub = match c.name {
            "fmt" => sub
                .arg(
                    Arg::new("check")
                        .long("check")
                        .help("Exit 3 if reformatting would change bytes")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("write")
                        .long("write")
                        .help("Rewrite files in place")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("files")
                        .num_args(0..)
                        .value_name("FILE")
                        .help("Files to format; `-` or none reads stdin (rule P)"),
                ),
            "check" => sub
                .arg(
                    Arg::new("library")
                        .long("library")
                        .help(
                            "Treat the store as a library root and resolve its parts, so the \
                             library pass can verify objects",
                        )
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("conformance")
                        .long("conformance")
                        .value_name("CLASS")
                        .help("Assert the store is consumable at a conformance class"),
                )
                .arg(
                    Arg::new("as")
                        .long("as")
                        .value_name("SCHEMA")
                        .action(ArgAction::Append)
                        .help("Report fidelity for a consumer implementing these schemas"),
                )
                .arg(
                    Arg::new("granularity")
                        .long("granularity")
                        .help("Report the granularity distribution of the store")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("estimator")
                        .long("estimator")
                        .value_name("ID")
                        .help("Count the granularity bounds with this estimator (F-2)"),
                )
                .arg(
                    Arg::new("pass")
                        .long("pass")
                        .value_name("NAME")
                        .action(ArgAction::Append)
                        .help("Run only these passes"),
                )
                .arg(
                    Arg::new("files")
                        .num_args(0..)
                        .value_name("FILE")
                        .help("Stores to check; `-` or none reads stdin (rule P)"),
                ),
            "diff" => sub
                .arg(
                    Arg::new("hop")
                        .long("hop")
                        .value_name("A..B")
                        .help("Partition units across a hop range instead of comparing stores"),
                )
                .arg(
                    Arg::new("by-agent")
                        .long("by-agent")
                        .help("Attribute each change to the agents responsible")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("recipe")
                        .long("recipe")
                        .help("Flag whether the prompt changed or the content did (D-8)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("inputs")
                        .num_args(1..)
                        .required(true)
                        .value_name("STORE")
                        .help("One store for --hop, two to compare"),
                ),
            "trace" => sub
                .arg(Arg::new("uid").required(true).value_name("UID"))
                .arg(
                    Arg::new("via")
                        .long("via")
                        .value_name("KINDS")
                        .help(
                            "Walk what a unit rests on over these edges: a preset (premises, \
                             dependency, support) or relation kinds, comma-separated",
                        ),
                )
                .arg(
                    Arg::new("depth")
                        .long("depth")
                        .value_name("N")
                        .help("How far back to walk"),
                )
                .arg(
                    Arg::new("parents")
                        .long("parents")
                        .help("Causal: attestation parents and supersession")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("grounds")
                        .long("grounds")
                        .help("Evidential: grounds and deps")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("both")
                        .long("both")
                        .help("Both walks at once")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("agents")
                        .long("agents")
                        .help("Name the agents behind each step")
                        .action(ArgAction::SetTrue),
                )
                .arg(Arg::new("store").value_name("PATH")),
            "text" => sub
                .arg(
                    Arg::new("action")
                        .required(true)
                        .value_name("ACTION")
                        .value_parser([
                            "add", "append", "ls", "show", "align", "redact", "index",
                        ])
                        .help(
                            "add a file, append a new version of one, list the catalog, \
                             show a passage, align two numberings, redact a part, or report \
                             and rebuild the index",
                        ),
                )
                .arg(
                    // `TARGET` rather than `FILE|REF`, which is what it is: `make
                    // cli-surface` records a positional by matching `[A-Z.]+` inside
                    // brackets, so a value name with a `|` in it is recorded as nothing and
                    // the gate goes blind to the argument. Found by reading the regenerated
                    // file rather than by a failure, because a missing line is what the gate
                    // records as correct.
                    Arg::new("target")
                        .value_name("TARGET")
                        .help(
                            "the file for `add` and `append`, `<alias|mid>#<locator>` for \
                             `show`, the first expression for `align`, or a tid for `redact`",
                        ),
                )
                .arg(
                    Arg::new("other")
                        .value_name("OTHER")
                        .help("The second expression, for `align`"),
                )
                .arg(
                    Arg::new("reader")
                        .long("reader")
                        .value_name("R")
                        .help("Reader id, such as `txt/1` or `osis/1` (`add`)"),
                )
                .arg(
                    Arg::new("versification")
                        .long("versification")
                        .value_name("S")
                        .help(
                            "The numbering this expression uses, such as `kjv` or `synodal` \
                             (`add`); carried forward by `append`",
                        ),
                )
                .arg(
                    Arg::new("scheme")
                        .long("scheme")
                        .value_name("FILE")
                        .help("A versification map to install and align by (`align`)"),
                )
                .arg(
                    Arg::new("alias")
                        .long("alias")
                        .value_name("A")
                        .help("The alias this expression is catalogued under (`add`, `append`)"),
                )
                .arg(
                    Arg::new("lang")
                        .long("lang")
                        .value_name("L")
                        .help("BCP-47 tag, when the source does not say"),
                )
                .arg(
                    Arg::new("licence")
                        .long("licence")
                        .value_name("SPDX")
                        .help("The licence the text is under (`add`)"),
                )
                .arg(
                    Arg::new("carry")
                        .long("carry")
                        .value_name("MODE")
                        .value_parser(["none", "ref", "text"])
                        .help("Whether the text itself may travel with a bundle"),
                )
                .arg(
                    Arg::new("part-policy")
                        .long("part-policy")
                        .value_name("P")
                        .help("The boundary rule and size targets parts are cut by"),
                )
                .arg(
                    Arg::new("param")
                        .long("param")
                        .value_name("K=V")
                        .action(ArgAction::Append)
                        .help("A reader parameter, recorded in the manifest beside the reader id"),
                )
                .arg(
                    Arg::new("pseudonymise")
                        .long("pseudonymise")
                        .help("Replace each speaker with a keyed pseudonym (`add`, `append`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("as")
                        .long("as")
                        .value_name("AGENT")
                        .help("The agent issuing the redaction (`redact`)"),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MS")
                        .help("Wall clock for the record, in milliseconds (`redact`)")
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(
                    Arg::new("reason")
                        .long("reason")
                        .value_name("UID")
                        .help("A unit saying why (`redact`)"),
                )
                .arg(
                    Arg::new("forks")
                        .long("forks")
                        .help("List only the aliases with more than one head (`ls`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("rebuild")
                        .long("rebuild")
                        .help("Rebuild the index from the catalog and write it (`index`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("raw")
                        .long("raw")
                        .help("Print the passage's bytes with no heading (`show`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("segments")
                        .long("segments")
                        .help("Print the segments the passage covers (`show`)")
                        .action(ArgAction::SetTrue),
                ),
            "date" => sub
                .arg(
                    Arg::new("action")
                        .required(true)
                        .value_name("ACTION")
                        .value_parser(["set", "order", "show"])
                        .help(
                            "write a dating, order two subjects (sugar for a relative \
                             `set`), or print effective time",
                        ),
                )
                .arg(
                    // `TARGET` and not `UID|TID|MID`: `make cli-surface` records a positional
                    // by matching `[A-Z.]+` inside brackets, so a value name with a `|` in it
                    // is recorded as nothing and the gate goes blind to the argument. The same
                    // trap `text`'s positional carries a comment about.
                    Arg::new("target")
                        .required(true)
                        .value_name("TARGET")
                        .help("A unit (uid or label), a part (`t3:`) or a manifest (`m3:`)"),
                )
                .arg(
                    // `date order <a> before <b>` is three positionals, and the middle one is
                    // the relation rather than a fixed word. Allen's seven spellings all read
                    // as English here, and restricting the sugar to `before` would mean
                    // `--value during:<b>` were the only way to say the commonest of the other
                    // six.
                    Arg::new("relation")
                        .value_name("RELATION")
                        .value_parser(smysl::Allen::ALL.iter().map(|a| a.as_str()).collect::<Vec<_>>())
                        .help("Allen relation, for `order`"),
                )
                .arg(
                    Arg::new("other")
                        .value_name("OTHER")
                        .help("The second subject, for `order`"),
                )
                .arg(
                    Arg::new("axis")
                        .long("axis")
                        .value_name("A")
                        .value_parser(["said", "composed", "about"])
                        .default_value("said")
                        .help("Which of a unit's times this is about"),
                )
                .arg(
                    Arg::new("value")
                        .long("value")
                        .value_name("V")
                        .help(
                            "An EDTF date, `offset:MS`, or `<allen>:<target>` (`set`)",
                        ),
                )
                .arg(
                    // The surface form spells a window as `window: [from, to]` beside the
                    // tid, so the flag does too rather than inventing a `<tid>@a..b`
                    // compound. One spelling per thing, in the document and on the
                    // command line.
                    Arg::new("window")
                        .long("window")
                        .value_name("FROM..TO")
                        .help("Narrow a part target to the units in a millisecond range"),
                )
                .arg(
                    Arg::new("basis")
                        .long("basis")
                        .value_name("UID")
                        .help("The unit giving the evidence; its status is the dating's"),
                )
                .arg(
                    Arg::new("as")
                        .long("as")
                        .value_name("AGENT")
                        .help("The agent issuing the dating (`set`, `order`)"),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MS")
                        .help("Wall clock for the record, in milliseconds")
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(
                    Arg::new("why")
                        .long("why")
                        .help("Print the chain that set each bound (`show`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Report what would be written, and write nothing")
                        .action(ArgAction::SetTrue),
                ),
            "same-as" => sub
                .arg(
                    Arg::new("action")
                        .required(true)
                        .value_name("ACTION")
                        .value_parser(["propose", "classes"])
                        .help("propose edges with an engine, or derive the classes edges add up to"),
                )
                .arg(
                    Arg::new("engine")
                        .long("engine")
                        .value_name("E")
                        // `semantic` is reserved for SMYSL-2.5 and deliberately not listed:
                        // a value that parsed and then said "not in this build" reads like a
                        // defect, and a value clap refuses reads like what it is.
                        .value_parser(["identical-span", "lexical", "anchored"])
                        .help("Which evidence proposes a pair (`propose`)"),
                )
                .arg(
                    Arg::new("policy")
                        .long("policy")
                        .value_name("P")
                        .default_value("strict")
                        .help("`strict` (normative), `component`, or `attested:N` (`classes`)"),
                )
                .arg(
                    Arg::new("scope")
                        .long("scope")
                        .value_name("WHO")
                        .num_args(1..)
                        .help("Restrict to the units drawn from these aliases or mids"),
                )
                .arg(
                    Arg::new("neighbours")
                        .long("neighbours")
                        .short('n')
                        .value_name("K")
                        .default_value("5")
                        .value_parser(clap::value_parser!(usize))
                        .help("Neighbours per unit (`--engine lexical`)"),
                )
                .arg(
                    Arg::new("no-adjacent")
                        .long("no-adjacent")
                        .help("Keep pairs that are not in the same or a neighbouring place (`--engine lexical`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MS")
                        .help("Wall clock for the attestations, in milliseconds")
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(
                    Arg::new("yes")
                        .long("yes")
                        .help("Commit the staged edges instead of leaving them for review")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Report what would be staged, and write nothing")
                        .action(ArgAction::SetTrue),
                ),
            "link" => sub
                .arg(
                    Arg::new("scope")
                        .long("scope")
                        .value_name("WHO")
                        .num_args(1..)
                        .help("Restrict to the units drawn from these aliases or mids"),
                )
                .arg(
                    Arg::new("candidates")
                        .long("candidates")
                        .help("List the pairs the generator chose and stop; calls no model")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("per-unit")
                        .long("per-unit")
                        .value_name("N")
                        .default_value("16")
                        .value_parser(clap::value_parser!(usize))
                        .help("Candidates one unit may contribute; this is what spends"),
                )
                .arg(
                    Arg::new("neighbours")
                        .long("neighbours")
                        .short('n')
                        .value_name("K")
                        .default_value("32")
                        .value_parser(clap::value_parser!(usize))
                        .help("BM25 neighbours retrieved per unit, before the budget"),
                )
                .arg(
                    Arg::new("hub")
                        .long("hub")
                        .value_name("N")
                        .default_value("32")
                        .value_parser(clap::value_parser!(usize))
                        .help("Above this many mentions an entity confirms but does not raise"),
                )
                .arg(
                    Arg::new("pairs")
                        .long("pairs")
                        .value_name("N")
                        .default_value("10")
                        .value_parser(clap::value_parser!(usize))
                        .help("Pairs per model turn"),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MS")
                        .help("Wall clock for the attestations, in milliseconds")
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(
                    Arg::new("yes")
                        .long("yes")
                        .help("Commit the staged edges instead of leaving them for review")
                        .action(ArgAction::SetTrue),
                ),
            "q" => sub
                .arg(
                    Arg::new("query")
                        .value_name("SQ")
                        .help("The query, in sq/1 (SMYSL-2.9); or --file"),
                )
                .arg(
                    Arg::new("file")
                        .long("file")
                        .value_name("Q.sq")
                        .help("Read the query from this file instead"),
                )
                .arg(
                    Arg::new("library")
                        .long("library")
                        .help(
                            "Treat the store as a library root: `texts`, `collection:`, \
                             `channel:` and the time stages resolve against its catalogue",
                        )
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("explain")
                        .long("explain")
                        .help("Print the plan instead of running it")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("why")
                        .long("why")
                        .help("After a ranked answer, print each hit's partition ranks and term contributions")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("param")
                        .long("param")
                        .value_name("K=V")
                        .action(clap::ArgAction::Append)
                        .help("Bind `$K` to V; repeatable"),
                )
                .arg(
                    Arg::new("all-versions")
                        .long("all-versions")
                        .help("Keep superseded units instead of substituting their newest head")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("idf")
                        .long("idf")
                        .value_name("SCOPE")
                        .value_parser(["partition", "collection", "library"])
                        .help("What term statistics are counted over [default: collection]"),
                )
                .arg(
                    Arg::new("per-partition")
                        .long("per-partition")
                        .value_name("N")
                        .default_value("10")
                        .help("Hits a source keeps per partition before fusion; `limit` is a stage")
                        .value_parser(clap::value_parser!(usize)),
                )
                .arg(
                    Arg::new("engine-model")
                        .long("engine-model")
                        .value_name("DIR")
                        .conflicts_with("candle-model")
                        .help(
                            "A Model2Vec model (S0) for ~sem, ~hybrid, near(…) and lang:any; needs \
                             --features semantic",
                        ),
                )
                .arg(
                    Arg::new("candle-model")
                        .long("candle-model")
                        .value_name("DIR")
                        .help(
                            "A transformer encoder (S1) for the same stages, its vectors cached \
                             beside the store; needs --features semantic-candle",
                        ),
                )
                .arg(
                    Arg::new("view")
                        .long("view")
                        .value_name("ID")
                        .help("The view id `pack … as view` writes under [default: v/query]"),
                )
                .arg(
                    Arg::new("save")
                        .long("save")
                        .help(
                            "After the run, append the query as an x.query/saved unit with an \
                             x.query/answers edge to each result",
                        )
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("rerun")
                        .long("rerun")
                        .value_name("UID")
                        .help(
                            "Run a saved query again, pinned to the revisions it recorded, and \
                             report the data effect",
                        ),
                )
                .arg(
                    Arg::new("current")
                        .long("current")
                        .help(
                            "With --rerun: also run under current revisions, return that \
                             answer and report the schema effect; with --save, supersede the \
                             saved query",
                        )
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("store")
                        .value_name("PATH")
                        .help("Store, or library root with --library"),
                ),
            "ask" => sub
                .arg(
                    Arg::new("question")
                        .value_name("QUESTION")
                        .required(true)
                        .help("The question, in any tier-1 language (en, de, es, fr, ru)"),
                )
                .arg(
                    Arg::new("scope")
                        .long("scope")
                        .value_name("ITEMS")
                        .help("sq scope items every slot is asked over, e.g. `texts` [default: the whole store]"),
                )
                .arg(
                    Arg::new("intent")
                        .long("intent")
                        .value_name("INTENT")
                        .value_parser([
                            "define", "explain", "verify", "attribute", "trace", "locate",
                            "depend", "compare",
                        ])
                        .help("The intent, when no rule recognises the question"),
                )
                .arg(
                    Arg::new("lang")
                        .long("lang")
                        .value_name("LANG")
                        .value_parser(["en", "de", "es", "fr", "ru"])
                        .help("Read the question by this language's rules only"),
                )
                .arg(
                    Arg::new("subject")
                        .long("subject")
                        .value_name("UID")
                        .help("The entity to take when the subject names more than one (SMY-W417)"),
                )
                .arg(
                    Arg::new("templates")
                        .long("templates")
                        .value_name("ID")
                        .default_value("smysl/dossier/1")
                        .help("The dossier template set"),
                )
                .arg(
                    Arg::new("budget")
                        .long("budget")
                        .value_name("TOKENS")
                        .value_parser(clap::value_parser!(u64))
                        .help("Pack the dossier to this many tokens; rule R keeps live rebuttals"),
                )
                .arg(
                    Arg::new("target")
                        .long("target")
                        .value_name("TARGET")
                        .help("Render the dossier as this target [default: markdown]"),
                )
                .arg(
                    Arg::new("save")
                        .long("save")
                        .help("Append the dossier: its question, slot queries, thread and view")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("answer")
                        .long("answer")
                        .help(
                            "Ask a model to answer from the dossier; its units are grounded on the \
                             dossier only, capped by rule M and staged (needs --save)",
                        )
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MS")
                        .help("Wall clock for the answer's attestations, in milliseconds")
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(
                    Arg::new("yes")
                        .long("yes")
                        .help("With --answer: commit the staged answer instead of leaving it for review")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("store")
                        .value_name("PATH")
                        .help("Store, or library root"),
                ),
            "measure" => sub
                .arg(
                    Arg::new("ids")
                        .value_name("IDS")
                        .required(true)
                        .help("The measures, comma-separated: T1–T7 over text, A1–A10 over units, C1–C5 over a channel, D1–D6 over a conversation"),
                )
                .arg(
                    Arg::new("scope")
                        .long("scope")
                        .required(true)
                        .value_name("SQ")
                        .help("The scope, an sq query; a text measure reads its `from`, e.g. `from collection:kjv`, and a unit measure what it answers with"),
                )
                .arg(
                    Arg::new("channel")
                        .long("channel")
                        .value_name("SQ")
                        .help("The target a channel measure (C1–C5) compares the scope with, an sq query; the scope is the source"),
                )
                .arg(
                    Arg::new("policy")
                        .long("policy")
                        .value_name("POLICY")
                        .help("Proposition classes for A2, A3, C1–C4, D3, D4 and D6: strict, component or attested:<n>"),
                )
                .arg(
                    Arg::new("profile")
                        .long("profile")
                        .value_name("NAME")
                        .help("The granularity profile A1's density is under"),
                )
                .arg(
                    Arg::new("axis")
                        .long("axis")
                        .value_name("AXIS")
                        .default_value("said")
                        .help("The time axis A3, A10, D1, D3 and D6 read: said, composed or about"),
                )
                .arg(
                    Arg::new("level")
                        .long("level")
                        .value_name("LEVEL")
                        .default_value("part")
                        .help("The segment a measure counts in: part, chapter, verse, message, window:<n>"),
                )
                .arg(
                    Arg::new("analyzer")
                        .long("analyzer")
                        .value_name("CHAIN:STREAM")
                        .help("Analyzer chain and stream, e.g. `smysl/an-en/1:stem` [default: the texts' language's, surface]"),
                )
                .arg(
                    Arg::new("param")
                        .long("param")
                        .value_name("[ID.]K=V")
                        .action(ArgAction::Append)
                        .help("A measure parameter, for every id that takes it or for one: `T5.order=2`"),
                )
                .arg(
                    Arg::new("min-n")
                        .long("min-n")
                        .value_name("[ID=]N")
                        .action(ArgAction::Append)
                        .help("The smallest sample, in each measure's own unit, for every id or for one: `T3=2`"),
                )
                .arg(
                    Arg::new("seed")
                        .long("seed")
                        .value_name("N")
                        .default_value("0")
                        .value_parser(clap::value_parser!(u64))
                        .help("The bootstrap seed"),
                )
                .arg(
                    Arg::new("allow-lossy")
                        .long("allow-lossy")
                        .help("Measure a lossy text too, flagged `lossy-admitted` (SMY-W408)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Print the values and refusals and write nothing")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MS")
                        .help("Wall clock for the attestations, in milliseconds")
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(
                    Arg::new("store")
                        .value_name("PATH")
                        .help("Library root"),
                ),
            "assess" => sub
                .arg(
                    Arg::new("action")
                        .required(true)
                        .value_name("ACTION")
                        .value_parser(["key", "score", "state", "diff"])
                        .help(
                            "key a view or dossier, score a reader against a key, write the \
                             reader's knowledge state, or diff two states",
                        ),
                )
                .arg(
                    Arg::new("target")
                        .required(true)
                        .value_name("TARGET")
                        .help("The view id for `key`, the key unit for `score` and `state`, the first state for `diff`"),
                )
                .arg(
                    Arg::new("other")
                        .value_name("OTHER")
                        .help("The second state, for `diff`"),
                )
                .arg(
                    Arg::new("reader")
                        .long("reader")
                        .value_name("AGENT")
                        .help("The reader whose attested units are the responses (`score`, `state`), such as `human:ada`"),
                )
                .arg(
                    Arg::new("policy")
                        .long("policy")
                        .value_name("POLICY")
                        .help("Proposition classes: strict, component or attested:<n>; required by `key`, and the key's own by default"),
                )
                .arg(
                    Arg::new("scope")
                        .long("scope")
                        .value_name("SQ")
                        .help("The texts a key's fabrication is judged against, an sq query (`key`)"),
                )
                .arg(
                    Arg::new("mode")
                        .long("mode")
                        .value_name("MODE")
                        .value_parser(["any", "all"])
                        .default_value("any")
                        .help("Whether an item is answered by any of its answers or needs all (`key`)"),
                )
                .arg(
                    Arg::new("judge")
                        .long("judge")
                        .value_name("JUDGE")
                        .value_parser(["accepted", "lexical", "anchored"])
                        .default_value("accepted")
                        .help("What matches a response to an answer beyond the store's same-as (`score`, `state`)"),
                )
                .arg(
                    Arg::new("neighbours")
                        .long("neighbours")
                        .value_name("K")
                        .default_value("1")
                        .value_parser(clap::value_parser!(usize))
                        .help("The lexical judge's nearest units per response"),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Print the result and write nothing")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MS")
                        .help("Wall clock for the attestations, in milliseconds")
                        .value_parser(clap::value_parser!(u64)),
                ),
            "summarise" => sub
                .arg(
                    Arg::new("scope")
                        .long("scope")
                        .required(true)
                        .value_name("ALIAS")
                        .help("The expression to summarise, by alias or `m3:` id"),
                )
                .arg(
                    Arg::new("levels")
                        .long("levels")
                        .value_name("L,...")
                        .default_value("chapter")
                        .help("Structure levels, lowest first; each is grounded on the one below"),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MS")
                        .help("Wall clock for the attestations, in milliseconds")
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(
                    Arg::new("yes")
                        .long("yes")
                        .help("Commit the staged summaries instead of leaving them for review")
                        .action(ArgAction::SetTrue),
                ),
            "view" => sub
                .arg(
                    Arg::new("roots")
                        .long("roots")
                        .value_name("UID")
                        .action(ArgAction::Append)
                        .help("Roots of the view"),
                )
                .arg(
                    Arg::new("id")
                        .long("id")
                        .value_name("NAME")
                        .help("View identifier"),
                )
                .arg(
                    Arg::new("threads")
                        .long("threads")
                        .value_name("ID")
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("requires")
                        .long("requires")
                        .value_name("SCHEMA")
                        .action(ArgAction::Append),
                )
                .arg(Arg::new("store").value_name("PATH")),
            "bundle" => sub
                .arg(
                    Arg::new("view")
                        .long("view")
                        .value_name("ID")
                        .help("Which view to bundle; the first if omitted"),
                )
                .arg(
                    Arg::new("include-retracted")
                        .long("include-retracted")
                        .help("Keep units that have been retracted")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("unknown")
                        .long("unknown")
                        .value_name("WHAT")
                        .value_parser(["keep", "drop"])
                        .default_value("keep")
                        .help("Records of a type this build cannot interpret (rule X)"),
                )
                .arg(Arg::new("store").value_name("PATH")),
            "merge" => sub
                .arg(
                    Arg::new("policy")
                        .long("policy")
                        .value_name("P")
                        .value_parser(["latest", "all", "contend"])
                        .help("Supersession policy"),
                )
                .arg(
                    Arg::new("retraction")
                        .long("retraction")
                        .value_name("P")
                        .value_parser(["strict", "advisory", "ignore"])
                        .help("Retraction policy"),
                )
                .arg(
                    Arg::new("staged")
                        .long("staged")
                        .help("Commit `.smysl/staged.smy` into the store")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("fail-on-contention")
                        .long("fail-on-contention")
                        .help("Exit 5 when the merged store carries a contention")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("max-contentions-per-agent")
                        .long("max-contentions-per-agent")
                        .value_name("N")
                        .help("Warn when one merge raises more than N contentions"),
                )
                .arg(
                    Arg::new("inputs")
                        .num_args(1..)
                        .required(true)
                        .value_name("STORE")
                        .help("Stores to merge; `-` reads stdin (rule P)"),
                ),
            "retract" => sub
                .arg(
                    Arg::new("uid")
                        .required(true)
                        .value_name("UID")
                        .help("The unit to retract; the display form is resolved as a prefix"),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Report the blast radius without applying anything")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("as")
                        .long("as")
                        .value_name("AGENT")
                        .action(ArgAction::Append)
                        .help("The agent(s) issuing the retraction"),
                )
                .arg(
                    Arg::new("reason")
                        .long("reason")
                        .value_name("TEXT")
                        .help("Why"),
                )
                .arg(
                    Arg::new("authority")
                        .long("authority")
                        .value_name("A")
                        .help("origin | any | quorum:N"),
                )
                .arg(
                    Arg::new("store")
                        .value_name("PATH")
                        .help("Store to retract from"),
                ),
            "commit" => sub
                .arg(
                    Arg::new("unit")
                        .required(true)
                        .value_name("UID")
                        .help("The unit being committed to, by uid or label, or a dating by did"),
                )
                .arg(
                    Arg::new("level")
                        .long("level")
                        .required(true)
                        .value_name("L")
                        .value_parser([
                            "floated",
                            "drafted",
                            "committed",
                            "canonical",
                            "retconned",
                        ])
                        .help("How settled it is"),
                )
                .arg(
                    Arg::new("as")
                        .long("as")
                        .required(true)
                        .value_name("AGENT")
                        .help("The agent committing"),
                )
                .arg(
                    Arg::new("note")
                        .long("note")
                        .value_name("UID")
                        .help("A unit saying why"),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MS")
                        .help("Wall clock for the record, in milliseconds")
                        .value_parser(clap::value_parser!(u64)),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .action(ArgAction::SetTrue)
                        .help("Report what would be recorded, and write nothing"),
                )
                .arg(Arg::new("store").value_name("PATH")),
            "withdraw" => sub
                .arg(
                    Arg::new("edge")
                        .required(true)
                        .value_name("EDGE")
                        .help("The edge: its rid (as `review` prints it), or `FROM --KIND--> TO`"),
                )
                .arg(
                    Arg::new("as")
                        .long("as")
                        .value_name("AGENT")
                        .action(ArgAction::Append)
                        .help("The agent(s) withdrawing it"),
                )
                .arg(
                    Arg::new("authority")
                        .long("authority")
                        .value_name("A")
                        .help("origin | any | quorum:N (origin reads the edge's attestations)"),
                )
                .arg(
                    Arg::new("reason")
                        .long("reason")
                        .value_name("UNIT")
                        .help("A unit saying why, by uid or label"),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MILLIS")
                        .value_parser(clap::value_parser!(u64))
                        .help("Timestamp, in milliseconds since the epoch [default: now]"),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Report what it would release without writing anything")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("store")
                        .value_name("PATH")
                        .help("Store to withdraw it in"),
                ),
            "resolve" => sub
                .arg(
                    Arg::new("item")
                        .required(true)
                        .value_name("ITEM")
                        .help("A contention id (`k/c…`), or a rebuts edge by rid or `FROM --rebuts--> TO`"),
                )
                .arg(
                    Arg::new("as")
                        .long("as")
                        .value_name("AGENT")
                        .help("The agent who reviewed it"),
                )
                .arg(
                    Arg::new("note")
                        .long("note")
                        .value_name("UNIT")
                        .help("A unit recording the decision, by uid or label"),
                )
                .arg(
                    Arg::new("at")
                        .long("at")
                        .value_name("MILLIS")
                        .value_parser(clap::value_parser!(u64))
                        .help("Timestamp, in milliseconds since the epoch [default: now]"),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Report what would be recorded without writing anything")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("store")
                        .value_name("PATH")
                        .help("Store to record it in"),
                ),
            "review" => sub
                .arg(
                    Arg::new("confirm")
                        .long("confirm")
                        .value_name("KINDS")
                        .help(
                            "Also list edges of these relation kinds until somebody attests them \
                             (comma-separated)",
                        ),
                )
                .arg(
                    Arg::new("confirmed-by")
                        .long("confirmed-by")
                        .value_name("KIND")
                        .value_parser(["human", "model", "tool"])
                        .help("Whose attestation confirms one [default: human]"),
                )
                .arg(
                    Arg::new("all")
                        .long("all")
                        .help("Include items already resolved")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("store")
                        .value_name("PATH")
                        .help("Store to review"),
                ),
            "find" => sub
                .arg(
                    Arg::new("query")
                        .value_name("QUERY")
                        .required(true)
                        .help("What to search for"),
                )
                .arg(
                    Arg::new("limit")
                        .long("limit")
                        .short('n')
                        .value_name("N")
                        .default_value("10")
                        .help("Maximum hits to return")
                        .value_parser(clap::value_parser!(usize)),
                )
                .arg(
                    Arg::new("kind")
                        .long("kind")
                        .value_name("TYPE")
                        .action(clap::ArgAction::Append)
                        .help("Restrict to this kernel type; repeatable"),
                )
                .arg(
                    Arg::new("min-status")
                        .long("min-status")
                        .value_name("STATUS")
                        .help("Restrict to units at or above this status"),
                )
                .arg(
                    Arg::new("engine")
                        .long("engine")
                        .value_name("E")
                        .value_parser(["lexical", "semantic", "hybrid"])
                        .help(
                            "Which retriever to rank with; semantic and hybrid need \
                             --features semantic and a model",
                        ),
                )
                .arg(
                    Arg::new("model")
                        .long("model")
                        .value_name("DIR")
                        .help(
                            "Model2Vec directory for --engine semantic|hybrid (or \
                             SMYSL_EMBED_MODEL)",
                        ),
                )
                .arg(
                    Arg::new("schema")
                        .long("schema")
                        .value_name("ID")
                        .action(clap::ArgAction::Append)
                        .help(
                            "Restrict to units of this schema, kernel or extension \
                             (x.domain/type); repeatable",
                        ),
                )
                .arg(
                    Arg::new("source")
                        .long("source")
                        .value_name("PREFIX")
                        .help(
                            "Restrict to units whose source reference starts with PREFIX, as \
                             `incident:41` selects one incident's units in a shared store",
                        ),
                )
                .arg(
                    Arg::new("payload")
                        .long("payload")
                        .value_name("KEY=VALUE[,VALUE]")
                        .help(
                            "Restrict to units whose payload has KEY equal to one of VALUE; a \
                             unit without the key is excluded",
                        ),
                )
                .arg(
                    Arg::new("lang")
                        .long("lang")
                        .value_name("TAG")
                        .help(
                            "Analyse the query and the corpus with this language's chain, as \
                             `ru` stems Russian; overridden by --analyzer",
                        ),
                )
                .arg(
                    Arg::new("analyzer")
                        .long("analyzer")
                        .value_name("ID")
                        .help(
                            "Name the analyzer by id (smysl/plain/1, smysl/fold-en/1, \
                             smysl/an-<lang>/1)",
                        ),
                )
                .arg(
                    Arg::new("library")
                        .long("library")
                        .help(
                            "Treat the store as a library root and score its persistent \
                             postings instead of analysing the corpus again",
                        )
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("why")
                        .long("why")
                        .action(ArgAction::SetTrue)
                        .help("Also print which query terms each hit matched, and what each contributed"),
                )
                .arg(
                    Arg::new("store")
                        .value_name("PATH")
                        .help("Store to search"),
                ),
            "salience" => sub
                .arg(
                    Arg::new("hop")
                        .long("hop")
                        .value_name("N")
                        .help("Measure recency against this handoff (needs a recency weight)")
                        .value_parser(clap::value_parser!(u32)),
                )
                .arg(
                    Arg::new("top")
                        .long("top")
                        .value_name("N")
                        .help("Show only the N highest-scoring units"),
                )
                .arg(
                    Arg::new("explain")
                        .long("explain")
                        .value_name("UID")
                        .help("Break one unit's score into its three terms"),
                )
                .arg(
                    Arg::new("weights")
                        .long("weights")
                        .value_name("C,R,T")
                        .help("Override the centrality, corroboration and role weights"),
                )
                .arg(
                    Arg::new("seed")
                        .long("seed")
                        .value_name("UID")
                        .action(ArgAction::Append)
                        .help("Personalise against these units; the view roots by default"),
                )
                .arg(Arg::new("store").value_name("PATH")),
            "pack" => sub
                .arg(
                    Arg::new("budget")
                        .long("budget")
                        .value_name("N")
                        .required(true)
                        .help("Token budget, counted with the recorded estimator"),
                )
                .arg(
                    Arg::new("reserve")
                        .long("reserve")
                        .value_name("N")
                        .help(
                            "Set this much of --budget aside for the rest of your prompt; the \
                             pack is solved against what is left",
                        ),
                )
                .arg(
                    Arg::new("support")
                        .long("support")
                        .value_name("KINDS")
                        .help(
                            "Carry what a selected unit rests on over these edges (C8): a preset \
                             (premises, dependency, support) or relation kinds, comma-separated",
                        ),
                )
                .arg(
                    Arg::new("focus")
                        .long("focus")
                        .value_name("UID")
                        .action(ArgAction::Append)
                        .help("Units that must reach L1; packing fails if they cannot"),
                )
                .arg(
                    Arg::new("query")
                        .long("query")
                        .value_name("TEXT")
                        .help("Focus on what this query finds, instead of naming uids"),
                )
                .arg(
                    Arg::new("payload")
                        .long("payload")
                        .value_name("KEY=VALUE[,VALUE]")
                        .help(
                            "Restrict what --query may focus on to units whose payload has KEY \
                             equal to one of VALUE",
                        ),
                )
                .arg(
                    Arg::new("source")
                        .long("source")
                        .value_name("PREFIX")
                        .help(
                            "Pack only units whose source reference starts with PREFIX — the \
                             whole of one incident out of a shared store",
                        ),
                )
                .arg(
                    Arg::new("engine")
                        .long("engine")
                        .value_name("E")
                        .value_parser(["lexical", "semantic", "hybrid"])
                        .help("Which retriever --query focuses with; see `find --engine`"),
                )
                .arg(
                    Arg::new("model")
                        .long("model")
                        .value_name("DIR")
                        .help(
                            "Model2Vec directory for --engine semantic|hybrid (or \
                             SMYSL_EMBED_MODEL)",
                        ),
                )
                .arg(
                    Arg::new("query-limit")
                        .long("query-limit")
                        .value_name("N")
                        .default_value("3")
                        .help("How many hits --query focuses on")
                        .value_parser(clap::value_parser!(usize)),
                )
                .arg(
                    Arg::new("lod")
                        .long("lod")
                        .value_name("L")
                        .value_parser(["auto", "L0", "L1", "L2"])
                        .help("Cap every unit at this level"),
                )
                .arg(
                    Arg::new("explain")
                        .long("explain")
                        .help("Say which constraint put each unit in")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("tokenizer")
                        .long("tokenizer")
                        .value_name("ID")
                        .help("Cost model; recorded in the packinfo either way (D-2)"),
                )
                .arg(
                    Arg::new("mode")
                        .long("mode")
                        .value_name("M")
                        .value_parser(["greedy", "exact"])
                        .help("`exact` proves optimality by branch and bound; needs the exact-pack feature"),
                )
                .arg(Arg::new("store").value_name("PATH")),
            "thread" => sub
                .arg(
                    Arg::new("derive")
                        .long("derive")
                        .value_name("SCHEMA")
                        .num_args(0..=1)
                        .value_parser([
                            "analysis",
                            "narrative",
                            "brief",
                            "qa",
                            "plan",
                            "exposition",
                            "dialogue",
                            "dossier",
                        ])
                        .help("Derive a thread from the graph; the schema may follow here or in --schema"),
                )
                .arg(
                    Arg::new("schema")
                        .long("schema")
                        .value_name("S")
                        .value_parser([
                            "analysis",
                            "narrative",
                            "brief",
                            "qa",
                            "plan",
                            "exposition",
                            "dialogue",
                            "dossier",
                        ])
                        .help("Schema to derive under (§23 spells it this way)"),
                )
                .arg(
                    Arg::new("only")
                        .long("only")
                        .help("Emit the thread record alone rather than the store it belongs to")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("list")
                        .long("list")
                        .help("List the threads the store already holds")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("show")
                        .long("show")
                        .value_name("ID")
                        .help("Print one thread, step by step"),
                )
                .arg(
                    Arg::new("id")
                        .long("id")
                        .value_name("T")
                        .help("Thread id for the derived thread"),
                )
                .arg(
                    Arg::new("as")
                        .long("as")
                        .value_name("AGENT")
                        .help("Owner of the derived thread"),
                )
                .arg(
                    Arg::new("scope")
                        .long("scope")
                        .value_name("UID")
                        .action(ArgAction::Append)
                        .help("Derive over these units only"),
                )
                .arg(
                    Arg::new("arity")
                        .long("arity")
                        .value_name("ROLE=N")
                        .action(ArgAction::Append)
                        .help("Override how many units a role may hold"),
                )
                .arg(
                    Arg::new("explain")
                        .long("explain")
                        .help("Say which role each unit took and what repair added")
                        .action(ArgAction::SetTrue),
                )
                .arg(Arg::new("store").value_name("PATH")),
            "render" => sub
                .arg(
                    Arg::new("thread")
                        .long("thread")
                        .value_name("ID")
                        .help("Thread to render; the store's only thread by default"),
                )
                .arg(
                    Arg::new("profile")
                        .long("profile")
                        .value_name("NAME")
                        .help("Built-in profile name, or a path to a profile file"),
                )
                .arg(
                    Arg::new("target")
                        .long("target")
                        .value_name("T")
                        .value_parser(["markdown", "md", "typst", "html", "slides", "json", "text"])
                        .help("Output format"),
                )
                .arg(
                    Arg::new("lod")
                        .long("lod")
                        .value_name("L")
                        .value_parser(["L0", "L1", "L2"])
                        .help("Cap every block at this level, whatever the profile says"),
                )
                .arg(
                    Arg::new("contentions")
                        .long("contentions")
                        .value_name("M")
                        .value_parser(["show", "suppress"])
                        .help("Override the profile's rule V2 setting"),
                )
                .arg(
                    Arg::new("as")
                        .long("as")
                        .value_name("AGENT")
                        .help("Whose thread, when several agents hold one under that id"),
                )
                .arg(
                    Arg::new("profiles")
                        .long("profiles")
                        .help("List the built-in profiles and exit")
                        .action(ArgAction::SetTrue),
                )
                .arg(Arg::new("store").value_name("PATH")),
            "ingest" => sub
                .arg(
                    Arg::new("file")
                        .value_name("FILE")
                        .help("Document to ingest; `-` reads stdin"),
                )
                .arg(
                    Arg::new("hop")
                        .long("hop")
                        .value_name("N")
                        .help("Which handoff of a pipeline this is; stamped on every attestation")
                        .value_parser(clap::value_parser!(u32)),
                )
                .arg(
                    Arg::new("rung")
                        .long("rung")
                        .value_name("R")
                        .value_parser(["computed", "document", "web", "model"])
                        .help("Trust rung of the source; caps what units may claim (rule T)"),
                )
                .arg(
                    Arg::new("granularity")
                        .long("granularity")
                        .value_name("P")
                        .value_parser(["coarse", "default", "standard", "fine"])
                        .help("Granularity preset recorded in the recipe (standard = default)"),
                )
                .arg(
                    Arg::new("estimator")
                        .long("estimator")
                        .value_name("ID")
                        .help("Count the gist bound with this estimator, and state it (F-2)"),
                )
                .arg(
                    Arg::new("max-output")
                        .long("max-output")
                        .value_name("N")
                        .value_parser(clap::value_parser!(usize))
                        .help("Output tokens per call [default: the provider's max_output, at least 2048]"),
                )
                .arg(
                    Arg::new("lang-policy")
                        .long("lang-policy")
                        .value_name("P")
                        // Not a closed list any more, and it cannot be one: `pivot:<lang>`
                        // names a language, so the set of accepted values is the set of
                        // language tags. `LangPolicy::parse` is what refuses the rest, and it
                        // says which part was wrong — the policy or the tag.
                        .help(
                            "What language the model writes in: source, or pivot:<lang> with \
                             --text [default: source]",
                        ),
                )
                .arg(
                    Arg::new("normaliser")
                        .long("normaliser")
                        .value_name("V")
                        .value_parser(["v1", "v2"])
                        .help("Quote-check comparison form [default: v1]"),
                )
                .arg(
                    Arg::new("temperature")
                        .long("temperature")
                        .value_name("T")
                        .value_parser(clap::value_parser!(f32))
                        .help("Sampling temperature, 0.0 to 2.0 [default: 0.0, or ingest.temperature]"),
                )
                .arg(
                    Arg::new("path")
                        .long("path")
                        .value_name("P")
                        .value_parser(["auto", "surface", "json-ast"])
                        .help("Override the path D-9 would choose"),
                )
                .arg(
                    Arg::new("repair")
                        .long("repair")
                        .value_name("N")
                        .help("Repair attempts before a span degrades to opaque prose"),
                )
                .arg(
                    Arg::new("prompt")
                        .long("prompt")
                        .value_name("FILE")
                        .help("Prompt override (HJSON: id, version, system, user, schema); beats `ingest.prompt` in the config"),
                )
                .arg(
                    Arg::new("yes")
                        .long("yes")
                        .help("Commit the staged batch to --store and exit 0 rather than 10")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Report what would be sent and to whom; make no call")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("text")
                        .long("text")
                        .value_name("WHO")
                        .help("Ingest an expression of the library at --store: its alias, or a manifest id"),
                )
                .arg(
                    Arg::new("window-level")
                        .long("window-level")
                        .value_name("L")
                        .help("Fill windows with whole nodes at this level [default: the reading's leaves]"),
                )
                .arg(
                    Arg::new("digest-budget")
                        .long("digest-budget")
                        .value_name("N")
                        .value_parser(clap::value_parser!(u32))
                        .help("Tokens of ledger digest sent with each window; 0 sends none [default: 512]"),
                )
                .arg(
                    Arg::new("reextract")
                        .long("reextract")
                        .help("Ignore the journal for this expression and extract every window again")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("single-pass")
                        .long("single-pass")
                        .help("One extraction per window instead of two; recorded in the recipe (§3.4)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("cost")
                        .long("cost")
                        .help("With --text: report what the journal says this expression cost, per language; make no call")
                        .action(ArgAction::SetTrue),
                ),
            "attest" => sub
                .arg(
                    Arg::new("what")
                        .long("what")
                        .value_name("W")
                        .value_parser(["gist-coverage", "warrant-plausibility", "granularity"])
                        .help("Which semantic question to ask"),
                )
                .arg(
                    Arg::new("sample")
                        .long("sample")
                        .value_name("N")
                        .help("How many units to ask about; `all` for the whole store"),
                )
                .arg(Arg::new("store").value_name("PATH")),
            "providers" => sub
                .arg(
                    Arg::new("probe")
                        .long("probe")
                        .help("Contact each provider and report what it actually is")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("models")
                        .long("models")
                        .help("List each provider's installed models")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("tasks")
                        .long("tasks")
                        .help("Report which tasks would send content off the machine")
                        .action(ArgAction::SetTrue),
                ),
            "usage" => sub
                .arg(
                    Arg::new("by")
                        .long("by")
                        .value_name("K")
                        .value_parser(["provider", "task", "run", "model"])
                        .help("How to group the ledger"),
                )
                .arg(
                    Arg::new("since")
                        .long("since")
                        .value_name("MS")
                        .help("Only calls at or after this epoch-millisecond timestamp"),
                )
                .arg(
                    Arg::new("reset")
                        .long("reset")
                        .help("Discard the ledger")
                        .action(ArgAction::SetTrue),
                ),
            "compact" => sub
                .arg(
                    Arg::new("path")
                        .help("Store to compact; `-` reads stdin (rule P)")
                        .value_name("PATH"),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Report what would be dropped without emitting anything")
                        .action(ArgAction::SetTrue),
                ),
            "relink" => sub
                .arg(
                    Arg::new("path")
                        .help("Store to relink; `-` reads stdin (rule P)")
                        .value_name("PATH"),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Report what would be re-pointed without emitting anything")
                        .action(ArgAction::SetTrue),
                ),
            "import" => sub
                .arg(
                    Arg::new("file")
                        .help("Delimiter-separated file to import; `-` reads stdin")
                        .value_name("PATH")
                        .required(true),
                )
                .arg(
                    Arg::new("key")
                        .long("key")
                        .help("Columns naming the reading rather than its value")
                        .value_name("COL")
                        .action(ArgAction::Append),
                )
                .arg(
                    Arg::new("kind")
                        .long("kind")
                        .help("Source kind recorded on each unit")
                        .value_parser(["file", "metric", "tool", "url", "doc"])
                        .default_value("file"),
                ),
            "ui" => sub.arg(
                Arg::new("path")
                    .help("Store to browse; `-` reads stdin (rule P)")
                    .value_name("PATH"),
            ),
            "reindex" => sub
                .arg(
                    Arg::new("verify")
                        .long("verify")
                        .help("Compare the rebuilt index against the sidecar instead of writing it")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("store")
                        .value_name("PATH")
                        .help("Store to reindex"),
                ),
            // `lib`'s six actions are values of `<ACTION>`, as `text`'s and `assess`'s are, and
            // not the nested clap subcommands SMYSL-2.7 §4.4 sketched. Nested, `lib --help`
            // would list six names and none of their flags, and `make cli-surface` reads each
            // command's `--help` once: the gate would record `lib` as taking nothing.
            "migrate" => sub
                .arg(
                    Arg::new("schema")
                        .long("schema")
                        .value_name("ID")
                        .required(true)
                        .help("The schema whose units are re-derived"),
                )
                .arg(
                    Arg::new("to")
                        .long("to")
                        .value_name("REV")
                        .required(true)
                        .help("The revision to re-derive them under"),
                )
                .arg(
                    Arg::new("agent")
                        .long("agent")
                        .value_name("ID")
                        .help("Who the migration is attributed to [default: tool:smysl-migrate]"),
                )
                .arg(
                    Arg::new("clock")
                        .long("clock")
                        .value_name("MS[.N]")
                        .help("The attestations' clock; otherwise one past the store's latest"),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Report what would be staged, and stage nothing")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("store")
                        .value_name("PATH")
                        .help("Store to migrate"),
                ),
            "schema" => sub
                .arg(
                    Arg::new("action")
                        .required(true)
                        .value_name("ACTION")
                        .value_parser(["show", "check"])
                        .help(
                            "show a schema's revisions, fields, lens steps and chains; or check \
                             rule N over every revision and lens totality over the store",
                        ),
                )
                .arg(
                    Arg::new("targets")
                        .value_name("TARGET")
                        .num_args(0..=2)
                        .action(ArgAction::Append)
                        .help(
                            "`show ID [STORE]`; `check [ID] [STORE]` — one target is the store \
                             unless --store gives it, then the id; `check` without an id checks \
                             every schema the store declares",
                        ),
                ),
            "lib" => sub
                .arg(
                    Arg::new("action")
                        .required(true)
                        .value_name("ACTION")
                        .value_parser(["import", "status", "accept", "discard", "export", "verify", "migrate"])
                        .help(
                            "import files into the library, list its imports, accept or \
                             discard a staged one, export the library as a collection, \
                             verify a collection against its index, or move a shard's \
                             records between its log and the unit store",
                        ),
                )
                .arg(
                    Arg::new("targets")
                        .value_name("TARGET")
                        .num_args(0..)
                        .action(ArgAction::Append)
                        .help(
                            "the files, directories and export indexes for `import` (`-` \
                             reads stdin); the import's name for `accept` and `discard`; the \
                             index, or its directory, for `verify`",
                        ),
                )
                .arg(
                    Arg::new("name")
                        .long("name")
                        .value_name("N")
                        .help("The import's name, `import:<N>` in its manifests (`import`); the first input's stem by default"),
                )
                .arg(
                    Arg::new("accept")
                        .long("accept")
                        .help("Append what passes, rather than stage it (`import`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("licence")
                        .long("licence")
                        .value_name("SPDX")
                        .help("The licence the manifests record (`import`); `unknown` by default"),
                )
                .arg(
                    Arg::new("origin")
                        .long("origin")
                        .value_name("URL")
                        .help("Where the files came from (`import`); otherwise each manifest records its file name"),
                )
                .arg(
                    Arg::new("import")
                        .long("import")
                        .value_name("NAME")
                        .help("Report this import only (`status`)"),
                )
                .arg(
                    Arg::new("fail-on-contention")
                        .long("fail-on-contention")
                        .help("Merge nothing if the import would give rise to a new contention (`accept`); exits 5")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("scope")
                        .long("scope")
                        .value_name("SQ")
                        .action(ArgAction::Append)
                        .help(
                            "What to export (`export`, required): an sq/1 query, such as \
                             `from library` or `from import:<name>`; `all`; or a unit by uid \
                             or label, repeated",
                        ),
                )
                .arg(
                    Arg::new("layout")
                        .long("layout")
                        .value_name("LAYOUT")
                        .value_parser(["single", "by-view", "by-expression", "by-collection", "by-time", "by-query"])
                        .default_value("single")
                        .help("One file, or one per corpus view, expression, collection or window of time (`export`)"),
                )
                .arg(
                    Arg::new("budget")
                        .long("budget")
                        .value_name("N")
                        .value_parser(clap::value_parser!(u64))
                        .help("The token budget each file is packed to, for `--format pack` (`export`)"),
                )
                .arg(
                    Arg::new("group-by")
                        .long("group-by")
                        .value_name("FIELD")
                        .help("The `group by` field `by-query` files units by, such as `kind` or `lang` (`export`)"),
                )
                .arg(
                    Arg::new("window")
                        .long("window")
                        .value_name("D")
                        .help("The width of a `by-time` window: `30m`, `6h`, `1d`, `2w` (`export`); a day by default"),
                )
                .arg(
                    Arg::new("as-of")
                        .long("as-of")
                        .value_name("EDTF")
                        .help("Export only what was known before the end of this date (`export`)"),
                )
                .arg(
                    Arg::new("target-revision")
                        .long("target-revision")
                        .value_name("ID@N")
                        .help("Export the selection as revision N of schema ID writes it, through its lenses (`export`)"),
                )
                .arg(
                    Arg::new("agent")
                        .long("agent")
                        .value_name("ID")
                        .help("The agent of the attestations --target-revision adds (`export`); tool:smysl-lib by default"),
                )
                .arg(
                    Arg::new("clock")
                        .long("clock")
                        .value_name("MS[.N]")
                        .help("The clock of those attestations (`export`); by default just after everything selected"),
                )
                .arg(
                    Arg::new("closure")
                        .long("closure")
                        .value_name("MODE")
                        .value_parser(["full", "ref"])
                        .default_value("full")
                        .help("Every file closed on its own, or every record written once (`export`)"),
                )
                .arg(
                    Arg::new("with-views")
                        .long("with-views")
                        .help("Export the corpus views as well (`export`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("with-packs")
                        .long("with-packs")
                        .help("Export the pack infos as well (`export`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("with-text")
                        .long("with-text")
                        .help("Export the text of the parts the units are drawn from, where the manifest's carry and licence allow it (`export`, a library)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("with-provenance")
                        .long("with-provenance")
                        .help("Export the import manifests the records arrived under (`export`, a library)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("with-staged")
                        .long("with-staged")
                        .help("Export staged imports as well (`export`, a library)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("with-quarantine")
                        .long("with-quarantine")
                        .help("Export quarantined records as well (`export`, a library)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("primary")
                        .long("primary")
                        .value_name("SHARD")
                        .action(ArgAction::Append)
                        .help("Make this shard primary: its records into the library's unit store, its log set aside, after a round trip that must reproduce its record-set digest (`migrate`)"),
                )
                .arg(
                    Arg::new("index")
                        .long("index")
                        .value_name("SHARD")
                        .action(ArgAction::Append)
                        .help("Make this primary shard an index-mode one again, writing its log back out of the unit store (`migrate`)"),
                )
                .arg(
                    Arg::new("primary-shards")
                        .long("primary-shards")
                        .help("Verify the library's primary shards against their exports instead of a collection; exits 9 on SMY-E505 (`verify`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("p2")
                        .long("p2")
                        .value_name("FILE")
                        .action(ArgAction::Append)
                        .help("Check P2 for a file: import it into a scratch library and export it again (`verify`)"),
                )
                .arg(
                    Arg::new("keep-scratch")
                        .long("keep-scratch")
                        .help("Keep the scratch libraries `--p2` builds, and say where (`verify`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("no-join")
                        .long("no-join")
                        .help("Propose no same-as edges to what the library holds (`import`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("keep-original")
                        .long("keep-original")
                        .help("Keep each file's bytes in the object store under their BLAKE3 (`import`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("edges")
                        .long("edges")
                        .value_name("LIST")
                        .help("Edges the closure follows beside deps and grounds, as `trace --edges` takes them (`export`); every edge by default"),
                )
                .arg(
                    Arg::new("unknown")
                        .long("unknown")
                        .value_name("WHAT")
                        .value_parser(["include", "drop"])
                        .default_value("include")
                        .help("Records this build cannot interpret: kept in `unattributed.cbor`, or left out and counted (`export`)"),
                )
                .arg(
                    Arg::new("no-verify")
                        .long("no-verify")
                        .help("Write without checking P1 and P3 in memory first (`export`)")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("force")
                        .long("force")
                        .help("Write into an output directory that is not empty (`export`)")
                        .action(ArgAction::SetTrue),
                ),
            _ => sub.arg(
                Arg::new("args")
                    .num_args(0..)
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true)
                    .hide(true),
            ),
        };
        cmd = cmd.subcommand(sub);
    }
    cmd
}

/// `smysl fmt` - canonicalise surface text (§23.1).
///
/// Also verifies the `surface -> CBOR -> surface` round trip, because that is the property
/// that makes reformatting safe: hashes are computed over CBOR only, so canonicalising
/// must never move a uid. Identity drift exits 9, not 3 - it is a different kind of wrong.
fn cmd_fmt(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let check = m.get_flag("check");
    let write = m.get_flag("write");
    // `--format cbor` converts rather than formats, so it does not combine with the two flags
    // that act on a text file — the same reasoning, in the other direction, as the refusal
    // below for `--check`/`--write` on a CBOR input. Refused here, before a file is read,
    // because a conversion that half-happened across several inputs is worse than one refused.
    let to_cbor = !wants_surface(global, true);
    if to_cbor && (check || write) {
        let flag = if check { "--check" } else { "--write" };
        eprintln!(
            "smysl fmt: --format cbor converts a document; {flag} formats one in place. \
             Use -o to name the destination."
        );
        return ExitCode::Usage;
    }
    let files: Vec<String> = m
        .get_many::<String>("files")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();

    let inputs: Vec<String> = if files.is_empty() {
        vec!["-".to_string()]
    } else {
        files
    };

    let n = inputs.len();
    let mut bar = Bar::new(progress_style(global), "formatting", n);
    let mut worst = ExitCode::Success;
    let n_inputs = inputs.len();
    for path in inputs {
        bar.set_label(format!("formatting {path}"));
        bar.tick();
        let bytes = match read_bytes(&path) {
            Ok(b) => b,
            Err(e) => {
                bar.abandon();
                eprintln!("smysl fmt: {path}: {e}");
                return ExitCode::Failure;
            }
        };

        // `check` reads either form; `fmt` read only text, and failed on a CBOR store with
        // "stream did not contain valid UTF-8". That asymmetry mattered because turning a
        // store back into something readable is exactly what `fmt` is for.
        //
        // The two flags do not carry over, and are refused rather than reinterpreted.
        // `--check` asks whether a *text* file is spelled canonically; a CBOR log is
        // canonical by construction, so there is nothing to compare. `--write` would
        // replace a binary store with text in place, which is a conversion rather than a
        // formatting, and not something to do to somebody's file on the strength of a flag
        // that means something else.
        if !looks_like_surface(&bytes) {
            if check || write {
                bar.abandon();
                let flag = if check { "--check" } else { "--write" };
                eprintln!(
                    "smysl fmt: {path}: {flag} applies to surface text; a CBOR log is \
                     already canonical. Drop the flag to emit it as surface."
                );
                return ExitCode::Usage;
            }
            match cbor_to_surface(&bytes) {
                Ok(text) => {
                    bar.suspend();
                    print!("{text}");
                    continue;
                }
                Err(e) => {
                    bar.abandon();
                    eprintln!("smysl fmt: {path}: {e}");
                    return ExitCode::Failure;
                }
            }
        }

        let src = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                bar.abandon();
                eprintln!("smysl fmt: {path}: {e}");
                return ExitCode::Failure;
            }
        };

        let out = match parse_surface(&src) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("smysl fmt: {path}: {e}");
                return e.into_exit_code();
            }
        };
        bar.suspend();
        for d in &out.diagnostics {
            eprintln!("{path}: {d}");
        }
        // A document that parses with warnings is still formattable, and under `--strict` it
        // is still something a gate asked to be told about.
        if global.get_flag("strict") && out.diagnostics.iter().any(|d| !d.is_error()) {
            worst = worse(worst, ExitCode::CheckErrors);
        }
        if out.has_errors() {
            worst = worse(worst, ExitCode::CheckErrors);
            continue;
        }

        // Comments are not part of any record, so canonical form cannot reproduce them.
        // This book recommends `fmt --write` as a pre-commit habit, which makes silent
        // deletion of a reviewer's notes the difference between a formatter and a hazard.
        // Say it instead, once per file, and let the operator decide.
        if out.comments > 0 {
            eprintln!(
                "{path}: warning: {} comment line(s) are not part of any record and will \
                 not survive formatting",
                out.comments
            );
        }

        // `.with_format_version` is what stops `fmt` relabelling a document. The wire carries
        // no version, so before 0.14 the writer rebuilt the header from the first supported
        // version — correct by coincidence while there was only one, and a silent rewrite the
        // moment there were two. `fmt` is the round trip a user runs on purpose, so it is the
        // command where that would have bitten first.
        let ctx = WriteContext::from_labels(&out.labels)
            .with_salience(out.salience.clone())
            .with_format_version(out.format_version.clone());
        let formatted = write_surface(out.view.as_ref(), &out.records, &ctx);

        // The round trip must not move a uid.
        match parse_surface(&formatted) {
            Ok(again) if again.labels == out.labels && again.records == out.records => {}
            _ => {
                eprintln!("{path}: canonical form does not reproduce the same records");
                worst = worse(worst, ExitCode::HashVerification);
                continue;
            }
        }

        // The records that were just proven to round-trip, as a CBOR sequence: the view, the
        // units, their bindings. Appendix E of the SMYSL-2 draft assumed this existed; what
        // existed was the reverse conversion only, so a surface document could be read as CBOR
        // and never written as it.
        let bytes: Vec<u8> = if to_cbor {
            smysl::to_cbor_seq(&out.records)
        } else {
            formatted.clone().into_bytes()
        };

        if let Some(dest) = global.get_one::<String>("output") {
            if n_inputs > 1 {
                eprintln!(
                    "smysl fmt: --output takes one document, and {n_inputs} were given; \
                     use --write to format them in place"
                );
                return ExitCode::Usage;
            }
            if let Err(e) = std::fs::write(dest, &bytes) {
                eprintln!("smysl fmt: {dest}: {e}");
                return ExitCode::Failure;
            }
            continue;
        }

        if check {
            if formatted != src {
                eprintln!("{path}: not canonically formatted");
                worst = worse(worst, ExitCode::CheckErrors);
            }
        } else if write && path != "-" {
            if let Err(e) = std::fs::write(&path, &formatted) {
                eprintln!("smysl fmt: {path}: {e}");
                return ExitCode::Failure;
            }
        } else {
            let mut stdout = std::io::stdout().lock();
            if stdout.write_all(&bytes).is_err() {
                return ExitCode::Failure;
            }
        }
    }
    finish_over(bar, n, "formatted");
    worst
}

/// End a bar over `n` inputs: a summary when there were several, silence when there was
/// one. A "processed 1 file" line is noise, and noise is what teaches people to stop
/// reading output.
fn finish_over(bar: Bar, n: usize, verb: &str) {
    if n > 1 {
        bar.finish(&format!("{verb} {n} file(s)"));
    } else {
        bar.abandon();
    }
}

/// `smysl check` - run the check pipeline (§23.1).
///
/// Exits 3 on any error-severity diagnostic, and `--strict` promotes warnings to that
/// threshold. `check` verifies consistency, never correctness (N13): it can tell you a
/// body reaches for a unit it never declared, not whether the claim is true.
fn cmd_check(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let strict = global.get_flag("strict");
    let json = global.get_flag("json");
    // A name that is not a pass is refused, not dropped: dropped, `--pass typo` left no pass
    // named and so ran them all, and reported a clean store for the pass it was asked for.
    let mut passes: Vec<Pass> = Vec::new();
    for name in m.get_many::<String>("pass").into_iter().flatten() {
        match Pass::parse(name) {
            Some(p) => passes.push(p),
            None => {
                eprintln!(
                    "smysl check: no pass `{name}`; the passes are {}",
                    Pass::ALL
                        .iter()
                        .map(|p| format!("`{}`", p.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                return ExitCode::Usage;
            }
        }
    }

    let files: Vec<String> = m
        .get_many::<String>("files")
        .map(|v| v.cloned().collect())
        .unwrap_or_else(|| {
            global
                .get_one::<String>("store")
                .map(|s| vec![s.clone()])
                .unwrap_or_else(|| vec!["-".to_string()])
        });

    // `--as` names the schemas a consumer implements; the kernel is always implied.
    let consumer: Option<ConsumerProfile> = m.get_many::<String>("as").map(|v| {
        let schemas: Vec<SchemaId> = v.filter_map(|s| SchemaId::parse(s).ok()).collect();
        let name = if schemas.is_empty() {
            "kernel-only".to_string()
        } else {
            schemas
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("+")
        };
        ConsumerProfile::new(name).implementing(schemas)
    });

    // `--estimator` overrides the count the bounds are compared against (F-2). An id this
    // build does not have is refused here rather than carried in as `Unknown`: the caller
    // asked for a specific count, and silently reporting every unit unevaluable would answer
    // a different question.
    let estimator: Option<ProfileEstimator> = match m.get_one::<String>("estimator") {
        None => None,
        Some(id) => match TokenEstimator::parse(id) {
            Some(e) => Some(ProfileEstimator::Known(e)),
            None => {
                eprintln!("smysl check: unknown estimator `{id}`");
                eprintln!(
                    "  known: {}",
                    TokenEstimator::ALL
                        .iter()
                        .map(|e| e.id())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                return ExitCode::Usage;
            }
        },
    };

    let class: Option<ConformanceClass> = m
        .get_one::<String>("conformance")
        .and_then(|s| ConformanceClass::parse(s));
    if m.contains_id("conformance")
        && class.is_none()
        && m.get_one::<String>("conformance").is_some()
    {
        eprintln!("smysl check: unknown conformance class");
        return ExitCode::Usage;
    }

    // Ten passes over a large store is real work, and a caller checking a directory of
    // them wants to know which file it is on when one of them is slow.
    let checked = files.len();
    let mut bar = Bar::new(progress_style(global), "checking", checked);
    let mut worst = ExitCode::Success;
    for path in files {
        bar.set_label(format!("checking {path}"));
        bar.tick();
        // A store is surface text or a CBOR log, and `merge -o` writes the latter - so
        // `merge … | check` would be a documented pipeline that did not work if this only
        // read one of them.
        // `--backend redb`: the unit store, with nothing to parse — a log carries no parse
        // diagnostics and no labels outside its binding records, which `check` reads itself.
        let (out, store) = match open_backend(&path) {
            Some(Ok(store)) => (smysl::ParseOutcome::default(), store),
            Some(Err(e)) => {
                bar.abandon();
                eprintln!("smysl check: {e}");
                return ExitCode::Failure;
            }
            None => match read_store(&path) {
                Ok(o) => {
                    let store = AnyStore::from(Store::from_records(o.records.clone()));
                    (o, store)
                }
                Err(code) => {
                    bar.abandon();
                    return code;
                }
            },
        };
        let mut opts = CheckOptions::default().with_labels(out.labels.clone());
        if !passes.is_empty() {
            opts = opts.only(passes.clone());
        }
        // `--library`: the object half of the library pass. Without a resolver `SMY-E446` and
        // `SMY-E401` are not checked and the report says so, which is the honest answer — a
        // log holds no text (OQ-39), so there is nothing to check the manifests against.
        if let Some(parts) = library_resolver(m, &path) {
            opts = match parts {
                Ok(p) => opts.with_parts(p),
                Err(code) => {
                    bar.abandon();
                    return code;
                }
            };
        }
        if let Some(f) = &consumer {
            opts = opts.as_consumer(f.clone());
        }
        opts.estimator = estimator.clone();
        let mut report = check(&store, opts);
        for d in &out.diagnostics {
            report.push(d.clone());
        }
        report.sort();

        // Everything below prints; the bar comes back on the next iteration.
        bar.suspend();

        if m.get_flag("granularity") {
            for (profile, n) in granularity_distribution(&store) {
                println!("{path}: {n} view(s) at granularity {profile}");
            }
        }

        if let Some(f) = &consumer {
            let report = fidelity(&store, f);
            println!("{path}: as `{}`: {:?}", f.name, report.overall);
            for (uid, schema) in &report.degraded {
                println!("{path}:   {uid} degraded: {schema} not implemented");
            }
        }

        for d in report.iter() {
            if json {
                // `json_escape`, not `{:?}`. Rust's debug form renders a control character
                // as `\u{1}`, which no JSON parser accepts — and a diagnostic message
                // quotes document content, so an authored gist or a model's output could
                // put one there and break whatever was consuming the stream. Verified
                // against a real parser rather than assumed.
                println!(
                    "{{\"code\":\"{}\",\"severity\":\"{}\",\"message\":{}}}",
                    d.code,
                    d.severity,
                    smysl::json_escape(&d.message)
                );
            } else {
                eprintln!("{path}: {d}");
            }
        }

        if let Some(c) = class {
            let verdict = conformance(&report, c);
            println!("{path}: {verdict}");
            if !verdict.passed {
                worst = worse(worst, ExitCode::CheckErrors);
            }
        }

        let threshold = if strict {
            Severity::Warn
        } else {
            Severity::Error
        };
        if report.fail_on(threshold).is_err() {
            worst = worse(worst, ExitCode::CheckErrors);
        } else if !json && !global.get_flag("quiet") {
            // `--quiet` suppresses exactly this: the line that says it worked. Diagnostics
            // still print, the exit code still carries the verdict, and anything explicitly
            // asked for — `--json`, `--explain`, the artifact itself — is untouched. The flag
            // promised "suppress non-error output" and had only ever dimmed the progress bar.
            println!(
                "{path}: {} records, {} units, {} diagnostic(s)",
                store.len(),
                store.units().count(),
                report.len()
            );
        }
    }
    finish_over(bar, checked, "checked");
    worst
}

/// Read a store for `check`, keeping the parse diagnostics and labels a surface file
/// carries.
///
/// A CBOR log has neither: its records were validated when they were decoded, and labels
/// have no wire record. So the CBOR path returns an outcome with an empty diagnostic set,
/// which is the truth rather than a convenience.
/// Render a CBOR log as canonical surface text.
///
/// Labels come from their bindings, which is what makes the output readable rather than a
/// wall of uids; before `Record::LabelBinding` there was nothing to recover them from.
fn cbor_to_surface(bytes: &[u8]) -> Result<String, String> {
    let (records, _) = smysl::from_cbor_seq(bytes).map_err(|e| e.to_string())?;
    let labels: std::collections::BTreeMap<smysl::Label, Uid> = records
        .iter()
        .filter_map(|r| match r {
            smysl::Record::LabelBinding(b) => Some((b.label.clone(), b.uid)),
            _ => None,
        })
        .collect();
    let view = records.iter().find_map(|r| match r {
        smysl::Record::View(v) => Some(v.clone()),
        _ => None,
    });
    let ctx = WriteContext::from_labels(&labels);
    Ok(write_surface(view.as_ref(), &records, &ctx))
}

/// Whether a store's bytes are surface text rather than a CBOR log.
///
/// Surface text starts with a sigil - but it may be *preceded* by blank lines and comments,
/// so sniffing the first byte is not enough. This looks past those to the first line that
/// carries anything.
///
/// Trying UTF-8 first is safe: a CBOR record sequence opens with an array head (`0x82`),
/// which is a continuation byte and never legal at the start of UTF-8, so bytes that decode
/// cleanly are text rather than a store.
///
/// One function rather than two call sites with the same literal, because there were two and
/// adding comments to the grammar broke both - the second one silently.
fn looks_like_surface(bytes: &[u8]) -> bool {
    let Ok(s) = std::str::from_utf8(bytes) else {
        return false;
    };
    match s
        .lines()
        .find(|l| !(l.trim().is_empty() || l.starts_with('#') || l.starts_with("//")))
    {
        // Empty, or nothing but comments: a valid, empty surface document.
        None => true,
        Some(l) => l.starts_with('@'),
    }
}

fn read_store(path: &str) -> Result<smysl::ParseOutcome, ExitCode> {
    let bytes = match read_bytes(path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("smysl check: {path}: {e}");
            return Err(ExitCode::Failure);
        }
    };

    if looks_like_surface(&bytes) {
        let src = match String::from_utf8(bytes) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("smysl check: {path}: {e}");
                return Err(ExitCode::Failure);
            }
        };
        return parse_surface(&src).map_err(|e| {
            eprintln!("smysl check: {path}: {e}");
            e.into_exit_code()
        });
    }

    match smysl::from_cbor_seq(&bytes) {
        // Built from the default and adjusted: `ParseOutcome` is `#[non_exhaustive]` as of
        // 0.13, so functional-update syntax is no longer available to a consumer either. The
        // attribute is deliberate here — §8's migration to `smysl/1.0` has to add a field to
        // this type to carry the version a document declared, and an exhaustive struct would
        // have made that a crate major.
        Ok((records, _)) => {
            let mut out = smysl::ParseOutcome::default();
            out.records = records;
            Ok(out)
        }
        Err(e) => {
            eprintln!("smysl check: {path}: {e}");
            Err(ExitCode::Failure)
        }
    }
}

fn read_bytes(path: &str) -> Result<Vec<u8>, String> {
    if path == "-" {
        let mut b = Vec::new();
        std::io::stdin()
            .read_to_end(&mut b)
            .map_err(|e| e.to_string())?;
        return Ok(b);
    }
    #[cfg(feature = "text")]
    if smysl::text::library::Library::is_library(std::path::Path::new(path)) {
        return library_bytes(std::path::Path::new(path));
    }
    std::fs::read(store_file(path)?).map_err(|e| e.to_string())
}

/// The file a `-s/--store` argument names.
///
/// Itself, for a path to a log or a surface document — which is what it has meant since 1.0,
/// and §4.4 keeps. A **library root** names its catalog log instead, recognised by the
/// `LIBRARY` marker rather than by being a directory, so that every command which takes a
/// store works on a library without a second flag and a mistyped directory is still an error.
///
/// One function, called by both readers, because the alternative is each command deciding for
/// itself and `check -s <library>` working while `trace -s <library>` does not.
#[cfg(feature = "text")]
fn store_file(path: &str) -> Result<std::path::PathBuf, String> {
    use smysl::text::library::{Library, CATALOG};
    let p = std::path::Path::new(path);
    if Library::is_library(p) {
        return Ok(p.join(CATALOG));
    }
    if p.is_dir() {
        return Err(format!(
            "is a directory and has no `{}` marker, so it is not a library",
            smysl::text::library::MARKER
        ));
    }
    Ok(p.to_path_buf())
}

#[cfg(not(feature = "text"))]
fn store_file(path: &str) -> Result<std::path::PathBuf, String> {
    let p = std::path::Path::new(path);
    if p.is_dir() {
        return Err(
            "is a directory; reading a library needs the library layer (build with \
             --features text)"
                .to_string(),
        );
    }
    Ok(p.to_path_buf())
}

/// The store `--backend` chose. Memory unless set; set once, in `main`.
static BACKEND: std::sync::OnceLock<Backend> = std::sync::OnceLock::new();

/// The shards `--shard` named, for a store that is a library (SMYSL-2.8 §4.4). Set once, in
/// `main`; unset means every shard, read as the library reads them.
static SHARDS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

/// Whether `--backend redb` was asked for.
fn redb_chosen() -> bool {
    BACKEND.get().is_some()
}

/// A library's shards as one store (TX-P13c step 1): the ones `--shard` named, or all of them,
/// each through the backend `--backend` chose.
///
/// A union of **some** shards consults the library's uid lists, so a reference into a shard not
/// named says `SMY-W500` and which shard it waits on, rather than `SMY-E060`.
#[cfg(feature = "text")]
fn open_library_union(root: &std::path::Path) -> Result<AnyStore, String> {
    use smysl::text::shards;
    let at = root.display();
    let all = shards::list(root).map_err(|e| format!("{at}: {e}"))?;
    let want: Vec<(String, std::path::PathBuf)> = match SHARDS.get() {
        Some(names) if !names.iter().any(|n| n == "all") => {
            let mut picked = Vec::new();
            for n in names {
                match all.iter().find(|(name, _)| name == n) {
                    Some(found) => picked.push(found.clone()),
                    None => {
                        return Err(format!(
                            "{at}: no shard `{n}`; this library has {}",
                            all.iter()
                                .map(|(name, _)| format!("`{name}`"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ))
                    }
                }
            }
            picked.sort();
            picked.dedup();
            picked
        }
        _ => all.clone(),
    };
    let subset = want.len() < all.len();
    let index = if subset {
        Some(std::sync::Arc::new(
            shards::uid_index(root).map_err(|e| format!("{at}: {e}"))?,
        ))
    } else {
        None
    };
    // One unit store for every shard, not one a shard: a thousand stores' fixed costs were most
    // of a thousand-shard library's resident set (SMYSL-2.8 §6.1, P13c.2), and the union over
    // them held a second copy of what they held. Opened for the shards asked for.
    #[cfg(feature = "store-redb")]
    if let Some(opts) = redb_options() {
        let only: Option<Vec<String>> =
            subset.then(|| want.iter().map(|(n, _)| n.clone()).collect());
        let (store, report) = AnyStore::open_redb_shards(
            &root.join(shards::UNIT_STORE),
            &all,
            only.as_deref(),
            index,
            opts,
        )
        .map_err(|e| format!("{at}: {e}"))?;
        for d in report.report.iter() {
            eprintln!("{at}: {d}");
        }
        return Ok(store);
    }
    let names: Vec<String> = want.iter().map(|(n, _)| n.clone()).collect();
    let all_bytes = shards::read_all_bytes(root, &names).map_err(|e| format!("{at}: {e}"))?;
    let mut stores = Vec::new();
    for (name, bytes) in names.into_iter().zip(all_bytes) {
        if bytes.is_empty() {
            continue;
        }
        let (records, _) =
            smysl::from_cbor_seq(&bytes).map_err(|e| format!("{at}: {name}: {e}"))?;
        stores.push((name, AnyStore::from(Store::from_records(records))));
    }
    Ok(AnyStore::shards(stores, index))
}

/// A library's logs as one byte sequence: every shard's complete records, in name order — the
/// order the library reads them in — so a reader of one log reads a library unchanged.
#[cfg(feature = "text")]
fn library_bytes(root: &std::path::Path) -> Result<Vec<u8>, String> {
    // A torn tail stays out, as one log's reader leaves it out; a primary shard's records come
    // from the unit store (TX-P13d).
    let names: Vec<String> = smysl::text::shards::list(root)
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    Ok(smysl::text::shards::read_all_bytes(root, &names)
        .map_err(|e| e.to_string())?
        .concat())
}

/// `--shard` named a store that is not a library.
fn refuse_shard_without_library(path: &str) -> Option<String> {
    SHARDS.get()?;
    #[cfg(feature = "text")]
    if path != "-" && smysl::text::library::Library::is_library(std::path::Path::new(path)) {
        return None;
    }
    Some(format!(
        "{path}: --shard reads the shards of a library, and this is not one{}",
        if cfg!(feature = "text") {
            ""
        } else {
            " (reading a library needs --features text)"
        }
    ))
}

enum Backend {
    /// The unit store, and the command's name for its writer's lock file.
    #[cfg_attr(not(feature = "store-redb"), allow(dead_code))]
    Redb(String),
}

/// With `--backend redb`, the options a unit store is opened with: the defaults, and the
/// command's name for the lock a writer takes. `None` for the in-memory backend.
#[cfg(feature = "store-redb")]
fn redb_options() -> Option<smysl::redb::RedbOptions> {
    let Backend::Redb(command) = BACKEND.get()?;
    let mut opts = smysl::redb::RedbOptions::default();
    opts.command = command.clone();
    Some(opts)
}

/// Whether a file is a CBOR log rather than surface text, from its first byte.
///
/// A record sequence opens with an array head, which is never the first byte of UTF-8 text;
/// see [`looks_like_surface`]. One byte rather than the file, because the point of the unit
/// store is not to read the log.
#[cfg(feature = "store-redb")]
fn is_cbor_log(path: &std::path::Path) -> bool {
    let mut b = [0u8; 1];
    std::fs::File::open(path)
        .and_then(|mut f| f.read(&mut b))
        .map(|n| n == 1 && b[0] >= 0x80)
        .unwrap_or(false)
}

/// With `--backend redb` and a CBOR log, the unit store for it (SMYSL-2.8 §4.4). `None` when
/// the log is to be read into memory as before: the in-memory backend, stdin, surface text,
/// or a build without the unit store. What the open had to say — a rebuild, a torn tail, a
/// store another process held — goes to stderr.
fn open_backend(path: &str) -> Option<Result<AnyStore, String>> {
    if let Some(e) = refuse_shard_without_library(path) {
        return Some(Err(e));
    }
    // A library under `--shard` or `--backend redb`: its shards as a union, each through the
    // backend. Without either it is read as one log, below, which is what it always was.
    #[cfg(feature = "text")]
    if path != "-"
        && smysl::text::library::Library::is_library(std::path::Path::new(path))
        && (SHARDS.get().is_some() || redb_chosen())
    {
        return Some(open_library_union(std::path::Path::new(path)));
    }
    #[cfg(feature = "store-redb")]
    {
        let opts = redb_options()?;
        if path == "-" {
            return None;
        }
        let file = match store_file(path) {
            Ok(f) => f,
            Err(e) => return Some(Err(format!("{path}: {e}"))),
        };
        if !is_cbor_log(&file) {
            return None;
        }
        Some(match AnyStore::open_redb(&file, opts) {
            Ok((store, report)) => {
                for d in report.report.iter() {
                    eprintln!("{path}: {d}");
                }
                Ok(store)
            }
            Err(e) => Err(format!("{path}: {e}")),
        })
    }
    #[cfg(not(feature = "store-redb"))]
    {
        let _ = path;
        None
    }
}

/// Read a store from surface text or a CBOR log, whichever it turns out to be.
///
/// An [`AnyStore`] since TX-P13a step 11 (SMYSL-2.8 §4.4): the in-memory backend today, and the
/// type `--backend` will choose between. Commands that only read take it as it is — every
/// operation takes a `StoreRead` — and the ones that need the log's own methods ask it for the
/// in-memory store with [`memory`].
fn load_store(
    path: &str,
) -> Result<(AnyStore, std::collections::BTreeMap<smysl::Label, Uid>), String> {
    if let Some(store) = open_backend(path) {
        let store = store?;
        let labels = store
            .hot_records(smysl::HotKind::LabelBindings)
            .filter_map(|r| match r {
                smysl::Record::LabelBinding(b) => Some((b.label.clone(), b.uid)),
                _ => None,
            })
            .collect();
        return Ok((store, labels));
    }
    let bytes = read_bytes(path).map_err(|e| {
        if path == "-" {
            e
        } else {
            format!("{path}: {e}")
        }
    })?;

    if looks_like_surface(&bytes) {
        let src = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        let out = parse_surface(&src).map_err(|e| e.to_string())?;
        Ok((Store::from_records(out.records.clone()).into(), out.labels))
    } else {
        let (records, _) = smysl::from_cbor_seq(&bytes).map_err(|e| e.to_string())?;
        // Recover the labels from their bindings. This is the whole point of the record
        // existing: before it, this arm returned an empty map, so every reference in a
        // re-emitted store came back as a bare uid - valid, checkable, and unreadable.
        let labels = records
            .iter()
            .filter_map(|r| match r {
                smysl::Record::LabelBinding(b) => Some((b.label.clone(), b.uid)),
                _ => None,
            })
            .collect();
        Ok((Store::from_records(records).into(), labels))
    }
}

/// [`load_store`], for a command that needs the in-memory store: one that appends, bundles,
/// or hands the store to a crate whose operations take `&Store` (the library, query and
/// measure layers, which TX-P13a does not port).
fn load_memory(
    path: &str,
) -> Result<(Store, std::collections::BTreeMap<smysl::Label, Uid>), String> {
    load_store(path).map(|(s, labels)| (s.into_memory(), labels))
}

/// Why a unit argument names no unit, with the exit code that says so.
struct Unresolved {
    code: ExitCode,
    message: String,
}

impl std::fmt::Display for Unresolved {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Resolve a unit argument against a store: a uid, its display form, or a label.
///
/// Labels were refused until 1.3 — `smysl trace c/pool-saturation` said "is not a uid" — on the
/// reasoning that a label "has no existence at the store level", which stopped being true when
/// `Record::LabelBinding` put labels on the wire in 0.2.
///
/// A label is resolved by `smysl::resolve_label`, against the store's binding records, and
/// **refused when it is bound to more than one unit**. The first version of this took a map
/// built by inserting bindings into a `BTreeMap`, so in a store merged from several extraction
/// runs the last binding in record order won, silently — and `retract` by that label retracted
/// a unit the caller had not chosen. Ambiguity exits 5, as `relink` does for a supersession
/// fork: the same kind of refusal, a tool declining to adjudicate.
///
/// Unambiguous by construction: a uid is spelled `b3:…` and a label never contains `:`. One
/// helper for every command that takes a unit, so none can disagree with another.
fn resolve(store: &(impl StoreRead + ?Sized), raw: &str) -> Result<Uid, Unresolved> {
    if let Ok(label) = smysl::Label::new(raw) {
        return smysl::resolve_label(store, &label).map_err(|e| match e {
            smysl::LabelError::Ambiguous(uids) => Unresolved {
                code: ExitCode::Contentions,
                message: format!(
                    "`{raw}` is bound to {} different units; name one by uid:{}",
                    uids.len(),
                    uids.iter()
                        .map(|u| format!(
                            "\n  {}  {}",
                            u.canonical(),
                            store
                                .get(u)
                                .map(|x| x.core.gist.clone())
                                .unwrap_or_else(|| "(not in this store)".into())
                        ))
                        .collect::<String>()
                ),
            },
            _ => Unresolved {
                code: ExitCode::Failure,
                message: format!("`{raw}` is a label, and nothing in this store is bound to it"),
            },
        });
    }
    let prefix = UidPrefix::parse(raw).map_err(|_| Unresolved {
        code: ExitCode::Failure,
        message: format!("`{raw}` is neither a uid nor a label"),
    })?;
    store.resolve_prefix(&prefix).map_err(|e| Unresolved {
        code: ExitCode::Failure,
        message: e.to_string(),
    })
}

/// The store argument, from the subcommand or the global flag.
fn store_arg(m: &ArgMatches, global: &ArgMatches) -> Option<String> {
    m.get_one::<String>("store")
        .or_else(|| global.get_one::<String>("store"))
        .cloned()
}

/// `smysl diff` - what changed, and who changed it (§23.1).
fn cmd_diff(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let inputs: Vec<String> = m
        .get_many::<String>("inputs")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();

    if let Some(range) = m.get_one::<String>("hop") {
        let Some((a, b)) = range.split_once("..") else {
            eprintln!("smysl diff: --hop takes A..B");
            return ExitCode::Usage;
        };
        let (Ok(from), Ok(to)) = (a.parse::<u32>(), b.parse::<u32>()) else {
            eprintln!("smysl diff: --hop takes two hop numbers");
            return ExitCode::Usage;
        };
        let path = &inputs[0];
        let (store, _) = match load_store(path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("smysl diff: {e}");
                return ExitCode::Failure;
            }
        };

        let d = smysl::hop_diff(&store, from, to, m.get_flag("recipe"));
        if d.total() == 0 && store.units().count() > 0 {
            // Silence here would read as "nothing changed" when it means "nothing could be
            // placed in time".
            eprintln!(
                "{path}: {} unit(s), none attested - a store with no provenance cannot be \
                 asked what changed when",
                store.units().count()
            );
            return ExitCode::Success;
        }
        println!(
            "{path}: hop {from}..{to}: {} survived, {} superseded, {} retracted, {} added",
            d.survived.len(),
            d.superseded.len(),
            d.retracted.len(),
            d.added.len()
        );
        println!("{path}: survival rate {:.3}", d.survival_rate());
        for (old, new) in &d.superseded {
            println!("{path}:   {old} superseded by {new}");
        }
        for u in &d.retracted {
            println!("{path}:   {u} retracted");
        }
        if m.get_flag("by-agent") {
            for (agent, act) in &d.by_agent {
                println!(
                    "{path}:   {agent}: +{} ~{} -{}",
                    act.added, act.superseded, act.retracted
                );
            }
        }
        for c in &d.recipe_changes {
            println!("{path}:   {} {}", c.uid, c.kind.as_str());
        }
        return ExitCode::Success;
    }

    if inputs.len() != 2 {
        eprintln!("smysl diff: two stores, or one with --hop");
        return ExitCode::Usage;
    }
    let mut stores = Vec::new();
    for p in &inputs {
        match load_store(p) {
            Ok((s, _)) => stores.push(s),
            Err(e) => {
                eprintln!("smysl diff: {e}");
                return ExitCode::Failure;
            }
        }
    }
    let d = smysl::diff(&stores[0], &stores[1]);
    if global.get_flag("json") {
        // One object, not a line per uid: a diff is a single answer about two stores, and
        // a caller wants the three sets rather than to reassemble them from prefixes.
        println!(
            "{{\"only_in_a\":[{}],\"only_in_b\":[{}],\"common\":[{}]}}",
            uid_array(&d.only_in_a),
            uid_array(&d.only_in_b),
            uid_array(&d.common)
        );
        return ExitCode::Success;
    }
    println!(
        "{} only, {} only, {} common",
        d.only_in_a.len(),
        d.only_in_b.len(),
        d.common.len()
    );
    for u in &d.only_in_a {
        println!("- {u}");
    }
    for u in &d.only_in_b {
        println!("+ {u}");
    }
    ExitCode::Success
}

/// A comma-separated JSON array of quoted uids.
///
/// Canonical form, not the display abbreviation: a machine reading this needs the identity,
/// and `SMY-E071` exists because a short uid in a record silently weakens it.
fn uid_array<'a>(uids: impl IntoIterator<Item = &'a Uid>) -> String {
    uids.into_iter()
        .map(|u| smysl::json_escape(&u.canonical()))
        .collect::<Vec<_>>()
        .join(",")
}

/// Which retrieval engine a command should use (1.7).
///
/// `Hybrid` has been measured against the shared query set since 0.8 — 0.84 MRR to lexical's
/// 0.74, and 0.50 to 0.12 on paraphrase, with identifier-shaped queries routed to lexical for
/// its perfect precision — and none of that was reachable from the command line: `find` and
/// `pack --query` built a `Bm25` unconditionally, so even a build compiled with
/// `--features semantic` retrieved lexically. The engine was built, measured, exported and
/// unplugged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    Lexical,
    Semantic,
    Hybrid,
}

impl Engine {
    fn parse(s: &str) -> Option<Engine> {
        match s {
            "lexical" => Some(Engine::Lexical),
            "semantic" => Some(Engine::Semantic),
            "hybrid" => Some(Engine::Hybrid),
            _ => None,
        }
    }
}

/// The engine and model a retrieval command was asked for.
///
/// The model directory comes from `--model` or `SMYSL_EMBED_MODEL`. Nothing is downloaded and
/// nothing is guessed: a semantic run without a model is an error saying so, because silently
/// falling back to lexical would report numbers from an engine the caller did not ask for.
fn engine_for(m: &ArgMatches, cmd: &str) -> Result<(Engine, Option<String>), ExitCode> {
    let engine = match m.get_one::<String>("engine") {
        Some(raw) => match Engine::parse(raw) {
            Some(e) => e,
            None => {
                eprintln!("smysl {cmd}: `{raw}` is not an engine: lexical, semantic, hybrid");
                return Err(ExitCode::Usage);
            }
        },
        None => Engine::Lexical,
    };
    let model = m
        .get_one::<String>("model")
        .cloned()
        .or_else(|| std::env::var("SMYSL_EMBED_MODEL").ok());
    if engine != Engine::Lexical && model.is_none() {
        eprintln!(
            "smysl {cmd}: --engine {} needs a model: pass --model DIR or set SMYSL_EMBED_MODEL",
            match engine {
                Engine::Semantic => "semantic",
                _ => "hybrid",
            }
        );
        return Err(ExitCode::Usage);
    }
    Ok((engine, model))
}

/// Rank `query` against `store` with the chosen engine.
#[cfg(feature = "semantic")]
fn ranked(
    store: &(impl StoreRead + ?Sized),
    query: &smysl::Query,
    engine: Engine,
    model: Option<&str>,
    cmd: &str,
) -> Result<Vec<smysl::Hit>, ExitCode> {
    use smysl::Retriever as _;
    let lexical = smysl::Bm25::index(store);
    if engine == Engine::Lexical {
        return Ok(lexical.search(query));
    }
    let dir = model.expect("engine_for refuses a model-less semantic run");
    let loaded = match smysl::EmbedModel::from_dir(dir) {
        Ok(mo) => mo,
        Err(e) => {
            eprintln!("smysl {cmd}: {e}");
            return Err(ExitCode::Failure);
        }
    };
    let semantic = smysl::Semantic::index(store, loaded);
    Ok(match engine {
        Engine::Semantic => semantic.search(query),
        _ => smysl::Hybrid::new(lexical, semantic).search(query),
    })
}

/// The same, for a build with no embedder compiled in.
#[cfg(not(feature = "semantic"))]
fn ranked(
    store: &(impl StoreRead + ?Sized),
    query: &smysl::Query,
    engine: Engine,
    _model: Option<&str>,
    cmd: &str,
) -> Result<Vec<smysl::Hit>, ExitCode> {
    use smysl::Retriever as _;
    if engine != Engine::Lexical {
        eprintln!(
            "smysl {cmd}: this build has no semantic retrieval (build with --features semantic)"
        );
        return Err(ExitCode::Failure);
    }
    Ok(smysl::Bm25::index(store).search(query))
}

/// The analyzer `find` was told to use, owning it for the length of the query (1.10).
///
/// Two variants because the two analyzer families live in two crates for a reason: the
/// tokeniser splits identifiers and belongs with retrieval, the chains fold and stem and belong
/// with language. A build without `--features text` has only the first, and says so rather than
/// quietly analysing Russian with an English fold.
enum Chosen {
    Tok(smysl::Tokenizer),
    #[cfg(feature = "text")]
    Chain(smysl::text::analyze::Chain),
}

impl Chosen {
    fn as_analyze(&self) -> &dyn smysl::Analyze {
        match self {
            Chosen::Tok(t) => t,
            #[cfg(feature = "text")]
            Chosen::Chain(c) => c,
        }
    }

    fn id(&self) -> &str {
        self.as_analyze().id()
    }
}

/// Every analyzer id this build can produce, for a message that has to list them.
fn analyzer_ids() -> Vec<String> {
    let mut out: Vec<String> = smysl::retrieve_analyze::ids().map(str::to_string).collect();
    #[cfg(feature = "text")]
    out.extend(smysl::text::analyze::ids().map(str::to_string));
    out
}

/// The analyzer named by `--analyzer`, or chosen by `--lang`, or `None` when neither was given.
///
/// An id this build cannot produce is a usage error and not a fallback: a query analysed by a
/// chain other than the one it named would retrieve almost nothing and report it as a ranking.
fn chosen_analyzer(m: &ArgMatches, cmd: &str) -> Result<Option<Chosen>, ExitCode> {
    if let Some(id) = m.get_one::<String>("analyzer") {
        if let Some(tok) = smysl::retrieve_analyze::by_id(id) {
            return Ok(Some(Chosen::Tok(tok)));
        }
        #[cfg(feature = "text")]
        if let Some(chain) = smysl::text::analyze::by_id(id) {
            return Ok(Some(Chosen::Chain(chain)));
        }
        eprintln!("smysl {cmd}: this build cannot produce the analyzer `{id}`");
        eprintln!("  it has: {}", analyzer_ids().join(", "));
        // The stemming chains are behind a feature for a dependency reason, so a missing id is
        // as often a build as a typo.
        eprintln!("  the stemming chains (smysl/an-<lang>/1) need --features stem on smysl-text");
        return Err(ExitCode::Usage);
    }
    let Some(raw) = m.get_one::<String>("lang") else {
        return Ok(None);
    };
    let Ok(tag) = smysl::LangTag::new(raw.as_str()) else {
        eprintln!("smysl {cmd}: `{raw}` is not a language tag");
        return Err(ExitCode::Usage);
    };
    #[cfg(feature = "text")]
    {
        Ok(Some(Chosen::Chain(smysl::text::analyze::Chain::new(&tag))))
    }
    // Without the library layer there are no chains, and `folding_for` is the one
    // language-dependent decision this tier can make: fold for `en`, leave everything else as
    // written (F-4).
    #[cfg(not(feature = "text"))]
    {
        Ok(Some(Chosen::Tok(smysl::Tokenizer::folding_for(&tag))))
    }
}

/// `find` over an analyzer the caller chose, and over persistent postings when asked.
///
/// The slow path is the one `find` has always taken, with the analyzer swapped: analyse the
/// corpus, then score it. The fast path is `--library`, which scores the `postings` table the
/// substrate index already holds — the work is the same work, done once by whoever last wrote
/// to the library.
#[cfg(feature = "text")]
fn find_with_chosen(
    store: &Store,
    q: &smysl::Query,
    m: &ArgMatches,
    path: &str,
    cmd: &str,
) -> Result<Vec<smysl::Hit>, ExitCode> {
    use smysl::Retriever as _;

    let chosen = chosen_analyzer(m, cmd)?;
    if !m.get_flag("library") {
        return Ok(in_memory_hits(store, q, chosen));
    }

    let library = match smysl::text::library::Library::open(std::path::Path::new(path)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl {cmd}: {path}: {e}");
            return Err(ExitCode::Failure);
        }
    };

    // The analyzer is settled before it is known whether the table exists, so that the fast path
    // and the fallback split the query the same way. They did not, in the first version of this:
    // with no postings table the fallback took `Bm25::index`'s plain tokeniser and returned a
    // different score for the same query than the table would have, which is the silent
    // difference this whole step is trying not to have.
    let chosen = match chosen {
        Some(c) => c,
        None => library_chain(&library),
    };

    // Whichever file holds them. The sidecar's table is scored where it lies; a library whose
    // postings are in the backing store is scored a term at a time out of that store, and the
    // ranking is the same one — `crates/smysl-text/tests/substrate_db.rs` compares the two bit
    // for bit. The flag does not say which, because a caller asking for the index it has
    // already paid for does not care which file it is in.
    let Some(view) = library.postings() else {
        // An open builds eight of the twelve tables and `postings` is not one of them, for the
        // reason `Table::ON_OPEN` records. Falling back is right — it is the same ranking — and
        // saying so is right too, because the point of the flag was not to pay for it.
        if library.has_postings_store() && !cfg!(feature = "substrate-redb") {
            // The one case with a different remedy: the rows are written down, in a file this
            // binary has no reader for. A `--rebuild` would not help, so it is not offered.
            eprintln!(
                "smysl {cmd}: {path}: the postings are in {}, which this build cannot read; \
                 analysing the corpus instead",
                smysl::text::index::STORE
            );
            eprintln!("  a build with `--features substrate-redb` reads it");
        } else {
            eprintln!(
                "smysl {cmd}: {path}: the index holds no postings yet; analysing the corpus instead"
            );
            eprintln!(
                "  `smysl text index --rebuild {path}` builds them once and writes them down"
            );
        }
        return Ok(in_memory_hits(library.store(), q, Some(chosen)));
    };

    // A table analysed by a chain the query was not split by retrieves only the terms the two
    // chains happen to agree about, which is a thin result rather than an error. Said once,
    // rather than refused: per-document routing is what the partitions are *for*, so a
    // multi-language library legitimately holds several chains and no one of them is the right
    // way to split a query the caller did not qualify.
    //
    // The ids come from the summary rather than from the resident table, because the table may
    // not be resident: a stored partition carries its analyzer in the row beside its document
    // count, which is what makes this question answerable without reading the postings.
    let ids: Vec<String> = library
        .postings_held()
        .map(|held| held.analyzers)
        .unwrap_or_default();
    if !ids.is_empty() && !ids.iter().any(|id| id == chosen.id()) {
        eprintln!(
            "smysl {cmd}: {path}: the query is split by `{}` and this library is analysed by \
             {} — only the terms they agree about can match",
            chosen.id(),
            ids.join(", ")
        );
        eprintln!("  --analyzer names one; `smysl text index {path}` lists them");
    }
    Ok(smysl::Bm25::over(&view, chosen.as_analyze(), library.store()).search(q))
}

/// The chain a library's own texts imply, for a query that named none.
///
/// The language of its texts when they agree about it, and the plain chain otherwise. Not a
/// refusal: a library of texts in three languages is a thing to have, and the terms of a query
/// have to be produced by *something* before anything can match. What the caller gets instead of
/// a refusal is a line saying which chains the index holds, and `--analyzer` to name one.
///
/// Head manifests only. A superseded text is not what the library is about any more, and its
/// language is not either.
#[cfg(feature = "text")]
fn library_chain(library: &smysl::text::library::Library) -> Chosen {
    use std::collections::BTreeSet;
    let langs: BTreeSet<&str> = library
        .index()
        .manifests
        .values()
        .filter(|row| row.head)
        .map(|row| row.lang.as_str())
        .collect();
    let mut it = langs.into_iter();
    match (it.next(), it.next()) {
        (Some(lang), None) => match smysl::LangTag::new(lang) {
            Ok(tag) => Chosen::Chain(smysl::text::analyze::Chain::new(&tag)),
            Err(_) => Chosen::Chain(smysl::text::analyze::Chain::plain()),
        },
        _ => Chosen::Chain(smysl::text::analyze::Chain::plain()),
    }
}

/// The same, for a build with no library layer compiled in./// The same, for a build with no library layer compiled in.
#[cfg(not(feature = "text"))]
fn find_with_chosen(
    store: &(impl StoreRead + ?Sized),
    q: &smysl::Query,
    m: &ArgMatches,
    _path: &str,
    cmd: &str,
) -> Result<Vec<smysl::Hit>, ExitCode> {
    let chosen = chosen_analyzer(m, cmd)?;
    if m.get_flag("library") {
        eprintln!("smysl {cmd}: this build has no library layer (build with --features text)");
        return Err(ExitCode::Usage);
    }
    Ok(in_memory_hits(store, q, chosen))
}

/// Analyse the corpus and score it, which is what `find` does without `--library`.
///
/// Through the postings seam rather than `Bm25::index_with`, because the chains are not
/// tokenisers and `index_with` takes a `Tokenizer`. The two produce bit-identical scores for the
/// same analyzer (`smysl-retrieve/tests/postings.rs`), so this is the same ranking by another
/// road; with no analyzer named it is literally the same call `find` has always made.
fn in_memory_hits(store: &Store, q: &smysl::Query, chosen: Option<Chosen>) -> Vec<smysl::Hit> {
    use smysl::Retriever as _;
    match chosen {
        None => smysl::Bm25::index(store).search(q),
        Some(c) => {
            let analyze = c.as_analyze();
            let postings = smysl::MemPostings::index(store, analyze);
            smysl::Bm25::over(&postings, analyze, store).search(q)
        }
    }
}

/// A payload restriction from `KEY=VALUE[,VALUE]` (1.6).
///
/// Shared by `find` and `pack` so the two cannot drift into reading the same argument
/// differently. An empty key or an empty value list is refused rather than treated as "no
/// restriction": a caller that wrote `--payload` meant to narrow something.
fn payload_filter(raw: &str, cmd: &str) -> Result<(String, Vec<String>), ExitCode> {
    let complaint = || {
        eprintln!("smysl {cmd}: --payload takes KEY=VALUE, with values comma-separated");
        ExitCode::Usage
    };
    let Some((key, values)) = raw.split_once('=') else {
        return Err(complaint());
    };
    let values: Vec<String> = values
        .split(',')
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .collect();
    if key.trim().is_empty() || values.is_empty() {
        return Err(complaint());
    }
    Ok((key.trim().to_string(), values))
}

/// An edge set from a comma-separated list: a preset, or relation kinds.
///
/// Extension kinds are interned per store, so the store resolves them; a kind nothing in the store
/// uses is refused rather than silently matching nothing, which would read as "no dependencies".
fn edge_set(
    store: &(impl StoreRead + ?Sized),
    raw: &str,
    cmd: &str,
) -> Result<smysl::EdgeSet, ExitCode> {
    let mut out = smysl::EdgeSet::of([]);
    for word in raw.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        match word {
            "premises" => out = out.union(&smysl::EdgeSet::premises()),
            "dependency" => out = out.union(&smysl::EdgeSet::dependency()),
            "support" => out = out.union(&smysl::EdgeSet::support()),
            "deps" => out = out.with(smysl::EdgeKind::Deps),
            "grounds" => out = out.with(smysl::EdgeKind::Grounds),
            other => match smysl::RelKind::parse(other)
                .ok()
                .and_then(|k| store.adjacency().edge_kind(&k))
            {
                Some(kind) => out = out.with(kind),
                None => {
                    eprintln!(
                        "smysl {cmd}: `{other}` is not a relation kind in this store, nor a \
                         preset (premises, dependency, support)"
                    );
                    return Err(ExitCode::Usage);
                }
            },
        }
    }
    Ok(out)
}

/// `smysl trace` - walk a unit's ancestry (§23.1). The direct answer to F3.
fn cmd_trace(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let Some(path) = store_arg(m, global) else {
        eprintln!("smysl trace: no store given");
        return ExitCode::Usage;
    };
    let (store, _) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl trace: {e}");
            return ExitCode::Failure;
        }
    };
    let target = match resolve(&store, m.get_one::<String>("uid").expect("required")) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("smysl trace: {e}");
            return e.code;
        }
    };

    let kind = if m.get_flag("both") {
        TraceKind::Both
    } else if m.get_flag("parents") {
        TraceKind::Parents
    } else {
        TraceKind::Grounds
    };
    let depth = m.get_one::<String>("depth").and_then(|s| s.parse().ok());

    let l = match m.get_one::<String>("via") {
        Some(raw) => match edge_set(&store, raw, "trace") {
            Ok(edges) => smysl::trace_via(&store, target, &edges, depth),
            Err(code) => return code,
        },
        None => smysl::trace(&store, target, kind, depth),
    };
    if global.get_flag("json") {
        // The tree's shape is what a caller wants, so `depth` and `via` travel per node
        // rather than being encoded as indentation a machine would have to count.
        let nodes: Vec<String> = l
            .nodes
            .iter()
            .map(|n| {
                let hop = n
                    .hop
                    .map(|h| h.to_string())
                    .unwrap_or_else(|| "null".into());
                let agents: Vec<String> = n
                    .agents
                    .iter()
                    .map(|a| smysl::json_escape(&a.to_string()))
                    .collect();
                format!(
                    "{{\"uid\":{},\"depth\":{},\"via\":{},\"hop\":{},\"agents\":[{}]}}",
                    smysl::json_escape(&n.uid.canonical()),
                    n.depth,
                    smysl::json_escape(n.via.as_str()),
                    hop,
                    agents.join(",")
                )
            })
            .collect();
        println!(
            "{{\"units\":{},\"steps\":{},\"nodes\":[{}]}}",
            l.len(),
            l.max_depth(),
            nodes.join(",")
        );
        return ExitCode::Success;
    }
    for n in &l.nodes {
        let indent = "  ".repeat(n.depth as usize);
        let hop = n.hop.map(|h| format!(" @hop{h}")).unwrap_or_default();
        let agents = if m.get_flag("agents") && !n.agents.is_empty() {
            let names: Vec<String> = n.agents.iter().map(ToString::to_string).collect();
            format!("  [{}]", names.join(", "))
        } else {
            String::new()
        };
        println!("{indent}{} ({}){hop}{agents}", n.uid, n.via.as_str());
    }
    println!("{path}: {} unit(s) over {} step(s)", l.len(), l.max_depth());
    ExitCode::Success
}

/// `smysl view` - define or print a view (§23.1).
///
/// A view is a name plus roots plus threads. It is never a container: membership is
/// computed from the roots, so nothing is copied and nothing is owned.
fn cmd_view(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let Some(path) = store_arg(m, global) else {
        eprintln!("smysl view: no store given");
        return ExitCode::Usage;
    };
    warn_output_is_a_report(global, "view");
    let (store, _) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl view: {e}");
            return ExitCode::Failure;
        }
    };

    let roots: Vec<Uid> = match m.get_many::<String>("roots") {
        Some(v) => {
            let mut out = Vec::new();
            for raw in v {
                match resolve(&store, raw) {
                    Ok(u) => out.push(u),
                    Err(e) => {
                        eprintln!("smysl view: {e}");
                        return e.code;
                    }
                }
            }
            out
        }
        None => Vec::new(),
    };

    let view = if roots.is_empty() {
        match store.views().next() {
            Some(v) => v.clone(),
            None => {
                eprintln!("smysl view: no roots given and the store declares no view");
                return ExitCode::Usage;
            }
        }
    } else {
        let id = m
            .get_one::<String>("id")
            .and_then(|s| ViewId::new(s).ok())
            .unwrap_or_else(|| ViewId::new("v/ad-hoc").expect("literal"));
        let mut v = View::new(id, "ad-hoc").with_roots(roots);
        if let Some(t) = m.get_many::<String>("threads") {
            v = v.with_threads(t.filter_map(|s| smysl::ThreadId::new(s).ok()));
        }
        if let Some(r) = m.get_many::<String>("requires") {
            v = v.requiring(r.filter_map(|s| SchemaId::parse(s).ok()));
        }
        v
    };

    let members = smysl::membership(&store, &view.roots);
    if global.get_flag("json") {
        println!(
            "{{\"view\":{},\"roots\":[{}],\"threads\":[{}],\"reachable\":[{}]}}",
            smysl::json_escape(view.id.as_str()),
            uid_array(&view.roots),
            view.threads
                .iter()
                .map(|t| smysl::json_escape(t.as_str()))
                .collect::<Vec<_>>()
                .join(","),
            uid_array(&members)
        );
        return ExitCode::Success;
    }
    println!(
        "{}: {} root(s), {} thread(s), {} unit(s) reachable",
        view.id,
        view.roots.len(),
        view.threads.len(),
        members.len()
    );
    for u in &members {
        println!("  {u}");
    }
    ExitCode::Success
}

/// `smysl bundle` - the reachable closure, as a portable store (§23.1).
fn cmd_bundle(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let Some(path) = store_arg(m, global) else {
        eprintln!("smysl bundle: no store given");
        return ExitCode::Usage;
    };
    let (store, _) = match load_memory(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl bundle: {e}");
            return ExitCode::Failure;
        }
    };

    let wanted = m.get_one::<String>("view");
    let view = match wanted {
        Some(id) => store.views().find(|v| v.id.as_str() == id).cloned(),
        None => store.views().next().cloned(),
    };
    let Some(view) = view else {
        eprintln!("smysl bundle: no such view");
        return ExitCode::Usage;
    };

    // Bindings travel with the bundle because `bundle_with` includes them - fixed in the
    // library rather than here, since a library caller building a bundle needs a readable
    // one just as much as the CLI does (rule A).
    // Rule X keeps records this build cannot name; `--unknown drop` is the escape for a
    // sender who must not forward what they cannot inspect, and so cannot evaluate for
    // redaction either (rule Z).
    let unknown = match m.get_one::<String>("unknown").map(String::as_str) {
        Some("drop") => smysl::UnknownRecords::Drop,
        _ => smysl::UnknownRecords::Keep,
    };
    let (bytes, report) =
        store.bundle_with_options(&view, m.get_flag("include-retracted"), unknown);
    // `SMY-W434`: forwarding what you could not read is a decision, and a silent one is how a
    // sender comes to believe they have read what they sent.
    if report.unknown_records > 0 {
        let what = if unknown == smysl::UnknownRecords::Drop {
            "left out of"
        } else {
            "carried in"
        };
        eprintln!(
            "smysl bundle: {}: {} record(s) of a type this build cannot interpret {what} the bundle",
            smysl::Code::W434,
            report.unknown_records
        );
    }

    // `--format surface` used to be accepted and ignored, so `bundle --format surface` wrote
    // CBOR to a terminal — the identical mistake `merge --format surface` shipped with, in the
    // command next door. The text form is rendered from the bundle's own bytes rather than
    // from the store a second time: a second selection could disagree with the first, and the
    // one artifact whose purpose is to be self-contained is the worst place for that.
    let bytes = if wants_surface(global, false) {
        let (records, _) = match smysl::from_cbor_seq(&bytes) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("smysl bundle: {e}");
                return ExitCode::Failure;
            }
        };
        let labels: std::collections::BTreeMap<smysl::Label, Uid> = records
            .iter()
            .filter_map(|r| match r {
                Record::LabelBinding(b) => Some((b.label.clone(), b.uid)),
                _ => None,
            })
            .collect();
        let ctx = WriteContext::from_labels(&labels);
        warn_surface_omissions("bundle", &records, Some(&view.id), &ctx);
        write_surface(Some(&view), &records, &ctx).into_bytes()
    } else {
        bytes
    };
    emit(global, "bundle", &bytes)
}

/// `smysl merge` - join-semilattice union, materialising disagreement (§23.1).
///
/// Merge never adjudicates. Where two agents disagree the disagreement becomes an object
/// in the report rather than a winner in the store.
fn cmd_merge(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let inputs: Vec<String> = m
        .get_many::<String>("inputs")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();

    // Rule S's other half: `--staged` is the confirmation `ingest` exits 10 waiting for.
    // It is a merge like any other, because a staged batch is just records - which is
    // exactly why staging can be reviewed with `cat` and reverted with `rm`.
    #[cfg(feature = "ingest")]
    let staged_records = if m.get_flag("staged") {
        match smysl::stage::read(project_root(global)) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("smysl merge: {e}");
                return ExitCode::Failure;
            }
        }
    } else {
        Vec::new()
    };
    #[cfg(not(feature = "ingest"))]
    let staged_records: Vec<Record> = {
        if m.get_flag("staged") {
            eprintln!("smysl merge: this build has no ingest layer (build with --features local)");
            return ExitCode::Usage;
        }
        Vec::new()
    };

    let mut opts = MergeOptions::default();
    if let Some(p) = m
        .get_one::<String>("policy")
        .and_then(|s| SupersessionPolicy::parse(s))
    {
        opts.supersession = p;
    }
    if let Some(p) = m
        .get_one::<String>("retraction")
        .and_then(|s| RetractionPolicy::parse(s))
    {
        opts.retraction = p;
    }
    opts.fail_on_contention = m.get_flag("fail-on-contention");
    opts.max_contentions_per_agent = m
        .get_one::<String>("max-contentions-per-agent")
        .and_then(|s| s.parse().ok());
    // A supplied clock keeps merge bit-reproducible; `smysl merge A B` twice is the same
    // bytes twice, which is what the determinism gate asserts.
    opts.now = Some(Hlc::new(
        0,
        0,
        AgentId::new("tool:smysl-merge").expect("literal"),
    ));

    // Merge is a fold over stores, and the interesting case is many of them.
    let merged = inputs.len();
    let mut bar = Bar::new(progress_style(global), "merging", merged);
    let mut store = Store::new();
    let mut labels = Vec::new();
    // `--strict` folds every input's report together: a warning on the third of five stores
    // is still a warning the gate asked to hear about.
    let mut strict_failed = false;
    for path in &inputs {
        bar.set_label(format!("merging {path}"));
        bar.tick();
        let (s, l) = match load_store(path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("smysl merge: {e}");
                return ExitCode::Failure;
            }
        };
        labels.push(l);
        opts.labels = labels.clone();
        let outcome = merge(&mut store, &s, opts.clone());
        bar.suspend();
        match outcome {
            Ok(r) => {
                for d in r.report.iter() {
                    eprintln!("{path}: {d}");
                }
                strict_failed |= strict_threshold(global, &r.report).is_some();
                for c in &r.new_contentions {
                    eprintln!(
                        "{path}: contention {} over {} ({} positions, {})",
                        c.id,
                        c.over,
                        c.positions.len(),
                        c.detected.kind
                    );
                }
            }
            Err(e) => {
                eprintln!("smysl merge: {e}");
                let code: ExitCode = e.exit_code();
                return code;
            }
        }
    }
    finish_over(bar, merged, "merged");

    if !staged_records.is_empty() {
        let n = staged_records.len();
        let staged = Store::from_records(staged_records);
        match merge(&mut store, &staged, opts.clone()) {
            Ok(r) => {
                for d in r.report.iter() {
                    eprintln!("staged: {d}");
                }
                strict_failed |= strict_threshold(global, &r.report).is_some();
                if !global.get_flag("quiet") {
                    eprintln!("smysl merge: committed {n} staged record(s)");
                }
            }
            Err(e) => {
                eprintln!("smysl merge: {e}");
                return e.exit_code();
            }
        }
    }

    let out = global.get_one::<String>("output");

    // `--format` is global and means the same thing here as everywhere else: the *shape*
    // of the output. Emitting the log unconditionally made a merged store the one artifact
    // in the tool a person could not read back - `fmt` takes surface text, so there was no
    // route from a merge result to something reviewable. Since the format's whole claim is
    // that the wire form and the review copy are the same document, that gap mattered more
    // than the line count of this branch suggests.
    //
    // Labels are folded across inputs first-wins: an earlier input's name for a uid beats a
    // later one, and any uid nobody named falls back to its canonical form inside
    // `write_surface`. Colliding labels are already reported as a `label-collision`
    // contention above, so this does not hide anything the merge did not already say.
    let surface = wants_surface(global, false);

    let bytes = if surface {
        let mut folded: std::collections::BTreeMap<smysl::Label, Uid> =
            std::collections::BTreeMap::new();
        for l in &labels {
            for (label, uid) in l {
                folded.entry(label.clone()).or_insert(*uid);
            }
        }
        let ctx = WriteContext::from_labels(&folded);
        let records: Vec<Record> = store.iter().cloned().collect();

        // Only units, relations and threads have a surface spelling; attestations,
        // contentions, pack info and schema declarations travel as CBOR only. For most
        // commands that gap is academic. For `merge` it is not: a contention is the whole
        // point of the command, and a surface rendering that dropped one silently would
        // hand back a document that reads unanimous over a store that is not - rule V2's
        // failure mode, committed by the tool that exists to prevent it.
        //
        // So the text form stays available and stops being silent about what it is: a
        // readable view of the store, not the store.
        // The `@doc` header is a View, and `write_surface` emits one only when it is handed
        // in as the `view` argument - a View sitting in `records` is skipped. Passing `None`
        // therefore dropped the header and then counted it as "no surface form", which was
        // wrong twice: it is expressible, and the warning blamed contentions for it.
        let view = store.views().next().cloned();
        let emitted = view.as_ref().map(|v| v.id.clone());

        // Only the one view handed to `write_surface` becomes a `@doc` header; the surface
        // grammar has room for at most one per file, so any further view in the store is
        // genuinely dropped and worth counting.
        //
        // A label binding is the same mistake the `@doc` header used to be, and it survived
        // the fix above: `folded` put it in `ctx` a few lines up and `write_surface` renders
        // it as the name on its unit, so `@claim c/a` reported "1 record(s) have no surface
        // form" over a document whose output said `@claim c/a`. Asking `ctx` which label it
        // will actually write is what makes this right rather than restating the rule —
        // `from_labels` keeps one label per uid, and the loser genuinely is dropped.
        warn_surface_omissions("merge", &records, emitted.as_ref(), &ctx);
        write_surface(view.as_ref(), &records, &ctx).into_bytes()
    } else {
        store.log_bytes()
    };

    match out {
        Some(p) => {
            if let Err(e) = std::fs::write(p, &bytes) {
                eprintln!("smysl merge: {p}: {e}");
                return ExitCode::Failure;
            }
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            if stdout.write_all(&bytes).is_err() {
                return ExitCode::Failure;
            }
        }
    }
    if strict_failed {
        return ExitCode::CheckErrors;
    }
    ExitCode::Success
}

/// Units that rest on `target` through `grounds` or `deps` and survive its retraction.
///
/// The impact a retraction does not show: they keep other support, so they are not unfounded,
/// but part of what they rest on is gone. Measured over the same edges retraction follows.
fn partly_resting(store: &(impl StoreRead + ?Sized), target: Uid, unfounded: &[Uid]) -> Vec<Uid> {
    let mut v: Vec<Uid> = smysl::dependents(store, target)
        .into_iter()
        .filter(|u| !unfounded.contains(u))
        .collect();
    v.sort();
    v
}

/// `smysl retract` - withdraw belief in a unit, blast radius first (§23.1).
///
/// `--dry-run` reports exactly what applying it would reach. Nobody should discover what a
/// retraction touches by performing it.
fn cmd_retract(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = match m
        .get_one::<String>("store")
        .or_else(|| global.get_one::<String>("store"))
    {
        Some(p) => p.clone(),
        None => {
            eprintln!("smysl retract: no store given");
            return ExitCode::Usage;
        }
    };
    warn_output_is_a_report(global, "retract");
    let (mut store, labels) = match load_memory(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl retract: {e}");
            return ExitCode::Failure;
        }
    };

    let raw = m.get_one::<String>("uid").expect("required");
    let target = match resolve(&store, raw) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("smysl retract: {e}");
            return e.code;
        }
    };

    let agents: Vec<AgentId> = m
        .get_many::<String>("as")
        .map(|v| v.filter_map(|s| AgentId::new(s).ok()).collect())
        .unwrap_or_default();
    let authority = m
        .get_one::<String>("authority")
        .and_then(|s| RetractionAuthority::parse(s))
        .unwrap_or_default();
    let policy = RetractionPolicy::default();

    let plan = plan_retraction(&store, target, &agents, policy, authority);
    if global.get_flag("json") {
        // `authorised` and `refusal` travel too: a caller gating on a retraction needs to
        // know it was refused, and refusal is reported on stderr in the text form where a
        // machine reading stdout would never see it.
        println!(
            "{{\"target\":{},\"blast_radius\":[{}],\"orphaned\":[{}],\"rest_partly_on\":[{}],\"authorised\":{},\"refusal\":{}}}",
            smysl::json_escape(&target.canonical()),
            uid_array(&plan.blast_radius),
            uid_array(&plan.orphaned),
            uid_array(&partly_resting(&store, target, &plan.blast_radius)),
            plan.authorised,
            plan.refusal
                .as_deref()
                .map(smysl::json_escape)
                .unwrap_or_else(|| "null".into())
        );
        if m.get_flag("dry-run") {
            return ExitCode::Success;
        }
    } else {
        // "would reach N" counted the target itself plus the units left with no support, so
        // retracting one of a decision's four prerequisites said "reach 1" — true of the
        // retraction, and read as "nothing depends on this". The first line now says what the
        // number is; the second says what the number leaves out.
        println!(
            "{path}: retracting {target} would leave {} unit(s) unfounded, {} of them orphaned",
            plan.blast_radius.len(),
            plan.orphaned.len()
        );
        for u in &plan.orphaned {
            println!("{path}:   {u} would lose all of its grounds");
        }
        let partly = partly_resting(&store, target, &plan.blast_radius);
        if !partly.is_empty() {
            println!(
                "{path}:   {} more unit(s) rest partly on it and keep other support",
                partly.len()
            );
        }
    }

    if m.get_flag("dry-run") {
        return ExitCode::Success;
    }
    // Now that it writes, a second run must not write twice: a CBOR log would deduplicate the
    // record, and a surface file would gain the same `@rel` line again.
    if effective_status(&store, policy).is_retracted(&target) {
        println!("{path}: {target} is already retracted; nothing to do");
        return ExitCode::Success;
    }
    if !plan.authorised {
        eprintln!(
            "smysl retract: {}",
            plan.refusal.unwrap_or_else(|| "refused".into())
        );
        return ExitCode::Failure;
    }

    // Written to the store it was read from. Until 1.4 this appended to the in-memory copy
    // `load_store` builds, printed "now read as unfounded", exited 0 and wrote nothing, so a
    // second run reported the same retraction as new.
    let retraction = Record::Relation(Relation::new(RelKind::Retracts, target, target));
    if let Err(e) = persist(&path, std::slice::from_ref(&retraction), &store, &labels) {
        eprintln!("smysl retract: {e}");
        return ExitCode::Failure;
    }
    if let Err(e) = store.append(&[retraction]) {
        eprintln!("smysl retract: {e}");
        return ExitCode::Failure;
    }
    let eff = effective_status(&store, policy);
    println!(
        "{path}: {} unit(s) now read as unfounded",
        eff.blast_radius().len()
    );
    ExitCode::Success
}

/// Write records to the store a command read them against.
///
/// A CBOR store is appended to, as the log it is. A surface store gets each record's surface
/// spelling appended to the file — written by the library's writer, with the store's labels and
/// edges, so it reads as the rest of the file does — and what was there, comments included, is
/// untouched. A record surface text cannot hold is refused rather than dropped: it would read back
/// as never written.
fn persist(
    path: &str,
    records: &[Record],
    store: &(impl StoreRead + ?Sized),
    labels: &std::collections::BTreeMap<smysl::Label, Uid>,
) -> Result<(), String> {
    if path == "-" {
        return Err("a store read from stdin cannot be written back; name the file".into());
    }
    if SHARDS.get().is_some() {
        return Err(format!(
            "{path}: --shard reads some of a library's shards; a write goes to the library whole"
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
    if looks_like_surface(&bytes) {
        for r in records {
            let expressible = match r {
                Record::Relation(_) => true,
                Record::Withdrawal(w) => smysl::surface::withdrawal_has_surface_form(w),
                Record::Resolution(x) => smysl::surface::resolution_has_surface_form(x),
                // A commitment always has one: it names a unit, and a uid can always be written.
                Record::Commit(_) => true,
                // A manifest usually has one (1.10); the predicate names the cases it does
                // not, among them an alias the parser would not read back.
                Record::Manifest(m) => smysl::surface::manifest_has_surface_form(m),
                // A dating usually has one. `@date` has been in the grammar since TX-P3 step 1
                // and this arm was missing, so `date set` on a surface document was refused by
                // the one writer every record-writing command goes through — the predicate
                // existed and nothing consulted it. The cases it still rules out are a
                // dating whose `ts` names a different agent than its author, one carrying
                // unknown keys, and one whose relative target is a window: the parser cannot
                // read any of the three back.
                Record::Dating(d) => smysl::surface::dating_has_surface_form(d),
                _ => false,
            };
            if !expressible {
                return Err(format!(
                    "{path}: this {} has no surface form, so a surface store cannot hold it; \
                     convert it first (`smysl merge {path} -o STORE.cbor`) and use that",
                    r.type_name()
                ));
            }
        }
        let ctx = WriteContext::from_labels(labels).with_relations(store.relations());
        let text = write_surface(None, records, &ctx);
        let separator = if bytes.ends_with(b"\n") { "\n" } else { "\n\n" };
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .map_err(|e| format!("{path}: {e}"))?;
        f.write_all(format!("{separator}{}", text.trim_end()).as_bytes())
            .and_then(|_| f.write_all(b"\n"))
            .map_err(|e| format!("{path}: {e}"))
    } else {
        // Through the unit store's writer under `--backend redb`, so the store describes the
        // log it was written to and a second writer meanwhile is refused (`SMY-E445`).
        #[cfg(feature = "store-redb")]
        if let Some(opts) = redb_options() {
            let (mut shard, _) =
                smysl::redb::RedbShard::open_rw(path, opts).map_err(|e| format!("{path}: {e}"))?;
            shard.append(records).map_err(|e| format!("{path}: {e}"))?;
            return Ok(());
        }
        let mut log = Store::open(path).map_err(|e| format!("{path}: {e}"))?;
        log.append(records).map_err(|e| format!("{path}: {e}"))?;
        Ok(())
    }
}

/// Commit a staged batch into the store at `path`, and discard the stage (H-13).
///
/// `--yes` promised a commit from the day it was written and performed none: it suppressed
/// exit 10 and stopped. 1.9 said so in the help and at runtime; this is the commit.
///
/// **Order matters.** The stage is discarded only after the write has succeeded. A batch that
/// failed to land is still staged, which is the whole point of staging — the alternative loses
/// work to a full disk.
///
/// **A surface store refuses rather than silently dropping provenance.** A staged batch carries
/// attestations, and attestations have no surface spelling. Appending the batch to a `.smy`
/// store would write the units and lose the recipe that says how they were made, which is the
/// one thing rule D exists to keep. `persist` already refuses records it cannot spell, but its
/// message names the record type; this one names the reason and the way out.
#[cfg(feature = "ingest")]
fn commit_staged(
    cmd: &str,
    path: &str,
    root: &std::path::Path,
    records: &[Record],
    store: &(impl StoreRead + ?Sized),
    labels: &std::collections::BTreeMap<smysl::Label, Uid>,
) -> Result<(), ExitCode> {
    if let Ok(bytes) = std::fs::read(path) {
        if looks_like_surface(&bytes) {
            let unspellable = records
                .iter()
                .filter(|r| matches!(r, Record::Attestation(_)))
                .count();
            if unspellable > 0 {
                eprintln!(
                    "smysl {cmd}: {path} is a surface store, and {unspellable} attestation(s) \
                     in this batch have no surface form - committing would keep the units and \
                     lose the recipe that made them"
                );
                eprintln!(
                    "  convert the store first (`smysl merge {path} -o STORE.cbor`) and commit \
                     to that, or review and `smysl merge --staged`"
                );
                return Err(ExitCode::Usage);
            }
        }
    }
    if let Err(e) = persist(path, records, store, labels) {
        eprintln!("smysl {cmd}: {e}");
        return Err(ExitCode::Failure);
    }
    // Only now: the records are in the store, so the staged copy is no longer the only one.
    if let Err(e) = smysl::stage::discard(root) {
        eprintln!("smysl {cmd}: committed, but the staged batch could not be removed: {e}");
        return Err(ExitCode::Failure);
    }
    Ok(())
}

/// The store path a command names positionally or with `--store`.
fn store_path(m: &ArgMatches, global: &ArgMatches, cmd: &str) -> Result<String, ExitCode> {
    match m
        .get_one::<String>("store")
        .or_else(|| global.get_one::<String>("store"))
    {
        Some(p) => Ok(p.clone()),
        None => {
            eprintln!("smysl {cmd}: no store given");
            Err(ExitCode::Usage)
        }
    }
}

/// An edge argument: a rid or its display prefix, or `FROM --KIND--> TO` with each end a uid
/// or a label.
fn resolve_edge(store: &(impl StoreRead + ?Sized), raw: &str) -> Result<Relation, Unresolved> {
    let fail = |message: String| Unresolved {
        code: ExitCode::Failure,
        message,
    };
    if let Some(arrow) = raw.find("-->") {
        let left = &raw[..arrow];
        let Some(open) = left.rfind("--") else {
            return Err(Unresolved {
                code: ExitCode::Usage,
                message: format!("`{raw}` is not `FROM --KIND--> TO`"),
            });
        };
        let from = resolve(store, left[..open].trim())?;
        let to = resolve(store, raw[arrow + 3..].trim())?;
        let kind = RelKind::parse(left[open + 2..].trim()).map_err(|e| Unresolved {
            code: ExitCode::Usage,
            message: format!("`{}` is not a relation kind: {e}", left[open + 2..].trim()),
        })?;
        return store
            .relations()
            .find(|r| r.kind == kind && r.from == from && r.to == to)
            .cloned()
            .ok_or_else(|| {
                fail(format!(
                    "no `{kind}` edge from {from} to {to} in this store"
                ))
            });
    }
    let prefix = UidPrefix::parse(raw).map_err(|_| Unresolved {
        code: ExitCode::Usage,
        message: format!("`{raw}` is neither a rid nor `FROM --KIND--> TO`"),
    })?;
    let found: Vec<&Relation> = store
        .relations()
        .filter(|r| prefix.matches(&r.uid()))
        .collect();
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(fail(format!("no edge in this store has the rid `{raw}`"))),
        many => Err(Unresolved {
            code: ExitCode::Usage,
            message: format!(
                "`{raw}` matches {} edges; give more of the rid:{}",
                many.len(),
                many.iter()
                    .map(|r| format!("\n  {}  {} --{}--> {}", r.uid(), r.from, r.kind, r.to))
                    .collect::<String>()
            ),
        }),
    }
}

/// `--at`, or the wall clock. A withdrawal and a resolution record when they happened, which is
/// what `--at` exists to make reproducible.
fn record_time(m: &ArgMatches) -> u64 {
    m.get_one::<u64>("at").copied().unwrap_or_else(now_millis)
}

/// How an edge reads in a report: `FROM --KIND--> TO`, with labels where the store has them.
fn edge_text(labels: &std::collections::BTreeMap<smysl::Label, Uid>, r: &Relation) -> String {
    let name = |u: &Uid| {
        labels
            .iter()
            .find(|(_, v)| *v == u)
            .map(|(l, _)| l.to_string())
            .unwrap_or_else(|| u.to_string())
    };
    format!("{} --{}--> {}", name(&r.from), r.kind, name(&r.to))
}

/// Items open for review in `store`, under the labels the store binds.
fn review_items(
    store: &(impl StoreRead + ?Sized),
    labels: &std::collections::BTreeMap<smysl::Label, Uid>,
) -> Vec<smysl::ReviewItem> {
    review_items_with(store, labels, &[], None)
}

/// The queue, with the edge kinds a caller expects somebody to stand behind (1.5).
fn review_items_with(
    store: &(impl StoreRead + ?Sized),
    labels: &std::collections::BTreeMap<smysl::Label, Uid>,
    confirm: &[RelKind],
    confirmed_by: Option<smysl::AgentKind>,
) -> Vec<smysl::ReviewItem> {
    let mut ctx = smysl::DetectionContext::default();
    ctx.labels = vec![labels.clone()];
    let mut opts = smysl::ReviewOptions::confirming(confirm.iter().cloned()).with_detection(ctx);
    if let Some(kind) = confirmed_by {
        opts = opts.confirmed_by(kind);
    }
    smysl::review_with(store, &opts)
}

/// `smysl withdraw` - an edge that should no longer be followed (1.4).
///
/// The edge's record stays; closure, lineage, detection and packing stop following it. What it
/// releases is reported first, as `retract` reports its blast radius.
/// `smysl commit` - record how settled a unit is (1.7).
///
/// A second axis beside `status`: status is how true, commitment is how settled. Recording one
/// does not move the unit's uid — the content did not change, the author's commitment to it did,
/// which is exactly the event a development history wants to keep.
fn cmd_commit(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = match store_path(m, global, "commit") {
        Ok(p) => p,
        Err(code) => return code,
    };
    warn_output_is_a_report(global, "commit");
    let (store, labels) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl commit: {e}");
            return ExitCode::Failure;
        }
    };
    let raw = m.get_one::<String>("unit").expect("required");
    // A commitment may name a **dating** as well as a unit: A-6 gives key 0 either, and a
    // `canonical` commitment on a dating is how rule E's liveness lock is written (A-12.2).
    // The did's bytes are the key, which is what `held_by_lock` reads, so the spelling is all
    // that was missing — and without it the lock had no way to be written at all, since
    // `@commit` in a document takes a uid or a label (OQ-73).
    let unit = if raw.starts_with(smysl::Did::PREFIX) {
        match smysl::Did::parse(raw) {
            Ok(d) => Uid::from_bytes(*d.as_bytes()),
            Err(e) => {
                eprintln!("smysl commit: `{raw}` is not a did: {e}");
                return ExitCode::Usage;
            }
        }
    } else {
        match resolve(&store, raw) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("smysl commit: {e}");
                return e.code;
            }
        }
    };
    let level = smysl::Commitment::parse(m.get_one::<String>("level").expect("required"))
        .expect("clap restricts the values");
    let agent = match AgentId::new(m.get_one::<String>("as").expect("required")) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl commit: --as: {e}");
            return ExitCode::Usage;
        }
    };
    let note = match m.get_one::<String>("note").map(|r| resolve(&store, r)) {
        None => None,
        Some(Ok(u)) => Some(u),
        Some(Err(e)) => {
            eprintln!("smysl commit: --note: {e}");
            return e.code;
        }
    };

    let at = record_time(m);
    let mut commit = smysl::Commit::new(unit, level, agent.clone(), Hlc::new(at, 0, agent));
    if let Some(u) = note {
        commit = commit.with_note(u);
    }
    let records = vec![Record::Commit(commit)];

    let before = store.commitment_of(&unit);
    let dry = m.get_flag("dry-run");
    // Recording the level it already has is a no-op worth saying out loud rather than a second
    // identical record: the log would carry both and answer the same either way.
    let unchanged = before == Some(level);
    let apply = !dry && !unchanged;
    if apply {
        if let Err(e) = persist(&path, &records, &store, &labels) {
            eprintln!("smysl commit: {e}");
            return ExitCode::Failure;
        }
    }

    if global.get_flag("json") {
        println!(
            "{{\"unit\":{},\"level\":{},\"was\":{},\"applied\":{}}}",
            smysl::json_escape(&unit.canonical()),
            smysl::json_escape(level.as_str()),
            before
                .map(|b| smysl::json_escape(b.as_str()))
                .unwrap_or_else(|| "null".into()),
            apply
        );
    } else {
        let name = labels
            .iter()
            .find(|(_, u)| **u == unit)
            .map(|(l, _)| l.as_str().to_string())
            .unwrap_or_else(|| unit.short());
        match before {
            Some(b) if b == level => println!("{path}: {name} is already {level}; nothing to do"),
            Some(b) => println!("{path}: {name}  {b} -> {level}"),
            None => println!("{path}: {name}  -> {level}"),
        }
        if dry && !unchanged {
            println!("{path}:   --dry-run, so nothing was written");
        }
    }
    ExitCode::Success
}

fn cmd_withdraw(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = match store_path(m, global, "withdraw") {
        Ok(p) => p,
        Err(code) => return code,
    };
    warn_output_is_a_report(global, "withdraw");
    let (store, labels) = match load_memory(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl withdraw: {e}");
            return ExitCode::Failure;
        }
    };
    let edge = match resolve_edge(&store, m.get_one::<String>("edge").expect("required")) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl withdraw: {e}");
            return e.code;
        }
    };
    if edge.kind.is_lifecycle() {
        eprintln!(
            "smysl withdraw: a `{}` edge cannot be withdrawn (SMY-W056); a withdrawal of it would \
             be kept and ignored",
            edge.kind
        );
        return ExitCode::Usage;
    }

    let mut agents: Vec<AgentId> = Vec::new();
    for raw in m.get_many::<String>("as").into_iter().flatten() {
        match AgentId::new(raw) {
            Ok(a) if !agents.contains(&a) => agents.push(a),
            Ok(_) => {}
            Err(e) => {
                eprintln!("smysl withdraw: `{raw}` is not an agent id: {e}");
                return ExitCode::Usage;
            }
        }
    }
    let authority = match m.get_one::<String>("authority") {
        Some(s) => match RetractionAuthority::parse(s) {
            Some(a) => a,
            None => {
                eprintln!("smysl withdraw: `{s}` is not an authority: origin, any or quorum:N");
                return ExitCode::Usage;
            }
        },
        None => RetractionAuthority::default(),
    };
    let reason = match m.get_one::<String>("reason").map(|r| resolve(&store, r)) {
        None => None,
        Some(Ok(u)) => Some(u),
        Some(Err(e)) => {
            eprintln!("smysl withdraw: --reason: {e}");
            return e.code;
        }
    };

    // The same authority rule as retraction, read off the edge: `origin` means an agent that
    // attested this edge.
    let refusal = if (agents.len() as u32) < authority.required_agents().max(1) {
        Some(format!(
            "{authority} requires {} distinct agent(s), got {}; name them with --as",
            authority.required_agents().max(1),
            agents.len()
        ))
    } else if authority.requires_origin()
        && !agents
            .iter()
            .any(|a| edge.attestations.iter().any(|x| &x.agent == a))
    {
        Some(format!(
            "origin authority: none of the {} requesting agent(s) attested this edge",
            agents.len()
        ))
    } else {
        None
    };

    let rid = edge.uid();
    let at = record_time(m);
    let records: Vec<Record> = agents
        .iter()
        .map(|a| {
            let mut w = smysl::Withdrawal::new(rid, a.clone(), Hlc::new(at, 0, a.clone()));
            if let Some(u) = reason {
                w = w.with_reason(u);
            }
            Record::Withdrawal(w)
        })
        .collect();
    let already = store.is_withdrawn(&edge);

    // What it releases: review items that are open now and would not be after.
    let open_before: Vec<smysl::ResolutionTarget> = review_items(&store, &labels)
        .into_iter()
        .filter(|i| !i.resolved)
        .map(|i| i.target())
        .collect();
    let mut after = store.clone();
    let _ = after.append(&records);
    let open_after: Vec<smysl::ResolutionTarget> = review_items(&after, &labels)
        .into_iter()
        .filter(|i| !i.resolved)
        .map(|i| i.target())
        .collect();
    let released = open_before
        .iter()
        .filter(|t| !open_after.contains(t))
        .count();
    let unpinned = if edge.kind == RelKind::Rebuts && store.is_live_rebuttal(&edge) {
        Some(edge.to)
    } else {
        None
    };
    let dry = m.get_flag("dry-run");
    let apply = !dry && refusal.is_none() && !already;

    if apply {
        if let Err(e) = persist(&path, &records, &store, &labels) {
            eprintln!("smysl withdraw: {e}");
            return ExitCode::Failure;
        }
    }

    if global.get_flag("json") {
        println!(
            "{{\"rid\":{},\"kind\":{},\"from\":{},\"to\":{},\"already_withdrawn\":{},\"released_from_review\":{},\"unpins\":{},\"authorised\":{},\"refusal\":{},\"applied\":{}}}",
            smysl::json_escape(&rid.canonical()),
            smysl::json_escape(edge.kind.as_str()),
            smysl::json_escape(&edge.from.canonical()),
            smysl::json_escape(&edge.to.canonical()),
            already,
            released,
            unpinned
                .map(|u| smysl::json_escape(&u.canonical()))
                .unwrap_or_else(|| "null".into()),
            refusal.is_none(),
            refusal
                .as_deref()
                .map(smysl::json_escape)
                .unwrap_or_else(|| "null".into()),
            apply
        );
    } else {
        println!("{path}: {rid}  {}", edge_text(&labels, &edge));
        if already {
            println!("{path}:   already withdrawn; nothing to do");
            return ExitCode::Success;
        }
        if let Some(claim) = unpinned {
            println!("{path}:   {claim} would no longer carry this rebuttal into a pack");
        }
        println!("{path}:   {released} item(s) would leave the review queue");
        if let (true, Some(r)) = (dry, &refusal) {
            println!("{path}:   would be refused: {r}");
        }
        if apply {
            println!(
                "{path}: withdrawn by {}",
                agents
                    .iter()
                    .map(|a| a.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
    }
    if let Some(r) = refusal {
        if !dry {
            eprintln!("smysl withdraw: {r}");
            return ExitCode::Failure;
        }
    }
    ExitCode::Success
}

/// The contentions **rule E derives** over this store, searched by id.
///
/// Three sources, all of them kind 4 or kind 5 and none of them in the log: a dating a lock is
/// holding, a live dating that moved no bound, and a set of records whose times cannot all be
/// right. Searched over all three axes, because a dating bears on one axis and the caller named
/// an id rather than an axis.
///
/// Returns the id itself and how it reads in a report, so the caller records the same bytes it
/// matched rather than re-deriving them.
#[cfg(feature = "text")]
fn derived_time_contention(
    store: &(impl StoreRead + ?Sized),
    raw: &str,
) -> Option<(smysl::ContentionId, String)> {
    use smysl::text::time::engine;
    for axis in [smysl::Axis::Said, smysl::Axis::Composed, smysl::Axis::About] {
        let e = engine::effective(store, axis);
        for did in &e.held {
            let Some(id) = store
                .library()
                .dating(did)
                .and_then(|d| engine::hold_contention(did, d))
            else {
                continue;
            };
            if id.as_str() == raw {
                let what = format!("{id}  dating not applied over {did} (held, {axis})");
                return Some((id, what));
            }
        }
        for c in e.not_applied.iter().chain(&e.inconsistent) {
            if c.id.as_str() == raw {
                let what = format!("{}  {} over {} ({axis})", c.id, c.kind, c.over);
                return Some((c.id.clone(), what));
            }
        }
    }
    None
}

/// Without the time engine there are no derived contentions to find.
#[cfg(not(feature = "text"))]
fn derived_time_contention(
    _store: &(impl StoreRead + ?Sized),
    _raw: &str,
) -> Option<(smysl::ContentionId, String)> {
    None
}

/// `smysl resolve` - record that a disagreement was reviewed (1.4).
///
/// Decides nothing: the reviewer's conclusion is a retraction or a withdrawal, recorded by
/// those commands. This takes the item off the review queue and, for a contention, stops it
/// pinning its positions into a pack.
fn cmd_resolve(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = match store_path(m, global, "resolve") {
        Ok(p) => p,
        Err(code) => return code,
    };
    warn_output_is_a_report(global, "resolve");
    let (store, labels) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl resolve: {e}");
            return ExitCode::Failure;
        }
    };
    let Some(raw_agent) = m.get_one::<String>("as") else {
        eprintln!("smysl resolve: name the reviewer with --as");
        return ExitCode::Usage;
    };
    let agent = match AgentId::new(raw_agent) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl resolve: `{raw_agent}` is not an agent id: {e}");
            return ExitCode::Usage;
        }
    };
    let note = match m.get_one::<String>("note").map(|r| resolve(&store, r)) {
        None => None,
        Some(Ok(u)) => Some(u),
        Some(Err(e)) => {
            eprintln!("smysl resolve: --note: {e}");
            return e.code;
        }
    };

    let raw = m.get_one::<String>("item").expect("required");
    let items = review_items(&store, &labels);
    let (target, what) = if raw.starts_with("k/") {
        match items.iter().find(|i| match &i.subject {
            smysl::ReviewSubject::Contention(c) => c.id.as_str() == raw,
            _ => false,
        }) {
            Some(i) => match &i.subject {
                smysl::ReviewSubject::Contention(c) => (
                    i.target(),
                    format!("{}  {} over {}", c.id, c.detected.kind, c.over),
                ),
                _ => unreachable!("matched a contention"),
            },
            None => match derived_time_contention(&store, raw) {
                // Rule E's contentions are **derived and never written** (A-8.2), so no amount
                // of `review` will list one and the recorded lookup above cannot find it. A
                // resolution naming one is nevertheless the documented way to release a held
                // dating (A-12.2), which made that release unreachable through this command
                // until the fallback existed. It is a second lookup and not a loosened guard:
                // the id has to be one rule E derives over *this* store right now.
                Some((id, what)) => (smysl::ResolutionTarget::Contention(id), what),
                None => {
                    eprintln!(
                        "smysl resolve: no contention `{raw}` is open or resolved in this store, \
                         and rule E derives none with that id; `smysl review --all` lists the \
                         recorded ones and `smysl check --pass time` reports the derived ones"
                    );
                    return ExitCode::Failure;
                }
            },
        }
    } else {
        let edge = match resolve_edge(&store, raw) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("smysl resolve: {e}");
                return e.code;
            }
        };
        if edge.kind != RelKind::Rebuts {
            eprintln!(
                "smysl resolve: only a contention or a `rebuts` edge is reviewed; this is a `{}` edge",
                edge.kind
            );
            return ExitCode::Usage;
        }
        // A rebuttal a thread presents is reviewed as its contention. Resolving the edge instead
        // would record a review and leave the contention open and pinning.
        if let Some(k) = items.iter().find_map(|i| match &i.subject {
            smysl::ReviewSubject::Contention(c)
                if !i.resolved
                    && c.detected.kind == smysl::DetectionKind::LiveRebuttal
                    && c.over == edge.to
                    && c.positions.contains(&edge.from) =>
            {
                Some(c.id.clone())
            }
            _ => None,
        }) {
            eprintln!(
                "smysl resolve: this rebuttal is under review as contention `{k}`; resolve that"
            );
            return ExitCode::Usage;
        }
        (
            smysl::ResolutionTarget::Relation(edge.uid()),
            format!("{}  {}", edge.uid(), edge_text(&labels, &edge)),
        )
    };

    let mine = store
        .resolutions()
        .any(|r| r.target == target && r.agent == agent);
    let at = record_time(m);
    let mut resolution = smysl::Resolution::new(
        target.clone(),
        agent.clone(),
        Hlc::new(at, 0, agent.clone()),
    );
    if let Some(u) = note {
        resolution = resolution.with_note(u);
    }
    let apply = !m.get_flag("dry-run") && !mine;
    if apply {
        if let Err(e) = persist(&path, &[Record::Resolution(resolution)], &store, &labels) {
            eprintln!("smysl resolve: {e}");
            return ExitCode::Failure;
        }
    }

    if global.get_flag("json") {
        let (kind, id) = match &target {
            smysl::ResolutionTarget::Contention(c) => ("contention", c.as_str().to_string()),
            smysl::ResolutionTarget::Relation(r) => ("rebuttal", r.canonical()),
            _ => ("unknown", String::new()),
        };
        println!(
            "{{\"target\":{},\"kind\":\"{kind}\",\"agent\":{},\"already_resolved_by_agent\":{mine},\"applied\":{apply}}}",
            smysl::json_escape(&id),
            smysl::json_escape(agent.as_str()),
        );
    } else {
        println!("{path}: {what}");
        if mine {
            println!("{path}:   {agent} already resolved this; nothing to do");
        } else if apply {
            println!("{path}: resolved by {agent}");
        } else {
            println!("{path}:   would be resolved by {agent}");
        }
    }
    ExitCode::Success
}

/// `smysl review` - what is open for review (1.4).
///
/// Exits 5 while anything is open, so a pipeline can gate on an empty queue.
fn cmd_review(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = match store_path(m, global, "review") {
        Ok(p) => p,
        Err(code) => return code,
    };
    warn_output_is_a_report(global, "review");
    let (store, labels) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl review: {e}");
            return ExitCode::Failure;
        }
    };
    let mut confirm: Vec<RelKind> = Vec::new();
    for word in m
        .get_one::<String>("confirm")
        .map(|s| s.split(',').map(str::trim).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter(|w| !w.is_empty())
    {
        match RelKind::parse(word) {
            Ok(k) => confirm.push(k),
            Err(e) => {
                eprintln!("smysl review: `{word}` is not a relation kind: {e}");
                return ExitCode::Usage;
            }
        }
    }
    let confirmed_by = m
        .get_one::<String>("confirmed-by")
        .map(|s| match s.as_str() {
            "model" => smysl::AgentKind::Model,
            "tool" => smysl::AgentKind::Tool,
            _ => smysl::AgentKind::Human,
        });

    let all = m.get_flag("all");
    let items: Vec<smysl::ReviewItem> = review_items_with(&store, &labels, &confirm, confirmed_by)
        .into_iter()
        .filter(|i| all || !i.resolved)
        .collect();
    let open = items.iter().filter(|i| !i.resolved).count();
    let gist = |u: &Uid| {
        store
            .get(u)
            .map(|x| x.core.gist.clone())
            .unwrap_or_else(|| "(not in this store)".into())
    };

    if global.get_flag("json") {
        let rows: Vec<String> = items
            .iter()
            .map(|i| match &i.subject {
                smysl::ReviewSubject::Contention(c) => format!(
                    "{{\"type\":\"contention\",\"id\":{},\"kind\":\"{}\",\"over\":{},\"positions\":[{}],\"resolved\":{}}}",
                    smysl::json_escape(c.id.as_str()),
                    c.detected.kind,
                    smysl::json_escape(&c.over.canonical()),
                    uid_array(&c.positions),
                    i.resolved
                ),
                smysl::ReviewSubject::Rebuttal(r) => format!(
                    "{{\"type\":\"rebuttal\",\"rid\":{},\"from\":{},\"to\":{},\"resolved\":{}}}",
                    smysl::json_escape(&r.uid().canonical()),
                    smysl::json_escape(&r.from.canonical()),
                    smysl::json_escape(&r.to.canonical()),
                    i.resolved
                ),
                smysl::ReviewSubject::Unconfirmed(r) => format!(
                    "{{\"type\":\"edge\",\"kind\":{},\"rid\":{},\"from\":{},\"to\":{},\"confirmed\":{}}}",
                    smysl::json_escape(r.kind.as_str()),
                    smysl::json_escape(&r.uid().canonical()),
                    smysl::json_escape(&r.from.canonical()),
                    smysl::json_escape(&r.to.canonical()),
                    i.resolved
                ),
                _ => "{\"type\":\"unknown\"}".into(),
            })
            .collect();
        println!("{{\"open\":{open},\"items\":[{}]}}", rows.join(","));
    } else {
        println!("{path}: {open} item(s) open for review");
        for i in &items {
            let mark = if i.resolved { "  (resolved)" } else { "" };
            match &i.subject {
                smysl::ReviewSubject::Contention(c) => {
                    println!("{}  {}{mark}", c.id, c.detected.kind);
                    for p in &c.positions {
                        println!("  {p}  {}", gist(p));
                    }
                }
                smysl::ReviewSubject::Rebuttal(r) => {
                    println!("{}  {}{mark}", r.uid(), edge_text(&labels, r));
                    println!("  {}  {}", r.to, gist(&r.to));
                    println!("  rebutted by {}  {}", r.from, gist(&r.from));
                }
                smysl::ReviewSubject::Unconfirmed(r) => {
                    let mark = if i.resolved { "  (confirmed)" } else { "" };
                    println!("{}  {}{mark}", r.uid(), edge_text(&labels, r));
                    println!("  awaiting confirmation: {}  {}", r.to, gist(&r.to));
                }
                _ => {}
            }
        }
    }
    if open > 0 {
        ExitCode::Contentions
    } else {
        ExitCode::Success
    }
}

/// `smysl pack` - budget-bounded, closure-complete selection (§23.1).
///
/// What a consuming agent calls instead of asking a model to summarise. Surface output is
/// truncated to the selected level - that is the thing you put in a prompt. CBOR output
/// carries the full records, because a CBOR pack is a portable sub-store and truncating a
/// unit would change its uid.
fn cmd_pack(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let Some(path) = store_arg(m, global) else {
        eprintln!("smysl pack: no store given");
        return ExitCode::Usage;
    };
    let (store, labels) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl pack: {e}");
            return ExitCode::Failure;
        }
    };

    let Some(budget) = m.get_one::<String>("budget").and_then(|s| parse_budget(s)) else {
        eprintln!("smysl pack: --budget takes a number, optionally with a `k` suffix");
        return ExitCode::Usage;
    };

    let mut req = PackRequest::budget(budget);
    if let Some(raw) = m.get_one::<String>("reserve") {
        let Some(n) = parse_budget(raw) else {
            eprintln!("smysl pack: --reserve takes a number, optionally with a `k` suffix");
            return ExitCode::Usage;
        };
        if n >= budget {
            eprintln!(
                "smysl pack: --reserve {n} leaves nothing of a --budget of {budget} to pack into"
            );
            return ExitCode::Usage;
        }
        req = req.reserving(n);
    }
    if let Some(raw) = m.get_one::<String>("support") {
        match edge_set(&store, raw, "pack") {
            Ok(edges) => req = req.resting_on(edges),
            Err(code) => return code,
        }
    }
    if let Some(v) = m.get_many::<String>("focus") {
        let mut focus = Vec::new();
        for raw in v {
            match resolve(&store, raw) {
                Ok(u) => focus.push(u),
                Err(e) => {
                    eprintln!("smysl pack: {e}");
                    return e.code;
                }
            }
        }
        req = req.focusing(focus);
    }

    // `--query` is the composition retrieval exists for. Retrieval answers *which units are
    // relevant*; packing answers *what fits, without holes* — it pulls in the grounds and
    // live rebuttals of whatever it selects, so the result is an argument rather than a list
    // of excerpts. That is the part ordinary retrieval does not do, and neither half
    // achieves it alone.
    //
    // The hits become `--focus`, so they are required to reach L1 and packing fails loudly
    // if the budget cannot hold them and their closure. Failing is the right answer: a pack
    // that silently dropped what you searched for would be worse than none.
    //
    // The default limit is 3 rather than `find`'s 10, because every focused unit drags its
    // closure in with it and ten of those exhaust an ordinary budget before anything is
    // chosen on merit.
    if let Some(prefix) = m.get_one::<String>("source") {
        let from = store.units_with_source_prefix(prefix);
        // `scope` is empty-means-everything too, so a prefix nobody matches must refuse rather
        // than quietly pack the whole store.
        if from.is_empty() {
            eprintln!("smysl pack: no unit has a source starting with `{prefix}`");
            return ExitCode::Failure;
        }
        if !global.get_flag("quiet") {
            eprintln!(
                "smysl pack: --source {prefix} scoped {} unit(s)",
                from.len()
            );
        }
        req = req.scoped(from);
    }
    if m.get_one::<String>("payload").is_some() && m.get_one::<String>("query").is_none() {
        // It restricts what `--query` may focus on and nothing else. Accepting it alone would
        // look like a filter on the pack, which is what `--scope` is.
        eprintln!("smysl pack: --payload restricts --query, so it needs one");
        return ExitCode::Usage;
    }
    if let Some(q) = m.get_one::<String>("query") {
        let limit = m.get_one::<usize>("query-limit").copied().unwrap_or(3);
        let mut query = smysl::Query::new(q.clone(), limit);
        if let Some(raw) = m.get_one::<String>("payload") {
            match payload_filter(raw, "pack") {
                Ok((key, values)) => query = query.with_payload(key, values),
                Err(code) => return code,
            }
        }
        let (engine, model) = match engine_for(m, "pack") {
            Ok(v) => v,
            Err(code) => return code,
        };
        let hits = match ranked(&store, &query, engine, model.as_deref(), "pack") {
            Ok(h) => h,
            Err(code) => return code,
        };
        if hits.is_empty() {
            eprintln!("smysl pack: --query matched nothing, so there is nothing to focus on");
            return ExitCode::Failure;
        }
        if !global.get_flag("quiet") {
            eprintln!(
                "smysl pack: --query focused {} unit(s): {}",
                hits.len(),
                hits.iter()
                    .map(|h| h.uid.short())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        req = req.focusing(hits.into_iter().map(|h| h.uid).collect::<Vec<_>>());
    }

    match m.get_one::<String>("lod").map(String::as_str) {
        Some("L0") => req = req.capped(Lod::L0),
        Some("L1") => req = req.capped(Lod::L1),
        Some("L2") => req = req.capped(Lod::L2),
        _ => {}
    }
    if m.get_one::<String>("mode").map(String::as_str) == Some("exact") {
        req = req.exact();
    }
    if let Some(id) = m.get_one::<String>("tokenizer") {
        match Estimator::parse(id) {
            Some(e) => req.estimator = e,
            None => {
                eprintln!("smysl pack: unknown tokenizer `{id}`");
                return ExitCode::Usage;
            }
        }
    }

    // Branch and bound over a large scope has no countable steps - a bar would have to
    // invent a denominator, and a bar that lies about how far along it is is worse than a
    // spinner that only says how long it has been going.
    let exact = req.mode == smysl::PackMode::Exact;
    let mut spin = Spinner::new(progress_style(global), "packing");

    let sal = smysl::salience(
        &store,
        &SalienceRequest::default().seeded(smysl::view_roots(&store)),
    );
    spin.set_label(if exact {
        "searching for the optimum"
    } else {
        "packing"
    });

    let outcome = smysl::pack(&store, &sal, &req);
    spin.finish("");

    let packed = match outcome {
        Ok(p) => p,
        Err(e) => {
            // Rule R: a budget too small to hold a claim and its rebuttals fails rather
            // than emitting the claim alone.
            eprintln!("smysl pack: {e}");
            return e.code_exit();
        }
    };

    for d in packed.report.iter() {
        eprintln!("{path}: {d}");
    }
    // A pack that warned — an exact search refused, a unit degraded — is a pack a gate may
    // reasonably want to stop on.
    let strict_failed = strict_threshold(global, &packed.report).is_some();

    if m.get_flag("explain") {
        for (uid, level) in &packed.selection {
            let why = packed
                .why
                .get(uid)
                .map(ToString::to_string)
                .unwrap_or_else(|| "earned on density".into());
            let c = packed.why.get(uid).map(|r| r.constraint()).unwrap_or("-");
            eprintln!("{uid} @{level}  {c}  {why}");
        }
        for (uid, reason) in &packed.info.dropped {
            eprintln!("{uid} dropped: {reason}");
        }
        eprintln!(
            "{path}: {} of {} unit(s), {} of {}{} tokens, {} mode, gap {:.3}{}",
            packed.len(),
            store.units().count(),
            packed.used(),
            packed.info.effective_budget(),
            if packed.info.reserved == 0 {
                String::new()
            } else {
                format!(
                    " ({} reserved of {})",
                    packed.info.reserved, packed.info.budget
                )
            },
            packed.info.optimality.mode,
            packed.info.optimality.gap,
            if packed.is_optimal() {
                " (proven optimal)"
            } else {
                ""
            }
        );
    }

    let surface = wants_surface(global, false);

    let bytes = if surface {
        emit_pack_surface(&store, &packed, &labels).into_bytes()
    } else {
        let mut records: Vec<Record> = packed
            .selection
            .keys()
            .filter_map(|u| store.get(u).map(|unit| Record::Unit(unit.core.clone())))
            .collect();
        // Carry the bindings for the units that survived. Without them a packed store
        // decoded to bare uids, so the surface path was readable and the CBOR path — the
        // one a pipeline actually moves — was not. Only the survivors: a binding for a
        // dropped unit would name something the pack does not contain.
        records.extend(bindings_for(&labels, |u| packed.selection.contains_key(u)));
        let mut bytes = smysl::to_cbor_seq(&records);
        bytes.extend_from_slice(&smysl::to_cbor(&Record::PackInfo(packed.info.clone())));
        bytes
    };

    let code = emit(global, "pack", &bytes);
    if strict_failed && code == ExitCode::Success {
        return ExitCode::CheckErrors;
    }
    code
}

/// The `LabelBinding` records for whichever uids a caller kept.
///
/// Every command that emits a store needs this, and each one that forgot it produced a
/// store whose references were bare uids: valid, re-checking clean, and unreadable. That
/// was the whole point of the record existing.
fn bindings_for(
    labels: &std::collections::BTreeMap<smysl::Label, Uid>,
    keep: impl Fn(&Uid) -> bool,
) -> Vec<Record> {
    labels
        .iter()
        .filter(|(_, uid)| keep(uid))
        .map(|(label, uid)| Record::LabelBinding(smysl::LabelBinding::new(label.clone(), *uid)))
        .collect()
}

/// `--strict` promotes a warning to the failure threshold, wherever a command has a report.
///
/// It was honoured by `check` and by one branch of `render`, and accepted-and-ignored by
/// `merge`, `pack`, `bundle`, `thread` and `fmt`. That is worse than an unimplemented flag:
/// this book recommends `--strict` for CI gates, so a pipeline running `merge --strict`
/// believed it would fail on a warning and would not.
///
/// Returns the exit code the caller should fold in, or `None` when there is nothing to fail
/// on — so a command reports its diagnostics exactly as before and only the *threshold*
/// moves.
fn strict_threshold(global: &ArgMatches, report: &smysl::Report) -> Option<ExitCode> {
    if !global.get_flag("strict") {
        return None;
    }
    report
        .fail_on(Severity::Warn)
        .err()
        .map(|_| ExitCode::CheckErrors)
}

/// Say so when `--output` cannot be honoured, instead of ignoring it.
///
/// These commands print a *report* — many lines, assembled as they are discovered, not one
/// artifact — so routing it through a file is shell redirection's job. What is not
/// acceptable is what they did before: accept a documented flag, write to stdout anyway,
/// and leave the caller with an empty file and no diagnostic.
///
/// `--json` is the answer for a caller who wants this machine-readable, and the message
/// says so rather than leaving them to guess.
fn warn_output_is_a_report(global: &ArgMatches, cmd: &str) {
    if global.get_one::<String>("output").is_some() {
        eprintln!(
            "smysl {cmd}: warning: --output is not honoured here — {cmd} prints a report, \
             not a store. Redirect stdout, and use --json if you are parsing it."
        );
    }
}

/// Count the records a surface rendering will leave out, and say so when there are any.
///
/// Extracted from `merge` for `bundle`, which needed the same count the moment `--format
/// surface` started working there — and which would otherwise have grown its own copy of a
/// predicate list that has already been wrong twice: once counting every commitment as omitted
/// while `write_surface` wrote it, and once blaming contentions for a dropped `@doc` header.
///
/// `emitted` is the one view that becomes the `@doc` header; the grammar has room for at most
/// one per file, so any other view in the records genuinely is dropped. A label the context did
/// not keep is dropped for the same kind of reason.
fn warn_surface_omissions(
    cmd: &str,
    records: &[Record],
    emitted: Option<&smysl::ViewId>,
    ctx: &WriteContext,
) {
    // One predicate, the writer's own: every list kept beside it has drifted (H-16).
    let dropped = records
        .iter()
        .filter(|r| !smysl::surface::has_surface_form(r, ctx, emitted))
        .count();
    if dropped > 0 {
        eprintln!(
            "smysl {cmd}: warning: {dropped} record(s) have no surface form and were omitted \
             (contentions, attestations, and any name the grammar has nowhere to put); the \
             default CBOR output preserves them"
        );
    }
}

/// Is the output surface text? `--format` when given, else the command's own default.
///
/// The three commands that read `--format` each did it inline, identically and separately,
/// which is why `thread`'s default being the other way round was invisible. The defaults still
/// differ — `merge`, `pack` and `bundle` write CBOR, `fmt` and `thread` write text — but the
/// disagreement is now one argument at one call site rather than three copies of a `map`.
///
/// Whether the value is *available* is settled in `main` before the command runs, so there is
/// no error case here.
fn wants_surface(global: &ArgMatches, default_surface: bool) -> bool {
    global
        .get_one::<String>("format")
        .map(|f| f == "surface")
        .unwrap_or(default_surface)
}

/// Write a command's output to `--output`, or to stdout when none was given.
///
/// The flag is global, so every subcommand advertises it — and six of nine ignored it,
/// writing to stdout regardless. Silently: a caller who passed `-o` got an empty file, no
/// diagnostic, and a terminal full of CBOR.
fn emit(global: &ArgMatches, cmd: &str, bytes: &[u8]) -> ExitCode {
    match global.get_one::<String>("output") {
        Some(p) => {
            if let Err(e) = std::fs::write(p, bytes) {
                eprintln!("smysl {cmd}: {p}: {e}");
                return ExitCode::Failure;
            }
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            if stdout.write_all(bytes).is_err() {
                return ExitCode::Failure;
            }
        }
    }
    ExitCode::Success
}

/// `8000` or `8k`.
fn parse_budget(s: &str) -> Option<u64> {
    match s.strip_suffix(['k', 'K']) {
        // `checked_mul`, because `v * 1000` overflowed. In a debug build that panicked; in
        // release it wrapped, so `--budget 18446744073709552k` silently became 384 tokens
        // and `--explain` then reported 384 *as the budget*. A budget that quietly becomes
        // a different budget is the failure mode this whole project argues against.
        Some(n) => n.parse::<u64>().ok().and_then(|v| v.checked_mul(1000)),
        None => s.parse().ok(),
    }
}

/// Emit a pack as surface text, truncated to each unit's selected level.
///
/// The uid is carried as the label, so identity survives truncation even though the text
/// does not - a consumer can always ask the origin store for the rest.
fn emit_pack_surface(
    store: &(impl StoreRead + ?Sized),
    packed: &smysl::Pack,
    labels: &std::collections::BTreeMap<smysl::Label, Uid>,
) -> String {
    let by_uid: std::collections::BTreeMap<Uid, &smysl::Label> =
        labels.iter().map(|(l, u)| (*u, l)).collect();

    let mut out = String::new();
    out.push_str("@doc ");
    out.push_str(smysl::FORMAT_VERSIONS_SUPPORTED[0]);
    out.push_str(" { id: v/pack, intent: pack }\n\n");

    for (uid, level) in &packed.selection {
        let Some(unit) = store.get(uid) else { continue };
        let name = by_uid
            .get(uid)
            .map(|l| l.as_str().to_string())
            .unwrap_or_else(|| uid.canonical());
        out.push_str(&format!(
            "@{} {} {{ status: {} }}\n~ {}\n",
            unit.core.schema, name, unit.core.status, unit.core.gist
        ));
        if *level >= Lod::L1 {
            if let Some(b) = &unit.core.body {
                out.push_str(&format!("\n{b}\n"));
            }
        }
        if *level >= Lod::L2 {
            if let Some(d) = &unit.core.detail {
                out.push_str(&format!("\n--\n{d}\n"));
            }
        }
        out.push('\n');
    }

    // Truncation is self-describing (§8).
    if !packed.info.is_complete() {
        out.push_str(&format!(
            "@packinfo k/info {{ status: speculative }}\n~ {} of {} tokens used; {} unit(s) dropped, {} degraded; estimator {}\n\n",
            packed.info.used,
            packed.info.budget,
            packed.info.dropped.len(),
            packed.info.degraded.len(),
            packed.info.estimator
        ));
    }
    out
}

/// `smysl salience` - what the graph says matters, and why (§23.1).
///
/// Pure: no model call, which is what makes packing precomputation rather than another
/// round of inference.
fn cmd_salience(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let Some(path) = store_arg(m, global) else {
        eprintln!("smysl salience: no store given");
        return ExitCode::Usage;
    };
    warn_output_is_a_report(global, "salience");
    let (store, _) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl salience: {e}");
            return ExitCode::Failure;
        }
    };

    let mut req = SalienceRequest::default();
    if let Some(raw) = m.get_one::<String>("weights") {
        let parts: Vec<f32> = raw
            .split(',')
            .filter_map(|p| p.trim().parse().ok())
            .collect();
        // Three or four. The fourth is recency, and leaving it off keeps the term at zero -
        // so every command line written before recency existed still means what it meant.
        if parts.len() != 3 && parts.len() != 4 {
            eprintln!("smysl salience: --weights takes three or four numbers, c,r,t[,recency]");
            return ExitCode::Usage;
        }
        // Built from the default and adjusted rather than written as a literal:
        // `SalienceWeights` is `#[non_exhaustive]` as of 0.13, so a fifth term added later
        // does not stop this compiling — and `--weights` keeps meaning what it meant.
        let mut w = SalienceWeights::default();
        w.centrality = parts[0];
        w.corroboration = parts[1];
        w.role = parts[2];
        w.recency = parts.get(3).copied().unwrap_or(0.0);
        req = req.with_weights(w);
    }
    if let Some(hop) = m.get_one::<u32>("hop") {
        req = req.at_hop(*hop);
    }
    match m.get_many::<String>("seed") {
        Some(v) => {
            let mut seed = Vec::new();
            for raw in v {
                match resolve(&store, raw) {
                    Ok(u) => seed.push(u),
                    Err(e) => {
                        eprintln!("smysl salience: {e}");
                        return e.code;
                    }
                }
            }
            req = req.seeded(seed);
        }
        None => req = req.seeded(smysl::view_roots(&store)),
    }

    let report = smysl::salience(&store, &req);

    if let Some(raw) = m.get_one::<String>("explain") {
        let uid = match resolve(&store, raw) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("smysl salience: {e}");
                return e.code;
            }
        };
        let Some(t) = report.explain(&uid) else {
            eprintln!("smysl salience: no such unit");
            return ExitCode::Failure;
        };
        println!("{uid}: {:.4}", report.get(&uid));
        println!(
            "  centrality      {:.4} x {:.2}",
            t.centrality, req.weights.centrality
        );
        println!(
            "  corroboration   {:.4} x {:.2}  ({} independent group(s))",
            t.corroboration,
            req.weights.corroboration,
            t.groups.len()
        );
        for g in &t.groups {
            println!("      counted: {g}");
        }
        for g in &t.dependent_groups {
            println!("      shared ancestry, not counted: {g}");
        }
        println!("  role            {:.4} x {:.2}", t.role, req.weights.role);
        println!("  raw             {:.4}", t.raw);
        if t.authored {
            println!("  authored override in force; the derived value is not used");
        }
        return ExitCode::Success;
    }

    let n = m
        .get_one::<String>("top")
        .and_then(|s| s.parse().ok())
        .unwrap_or(usize::MAX);
    if global.get_flag("json") {
        let rows: Vec<String> = report
            .top(n)
            .into_iter()
            .map(|(uid, score)| {
                format!(
                    "{{\"uid\":{},\"score\":{score:.4}}}",
                    smysl::json_escape(&uid.canonical())
                )
            })
            .collect();
        println!("{{\"ranking\":[{}]}}", rows.join(","));
        return ExitCode::Success;
    }
    for (uid, score) in report.top(n) {
        println!("{score:.4}  {uid}");
    }
    ExitCode::Success
}

/// `smysl find` - rank units against a query.
///
/// Pure, which is not the usual shape for search: the default engine is BM25 over gists,
/// bodies and details, with no model, no index on disk and no network. Two runs over the
/// same store return the same ranking, ties broken by uid.
///
/// It indexes the **gist** principally, and that is what makes it work across payload kinds.
/// A unit's payload may be a stack trace, a metric series or a diff, but its gist is a
/// natural-language summary of whatever that is, and the summary is what you search.
///
/// Measured over the corpus: near-perfect on identifiers and on `evidence` units, and weak
/// on `claim` units phrased differently from the query — the right claim reaches the top
/// five three times in four but ranks first once in eight. `--kind` exists partly for that
/// reason: narrowing to what you are actually after beats reading past what you are not.
fn cmd_find(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let Some(path) = store_arg(m, global) else {
        eprintln!("smysl find: no store given");
        return ExitCode::Usage;
    };
    warn_output_is_a_report(global, "find");
    let (store, _) = match load_memory(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl find: {e}");
            return ExitCode::Failure;
        }
    };

    let text = m.get_one::<String>("query").cloned().unwrap_or_default();
    let limit = m.get_one::<usize>("limit").copied().unwrap_or(10);
    let mut q = smysl::Query::new(text, limit);

    if let Some(kinds) = m.get_many::<String>("kind") {
        let mut parsed = Vec::new();
        for raw in kinds {
            match smysl::KernelType::parse(raw) {
                Some(k) => parsed.push(k),
                None => {
                    eprintln!("smysl find: `{raw}` is not a kernel type");
                    return ExitCode::Usage;
                }
            }
        }
        q = q.kinds(parsed);
    }
    if let Some(raw) = m.get_one::<String>("min-status") {
        match smysl::Status::parse(raw) {
            Some(st) => q = q.min_status(st),
            None => {
                eprintln!("smysl find: `{raw}` is not a status");
                return ExitCode::Usage;
            }
        }
    }

    if let Some(prefix) = m.get_one::<String>("source") {
        let from = store.units_with_source_prefix(prefix);
        // An empty `within` means *unrestricted*, so a prefix that matches nothing would
        // silently search the whole store — the opposite of what the caller asked for.
        if from.is_empty() {
            eprintln!("{path}: no unit has a source starting with `{prefix}`");
            return ExitCode::Success;
        }
        q = q.within(from);
    }
    if let Some(ids) = m.get_many::<String>("schema") {
        let mut parsed = Vec::new();
        for raw in ids {
            match smysl::SchemaId::parse(raw) {
                Ok(s) => parsed.push(s),
                Err(_) => {
                    eprintln!("smysl find: `{raw}` is not a schema id");
                    return ExitCode::Usage;
                }
            }
        }
        q = q.schemas(parsed);
    }
    if let Some(raw) = m.get_one::<String>("payload") {
        match payload_filter(raw, "find") {
            Ok((key, values)) => q = q.with_payload(key, values),
            Err(code) => return code,
        }
    }

    // The trait is the seam, and the command depends on the interface rather than on an
    // engine: which one is `--engine`'s to say.
    let (engine, model) = match engine_for(m, "find") {
        Ok(v) => v,
        Err(code) => return code,
    };
    // `--lang`, `--analyzer` and `--library` are about how the *lexical* index is analysed and
    // where it comes from. A semantic run has neither an analyzer nor a postings table, so
    // accepting them there would be accepting an argument that does nothing — the F-13 class of
    // defect this repository has one of already.
    let chose = m.get_one::<String>("lang").is_some()
        || m.get_one::<String>("analyzer").is_some()
        || m.get_flag("library");
    let hits = if chose && engine != Engine::Lexical {
        eprintln!(
            "smysl find: --lang, --analyzer and --library choose how the lexical index is \
             analysed, and --engine is not lexical"
        );
        return ExitCode::Usage;
    } else if chose {
        match find_with_chosen(&store, &q, m, &path, "find") {
            Ok(h) => h,
            Err(code) => return code,
        }
    } else {
        match ranked(&store, &q, engine, model.as_deref(), "find") {
            Ok(h) => h,
            Err(code) => return code,
        }
    };

    if global.get_flag("json") {
        let rows: Vec<String> = hits
            .iter()
            .map(|h| {
                let terms: Vec<String> = h
                    .terms
                    .iter()
                    .map(|(term, c)| format!("[{},{c:.4}]", smysl::json_escape(term)))
                    .collect();
                format!(
                    "{{\"uid\":{},\"score\":{:.4},\"terms\":[{}]}}",
                    smysl::json_escape(&h.uid.canonical()),
                    h.score,
                    terms.join(",")
                )
            })
            .collect();
        println!("{{\"hits\":[{}]}}", rows.join(","));
        return ExitCode::Success;
    }

    if hits.is_empty() {
        // On stderr, so a caller piping stdout gets an empty result rather than a sentence
        // it has to recognise and strip.
        eprintln!("{path}: nothing matched");
        return ExitCode::Success;
    }
    let why = m.get_flag("why");
    for h in &hits {
        let gist = store
            .get(&h.uid)
            .map(|u| u.core.gist.clone())
            .unwrap_or_default();
        println!("{:.4}  {}  {}", h.score, h.uid, gist);
        if why {
            // On stderr with the uid repeated, so a caller piping stdout still gets the ranking
            // it had and a reader watching the terminal sees which term did the work.
            let terms: Vec<String> = h.terms.iter().map(|(t, c)| format!("{t} {c:.4}")).collect();
            eprintln!(
                "{}  matched: {}",
                h.uid,
                if terms.is_empty() {
                    "(this retriever does not say)".to_string()
                } else {
                    terms.join(", ")
                }
            );
        }
    }
    ExitCode::Success
}

/// `smysl q` - run an `sq/1` query (SMYSL-2.5 §4.4, TX-P6 step 7).
///
/// The command is a thin dispatcher over `smysl::query`: parse, plan, run, write. What it
/// adds is the three things the crate cannot do for itself because it is pure — choose which
/// scope to bind (a plain store, or a library with its times and entities), write the `View`
/// record `pack … as view` asks for, and perform `--seed-check`'s stronger assertion: run the
/// plan twice and compare, as §3.2 specifies, rather than take the plan's purity label on
/// trust. The second run builds its own postings tables and scratch, because the executor
/// builds them per run and keeps nothing between two.
fn cmd_q(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::query::ast::Shaping;
    use smysl::query::{Env, IdfScope, PlanOptions};

    // `--rerun <uid>` takes no query: the text is the saved unit's. One positional then means
    // the store, which is what a reader typing `smysl q --rerun b3:… store.smy` wrote.
    let rerun = m.get_one::<String>("rerun").cloned();
    let (text, positional_store) = match (
        &rerun,
        m.get_one::<String>("query"),
        m.get_one::<String>("file"),
    ) {
        (Some(_), _, Some(_)) => {
            eprintln!("smysl q: --rerun runs the saved query's own text; --file is not taken");
            return ExitCode::Usage;
        }
        (Some(_), q, None) => (None, q.cloned()),
        (None, Some(_), Some(_)) => {
            eprintln!("smysl q: give the query once, inline or with --file");
            return ExitCode::Usage;
        }
        (None, Some(t), None) => (Some(t.clone()), None),
        (None, None, Some(f)) => match std::fs::read_to_string(f) {
            Ok(t) => (Some(t), None),
            Err(e) => {
                eprintln!("smysl q: {f}: {e}");
                return ExitCode::Failure;
            }
        },
        (None, None, None) => {
            eprintln!("smysl q: no query given");
            return ExitCode::Usage;
        }
    };
    let Some(path) = store_arg(m, global).or(positional_store) else {
        eprintln!("smysl q: no store given");
        return ExitCode::Usage;
    };
    warn_output_is_a_report(global, "q");

    let mut opts = PlanOptions::default();
    opts.all_versions = m.get_flag("all-versions");
    opts.limit = m.get_one::<usize>("per-partition").copied().unwrap_or(10);
    opts.idf_scope = match m.get_one::<String>("idf").map(String::as_str) {
        Some("partition") => IdfScope::Partition,
        Some("library") => IdfScope::Library,
        _ => IdfScope::Collection,
    };
    if let Some(params) = m.get_many::<String>("param") {
        for raw in params {
            let Some((k, v)) = raw.split_once('=') else {
                eprintln!("smysl q: --param takes K=V, not `{raw}`");
                return ExitCode::Usage;
            };
            opts.params.insert(k.to_string(), v.to_string());
        }
    }

    let job = match (rerun, text) {
        (Some(uid), _) => {
            if m.get_flag("save") && !m.get_flag("current") {
                // `--current --save` writes a new saved query that supersedes the old one
                // (SMYSL-2.5 §3.6); a pinned rerun saved again would be the same query with the
                // same recipe over the same text, which is what the old unit already is.
                eprintln!(
                    "smysl q: --rerun --save supersedes a saved query under current revisions, \
                     which needs --current"
                );
                return ExitCode::Usage;
            }
            Job::Rerun {
                uid,
                current: m.get_flag("current"),
            }
        }
        (None, Some(text)) => {
            let parsed = match smysl::query::parse(&text) {
                Ok(p) => p,
                Err(d) => {
                    eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                    return ExitCode::Usage;
                }
            };
            for w in &parsed.warnings {
                eprintln!("smysl q: {}: {}", w.code.as_str(), w.message);
            }
            if m.get_flag("current") {
                eprintln!("smysl q: --current is a rerun's flag; give --rerun <uid>");
                return ExitCode::Usage;
            }

            // `pack … as view` is a query that writes, and the pure crate refuses it
            // (`SMY-E461`, "writing is the CLI's"). This is the CLI: the stage is planned
            // without its write, and the write happens here, after the answer, as a `View`
            // whose roots are what was packed.
            let mut query = parsed.query.clone();
            let mut as_view = false;
            for stage in &mut query.shaping {
                if let Shaping::Pack { as_view: wants, .. } = stage {
                    if *wants {
                        as_view = true;
                        *wants = false;
                    }
                }
            }
            Job::Run {
                query: Box::new(query),
                as_view,
            }
        }
        (None, None) => unreachable!("a run has text and a rerun has a uid"),
    };

    // The scope's name is the file's stem, so a query written `from store:F1-incident` over
    // `F1-incident.smy` resolves, and the plan goldens in `fixtures/sq/plans/` — produced
    // under that naming — are what `--explain` prints here, byte for byte.
    let name = std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());

    if m.get_flag("library") {
        #[cfg(not(feature = "text"))]
        {
            let _ = (&job, &opts, &name);
            eprintln!(
                "smysl q: {path}: --library needs the library layer, and this build has no \
                 `text` feature"
            );
            ExitCode::Usage
        }
        #[cfg(feature = "text")]
        {
            let library = match smysl::text::library::Library::open(std::path::Path::new(&path)) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("smysl q: {path}: {e}");
                    return ExitCode::Failure;
                }
            };
            let scope = smysl::text::scope::LibraryScope::new(name, &library);
            let port = match q_semantic(
                m,
                library.store(),
                &library.root().join("index").join("units"),
            ) {
                Ok(p) => p,
                Err(code) => return code,
            };
            let mut env = Env::new(library.store(), &scope)
                .with_times(&scope)
                .with_entities(&scope);
            if let Some(port) = port.as_deref() {
                env = env.with_semantic(port);
            }
            let (code, records) = q_run(&job, &env, &opts, m, global);
            if !records.is_empty() {
                // Through the library, so its index is maintained; the records then live in
                // the catalogue shard like any other.
                let mut library = library;
                if let Err(e) = library.append_records(&records) {
                    eprintln!("smysl q: {path}: {e}");
                    return ExitCode::Failure;
                }
            }
            code
        }
    } else {
        let (store, labels) = match load_memory(&path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("smysl q: {e}");
                return ExitCode::Failure;
            }
        };
        let scope = smysl::query::StoreScope::new(name, &store);
        let port = match q_semantic(m, &store, std::path::Path::new(&path)) {
            Ok(p) => p,
            Err(code) => return code,
        };
        // The entity port too (TX-P8 step 3): a plain store answers `about entity(…)` from its
        // own `x.text/entity` units and `same-as` classes, which is what lets a dossier slot
        // saved over it be rerun here.
        let mut env = Env::new(&store, &scope).with_entities(&scope);
        if let Some(port) = port.as_deref() {
            env = env.with_semantic(port);
        }
        let (code, records) = q_run(&job, &env, &opts, m, global);
        if !records.is_empty() {
            if let Err(e) = persist(&path, &records, &store, &labels) {
                eprintln!("smysl q: {e}");
                return ExitCode::Failure;
            }
        }
        code
    }
}

/// What every `ask` says first, on stderr: GE-T10's kill criterion, applied (SMYSL-2.5 §6 TX-P8
/// step 4).
///
/// Draft 3 §23 keeps `ask` out of defaults when *definition* or *counter-evidence* precision
/// is below 0.8, and counter-evidence measured 0.750. This build has no tier of commands that
/// are built but not offered, so "out of defaults" is this: the command is labelled
/// experimental where it is listed and every run says so, with the figure and where it was
/// measured, and nothing a script reads on stdout changes. The note goes when a gold run puts
/// the slot over the bar.
#[cfg(feature = "text")]
const ASK_EXPERIMENTAL: &str = "smysl ask: experimental — GE-T10 measured counter-evidence \
     precision at 0.750, below draft 3's 0.8 (SMYSL-2.5 §6 TX-P8 step 4); a counter slot can \
     hold a rebuttal of a claim the question did not ask about";

/// `smysl ask` — a dossier for a question (SMYSL-2.5 §3.13, TX-P8 step 3).
///
/// Pure unless `--answer`: the intent comes from a rule table, the slots from a template set
/// compiled into the binary, and every slot is a pure plan, so the same question over the same
/// store is the same dossier byte for byte. The store is a library when its root holds one and a
/// plain store otherwise — `ask` resolves its subject through `x.text/entity` units, which both
/// can hold, so there is no `--library` to forget. The dossier is rendered to stdout through the
/// same profile and backends `render` uses; `--save` appends it, and `--answer` asks a model to
/// answer from it and stages what comes back (rule S), which is why `--answer` needs `--save`:
/// the answer's grounds are the dossier's units, and a staged unit grounded on a question the
/// store does not hold is a batch that cannot be committed.
#[cfg(feature = "text")]
fn cmd_ask(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::query::dossier::Intent;
    use smysl::query::{ask, AskError, AskOptions, Env};

    let question = m.get_one::<String>("question").expect("required by clap");
    let Some(path) = store_arg(m, global).or(m.get_one::<String>("store").cloned()) else {
        eprintln!("smysl ask: no store given");
        return ExitCode::Usage;
    };
    warn_output_is_a_report(global, "ask");
    eprintln!("{ASK_EXPERIMENTAL}");
    if m.get_flag("answer") && !m.get_flag("save") {
        eprintln!(
            "smysl ask: --answer grounds the model's units on the dossier, so the dossier is \
             appended first; add --save"
        );
        return ExitCode::Usage;
    }
    if m.get_flag("yes") && !m.get_flag("answer") {
        eprintln!("smysl ask: --yes commits a staged answer; there is none without --answer");
        return ExitCode::Usage;
    }

    let owner = match AgentId::new("tool:smysl-ask") {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl ask: {e}");
            return ExitCode::Failure;
        }
    };
    let mut opts = AskOptions::new(owner.clone()).with_templates(
        m.get_one::<String>("templates")
            .cloned()
            .unwrap_or_default(),
    );
    opts = opts.with_ts(Hlc::new(0, 0, owner));
    if let Some(i) = m.get_one::<String>("intent") {
        opts = opts.with_intent(Intent::parse(i).expect("checked by clap"));
    }
    if let Some(l) = m.get_one::<String>("lang") {
        opts = opts.with_lang(l.clone());
    }
    if let Some(sc) = m.get_one::<String>("scope") {
        opts = opts.with_scope(sc.clone());
    }
    if let Some(b) = m.get_one::<u64>("budget") {
        opts = opts.with_budget(*b);
    }
    let target = match m.get_one::<String>("target") {
        Some(t) => match Target::parse(t) {
            Some(t) => t,
            None => {
                eprintln!("smysl ask: `{t}` is not a target");
                return ExitCode::Usage;
            }
        },
        None => Target::Markdown,
    };

    let mut corpus = match Corpus::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("smysl ask: {path}: {e}");
            return ExitCode::Failure;
        }
    };
    if let Some(raw) = m.get_one::<String>("subject") {
        match UidPrefix::parse(raw).and_then(|p| corpus.store().resolve_prefix(&p)) {
            Ok(u) => opts = opts.with_subject(u),
            Err(e) => {
                eprintln!("smysl ask: --subject {raw}: {e}");
                return ExitCode::Usage;
            }
        }
    }
    let name = std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());

    let run = || match &corpus {
        Corpus::Library(library) => {
            let scope = smysl::text::scope::LibraryScope::new(name.clone(), library);
            let env = Env::new(library.store(), &scope)
                .with_times(&scope)
                .with_entities(&scope);
            ask(question, &env, &opts)
        }
        Corpus::Store(store, _) => {
            let scope = smysl::query::StoreScope::new(name.clone(), store);
            let env = Env::new(store, &scope).with_entities(&scope);
            ask(question, &env, &opts)
        }
    };
    let asked = run();
    // `--seed-check`: §3.2's assertion, as `q` makes it — the whole pipeline twice, the second
    // time with every table rebuilt, and the dossiers compared record for record.
    if global.get_flag("seed-check") {
        if let (Ok(a), Ok(b)) = (&asked, &run()) {
            if a.records != b.records {
                eprintln!(
                    "smysl ask: --seed-check asked twice and the dossiers differ; this is an \
                     internal error, not a property of the question"
                );
                return ExitCode::Failure;
            }
        }
    }
    let dossier = match asked {
        Ok(d) => d,
        Err(AskError::Ambiguous {
            name,
            candidates,
            diagnostic,
        }) => {
            eprintln!(
                "smysl ask: {}: {}",
                diagnostic.code.as_str(),
                diagnostic.message
            );
            for (i, group) in candidates.iter().enumerate() {
                let label = group
                    .iter()
                    .filter_map(|u| corpus.store().get(u))
                    .map(|u| match u.core.source.as_ref() {
                        Some(s) => format!("{} ({})", u.core.gist, s.reference),
                        None => u.core.gist.clone(),
                    })
                    .collect::<Vec<_>>()
                    .join(" = ");
                eprintln!("  {}. --subject {}  {label}", i + 1, group[0].canonical());
            }
            let _ = name;
            return ExitCode::Usage;
        }
        Err(e @ (AskError::Refused(_) | AskError::NotACandidate { .. })) => {
            eprintln!("smysl ask: {e}");
            return ExitCode::Usage;
        }
        Err(e) => {
            eprintln!("smysl ask: {e}");
            return ExitCode::Failure;
        }
    };

    let gaps = dossier
        .thread
        .steps
        .iter()
        .filter(|s| s.role == Role::Gap)
        .count();
    let recipe: String = dossier.recipe[..6]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    eprintln!(
        "smysl ask: {} — {} ({}), subject `{}` ({} entit{}), {} slot(s), {} gap(s), recipe {recipe}",
        dossier.thread.id,
        dossier.classified.intent,
        dossier.classified.rule,
        dossier.subject,
        dossier.entities.len(),
        if dossier.entities.len() == 1 { "y" } else { "ies" },
        dossier.slots.len(),
        gaps,
    );
    for w in &dossier.warnings {
        eprintln!("smysl ask: {}: {}", w.code.as_str(), w.message);
    }

    // The records a save appends: the declaration with the first saved query a store receives,
    // then the dossier.
    let mut records: Vec<Record> = Vec::new();
    if !declares_query(corpus.store()) {
        records.push(Record::SchemaDecl(smysl::schemas::query::decl()));
    }
    records.extend(dossier.records.iter().cloned());

    // Rendered over the store with the dossier in it, saved or not, so every step resolves.
    let mut scratch = corpus.store().clone();
    if let Err(e) = scratch.append(&records) {
        eprintln!("smysl ask: the dossier does not append: {e}");
        return ExitCode::Failure;
    }
    let profile = Profile::builtin("plain").expect("the plain profile loads");
    let ir = smysl::build_ir(
        &scratch,
        &dossier.thread,
        &profile,
        &BuildOptions::default(),
    );
    match smysl::emit_artifact(target, &ir, &profile) {
        Ok(a) => print!("{}", a.text),
        Err(e) => {
            eprintln!("smysl ask: {e}");
            return ExitCode::Failure;
        }
    }

    if !m.get_flag("save") {
        return ExitCode::Success;
    }
    match &mut corpus {
        Corpus::Library(library) => {
            if let Err(e) = library.append_records(&records) {
                eprintln!("smysl ask: {path}: {e}");
                return ExitCode::Failure;
            }
        }
        Corpus::Store(store, labels) => {
            if let Err(e) = persist(&path, &records, store, labels) {
                eprintln!("smysl ask: {e}");
                return ExitCode::Failure;
            }
        }
    }
    eprintln!(
        "smysl ask: saved {} record(s): question {}, view {}",
        records.len(),
        dossier.question,
        dossier.view.id
    );
    if !m.get_flag("answer") {
        return ExitCode::Success;
    }
    ask_answer(m, global, &path, corpus, scratch, &dossier)
}

#[cfg(not(feature = "text"))]
fn cmd_ask(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl ask: needs the library layer (build with --features text)");
    ExitCode::Usage
}

/// A `--param` value as a recipe parameter: an integer, a boolean, a ratio `n/d`, or text.
#[cfg(feature = "text")]
fn measure_param(v: &str) -> smysl::measure::Param {
    use smysl::measure::Param;
    if let Ok(i) = v.parse::<i64>() {
        return Param::Int(i);
    }
    match v {
        "true" => return Param::Bool(true),
        "false" => return Param::Bool(false),
        _ => {}
    }
    if let Some((n, d)) = v.split_once('/') {
        if let (Ok(n), Ok(d)) = (n.parse::<u32>(), d.parse::<u32>()) {
            return Param::Ratio(n, d);
        }
    }
    Param::Text(v.to_string())
}

/// `--level`'s spellings, as SMYSL-2.6 §3.4 writes them.
#[cfg(feature = "text")]
fn measure_level(s: &str) -> Option<smysl::measure::Level> {
    use smysl::measure::Level;
    Some(match s {
        "part" => Level::Part,
        "chapter" => Level::Chapter,
        "verse" => Level::Verse,
        "message" => Level::Message,
        w => Level::Window(w.strip_prefix("window:")?.parse().ok()?),
    })
}

/// `smysl measure` — the text-level measures as derived data units (SMYSL-2.6 §4.4, TX-P8 M7).
///
/// Thin, as `q` is: everything a measure computes is `smysl::measure`'s, and what is here is
/// what the pure crate cannot do for itself. It opens the library; resolves the scope with sq
/// and saves it as the results' basis; reads the text once at `--level`; builds **one recipe
/// per id**, so that a parameter or a `min:n` given for one measure is not written into
/// another's recipe and does not move its hash; runs them in the order given; and appends what
/// ran — the basis, each result's unit and an attestation of this run — through the library.
///
/// A unit's identity is its recipe, its scope and its value, so a second run over the same
/// library adds no unit: the report says how many were new and how many the store already
/// held, and only the run's attestations are appended again. `--dry-run` writes nothing.
/// `--seed-check` computes every result twice and compares the payload bytes.
#[cfg(feature = "text")]
fn cmd_measure(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use std::collections::BTreeMap;

    use smysl::measure::text::{texts, Corpus, TextError};
    use smysl::measure::{self as ms, NotPersisted, Outcome, Recipe};
    use smysl::query::{Env, PlanOptions};
    use smysl::text::library::Library;

    let mut ids: Vec<String> = Vec::new();
    for raw in m
        .get_one::<String>("ids")
        .expect("required by clap")
        .split(',')
    {
        let id = raw.trim();
        if !id.is_empty() && !ids.iter().any(|i| i == id) {
            ids.push(id.to_string());
        }
    }
    if ids.is_empty() {
        eprintln!("smysl measure: no measure named; give ids such as T1,T2");
        return ExitCode::Usage;
    }
    let scope_text = m.get_one::<String>("scope").expect("required by clap");
    let channel_text = m.get_one::<String>("channel");
    let Some(path) = store_arg(m, global) else {
        eprintln!("smysl measure: no library given");
        return ExitCode::Usage;
    };
    warn_output_is_a_report(global, "measure");
    let raw_level = m.get_one::<String>("level").expect("has a default");
    let Some(level) = measure_level(raw_level) else {
        eprintln!(
            "smysl measure: --level is part, chapter, verse, message or window:<n>, not `{raw_level}`"
        );
        return ExitCode::Usage;
    };
    let seed_check = global.get_flag("seed-check");
    let policy = match m.get_one::<String>("policy") {
        None => None,
        Some(raw) => match measure_policy(raw) {
            Some(p) => Some(p),
            None => {
                eprintln!(
                    "smysl measure: --policy is strict, component or attested:<n>, not `{raw}`"
                );
                return ExitCode::Usage;
            }
        },
    };
    let raw_axis = m.get_one::<String>("axis").expect("has a default");
    let axis = match raw_axis.as_str() {
        "said" => smysl::measure::Axis::Said,
        "composed" => smysl::measure::Axis::Composed,
        "about" => smysl::measure::Axis::About,
        other => {
            eprintln!("smysl measure: --axis is said, composed or about, not `{other}`");
            return ExitCode::Usage;
        }
    };

    // A target no id reads would be a recipe field written for nothing, as an unread
    // parameter would.
    let reads_channel = |id: &String| {
        ms::find(ms::ALL, id, None).is_ok_and(|x| x.needs().contains(ms::Needs::CHANNEL))
    };
    if channel_text.is_some() && !ids.iter().any(reads_channel) {
        eprintln!(
            "smysl measure: --channel: none of the ids asked for compares two scopes (C1–C5 do)"
        );
        return ExitCode::Usage;
    }

    // Each id's defaults, so a parameter can be routed to the ids that take it. An id this
    // build does not have is not refused here: it is a result, not run (`SMY-E489`), so the
    // report lists every id asked for and a typo stands beside the measures that ran.
    let mut defaults: BTreeMap<&str, Recipe> = BTreeMap::new();
    for id in &ids {
        if let Ok(meas) = ms::find(ms::ALL, id, None) {
            if seed_check && meas.purity() != ms::Purity::Pure {
                eprintln!(
                    "smysl measure: --seed-check asserts rule D, and {id} calls a model, so two \
                     runs need not agree"
                );
                return ExitCode::Usage;
            }
            let mut r = Recipe::new(id.as_str(), meas.version(), scope_text.as_str());
            meas.complete(&mut r);
            defaults.insert(id.as_str(), r);
        }
    }

    let mut params: BTreeMap<String, BTreeMap<String, smysl::measure::Param>> = BTreeMap::new();
    for raw in m.get_many::<String>("param").into_iter().flatten() {
        let Some((lhs, v)) = raw.split_once('=') else {
            eprintln!("smysl measure: --param takes [ID.]K=V, not `{raw}`");
            return ExitCode::Usage;
        };
        let targets: Vec<&str> = match lhs.split_once('.') {
            Some((id, key)) => {
                if !ids.iter().any(|i| i == id) {
                    eprintln!("smysl measure: --param {raw}: {id} is not one of the ids asked for");
                    return ExitCode::Usage;
                }
                match defaults.get(id) {
                    Some(d) if d.params.contains_key(key) => vec![id],
                    Some(d) => {
                        let has: Vec<&str> = d.params.keys().map(String::as_str).collect();
                        eprintln!(
                            "smysl measure: --param {raw}: {id} takes no `{key}`; it takes {}",
                            if has.is_empty() {
                                "none".to_string()
                            } else {
                                has.join(", ")
                            }
                        );
                        return ExitCode::Usage;
                    }
                    None => vec![id],
                }
            }
            None => {
                let takers: Vec<&str> = defaults
                    .iter()
                    .filter(|(_, d)| d.params.contains_key(lhs))
                    .map(|(id, _)| *id)
                    .collect();
                if takers.is_empty() {
                    eprintln!(
                        "smysl measure: --param {raw}: none of the ids asked for takes `{lhs}`"
                    );
                    return ExitCode::Usage;
                }
                takers
            }
        };
        let key = lhs.split_once('.').map_or(lhs, |(_, k)| k);
        for id in targets {
            params
                .entry(id.to_string())
                .or_default()
                .insert(key.to_string(), measure_param(v));
        }
    }

    // `min:n` is in each measure's own unit — tokens, characters, symbols, texts — so one
    // value for every id is allowed and is what it says, and `ID=N` sets one.
    let (mut min_all, mut min_one): (Option<u64>, BTreeMap<String, u64>) = (None, BTreeMap::new());
    for raw in m.get_many::<String>("min-n").into_iter().flatten() {
        let (id, n) = match raw.split_once('=') {
            Some((id, n)) => (Some(id), n),
            None => (None, raw.as_str()),
        };
        let Ok(n) = n.parse::<u64>() else {
            eprintln!("smysl measure: --min-n takes [ID=]N, not `{raw}`");
            return ExitCode::Usage;
        };
        match id {
            Some(id) if !ids.iter().any(|i| i == id) => {
                eprintln!("smysl measure: --min-n {raw}: {id} is not one of the ids asked for");
                return ExitCode::Usage;
            }
            Some(id) => {
                min_one.insert(id.to_string(), n);
            }
            None => min_all = Some(n),
        }
    }

    let root = std::path::Path::new(&path);
    if !Library::is_library(root) {
        eprintln!(
            "smysl measure: {path}: not a library; every measure this build has reads a \
             library, its text or the units drawn from it (`smysl text add`)"
        );
        return ExitCode::Usage;
    }
    let mut library = match Library::open(root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl measure: {path}: {e}");
            return ExitCode::Failure;
        }
    };
    let name = root
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());
    let analyzer = m.get_one::<String>("analyzer").cloned();

    let (resolved, target, results) = {
        let scope = smysl::text::scope::LibraryScope::new(name, &library);
        let env = Env::new(library.store(), &scope)
            .with_times(&scope)
            .with_entities(&scope);
        let resolve = |text: &str| match ms::resolve(text, &env, &PlanOptions::default()) {
            Ok(r) => Some(r),
            Err(ds) => {
                for d in ds {
                    eprintln!("smysl measure: {}: {}", d.code.as_str(), d.message);
                }
                None
            }
        };
        let Some(resolved) = resolve(scope_text) else {
            return ExitCode::Usage;
        };
        let target = match channel_text {
            None => None,
            Some(t) => match resolve(t) {
                Some(r) => Some(r),
                None => return ExitCode::Usage,
            },
        };
        // The text is read only when an id reads text: a unit measure's scope may filter, which
        // a text scope refuses (`SMY-E461`), and reading a Bible to count its units' statuses
        // would be work nobody asked for.
        let reads_text = ids.iter().any(|id| {
            ms::find(ms::ALL, id, None).is_ok_and(|x| x.needs().contains(ms::Needs::TEXT))
        });
        // Each side's text is read under its own language's analyzer unless one is named,
        // since a channel's target is often another language.
        let load = |text: &str, analyzer: Option<&str>| -> Result<Corpus, ExitCode> {
            let found = match texts(text, &library) {
                Ok(t) => t,
                Err(d) => {
                    eprintln!("smysl measure: {}: {}", d.code.as_str(), d.message);
                    return Err(ExitCode::Usage);
                }
            };
            match Corpus::load(&library, &found, level, analyzer) {
                Ok(c) => Ok(c),
                Err(TextError::Refused(d)) => {
                    eprintln!("smysl measure: {}: {}", d.code.as_str(), d.message);
                    Err(ExitCode::Usage)
                }
                Err(e) => {
                    eprintln!("smysl measure: {path}: {e}");
                    Err(ExitCode::Failure)
                }
            }
        };
        let corpus = if reads_text {
            match load(scope_text, analyzer.as_deref()) {
                Ok(c) => Some(c),
                Err(code) => return code,
            }
        } else {
            None
        };
        let target_corpus = match channel_text {
            Some(t)
                if ids.iter().any(|id| {
                    reads_channel(id)
                        && ms::find(ms::ALL, id, None)
                            .is_ok_and(|x| x.needs().contains(ms::Needs::TEXT))
                }) =>
            {
                match load(t, None) {
                    Ok(c) => Some(c),
                    Err(code) => return code,
                }
            }
            _ => None,
        };
        let mut input = ms::Input::new(library.store(), &resolved).with_library(&library);
        if let Some(c) = &corpus {
            input = input.with_text(c);
        }
        if let Some(t) = &target {
            input = input.with_channel(t);
        }
        if let Some(c) = &target_corpus {
            input = input.with_target_text(c);
        }
        let mut results = Vec::with_capacity(ids.len());
        for id in &ids {
            let mut recipe = Recipe::new("", 0, scope_text.as_str());
            recipe.level = level;
            recipe.analyzer = analyzer.clone();
            recipe.seed = *m.get_one::<u64>("seed").expect("has a default");
            recipe.lossy = m.get_flag("allow-lossy");
            recipe.policy = policy;
            recipe.profile = m.get_one::<String>("profile").cloned();
            recipe.axis = axis;
            recipe.min_n = min_one.get(id).copied().or(min_all);
            recipe.params = params.get(id).cloned().unwrap_or_default();
            let r = ms::run(&[id.as_str()], &recipe, &input).remove(0);
            if seed_check {
                let again = ms::run(&[id.as_str()], &recipe, &input).remove(0);
                let bytes = |r: &ms::MeasureResult| match &r.outcome {
                    Outcome::Value(v) => ms::to_payload(&r.recipe, v).ok(),
                    _ => None,
                };
                if again != r || bytes(&again) != bytes(&r) {
                    eprintln!(
                        "smysl measure: --seed-check computed {id} twice and the payloads \
                         differ; rule D does not hold for this run"
                    );
                    return ExitCode::Failure;
                }
            }
            results.push(r);
        }
        (resolved, target, results)
    };

    // Each result's unit, if it ran and has a basis to stand on.
    let mut units: Vec<Option<(Uid, smysl::UnitCore, bool)>> = Vec::with_capacity(results.len());
    for r in &results {
        for d in &r.diagnostics {
            eprintln!("smysl measure: {}: {}", d.code.as_str(), d.message);
        }
        let channel = r.recipe.channel.is_some();
        let bases = if channel {
            vec![
                resolved.basis.as_ref(),
                target.as_ref().and_then(|t| t.basis.as_ref()),
            ]
        } else {
            vec![resolved.basis.as_ref()]
        };
        match ms::to_unit_on(r, &bases) {
            Ok(core) => {
                let uid = smysl::canonical_uid(&core);
                let present = library.store().get(&uid).is_some();
                units.push(Some((uid, core, present)));
            }
            Err(NotPersisted::NotRun) => units.push(None),
            Err(NotPersisted::NoBasis(d)) | Err(NotPersisted::Unwritable(d)) => {
                eprintln!("smysl measure: {}: {}", d.code.as_str(), d.message);
                units.push(None);
            }
            Err(_) => units.push(None),
        }
    }
    let dry_run = m.get_flag("dry-run");
    let new = units.iter().flatten().filter(|u| !u.2).count();
    let present = units.iter().flatten().filter(|u| u.2).count();

    if global.get_flag("json") {
        let rows: Vec<String> = results
            .iter()
            .zip(&units)
            .map(|(r, u)| measure_json(r, u.as_ref().map(|(uid, _, p)| (*uid, *p))))
            .collect();
        println!(
            "{{\"scope\":{},\"basis\":{},\"channel\":{},\"channel_basis\":{},\"dry_run\":{dry_run},\"new\":{new},\"present\":{present},\"results\":[{}]}}",
            smysl::json_escape(scope_text),
            resolved
                .basis()
                .map_or("null".to_string(), |b| smysl::json_escape(&b.canonical())),
            channel_text.map_or("null".to_string(), |t| smysl::json_escape(t)),
            target
                .as_ref()
                .and_then(|t| t.basis())
                .map_or("null".to_string(), |b| smysl::json_escape(&b.canonical())),
            rows.join(",")
        );
    } else {
        for (r, u) in results.iter().zip(&units) {
            measure_print(r, u.as_ref().map(|(uid, _, p)| (*uid, *p)), dry_run);
        }
    }

    if dry_run {
        eprintln!("smysl measure: --dry-run: nothing written");
        return ExitCode::Success;
    }
    if let (Some(basis), true) = (&resolved.basis, units.iter().any(Option::is_some)) {
        let agent = match AgentId::new("tool:smysl-measure") {
            Ok(a) => a,
            Err(e) => {
                eprintln!("smysl measure: {e}");
                return ExitCode::Failure;
            }
        };
        let at = m.get_one::<u64>("at").copied().unwrap_or_else(now_millis);
        let mut records = Vec::new();
        if !declares_query(library.store()) {
            records.push(Record::SchemaDecl(smysl::schemas::query::decl()));
        }
        records.push(Record::Unit(basis.unit.clone()));
        records.extend(basis.edges.iter().cloned().map(Record::Relation));
        // A channel result rests on the target's saved scope as well; the store deduplicates
        // it when the two are one query.
        if let Some(tb) = target.as_ref().and_then(|t| t.basis.as_ref()) {
            if results.iter().any(|r| r.recipe.channel.is_some()) {
                records.push(Record::Unit(tb.unit.clone()));
                records.extend(tb.edges.iter().cloned().map(Record::Relation));
            }
        }
        for ((uid, core, _), r) in units
            .iter()
            .zip(&results)
            .filter_map(|(u, r)| u.as_ref().map(|u| (u, r)))
        {
            let mut att = smysl::Attestation::new(
                *uid,
                agent.clone(),
                smysl::Op::Transformed,
                smysl::Rung::Computed,
                Hlc::new(at, 0, agent.clone()),
            );
            att.recipe = r.recipe.hash().ok();
            records.push(Record::Unit(core.clone()));
            records.push(Record::Attestation(att));
        }
        if let Err(e) = library.append_records(&records) {
            eprintln!("smysl measure: {path}: {e}");
            return ExitCode::Failure;
        }
    }
    eprintln!(
        "smysl measure: {} result(s), {} unit(s): {new} new, {present} already in the store",
        results.len(),
        new + present
    );
    ExitCode::Success
}

/// `--policy`: `strict`, `component` or `attested:<n>`.
#[cfg(feature = "text")]
fn measure_policy(raw: &str) -> Option<smysl::measure::ClassPolicy> {
    use smysl::measure::ClassPolicy;
    match raw {
        "strict" => Some(ClassPolicy::Strict),
        "component" => Some(ClassPolicy::Component),
        other => other
            .strip_prefix("attested:")
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|n| *n > 0)
            .map(ClassPolicy::Attested),
    }
}

/// One result as the text report prints it: the id, the unit or why there is none, the gist,
/// and every number under it.
#[cfg(feature = "text")]
fn measure_print(r: &smysl::measure::MeasureResult, unit: Option<(Uid, bool)>, dry_run: bool) {
    use smysl::measure::Outcome;

    let state = match unit {
        Some((uid, present)) => format!(
            "{uid}  {}",
            if dry_run {
                "not written"
            } else if present {
                "already in the store"
            } else {
                "new"
            }
        ),
        None => "no unit".to_string(),
    };
    println!("{} v{}  {state}", r.recipe.id, r.recipe.v);
    match &r.outcome {
        Outcome::Value(v) => {
            println!("  {}", smysl::measure::gist(&r.recipe, v));
            for (k, n) in &v.n {
                println!("  n.{k} = {n}");
            }
            for (k, x) in &v.values {
                match v.ci.get(k) {
                    Some(ci) => println!("  {k} = {x}  [{}, {}]", ci.lo, ci.hi),
                    None => println!("  {k} = {x}"),
                }
            }
            for f in &v.flags {
                println!("  flag {f}");
            }
        }
        Outcome::NotRun { why, code } => match code {
            Some(c) => println!("  not run: {}: {why}", c.as_str()),
            None => println!("  not run: {why}"),
        },
        _ => println!("  not run"),
    }
}

/// One result as `--json`'s object: numbers rendered from their stored integers, as the gist is.
#[cfg(feature = "text")]
fn measure_json(r: &smysl::measure::MeasureResult, unit: Option<(Uid, bool)>) -> String {
    use smysl::measure::Outcome;

    let hash = r
        .recipe
        .hash()
        .map(|h| h.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .unwrap_or_default();
    let (uid, present) = match unit {
        Some((u, p)) => (smysl::json_escape(&u.canonical()), p.to_string()),
        None => ("null".to_string(), "null".to_string()),
    };
    let head = format!(
        "\"id\":{},\"v\":{},\"recipe\":\"{hash}\",\"uid\":{uid},\"present\":{present}",
        smysl::json_escape(&r.recipe.id),
        r.recipe.v
    );
    match &r.outcome {
        Outcome::Value(v) => {
            let n: Vec<String> =
                v.n.iter()
                    .map(|(k, x)| format!("{}:{x}", smysl::json_escape(k)))
                    .collect();
            let values: Vec<String> = v
                .values
                .iter()
                .map(|(k, x)| {
                    format!(
                        "{}:{}",
                        smysl::json_escape(k),
                        smysl::json_escape(&x.render())
                    )
                })
                .collect();
            let flags: Vec<String> = v.flags.iter().map(|f| smysl::json_escape(f)).collect();
            format!(
                "{{{head},\"outcome\":\"value\",\"gist\":{},\"n\":{{{}}},\"values\":{{{}}},\"flags\":[{}]}}",
                smysl::json_escape(&smysl::measure::gist(&r.recipe, v)),
                n.join(","),
                values.join(","),
                flags.join(",")
            )
        }
        Outcome::NotRun { why, code } => format!(
            "{{{head},\"outcome\":\"not-run\",\"code\":{},\"why\":{}}}",
            code.map_or("null".to_string(), |c| smysl::json_escape(c.as_str())),
            smysl::json_escape(why)
        ),
        _ => format!("{{{head},\"outcome\":\"not-run\",\"code\":null,\"why\":\"\"}}"),
    }
}

#[cfg(not(feature = "text"))]
fn cmd_measure(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!(
        "smysl measure: every measure reads a library's text, and this build has no `text` feature"
    );
    ExitCode::Usage
}

/// `smysl assess key|score|state|diff` (SMYSL-2.6 §3.11, TX-P9 N6).
///
/// `key` reads a view in the store (`smysl view`, or a dossier's) and writes the key unit and
/// a view of its questions; `score` and `state` read a key unit and a reader's attested units
/// and write a score or a state with its view; `diff` reads two states and writes nothing.
/// Every written unit carries an attestation by `tool:smysl-assess` at rung `computed`, as a
/// measure does. Rule A: everything here is `smysl::measure::assess`.
#[cfg(feature = "text")]
fn cmd_assess(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::measure::assess::{self as asx, Judge, Key, Mode, State};
    use smysl::query::{Env, PlanOptions};
    use smysl::text::library::Library;

    let path = match store_path(m, global, "assess") {
        Ok(p) => p,
        Err(code) => return code,
    };
    let action = m
        .get_one::<String>("action")
        .map(String::as_str)
        .unwrap_or("");
    let target = m.get_one::<String>("target").cloned().unwrap_or_default();
    let other = m.get_one::<String>("other");
    if (action == "diff") != other.is_some() {
        eprintln!(
            "smysl assess {action}: {}",
            if action == "diff" {
                "diff compares two states; name the second"
            } else {
                "one target only; the store is `--store` or `-s`"
            }
        );
        return ExitCode::Usage;
    }
    let policy = match m.get_one::<String>("policy") {
        None => None,
        Some(raw) => match measure_policy(raw) {
            Some(p) => Some(p),
            None => {
                eprintln!(
                    "smysl assess: --policy is strict, component or attested:<n>, not `{raw}`"
                );
                return ExitCode::Usage;
            }
        },
    };
    let reader = match (action, m.get_one::<String>("reader")) {
        ("score" | "state", None) => {
            eprintln!("smysl assess {action}: --reader names whose responses are scored");
            return ExitCode::Usage;
        }
        (_, Some(raw)) => match AgentId::new(raw.as_str()) {
            Ok(a) => Some(a),
            Err(e) => {
                eprintln!("smysl assess: --reader: {e}");
                return ExitCode::Usage;
            }
        },
        _ => None,
    };
    let dry_run = m.get_flag("dry-run");
    let json = global.get_flag("json");
    let root = std::path::Path::new(&path);
    if !Library::is_library(root) {
        eprintln!("smysl assess: {path}: not a library (`smysl text add` makes one)");
        return ExitCode::Usage;
    }
    let mut library = match Library::open(root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl assess: {path}: {e}");
            return ExitCode::Failure;
        }
    };
    let refused = |r: smysl::measure::Refusal| {
        eprintln!("smysl assess {action}: {}: {}", r.code.as_str(), r.message);
        if r.code.severity() == Severity::Warn {
            ExitCode::Success
        } else {
            ExitCode::Failure
        }
    };
    let unit_of = |raw: &str| -> Result<Uid, ExitCode> {
        resolve(library.store(), raw).map_err(|e| {
            eprintln!("smysl assess {action}: {e}");
            e.code
        })
    };
    let table = library.index().align.clone();
    let judge = match m.get_one::<String>("judge").map(String::as_str) {
        Some("lexical") => Judge::Lexical(smysl::text::proposition::Lexical {
            k: *m.get_one::<usize>("neighbours").unwrap_or(&1),
            adjacent_only: false,
        }),
        Some("anchored") => Judge::Anchored(&table),
        _ => Judge::Accepted,
    };

    // What is written: units, views, and an attestation per unit.
    let mut out: Vec<Record> = Vec::new();
    let mut written: Vec<Uid> = Vec::new();
    // `--json`: one object, its values already JSON.
    let mut report: Vec<(&str, String)> = vec![("action", smysl::json_escape(action))];
    let push_unit = |out: &mut Vec<Record>, written: &mut Vec<Uid>, core: smysl::UnitCore| {
        let uid = smysl::canonical_uid(&core);
        written.push(uid);
        out.push(Record::Unit(core));
        uid
    };
    let view_id = |kind: &str, uid: &Uid| {
        let c = uid.canonical();
        let tail: String = c.trim_start_matches("b3:").chars().take(16).collect();
        ViewId::new(format!("v/{kind}-{tail}"))
    };

    match action {
        "key" => {
            let Some(view) = library
                .store()
                .views()
                .find(|v| v.id.as_str() == target)
                .cloned()
            else {
                eprintln!("smysl assess key: no view `{target}` in {path}");
                return ExitCode::Failure;
            };
            let Some(policy) = policy else {
                return refused(smysl::measure::Refusal::new(
                    Code::E481,
                    "a key records the class policy it is scored under; give --policy",
                ));
            };
            let mode = m
                .get_one::<String>("mode")
                .and_then(|s| Mode::parse(s))
                .unwrap_or_default();
            let scope = m.get_one::<String>("scope").cloned().unwrap_or_default();
            let key = match asx::key_from_view(library.store(), &view, policy, &scope, mode) {
                Ok(k) => k,
                Err(r) => return refused(r),
            };
            let core = match key.to_unit() {
                Ok(c) => c,
                Err(r) => return refused(r),
            };
            let uid = push_unit(&mut out, &mut written, core);
            match view_id("key", &uid) {
                Ok(id) => out.push(Record::View(key.as_view(id))),
                Err(e) => {
                    eprintln!("smysl assess key: {e}");
                    return ExitCode::Failure;
                }
            }
            if !json {
                println!(
                    "key {}  {} item(s) from {}",
                    uid.canonical(),
                    key.items.len(),
                    key.view
                );
                for i in &key.items {
                    println!(
                        "  {}  {} answer(s), d_path {}, d_spans {}",
                        i.q.canonical(),
                        i.answers.len(),
                        i.d_path.map_or("-".to_string(), |d| d.to_string()),
                        i.d_spans
                    );
                }
            }
            report.push(("key", smysl::json_escape(&uid.canonical())));
            report.push(("items", key.items.len().to_string()));
        }
        "score" | "state" => {
            let key_uid = match unit_of(&target) {
                Ok(u) => u,
                Err(code) => return code,
            };
            let key = match Key::from_unit(library.store(), &key_uid) {
                Ok(k) => k,
                Err(r) => return refused(r),
            };
            let reader = reader.expect("checked above");
            if action == "score" {
                let scope: Option<std::collections::BTreeSet<Uid>> = if key.scope.is_empty() {
                    None
                } else {
                    let name = root
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.clone());
                    let ls = smysl::text::scope::LibraryScope::new(name, &library);
                    let env = Env::new(library.store(), &ls)
                        .with_times(&ls)
                        .with_entities(&ls);
                    match smysl::measure::resolve(&key.scope, &env, &PlanOptions::default()) {
                        Ok(r) => Some(r.uids),
                        Err(ds) => {
                            for d in ds {
                                eprintln!("smysl assess score: {}: {}", d.code.as_str(), d.message);
                            }
                            return ExitCode::Failure;
                        }
                    }
                };
                let sc = match asx::score(
                    library.store(),
                    &key,
                    &reader,
                    policy,
                    &judge,
                    scope.as_ref(),
                ) {
                    Ok(s) => s,
                    Err(r) => return refused(r),
                };
                for d in &sc.diagnostics {
                    eprintln!("smysl assess score: {}: {}", d.code.as_str(), d.message);
                }
                let core = match sc.to_unit(&key_uid) {
                    Ok(c) => c,
                    Err(r) => return refused(r),
                };
                let uid = push_unit(&mut out, &mut written, core);
                let r = |(n, d): (u64, u64)| format!("{n}/{d}");
                if !json {
                    println!(
                        "score {}  {} under {}, judge {}",
                        uid.canonical(),
                        reader.as_str(),
                        sc.policy,
                        sc.judge
                    );
                    println!("  accuracy     {}", r(sc.accuracy));
                    println!("  unanswered   {}", sc.unanswered);
                    println!("  wrong        {}", sc.wrong);
                    println!("  inflation    {}", r(sc.inflation));
                    println!("  holder       {}", r(sc.holder));
                    if let Some(f) = sc.fabrication {
                        println!("  fabrication  {}", r(f));
                    }
                    println!("  unscorable   {}", sc.unscorable);
                    println!("  drifted      {}", sc.drifted);
                }
                report.push(("score", smysl::json_escape(&uid.canonical())));
                report.push(("judge", smysl::json_escape(sc.judge)));
                report.push(("accuracy", smysl::json_escape(&r(sc.accuracy))));
                report.push(("unanswered", sc.unanswered.to_string()));
                report.push(("wrong", sc.wrong.to_string()));
                report.push(("inflation", smysl::json_escape(&r(sc.inflation))));
                report.push(("holder", smysl::json_escape(&r(sc.holder))));
                report.push((
                    "fabrication",
                    sc.fabrication
                        .map_or("null".to_string(), |f| smysl::json_escape(&r(f))),
                ));
                report.push(("unscorable", sc.unscorable.to_string()));
                report.push(("drifted", sc.drifted.to_string()));
            } else {
                let st = match asx::state(library.store(), &key, &key_uid, &reader, policy, &judge)
                {
                    Ok(s) => s,
                    Err(r) => return refused(r),
                };
                let core = match st.to_unit() {
                    Ok(c) => c,
                    Err(r) => return refused(r),
                };
                let uid = push_unit(&mut out, &mut written, core);
                match view_id("state", &uid) {
                    Ok(id) => out.push(Record::View(st.as_view(id))),
                    Err(e) => {
                        eprintln!("smysl assess state: {e}");
                        return ExitCode::Failure;
                    }
                }
                if !json {
                    println!(
                        "state {}  {}: {} class(es) reproduced",
                        uid.canonical(),
                        reader.as_str(),
                        st.classes.len()
                    );
                    for c in &st.classes {
                        println!("  {}  {} member(s)", c[0].canonical(), c.len());
                    }
                }
                report.push(("state", smysl::json_escape(&uid.canonical())));
                report.push(("classes", st.classes.len().to_string()));
            }
        }
        _ => {
            let (a, b) = match (unit_of(&target), unit_of(other.expect("checked above"))) {
                (Ok(a), Ok(b)) => (a, b),
                (Err(code), _) | (_, Err(code)) => return code,
            };
            let (sa, sb) = match (
                State::from_unit(library.store(), &a),
                State::from_unit(library.store(), &b),
            ) {
                (Ok(x), Ok(y)) => (x, y),
                (Err(r), _) | (_, Err(r)) => return refused(r),
            };
            let policy = match policy {
                Some(p) => p,
                None if sa.policy == sb.policy => sa.policy,
                None => {
                    return refused(smysl::measure::Refusal::new(
                        Code::E489,
                        format!(
                            "the states were taken under {} and {}; name the policy to compare them under",
                            sa.policy, sb.policy
                        ),
                    ))
                }
            };
            let d = match asx::diff(library.store(), &sa, &sb, policy) {
                Ok(d) => d,
                Err(r) => return refused(r),
            };
            if !json {
                println!("diff under {policy}");
                for (k, v) in [
                    ("kept", d.kept),
                    ("gained", d.gained),
                    ("lost", d.lost),
                    ("split", d.split),
                    ("merged", d.merged),
                    ("rose", d.rose),
                    ("fell", d.fell),
                ] {
                    println!("  {k:<7}{v}");
                }
            }
            for (k, v) in [
                ("kept", d.kept),
                ("gained", d.gained),
                ("lost", d.lost),
                ("split", d.split),
                ("merged", d.merged),
                ("rose", d.rose),
                ("fell", d.fell),
            ] {
                report.push((k, v.to_string()));
            }
        }
    }
    if json {
        let fields: Vec<String> = report
            .iter()
            .map(|(k, v)| format!("{}:{v}", smysl::json_escape(k)))
            .collect();
        println!("{{{}}}", fields.join(","));
    }
    if dry_run || written.is_empty() {
        if dry_run {
            eprintln!("smysl assess {action}: --dry-run: nothing written");
        }
        return ExitCode::Success;
    }
    let agent = match AgentId::new("tool:smysl-assess") {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl assess: {e}");
            return ExitCode::Failure;
        }
    };
    let at = m.get_one::<u64>("at").copied().unwrap_or_else(now_millis);
    let present = written
        .iter()
        .filter(|u| library.store().get(u).is_some())
        .count();
    for uid in &written {
        out.push(Record::Attestation(smysl::Attestation::new(
            *uid,
            agent.clone(),
            smysl::Op::Transformed,
            smysl::Rung::Computed,
            Hlc::new(at, 0, agent.clone()),
        )));
    }
    if let Err(e) = library.append_records(&out) {
        eprintln!("smysl assess: {path}: {e}");
        return ExitCode::Failure;
    }
    eprintln!(
        "smysl assess {action}: {} unit(s): {} new, {present} already in the store",
        written.len(),
        written.len() - present
    );
    ExitCode::Success
}

#[cfg(not(feature = "text"))]
fn cmd_assess(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl assess: an assessment reads a library, and this build has no `text` feature");
    ExitCode::Usage
}

// ---------------------------------------------------------------------------
// `lib` — import and export (RFC SMYSL-2.7 §4.4, TX-P11 step 5)
// ---------------------------------------------------------------------------

/// `lib <ACTION>`. `--format` is decided here, per action: the table lets `lib` through with
/// every form, because `export` names them all, and the other five write no document.
/// `smysl schema show|check` (SMYSL-2.5 §3.11, TX-P12 step 5).
///
/// Pure: both read the store's declarations through `smysl_core::schema` and nothing else.
/// `check` holds every revision of an id against every earlier one under rule N (`SMY-E426`),
/// each lens step against the revision it leaves (`SMY-E471`), reports forks (`SMY-W429`), and
/// lenses every unit the schema governs to its current revision, listing what does not read
/// whole (`SMY-W420`). Errors exit 3, as `check`'s do.
fn cmd_schema(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::schema::SchemaSet;

    let action = m
        .get_one::<String>("action")
        .map(String::as_str)
        .unwrap_or("");
    let targets: Vec<String> = m
        .get_many::<String>("targets")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    let flag_store = global.get_one::<String>("store").cloned();
    let (id, path) = match (action, targets.as_slice(), flag_store) {
        (_, [id, store], _) => (Some(id.clone()), store.clone()),
        ("show", [id], Some(store)) => (Some(id.clone()), store),
        ("show", [_], None) | ("show", [], _) => {
            eprintln!("smysl schema show: give the schema id and the store");
            return ExitCode::Usage;
        }
        ("check", [id], Some(store)) => (Some(id.clone()), store),
        ("check", [store], None) => (None, store.clone()),
        ("check", [], Some(store)) => (None, store),
        _ => {
            eprintln!("smysl schema {action}: no store given");
            return ExitCode::Usage;
        }
    };
    let id = match id
        .map(|i| SchemaId::parse(&i).map_err(|e| (i, e)))
        .transpose()
    {
        Ok(id) => id,
        Err((i, e)) => {
            eprintln!("smysl schema {action}: `{i}` is not a schema id: {e}");
            return ExitCode::Usage;
        }
    };
    warn_output_is_a_report(global, "schema");
    let (store, _) = match load_memory(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl schema: {e}");
            return ExitCode::Failure;
        }
    };
    let mut decoding = smysl::Report::new();
    let set = SchemaSet::from_decls(store.schema_decls(), &mut decoding);
    for d in decoding.iter() {
        eprintln!("smysl schema: {}: {}", d.code.as_str(), d.message);
    }
    let ids: Vec<SchemaId> = match &id {
        Some(i) => {
            if set.revisions(i).is_empty() {
                eprintln!("smysl schema {action}: the store declares no `{i}`");
                return ExitCode::Failure;
            }
            vec![i.clone()]
        }
        None => set.ids().filter_map(|i| SchemaId::parse(i).ok()).collect(),
    };
    match action {
        "show" => {
            for i in &ids {
                print!("{}", schema_show(&set, i));
            }
            ExitCode::Success
        }
        _ => {
            let mut errors = 0;
            for i in &ids {
                let mut report = smysl::Report::new();
                let n = set.check_rule_n(i, &mut report);
                let t = set.check_units(i, store.units().map(|(u, x)| (u, &x.core)), &mut report);
                for d in report.iter() {
                    eprintln!("smysl schema: {}: {}", d.code.as_str(), d.message);
                }
                for (uid, lost) in &t.unmappable {
                    if lost.is_empty() {
                        eprintln!("  {uid}: nothing reads (no usable path to current)");
                    } else {
                        let keys: Vec<&str> = lost.iter().map(String::as_str).collect();
                        eprintln!("  {uid}: {}", keys.join(", "));
                    }
                }
                let revs = {
                    let mut v: Vec<u32> = set.revisions(i).iter().map(|r| r.version).collect();
                    v.dedup();
                    v.len()
                };
                println!(
                    "{i}: {} over {revs} revision(s); {} unit(s), {} whole, {} unmappable",
                    if n == 0 {
                        "rule N holds".to_string()
                    } else {
                        format!("{n} error(s)")
                    },
                    t.units,
                    t.whole,
                    t.unmappable.len()
                );
                errors += n;
            }
            if errors > 0 {
                ExitCode::CheckErrors
            } else {
                ExitCode::Success
            }
        }
    }
}

/// `smysl migrate --schema ID --to REV` (SMYSL-2.5 §3.11, TX-P12 step 6).
///
/// Deterministic: each unit of the schema written below REV is read forward through the chain
/// and staged as a new unit, attested `Transformed` from the old one with the chain hash as
/// recipe, with a `supersedes` edge new → old; rule M caps each at its grounds, commitments and
/// datings are proposed for the new uids (`SMY-W472`), and nothing is appended — the batch
/// waits in `.smysl/staged.smy` for `merge --staged`, and `relink` re-points references once
/// it is accepted. A unit the target requires a key of that no `default` supplies is
/// `SMY-E473`. Exits 10 when it stages, 11 when rule M lowered a unit, as `ingest` does.
#[cfg(feature = "stage")]
fn cmd_migrate(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::migrate::{migrate, MigrateOptions};

    let path = match store_path(m, global, "migrate") {
        Ok(p) => p,
        Err(code) => return code,
    };
    let id = match m.get_one::<String>("schema").map(|s| SchemaId::parse(s)) {
        Some(Ok(id)) => id,
        Some(Err(e)) => {
            eprintln!("smysl migrate: --schema: {e}");
            return ExitCode::Usage;
        }
        None => return ExitCode::Usage,
    };
    let Some(to) = m
        .get_one::<String>("to")
        .and_then(|s| s.parse::<u32>().ok())
    else {
        eprintln!("smysl migrate: --to takes a revision number");
        return ExitCode::Usage;
    };
    let agent_name = m
        .get_one::<String>("agent")
        .map(String::as_str)
        .unwrap_or("tool:smysl-migrate");
    let agent = match smysl::AgentId::new(agent_name) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl migrate: --agent {agent_name}: {e}");
            return ExitCode::Usage;
        }
    };
    let (store, _) = match load_memory(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl migrate: {e}");
            return ExitCode::Failure;
        }
    };
    let now = match m.get_one::<String>("clock") {
        Some(c) => {
            let (ms, n) = c.split_once('.').unwrap_or((c.as_str(), "0"));
            match (ms.parse::<u64>(), n.parse::<u32>()) {
                (Ok(ms), Ok(n)) => smysl::Hlc::new(ms, n, agent.clone()),
                _ => {
                    eprintln!(
                        "smysl migrate: --clock {c}: milliseconds, then optionally `.` and a \
                         counter"
                    );
                    return ExitCode::Usage;
                }
            }
        }
        // One past every attestation the store holds, so the migration sorts after what it
        // migrates and is a function of the store, not of when it ran.
        None => {
            let latest = store
                .iter()
                .filter_map(|r| match r {
                    Record::Attestation(a) => Some(a.ts.wall_ms),
                    _ => None,
                })
                .max()
                .unwrap_or(0);
            smysl::Hlc::new(latest + 1, 0, agent.clone())
        }
    };
    let migration = match migrate(&store, &MigrateOptions::new(id.clone(), to, agent, now)) {
        Ok(mg) => mg,
        Err(e) => {
            let d = e.diagnostic();
            eprintln!("smysl migrate: {}: {}", d.code.as_str(), d.message);
            return ExitCode::Failure;
        }
    };
    let staged = &migration.staged;
    for d in staged.report.iter() {
        eprintln!("smysl migrate: {}: {}", d.code.as_str(), d.message);
    }
    let summary = format!(
        "{} unit(s) of {id} re-derived under @{to}; {} already there; {} unmappable; {} \
         commitment(s) and {} dating(s) proposed",
        migration.pairs.len(),
        migration.current,
        migration.unmappable.len(),
        migration.proposed_commitments,
        migration.proposed_datings
    );
    if m.get_flag("dry-run") {
        println!("{summary} (dry run: nothing staged)");
        return ExitCode::Success;
    }
    if migration.pairs.is_empty() {
        println!("{summary}; nothing to stage");
        return ExitCode::Success;
    }
    if staged.has_errors() {
        eprintln!("smysl migrate: the batch does not check; nothing staged");
        return ExitCode::CheckErrors;
    }
    let root = root_beside(Some(&path));
    if let Err(e) = smysl::stage::write(&root, staged) {
        eprintln!("smysl migrate: {e}");
        return ExitCode::Failure;
    }
    println!(
        "{summary}; staged in {} — review, then `smysl merge --staged`, then `smysl relink`",
        root.join(smysl::stage::PATH).display()
    );
    if staged.weakened.is_empty() {
        ExitCode::Staged
    } else {
        ExitCode::StagedWithCorrections
    }
}

#[cfg(not(feature = "stage"))]
fn cmd_migrate(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl migrate: this build has no staging layer (build with --features stage)");
    ExitCode::Usage
}

/// `schema show`'s text for one id: revisions, each revision's field table, the lens steps
/// with their ops, the chain from each revision to current with its hash, and forks.
fn schema_show(set: &smysl::schema::SchemaSet, id: &SchemaId) -> String {
    use smysl::schema::{Cond, Fields};
    let mut out = String::new();
    let revs = set.revisions(id);
    let current = set.current(id).map(|r| r.version);
    out.push_str(&format!(
        "{id}\n  current: {}\n",
        match current {
            Some(v) => format!("@{v}"),
            None => "none (forked)".to_string(),
        }
    ));
    let cond = |c: &Option<Cond>| match c {
        Some(c) => format!(" when {} = {}", c.key, hvalue_text(&c.value)),
        None => String::new(),
    };
    for r in revs {
        out.push_str(&format!("  @{}", r.version));
        if !r.types.is_empty() {
            let t: Vec<&str> = r.types.iter().map(|t| t.as_str()).collect();
            out.push_str(&format!("  types {}", t.join(", ")));
        }
        if !r.relations.is_empty() {
            out.push_str(&format!("  relations {}", r.relations.len()));
        }
        out.push('\n');
        match &r.fields {
            Fields::Typed(table) => {
                for (k, f) in table {
                    let mut line = format!("      {k}  {}", f.ty.as_string());
                    if !f.values.is_empty() {
                        line.push_str(&format!(" [{}]", f.values.join(", ")));
                    }
                    if !f.literals.is_empty() {
                        line.push_str(&format!(" literals [{}]", f.literals.join(", ")));
                    }
                    if let Some(v) = f.required_from {
                        line.push_str(&format!("  required-from {v}{}", cond(&f.required_when)));
                    }
                    if let Some(v) = f.retired_in {
                        line.push_str(&format!("  retired-in {v}"));
                    }
                    out.push_str(&line);
                    out.push('\n');
                }
            }
            Fields::Opaque => out.push_str("      fields: opaque (SMY-W430)\n"),
            _ => out.push_str("      fields: none declared\n"),
        }
    }
    let steps = set.steps(id);
    if !steps.is_empty() {
        out.push_str("  steps:\n");
        for s in steps {
            let ops: Vec<String> = s
                .ops
                .iter()
                .map(|o| {
                    use smysl::schema::LensOp;
                    match o {
                        LensOp::RenameType { from, to } => format!("rename-type {from} -> {to}"),
                        LensOp::RenameRelation { from, to } => {
                            format!("rename-relation {} -> {}", from.as_str(), to.as_str())
                        }
                        LensOp::RenameKey { from, to } => format!("rename-key {from} -> {to}"),
                        LensOp::MapValue { key, map } => format!(
                            "map-value {key} {{{}}}",
                            map.iter()
                                .map(|(a, b)| format!("{a} -> {b}"))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        LensOp::SplitValue { key, from, to, add } => format!(
                            "split-value {key} {from} -> {to} + {}",
                            add.iter()
                                .map(|(k, v)| format!("{k} = {}", hvalue_text(v)))
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        LensOp::Require { key, when } => format!("require {key}{}", cond(when)),
                        LensOp::Default { key, value, when } => {
                            format!("default {key} = {}{}", hvalue_text(value), cond(when))
                        }
                        LensOp::Retire { key } => format!("retire {key}"),
                        other => format!("{} (unknown to this build, SMY-W469)", other.tag()),
                    }
                })
                .collect();
            out.push_str(&format!("    {}->{}  {}\n", s.from, s.to, ops.join("; ")));
        }
    }
    if let Some(cur) = current {
        out.push_str("  chains to current:\n");
        let mut seen = std::collections::BTreeSet::new();
        for r in revs {
            if !seen.insert(r.version) {
                continue;
            }
            match set.chain(id, r.version, cur) {
                Ok(c) => {
                    let hex: String = c.hash.iter().map(|b| format!("{b:02x}")).collect();
                    out.push_str(&format!("    @{}->@{cur}  {hex}\n", r.version));
                }
                Err(e) => out.push_str(&format!(
                    "    @{}->@{cur}  {}: {e}\n",
                    r.version,
                    e.code().as_str()
                )),
            }
        }
    }
    let forks: Vec<String> = set
        .forks()
        .into_iter()
        .filter(|(i, _)| i == id)
        .map(|(_, v)| format!("@{v}"))
        .collect();
    out.push_str(&format!(
        "  forks: {}\n",
        if forks.is_empty() {
            "none".to_string()
        } else {
            forks.join(", ")
        }
    ));
    out
}

/// A payload value as `schema show` writes it.
fn hvalue_text(v: &smysl::surface::HValue) -> String {
    use smysl::surface::HValue;
    match v {
        HValue::Str(s) => s.clone(),
        HValue::Int(i) => i.to_string(),
        HValue::Bool(b) => b.to_string(),
        HValue::Null => "null".to_string(),
        other => format!("{other:?}"),
    }
}

#[cfg(feature = "text")]
fn cmd_lib(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let action = m.get_one::<String>("action").expect("required").as_str();
    if action != "export" && global.get_one::<String>("format").is_some() {
        eprintln!(
            "smysl lib {action}: --format is not honoured here - lib {action} does not write a \
             document. Use --json if you are parsing its output."
        );
        return ExitCode::Usage;
    }
    if action != "export" {
        warn_output_is_a_report(global, &format!("lib {action}"));
    }
    match action {
        "import" => lib_import(m, global),
        "status" => lib_status(m, global),
        "accept" => lib_accept(m, global),
        "discard" => lib_discard(m, global),
        "export" => lib_export(m, global),
        "verify" if m.get_flag("primary-shards") => lib_verify_primary(m, global),
        "verify" => lib_verify(m, global),
        "migrate" => lib_migrate(m, global),
        other => {
            eprintln!(
                "smysl lib: `{other}` is not an action; it is import, status, accept, discard, \
                 export, verify or migrate"
            );
            ExitCode::Usage
        }
    }
}

#[cfg(not(feature = "text"))]
fn cmd_lib(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!(
        "smysl lib: import and export work on a library, and this build has no `text` feature"
    );
    ExitCode::Usage
}

#[cfg(feature = "text")]
fn lib_targets(m: &ArgMatches) -> Vec<String> {
    m.get_many::<String>("targets")
        .map(|v| v.cloned().collect())
        .unwrap_or_default()
}

/// The one target `accept`, `discard` and `verify` take.
#[cfg(feature = "text")]
fn lib_one_target(m: &ArgMatches, action: &str, what: &str) -> Result<String, ExitCode> {
    match lib_targets(m).as_slice() {
        [one] => Ok(one.clone()),
        [] => {
            eprintln!("smysl lib {action}: no {what} given");
            Err(ExitCode::Usage)
        }
        more => {
            eprintln!("smysl lib {action}: one {what}, not {}", more.len());
            Err(ExitCode::Usage)
        }
    }
}

/// What a refusal from `smysl-lib` exits with: a name that does not fit or names nothing is
/// the caller's, a contention is `Contentions`, and the rest is the filesystem's or the
/// library's.
#[cfg(feature = "text")]
fn lib_error_code(e: &smysl::lib::LibError) -> ExitCode {
    use smysl::lib::LibError as E;
    match e {
        E::BadName(_)
        | E::NameTaken(_)
        | E::NoImport(_)
        | E::NotStaged(_)
        | E::NotInStore(_)
        | E::AsOf(_)
        | E::Query(_) => ExitCode::Usage,
        E::Pack(_) => ExitCode::PackInfeasible,
        E::Contentions(_) => ExitCode::Contentions,
        E::TargetRevision(smysl::Code::E470, _) => ExitCode::Usage,
        _ => ExitCode::Failure,
    }
}

/// Run `f` on the library `-s` names, under its lock, as `text add` does.
///
/// `create` for `import`, which may be a library's first write, as `text add` may be; `accept`
/// and `discard` name an import, and a library that does not exist has none.
#[cfg(feature = "text")]
fn with_library<T>(
    m: &ArgMatches,
    global: &ArgMatches,
    action: &str,
    create: bool,
    f: impl FnOnce(&mut smysl::text::library::Library) -> T,
) -> Result<T, ExitCode> {
    use smysl::text::library::Library;
    let cmd = format!("lib {action}");
    let root = library_root(m, global, &cmd)?;
    if !create && !Library::is_library(&root) {
        eprintln!(
            "smysl {cmd}: {} is not a library (it has no `{}` marker)",
            root.display(),
            smysl::text::library::MARKER
        );
        return Err(ExitCode::Usage);
    }
    let opened = if create {
        Library::create(&root)
    } else {
        Library::open(&root)
    };
    let mut library = opened.map_err(|e| {
        eprintln!("smysl {cmd}: {}: {e}", root.display());
        ExitCode::Failure
    })?;
    let shards = library.shards();
    let _locks = smysl::text::lock::acquire_all(&root, &shards, &cmd).map_err(|e| {
        eprintln!("smysl {cmd}: {e}");
        ExitCode::Failure
    })?;
    Ok(f(&mut library))
}

/// `lib migrate --primary <shard>` / `--index <shard>` (SMYSL-2.8 §3.9, TX-P13d): a shard's
/// records into the library's unit store, after a round trip that must reproduce its record-set
/// digest (exit 9, `SMY-E505`, when it does not), or back out into its log.
#[cfg(feature = "text")]
fn lib_migrate(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let named = |id: &str| -> Vec<String> {
        m.get_many::<String>(id)
            .map(|v| v.cloned().collect())
            .unwrap_or_default()
    };
    let (primary, index) = (named("primary"), named("index"));
    if primary.is_empty() && index.is_empty() {
        eprintln!("smysl lib migrate: name a shard with --primary or --index");
        return ExitCode::Usage;
    }
    #[cfg(not(feature = "store-redb"))]
    {
        let _ = global;
        eprintln!(
            "smysl lib migrate: a primary shard's records live in the unit store, and this build \
             has no `store-redb` feature"
        );
        ExitCode::Usage
    }
    #[cfg(feature = "store-redb")]
    {
        let outcome = with_library(m, global, "migrate", false, |lib| {
            let digests = if primary.is_empty() {
                Vec::new()
            } else {
                lib.migrate_primary_all(&primary)?
            };
            for (shard, digest) in primary.iter().zip(digests) {
                println!(
                    "lib migrate: {shard} is primary; its record-set digest is {}, and its log is \
                     set aside as {}",
                    hex32(&digest),
                    smysl::text::shards::set_aside(shard).display()
                );
            }
            for shard in &index {
                lib.migrate_index(shard)?;
                println!(
                    "lib migrate: {shard} is an index shard again; its log is {}",
                    smysl::text::shards::log_path(shard).display()
                );
            }
            Ok::<(), smysl::text::LibError>(())
        });
        match outcome {
            Ok(Ok(())) => ExitCode::Success,
            Ok(Err(e)) => {
                eprintln!("smysl lib migrate: {e}");
                e.code_exit()
            }
            Err(code) => code,
        }
    }
}

/// `lib verify --primary-shards`: each primary shard's export against what the unit store's
/// tables hold (`SMY-E505`, exit 9, on a difference).
#[cfg(feature = "text")]
fn lib_verify_primary(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    #[cfg(not(feature = "store-redb"))]
    {
        let _ = (m, global);
        eprintln!(
            "smysl lib verify: a primary shard's records live in the unit store, and this build \
             has no `store-redb` feature"
        );
        ExitCode::Usage
    }
    #[cfg(feature = "store-redb")]
    {
        let outcome = with_library(m, global, "verify", false, |lib| {
            let shards = lib.primary_shards();
            let mut worst = ExitCode::Success;
            if shards.is_empty() {
                println!("lib verify: this library has no primary shard");
            }
            for shard in shards {
                match lib.primary_digest(&shard) {
                    Ok(d) => println!("lib verify: {shard}: holds ({})", hex32(&d)),
                    Err(e) => {
                        eprintln!("lib verify: {shard}: {e}");
                        worst = worst.max(e.code_exit());
                    }
                }
            }
            worst
        });
        outcome.unwrap_or_else(|code| code)
    }
}

#[cfg(feature = "text")]
fn hex32(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// `lib import <inputs…>`. Exits 1 when an input did not decode or a collection did not match
/// its index, 3 under `--strict` when anything was quarantined, 10 when anything was staged,
/// and 0 otherwise.
#[cfg(feature = "text")]
fn lib_import(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::lib::{ImportOptions, Input, INDEX_FILE};
    let raw = lib_targets(m);
    if raw.is_empty() {
        eprintln!("smysl lib import: no input given - files, directories, or an export index");
        return ExitCode::Usage;
    }
    let mut inputs = Vec::new();
    for t in &raw {
        let p = std::path::Path::new(t);
        inputs.push(if t == "-" {
            let mut bytes = Vec::new();
            if let Err(e) = std::io::stdin().read_to_end(&mut bytes) {
                eprintln!("smysl lib import: stdin: {e}");
                return ExitCode::Failure;
            }
            Input::Bytes {
                name: "stdin".to_string(),
                bytes,
            }
        } else if p.is_dir() && p.join(INDEX_FILE).is_file() {
            // A directory an export wrote is a collection, and is checked against its index
            // before anything in it is read; any other directory is its files.
            Input::Index(p.join(INDEX_FILE))
        } else if p.is_dir() {
            Input::Dir(p.to_path_buf())
        } else if p.file_name().is_some_and(|n| n == INDEX_FILE) {
            Input::Index(p.to_path_buf())
        } else {
            Input::File(p.to_path_buf())
        });
    }
    let mut opts = ImportOptions::new()
        .accept(m.get_flag("accept"))
        .keep_original(m.get_flag("keep-original"))
        .join(!m.get_flag("no-join"));
    if let Some(n) = m.get_one::<String>("name") {
        opts = opts.name(n.clone());
    }
    if let Some(l) = m.get_one::<String>("licence") {
        opts = opts.licence(l.clone());
    }
    if let Some(o) = m.get_one::<String>("origin") {
        opts = opts.origin(o.clone());
    }
    let outcome = match with_library(m, global, "import", true, |lib| {
        smysl::lib::import(lib, &inputs, &opts)
    }) {
        Ok(o) => o,
        Err(code) => return code,
    };
    let r = match outcome {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl lib import: {e}");
            return lib_error_code(&e);
        }
    };
    for d in r.report.iter() {
        eprintln!("import:{}: {d}", r.name);
    }
    for c in &r.contentions {
        eprintln!(
            "import:{}: contention {} over {} ({} positions, {})",
            r.name,
            c.id,
            c.over,
            c.positions.len(),
            c.detected.kind
        );
    }
    let sum = |f: fn(&smysl::lib::FileOutcome) -> usize| r.files.iter().map(f).sum::<usize>();
    let (staged, quarantined) = (sum(|f| f.staged), sum(|f| f.quarantined));
    if global.get_flag("json") {
        let files: Vec<String> = r
            .files
            .iter()
            .map(|f| {
                format!(
                    "{{\"file\":{},\"mid\":{},\"original\":\"{}\",\"records\":\"{}\",\
                     \"added\":{},\"duplicates\":{},\"accepted\":{},\"staged\":{},\
                     \"waiting\":{},\"quarantined\":{},\"text\":{},\"redacted\":{},\
                     \"already_imported\":{},\"rejected\":{}}}",
                    smysl::json_escape(&f.file),
                    f.mid
                        .map(|m| smysl::json_escape(&m.canonical()))
                        .unwrap_or_else(|| "null".to_string()),
                    hex32(&f.original),
                    hex32(&f.rsd),
                    f.added,
                    f.duplicates,
                    f.accepted,
                    f.staged,
                    f.waiting,
                    f.quarantined,
                    f.text,
                    f.redacted,
                    f.already_imported,
                    f.rejected
                )
            })
            .collect();
        println!(
            "{{\"name\":{},\"files\":[{}],\"contentions\":{},\"joins\":{}}}",
            smysl::json_escape(&r.name),
            files.join(","),
            r.contentions.len(),
            r.joins
        );
    } else if !global.get_flag("quiet") {
        println!("import  {}", r.name);
        for f in &r.files {
            let what = if f.rejected {
                "not decoded (SMY-E493); nothing from it was used".to_string()
            } else if f.already_imported {
                "already imported".to_string()
            } else {
                format!(
                    "{} new, {} already held: {} accepted, {} staged, {} waiting, {} quarantined{}",
                    f.added,
                    f.duplicates,
                    f.accepted,
                    f.staged,
                    f.waiting,
                    f.quarantined,
                    if f.redacted > 0 {
                        format!("; {} redacted text record(s) dropped", f.redacted)
                    } else {
                        String::new()
                    }
                )
            };
            println!("  {}  {what}", f.file);
        }
        if r.joins > 0 {
            println!(
                "join    {} same-as edge(s) staged as join:{}; `lib accept join:{}` merges them",
                r.joins, r.name, r.name
            );
        }
    }
    let bad_input = r.files.iter().any(|f| f.rejected)
        || r.report
            .iter()
            .any(|d| matches!(d.code, smysl::Code::E493 | smysl::Code::E494));
    if bad_input {
        ExitCode::Failure
    } else if global.get_flag("strict") && quarantined > 0 {
        ExitCode::CheckErrors
    } else if staged > 0 {
        ExitCode::Staged
    } else {
        ExitCode::Success
    }
}

/// `lib status [--import NAME]`: every import, or one, with its state and counts.
#[cfg(feature = "text")]
fn lib_status(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let root = match library_root(m, global, "lib status") {
        Ok(r) => r,
        Err(code) => return code,
    };
    if !smysl::text::library::Library::is_library(&root) {
        eprintln!(
            "smysl lib status: {} is not a library (it has no `{}` marker)",
            root.display(),
            smysl::text::library::MARKER
        );
        return ExitCode::Usage;
    }
    let batches = match smysl::lib::status(&root, m.get_one::<String>("import").map(String::as_str))
    {
        Ok(b) => b,
        Err(e) => {
            eprintln!("smysl lib status: {e}");
            return lib_error_code(&e);
        }
    };
    if global.get_flag("json") {
        let rows: Vec<String> = batches
            .iter()
            .map(|b| {
                format!(
                    "{{\"name\":{},\"state\":\"{}\",\"manifests\":[{}],\"accepted\":{},\
                     \"staged\":{},\"waiting\":{},\"quarantined\":{},\"tool\":{}}}",
                    smysl::json_escape(&b.name),
                    b.state.as_str(),
                    b.mids
                        .iter()
                        .map(|m| smysl::json_escape(&m.canonical()))
                        .collect::<Vec<_>>()
                        .join(","),
                    b.accepted,
                    b.staged,
                    b.waiting,
                    b.quarantined,
                    smysl::json_escape(&b.tool)
                )
            })
            .collect();
        println!("[{}]", rows.join(","));
        return ExitCode::Success;
    }
    for b in &batches {
        println!(
            "{:<24} {:<11} {} accepted, {} staged, {} waiting, {} quarantined ({} file(s))",
            b.name,
            b.state.as_str(),
            b.accepted,
            b.staged,
            b.waiting,
            b.quarantined,
            b.mids.len()
        );
    }
    ExitCode::Success
}

/// `lib accept <name>`: merge a staged import. Exits 5 under `--fail-on-contention` when the
/// import would give rise to a contention, and merges nothing.
#[cfg(feature = "text")]
fn lib_accept(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let name = match lib_one_target(m, "accept", "import name") {
        Ok(n) => n,
        Err(code) => return code,
    };
    let fail = m.get_flag("fail-on-contention");
    let outcome = match with_library(m, global, "accept", false, |lib| {
        smysl::lib::accept(lib, &name, fail)
    }) {
        Ok(o) => o,
        Err(code) => return code,
    };
    match outcome {
        Ok(r) => {
            for d in r.report.iter() {
                eprintln!("import:{name}: {d}");
            }
            for c in &r.contentions {
                eprintln!(
                    "import:{name}: contention {} over {} ({} positions, {})",
                    c.id,
                    c.over,
                    c.positions.len(),
                    c.detected.kind
                );
            }
            if !global.get_flag("quiet") {
                println!("accepted  {name}");
            }
            ExitCode::Success
        }
        Err(e) => {
            eprintln!("smysl lib accept: {e}");
            lib_error_code(&e)
        }
    }
}

/// `lib discard <name>`: drop a staged import's staged and waiting records. Its manifests and
/// its quarantine stay.
#[cfg(feature = "text")]
fn lib_discard(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let name = match lib_one_target(m, "discard", "import name") {
        Ok(n) => n,
        Err(code) => return code,
    };
    let outcome = match with_library(m, global, "discard", false, |lib| {
        smysl::lib::discard(lib, &name)
    }) {
        Ok(o) => o,
        Err(code) => return code,
    };
    match outcome {
        Ok(n) => {
            if !global.get_flag("quiet") {
                println!("discarded  {name}: {n} record(s)");
            }
            ExitCode::Success
        }
        Err(e) => {
            eprintln!("smysl lib discard: {e}");
            lib_error_code(&e)
        }
    }
}

/// `--target-revision ID@N`, with `--agent` and `--clock` (SMYSL-2.7 §3.11, OQ-63); the last
/// two mean nothing without the first, and are refused alone.
#[cfg(feature = "text")]
fn lib_target_revision(m: &ArgMatches) -> Result<Option<smysl::lib::TargetRevision>, ExitCode> {
    let Some(spec) = m.get_one::<String>("target-revision") else {
        if m.get_one::<String>("agent").is_some() || m.get_one::<String>("clock").is_some() {
            eprintln!(
                "smysl lib export: --agent and --clock are the attestations --target-revision adds"
            );
            return Err(ExitCode::Usage);
        }
        return Ok(None);
    };
    let mut t = match smysl::lib::TargetRevision::parse(spec) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("smysl lib export: {e}");
            return Err(ExitCode::Usage);
        }
    };
    if let Some(a) = m.get_one::<String>("agent") {
        match AgentId::new(a) {
            Ok(a) => t = t.agent(a),
            Err(e) => {
                eprintln!("smysl lib export: --agent {a}: {e}");
                return Err(ExitCode::Usage);
            }
        }
    }
    if let Some(c) = m.get_one::<String>("clock") {
        let (ms, n) = c.split_once('.').unwrap_or((c.as_str(), "0"));
        match (ms.parse::<u64>(), n.parse::<u32>()) {
            (Ok(ms), Ok(n)) => t = t.clock(ms, n),
            _ => {
                eprintln!("smysl lib export: --clock {c}: milliseconds, then optionally `.` and a counter");
                return Err(ExitCode::Usage);
            }
        }
    }
    Ok(Some(t))
}

/// `lib export --scope … -o DIR`: the store `-s` names, as a collection in `DIR`.
///
/// Verified in memory before anything is written, unless `--no-verify`: a collection whose P1
/// or P3 fails is not written at all and exits 9. That is stricter than draft 3 §16.8's index
/// marked incomplete, which is for what an export cannot spell (`SMY-E424`, surface); a cbor
/// collection that does not verify is this build's defect, and a directory holding one would
/// be mistaken for an export.
#[cfg(feature = "text")]
fn lib_export(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::lib::{
        export_library, export_store, snapshot, verify_collection, write_collection, Closure,
        ExportOptions, Layout, Selection, UnknownRecords, With,
    };
    use smysl::text::library::Library;
    let path = match store_path(m, global, "lib export") {
        Ok(p) => p,
        Err(code) => return code,
    };
    let Some(scope) = m.get_many::<String>("scope") else {
        eprintln!("smysl lib export: --scope is required: `all`, or the units to export");
        return ExitCode::Usage;
    };
    let scope: Vec<&String> = scope.collect();
    let Some(out) = global.get_one::<String>("output") else {
        eprintln!("smysl lib export: -o DIR is required - a collection is a directory of files");
        return ExitCode::Usage;
    };
    let dir = std::path::Path::new(out);
    if dir.is_file() {
        eprintln!("smysl lib export: {out} is a file; a collection is written into a directory");
        return ExitCode::Usage;
    }
    let occupied = std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_some());
    if occupied && !m.get_flag("force") {
        eprintln!(
            "smysl lib export: {out} is not empty; pass --force to write into it (files the \
             new index does not name are left as they are)"
        );
        return ExitCode::Usage;
    }
    let with = With::new()
        .views(m.get_flag("with-views"))
        .packs(m.get_flag("with-packs"))
        .text(m.get_flag("with-text"))
        .provenance(m.get_flag("with-provenance"))
        .staged(m.get_flag("with-staged"))
        .quarantine(m.get_flag("with-quarantine"));
    // A library is exported from its snapshot, with its object store for `--with-text` and its
    // tables and batches for the other three. A plain store has none of those, so asking for
    // them of one is refused rather than answered with nothing.
    let library = if Library::is_library(std::path::Path::new(&path)) {
        match Library::open(std::path::Path::new(&path)) {
            Ok(l) => Some(l),
            Err(e) => {
                eprintln!("smysl lib export: {path}: {e}");
                return ExitCode::Failure;
            }
        }
    } else {
        None
    };
    if library.is_none() && (with.text || with.provenance || with.staged || with.quarantine) {
        eprintln!(
            "smysl lib export: --with-text, --with-provenance, --with-staged and \
             --with-quarantine read a library, and {path} is not one"
        );
        return ExitCode::Usage;
    }
    let store = match &library {
        Some(l) => match snapshot(l, &with) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("smysl lib export: {e}");
                return lib_error_code(&e);
            }
        },
        None => match load_memory(&path) {
            Ok((s, _)) => s,
            Err(e) => {
                eprintln!("smysl lib export: {e}");
                return ExitCode::Failure;
            }
        },
    };
    let query_text = match scope.as_slice() {
        [one] if one.contains(char::is_whitespace) => Some(one.as_str()),
        _ => None,
    };
    let selection = if let Some(text) = query_text {
        match lib_query_scope(&path, library.as_ref(), &store, text) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("smysl lib export: {e}");
                return lib_error_code(&e);
            }
        }
    } else if scope.iter().any(|s| s.as_str() == "all") {
        if scope.len() > 1 {
            eprintln!(
                "smysl lib export: --scope all selects everything, and takes nothing beside it"
            );
            return ExitCode::Usage;
        }
        Selection::all(&store)
    } else {
        let mut uids = Vec::new();
        for s in &scope {
            match resolve(&store, s) {
                Ok(u) => uids.push(u),
                Err(e) => {
                    eprintln!("smysl lib export: --scope {s}: {e}");
                    return e.code;
                }
            }
        }
        Selection::of(uids)
    };
    let layout = m.get_one::<String>("layout").expect("defaulted");
    let closure = m.get_one::<String>("closure").expect("defaulted");
    let selection = match (layout.as_str(), m.get_one::<String>("group-by")) {
        ("by-query", Some(field)) => match smysl::lib::scope::group(&store, selection, field) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("smysl lib export: --group-by {field}: {e}");
                return ExitCode::Usage;
            }
        },
        ("by-query", None) => {
            eprintln!("smysl lib export: --layout by-query files units by --group-by FIELD");
            return ExitCode::Usage;
        }
        (_, Some(_)) => {
            eprintln!("smysl lib export: --group-by is the field of `--layout by-query`");
            return ExitCode::Usage;
        }
        (_, None) => selection,
    };
    let mut opts = ExportOptions::new()
        .layout(Layout::parse(layout).expect("the parser's values"))
        .closure(Closure::parse(closure).expect("the parser's values"))
        .with(with);
    if m.get_one::<String>("unknown").map(String::as_str) == Some("drop") {
        opts = opts.unknown(UnknownRecords::Drop);
    }
    if let Some(raw) = m.get_one::<String>("edges") {
        match edge_set(&store, raw, "lib export") {
            Ok(e) => opts = opts.edges(e),
            Err(code) => return code,
        }
    }
    if let Some(raw) = m.get_one::<String>("window") {
        if layout != "by-time" {
            eprintln!("smysl lib export: --window is the width of a `by-time` window");
            return ExitCode::Usage;
        }
        match window_ms(raw) {
            Some(ms) => opts = opts.window(ms),
            None => {
                eprintln!(
                    "smysl lib export: --window {raw}: a positive whole number of s, m, h, d or w"
                );
                return ExitCode::Usage;
            }
        }
    }
    if let Some(edtf) = m.get_one::<String>("as-of") {
        opts = opts.as_of(edtf.clone());
    }
    match lib_target_revision(m) {
        Ok(Some(t)) => opts = opts.target_revision(t),
        Ok(None) => {}
        Err(code) => return code,
    }
    let budget = m.get_one::<u64>("budget").copied();
    match (
        global.get_one::<String>("format").map(String::as_str),
        budget,
    ) {
        (Some("surface"), None) => opts = opts.format(smysl::lib::Format::Surface),
        (Some("pack"), Some(n)) => opts = opts.format(smysl::lib::Format::Pack { budget: n }),
        (Some("pack"), None) => {
            eprintln!("smysl lib export: --format pack packs each file to --budget N tokens");
            return ExitCode::Usage;
        }
        (_, Some(_)) => {
            eprintln!("smysl lib export: --budget is the budget of `--format pack`");
            return ExitCode::Usage;
        }
        _ => {}
    }
    let exported = match &library {
        Some(l) => export_library(l, &store, &selection, &opts),
        None => export_store(&store, &selection, &opts),
    };
    let c = match exported {
        Ok(c) => c,
        Err(e) => {
            eprintln!("smysl lib export: {e}");
            return lib_error_code(&e);
        }
    };
    for d in c.report.iter() {
        eprintln!("lib export: {d}");
    }
    let verified = !m.get_flag("no-verify");
    if verified {
        let v = verify_collection(&c);
        if !v.holds() {
            eprintln!("smysl lib export: the collection does not verify, and nothing was written");
            print_verify(&v, true);
            return ExitCode::HashVerification;
        }
    }
    if let Err(e) = write_collection(&c, dir) {
        eprintln!("smysl lib export: {e}");
        return lib_error_code(&e);
    }
    let records: usize = c.files.iter().map(|f| f.records).sum();
    if global.get_flag("json") {
        let files: Vec<String> = c
            .files
            .iter()
            .map(|f| {
                format!(
                    "{{\"path\":{},\"records\":{},\"rsd\":\"{}\"}}",
                    smysl::json_escape(&f.path),
                    f.records,
                    hex32(&f.rsd)
                )
            })
            .collect();
        println!(
            "{{\"files\":[{}],\"records\":{records},\"rsd\":\"{}\",\"duplication\":[{},{}],\
             \"verified\":{verified}}}",
            files.join(","),
            hex32(&c.rsd()),
            c.duplication().0,
            c.duplication().1
        );
    } else if !global.get_flag("quiet") {
        println!(
            "{} file(s), {records} record(s), rsd {}{}",
            c.files.len(),
            hex32(&c.rsd()),
            if verified {
                "; P1 and P3 hold"
            } else {
                "; not verified"
            }
        );
        for f in &c.files {
            println!("  {}  {}", f.path, f.records);
        }
    }
    // A surface file that does not read back as its selection (`SMY-E424`): written, with the
    // index saying it is incomplete, and exit 3 (§4.4).
    if !c.index.complete {
        return ExitCode::CheckErrors;
    }
    match strict_threshold(global, &c.report) {
        Some(code) => code,
        None => ExitCode::Success,
    }
}

/// An `sq/1` scope over what `lib export` reads (TX-P11 step 8).
///
/// Over a library, through its own ports, as `q --library` runs one; and `from import:<name>`
/// alone, for an import of a collection `lib export` wrote, is the union of the roots of that
/// collection's views (SMYSL-2.7 §3.9) rather than every unit that arrived. Over a plain store,
/// through `StoreScope` named by the file's stem, as `q` names it.
#[cfg(feature = "text")]
fn lib_query_scope(
    path: &str,
    library: Option<&smysl::text::library::Library>,
    store: &Store,
    text: &str,
) -> Result<smysl::lib::Selection, smysl::lib::LibError> {
    use smysl::query::{Env, StoreScope};
    let name = std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| path.to_string());
    match library {
        Some(lib) => {
            if let Some(import) = smysl::lib::scope::names_an_import(text) {
                if let Some(sel) = smysl::lib::scope::import_scope(lib, import)? {
                    return Ok(sel);
                }
            }
            let scope = smysl::text::scope::LibraryScope::new(name, lib);
            let env = Env::new(store, &scope)
                .with_times(&scope)
                .with_entities(&scope);
            smysl::lib::scope::query(&env, text)
        }
        None => {
            let scope = StoreScope::new(name, store);
            let env = Env::new(store, &scope).with_entities(&scope);
            smysl::lib::scope::query(&env, text)
        }
    }
}

/// A `--window` width in milliseconds: a positive whole number and one unit of s, m, h, d or w.
#[cfg(feature = "text")]
fn window_ms(raw: &str) -> Option<u64> {
    let (n, unit) = raw.split_at(raw.find(|c: char| !c.is_ascii_digit())?);
    let n: u64 = n.parse().ok().filter(|n| *n > 0)?;
    let ms = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        "w" => 604_800_000,
        _ => return None,
    };
    n.checked_mul(ms)
}

/// `lib verify <index|dir>`: every file against the index, then P1 and P3. Exits 9 when a file
/// or a digest does not match.
#[cfg(feature = "text")]
fn lib_verify(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    if let Some(files) = m.get_many::<String>("p2") {
        let files: Vec<String> = files.cloned().collect();
        if !lib_targets(m).is_empty() {
            eprintln!("smysl lib verify: --p2 checks files, not an index; give one or the other");
            return ExitCode::Usage;
        }
        return lib_verify_p2(&files, m.get_flag("keep-scratch"), global);
    }
    let target = match lib_one_target(m, "verify", "index") {
        Ok(t) => t,
        Err(code) => return code,
    };
    let mut p = std::path::PathBuf::from(&target);
    if p.is_dir() {
        p = p.join(smysl::lib::INDEX_FILE);
    }
    let v = match smysl::lib::verify(&p) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl lib verify: {e}");
            return lib_error_code(&e);
        }
    };
    for d in v.report.iter() {
        eprintln!("lib verify: {d}");
    }
    if global.get_flag("json") {
        let prop = |p: &smysl::lib::Property| match p {
            smysl::lib::Property::Holds => "\"holds\"",
            smysl::lib::Property::NotApplicable(_) => "\"not-applicable\"",
            _ => "\"fails\"",
        };
        let files: Vec<String> = v
            .files
            .iter()
            .map(|f| {
                format!(
                    "{{\"path\":{},\"problem\":{}}}",
                    smysl::json_escape(&f.path),
                    f.problem
                        .as_deref()
                        .map(smysl::json_escape)
                        .unwrap_or_else(|| "null".to_string())
                )
            })
            .collect();
        println!(
            "{{\"holds\":{},\"files\":[{}],\"p1\":{},\"p3\":{}}}",
            v.holds(),
            files.join(","),
            prop(&v.p1),
            prop(&v.p3)
        );
    } else if !global.get_flag("quiet") || !v.holds() {
        print_verify(&v, false);
    }
    v.exit_code()
}

/// `lib verify --p2 FILE…` (SMYSL-2.7 §3.9): each file imported with `--accept` into a scratch
/// library of its own, and exported again from it; P2 holds when everything the import accepted
/// comes back. Exits 9 when it does not hold for a file, as any failed round trip does.
#[cfg(feature = "text")]
fn lib_verify_p2(files: &[String], keep: bool, global: &ArgMatches) -> ExitCode {
    use smysl::lib::{import, p2, ImportOptions, Input, Property};
    let mut worst = ExitCode::Success;
    for (i, file) in files.iter().enumerate() {
        let scratch = std::env::temp_dir().join(format!("smysl-p2-{}-{i}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let mut lib = match smysl::text::library::Library::create(&scratch) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("smysl lib verify --p2: a scratch library: {e}");
                return ExitCode::Failure;
            }
        };
        let outcome = import(
            &mut lib,
            &[Input::File(std::path::PathBuf::from(file))],
            &ImportOptions::new().accept(true).join(false),
        )
        .and_then(|r| p2(&lib, &r.name).map(|p| (r, p)));
        let line = match outcome {
            Ok((r, _)) if r.files.iter().any(|f| f.rejected) => {
                worst = ExitCode::Failure;
                format!("{file}: does not decode (SMY-E493)")
            }
            Ok((_, Property::Holds)) => format!("{file}: P2 holds"),
            Ok((_, Property::Fails { missing, .. })) => {
                worst = ExitCode::HashVerification;
                format!(
                    "{file}: P2 fails: {} accepted record(s) do not come back",
                    missing.len()
                )
            }
            Ok((_, other)) => format!("{file}: P2 {other:?}"),
            Err(e) => {
                worst = lib_error_code(&e);
                format!("{file}: {e}")
            }
        };
        if !global.get_flag("quiet") || worst != ExitCode::Success {
            println!("{line}");
        }
        if keep {
            eprintln!(
                "smysl lib verify --p2: {file}: the scratch library is kept at {}",
                scratch.display()
            );
        } else {
            let _ = std::fs::remove_dir_all(&scratch);
        }
    }
    worst
}

/// A verify report, one line per file that does not match and one per property.
#[cfg(feature = "text")]
fn print_verify(v: &smysl::lib::VerifyReport, to_stderr: bool) {
    use smysl::lib::Property;
    let mut lines = Vec::new();
    let ok = v.files.iter().filter(|f| f.problem.is_none()).count();
    lines.push(format!("{ok} of {} file(s) match the index", v.files.len()));
    for f in &v.files {
        if let Some(p) = &f.problem {
            lines.push(format!("  {}: {p}", f.path));
        }
    }
    for (name, p) in [("P1", &v.p1), ("P3", &v.p3)] {
        lines.push(match p {
            Property::Holds => format!("{name} holds"),
            Property::NotApplicable(why) => format!("{name} not checked: {why}"),
            Property::Fails {
                missing,
                extra,
                first_diff,
            } => {
                let mut s = format!(
                    "{name} fails: {} record(s) missing, {} extra",
                    missing.len(),
                    extra.len()
                );
                if let Some((file, at)) = first_diff {
                    s.push_str(&format!("; {file} first differs at byte {at}"));
                }
                s
            }
            _ => format!("{name}: an outcome this build does not know"),
        });
    }
    for l in lines {
        if to_stderr {
            eprintln!("{l}");
        } else {
            println!("{l}");
        }
    }
}

/// `ask --answer`: the model's turn, after the dossier is in the store.
///
/// The members are the thread's steps less its gaps — a gap's unit is a saved query, whose gist
/// is query text and not anything the corpus said.
#[cfg(all(feature = "text", feature = "ingest", feature = "providers"))]
fn ask_answer(
    m: &ArgMatches,
    global: &ArgMatches,
    path: &str,
    corpus: Corpus,
    scratch: Store,
    dossier: &smysl::query::Dossier,
) -> ExitCode {
    let members: Vec<smysl::answer::Member> = dossier
        .thread
        .steps
        .iter()
        .filter(|s| s.role != Role::Gap)
        .filter_map(|s| {
            scratch.get(&s.unit).map(|u| smysl::answer::Member {
                role: s.role.to_string(),
                uid: s.unit,
                status: u.core.status,
                gist: u.core.gist.clone(),
            })
        })
        .collect();
    let registry = match load_registry(global) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl ask: {}", e.message);
            return e.code;
        }
    };
    let opts = match model_options(m, "ask") {
        Ok(o) => o,
        Err(code) => return code,
    };
    let report = match smysl::Ingestor::new(&registry, opts).answer(&scratch, &members) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl ask: {e}");
            return ExitCode::Failure;
        }
    };
    for d in &report.diagnostics {
        eprintln!("  {}", d.code.as_str());
        eprintln!("    {}", d.message);
    }
    eprintln!(
        "smysl ask: answer: {} proposed, {} dropped for grounding outside the dossier (SMY-W478), \
         {} lowered by rule M (SMY-W036)",
        report.proposed, report.dropped, report.lowered
    );
    let Some(staged) = report.staged else {
        eprintln!("smysl ask: nothing to stage");
        return ExitCode::Success;
    };
    let corpus = match corpus {
        Corpus::Store(_, labels) => Corpus::Store(Box::new(scratch), labels),
        library => library,
    };
    stage_or_commit("ask", path, corpus, &staged, m.get_flag("yes"))
}

#[cfg(all(feature = "text", not(all(feature = "ingest", feature = "providers"))))]
fn ask_answer(
    _m: &ArgMatches,
    _global: &ArgMatches,
    _path: &str,
    _corpus: Corpus,
    _scratch: Store,
    _dossier: &smysl::query::Dossier,
) -> ExitCode {
    eprintln!(
        "smysl ask: --answer asks a model, and this build has no provider (build with --features \
         ingest,providers); the dossier was saved"
    );
    ExitCode::Usage
}

/// What `q` was asked to do, once the arguments are settled.
enum Job {
    /// Boxed because a `Query` is a tree and a uid is a string, and an enum the size of its
    /// largest variant would carry the tree's bytes for every rerun.
    Run {
        query: Box<smysl::query::Query>,
        as_view: bool,
    },
    Rerun {
        uid: String,
        current: bool,
    },
}

/// Whether the store already declares `x.query/v1`, so a save declares it once and not with
/// every saved query (A-11; the same rule `ingest` applies to `x.text/v1`).
fn declares_query(store: &(impl StoreRead + ?Sized)) -> bool {
    store
        .hot_records(smysl::HotKind::SchemaDecls)
        .any(|r| matches!(r, Record::SchemaDecl(d) if d.id.as_str() == smysl::schemas::query::ID))
}

/// Plan, run, and write one query over a bound environment. The records to append — a `View`
/// for `as view`, a saved query and its result edges for `--save` — are returned rather than
/// written, because which file they go to is the caller's.
/// The semantic port `q --engine-model` or `--candle-model` asks for (TX-P10 step 4), indexed
/// over `store`; `None` when neither was given.
///
/// S1's vectors are cached in `<base>.vec-<hash12>` (`VectorCache::sidecar`): the store's own
/// path for a plain store, `index/units` inside a library. A sidecar under another model is
/// reported (`SMY-W476`) and rewritten. S1's pooling is the one the model directory declares in
/// `1_Pooling/config.json`, and mean, the e5 card's, when it declares none.
#[allow(unused_variables)]
fn q_semantic(
    m: &ArgMatches,
    store: &smysl::Store,
    base: &std::path::Path,
) -> Result<Option<Box<dyn smysl::query::SemanticSource>>, ExitCode> {
    if let Some(dir) = m.get_one::<String>("engine-model") {
        #[cfg(not(feature = "semantic"))]
        {
            eprintln!("smysl q: --engine-model needs a build with --features semantic");
            return Err(ExitCode::Usage);
        }
        #[cfg(feature = "semantic")]
        {
            let model = smysl::EmbedModel::from_dir(dir).map_err(|e| {
                eprintln!("smysl q: --engine-model {dir}: {e}");
                ExitCode::Failure
            })?;
            let index = smysl::Semantic::index(store, model);
            let hash = index.model_hash();
            return Ok(Some(Box::new(smysl::query::RetrieverSource::new(
                index, hash,
            ))));
        }
    }
    if let Some(dir) = m.get_one::<String>("candle-model") {
        #[cfg(not(feature = "semantic-candle"))]
        {
            eprintln!("smysl q: --candle-model needs a build with --features semantic-candle");
            return Err(ExitCode::Usage);
        }
        #[cfg(feature = "semantic-candle")]
        {
            let pooling = smysl::Pooling::from_dir(dir).unwrap_or(smysl::Pooling::Mean);
            let encoder = smysl::CandleEncoder::from_dir(dir, pooling).map_err(|e| {
                eprintln!("smysl q: --candle-model {dir}: {e}");
                ExitCode::Failure
            })?;
            let hash = encoder.model_hash();
            let side = smysl::VectorCache::sidecar(base, &hash);
            let (mut cache, diags) = smysl::VectorCache::open(&side, &encoder).map_err(|e| {
                eprintln!("smysl q: {e}");
                ExitCode::Failure
            })?;
            for d in &diags {
                eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
            }
            let index =
                smysl::CandleSemantic::index_cached(store, encoder, &mut cache).map_err(|e| {
                    eprintln!("smysl q: --candle-model {dir}: {e}");
                    ExitCode::Failure
                })?;
            if let Err(e) = cache.save(&side) {
                // The answer does not depend on the cache, so a sidecar that cannot be written
                // is said and not fatal: the next run embeds again.
                eprintln!("smysl q: {e}");
            }
            return Ok(Some(Box::new(smysl::query::RetrieverSource::new(
                index, hash,
            ))));
        }
    }
    Ok(None)
}

fn q_run(
    job: &Job,
    env: &smysl::query::Env<'_>,
    opts: &smysl::query::PlanOptions,
    m: &ArgMatches,
    global: &ArgMatches,
) -> (ExitCode, Vec<Record>) {
    use smysl::query::{explain, run};

    let mut records: Vec<Record> = Vec::new();
    let (plan, first, as_view, effect) = match job {
        Job::Rerun { uid, current } => {
            let uid = match UidPrefix::parse(uid).and_then(|p| env.store.resolve_prefix(&p)) {
                Ok(u) => u,
                Err(e) => {
                    eprintln!("smysl q: --rerun {uid}: {e}");
                    return (ExitCode::Usage, records);
                }
            };
            let saved = match smysl::query::load(env.store, &uid) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("smysl q: --rerun {uid}: {e}");
                    return (ExitCode::Usage, records);
                }
            };
            if m.get_flag("explain") {
                // The saved text, planned today: what a rerun would do, without doing it.
                let parsed = match smysl::query::parse(&saved.text) {
                    Ok(p) => p,
                    Err(d) => {
                        eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                        return (ExitCode::Failure, records);
                    }
                };
                return match smysl::query::plan(&parsed.query, env, opts) {
                    Ok(plan) => {
                        print!("{}", explain(&plan));
                        (ExitCode::Success, records)
                    }
                    Err(diags) => {
                        for d in diags {
                            eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                        }
                        (ExitCode::Usage, records)
                    }
                };
            }
            let again = match smysl::query::rerun(&saved, env, opts, *current) {
                Ok(r) => r,
                Err(diags) => {
                    for d in diags {
                        eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                    }
                    return (ExitCode::Usage, records);
                }
            };
            if *current && m.get_flag("save") {
                // The query as `--current` ran it: the saved text with its schema pins
                // dropped, saved anew and superseding the old unit (§3.6).
                let mut at_cur = match smysl::query::parse(&saved.text) {
                    Ok(p) => p.query,
                    Err(d) => {
                        eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                        return (ExitCode::Failure, records);
                    }
                };
                at_cur
                    .header
                    .retain(|u| !matches!(u, smysl::query::ast::Using::Schema { .. }));
                match smysl::query::to_records(&at_cur, &again.plan, &again.run, opts) {
                    Ok((unit, edges)) => {
                        let new = smysl::canonical_uid(&unit);
                        records.push(Record::Unit(unit));
                        records.extend(edges.into_iter().map(Record::Relation));
                        records.push(Record::Relation(Relation::new(
                            RelKind::Supersedes,
                            new,
                            uid,
                        )));
                        eprintln!("smysl q: saved {new}, superseding {uid}");
                    }
                    Err(d) => {
                        eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                        return (ExitCode::Failure, records);
                    }
                }
            }
            let effect = Some((again.data_effect, again.recipe_matches, again.schema_effect));
            (again.plan, again.run, false, effect)
        }
        Job::Run { query, as_view } => {
            // Bound before planning, so the text a save records is the text after binding
            // (SMYSL-2.5 §3.6) and the plan is of exactly that text. An unbound parameter
            // is `SMY-E460` here as it would be at execution.
            let bound = match smysl::query::bind(query, &opts.params) {
                Ok(q) => q,
                Err(d) => {
                    eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                    return (ExitCode::Usage, records);
                }
            };
            let plan = match smysl::query::plan(&bound, env, opts) {
                Ok(p) => p,
                Err(diags) => {
                    for d in diags {
                        eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                    }
                    return (ExitCode::Usage, records);
                }
            };
            if m.get_flag("explain") {
                print!("{}", explain(&plan));
                return (ExitCode::Success, records);
            }

            let seed_check = global.get_flag("seed-check");
            if seed_check {
                // §3.2: refused before anything executes (TX-P10 step 4).
                if let Err(d) = plan.seed_check() {
                    eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                    return (ExitCode::Usage, records);
                }
            }

            let first = match run(&plan, env, opts) {
                Ok(r) => r,
                Err(diags) => {
                    for d in diags {
                        eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                    }
                    return (ExitCode::Failure, records);
                }
            };
            if seed_check {
                // §3.2: a pure plan "executes twice — the second time with freshly built
                // indexes and scratch buffers — and compares result hashes; a mismatch is an
                // internal error". The executor builds its tables inside the run, so the
                // second run is fresh by construction, and the comparison is of the answers
                // themselves rather than of a hash of them — a hash would only add a way for
                // two equal answers to look different.
                match run(&plan, env, opts) {
                    Ok(second) if second == first => {}
                    Ok(_) => {
                        eprintln!(
                            "smysl q: --seed-check ran the plan twice and the answers differ; \
                             this is an internal error, not a property of the query"
                        );
                        return (ExitCode::Failure, records);
                    }
                    Err(diags) => {
                        for d in diags {
                            eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                        }
                        return (ExitCode::Failure, records);
                    }
                }
            }

            if m.get_flag("save") {
                // After the run, never before (OQ-82): three of the seven keys and every
                // result edge are facts about a run. The declaration goes with the first
                // saved query a store receives, as `x.text/v1` goes with the first batch.
                match smysl::query::to_records(&bound, &plan, &first, opts) {
                    Ok((unit, edges)) => {
                        let uid = smysl::canonical_uid(&unit);
                        if !declares_query(env.store) {
                            records.push(Record::SchemaDecl(smysl::schemas::query::decl()));
                        }
                        records.push(Record::Unit(unit));
                        records.extend(edges.into_iter().map(Record::Relation));
                        eprintln!("smysl q: saved {uid}");
                    }
                    Err(d) => {
                        eprintln!("smysl q: {}: {}", d.code.as_str(), d.message);
                        return (ExitCode::Failure, records);
                    }
                }
            }
            (plan, first, *as_view, None)
        }
    };
    let _ = &plan;

    for w in &first.warnings {
        eprintln!("smysl q: {}: {}", w.code.as_str(), w.message);
    }
    q_print(&first, env.store, m, global);

    if let Some((diff, recipe_matches, schema)) = effect {
        // On stderr beside the warnings: the answer is the rerun's result, and the effect is
        // a statement *about* it that a caller piping the result should not have to strip.
        eprintln!(
            "smysl q: data effect: {} added, {} removed{}",
            diff.added.len(),
            diff.removed.len(),
            if recipe_matches {
                ""
            } else {
                " (recipe differs from the saved one; the effect is not comparable)"
            }
        );
        for u in &diff.added {
            eprintln!("  + {u}");
        }
        for u in &diff.removed {
            eprintln!("  - {u}");
        }
        if let Some(schema) = schema {
            // `diff(E_pin, E_cur)`: what the declarations' growth changed (§3.6).
            eprintln!(
                "smysl q: schema effect: {} added, {} removed",
                schema.added.len(),
                schema.removed.len()
            );
            for u in &schema.added {
                eprintln!("  + {u}");
            }
            for u in &schema.removed {
                eprintln!("  - {u}");
            }
        }
    }

    if as_view {
        let id = m
            .get_one::<String>("view")
            .and_then(|s| ViewId::new(s).ok())
            .unwrap_or_else(|| ViewId::new("v/query").expect("literal"));
        records.push(Record::View(
            View::new(id, "query").with_roots(first.answer.uids()),
        ));
    }
    (ExitCode::Success, records)
}

/// Write an answer the way `q` writes one: a table, or `--json`'s one object.
fn q_print(first: &smysl::query::Run, store: &Store, m: &ArgMatches, global: &ArgMatches) {
    use smysl::query::{why, Answer};

    let gist_of = |uid: &Uid| store.get(uid).map(|u| u.core.gist.as_str()).unwrap_or("");
    if global.get_flag("json") {
        let warnings: Vec<String> = first
            .warnings
            .iter()
            .map(|w| {
                format!(
                    "{{\"code\":{},\"message\":{}}}",
                    smysl::json_escape(w.code.as_str()),
                    smysl::json_escape(&w.message)
                )
            })
            .collect();
        let answer = match &first.answer {
            Answer::Count(n) => format!("{{\"kind\":\"count\",\"count\":{n}}}"),
            Answer::Ranked(fused) => {
                let hits: Vec<String> = fused
                    .iter()
                    .map(|f| {
                        format!(
                            "{{\"uid\":{},\"score\":{:.6}}}",
                            smysl::json_escape(&f.uid.canonical()),
                            f.score
                        )
                    })
                    .collect();
                format!("{{\"kind\":\"ranked\",\"hits\":[{}]}}", hits.join(","))
            }
            Answer::Groups(groups) => {
                let rows: Vec<String> = groups
                    .iter()
                    .map(|(value, uids)| {
                        format!(
                            "{{\"value\":{},\"uids\":[{}]}}",
                            smysl::json_escape(value),
                            uid_array(uids)
                        )
                    })
                    .collect();
                format!("{{\"kind\":\"groups\",\"groups\":[{}]}}", rows.join(","))
            }
            other => format!(
                "{{\"kind\":\"units\",\"uids\":[{}]}}",
                uid_array(&other.uids())
            ),
        };
        println!(
            "{{\"answer\":{answer},\"warnings\":[{}]}}",
            warnings.join(",")
        );
        return;
    }
    match &first.answer {
        Answer::Count(n) => println!("{n}"),
        Answer::Ranked(fused) => {
            for f in fused {
                println!("{:.6}  {}  {}", f.score, f.uid, gist_of(&f.uid));
            }
            if m.get_flag("why") {
                // On stderr, as `find --why` does, so a caller piping stdout keeps the
                // ranking and a reader at the terminal sees what put each hit there.
                eprint!("{}", why(fused));
            }
        }
        Answer::Groups(groups) => {
            for (value, uids) in groups {
                println!("{value}:");
                for uid in uids {
                    println!("  {uid}  {}", gist_of(uid));
                }
            }
        }
        other => {
            for uid in other.uids() {
                println!("{uid}  {}", gist_of(&uid));
            }
        }
    }
}

/// `smysl thread` - derive, list, or show a thread (§19, §23.1).
///
/// Derivation is pure: no model is consulted, so the same store yields the same thread on
/// any machine. `--refine`, which does consult one, is what makes the command *mixed* - and
/// it arrives with the provider layer, because there is nothing to refine with until then.
fn cmd_thread(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let mut strict_failed = false;
    let Some(path) = store_arg(m, global) else {
        eprintln!("smysl thread: no store given");
        return ExitCode::Usage;
    };
    let (store, labels) = match load_memory(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl thread: {e}");
            return ExitCode::Failure;
        }
    };

    if m.get_flag("list") {
        let mut any = false;
        for t in store.threads() {
            any = true;
            println!(
                "{}  {}  {} step(s)  {}",
                t.id,
                t.schema,
                t.steps.len(),
                t.gist
            );
        }
        if !any {
            println!("{path}: no threads");
        }
        return ExitCode::Success;
    }

    if let Some(want) = m.get_one::<String>("show") {
        let Some(t) = store.threads().find(|t| t.id.as_str() == want) else {
            eprintln!("smysl thread: no thread `{want}` in {path}");
            return ExitCode::Failure;
        };
        return show_thread(&store, t);
    }

    // §23 spells this `--derive --schema S`; `--derive S` is accepted as the shorter
    // form of the same thing.
    if !m.contains_id("derive") && m.get_one::<String>("schema").is_none() {
        eprintln!("smysl thread: one of --derive, --list or --show is required");
        return ExitCode::Usage;
    }
    let raw = match m
        .get_one::<String>("derive")
        .or_else(|| m.get_one::<String>("schema"))
    {
        Some(v) => v,
        None => {
            eprintln!("smysl thread: --derive needs a schema, here or in --schema");
            return ExitCode::Usage;
        }
    };
    let Some(schema) = smysl::ThreadSchema::parse(raw) else {
        eprintln!("smysl thread: `{raw}` is not a schema");
        return ExitCode::Usage;
    };
    // A dossier has no matcher rules: its steps are slots a template filled, and only `ask`
    // fills them (SMYSL-2.5 §3.13). Deriving one from the graph would return an empty thread
    // — correct and useless — so it is refused here, with the command that does produce one.
    if !smysl::derivable(schema) {
        eprintln!(
            "smysl thread: `{raw}` cannot be derived from the graph; a {raw} thread is produced \
             by `ask` (TX-P8 step 3)"
        );
        return ExitCode::Usage;
    }

    let id = match m.get_one::<String>("id") {
        Some(s) => match smysl::ThreadId::new(s) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("smysl thread: `{s}` is not a thread id: {e}");
                return ExitCode::Usage;
            }
        },
        // Threads are keyed by (id, owner), so an authored `t/brief` and a derived one
        // would coexist under the same name and force every reader to disambiguate. The
        // default id says which it is.
        None => smysl::ThreadId::new(format!("t/derived-{schema}"))
            .expect("a schema name is a valid id"),
    };
    let owner = match m.get_one::<String>("as") {
        Some(s) => match AgentId::new(s) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("smysl thread: `{s}` is not an agent id: {e}");
                return ExitCode::Usage;
            }
        },
        None => AgentId::new("tool:smysl").expect("a valid literal"),
    };

    let mut opts = DeriveOptions::new(id, owner.clone());
    if let Some(v) = m.get_many::<String>("scope") {
        let mut scope = Vec::new();
        for raw in v {
            match resolve(&store, raw) {
                Ok(u) => scope.push(u),
                Err(e) => {
                    eprintln!("smysl thread: {e}");
                    return e.code;
                }
            }
        }
        opts = opts.scoped(scope);
    }
    for raw in m.get_many::<String>("arity").into_iter().flatten() {
        let Some((role, n)) = raw.split_once('=') else {
            eprintln!("smysl thread: --arity takes ROLE=N, not `{raw}`");
            return ExitCode::Usage;
        };
        let (Some(role), Ok(n)) = (Role::parse(role.trim()), n.trim().parse::<usize>()) else {
            eprintln!("smysl thread: `{raw}` is not a role and a count");
            return ExitCode::Usage;
        };
        if !schema.allows(role) {
            eprintln!("smysl thread: {schema} has no {role} role");
            return ExitCode::Usage;
        }
        opts = opts.with_arity(role, n);
    }
    // A supplied clock, as merge does: `smysl thread --derive` is a rule D operation, and
    // an operation whose output carries the wall clock is not bit-reproducible. The
    // timestamp on a derived thread says which derivation it was, not when it happened -
    // and the derivation is the same one whenever it is run.
    opts = opts.with_ts(Hlc::new(0, 0, owner));

    let (thread, report) = smysl::derive_thread(&store, schema, &opts);

    if m.get_flag("explain") {
        for role in smysl::schema_definition(schema).roles {
            let n = thread.steps.iter().filter(|s| s.role == *role).count();
            let a = smysl::schema_definition(schema).arity_of(*role);
            eprintln!("{path}: {role:<12} {n} of {}..{}", a.start(), a.end());
        }
        for role in &report.unfilled {
            eprintln!("{path}: {role} is required by {schema} and nothing could fill it");
        }
        // `derive` has no `Report` to threshold, so `--strict` keys on the condition it
        // prints: a role the schema requires that nothing could fill. The thread is still
        // emitted — the caller asked for one and gets one — but a gate is told.
        if global.get_flag("strict") && !report.unfilled.is_empty() {
            strict_failed = true;
        }
        for (added, needed_by) in &report.repaired {
            eprintln!("{path}: {added} added by repair; {needed_by} depends on it");
        }
        eprintln!("{path}: {} unit(s) not selected", report.unselected);
    }

    // The store travels with the thread, so `thread --derive | render` works: a thread
    // record alone names uids the next stage would have no way to resolve. `--only` asks
    // for the record by itself.
    let ctx = WriteContext::from_labels(&labels);
    let mut records: Vec<Record> = if m.get_flag("only") {
        Vec::new()
    } else {
        store.iter().cloned().collect()
    };
    records.push(Record::Thread(thread));

    // Three things this got wrong, all of them the same shape as bugs found elsewhere in
    // this audit: `--format` was advertised and ignored, so rule P did not hold and a pipe
    // got text; `-o` was advertised and ignored, so a caller who asked for a file got
    // nothing; and `write_surface` was handed `None` for the view, which drops the `@doc`
    // header — the identical mistake `merge --format surface` shipped with.
    // `--format` is now honoured when given. The *default* is left as surface, which is
    // where this command sits awkwardly against rule P: `merge` and `pack` default to CBOR
    // and `thread` does not. Changing it would be right by the rule and would also change
    // what every documented `thread --derive` example prints, so it is left for a decision
    // rather than taken here.
    let surface = wants_surface(global, true);

    let bytes = if surface {
        let view = if m.get_flag("only") {
            None
        } else {
            store.views().next().cloned()
        };
        write_surface(view.as_ref(), &records, &ctx).into_bytes()
    } else {
        smysl::to_cbor_seq(&records)
    };

    let code = emit(global, "thread", &bytes);
    if strict_failed && code == ExitCode::Success {
        return ExitCode::CheckErrors;
    }
    code
}

fn show_thread(store: &(impl StoreRead + ?Sized), t: &smysl::Thread) -> ExitCode {
    println!("{}  {}", t.id, t.schema);
    println!("~ {}", t.gist);
    for (i, step) in t.steps.iter().enumerate() {
        let gist = store
            .get(&step.unit)
            .map(|u| u.core.gist.clone())
            .unwrap_or_else(|| "(not in this store)".into());
        println!("{:>3}. {:<12} {}  {}", i + 1, step.role, step.unit, gist);
    }
    let broken = smysl::satisfies_rule_l(store, t);
    for (unit, dep) in &broken {
        eprintln!("  rule L: {unit} references {dep}, which the thread does not name");
    }
    if broken.is_empty() {
        ExitCode::Success
    } else {
        ExitCode::Failure
    }
}

/// `smysl render` - thread plus profile to artifact (§10, §20, §23.1).
///
/// Pure. Rule V1 is applied when the profile loads, so a profile that would flatten
/// epistemic status exits before anything is rendered; rule V2 is applied when the IR is
/// built, so a suppressed contention is recorded in the output whatever the target.
fn cmd_render(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    if m.get_flag("profiles") {
        for name in Profile::builtin_names() {
            let p = Profile::builtin(name).expect("built-ins load");
            println!(
                "{name:<10} {} {} · lod {} · status {}",
                match p.register {
                    smysl::Register::Formal => "formal ",
                    smysl::Register::Plain => "plain  ",
                    _ => "neutral",
                },
                p.audience.as_deref().unwrap_or("-"),
                p.lod.default,
                match p.show.status {
                    smysl::StatusDisplay::Word => "word",
                    _ => "marker",
                }
            );
        }
        return ExitCode::Success;
    }

    let Some(path) = store_arg(m, global) else {
        eprintln!("smysl render: no store given");
        return ExitCode::Usage;
    };
    let (store, _) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl render: {e}");
            return ExitCode::Failure;
        }
    };

    // Rule V1, before anything else: a profile that flattens must not get as far as
    // producing bytes.
    let profile = match m.get_one::<String>("profile") {
        Some(name) => match Profile::builtin(name) {
            Some(p) => p,
            None => match std::fs::read_to_string(name) {
                Ok(src) => match Profile::load(&src) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("smysl render: {name}: {e}");
                        return ExitCode::CheckErrors;
                    }
                },
                Err(e) => {
                    eprintln!("smysl render: {name} is neither a built-in profile nor a file: {e}");
                    return ExitCode::Usage;
                }
            },
        },
        None => Profile::builtin("plain").expect("the plain profile loads"),
    };

    // Threads are keyed by (id, owner), so a name may legitimately match more than one.
    // Guessing between two agents' readings of the same graph is exactly the flattening
    // this format exists to prevent, so it is an error rather than a coin toss.
    let want = m.get_one::<String>("thread");
    let owner = m.get_one::<String>("as");
    let candidates: Vec<&smysl::Thread> = store
        .threads()
        .filter(|t| match want {
            Some(w) => t.id.as_str() == w,
            None => true,
        })
        .filter(|t| match owner {
            Some(o) => t.owner.as_str() == o,
            None => true,
        })
        .collect();
    let thread = match candidates.as_slice() {
        [only] => (*only).clone(),
        [] => {
            eprintln!(
                "smysl render: {path} holds no thread matching {}",
                want.map(String::as_str).unwrap_or("any id")
            );
            return ExitCode::Failure;
        }
        many => {
            eprintln!(
                "smysl render: {path} holds {} matching threads; narrow with --thread or --as",
                many.len()
            );
            for t in many {
                eprintln!("smysl render:   {} ({})", t.id, t.owner);
            }
            return ExitCode::Usage;
        }
    };

    let target = match m.get_one::<String>("target") {
        Some(t) => match Target::parse(t) {
            Some(t) => t,
            None => {
                eprintln!("smysl render: `{t}` is not a target");
                return ExitCode::Usage;
            }
        },
        None => Target::Markdown,
    };

    let mut opts = BuildOptions::default();
    if let Some(l) = m.get_one::<String>("lod") {
        opts.lod_cap = match l.as_str() {
            "L0" => Some(Lod::L0),
            "L1" => Some(Lod::L1),
            _ => Some(Lod::L2),
        };
    }
    if let Some(c) = m.get_one::<String>("contentions") {
        opts.contentions = Some(match c.as_str() {
            "suppress" => Contentions::Suppress,
            _ => Contentions::Always,
        });
    }

    let ir = smysl::build_ir(&store, &thread, &profile, &opts);
    let artifact = match smysl::emit_artifact(target, &ir, &profile) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl render: {e}");
            return ExitCode::Failure;
        }
    };

    // Rule V2 is a warning, not a refusal: the artifact is emitted and the suppression is
    // reported, because the caller asked for it and the metadata records it either way.
    if ir.meta.contentions_suppressed {
        eprintln!(
            "smysl render: {}: {} open contention(s) suppressed by profile {}",
            Code::W211,
            ir.meta.open_contentions.len(),
            profile.name
        );
    }

    match global.get_one::<String>("output") {
        Some(out) => {
            if let Err(e) = std::fs::write(out, artifact.as_bytes()) {
                eprintln!("smysl render: {out}: {e}");
                return ExitCode::Failure;
            }
        }
        None => print!("{}", artifact.text),
    }

    if global.get_flag("strict") && ir.meta.contentions_suppressed {
        return ExitCode::CheckErrors;
    }
    ExitCode::Success
}

/// Where a project's `.smysl/` directory lives: beside the store, or under the cwd.
#[cfg(feature = "providers")]
fn project_file(global: &ArgMatches, name: &str) -> std::path::PathBuf {
    root_beside(global.get_one::<String>("store").map(String::as_str)).join(name)
}

/// The directory a sidecar — or a `-C` file's own relative paths — belongs in.
///
/// Separated from the two helpers that call it because they take an `ArgMatches`, which is
/// what made the `!` in the filter untestable — two mutants survived on the same expression
/// in `project_root` and `project_file`. The rule it encodes is small and worth stating: a
/// store named with no directory part (`smysl --store notes.smy`) has a *parent* of `""`,
/// which is not the current directory and would put the sidecar at the filesystem root.
#[cfg(feature = "providers")]
fn root_beside(store: Option<&str>) -> std::path::PathBuf {
    store
        .map(std::path::PathBuf::from)
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// A configuration that could not be loaded, and the exit code that says whose mistake it was.
///
/// The two cases were one while `-C` was ignored, because the only way to reach this error was a
/// project sidecar the caller never named. A file the caller named by hand is different: if it is
/// not there, the command line is wrong, and that is a usage error.
#[cfg(feature = "providers")]
struct ConfigError {
    code: ExitCode,
    message: String,
}

#[cfg(feature = "providers")]
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The configuration file to read, and whether it is allowed not to exist.
///
/// `-C FILE` names the file outright, so it is required: silently falling back to the project
/// sidecar — or to the all-local default — would run the command against a configuration the
/// caller did not ask for and did not see.
#[cfg(feature = "providers")]
fn config_path(global: &ArgMatches) -> (std::path::PathBuf, bool) {
    match global.get_one::<String>("config") {
        Some(p) => (std::path::PathBuf::from(p), true),
        None => (project_file(global, smysl::ProviderConfigFile::PATH), false),
    }
}

/// Where a relative path written *inside* the configuration points.
///
/// Beside the file that spells it, which for the project sidecar is the project and for `-C` is
/// wherever the caller keeps it. `PromptOverride::load_file` already resolves its own `*_file`
/// keys this way; a config two directories away whose `ingest.prompt` resolved against the store
/// would name a file the author of that config had no way to predict.
//
// Gated on `ingest` rather than `providers`, like its one caller: `providers` alone compiles no
// command that reads a path out of the configuration, and a function that is live in one feature
// row and dead in another is the dead-code error that failed `-D warnings` for three releases.
#[cfg(feature = "ingest")]
fn config_relative(global: &ArgMatches, p: &str) -> std::path::PathBuf {
    match global.get_one::<String>("config") {
        Some(c) => root_beside(Some(c.as_str())).join(p),
        None => project_file(global, p),
    }
}

/// The configuration named by `-C`, or the project's, or the all-local default when neither is
/// there.
#[cfg(feature = "providers")]
fn load_config(global: &ArgMatches) -> Result<smysl::ProviderConfigFile, ConfigError> {
    let (path, required) = config_path(global);
    match std::fs::read_to_string(&path) {
        Ok(src) => smysl::ProviderConfigFile::load(&src).map_err(|e| ConfigError {
            code: ExitCode::Failure,
            message: format!("{}: {e}", path.display()),
        }),
        Err(e) if required => Err(ConfigError {
            code: ExitCode::Usage,
            message: format!("{}: {e}", path.display()),
        }),
        // A default that reached a hosted provider would mean a first run egressing
        // content nobody asked to send, so the default is entirely local.
        Err(_) => Ok(smysl::ProviderConfigFile::local_default()),
    }
}

/// The prompt override for `ingest`: `--prompt`, else `ingest.prompt` from the config.
///
/// A config path is relative to the config that spells it — the project for a sidecar, its own
/// directory under `-C`; a flag is relative to where the command was typed, like every other
/// argument.
#[cfg(feature = "ingest")]
fn load_prompt(
    m: &ArgMatches,
    global: &ArgMatches,
) -> Result<Option<smysl::PromptOverride>, ConfigError> {
    let path = match m.get_one::<String>("prompt") {
        Some(p) => std::path::PathBuf::from(p),
        None => match load_config(global)?.ingest_prompt {
            Some(p) => config_relative(global, &p),
            None => return Ok(None),
        },
    };
    smysl::PromptOverride::load_file(&path)
        .map(Some)
        .map_err(|e| ConfigError {
            code: ExitCode::Failure,
            message: e,
        })
}

/// Load the provider configuration, falling back to the all-local default.
#[cfg(feature = "providers")]
fn load_registry(global: &ArgMatches) -> Result<smysl::Registry, ConfigError> {
    let cfg = load_config(global)?;

    let mut r = smysl::Registry::new().offline(global.get_flag("offline"));
    for p in cfg.providers.values() {
        match smysl::build_provider(p) {
            Ok(built) => r = r.with_provider(built),
            // A provider this build cannot drive is reported and skipped rather than
            // fatal: the other providers still work, and `providers` is the command a
            // caller runs precisely to find out what is wrong.
            Err(e) => eprintln!("smysl providers: {}: {e}", p.id),
        }
    }
    for (task, id) in &cfg.routing {
        r = r.route(*task, id.clone());
    }
    Ok(r.with_fallback(cfg.fallback))
}

/// `smysl ingest` - prose or data to staged units (§23.1). **Model-dependent.**
///
/// Exits 10 unless `--yes`: rule S says model output is staged and confirmed, and an exit
/// code is how a pipeline learns that a decision is waiting for it.
/// `ingest --text <alias|mid>`: one expression of a library, window by window (§4.4).
///
/// Commits as it goes, and that is not a convenience. The journal entry that makes a window
/// resumable is written *after* its batch is committed, so a run that staged to a file and
/// committed later could not write a journal at all — a kill would then lose everything or
/// claim everything. `--yes` is therefore required rather than optional here, and says the same
/// thing it says everywhere else: this writes to the store you named.
#[cfg(all(feature = "ingest", feature = "text"))]
fn cmd_ingest_text(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::text::library::Library;
    use smysl::text::reading::Level;

    let who = m.get_one::<String>("text").expect("checked by the caller");
    // The cost report reads the journal and nothing else: no provider, no egress, and no
    // `--yes`, because there is nothing to commit. Handled before the registry is loaded, so a
    // machine with no provider configured can still price what it already extracted.
    if m.get_flag("cost") {
        return cmd_ingest_cost(m, global, who);
    }
    let registry = match load_registry(global) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl ingest: {e}");
            return e.code;
        }
    };
    let Some(root) = store_arg(m, global) else {
        eprintln!("smysl ingest: --text reads a library, so it needs one: pass --store <LIBRARY>");
        return ExitCode::Usage;
    };
    let root = std::path::PathBuf::from(root);
    if !m.get_flag("yes") && !m.get_flag("dry-run") {
        eprintln!(
            "smysl ingest: --text commits each window as it is staged, so it needs --yes; \
             --dry-run reports what would be sent"
        );
        return ExitCode::Usage;
    }
    let mut lib = match Library::open(&root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl ingest: {}: {e}", root.display());
            return ExitCode::Failure;
        }
    };

    // The target: an alias, or a manifest id for a corpus whose alias has forked.
    let target = match smysl::Mid::parse(who) {
        Ok(mid) => smysl::TextTarget::Manifest(mid),
        Err(_) => smysl::TextTarget::Alias(who.clone()),
    };

    // The window level: the caller's, else the reading's leaves. Reported either way, because a
    // default that decides how every claim was extracted is a default a reader has to see.
    let level = match m.get_one::<String>("window-level") {
        Some(l) => match Level::new(l) {
            Some(level) => Some(level),
            None => {
                eprintln!("smysl ingest: `{l}` is not a level: lowercase, digits and `-`");
                return ExitCode::Usage;
            }
        },
        None => None,
    };
    let level = match level {
        Some(l) => l,
        None => match default_level(&lib, &target) {
            Ok(l) => {
                // Said once. A dry run reports the level in its own block below, and a real run
                // reports it here — because a default that decides how every claim in a corpus
                // was extracted is not a default to leave unsaid.
                if !m.get_flag("dry-run") {
                    println!("window-level {l} (the reading's leaves)");
                }
                l
            }
            Err(code) => return code,
        },
    };

    let rung = m
        .get_one::<String>("rung")
        .and_then(|s| smysl::Rung::parse(s))
        .unwrap_or(smysl::Rung::Document);
    let mut opts = smysl::IngestOptions::at_rung(rung);
    // Two passes per window is the default here and nowhere else (§3.4), because the doubling
    // is `ingest --text`'s: `--single-pass` opts out and is recorded in the recipe.
    opts = opts.with_passes(if m.get_flag("single-pass") {
        smysl::Passes::Single
    } else {
        smysl::Passes::TEXT
    });
    if let Some(hop) = m.get_one::<u32>("hop") {
        opts = opts.at_hop(*hop);
    }
    if let Some(g) = m.get_one::<String>("granularity") {
        opts = opts.with_granularity(g);
    }
    if let Some(id) = m.get_one::<String>("estimator") {
        match TokenEstimator::parse(id) {
            Some(e) => opts = opts.with_estimator(e),
            None => {
                eprintln!("smysl ingest: unknown estimator `{id}`");
                return ExitCode::Usage;
            }
        }
    }
    if let Some(n) = m.get_one::<usize>("max-output") {
        opts = opts.with_max_output(*n);
    }
    if let Some(p) = m.get_one::<String>("lang-policy") {
        match smysl::LangPolicy::parse_with_lang(p) {
            Ok((policy, lang)) => {
                opts = opts.with_lang_policy(policy);
                if let Some(l) = lang {
                    opts = opts.with_pivot(l);
                }
            }
            Err(e) => {
                eprintln!("smysl ingest: {e}");
                return ExitCode::Usage;
            }
        }
    }
    if let Some(n) = m.get_one::<String>("normaliser") {
        opts = opts.with_normaliser(match n.as_str() {
            "v2" => smysl::QuoteNormaliser::V2,
            _ => smysl::QuoteNormaliser::V1,
        });
    }
    if let Some(t) = m.get_one::<f32>("temperature").copied() {
        if !(0.0..=2.0).contains(&t) {
            eprintln!("smysl ingest: --temperature {t} is outside 0.0 to 2.0");
            return ExitCode::Usage;
        }
        opts = opts.with_temperature(t);
    }
    if let Some(n) = m.get_one::<String>("repair").and_then(|s| s.parse().ok()) {
        opts = opts.with_repair_attempts(n);
    }
    match load_prompt(m, global) {
        Ok(Some(p)) => opts = opts.with_prompt(p),
        Ok(None) => {}
        Err(e) => {
            eprintln!("smysl ingest: prompt override: {e}");
            return e.code;
        }
    }

    let digest_budget = m.get_one::<u32>("digest-budget").copied().unwrap_or(512);

    if m.get_flag("dry-run") {
        let provider = match registry.for_task(smysl::Task::ContentIngest) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("smysl ingest: {e}");
                return e.exit_code();
            }
        };
        let caps = provider.caps();
        println!("provider     {}", provider.id());
        println!(
            "egress       {}",
            if caps.offline {
                "no - local"
            } else {
                "YES - leaves the machine"
            }
        );
        println!("expression   {who}");
        println!("window-level {level}");
        println!("passes       {} per window", opts.passes.count());
        println!("digest       {digest_budget} token(s) per window");
        println!("rung         {rung} (ceiling {})", smysl::ceiling(rung));
        println!(
            "max output   {} token(s) per call",
            opts.output_budget(&caps)
        );
        return ExitCode::Success;
    }

    let mut ingest =
        smysl::TextIngest::new(&mut lib, target, level).with_digest_budget(digest_budget);
    ingest.reextract = m.get_flag("reextract");

    let spin = Spinner::new(progress_style(global), format!("ingesting {who}"));
    let outcome = smysl::Ingestor::new(&registry, opts).ingest_text(&mut ingest);
    spin.finish("");

    let report = match outcome {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl ingest: {e}");
            return e.exit_code();
        }
    };
    for d in &report.diagnostics {
        eprintln!("smysl ingest: {d}");
    }
    println!(
        "{} window(s), {} skipped, {} call(s), {} unit(s), {} entit(ies)",
        report.windows, report.skipped, report.calls, report.units, report.entities
    );
    println!(
        "spans        {} attached, {} without one{}",
        report.spanned,
        report.spanless,
        match report.span_coverage() {
            Some(r) => format!(" ({:.1}% coverage)", 100.0 * r),
            None => String::new(),
        }
    );
    println!(
        "quotes       {} present, {} loose, {} absent",
        report.verdicts.present, report.verdicts.loose, report.verdicts.absent
    );
    // The path is chosen rather than configured (D-9), and §3.9.2 does not treat the two as
    // equivalent — so a run says which one it took.
    if let Some(path) = report.path {
        println!("path         {}", path.as_str());
    }
    // GE-T2's figure for this run (§5.5). Absent for a single pass, which agrees with nothing.
    match report.stability() {
        Some(r) => println!(
            "stability    {:.3} identical-span ({} of {} place(s), {} uncomparable)",
            r, report.agreement.agreed, report.agreement.comparable, report.agreement.spanless,
        ),
        None => println!("stability    - (one pass, or nothing comparable)"),
    }
    // Printed only under the policy that runs it, so a `source` run's output is unchanged.
    if report.pivot_calls > 0 || report.pivoted > 0 || report.pivot_missing > 0 {
        println!(
            "pivot        {} translated, {} not ({} call(s))",
            report.pivoted, report.pivot_missing, report.pivot_calls
        );
    }
    println!(
        "tokens       {} in, {} out",
        report.usage.input_tokens, report.usage.output_tokens
    );
    if let Some(r) = report.recipe {
        println!("recipe       {}", smysl::recipe_short(&r));
    }
    ExitCode::Success
}

/// `ingest --text <who> --cost`: what the journal says an extraction cost (§3.7, §6 TX-P5
/// step 5).
///
/// A measurement and not a plan. Draft 1 of SMYSL-2.4 sized this phase from "≈ 1.6k in + ≈ 1.2k
/// out, ×1.3 for repairs" per chapter, a figure only ever given for English, and S0 measured
/// 3.96x it for hosted Russian. So the retry rate is a **measured per-language term** and this
/// is where a caller reads it.
#[cfg(all(feature = "ingest", feature = "text"))]
fn cmd_ingest_cost(m: &ArgMatches, global: &ArgMatches, who: &str) -> ExitCode {
    use smysl::text::library::Library;

    let Some(root) = store_arg(m, global) else {
        eprintln!("smysl ingest: --cost reads a library's journal: pass --store <LIBRARY>");
        return ExitCode::Usage;
    };
    let root = std::path::PathBuf::from(root);
    let lib = match Library::open(&root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl ingest: {}: {e}", root.display());
            return ExitCode::Failure;
        }
    };
    // An alias names the windows the journal keyed; a manifest id names its expression's alias,
    // because the journal keys on the alias and a fork of it is the same extraction history.
    let alias = match smysl::Mid::parse(who) {
        Ok(mid) => match lib.store().manifest(&mid) {
            Some(manifest) => manifest.alias.clone(),
            None => {
                eprintln!("smysl ingest: no manifest {}", mid.canonical());
                return ExitCode::Failure;
            }
        },
        Err(_) => who.to_string(),
    };

    let journal = smysl::ingest_text::journal::Journal::open(&root);
    let report = smysl::ingest_text::cost::cost(&journal, Some(&alias));
    println!("expression   {alias}");
    if report.total.windows == 0 {
        println!("windows      0 (the journal records no window for this expression)");
        return ExitCode::Success;
    }
    for row in report
        .by_lang
        .values()
        .chain(std::iter::once(&report.total))
    {
        println!(
            "{:<12} {} window(s), {} call(s), {} repair turn(s)",
            row.lang, row.windows, row.calls, row.retries
        );
        match row.retry_rate() {
            Some(r) => println!("             retry rate {r:.3} (repairs per first attempt)"),
            None => println!("             retry rate - (nothing was attempted)"),
        }
        print!(
            "             tokens {} in, {} out",
            row.input_tokens, row.output_tokens
        );
        match row.tokens_per_window() {
            Some((i, o)) => println!("; per window {i:.1} in, {o:.1} out"),
            None => println!(),
        }
        println!(
            "             quotes {} present, {} loose, {} absent{}",
            row.verdicts.present,
            row.verdicts.loose,
            row.verdicts.absent,
            match row.support_rate() {
                Some(r) => format!(" ({:.1}% supported)", 100.0 * r),
                None => String::new(),
            }
        );
        if row.unpriced > 0 {
            println!(
                "             {} window(s) left out: written before this release, their tokens \
                 are a run's running total",
                row.unpriced,
            );
        }
    }
    ExitCode::Success
}

/// The reading's leaves, for a run that named no `--window-level`.
#[cfg(all(feature = "ingest", feature = "text"))]
fn default_level(
    lib: &smysl::text::library::Library,
    target: &smysl::TextTarget,
) -> Result<smysl::text::reading::Level, ExitCode> {
    use smysl::text::limits::Caps;

    let mid = match target {
        // `TextTarget` is `#[non_exhaustive]`: a variant a later release adds has no default
        // level here, and saying so beats windowing it at a level nobody chose.
        smysl::TextTarget::Alias(alias) => match lib.head(alias) {
            Ok(mid) => mid,
            Err(e) => {
                eprintln!("smysl ingest: {alias}: {e}");
                return Err(ExitCode::Failure);
            }
        },
        smysl::TextTarget::Manifest(mid) => *mid,
        _ => {
            eprintln!("smysl ingest: that target needs an explicit --window-level");
            return Err(ExitCode::Usage);
        }
    };
    let Some(manifest) = lib.store().manifest(&mid) else {
        eprintln!("smysl ingest: no manifest {}", mid.canonical());
        return Err(ExitCode::Failure);
    };
    let Some(first) = manifest.parts.first() else {
        eprintln!("smysl ingest: that expression has no parts");
        return Err(ExitCode::Failure);
    };
    let structure = match lib.structure(&mid, &first.tid, &Caps::DEFAULT) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("smysl ingest: {}: {e}", first.tid.canonical());
            return Err(ExitCode::Failure);
        }
    };
    match smysl::ingest_text::window::leaf_level(&structure) {
        Some(l) => Ok(l),
        None => {
            eprintln!("smysl ingest: that reading has no nodes to window");
            Err(ExitCode::Failure)
        }
    }
}

#[cfg(feature = "ingest")]
fn cmd_ingest(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    // `--text` is a different ingest: the input is an expression of a library rather than a
    // file, each window is committed as it is staged, and the journal is what makes a second
    // run cheap. Dispatched before anything else because almost none of the document path's
    // setup applies — there is no file to read and nothing to stage to a `staged.smy`.
    #[cfg(feature = "text")]
    if m.contains_id("text") && m.get_one::<String>("text").is_some() {
        return cmd_ingest_text(m, global);
    }
    #[cfg(not(feature = "text"))]
    if m.get_one::<String>("text").is_some() {
        eprintln!("smysl ingest: --text needs the `text` feature; this build has no library layer");
        return ExitCode::Usage;
    }
    let style = progress_style(global);
    let registry = match load_registry(global) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl ingest: {e}");
            return e.code;
        }
    };

    let source = m
        .get_one::<String>("file")
        .cloned()
        .unwrap_or_else(|| "-".to_string());
    let input = match read_input(&source) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("smysl ingest: {source}: {e}");
            return ExitCode::Failure;
        }
    };

    // `--yes` commits to `--store` (H-13), so a run that cannot commit is refused here —
    // before any provider call, because the alternative is to spend a model call and then
    // discover there is nowhere to put the result.
    let store_target = store_arg(m, global);
    if m.get_flag("yes") && store_target.is_none() {
        eprintln!("smysl ingest: `--yes` commits the batch, so it needs a store: pass --store");
        return ExitCode::Usage;
    }
    let (store, store_labels) = match &store_target {
        Some(path) => match load_memory(path) {
            Ok((s, l)) => (s, l),
            Err(e) => {
                eprintln!("smysl ingest: {e}");
                return ExitCode::Failure;
            }
        },
        None => (Store::new(), std::collections::BTreeMap::new()),
    };

    let rung = m
        .get_one::<String>("rung")
        .and_then(|s| smysl::Rung::parse(s))
        .unwrap_or(smysl::Rung::Document);

    let mut opts = smysl::IngestOptions::at_rung(rung);
    if let Some(hop) = m.get_one::<u32>("hop") {
        opts = opts.at_hop(*hop);
    }
    if let Some(g) = m.get_one::<String>("granularity") {
        opts = opts.with_granularity(g);
    }
    // The estimator decides both the bound stated in the prompt and the bound the staged units
    // are checked against, so an id this build lacks is refused rather than defaulted.
    if let Some(id) = m.get_one::<String>("estimator") {
        match TokenEstimator::parse(id) {
            Some(e) => opts = opts.with_estimator(e),
            None => {
                eprintln!("smysl ingest: unknown estimator `{id}`");
                eprintln!(
                    "  known: {}",
                    TokenEstimator::ALL
                        .iter()
                        .map(|e| e.id())
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                return ExitCode::Usage;
            }
        }
    }
    if let Some(n) = m.get_one::<usize>("max-output") {
        opts = opts.with_max_output(*n);
    }
    // Two defaults come from the configuration, so it is read once rather than per flag.
    let cfg = match load_config(global) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("smysl ingest: {e}");
            return e.code;
        }
    };
    // `--path`, else `ingest.path` from the config. `auto` in either means no override.
    let path_arg = match m.get_one::<String>("path") {
        Some(p) => Some(p.clone()),
        None => cfg.ingest_path.clone(),
    };
    if let Some(p) = path_arg.as_deref().and_then(smysl::IngestPath::parse) {
        opts = opts.with_path(p);
    }
    // `pivot:<lang>` belongs to `--text`, which has a reading to attach a translation to.
    // Refused here as a usage error rather than left to the library's pre-egress refusal, so
    // that a caller who named it on the wrong command gets exit 2 and a sentence saying where
    // it does work — and not a provider error after a registry load.
    if let Some(p) = m.get_one::<String>("lang-policy") {
        match smysl::LangPolicy::parse_with_lang(p) {
            Ok((_, Some(l))) => {
                eprintln!(
                    "smysl ingest: `pivot:{l}` is for `ingest --text <alias>`, which has a \
                     reading to attach the translation to; document ingest takes `source`"
                );
                return ExitCode::Usage;
            }
            Ok((policy, None)) => opts = opts.with_lang_policy(policy),
            Err(e) => {
                eprintln!("smysl ingest: {e}");
                return ExitCode::Usage;
            }
        }
    }
    // V2 answers five probes that V1 calls `Loose` or `Absent` on quotes a reader would call
    // verbatim, and `E307` on an honest quote refuses a correct attribution. It is opt-in in 1.9
    // because it changes the recipe of any run that asks for it, and a comparison form is the
    // kind of thing a caller should choose rather than discover.
    if let Some(n) = m.get_one::<String>("normaliser") {
        opts = opts.with_normaliser(match n.as_str() {
            "v2" => smysl::QuoteNormaliser::V2,
            _ => smysl::QuoteNormaliser::V1,
        });
    }
    // `--temperature`, else `ingest.temperature`. The range is checked before any call: a
    // request a provider will reject is not worth paying for, and the configuration's own copy
    // of this check happens at load for the same reason.
    if let Some(t) = m
        .get_one::<f32>("temperature")
        .copied()
        .or(cfg.ingest_temperature)
    {
        if !(0.0..=2.0).contains(&t) {
            eprintln!("smysl ingest: --temperature {t} is outside 0.0 to 2.0");
            return ExitCode::Usage;
        }
        opts = opts.with_temperature(t);
    }
    if let Some(n) = m.get_one::<String>("repair").and_then(|s| s.parse().ok()) {
        opts = opts.with_repair_attempts(n);
    }
    // A named file is its units' source. The model cannot know what the document is called, and
    // asked to cite it wrote `the input document` on every unit of a live run — the placeholder
    // from its template. `FillMissing`, not `Override`: a source the document itself names, a URL
    // or a paper it quotes, is the model's to keep. Standard input names nothing, so it gets none.
    if source != "-" {
        opts = opts.with_source(
            smysl::SourceRef::new(smysl::SourceKind::File, source.clone()),
            smysl::SourcePolicy::FillMissing,
        );
    }
    match load_prompt(m, global) {
        Ok(Some(p)) => opts = opts.with_prompt(p),
        Ok(None) => {}
        Err(e) => {
            eprintln!("smysl ingest: prompt override: {e}");
            return e.code;
        }
    }
    let requested = match opts.requested_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("smysl ingest: prompt override {e}");
            return ExitCode::Failure;
        }
    };

    // `--dry-run` answers the question a caller most wants answered *before* egress: what
    // would be sent, and to whom. It makes no call, which is the whole point.
    if m.get_flag("dry-run") {
        let provider = match registry.for_task(smysl::Task::ContentIngest) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("smysl ingest: {e}");
                return e.exit_code();
            }
        };
        let caps = provider.caps();
        let choice =
            smysl::choose_ingest_path(&caps, smysl::Task::ContentIngest, input.len(), requested);
        println!("provider     {}", provider.id());
        println!(
            "egress       {}",
            if caps.offline {
                "no - local"
            } else {
                "YES - leaves the machine"
            }
        );
        // When the prompt's schema moved `auto`, the choice reads as a caller override — which
        // is true of the code path and false of the caller, who typed no `--path`.
        if requested != opts.path {
            println!(
                "path         {} (the prompt override supplies a schema)",
                choice.path
            );
        } else {
            println!("path         {} ({})", choice.path, choice.reason.as_str());
        }
        match &opts.prompt {
            Some(p) => println!(
                "prompt       {} v{}{}",
                p.id,
                p.version,
                if p.schema.is_some() {
                    ", own schema"
                } else {
                    ""
                }
            ),
            None => println!("prompt       built-in"),
        }
        match &opts.source {
            Some((s, policy)) => println!(
                "source       {}:{} ({})",
                s.kind.as_str(),
                s.reference,
                policy.as_str()
            ),
            None => println!("source       none - the model names any it can"),
        }
        println!(
            "max output   {} token(s) per call",
            opts.output_budget(&caps)
        );
        println!("rung         {rung} (ceiling {})", smysl::ceiling(rung));
        println!(
            "input        {} bytes, {} token(s)",
            input.len(),
            smysl::tokens(&input)
        );
        return ExitCode::Success;
    }

    // One model call per chunk at least, so the wait is real and worth reporting.
    let spin = Spinner::new(style, format!("ingesting {source}"));
    let outcome = smysl::Ingestor::new(&registry, opts).ingest(&store, &input);
    spin.finish("");

    let (staged, report) = match outcome {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl ingest: {e}");
            return e.exit_code();
        }
    };

    for d in &report.diagnostics {
        eprintln!("smysl ingest: {d}");
    }
    // Rule M lowered these rather than dropping them, so the line says what changed and
    // not what is missing. A reviewer reads `staged.smy` next; this tells them where to
    // look before they do.
    for w in &staged.weakened {
        eprintln!(
            "smysl ingest: rule M lowered a unit from {} to {} - its grounds support no more",
            w.from, w.to
        );
    }
    eprintln!(
        "smysl ingest: {} chunk(s), {} call(s), {} unit(s), {} weakened, {} degraded, \
         {} repaired, {} token(s)",
        report.chunks,
        report.calls,
        staged.len(),
        staged.weakened.len(),
        report.degraded,
        report.repaired,
        report.usage.total()
    );

    // The ledger records counts, models, task and recipe - never content (§29).
    if let Some(provider) = &report.provider {
        let mut ledger = smysl::Ledger::open(project_file(global, smysl::Ledger::PATH));
        let entry = smysl::LedgerEntry::new(
            now_millis(),
            provider.clone(),
            // The resolved name, not `--model`: a run that named no model still ran one, and
            // writing the empty string here is what made `usage --by model` group every
            // default run under nothing (H-9).
            report.model.clone().unwrap_or_default(),
            smysl::Task::ContentIngest,
            report.usage,
        );
        let entry = match report.recipe {
            Some(r) => entry.with_recipe(smysl::recipe_short(&r)),
            None => entry,
        };
        if let Err(e) = ledger.record(entry) {
            eprintln!("smysl ingest: the ledger could not be written: {e}");
        }
    }

    let root = project_root(global);
    if let Err(e) = smysl::stage::write(&root, &staged) {
        eprintln!("smysl ingest: {e}");
        return ExitCode::Failure;
    }

    // Rule M corrected the model, and a pipeline should be able to branch on that without
    // parsing `--json`. It refines the outcome rather than replacing it: the batch is intact
    // either way, and every lowered unit is in it at the status its grounds support.
    let corrected = !staged.weakened.is_empty();

    if m.get_flag("yes") {
        // 1.9 warned that this did not commit and said 1.10 would; this is 1.10. The warning
        // shipped a release ahead of the behaviour because implementing it turns a read-only
        // invocation into one that writes `--store`, and anyone passing `--yes` to suppress
        // exit 10 rather than to ask for a write had to be told first. The precedent is
        // `SMY-W432`: when a change moves somebody's data, the warning ships first.
        let path = store_target
            .as_deref()
            .expect("refused above without a store");
        let records = staged.records();
        let n = staged.len();
        if let Err(code) = commit_staged("ingest", path, &root, &records, &store, &store_labels) {
            return code;
        }
        println!("{n} unit(s) staged and committed to {path}");
        // `--yes` used to return 0 here whatever happened, so the one outcome most worth
        // knowing about - the model over-claimed and was corrected - was the outcome
        // indistinguishable from nothing having happened.
        return if corrected {
            ExitCode::StagedWithCorrections
        } else {
            ExitCode::Success
        };
    }
    println!(
        "{} unit(s) staged in {}; review, then `smysl merge --staged`",
        staged.len(),
        root.join(smysl::stage::PATH).display()
    );
    // Rule S: staged output awaits confirmation, and exit 10 is how a pipeline is told.
    if corrected {
        ExitCode::StagedWithCorrections
    } else {
        ExitCode::Staged
    }
}

/// `smysl attest` - semantic checks that require a model (§23.1). **Model-dependent.**
///
/// Never mutates cores: a judgement is a separate record, so it can be wrong, disputed, or
/// superseded without changing the uid of the claim it is about.
#[cfg(feature = "ingest")]
fn cmd_attest(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let Some(path) = store_arg(m, global) else {
        eprintln!("smysl attest: no store given");
        return ExitCode::Usage;
    };
    let (store, _) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl attest: {e}");
            return ExitCode::Failure;
        }
    };
    let registry = match load_registry(global) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl attest: {e}");
            return e.code;
        }
    };

    let what = m
        .get_one::<String>("what")
        .and_then(|s| smysl::What::parse(s))
        .unwrap_or(smysl::What::GistCoverage);
    let sample = match m.get_one::<String>("sample").map(String::as_str) {
        Some("all") => None,
        Some(n) => n.parse().ok().or(Some(10)),
        None => Some(10),
    };

    let opts = smysl::AttestOptions::new(what).with_sample(sample);
    let spin = Spinner::new(progress_style(global), format!("attesting {what}"));
    let outcome = smysl::attest(&store, &registry, &opts);
    spin.finish("");

    let report = match outcome {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl attest: {e}");
            return e.exit_code();
        }
    };

    for j in &report.judgements {
        let verdict = match j.holds {
            Some(true) => "yes",
            Some(false) => "NO ",
            // Recording an unreadable answer as a failure would manufacture evidence.
            None => "?  ",
        };
        println!("{verdict} {} {}", j.uid.short(), j.reason);
    }
    println!(
        "{path}: {} judged, {} failed, {} unreadable, {} token(s)",
        report.judgements.len(),
        report.failed().len(),
        report.unreadable,
        report.usage.total()
    );
    ExitCode::Success
}

#[cfg(not(feature = "ingest"))]
fn cmd_ingest(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl ingest: this build has no ingest layer (build with --features local)");
    ExitCode::Usage
}

#[cfg(not(feature = "ingest"))]
fn cmd_attest(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl attest: this build has no ingest layer (build with --features local)");
    ExitCode::Usage
}

/// Milliseconds since the epoch, for the usage ledger.
///
/// The one place the CLI reads a clock. Everything else takes its timestamp as an argument
/// so it stays reproducible; the ledger is a record of when things happened, so it does not.
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The project root: the directory holding `.smysl/`.
#[cfg(feature = "providers")]
fn project_root(global: &ArgMatches) -> std::path::PathBuf {
    root_beside(global.get_one::<String>("store").map(String::as_str))
}

/// `smysl providers` - what is configured, what is reachable, and what would egress
/// (§23.1, §29). **No content egress.**
#[cfg(feature = "providers")]
fn cmd_providers(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let style = progress_style(global);
    let registry = match load_registry(global) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl providers: {e}");
            return e.code;
        }
    };

    let ids = registry.ids();
    if ids.is_empty() {
        println!(
            "no providers configured; see {}",
            smysl::ProviderConfigFile::PATH
        );
        return ExitCode::Success;
    }

    // --tasks: what would leave the machine. Reported without contacting anything, so it
    // is safe to run when the answer is the reason you are asking.
    if m.get_flag("tasks") {
        println!("{:<20} {:<14} {:<10} command", "task", "provider", "egress");
        for row in registry.egress_report() {
            println!(
                "{:<20} {:<14} {:<10} {}",
                row.task,
                row.provider
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "-".into()),
                match (row.routed, row.leaves_machine) {
                    (false, _) => "unrouted",
                    (true, true) => "LEAVES",
                    (true, false) => "local",
                },
                row.task.command()
            );
        }
        if registry.is_offline() {
            println!("--offline: any task marked LEAVES will exit 7 rather than run");
        }
        return ExitCode::Success;
    }

    if !m.get_flag("probe") && !m.get_flag("models") {
        for id in &ids {
            let Some(p) = registry.get(id) else { continue };
            let c = p.caps();
            println!(
                "{id:<14} ctx {:<8} out {:<6} {:<12} {}",
                c.context_window,
                c.max_output,
                c.structured,
                if c.offline { "local" } else { "hosted" }
            );
        }
        println!("(--probe contacts them; --tasks reports what would egress)");
        return ExitCode::Success;
    }

    // Probing is one round trip per provider, so it gets a bar: a caller with six
    // providers and a slow network should be able to see which one it is waiting on. With
    // progress off, one line up front replaces it - a caller still wants to know that a
    // silent thirty seconds is a network wait rather than a hang.
    if !style.is_enabled() && ids.len() > 1 {
        eprintln!("smysl providers: probing {} provider(s)", ids.len());
    }
    let mut bar = Bar::new(style, "probing", ids.len());
    let mut rows: Vec<(smysl::ProviderId, String)> = Vec::new();
    let mut worst = ExitCode::Success;

    for id in &ids {
        bar.set_label(format!("probing {id}"));
        let Some(p) = registry.get(id) else { continue };

        let line = if registry.is_offline() && !p.caps().offline {
            // Probing a forbidden provider would be the network call --offline exists to
            // prevent, however harmless the payload.
            worst = worse(worst, ExitCode::Offline);
            format!("{id:<14} refused: --offline and this provider is hosted")
        } else {
            match p.probe() {
                Ok(probe) if probe.reachable => {
                    let c = probe.caps.unwrap_or_else(|| p.caps());
                    let models = if m.get_flag("models") {
                        format!("\n{:<14} models: {}", "", probe.models.join(", "))
                    } else {
                        String::new()
                    };
                    format!(
                        "{id:<14} up    ctx {:<8} out {:<6} {:<12} {:<7} {}{models}",
                        c.context_window,
                        c.max_output,
                        c.structured,
                        if c.offline { "local" } else { "hosted" },
                        probe.detail
                    )
                }
                Ok(probe) => {
                    worst = worse(worst, ExitCode::Provider);
                    format!("{id:<14} down  {}", probe.detail)
                }
                Err(e) => {
                    worst = worse(worst, e.exit_code());
                    format!("{id:<14} error {e}")
                }
            }
        };
        rows.push((id.clone(), line));
        bar.tick();
    }
    bar.finish(&format!("probed {} provider(s)", ids.len()));

    for (_, line) in rows {
        println!("{line}");
    }
    worst
}

/// `smysl usage` - the token ledger (§23.1). Informational; caps never block.
#[cfg(feature = "providers")]
fn cmd_usage(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = project_file(global, smysl::Ledger::PATH);
    let mut ledger = smysl::Ledger::open(&path);

    if m.get_flag("reset") {
        let n = ledger.entries().len();
        if let Err(e) = ledger.reset() {
            eprintln!("smysl usage: {e}");
            return ExitCode::Failure;
        }
        println!("{}: discarded {n} entr(ies)", path.display());
        return ExitCode::Success;
    }

    if ledger.is_empty() {
        println!("{}: no model calls recorded", path.display());
        return ExitCode::Success;
    }

    let by = m
        .get_one::<String>("by")
        .and_then(|s| smysl::GroupBy::parse(s))
        .unwrap_or_default();
    let since = m
        .get_one::<String>("since")
        .and_then(|s| s.parse::<u64>().ok());

    let rows = ledger.totals(by, since);
    for row in &rows {
        println!("{row}");
    }
    let total: u64 = rows.iter().map(|r| r.total()).sum();
    let calls: u64 = rows.iter().map(|r| r.calls).sum();
    println!("{:-<24} {calls:>6} call(s)  {total:>9} tokens", "");

    // `SMY-W305`, finally emitted. The ledger has recorded `estimated` per entry since the
    // provider layer landed, and nothing ever surfaced it — so a total built partly from
    // our own token estimate was indistinguishable from one the provider reported, and the
    // number was read as authoritative either way. It is a warning rather than a footnote
    // because the difference matters when someone is reconciling a bill.
    let estimated = ledger.entries().iter().filter(|e| e.estimated).count();
    if estimated > 0 {
        eprintln!(
            "smysl usage: warning: SMY-W305: {estimated} of {} entr(ies) carry an estimated \
             token count rather than one the provider reported",
            ledger.entries().len()
        );
    }
    ExitCode::Success
}

#[cfg(not(feature = "providers"))]
fn cmd_providers(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl providers: this build has no provider layer (build with --features local)");
    ExitCode::Usage
}

#[cfg(not(feature = "providers"))]
fn cmd_usage(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl usage: this build has no provider layer (build with --features local)");
    ExitCode::Usage
}

/// `smysl compact` - drop superseded units that nothing needs.
///
/// **Never in place.** The log is grow-only, which is what makes merge a join-semilattice,
/// so compaction is not an operation inside that algebra - it is a lossy projection to a new
/// store. A compaction that turns out to be wrong is not undoable from its own output, and a
/// peer that never compacted will bring the dropped records back on the next merge.
fn cmd_compact(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = match m
        .get_one::<String>("path")
        .or_else(|| global.get_one::<String>("store"))
    {
        Some(p) => p.clone(),
        None => {
            eprintln!("smysl compact: no store given");
            return ExitCode::Usage;
        }
    };
    // A CBOR log is opened as it is on disk, repeats included, so compaction can see and remove
    // them; `load_store` builds a store that already holds each record once.
    let is_log = path != "-"
        && std::fs::read(&path)
            .map(|b| !looks_like_surface(&b))
            .unwrap_or(false);
    let loaded = if is_log {
        Store::open(&path).map_err(|e| format!("{path}: {e}"))
    } else {
        load_memory(&path).map(|(s, _)| s)
    };
    let store = match loaded {
        Ok(s) => s,
        Err(e) => {
            eprintln!("smysl compact: {e}");
            return ExitCode::Failure;
        }
    };

    let out = smysl::compact(&store);
    if out.duplicates > 0 {
        eprintln!(
            "smysl compact: {} record(s) the log held more than once removed",
            out.duplicates
        );
    }
    if !out.still_referenced.is_empty() {
        eprintln!(
            "smysl compact: {} superseded unit(s) kept because something still points at \
             them; `smysl relink` moves those references first",
            out.still_referenced.len()
        );
    }
    if !out.retracted.is_empty() {
        eprintln!(
            "smysl compact: {} retracted unit(s) kept; a retraction must outlive what it \
             retracts",
            out.retracted.len()
        );
    }
    if out.is_empty() {
        eprintln!("smysl compact: nothing to drop");
        return ExitCode::Success;
    }
    eprintln!(
        "smysl compact: {} unit(s) dropped, {} of {} record(s) removed",
        out.dropped.len(),
        out.saved(),
        out.records_before
    );

    if m.get_flag("dry-run") {
        return ExitCode::Success;
    }

    // Refusing to overwrite the input is the point: this is the one command whose output
    // cannot reconstruct its input.
    let Some(dest) = global.get_one::<String>("output") else {
        eprintln!(
            "smysl compact: refusing to write to stdout without -o; compaction is not \
             reversible from its own output"
        );
        return ExitCode::Usage;
    };
    if dest == &path {
        eprintln!("smysl compact: refusing to overwrite the store being compacted");
        return ExitCode::Usage;
    }
    let bytes = Store::from_records(out.records).log_bytes();
    if let Err(e) = std::fs::write(dest, &bytes) {
        eprintln!("smysl compact: {dest}: {e}");
        return ExitCode::Failure;
    }
    ExitCode::Success
}

/// `smysl relink` - re-point references onto the units that replaced their targets.
///
/// Identity is content, so correcting a unit produces a *different* unit and whatever rested
/// on the original still rests on the original. Within one document that never shows,
/// because references are labels and uids are recomputed on parse. Across stores it does.
///
/// Append-only throughout: the corrected units are new records carrying `supersedes`, so the
/// old reading stays available and rule U still holds.
fn cmd_relink(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = match m
        .get_one::<String>("path")
        .or_else(|| global.get_one::<String>("store"))
    {
        Some(p) => p.clone(),
        None => {
            eprintln!("smysl relink: no store given");
            return ExitCode::Usage;
        }
    };
    let (store, _) = match load_memory(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("smysl relink: {e}");
            return ExitCode::Failure;
        }
    };

    let out = smysl::relink(&store);
    for (holder, target) in &out.dangling {
        eprintln!(
            "smysl relink: {} names {}, which this store has never held",
            &holder.to_string()[..14],
            &target.to_string()[..14]
        );
    }
    // A fork is a disagreement about what replaced what. Choosing a side would be
    // adjudicating it, which merge refuses to do and so does this.
    for target in &out.forked {
        eprintln!(
            "smysl relink: {} has two replacements and no ordering between them; \
             resolve the contention first",
            &target.to_string()[..14]
        );
    }

    if out.is_empty() {
        eprintln!("smysl relink: nothing to re-point");
        // Exit 5 for a fork: it *is* an unresolved contention, and the same code merge
        // uses for one means a pipeline does not need a second convention to read.
        return if !out.forked.is_empty() {
            ExitCode::Contentions
        } else if out.needs_attention() {
            ExitCode::CheckErrors
        } else {
            ExitCode::Success
        };
    }
    eprintln!("smysl relink: {} unit(s) re-pointed", out.moved.len());

    if m.get_flag("dry-run") {
        for (was, now) in &out.moved {
            eprintln!("  {} -> {}", &was.to_string()[..14], &now.to_string()[..14]);
        }
        return ExitCode::Success;
    }

    let mut records: Vec<Record> = store.iter().cloned().collect();
    records.extend(out.records);
    let relinked = Store::from_records(records);
    let bytes = relinked.log_bytes();
    match global.get_one::<String>("output") {
        Some(p) => {
            if let Err(e) = std::fs::write(p, &bytes) {
                eprintln!("smysl relink: {p}: {e}");
                return ExitCode::Failure;
            }
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            if stdout.write_all(&bytes).is_err() {
                return ExitCode::Failure;
            }
        }
    }
    ExitCode::Success
}

/// `smysl import` - tabular readings to `measured` units.
///
/// The only command that produces `measured`, and the only producer of units that consults
/// no model: it transcribes a file rather than interpreting one. That is what earns it the
/// top of the status ladder, and why the attestation it writes is `op: Imported` at the
/// `computed` rung - the licence, not a decoration.
#[cfg(feature = "ingest")]
fn cmd_import(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::{from_csv, ImportOptions};

    let path = m.get_one::<String>("file").expect("required");
    let text = if path == "-" {
        let mut s = String::new();
        if let Err(e) = std::io::Read::read_to_string(&mut std::io::stdin(), &mut s) {
            eprintln!("smysl import: {e}");
            return ExitCode::Failure;
        }
        s
    } else {
        match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("smysl import: {path}: {e}");
                return ExitCode::Failure;
            }
        }
    };

    let agent = match smysl::AgentId::new("tool:smysl-import") {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl import: {e}");
            return ExitCode::Failure;
        }
    };
    // Advanced from zero rather than read raw: `Hlc::now` takes the previous stamp, so an
    // import records a clock that is a function of this run and nothing earlier.
    let now = smysl::Hlc::now(&smysl::Hlc::zero(agent.clone()), &agent);
    let mut opts = ImportOptions::new(path.clone(), agent.clone(), now);
    if let Some(kind) = m
        .get_one::<String>("kind")
        .and_then(|k| smysl::SourceKind::parse(k))
    {
        opts.kind = kind;
    }
    if let Some(keys) = m.get_many::<String>("key") {
        opts.key = keys.cloned().collect();
    }

    let out = from_csv(&text, &opts);
    for d in &out.diagnostics {
        eprintln!("smysl import: {d}");
    }
    if out.is_empty() {
        eprintln!("smysl import: nothing to import");
        return ExitCode::Failure;
    }

    // Emitted as a store, the same way `merge` does: an import is records like any other,
    // and a caller pipes it into `merge` or `check` without a second format to learn.
    let store = Store::from_records(out.records());
    let bytes = store.log_bytes();
    match global.get_one::<String>("output") {
        Some(p) => {
            if let Err(e) = std::fs::write(p, &bytes) {
                eprintln!("smysl import: {p}: {e}");
                return ExitCode::Failure;
            }
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            if stdout.write_all(&bytes).is_err() {
                return ExitCode::Failure;
            }
        }
    }
    eprintln!(
        "smysl import: {} measured unit(s) from {path}",
        out.units.len()
    );
    ExitCode::Success
}

#[cfg(not(feature = "ingest"))]
fn cmd_import(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl import: this build has no ingest layer (build with --features local)");
    ExitCode::Usage
}

/// `smysl ui` - the seven-pane browser (§26).
///
/// A terminal is required, and refusing early is the point: a full-screen program started
/// on a pipe leaves escape sequences in whatever was collecting the output, and the person
/// who finds them is rarely the person who ran it.
#[cfg(feature = "tui")]
fn cmd_ui(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use std::io::IsTerminal;

    let path = match m
        .get_one::<String>("path")
        .or_else(|| global.get_one::<String>("store"))
    {
        Some(p) => p.clone(),
        None => {
            eprintln!("smysl ui: no store given");
            return ExitCode::Usage;
        }
    };

    if !std::io::stdout().is_terminal() {
        eprintln!("smysl ui: stdout is not a terminal; the UI needs one");
        return ExitCode::Usage;
    }

    let (store, labels) = match load_store(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("smysl ui: {e}");
            return ExitCode::Failure;
        }
    };

    match smysl::tui::run(smysl::tui::App::with_store(store, labels)) {
        Ok(()) => ExitCode::Success,
        Err(e) => {
            eprintln!("smysl ui: {e}");
            ExitCode::Failure
        }
    }
}

#[cfg(not(feature = "tui"))]
fn cmd_ui(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl ui: this build has no TUI (build with --features tui)");
    ExitCode::Usage
}

/// How progress should be reported for this invocation.
///
/// One place, so a command cannot draw in one branch and stay silent in another.
fn progress_style(global: &ArgMatches) -> Style {
    // `--noprogress` is absolute: it does not depend on whether stderr is a terminal, so a
    // caller can turn drawing off without also having to redirect anything.
    if global.get_flag("noprogress") {
        return Style::silent();
    }
    Style::detect(
        global.get_flag("quiet"),
        global.get_flag("json"),
        global.get_flag("no-color"),
    )
}

/// `smysl reindex` - rebuild the derived index from the log alone (§23.1).
///
/// The index is never authoritative, so this can always be run and never loses anything.
/// `--verify` is the interesting mode: it asserts that a rebuild reproduces the sidecar
/// byte for byte, which is how the two index paths are kept from drifting apart.
fn cmd_reindex(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let path = match m
        .get_one::<String>("store")
        .or_else(|| global.get_one::<String>("store"))
    {
        Some(p) => p.clone(),
        None => {
            eprintln!("smysl reindex: no store given");
            return ExitCode::Usage;
        }
    };

    #[cfg(feature = "store-redb")]
    if let Some(opts) = redb_options() {
        return reindex_redb(&path, opts, m.get_flag("verify"));
    }

    let (store, open) = match Store::open_with(&path, StoreOptions::default()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl reindex: {path}: {e}");
            return ExitCode::Failure;
        }
    };
    for d in open.report.iter() {
        eprintln!("{path}: {d}");
    }

    // `open_with` has just built the store from the log alone, which is everything
    // `Store::reindex` would do again (TX-P13a step 2): the index of a store fresh from its
    // log *is* the rebuild. The second absorb doubled the command for nothing.
    let rebuilt = store.index().to_bytes();

    if m.get_flag("verify") {
        let existing = std::fs::read(Store::index_path(std::path::Path::new(&path)));
        return match existing {
            Ok(bytes) if bytes == rebuilt => {
                println!("{path}: index matches a rebuild ({} bytes)", rebuilt.len());
                ExitCode::Success
            }
            Ok(_) => {
                eprintln!("{path}: the sidecar does not match a rebuild from the log");
                ExitCode::HashVerification
            }
            Err(e) => {
                eprintln!("{path}: no index to verify: {e}");
                ExitCode::Failure
            }
        };
    }

    if let Err(e) = store.write_index() {
        eprintln!("smysl reindex: {path}: {e}");
        return ExitCode::Failure;
    }
    println!(
        "{path}: {} records, {} units, index {} bytes",
        store.len(),
        store.units().count(),
        rebuilt.len()
    );
    ExitCode::Success
}

/// `smysl reindex --backend redb`: build the unit store from the log, or with `--verify` check
/// the one there against an in-memory rebuild (SMYSL-2.8 §4.4).
///
/// The verify compares what a store is for — the record-set digest, the units and their
/// state, the adjacency — not the file, which is not byte-reproducible (§3.5.6). A store that
/// had to be rebuilt to open did not describe its log, and fails the verify the way a sidecar
/// that does not match its rebuild does: exit 9.
#[cfg(feature = "store-redb")]
fn reindex_redb(path: &str, mut opts: smysl::redb::RedbOptions, verify: bool) -> ExitCode {
    use smysl::redb::RedbStore;
    opts.force_rebuild = !verify;
    let (store, report) = match RedbStore::open(path, opts) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl reindex: {path}: {e}");
            return ExitCode::Failure;
        }
    };
    if !verify {
        println!(
            "{path}: {} records, {} units, unit store rebuilt",
            store.len(),
            store.unit_count()
        );
        return ExitCode::Success;
    }
    for d in report.report.iter() {
        eprintln!("{path}: {d}");
    }
    if report.index_rebuilt {
        eprintln!("{path}: the unit store did not describe the log, and was rebuilt to open");
        return ExitCode::HashVerification;
    }
    let memory = match Store::open(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("smysl reindex: {path}: {e}");
            return ExitCode::Failure;
        }
    };
    let differs: Vec<&str> = [
        (
            "record-set digest",
            store.record_set_digest() == memory.record_set_digest(),
        ),
        (
            "units",
            store.unit_count() == StoreRead::unit_count(&memory),
        ),
        ("state", store.state_hash() == memory.state_hash()),
        (
            "adjacency",
            store.adjacency() == StoreRead::adjacency(&memory),
        ),
    ]
    .into_iter()
    .filter(|(_, same)| !same)
    .map(|(what, _)| what)
    .collect();
    if differs.is_empty() {
        println!(
            "{path}: unit store matches a rebuild ({} records, {} units)",
            store.len(),
            store.unit_count()
        );
        ExitCode::Success
    } else {
        eprintln!(
            "{path}: the unit store differs from a rebuild from the log in: {}",
            differs.join(", ")
        );
        ExitCode::HashVerification
    }
}

/// Gated to match its only caller, `cmd_ingest`. Without the gate this is dead code in
/// every build that leaves `ingest` out — which CI compiles with `-D warnings`, so the
/// build failed rather than warned. The determinism job builds exactly that configuration
/// (`--no-default-features --features cli`) to run its permutations, so it reported "rule D
/// failed" when nothing about determinism was wrong at all: the binary would not compile.
#[cfg(feature = "ingest")]
fn read_input(path: &str) -> std::io::Result<String> {
    if path == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s)?;
        Ok(s)
    } else {
        std::fs::read_to_string(path)
    }
}

fn worse(a: ExitCode, b: ExitCode) -> ExitCode {
    if b.as_i32() > a.as_i32() {
        b
    } else {
        a
    }
}

fn main() -> ProcExitCode {
    let matches = cli().get_matches();
    let Some((name, sub)) = matches.subcommand() else {
        return ProcExitCode::from(ExitCode::Usage.as_i32() as u8);
    };

    // `==` here survives being mutated to `!=`, and that is *equivalent* rather than untested.
    //
    // `cmd` is read in exactly one place — the `_ =>` arm at the bottom of the dispatch, which
    // no input reaches while every command in `COMMANDS` has an arm above it. Flipping the
    // comparison selects the wrong `Cmd`, and nothing ever looks at it. The `else` is
    // unreachable for the same reason from the other side: clap only yields subcommand names
    // that `cli()` registered, and `cli()` registers exactly `COMMANDS`.
    //
    // Recorded the way 0.13 recorded `worse`'s `>=` and 1.1 recorded `Lineage::is_empty`, rather
    // than answered with a test that cannot distinguish the two programs. What *is* tested is
    // the property this lookup exists to support: `tests/dispatch.rs` runs all twenty-five
    // commands and fails if any is unrouted.
    let Some(cmd) = COMMANDS.iter().find(|c| c.name == name) else {
        eprintln!("smysl: unknown command `{name}`");
        return ProcExitCode::from(ExitCode::Usage.as_i32() as u8);
    };

    // `--backend`, read once and kept, because the functions that load a store are called from
    // thirty places that were never handed the global matches (SMYSL-2.8 §4.4).
    if matches.get_one::<String>("backend").map(String::as_str) == Some("redb") {
        if !cfg!(feature = "store-redb") {
            eprintln!(
                "smysl {name}: --backend redb needs a build with `--features store-redb`; \
                 this one reads every log into memory"
            );
            return ProcExitCode::from(ExitCode::Usage.as_i32() as u8);
        }
        let _ = BACKEND.set(Backend::Redb(name.to_string()));
    }
    if let Some(shards) = matches.get_many::<String>("shard") {
        let _ = SHARDS.set(shards.cloned().collect());
    }

    // Rule D, asserted per invocation and refused before anything runs. A caller passing
    // `--seed-check` is saying "I am about to depend on these bytes"; the useful moment to be
    // told otherwise is before the command spends anything, not after it has written output
    // the caller now believes is reproducible.
    //
    // Running twice and comparing bytes would be a stronger assertion. It belongs to `sq`
    // (`SMY-E416`), and claiming it here on the strength of a label would be the same mistake
    // the flag already made.
    if matches.get_flag("seed-check") {
        match reproducible(cmd, sub) {
            Some(true) => {}
            Some(false) => {
                eprintln!(
                    "smysl {name}: --seed-check asserts rule D, and this invocation is {}{}",
                    cmd.purity.tag(),
                    match cmd.impure_when {
                        Some(w) => format!(" ({w})"),
                        None => String::new(),
                    }
                );
                return ProcExitCode::from(ExitCode::Usage.as_i32() as u8);
            }
            // A `Mixed` command the judgement does not know how to narrow. Refused rather than
            // waved through: the whole point of the flag is that it does not take a label's
            // word for it.
            None => {
                eprintln!(
                    "smysl {name}: --seed-check cannot tell whether this invocation is \
                     reproducible; {name} is mixed and the dispatcher does not know which of \
                     its invocations are pure"
                );
                return ProcExitCode::from(ExitCode::Usage.as_i32() as u8);
            }
        }
    }

    // One place, before anything runs, because the alternative is 26 commands each remembering
    // to check — and 23 of them did not. The message says what the command *does* write, since
    // a refusal that only says no leaves the caller to guess which flag they wanted.
    if let Some(want) = matches.get_one::<String>("format") {
        if !cmd.forms.contains(&want.as_str()) {
            if want == "pack" {
                eprintln!(
                    "smysl {name}: --format pack is not available - a pack is a lossy \
                     selection, and only `lib export` writes one. Use `smysl pack` for a \
                     budgeted selection of a store."
                );
            } else if cmd.forms.is_empty() {
                eprintln!(
                    "smysl {name}: --format is not honoured here - {name} does not write a \
                     document. Use --json if you are parsing its output."
                );
            } else {
                eprintln!(
                    "smysl {name}: --format {want} is not available - {name} writes a store \
                     log, which has no surface spelling. Pipe it through `smysl fmt` to read \
                     it as text."
                );
            }
            return ProcExitCode::from(ExitCode::Usage.as_i32() as u8);
        }
    }

    let code = match name {
        "fmt" => cmd_fmt(sub, &matches),
        "check" => cmd_check(sub, &matches),
        "merge" => cmd_merge(sub, &matches),
        "diff" => cmd_diff(sub, &matches),
        "trace" => cmd_trace(sub, &matches),
        "view" => cmd_view(sub, &matches),
        "bundle" => cmd_bundle(sub, &matches),
        "salience" => cmd_salience(sub, &matches),
        "find" => cmd_find(sub, &matches),
        "pack" => cmd_pack(sub, &matches),
        "retract" => cmd_retract(sub, &matches),
        "commit" => cmd_commit(sub, &matches),
        "withdraw" => cmd_withdraw(sub, &matches),
        "resolve" => cmd_resolve(sub, &matches),
        "review" => cmd_review(sub, &matches),
        "thread" => cmd_thread(sub, &matches),
        "render" => cmd_render(sub, &matches),
        "ingest" => cmd_ingest(sub, &matches),
        "attest" => cmd_attest(sub, &matches),
        "providers" => cmd_providers(sub, &matches),
        "usage" => cmd_usage(sub, &matches),
        "reindex" => cmd_reindex(sub, &matches),
        "import" => cmd_import(sub, &matches),
        "relink" => cmd_relink(sub, &matches),
        "compact" => cmd_compact(sub, &matches),
        "ui" => cmd_ui(sub, &matches),
        "text" => cmd_text(sub, &matches),
        "date" => cmd_date(sub, &matches),
        "same-as" => cmd_same_as(sub, &matches),
        "link" => cmd_link(sub, &matches),
        "summarise" => cmd_summarise(sub, &matches),
        "q" => cmd_q(sub, &matches),
        "ask" => cmd_ask(sub, &matches),
        "measure" => cmd_measure(sub, &matches),
        "assess" => cmd_assess(sub, &matches),
        "lib" => cmd_lib(sub, &matches),
        "schema" => cmd_schema(sub, &matches),
        "migrate" => cmd_migrate(sub, &matches),
        _ => {
            // Unreachable while every command in `COMMANDS` has an arm above, which
            // `command_table_matches_section_23` enforces. Kept as the honest answer if one
            // is ever added to the table and not to the dispatch.
            //
            // It used to cite "RFC SMYSL-1 §26". That RFC was retired in 0.5.0, and a user
            // facing an error should not be sent to a document that no longer governs
            // anything.
            eprintln!(
                "smysl {}: declared in the command table but not wired in this build ({})",
                cmd.name, cmd.phase
            );
            ExitCode::Failure
        }
    };
    ProcExitCode::from(code.as_i32() as u8)
}

/// The resolver `check --library` supplies, or `None` when the flag was not given.
///
/// `Some(Err(..))` rather than a silent fall-back when the flag is given and the path is not a
/// library: a caller who asked for the objects to be verified and got a report that did not
/// verify them would read it as "the objects are fine".
#[cfg(feature = "text")]
fn library_resolver(
    m: &ArgMatches,
    path: &str,
) -> Option<
    Result<
        std::sync::Arc<dyn smysl::PartResolver + Send + Sync + core::panic::RefUnwindSafe>,
        ExitCode,
    >,
> {
    use smysl::text::library::Library;
    if !m.get_flag("library") {
        return None;
    }
    let root = std::path::Path::new(path);
    Some(match Library::open(root) {
        Ok(l) => Ok(std::sync::Arc::new(l)),
        Err(e) => {
            eprintln!("smysl check: --library {path}: {e}");
            Err(ExitCode::Usage)
        }
    })
}

#[cfg(not(feature = "text"))]
fn library_resolver(
    m: &ArgMatches,
    _path: &str,
) -> Option<
    Result<
        std::sync::Arc<dyn smysl::PartResolver + Send + Sync + core::panic::RefUnwindSafe>,
        ExitCode,
    >,
> {
    if m.get_flag("library") {
        eprintln!("smysl check: --library needs the library layer (build with --features text)");
        return Some(Err(ExitCode::Usage));
    }
    None
}

// ---------------------------------------------------------------------------
// `text` — the library (RFC SMYSL-2.4, TX-P1 step 6)
// ---------------------------------------------------------------------------

/// `-s/--store` pointing at a library root rather than at a log.
///
/// No new global flag, per §4.4: a library is recognised by its `LIBRARY` marker, and a plain
/// file path keeps the meaning it has had since 1.0. The marker is a file rather than a naming
/// convention precisely so a mistyped directory is an error instead of an empty library.
#[cfg(feature = "text")]
fn library_root(
    m: &ArgMatches,
    global: &ArgMatches,
    cmd: &str,
) -> Result<std::path::PathBuf, ExitCode> {
    let path = store_path(m, global, cmd)?;
    Ok(std::path::PathBuf::from(path))
}

#[cfg(feature = "text")]
fn cmd_text(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let action = m
        .get_one::<String>("action")
        .map(String::as_str)
        .unwrap_or("ls");
    let root = match library_root(m, global, "text") {
        Ok(r) => r,
        Err(code) => return code,
    };
    match action {
        "add" => text_add(m, &root),
        "append" => text_append(m, &root),
        "ls" => text_ls(m, &root),
        "show" => text_show(m, &root),
        "align" => text_align(m, &root),
        "redact" => text_redact(m, &root),
        "index" => text_index(m, &root),
        other => {
            eprintln!(
                "smysl text: `{other}` is not an action; it is add, append, ls, show, align, \
                 redact or index"
            );
            ExitCode::Usage
        }
    }
}

#[cfg(feature = "text")]
fn text_add(m: &ArgMatches, root: &std::path::Path) -> ExitCode {
    use smysl::text::library::{AddSpec, Library};
    use smysl::text::limits::Caps;
    use smysl::text::part::Policy;
    use smysl::text::readers::{Input, Params};

    let Some(file) = m.get_one::<String>("target") else {
        eprintln!("smysl text add: no file given");
        return ExitCode::Usage;
    };
    let (Some(reader), Some(alias), Some(licence)) = (
        m.get_one::<String>("reader"),
        m.get_one::<String>("alias"),
        m.get_one::<String>("licence"),
    ) else {
        eprintln!("smysl text add: --reader, --alias and --licence are required");
        eprintln!("  none of the three has a defensible default: the reader id and the part");
        eprintln!("  policy are recorded in the manifest and checked on re-read, and a missing");
        eprintln!("  licence is not `unknown` — it is a decision nobody made");
        return ExitCode::Usage;
    };

    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("smysl text add: {file}: {e}");
            return ExitCode::Failure;
        }
    };

    let mut spec = AddSpec::new(reader.clone(), alias.clone(), licence.clone());
    if let Some(lang) = m.get_one::<String>("lang") {
        match smysl::LangTag::new(lang) {
            Ok(t) => spec.lang = Some(t),
            Err(e) => {
                eprintln!("smysl text add: --lang {lang}: {e}");
                return ExitCode::Usage;
            }
        }
    }
    if let Some(carry) = m.get_one::<String>("carry") {
        match smysl::Carry::parse(carry) {
            Some(c) => spec.carry = c,
            None => {
                eprintln!("smysl text add: --carry {carry}: it is none, ref or text");
                return ExitCode::Usage;
            }
        }
    }
    if let Some(policy) = m.get_one::<String>("part-policy") {
        match Policy::parse(policy) {
            Some(p) => spec.policy = Some(p),
            None => {
                eprintln!("smysl text add: --part-policy {policy}: not a policy this build writes");
                return ExitCode::Usage;
            }
        }
    }
    // A name, not a map: `kjv`, `synodal`, `lxx`. What it buys is `text align`, which cannot
    // align two expressions that have not each said how they are numbered.
    if let Some(scheme) = m.get_one::<String>("versification") {
        spec.versification = Some(scheme.clone());
    }
    let mut params = Params::default();
    for pair in m
        .get_many::<String>("param")
        .map(|v| v.cloned().collect::<Vec<_>>())
        .unwrap_or_default()
    {
        let Some((k, v)) = pair.split_once('=') else {
            eprintln!("smysl text add: --param {pair}: a parameter is `key=value`");
            return ExitCode::Usage;
        };
        if let Err(e) = params.set(reader, k, v) {
            eprintln!("smysl text add: --param {pair}: {e}");
            return ExitCode::Usage;
        }
    }
    spec.params = params;

    // The key is read, or created, before the library is opened: creating it is the one part of
    // `--pseudonymise` the library layer deliberately cannot do (`smysl::text::secrets` says
    // why), and finding out that it cannot be written is better before a corpus is half added
    // than after.
    if m.get_flag("pseudonymise") {
        let fresh = || {
            let mut bytes = [0u8; 32];
            // A failure here is the operating system saying it has no randomness, which is not
            // a thing to paper over with a weaker key: the process ends instead.
            getrandom::fill(&mut bytes).expect("the operating system's randomness");
            bytes
        };
        match smysl::text::secrets::load_or_create(root, fresh) {
            Ok(key) => spec.pseudonym_key = Some(key),
            Err(e) => {
                eprintln!("smysl text add: --pseudonymise: {e}");
                return ExitCode::Failure;
            }
        }
    }

    let mut library = match Library::create(root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text add: {}: {e}", root.display());
            return ExitCode::Failure;
        }
    };
    // The library lock, then the shards in name order (§3.9.3). Held for the whole add, which
    // is why `Library::add` does not take it: a method that acquired its own would deadlock
    // with a caller that already holds one.
    let shards = library.shards();
    let _locks = match smysl::text::lock::acquire_all(root, &shards, "text add") {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text add: {e}");
            return ExitCode::Failure;
        }
    };

    // Under the lock, and before the append: a library this release has not indexed yet gets
    // its two object-derived tables built once, here, by the command that is already paying
    // for I/O - rather than by every later `text ls`. See `Library::complete_index`.
    library.complete_index();

    match library.add(&Input::new(&bytes), &spec, &Caps::DEFAULT) {
        Ok(added) => {
            println!("manifest  {}", added.mid.canonical());
            println!("alias     {alias}");
            println!("reader    {reader}");
            println!("bytes     {}", added.bytes);
            println!("parts     {}", added.parts.len());
            for (i, tid) in added.parts.iter().enumerate() {
                println!("  part {i}  {}", tid.canonical());
            }
            println!("objects   {} written", added.objects_written);
            persist_index(&library, "text add");
            // Rule Z. Said out loud because `objects 0 written` otherwise reads as "already
            // there", and the two are opposite facts: one means the library holds the text, the
            // other that it is forbidden to.
            if added.objects_redacted > 0 {
                println!(
                    "redacted  {} object(s) withheld: this text holds a part somebody redacted",
                    added.objects_redacted
                );
            }
            if added.lossy {
                println!("lossy     the reader dropped something the source carried");
            }
            if spec.pseudonym_key.is_some() {
                // The file, never the key. The path is relative to the library, which is also
                // what a refusal from `secrets` prints.
                println!(
                    "speakers  pseudonymised under {}",
                    smysl::text::secrets::PSEUDONYM
                );
            }
            ExitCode::Success
        }
        Err(e) => {
            eprintln!("smysl text add: {e}");
            ExitCode::Failure
        }
    }
}

/// `smysl text append <file> --alias A` — a new version of an expression (SMYSL-2.4 §3.1).
///
/// **`--alias` rather than a second positional**, which is how the RFC spells it
/// (`text append <alias> <file>`). `TARGET` is one positional and already means three things
/// depending on the action; making it mean two things *at once* for one of them would be the
/// shape nobody can read back out of `--help`. `text add <file> --alias A` and
/// `text append <file> --alias A` put the same two pieces of information in the same places,
/// which is the stronger argument.
///
/// Everything else — the reader, its parameters, the part policy, the language, the licence and
/// the carry mode — comes from the head manifest, so there is nothing else to pass.
#[cfg(feature = "text")]
fn text_append(m: &ArgMatches, root: &std::path::Path) -> ExitCode {
    use smysl::text::library::Library;
    use smysl::text::limits::Caps;
    use smysl::text::readers::Input;

    let Some(file) = m.get_one::<String>("target") else {
        eprintln!("smysl text append: no file given");
        return ExitCode::Usage;
    };
    let Some(alias) = m.get_one::<String>("alias") else {
        eprintln!("smysl text append: --alias names the expression to append to");
        eprintln!("  the reader, its parameters and the part policy come from its head");
        eprintln!("  manifest: an append that restated them could state them differently, and");
        eprintln!("  a second version cut by another policy cannot be compared with the first");
        return ExitCode::Usage;
    };
    for flag in ["reader", "licence", "lang", "carry", "part-policy"] {
        if m.contains_id(flag)
            && m.value_source(flag) == Some(clap::parser::ValueSource::CommandLine)
        {
            eprintln!("smysl text append: --{flag} is not an append's to set; it comes from the");
            eprintln!("  head manifest, which is what makes the two versions comparable");
            return ExitCode::Usage;
        }
    }

    let bytes = match std::fs::read(file) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("smysl text append: {file}: {e}");
            return ExitCode::Failure;
        }
    };

    // Loaded, never created. A `--pseudonymise` that minted a key here would mint a *different*
    // key from the one the expression's existing speakers were derived under, and every
    // appended message would get a second pseudonym for a person already in the corpus — which
    // is the exact failure `SMY-E450` exists to prevent, arrived at by way of the flag that was
    // supposed to prevent it.
    let key = if m.get_flag("pseudonymise") {
        match smysl::text::secrets::load(root) {
            Ok(Some(key)) => Some(key),
            Ok(None) => {
                eprintln!(
                    "smysl text append: --pseudonymise: this library has no {}",
                    smysl::text::secrets::PSEUDONYM
                );
                eprintln!("  a key is created by `text add --pseudonymise`, and an append cannot");
                eprintln!("  create one: a new key would give the same people new pseudonyms");
                return ExitCode::Failure;
            }
            Err(e) => {
                eprintln!("smysl text append: --pseudonymise: {e}");
                return ExitCode::Failure;
            }
        }
    } else {
        None
    };

    let mut library = match Library::open(root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text append: {}: {e}", root.display());
            return ExitCode::Failure;
        }
    };
    let shards = library.shards();
    let _locks = match smysl::text::lock::acquire_all(root, &shards, "text append") {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text append: {e}");
            return ExitCode::Failure;
        }
    };

    // Under the lock, and before the append: a library this release has not indexed yet gets
    // its two object-derived tables built once, here, by the command that is already paying
    // for I/O - rather than by every later `text ls`. See `Library::complete_index`.
    library.complete_index();

    match library.append(&Input::new(&bytes), alias, key.as_ref(), &Caps::DEFAULT) {
        Ok(added) => match added.supersedes {
            Some(previous) => {
                println!("manifest  {}", added.mid.canonical());
                println!("supersedes {}", previous.canonical());
                println!("alias     {alias}");
                println!("bytes     {}", added.bytes);
                println!("parts     {}", added.parts.len());
                for (i, tid) in added.parts.iter().enumerate() {
                    println!("  part {i}  {}", tid.canonical());
                }
                // Two objects per part that moved. Printed because it is the one number that
                // says how much of the corpus this append touched, and the answer for a growing
                // text is "one part" however long the text is.
                println!("objects   {} written", added.objects_written);
                persist_index(&library, "text append");
                if added.objects_redacted > 0 {
                    println!(
                        "redacted  {} object(s) withheld: this text holds a part somebody \
                         redacted",
                        added.objects_redacted
                    );
                }
                if added.lossy {
                    println!("lossy     the reader dropped something the source carried");
                }
                if key.is_some() {
                    println!(
                        "speakers  pseudonymised under {}",
                        smysl::text::secrets::PSEUDONYM
                    );
                }
                ExitCode::Success
            }
            // The same corpus. No version was created, which is a result and not a failure:
            // re-running a sync before the export has grown is the ordinary way to find out
            // that it has not.
            None => {
                println!("manifest  {}", added.mid.canonical());
                println!("alias     {alias}");
                println!("unchanged no version was written: this file is the corpus already");
                ExitCode::Success
            }
        },
        Err(e) => {
            eprintln!("smysl text append: {e}");
            e.code_exit()
        }
    }
}

/// `smysl text redact <tid> --as AGENT` — rule Z (SMYSL-2.4 §3.1, §4.3.2).
///
/// Writes record 19 and then unlinks the bytes, in that order: a process killed between the two
/// leaves a catalog that has said what it is doing and an object store that has not caught up,
/// and opening the library is where that is put right.
///
/// What it does **not** do is retract anything. The manifests that name the part stay, the units
/// drawn from it stay, their spans and uids stay. A text that may no longer be held does not
/// stop having been read, and a corpus that quietly dropped the claims made from it would be
/// answering a legal demand by falsifying its own history.
#[cfg(feature = "text")]
fn text_redact(m: &ArgMatches, root: &std::path::Path) -> ExitCode {
    use smysl::text::library::Library;

    let Some(target) = m.get_one::<String>("target") else {
        eprintln!("smysl text redact: no tid given");
        return ExitCode::Usage;
    };
    let tid = match smysl::Tid::parse(target) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("smysl text redact: `{target}` is not a tid: {e}");
            eprintln!("  the full 52-character form, as `text add` and `text show` print it:");
            eprintln!("  an abbreviated identity in a redaction names no part or the wrong one");
            return ExitCode::Usage;
        }
    };
    let Some(agent_raw) = m.get_one::<String>("as") else {
        eprintln!("smysl text redact: --as names the agent issuing the redaction");
        eprintln!("  a redaction is an act by somebody, and a default would invent one");
        return ExitCode::Usage;
    };
    let agent = match AgentId::new(agent_raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl text redact: `{agent_raw}` is not an agent id: {e}");
            return ExitCode::Usage;
        }
    };

    let mut library = match Library::open(root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text redact: {}: {e}", root.display());
            return ExitCode::Failure;
        }
    };
    let reason = match m
        .get_one::<String>("reason")
        .map(|r| resolve(library.store(), r))
    {
        None => None,
        Some(Ok(u)) => Some(u),
        Some(Err(e)) => {
            eprintln!("smysl text redact: --reason: {e}");
            return e.code;
        }
    };

    let shards = library.shards();
    let _locks = match smysl::text::lock::acquire_all(root, &shards, "text redact") {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text redact: {e}");
            return ExitCode::Failure;
        }
    };

    // Under the lock, and before the append: a library this release has not indexed yet gets
    // its two object-derived tables built once, here, by the command that is already paying
    // for I/O - rather than by every later `text ls`. See `Library::complete_index`.
    library.complete_index();

    let at = record_time(m);
    let ts = smysl::Hlc::new(at, 0, agent.clone());
    match library.redact(&tid, &agent, ts, reason) {
        Ok(done) => {
            println!("redacted  {}", done.tid.canonical());
            println!("agent     {agent}");
            println!("objects   {} unlinked", done.objects_removed);
            println!("manifests {} still name this part", done.manifests);
            persist_index(&library, "text redact");
            if done.already {
                println!("already   this part was redacted before this call");
            }
            ExitCode::Success
        }
        Err(e) => {
            eprintln!("smysl text redact: {e}");
            e.code_exit()
        }
    }
}

/// Persist the substrate index after a command that grew the library.
///
/// A warning and never a failure. The operation the operator asked for has already succeeded —
/// the manifest is in the catalog and the objects are on disk — and the index is derived, so a
/// library whose sidecar could not be written is a library that rebuilds on its next open. The
/// warning is still worth printing: a corpus that pays a rebuild every time is usually a
/// directory somebody has made read-only, and that is a fact about the deployment rather than
/// about the corpus.
#[cfg(feature = "text")]
fn persist_index(library: &smysl::text::library::Library, cmd: &str) {
    if let Err(e) = library.write_index() {
        eprintln!("smysl {cmd}: the index was not written ({e}); it will be rebuilt on open");
    }
}

/// `text index` - what the substrate index holds, and `--rebuild` to write it (TX-P4 step 1).
///
/// Reporting and rebuilding in one action rather than two, because the report is what tells an
/// operator whether a rebuild is worth asking for. A table this build does not fill prints
/// `not built` and the step that will fill it, which is the distinction `Index::rows` returns
/// `None` for: an empty table and a table nobody wrote are different sentences, and the
/// difference matters most to someone reading `postings 0` and concluding their corpus is
/// unsearchable.
/// `smysl text align A B --scheme FILE` (TX-P4 step 4).
///
/// Installs the map into the library and rebuilds the `align` table. Installing is the whole
/// reason this writes anything: the table is derived, and §3.8's gate is that a rebuild from the
/// library alone reproduces it — so a map that stayed on the command line would make the table
/// reproducible only by whoever still had the command line.
#[cfg(feature = "text")]
fn text_align(m: &ArgMatches, root: &std::path::Path) -> ExitCode {
    use smysl::text::index::align;
    use smysl::text::library::Library;

    let (Some(a), Some(b)) = (m.get_one::<String>("target"), m.get_one::<String>("other")) else {
        eprintln!("smysl text align: two expressions are needed, as `text align kjv synodal`");
        return ExitCode::Usage;
    };

    let mut library = match Library::open(root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text align: {}: {e}", root.display());
            return ExitCode::Failure;
        }
    };
    let (Ok(mid_a), Ok(mid_b)) = (library.head(a), library.head(b)) else {
        eprintln!("smysl text align: `{a}` and `{b}` must each name one head of this library");
        return ExitCode::Usage;
    };

    // Both must say how they are numbered, and the message names the key: an expression that has
    // not said is not a failure of this command, it is a manifest that was written without the
    // field, and `text add --versification` is where that is fixed.
    for (who, mid) in [(a, &mid_a), (b, &mid_b)] {
        let declared = library
            .store()
            .manifest(mid)
            .and_then(|manifest| manifest.versification.clone());
        if declared.is_none() {
            eprintln!(
                "smysl text align: `{who}` declares no versification (manifest key 14), so there \
                 is nothing to align it by"
            );
            eprintln!("  `text add --versification SCHEME` records it when the text is added");
            return ExitCode::Usage;
        }
    }

    if let Some(file) = m.get_one::<String>("scheme") {
        let path = std::path::Path::new(file);
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            eprintln!("smysl text align: --scheme {file}: a map is named by its file");
            return ExitCode::Usage;
        };
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("smysl text align: --scheme {file}: {e}");
                return ExitCode::Failure;
            }
        };
        // Under the locks, like every other write to a library: installing a map changes what
        // the next rebuild produces, which is the same kind of change appending a record is.
        let shards = library.shards();
        let _locks = match smysl::text::lock::acquire_all(root, &shards, "text align") {
            Ok(l) => l,
            Err(e) => {
                eprintln!("smysl text align: {e}");
                return ExitCode::Failure;
            }
        };
        match smysl::text::versification::install(root, name, &text) {
            Ok(scheme) => println!(
                "installed  {name} ({} → {}, {} row(s))",
                scheme.from,
                scheme.to,
                scheme.len()
            ),
            Err(e) => {
                eprintln!("smysl text align: --scheme {file}: {e}");
                return ExitCode::Usage;
            }
        }
    }

    library.reindex();
    if let Err(e) = library.write_index() {
        eprintln!("smysl text align: {e}");
        return ExitCode::Failure;
    }

    let table = &library.index().align;
    let Some(rows) = table.get(&(mid_a, mid_b)) else {
        eprintln!(
            "smysl text align: no installed map connects the two numberings `{a}` and `{b}` \
             declare"
        );
        eprintln!("  --scheme FILE installs one; `text index` says what the table holds");
        return ExitCode::Failure;
    };
    println!("scheme     {}", rows.scheme);
    println!("aligned    {} locator(s)", rows.rows.len());
    // `SMY-W451` as a count rather than a line per locator: a psalter has thousands, and the
    // number is what says whether the map covers the corpus. The pairs are in the table for a
    // caller that wants them.
    println!("unaligned  {} locator(s) (SMY-W451)", rows.uncovered);
    if let Some(back) = table.get(&(mid_b, mid_a)) {
        println!("reverse    {} locator(s)", back.rows.len());
    }
    println!(
        "table      {} pair(s), {} row(s)",
        table.len(),
        align::rows(table)
    );
    ExitCode::Success
}

#[cfg(feature = "text")]
fn text_index(m: &ArgMatches, root: &std::path::Path) -> ExitCode {
    use smysl::text::index::Table;
    use smysl::text::library::Library;

    let mut library = match Library::open(root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text index: {}: {e}", root.display());
            return ExitCode::Failure;
        }
    };

    if m.get_flag("rebuild") {
        // A library-wide operation, so it takes the library lock and then the shard locks in
        // name order, as §3.9 requires of every one of them. Writing the sidecar without the
        // lock would race a concurrent `text add` that is maintaining the same file.
        let shards = library.shards();
        let _locks = match smysl::text::lock::acquire_all(root, &shards, "text index") {
            Ok(l) => l,
            Err(e) => {
                eprintln!("smysl text index: {e}");
                return ExitCode::Failure;
            }
        };
        library.reindex();
        if let Err(e) = library.write_index() {
            eprintln!("smysl text index: {e}");
            return ExitCode::Failure;
        }
        println!("rebuilt   {}", smysl::text::index::SIDECAR);
        // Two files when the build has the backing store, and the second one is where the
        // `postings` table went — so a reader of this output can see which file the next open
        // will read the rows out of.
        if library.has_postings_store() {
            println!("rebuilt   {}", smysl::text::index::STORE);
        }
    }

    for table in Table::ALL {
        // `postings` is the one table whose row count is not the whole story, and since the
        // backing store it is also the one table that need not be in the sidecar at all. So it
        // is asked of the **library** rather than of the resident index: the rows are terms,
        // what a reader needs beside them is how the corpus was divided and which chains
        // analysed it, and which file holds them decides what `--rebuild` would rewrite.
        if *table == Table::Postings {
            match library.postings_held() {
                Some(held) => println!(
                    "{:<11} {} term(s) across {} partition(s), analysed by {}{}",
                    table.as_str(),
                    held.terms,
                    held.partitions,
                    if held.analyzers.is_empty() {
                        "nothing".to_string()
                    } else {
                        held.analyzers.join(", ")
                    },
                    if held.stored {
                        format!(", in {}", smysl::text::index::STORE)
                    } else {
                        String::new()
                    }
                ),
                // Two reasons for a `None` and two remedies, which is why the sentence is not
                // one sentence: a table nobody built is a `--rebuild` away, and a table in a
                // file this binary has no reader for is a build away. Saying `--rebuild` to
                // the second would be advice that overwrites what it could not read.
                None if library.has_postings_store() && !cfg!(feature = "substrate-redb") => {
                    println!(
                        "{:<11} in {}, which this build cannot read; `--features substrate-redb`",
                        table.as_str(),
                        smysl::text::index::STORE
                    )
                }
                None => println!(
                    "{:<11} not built for this library; `text index --rebuild` builds it",
                    table.as_str()
                ),
            }
            continue;
        }
        match library.index().rows(*table) {
            Some(n) => println!("{:<11} {n} row(s)", table.as_str()),
            // One reason for a `None` now, where step 1 had two. A table this *library* has not
            // had built is one `--rebuild` away; a table this *build* has no builder for was a
            // phase step away, and as of TX-P4 step 4 there is no such table — every one of
            // §3.8's twelve is built, which `every_table_is_named_and_says_which_step_builds_it`
            // asserts by comparing `Table::BUILT` against `Table::ALL`.
            None => println!(
                "{:<11} not built for this library; `text index --rebuild` builds it",
                table.as_str()
            ),
        }
    }
    ExitCode::Success
}

#[cfg(feature = "text")]
fn text_ls(m: &ArgMatches, root: &std::path::Path) -> ExitCode {
    use smysl::text::library::Library;

    let library = match Library::open(root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text ls: {}: {e}", root.display());
            return ExitCode::Failure;
        }
    };
    let forks_only = m.get_flag("forks");
    let catalog = library.catalog();
    let mut shown = 0usize;
    for (alias, heads) in &catalog {
        if forks_only && heads.len() < 2 {
            continue;
        }
        shown += 1;
        // A fork is printed as a fork rather than resolved: `SMY-W418`, and the operator is
        // the one who decides which of two catalogue entries is the one they meant.
        let note = if heads.len() > 1 {
            "  (SMY-W418: fork)"
        } else {
            ""
        };
        println!("{alias}{note}");
        for mid in heads {
            let parts = library
                .store()
                .manifest(mid)
                .map(|man| man.parts.len())
                .unwrap_or(0);
            println!("  {}  {parts} part(s)", mid.canonical());
        }
    }
    if shown == 0 {
        eprintln!(
            "smysl text ls: {}",
            if forks_only {
                "no alias has more than one head"
            } else {
                "the catalog is empty"
            }
        );
    }
    ExitCode::Success
}

#[cfg(feature = "text")]
fn text_show(m: &ArgMatches, root: &std::path::Path) -> ExitCode {
    use smysl::text::library::Library;
    use smysl::text::limits::Caps;
    use smysl::text::locator;

    let Some(reference) = m.get_one::<String>("target") else {
        eprintln!("smysl text show: no reference given; it is `<alias|mid>#<locator>`");
        return ExitCode::Usage;
    };
    let Some((who, loc_text)) = reference.split_once('#') else {
        eprintln!("smysl text show: `{reference}` has no `#`; it is `<alias|mid>#<locator>`");
        return ExitCode::Usage;
    };
    let loc = match locator::parse(loc_text) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text show: `{loc_text}`: {e}");
            return ExitCode::Usage;
        }
    };

    let library = match Library::open(root) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl text show: {}: {e}", root.display());
            return ExitCode::Failure;
        }
    };
    let passage = match library.passage(who, &loc, &Caps::DEFAULT) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("smysl text show: {e}");
            return ExitCode::Failure;
        }
    };

    if m.get_flag("raw") {
        print!("{}", passage.text);
        return ExitCode::Success;
    }
    println!("manifest  {}", passage.mid.canonical());
    println!("part      {}", passage.tid.canonical());
    println!("range     {}..{}", passage.range.start, passage.range.end);
    println!();
    println!("{}", passage.text);

    if m.get_flag("segments") {
        // The reading rather than the structure: a node is a range, and a row is a range plus
        // who said it and when. For the six TX-P1 readers the two print alike, because none of
        // them fills those fields; for a chat reading the speaker is the thing somebody came
        // here to see, and a pseudonym is what `--pseudonymise` put there.
        match library.part_reading(&passage.mid, &passage.tid) {
            Ok(reading) => {
                println!();
                println!("segments");
                for row in &reading.rows {
                    if row.start < passage.range.start || row.end > passage.range.end {
                        continue;
                    }
                    let mut extra = String::new();
                    if let Some(who) = &row.speaker {
                        extra.push_str("  ");
                        extra.push_str(who);
                    }
                    if let Some(at) = row.observed {
                        extra.push_str(&format!("  {at}"));
                    }
                    println!(
                        "  {:<9} {} {}..{}{extra}",
                        row.level, row.locator, row.start, row.end
                    );
                }
            }
            Err(e) => {
                eprintln!("smysl text show: --segments: {e}");
                return ExitCode::Failure;
            }
        }
    }
    ExitCode::Success
}

#[cfg(not(feature = "text"))]
fn cmd_text(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl text: this build has no library layer (build with --features text)");
    ExitCode::Usage
}

// ---------------------------------------------------------------------------
// `date` (RFC SMYSL-2.4 §4.4, TX-P3 step 4)
// ---------------------------------------------------------------------------

#[cfg(feature = "text")]
fn cmd_date(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    match m.get_one::<String>("action").expect("required").as_str() {
        "set" => date_set(m, global),
        "order" => date_order(m, global),
        "show" => date_show(m, global),
        other => {
            eprintln!("smysl date: unknown action `{other}`");
            ExitCode::Usage
        }
    }
}

/// A millisecond range, spelled as the surface form spells it.
#[cfg(feature = "text")]
fn parse_window(raw: &str) -> Result<(u64, u64), String> {
    let Some((a, b)) = raw.split_once("..") else {
        return Err(format!("`{raw}` is not a range; write `FROM..TO`"));
    };
    // `u64`, because a window is matched against a unit's `observed` and that is a `u64`
    // millisecond clock. An offset is signed and a window is not: one is a correction, the
    // other a span of the record's own time.
    let from: u64 = a
        .trim()
        .parse()
        .map_err(|_| format!("`{a}` is not a millisecond instant"))?;
    let to: u64 = b
        .trim()
        .parse()
        .map_err(|_| format!("`{b}` is not a millisecond instant"))?;
    // Refused here rather than written and warned about later. `DatingTarget::selects_anything`
    // exists because the *format* cannot refuse an empty window — a record has to leave again
    // as it arrived — but a person typing one has made a mistake, and `SMY-W412` three commands
    // later is a worse answer than a message now.
    if from >= to {
        return Err(format!(
            "`{raw}` selects nothing: {from} is not before {to}"
        ));
    }
    Ok((from, to))
}

/// A target, in the three spellings a dating's target has.
#[cfg(feature = "text")]
fn date_target(
    store: &(impl StoreRead + ?Sized),
    raw: &str,
    window: Option<&String>,
) -> Result<smysl::DatingTarget, Unresolved> {
    use smysl::{DatingTarget, Mid, Tid};
    let usage = |message: String| Unresolved {
        code: ExitCode::Usage,
        message,
    };
    let range = match window {
        Some(w) => Some(parse_window(w).map_err(usage)?),
        None => None,
    };
    if raw.starts_with(Tid::PREFIX) {
        let tid = Tid::parse(raw).map_err(|e| usage(format!("`{raw}` is not a tid: {e}")))?;
        return Ok(match range {
            Some((from_ms, to_ms)) => DatingTarget::Window {
                tid,
                from_ms,
                to_ms,
            },
            None => DatingTarget::Part(tid),
        });
    }
    if raw.starts_with(Mid::PREFIX) {
        if range.is_some() {
            return Err(usage(
                "--window narrows a part; a manifest is already the units of its parts".into(),
            ));
        }
        let mid = Mid::parse(raw).map_err(|e| usage(format!("`{raw}` is not a mid: {e}")))?;
        return Ok(DatingTarget::Manifest(mid));
    }
    if range.is_some() {
        return Err(usage(
            "--window narrows a part target; a unit is one subject".into(),
        ));
    }
    Ok(DatingTarget::Unit(resolve(store, raw)?))
}

/// `--value`: an EDTF date, an offset, or a relation to another target.
///
/// The three are told apart by an exact prefix and never by "contains a colon": an EDTF
/// date-time carries two of them (`1984-01-01T12:30:00Z`), so a looser test would read a
/// perfectly good absolute value as a malformed offset.
#[cfg(feature = "text")]
fn date_value(
    store: &(impl StoreRead + ?Sized),
    raw: &str,
) -> Result<smysl::DatingValue, Unresolved> {
    use smysl::{Allen, DatingValue};
    let usage = |message: String| Unresolved {
        code: ExitCode::Usage,
        message,
    };
    if let Some(ms) = raw.strip_prefix("offset:") {
        let ms: i64 = ms
            .trim()
            .parse()
            .map_err(|_| usage(format!("`{ms}` is not a millisecond offset")))?;
        return Ok(DatingValue::Offset(ms));
    }
    for allen in Allen::ALL.iter().copied() {
        let prefix = format!("{}:", allen.as_str());
        if let Some(rest) = raw.strip_prefix(&prefix) {
            let target = date_target(store, rest.trim(), None)?;
            return Ok(DatingValue::Relative { allen, target });
        }
    }
    // `SMY-E410` at the writing end, which is what makes the check pass's own fixture for it a
    // Rust test: no document and no command can put a malformed value into a store.
    if !smysl::edtf::is_valid(raw) {
        return Err(Unresolved {
            code: ExitCode::Failure,
            message: format!(
                "`{raw}` is not an EDTF level-0 or level-1 value ({})",
                Code::E410
            ),
        });
    }
    Ok(DatingValue::Absolute(raw.to_string()))
}

#[cfg(feature = "text")]
fn axis_of(m: &ArgMatches) -> smysl::Axis {
    match m.get_one::<String>("axis").expect("has a default").as_str() {
        "composed" => smysl::Axis::Composed,
        "about" => smysl::Axis::About,
        _ => smysl::Axis::Said,
    }
}

#[cfg(feature = "text")]
fn date_set(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let Some(value) = m.get_one::<String>("value") else {
        eprintln!("smysl date set: --value is required");
        return ExitCode::Usage;
    };
    write_dating(m, global, "set", |store| date_value(store, value))
}

#[cfg(feature = "text")]
fn date_order(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    if m.get_one::<String>("value").is_some() {
        eprintln!("smysl date order: --value is what `order` writes for you");
        return ExitCode::Usage;
    }
    let (Some(relation), Some(other)) = (
        m.get_one::<String>("relation"),
        m.get_one::<String>("other"),
    ) else {
        eprintln!("smysl date order: write `date order <a> before <b>`");
        return ExitCode::Usage;
    };
    let allen = smysl::Allen::ALL
        .iter()
        .copied()
        .find(|a| a.as_str() == relation)
        .expect("clap restricts the values");
    write_dating(m, global, "order", |store| {
        // The second subject takes no `--window`: one flag cannot narrow two targets, and
        // `date set <a> --value during:<tid>` is there for the case that wants it.
        let target = date_target(store, other, None)?;
        Ok(smysl::DatingValue::Relative { allen, target })
    })
}

/// What `set` and `order` share: resolve, build record 17, append, report.
#[cfg(feature = "text")]
fn write_dating(
    m: &ArgMatches,
    global: &ArgMatches,
    action: &str,
    value: impl FnOnce(&Store) -> Result<smysl::DatingValue, Unresolved>,
) -> ExitCode {
    let path = match store_path(m, global, "date") {
        Ok(p) => p,
        Err(code) => return code,
    };
    warn_output_is_a_report(global, "date");
    let (store, labels) = match load_memory(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl date {action}: {e}");
            return ExitCode::Failure;
        }
    };
    let raw = m.get_one::<String>("target").expect("required");
    let target = match date_target(&store, raw, m.get_one::<String>("window")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("smysl date {action}: {e}");
            return e.code;
        }
    };
    let value = match value(&store) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl date {action}: {e}");
            return e.code;
        }
    };
    let Some(agent) = m.get_one::<String>("as") else {
        eprintln!("smysl date {action}: --as is required; a dating records who dated it");
        return ExitCode::Usage;
    };
    let agent = match AgentId::new(agent) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl date {action}: --as: {e}");
            return ExitCode::Usage;
        }
    };
    let basis = match m.get_one::<String>("basis").map(|r| resolve(&store, r)) {
        None => None,
        Some(Ok(u)) => Some(u),
        Some(Err(e)) => {
            eprintln!("smysl date {action}: --basis: {e}");
            return e.code;
        }
    };

    let at = record_time(m);
    let axis = axis_of(m);
    let mut dating = smysl::Dating::new(
        target.clone(),
        axis,
        value,
        agent.clone(),
        Hlc::new(at, 0, agent),
    );
    if let Some(u) = basis {
        dating = dating.with_basis(u);
    }
    let did = dating.did();
    let known = store.dating(&did).is_some();
    let dry = m.get_flag("dry-run");
    // A did is the hash of the dating, so re-running the same command names the record already
    // there. Saying so beats appending a byte-identical second one, which the log would carry
    // and which would answer the same.
    let apply = !dry && !known;
    if apply {
        let records = vec![Record::Dating(dating)];
        if let Err(e) = persist(&path, &records, &store, &labels) {
            eprintln!("smysl date {action}: {e}");
            return ExitCode::Failure;
        }
    }

    // What the dating selects *now*, which is the number a person wants back: a window over a
    // part with no units in range is accepted by the format and dates nothing, and that is
    // worth hearing at the moment of writing rather than from `check` afterwards.
    let selects = smysl::text::time::engine::subjects(&store, &target).len();
    if global.get_flag("json") {
        println!(
            "{{\"did\":{},\"axis\":{},\"selects\":{},\"applied\":{},\"known\":{}}}",
            smysl::json_escape(&did.canonical()),
            smysl::json_escape(axis.as_str()),
            selects,
            apply,
            known
        );
    } else {
        println!("{path}: {} {axis}  {did}", target_label(&target, &labels));
        println!("{path}:   selects {selects} subject(s)");
        if known {
            println!("{path}:   this dating is already recorded; nothing to do");
        } else if dry {
            println!("{path}:   --dry-run, so nothing was written");
        }
    }
    ExitCode::Success
}

/// How a target reads in a report, with a label where the store has one.
#[cfg(feature = "text")]
fn target_label(
    t: &smysl::DatingTarget,
    labels: &std::collections::BTreeMap<smysl::Label, Uid>,
) -> String {
    use smysl::DatingTarget;
    match t {
        DatingTarget::Unit(u) => labels
            .iter()
            .find(|(_, x)| *x == u)
            .map(|(l, _)| l.as_str().to_string())
            .unwrap_or_else(|| u.short()),
        DatingTarget::Part(tid) => tid.short(),
        DatingTarget::Manifest(mid) => mid.short(),
        DatingTarget::Window {
            tid,
            from_ms,
            to_ms,
        } => format!("{} [{from_ms}..{to_ms}]", tid.short()),
        _ => "an unknown target kind".into(),
    }
}

#[cfg(feature = "text")]
fn date_show(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::text::time::engine;
    let path = match store_path(m, global, "date") {
        Ok(p) => p,
        Err(code) => return code,
    };
    let (store, labels) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl date show: {e}");
            return ExitCode::Failure;
        }
    };
    let raw = m.get_one::<String>("target").expect("required");
    let target = match date_target(&store, raw, m.get_one::<String>("window")) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("smysl date show: {e}");
            return e.code;
        }
    };
    let axis = axis_of(m);
    let eff = engine::effective(&store, axis);
    let subjects = engine::subjects(&store, &target);
    let why = m.get_flag("why");

    let name = |s: &smysl::text::time::Subject| match s {
        smysl::text::time::Subject::Unit(u) => labels
            .iter()
            .find(|(_, x)| *x == u)
            .map(|(l, _)| l.as_str().to_string())
            .unwrap_or_else(|| u.short()),
        smysl::text::time::Subject::Manifest(mid) => mid.short(),
    };

    if global.get_flag("json") {
        let rows: Vec<String> = subjects
            .iter()
            .map(|s| {
                let d = eff.of(s);
                format!(
                    "{{\"subject\":{},\"interval\":{},\"lo\":{},\"hi\":{},\"contested\":{},\"why\":[{}]}}",
                    smysl::json_escape(&s.to_string()),
                    smysl::json_escape(&eff.interval(s).to_string()),
                    d.map(|d| smysl::json_escape(d.lo_status.as_str()))
                        .unwrap_or_else(|| "null".into()),
                    d.map(|d| smysl::json_escape(d.hi_status.as_str()))
                        .unwrap_or_else(|| "null".into()),
                    d.is_some_and(|d| d.contested),
                    d.map(why_json).unwrap_or_default()
                )
            })
            .collect();
        println!(
            "{{\"axis\":{},\"subjects\":[{}]}}",
            smysl::json_escape(axis.as_str()),
            rows.join(",")
        );
        return ExitCode::Success;
    }

    if subjects.is_empty() {
        println!(
            "{path}: {} names no subject in this store",
            target_label(&target, &labels)
        );
        return ExitCode::Success;
    }
    println!("{path}: {axis}");
    for s in &subjects {
        let Some(d) = eff.of(s) else {
            println!("  {}  undated", name(s));
            continue;
        };
        let mark = if d.contested { "  contested" } else { "" };
        println!(
            "  {}  {}  lo {} / hi {}{mark}",
            name(s),
            d.interval,
            d.lo_status,
            d.hi_status
        );
        if why {
            // Both ends, always, and labelled. The two differ often enough that printing one
            // chain would be printing the wrong one half the time: a `measured` instant under
            // a `cited` ceiling is an ordinary thing for a corpus to hold.
            for (end, w) in [("lo", d.why_lo), ("hi", d.why_hi)] {
                match w {
                    Some(w) => {
                        let via = match w.through {
                            Some(t) => format!(" through {}", name(&t)),
                            None => String::new(),
                        };
                        println!("    {end}: {} at {}{via}", w.cause, w.at);
                    }
                    None => println!("    {end}: nothing set this bound"),
                }
            }
        }
    }
    ExitCode::Success
}

#[cfg(feature = "text")]
fn why_json(d: &smysl::text::time::engine::Dated) -> String {
    let one = |end: &str, w: Option<smysl::text::time::engine::Why>| {
        w.map(|w| {
            format!(
                "{{\"end\":{},\"cause\":{},\"at\":{},\"through\":{}}}",
                smysl::json_escape(end),
                smysl::json_escape(&w.cause.to_string()),
                smysl::json_escape(w.at.as_str()),
                w.through
                    .map(|t| smysl::json_escape(&t.to_string()))
                    .unwrap_or_else(|| "null".into())
            )
        })
    };
    [one("lo", d.why_lo), one("hi", d.why_hi)]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(not(feature = "text"))]
fn cmd_date(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl date: this build has no time engine (build with --features text)");
    ExitCode::Usage
}

// ---------------------------------------------------------------------------
// `same-as` — proposing edges and counting classes (RFC SMYSL-2.4 §3.6, TX-P7 step 2)
// ---------------------------------------------------------------------------

/// `smysl same-as propose|classes`.
///
/// Two halves that look alike and are not. `classes` is a **read**: it counts what the store's
/// edges already add up to under A-12.4 and writes nothing. `propose` runs an engine and
/// **stages** what it finds, under rule S like any other batch, because a tool that accepted
/// its own proposals would make OQ-44 — may a lexical proposal be auto-accepted? — a question
/// nobody could ask.
///
/// `Purity::Pure` in the command table, and that is the whole of the claim: every engine in
/// this build is a computation over records. `semantic` is SMYSL-2.5's and is the value clap
/// refuses here; if it lands, this row becomes `Mixed` and §4.4's table says so already.
#[cfg(feature = "text")]
fn cmd_same_as(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let action = m
        .get_one::<String>("action")
        .map(String::as_str)
        .expect("required");
    let path = match store_path(m, global, "same-as") {
        Ok(p) => p,
        Err(code) => return code,
    };
    warn_output_is_a_report(global, "same-as");
    match action {
        "propose" => same_as_propose(m, global, &path),
        "classes" => same_as_classes(m, &path),
        other => {
            eprintln!("smysl same-as: unknown action `{other}`");
            ExitCode::Usage
        }
    }
}

#[cfg(not(feature = "text"))]
fn cmd_same_as(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl same-as: needs the library layer (build with --features text)");
    ExitCode::Usage
}

/// A library if the path is one, and the plain store otherwise.
///
/// Both are legitimate. `identical-span` and `lexical` read units and nothing else, so a store
/// of text units exported from a library is a fair thing to run them over; `anchored` reads an
/// alignment, which only a library has. The difference is reported where it bites rather than
/// refused up front.
#[cfg(feature = "text")]
// The labels are read only where a batch is committed, which is `ingest`'s. A build with
// `text` and no `ingest` answers the pure half of `same-as` and `link` and never commits, so
// the field is carried and unread there — which is the right shape: one `Corpus` for both
// builds, rather than two that could drift.
#[cfg_attr(not(feature = "ingest"), allow(dead_code))]
enum Corpus {
    Library(Box<smysl::text::library::Library>),
    // Both boxed, and the store for the reason clippy gives: a `Store` is some 2.6 KiB of
    // indexes, so an unboxed variant makes every `Corpus` that big whichever one it holds.
    Store(Box<Store>, std::collections::BTreeMap<smysl::Label, Uid>),
}

#[cfg(feature = "text")]
impl Corpus {
    fn open(path: &str) -> Result<Corpus, String> {
        let root = std::path::Path::new(path);
        if root.join("LIBRARY").exists() {
            return smysl::text::library::Library::open(root)
                .map(|l| Corpus::Library(Box::new(l)))
                .map_err(|e| e.to_string());
        }
        load_memory(path).map(|(s, l)| Corpus::Store(Box::new(s), l))
    }

    fn store(&self) -> &Store {
        match self {
            Corpus::Library(l) => l.store(),
            Corpus::Store(s, _) => s,
        }
    }

    /// The uids `--scope` names, or `None` for the whole corpus.
    ///
    /// A scope is written in aliases and mids, because that is how a person holds a corpus —
    /// and it is applied as a set of **units**, because that is what an engine takes. A unit is
    /// in scope when the manifest its span is an offset into is one of the named ones, which is
    /// the only link between a unit and an expression that A-2.4 lets a unit carry.
    fn scope(&self, names: &[String]) -> Result<std::collections::BTreeSet<Uid>, String> {
        use std::collections::BTreeSet;
        let mut mids: BTreeSet<smysl::text::Mid> = BTreeSet::new();
        for name in names {
            if let Ok(mid) = smysl::text::Mid::parse(name) {
                mids.insert(mid);
                continue;
            }
            match self {
                Corpus::Library(l) => match l.head(name) {
                    Ok(mid) => {
                        mids.insert(mid);
                    }
                    Err(e) => return Err(format!("--scope {name}: {e}")),
                },
                Corpus::Store(..) => {
                    return Err(format!(
                        "--scope {name}: an alias needs a library to resolve it; pass an `m3:` \
                         manifest id, or point --store at the library"
                    ))
                }
            }
        }
        let scope: BTreeSet<Uid> = self
            .store()
            .units()
            .filter(|(_, u)| {
                u.core
                    .source
                    .as_ref()
                    .and_then(|s| s.manifest())
                    .is_some_and(|m| mids.contains(m))
            })
            .map(|(uid, _)| *uid)
            .collect();
        if scope.is_empty() {
            return Err("--scope names no unit in this corpus".to_string());
        }
        Ok(scope)
    }
}

#[cfg(feature = "text")]
fn same_as_scope(
    m: &ArgMatches,
    corpus: &Corpus,
) -> Result<Option<std::collections::BTreeSet<Uid>>, ExitCode> {
    let Some(names) = m.get_many::<String>("scope") else {
        return Ok(None);
    };
    let names: Vec<String> = names.cloned().collect();
    match corpus.scope(&names) {
        Ok(s) => Ok(Some(s)),
        Err(e) => {
            eprintln!("smysl same-as: {e}");
            Err(ExitCode::Usage)
        }
    }
}

/// `propose` needs the staging layer, which `classes` does not.
///
/// Counting classes reads records; proposing stages a batch, and a batch is `smysl-ingest`'s.
/// A build with the library layer and no ingest crate can therefore answer the pure half and
/// says so about the other, rather than offering an action it cannot complete.
#[cfg(not(all(feature = "text", feature = "ingest")))]
#[cfg(feature = "text")]
fn same_as_propose(_m: &ArgMatches, _global: &ArgMatches, _path: &str) -> ExitCode {
    eprintln!(
        "smysl same-as propose: staging needs the ingest layer (build with --features ingest);          `same-as classes` is pure and works in this build"
    );
    ExitCode::Usage
}

#[cfg(all(feature = "text", feature = "ingest"))]
fn same_as_propose(m: &ArgMatches, global: &ArgMatches, path: &str) -> ExitCode {
    use smysl::text::proposition::{Anchored, Engine, IdenticalSpan, Lexical};

    let Some(engine) = m.get_one::<String>("engine") else {
        eprintln!("smysl same-as propose: --engine is required; the engine is the evidence");
        return ExitCode::Usage;
    };
    let mut corpus = match Corpus::open(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("smysl same-as propose: {path}: {e}");
            return ExitCode::Failure;
        }
    };
    let scope = match same_as_scope(m, &corpus) {
        Ok(s) => s,
        Err(code) => return code,
    };

    // The clock is supplied, never read (guarantee A2): a replayed run produces the same
    // attestations, and an engine is exactly the sort of thing that gets replayed.
    let now = smysl::Hlc::new(
        m.get_one::<u64>("at").copied().unwrap_or_else(now_millis),
        0,
        match AgentId::new(format!("tool:smysl-same-as-{engine}")) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("smysl same-as propose: {e}");
                return ExitCode::Failure;
            }
        },
    );

    // The alignment, held outside the match so the borrow outlives the engine that reads it.
    let table = match (engine.as_str(), &corpus) {
        ("anchored", Corpus::Library(l)) => Some(l.index().align.clone()),
        ("anchored", Corpus::Store(..)) => {
            eprintln!(
                "smysl same-as propose: --engine anchored reads a versification alignment, which \
                 only a library has: point --store at the library"
            );
            return ExitCode::Usage;
        }
        _ => None,
    };
    let lexical = Lexical {
        k: m.get_one::<usize>("neighbours").copied().unwrap_or(5),
        adjacent_only: !m.get_flag("no-adjacent"),
    };
    let chosen: Box<dyn Engine> = match engine.as_str() {
        "identical-span" => Box::new(IdenticalSpan),
        "lexical" => Box::new(lexical),
        "anchored" => Box::new(Anchored::new(table.as_ref().expect("set above"))),
        other => {
            eprintln!("smysl same-as propose: unknown engine `{other}`");
            return ExitCode::Usage;
        }
    };

    let found = smysl::same_as::propose(corpus.store(), chosen.as_ref(), scope.as_ref(), now);
    let Some(staged) = found.staged else {
        println!(
            "{}: {} proposal(s), all of which this store already holds",
            found.engine, found.already
        );
        return ExitCode::Success;
    };
    for p in &found.proposals {
        println!("  {} {} — {}", p.a.short(), p.b.short(), p.why);
    }
    if staged.has_errors() {
        eprintln!("smysl same-as propose: the batch did not check clean");
        for d in &staged.report.diagnostics {
            eprintln!("  {d}");
        }
        return ExitCode::Failure;
    }

    // `--dry-run` before anything is written, and it is in the determinism matrix for exactly
    // that reason: a pure engine's output is a function of its input, and an operation that
    // left a sidecar behind could not be run twice to prove it.
    if m.get_flag("dry-run") {
        println!(
            "{} edge(s) would be staged by {} ({} already held)",
            staged.relations.len(),
            found.engine,
            found.already
        );
        return ExitCode::Success;
    }

    let root = project_root(global);
    if m.get_flag("yes") {
        let records = staged.records();
        let n = staged.relations.len();
        match &mut corpus {
            Corpus::Library(l) => {
                if let Err(e) = l.append_records(&records) {
                    eprintln!("smysl same-as propose: {e}");
                    return ExitCode::Failure;
                }
                let _ = smysl::stage::discard(&root);
            }
            Corpus::Store(store, labels) => {
                if let Err(e) = smysl::stage::write(&root, &staged) {
                    eprintln!("smysl same-as propose: {e}");
                    return ExitCode::Failure;
                }
                let store = &*store;
                if let Err(code) = commit_staged("same-as", path, &root, &records, store, labels) {
                    return code;
                }
            }
        }
        println!("{n} edge(s) proposed by {} and committed", found.engine);
        return ExitCode::Success;
    }

    if let Err(e) = smysl::stage::write(&root, &staged) {
        eprintln!("smysl same-as propose: {e}");
        return ExitCode::Failure;
    }
    println!(
        "{} edge(s) staged in {} by {} ({} already held); review, then `smysl merge --staged`",
        staged.relations.len(),
        root.join(smysl::stage::PATH).display(),
        found.engine,
        found.already
    );
    // Rule S, as every staging command: exit 10 is how a pipeline is told that nothing is
    // committed yet.
    ExitCode::Staged
}

#[cfg(feature = "text")]
fn same_as_classes(m: &ArgMatches, path: &str) -> ExitCode {
    use smysl::text::proposition::{classes, Policy};

    let policy = m
        .get_one::<String>("policy")
        .map(String::as_str)
        .unwrap_or("strict");
    let policy = match policy {
        "strict" => Policy::Strict,
        "component" => Policy::Component,
        other => match other.strip_prefix("attested:").map(str::parse::<u32>) {
            Some(Ok(n)) if n > 0 => Policy::Attested(n),
            _ => {
                eprintln!(
                    "smysl same-as classes: --policy `{other}` is not `strict`, `component` or \
                     `attested:N` with N at least 1"
                );
                return ExitCode::Usage;
            }
        },
    };
    let corpus = match Corpus::open(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("smysl same-as classes: {path}: {e}");
            return ExitCode::Failure;
        }
    };
    let scope = match same_as_scope(m, &corpus) {
        Ok(s) => s,
        Err(code) => return code,
    };

    let found = classes(corpus.store(), policy, scope.as_ref());
    let mut members = 0usize;
    for class in &found {
        members += class.len();
        match class.diameter {
            Some(d) => println!(
                "  {} ({} member(s), diameter {d})",
                class.id.short(),
                class.len()
            ),
            None => println!("  {} ({} member(s))", class.id.short(), class.len()),
        }
        for uid in &class.members {
            if *uid != class.id {
                println!("      {}", uid.short());
            }
        }
    }
    // The count and the singletons, because a run of this command is usually a question about
    // one of them. A singleton class is not a failure (A-12.4): it means a unit is claimed to
    // be the same proposition as something and the claims do not agree closely enough to put
    // them together, which is a thing to read.
    let singletons = found.iter().filter(|c| c.len() == 1).count();
    println!(
        "{} class(es) over {members} unit(s), {singletons} of them singleton(s)",
        found.len()
    );
    ExitCode::Success
}

/// The versification alignment, where the corpus is a library that has one.
///
/// Cloned out of the index rather than borrowed, so the table outlives the borrow of the
/// library it came from — `link` hands it to a generator and then hands the library itself to
/// the staging step, and a borrow would make those two the same lifetime.
#[cfg(feature = "text")]
impl Corpus {
    fn alignment(&self) -> Option<smysl::text::index::align::Table> {
        match self {
            Corpus::Library(l) => Some(l.index().align.clone()),
            Corpus::Store(..) => None,
        }
    }
}

/// Rule S, the same way for every command that stages: write, or write and commit on `--yes`.
///
/// Extracted from `same-as propose` when `link` and `summarise` needed the identical sequence.
/// Three commands spelling out "append to a library, discard the sidecar; or write the sidecar
/// and commit a store" would be three chances to get the library branch's `discard` wrong, and
/// forgetting it leaves a staged file that the next `merge --staged` applies twice.
#[cfg(all(feature = "text", feature = "ingest"))]
fn stage_or_commit(
    cmd: &str,
    path: &str,
    mut corpus: Corpus,
    staged: &smysl::stage::Staged,
    yes: bool,
) -> ExitCode {
    if staged.has_errors() {
        eprintln!("smysl {cmd}: the batch did not check clean");
        for d in &staged.report.diagnostics {
            eprintln!("  {d}");
        }
        return ExitCode::Failure;
    }
    let root = std::path::PathBuf::from(".");
    let n = staged.len();
    if yes {
        let records = staged.records();
        match &mut corpus {
            Corpus::Library(l) => {
                if let Err(e) = l.append_records(&records) {
                    eprintln!("smysl {cmd}: {e}");
                    return ExitCode::Failure;
                }
                let _ = smysl::stage::discard(&root);
            }
            Corpus::Store(store, labels) => {
                if let Err(e) = smysl::stage::write(&root, staged) {
                    eprintln!("smysl {cmd}: {e}");
                    return ExitCode::Failure;
                }
                let store = &**store;
                if let Err(code) = commit_staged(cmd, path, &root, &records, store, labels) {
                    return code;
                }
            }
        }
        println!("{n} record(s) committed");
        return ExitCode::Success;
    }
    if let Err(e) = smysl::stage::write(&root, staged) {
        eprintln!("smysl {cmd}: {e}");
        return ExitCode::Failure;
    }
    println!(
        "{n} record(s) staged in {}; review, then `smysl merge --staged`",
        root.join(smysl::stage::PATH).display()
    );
    // Rule S: exit 10 is how a pipeline is told that nothing is committed yet.
    ExitCode::Staged
}

/// The options a model-backed TX-P7 pass runs under.
///
/// The agent is `tool:smysl-<pass>` and the rung is `model`, which is the pair `ingest` uses
/// and says the right two things: this tool wrote the record, and a model's reading is the
/// evidence for it. The clock is supplied and never read (guarantee A2).
#[cfg(all(feature = "text", feature = "ingest", feature = "providers"))]
fn model_options(m: &ArgMatches, pass: &str) -> Result<smysl::IngestOptions, ExitCode> {
    let agent = match AgentId::new(format!("tool:smysl-{pass}")) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("smysl {pass}: {e}");
            return Err(ExitCode::Failure);
        }
    };
    let at = m.get_one::<u64>("at").copied().unwrap_or_else(now_millis);
    Ok(smysl::IngestOptions::at_rung(smysl::Rung::Model)
        .with_agent(agent.clone())
        .with_now(smysl::Hlc::new(at, 0, agent)))
}

/// `smysl link` — judge the candidate pairs and stage what the model says (§3.5, TX-P7).
///
/// Two commands in one, and the flag that separates them is `--candidates`: the generator is
/// pure and the judge is not, so a reader who wants to see what would be asked can see it
/// without paying for an answer. That is also what makes the command registrable in the
/// determinism matrix, where `--candidates` is the operation checked.
#[cfg(feature = "text")]
fn cmd_link(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::text::link::Budget;

    let path = match store_path(m, global, "link") {
        Ok(p) => p,
        Err(code) => return code,
    };
    warn_output_is_a_report(global, "link");

    let corpus = match Corpus::open(&path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("smysl link: {path}: {e}");
            return ExitCode::Failure;
        }
    };
    let scope = match same_as_scope(m, &corpus) {
        Ok(s) => s,
        Err(code) => return code,
    };
    let mut budget = Budget::DEFAULT;
    budget.per_unit = *m.get_one::<usize>("per-unit").unwrap_or(&budget.per_unit);
    budget.neighbours = *m
        .get_one::<usize>("neighbours")
        .unwrap_or(&budget.neighbours);
    budget.hub = *m.get_one::<usize>("hub").unwrap_or(&budget.hub);
    if budget.per_unit == 0 {
        eprintln!("smysl link: --per-unit 0 asks about nothing");
        return ExitCode::Usage;
    }
    // The alignment, where the corpus is a library that has one. A plain store has no
    // versification to anchor against, and the anchored source is simply silent there — it is
    // one of four, and the other three do not need a library.
    let table = corpus.alignment();

    if m.get_flag("candidates") {
        let found =
            smysl::text::link::candidates(corpus.store(), scope.as_ref(), table.as_ref(), budget);
        for c in &found {
            let sources: Vec<&str> = c.sources.iter().map(|s| s.as_str()).collect();
            println!(
                "  {} {} — {} ({})",
                c.a.short(),
                c.b.short(),
                c.why,
                sources.join("+")
            );
        }
        println!(
            "{} candidate(s) at {} per unit, {} neighbour(s), hub {}",
            found.len(),
            budget.per_unit,
            budget.neighbours,
            budget.hub
        );
        return ExitCode::Success;
    }

    link_judge(m, global, &path, corpus, scope, table, budget)
}

#[cfg(not(feature = "text"))]
fn cmd_link(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!("smysl link: needs the library layer (build with --features text)");
    ExitCode::Usage
}

/// The impure half: a provider, a turn per batch of pairs, and rule S.
#[cfg(all(feature = "text", feature = "ingest", feature = "providers"))]
#[allow(clippy::too_many_arguments)]
fn link_judge(
    m: &ArgMatches,
    global: &ArgMatches,
    path: &str,
    corpus: Corpus,
    scope: Option<std::collections::BTreeSet<Uid>>,
    table: Option<smysl::text::index::align::Table>,
    budget: smysl::text::link::Budget,
) -> ExitCode {
    let registry = match load_registry(global) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl link: {}", e.message);
            return e.code;
        }
    };
    let opts = match model_options(m, "link") {
        Ok(o) => o,
        Err(code) => return code,
    };
    let pairs = *m.get_one::<usize>("pairs").unwrap_or(&10);
    let report = match smysl::Ingestor::new(&registry, opts).link(
        corpus.store(),
        scope.as_ref(),
        table.as_ref(),
        budget,
        pairs,
    ) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl link: {e}");
            return ExitCode::Failure;
        }
    };

    for d in &report.diagnostics {
        eprintln!("  {}", d.code.as_str());
        eprintln!("    {}", d.message);
    }
    println!(
        "{} candidate(s), {} asked in {} turn(s); {} contradicts, {} entails, {} unrelated, \
         {} unanswered",
        report.candidates,
        report.asked,
        report.turns,
        report.contradicts,
        report.entails,
        report.unrelated,
        report.unanswered
    );
    if report.already > 0 {
        println!(
            "  {} pair(s) the store already relates, which were not asked again",
            report.already
        );
    }
    let Some(staged) = report.staged else {
        println!("nothing to stage");
        return ExitCode::Success;
    };
    stage_or_commit("link", path, corpus, &staged, m.get_flag("yes"))
}

#[cfg(all(feature = "text", not(all(feature = "ingest", feature = "providers"))))]
#[allow(clippy::too_many_arguments)]
fn link_judge(
    _m: &ArgMatches,
    _global: &ArgMatches,
    _path: &str,
    _corpus: Corpus,
    _scope: Option<std::collections::BTreeSet<Uid>>,
    _table: Option<smysl::text::index::align::Table>,
    _budget: smysl::text::link::Budget,
) -> ExitCode {
    eprintln!(
        "smysl link: judging needs a provider and the staging layer (build with --features \
         ingest); `link --candidates` is pure and works in this build"
    );
    ExitCode::Usage
}

/// `smysl summarise` — a grounded summary tree over one expression (§3.5, TX-P7).
#[cfg(all(feature = "text", feature = "ingest", feature = "providers"))]
fn cmd_summarise(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    use smysl::text::library::Library;
    use smysl::text::reading::Level;

    let path = match store_path(m, global, "summarise") {
        Ok(p) => p,
        Err(code) => return code,
    };
    warn_output_is_a_report(global, "summarise");

    let lib = match Library::open(std::path::Path::new(&path)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("smysl summarise: {path}: {e}");
            eprintln!("  a summary tree takes its levels from a reading, which only a library has");
            return ExitCode::Failure;
        }
    };
    let alias = m.get_one::<String>("scope").expect("required by clap");
    let named = m
        .get_one::<String>("levels")
        .map(String::as_str)
        .unwrap_or("chapter");
    let mut levels = Vec::new();
    for name in named.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match Level::new(name) {
            Some(l) => levels.push(l),
            None => {
                eprintln!("smysl summarise: --levels `{name}` is not a level name");
                eprintln!(
                    "  a level is lowercase letters, as a reader writes them: {}",
                    smysl::text::reading::LEVELS.join(", ")
                );
                return ExitCode::Usage;
            }
        }
    }
    if levels.is_empty() {
        eprintln!("smysl summarise: --levels names no level");
        return ExitCode::Usage;
    }

    let registry = match load_registry(global) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl summarise: {}", e.message);
            return e.code;
        }
    };
    let opts = match model_options(m, "summarise") {
        Ok(o) => o,
        Err(code) => return code,
    };
    let report = match smysl::Ingestor::new(&registry, opts).summarise(&lib, alias, &levels, None) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl summarise: {e}");
            return ExitCode::Failure;
        }
    };
    for d in &report.diagnostics {
        eprintln!("  {}", d.code.as_str());
        eprintln!("    {}", d.message);
    }
    for (level, n) in &report.levels {
        println!("  {level}: {n} summary(ies)");
    }
    println!(
        "{} summary(ies) in {} turn(s); {} capped by rule M, {} node(s) too thin to summarise",
        report.summaries, report.turns, report.capped, report.thin
    );
    let Some(staged) = report.staged else {
        println!("nothing to stage");
        return ExitCode::Success;
    };
    stage_or_commit(
        "summarise",
        &path,
        Corpus::Library(Box::new(lib)),
        &staged,
        m.get_flag("yes"),
    )
}

#[cfg(not(all(feature = "text", feature = "ingest", feature = "providers")))]
fn cmd_summarise(_m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
    eprintln!(
        "smysl summarise: needs the library layer and a provider (build with --features \
         text,ingest)"
    );
    ExitCode::Usage
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        cli().debug_assert();
    }

    /// H-13: a committed batch is in the store and is no longer staged.
    ///
    /// `--yes` promised this from the day it was written and did nothing; 1.9 warned, 1.10
    /// commits. The order is the part worth pinning: the stage goes only after the write
    /// landed, so a failed write leaves the work recoverable.
    #[cfg(feature = "ingest")]
    #[test]
    fn commit_staged_writes_the_batch_then_drops_the_stage() {
        use smysl::{KernelType, Record, Status, Store, UnitCoreBuilder};

        let dir = std::env::temp_dir().join(format!("smysl-h13-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".smysl")).unwrap();

        let core = UnitCoreBuilder::new(
            KernelType::Claim,
            "the pool saturated at four in the afternoon",
            Status::Speculative,
        )
        .build()
        .unwrap();
        let uid = smysl::canonical_uid(&core);
        let records = vec![Record::Unit(core)];

        // A CBOR store to commit into, and a staged file standing in for `stage::write`.
        //
        // The store holds a record already, because an *empty* file is indistinguishable from
        // surface text to `looks_like_surface` and would send `persist` down the wrong branch.
        // That is a real edge for a brand-new store and not this test's subject.
        let seed = UnitCoreBuilder::new(
            KernelType::Claim,
            "a claim that was already in the store",
            Status::Speculative,
        )
        .build()
        .unwrap();
        let path = dir.join("store.cbor");
        std::fs::write(&path, smysl::to_cbor_seq(&[Record::Unit(seed)])).unwrap();
        std::fs::write(dir.join(smysl::stage::PATH), smysl::to_cbor_seq(&records)).unwrap();

        let store = Store::new();
        let labels = std::collections::BTreeMap::new();
        let path_s = path.to_str().unwrap();
        commit_staged("ingest", path_s, &dir, &records, &store, &labels).unwrap();

        let (back, _) = load_store(path_s).unwrap();
        assert!(
            back.get(&uid).is_some(),
            "the batch did not reach the store"
        );
        assert!(
            !dir.join(smysl::stage::PATH).exists(),
            "the stage outlived the commit"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A surface store refuses the batch rather than keeping the units and losing the recipe.
    ///
    /// Attestations have no surface spelling, so appending a staged batch to a `.smy` store
    /// would write what the model said and drop how it was asked — which is the one thing the
    /// recipe exists to carry.
    #[cfg(feature = "ingest")]
    #[test]
    fn commit_staged_refuses_a_surface_store_that_would_lose_the_attestations() {
        use smysl::{AgentId, Hlc, KernelType, Record, Rung, Status, Store, UnitCoreBuilder};

        let dir = std::env::temp_dir().join(format!("smysl-h13-surface-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".smysl")).unwrap();

        let core = UnitCoreBuilder::new(KernelType::Claim, "a staged claim", Status::Speculative)
            .build()
            .unwrap();
        let uid = smysl::canonical_uid(&core);
        let agent = AgentId::new("tool:test").unwrap();
        let attest = smysl::stage::Attest::new(agent.clone(), Rung::Document, Hlc::zero(agent));
        let records = vec![
            Record::Unit(core),
            Record::Attestation(attest.for_unit(uid)),
        ];

        let path = dir.join("store.smy");
        std::fs::write(
            &path,
            "@doc smysl/1.0 {\n  id: v/x\n  intent: t\n  lang: en\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join(smysl::stage::PATH), smysl::to_cbor_seq(&records)).unwrap();

        let store = Store::new();
        let labels = std::collections::BTreeMap::new();
        let err = commit_staged(
            "ingest",
            path.to_str().unwrap(),
            &dir,
            &records,
            &store,
            &labels,
        )
        .unwrap_err();
        assert_eq!(err, ExitCode::Usage);
        assert!(
            dir.join(smysl::stage::PATH).exists(),
            "a refused commit must leave the batch staged"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §23's table, plus one.
    ///
    /// **`import`, `relink` and `compact` are additions.** §23 has no command that produces units
    /// without a model, and without one the top of the status ladder was unreachable:
    /// `measured` needs an `op: Imported` attestation at the `computed` rung and nothing
    /// wrote one. It also has no command for re-pointing a reference after the unit it
    /// names is replaced, which content-addressed identity makes a routine consequence of
    /// editing. Nor one for dropping what supersession has settled, which a store that
    /// lives for weeks needs and a grow-only log cannot do for itself. Divergences to
    /// reconcile, not a miscount.
    #[test]
    fn command_table_matches_section_23() {
        assert_eq!(COMMANDS.len(), 38);
        let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        assert_eq!(
            names,
            [
                "fmt",
                "check",
                "pack",
                "merge",
                "diff",
                "trace",
                "view",
                "bundle",
                "thread",
                "salience",
                "find",
                "retract",
                "withdraw",
                "resolve",
                "review",
                "commit",
                "render",
                "import",
                "relink",
                "compact",
                "ingest",
                "attest",
                "providers",
                "usage",
                "reindex",
                "ui",
                "text",
                "date",
                "same-as",
                "link",
                "summarise",
                "q",
                "ask",
                "measure",
                "assess",
                "lib",
                "schema",
                "migrate"
            ]
        );
    }

    /// Only `ingest`, `attest`, and `thread --refine` may depend on a model (rule D).
    ///
    /// `thread`'s `Mixed` is a reservation rather than a description: `--refine` is routed as a
    /// `Task` and documented in the derivation module, and no argument reaches it. The manual
    /// gives the reasoning — the label describes what the command is permitted to become, so
    /// nothing downstream is re-audited the day the flag lands — and that is a deliberate
    /// pessimism worth keeping. What is not worth keeping is a help line promising a flag a
    /// reader cannot pass, which is what 1.9 briefly shipped; `impure_when` now says the flag
    /// is not yet wired, and `--seed-check` lets every `thread` invocation run.
    #[test]
    fn only_ingest_attest_and_summarise_are_model_dependent() {
        let model: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.purity == Purity::Model)
            .map(|c| c.name)
            .collect();
        assert_eq!(
            model,
            ["ingest", "attest", "summarise"],
            "a summary is a model's sentence; there is no pure invocation of it"
        );

        let mixed: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.purity == Purity::Mixed)
            .map(|c| c.name)
            .collect();
        assert_eq!(
            mixed,
            ["pack", "thread", "find", "link", "q", "ask", "measure", "assess"],
            "a command is mixed when some invocation of it calls a model"
        );

        // `mixed` on its own tells a reader to assume the worst of every invocation, which is
        // its own kind of wrong: all three are pure by default. Every mixed command therefore
        // has to say which invocations are not.
        for c in COMMANDS.iter().filter(|c| c.purity == Purity::Mixed) {
            assert!(
                c.impure_when.is_some(),
                "{} is mixed and does not say when",
                c.name
            );
        }
        for c in COMMANDS.iter().filter(|c| c.purity != Purity::Mixed) {
            assert!(
                c.impure_when.is_none(),
                "{} is not mixed, so there is no exception to name",
                c.name
            );
        }
    }

    /// Every mixed command's impure invocations are ones `--seed-check` can name.
    ///
    /// `reproducible` returns `None` for a mixed command it cannot narrow, and the dispatcher
    /// refuses on `None` rather than assuming the best. That refusal is correct and useless: it
    /// would tell a caller that smysl does not know its own command. So the two lists are tied
    /// together here — adding a mixed command without teaching `reproducible` about it fails
    /// this test rather than shipping a flag that answers "I cannot say".
    #[test]
    fn seed_check_can_judge_every_mixed_command() {
        let judged = [
            "find", "pack", "thread", "link", "q", "ask", "measure", "assess",
        ];
        for c in COMMANDS.iter().filter(|c| c.purity == Purity::Mixed) {
            assert!(
                judged.contains(&c.name),
                "{} is mixed and `reproducible` has no arm for it",
                c.name
            );
        }
        // And nothing in `judged` has stopped being mixed, which would leave an arm that can
        // never be reached — the other direction of the same drift.
        for name in judged {
            let c = COMMANDS.iter().find(|c| c.name == name).expect("a command");
            assert!(
                c.purity == Purity::Mixed,
                "{name} has an arm in `reproducible` and is no longer mixed"
            );
        }
    }

    /// Every command records where it came from.
    ///
    /// `SM-Pnn` for the original delivery phases, which ended at SM-P15. A command wired
    /// after that names the release cycle instead — `find` is the first, and writing
    /// `SM-P16` for it would invent a phase that never existed to keep a string pattern
    /// happy. What the field is for is answering "when did this arrive and why", and a
    /// version answers that as well as a phase does.
    #[test]
    fn every_command_names_the_phase_that_wires_it() {
        for c in COMMANDS {
            // Two phase vocabularies, because there are two RFCs: `SM-P*` is SMYSL-1's
            // delivery plan and `TX-P*` is SMYSL-2.4's. A release number is the third form,
            // for a command that arrived outside either plan.
            let phase = c.phase.starts_with("SM-P") || c.phase.starts_with("TX-P");
            let cycle = c
                .phase
                .split('.')
                .all(|p| !p.is_empty() && p.chars().all(|ch| ch.is_ascii_digit()));
            assert!(
                phase || cycle,
                "{}: `{}` is neither a phase nor a release",
                c.name,
                c.phase
            );
            assert!(!c.about.is_empty(), "{}: no description", c.name);
        }
    }

    // ---- the pure helpers, which 0.13's mutation pass found untested -------------------
    //
    // `src/main.rs` had four tests across 3,600 lines and 121 survivors. These are the ones
    // reachable without driving the binary: small functions with a decision in them, where a
    // mutant changes an answer rather than a side effect.

    /// Which byte streams are read as surface rather than CBOR. Getting this wrong sends a
    /// CBOR file through the surface parser and reports a syntax error for a valid document.
    #[test]
    fn surface_detection_looks_past_blank_lines_and_comments() {
        assert!(looks_like_surface(b"@doc smysl/0.1\n"));
        assert!(looks_like_surface(b"\n\n@doc smysl/0.1\n"));
        assert!(looks_like_surface(b"# a comment\n@doc smysl/0.1\n"));
        assert!(looks_like_surface(b"// a comment\n@doc smysl/0.1\n"));
        assert!(looks_like_surface(b"\n#c\n//c\n@doc smysl/0.1\n"));

        // Column 0 only, matching `lex.rs`: "an indented `#` is inside a body or a step,
        // where it is prose". A heuristic that disagreed with the lexer would classify as
        // surface a document the surface parser then rejects.
        assert!(!looks_like_surface(b" # indented\n@doc smysl/0.1\n"));

        // Nothing but comments is a valid, empty surface document.
        assert!(looks_like_surface(b""));
        assert!(looks_like_surface(b"# only a comment\n"));

        // The three skip-conditions are separate claims: a line that is *not* blank and not a
        // comment decides the answer, whatever it is.
        assert!(!looks_like_surface(b"not a directive\n"));
        // A comment *after* a non-directive line does not rescue it: the first line that is
        // neither blank nor a comment is the one that decides.
        assert!(!looks_like_surface(b"not a directive\n# a comment\n@doc\n"));

        // Invalid UTF-8 is CBOR, not surface.
        assert!(!looks_like_surface(&[0xff, 0xfe, 0x00]));
    }

    /// Every command declares which output forms it can write, and the set is one of three.
    ///
    /// The list the CLI matrix asserts lives in `tests/global_flags.rs`, deliberately written
    /// out there rather than read from here. This is the invariant that list cannot express: a
    /// command's forms are either both, or the store log alone, or none — never some other
    /// subset, and never, as happened for twenty-three commands, a flag the dispatcher accepts
    /// and the command ignores. `lib` is the one exception, with all three: its actions differ,
    /// and `cmd_lib` refuses per action what the table cannot (TX-P11 step 5).
    #[test]
    fn every_command_declares_its_output_forms() {
        let mut documents = 0;
        for c in COMMANDS {
            assert!(
                c.forms == BOTH
                    || c.forms == CBOR_ONLY
                    || c.forms == NO_DOCUMENT
                    || (c.name == "lib" && c.forms == LIB_FORMS),
                "{} declares an unexpected form set: {:?}",
                c.name,
                c.forms
            );
            if !c.forms.is_empty() {
                documents += 1;
            }
            for f in c.forms {
                assert!(
                    *f == "surface" || *f == "cbor" || *f == "pack",
                    "{} names a form `--format` does not accept: {f}",
                    c.name
                );
            }
        }
        // Five write both, three write a store log, and `lib export` writes a collection. The
        // rest report, mutate in place, or render an artifact through their own `--target`.
        assert_eq!(documents, 9);
    }

    /// A store named with no directory part has a parent of `""`, which is not the current
    /// directory — dropping the filter would put every sidecar at the filesystem root.
    #[cfg(feature = "providers")]
    #[test]
    fn a_sidecar_sits_beside_its_store() {
        assert_eq!(
            root_beside(Some("notes/store.smy")),
            std::path::Path::new("notes")
        );
        assert_eq!(root_beside(Some("store.smy")), std::path::Path::new("."));
        assert_eq!(root_beside(None), std::path::Path::new("."));
        assert_eq!(
            root_beside(Some("/a/b/store.smy")),
            std::path::Path::new("/a/b")
        );
    }

    /// `-` means stdin and nothing else does. Inverting the test would read stdin for a real
    /// path and hand `-` to the filesystem.
    ///
    /// Reads a fixture rather than a temp file: the first version wrote into a shared
    /// directory under `TMPDIR` and failed in CI with `NotFound`, which is a test that
    /// depends on the state of the machine rather than on the code.
    #[cfg(feature = "ingest")]
    #[test]
    fn a_path_that_is_not_a_dash_is_read_from_the_filesystem() {
        let text = read_input("fixtures/corpus/F1-incident.smy").expect("the fixture is there");
        assert!(
            text.starts_with("@doc"),
            "read something that is not the fixture: {:?}",
            &text[..text.len().min(40)]
        );
    }

    /// A summary over one file is noise; over several it is information. Three mutants sat on
    /// this one comparison because nothing could see what the bar printed.
    #[test]
    fn a_summary_line_appears_only_when_there_was_more_than_one_file() {
        for (n, expect_message) in [(0usize, false), (1, false), (2, true), (9, true)] {
            let (bar, buf) = progress::Bar::to_buffer(
                progress::Style::decide(true, false, false, true),
                "working",
                n,
            );
            finish_over(bar, n, "checked");
            let out = buf.lock().unwrap().clone();
            assert_eq!(
                out.contains("checked"),
                expect_message,
                "n={n} printed {out:?}"
            );
        }
    }

    /// `worse` keeps the higher exit code. Its `>` survives mutation to `>=`, and that is an
    /// *equivalent* mutant rather than a gap: `ExitCode` is a fieldless enum with distinct
    /// discriminants, so equal codes are the same variant and returning either is the same
    /// answer. Recorded here so the survivor is a decision rather than an oversight.
    #[test]
    fn the_worse_of_two_exit_codes_is_the_higher_one() {
        use smysl::ExitCode as E;
        assert_eq!(worse(E::Success, E::Failure), E::Failure);
        assert_eq!(worse(E::Failure, E::Success), E::Failure);
        assert_eq!(worse(E::CheckErrors, E::Usage), E::CheckErrors);
        assert_eq!(worse(E::Success, E::Success), E::Success);
    }
}
