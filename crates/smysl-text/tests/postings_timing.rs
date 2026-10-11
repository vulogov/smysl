//! What `find` costs with the postings table and without it (TX-P4 step 2's exit clause).
//!
//! # Why these are `#[ignore]`d
//!
//! A timing assertion in CI is a flake generator: the figures depend on the machine, the
//! profile and what else is running. What the suite pins is the *behaviour* — that the
//! persistent table and the in-memory one return the same hits, in `tests/index.rs` and in
//! `smysl-retrieve/tests/postings.rs` — and what is here is the harness that produced the
//! numbers in the CHANGELOG and in RFC SMYSL-2.4 §6. Rerun it with:
//!
//! ```text
//! cargo test -p smysl-text --release --test postings_timing -- --ignored --nocapture
//! ```
//!
//! # What the exit clause asks, and what can be measured against it
//!
//! "`find` p95 < 200 ms at 155k units (13 s today, F-10) on the Appendix E corpus" names a
//! corpus that is a **store** — `bench-en-155510.smy`, generated for SMYSL-2.8 §2.8 — and the
//! table this step builds is a **library** table (§3.8), because a partition is a manifest and a
//! language and a bare store has neither. The two are not the same corpus, and no amount of
//! care makes a figure from one a figure about the other. So what is measured here is a library
//! of the same size, built from synthetic units, and the two quantities the clause is actually
//! comparing:
//!
//! - what a query costs when the corpus has already been analysed (the postings path), and
//! - what a query costs when it has not (the in-memory path, which is what F-10 timed).
//!
//! The synthetic units are the same shape as `smysl-graph/tests/append_timing.rs` uses and for
//! the same reason: what is being timed is the cost model, and a cost model does not care which
//! words are in the gists — only how many there are and how they repeat.

use std::time::Instant;

use smysl_core::ids::{LangTag, Rdid, Tid};
use smysl_core::types::{Carry, Manifest, PartEntry, Record, SourceKind, SourceRef};
use smysl_core::{KernelType, Status, UnitCoreBuilder};
use smysl_retrieve::{Bm25, MemPostings, Postings as _, Query, Retriever as _};
use smysl_text::analyze::Chain;
use smysl_text::library::Library;

/// How many units the exit clause names.
const UNITS: usize = 155_510;

/// The first unit's instant; each later one is an hour after the one before.
const OBSERVED_FROM: i64 = 1_700_000_000_000;

/// How many manifests the corpus is divided between, and therefore how many partitions.
///
/// Not one: `postings::View` answers `df` by asking every partition, so the partition count is
/// a term in the query cost and a harness with one partition would hide it. A Bible cut at the
/// measured 1 KiB minimum is thousands of parts under one manifest, and a library of five is
/// five manifests — so the partition count follows the *texts*, not the parts.
const TEXTS: usize = 5;

/// The queries the p95 is over.
///
/// Twenty, of one to four terms, with terms of every frequency in the corpus: a p95 over one
/// query is a timing, not a distribution. The common terms are the expensive ones — their
/// postings lists are the long ones — so a set of only rare terms would report a number no
/// caller will see.
const QUERIES: &[&str] = &[
    "claim",
    "synthetic",
    "the",
    "unit",
    "number 400",
    "synthetic claim",
    "the unit",
    "claim number 41000",
    "harness",
    "timing harness",
    "synthetic claim number",
    "the synthetic unit",
    "number",
    "unit 155000",
    "claim 1",
    "synthetic timing claim unit",
    "the the the",
    "99999",
    "nothing matches this at all",
    "claim harness number timing",
];

/// A gist of about twenty-five words, which is draft 3 Appendix E's shape.
///
/// The length matters to the comparison and not to the table: analysing a corpus is linear in
/// its words, so a harness with ten-word gists would understate what a query that analyses the
/// corpus costs — the figure F-10 measured — while measuring the same lookup. Appendix E's
/// `@prose` units are twenty-five words, so these are.
fn gist(i: usize) -> String {
    format!(
        "synthetic claim number {i} for the timing harness, written at about the length a \
         prose unit of the benchmark corpus is written at, so that analysing it costs what \
         analysing one of those costs"
    )
}

fn manifest(i: usize, tid: Tid) -> Manifest {
    let mut m = Manifest::new(
        format!("text{i}"),
        LangTag::new("en").expect("a tag"),
        "txt/1",
        "public-domain",
        "smysl/parts/1 level=line min=65536 max=4194304",
    );
    m.carry = Carry::Text;
    m.parts.push(PartEntry::new(
        tid,
        1024,
        [0u8; 32],
        Rdid::from_bytes([0u8; 32]),
    ));
    m
}

/// A library of `UNITS` units, divided between `TEXTS` manifests.
///
/// The manifests are hand-built rather than added through a reader: what is being timed is the
/// postings table, and a reader would spend the harness's time cutting parts and writing objects
/// to produce the one field the table reads — the part a unit's source names.
fn corpus(dir: &std::path::Path) -> Library {
    let mut lib = Library::create(dir).expect("created");
    let tids: Vec<Tid> = (0..TEXTS)
        .map(|t| Tid::from_normalised_bytes(&(t as u64).to_le_bytes()))
        .collect();

    let mut records: Vec<Record> = tids
        .iter()
        .enumerate()
        .map(|(i, tid)| Record::Manifest(manifest(i, *tid)))
        .collect();
    records.reserve(UNITS);
    for i in 0..UNITS {
        let tid = tids[i % TEXTS];
        // An instant an hour after the one before, which is what gives step 3's three range
        // tables something to hold: a unit's own `observed` is the `said` axis's seed, and a
        // corpus of undated units leaves all three empty and measures nothing.
        let mut source = SourceRef::new(SourceKind::Doc, format!("{}#L{i}", tid.canonical()));
        source.observed = Some((OBSERVED_FROM + (i as i64) * 3_600_000) as u64);
        let core = UnitCoreBuilder::new(KernelType::Claim, gist(i), Status::Speculative)
            .source(source)
            .build()
            .expect("a unit");
        records.push(Record::Unit(core));
    }
    // One call, because `append` rebuilds derived state per *call* — the cost model
    // `smysl-graph/tests/append_timing.rs` measured. A loop here would time that instead.
    lib.append_records(&records).expect("appended");
    lib
}

/// The p95 of a sorted-by-measurement run, by nearest rank.
fn p95(mut times: Vec<f64>) -> f64 {
    times.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let at = ((times.len() as f64) * 0.95).ceil() as usize - 1;
    times[at.min(times.len() - 1)]
}

#[test]
#[ignore]
fn find_over_persistent_postings_against_analysing_the_corpus() {
    let dir = std::env::temp_dir().join(format!("smysl-postings-timing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory");

    let start = Instant::now();
    let mut lib = corpus(&dir);
    println!(
        "corpus      {UNITS} units in {TEXTS} texts, built in {:?}",
        start.elapsed()
    );

    // The one-off: analysing every unit once. This is the cost `find` pays per invocation today
    // and the cost the table exists to pay once.
    let start = Instant::now();
    lib.reindex();
    let rebuild = start.elapsed();
    let start = Instant::now();
    lib.write_index().expect("wrote the sidecar");
    let write = start.elapsed();
    let bytes = std::fs::metadata(smysl_text::index::Index::path(&dir))
        .map(|m| m.len())
        .unwrap_or_default();
    println!("rebuild     every table, {rebuild:?}");
    println!("sidecar     {bytes} bytes written in {write:?}");

    // What `find --library` pays before it can score anything, split into the two halves it is
    // made of. The catalog parse is a floor this phase cannot move — SMYSL-2.8's M-7 is that the
    // sidecar never short-circuits a load — and the second number is what carrying the postings
    // in that sidecar adds to it.
    let moved = dir.join("index-moved");
    std::fs::rename(smysl_text::index::Index::path(&dir), &moved).expect("moved the sidecar");
    let start = Instant::now();
    let bare = Library::open(&dir).expect("reopened without a sidecar");
    let catalog_only = start.elapsed();
    assert!(
        bare.index().postings().is_none(),
        "an open with no sidecar built the postings table, which is the cost it must not pay"
    );
    drop(bare);
    std::fs::rename(&moved, smysl_text::index::Index::path(&dir)).expect("put it back");

    let start = Instant::now();
    let reopened = Library::open(&dir).expect("reopened");
    let open = start.elapsed();
    println!("open        {catalog_only:?} with no sidecar, so building ON_OPEN's tables");
    let view = reopened
        .index()
        .postings()
        .expect("the sidecar carried the postings table");
    println!(
        "open        {open:?}, {} documents in {} partition(s)",
        view.docs(),
        view.partitions()
    );
    assert_eq!(view.docs() as usize, UNITS, "the table lost documents");

    let chain = Chain::new(&LangTag::new("en").expect("a tag"));
    let scored: Vec<f64> = QUERIES
        .iter()
        .map(|text| {
            let q = Query::new(*text, 10);
            let start = Instant::now();
            let hits = Bm25::over(&view, &chain, reopened.store()).search(&q);
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            println!("  {ms:>9.3} ms  {:>3} hits  {text}", hits.len());
            ms
        })
        .collect();
    let persistent = p95(scored);

    // The path F-10 timed: analyse the corpus, then score it. One query, because twenty of
    // these is twenty times the analysis and the number is the same every time.
    let start = Instant::now();
    let memory = MemPostings::index(reopened.store(), &chain);
    let analysis = start.elapsed();
    let start = Instant::now();
    let hits = Bm25::over(&memory, &chain, reopened.store()).search(&Query::new("the unit", 10));
    let in_memory = start.elapsed().as_secs_f64() * 1000.0 + analysis.as_secs_f64() * 1000.0;
    println!("analysis    {analysis:?} for {} documents", memory.docs());
    println!(
        "  {in_memory:>9.3} ms  {:>3} hits  the unit (analysed per query)",
        hits.len()
    );

    println!();
    println!(
        "open, no sidecar       {:.3} ms  (catalog parse plus ON_OPEN's tables)",
        catalog_only.as_secs_f64() * 1000.0
    );
    println!(
        "open, sidecar current  {:.3} ms  (catalog parse plus {bytes} bytes decoded)",
        open.as_secs_f64() * 1000.0
    );
    println!("p95 over postings      {persistent:.3} ms  (exit clause: < 200 ms)");
    println!(
        "command, end to end    {:.3} ms",
        open.as_secs_f64() * 1000.0 + persistent
    );
    println!("one query, analysed    {in_memory:.3} ms");
    println!(
        "ratio                  {:.1}x",
        in_memory / persistent.max(f64::MIN_POSITIVE)
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// A `said during` range over a 155k-unit library, end to end (TX-P4 step 3's exit clause).
///
/// `tests/ranges.rs` asserts the claim on synthesised rows, which is the right instrument for
/// the structure. This is the same claim through the library: a corpus of units with recorded
/// instants, the table built by `reindex` and carried in the sidecar, and a window asked of what
/// came back off the disk.
#[test]
#[ignore]
fn a_said_range_over_a_reopened_library() {
    let dir = std::env::temp_dir().join(format!("smysl-said-range-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory");

    let mut lib = corpus(&dir);
    lib.reindex();
    lib.write_index().expect("wrote the sidecar");
    drop(lib);

    let start = Instant::now();
    let reopened = Library::open(&dir).expect("reopened");
    println!("open        {:?}", start.elapsed());
    let said = &reopened.index().said;
    println!(
        "said        {} row(s), danglers {}",
        said.len(),
        said.danglers()
    );
    assert_eq!(
        said.len(),
        UNITS,
        "every unit has an observed instant and so a said interval"
    );

    // A day in the middle of the corpus, which the generator spaces an hour apart.
    let lo = OBSERVED_FROM + (UNITS as i64 / 2) * 3_600_000;
    let window = smysl_text::time::Interval::of(lo, lo + 86_400_000);
    let start = Instant::now();
    let (answer, visited) = said.instrumented(window);
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    println!(
        "said during {ms:.4} ms, {} answer(s), {visited} of {UNITS} rows read",
        answer.len()
    );
    assert!(!answer.is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}

/// What each table costs to build, at the size the exit clause names.
///
/// Step 1 split the tables into "arithmetic over a log the open has already parsed" and "one
/// object read per part", and put five tables in the first class on the strength of that
/// reasoning. The reasoning was wrong about one of them, and this is the measurement that says
/// which: `journal` is `O(units)` and parses a canonical identity out of every unit's source
/// reference, so it belongs with `postings` and not with the four tables that are `O(manifests)`.
#[test]
#[ignore]
fn what_each_table_costs_to_build() {
    use smysl_core::types::library::Axis;
    use smysl_text::index::{journal, postings, ranges, tables};

    let dir = std::env::temp_dir().join(format!("smysl-table-cost-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory");
    let lib = corpus(&dir);
    println!("{UNITS} units, {TEXTS} manifests");

    macro_rules! timed {
        ($name:literal, $what:expr) => {{
            let start = Instant::now();
            let built = $what;
            println!(
                "  {:<11} {:>9.3} ms",
                $name,
                start.elapsed().as_secs_f64() * 1000.0
            );
            built
        }};
    }

    let manifests = timed!("manifests", tables::manifests_rebuild(&lib));
    let _parts = timed!("parts", tables::parts_rebuild(&lib));
    let by_part = timed!("by-part", tables::by_part_rebuild(&lib));
    let _redactions = timed!("redactions", tables::redactions_rebuild(&lib));
    let _journal = timed!("journal", journal::rebuild(&lib));
    let _said = timed!("said", ranges::axis_rebuild(&lib, Axis::Said));
    let _composed = timed!("composed", ranges::axis_rebuild(&lib, Axis::Composed));
    let _observed = timed!("observed", ranges::observed_rebuild(&lib));
    let _segments = timed!("segments", tables::segments_rebuild(&lib));
    let _locators = timed!("locators", tables::locators_rebuild(&lib));
    let _postings = timed!("postings", postings::rebuild(&lib, &manifests, &by_part));

    let _ = std::fs::remove_dir_all(&dir);
}

/// What one axis of effective time costs to compute, at the size step 3's clause names.
///
/// The `said` and `composed` tables are the rule E engine's answer written down, and the engine
/// is a stratified shortest-paths solve over every subject in the store — so the question that
/// decides whether those tables can be maintained at all is what one solve costs. A table whose
/// incremental builder is "recompute" is only usable if recomputing is affordable on the paths
/// that write.
#[test]
#[ignore]
fn what_one_axis_of_effective_time_costs() {
    use smysl_core::types::library::Axis;

    let dir = std::env::temp_dir().join(format!("smysl-axis-cost-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory");
    let lib = corpus(&dir);

    for axis in Axis::ALL {
        let start = Instant::now();
        let eff = smysl_text::time::engine::effective(lib.store(), *axis);
        println!(
            "  {:<9} {:>9.3} ms, {} dated subject(s)",
            axis.as_str(),
            start.elapsed().as_secs_f64() * 1000.0,
            eff.dated.len()
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
