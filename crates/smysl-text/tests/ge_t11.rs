//! **GE-T11** — does the linker's candidate generator still hold the pair, after the budget?
//!
//! SMYSL-2.4 §6 TX-P7 step 3: *recall ≥ 0.6 on 50 planted far contradictions at the default
//! candidate budget*. The gate is about one failure and not about judging: a corpus holds
//! `n²/2` pairs, a model turn is bought per pair asked about, and
//! [`smysl_text::link::candidates`] is what decides which pairs are bought. If the true pair is
//! not in that set, no judge however good will find the contradiction, and the whole of
//! `smysl link` is answering questions nobody needed asked.
//!
//! So the measurement is pure. Nothing here calls a model, and the figure is a property of the
//! generator and the corpus, which is what makes it reproducible and what makes a regression in
//! it visible on an ordinary `make test`.
//!
//! # What is planted, and who planted it
//!
//! `fixtures/library/link/planted.tsv` holds fifty pairs of claims that cannot both be true,
//! each half cited at a locator in a different book so that no adjacency rule reaches between
//! them — *far*, in GE-T11's sense, and the sense that matters: a text contradicting itself two
//! verses later is a copy-editing mistake, and a text contradicting itself two books later is
//! the finding a reader cannot make by reading forwards.
//!
//! A person wrote them, by hand, and that is the figure's first limit. The file says so at
//! length and so does this file, because a planted set written by the same hand that wrote the
//! generator is a set whose difficulty that hand chose.
//!
//! # The second limit, which is the corpus
//!
//! Recall is only hard when there is something to lose the pair among. The committed corpus is
//! `fixtures/library/bibles/slice/eng-kjv/`, which is Ruth and Jude: a hundred and ten verses.
//! At a budget of eight candidates per unit over a corpus that small, almost every pair is
//! affordable, so a recall measured here says **the instrument works** and says very little
//! about the linker. The discriminating run needs a corpus with thousands of units in one
//! language, which this repository does not hold and will not grow by itself.
//!
//! That run is one environment variable away and takes no code change:
//!
//! ```text
//! SMYSL_GE_T11_CORPUS=<dir of *.smy>      the distractor pool, instead of the slice
//! SMYSL_GE_T11_PLANTED=<file.tsv>         a planted set somebody else wrote
//! SMYSL_GE_T11_OUT=<file>                 where to write the per-pair result
//! ```
//!
//! The harness reports the corpus size with the figure for exactly this reason: a recall
//! without a denominator is not a measurement, and the number of units the pair had to beat is
//! the denominator here.

#![cfg(feature = "reader-usfm")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use smysl_core::ids::{LangTag, Uid};
use smysl_core::types::Carry;
use smysl_core::{canonical_uid, KernelType, Record, Status, UnitCore, UnitCoreBuilder};
use smysl_graph::Store;
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::link::{candidates, Budget, Candidate};
use smysl_text::readers::Input;
use smysl_text::{locator, provenance};

/// GE-T11's bar.
const BAR: f64 = 0.6;

/// One planted contradiction, as a row of the TSV.
struct Plant {
    id: String,
    lang: String,
    at_a: String,
    gist_a: String,
    at_b: String,
    gist_b: String,
}

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// The planted set: the committed one, or whatever `SMYSL_GE_T11_PLANTED` names.
fn plants() -> Vec<Plant> {
    let path = std::env::var_os("SMYSL_GE_T11_PLANTED")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo("fixtures/library/link/planted.tsv"));
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
            6,
            "a planted row is `id lang at_a gist_a at_b gist_b`: {line}"
        );
        out.push(Plant {
            id: f[0].to_string(),
            lang: f[1].to_string(),
            at_a: f[2].to_string(),
            gist_a: f[3].to_string(),
            at_b: f[4].to_string(),
            gist_b: f[5].to_string(),
        });
    }
    out
}

/// A unit standing free of any library: a gist, a language, and a locator it claims.
///
/// The planted halves are not drawn from a part — there is no part that says them, which is
/// what planting is — so they carry `speculative` and a reference naming the locator alone.
/// What the generator reads of a unit is its gist, its language and its place, and all three
/// are here.
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

/// The distractor corpus: one unit per verse of the committed slice.
///
/// Real material, read by `usfm/1` and cited where the reader put it, so the plants compete
/// against sentences somebody actually wrote rather than against filler.
fn slice_corpus() -> Vec<UnitCore> {
    // A counter as well as the pid: two tests here both build the corpus, on two threads, and
    // one name would have them opening and deleting each other's library.
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "smysl-ge-t11-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch directory");
    let mut lib = Library::create(&root).expect("a library");

    let mut units = Vec::new();
    for book in ["08-RUT.usfm", "65-JUD.usfm"] {
        let path = repo("fixtures/library/bibles/slice/eng-kjv").join(book);
        let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let alias = format!("kjv/{}", book.trim_end_matches(".usfm").to_lowercase());
        let mut spec = AddSpec::new("usfm/1", &alias, "public-domain")
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
                    &alias,
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
                units.push(unit);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
    units
}

/// The distractor corpus `SMYSL_GE_T11_CORPUS` names: every unit of every `*.smy` under it.
fn corpus_from(dir: &Path) -> Vec<UnitCore> {
    let mut files: Vec<PathBuf> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x == "smy") {
                files.push(p);
            }
        }
    }
    assert!(!files.is_empty(), "no `*.smy` under {}", dir.display());
    files.sort();
    let mut units = Vec::new();
    for f in &files {
        let Ok(bytes) = std::fs::read(f) else {
            continue;
        };
        let Ok((records, _)) = smysl_core::from_cbor_seq(&bytes) else {
            continue;
        };
        for r in records {
            if let Record::Unit(u) = r {
                units.push(u);
            }
        }
    }
    units
}

/// The names a corpus holds, found by a rule rather than by a reader.
///
/// # Why a corpus of bare verses would measure one source out of four
///
/// §3.5's generator has four sources and a corpus of raw verses supports exactly one of them:
/// nothing has extracted an entity, nothing has named a theme, and one edition in one language
/// has no alignment to anchor against. Measuring GE-T11 that way measures `lexical` and reports
/// the number as though it were the linker's.
///
/// A real corpus arrives with entities, because the extractor that produced its units produced
/// them. This harness stands in for that extractor with a **rule, calibrated on the corpus
/// itself**: a token is a name when it is written with a capital more often than without,
/// counting every occurrence wherever it falls. "Naomi" is always capitalised and is a name;
/// "word" is capitalised once at the head of a sentence and is not; "the" is not a candidate
/// at all.
///
/// Two earlier rules were wrong in instructive ways. Taking every capitalised token made every
/// sentence-initial word a person. Taking every capitalised token *except the first* lost the
/// subject of the sentence — which in this corpus is the name, because these are sentences
/// about Naomi and Boaz, and the entity source then found nothing on precisely the pairs it
/// exists for. The rule has to be calibrated on usage or it is calibrated on punctuation.
///
/// Crude is the point either way. A hand-chosen entity list would be the measurer deciding
/// which pairs the entity source finds, which is the one thing this file must not do.
fn name_vocabulary(gists: &[String]) -> BTreeSet<String> {
    let mut upper: BTreeMap<String, usize> = BTreeMap::new();
    let mut lower: BTreeMap<String, usize> = BTreeMap::new();
    for g in gists {
        for raw in g.split_whitespace() {
            let token: String = raw
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '\u{2019}')
                .collect();
            let token = token.trim_matches(|c: char| !c.is_alphanumeric());
            if token.chars().count() < 3 {
                continue;
            }
            let key = token.to_lowercase();
            if token.chars().next().is_some_and(char::is_uppercase) {
                *upper.entry(key).or_default() += 1;
            } else {
                *lower.entry(key).or_default() += 1;
            }
        }
    }
    upper
        .into_iter()
        .filter(|(k, n)| *n > lower.get(k).copied().unwrap_or(0))
        .map(|(k, _)| k)
        .collect()
}

/// The names one gist holds, against a vocabulary calibrated over the whole corpus.
fn names(gist: &str, vocabulary: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for raw in gist.split_whitespace() {
        let token: String = raw
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '\u{2019}')
            .collect();
        let token = token.trim_matches(|c: char| !c.is_alphanumeric());
        if !token.chars().next().is_some_and(char::is_uppercase) {
            continue;
        }
        let key = token.to_lowercase();
        if vocabulary.contains(&key) {
            out.insert(key);
        }
    }
    out
}

/// The entity units and `x.text/mentions` edges the rule above produces over a batch.
fn extracted_entities(units: &[(Uid, UnitCore)]) -> Vec<Record> {
    use smysl_core::ids::SchemaId;
    use smysl_core::types::RelKind;
    use smysl_core::Relation;

    let gists: Vec<String> = units.iter().map(|(_, u)| u.gist.clone()).collect();
    let vocabulary = name_vocabulary(&gists);
    let mut by_name: BTreeMap<String, Vec<Uid>> = BTreeMap::new();
    for (uid, u) in units {
        for n in names(&u.gist, &vocabulary) {
            by_name.entry(n).or_default().push(*uid);
        }
    }
    let mut out = Vec::new();
    for (name, mentioners) in by_name {
        let entity = UnitCoreBuilder::new(
            SchemaId::Extension("x.text/entity".to_string()),
            &name,
            Status::Speculative,
        )
        .build()
        .expect("an entity unit");
        let eid = canonical_uid(&entity);
        out.push(Record::Unit(entity));
        for m in mentioners {
            out.push(Record::Relation(Relation::new(
                RelKind::Extension("x.text/mentions".to_string()),
                m,
                eid,
            )));
        }
    }
    out
}

/// Was this pair offered?
fn offered(cs: &[Candidate], a: Uid, b: Uid) -> Option<&Candidate> {
    let key = if a < b { (a, b) } else { (b, a) };
    cs.iter().find(|c| (c.a, c.b) == key)
}

/// The gate.
#[test]
fn the_linker_offers_the_planted_far_contradictions() {
    let plants = plants();
    assert_eq!(
        plants.len(),
        50,
        "GE-T11 is fifty planted pairs; this set has {}",
        plants.len()
    );

    let distractors = match std::env::var_os("SMYSL_GE_T11_CORPUS") {
        Some(dir) => corpus_from(Path::new(&dir)),
        None => slice_corpus(),
    };

    let mut all: Vec<(Uid, UnitCore)> = distractors
        .into_iter()
        .map(|u| (canonical_uid(&u), u))
        .collect();
    let mut pairs: Vec<(String, Uid, Uid)> = Vec::new();
    for p in &plants {
        let a = planted_unit(&p.gist_a, &p.lang, &p.at_a);
        let b = planted_unit(&p.gist_b, &p.lang, &p.at_b);
        let (ua, ub) = (canonical_uid(&a), canonical_uid(&b));
        assert_ne!(ua, ub, "`{}` planted one claim twice", p.id);
        pairs.push((p.id.clone(), ua, ub));
        all.push((ua, a));
        all.push((ub, b));
    }
    // The entities an extractor would have left behind, by rule: see `names`.
    let mut records: Vec<Record> = all.iter().map(|(_, u)| Record::Unit(u.clone())).collect();
    records.extend(extracted_entities(&all));
    let store = Store::from_records(records);
    let total = all.len();

    let cs = candidates(&store, None, None, Budget::DEFAULT);

    let mut found: Vec<(String, Option<String>)> = Vec::new();
    for (id, a, b) in &pairs {
        found.push((
            id.clone(),
            offered(&cs, *a, *b).map(|c| {
                let names: Vec<&str> = c.sources.iter().map(|s| s.as_str()).collect();
                format!("{} ({})", names.join("+"), c.why)
            }),
        ));
    }
    let hit = found.iter().filter(|(_, f)| f.is_some()).count();
    let recall = hit as f64 / pairs.len() as f64;

    if let Some(out) = std::env::var_os("SMYSL_GE_T11_OUT") {
        let mut s = String::from("# pair\tfound\twhy\n");
        for (id, f) in &found {
            s.push_str(&format!(
                "{id}\t{}\t{}\n",
                if f.is_some() { "yes" } else { "no" },
                f.as_deref().unwrap_or("")
            ));
        }
        std::fs::write(&out, s).expect("the per-pair result");
    }

    println!("  GE-T11 (SMYSL-2.4 §6 TX-P7 step 3)");
    println!(
        "    corpus: {total} unit(s), of which {} planted",
        pairs.len() * 2
    );
    println!(
        "    budget: {} candidate(s) per unit, {} neighbour(s), hub {}",
        Budget::DEFAULT.per_unit,
        Budget::DEFAULT.neighbours,
        Budget::DEFAULT.hub
    );
    println!("    candidates offered: {}", cs.len());
    {
        use smysl_text::link::Source;
        for src in [
            Source::Entity,
            Source::Theme,
            Source::Lexical,
            Source::Anchored,
        ] {
            println!(
                "      by source {:>8}: {}",
                src.as_str(),
                cs.iter().filter(|c| c.sources.contains(&src)).count()
            );
        }
    }
    println!("    recall: {recall:.4} ({hit} of {})", pairs.len());
    // A miss has two quite different causes and they need different work: the pair was never
    // generated by any source, or it was generated and the budget did not reach it. Printed
    // apart, because the first is a source to add and the second is a budget to argue about.
    let mut b = Budget::DEFAULT;
    b.per_unit = usize::MAX;
    b.neighbours = 256;
    let unlimited = candidates(&store, None, None, b);
    for ((id, f), (_, a, bb)) in found.iter().zip(pairs.iter()) {
        if f.is_some() {
            continue;
        }
        match offered(&unlimited, *a, *bb) {
            Some(c) => println!("      missed (outranked): {id} — {}", c.why),
            None => println!("      missed (not generated): {id}"),
        }
    }

    // The shape, not only the point. A single figure at one budget cannot say whether the
    // generator is close to the bar or nowhere near it, and that is the difference between a
    // budget to raise and a source to add.
    println!("    recall by budget:");
    for (per_unit, neighbours, hub) in [
        // The two fields that move the figure, around the default. `hub` is held at the
        // default throughout because moving it between 4 and ten thousand moved nothing —
        // which is itself the finding recorded at `Budget::DEFAULT`.
        (8, 12, 32),
        (8, 24, 32),
        (8, 32, 32),
        (12, 32, 32),
        (16, 24, 32),
        (16, 32, 32),
        (16, 48, 32),
        (24, 32, 32),
        (32, 32, 32),
    ] {
        let mut b = Budget::DEFAULT;
        b.per_unit = per_unit;
        b.neighbours = neighbours;
        b.hub = hub;
        let cs = candidates(&store, None, None, b);
        let n = pairs
            .iter()
            .filter(|(_, a, bb)| offered(&cs, *a, *bb).is_some())
            .count();
        println!(
            "      per_unit {per_unit:>2}, neighbours {neighbours:>3}, hub {hub:>5}: {:.4} ({n} of {}), {} candidate(s)",
            n as f64 / pairs.len() as f64,
            pairs.len(),
            cs.len()
        );
    }

    assert!(
        recall >= BAR,
        "GE-T11: recall {recall:.4} is below {BAR} on {} planted pairs",
        pairs.len()
    );
}

/// The pairs the generator offers are the pairs a budget could afford.
///
/// Stated as arithmetic rather than as a figure, because it is the property the budget exists
/// to give and it has to hold on every corpus, not on this one: a run is bounded by
/// `units × per_unit` candidates, so the model turns `smysl link` will buy are known before it
/// is run. A generator that quietly offered the quadratic set would pass the recall gate
/// perfectly and be useless.
#[test]
fn the_budget_bounds_what_the_run_will_cost() {
    let distractors = slice_corpus();
    let records: Vec<Record> = distractors.iter().cloned().map(Record::Unit).collect();
    let store = Store::from_records(records);
    let n = store.units().count();
    assert!(n > 100, "Ruth and Jude are more than a hundred verses: {n}");

    for per_unit in [1, 4, Budget::DEFAULT.per_unit] {
        let mut budget = Budget::DEFAULT;
        budget.per_unit = per_unit;
        let cs = candidates(&store, None, None, budget);
        assert!(
            cs.len() <= n * per_unit,
            "a budget of {per_unit} over {n} units offered {}",
            cs.len()
        );
    }

    // And the quadratic set is what it is being held below.
    let all = n * (n - 1) / 2;
    let cs = candidates(&store, None, None, Budget::DEFAULT);
    assert!(
        cs.len() * 4 < all,
        "{} candidates is not meaningfully fewer than the {all} pairs of the corpus",
        cs.len()
    );
}

/// Every planted pair is far: the two halves are in different books.
///
/// A property of the fixture, asserted because the fixture is the measurement. A pair that
/// drifted into one book would be measured by a gate about *far* contradictions while not
/// being one, and would raise the figure by being easy.
#[test]
fn every_planted_pair_is_in_two_books() {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut books: BTreeMap<String, usize> = BTreeMap::new();
    for p in plants() {
        assert!(seen.insert(p.id.clone()), "`{}` is named twice", p.id);
        let book = |at: &str| at.split('.').next().unwrap_or(at).to_string();
        let (ba, bb) = (book(&p.at_a), book(&p.at_b));
        assert_ne!(
            ba, bb,
            "`{}` plants both halves in {ba}, which is not far",
            p.id
        );
        *books.entry(ba).or_default() += 1;
        *books.entry(bb).or_default() += 1;
        assert!(
            !p.gist_a.is_empty() && !p.gist_b.is_empty(),
            "`{}` has an empty half",
            p.id
        );
    }
    assert_eq!(seen.len(), 50);
}
