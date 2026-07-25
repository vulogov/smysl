//! The `smysl` binary - a thin shell over the library (rule A, principle P8).
//!
//! Every subcommand here dispatches into `smysl::*`. Nothing is implemented in the
//! binary that an embedder cannot reach from the library, which is the cheapest way to
//! keep the API honest.
//!
//! SM-P0 declares the full command surface of §23 and reports, per command, the phase
//! that wires it up. Commands are wired only once their library API has stabilised.

use std::process::ExitCode as ProcExitCode;

use clap::{Arg, ArgAction, Command};
use smysl::ExitCode;

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
        cmd = cmd.subcommand(
            Command::new(c.name)
                .about(c.about)
                .after_help(format!("Purity: {}", c.purity.tag()))
                .arg(
                    Arg::new("args")
                        .num_args(0..)
                        .trailing_var_arg(true)
                        .allow_hyphen_values(true)
                        .hide(true),
                ),
        );
    }
    cmd
}

fn main() -> ProcExitCode {
    let matches = cli().get_matches();
    let Some((name, _sub)) = matches.subcommand() else {
        return ProcExitCode::from(ExitCode::Usage.as_i32() as u8);
    };

    let Some(cmd) = COMMANDS.iter().find(|c| c.name == name) else {
        eprintln!("smysl: unknown command `{name}`");
        return ProcExitCode::from(ExitCode::Usage.as_i32() as u8);
    };

    eprintln!(
        "smysl {}: not wired in this build; lands in {} (see RFC SMYSL-1 \u{00a7}26)",
        cmd.name, cmd.phase
    );
    ProcExitCode::from(ExitCode::Failure.as_i32() as u8)
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
