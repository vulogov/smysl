//! `fixtures/tel/diff/` (RFC SMYSL-3 §8, §16 C-Tel-Produce): what the canonical diff makes of
//! eight pairs of Deployment snapshots, by uid.
//!
//! Every case is `base.json` as S₀ and `<case>/s1.json` as S₁, under `profile.hjson`. The
//! manifest and the instant are fixed in `expected.tsv`'s header, and S₁'s tid is computed
//! from S₁'s part text (§8.3), so a second implementation reproduces every uid from the files
//! alone: the part text, its tid, then each change's payload and source. The object is the
//! Deployment's own series unit, `k8s` scheme, key `apiVersion/kind/namespace/name#uid`
//! (§6), from S₀.
//!
//! `SMYSL_BLESS=1` rewrites `expected.tsv`. That is a decision: C-Tel-Produce is defined by it.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::schemas::tel;
use smysl_core::surface::hjson::HValue;
use smysl_core::{canonical_uid, hash_bytes, Mid, PartText, UnitCore};
use smysl_tel::diff::{diff, part_text, Snapshot};
use smysl_tel::{json, Profile};

const CASES: &[&str] = &[
    "image-bump",
    "scale",
    "heartbeat-only",
    "ready-drop",
    "reorder",
    "labels",
    "recreate",
    "env-duplicate",
    "spelled-numbers",
];

/// 2026-01-16T10:00:00Z.
const AT_MS: u64 = 1_768_557_600_000;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/tel/diff")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(dir().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn field(u: &UnitCore, k: &str) -> String {
    let o = smysl_core::surface::payload_to_object(u.payload.as_ref().unwrap()).unwrap();
    match o.get(k).map(|v| &v.value) {
        Some(HValue::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn build() -> String {
    let profile = Profile::parse(&read("profile.hjson")).expect("the fixture's profile");
    // A fixed manifest id: the fixture holds no manifest, and the id is an input like the
    // instant. Derived from a stated string, so a port can derive it too.
    let mid = Mid::from_bytes(hash_bytes(b"fixtures/tel/diff"));
    let s0 = json::parse(&read("base.json")).expect("base.json");
    let m = s0.get("metadata").unwrap();
    let key = format!(
        "apps/v1/Deployment/{}/{}#{}",
        m.get("namespace").and_then(json::Json::as_str).unwrap(),
        m.get("name").and_then(json::Json::as_str).unwrap(),
        m.get("uid").and_then(json::Json::as_str).unwrap(),
    );
    let object = canonical_uid(&tel::series("k8s", &key).unwrap());

    let mut out = String::new();
    writeln!(
        out,
        "# RFC SMYSL-3 §8: the canonical diff of base.json against each case's s1.json"
    )
    .unwrap();
    writeln!(
        out,
        "# under profile.hjson. Written by crates/smysl-tel/tests/diff_fixture.rs;"
    )
    .unwrap();
    writeln!(out, "# SMYSL_BLESS=1 rewrites it.").unwrap();
    writeln!(out, "profile\t{}", profile.hash_text()).unwrap();
    writeln!(
        out,
        "mid\t{}\tblake3(\"fixtures/tel/diff\")",
        mid.canonical()
    )
    .unwrap();
    writeln!(out, "at_ms\t{AT_MS}").unwrap();
    writeln!(out, "object\t{}\tseries k8s {key}", object.canonical()).unwrap();
    let (base_part, _) = part_text(&s0, &profile);
    writeln!(
        out,
        "base\tpart\t{}",
        PartText::new(base_part.into_bytes()).tid.canonical()
    )
    .unwrap();
    for case in CASES {
        let doc = json::parse(&read(&format!("{case}/s1.json"))).expect(case);
        let (text, _) = part_text(&doc, &profile);
        let tid = PartText::new(text.into_bytes()).tid;
        writeln!(out, "{case}\tpart\t{}", tid.canonical()).unwrap();
        let s1 = Snapshot {
            doc: &doc,
            tid: &tid,
            mid: &mid,
            at_ms: AT_MS,
        };
        let d = diff(&s0, &s1, &profile, Some(&object));
        for u in &d.units {
            let what = if u.schema.as_str() == tel::RECREATE {
                format!(
                    "recreate\t{}",
                    u.source
                        .as_ref()
                        .unwrap()
                        .reference
                        .split('#')
                        .nth(1)
                        .unwrap()
                )
            } else {
                format!("{}\t{}", field(u, "tel:op"), field(u, "tel:pointer"))
            };
            writeln!(
                out,
                "{case}\tunit\t{}\t{what}",
                canonical_uid(u).canonical()
            )
            .unwrap();
        }
        for n in &d.notes {
            writeln!(out, "{case}\tnote\t{}\t{}", n.code(), n.pointer()).unwrap();
        }
        writeln!(out, "{case}\trouted\t{}", d.routed.len()).unwrap();
    }
    out
}

#[test]
fn the_diff_fixture_is_current() {
    let built = build();
    let path = dir().join("expected.tsv");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &built).expect("writing expected.tsv");
    }
    let on_disk = std::fs::read_to_string(&path).expect("fixtures/tel/diff/expected.tsv");
    assert_eq!(
        on_disk, built,
        "the canonical diff no longer produces fixtures/tel/diff/expected.tsv; SMYSL_BLESS=1 \
         rewrites it, and that redefines C-Tel-Produce"
    );
}

/// What the cases are for, said as assertions rather than left to a reader of the TSV.
#[test]
fn each_case_says_what_it_is_for() {
    let t = build();
    let units = |case: &str| {
        t.lines()
            .filter(|l| l.starts_with(&format!("{case}\tunit\t")))
            .count()
    };
    let notes = |case: &str| {
        t.lines()
            .filter(|l| l.starts_with(&format!("{case}\tnote\t")))
            .count()
    };
    let part = |case: &str| {
        t.lines()
            .find(|l| l.starts_with(&format!("{case}\tpart\t")))
            .unwrap()
            .rsplit('\t')
            .next()
            .unwrap()
            .to_string()
    };
    assert_eq!(units("image-bump"), 1);
    assert_eq!(units("scale"), 1);
    // Noise only: nothing changed, and the part is the base's to the byte.
    assert_eq!(units("heartbeat-only"), 0);
    assert_eq!(
        part("heartbeat-only"),
        part("base"),
        "noise does not reach the part"
    );
    // A gauge moved: no unit, but the part is a new part, because a gauge is not noise.
    assert_eq!(units("ready-drop"), 0);
    assert_ne!(part("ready-drop"), part("base"));
    // Keyed arrays in another order are the same object.
    assert_eq!(units("reorder"), 0);
    assert_eq!(part("reorder"), part("base"));
    // `3.0`, `1e1` and `8.08E3` are 3, 10 and 8080.
    assert_eq!(units("spelled-numbers"), 0);
    assert_eq!(part("spelled-numbers"), part("base"));
    assert_eq!(units("labels"), 2, "a changed label and a new one");
    assert!(t.contains("recreate\tunit\t") && t.contains("\trecreate\t/metadata/uid"));
    assert_eq!(notes("env-duplicate"), 1);
    assert!(
        t.contains("env-duplicate\tnote\tSMY-W608\t/spec/template/spec/containers/checkout/env")
    );
}
