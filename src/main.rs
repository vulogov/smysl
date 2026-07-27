//! The `smysl` binary - a thin shell over the library (rule A, principle P8).
//!
//! Every subcommand here dispatches into `smysl::*`. Nothing is implemented in the
//! binary that an embedder cannot reach from the library, which is the cheapest way to
//! keep the API honest.
//!
//! SM-P0 declares the full command surface of §23 and reports, per command, the phase
//! that wires it up. Commands are wired only once their library API has stabilised.

use std::io::{Read, Write};
use std::process::ExitCode as ProcExitCode;

use clap::{Arg, ArgAction, ArgMatches, Command};
use smysl::ExitCode;
use smysl::{
    check, conformance, effective_status, fidelity, granularity_distribution, merge, parse_surface,
    plan_retraction, write_surface, AgentId, CheckOptions, ConformanceClass, ConsumerProfile,
    DeriveOptions, Estimator, Hlc, Lod, MergeOptions, PackRequest, Pass, Record, RelKind, Relation,
    RetractionAuthority, RetractionPolicy, Role, SalienceRequest, SalienceWeights, SchemaId,
    Severity, Store, StoreOptions, SupersessionPolicy, TraceKind, Uid, UidPrefix, View, ViewId,
    WriteContext,
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
    Cmd { name: "retract",   about: "Retract a unit; report the blast radius first",       purity: Purity::Pure,  phase: "SM-P6"  },
    Cmd { name: "render",    about: "Thread plus profile to artifact",                     purity: Purity::Pure,  phase: "SM-P12" },
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
            "salience" => sub
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
                        .value_parser(["analysis", "narrative", "brief", "qa", "plan"])
                        .help("Derive a thread of this schema from the graph"),
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
fn cmd_fmt(m: &ArgMatches) -> ExitCode {
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

    let mut worst = ExitCode::Success;
    for path in inputs {
        let src = match read_input(&path) {
            Ok(s) => s,
            Err(e) => {
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
        for d in &out.diagnostics {
            eprintln!("{path}: {d}");
        }
        if out.has_errors() {
            worst = worse(worst, ExitCode::CheckErrors);
            continue;
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
    worst
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

    let mut worst = ExitCode::Success;
    for path in files {
        let src = match read_input(&path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("smysl check: {path}: {e}");
                return ExitCode::Failure;
            }
        };
        let out = match parse_surface(&src) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("smysl check: {path}: {e}");
                return e.into_exit_code();
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
                println!(
                    "{{\"code\":\"{}\",\"severity\":\"{}\",\"message\":{:?}}}",
                    d.code, d.severity, d.message
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
        } else if !json {
            println!(
                "{path}: {} records, {} units, {} diagnostic(s)",
                store.len(),
                store.units().count(),
                report.len()
            );
        }
    }
    worst
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

    // Surface text always starts with a sigil; a CBOR sequence starts with an array head.
    if bytes.first() == Some(&b'@') || bytes.is_empty() {
        let src = String::from_utf8(bytes).map_err(|e| e.to_string())?;
        let out = parse_surface(&src).map_err(|e| e.to_string())?;
        Ok((Store::from_records(out.records.clone()), out.labels))
    } else {
        let (records, _) = smysl::from_cbor_seq(&bytes).map_err(|e| e.to_string())?;
        Ok((Store::from_records(records), Default::default()))
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
fn cmd_diff(m: &ArgMatches, _global: &ArgMatches) -> ExitCode {
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

    let bytes = store.bundle_with(&view, m.get_flag("include-retracted"));
    match global.get_one::<String>("output") {
        Some(p) => {
            if let Err(e) = std::fs::write(p, &bytes) {
                eprintln!("smysl bundle: {p}: {e}");
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

/// `smysl merge` - join-semilattice union, materialising disagreement (§23.1).
///
/// Merge never adjudicates. Where two agents disagree the disagreement becomes an object
/// in the report rather than a winner in the store.
fn cmd_merge(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
    let inputs: Vec<String> = m
        .get_many::<String>("inputs")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();

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

    let mut store = Store::new();
    let mut labels = Vec::new();
    for path in &inputs {
        let (s, l) = match load_store(path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("smysl merge: {e}");
                return ExitCode::Failure;
            }
        };
        labels.push(l);
        opts.labels = labels.clone();
        match merge(&mut store, &s, opts.clone()) {
            Ok(r) => {
                for d in r.report.iter() {
                    eprintln!("{path}: {d}");
                }
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

    let out = global.get_one::<String>("output");
    let bytes = store.log_bytes();
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
    println!(
        "{path}: retracting {target} would reach {} unit(s), orphaning {}",
        plan.blast_radius.len(),
        plan.orphaned.len()
    );
    for u in &plan.orphaned {
        println!("{path}:   {u} would lose all of its grounds");
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

    let sal = smysl::salience(
        &store,
        &SalienceRequest::default().seeded(smysl::view_roots(&store)),
    );
    let packed = match smysl::pack(&store, &sal, &req) {
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

    if surface {
        let text = emit_pack_surface(&store, &packed, &labels);
        let mut stdout = std::io::stdout().lock();
        if stdout.write_all(text.as_bytes()).is_err() {
            return ExitCode::Failure;
        }
    } else {
        let records: Vec<Record> = packed
            .selection
            .keys()
            .filter_map(|u| store.get(u).map(|unit| Record::Unit(unit.core.clone())))
            .collect();
        let mut bytes = smysl::to_cbor_seq(&records);
        bytes.extend_from_slice(&smysl::to_cbor(&Record::PackInfo(packed.info.clone())));
        let mut stdout = std::io::stdout().lock();
        if stdout.write_all(&bytes).is_err() {
            return ExitCode::Failure;
        }
    }
    ExitCode::Success
}

/// `8000` or `8k`.
fn parse_budget(s: &str) -> Option<u64> {
    match s.strip_suffix(['k', 'K']) {
        Some(n) => n.parse::<u64>().ok().map(|v| v * 1000),
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
        if parts.len() != 3 {
            eprintln!("smysl salience: --weights takes three numbers, c,r,t");
            return ExitCode::Usage;
        }
        req = req.with_weights(SalienceWeights {
            centrality: parts[0],
            corroboration: parts[1],
            role: parts[2],
        });
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
    for (uid, score) in report.top(n) {
        println!("{score:.4}  {uid}");
    }
    ExitCode::Success
}

/// `smysl thread` - derive, list, or show a thread (§19, §23.1).
///
/// Derivation is pure: no model is consulted, so the same store yields the same thread on
/// any machine. `--refine`, which does consult one, is what makes the command *mixed* - and
/// it arrives with the provider layer, because there is nothing to refine with until then.
fn cmd_thread(m: &ArgMatches, global: &ArgMatches) -> ExitCode {
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

    let Some(raw) = m.get_one::<String>("derive") else {
        eprintln!("smysl thread: one of --derive, --list or --show is required");
        return ExitCode::Usage;
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
        None => smysl::ThreadId::new(format!("t/{schema}")).expect("a schema name is a valid id"),
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
        for (added, needed_by) in &report.repaired {
            eprintln!("{path}: {added} added by repair; {needed_by} depends on it");
        }
        eprintln!("{path}: {} unit(s) not selected", report.unselected);
    }

    let ctx = WriteContext::from_labels(&labels);
    print!("{}", write_surface(None, &[Record::Thread(thread)], &ctx));
    ExitCode::Success
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
        "fmt" => cmd_fmt(sub),
        "check" => cmd_check(sub, &matches),
        "merge" => cmd_merge(sub, &matches),
        "diff" => cmd_diff(sub, &matches),
        "trace" => cmd_trace(sub, &matches),
        "view" => cmd_view(sub, &matches),
        "bundle" => cmd_bundle(sub, &matches),
        "salience" => cmd_salience(sub, &matches),
        "pack" => cmd_pack(sub, &matches),
        "retract" => cmd_retract(sub, &matches),
        "thread" => cmd_thread(sub, &matches),
        "reindex" => cmd_reindex(sub, &matches),
        _ => {
            eprintln!(
                "smysl {}: not wired in this build; lands in {} (see RFC SMYSL-1 \u{00a7}26)",
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

    #[test]
    fn command_table_matches_section_23() {
        assert_eq!(COMMANDS.len(), 18);
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
                "retract",
                "render",
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

    #[test]
    fn every_command_names_the_phase_that_wires_it() {
        for c in COMMANDS {
            assert!(c.phase.starts_with("SM-P"), "{}: bad phase", c.name);
            assert!(!c.about.is_empty(), "{}: no description", c.name);
        }
    }
}
