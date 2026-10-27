//! **GE-T12**: does the natural-language front end write the query a person would have?
//! (SMYSL-2.5 §5.6, TX-P10 step 5.)
//!
//! ```text
//! cargo test --release --test ge_t12 -- --ignored --nocapture        # local qwen2.5:14b-instruct
//! SMYSL_GE_T12_PROVIDER=deepseek cargo test --release --test ge_t12 -- --ignored --nocapture
//! SMYSL_GE_T12_MODEL=<name>   another model    SMYSL_GE_T12_SET=<file.tsv>   another set
//! ```
//!
//! **The metric is equivalent results, not equal text** (§5.6): a translation is right when it
//! is accepted (`smysl::query::accept`, the check `q --from-question` makes) and its answer is
//! the gold query's — the same set of uids, or the same number for a `count`. `kind = claim and
//! status = inferred` and `status = inferred and kind = claim` are one answer, and a test that
//! compared strings would mark one of them wrong.
//!
//! **The bar is §5.6's: below 70% the front end stays experimental**, which `q --from-question`
//! then says on every run.
//!
//! **What the set is.** `fixtures/nl/ge-t12/questions.tsv`: a hundred questions, twenty over
//! each of `fixtures/corpus` F1–F5 — kinds, statuses, term filters, negation, traversals both
//! ways, closures, payload fields and counts — each with the query that answers it, written by
//! one hand before any model saw them. That hand is the set's limit, as for GE-T11's planted
//! contradictions: §5.6 asks for expert-written queries and this repository has one author, so
//! the difficulty is the difficulty one person chose. Several questions say "include the …"
//! because an `expand` adds to the set it was given, and a question that did not say so would
//! be marking a model wrong for a reading of English, not of the store.
//!
//! The first test runs on every `make test` and needs no model: every gold query passes the
//! same check a translation must, and answers with something. The second is the measurement,
//! ignored by default because it calls a model a hundred times.

#![cfg(feature = "cli")]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl::query::{accept, run, Answer, Env, PlanOptions, StoreScope};
use smysl::Store;

const BAR: f64 = 0.70;

struct Item {
    id: String,
    store: String,
    question: String,
    gold: String,
}

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn items() -> Vec<Item> {
    let path = std::env::var_os("SMYSL_GE_T12_SET")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo("fixtures/nl/ge-t12/questions.tsv"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(f.len(), 4, "id, store, question, gold: {l}");
            Item {
                id: f[0].into(),
                store: f[1].into(),
                question: f[2].into(),
                gold: f[3].into(),
            }
        })
        .collect()
}

fn store(name: &str) -> Store {
    let src = std::fs::read_to_string(repo(&format!("fixtures/corpus/{name}.smy"))).unwrap();
    Store::from_records(smysl::parse_surface(&src).unwrap().records)
}

/// An answer, as the metric compares it.
#[derive(Debug, PartialEq, Eq)]
enum Result_ {
    Count(usize),
    Units(BTreeSet<smysl::Uid>),
}

fn answer_of(text: &str, env: &Env<'_>) -> Result<Result_, String> {
    let opts = PlanOptions::default();
    let a = accept(text, env, &opts).map_err(|d| format!("{}: {}", d.code.as_str(), d.message))?;
    let r = run(&a.plan, env, &opts)
        .map_err(|d| format!("{}: {}", d[0].code.as_str(), d[0].message))?;
    Ok(match r.answer {
        Answer::Count(n) => Result_::Count(n),
        other => Result_::Units(other.uids().into_iter().collect()),
    })
}

#[test]
fn every_gold_query_is_accepted_and_answers() {
    let set = items();
    assert_eq!(set.len(), 100, "§5.6 asks for a hundred");
    let ids: BTreeSet<&str> = set.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids.len(), 100, "ids are unique");
    for i in &set {
        let s = store(&i.store);
        let scope = StoreScope::new(i.store.clone(), &s);
        let env = Env::new(&s, &scope);
        match answer_of(&i.gold, &env) {
            Ok(Result_::Count(n)) => assert!(n > 0, "{}: counts nothing", i.id),
            Ok(Result_::Units(u)) => assert!(!u.is_empty(), "{}: answers nothing", i.id),
            Err(e) => panic!("{}: the gold query is refused: {e}", i.id),
        }
    }
}

#[cfg(feature = "local")]
fn provider() -> Box<dyn smysl::Provider> {
    let which = std::env::var("SMYSL_GE_T12_PROVIDER").unwrap_or_else(|_| "ollama".into());
    let (kind, endpoint, model, key) = match which.as_str() {
        "ollama" => (
            "ollama",
            "http://127.0.0.1:11434",
            "qwen2.5:14b-instruct",
            None,
        ),
        "deepseek" => (
            "deepseek",
            "https://api.deepseek.com",
            "deepseek-chat",
            Some("DEEPSEEK_API_KEY"),
        ),
        "gemini" => (
            "gemini",
            "https://generativelanguage.googleapis.com",
            "gemini-2.0-flash",
            Some("GEMINI_API_KEY"),
        ),
        other => panic!("SMYSL_GE_T12_PROVIDER={other}: ollama, deepseek or gemini"),
    };
    let mut cfg = smysl::ProviderConfig::new(smysl::ProviderId::new(kind).unwrap(), kind);
    cfg.endpoint = endpoint.into();
    cfg.model = std::env::var("SMYSL_GE_T12_MODEL")
        .ok()
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| model.into());
    cfg.context_window = 32_768;
    cfg.max_output = 512;
    cfg.timeout_secs = 300;
    cfg.api_key_env = key.map(str::to_string);
    smysl::build_provider(&cfg).unwrap_or_else(|e| panic!("{kind}: {e}"))
}

#[cfg(feature = "local")]
#[test]
#[ignore = "calls a model a hundred times; run with --ignored"]
fn ge_t12_equivalent_results() {
    use smysl::query::{TranslateContext, Translator};

    let p = provider();
    let t = smysl::translate::ProviderTranslator::new(&*p, "");
    let set = items();
    let started = std::time::Instant::now();
    let mut right = 0usize;
    let mut refused = 0usize;
    let mut per: std::collections::BTreeMap<String, (usize, usize)> = Default::default();
    for i in &set {
        let s = store(&i.store);
        let scope = StoreScope::new(i.store.clone(), &s);
        let env = Env::new(&s, &scope);
        let gold = answer_of(&i.gold, &env).expect("the gold set is checked by the other test");
        let ctx = TranslateContext::of(&env);
        let text = t
            .translate(&i.question, &ctx)
            .unwrap_or_else(|e| panic!("{}: {e}", i.id));
        let got = answer_of(&text, &env);
        let ok = got.as_ref() == Ok(&gold);
        let e = per.entry(i.store.clone()).or_default();
        e.1 += 1;
        if ok {
            right += 1;
            e.0 += 1;
        } else {
            if got.is_err() {
                refused += 1;
            }
            let why = match &got {
                Err(e) => e.chars().take(140).collect::<String>(),
                Ok(_) => "a different answer".into(),
            };
            println!(
                "miss {}: {}\n  gold: {}\n  got:  {}\n  ({why})",
                i.id,
                i.question,
                i.gold,
                text.trim().replace('\n', " | ")
            );
        }
    }
    let rate = right as f64 / set.len() as f64;
    println!(
        "\nGE-T12, {} ({}), {} questions in {:.0} s",
        p.id().as_str(),
        t.model(),
        set.len(),
        started.elapsed().as_secs_f64()
    );
    for (store, (r, n)) in &per {
        println!("  {store:<14} {r:>3}/{n}");
    }
    println!(
        "equivalent results {right}/{} = {rate:.2}; refused (SMY-E474) {refused}; bar {BAR:.2} — {}",
        set.len(),
        if rate >= BAR { "met" } else { "the front end stays experimental" }
    );
}
