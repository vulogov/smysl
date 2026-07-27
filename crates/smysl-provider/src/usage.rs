//! The usage ledger (§7.3 `.smysl/usage.log`, §29).
//!
//! Counts, models, task, and recipe. **Never prompt or completion text** - the ledger is
//! written on every model call and read by anyone auditing cost, so putting content in it
//! would create a second copy of everything the store already holds, outside the log's
//! integrity guarantees and outside retraction.
//!
//! One line per call, append-only, so a concurrent writer cannot corrupt an earlier entry
//! and a truncated file is still readable up to its last complete line.
//!
//! Informational. Caps never block (§23.1): a ledger that could stop work would make cost
//! accounting a failure mode.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use smysl_core::error::ProviderError;
use smysl_core::json_escape;

use crate::{ProviderId, Task, Usage};

/// One model call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LedgerEntry {
    /// Milliseconds since the Unix epoch. Supplied by the caller, never read here, so the
    /// ledger stays a pure function of what it is told (guarantee A2).
    pub at: u64,
    pub provider: ProviderId,
    pub model: String,
    pub task: Task,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated: bool,
    pub retries: u32,
    /// The recipe hash, when the caller has one (D-8). Short form.
    pub recipe: Option<String>,
    /// A run identifier, so `usage --by run` can group one command's calls.
    pub run: Option<String>,
}

impl LedgerEntry {
    pub fn new(
        at: u64,
        provider: ProviderId,
        model: impl Into<String>,
        task: Task,
        u: Usage,
    ) -> LedgerEntry {
        LedgerEntry {
            at,
            provider,
            model: model.into(),
            task,
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            estimated: u.estimated,
            retries: u.retries,
            recipe: None,
            run: None,
        }
    }

    pub fn with_recipe(mut self, r: impl Into<String>) -> LedgerEntry {
        self.recipe = Some(r.into());
        self
    }

    pub fn with_run(mut self, r: impl Into<String>) -> LedgerEntry {
        self.run = Some(r.into());
        self
    }

    pub const fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    /// One JSON object on one line. Keys in a fixed order, so a diff over the ledger is a
    /// diff over what happened.
    pub fn to_line(&self) -> String {
        let mut s = String::new();
        s.push('{');
        s.push_str(&format!("\"at\":{}", self.at));
        s.push_str(&format!(
            ",\"provider\":{}",
            json_escape(self.provider.as_str())
        ));
        s.push_str(&format!(",\"model\":{}", json_escape(&self.model)));
        s.push_str(&format!(",\"task\":{}", json_escape(self.task.as_str())));
        s.push_str(&format!(",\"in\":{}", self.input_tokens));
        s.push_str(&format!(",\"out\":{}", self.output_tokens));
        s.push_str(&format!(",\"estimated\":{}", self.estimated));
        s.push_str(&format!(",\"retries\":{}", self.retries));
        if let Some(r) = &self.recipe {
            s.push_str(&format!(",\"recipe\":{}", json_escape(r)));
        }
        if let Some(r) = &self.run {
            s.push_str(&format!(",\"run\":{}", json_escape(r)));
        }
        s.push('}');
        s
    }

    /// Read one line back.
    ///
    /// A hand-rolled reader for a format this crate also writes: pulling in a JSON parser
    /// to read eight scalar fields would be a dependency in service of nothing. Unknown
    /// keys are ignored, so a ledger written by a later version still reads.
    pub fn from_line(line: &str) -> Option<LedgerEntry> {
        let fields = scan(line);
        Some(LedgerEntry {
            at: fields.get("at")?.parse().ok()?,
            provider: ProviderId::new(fields.get("provider")?.clone())?,
            model: fields.get("model")?.clone(),
            task: Task::parse(fields.get("task")?)?,
            input_tokens: fields.get("in")?.parse().ok()?,
            output_tokens: fields.get("out")?.parse().ok()?,
            estimated: fields
                .get("estimated")
                .map(|v| v == "true")
                .unwrap_or(false),
            retries: fields
                .get("retries")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            recipe: fields.get("recipe").cloned(),
            run: fields.get("run").cloned(),
        })
    }
}

/// Split one flat JSON object into string values, unescaping as it goes.
fn scan(line: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let bytes: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < bytes.len() {
        // A key is the next quoted string before a colon.
        while i < bytes.len() && bytes[i] != '"' {
            i += 1;
        }
        let Some((key, next)) = read_string(&bytes, i) else {
            break;
        };
        i = next;
        while i < bytes.len() && bytes[i] != ':' {
            i += 1;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }

        let value = if bytes[i] == '"' {
            match read_string(&bytes, i) {
                Some((v, next)) => {
                    i = next;
                    v
                }
                None => break,
            }
        } else {
            let start = i;
            while i < bytes.len() && bytes[i] != ',' && bytes[i] != '}' {
                i += 1;
            }
            bytes[start..i]
                .iter()
                .collect::<String>()
                .trim()
                .to_string()
        };
        out.insert(key, value);
    }
    out
}

/// Read a quoted string starting at `i`, returning it and the index after the close quote.
fn read_string(s: &[char], i: usize) -> Option<(String, usize)> {
    if s.get(i) != Some(&'"') {
        return None;
    }
    let mut out = String::new();
    let mut j = i + 1;
    while j < s.len() {
        match s[j] {
            '"' => return Some((out, j + 1)),
            '\\' => {
                j += 1;
                match s.get(j)? {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    'r' => out.push('\r'),
                    'b' => out.push('\u{8}'),
                    'f' => out.push('\u{c}'),
                    'u' => {
                        let hex: String = s.get(j + 1..j + 5)?.iter().collect();
                        let code = u32::from_str_radix(&hex, 16).ok()?;
                        out.push(char::from_u32(code)?);
                        j += 4;
                    }
                    c => out.push(*c),
                }
                j += 1;
            }
            c => {
                out.push(c);
                j += 1;
            }
        }
    }
    None
}

/// The ledger.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ledger {
    entries: Vec<LedgerEntry>,
    path: Option<PathBuf>,
}

/// How to group a report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum GroupBy {
    #[default]
    Provider,
    Task,
    Run,
    Model,
}

impl GroupBy {
    pub fn parse(s: &str) -> Option<GroupBy> {
        match s {
            "provider" => Some(GroupBy::Provider),
            "task" => Some(GroupBy::Task),
            "run" => Some(GroupBy::Run),
            "model" => Some(GroupBy::Model),
            _ => None,
        }
    }
}

/// One row of `smysl usage`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Totals {
    pub key: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub retries: u64,
    /// True when any call in the group had estimated counts, so a reader knows the total
    /// is not authoritative.
    pub estimated: bool,
}

impl Totals {
    pub const fn total(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }
}

impl fmt::Display for Totals {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:<24} {:>6} call(s)  {:>9} in  {:>9} out{}",
            self.key,
            self.calls,
            self.input_tokens,
            self.output_tokens,
            if self.estimated { "  (estimated)" } else { "" }
        )
    }
}

impl Ledger {
    pub fn new() -> Ledger {
        Ledger::default()
    }

    /// Where a project's ledger lives, relative to its root.
    pub const PATH: &'static str = ".smysl/usage.log";

    /// Read a ledger, tolerating a truncated or partly corrupt file.
    ///
    /// A ledger is informational, so an unreadable line is skipped rather than fatal:
    /// losing one row of cost accounting must never stop the work that generates it.
    pub fn open(path: impl AsRef<Path>) -> Ledger {
        let path = path.as_ref().to_path_buf();
        let entries = std::fs::read_to_string(&path)
            .map(|text| {
                text.lines()
                    .filter(|l| !l.trim().is_empty())
                    .filter_map(LedgerEntry::from_line)
                    .collect()
            })
            .unwrap_or_default();
        Ledger {
            entries,
            path: Some(path),
        }
    }

    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Append one entry, in memory and - if the ledger has a path - on disk.
    pub fn record(&mut self, e: LedgerEntry) -> Result<(), ProviderError> {
        if let Some(p) = &self.path {
            if let Some(dir) = p.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| ProviderError::Malformed(format!("{}: {e}", dir.display())))?;
            }
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(p)
                .map_err(|e| ProviderError::Malformed(format!("{}: {e}", p.display())))?;
            writeln!(f, "{}", e.to_line()).map_err(|e| ProviderError::Malformed(e.to_string()))?;
        }
        self.entries.push(e);
        Ok(())
    }

    /// Entries at or after a timestamp.
    pub fn since(&self, at: u64) -> Vec<&LedgerEntry> {
        self.entries.iter().filter(|e| e.at >= at).collect()
    }

    /// Group and total. Rows come back in key order, so two runs of `usage` over the same
    /// ledger print the same thing.
    pub fn totals(&self, by: GroupBy, since: Option<u64>) -> Vec<Totals> {
        let mut acc: BTreeMap<String, Totals> = BTreeMap::new();
        for e in self.entries.iter().filter(|e| match since {
            Some(t) => e.at >= t,
            None => true,
        }) {
            let key = match by {
                GroupBy::Provider => e.provider.to_string(),
                GroupBy::Task => e.task.to_string(),
                GroupBy::Model => e.model.clone(),
                GroupBy::Run => e.run.clone().unwrap_or_else(|| "-".into()),
            };
            let row = acc.entry(key.clone()).or_insert(Totals {
                key,
                calls: 0,
                input_tokens: 0,
                output_tokens: 0,
                retries: 0,
                estimated: false,
            });
            row.calls += 1;
            row.input_tokens += e.input_tokens;
            row.output_tokens += e.output_tokens;
            row.retries += e.retries as u64;
            row.estimated |= e.estimated;
        }
        acc.into_values().collect()
    }

    /// Discard everything. `usage --reset`.
    pub fn reset(&mut self) -> Result<(), ProviderError> {
        self.entries.clear();
        if let Some(p) = &self.path {
            match std::fs::remove_file(p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(ProviderError::Malformed(format!("{}: {e}", p.display()))),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> ProviderId {
        ProviderId::new(s).unwrap()
    }

    fn entry(at: u64, provider: &str, task: Task, input: u64, output: u64) -> LedgerEntry {
        LedgerEntry::new(
            at,
            id(provider),
            "llama3.2",
            task,
            Usage {
                input_tokens: input,
                output_tokens: output,
                ..Usage::default()
            },
        )
    }

    #[test]
    fn an_entry_round_trips_through_its_line() {
        let e = entry(1700, "ollama", Task::ContentIngest, 30, 12)
            .with_recipe("b3:abc")
            .with_run("run-1");
        assert_eq!(LedgerEntry::from_line(&e.to_line()), Some(e));
    }

    /// §29: the ledger records counts, models, task, and recipe - never prompt or
    /// completion text. Asserted over the *key set* rather than by scanning for words: the
    /// task is literally called `content-ingest`, so a substring search would both false-
    /// positive on that and miss a field named something innocuous.
    #[test]
    fn a_line_carries_no_content() {
        let e = entry(1, "ollama", Task::ContentIngest, 30, 12)
            .with_recipe("b3:abc")
            .with_run("r");
        let keys: Vec<String> = scan(&e.to_line()).into_keys().collect();
        assert_eq!(
            keys,
            vec![
                "at",
                "estimated",
                "in",
                "model",
                "out",
                "provider",
                "recipe",
                "retries",
                "run",
                "task"
            ],
            "the ledger grew a field; is it content?"
        );
    }

    #[test]
    fn a_line_is_one_line() {
        let e = entry(1, "ollama", Task::ContentIngest, 1, 1);
        assert_eq!(e.to_line().lines().count(), 1);
    }

    #[test]
    fn a_model_name_with_quotes_survives_the_round_trip() {
        let mut e = entry(1, "ollama", Task::Attest, 1, 1);
        e.model = "weird \"name\"\twith\nescapes".into();
        assert_eq!(LedgerEntry::from_line(&e.to_line()), Some(e));
    }

    /// A ledger written by a later version must still read.
    #[test]
    fn an_unknown_key_is_ignored_rather_than_fatal() {
        let mut line = entry(5, "ollama", Task::Attest, 1, 2).to_line();
        line.pop();
        line.push_str(",\"cost_usd\":0.004}");
        let e = LedgerEntry::from_line(&line).expect("still readable");
        assert_eq!(e.at, 5);
        assert_eq!(e.output_tokens, 2);
    }

    #[test]
    fn a_malformed_line_is_none_rather_than_a_panic() {
        for line in [
            "",
            "{",
            "not json",
            "{\"at\":}",
            "{\"at\":\"x\"}",
            "{\"at\":1}",
        ] {
            assert!(LedgerEntry::from_line(line).is_none(), "{line}");
        }
    }

    // -- totals --------------------------------------------------------------

    fn sample() -> Ledger {
        let mut l = Ledger::new();
        l.record(entry(100, "ollama", Task::ContentIngest, 10, 5).with_run("a"))
            .unwrap();
        l.record(entry(200, "ollama", Task::Attest, 20, 7).with_run("a"))
            .unwrap();
        let mut estimated = entry(300, "hosted", Task::ContentIngest, 40, 9).with_run("b");
        estimated.estimated = true;
        estimated.retries = 2;
        l.record(estimated).unwrap();
        l
    }

    #[test]
    fn totals_group_by_provider() {
        let rows = sample().totals(GroupBy::Provider, None);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].key, "hosted");
        assert_eq!(rows[1].key, "ollama");
        assert_eq!(rows[1].input_tokens, 30);
        assert_eq!(rows[1].total(), 42);
    }

    #[test]
    fn totals_group_by_task_run_and_model() {
        let l = sample();
        assert_eq!(l.totals(GroupBy::Task, None).len(), 2);
        assert_eq!(l.totals(GroupBy::Run, None).len(), 2);
        assert_eq!(l.totals(GroupBy::Model, None).len(), 1);
    }

    /// An estimated count in a group makes the whole total non-authoritative, and the row
    /// says so rather than quietly rounding.
    #[test]
    fn an_estimated_call_marks_its_whole_group() {
        let rows = sample().totals(GroupBy::Provider, None);
        assert!(rows[0].estimated, "hosted");
        assert!(!rows[1].estimated, "ollama");
    }

    #[test]
    fn retries_are_counted_in_the_ledger() {
        let rows = sample().totals(GroupBy::Provider, None);
        assert_eq!(rows[0].retries, 2);
    }

    #[test]
    fn since_filters_by_timestamp() {
        let l = sample();
        assert_eq!(l.since(200).len(), 2);
        assert_eq!(l.totals(GroupBy::Provider, Some(250)).len(), 1);
    }

    #[test]
    fn rows_come_back_in_key_order() {
        let l = sample();
        let a: Vec<String> = l
            .totals(GroupBy::Provider, None)
            .iter()
            .map(|r| r.key.clone())
            .collect();
        let mut sorted = a.clone();
        sorted.sort();
        assert_eq!(a, sorted);
    }

    #[test]
    fn group_names_parse() {
        for (s, g) in [
            ("provider", GroupBy::Provider),
            ("task", GroupBy::Task),
            ("run", GroupBy::Run),
            ("model", GroupBy::Model),
        ] {
            assert_eq!(GroupBy::parse(s), Some(g));
        }
        assert_eq!(GroupBy::parse("phase-of-the-moon"), None);
    }

    // -- persistence ---------------------------------------------------------

    fn tmp(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("smysl-ledger-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p.push(Ledger::PATH);
        p
    }

    #[test]
    fn a_ledger_persists_and_reopens() {
        let path = tmp("persist");
        let mut l = Ledger::open(&path);
        assert!(l.is_empty());
        l.record(entry(1, "ollama", Task::Attest, 3, 4)).unwrap();
        l.record(entry(2, "ollama", Task::Attest, 5, 6)).unwrap();

        let reopened = Ledger::open(&path);
        assert_eq!(reopened.entries().len(), 2);
        assert_eq!(reopened.totals(GroupBy::Provider, None)[0].total(), 18);
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    /// Losing one row of cost accounting must never stop the work that generates it.
    #[test]
    fn a_corrupt_line_is_skipped_rather_than_fatal() {
        let path = tmp("corrupt");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let good = entry(1, "ollama", Task::Attest, 3, 4).to_line();
        std::fs::write(&path, format!("{good}\ngarbage\n{{\"at\":\n{good}\n")).unwrap();

        let l = Ledger::open(&path);
        assert_eq!(l.entries().len(), 2, "the readable lines still read");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn a_missing_ledger_is_an_empty_one_rather_than_an_error() {
        assert!(Ledger::open("/nonexistent/smysl/usage.log").is_empty());
    }

    #[test]
    fn reset_empties_the_ledger_and_removes_the_file() {
        let path = tmp("reset");
        let mut l = Ledger::open(&path);
        l.record(entry(1, "ollama", Task::Attest, 1, 1)).unwrap();
        assert!(path.exists());
        l.reset().unwrap();
        assert!(l.is_empty());
        assert!(!path.exists());
        assert!(l.reset().is_ok(), "resetting twice is not an error");
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn an_in_memory_ledger_needs_no_path() {
        let mut l = Ledger::new();
        l.record(entry(1, "ollama", Task::Attest, 1, 1)).unwrap();
        assert_eq!(l.entries().len(), 1);
        assert!(l.reset().is_ok());
    }

    #[test]
    fn the_ledger_path_is_the_documented_one() {
        assert_eq!(Ledger::PATH, ".smysl/usage.log");
    }
}
