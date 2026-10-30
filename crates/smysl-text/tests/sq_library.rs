//! TX-P6 step 6's exit: one query per capability the step enables, over the Bible and chat
//! fixtures, as `fixtures/sq/library/` goldens.
//!
//! The library is built here, from the committed sources, because no committed library holds
//! units: `text add` writes manifests, parts and readings, and the units a query ranks come
//! from ingest, which is impure and not in this test's tier. So the units are drawn by hand
//! the way `ge_t11.rs` draws its distractor corpus — one per verse and one per message, stamped
//! with the provenance a real extraction would carry — and two entities are planted with the
//! `x.text/mentions` edges a real extraction would have written, plus one `x.text/same-as`
//! between them, so the entity bridge has a hop to make.
//!
//! What is deterministic about this is everything: the readers are pinned by GE-T1, the units
//! are a function of the readings, and the goldens are therefore a function of the fixtures.
//! `SMYSL_BLESS=1` rewrites them.
//!
//! The `W415` clause of the exit — *"counts match"* — is asserted in code and not only in the
//! goldens: the number the warning reports is compared to the number of candidates with no
//! time on the axis asked, counted here independently.

#![cfg(all(
    feature = "reader-usfm",
    feature = "reader-telegram",
    feature = "reader-whatsapp",
    feature = "reader-slack",
    feature = "stem"
))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_core::ids::{LangTag, Uid};
use smysl_core::types::{Carry, Record, RelKind, Relation};
use smysl_core::{canonical_uid, KernelType, SchemaId, Status, UnitCoreBuilder};
use smysl_query::{explain, plan, run, Answer, Env, PlanOptions};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::{Input, Params};
use smysl_text::scope::LibraryScope;
use smysl_text::{locator, provenance};

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn blessing() -> bool {
    std::env::var("SMYSL_BLESS").is_ok_and(|v| v == "1")
}

fn compare(path: &Path, got: &str) {
    if blessing() {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the directory");
        std::fs::write(path, got).expect("expectation is writable");
        return;
    }
    let want = std::fs::read_to_string(path).unwrap_or_else(|_| {
        panic!(
            "{} is missing; run with SMYSL_BLESS=1 to write it",
            path.display()
        )
    });
    assert_eq!(want, got, "{} changed", path.display());
}

/// The three editions and the import, then the three chats.
///
/// `(alias, lang, source dir)` for the Bibles. The import is the KJV Ruth a second time under
/// the reserved namespace SMYSL-2.3 A-5 keeps for imports — same tid, a different manifest —
/// which is what makes `import:` answerable and what makes placing a unit by its own manifest
/// necessary.
const BIBLES: &[(&str, &str, &str)] = &[
    ("kjv", "en", "eng-kjv"),
    ("synodal", "ru", "russyn"),
    ("luther", "de", "deu1912"),
];

type Chat = (
    &'static str,
    &'static str,
    &'static str,
    &'static [(&'static str, &'static str)],
);
const CHATS: &[Chat] = &[
    ("telegram-chat", "telegram.json", "telegram/1", &[]),
    (
        "whatsapp-chat",
        "whatsapp.txt",
        "whatsapp/1",
        &[("date-format", "dmy")],
    ),
    ("slack-export", "slack.zip", "slack/1", &[]),
];

struct Built {
    root: PathBuf,
    library: Library,
    /// Every unit drawn, with its text, for the independent `W415` count and the entity plants.
    drawn: BTreeMap<Uid, (String, Option<u64>)>,
}

impl Drop for Built {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn build() -> Built {
    // A counter as well as the pid: two tests build the library on two threads, and one name
    // would have them deleting each other's objects mid-read.
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "smysl-sq-library-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch directory");
    let mut library = Library::create(&root).expect("a library");
    let mut records: Vec<Record> = Vec::new();
    let mut drawn: BTreeMap<Uid, (String, Option<u64>)> = BTreeMap::new();

    let add = |library: &mut Library,
               alias: &str,
               lang: &str,
               reader: &str,
               bytes: &[u8],
               params: &[(&str, &str)],
               levels: &[&str],
               records: &mut Vec<Record>,
               drawn: &mut BTreeMap<Uid, (String, Option<u64>)>| {
        let mut p = Params::new();
        for (k, v) in params {
            p.set(reader, *k, *v).expect("a parameter");
        }
        let mut spec = AddSpec::new(reader, alias, "public-domain")
            .with_lang(LangTag::new(lang).expect("a tag"))
            .with_carry(Carry::Text);
        spec.params = p;
        if reader == "usfm/1" {
            spec.versification = Some("kjv".to_string());
        }
        let added = library
            .add(&Input::new(bytes), &spec, &Caps::DEFAULT)
            .unwrap_or_else(|e| panic!("{alias}: {e}"));
        let manifest = library
            .store()
            .manifest(&added.mid)
            .expect("its manifest")
            .clone();
        for tid in &added.parts {
            let reading = library
                .part_reading(&added.mid, tid)
                .expect("the part's reading");
            for row in &reading.rows {
                if !levels.contains(&row.level.as_str()) {
                    continue;
                }
                let at = row.locator.to_string();
                let Ok(passage) = library.passage(
                    alias,
                    &locator::parse(&at).expect("the reader's own locator"),
                    &Caps::DEFAULT,
                ) else {
                    continue;
                };
                let gist = passage.text.trim().to_string();
                if gist.is_empty() {
                    continue;
                }
                let b = UnitCoreBuilder::new(KernelType::Claim, &gist, Status::Cited);
                let unit = provenance::stamp(
                    b,
                    &provenance::Drawn {
                        tid: passage.tid,
                        locator: Some(&at),
                        manifest: &manifest,
                        observed: row.observed,
                        span: Some((passage.range.start, passage.range.end)),
                        lang: Some(&manifest.lang),
                    },
                )
                .build()
                .expect("a unit of a row");
                drawn.insert(canonical_uid(&unit), (gist, row.observed));
                records.push(Record::Unit(unit));
            }
        }
    };

    for (alias, lang, dir) in BIBLES {
        for book in ["08-RUT.usfm", "65-JUD.usfm"] {
            let path = repo("fixtures/library/bibles/slice").join(dir).join(book);
            let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let alias = format!("{alias}/{}", book.trim_end_matches(".usfm").to_lowercase());
            add(
                &mut library,
                &alias,
                lang,
                "usfm/1",
                &raw,
                &[],
                &["verse"],
                &mut records,
                &mut drawn,
            );
        }
    }
    {
        let path = repo("fixtures/library/bibles/slice/eng-kjv/08-RUT.usfm");
        let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        add(
            &mut library,
            "import:gutenberg/08-rut",
            "en",
            "usfm/1",
            &raw,
            &[],
            &["verse"],
            &mut records,
            &mut drawn,
        );
    }
    for (alias, file, reader, params) in CHATS {
        let path = repo("fixtures/library/readers").join(file);
        let raw = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        add(
            &mut library,
            alias,
            "mul",
            reader,
            &raw,
            params,
            &["message"],
            &mut records,
            &mut drawn,
        );
    }

    // The entity plants. Rule 7 makes an entity's identity its name in one expression, so
    // the same person in the English and Russian editions is two units; the same-as edge is
    // the record that says they are one, and it is what `about entity("Naomi")` has to cross
    // to answer in Russian.
    let entity = SchemaId::parse(smysl_core::schemas::text::ENTITY).expect("a schema id");
    // `Speculative`, because an entity has no source to cite — rule 7 forbids it a span —
    // and `Cited` is refused without one.
    let naomi = UnitCoreBuilder::new(entity.clone(), "Naomi", Status::Speculative)
        .build()
        .expect("an entity");
    let noemin = UnitCoreBuilder::new(entity, "Ноеминь", Status::Speculative)
        .build()
        .expect("an entity");
    let (naomi_uid, noemin_uid) = (canonical_uid(&naomi), canonical_uid(&noemin));
    records.push(Record::Unit(naomi));
    records.push(Record::Unit(noemin));
    let mentions = RelKind::parse(smysl_core::schemas::text::MENTIONS).expect("a kind");
    let same_as = RelKind::parse(smysl_core::schemas::text::SAME_AS).expect("a kind");
    for (uid, (gist, _)) in &drawn {
        if gist.contains("Naomi") {
            records.push(Record::Relation(Relation::new(
                mentions.clone(),
                *uid,
                naomi_uid,
            )));
        }
        if gist.contains("Ноеминь") {
            records.push(Record::Relation(Relation::new(
                mentions.clone(),
                *uid,
                noemin_uid,
            )));
        }
    }
    records.push(Record::Relation(Relation::new(
        same_as, naomi_uid, noemin_uid,
    )));

    library
        .append_records(&records)
        .expect("the units and edges append");
    Built {
        root,
        library,
        drawn,
    }
}

fn render(name: &str, built: &Built, query: &str) -> String {
    let scope = LibraryScope::new("library", &built.library);
    let env = Env::new(built.library.store(), &scope)
        .with_times(&scope)
        .with_entities(&scope);
    let opts = PlanOptions::default();

    let mut out = String::new();
    out.push_str(&format!("query {name}\n"));
    for line in query.lines() {
        out.push_str(&format!("  | {line}\n"));
    }
    let parsed = match smysl_query::parse(query) {
        Ok(p) => p,
        Err(d) => {
            out.push_str(&format!(
                "parse refused: {} {}\n",
                d.code.as_str(),
                d.message
            ));
            return out;
        }
    };
    let plan = match plan(&parsed.query, &env, &opts) {
        Ok(p) => p,
        Err(diags) => {
            for d in diags {
                out.push_str(&format!("refused: {} {}\n", d.code.as_str(), d.message));
            }
            return out;
        }
    };
    out.push_str(&explain(&plan));
    match run(&plan, &env, &opts) {
        Ok(result) => {
            for w in &result.warnings {
                out.push_str(&format!("warning: {} {}\n", w.code.as_str(), w.message));
                if w.code.as_str() == "SMY-W415" {
                    // The exit's clause: the count the warning reports is the count of
                    // candidates with no time on the axis asked, taken independently of the
                    // executor — here, from the `observed` the reading rows carried, which is
                    // the only time any unit in this library has on either axis.
                    let candidates = plan.candidates().expect("not a set expression");
                    let undated = candidates
                        .iter()
                        .filter(|u| built.drawn.get(u).is_some_and(|(_, at)| at.is_none()))
                        .count();
                    assert!(
                        w.message.starts_with(&format!("{undated} unit(s)")),
                        "{name}: W415 reported `{}`, but {undated} candidate(s) carry no time",
                        w.message
                    );
                }
            }
            match &result.answer {
                Answer::Count(n) => out.push_str(&format!("count {n}\n")),
                Answer::Ranked(fused) => {
                    out.push_str(&format!("ranked {}\n", fused.len()));
                    for f in fused {
                        let gist = built
                            .drawn
                            .get(&f.uid)
                            .map(|(g, _)| g.as_str())
                            .unwrap_or("");
                        out.push_str(&format!("  {} {:.6} {}\n", f.uid, f.score, gist));
                    }
                }
                other => {
                    let uids = other.uids();
                    out.push_str(&format!("units {}\n", uids.len()));
                    for uid in uids {
                        let gist = built.drawn.get(&uid).map(|(g, _)| g.as_str()).unwrap_or("");
                        out.push_str(&format!("  {uid} {gist}\n"));
                    }
                }
            }
        }
        Err(diags) => {
            for d in diags {
                out.push_str(&format!("run refused: {} {}\n", d.code.as_str(), d.message));
            }
        }
    }
    out
}

/// Every `.sq` under `fixtures/sq/library/`, against its `.golden`.
#[test]
fn every_capability_of_step_six_answers_over_the_bible_and_chat_fixtures() {
    let built = build();
    let dir = repo("fixtures/sq/library");
    let mut queries: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "sq"))
        .collect();
    queries.sort();
    assert!(!queries.is_empty(), "no queries under {}", dir.display());

    for path in &queries {
        let name = path
            .file_stem()
            .expect("a stem")
            .to_string_lossy()
            .to_string();
        let query = std::fs::read_to_string(path).expect("the query");
        let got = render(&name, &built, &query);
        compare(&path.with_extension("golden"), &got);
    }
}

/// The six items `StoreScope` refuses are each answered here, and the one `StoreScope` answers
/// still is.
#[test]
fn the_six_scope_items_resolve_and_store_still_does() {
    let built = build();
    let scope = LibraryScope::new("library", &built.library);
    let env = Env::new(built.library.store(), &scope);
    let opts = PlanOptions::default();
    for item in [
        "texts",
        "library",
        "collection:kjv",
        "import:gutenberg",
        "channels",
        "channel:telegram-chat",
        "channel:general",
        "store:library",
    ] {
        let q = format!("from {item}\ncount");
        let parsed = smysl_query::parse(&q).expect("parses");
        let plan =
            plan(&parsed.query, &env, &opts).unwrap_or_else(|d| panic!("{item}: {}", d[0].message));
        let n = plan.candidates().expect("one pipeline").len();
        assert!(n > 0, "`from {item}` resolved to nothing");
    }
    for item in [
        "collection:nowhere",
        "import:nobody",
        "channel:silence",
        "store:other",
    ] {
        let q = format!("from {item}\ncount");
        let parsed = smysl_query::parse(&q).expect("parses");
        let diags = plan(&parsed.query, &env, &opts).expect_err("refused");
        assert_eq!(diags[0].code.as_str(), "SMY-E511", "{item}");
    }
}

/// TX-P8 step 3 over a library: `ask` resolves its subject through `LibraryScope`, and one
/// candidate joined by `same-as` answers in both languages.
#[test]
fn a_dossier_over_the_library_crosses_same_as_and_its_slots_are_saved_queries() {
    use smysl_query::{ask, AskOptions};
    let built = build();
    let scope = LibraryScope::new("library", &built.library);
    let env = Env::new(built.library.store(), &scope)
        .with_times(&scope)
        .with_entities(&scope);
    let owner = smysl_core::ids::AgentId::new("tool:test").expect("an agent");
    let d = ask("Who is Naomi?", &env, &AskOptions::new(owner.clone())).expect("a dossier");
    assert_eq!(d.subject, "Naomi");
    assert_eq!(d.entities.len(), 2, "Naomi and Ноеминь are one candidate");
    let characteristic = d
        .slots
        .iter()
        .find(|s| s.name == "characteristic")
        .expect("the slot");
    // Every drawn unit is a claim, so `characteristic` holds the mentions, capped at twelve by
    // the template; and the slot reached them across the same-as edge, since the candidate it
    // was asked under is the pair.
    assert_eq!(characteristic.units.len(), 12);
    assert!(characteristic.text.contains("about entity(\"Naomi\")"));
    // Lowercase works, because a question is typed by a person.
    let lower = ask("who is naomi", &env, &AskOptions::new(owner)).expect("a dossier");
    assert_eq!(lower.entities, d.entities);
    assert_eq!(lower.subject, "naomi");
}

/// OQ-91: a lexical source reads the postings the library keeps, and a library keeping none
/// builds them for the query's units by the same rule — so the two give one answer, rank for
/// rank and score for score, at every scope and under every IDF scope.
#[test]
fn find_reads_the_kept_postings_and_a_build_gives_the_same_answer() {
    use smysl_query::ports::IdfScope;
    let mut built = build();
    // The kept table is what `text index --rebuild` writes.
    built.library.reindex();
    assert!(
        built.library.postings().is_some(),
        "the library keeps no postings, so the kept path would not be exercised"
    );
    let queries = [
        "from library\nfind \"Ruth\"\nlimit 10",
        "from library\nfind \"the lord\"\nlimit 25",
        "from library\nfind \"Gott\"\nlimit 10",
        "from collection:kjv\nfind \"Naomi\"\nlimit 10",
        "from texts\nfind \"Boas\"\nlimit 10",
        "from channels\nfind \"meeting\"\nlimit 10",
    ];
    let mut ranked = 0usize;
    for idf in [IdfScope::Partition, IdfScope::Collection, IdfScope::Library] {
        let mut opts = PlanOptions::default();
        opts.idf_scope = idf;
        for q in queries {
            let answer = |scope: &LibraryScope<'_>| {
                let env = Env::new(built.library.store(), scope)
                    .with_times(scope)
                    .with_entities(scope);
                let parsed = smysl_query::parse(q).expect("parses");
                let plan = plan(&parsed.query, &env, &opts).expect("plans");
                format!("{:?}", run(&plan, &env, &opts).expect("runs").answer)
            };
            let kept = answer(&LibraryScope::new("library", &built.library));
            let fresh = answer(&LibraryScope::new("library", &built.library).building_postings());
            assert_eq!(
                kept, fresh,
                "{idf:?} {q}: the kept postings and a build disagree"
            );
            if kept.contains("score") {
                ranked += 1;
            }
        }
    }
    assert!(
        ranked > 6,
        "too few queries ranked anything to compare: {ranked}"
    );
}
