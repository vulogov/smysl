//! What rule 26 costs as a subject is mentioned more (1.12.0): a narrowed slot's forms are found
//! by stemming every word of every unit about the subject, which is nothing over GE-T10's
//! corpus and is what a popular subject in a large library would make it pay. One subject,
//! mentioned by N claims of twenty-odd words each from a vocabulary that grows with N, and
//! `ask` timed on a verification through the production port (stems) and through the plain
//! one (no stems, so no forms), on a question both narrow — the difference is the rule's.
//!
//! ```text
//! cargo test --release -p smysl-text --features stem --test narrow_scale -- --ignored --nocapture
//! SMYSL_NARROW_SIZES=1000,10000 cargo test …
//! ```
//!
//! A measurement, not a gate.

#![cfg(feature = "stem")]

use std::fmt::Write as _;
use std::time::Instant;

use smysl_core::ids::AgentId;
use smysl_core::surface::parse_surface;
use smysl_graph::Store;
use smysl_query::ports::Entities;
use smysl_query::{ask, AskOptions, Env, StoreScope};
use smysl_text::scope::Stemmed;

const WORDS: &[&str] = &[
    "sailors",
    "voyage",
    "lemons",
    "oranges",
    "cured",
    "curing",
    "surgeon",
    "wrote",
    "crew",
    "ration",
    "fleet",
    "harbour",
    "winter",
    "sickness",
    "gums",
    "bleeding",
    "remedy",
    "juice",
    "fresh",
    "salted",
    "meat",
    "biscuit",
    "captain",
    "logbook",
    "weeks",
    "months",
    "island",
    "returned",
    "recovered",
    "died",
];

fn corpus(n: usize) -> String {
    let mut s = String::from(
        "@schema x.text/v1 { version: 1, types: [\"x.text/entity\"], relations: \
         [x.text/mentions, x.text/same-as] }\n\
         @x.text/entity e/scurvy { status: speculative, source: { kind: doc, ref: \"alias:en\" } }\n\
         ~ scurvy\n",
    );
    for i in 0..n {
        let mut text = format!("Scurvy struck ship{} ", i % (n / 10).max(1));
        for k in 0..20 {
            text.push_str(WORDS[(i * 7 + k * 13) % WORDS.len()]);
            text.push(' ');
        }
        // A vocabulary that grows with the store: a tenth of the claims bring a word of their own.
        let _ = write!(text, "port{}", i % (n / 3).max(1));
        let _ = writeln!(
            s,
            "@claim c/u{i} {{ status: cited, source: {{ kind: doc, ref: \"log:{i}\" }} }}\n~ {text}.\n\
             @rel c/u{i} --x.text/mentions--> e/scurvy"
        );
    }
    s
}

#[test]
#[ignore = "a measurement, not a gate"]
fn rule_26_cost_by_mentions() {
    let sizes: Vec<usize> = std::env::var("SMYSL_NARROW_SIZES")
        .unwrap_or_else(|_| "1000,10000,100000".to_string())
        .split(',')
        .map(|v| v.trim().parse().expect("a whole number"))
        .collect();
    // Its words are the store's as spelled, so both ports narrow and the difference is the
    // forms alone (`curing`), not narrowing, which a question in the store's words pays anyway.
    let question = "Is it true that lemons cured scurvy?";
    let opts = AskOptions::new(AgentId::new("tool:narrow-scale").unwrap());
    println!("rule 26 by mentions of the subject: ask, plain port vs stemming port");
    println!(
        "  {:>8} {:>10} {:>10} {:>8}  forms",
        "units", "plain ms", "stems ms", "ratio"
    );
    for n in sizes {
        let parsed = parse_surface(&corpus(n)).expect("the corpus parses");
        let store = Store::from_records(parsed.records);
        let scope = StoreScope::new("scale", &store);
        let stemmed = Stemmed::new(&store, &scope);
        let plain_env = Env::new(&store, &scope).with_entities(&scope as &dyn Entities);
        let stem_env = Env::new(&store, &scope).with_entities(&stemmed as &dyn Entities);
        // Each once to warm, then the best of three.
        let time = |env: &Env<'_>| {
            let _ = ask(question, env, &opts).expect("asked");
            (0..3)
                .map(|_| {
                    let t = Instant::now();
                    let d = ask(question, env, &opts).expect("asked");
                    (t.elapsed().as_secs_f64() * 1000.0, d.forms)
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .expect("three runs")
        };
        let (plain, _) = time(&plain_env);
        let (stems, forms) = time(&stem_env);
        assert!(
            !forms.is_empty(),
            "the stemming port found no forms: nothing was measured"
        );
        println!(
            "  {n:>8} {plain:>10.1} {stems:>10.1} {:>8.2}  [{}]",
            stems / plain,
            forms.join(" ")
        );
    }
}
