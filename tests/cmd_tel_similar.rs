//! `tel similar EPISODE` (RFC SMYSL-3 §21, TL-P9 step 5), against the binary.
//!
//! A library of three episodes, grouped by `smysl_tel::detect::episode` from alert units: two
//! of one shape (four alerts in common of five) and one of another. The two are each other's
//! nearest, the third is last, and the command writes nothing.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use smysl::schemas::tel;
use smysl::stage::{prepare_declared, Attest};
use smysl::surface::hjson::{HObject, HValue, Spanned};
use smysl::tel::detect::episode;
use smysl::text::library::Library;
use smysl::{canonical_uid, AgentId, Hlc, Rung, SchemaId, SourceKind, SourceRef, Status};
use smysl::{Uid, UnitCore, UnitCoreBuilder};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const T0: u64 = 1_768_557_600_000;
const H: u64 = 3_600_000;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-similar-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary runs")
}

fn alert(key: &str, t: u64) -> UnitCore {
    let span = smysl::Span::new(0, 0);
    let mut o = HObject::new();
    for (k, v) in [
        ("tel:system", "alertmanager"),
        ("tel:key", key),
        ("tel:state", "firing"),
    ] {
        o.insert(
            Spanned::new(k.to_string(), span),
            Spanned::new(HValue::Str(v.into()), span),
        );
    }
    UnitCoreBuilder::new(
        SchemaId::Extension(tel::ALERT.to_string()),
        format!("alert {key}"),
        Status::Cited,
    )
    .source(SourceRef::new(SourceKind::Tool, "alertmanager").observed_at(t))
    .payload(smysl::surface::object_to_payload(&o).expect("a payload"))
    .build()
    .expect("an alert")
}

/// The library, and its three episodes in time order.
fn library(dir: &Path) -> Vec<Uid> {
    let shapes: [&[&str]; 3] = [
        &[
            "HighErrorRate",
            "HighLatency",
            "PodCrashLooping",
            "OOMKilled",
            "Watchdog",
        ],
        &[
            "HighErrorRate",
            "HighLatency",
            "PodCrashLooping",
            "OOMKilled",
            "TargetDown",
        ],
        &["DiskFull", "NodeNotReady", "KubeletDown"],
    ];
    let mut units = Vec::new();
    for (i, keys) in shapes.iter().enumerate() {
        let t = T0 + i as u64 * 6 * H;
        for (k, key) in keys.iter().enumerate() {
            units.push(alert(key, t + k as u64 * 1_000));
        }
    }
    let eps = episode::run(&units, T0, T0 + 24 * H, &episode::Params::default());
    assert_eq!(eps.len(), 3);
    let mut all = units.clone();
    let mut relations = Vec::new();
    for e in &eps {
        all.push(e.unit.clone());
        relations.extend(e.relations.iter().cloned());
    }
    let mut lib = Library::create(&dir.join("lib")).expect("a library");
    let agent = AgentId::new("tool:smysl-tel").unwrap();
    let attest = Attest::new(
        agent.clone(),
        Rung::Computed,
        Hlc::new(T0 + 24 * H, 0, agent),
    );
    let staged = prepare_declared(
        lib.store(),
        all,
        relations,
        BTreeMap::new(),
        vec![tel::decl()],
        &attest,
    );
    assert!(!staged.has_errors(), "{:?}", staged.report.diagnostics);
    lib.append_records(&staged.records()).expect("the append");
    eps.iter().map(|e| canonical_uid(&e.unit)).collect()
}

#[test]
fn the_episode_of_one_shape_is_nearest_and_nothing_is_written() {
    let d = scratch("rank");
    let eps = library(&d);
    let before = std::fs::read_dir(d.join("lib")).unwrap().count();
    let o = run(&d, &["tel", "similar", &eps[0].canonical(), "-s", "lib"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let out = String::from_utf8_lossy(&o.stdout);
    let similar: Vec<&str> = out.lines().filter(|l| l.starts_with("similar")).collect();
    assert_eq!(similar.len(), 2, "{out}");
    assert!(similar[0].contains(&eps[1].canonical()), "{out}");
    assert!(similar[0].contains("jaccard 4/6"), "{out}");
    assert!(
        similar[1].contains(&eps[2].canonical()) && similar[1].contains("jaccard 0/8"),
        "{out}"
    );
    assert!(out.contains("tel/minhash/1?k=128&seed=0"), "{out}");
    assert_eq!(std::fs::read_dir(d.join("lib")).unwrap().count(), before);
    // Another seed is another estimate of the same index.
    let o = run(
        &d,
        &[
            "tel",
            "similar",
            &eps[0].canonical(),
            "--seed",
            "9",
            "-s",
            "lib",
        ],
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("jaccard 4/6"));
}

#[test]
fn what_is_not_an_episode_is_said() {
    let d = scratch("usage");
    let eps = library(&d);
    let _ = eps;
    assert_eq!(
        run(&d, &["tel", "similar", "nope", "-s", "lib"])
            .status
            .code(),
        Some(2)
    );
    let o = run(
        &d,
        &[
            "tel",
            "similar",
            &Uid::from_bytes([3; 32]).canonical(),
            "-s",
            "lib",
        ],
    );
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn a_similarity_recipe_is_read_printed_and_checked() {
    let d = scratch("recipe");
    let eps = library(&d);
    let ep = eps[0].canonical();
    let o = run(
        &d,
        &[
            "tel",
            "similar",
            &ep,
            "--similarity",
            "tel/minhash/2?seed=4&kinds=on&templates=novel",
            "-s",
            "lib",
        ],
    );
    assert_eq!(o.status.code(), Some(0));
    let out = String::from_utf8_lossy(&o.stdout);
    // The text written in full; alerts have no kind, so the ranking is the same sets'.
    assert!(
        out.contains("tel/minhash/2?k=128&seed=4&templates=novel&rise=4&kinds=on"),
        "{out}"
    );
    let first = out.lines().find(|l| l.starts_with("similar")).unwrap();
    assert!(
        first.contains(&eps[1].canonical()) && first.contains("jaccard 4/6"),
        "{out}"
    );
    // `min=` (1.22) is read and printed when it is not 1.
    let o = run(
        &d,
        &[
            "tel",
            "similar",
            &ep,
            "--similarity",
            "tel/minhash/2?min=2",
            "-s",
            "lib",
        ],
    );
    assert_eq!(o.status.code(), Some(0));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        out.contains("tel/minhash/2?k=128&seed=0&templates=overlap&kinds=off&min=2"),
        "{out}"
    );
    for bad in [
        &["--similarity", "tel/minhash/2?min=0"][..],
        &["--similarity", "tel/minhash/2?rise=2"][..],
        &["--similarity", "tel/minhash/9"][..],
        &["--similarity", "tel/minhash/2", "--seed", "1"][..],
    ] {
        let mut args = vec!["tel", "similar", &ep, "-s", "lib"];
        args.extend_from_slice(bad);
        assert_eq!(run(&d, &args).status.code(), Some(2), "{bad:?}");
    }
}
