//! Pass 3 - shape (§17).
//!
//! Most of what this pass looks for cannot happen in a store built through the API:
//! `UnitCore::new` refuses an empty gist, a detail without a body, `measured` without a
//! source, `derived` without grounds, and an authored `unfounded`, and the CBOR decoder
//! runs the same constructor. That is deliberate - the invariant is established once, at
//! construction, so encoding and hashing are infallible.
//!
//! Two things remain for this pass. `SMY-E022` is genuinely its own: the gist bound is
//! *relative to a granularity profile*, which a constructor has no access to. The rest is
//! defence in depth, cheap to run, and the thing that would catch a future constructor
//! bypass before it reached a store.

use smysl_core::diag::{Code, Diagnostic, Report};
use smysl_core::{tokens, GranularityProfile, Uid, UnitCore};
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
    // `SMY-E022` - the gist bound is per-profile, so only this pass can see it.
    let gist_tokens = tokens(&core.gist);
    if !granularity.gist_within_bound(gist_tokens) {
        report.push(
            Diagnostic::on(Code::E022, *uid)
                .with_message(format!(
                    "gist is {gist_tokens} tokens, {} allows {}",
                    granularity.profile, granularity.l0_max
                ))
                .with_suggestion("move the detail into `body` and shorten the gist"),
        );
    }

    // Defence in depth: the constructor guarantees each of these, so a hit here means a
    // unit reached the store without passing through it.
    if core.gist.trim().is_empty() {
        report.push(Diagnostic::on(Code::E021, *uid).with_message("empty gist"));
    }
    if core.detail.is_some() && core.body.is_none() {
        report.push(Diagnostic::on(Code::E023, *uid).with_message("detail without body"));
    }
    if core.status == smysl_core::Status::Unfounded {
        report.push(
            Diagnostic::on(Code::E034, *uid)
                .with_message("unfounded is reachable only by retraction, never by authoring"),
        );
    }
    if core.status.requires_source() && core.source.is_none() {
        report.push(
            Diagnostic::on(Code::E032, *uid)
                .with_message(format!("{} without a source", core.status)),
        );
    }
    if core.status.requires_grounds() && core.grounds.is_empty() {
        report.push(
            Diagnostic::on(Code::E031, *uid)
                .with_message(format!("{} with no grounds", core.status)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Record, SourceKind, SourceRef, Status, UnitCoreBuilder};

    fn check(core: UnitCore, g: &GranularityProfile) -> Report {
        let store = Store::from_records(vec![Record::Unit(core)]);
        let mut r = Report::new();
        run(&store, g, &mut r);
        r
    }

    fn short() -> UnitCore {
        UnitCoreBuilder::new(
            KernelType::Claim,
            "p95 auth latency tripled",
            Status::Speculative,
        )
        .build()
        .unwrap()
    }

    #[test]
    fn a_short_gist_passes_every_profile() {
        for g in [
            GranularityProfile::coarse(),
            GranularityProfile::standard(),
            GranularityProfile::fine(),
        ] {
            assert!(check(short(), &g).is_empty(), "{}", g.profile);
        }
    }

    /// The gist bound is the one shape rule a constructor cannot enforce, because it is
    /// relative to a profile the constructor never sees.
    #[test]
    fn an_oversized_gist_is_e022() {
        let long = "word ".repeat(40);
        let core = UnitCoreBuilder::new(KernelType::Claim, long, Status::Speculative)
            .build()
            .unwrap();
        let r = check(core, &GranularityProfile::standard());
        assert_eq!(r.count(Code::E022), 1);
        let d = r.iter().next().unwrap();
        assert!(
            d.message.contains("30"),
            "the bound must be named: {}",
            d.message
        );
        assert!(d.suggestion.is_some());
    }

    #[test]
    fn the_gist_bound_is_measured_in_estimator_tokens() {
        // 120 bytes -> 30 tokens, exactly at the bound.
        let at_bound = "x".repeat(120);
        assert_eq!(tokens(&at_bound), 30);
        let core = UnitCoreBuilder::new(KernelType::Claim, at_bound, Status::Speculative)
            .build()
            .unwrap();
        assert!(check(core, &GranularityProfile::standard()).is_empty());

        let over = "x".repeat(124);
        assert_eq!(tokens(&over), 31);
        let core = UnitCoreBuilder::new(KernelType::Claim, over, Status::Speculative)
            .build()
            .unwrap();
        assert_eq!(
            check(core, &GranularityProfile::standard()).count(Code::E022),
            1
        );
    }

    /// Every profile shares the same gist bound, so a document does not become invalid by
    /// being re-granulated (D-5).
    #[test]
    fn the_gist_bound_does_not_vary_by_profile() {
        let over = "x".repeat(200);
        let core = UnitCoreBuilder::new(KernelType::Claim, over, Status::Speculative)
            .build()
            .unwrap();
        for g in [
            GranularityProfile::coarse(),
            GranularityProfile::standard(),
            GranularityProfile::fine(),
        ] {
            assert_eq!(
                check(core.clone(), &g).count(Code::E022),
                1,
                "{}",
                g.profile
            );
        }
    }

    /// The constructor already refuses all of these, so a well-formed store cannot
    /// contain them. Asserting that is the point: the pass is a backstop, not the
    /// primary defence.
    #[test]
    fn the_constructor_makes_the_remaining_shape_rules_unreachable() {
        use smysl_core::ShapeError;
        type Build = Box<dyn Fn() -> Result<UnitCore, ShapeError>>;
        let cases: Vec<(ShapeError, Build)> = vec![
            (
                ShapeError::MissingGist,
                Box::new(|| {
                    UnitCoreBuilder::new(KernelType::Claim, "", Status::Speculative).build()
                }),
            ),
            (
                ShapeError::DetailWithoutBody,
                Box::new(|| {
                    UnitCoreBuilder::new(KernelType::Claim, "g", Status::Speculative)
                        .detail("d")
                        .build()
                }),
            ),
            (
                ShapeError::UnfoundedAuthored,
                Box::new(|| {
                    UnitCoreBuilder::new(KernelType::Claim, "g", Status::Unfounded).build()
                }),
            ),
            (
                ShapeError::SourceRequired,
                Box::new(|| UnitCoreBuilder::new(KernelType::Claim, "g", Status::Measured).build()),
            ),
            (
                ShapeError::GroundsRequired,
                Box::new(|| UnitCoreBuilder::new(KernelType::Claim, "g", Status::Derived).build()),
            ),
        ];
        for (expected, build) in cases {
            assert_eq!(build().unwrap_err(), expected);
        }
    }

    #[test]
    fn a_well_formed_measured_unit_passes() {
        let core = UnitCoreBuilder::new(KernelType::Evidence, "traces", Status::Measured)
            .source(SourceRef::new(SourceKind::Metric, "x"))
            .build()
            .unwrap();
        assert!(check(core, &GranularityProfile::standard()).is_empty());
    }

    #[test]
    fn the_pass_reports_every_unit_not_just_the_first() {
        let a = UnitCoreBuilder::new(KernelType::Claim, "x".repeat(200), Status::Speculative)
            .build()
            .unwrap();
        let b = UnitCoreBuilder::new(KernelType::Claim, "y".repeat(200), Status::Speculative)
            .build()
            .unwrap();
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);
        let mut r = Report::new();
        run(&store, &GranularityProfile::standard(), &mut r);
        assert_eq!(r.count(Code::E022), 2);
    }
}
