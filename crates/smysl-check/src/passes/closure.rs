//! Pass 4 - rule L closure (§17).
//!
//! Rule L says a gist is interpretable from the L0 of its `deps` alone, and a body from
//! the L0 of its `deps` and `grounds`. Semantic closure needs a model and is the optional
//! `attest` operation; what `check` verifies is **structural** closure - that a body does
//! not reach for a unit it never declared a relationship with.
//!
//! Both findings here are deliberately conservative. `SMY-E020` fires only on an explicit
//! reference, and `SMY-W024` is a warning that names itself as a heuristic, because an
//! error resting on a guess about English would be worse than no check at all.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::diag::{Code, Diagnostic, Report};
use smysl_core::{Label, Uid, UidPrefix, UnitCore};
use smysl_graph::Store;

/// Anaphora with no antecedent inside the gist. A gist that opens with one of these is
/// reaching for something the reader has not been given yet.
const DANGLING_OPENERS: &[&str] = &[
    "this ",
    "that ",
    "these ",
    "those ",
    "it ",
    "its ",
    "they ",
    "them ",
    "such ",
    "the same ",
    "the above",
    "the former",
    "the latter",
    "as described",
    "as noted",
    "see below",
];

pub fn run(store: &Store, labels: &BTreeMap<Label, Uid>, report: &mut Report) {
    for (uid, unit) in store.units() {
        let declared: BTreeSet<Uid> = unit.core.references().copied().collect();
        body_closure(uid, &unit.core, &declared, labels, store, report);
        gist_self_containment(uid, &unit.core, report);
    }
}

/// `SMY-E020` - a body references a uid absent from `deps` and `grounds`.
///
/// References are recognised in two spellings: a canonical `b3:` uid, and a label the
/// caller supplied a binding for. Prose that merely *mentions* a unit in words is not a
/// reference and is not reported - that is what `attest` is for.
fn body_closure(
    uid: &Uid,
    core: &UnitCore,
    declared: &BTreeSet<Uid>,
    labels: &BTreeMap<Label, Uid>,
    store: &Store,
    report: &mut Report,
) {
    let Some(body) = &core.body else { return };
    let mut offenders: BTreeSet<Uid> = BTreeSet::new();

    let mut ambiguous: BTreeSet<String> = BTreeSet::new();
    for token in tokenise(body) {
        // A uid in prose is usually the 26-character display form, so resolve it as a
        // prefix. Ambiguity is reported, never guessed at (§1.2).
        let referenced = if token.starts_with("b3:") {
            match UidPrefix::parse(token) {
                Ok(p) => match store.resolve_prefix(&p) {
                    Ok(u) => Some(u),
                    Err(_) if !store.matching_prefix(&p).is_empty() => {
                        ambiguous.insert(token.to_string());
                        None
                    }
                    Err(_) => None,
                },
                Err(_) => None,
            }
        } else {
            Label::new(token).ok().and_then(|l| labels.get(&l).copied())
        };
        if let Some(r) = referenced {
            if r != *uid && !declared.contains(&r) {
                offenders.insert(r);
            }
        }
    }

    for token in ambiguous {
        report.push(
            Diagnostic::on(Code::E072, *uid)
                .with_message(format!(
                    "the body reference `{token}` matches more than one unit"
                ))
                .with_suggestion("use the full 52-character form"),
        );
    }

    for o in offenders {
        report.push(
            Diagnostic::on(Code::E020, *uid)
                .with_message(format!(
                    "the body references {o}, which is neither a dep nor a ground"
                ))
                .with_suggestion(
                    "add it to `deps` if it is a prerequisite, or `grounds` if it is support",
                ),
        );
    }
}

/// `SMY-W024` - the gist appears to depend on the body.
///
/// A warning, and it says so: this is a word-list heuristic over English, and the way to
/// settle it is `attest --what gist-coverage`, which has a model to hand.
fn gist_self_containment(uid: &Uid, core: &UnitCore, report: &mut Report) {
    if core.body.is_none() {
        return;
    }
    let lower = format!("{} ", core.gist.trim().to_lowercase());
    let hit = DANGLING_OPENERS.iter().find(|o| lower.starts_with(**o));
    if let Some(o) = hit {
        report.push(
            Diagnostic::on(Code::W024, *uid)
                .with_message(format!(
                    "the gist opens with `{}`, which has no antecedent at L0",
                    o.trim()
                ))
                .with_suggestion("confirm with `attest --what gist-coverage`"),
        );
    }
}

/// Split prose into candidate reference tokens: `b3:` uids and `a/b` labels.
fn tokenise(s: &str) -> impl Iterator<Item = &str> {
    s.split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, ':' | '/' | '-' | '_' | '.')))
        .filter(|t| t.contains('/') || t.starts_with("b3:"))
        .map(|t| t.trim_matches(|c| matches!(c, '.' | '-' | '_')))
        .filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{canonical_uid, KernelType, Record, Status, UnitCoreBuilder};

    fn store_of(cores: Vec<UnitCore>) -> Store {
        Store::from_records(cores.into_iter().map(Record::Unit).collect())
    }

    fn check(cores: Vec<UnitCore>, labels: BTreeMap<Label, Uid>) -> Report {
        let store = store_of(cores);
        let mut r = Report::new();
        run(&store, &labels, &mut r);
        r
    }

    fn claim(gist: &str, body: Option<&str>, deps: Vec<Uid>) -> UnitCore {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative).deps(deps);
        if let Some(t) = body {
            b = b.body(t);
        }
        b.build().unwrap()
    }

    #[test]
    fn a_gist_only_unit_has_nothing_to_close_over() {
        assert!(check(vec![claim("a claim", None, vec![])], BTreeMap::new()).is_empty());
    }

    #[test]
    fn a_body_referencing_a_declared_dep_is_closed() {
        let dep = claim("the definition", None, vec![]);
        let u = canonical_uid(&dep);
        let c = claim(
            "a claim",
            Some(&format!("As {u} says, p95 tripled.")),
            vec![u],
        );
        assert!(check(vec![dep, c], BTreeMap::new()).is_empty());
    }

    #[test]
    fn a_body_referencing_an_undeclared_unit_is_e020() {
        let other = claim("something else", None, vec![]);
        let u = canonical_uid(&other);
        let c = claim("a claim", Some(&format!("See {u} for the rest.")), vec![]);
        let r = check(vec![other, c], BTreeMap::new());
        assert_eq!(r.count(Code::E020), 1);
        assert!(r.iter().next().unwrap().suggestion.is_some());
    }

    #[test]
    fn a_reference_by_label_is_recognised_when_a_binding_is_supplied() {
        let other = claim("something else", None, vec![]);
        let u = canonical_uid(&other);
        let c = claim("a claim", Some("See d/p95 for the definition."), vec![]);
        let labels = BTreeMap::from([(Label::new("d/p95").unwrap(), u)]);
        assert_eq!(check(vec![other, c], labels).count(Code::E020), 1);
    }

    /// Without a binding a label is just prose. The pass must not invent references.
    #[test]
    fn a_label_with_no_binding_is_not_a_reference() {
        let c = claim("a claim", Some("See d/p95 for the definition."), vec![]);
        assert!(check(vec![c], BTreeMap::new()).is_empty());
    }

    /// A uid that names nothing in this store is pass 2's business, not pass 4's.
    #[test]
    fn a_reference_to_an_absent_unit_is_not_reported_here() {
        let ghost = Uid::from_bytes([9; 32]);
        let c = claim(
            "a claim",
            Some(&format!("See {}.", ghost.canonical())),
            vec![],
        );
        assert!(check(vec![c], BTreeMap::new()).is_empty());
    }

    /// The display form is what a human writes, so it has to resolve - and where it is
    /// ambiguous, say so rather than pick.
    #[test]
    fn the_display_form_resolves_as_a_prefix() {
        let other = claim("something else", None, vec![]);
        let u = canonical_uid(&other);
        let c = claim("a claim", Some(&format!("See {u} for the rest.")), vec![]);
        assert_eq!(check(vec![other, c], BTreeMap::new()).count(Code::E020), 1);
    }

    #[test]
    fn a_body_referencing_its_own_unit_is_not_a_violation() {
        let c = claim("a claim", Some("placeholder"), vec![]);
        let u = canonical_uid(&c);
        let c = claim("a claim", Some(&format!("This unit is {u}.")), vec![]);
        let _ = u;
        assert_eq!(check(vec![c], BTreeMap::new()).count(Code::E020), 0);
    }

    #[test]
    fn each_undeclared_reference_is_reported_once_however_often_it_appears() {
        let other = claim("something else", None, vec![]);
        let u = canonical_uid(&other);
        let c = claim(
            "a claim",
            Some(&format!("See {u}, and again {u}, and once more {u}.")),
            vec![],
        );
        assert_eq!(check(vec![other, c], BTreeMap::new()).count(Code::E020), 1);
    }

    #[test]
    fn a_gist_that_opens_with_an_anaphor_warns() {
        for gist in [
            "This shows the pool was saturated",
            "It follows that the pool saturated",
            "Such behaviour indicates saturation",
            "The above implies saturation",
        ] {
            let r = check(vec![claim(gist, Some("body"), vec![])], BTreeMap::new());
            assert_eq!(r.count(Code::W024), 1, "{gist}");
            assert!(r.is_clean(), "a heuristic must not fail a check");
        }
    }

    #[test]
    fn a_self_contained_gist_does_not_warn() {
        for gist in [
            "p95 auth latency tripled after the 4.2 rollout",
            "Thistle counts are unaffected",
            "Items in the queue doubled",
        ] {
            let r = check(vec![claim(gist, Some("body"), vec![])], BTreeMap::new());
            assert_eq!(r.count(Code::W024), 0, "{gist}");
        }
    }

    /// A gist-only unit has no body to depend on, so the heuristic must stay quiet.
    #[test]
    fn the_heuristic_does_not_fire_without_a_body() {
        let r = check(
            vec![claim("This shows saturation", None, vec![])],
            BTreeMap::new(),
        );
        assert!(r.is_empty());
    }

    #[test]
    fn tokenisation_finds_uids_and_labels_and_ignores_prose() {
        let u = Uid::from_bytes([1; 32]).canonical();
        let text = format!("Prose about {u} and d/p95, but not plainword or 10/4 ratios.");
        let found: Vec<&str> = tokenise(&text).collect();
        assert!(found.contains(&u.as_str()));
        assert!(found.contains(&"d/p95"));
        assert!(!found.contains(&"plainword"));
    }
}
