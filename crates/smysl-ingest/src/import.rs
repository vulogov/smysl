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

use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::{
    canonical_uid, AgentId, Attestation, Diagnostic, GranularityProfile, Hlc, KernelType, Op,
    Record, Rung, SourceKind, SourceRef, Span, Status, UnitCore, UnitCoreBuilder,
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

        let full = match (subject.is_empty(), values.is_empty()) {
            (_, true) => subject.join(", "),
            (true, _) => values.join(", "),
            _ => format!("{}: {}", subject.join(", "), values.join(", ")),
        };
        let gist = fit_gist(&full);
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

/// A row's summary, within the gist bound `check` enforces.
///
/// Key columns first, then the values, cut at `l0_max` as the estimator counts it — four bytes a
/// token — on a word boundary, with an ellipsis. Until R12 (1.4) the whole row went into the gist,
/// so a row of seven columns, or three with a long test name, imported as a `measured` unit that
/// `smysl check` then refused with `SMY-E022`. Nothing is lost to the cut: every cell is in the
/// payload. A gist that already fits is unchanged, so an import of an ordinary file keeps its uids.
fn fit_gist(full: &str) -> String {
    let budget = GranularityProfile::default().l0_max as usize * 4;
    if full.len() <= budget {
        return full.to_string();
    }
    let ellipsis = '\u{2026}'.len_utf8();
    let mut out = String::new();
    for c in full.chars() {
        if out.len() + c.len_utf8() + ellipsis > budget {
            break;
        }
        out.push(c);
    }
    // At a cell boundary if the cut leaves one, so the gist does not end on a column name whose
    // value was cut away; at a word otherwise, when a single key is longer than the bound.
    let boundary = [", ", ": "]
        .iter()
        .filter_map(|sep| out.rfind(sep))
        .max()
        .filter(|&i| i > 0)
        .or_else(|| out.rfind(char::is_whitespace).filter(|&i| i > 0));
    if let Some(i) = boundary {
        out.truncate(i);
    }
    let trimmed = out.trim_end_matches([',', ':', ' ']).len();
    out.truncate(trimmed);
    out.push('\u{2026}');
    out
}

/// The row as deterministic CBOR, keyed by column name, through the core's canonical encoder.
///
/// It was hand-encoded, and the hand encoding had two limits nobody had met: a map header that
/// could not count past 23 columns, and a text head that could not say more than 255 bytes, so a
/// wider row silently lost columns and a longer cell was cut — in the one field documented as
/// keeping the row verbatim. The core encoder has neither limit, normalises to NFC as every other
/// text field is, and produces the same bytes the hand encoding did wherever that one was right.
/// A column named twice keeps its first cell.
fn row_payload(columns: &[String], cells: &[&str]) -> Vec<u8> {
    let span = Span::new(0, 0);
    let mut o = HObject::new();
    for (k, v) in columns.iter().zip(cells.iter()) {
        if o.contains(k) {
            continue;
        }
        o.insert(
            Spanned::new(k.clone(), span),
            Spanned::new(HValue::Str((*v).to_string()), span),
        );
    }
    smysl_core::surface::payload::object_to_payload(&o).unwrap_or_else(|| vec![0xa0])
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

    /// R12. A long key column, or a wide row, fits the gist bound and checks clean, and every cell
    /// is still in the payload.
    #[test]
    fn a_long_or_wide_row_checks_without_e022_and_keeps_every_cell() {
        let long_test = format!("tests::{}", "a_very_long_test_name_".repeat(4));
        assert!(long_test.len() >= 90);
        let csv = format!(
            "test,commit,outcome,run_seconds,toolchain,target,profile\n\
             {long_test},4968383,passed,0.1,stable-1.89,aarch64-apple-darwin,debug\n"
        );
        for key in [vec!["test".to_string()], Vec::new()] {
            let mut o = opts();
            o.key = key.clone();
            let out = from_csv(&csv, &o);
            assert_eq!(out.units.len(), 1, "{:?}", out.diagnostics);
            let u = &out.units[0];
            assert!(
                smysl_core::tokens(&u.gist) <= GranularityProfile::default().l0_max,
                "{key:?}: {} tokens: {}",
                smysl_core::tokens(&u.gist),
                u.gist
            );
            assert!(u.gist.starts_with("test tests::"), "{}", u.gist);
            assert!(u.gist.ends_with('\u{2026}'), "{}", u.gist);
            assert!(
                !u.gist.contains(": commit"),
                "cut inside a cell rather than between cells: {}",
                u.gist
            );

            let store = Store::from_records(out.records());
            let report = check(&store, CheckOptions::default());
            assert!(
                !report.iter().any(|d| d.code == smysl_core::Code::E022),
                "{key:?}: {report}"
            );
            assert!(
                report.fail_on(smysl_core::Severity::Error).is_ok(),
                "{report}"
            );

            let payload = smysl_core::surface::payload::payload_to_object(
                u.payload.as_ref().expect("a payload"),
            )
            .unwrap();
            for (col, cell) in [
                ("test", long_test.as_str()),
                ("commit", "4968383"),
                ("toolchain", "stable-1.89"),
                ("profile", "debug"),
            ] {
                assert_eq!(
                    payload.get(col).and_then(|v| v.value.as_str()),
                    Some(cell),
                    "{col}"
                );
            }
        }
    }

    /// The payload keeps a row wider than 23 columns and a cell longer than 255 bytes, which the
    /// hand encoding could not.
    #[test]
    fn the_payload_keeps_wide_rows_and_long_cells() {
        let columns: Vec<String> = (0..30).map(|i| format!("c{i}")).collect();
        let long = "x".repeat(300);
        let mut row: Vec<String> = (0..30).map(|i| i.to_string()).collect();
        row[29] = long.clone();
        let csv = format!("{}\n{}\n", columns.join(","), row.join(","));
        let out = from_csv(&csv, &opts());
        let payload =
            smysl_core::surface::payload::payload_to_object(out.units[0].payload.as_ref().unwrap())
                .unwrap();
        assert_eq!(payload.len(), 30);
        assert_eq!(
            payload.get("c29").and_then(|v| v.value.as_str()),
            Some(long.as_str())
        );
    }

    /// Where the old hand encoding was right, the core encoder writes the same bytes, so an import
    /// of an ordinary file keeps the uids it had.
    #[test]
    fn an_ordinary_row_encodes_as_it_did_before() {
        fn old(columns: &[String], cells: &[&str]) -> Vec<u8> {
            let mut pairs: Vec<(&str, &str)> = columns
                .iter()
                .map(String::as_str)
                .zip(cells.iter().copied())
                .collect();
            pairs.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(b.0)));
            let mut out = vec![0xa0 | pairs.len() as u8];
            for (k, v) in pairs {
                for s in [k, v] {
                    let b = s.as_bytes();
                    if b.len() < 24 {
                        out.push(0x60 | b.len() as u8);
                    } else {
                        out.push(0x78);
                        out.push(b.len() as u8);
                    }
                    out.extend_from_slice(b);
                }
            }
            out
        }
        let columns: Vec<String> = ["region", "p50_ms", "p95_ms", "a_long_column_name_here_ok"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let cells = [
            "eu-west",
            "180",
            "610",
            "a cell value that is longer than 24 bytes",
        ];
        assert_eq!(row_payload(&columns, &cells), old(&columns, &cells));

        // And the gist of an ordinary row is what it always was.
        let out = from_csv(CSV, &opts());
        assert_eq!(out.units[0].gist, "region eu-west: p50_ms 180, p95_ms 610");
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
