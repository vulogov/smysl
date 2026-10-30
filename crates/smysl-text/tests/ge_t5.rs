//! **GE-T5**, within-language arm — is the `lexical` engine at the ceiling the coders are at?
//!
//! SMYSL-2.4 §6 TX-P7 step 4, which is the phase exit. The clause is **restated** (§1.1 item 5,
//! argued in §0.1): draft 1 asked for *precision ≥ 0.9 at recall 0.7*, and S0 measured the gold
//! those figures are scored against at a binary α of 0.600, CI [0.550, 0.646], over three model
//! coders. A precision of 0.9 is above what a gold that unreliable can support, so the pilot's
//! 0.835 measured the gold and not the engine. §5.5 asks instead whether an engine's agreement
//! with the pooled gold, **scored on the coders' own scale**, is distinguishable from the
//! coders' agreement with each other — whether the engine joins the pool rather than beats it.
//!
//! So this file holds two measurements and reports both, because they answer different
//! questions and one of them is owed.
//!
//! # The first: draft 1's statistic, over material of record
//!
//! `fixtures/proposition/ge-t5/paraphrase.tsv` is a second English expression of Ruth, written
//! by hand: forty rows that restate one King James verse each in modern English, and ten placed
//! at a locator that already carries one of those forty, close to it in vocabulary and saying
//! something else. The engine's job is to pair a verse with its restatement and not with its
//! neighbour, and that is precision and recall against a gold which is **of record by
//! construction** — a row's `relation` is what its author wrote it to be, and nothing judged it
//! afterwards.
//!
//! That hand is the figure's limit, and it is the same limit GE-T11's planted set has: the
//! difficulty is the difficulty one person chose. `SMYSL_GE_T5_PARAPHRASE=<file>` replaces the
//! expression with somebody else's.
//!
//! # The second: §5.5's restated clause, which needs coders
//!
//! α is defined over a *reliability matrix* — units down, coders across — and is corrected for
//! chance, which is what lets a two-coder figure and a three-coder figure be compared at all.
//! The restated clause is then three numbers:
//!
//! 1. `α(coders)` — the three coders against each other.
//! 2. `α(gold, engine)` — the pooled gold, by majority, against the engine scored as one more
//!    coder: `same` where it proposed the pair, `not` where it did not.
//! 3. a **paired bootstrap** on `δ = α(coders) − α(gold, engine)`, resampling the pairs and
//!    recomputing both α on each resample, so the two figures move together and the interval is
//!    about their difference rather than about either one. The clause is met when the 95%
//!    interval for δ contains zero.
//!
//! A paired bootstrap rather than two intervals and an eye: two intervals can overlap while the
//! difference is reliably positive, and can fail to overlap while it is not, because the two
//! statistics are computed over the same pairs and are correlated.
//!
//! **What is owed is the sheet.** Three coders judging the same pairs independently is material,
//! not code, and this repository does not hold any — S0's three model coders produced one and it
//! is not here. `fixtures/proposition/ge-t5/sheet.tsv` is three *scripted* columns, written in
//! one pass by one hand with the disagreements placed on purpose; it exists so the instrument
//! runs on every `make test` and fails when it breaks. An α over it is a fact about that file.
//! The harness therefore reports [`Decision::Owed`] for the committed sheet and decides the
//! clause only for a sheet somebody else supplies:
//!
//! ```text
//! SMYSL_GE_T5_SHEET=<file.tsv>        three or more coders over the same pairs
//! SMYSL_GE_T5_PARAPHRASE=<file.tsv>   a within-language expression somebody else wrote
//! ```
//!
//! Both measurements are **pure**. Nothing here calls a model: the engine is a BM25 index and
//! an adjacency rule, the gold is a committed file, and α is arithmetic. A measurement that
//! costs model turns is a measurement nobody runs.

#![cfg(feature = "reader-usfm")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smysl_core::ids::{LangTag, Uid};
use smysl_core::types::Carry;
use smysl_core::{canonical_uid, KernelType, Record, Status, UnitCore, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::proposition::{Engine, Lexical};
use smysl_text::readers::Input;
use smysl_text::{locator, provenance};

/// Draft 1's recall, at which its precision was read. The precision bar it came with is gone
/// (§0.1); the recall is not, because it is the point on the curve the figure is read at.
const RECALL_AT: f64 = 0.7;

/// Resamples in the paired bootstrap. 2,000 is enough for a 95% percentile interval on a
/// difference of this size and cheap enough to run on every `make test`.
const RESAMPLES: usize = 2_000;

/// One row of `paraphrase.tsv`: a planted unit, and whether it is the verse's claim.
struct Row {
    id: String,
    lang: String,
    at: String,
    same: bool,
    gist: String,
}

/// What the restated clause came to.
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// δ's interval contains zero: the engine is not distinguishable from the coders.
    Met,
    /// δ's interval is wholly above zero: the coders agree with each other more than the
    /// engine agrees with them.
    Below,
    /// There is no sheet of coders, so there is nothing to be at the ceiling of.
    Owed,
}

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The within-language expression: the committed one, or `SMYSL_GE_T5_PARAPHRASE`.
fn rows() -> Vec<Row> {
    let path = std::env::var_os("SMYSL_GE_T5_PARAPHRASE")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo("fixtures/proposition/ge-t5/paraphrase.tsv"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = Vec::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f[0] == "id" {
            continue;
        }
        assert_eq!(
            f.len(),
            5,
            "a paraphrase row is `id lang at relation gist`: {line}"
        );
        let same = match f[3] {
            "same" => true,
            "not" => false,
            other => panic!("`{other}` is neither `same` nor `not`: {line}"),
        };
        out.push(Row {
            id: f[0].to_string(),
            lang: f[1].to_string(),
            at: f[2].to_string(),
            same,
            gist: f[4].to_string(),
        });
    }
    out
}

/// The sheet: `(pair, coder) -> verdict`, from the committed file or `SMYSL_GE_T5_SHEET`.
///
/// The second return says whether a real sheet was supplied, which is the whole difference
/// between reporting a figure and deciding a clause.
fn sheet() -> (BTreeMap<(String, String), String>, bool) {
    let supplied = std::env::var_os("SMYSL_GE_T5_SHEET");
    let path = supplied
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(|| repo("fixtures/proposition/ge-t5/sheet.tsv"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out = BTreeMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f[0] == "pair" {
            continue;
        }
        assert_eq!(f.len(), 3, "a sheet row is `pair coder verdict`: {line}");
        out.insert((f[0].to_string(), f[1].to_string()), f[2].to_string());
    }
    (out, supplied.is_some())
}

/// A planted unit: a gist, a language, and the locator it claims.
///
/// The restatements are not drawn from a part — no part says them, which is what planting is —
/// so they carry `speculative` and a reference naming the locator alone. What the engine reads
/// of a unit is its gist, its language and its place, and all three are here. The tid is a
/// canonical constant and therefore not the King James part's, which is the point: the two
/// halves of a true pair are at one locator in two expressions, and `identical-span` cannot see
/// them because no bytes are shared.
fn planted_unit(gist: &str, lang: &str, at: &str) -> UnitCore {
    UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
        .lang(LangTag::new(lang).expect("a language tag"))
        .source(smysl_core::types::SourceRef::new(
            smysl_core::types::SourceKind::Doc,
            format!("t3:{}#{at}", "p".repeat(52)),
        ))
        .build()
        .expect("a unit")
}

/// The King James Ruth, one unit per verse, with the locator each was drawn at.
///
/// Four chapters and not two: the planted rows cover Ruth 1 and 2, and chapters 3 and 4 are
/// there so that a pair has somewhere to be lost. Real material, read by `usfm/1` and cited
/// where the reader put it.
fn kjv_ruth() -> Vec<(String, UnitCore)> {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "smysl-ge-t5-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch directory");
    let mut lib = Library::create(&root).expect("a library");

    let path = repo("fixtures/library/bibles/slice/eng-kjv/08-RUT.usfm");
    let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let alias = "kjv/08-rut";
    let mut spec = AddSpec::new("usfm/1", alias, "public-domain")
        .with_lang(LangTag::new("en").expect("a tag"))
        .with_carry(Carry::Text);
    spec.versification = Some("kjv".to_string());
    let added = lib
        .add(&Input::new(&raw), &spec, &Caps::DEFAULT)
        .expect("the book is added");
    let manifest = lib
        .store()
        .manifest(&added.mid)
        .expect("its manifest")
        .clone();

    let mut out = Vec::new();
    for tid in &added.parts {
        let reading = lib
            .part_reading(&added.mid, tid)
            .expect("the part's reading");
        for row in &reading.rows {
            if row.level.as_str() != "verse" {
                continue;
            }
            let at = row.locator.to_string();
            let Ok(passage) = lib.passage(
                alias,
                &locator::parse(&at).expect("the reader's own locator"),
                &Caps::DEFAULT,
            ) else {
                continue;
            };
            let gist = passage.text.trim();
            if gist.is_empty() {
                continue;
            }
            let b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Cited);
            let lang = manifest.lang.clone();
            let unit = provenance::stamp(
                b,
                &provenance::Drawn {
                    tid: passage.tid,
                    locator: Some(&at),
                    manifest: &manifest,
                    observed: None,
                    span: Some((passage.range.start, passage.range.end)),
                    lang: Some(&lang),
                },
            )
            .build()
            .expect("a unit of a verse");
            out.push((at, unit));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    out
}

/// Krippendorff's α for nominal data, over a reliability matrix: a row per unit, a column per
/// coder, `None` where a coder did not judge that unit.
///
/// The coincidence-matrix form. A unit with `m` pairable values contributes each of its `m(m−1)`
/// ordered pairs with weight `1/(m−1)`, so a unit judged by three coders does not count for
/// three times as much as one judged by two; a unit with fewer than two values contributes
/// nothing, which is what "pairable" means and why a missing verdict is missing rather than a
/// disagreement.
///
/// `None` when nothing is pairable, or when every value is the same — the second case is the
/// one worth naming: with one value in the whole matrix the expected disagreement is zero, α is
/// `0/0`, and a function that returned 1.0 there would report perfect reliability for a sheet
/// that says nothing. The caller has to decide what that means.
fn alpha(matrix: &[Vec<Option<&str>>]) -> Option<f64> {
    let mut labels: BTreeSet<&str> = BTreeSet::new();
    for unit in matrix {
        for v in unit.iter().flatten() {
            labels.insert(v);
        }
    }
    let index: BTreeMap<&str, usize> = labels.iter().enumerate().map(|(i, v)| (*v, i)).collect();
    let k = labels.len();
    if k == 0 {
        return None;
    }
    let mut o = vec![0.0f64; k * k];
    for unit in matrix {
        let present: Vec<usize> = unit.iter().flatten().map(|v| index[*v]).collect();
        let m = present.len();
        if m < 2 {
            continue;
        }
        let w = 1.0 / (m as f64 - 1.0);
        for (i, a) in present.iter().enumerate() {
            for (j, b) in present.iter().enumerate() {
                if i != j {
                    o[a * k + b] += w;
                }
            }
        }
    }
    let n: f64 = o.iter().sum();
    if n < 2.0 {
        return None;
    }
    let marginal: Vec<f64> = (0..k).map(|c| (0..k).map(|x| o[c * k + x]).sum()).collect();
    let mut observed = 0.0;
    let mut expected = 0.0;
    for c in 0..k {
        for x in 0..k {
            if c == x {
                continue;
            }
            observed += o[c * k + x];
            expected += marginal[c] * marginal[x];
        }
    }
    expected /= n - 1.0;
    if expected == 0.0 {
        return None;
    }
    Some(1.0 - observed / expected)
}

/// The pooled gold: the majority verdict per pair, and the pairs that tied.
///
/// A tie is **dropped and counted**, not broken. Breaking it would invent a gold where the
/// coders produced none, and reporting the count is how a reader sees how much of the sheet
/// that was.
fn pooled(
    pairs: &[String],
    sheet: &BTreeMap<(String, String), String>,
    coders: &[String],
) -> (BTreeMap<String, String>, usize) {
    let mut gold = BTreeMap::new();
    let mut tied = 0;
    for p in pairs {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for c in coders {
            if let Some(v) = sheet.get(&(p.clone(), c.clone())) {
                *counts.entry(v.as_str()).or_default() += 1;
            }
        }
        let best = counts.values().copied().max().unwrap_or(0);
        if best == 0 {
            continue;
        }
        let winners: Vec<&str> = counts
            .iter()
            .filter(|(_, n)| **n == best)
            .map(|(v, _)| *v)
            .collect();
        if winners.len() > 1 {
            tied += 1;
            continue;
        }
        gold.insert(p.clone(), winners[0].to_string());
    }
    (gold, tied)
}

/// The restated clause, read off the interval.
///
/// A function rather than three lines inside the measurement, because two of its three answers
/// cannot be reached by the committed material: `Met` and `Below` need a sheet somebody else
/// wrote, and a branch no test enters is a branch that is wrong when it is finally taken.
///
/// **One-sided, and that is the clause and not a convenience.** §5.5 asks whether the engine
/// *joins* the pool: `δ = α(coders) − α(gold, engine)` wholly above zero says the coders agree
/// with each other more than the engine agrees with them, which is the failure. An interval
/// wholly *below* zero says the engine agrees with the pooled gold better than three coders
/// agree with each other — surprising, worth reading twice, and not a failure of anything
/// GE-T5 asks about. So the upper end of the interval is printed and not read: only `lo > 0`
/// is `Below`.
fn decide(lo: f64, supplied: bool) -> Decision {
    if !supplied {
        Decision::Owed
    } else if lo > 0.0 {
        Decision::Below
    } else {
        Decision::Met
    }
}

/// SplitMix64, so the bootstrap is a measurement and not a mood.
struct Seeded(u64);

impl Seeded {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// The 95% percentile interval for `α(coders) − α(gold, engine)`, resampling pairs.
///
/// Both α are recomputed on the *same* resample, which is what makes the interval about the
/// difference. A resample whose α is undefined — every value identical, which happens when the
/// draw lands on one kind of pair — is skipped and counted, because the alternative is to
/// substitute a number for it.
fn bootstrap(
    coder_rows: &[Vec<Option<&str>>],
    engine_rows: &[Vec<Option<&str>>],
    seed: u64,
) -> (f64, f64, usize) {
    assert_eq!(coder_rows.len(), engine_rows.len());
    let n = coder_rows.len();
    let mut rng = Seeded(seed);
    let mut deltas = Vec::with_capacity(RESAMPLES);
    let mut skipped = 0;
    for _ in 0..RESAMPLES {
        let pick: Vec<usize> = (0..n).map(|_| rng.below(n)).collect();
        let c: Vec<Vec<Option<&str>>> = pick.iter().map(|i| coder_rows[*i].clone()).collect();
        let e: Vec<Vec<Option<&str>>> = pick.iter().map(|i| engine_rows[*i].clone()).collect();
        match (alpha(&c), alpha(&e)) {
            (Some(a), Some(b)) => deltas.push(a - b),
            _ => skipped += 1,
        }
    }
    assert!(!deltas.is_empty(), "every resample was degenerate");
    deltas.sort_by(|x, y| x.partial_cmp(y).expect("α is finite"));
    let at = |q: f64| -> f64 {
        let i = ((deltas.len() as f64 - 1.0) * q).round() as usize;
        deltas[i]
    };
    (at(0.025), at(0.975), skipped)
}

/// The corpus both measurements are made over: the King James Ruth plus the planted expression.
fn corpus() -> (Store, BTreeMap<String, Uid>, BTreeMap<String, Uid>) {
    let kjv = kjv_ruth();
    let plants = rows();
    let mut by_locator: BTreeMap<String, Uid> = BTreeMap::new();
    let mut by_id: BTreeMap<String, Uid> = BTreeMap::new();
    let mut records: Vec<Record> = Vec::new();
    for (at, unit) in &kjv {
        by_locator.insert(at.clone(), canonical_uid(unit));
        records.push(Record::Unit(unit.clone()));
    }
    for row in &plants {
        let unit = planted_unit(&row.gist, &row.lang, &row.at);
        let uid = canonical_uid(&unit);
        assert!(
            by_id.insert(row.id.clone(), uid).is_none(),
            "`{}` is planted twice",
            row.id
        );
        records.push(Record::Unit(unit));
    }
    (Store::from_records(records), by_locator, by_id)
}

/// What the engine proposed, as unordered pairs.
fn proposed(store: &Store, k: usize, adjacent_only: bool) -> BTreeSet<(Uid, Uid)> {
    Lexical { k, adjacent_only }
        .propose(store, None)
        .into_iter()
        .map(|p| (p.a, p.b))
        .collect()
}

/// The pair a row names: the planted unit, and the King James verse at the same locator.
fn pair_of(
    row: &Row,
    by_locator: &BTreeMap<String, Uid>,
    by_id: &BTreeMap<String, Uid>,
) -> (Uid, Uid) {
    let a = by_id[&row.id];
    let b = by_locator.get(&row.at).unwrap_or_else(|| {
        panic!(
            "`{}` is planted at {}, which Ruth has no verse at",
            row.id, row.at
        )
    });
    unordered(a, *b)
}

fn unordered(a: Uid, b: Uid) -> (Uid, Uid) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// The verse next to a locator: one further on, or one back where the chapter ends there.
///
/// A rule and not a choice. The neighbour pairs below are the pool's negatives, and a measurer
/// who picked them by hand would be picking how hard the pool is.
fn neighbour(at: &str, by_locator: &BTreeMap<String, Uid>) -> Option<(String, Uid)> {
    let (stem, last) = at.rsplit_once('.')?;
    let n: u64 = last.parse().ok()?;
    for cand in [format!("{stem}.{}", n + 1), format!("{stem}.{}", n - 1)] {
        if let Some(uid) = by_locator.get(&cand) {
            return Some((cand, *uid));
        }
    }
    None
}

/// The pool the restated clause is decided over: every planted row against the verse at its own
/// locator, and against the verse next to it.
///
/// Both halves are defined by a rule, so the pool does not depend on what the engine answered —
/// a pool drawn from the engine's own proposals would be a measurement of how it was drawn. The
/// base rate matters and is reported with the figure: α is a chance-corrected statistic, and a
/// pool that was four-fifths positive would let an engine that proposes everything look
/// agreeable on accuracy while α called it what it is.
///
/// `<id>` is the pair at the row's own locator and `<id>+1` the neighbour pair, which is how
/// `sheet.tsv` names them.
fn pool(
    plants: &[Row],
    by_locator: &BTreeMap<String, Uid>,
    by_id: &BTreeMap<String, Uid>,
) -> Vec<(String, (Uid, Uid), bool)> {
    let mut out = Vec::new();
    for row in plants {
        out.push((row.id.clone(), pair_of(row, by_locator, by_id), row.same));
        if let Some((_, uid)) = neighbour(&row.at, by_locator) {
            out.push((
                format!("{}+1", row.id),
                unordered(by_id[&row.id], uid),
                false,
            ));
        }
    }
    out
}

/// Draft 1's statistic: precision at recall 0.7, over a gold of record.
///
/// Reported and not thresholded, for §0.1's reason — the 0.9 bar is above what a gold of α
/// 0.600 can support, and this gold's reliability is unmeasured because one hand wrote it, so
/// the figure has no bar to be above or below. What it is good for is a regression: an engine
/// that stopped pairing a verse with its own restatement would show here.
#[test]
fn the_engine_pairs_a_verse_with_its_restatement_and_not_with_its_neighbour() {
    let (store, by_locator, by_id) = corpus();
    let plants = rows();
    let gold: BTreeSet<(Uid, Uid)> = plants
        .iter()
        .filter(|r| r.same)
        .map(|r| pair_of(r, &by_locator, &by_id))
        .collect();
    let planted_negatives: BTreeSet<(Uid, Uid)> = plants
        .iter()
        .filter(|r| !r.same)
        .map(|r| pair_of(r, &by_locator, &by_id))
        .collect();
    assert_eq!(gold.len(), 40, "forty restatements");
    assert_eq!(planted_negatives.len(), 10, "ten near misses");

    println!(
        "  corpus: {} units ({} verses of the King James Ruth, {} planted)",
        by_locator.len() + by_id.len(),
        by_locator.len(),
        by_id.len()
    );
    println!("  k  adjacency  proposed  true  precision  recall  planted-fp");
    let mut at_recall: Option<(usize, f64, f64)> = None;
    // The precision of the deepest adjacency-on run, which is as close to "every adjacent pair
    // in the corpus" as the sweep goes.
    let mut liberal = 0.0f64;
    for adjacent_only in [true, false] {
        for k in [1usize, 2, 3, 5, 8, 13, 21] {
            let p = proposed(&store, k, adjacent_only);
            let hits = p.intersection(&gold).count();
            let precision = if p.is_empty() {
                0.0
            } else {
                hits as f64 / p.len() as f64
            };
            let recall = hits as f64 / gold.len() as f64;
            let planted_fp = p.intersection(&planted_negatives).count();
            println!(
                "  {k:<3}{:<11}{:<10}{hits:<6}{precision:<11.4}{recall:<8.4}{planted_fp}",
                if adjacent_only { "on" } else { "off" },
                p.len(),
            );
            if adjacent_only && at_recall.is_none() && recall >= RECALL_AT {
                at_recall = Some((k, precision, recall));
            }
            if adjacent_only {
                liberal = precision;
            }
        }
    }
    match at_recall {
        Some((k, precision, recall)) => println!(
            "  draft 1's statistic: precision {precision:.4} at recall {recall:.4} (k = {k}), \
             reported against no bar (§0.1)"
        ),
        None => println!("  draft 1's statistic: recall never reached {RECALL_AT}"),
    }

    // Two claims, neither of which is GE-T5's. First: draft 1's statistic is defined at all —
    // the engine finds the restatements at the depth the figure is read at. Without this the
    // sweep above could print zeros and nobody would know the harness had stopped measuring.
    let (k, precision, recall) = at_recall.expect("recall reaches 0.7 somewhere in the sweep");
    assert!(
        recall >= RECALL_AT,
        "recall {recall:.4} at k = {k} is below {RECALL_AT}"
    );
    // Second: depth buys precision. The most liberal run in the sweep proposes nearly every
    // adjacent pair in the corpus, and an engine whose read point were no better than that
    // would be an adjacency rule with a BM25 index attached for show.
    assert!(
        precision > liberal * 1.5,
        "precision {precision:.4} at the read point is not meaningfully above the {liberal:.4} \
         of the most liberal run, so the neighbour count is not doing anything"
    );
}

/// §5.5's restated clause, end to end — and what it comes to over the committed sheet.
#[test]
fn the_restated_clause_needs_three_coders_and_says_so_when_it_has_none() {
    let (store, by_locator, by_id) = corpus();
    let plants = rows();
    let (sheet, supplied) = sheet();
    let pool = pool(&plants, &by_locator, &by_id);

    let mut coders: BTreeSet<String> = BTreeSet::new();
    for (_, c) in sheet.keys() {
        coders.insert(c.clone());
    }
    let coders: Vec<String> = coders.into_iter().collect();
    let keys: Vec<String> = pool.iter().map(|(k, _, _)| k.clone()).collect();
    let (gold, tied) = pooled(&keys, &sheet, &coders);
    let positives = pool.iter().filter(|(_, _, t)| *t).count();
    println!(
        "  pool: {} pairs by rule, {positives} the same claim by construction (base rate {:.2})",
        pool.len(),
        positives as f64 / pool.len() as f64
    );
    println!(
        "  sheet: {} coders, {} verdicts, {} pooled, {tied} tied{}",
        coders.len(),
        sheet.len(),
        gold.len(),
        if supplied {
            ""
        } else {
            " (committed, scripted)"
        }
    );
    assert!(
        coders.len() >= 3,
        "a sheet with {} coders cannot say what the coders' agreement is",
        coders.len()
    );
    assert!(
        gold.len() * 4 >= pool.len() * 3,
        "the sheet pools only {} of {} pool pairs, which decides the clause over a quarter of it",
        gold.len(),
        pool.len()
    );

    let default = Lexical::default();
    let judged = proposed(&store, default.k, default.adjacent_only);
    let mut coder_rows: Vec<Vec<Option<&str>>> = Vec::new();
    let mut engine_rows: Vec<Vec<Option<&str>>> = Vec::new();
    let mut agree = 0usize;
    let mut said_same = 0usize;
    for (key, pair, _) in &pool {
        let Some(g) = gold.get(key) else {
            continue;
        };
        let verdict = if judged.contains(pair) {
            said_same += 1;
            "same"
        } else {
            "not"
        };
        coder_rows.push(
            coders
                .iter()
                .map(|c| sheet.get(&(key.clone(), c.clone())).map(|v| v.as_str()))
                .collect(),
        );
        engine_rows.push(vec![Some(g.as_str()), Some(verdict)]);
        if g == verdict {
            agree += 1;
        }
    }

    let a_coders = alpha(&coder_rows).expect("three coders who do not all say one thing");
    let a_engine = alpha(&engine_rows).expect("an engine that does not say one thing");
    let (lo, hi, skipped) = bootstrap(&coder_rows, &engine_rows, 0x5115_1a1e);
    let decision = decide(lo, supplied);
    println!(
        "  engine at k = {}, adjacency {}: `same` on {said_same} of {} pooled pairs",
        default.k,
        if default.adjacent_only { "on" } else { "off" },
        coder_rows.len()
    );
    println!("  α(coders)        {a_coders:.4}");
    println!(
        "  α(gold, engine)  {a_engine:.4}  (agreeing on {agree} of {} pooled pairs)",
        coder_rows.len()
    );
    println!(
        "  δ = α(coders) − α(gold, engine): 95% [{lo:.4}, {hi:.4}] over {RESAMPLES} resamples, \
         {skipped} degenerate"
    );
    println!("  decision: {decision:?}");
    if decision == Decision::Owed {
        println!(
            "  GE-T5's within-language arm is OWED: the sheet above is three scripted columns, \
             so α(coders) is a fact about that file. SMYSL_GE_T5_SHEET=<file> decides the clause."
        );
    }

    match decision {
        Decision::Owed => assert!(!supplied, "a supplied sheet decides the clause"),
        Decision::Met => {}
        Decision::Below => panic!(
            "the engine is distinguishable from the coders: δ 95% [{lo:.4}, {hi:.4}] is \
             wholly above zero"
        ),
    }
}

/// α reproduces arithmetic a reader can check, which is the only validation available here.
///
/// SMYSL-2.6 §5.2 asks its own α to reproduce the published worked example in Krippendorff
/// (2011) and says the value is "to be transcribed; unverified here". Asserting a number from
/// memory would be worse than asserting none, so this test derives one instead, in full:
///
/// Four units, two coders, values `a` and `b`: `(a,a) (a,a) (b,b) (a,b)`. Every unit has two
/// pairable values, so each contributes its two ordered pairs at weight `1/(2−1) = 1`.
/// Coincidences: `o(a,a) = 4`, `o(b,b) = 2`, `o(a,b) = o(b,a) = 1`; `n = 8`; marginals
/// `n(a) = 5`, `n(b) = 3`. Observed disagreement `D_o = 1 + 1 = 2`. Expected
/// `D_e = (5·3 + 3·5)/(8−1) = 30/7`. So `α = 1 − 2/(30/7) = 1 − 14/30 = 0.5333…`.
#[test]
fn alpha_reproduces_an_arithmetic_a_reader_can_follow() {
    let m: Vec<Vec<Option<&str>>> = vec![
        vec![Some("a"), Some("a")],
        vec![Some("a"), Some("a")],
        vec![Some("b"), Some("b")],
        vec![Some("a"), Some("b")],
    ];
    let got = alpha(&m).expect("two values and four pairable units");
    assert!(
        (got - (1.0 - 14.0 / 30.0)).abs() < 1e-12,
        "α = {got}, and the arithmetic above says {}",
        1.0 - 14.0 / 30.0
    );
}

/// The two ends of the scale, and the one place the formula has no answer.
#[test]
fn alpha_is_one_on_perfect_agreement_and_undefined_when_nothing_varies() {
    let perfect: Vec<Vec<Option<&str>>> = vec![
        vec![Some("a"), Some("a"), Some("a")],
        vec![Some("b"), Some("b"), Some("b")],
        vec![Some("a"), Some("a"), Some("a")],
    ];
    assert!((alpha(&perfect).expect("two values") - 1.0).abs() < 1e-12);

    // Perfect agreement on *one* value is not reliability, and α is 0/0 there. The function
    // says `None` rather than 1.0, which is the difference between a sheet that agrees and a
    // sheet that says nothing.
    let flat: Vec<Vec<Option<&str>>> = vec![vec![Some("a"), Some("a")], vec![Some("a"), Some("a")]];
    assert_eq!(alpha(&flat), None);

    // One coder is not agreement either: nothing is pairable.
    let alone: Vec<Vec<Option<&str>>> = vec![vec![Some("a")], vec![Some("b")]];
    assert_eq!(alone.len(), 2);
    assert_eq!(alpha(&alone), None);
}

/// A verdict a coder did not give is missing, not a disagreement.
#[test]
fn a_missing_verdict_drops_out_rather_than_counting_against_the_coders() {
    let two: Vec<Vec<Option<&str>>> = vec![
        vec![Some("a"), Some("a")],
        vec![Some("b"), Some("b")],
        vec![Some("a"), Some("b")],
    ];
    let with_absences: Vec<Vec<Option<&str>>> = vec![
        vec![Some("a"), Some("a"), None],
        vec![Some("b"), Some("b"), None],
        vec![Some("a"), Some("b"), None],
    ];
    assert_eq!(alpha(&two), alpha(&with_absences));

    // And a unit only one coder judged contributes nothing at all, rather than contributing
    // agreement with itself.
    let plus_lonely: Vec<Vec<Option<&str>>> = {
        let mut m = two.clone();
        m.push(vec![Some("a"), None]);
        m
    };
    assert_eq!(alpha(&two), alpha(&plus_lonely));
}

/// The pooled gold drops a tie and counts it.
#[test]
fn the_pooled_gold_drops_a_tie_and_counts_it() {
    let coders: Vec<String> = ["c1", "c2"].iter().map(|s| s.to_string()).collect();
    let mut sheet = BTreeMap::new();
    sheet.insert(("p1".to_string(), "c1".to_string()), "same".to_string());
    sheet.insert(("p1".to_string(), "c2".to_string()), "not".to_string());
    sheet.insert(("p2".to_string(), "c1".to_string()), "same".to_string());
    sheet.insert(("p2".to_string(), "c2".to_string()), "same".to_string());
    let pairs = vec!["p1".to_string(), "p2".to_string(), "p3".to_string()];
    let (gold, tied) = pooled(&pairs, &sheet, &coders);
    assert_eq!(tied, 1, "p1 is one each way");
    assert_eq!(gold.len(), 1, "p2 is pooled and p3 was never judged");
    assert_eq!(gold["p2"], "same");
}

/// The interval is seeded, so two runs of the gate report the same one.
#[test]
fn the_bootstrap_reports_the_same_interval_twice() {
    let coder_rows: Vec<Vec<Option<&str>>> = (0..40)
        .map(|i| {
            if i % 7 == 0 {
                vec![Some("same"), Some("not"), Some("same")]
            } else if i % 3 == 0 {
                vec![Some("not"), Some("not"), Some("not")]
            } else {
                vec![Some("same"), Some("same"), Some("same")]
            }
        })
        .collect();
    let engine_rows: Vec<Vec<Option<&str>>> = (0..40)
        .map(|i| {
            if i % 5 == 0 {
                vec![Some("same"), Some("not")]
            } else if i % 3 == 0 {
                vec![Some("not"), Some("not")]
            } else {
                vec![Some("same"), Some("same")]
            }
        })
        .collect();
    let first = bootstrap(&coder_rows, &engine_rows, 7);
    let again = bootstrap(&coder_rows, &engine_rows, 7);
    assert_eq!(first, again);
    let other = bootstrap(&coder_rows, &engine_rows, 8);
    assert_ne!(
        first, other,
        "two seeds that gave one interval are not resampling"
    );
}

/// All three answers the clause can have, including the two the committed sheet cannot reach.
#[test]
fn the_clause_is_owed_without_a_sheet_met_on_an_interval_that_straddles_zero_and_below_above_it() {
    assert_eq!(decide(-0.13, false), Decision::Owed);
    assert_eq!(decide(-0.13, true), Decision::Met);
    // Zero at the very edge is inside the interval: the clause asks whether the difference is
    // distinguishable from zero, and an interval that reaches it has not distinguished it.
    assert_eq!(decide(0.0, true), Decision::Met);
    assert_eq!(decide(-0.40, true), Decision::Met);
    assert_eq!(decide(0.24, true), Decision::Below);
    // And an engine *above* the coders is not distinguishable in the direction the clause cares
    // about — it joined the pool, which is all §5.5 asks. `Below` is the one-sided answer,
    // because an engine that agrees with the pooled gold better than the coders agree with each
    // other has not failed anything.
    assert_eq!(decide(-0.90, true), Decision::Met);
}
