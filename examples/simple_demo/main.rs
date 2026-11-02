//! **A first look at smysl**: the format and the library, without the text-corpus layer.
//!
//! ```text
//! cargo run --example simple_demo
//! ```
//!
//! One short document, written in smysl's surface syntax, about why a CI build got slow. The
//! program takes it through the basic operations, one numbered step each:
//!
//! 1. parse the surface text into records;
//! 2. identity: a unit's uid is a hash of its content, so the same claim is the same unit;
//! 3. CBOR, the wire form, round-trips byte for byte;
//! 4. a store, and `check` over it, with one broken document for contrast;
//! 5. sq/1 queries: filters, a walk over grounds, rebuttals;
//! 6. salience: which units matter most to the document's question;
//! 7. packing the document into a token budget, each unit at its own level of detail;
//! 8. rendering the brief thread as Markdown;
//! 9. withdrawing an edge, and what changes;
//! 10. `fmt`: writing canonical surface text back, which parses to the same records.
//!
//! Everything here is pure: no network, no clock, no files. Run it twice and every uid is the same.

use smysl::query::{self as sq, Env, PlanOptions, StoreScope};
use smysl::{
    canonical_uid, check, emit_artifact, from_cbor_seq, pack, parse_surface, salience, to_cbor_seq,
    write_surface, BuildOptions, CheckOptions, KernelType, PackRequest, Profile, Record,
    SalienceRequest, Status, Store, Target, Uid, UnitCoreBuilder, WriteContext,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

/// The document. Evidence first, then what is claimed from it, then how the claims relate, then
/// a thread that tells it as a brief: bottom line, support, risk.
const DOC: &str = r#"@doc smysl/0.1 {
  id: v/slow-ci
  intent: incident-brief
  lang: en
  roots: [q/why-slow]
}

@question q/why-slow { status: speculative }
~ Why did the main branch's CI time double this week?

@evidence e/timings { status: measured, source: { kind: metric, ref: "ci://pipelines/main/duration", captured: 2026-10-30 } }
~ Median pipeline time rose from 11 to 23 minutes on 2026-10-27.

@evidence e/cache-misses { status: measured, source: { kind: file, ref: "ci://jobs/build/cache.log", captured: 2026-10-30 } }
~ The build job's dependency cache missed on every run since 2026-10-27.

@evidence e/lockfile { status: measured, source: { kind: doc, ref: "git://main/Cargo.lock@a1b2c3" } }
~ A lockfile change on 2026-10-27 bumped forty transitive dependencies.

@evidence e/runner { status: measured, source: { kind: metric, ref: "ci://runners/load", captured: 2026-10-30 } }
~ Runner queue time stayed under one minute all week.

@claim c/cache-cold { status: derived, grounds: [e/cache-misses, e/lockfile] }
~ The cache key changed with the lockfile, so every build compiles from scratch.

@claim c/runners-busy { status: speculative }
~ The runners are overloaded.

@finding f/cause { status: inferred, grounds: [c/cache-cold, e/timings] }
~ A cold dependency cache, after the lockfile bump, doubled the build time.

@rel c/cache-cold --causes--> e/timings
@rel e/runner --rebuts--> c/runners-busy
@rel f/cause --answers--> q/why-slow

@thread t/brief { schema: brief, owner: "human:demo" }
~ CI doubled because the lockfile bump made the dependency cache cold.
  bottom-line → f/cause
  support → c/cache-cold
  risk → c/runners-busy
"#;

fn step(n: u32, title: &str) {
    println!("\n{n}. {title}\n{}", "─".repeat(title.len() + 3));
}

fn main() -> Result<()> {
    // 1. Parse ------------------------------------------------------------------------------
    step(1, "Parse the surface text");
    let parsed = parse_surface(DOC)?;
    for d in &parsed.diagnostics {
        println!("  {} {}", d.code.as_str(), d.message);
    }
    let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
    for r in &parsed.records {
        *kinds.entry(r.type_name()).or_default() += 1;
    }
    println!("{} records: {kinds:?}", parsed.records.len());
    println!("{} labels, for example:", parsed.labels.len());
    for (label, uid) in parsed.labels.iter().take(3) {
        println!("  {:<16} {}", label.as_str(), uid.canonical());
    }
    let label = |l: &str| -> Uid {
        parsed
            .labels
            .iter()
            .find(|(k, _)| k.as_str() == l)
            .map(|(_, u)| *u)
            .expect("a label the document declares")
    };

    // 2. Identity ---------------------------------------------------------------------------
    step(2, "Identity is content");
    // The same claim, built in code rather than parsed, is the same unit: a uid is BLAKE3 over
    // the canonical CBOR of what the unit says, not over where or when it was written.
    let same = UnitCoreBuilder::new(
        KernelType::Claim,
        "The runners are overloaded.",
        Status::Speculative,
    )
    .build()?;
    println!(
        "built in code:  {}\nparsed:         {}",
        canonical_uid(&same).canonical(),
        label("c/runners-busy").canonical()
    );
    // Change anything it says, even one word, and it is a different unit.
    let other = UnitCoreBuilder::new(
        KernelType::Claim,
        "The runners are overloaded today.",
        Status::Speculative,
    )
    .build()?;
    println!("one word more:  {}", canonical_uid(&other).canonical());
    // And a status has to be earned: `inferred` without grounds is refused at construction.
    let refused = UnitCoreBuilder::new(
        KernelType::Claim,
        "The runners are overloaded.",
        Status::Inferred,
    )
    .build();
    println!("as `inferred` with no grounds: {:?}", refused.err());

    // 3. CBOR ------------------------------------------------------------------------------
    step(3, "CBOR, the wire form");
    let bytes = to_cbor_seq(&parsed.records);
    // The second value is where decoding stopped: a log may be read while a writer appends.
    let (back, _unknown) = from_cbor_seq(&bytes)?;
    println!(
        "{} records in {} bytes of CBOR ({} bytes of surface text); decoded back equal: {}",
        parsed.records.len(),
        bytes.len(),
        DOC.len(),
        back == parsed.records
    );

    // 4. Store and check ---------------------------------------------------------------------
    step(4, "A store, and check");
    let mut store = Store::from_records(parsed.records.clone());
    let report = check(&store, CheckOptions::default());
    println!(
        "{} units, {} relations; check: {} diagnostic(s)",
        store.units().count(),
        store.relations().count(),
        report.diagnostics.len()
    );
    for d in &report.diagnostics {
        println!("  {} {}", d.code.as_str(), d.message);
    }
    // For contrast: a finding grounded on a label nothing declares, and a `measured` claim
    // with no source to have measured it from.
    let broken =
        "@finding f/x { status: inferred, grounds: [e/missing] }\n~ Something follows.\n\n\
                  @claim c/y { status: measured }\n~ A measurement from nowhere.\n";
    let parsed_broken = parse_surface(broken)?;
    println!("a broken document:");
    for d in &parsed_broken.diagnostics {
        println!("  {} {}", d.code.as_str(), d.message);
    }

    // 5. Queries ---------------------------------------------------------------------------
    step(5, "sq/1 queries");
    let gist = |store: &Store, u: &Uid| {
        store
            .get(u)
            .map(|x| x.core.gist.clone())
            .unwrap_or_default()
    };
    let run = |store: &Store, query: &str| -> Result<()> {
        let scope = StoreScope::new("demo", store);
        let env = Env::new(store, &scope);
        let opts = PlanOptions::default();
        let q = sq::parse(query).map_err(|d| d.message)?;
        let plan = sq::plan(&q.query, &env, &opts).map_err(|d| d[0].message.clone())?;
        let result = sq::run(&plan, &env, &opts).map_err(|d| d[0].message.clone())?;
        println!("» {}", query.replace('\n', " │ "));
        match &result.answer {
            sq::Answer::Count(n) => println!("  {n}"),
            other => {
                for u in other.uids() {
                    println!("  {}", gist(store, &u));
                }
            }
        }
        Ok(())
    };
    // Every claim or finding at `derived` or firmer.
    run(
        &store,
        "from store:demo\nwhere (kind = claim or kind = finding) and status >= derived\nunits",
    )?;
    // What the finding rests on, two steps down its grounds.
    run(
        &store,
        "from store:demo\nwhere kind = finding\nexpand grounds depth 2 only\nunits",
    )?;
    // What rebuts the speculative claim. An arrow in `expand` points the way a unit *rests on*
    // others, as grounds do, so `->rebuts` from a claim reaches what rebuts it.
    run(
        &store,
        "from store:demo\nwhere status = speculative and kind = claim\nexpand ->rebuts only\nunits",
    )?;
    // Lexical search over the gists.
    run(&store, "from store:demo\nfind \"cache\"\nunits")?;

    // 6. Salience ---------------------------------------------------------------------------
    step(6, "Salience");
    // Personalised PageRank over the graph, seeded at the document's finding: what the answer
    // rests on scores highest, and the speculation it does not rest on scores least.
    let scores = salience(
        &store,
        &SalienceRequest::default().seeded([label("f/cause")]),
    );
    let mut ranked: Vec<(&Uid, &f32)> = scores.scores.iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(a.1));
    for (uid, s) in ranked.iter().take(5) {
        println!("  {s:.3}  {}", gist(&store, uid));
    }

    // 7. Pack ------------------------------------------------------------------------------
    step(7, "Pack into a token budget");
    // The packer picks units and a level of detail for each so that the whole fits, keeping
    // what the selection needs to stay honest (a claim's grounds come with it).
    for budget in [40, 120] {
        match pack(&store, &scores, &PackRequest::budget(budget)) {
            Ok(p) => {
                println!("budget {budget}: {} unit(s)", p.selection.len());
                for (uid, lod) in &p.selection {
                    println!("  {lod:<8?} {}", gist(&store, uid));
                }
            }
            Err(e) => println!("budget {budget}: {e}"),
        }
    }

    // 8. Render ----------------------------------------------------------------------------
    step(8, "Render the brief as Markdown");
    let thread = parsed
        .records
        .iter()
        .find_map(|r| match r {
            Record::Thread(t) => Some(t.clone()),
            _ => None,
        })
        .ok_or("the document's thread")?;
    let profile = Profile::builtin("plain").ok_or("the plain profile")?;
    let ir = smysl::build_ir(&store, &thread, &profile, &BuildOptions::default());
    let md = emit_artifact(Target::Markdown, &ir, &profile)?;
    println!("{}", md.text.trim_end());

    // 9. Withdraw --------------------------------------------------------------------------
    step(9, "Withdraw an edge");
    // A withdrawal is a record of its own: the edge stays in the log, and is no longer live.
    // Its endpoints are labels of the first document, which a second document cannot see, so
    // they are written as uids — as `smysl withdraw` does when it appends to a store.
    let text = format!(
        "@withdraw {} --rebuts--> {} {{ agent: human:demo, ts: [1761800000000, 0] }}",
        label("e/runner").canonical(),
        label("c/runners-busy").canonical()
    );
    let w = parse_surface(&text)?;
    store.append(&w.records)?;
    run(
        &store,
        "from store:demo\nwhere status = speculative and kind = claim\nexpand ->rebuts only\nunits",
    )?;
    println!("(nothing rebuts it now: the edge was withdrawn)");

    // 10. fmt ------------------------------------------------------------------------------
    step(10, "Canonical surface text");
    // The labels come back as authored, and the header keeps the version the document declared.
    let ctx = WriteContext::from_labels(&parsed.labels)
        .with_format_version(parsed.format_version.clone());
    let view = parsed.records.iter().find_map(|r| match r {
        Record::View(v) => Some(v),
        _ => None,
    });
    let canonical = write_surface(view, &parsed.records, &ctx);
    let again = parse_surface(&canonical)?;
    println!(
        "{} bytes written; parses back to the same records: {}",
        canonical.len(),
        again.records == parsed.records
    );
    println!(
        "{}",
        canonical.lines().take(6).collect::<Vec<_>>().join("\n")
    );
    Ok(())
}
