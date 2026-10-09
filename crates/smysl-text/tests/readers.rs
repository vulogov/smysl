//! TX-P1 step 3's exit: every reader, over a real sample, with every identity pinned.
//!
//! One sample per reader in `fixtures/library/readers/`, read the way `text add` will read it
//! — normalise, build the structure, cut parts at the reader's own top level, make a reading,
//! build a manifest — and compared against the `.expected` file beside it. `SMYSL_BLESS=1`
//! writes those files; without it they are only ever read, so a change to a reader fails this
//! suite instead of quietly updating its own expectations.
//!
//! What is pinned is what a corpus is addressed by: the tid of each part, the structure hash,
//! the rdid and the mid. A reader is the thing those are derived from, so a reader that
//! changed its output by one byte would rename every part taken with it — which is why
//! SMYSL-2.4 §7 pins the parsers with `=` and why this file exists.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::ids::LangTag;
use smysl_core::types::Carry;
use smysl_text::limits::{Budget, Caps};
use smysl_text::part::{self, Policy};
use smysl_text::readers::{self, Input, Params, ReadOutput};
use smysl_text::reading::Reading;
use smysl_text::structure::Structure;
use smysl_text::ManifestBuilder;

/// A sample: the file, the reader that reads it, the alias and licence a manifest records.
struct Sample {
    file: &'static str,
    reader: &'static str,
    alias: &'static str,
    lang: &'static str,
    licence: &'static str,
    /// The reader's parameters, which are part of what produced the sample.
    ///
    /// Empty for the six TX-P1 readers. `whatsapp/1` is why the field exists: its date order is
    /// required, so a sample read without it is not a sample at all — and the field a manifest
    /// records is `whatsapp/1 date-format=dmy`, which means the expectation below pins the
    /// settings along with the identities.
    params: &'static [(&'static str, &'static str)],
}

const SAMPLES: &[Sample] = &[
    Sample {
        file: "gen1.usfm",
        reader: "usfm/1",
        alias: "kjv1769",
        lang: "en",
        licence: "public-domain",
        params: &[],
    },
    // A verse bridge, which is the only place a reader emits a *range* locator. It is here
    // because a bridge is a node whose own address is a range, and the resolver used to
    // decompose every range into its ends — so no bridge could be found by the address it was
    // emitted under. Found by fuzzing; pinned here so the fixtures cover the shape.
    Sample {
        file: "exod20.usfm",
        reader: "usfm/1",
        alias: "kjv1769-exodus",
        lang: "en",
        licence: "public-domain",
        params: &[],
    },
    Sample {
        file: "gen1.osis",
        reader: "osis/1",
        alias: "kjv1769-osis",
        lang: "en",
        licence: "public-domain",
        params: &[],
    },
    Sample {
        file: "gen1-milestone.osis",
        reader: "osis/1",
        alias: "kjv1769-milestone",
        lang: "en",
        licence: "public-domain",
        params: &[],
    },
    Sample {
        file: "gen1.zefania",
        reader: "zefania/1",
        alias: "luther1912",
        lang: "de",
        licence: "public-domain",
        params: &[],
    },
    Sample {
        file: "notes.md",
        reader: "md/1",
        alias: "notes-md",
        lang: "en",
        licence: "CC0-1.0",
        params: &[],
    },
    Sample {
        file: "notes.txt",
        reader: "txt/1",
        alias: "notes-txt",
        lang: "en",
        licence: "CC0-1.0",
        params: &[],
    },
    Sample {
        file: "chat.json",
        reader: "json/1",
        alias: "exchange",
        lang: "en",
        licence: "CC0-1.0",
        params: &[],
    },
    // The three chat readers (TX-P2 step 2). `mul` rather than `en`: each sample holds English
    // and Russian messages, which is the shape a chat export actually has and the reason
    // segment key 4 (a row's own language) exists.
    Sample {
        file: "telegram.json",
        reader: "telegram/1",
        alias: "reading-group-telegram",
        lang: "mul",
        licence: "CC0-1.0",
        params: &[],
    },
    Sample {
        file: "whatsapp.txt",
        reader: "whatsapp/1",
        alias: "reading-group-whatsapp",
        lang: "mul",
        licence: "CC0-1.0",
        params: &[("date-format", "dmy")],
    },
    Sample {
        file: "slack.zip",
        reader: "slack/1",
        alias: "reading-group-slack",
        lang: "mul",
        licence: "CC0-1.0",
        params: &[],
    },
];

/// The sample with this file name, so a test names what it means rather than an index.
///
/// Only the cross-format tests need it, and both of them need `reader-osis` — so the helper
/// carries the same gate. Without it, a build with one reader feature has a function nothing
/// calls, and `make crate-features` compiles with `-D warnings`.
#[cfg(feature = "reader-osis")]
fn sample(file: &str) -> &'static Sample {
    SAMPLES
        .iter()
        .find(|s| s.file == file)
        .unwrap_or_else(|| panic!("{file} is not a sample"))
}

/// The samples whose reader this build has, with a check that nothing is skipped silently.
///
/// Each reader sits behind its own feature, so `cargo test -p smysl-text --features reader-txt`
/// has one reader and `--all-features` has six. A suite that asked for an unbuilt reader would
/// fail that configuration — `make crate-features` is what runs them — and a suite that just
/// skipped would pass while checking nothing. So the skipping is deliberate *and* guarded:
/// every reader this build has must have a sample here.
fn built_samples() -> Vec<&'static Sample> {
    let built: Vec<&Sample> = SAMPLES
        .iter()
        .filter(|s| readers::reader(s.reader).is_ok())
        .collect();
    for id in readers::available() {
        assert!(
            built.iter().any(|s| s.reader == id),
            "{id} is built and has no sample in fixtures/library/readers"
        );
    }
    built
}

fn fixtures() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is this crate; the fixtures are the workspace's. No absolute path
    // is ever printed: a diagnostic, an expectation file or a failure message carrying one
    // would put somebody's home directory into the repository.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root is two levels up")
        .join("fixtures/library/readers")
}

fn read(sample: &Sample) -> (ReadOutput, u64) {
    let path = fixtures().join(sample.file);
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("fixtures/library/readers/{}: {e}", sample.file));
    let reader =
        readers::reader(sample.reader).unwrap_or_else(|e| panic!("{}: {e}", sample.reader));
    let mut params = Params::new();
    for (key, value) in sample.params {
        params
            .set(sample.reader, *key, *value)
            .unwrap_or_else(|e| panic!("{}: --param {key}={value}: {e}", sample.file));
    }
    let mut budget = Budget::new(Caps::DEFAULT, bytes.len() as u64).expect("a budget");
    let out = readers::read_with(reader, &Input::new(&bytes), &params, &mut budget)
        .unwrap_or_else(|e| panic!("{}: {e}", sample.file));
    (out, bytes.len() as u64)
}

/// Everything a corpus would record for one sample, rendered for comparison.
fn describe(sample: &Sample, out: &ReadOutput) -> String {
    let mut budget = Budget::new(Caps::DEFAULT, out.text.len() as u64).expect("a budget");
    let structure = Structure::build(&out.rows, out.text.len() as u64, &mut budget)
        .unwrap_or_else(|e| panic!("{}: the rows are not a structure: {e}", sample.file));

    // The part policy is the reader's own top level, not the default `chapter`: a policy whose
    // boundary level names no node in the reading would cut the text into **no parts at all**,
    // and a text that silently becomes nothing is the one outcome worth refusing outright.
    let policy = Policy {
        boundary_level: out.top_level.clone(),
        ..Policy::default()
    };
    let boundaries: Vec<std::ops::Range<u64>> = structure
        .at_level(&out.top_level)
        .map(|n| n.range.clone())
        .collect();
    assert!(
        !boundaries.is_empty(),
        "{}: the policy's boundary level `{}` names no node",
        sample.file,
        out.top_level
    );
    let plans = part::group(&boundaries, &policy, out.text.len() as u64, &mut budget)
        .unwrap_or_else(|e| panic!("{}: grouping: {e}", sample.file));

    let lang = LangTag::new(sample.lang).expect("a language tag");
    // The field a manifest records is the id **with its settings** (key 3), not the id. A
    // fixture that pinned the bare id would pin a mid no `text add` could reproduce.
    let mut params = Params::new();
    for (key, value) in sample.params {
        params
            .set(sample.reader, *key, *value)
            .expect("a parameter");
    }
    let reader_field = readers::reader_field(sample.reader, &params);
    let mut manifest = ManifestBuilder::new(
        sample.alias,
        lang.clone(),
        &reader_field,
        sample.licence,
        &policy,
    )
    .unwrap_or_else(|e| panic!("{}: {e}", sample.file))
    .carry(Carry::Text);
    if let Some(title) = &out.title {
        manifest = manifest.title(title.clone());
    }
    // `raw` is inside the manifest and therefore inside the mid, so the chat round trip
    // SMYSL-2.4 asks for — "with `raw` intact" — is pinned by the mid below as well as by the
    // bytes printed with it.
    if let Some(raw) = &out.raw {
        manifest = manifest.raw(raw.clone());
    }

    let mut rendered = String::new();
    writeln!(rendered, "file      {}", sample.file).expect("write");
    writeln!(rendered, "reader    {}", sample.reader).expect("write");
    writeln!(rendered, "alias     {}", sample.alias).expect("write");
    writeln!(rendered, "licence   {}", sample.licence).expect("write");
    writeln!(rendered, "reader-field {reader_field}").expect("write");
    writeln!(rendered, "policy    {}", policy.id()).expect("write");
    writeln!(rendered, "top-level {}", out.top_level).expect("write");
    writeln!(rendered, "lossy     {}", out.lossy).expect("write");
    writeln!(
        rendered,
        "title     {}",
        out.title.as_deref().unwrap_or("-")
    )
    .expect("write");
    writeln!(
        rendered,
        "lang      {}",
        out.lang.as_ref().map(|l| l.as_str()).unwrap_or("-")
    )
    .expect("write");
    writeln!(rendered, "text      {} bytes", out.text.len()).expect("write");
    writeln!(rendered, "nodes     {}", out.rows.len()).expect("write");
    writeln!(rendered, "parts     {}", plans.len()).expect("write");
    if let Some(raw) = &out.raw {
        writeln!(rendered, "raw       {} bytes {}", raw.len(), bytes_hex(raw)).expect("write");
    }

    for (index, plan) in plans.iter().enumerate() {
        let text = part::text_of(&out.text, plan.range.clone())
            .unwrap_or_else(|| panic!("{}: part {index} is not a normalised slice", sample.file));
        let rows: Vec<_> = out
            .rows
            .iter()
            .filter(|r| r.start >= plan.range.start && r.end <= plan.range.end)
            .cloned()
            .collect();
        let reading = Reading {
            tid: text.tid,
            reader: sample.reader.to_string(),
            rows,
            raw: None,
        };
        let entry = reading.entry(plan.len());
        writeln!(rendered).expect("write");
        writeln!(rendered, "part {index}").expect("write");
        writeln!(
            rendered,
            "  range     {}..{}",
            plan.range.start, plan.range.end
        )
        .expect("write");
        // The canonical 52-character form, not `Display`'s 26-character short one. `Tid::parse`
        // refuses an abbreviation for the reason `Uid::parse` does — an abbreviated identity
        // weakens identity silently — so a fixture that pinned the short form would be pinning
        // something no reader of the corpus could parse back.
        writeln!(rendered, "  tid       {}", text.tid.canonical()).expect("write");
        writeln!(rendered, "  structure {}", hex(&reading.structure_hash())).expect("write");
        writeln!(rendered, "  rdid      {}", reading.rdid().canonical()).expect("write");
        writeln!(rendered, "  segments  {}", reading.rows.len()).expect("write");
        // The entry is what the manifest carries, and verifying it here is what makes the
        // pinned hashes a claim about the *round trip* rather than about one computation.
        reading
            .verify_entry(&entry)
            .unwrap_or_else(|e| panic!("{}: part {index}: {e}", sample.file));
        manifest = manifest.part(entry);
    }

    let manifest = manifest
        .build()
        .unwrap_or_else(|e| panic!("{}: building the manifest: {e}", sample.file));
    writeln!(rendered).expect("write");
    writeln!(rendered, "mid       {}", manifest.mid().canonical()).expect("write");

    writeln!(rendered, "\nlocators").expect("write");
    for row in &out.rows {
        // The metadata half of a row — who, when, and the platform's own ids — is printed only
        // where a reader filled it, so the six TX-P1 expectations are unchanged by its arrival
        // and the three chat ones say what a chat reading holds.
        let mut extra = String::new();
        if let Some(who) = &row.speaker {
            write!(extra, " spk={who}").expect("write");
        }
        if let Some(at) = row.observed {
            write!(extra, " at={at}").expect("write");
        }
        if let Some(tz) = row.tz_offset {
            write!(extra, " tz={tz}").expect("write");
        }
        for (key, value) in &row.ids {
            write!(extra, " {key}={value}").expect("write");
        }
        writeln!(
            rendered,
            "  {:<9} {:<28} {}..{}{extra}",
            row.level, row.locator, row.start, row.end
        )
        .expect("write");
    }
    rendered
}

fn bytes_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(out, "{b:02x}").expect("write");
    }
    out
}

fn hex(bytes: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for b in bytes {
        write!(out, "{b:02x}").expect("write");
    }
    out
}

/// Every sample reads, and every identity it produces is the one recorded beside it.
#[test]
fn every_reader_produces_the_identities_recorded_beside_its_sample() {
    let bless = std::env::var_os("SMYSL_BLESS").is_some();
    let mut failures = Vec::new();
    for sample in built_samples() {
        let (out, _) = read(sample);
        let rendered = describe(sample, &out);
        let expected_path = fixtures().join(format!("{}.expected", sample.file));
        if bless {
            std::fs::write(&expected_path, &rendered)
                .unwrap_or_else(|e| panic!("writing {}.expected: {e}", sample.file));
            continue;
        }
        match std::fs::read_to_string(&expected_path) {
            Ok(expected) if expected == rendered => {}
            Ok(expected) => {
                let line = expected
                    .lines()
                    .zip(rendered.lines())
                    .position(|(a, b)| a != b)
                    .unwrap_or(0);
                failures.push(format!(
                    "{}: differs from its expectation at line {}:\n  expected: {:?}\n  got:      {:?}",
                    sample.file,
                    line + 1,
                    expected.lines().nth(line).unwrap_or("<end>"),
                    rendered.lines().nth(line).unwrap_or("<end>"),
                ));
            }
            Err(e) => failures.push(format!(
                "{}.expected: {e} (run with SMYSL_BLESS=1 to write it)",
                sample.file
            )),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The two OSIS forms of the same chapter name the same part.
///
/// Container and milestone verses are different XML with nothing structurally in common, both
/// are in wide circulation, and a corpus built from one must align with a corpus built from
/// the other. Equality of the tid is what makes that true rather than hoped for.
#[cfg(feature = "reader-osis")]
#[test]
fn the_two_osis_forms_name_the_same_text() {
    let container = read(sample("gen1.osis")).0;
    let milestone = read(sample("gen1-milestone.osis")).0;
    assert_eq!(container.text.as_str(), milestone.text.as_str());
    assert_eq!(container.text.tid(), milestone.text.tid());
    let ids = |out: &ReadOutput| -> Vec<String> {
        out.rows.iter().map(|r| r.locator.to_string()).collect()
    };
    assert_eq!(ids(&container), ids(&milestone));
}

/// Three file formats, one identity.
///
/// `gen1.usfm`, `gen1.osis` and `gen1-milestone.osis` are the same five verses of the same
/// edition written three ways — backslash markers, container XML, milestone XML — and all
/// three name the same part. That is what the normalisation and the identity rules are *for*:
/// a library that received this chapter from three publishers holds it once.
///
/// It is also the sharpest test of the readers' spacing. Every one of them has to join verses
/// with exactly one space, keep the apparatus out, and leave the punctuation alone; a single
/// byte of disagreement anywhere and these three tids are three tids.
#[cfg(all(feature = "reader-usfm", feature = "reader-osis"))]
#[test]
fn three_formats_of_one_chapter_name_one_part() {
    let usfm = read(sample("gen1.usfm")).0;
    let container = read(sample("gen1.osis")).0;
    let milestone = read(sample("gen1-milestone.osis")).0;
    assert_eq!(usfm.text.as_str(), container.text.as_str());
    assert_eq!(usfm.text.tid(), container.text.tid());
    assert_eq!(usfm.text.tid(), milestone.text.tid());
    // The tables are not identical — USFM states a book and a chapter, the milestone form
    // states them as points — but every verse is addressed the same way in all three.
    let verses = |out: &ReadOutput| -> Vec<String> {
        out.rows
            .iter()
            .filter(|r| r.level.as_str() == "verse")
            .map(|r| format!("{} {}..{}", r.locator, r.start, r.end))
            .collect()
    };
    assert_eq!(verses(&usfm), verses(&container));
    assert_eq!(verses(&usfm), verses(&milestone));
}

/// Reading the same bytes twice gives the same everything.
///
/// The property a reader exists to have, and the cheapest one to lose: an iteration over a
/// hash map, a timestamp, a locale-dependent comparison. None of those is in this crate by
/// construction, and this is the test that would notice if one arrived.
#[test]
fn reading_is_reproducible() {
    for sample in built_samples() {
        let first = read(sample).0;
        let second = read(sample).0;
        assert_eq!(first.text.tid(), second.text.tid(), "{}", sample.file);
        assert_eq!(first.rows, second.rows, "{}", sample.file);
    }
}

/// Every locator a reader emitted resolves to that node's range.
///
/// §5.1's structure row, over real samples rather than over a hand-built table: a locator the
/// reader emitted and the structure cannot resolve is an address in a corpus that points at
/// nothing.
#[test]
fn every_locator_emitted_resolves_to_its_own_node() {
    for sample in built_samples() {
        let (out, _) = read(sample);
        let mut budget = Budget::new(Caps::DEFAULT, out.text.len() as u64).expect("a budget");
        let structure = Structure::build(&out.rows, out.text.len() as u64, &mut budget)
            .unwrap_or_else(|e| panic!("{}: {e}", sample.file));
        for row in &out.rows {
            assert_eq!(
                structure.resolve(&row.locator),
                Some(row.range()),
                "{}: {}",
                sample.file,
                row.locator
            );
        }
    }
}

/// No node's range begins or ends inside the spacing that joins it to its neighbours.
///
/// The defect this catches was real, in `usfm/1`, before the row placement moved into one
/// shared builder: a row that takes its start at the marker which opened it carries one byte
/// of its predecessor's separator. Asserted over every sample from every reader, because the
/// three structured readers now share the code that got it wrong once.
#[test]
fn no_node_range_holds_its_separator() {
    for sample in built_samples() {
        let (out, _) = read(sample);
        let text = out.text.as_str();
        for row in &out.rows {
            let slice = &text[row.start as usize..row.end as usize];
            if slice.is_empty() {
                continue;
            }
            assert!(
                !slice.starts_with(char::is_whitespace) && !slice.ends_with(char::is_whitespace),
                "{}: {} holds spacing: {slice:?}",
                sample.file,
                row.locator
            );
        }
    }
}
