//! `smysl-check` - the check pipeline (§17).
//!
//! Passes run in dependency order and **MUST NOT short-circuit**. A full diagnostic set is
//! more useful than a first error, and the ingest repair loop (§22.3) needs all of them at
//! once: it resends the offending spans, so one pass reporting and the rest staying silent
//! would turn a one-round repair into several.
//!
//! `check` verifies consistency, never correctness (N13). It can tell you a claim's status
//! exceeds its weakest ground; it cannot tell you whether the claim is true.
//!
//! SM-P5 adds rules M and T and the extension pass. Retraction integrity lands with merge
//! in SM-P6; hash verification is the store's, and runs through `Store::verify_against`.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod passes;

use std::collections::BTreeMap;

use smysl_core::{Error, GranularityProfile, Label, Severity, Uid};
use smysl_graph::Store;

pub use passes::extension::{fidelity, ConsumerProfile, FidelityReport};
pub use smysl_core::diag::{Code, Diagnostic, Report, Span, Subject};

/// The ten passes of §17. A pass that has not landed yet reports itself as unavailable
/// rather than silently doing nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Pass {
    /// 1 - envelope and codec. Runs at read time, not here.
    Codec,
    /// 2 - reference integrity.
    Integrity,
    /// 3 - shape.
    Shape,
    /// 4 - rule L closure.
    Closure,
    /// 5 - granularity.
    Granularity,
    /// 6 - rule M epistemics.
    Epistemics,
    /// 7 - rule T trust ceiling.
    Trust,
    /// 8 - retraction integrity.
    Retraction,
    /// 9 - extension and conformance.
    Extension,
    /// 10 - hash verification.
    Hashes,
}

impl Pass {
    pub const ALL: &'static [Pass] = &[
        Pass::Codec,
        Pass::Integrity,
        Pass::Shape,
        Pass::Closure,
        Pass::Granularity,
        Pass::Epistemics,
        Pass::Trust,
        Pass::Retraction,
        Pass::Extension,
        Pass::Hashes,
    ];

    /// The passes this build actually runs.
    pub const IMPLEMENTED: &'static [Pass] = &[
        Pass::Integrity,
        Pass::Shape,
        Pass::Closure,
        Pass::Granularity,
        Pass::Epistemics,
        Pass::Trust,
        Pass::Extension,
    ];

    pub const fn number(self) -> u8 {
        match self {
            Pass::Codec => 1,
            Pass::Integrity => 2,
            Pass::Shape => 3,
            Pass::Closure => 4,
            Pass::Granularity => 5,
            Pass::Epistemics => 6,
            Pass::Trust => 7,
            Pass::Retraction => 8,
            Pass::Extension => 9,
            Pass::Hashes => 10,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Pass::Codec => "codec",
            Pass::Integrity => "integrity",
            Pass::Shape => "shape",
            Pass::Closure => "closure",
            Pass::Granularity => "granularity",
            Pass::Epistemics => "epistemics",
            Pass::Trust => "trust",
            Pass::Retraction => "retraction",
            Pass::Extension => "extension",
            Pass::Hashes => "hashes",
        }
    }

    pub fn parse(s: &str) -> Option<Pass> {
        Pass::ALL.iter().copied().find(|p| p.as_str() == s)
    }

    pub fn is_implemented(self) -> bool {
        Pass::IMPLEMENTED.contains(&self)
    }
}

impl core::fmt::Display for Pass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.number(), self.as_str())
    }
}

/// What to check, and against what.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CheckOptions {
    /// The granularity the units were produced under. Taken from the store's view when
    /// absent; granularity constrains production, not the store, so a caller may override
    /// it (D-5).
    pub granularity: Option<GranularityProfile>,
    /// Label bindings, so a body that references a unit by name can be closure-checked.
    /// Labels have no wire record, so only a caller that parsed the surface text has them.
    pub labels: BTreeMap<Label, Uid>,
    /// Restrict to these passes. Empty means every implemented pass.
    pub only: Vec<Pass>,
    /// What the consumer implements, for the `--as` degradation report. Absent means no
    /// `SMY-W010` is emitted: nobody asked what a particular consumer would lose.
    pub consumer: Option<ConsumerProfile>,
}

impl CheckOptions {
    /// Everything this build can check.
    pub fn strict() -> CheckOptions {
        CheckOptions::default()
    }

    pub fn with_granularity(mut self, g: GranularityProfile) -> CheckOptions {
        self.granularity = Some(g);
        self
    }

    pub fn with_labels(mut self, labels: BTreeMap<Label, Uid>) -> CheckOptions {
        self.labels = labels;
        self
    }

    pub fn only(mut self, passes: impl IntoIterator<Item = Pass>) -> CheckOptions {
        self.only = passes.into_iter().collect();
        self
    }

    pub fn as_consumer(mut self, p: ConsumerProfile) -> CheckOptions {
        self.consumer = Some(p);
        self
    }

    fn runs(&self, p: Pass) -> bool {
        p.is_implemented() && (self.only.is_empty() || self.only.contains(&p))
    }
}

/// Run the pipeline.
///
/// Never short-circuits: every requested pass runs, whatever the earlier ones found.
pub fn check(store: &Store, opts: CheckOptions) -> Report {
    let granularity = opts
        .granularity
        .clone()
        .or_else(|| store.views().next().map(|v| v.granularity.clone()))
        .unwrap_or_default();

    let mut report = Report::new();
    if opts.runs(Pass::Integrity) {
        passes::integrity::run(store, &mut report);
    }
    if opts.runs(Pass::Shape) {
        passes::shape::run(store, &granularity, &mut report);
    }
    if opts.runs(Pass::Closure) {
        passes::closure::run(store, &opts.labels, &mut report);
    }
    if opts.runs(Pass::Granularity) {
        passes::granularity::run(store, &granularity, &mut report);
    }
    if opts.runs(Pass::Epistemics) {
        passes::epistemics::run(store, &mut report);
    }
    if opts.runs(Pass::Trust) {
        passes::trust::run(store, &mut report);
    }
    if opts.runs(Pass::Extension) {
        passes::extension::run(store, opts.consumer.as_ref(), &mut report);
    }
    report.sort();
    report
}

/// Whether a store is consumable at a conformance class, and why not if it is not.
///
/// The classes of §11 are about implementations, but a *store* can put a conforming
/// implementation in an impossible position: a consumer at C-Consume must enforce rules M
/// and R, so a store that violates rule M cannot be consumed at that class however
/// correct the consumer is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceVerdict {
    pub class: ConformanceClass,
    pub passed: bool,
    /// Codes present in the store that this class forbids.
    pub blocking: Vec<Code>,
}

impl core::fmt::Display for ConformanceVerdict {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        if self.passed {
            write!(f, "{}: pass", self.class)
        } else {
            let codes: Vec<String> = self.blocking.iter().map(ToString::to_string).collect();
            write!(f, "{}: fail ({})", self.class, codes.join(", "))
        }
    }
}

/// Assess a report against a conformance class.
pub fn conformance(report: &Report, class: ConformanceClass) -> ConformanceVerdict {
    let mut blocking: Vec<Code> = report
        .iter()
        .filter(|d| d.is_error() && class.forbids(d.code))
        .map(|d| d.code)
        .collect();
    blocking.sort();
    blocking.dedup();
    ConformanceVerdict {
        class,
        passed: blocking.is_empty(),
        blocking,
    }
}

/// `check` plus the failure decision, for callers that want one call.
pub fn check_and_fail_on(
    store: &Store,
    opts: CheckOptions,
    severity: Severity,
) -> Result<Report, Error> {
    let r = check(store, opts);
    r.fail_on(severity)?;
    Ok(r)
}

/// The granularity distribution of a store.
///
/// Mixed granularity in a merged store is legal, not an error (D-5). `check --granularity`
/// reports the distribution so a reader can see what they are looking at rather than being
/// told off for it.
pub fn granularity_distribution(store: &Store) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for v in store.views() {
        *out.entry(v.granularity.profile.clone()).or_insert(0) += 1;
    }
    out
}

/// The conformance classes of §11. A minimal downstream agent needs `Consume`; the
/// reference implementation targets `Full`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum ConformanceClass {
    Read,
    Consume,
    Produce,
    Merge,
    Full,
}

impl ConformanceClass {
    pub const ALL: &'static [ConformanceClass] = &[
        ConformanceClass::Read,
        ConformanceClass::Consume,
        ConformanceClass::Produce,
        ConformanceClass::Merge,
        ConformanceClass::Full,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            ConformanceClass::Read => "C-Read",
            ConformanceClass::Consume => "C-Consume",
            ConformanceClass::Produce => "C-Produce",
            ConformanceClass::Merge => "C-Merge",
            ConformanceClass::Full => "C-Full",
        }
    }

    pub fn parse(s: &str) -> Option<ConformanceClass> {
        ConformanceClass::ALL
            .iter()
            .copied()
            .find(|c| c.as_str().eq_ignore_ascii_case(s))
    }

    /// Every class builds on C-Read (§11).
    pub const fn requires_read(self) -> bool {
        true
    }

    /// Whether a store carrying this code is unusable at this class.
    ///
    /// C-Read only has to parse and verify hashes, so a rule M violation does not stop it
    /// reading. C-Consume must enforce rules M and R when packing, so the same violation
    /// makes the store unconsumable - the constraint is on what the class *promises*, not
    /// on how careful the reader is.
    pub fn forbids(self, code: Code) -> bool {
        let structural = matches!(
            code,
            Code::E001
                | Code::E002
                | Code::E003
                | Code::E004
                | Code::E080
                | Code::E081
                | Code::E070
                | Code::E071
                | Code::E060
                | Code::E061
                | Code::E011
        );
        let epistemic = matches!(code, Code::E030 | Code::E033 | Code::E034 | Code::E012);
        let shape = matches!(
            code,
            Code::E020
                | Code::E021
                | Code::E022
                | Code::E023
                | Code::E031
                | Code::E032
                | Code::E040
        );
        let lifecycle = matches!(code, Code::E050 | Code::E051);
        let render = matches!(code, Code::E210);

        match self {
            ConformanceClass::Read => structural,
            ConformanceClass::Consume => structural || epistemic,
            ConformanceClass::Produce => structural || epistemic || shape,
            ConformanceClass::Merge => structural || epistemic || lifecycle,
            ConformanceClass::Full => structural || epistemic || shape || lifecycle || render,
        }
    }
}

impl core::fmt::Display for ConformanceClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.pad(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Record, Status, UnitCoreBuilder, View, ViewId};

    fn claim(gist: &str) -> Record {
        Record::Unit(
            UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
                .build()
                .unwrap(),
        )
    }

    #[test]
    fn five_conformance_classes_with_stable_names() {
        assert_eq!(ConformanceClass::ALL.len(), 5);
        assert_eq!(ConformanceClass::Read.to_string(), "C-Read");
        assert_eq!(ConformanceClass::Full.to_string(), "C-Full");
        for c in ConformanceClass::ALL {
            assert!(c.as_str().starts_with("C-"));
            assert!(c.requires_read());
            assert_eq!(ConformanceClass::parse(c.as_str()), Some(*c));
        }
        assert_eq!(
            ConformanceClass::parse("c-full"),
            Some(ConformanceClass::Full)
        );
        assert_eq!(ConformanceClass::parse("C-Nonsense"), None);
    }

    #[test]
    fn ten_passes_numbered_as_in_section_17() {
        assert_eq!(Pass::ALL.len(), 10);
        for (i, p) in Pass::ALL.iter().enumerate() {
            assert_eq!(p.number() as usize, i + 1);
            assert_eq!(Pass::parse(p.as_str()), Some(*p));
        }
    }

    /// A pass that has not landed reports itself unavailable rather than passing
    /// silently, which is the difference between "checked and clean" and "not checked".
    #[test]
    fn only_the_landed_passes_are_implemented() {
        let implemented: Vec<&str> = Pass::ALL
            .iter()
            .filter(|p| p.is_implemented())
            .map(|p| p.as_str())
            .collect();
        assert_eq!(
            implemented,
            [
                "integrity",
                "shape",
                "closure",
                "granularity",
                "epistemics",
                "trust",
                "extension"
            ]
        );
        assert!(
            !Pass::Retraction.is_implemented(),
            "lands with merge in SM-P6"
        );
        assert!(
            !Pass::Hashes.is_implemented(),
            "hash verification belongs to the store, via Store::verify_against"
        );
    }

    #[test]
    fn a_clean_store_produces_an_empty_report() {
        let store = Store::from_records(vec![claim("p95 auth latency tripled")]);
        let r = check(&store, CheckOptions::default());
        assert!(r.is_empty(), "{r}");
        assert!(r.fail_on(Severity::Warn).is_ok());
    }

    /// The pipeline must not short-circuit: a store with defects in several passes
    /// reports all of them in one run.
    #[test]
    fn every_pass_runs_whatever_the_earlier_ones_found() {
        let dangling =
            UnitCoreBuilder::new(KernelType::Claim, "x".repeat(200), Status::Speculative)
                .deps([Uid::from_bytes([9; 32])])
                .body("- one\n- two")
                .build()
                .unwrap();
        let store = Store::from_records(vec![Record::Unit(dangling)]);
        let r = check(&store, CheckOptions::default());

        assert_eq!(r.count(Code::E060), 1, "pass 2 ran");
        assert_eq!(r.count(Code::E022), 1, "pass 3 ran");
        assert_eq!(r.count(Code::E040), 1, "pass 5 ran");
        assert_eq!(r.count(Code::W041), 1, "pass 5 ran twice");
    }

    #[test]
    fn passes_can_be_selected_individually() {
        let bad = UnitCoreBuilder::new(KernelType::Claim, "x".repeat(200), Status::Speculative)
            .deps([Uid::from_bytes([9; 32])])
            .build()
            .unwrap();
        let store = Store::from_records(vec![Record::Unit(bad)]);

        let only_shape = check(&store, CheckOptions::default().only([Pass::Shape]));
        assert_eq!(only_shape.count(Code::E022), 1);
        assert_eq!(only_shape.count(Code::E060), 0);

        let only_integrity = check(&store, CheckOptions::default().only([Pass::Integrity]));
        assert_eq!(only_integrity.count(Code::E060), 1);
        assert_eq!(only_integrity.count(Code::E022), 0);
    }

    /// Selecting a pass that has not landed must not silently report clean.
    #[test]
    fn selecting_an_unimplemented_pass_runs_nothing() {
        let store = Store::from_records(vec![claim("a claim")]);
        let r = check(&store, CheckOptions::default().only([Pass::Epistemics]));
        assert!(r.is_empty());
    }

    #[test]
    fn the_granularity_comes_from_the_view_when_not_given() {
        let body = "x".repeat(400); // 100 tokens: inside `default`, outside `fine`
        let core = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative)
            .body(body)
            .build()
            .unwrap();
        let view = View::new(ViewId::new("v/x").unwrap(), "i")
            .with_granularity(GranularityProfile::fine());
        let store = Store::from_records(vec![Record::Unit(core), Record::View(view)]);

        assert_eq!(
            check(&store, CheckOptions::default()).count(Code::W041),
            1,
            "the view's profile should apply"
        );
        assert_eq!(
            check(
                &store,
                CheckOptions::default().with_granularity(GranularityProfile::standard())
            )
            .count(Code::W041),
            0,
            "an explicit profile should override the view"
        );
    }

    #[test]
    fn the_report_is_sorted_and_therefore_reproducible() {
        let bad = UnitCoreBuilder::new(KernelType::Claim, "x".repeat(200), Status::Speculative)
            .deps([Uid::from_bytes([9; 32])])
            .build()
            .unwrap();
        let store = Store::from_records(vec![Record::Unit(bad)]);
        let a = check(&store, CheckOptions::default());
        let b = check(&store, CheckOptions::default());
        assert_eq!(a.diagnostics, b.diagnostics);
    }

    #[test]
    fn fail_on_projects_the_report_into_a_result() {
        let bad = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative)
            .deps([Uid::from_bytes([9; 32])])
            .build()
            .unwrap();
        let store = Store::from_records(vec![Record::Unit(bad)]);
        assert!(check_and_fail_on(&store, CheckOptions::default(), Severity::Error).is_err());

        let clean = Store::from_records(vec![claim("a claim")]);
        assert!(check_and_fail_on(&clean, CheckOptions::default(), Severity::Warn).is_ok());
    }

    /// The two halves of the anti-laundering guarantee run in the same pass list, and a
    /// store can violate both at once.
    #[test]
    fn rules_m_and_t_both_run() {
        use smysl_core::{
            canonical_uid, AgentId, Attestation, Hlc, Op, Rung, SourceKind, SourceRef, Status,
        };
        let guess = UnitCoreBuilder::new(KernelType::Hypothesis, "a guess", Status::Speculative)
            .build()
            .unwrap();
        let ug = canonical_uid(&guess);
        let promoted = UnitCoreBuilder::new(KernelType::Claim, "promoted", Status::Derived)
            .grounds([ug])
            .build()
            .unwrap();
        let up = canonical_uid(&promoted);
        let ag = AgentId::new("model:openai/gpt").unwrap();

        let store = Store::from_records(vec![
            Record::Unit(guess),
            Record::Unit(promoted),
            Record::Attestation(Attestation::new(
                up,
                ag.clone(),
                Op::Authored,
                Rung::Model,
                Hlc::zero(ag),
            )),
        ]);
        let r = check(&store, CheckOptions::default());
        assert_eq!(r.count(Code::E030), 1, "rule M: derived on speculative");
        assert_eq!(
            r.count(Code::E033),
            1,
            "rule T: a model cannot claim derived"
        );
        let _ = SourceRef::new(SourceKind::Doc, "x");
    }

    #[test]
    fn conformance_verdicts_differ_by_class() {
        use smysl_core::{canonical_uid, Status};
        let guess = UnitCoreBuilder::new(KernelType::Hypothesis, "a guess", Status::Speculative)
            .build()
            .unwrap();
        let ug = canonical_uid(&guess);
        let promoted = UnitCoreBuilder::new(KernelType::Claim, "promoted", Status::Derived)
            .grounds([ug])
            .build()
            .unwrap();
        let store = Store::from_records(vec![Record::Unit(guess), Record::Unit(promoted)]);
        let report = check(&store, CheckOptions::default());

        // A rule M violation does not stop a reader parsing, but it does stop a consumer
        // promising rules M and R.
        assert!(conformance(&report, ConformanceClass::Read).passed);
        let consume = conformance(&report, ConformanceClass::Consume);
        assert!(!consume.passed);
        assert_eq!(consume.blocking, vec![Code::E030]);
        assert!(!conformance(&report, ConformanceClass::Full).passed);
    }

    #[test]
    fn a_clean_store_conforms_at_every_class() {
        let store = Store::from_records(vec![claim("a claim")]);
        let report = check(&store, CheckOptions::default());
        for c in ConformanceClass::ALL {
            let v = conformance(&report, *c);
            assert!(v.passed, "{v}");
            assert_eq!(v.to_string(), format!("{c}: pass"));
        }
    }

    /// A dangling reference blocks every class, because nothing can be done with a store
    /// whose references do not resolve.
    #[test]
    fn a_structural_defect_blocks_every_class() {
        let bad = UnitCoreBuilder::new(KernelType::Claim, "a claim", Status::Speculative)
            .deps([Uid::from_bytes([9; 32])])
            .build()
            .unwrap();
        let store = Store::from_records(vec![Record::Unit(bad)]);
        let report = check(&store, CheckOptions::default());
        for c in ConformanceClass::ALL {
            assert!(!conformance(&report, *c).passed, "{c}");
        }
    }

    /// Warnings never block: degraded fidelity is a fact about a consumer, not a defect
    /// in the store.
    #[test]
    fn warnings_do_not_block_conformance() {
        use smysl_core::SchemaId;
        let store = Store::from_records(vec![Record::Unit(
            UnitCoreBuilder::new(
                SchemaId::parse("x.sre/incident").unwrap(),
                "an incident",
                Status::Speculative,
            )
            .build()
            .unwrap(),
        )]);
        let report = check(
            &store,
            CheckOptions::default().as_consumer(ConsumerProfile::default()),
        );
        assert_eq!(report.count(Code::W010), 1);
        assert!(conformance(&report, ConformanceClass::Full).passed);
    }

    /// Mixed granularity is legal (D-5), so it is reported as a distribution rather than
    /// as a defect.
    #[test]
    fn the_granularity_distribution_is_reported_not_rejected() {
        let store = Store::from_records(vec![
            Record::View(
                View::new(ViewId::new("v/a").unwrap(), "a")
                    .with_granularity(GranularityProfile::fine()),
            ),
            Record::View(
                View::new(ViewId::new("v/b").unwrap(), "b")
                    .with_granularity(GranularityProfile::coarse()),
            ),
        ]);
        let d = granularity_distribution(&store);
        assert_eq!(d.get("fine"), Some(&1));
        assert_eq!(d.get("coarse"), Some(&1));
        assert!(check(&store, CheckOptions::default()).is_empty());
    }
}
