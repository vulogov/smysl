//! **Information theory, end to end**: every layer of SMYSL-2 over one small library.
//!
//! ```text
//! cargo run --example information_theory
//! cargo run --example information_theory --features store-redb   # adds §14, the unit store
//! ```
//!
//! The program builds a library in a scratch directory, works through it section by section,
//! and prints what each layer did. Nothing touches the network: the one model call is answered
//! by a scripted provider (`provider.rs`), and every text is in `corpus.rs`.
//!
//! | § | layer | RFC |
//! |---|---|---|
//! | 1 | library: readers, manifests, parts, readings, the four identities | 2.3 A-3/A-5, 2.4 §3.1 |
//! | 2 | pseudonymised speakers in a chat export | 2.4 §3.2 |
//! | 3 | passages by locator; units drawn with provenance and spans | 2.4 §3.4 |
//! | 4 | text ingest: windows, consensus passes, agreement, reply edges | 2.4 §3.4, OQ-74 |
//! | 5 | same-as by `identical-span`, and proposition classes | 2.4 TX-P7 |
//! | 6 | datings, effective time (rule E), a lock spelled by its did | 2.3 A-5/A-6, OQ-73 |
//! | 7 | sq/1 queries over the library | 2.5, 2.9 |
//! | 8 | a saved query, appended to and rerun | 2.5 §3.9 |
//! | 9 | a dossier for a question | 2.5 TX-P8 |
//! | 10 | text measures: entropy, compression, Zipf, n-gram surprisal, divergence | 2.6 |
//! | 11 | export, verify and import | 2.7 |
//! | 12 | redaction (rule Z) | 2.3 A-5, 2.4 §3.1 |
//! | 13 | `check` over the whole store | 2.1 |
//! | 14 | the on-disk unit store (`--features store-redb`) | 2.8 |

mod corpus;
mod provider;

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use smysl::query::{self as sq, AskOptions, Env, PlanOptions};
use smysl::text::library::{AddSpec, Library};
use smysl::text::limits::Caps;
use smysl::text::readers::Input;
use smysl::text::reading::Level;
use smysl::text::scope::LibraryScope;
use smysl::text::{locator, provenance};
use smysl::{
    canonical_uid, AgentId, Axis, Carry, Commit, Commitment, Dating, DatingTarget, DatingValue,
    Hlc, IngestOptions, Ingestor, KernelType, LangTag, Passes, Record, RelKind, Relation, Rung,
    SchemaId, Status, TextIngest, TextTarget, Uid, UnitCoreBuilder, WriteContext,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Diagnostics as an error, for the few calls that refuse with a list of them.
fn refused(ds: Vec<smysl::Diagnostic>) -> Box<dyn Error> {
    ds.iter()
        .map(|d| format!("{}: {}", d.code.as_str(), d.message))
        .collect::<Vec<_>>()
        .join("; ")
        .into()
}

fn section(n: u32, title: &str) {
    println!("\n§{n}  {title}\n{}", "─".repeat(title.len() + 5));
}

fn agent() -> AgentId {
    AgentId::new("human:example").expect("an agent id")
}

/// A clock reading for records this program writes. Fixed, so a second run writes the same
/// records and therefore the same identities.
fn at(ms: u64) -> Hlc {
    Hlc::new(1_726_500_000_000 + ms, 0, agent())
}

fn scratch(name: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "smysl-information-theory-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// What the sections hand each other.
struct World {
    root: PathBuf,
    lib: Library,
    /// Units drawn from the notes, by their gist.
    drawn: BTreeMap<String, Uid>,
    shannon: Uid,
}

fn main() -> Result<()> {
    let root = scratch("library")?;
    let mut w = library(&root)?;
    pseudonyms(&mut w)?;
    draw(&mut w)?;
    ingest(&mut w)?;
    same_as(&mut w)?;
    time(&mut w)?;
    queries(&w)?;
    saved(&mut w)?;
    dossier(&w)?;
    measures(&mut w)?;
    export(&w)?;
    redaction(&mut w)?;
    check(&w)?;
    scale(&w)?;
    println!(
        "\nThe library is at {}; remove it when done.",
        w.root.display()
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §1  The library
// ---------------------------------------------------------------------------------------------

/// A library is a catalog of manifests over content-addressed parts. `add` runs a reader, cuts
/// the text into parts, records each part's reading (its segment table) and writes a manifest.
fn library(root: &Path) -> Result<World> {
    section(1, "A library of texts");
    let mut lib = Library::create(root)?;
    let en = LangTag::new("en")?;

    let mut add = |alias: &str, reader: &str, bytes: &[u8], licence: &str| -> Result<()> {
        let spec = AddSpec::new(reader, alias, licence)
            .with_lang(en.clone())
            .with_carry(Carry::Text);
        let added = lib.add(&Input::new(bytes), &spec, &Caps::DEFAULT)?;
        println!("{alias:<18} {reader:<8} {}", added.mid.canonical());
        for tid in &added.parts {
            let reading = lib.part_reading(&added.mid, tid)?;
            println!(
                "{:<27} part {}  {} row(s)",
                "",
                tid.short(),
                reading.rows.len()
            );
        }
        Ok(())
    };
    add("notes", "md/1", corpus::NOTES.as_bytes(), "CC0-1.0")?;
    add(
        "lecture/coding",
        "txt/1",
        corpus::CODING.as_bytes(),
        "CC0-1.0",
    )?;
    add(
        "anthology/coding",
        "txt/1",
        corpus::CODING.as_bytes(),
        "CC-BY-4.0",
    )?;
    add("draft", "txt/1", corpus::DRAFT.as_bytes(), "CC0-1.0")?;

    // The four identities: a tid names normalised bytes, a mid a manifest, an rdid a reading,
    // and a manifest's part entry pins the reading's structure hash beside them.
    let (mid, manifest) = lib
        .store()
        .manifests()
        .find(|(_, m)| m.alias.as_str() == "notes")
        .ok_or("the notes manifest")?;
    let entry = &manifest.parts[0];
    println!(
        "\nnotes, part 0:\n  tid       {}\n  rdid      {}\n  structure {}\n  mid       {}",
        entry.tid.canonical(),
        entry.rdid.canonical(),
        hex(&entry.structure),
        mid.canonical()
    );
    let parts: std::collections::BTreeSet<_> = lib
        .store()
        .manifests()
        .filter(|(_, m)| m.alias.as_str().ends_with("/coding"))
        .flat_map(|(_, m)| m.parts.iter().map(|e| e.tid))
        .collect();
    println!(
        "lecture/coding and anthology/coding share {} part: the same bytes are one tid",
        parts.len()
    );

    Ok(World {
        root: root.to_path_buf(),
        lib,
        drawn: BTreeMap::new(),
        shannon: Uid::from_bytes([0; 32]),
    })
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ---------------------------------------------------------------------------------------------
// §2  Pseudonymised speakers
// ---------------------------------------------------------------------------------------------

/// A chat export's speakers are platform ids. Under a pseudonym key they are replaced in the
/// reading by keyed pseudonyms, the same person getting the same pseudonym in every append.
/// The key is fixed here so the example is reproducible; `smysl text add --pseudonymise` keeps
/// a random one in `<library>/secrets/`.
fn pseudonyms(w: &mut World) -> Result<()> {
    section(2, "A chat, with its speakers pseudonymised");
    let key = smysl::text::speaker::Key::from_bytes([42; 32]);
    let spec = AddSpec::new("telegram/1", "study-group", "CC0-1.0")
        .with_lang(LangTag::new("en")?)
        .with_carry(Carry::Text)
        .pseudonymised(key);
    let chat = corpus::chat();
    let added = w
        .lib
        .add(&Input::new(chat.as_bytes()), &spec, &Caps::DEFAULT)?;
    println!("study-group        telegram/1 {}", added.mid.canonical());
    let reading = w.lib.part_reading(&added.mid, &added.parts[0])?;
    for row in reading
        .rows
        .iter()
        .filter(|r| r.level.as_str() == "message")
    {
        println!(
            "  {:<6} {:<26} observed {} ms",
            row.locator.to_string(),
            row.speaker.as_deref().unwrap_or("-"),
            row.observed.unwrap_or(0)
        );
    }
    println!("the platform ids `user-ada`, `user-bruno` and `user-clare` are in no row");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §3  Passages and drawn units
// ---------------------------------------------------------------------------------------------

/// A passage is the bytes under a locator. A unit drawn from one carries the part's tid, the
/// manifest, the byte span and the language — the provenance `provenance::stamp` writes.
fn draw(w: &mut World) -> Result<()> {
    section(3, "Passages and the units drawn from them");
    let (mid, manifest) = w
        .lib
        .store()
        .manifests()
        .find(|(_, m)| m.alias.as_str() == "notes")
        .map(|(mid, m)| (*mid, m.clone()))
        .ok_or("the notes manifest")?;
    let mut records = Vec::new();
    for tid in manifest.parts.iter().map(|e| e.tid) {
        let reading = w.lib.part_reading(&mid, &tid)?;
        for row in reading
            .rows
            .iter()
            .filter(|r| r.level.as_str() == "paragraph")
        {
            let at = row.locator.to_string();
            let passage = w
                .lib
                .passage("notes", &locator::parse(&at)?, &Caps::DEFAULT)?;
            // A gist is a sentence, not a paragraph: the default granularity profile allows
            // thirty tokens (`SMY-E022`). The first clause of the first sentence is enough here.
            let gist = passage
                .text
                .split(['.', ',', ':'])
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            let unit = provenance::stamp(
                UnitCoreBuilder::new(KernelType::Claim, &gist, Status::Cited),
                &provenance::Drawn {
                    tid: passage.tid,
                    locator: Some(&at),
                    manifest: &manifest,
                    observed: row.observed,
                    span: Some((passage.range.start, passage.range.end)),
                    lang: Some(&manifest.lang),
                },
            )
            .build()?;
            println!("{at:<22} {gist}");
            w.drawn.insert(gist, canonical_uid(&unit));
            records.push(Record::Unit(unit));
        }
    }

    // `x.text/mentions` and the entity type belong to the `x.text/v1` schema, which a store
    // declares before it uses them (`SMY-W013` otherwise). Ingest declares it for itself.
    records.insert(0, Record::SchemaDecl(smysl::schemas::text::decl()));
    // And `x.query/v1`, whose `answers` edges a saved query writes (§8).
    records.insert(1, Record::SchemaDecl(smysl::schemas::query::decl()));

    // An entity, and the edges that say which claims mention it. Rule 7 gives an entity no
    // span, so it is `speculative`: there is nothing to cite.
    let entity = SchemaId::parse(smysl::schemas::text::ENTITY)?;
    // Its source is the expression it was named in, `alias:<alias>`, and nothing finer: an
    // entity's identity is its name in one expression, so fifty windows naming it are one unit.
    let shannon = UnitCoreBuilder::new(entity, "Shannon", Status::Speculative)
        .source(smysl::SourceRef::new(smysl::SourceKind::Doc, "alias:notes"))
        .build()?;
    w.shannon = canonical_uid(&shannon);
    records.push(Record::Unit(shannon));
    let mentions = RelKind::parse(smysl::schemas::text::MENTIONS)?;
    for (gist, uid) in &w.drawn {
        if gist.contains("Shannon") {
            records.push(Record::Relation(Relation::new(
                mentions.clone(),
                *uid,
                w.shannon,
            )));
        }
    }
    let n = records.len();
    w.lib.append_records(&records)?;
    println!("appended {n} records: the claims, the entity `Shannon` and its mentions");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §4  Text ingest
// ---------------------------------------------------------------------------------------------

/// `ingest_text` windows an expression, asks the model for units twice per window (consensus),
/// attaches each quote to its bytes, and draws `x.text/replies` between the units of a reply
/// and the message it answers.
fn ingest(w: &mut World) -> Result<()> {
    section(4, "Text ingest over the chat");
    let registry = provider::registry();
    let opts = IngestOptions::at_rung(Rung::Document)
        .with_passes(Passes::Consensus)
        .with_model("scripted")
        .with_max_output(512)
        .with_agent(AgentId::new("tool:example-ingest")?)
        // The clock the ingest attests under. A unit was said no later than the store first
        // heard of it (rule E's first-seen bound), so a run stamped before the chat happened
        // would make every message's time contradict its own attestation.
        .with_now(Hlc::new(1_705_400_000_000, 0, agent()));
    let mut t = TextIngest::new(
        &mut w.lib,
        TextTarget::Alias("study-group".to_string()),
        Level::new("message").ok_or("a level")?,
    )
    .with_digest_budget(0);
    let report = Ingestor::new(&registry, opts).ingest_text(&mut t)?;
    println!(
        "windows {}  calls {}  passes {}  units {}  spanned {}  reply edges {}",
        report.windows, report.calls, report.passes, report.units, report.spanned, report.replies
    );
    let a = &report.agreement;
    println!(
        "agreement: {} of {} place(s) identical-span; {} of {} window(s) answered identically by \
         both passes, so the provider reproduced: {:?}",
        a.agreed,
        a.comparable,
        a.reproduced,
        a.answered,
        a.reproduced()
    );
    for d in &report.diagnostics {
        println!("  {} {}", d.code.as_str(), d.message);
    }

    // The reply edges, as the gists at either end.
    let replies = RelKind::parse(smysl::schemas::text::REPLIES)?;
    for r in w.lib.store().relations_of_kind(&replies) {
        let gist = |u| {
            w.lib
                .store()
                .get(u)
                .map(|x| x.core.gist.clone())
                .unwrap_or_default()
        };
        println!("  {}\n    replies to {}", gist(&r.from), gist(&r.to));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §5  Same-as
// ---------------------------------------------------------------------------------------------

/// Two expressions that hold the same part, each with a unit over the same span saying the same
/// thing, are one proposition. The `identical-span` engine proposes the edge; the classes are
/// the store's.
fn same_as(w: &mut World) -> Result<()> {
    section(5, "Same-as: one paragraph, two expressions");
    let gist = "A prefix code gives likelier symbols shorter words";
    let mut records = Vec::new();
    for alias in ["lecture/coding", "anthology/coding"] {
        let (_, manifest) = w
            .lib
            .store()
            .manifests()
            .find(|(_, m)| m.alias.as_str() == alias)
            .map(|(mid, m)| (*mid, m.clone()))
            .ok_or("the coding manifest")?;
        let tid = manifest.parts[0].tid;
        let len = w.lib.part(&tid)?.text.len() as u64;
        let unit = provenance::stamp(
            UnitCoreBuilder::new(KernelType::Claim, gist, Status::Cited),
            &provenance::Drawn {
                tid,
                locator: Some("p.1"),
                manifest: &manifest,
                observed: None,
                span: Some((0, len)),
                lang: Some(&manifest.lang),
            },
        )
        .build()?;
        records.push(Record::Unit(unit));
    }
    w.lib.append_records(&records)?;

    let found = smysl::same_as::propose(
        w.lib.store(),
        &smysl::text::proposition::IdenticalSpan,
        None,
        at(10),
    );
    for p in &found.proposals {
        println!(
            "proposed {} = {}\n  because {}",
            p.a.short(),
            p.b.short(),
            p.why
        );
    }
    if let Some(staged) = &found.staged {
        w.lib.append_records(&staged.records())?;
    }
    let classes =
        smysl::proposition::classes(w.lib.store(), smysl::proposition::Policy::Strict, None);
    println!("{} class(es) of more than one unit", classes.len());
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §6  Time
// ---------------------------------------------------------------------------------------------

/// A dating says when something was said, composed or is about. Rule E solves every dating and
/// ordering constraint into each subject's effective interval. A `canonical` commitment on a
/// dating locks it: a later, conflicting dating is held for review.
fn time(w: &mut World) -> Result<()> {
    section(6, "Datings and effective time");
    let history = *w
        .drawn
        .iter()
        .find(|(g, _)| g.contains("1948"))
        .map(|(_, u)| u)
        .ok_or("the history claim")?;
    let entropy = *w
        .drawn
        .iter()
        .find(|(g, _)| g.starts_with("Entropy"))
        .map(|(_, u)| u)
        .ok_or("the entropy claim")?;

    // On the `about` axis: the claim was written in 2024, but it is about 1948.
    let published = Dating::new(
        DatingTarget::Unit(history),
        Axis::About,
        DatingValue::Absolute("1948".to_string()),
        agent(),
        at(20),
    )
    // A dating applies only when something grounds it: here, the claim that says 1948.
    .with_basis(history);
    // The entropy claim is about a time after the history it rests on: an Allen relation, not
    // a date, and rule E carries the 1948 floor across it.
    let after = Dating::new(
        DatingTarget::Unit(entropy),
        Axis::About,
        DatingValue::Relative {
            allen: smysl::Allen::After,
            target: DatingTarget::Unit(history),
        },
        agent(),
        at(21),
    )
    .with_basis(history);
    let lock = Commit::new(
        published.did().as_uid(),
        Commitment::Canonical,
        agent(),
        at(22),
    );
    let records = vec![
        Record::Dating(published.clone()),
        Record::Dating(after),
        Record::Commit(lock),
    ];
    w.lib.append_records(&records)?;

    use smysl::text::time::engine;
    let show = |lib: &Library, when: &str| {
        let effective = engine::effective(lib.store(), Axis::About);
        for (name, uid) in [("history", history), ("entropy", entropy)] {
            println!(
                "{when:<16} {name:<8} about {}",
                effective.interval(&smysl::text::Subject::Unit(uid))
            );
        }
        effective.held.len()
    };
    // A `canonical` lock holds the dating it names until somebody has reviewed it.
    let held = show(&w.lib, "locked");
    println!("{held} dating(s) held for review");

    // Reviewing it is a resolution naming the contention the hold derives.
    let contention = engine::hold_contention(&published.did(), &published)
        .ok_or("a held dating derives a contention")?;
    let resolution = smysl::Resolution::new(
        smysl::ResolutionTarget::Contention(contention.clone()),
        agent(),
        at(23),
    );
    w.lib.append_records(&[Record::Resolution(resolution)])?;
    println!("resolved {}", contention.as_str());
    show(&w.lib, "after review");

    // The same three records in surface text. The lock names a dating, so it is spelled by the
    // dating's did (`d3:`), not as a uid (OQ-73).
    let text = smysl::write_surface(None, &records, &WriteContext::default());
    println!("\n{text}");
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §7  Queries
// ---------------------------------------------------------------------------------------------

/// Plan and run one sq/1 query over the library, printing the plan's shape and the answer.
fn ask_sq(w: &World, query: &str) -> Result<Vec<Uid>> {
    let scope = LibraryScope::new("library", &w.lib);
    let env = Env::new(w.lib.store(), &scope)
        .with_times(&scope)
        .with_entities(&scope);
    let opts = PlanOptions::default();
    let parsed = sq::parse(query).map_err(|d| refused(vec![d]))?;
    let plan = sq::plan(&parsed.query, &env, &opts).map_err(refused)?;
    let result = sq::run(&plan, &env, &opts).map_err(refused)?;
    println!("» {}", query.replace('\n', " │ "));
    for d in &result.warnings {
        println!("  {} {}", d.code.as_str(), d.message);
    }
    let gist = |u: &Uid| {
        w.lib
            .store()
            .get(u)
            .map(|x| x.core.gist.clone())
            .unwrap_or_default()
    };
    match &result.answer {
        sq::Answer::Count(n) => {
            println!("  count {n}");
            Ok(Vec::new())
        }
        sq::Answer::Ranked(fused) => {
            for f in fused.iter().take(4) {
                println!("  {:.4}  {}", f.score, gist(&f.uid));
            }
            Ok(fused.iter().map(|f| f.uid).collect())
        }
        other => {
            let uids = other.uids();
            for u in uids.iter().take(4) {
                println!("  {}", gist(u));
            }
            if uids.len() > 4 {
                println!("  … {} in all", uids.len());
            }
            Ok(uids)
        }
    }
}

fn queries(w: &World) -> Result<()> {
    section(7, "sq/1 over the library");
    for q in [
        // Scopes: the whole library, its texts, its channels.
        "from library\ncount",
        "from channels\ncount",
        // Lexical search, ranked by BM25 over the library's analyzer chain.
        "from texts\nfind \"entropy\"",
        // An entity, reached through the `x.text/mentions` edges.
        "from texts\nabout entity(\"Shannon\")\nunits",
        // Time, on the said axis, from the chat's own timestamps.
        "from channels\nobserved during 2024-01-15\ncount",
        "from channels\nsaid during 2024-01-15\ncount",
        // A walk over the reply edges, keeping only what the walk reached.
        "from channels\nfind \"redundancy\"\nexpand -[x.text/replies]- depth 2 only\nunits",
        // A filter, and a sort on more than one key.
        "from library\nwhere kind = claim and status >= cited\norder by salience descending, \
         status ascending\nlimit 3\nunits",
        // Set operations. The reprinted paragraph answers twice: one unit under each expression.
        "from texts\nfind \"code\"\nintersect\nfrom texts\nfind \"symbol\"\nunits",
    ] {
        ask_sq(w, q)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §8  Saved queries
// ---------------------------------------------------------------------------------------------

/// A saved query is a unit: its text, its recipe and the results it had. Rerun later, it
/// reports exactly what the data added and removed since.
fn saved(w: &mut World) -> Result<()> {
    section(8, "A saved query, rerun after the data grew");
    let query = "from library\nfind \"capacity\"";
    let opts = PlanOptions::default();
    let records = {
        let scope = LibraryScope::new("library", &w.lib);
        let env = Env::new(w.lib.store(), &scope);
        let parsed = sq::parse(query).map_err(|d| refused(vec![d]))?;
        let bound = sq::bind(&parsed.query, &opts.params).map_err(|d| refused(vec![d]))?;
        let plan = sq::plan(&bound, &env, &opts).map_err(refused)?;
        let run = sq::run(&plan, &env, &opts).map_err(refused)?;
        let (unit, edges) =
            sq::to_records(&bound, &plan, &run, &opts).map_err(|d| refused(vec![d]))?;
        println!(
            "saved `{}` with {} result(s)",
            query.replace('\n', " │ "),
            run.answer.uids().len()
        );
        let mut records = vec![Record::Unit(unit)];
        records.extend(edges.into_iter().map(Record::Relation));
        records
    };
    let saved_uid = match &records[0] {
        Record::Unit(u) => canonical_uid(u),
        _ => unreachable!(),
    };
    w.lib.append_records(&records)?;

    // New data the query will find.
    let claim = UnitCoreBuilder::new(
        KernelType::Claim,
        "The capacity of a binary symmetric channel falls as its crossover probability rises",
        Status::Speculative,
    )
    .build()?;
    w.lib.append_records(&[Record::Unit(claim)])?;

    let scope = LibraryScope::new("library", &w.lib);
    let env = Env::new(w.lib.store(), &scope);
    let loaded = sq::load(w.lib.store(), &saved_uid).map_err(|e| format!("{e:?}"))?;
    let again = sq::rerun(&loaded, &env, &opts, false).map_err(refused)?;
    println!(
        "rerun: recipe unchanged {}, {} added, {} removed",
        again.recipe_matches,
        again.data_effect.added.len(),
        again.data_effect.removed.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §9  Dossiers
// ---------------------------------------------------------------------------------------------

/// `ask` classifies a question, finds its subject, and fills a template's slots with saved
/// queries — characteristic claims, counter-evidence, what is reported — over the library.
fn dossier(w: &World) -> Result<()> {
    section(9, "A dossier for a question");
    let scope = LibraryScope::new("library", &w.lib);
    let env = Env::new(w.lib.store(), &scope)
        .with_times(&scope)
        .with_entities(&scope);
    let d = sq::ask("Who is Shannon?", &env, &AskOptions::new(agent()))
        .map_err(|e| format!("{e:?}"))?;
    println!(
        "subject `{}`, {} entity candidate(s)",
        d.subject,
        d.entities.len()
    );
    for slot in &d.slots {
        println!("  {:<16} {} unit(s)", slot.name, slot.units.len());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §10  Measures
// ---------------------------------------------------------------------------------------------

/// The text measures of SMYSL-2.6 over the library's texts: T1 entropy, T2 compression, T4
/// Zipf–Heaps, T5 Kneser–Ney surprisal and T7 Jensen–Shannon divergence between texts. Each
/// result is a value with a recipe that hashes the same everywhere, and can be written back as
/// a `@data` unit.
fn measures(w: &mut World) -> Result<()> {
    section(10, "Measures over the text");
    use smysl::measure::text::{texts, Corpus};
    use smysl::measure::{self as ms, Outcome, Recipe};

    let scope_text = "from texts";
    let level = ms::Level::Part;
    let scope = LibraryScope::new("library", &w.lib);
    let env = Env::new(w.lib.store(), &scope)
        .with_times(&scope)
        .with_entities(&scope);
    let resolved = ms::resolve(scope_text, &env, &PlanOptions::default()).map_err(refused)?;
    let found = texts(scope_text, &w.lib).map_err(|d| refused(vec![d]))?;
    let corpus = Corpus::load(&w.lib, &found, level, None).map_err(|e| format!("{e:?}"))?;
    let input = ms::Input::new(w.lib.store(), &resolved)
        .with_library(&w.lib)
        .with_text(&corpus);
    for id in ["T1", "T2", "T4", "T5", "T7", "A2", "A4", "A9"] {
        let mut recipe = Recipe::new("", 0, scope_text);
        recipe.level = level;
        // The text measures want ten thousand tokens before they report; this corpus has a few
        // hundred, so the floor is lowered and the result is flagged `low-n`.
        recipe.min_n = Some(10);
        // Counting propositions needs a rule for when two units are one (A2).
        recipe.policy = Some(ms::ClassPolicy::Strict);
        let r = ms::run(&[id], &recipe, &input).remove(0);
        match &r.outcome {
            Outcome::Value(v) if v.headline.is_some() => {
                println!("{id:<3} {}", ms::gist(&r.recipe, v))
            }
            // T7 has no single headline: it is a divergence per pair of texts.
            Outcome::Value(v) => println!(
                "{id:<3} {} value(s): {}",
                v.values.len(),
                v.values
                    .keys()
                    .take(4)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Outcome::NotRun { why, .. } => println!("{id:<3} not run: {why}"),
            _ => println!("{id:<3} an outcome this build does not print"),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §11  Export and import
// ---------------------------------------------------------------------------------------------

/// Export cuts a selection into files with an index; `verify` checks P1 (what an import retains
/// is what was exported) and P3 (a re-export is byte-identical); import brings it into a
/// second library, text and all.
fn export(w: &World) -> Result<()> {
    section(11, "Export, verify, import");
    use smysl::lib::{
        export_library, import, retained, snapshot, verify_collection, write_collection, Closure,
        ExportOptions, ImportOptions, Input as ImportInput, Layout, Selection, With, INDEX_FILE,
    };
    let snap = snapshot(&w.lib, &With::new())?;
    let sel = Selection::all(&snap);
    let opts = ExportOptions::new()
        .layout(Layout::ByExpression)
        .closure(Closure::Full)
        .with(With::new().text(true));
    let c = export_library(&w.lib, &snap, &sel, &opts)?;
    let v = verify_collection(&c);
    println!(
        "{} file(s), {} record(s); P1 {}  P3 {}",
        c.files.len(),
        c.records().len(),
        v.p1.holds(),
        v.p3.holds()
    );

    let out = scratch("export")?;
    write_collection(&c, &out)?;
    let mut to = Library::create(&scratch("imported")?)?;
    let r = import(
        &mut to,
        &[ImportInput::Index(out.join(INDEX_FILE))],
        &ImportOptions::new().accept(true),
    )?;
    for f in &r.files {
        println!(
            "  {:<24} {} added, {} duplicate(s), {} accepted, {} waiting, {} quarantined, {} text object(s)",
            f.file, f.added, f.duplicates, f.accepted, f.waiting, f.quarantined, f.text
        );
    }
    let kept = retained(&to, &r.name)?;
    println!(
        "imported as `{}` into a second library: {} record(s) retained",
        r.name,
        kept.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §12  Redaction
// ---------------------------------------------------------------------------------------------

/// A redaction removes a part's bytes and readings from the library and records that it did.
/// The claims drawn from it stay: a redaction is not a retraction.
fn redaction(w: &mut World) -> Result<()> {
    section(12, "Redaction");
    let tid = w
        .lib
        .store()
        .manifests()
        .find(|(_, m)| m.alias.as_str() == "draft")
        .map(|(_, m)| m.parts[0].tid)
        .ok_or("the draft")?;
    let r = w.lib.redact(&tid, &agent(), at(30), None)?;
    println!(
        "redacted {}: {} object(s) removed, named by {} manifest(s); held now: {}",
        tid.short(),
        r.objects_removed,
        r.manifests,
        w.lib.part(&tid).is_ok()
    );
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §13  check
// ---------------------------------------------------------------------------------------------

/// `check` runs every pass — integrity, epistemics, time, the library's own — over the store.
fn check(w: &World) -> Result<()> {
    section(13, "check");
    let report = smysl::check(w.lib.store(), smysl::CheckOptions::default());
    let errors = report
        .diagnostics
        .iter()
        .filter(|d| d.severity == smysl::Severity::Error)
        .count();
    println!(
        "{} diagnostic(s), {} error(s)",
        report.diagnostics.len(),
        errors
    );
    for d in &report.diagnostics {
        println!("  {} {}", d.code.as_str(), d.message);
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// §14  The on-disk unit store
// ---------------------------------------------------------------------------------------------

/// SMYSL-2.8's unit store keeps cores in the log until asked for, behind the same `StoreRead`
/// every layer above reads. Built with `--features store-redb`.
#[cfg(feature = "store-redb")]
fn scale(w: &World) -> Result<()> {
    section(14, "The on-disk unit store");
    let log = scratch("redb")?.join("units.smy");
    let records: Vec<Record> = w
        .lib
        .store()
        .units()
        .map(|(_, u)| Record::Unit(u.core.clone()))
        .collect();
    std::fs::write(&log, b"")?;
    smysl::Store::open(&log)?.append(&records)?;
    let (store, report) = smysl::redb::RedbStore::open(&log, smysl::redb::RedbOptions::default())?;
    println!(
        "{} unit(s) from a {}-byte log; index rebuilt: {}",
        store.len(),
        store.log_len(),
        report.index_rebuilt
    );
    for d in &report.report.diagnostics {
        println!("  {} {}", d.code.as_str(), d.message);
    }
    Ok(())
}

#[cfg(not(feature = "store-redb"))]
fn scale(_: &World) -> Result<()> {
    section(14, "The on-disk unit store");
    println!("built without `store-redb`; run with `--features store-redb` to see it");
    Ok(())
}
