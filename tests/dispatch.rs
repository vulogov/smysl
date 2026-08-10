//! Every command in the table is reachable, and accepts what it is documented to accept.
//!
//! Fourteen mutants survived at 1.1 across `cli` and `main`, seven commands with one in each:
//! `ingest`, `usage`, `reindex`, `import`, `relink`, `compact` and `ui`. They are **not** the
//! same mutant twice, and assuming they were is how the first version of this file came to test
//! only half of them:
//!
//! - deleting a command's arm in **`main`** removes its *routing*. The subcommand still parses;
//!   the router falls through to "not wired in this build". Invoking the command finds this.
//! - deleting its arm in **`cli()`** removes its *arguments*, and nothing else. `cli()` registers
//!   all twenty-two subcommands from the `COMMANDS` table unconditionally, so the command is
//!   still there, still routes, still runs — it has simply lost every flag and positional of its
//!   own. Invoking it with no arguments notices nothing at all.
//!
//! That second one was found the only way it could be: by deleting an arm and watching the test
//! keep passing. `every_command_dispatches` covers the first, `the_argument_surface_is_recorded`
//! covers the second, and the second needs a golden file because "what arguments should this
//! command have" has no shorter honest answer.
//!
//! `--help` alone would not do for the first: clap answers `smysl ingest --help` itself, before
//! `main`'s match is reached.

#![cfg(feature = "cli")]

use std::path::PathBuf;
use std::process::{Command, Stdio};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

/// The twenty-two commands, written out rather than read from the binary.
///
/// This list must not come from the thing it is testing. Parsing `smysl --help` for the names
/// and then checking each one dispatches is a check that cannot fail: delete a `cli` arm and the
/// command vanishes from the help *and* from the list, so the loop simply stops testing it and
/// reports success. Deriving the expectation from the subject is the failure this repository
/// keeps rediscovering, and it would have been the natural way to write this file.
///
/// Hardcoding it costs one thing — the list can go stale when a command is added — and
/// `the_help_lists_exactly_these_commands` below is what covers that, in both directions.
const COMMANDS: [&str; 22] = [
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
    "ui",
];

/// Clap's answer when `cli()` never registered the subcommand.
const NOT_REGISTERED: &str = "unrecognized subcommand";

/// `main`'s answer when the table has a command the dispatch does not.
const NOT_WIRED: &str = "not wired in this build";

/// Clap's answer when the invocation never got as far as the router.
///
/// This one is not a defect in the binary — it is a defect in *this test*, and it is checked
/// because it happened. Seven commands take a required argument, and clap rejects the bare name
/// before `main`'s match is reached, so a no-argument invocation cannot see whether the command
/// is routed. Six of the seven were caught anyway by other files that exercise them for real;
/// `import` is exercised by nothing else, and its mutant survived a run of this very test.
///
/// Treating it as a failure rather than silently accepting it is what stops the hole reopening:
/// add a required argument to a command tomorrow and this test says so, instead of quietly
/// ceasing to cover it.
const NOT_REACHED: &str = "required arguments were not provided";

/// The least that has to be typed for a command to reach the router.
///
/// Deliberately invalid where an argument is needed — `smysl trace no-such-uid` fails, and
/// failing is fine. The claim is only that the failure came from the command.
fn minimal_args(command: &str) -> Vec<&'static str> {
    match command {
        "pack" => vec!["--budget", "100"],
        "merge" | "diff" => vec!["no-such-store.smy"],
        "trace" | "retract" => vec!["b3:aaaaaaaaaaaaaaaaaaaaaaaaaa"],
        "find" => vec!["a-query-matching-nothing"],
        "import" => vec!["no-such-file.json"],
        _ => vec![],
    }
}

/// A directory of this command's own, because several of them write.
///
/// `ingest` appends to `.smysl/staged.smy` and `usage` to `.smysl/usage.log`, both resolved
/// against the working directory. Running the twenty-two in a checkout leaves two untracked
/// files behind — which is not hypothetical: probing this by hand did exactly that, and the
/// `usage` invocation then read a ledger the `ingest` invocation had just written. A test that
/// dirties the tree is one step from a test that depends on the tree.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-dispatch-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Run one command with the least it needs, from a directory of its own, and return its output.
///
/// Stdin is `/dev/null` rather than inherited: `fmt` and `check` read stdin when given no path,
/// and a test harness that hands them a terminal would hang instead of failing.
fn dispatch(command: &str) -> String {
    let out = Command::new(BIN)
        .arg(command)
        .args(minimal_args(command))
        .current_dir(scratch(command))
        .stdin(Stdio::null())
        .output()
        .expect("the binary under test must run");
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// Every command in the table can be reached by typing its name.
///
/// The exit code is not asserted. Most of these fail without a store, and *how* they fail is
/// what the rest of the suite is for; the claim here is only that the failure came from the
/// command rather than from the router.
#[test]
fn every_command_dispatches() {
    let mut unreachable = Vec::new();
    for command in COMMANDS {
        let out = dispatch(command);
        if out.contains(NOT_REGISTERED) {
            unreachable.push(format!(
                "{command}: never registered by `cli()` — {NOT_REGISTERED}"
            ));
        } else if out.contains(NOT_WIRED) {
            unreachable.push(format!(
                "{command}: in the table, absent from `main` — {NOT_WIRED}"
            ));
        } else if out.contains(NOT_REACHED) {
            unreachable.push(format!(
                "{command}: this test never reached the router — clap refused the invocation \
                 first. Add what it needs to `minimal_args`."
            ));
        }
    }
    assert!(
        unreachable.is_empty(),
        "{} of {} commands do not dispatch:\n  {}",
        unreachable.len(),
        COMMANDS.len(),
        unreachable.join("\n  ")
    );
}

/// The list above is the same list the binary offers, in the same order.
///
/// This is what keeps the hardcoded array honest, and it fails in both directions: a command
/// removed from `cli()` disappears from the help and is missing here; a command added to the
/// binary and not to the array shows up as an extra. Either way somebody has to type the change,
/// which is the whole point of a golden list.
///
/// `help` is clap's own and is not one of the twenty-two.
#[test]
fn the_help_lists_exactly_these_commands() {
    let out = Command::new(BIN)
        .arg("--help")
        .stdin(Stdio::null())
        .output()
        .expect("the binary under test must run");
    let help = String::from_utf8_lossy(&out.stdout);

    let listed: Vec<&str> = help
        .lines()
        .skip_while(|l| !l.starts_with("Commands:"))
        .skip(1)
        .take_while(|l| !l.trim().is_empty() && l.starts_with("  "))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| *n != "help")
        .collect();

    assert_eq!(
        listed,
        COMMANDS.to_vec(),
        "the binary's commands and this file's list have diverged"
    );
}

/// Every argument every command accepts, against `tests/cli-surface.txt`.
///
/// This is the one that reaches the `cli()` arms. Deleting one strips a command's own flags
/// while leaving it registered and routed, so the only way to see it is to ask what the command
/// accepts and compare against a record kept somewhere the parser cannot edit.
///
/// The comparison is a whole-file `assert_eq!` on the parsed pairs rather than a per-command
/// loop, so a diff shows every drift at once instead of stopping at the first.
#[test]
fn the_argument_surface_is_recorded() {
    let recorded: Vec<(String, String)> = include_str!("cli-surface.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let mut it = l.split_whitespace();
            let c = it.next().expect("a command").to_string();
            let a = it.next().expect("an argument").to_string();
            (c, a)
        })
        .collect();

    let mut actual: Vec<(String, String)> = Vec::new();
    for command in COMMANDS {
        let out = Command::new(BIN)
            .args([command, "--help"])
            .stdin(Stdio::null())
            .output()
            .expect("the binary under test must run");
        let help = String::from_utf8_lossy(&out.stdout);
        for line in help.lines() {
            // Only indented entries. `Usage: smysl fmt [OPTIONS] [FILE]...` sits at column
            // zero and would otherwise contribute a phantom positional to every command.
            if !line.starts_with("  ") {
                continue;
            }
            let t = line.trim_start();
            // A long flag — taken from the first `--`, so `-C, --config <FILE>` and
            // `    --policy <P>` reduce identically. The short form is dropped because every
            // one of them has a long form beside it, and recording both would double the file
            // to say the same thing once.
            let arg = if t.starts_with('-') {
                t.find("--").map(|at| {
                    let flag: String = t[at..]
                        .chars()
                        .take_while(|c| c.is_ascii_lowercase() || *c == '-')
                        .collect();
                    flag
                })
            // A positional, `<STORE>...` or `[PATH]`, kept without its repetition marker
            // because "may be given more than once" is arity rather than surface.
            } else if t.starts_with('<') || t.starts_with('[') {
                let close = if t.starts_with('<') { '>' } else { ']' };
                t.find(close).map(|at| t[..=at].to_string())
            } else {
                None
            };
            if let Some(a) = arg.filter(|s| s.len() > 2) {
                actual.push((command.to_string(), a));
            }
        }
    }

    assert_eq!(
        actual, recorded,
        "the CLI's argument surface moved. If deliberate, run `make cli-surface`."
    );
}
