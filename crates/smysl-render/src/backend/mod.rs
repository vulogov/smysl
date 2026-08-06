//! Backends (§20 stage two): IR → artifact.
//!
//! Every backend implements one trait and sees only the IR. None of them can reach the
//! store, which is what stops two targets from disagreeing about what the document says.
//!
//! Rule V2 is not each backend's decision - the IR already carries
//! `contentions_suppressed` and the contention set. What a backend must do is *emit the
//! metadata*, always, in whatever form its format has. `every_backend_records_suppression`
//! asserts that across all of them at once, so a new backend cannot quietly omit it.

pub mod json;
pub mod markdown;
pub mod text;

#[cfg(feature = "html")]
pub mod html;
#[cfg(feature = "typst")]
pub mod slides;
#[cfg(feature = "typst")]
pub mod typst;

use smysl_core::error::RenderError;

use crate::ir::Ir;
use crate::profile::Profile;
use crate::Target;

/// A rendered document.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Artifact {
    pub target: Target,
    pub text: String,
}

impl Artifact {
    pub fn new(target: Target, text: impl Into<String>) -> Artifact {
        Artifact {
            target,
            text: text.into(),
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.text.as_bytes()
    }

    /// The conventional file extension, for `--output`.
    pub const fn extension(&self) -> &'static str {
        match self.target {
            Target::Markdown => "md",
            Target::Typst | Target::Slides => "typ",
            Target::Html => "html",
            Target::Json => "json",
            Target::Text => "txt",
        }
    }
}

/// One trait, one method: the IR and the profile in, an artifact out.
pub trait Backend {
    fn emit(&self, ir: &Ir, p: &Profile) -> Result<Artifact, RenderError>;
}

/// Render to a target, refusing one this build cannot produce.
///
/// The refusal is explicit rather than a silent fallback to markdown: an artifact in the
/// wrong format is more surprising than an error saying the format is unavailable.
pub fn emit(target: Target, ir: &Ir, p: &Profile) -> Result<Artifact, RenderError> {
    if !target.available() {
        return Err(RenderError::UnsupportedTarget {
            target: target.to_string(),
        });
    }
    match target {
        Target::Markdown => markdown::Markdown.emit(ir, p),
        Target::Json => json::Json.emit(ir, p),
        Target::Text => text::Text.emit(ir, p),
        #[cfg(feature = "typst")]
        Target::Typst => typst::Typst.emit(ir, p),
        #[cfg(feature = "typst")]
        Target::Slides => slides::Slides.emit(ir, p),
        #[cfg(feature = "html")]
        Target::Html => html::Html.emit(ir, p),
        // Unreachable while `available()` and this match agree - which is the whole point
        // of keeping it: if they ever drift, the caller gets a refusal rather than a
        // panic. With every backend compiled in there is nothing left for it to catch,
        // hence the allow.
        #[allow(unreachable_patterns)]
        other => Err(RenderError::UnsupportedTarget {
            target: other.to_string(),
        }),
    }
}

/// The line every backend emits when rule V2 suppression applied. Shared so the wording
/// cannot drift between formats.
pub(crate) fn suppression_note(ir: &Ir) -> Option<String> {
    if !ir.meta.contentions_suppressed {
        return None;
    }
    Some(format!(
        "SMY-W211: {} open contention(s) suppressed by profile {}: {}",
        ir.meta.open_contentions.len(),
        ir.meta.profile,
        ir.meta
            .open_contentions
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[cfg(test)]
pub(crate) mod fixture {
    //! One store and one thread, shared by every backend's tests, so a difference between
    //! two artifacts is a difference in the backend rather than in what it was given.

    use smysl_core::{
        canonical_uid, AgentId, Attestation, Contention, ContentionId, Detected, DetectionKind,
        Hlc, KernelType, Op, Record, RelKind, Relation, Role, Rung, SourceKind, SourceRef, Status,
        Step, Thread, ThreadId, ThreadSchema, UnitCoreBuilder,
    };
    use smysl_graph::Store;

    use crate::ir::{build, BuildOptions, Ir};
    use crate::profile::Profile;

    /// A three-block brief with a measured ground, a speculative risk, and an open
    /// contention over the finding - enough to exercise markers, connectives, notes, and
    /// rule V2 at once.
    pub fn ir(profile: &Profile) -> Ir {
        let (store, thread) = corpus();
        build(&store, &thread, profile, &BuildOptions::default())
    }

    pub fn corpus() -> (Store, Thread) {
        let agent = AgentId::new("model:vendor/m").unwrap();

        let evidence = UnitCoreBuilder::new(
            KernelType::Evidence,
            "p99 latency rose from 180ms to 410ms",
            Status::Measured,
        )
        .source(SourceRef::new(SourceKind::Metric, "p99_request_seconds"))
        .body("Measured over the eu-west shard across a one-minute window.")
        .build()
        .unwrap();
        let ue = canonical_uid(&evidence);

        let finding = UnitCoreBuilder::new(
            KernelType::Finding,
            "Pool saturation is the leading cause",
            Status::Derived,
        )
        .grounds([ue])
        .body("Wait time tracks the latency curve within the noise floor.")
        .build()
        .unwrap();
        let uf = canonical_uid(&finding);

        let risk = UnitCoreBuilder::new(
            KernelType::Hypothesis,
            "The canary shard was clean throughout",
            Status::Speculative,
        )
        .build()
        .unwrap();
        let ur = canonical_uid(&risk);

        let contention = Contention::new(
            ContentionId::new("k/pool-vs-canary").unwrap(),
            uf,
            vec![ur],
            Detected::new(DetectionKind::LiveRebuttal, Hlc::zero(agent.clone())),
        );

        let store = Store::from_records(vec![
            Record::Unit(evidence),
            Record::Unit(finding),
            Record::Unit(risk),
            Record::Relation(Relation::new(RelKind::Causes, uf, ue)),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uf)),
            Record::Attestation(Attestation::new(
                ue,
                agent.clone(),
                Op::Imported,
                Rung::Computed,
                Hlc::zero(agent.clone()),
            )),
            Record::Contention(contention),
        ]);

        let thread = Thread::new(
            ThreadId::new("t/brief").unwrap(),
            ThreadSchema::Brief,
            agent.clone(),
            "Pool saturation is the leading cause, though the canary is unexplained",
            Hlc::zero(agent),
        )
        .with_steps(vec![
            Step::new(Role::BottomLine, uf),
            Step::new(Role::Support, ue),
            Step::new(Role::Risk, ur),
        ]);

        (store, thread)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{build, BuildOptions};
    use crate::profile::Profile;

    fn available_targets() -> Vec<Target> {
        Target::ALL
            .iter()
            .copied()
            .filter(|t| t.available())
            .collect()
    }

    #[test]
    fn every_available_target_emits() {
        let p = Profile::builtin("plain").unwrap();
        let ir = fixture::ir(&p);
        for t in available_targets() {
            let a = emit(t, &ir, &p).unwrap_or_else(|e| panic!("{t}: {e}"));
            assert_eq!(a.target, t);
            assert!(!a.text.is_empty(), "{t} emitted nothing");
        }
    }

    /// An artifact in the wrong format is more surprising than an error saying the format
    /// is unavailable, so an uncompiled target refuses rather than falling back.
    #[test]
    fn an_unavailable_target_refuses_rather_than_substituting() {
        let p = Profile::builtin("plain").unwrap();
        let ir = fixture::ir(&p);
        for t in Target::ALL.iter().copied().filter(|t| !t.available()) {
            let e = emit(t, &ir, &p).expect_err("should refuse");
            assert!(matches!(e, RenderError::UnsupportedTarget { .. }), "{t}");
        }
    }

    /// **The gate**, across every backend at once: suppression is always recorded, so a
    /// new backend cannot quietly omit it.
    #[test]
    fn every_backend_records_suppression_in_its_output() {
        let p = Profile::load("profile quiet { show: { contentions: suppress } }").unwrap();
        let (store, thread) = fixture::corpus();
        let ir = build(&store, &thread, &p, &BuildOptions::default());
        assert!(ir.meta.contentions_suppressed, "the fixture must suppress");

        for t in available_targets() {
            let a = emit(t, &ir, &p).unwrap();
            assert!(
                a.text.contains("W211"),
                "{t} does not record the suppression"
            );
            assert!(
                a.text.contains("k/pool-vs-canary"),
                "{t} does not say which contention was suppressed"
            );
        }
    }

    /// ...and conversely, a document with nothing suppressed must not claim otherwise.
    #[test]
    fn no_backend_reports_suppression_that_did_not_happen() {
        let p = Profile::builtin("plain").unwrap();
        let ir = fixture::ir(&p);
        for t in available_targets() {
            let a = emit(t, &ir, &p).unwrap();
            assert!(!a.text.contains("W211"), "{t} invented a suppression");
        }
    }

    /// Rule V1 reaches the artifact: two statuses that the profile distinguishes must
    /// still be distinguishable after the backend has had its way with them.
    #[test]
    fn every_backend_keeps_statuses_distinguishable() {
        let p = Profile::builtin("plain").unwrap();
        let ir = fixture::ir(&p);
        for t in available_targets() {
            let a = emit(t, &ir, &p).unwrap();
            for b in &ir.blocks {
                assert!(
                    a.text.contains(&b.marker),
                    "{t} dropped the marker for {}",
                    b.status
                );
            }
        }
    }

    #[test]
    fn every_backend_renders_every_block() {
        let p = Profile::builtin("plain").unwrap();
        let ir = fixture::ir(&p);
        for t in available_targets() {
            let a = emit(t, &ir, &p).unwrap();
            for b in &ir.blocks {
                let first = b.text.lines().next().unwrap_or_default();
                assert!(a.text.contains(first), "{t} dropped a block");
            }
        }
    }

    #[test]
    fn emitting_is_deterministic() {
        let p = Profile::builtin("analyst").unwrap();
        let ir = fixture::ir(&p);
        for t in available_targets() {
            assert_eq!(emit(t, &ir, &p).unwrap(), emit(t, &ir, &p).unwrap());
        }
    }

    #[test]
    fn extensions_are_conventional() {
        assert_eq!(Artifact::new(Target::Markdown, "").extension(), "md");
        assert_eq!(Artifact::new(Target::Json, "").extension(), "json");
        assert_eq!(Artifact::new(Target::Slides, "").extension(), "typ");
    }

    #[test]
    fn an_empty_ir_still_produces_a_document() {
        let p = Profile::builtin("plain").unwrap();
        let ir = build(
            &smysl_graph::Store::new(),
            &smysl_core::Thread::new(
                smysl_core::ThreadId::new("t/empty").unwrap(),
                smysl_core::ThreadSchema::Brief,
                smysl_core::AgentId::new("tool:t").unwrap(),
                "nothing to say",
                smysl_core::Hlc::zero(smysl_core::AgentId::new("tool:t").unwrap()),
            ),
            &p,
            &BuildOptions::default(),
        );
        for t in available_targets() {
            let a = emit(t, &ir, &p).unwrap();
            assert!(!a.text.is_empty(), "{t} emitted nothing at all");
        }
    }
}
