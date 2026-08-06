//! Golden artifacts - the byte-for-byte half of the SM-P12 gate.
//!
//! Rendering is a pure function of graph, thread and profile (rule D). The way to assert
//! that over a whole document, rather than over the handful of properties a unit test can
//! name, is to write the document down and compare bytes.
//!
//! A diff here is never noise: nothing in these files comes from a model. A change means
//! the renderer moved. If the move is intended, re-bless with
//! `SMYSL_BLESS=1 cargo test -p smysl-render --test golden --all-features` and read the
//! diff - it is the evidence that the change did what it claimed.

use std::path::PathBuf;

use smysl_core::{
    canonical_uid, AgentId, Attestation, Contention, ContentionId, Detected, DetectionKind, Hlc,
    KernelType, Op, Record, RelKind, Relation, Role, Rung, SourceKind, SourceRef, Status, Step,
    Thread, ThreadId, ThreadSchema, UnitCoreBuilder,
};
use smysl_graph::Store;
use smysl_render::{build, emit, BuildOptions, Profile, Target};

fn golden_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/golden/render")
}

/// The incident graph of Appendix G, in miniature: a measured ground, a derived finding,
/// a speculative rebuttal, and the contention between them.
fn corpus() -> (Store, Thread) {
    let author = AgentId::new("human:vladimir").unwrap();
    let tool = AgentId::new("model:vendor/m").unwrap();

    let p95 = UnitCoreBuilder::new(
        KernelType::Evidence,
        "p95 request latency rose from 180ms to 410ms after the 4.2 rollout",
        Status::Measured,
    )
    .source(SourceRef::new(SourceKind::Metric, "p95_request_seconds"))
    .body("Measured on the eu-west shard over one-minute windows, 14:00-16:00 UTC.")
    .detail("Sampled at 10s resolution; the pre-rollout baseline is the trailing 7-day median.")
    .build()
    .unwrap();
    let u_p95 = canonical_uid(&p95);

    let pool = UnitCoreBuilder::new(
        KernelType::Finding,
        "Pool saturation is the leading cause",
        Status::Derived,
    )
    .grounds([u_p95])
    .body("Connection wait time tracks the latency curve within the noise floor.")
    .build()
    .unwrap();
    let u_pool = canonical_uid(&pool);

    let canary = UnitCoreBuilder::new(
        KernelType::Hypothesis,
        "The canary shard was clean throughout",
        Status::Speculative,
    )
    .build()
    .unwrap();
    let u_canary = canonical_uid(&canary);

    let rollback = UnitCoreBuilder::new(
        KernelType::Decision,
        "Roll the eu-west shard back to 4.1 and re-measure",
        Status::Inferred,
    )
    .grounds([u_pool])
    .build()
    .unwrap();
    let u_rollback = canonical_uid(&rollback);

    let store = Store::from_records(vec![
        Record::Unit(p95),
        Record::Unit(pool),
        Record::Unit(canary),
        Record::Unit(rollback),
        Record::Relation(Relation::new(RelKind::Causes, u_pool, u_p95)),
        Record::Relation(Relation::new(RelKind::Rebuts, u_canary, u_pool)),
        Record::Relation(Relation::new(RelKind::Sequences, u_rollback, u_pool)),
        Record::Attestation(Attestation::new(
            u_p95,
            tool.clone(),
            Op::Imported,
            Rung::Computed,
            Hlc::zero(tool.clone()),
        )),
        Record::Contention(Contention::new(
            ContentionId::new("k/pool-vs-canary").unwrap(),
            u_pool,
            vec![u_canary],
            Detected::new(DetectionKind::LiveRebuttal, Hlc::zero(tool)),
        )),
    ]);

    let thread = Thread::new(
        ThreadId::new("t/brief").unwrap(),
        ThreadSchema::Brief,
        author.clone(),
        "Pool saturation is the leading cause; the clean canary is unexplained",
        Hlc::zero(author),
    )
    .with_steps(vec![
        Step::new(Role::BottomLine, u_pool),
        Step::new(Role::Support, u_p95),
        Step::new(Role::Risk, u_canary),
        Step::new(Role::Ask, u_rollback),
    ]);

    (store, thread)
}

fn render(profile: &str, target: Target) -> String {
    let p = Profile::builtin(profile).expect("built-in profile");
    let (store, thread) = corpus();
    let ir = build(&store, &thread, &p, &BuildOptions::default());
    emit(target, &ir, &p).expect("target is available").text
}

fn check(name: &str, text: &str) {
    let path = golden_dir().join(name);

    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).expect("create the golden directory");
        std::fs::write(&path, text).expect("write the golden file");
        return;
    }

    let golden = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "{}: {e}\nrun with SMYSL_BLESS=1 to create it",
            path.display()
        )
    });

    if golden != text {
        // Report the first differing line rather than two whole documents: the line is
        // what a reader needs and the documents are what the file already holds.
        let at = golden
            .lines()
            .zip(text.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| golden.lines().count().min(text.lines().count()));
        panic!(
            "{} differs at line {}\n  golden: {:?}\n  actual: {:?}",
            path.display(),
            at + 1,
            golden.lines().nth(at),
            text.lines().nth(at),
        );
    }
}

#[test]
fn markdown_under_the_exec_profile() {
    check("exec.md", &render("exec", Target::Markdown));
}

#[test]
fn markdown_under_the_analyst_profile() {
    check("analyst.md", &render("analyst", Target::Markdown));
}

#[test]
fn plain_text_under_the_plain_profile() {
    check("plain.txt", &render("plain", Target::Text));
}

#[test]
fn json_under_the_analyst_profile() {
    check("analyst.json", &render("analyst", Target::Json));
}

#[cfg(feature = "typst")]
#[test]
fn typst_under_the_exec_profile() {
    check("exec.typ", &render("exec", Target::Typst));
}

#[cfg(feature = "typst")]
#[test]
fn slides_under_the_exec_profile() {
    check("exec.slides.typ", &render("exec", Target::Slides));
}

#[cfg(feature = "html")]
#[test]
fn html_under_the_analyst_profile() {
    check("analyst.html", &render("analyst", Target::Html));
}

/// The same graph, two profiles: the RFC's claim in Appendix G is that no conversion step
/// exists and only the thread and the profile differ. If two profiles produced the same
/// bytes, the profile would not be doing anything.
#[test]
fn two_profiles_over_one_graph_produce_different_documents() {
    let exec = render("exec", Target::Markdown);
    let analyst = render("analyst", Target::Markdown);
    assert_ne!(exec, analyst);
    // ...but they must agree about what the graph says.
    for uid in ["Pool saturation", "The canary shard was clean"] {
        assert!(exec.contains(uid), "exec dropped `{uid}`");
        assert!(analyst.contains(uid), "analyst dropped `{uid}`");
    }
}

/// Rule D over the whole document rather than over a property of it.
#[test]
fn rendering_the_same_inputs_twice_is_byte_identical() {
    for profile in ["plain", "exec", "analyst"] {
        for target in Target::ALL.iter().copied().filter(|t| t.available()) {
            assert_eq!(
                render(profile, target),
                render(profile, target),
                "{profile}/{target}"
            );
        }
    }
}

/// Rule V1 reaching the artifact: `derived` and `speculative` are distinguishable in every
/// golden document, which is the property the whole format exists to preserve at the last
/// hop.
#[test]
fn the_golden_documents_distinguish_derived_from_speculative() {
    for profile in ["plain", "exec", "analyst"] {
        let p = Profile::builtin(profile).unwrap();
        let derived = p.marker(Status::Derived);
        let speculative = p.marker(Status::Speculative);
        assert_ne!(derived, speculative);

        for target in Target::ALL.iter().copied().filter(|t| t.available()) {
            let text = render(profile, target);
            assert!(text.contains(derived), "{profile}/{target}: no derived");
            assert!(
                text.contains(speculative),
                "{profile}/{target}: no speculative"
            );
        }
    }
}
