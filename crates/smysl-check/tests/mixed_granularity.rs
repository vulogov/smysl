//! A store merged from documents with different granularity profiles is judged by all of them,
//! not by whichever document's id sorts first.
//!
//! Found checking a store merged from decision rationales written under `fine` (bodies 20..60
//! tokens) and other documents written under `default` (40..120): every rationale warned
//! `SMY-W041`, "under the default range". The profile had been `store.views().next()`, and views
//! are ordered by id — renaming the `default` document from `v/other` to `v/zzz` made the
//! warnings disappear with no unit changed.

use smysl_check::{check, CheckOptions};
use smysl_core::surface::parse_surface;
use smysl_core::Code;
use smysl_graph::Store;

/// 37 tokens by the estimator `check` uses: inside `fine`, under `default`.
const RATIONALE: &str = "The team kept surface ingest because it recovers partial output, and \
                         json-ast truncation would lose whole batches when a response is cut off \
                         early.";

fn doc(id: &str, profile: Option<&str>, body: &str) -> String {
    let granularity = profile
        .map(|p| format!("  granularity: {{ profile: {p} }}\n"))
        .unwrap_or_default();
    format!(
        "@doc smysl/1.0 {{\n  id: {id}\n  intent: rationale\n  lang: en\n{granularity}}}\n\n\
         @decision d/{} {{ status: speculative }}\n~ Keep the surface path.\n{body}\n",
        id.replace("v/", "")
    )
}

fn store_of(docs: &[String]) -> Store {
    let mut records = Vec::new();
    for d in docs {
        records.extend(parse_surface(d).unwrap().records);
    }
    Store::from_records(records)
}

fn w041(store: &Store) -> usize {
    check(store, CheckOptions::strict()).count(Code::W041)
}

#[test]
fn a_mixed_store_does_not_judge_units_by_the_first_view_id() {
    // Both orders of id. The bug was the order; a fix that only held for one would be luck.
    for other in ["v/aaa", "v/zzz"] {
        let store = store_of(&[
            doc("v/rationale", Some("fine"), RATIONALE),
            doc(other, None, "A second decision, with a body long enough for either profile to accept without complaint, running on past forty tokens so that nothing about it is borderline under the default range or the fine one."),
        ]);
        assert_eq!(
            w041(&store),
            0,
            "with the default document named {other}, a fine-profile rationale still warned"
        );
    }
}

#[test]
fn a_body_outside_every_profile_in_the_store_still_warns() {
    // The control. Widening to the envelope must not become "never warn": a body shorter than
    // any profile present is too short under all of them.
    for other in ["v/aaa", "v/zzz"] {
        let store = store_of(&[
            doc("v/rationale", Some("fine"), "Too short."),
            doc(other, None, RATIONALE),
        ]);
        let r = check(&store, CheckOptions::strict());
        let msgs: Vec<String> = r
            .iter()
            .filter(|d| d.code == Code::W041)
            .map(|d| d.message.clone())
            .collect();
        assert!(
            msgs.iter().any(|m| m.contains("widest of default, fine")),
            "{other}: the short body did not warn against the envelope: {msgs:?}"
        );
    }
}

#[test]
fn a_store_whose_views_agree_is_checked_exactly_as_before() {
    let store = store_of(&[
        doc("v/a", None, RATIONALE),
        doc("v/b", None, "Another short one, also under forty."),
    ]);
    let r = check(&store, CheckOptions::strict());
    assert!(
        r.count(Code::W041) >= 1,
        "a default-only store stopped warning"
    );
    assert!(
        r.iter()
            .filter(|d| d.code == Code::W041)
            .all(|d| d.message.contains("default range 40..120")),
        "the message named something other than the one profile present"
    );
}
