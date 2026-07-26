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
use smysl::{parse_surface, write_surface, Store, StoreOptions, WriteContext};

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
