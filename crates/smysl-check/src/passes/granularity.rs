//! Pass 5 - granularity (§17).
//!
//! Granularity constrains *production*, not the store: mixed granularity in a merged
//! store is legal (D-5), and this pass reports against whichever profile the caller says
//! the units were produced under.
//!
//! `SMY-E040` is an error, so it is decided **structurally** rather than semantically. A
//! body that is two paragraphs, or a list of two or more items, is more than one assertion
//! by its own layout; a single paragraph never trips it however long it is. Deciding
//! "multi-assertion" by reading the prose would need a model, and an error that rests on a
//! guess is worse than no error at all.

use smysl_core::diag::{Code, Diagnostic, Report};
use smysl_core::{tokens, Admission, GranularityProfile, Uid, UnitCore};
use smysl_graph::Store;

pub fn run(store: &Store, granularity: &GranularityProfile, report: &mut Report) {
    for (uid, unit) in store.units() {
        check_unit(uid, &unit.core, granularity, report);
    }
}

pub fn check_unit(
    uid: &Uid,
    core: &UnitCore,
    granularity: &GranularityProfile,
    report: &mut Report,
) {
    let Some(body) = &core.body else { return };

    // `prose` is the kernel type for opaque unstructured text, so requiring it to be a
    // single assertion is requiring it not to be prose. Rule I depends on this: an
    // unrepairable span degrades to a `prose` unit carrying the raw span verbatim, and a
    // raw span is very often several paragraphs. Without the exemption the two rules
    // contradict each other and ingest cannot both make progress and stay conformant.
    let opaque = core.schema.kernel() == Some(smysl_core::KernelType::Prose);

    // `SMY-E040` - structurally more than one assertion.
    if granularity.admission == Admission::SingleAssertion && !opaque {
        if let Some(reason) = multi_assertion(body) {
            report.push(
                Diagnostic::on(Code::E040, *uid)
                    .with_message(format!("{reason} under single-assertion admission"))
                    .with_suggestion("split into one unit per assertion, or move to `coarse`"),
            );
        }
    }

    // `SMY-W041` - outside the profile's body range.
    let n = tokens(body);
    if !granularity.body_in_range(n) {
        let direction = if n < granularity.l1_min {
            "under"
        } else {
            "over"
        };
        report.push(Diagnostic::on(Code::W041, *uid).with_message(format!(
            "body is {n} tokens, {direction} the {} range {}..{}",
            granularity.profile, granularity.l1_min, granularity.l1_max
        )));
    }
}

/// Whether a body is structurally more than one assertion, and why.
fn multi_assertion(body: &str) -> Option<&'static str> {
    if paragraphs(body) > 1 {
        return Some("body has more than one paragraph");
    }
    if list_items(body) > 1 {
        return Some("body is a list of more than one item");
    }
    None
}

fn paragraphs(body: &str) -> usize {
    body.split("\n\n").filter(|p| !p.trim().is_empty()).count()
}

fn list_items(body: &str) -> usize {
    body.lines()
        .filter(|l| {
            let t = l.trim_start();
            t.starts_with("- ")
                || t.starts_with("* ")
                || t.starts_with("+ ")
                || t.split_once(". ")
                    .is_some_and(|(n, _)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{canonical_uid, KernelType, Record, Status, UnitCoreBuilder};

    fn check(body: &str, g: &GranularityProfile) -> Report {
        let core = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative)
            .body(body)
            .build()
            .unwrap();
        let store = Store::from_records(vec![Record::Unit(core)]);
        let mut r = Report::new();
        run(&store, g, &mut r);
        r
    }

    /// 240 bytes -> 60 tokens, inside the default 40..120 range.
    fn in_range() -> String {
        "word ".repeat(48)
    }

    /// Rule I degrades an unrepairable span to a `prose` unit carrying the span verbatim,
    /// which is very often several paragraphs. Without this exemption rule I and
    /// single-assertion granularity contradict each other.
    #[test]
    fn a_prose_unit_is_exempt_from_single_assertion_admission() {
        let opaque = UnitCoreBuilder::new(KernelType::Prose, "an opaque span", Status::Speculative)
            .body("First paragraph.\n\nSecond paragraph.\n\nThird.")
            .build()
            .unwrap();

        let mut report = Report::new();
        check_unit(
            &canonical_uid(&opaque),
            &opaque,
            &GranularityProfile::standard(),
            &mut report,
        );
        assert!(
            !report.iter().any(|d| d.code == Code::E040),
            "prose must be allowed to be prose"
        );
    }

    /// ...and the exemption is only for `prose`: every other type still has to be one
    /// assertion, or the escape hatch would be the whole format's undoing.
    #[test]
    fn no_other_type_is_exempt() {
        for t in KernelType::ALL {
            if *t == KernelType::Prose {
                continue;
            }
            let Ok(core) = UnitCoreBuilder::new(*t, "a gist", Status::Speculative)
                .body("First paragraph.\n\nSecond paragraph.")
                .build()
            else {
                continue;
            };
            let mut report = Report::new();
            check_unit(
                &canonical_uid(&core),
                &core,
                &GranularityProfile::standard(),
                &mut report,
            );
            assert!(
                report.iter().any(|d| d.code == Code::E040),
                "{t} slipped past single-assertion admission"
            );
        }
    }

    #[test]
    fn a_single_paragraph_in_range_is_clean() {
        assert!(check(&in_range(), &GranularityProfile::standard()).is_empty());
    }

    #[test]
    fn two_paragraphs_are_e040_under_single_assertion() {
        let body = format!("{}\n\n{}", "word ".repeat(24), "word ".repeat(24));
        let r = check(&body, &GranularityProfile::standard());
        assert_eq!(r.count(Code::E040), 1);
        assert!(r.iter().next().unwrap().suggestion.is_some());
    }

    /// Topical admission is what `coarse` is for, so the same body must pass there.
    #[test]
    fn topical_admission_permits_several_paragraphs() {
        let body = format!("{}\n\n{}", "word ".repeat(80), "word ".repeat(80));
        let r = check(&body, &GranularityProfile::coarse());
        assert_eq!(r.count(Code::E040), 0);
    }

    #[test]
    fn a_multi_item_list_is_e040_under_single_assertion() {
        for body in [
            "- first thing\n- second thing",
            "* first thing\n* second thing",
            "1. first thing\n2. second thing",
        ] {
            let r = check(body, &GranularityProfile::standard());
            assert_eq!(r.count(Code::E040), 1, "{body}");
        }
    }

    #[test]
    fn a_single_item_list_is_not_multi_assertion() {
        let r = check(
            &format!("- {}", in_range()),
            &GranularityProfile::standard(),
        );
        assert_eq!(r.count(Code::E040), 0);
    }

    /// However long a single paragraph is, it is one assertion structurally. Reading it
    /// to decide otherwise would need a model.
    #[test]
    fn one_long_paragraph_is_never_multi_assertion() {
        let body = "This is one sentence. And here is another. And a third. ".repeat(10);
        let r = check(&body, &GranularityProfile::standard());
        assert_eq!(r.count(Code::E040), 0);
        assert_eq!(
            r.count(Code::W041),
            1,
            "it is over-length, but that is a warning"
        );
    }

    #[test]
    fn a_short_body_warns_w041() {
        let r = check("too short", &GranularityProfile::standard());
        assert_eq!(r.count(Code::W041), 1);
        assert!(r.iter().next().unwrap().message.contains("under"));
        assert!(r.is_clean(), "length is advisory, not fatal");
    }

    #[test]
    fn a_long_body_warns_w041() {
        let r = check(&"word ".repeat(200), &GranularityProfile::standard());
        assert_eq!(r.count(Code::W041), 1);
        assert!(r.iter().next().unwrap().message.contains("over"));
    }

    #[test]
    fn the_range_is_inclusive_at_both_ends() {
        let g = GranularityProfile::standard();
        // 160 bytes -> 40 tokens; 480 bytes -> 120 tokens.
        assert!(check(&"x".repeat(160), &g).is_empty());
        assert!(check(&"x".repeat(480), &g).is_empty());
        assert_eq!(check(&"x".repeat(156), &g).count(Code::W041), 1);
        assert_eq!(check(&"x".repeat(484), &g).count(Code::W041), 1);
    }

    /// Each profile has its own range, and the same body can be right for one and wrong
    /// for another - which is why granularity is a property of production, not of truth.
    #[test]
    fn the_range_follows_the_profile() {
        let body = "x".repeat(400); // 100 tokens
        assert!(check(&body, &GranularityProfile::standard()).is_empty());
        assert_eq!(
            check(&body, &GranularityProfile::fine()).count(Code::W041),
            1,
            "fine tops out at 60"
        );
        assert_eq!(
            check(&body, &GranularityProfile::coarse()).count(Code::W041),
            1,
            "coarse starts at 120"
        );
    }

    #[test]
    fn a_gist_only_unit_has_no_granularity_to_check() {
        let core = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative)
            .build()
            .unwrap();
        let store = Store::from_records(vec![Record::Unit(core)]);
        let mut r = Report::new();
        run(&store, &GranularityProfile::fine(), &mut r);
        assert!(r.is_empty());
    }

    #[test]
    fn both_findings_can_fire_on_one_unit() {
        let body = "- a\n- b";
        let r = check(body, &GranularityProfile::standard());
        assert_eq!(r.count(Code::E040), 1);
        assert_eq!(r.count(Code::W041), 1, "it is also far too short");
    }

    /// A numbered list item is a number, a full stop and a space — not any sentence.
    ///
    /// `list_items` reads `!n.is_empty() && n.bytes().all(is_ascii_digit)` over the text before
    /// the first `". "`. Mutation testing in 0.11 flipped that `&&` to `||` with nothing
    /// failing, and the two failure directions are both bad in the same way: `". foo"` yields an
    /// empty prefix, whose `all()` is vacuously true, and `"One thing. Another"` yields `"One
    /// thing"`, which is non-empty. So under `||` any prose with two sentences counts as a list
    /// of two items, and `multi_assertion` starts telling authors their paragraph is a list.
    ///
    /// Rule S admits one assertion per unit, so this decides whether ordinary prose is refused.
    #[test]
    fn ordinary_prose_is_not_a_list() {
        assert_eq!(
            list_items("The pool saturated. The canary did not. Both were checked."),
            0,
            "three sentences on one line were counted as list items"
        );
        assert_eq!(
            list_items("One thing.\nAnother thing.\nA third."),
            0,
            "sentences on separate lines are still sentences"
        );
        assert_eq!(
            list_items(". a leading full stop"),
            0,
            "an empty prefix is not a number"
        );
        assert_eq!(
            list_items("v1. the first version\n2. the second"),
            1,
            "`v1` is not a number; only the genuine `2.` counts"
        );
    }

    /// The control. Without it `list_items -> 0` passes every assertion above, and the
    /// multi-assertion check silently stops firing on real lists.
    #[test]
    fn a_real_list_is_counted() {
        assert_eq!(list_items("- first\n- second\n- third"), 3);
        assert_eq!(list_items("1. first\n2. second"), 2);
        assert_eq!(list_items("* starred\n+ plussed"), 2);
        assert!(
            multi_assertion("1. first\n2. second").is_some(),
            "a two-item list is more than one assertion"
        );
        assert!(
            multi_assertion("The pool saturated. The canary did not.").is_none(),
            "and two sentences are not"
        );
    }
}
