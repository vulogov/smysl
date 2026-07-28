//! Tool import: tabular readings to `measured` units (rule T's other half).
//!
//! Everything else that produces units goes through a model. This does not: it reads a file
//! and transcribes it, which is why it is the only path allowed to record `measured`.
//!
//! **Why that matters more than it looks.** The distinction between `measured` and
//! `inferred` is the sharpest thing the format carries, and until something wrote the top of
//! the ladder it was a claim about a design rather than a property of a system — every
//! `measured` unit in the corpus was hand-authored. An adapter closes that gap: a reading
//! taken by an instrument enters as `measured`, with an attestation that says which file it
//! came from, and rule M then caps everything reasoning from it at that.
//!
//! No network and no vendor: a file is something you can export from anything, and it keeps
//! this crate's only I/O a `read` the caller already chose to make.
//!
//! ```text
//! region,p50_ms,p95_ms,orders
//! eu-west,180,610,412000        ->  @data  status: measured
//! us-east,175,240,388000            source: { kind: file, ref: "latency.csv#row=1" }
//! ```

use std::collections::BTreeMap;

use smysl_core::{
    canonical_uid, AgentId, Attestation, Diagnostic, Hlc, KernelType, Op, Record, Rung, SourceKind,
    SourceRef, Status, UnitCore, UnitCoreBuilder,
};

/// What an import produced.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Imported {
    pub units: Vec<UnitCore>,
    /// One per unit, recording `op: Imported` at the `computed` rung. Without these the
    /// units would not be permitted `measured` at all — the attestation *is* the licence.
    pub attestations: Vec<Attestation>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Imported {
    pub fn records(&self) -> Vec<Record> {
        let mut out: Vec<Record> = self.units.iter().cloned().map(Record::Unit).collect();
        out.extend(self.attestations.iter().cloned().map(Record::Attestation));
        out
    }

    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }
}

/// How to read the file.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ImportOptions {
    /// What to call the source. The file path, normally.
    pub source: String,
    /// The agent recording the import.
    pub agent: AgentId,
    /// Supplied rather than read, so a replayed import produces identical attestations.
    pub now: Hlc,
    /// The kind of source this is. `file` for an export, `metric` for a scrape.
    pub kind: SourceKind,
    /// Columns to treat as the reading's identity rather than its value. Empty means the
    /// first column.
    pub key: Vec<String>,
}

impl ImportOptions {
    pub fn new(source: impl Into<String>, agent: AgentId, now: Hlc) -> ImportOptions {
        ImportOptions {
            source: source.into(),
            agent,
            now,
            kind: SourceKind::File,
            key: Vec::new(),
        }
    }
}

/// Parse delimiter-separated text into `measured` units, one per row.
///
/// The header row names the columns. Each subsequent row becomes one `data` unit whose gist
/// states the reading and whose payload carries the cells verbatim, so nothing in the file
/// is lost to the summary in the gist.
pub fn from_csv(text: &str, opts: &ImportOptions) -> Imported {
    let mut out = Imported::default();

    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else {
        out.diagnostics.push(
            Diagnostic::new(smysl_core::Code::E001).with_message("the file is empty".to_string()),
        );
        return out;
    };
    let columns: Vec<String> = header.split(',').map(|c| c.trim().to_string()).collect();

    for (i, line) in lines.enumerate() {
        let cells: Vec<&str> = line.split(',').map(str::trim).collect();
        if cells.len() != columns.len() {
            out.diagnostics.push(
                Diagnostic::new(smysl_core::Code::E001).with_message(format!(
                    "row {}: {} cell(s) against {} column(s)",
                    i + 1,
                    cells.len(),
                    columns.len()
                )),
            );
            continue;
        }

        let row: BTreeMap<&str, &str> = columns
            .iter()
            .map(String::as_str)
            .zip(cells.iter().copied())
            .collect();

        // The identity columns name the reading; the rest are its values.
        let key_names: Vec<&str> = if opts.key.is_empty() {
            columns.first().map(String::as_str).into_iter().collect()
        } else {
            opts.key.iter().map(String::as_str).collect()
        };
        let subject: Vec<String> = key_names
            .iter()
            .filter_map(|k| row.get(k).map(|v| format!("{k} {v}")))
            .collect();
        let values: Vec<String> = columns
            .iter()
            .filter(|c| !key_names.contains(&c.as_str()))
            .filter_map(|c| row.get(c.as_str()).map(|v| format!("{c} {v}")))
            .collect();

        let gist = match (subject.is_empty(), values.is_empty()) {
            (_, true) => subject.join(", "),
            (true, _) => values.join(", "),
            _ => format!("{}: {}", subject.join(", "), values.join(", ")),
        };
        if gist.is_empty() {
            continue;
        }

        // A row-addressed source, so a reader can go back to the line this came from. A
        // `measured` unit whose source cannot be checked is a `measured` unit nobody can
        // audit, which is most of the reason to record one.
        let source = SourceRef::new(opts.kind, format!("{}#row={}", opts.source, i + 1));

        match UnitCoreBuilder::new(KernelType::Data, &gist, Status::Measured)
            .source(source)
            .payload(row_payload(&columns, &cells))
            .build()
        {
            Ok(core) => {
                let uid = canonical_uid(&core);
                out.attestations.push(
                    Attestation::new(
                        uid,
                        opts.agent.clone(),
                        // `Imported` at `computed` is what permits `measured`. Nothing else
                        // in the system may record it.
                        Op::Imported,
                        Rung::Computed,
                        opts.now.clone(),
                    )
                    .with_parents(Default::default()),
                );
                out.units.push(core);
            }
            Err(e) => out.diagnostics.push(
                Diagnostic::new(smysl_core::Code::E001).with_message(format!("row {}: {e}", i + 1)),
            ),
        }
    }

    out
}

/// The row as deterministic CBOR, keyed by column name.
///
/// Hand-encoded, because this crate has no JSON or CBOR writer and a map of short text keys
/// to short text values is a few bytes. Keys are emitted in sorted order, which is what the
/// canonical encoding requires.
fn row_payload(columns: &[String], cells: &[&str]) -> Vec<u8> {
    let mut pairs: Vec<(&str, &str)> = columns
        .iter()
        .map(String::as_str)
        .zip(cells.iter().copied())
        .collect();
    // Canonical order is by encoded bytes; for short text keys that is length then value.
    pairs.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(b.0)));

    let mut out = Vec::new();
    out.push(0xa0 | (pairs.len().min(23) as u8));
    for (k, v) in pairs.iter().take(23) {
        text(&mut out, k);
        text(&mut out, v);
    }
    out
}

fn text(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    if b.len() < 24 {
        out.push(0x60 | b.len() as u8);
    } else {
        out.push(0x78);
        out.push(b.len().min(255) as u8);
    }
    out.extend_from_slice(&b[..b.len().min(255)]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_check::{check, CheckOptions};
    use smysl_graph::Store;

    const CSV: &str = "region,p50_ms,p95_ms\n\
                       eu-west,180,610\n\
                       us-east,175,240\n";

    fn opts() -> ImportOptions {
        let agent = AgentId::new("tool:importer").unwrap();
        ImportOptions::new("latency.csv", agent.clone(), Hlc::zero(agent))
    }

    #[test]
    fn each_row_becomes_one_measured_unit() {
        let out = from_csv(CSV, &opts());
        assert_eq!(out.units.len(), 2, "{:?}", out.diagnostics);
        assert!(out.units.iter().all(|u| u.status == Status::Measured));
        assert_eq!(out.attestations.len(), 2, "one licence per unit");
    }

    /// **The point of the whole adapter.** An instrument reading enters as `measured` and
    /// the store accepts it — which nothing else in the system can do, and which was
    /// impossible until an attestation's ceiling stopped depending on the rung alone.
    #[test]
    fn an_imported_measurement_checks_clean() {
        let out = from_csv(CSV, &opts());
        let store = Store::from_records(out.records());
        let report = check(&store, CheckOptions::default());
        assert!(
            report.fail_on(smysl_core::Severity::Error).is_ok(),
            "an imported measurement did not check: {report}"
        );
    }

    /// A `measured` unit whose source cannot be followed is one nobody can audit.
    #[test]
    fn the_source_addresses_the_row_it_came_from() {
        let out = from_csv(CSV, &opts());
        let refs: Vec<String> = out
            .units
            .iter()
            .filter_map(|u| u.source.as_ref().map(|s| s.reference.clone()))
            .collect();
        assert_eq!(refs, ["latency.csv#row=1", "latency.csv#row=2"]);
    }

    /// The gist summarises; the payload keeps the row. Nothing in the file is lost to the
    /// summary, so a later reader can recover what was actually measured.
    #[test]
    fn the_row_survives_verbatim_in_the_payload() {
        let out = from_csv(CSV, &opts());
        let payload = out.units[0].payload.as_ref().expect("a payload");
        let text = String::from_utf8_lossy(payload);
        for cell in ["region", "eu-west", "p95_ms", "610"] {
            assert!(text.contains(cell), "{cell} missing from the payload");
        }
    }

    #[test]
    fn the_gist_names_the_subject_and_its_values() {
        let out = from_csv(CSV, &opts());
        assert!(
            out.units[0].gist.contains("eu-west"),
            "{}",
            out.units[0].gist
        );
        assert!(out.units[0].gist.contains("610"), "{}", out.units[0].gist);
    }

    /// Replaying an import must not grow the log: same file, same clock, same uids.
    #[test]
    fn an_import_is_a_function_of_its_inputs() {
        let a = from_csv(CSV, &opts());
        let b = from_csv(CSV, &opts());
        assert_eq!(a.records(), b.records());
    }

    /// A ragged row is reported and skipped rather than taking the file down: half a
    /// spreadsheet imported is more useful than none.
    #[test]
    fn a_ragged_row_is_skipped_and_reported() {
        let out = from_csv("a,b\n1,2\n3\n4,5\n", &opts());
        assert_eq!(out.units.len(), 2);
        assert_eq!(out.diagnostics.len(), 1);
    }

    #[test]
    fn an_empty_file_is_a_diagnostic_not_a_panic() {
        let out = from_csv("", &opts());
        assert!(out.is_empty());
        assert!(!out.diagnostics.is_empty());
    }

    /// Chosen key columns name the reading; everything else is its value.
    #[test]
    fn the_key_columns_choose_the_subject() {
        let mut o = opts();
        o.key = vec!["p95_ms".into()];
        let out = from_csv(CSV, &o);
        assert!(
            out.units[0].gist.starts_with("p95_ms 610"),
            "{}",
            out.units[0].gist
        );
    }
}
