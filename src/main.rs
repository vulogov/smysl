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
    plan_retraction, write_surface, AgentId, BuildOptions, CheckOptions, Code, ConformanceClass,
    ConsumerProfile, Contentions, DeriveOptions, Estimator, Hlc, Lod, MergeOptions, PackRequest,
    Pass, Profile, Record, RelKind, Relation, RetractionAuthority, RetractionPolicy, Role,
    SalienceRequest, SalienceWeights, SchemaId, Severity, Store, StoreOptions, SupersessionPolicy,
    Target, TraceKind, Uid, UidPrefix, View, ViewId, WriteContext,
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

struct Cmd {
    name: &'static str,
    about: &'static str,
    purity: Purity,
    /// The delivery phase that wires this command to its library API.
    phase: &'static str,
}

/// The command table of §23, in table order.
#[rustfmt::skip]
const COMMANDS: &[Cmd] = &[
    Cmd { name: "fmt",       about: "Canonicalise surface text and verify the round-trip", purity: Purity::Pure,  phase: "SM-P2"  },
    Cmd { name: "check",     about: "Run the check pipeline over a store",                 purity: Purity::Pure,  phase: "SM-P4"  },
    Cmd { name: "pack",      about: "Budget-bounded, closure-complete selection",          purity: Purity::Pure,  phase: "SM-P9"  },
    Cmd { name: "merge",     about: "Join-semilattice union; materialise contentions",     purity: Purity::Pure,  phase: "SM-P6"  },
    Cmd { name: "diff",      about: "Partition uids across stores or hops",                purity: Purity::Pure,  phase: "SM-P7"  },
    Cmd { name: "trace",     about: "Walk provenance or evidential support",               purity: Purity::Pure,  phase: "SM-P7"  },
    Cmd { name: "view",      about: "Define or print a view",                              purity: Purity::Pure,  phase: "SM-P7"  },
    Cmd { name: "bundle",    about: "Emit the reachable closure of a view",                purity: Purity::Pure,  phase: "SM-P7"  },
    Cmd { name: "thread",    about: "Derive, refine, list, show, or import threads",       purity: Purity::Mixed, phase: "SM-P11" },
    Cmd { name: "salience",  about: "Report derived salience with per-term breakdown",     purity: Purity::Pure,  phase: "SM-P8"  },
    Cmd { name: "find",      about: "Rank units against a query, lexically",                purity: Purity::Pure,  phase: "0.5.0"  },
    Cmd { name: "retract",   about: "Retract a unit; report the blast radius first",       purity: Purity::Pure,  phase: "SM-P6"  },
    Cmd { name: "render",    about: "Thread plus profile to artifact",                     purity: Purity::Pure,  phase: "SM-P12" },
    Cmd { name: "import",    about: "Tabular readings to measured units, without a model",  purity: Purity::Pure,  phase: "SM-P15" },
    Cmd { name: "relink",    about: "Re-point references onto superseded units",             purity: Purity::Pure,  phase: "SM-P15" },
    Cmd { name: "compact",   about: "Drop superseded units nothing needs; never in place",   purity: Purity::Pure,  phase: "SM-P15" },
    Cmd { name: "ingest",    about: "Prose or data to staged units",                       purity: Purity::Model, phase: "SM-P14" },
    Cmd { name: "attest",    about: "Semantic checks that require a model",                purity: Purity::Model, phase: "SM-P14" },
    Cmd { name: "providers", about: "List providers, capabilities, and what would egress", purity: Purity::Pure,  phase: "SM-P13" },
    Cmd { name: "usage",     about: "Token and cost ledger",                               purity: Purity::Pure,  phase: "SM-P13" },
    Cmd { name: "reindex",   about: "Rebuild the derived index from the log alone",        purity: Purity::Pure,  phase: "SM-P3"  },
    Cmd { name: "ui",        about: "Terminal UI",                                         purity: Purity::Pure,  phase: "SM-P15" },
];

fn cli() -> Command {
    let globals = [
        Arg::new("config")
            .short('C')
            .long("config")
            .global(true)
            .help("Configuration file")
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
            .value_parser(["surface", "cbor"]),
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
            .after_help(format!("Purity: {}", c.purity.tag()));
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
                        .value_parser(["analysis", "narrative", "brief", "qa", "plan"])
                        .help("Derive a thread from the graph; the schema may follow here or in --schema"),
                )
                .arg(
                    Arg::new("schema")
                        .long("schema")
                        .value_name("S")
                        .value_parser(["analysis", "narrative", "brief", "qa", "plan"])
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
                        .help("Granularity profile the units are produced under"),
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
                    Arg::new("yes")
                        .long("yes")
                        .help("Commit the staged batch instead of exiting 10")
                        .action(ArgAction::SetTrue),
                )
                .arg(
                    Arg::new("dry-run")
                        .long("dry-run")
                        .help("Report what would be sent and to whom; make no call")
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

        let ctx = WriteContext::from_labels(&out.labels).with_salience(out.salience.clone());
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

        if let Some(dest) = global.get_one::<String>("output") {
            if n_inputs > 1 {
                eprintln!(
                    "smysl fmt: --output takes one document, and {n_inputs} were given; \
                     use --write to format them in place"
                );
                return ExitCode::Usage;
            }
            if let Err(e) = std::fs::write(dest, formatted.as_bytes()) {
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
            if stdout.write_all(formatted.as_bytes()).is_err() {
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
    let passes: Vec<Pass> = m
        .get_many::<String>("pass")
        .map(|v| v.filter_map(|s| Pass::parse(s)).collect())
        .unwrap_or_default();

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
        let out = match read_store(&path) {
            Ok(o) => o,
            Err(code) => {
                bar.abandon();
                return code;
            }
        };

        let store = Store::from_records(out.records.clone());
        let mut opts = CheckOptions::default().with_labels(out.labels.clone());
        if !passes.is_empty() {
            opts = opts.only(passes.clone());
        }
        if let Some(f) = &consumer {
            opts = opts.as_consumer(f.clone());
        }
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
        Ok((records, _)) => Ok(smysl::ParseOutcome {
            records,
            ..Default::default()
        }),
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
    std::fs::read(path).map_err(|e| e.to_string())
}

/// Read a store from surface text or a CBOR log, whichever it turns out to be.
fn load_store(
    path: &str,
) -> Result<(Store, std::collections::BTreeMap<smysl::Label, Uid>), String> {
    let bytes = if path == "-" {
        let mut b = Vec::new();
        std::io::stdin()
            .read_to_end(&mut b)
            .map_err(|e| e.to_string())?;
        b
    } else {
        std::fs::read(path).map_err(|e| format!("{path}: {e}"))?
    };

    if looks_like_surface(&bytes) {
        let src = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        let out = parse_surface(&src).map_err(|e| e.to_string())?;
        Ok((Store::from_records(out.records.clone()), out.labels))
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
        Ok((Store::from_records(records), labels))
    }
}

/// Resolve a uid argument against a store, accepting the display form.
fn resolve(store: &Store, raw: &str) -> Result<Uid, String> {
    let prefix = UidPrefix::parse(raw).map_err(|_| format!("`{raw}` is not a uid"))?;
    store.resolve_prefix(&prefix).map_err(|e| e.to_string())
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
            return ExitCode::Failure;
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

    let l = smysl::trace(&store, target, kind, depth);
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
                        return ExitCode::Failure;
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
    let (store, _) = match load_store(&path) {
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
    let bytes = store.bundle_with(&view, m.get_flag("include-retracted"));
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
    let surface = global
        .get_one::<String>("format")
        .map(|f| f == "surface")
        .unwrap_or(false);

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
        let dropped = records
            .iter()
            .filter(|r| match r {
                Record::Unit(_) | Record::Relation(_) | Record::Thread(_) => false,
                Record::View(v) => Some(&v.id) != emitted.as_ref(),
                _ => true,
            })
            .count();
        if dropped > 0 {
            eprintln!(
                "smysl merge: warning: {dropped} record(s) have no surface form and were \
                 omitted (contentions, attestations); the default CBOR output preserves them"
            );
        }
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
    let (mut store, _) = match load_store(&path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl retract: {e}");
            return ExitCode::Failure;
        }
    };

    let raw = m.get_one::<String>("uid").expect("required");
    let target = match UidPrefix::parse(raw).ok().map(|p| store.resolve_prefix(&p)) {
        Some(Ok(u)) => u,
        Some(Err(e)) => {
            eprintln!("smysl retract: {e}");
            return ExitCode::Failure;
        }
        None => {
            eprintln!("smysl retract: `{raw}` is not a uid");
            return ExitCode::Usage;
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
            "{{\"target\":{},\"blast_radius\":[{}],\"orphaned\":[{}],\"authorised\":{},\"refusal\":{}}}",
            smysl::json_escape(&target.canonical()),
            uid_array(&plan.blast_radius),
            uid_array(&plan.orphaned),
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
        println!(
            "{path}: retracting {target} would reach {} unit(s), orphaning {}",
            plan.blast_radius.len(),
            plan.orphaned.len()
        );
        for u in &plan.orphaned {
            println!("{path}:   {u} would lose all of its grounds");
        }
    }

    if m.get_flag("dry-run") {
        return ExitCode::Success;
    }
    if !plan.authorised {
        eprintln!(
            "smysl retract: {}",
            plan.refusal.unwrap_or_else(|| "refused".into())
        );
        return ExitCode::Failure;
    }

    if let Err(e) = store.append(&[Record::Relation(Relation::new(
        RelKind::Retracts,
        target,
        target,
    ))]) {
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
    if let Some(v) = m.get_many::<String>("focus") {
        let mut focus = Vec::new();
        for raw in v {
            match resolve(&store, raw) {
                Ok(u) => focus.push(u),
                Err(e) => {
                    eprintln!("smysl pack: {e}");
                    return ExitCode::Failure;
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
    if let Some(q) = m.get_one::<String>("query") {
        use smysl::Retriever as _;
        let limit = m.get_one::<usize>("query-limit").copied().unwrap_or(3);
        let hits = smysl::Bm25::index(&store).search(&smysl::Query::new(q.clone(), limit));
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
            "{path}: {} of {} unit(s), {} of {} tokens, {} mode, gap {:.3}{}",
            packed.len(),
            store.units().count(),
            packed.used(),
            packed.info.budget,
            packed.info.optimality.mode,
            packed.info.optimality.gap,
            if packed.is_optimal() {
                " (proven optimal)"
            } else {
                ""
            }
        );
    }

    let surface = global
        .get_one::<String>("format")
        .map(|f| f == "surface")
        .unwrap_or(false);

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
    store: &Store,
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
        req = req.with_weights(SalienceWeights {
            centrality: parts[0],
            corroboration: parts[1],
            role: parts[2],
            recency: parts.get(3).copied().unwrap_or(0.0),
        });
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
                        return ExitCode::Failure;
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
                return ExitCode::Failure;
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
    let (store, _) = match load_store(&path) {
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

    // `Retriever` must be in scope for `search`; the trait is the seam, and using it here
    // keeps the command honest about depending on the interface rather than the engine.
    use smysl::Retriever as _;
    let hits = smysl::Bm25::index(&store).search(&q);

    if global.get_flag("json") {
        let rows: Vec<String> = hits
            .iter()
            .map(|h| {
                format!(
                    "{{\"uid\":{},\"score\":{:.4}}}",
                    smysl::json_escape(&h.uid.canonical()),
                    h.score
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
    for h in &hits {
        let gist = store
            .get(&h.uid)
            .map(|u| u.core.gist.as_str())
            .unwrap_or("");
        println!("{:.4}  {}  {}", h.score, h.uid, gist);
    }
    ExitCode::Success
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
    let (store, labels) = match load_store(&path) {
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
                    return ExitCode::Failure;
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
    let surface = global
        .get_one::<String>("format")
        .map(|f| f == "surface")
        .unwrap_or(true);

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

fn show_thread(store: &Store, t: &smysl::Thread) -> ExitCode {
    println!("{}  {}", t.id, t.schema);
    println!("~ {}", t.gist);
    for (i, step) in t.steps.iter().enumerate() {
        let gist = store
            .get(&step.unit)
            .map(|u| u.core.gist.as_str())
            .unwrap_or("(not in this store)");
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
    let base = global
        .get_one::<String>("store")
        .map(std::path::PathBuf::from)
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    base.join(name)
}

/// Load the provider configuration, falling back to the all-local default.
#[cfg(feature = "providers")]
fn load_registry(global: &ArgMatches) -> Result<smysl::Registry, String> {
    let path = project_file(global, smysl::ProviderConfigFile::PATH);
    let cfg = match std::fs::read_to_string(&path) {
        Ok(src) => {
            smysl::ProviderConfigFile::load(&src).map_err(|e| format!("{}: {e}", path.display()))?
        }
        // A default that reached a hosted provider would mean a first run egressing
        // content nobody asked to send, so the default is entirely local.
        Err(_) => smysl::ProviderConfigFile::local_default(),
    };

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
#[cfg(feature = "ingest")]
fn cmd_ingest(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let style = progress_style(global);
    let registry = match load_registry(global) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("smysl ingest: {e}");
            return ExitCode::Failure;
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

    let store = match store_arg(m, global) {
        Some(path) => match load_store(&path) {
            Ok((s, _)) => s,
            Err(e) => {
                eprintln!("smysl ingest: {e}");
                return ExitCode::Failure;
            }
        },
        None => Store::new(),
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
    if let Some(p) = m.get_one::<String>("path") {
        if let Some(p) = smysl::IngestPath::parse(p) {
            opts = opts.with_path(p);
        }
    }
    if let Some(n) = m.get_one::<String>("repair").and_then(|s| s.parse().ok()) {
        opts = opts.with_repair_attempts(n);
    }

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
            smysl::choose_ingest_path(&caps, smysl::Task::ContentIngest, input.len(), opts.path);
        println!("provider     {}", provider.id());
        println!(
            "egress       {}",
            if caps.offline {
                "no - local"
            } else {
                "YES - leaves the machine"
            }
        );
        println!("path         {} ({})", choice.path, choice.reason.as_str());
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
        "smysl ingest: {} chunk(s), {} call(s), {} unit(s), {} weakened, {} degraded, {} token(s)",
        report.chunks,
        report.calls,
        staged.len(),
        staged.weakened.len(),
        report.degraded,
        report.usage.total()
    );

    // The ledger records counts, models, task and recipe - never content (§29).
    if let Some(provider) = &report.provider {
        let mut ledger = smysl::Ledger::open(project_file(global, smysl::Ledger::PATH));
        let entry = smysl::LedgerEntry::new(
            now_millis(),
            provider.clone(),
            "",
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
        println!("{} unit(s) staged and confirmed", staged.len());
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
            return ExitCode::Failure;
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
#[cfg(feature = "providers")]
fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The project root: the directory holding `.smysl/`.
#[cfg(feature = "providers")]
fn project_root(global: &ArgMatches) -> std::path::PathBuf {
    global
        .get_one::<String>("store")
        .map(std::path::PathBuf::from)
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::PathBuf::from("."))
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
            return ExitCode::Failure;
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
    let (store, _) = match load_store(&path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("smysl compact: {e}");
            return ExitCode::Failure;
        }
    };

    let out = smysl::compact(&store);
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
    let (store, _) = match load_store(&path) {
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

    match smysl::tui::run(smysl::tui::App::new(store, labels)) {
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

    let (mut store, open) = match Store::open_with(&path, StoreOptions::default()) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("smysl reindex: {path}: {e}");
            return ExitCode::Failure;
        }
    };
    for d in open.report.iter() {
        eprintln!("{path}: {d}");
    }

    let rebuilt = store.reindex().to_bytes();

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

    let Some(cmd) = COMMANDS.iter().find(|c| c.name == name) else {
        eprintln!("smysl: unknown command `{name}`");
        return ProcExitCode::from(ExitCode::Usage.as_i32() as u8);
    };

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_definition_is_valid() {
        cli().debug_assert();
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
        assert_eq!(COMMANDS.len(), 22);
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
                "render",
                "import",
                "relink",
                "compact",
                "ingest",
                "attest",
                "providers",
                "usage",
                "reindex",
                "ui"
            ]
        );
    }

    /// Only `ingest`, `attest`, and `thread --refine` may depend on a model (rule D).
    #[test]
    fn only_ingest_and_attest_are_model_dependent() {
        let model: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.purity == Purity::Model)
            .map(|c| c.name)
            .collect();
        assert_eq!(model, ["ingest", "attest"]);

        let mixed: Vec<&str> = COMMANDS
            .iter()
            .filter(|c| c.purity == Purity::Mixed)
            .map(|c| c.name)
            .collect();
        assert_eq!(
            mixed,
            ["thread"],
            "thread is mixed only because of --refine"
        );
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
            let phase = c.phase.starts_with("SM-P");
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
}
