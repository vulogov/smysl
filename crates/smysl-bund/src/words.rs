//! Bund2's words, and which of them a script may name (RFC SMYSL-4 §6).
//!
//! `words/bund2-0.56.0.txt` is `bund2 words` from a 0.56.0 build with `--features smysl`, read
//! in BQ-P1. The refused list below was made by reading it against Bund2's source. A word not
//! in that file is a word nobody here has read, so the default allowlist does not hold it.
//!
//! Bund2 registers most words twice, `w` acting on the stack and `w.` (or `w,`) on the
//! workbench. A refused entry covers both forms.

use std::collections::BTreeSet;

/// `bund2 words` of the release the lists were read against.
pub const BASELINE: &str = include_str!("../words/bund2-0.56.0.txt");

/// Why a word is refused, by the section of the RFC that refuses it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Refusal {
    /// §6.1: it writes a library, or calls a model.
    LibraryOrModel,
    /// §6.2: it runs code that the script does not spell out.
    RunsCode,
    /// §6.3: it reaches the host, a clock, a terminal, a channel, or randomness.
    Host,
}

/// An entry: a word, or a family when it ends in `*`.
type Entry = (&'static str, Refusal, &'static str);

use Refusal::{Host, LibraryOrModel, RunsCode};

/// §6.1 to §6.3. The third field says why, and is what the gate reports.
pub const REFUSED: &[Entry] = &[
    // §6.1 — library writers.
    ("smysl.create", LibraryOrModel, "creates a library"),
    ("smysl.unit", LibraryOrModel, "writes a unit"),
    ("smysl.rel", LibraryOrModel, "writes a relation"),
    ("smysl.label", LibraryOrModel, "binds a label"),
    ("smysl.append", LibraryOrModel, "appends records"),
    (
        "smysl.text.add",
        LibraryOrModel,
        "adds a text, reading a host path",
    ),
    ("smysl.retract", LibraryOrModel, "retracts a unit"),
    (
        "smysl.prove",
        LibraryOrModel,
        "writes its proofs as units; use smysl.kb with kb.prove",
    ),
    // §6.1 — the model layer (`smysl-model` builds).
    ("smysl.ingest", LibraryOrModel, "calls a model"),
    ("smysl.answer", LibraryOrModel, "calls a model"),
    (
        "smysl.ingest.text",
        LibraryOrModel,
        "calls a model and writes",
    ),
    ("smysl.commit", LibraryOrModel, "writes a staged ingest"),
    // §6.2 — a call by a name the script does not spell out.
    ("!", RunsCode, "executes a value"),
    ("execute", RunsCode, "executes a value"),
    ("!!", RunsCode, "evaluates a string"),
    ("bund.eval", RunsCode, "evaluates a string"),
    ("bund.eval-file", RunsCode, "evaluates a file"),
    ("apply", RunsCode, "applies a value as code"),
    ("lambda!", RunsCode, "runs a lambda built at run time"),
    ("do", RunsCode, "runs a stack as code"),
    (
        "resolve",
        RunsCode,
        "calls a word by a name computed at run time",
    ),
    ("ptr", RunsCode, "makes a pointer to a word by name"),
    ("make.call", RunsCode, "turns a string into a call"),
    ("call,", RunsCode, "turns a string into a call"),
    ("register", RunsCode, "defines a word at run time"),
    ("unregister", RunsCode, "removes a word at run time"),
    ("alias", RunsCode, "renames a word at run time"),
    ("alias=", RunsCode, "renames a word at run time"),
    ("unalias", RunsCode, "renames a word at run time"),
    (
        "compile",
        RunsCode,
        "compiles source text, even under --noeval",
    ),
    ("use", RunsCode, "loads code"),
    ("load", RunsCode, "loads code"),
    ("kb.call", RunsCode, "calls a word by its functor name"),
    ("kb.word", RunsCode, "binds a word into a KB by name"),
    ("kb.consult*", RunsCode, "loads clauses"),
    (
        "?",
        RunsCode,
        "a conditional whose type may be csv or sqlite, reaching files",
    ),
    (
        "conditional",
        RunsCode,
        "a conditional whose type may be csv or sqlite, reaching files",
    ),
    ("query", RunsCode, "the csv and sqlite conditionals' query"),
    // §6.3 — shell and processes.
    ("system.*", Host, "the host system"),
    ("sh", Host, "runs /bin/sh"),
    ("vm.*", Host, "starts a VM thread"),
    ("exit", Host, "ends the process"),
    ("bund.exit", Host, "ends the process"),
    ("bund.prompt", Host, "reads the terminal"),
    // §6.3 — files and fetching.
    ("fs.*", Host, "the filesystem"),
    ("fs_is_file", Host, "the filesystem"),
    ("rm", Host, "the filesystem"),
    ("cp", Host, "the filesystem"),
    ("mv", Host, "the filesystem"),
    ("ls", Host, "the filesystem"),
    ("cwd", Host, "the filesystem"),
    ("filename", Host, "the filesystem"),
    ("file", Host, "reads a file"),
    ("file.*", Host, "the filesystem"),
    ("url", Host, "an HTTP request"),
    ("url.*", Host, "an HTTP request"),
    ("io.*", Host, "files and the terminal"),
    ("csv", Host, "reads files"),
    ("sqlite", Host, "opens a database file"),
    ("save", Host, "writes a world file"),
    ("save.*", Host, "writes files"),
    ("load.*", Host, "reads files"),
    ("bootstrap", Host, "reads files"),
    // §6.3 — time, environment, host identity.
    ("time.*", Host, "the clock"),
    ("sleep", Host, "the clock"),
    ("sleep.*", Host, "the clock"),
    ("args", Host, "the command line"),
    ("args.*", Host, "the command line"),
    ("version", Host, "the host"),
    ("sysinfo.*", Host, "the host"),
    ("debug", Host, "the debugger"),
    ("debug.*", Host, "the debugger, the host, files"),
    // §6.3 — terminal and channels.
    ("input", Host, "reads the terminal"),
    ("input*", Host, "reads the terminal"),
    ("password", Host, "reads the terminal"),
    ("display", Host, "the terminal"),
    ("console.*", Host, "the terminal"),
    ("spinner.*", Host, "the terminal"),
    ("send", Host, "a channel between VMs"),
    ("send.*", Host, "a channel between VMs"),
    ("recv", Host, "a channel between VMs"),
    ("bus.*", Host, "a channel between VMs"),
    ("log.*", Host, "standard error"),
    ("json.path", Host, "prints as a side effect"),
    // §6.3 — randomness, and what is seeded from it.
    ("rng.*", Host, "random"),
    ("math.random.*", Host, "random"),
    ("math.securerandom.*", Host, "random"),
    ("string.random.*", Host, "random"),
    ("id.*", Host, "random identifiers"),
    ("dist.sample*", Host, "random"),
    ("generator", Host, "random"),
    ("generator.*", Host, "random"),
    ("sim.*", Host, "random"),
    ("popgen.wf*", Host, "random"),
    ("fmt", Host, "reads the locale"),
];

/// The stack form of a workbench word: `w.` and `w,` are `w`'s.
fn stem(word: &str) -> &str {
    match word.strip_suffix('.').or_else(|| word.strip_suffix(',')) {
        Some(s) if !s.is_empty() => s,
        _ => word,
    }
}

fn matches(entry: &str, word: &str) -> bool {
    match entry.strip_suffix('*') {
        Some(prefix) => word.starts_with(prefix),
        None => entry == word,
    }
}

/// Why `word` is refused, if it is, in either of its forms.
pub fn refused(word: &str) -> Option<&'static Entry> {
    let forms = [word, stem(word)];
    REFUSED
        .iter()
        .find(|(entry, _, _)| forms.iter().any(|w| matches(entry, w)))
}

/// Every word of [`BASELINE`].
pub fn baseline() -> BTreeSet<String> {
    BASELINE
        .lines()
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// §6.4: the baseline less every refused word, and less `smysl.open`, which only the bridge's
/// preamble names.
pub fn default_allowlist() -> BTreeSet<String> {
    baseline()
        .into_iter()
        .filter(|w| refused(w).is_none() && stem(w) != "smysl.open")
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_baseline_is_the_whole_listing() {
        let words = baseline();
        assert_eq!(words.len(), 1210);
        assert_eq!(words.iter().filter(|w| w.starts_with("smysl")).count(), 24);
    }

    /// Every refused entry that names one word names a word Bund2 0.56.0 has, so a typo cannot
    /// leave a word allowed while the list seems to refuse it. The model words are the
    /// exception: this baseline is a build without `smysl-model`.
    #[test]
    fn every_refused_word_exists() {
        let words = baseline();
        let model = [
            "smysl.ingest",
            "smysl.answer",
            "smysl.ingest.text",
            "smysl.commit",
        ];
        for (entry, _, _) in REFUSED {
            if entry.ends_with('*') || model.contains(entry) {
                continue;
            }
            let workbench = [format!("{entry}."), format!("{entry},")];
            assert!(
                words.contains(*entry) || workbench.iter().any(|w| words.contains(w)),
                "{entry} is not a Bund2 0.56.0 word"
            );
        }
    }

    /// And every family matches at least one word.
    #[test]
    fn every_refused_family_matches() {
        let words = baseline();
        for (entry, _, _) in REFUSED {
            if let Some(prefix) = entry.strip_suffix('*') {
                assert!(
                    words.iter().any(|w| w.starts_with(prefix)),
                    "{entry} matches nothing"
                );
            }
        }
    }

    #[test]
    fn a_workbench_form_is_refused_with_its_word() {
        assert!(refused("system.shell.").is_some());
        assert!(refused("sh.").is_some());
        assert!(refused("make.call").is_some());
        assert!(refused("call,").is_some());
        assert!(refused("execute.").is_some());
        assert!(refused("ls.").is_some());
    }

    /// The allowlist keeps what a read-only query needs, and none of what §6 refuses.
    #[test]
    fn the_allowlist() {
        let allow = default_allowlist();
        for w in [
            "smysl.query",
            "smysl.get",
            "smysl.trace",
            "smysl.kb",
            "smysl",
            "kb.query",
            "kb.prove",
            "kb.inferences",
            "kb!",
            "dup",
            "swap",
            "+",
            "map",
            "loop",
            "if",
            "var",
            "var?",
            "get",
            "set",
            "len",
            "println",
            "type",
            "lambda",
            "?try",
        ] {
            assert!(allow.contains(w), "{w} should be allowed");
        }
        for w in [
            "smysl.open",
            "smysl.create",
            "smysl.prove",
            "system.shell",
            "sh",
            "!",
            "!!",
            "make.call",
            "call,",
            "resolve",
            "compile",
            "url",
            "file",
            "time.now",
            "rng.uniform",
            "id.uuid",
            "vm.spawn",
            "debug.display_stack",
            "alias",
            "?",
            "conditional",
            "sqlite",
        ] {
            assert!(!allow.contains(w), "{w} should not be allowed");
        }
    }
}
