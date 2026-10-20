//! `smysl measure` (SMYSL-2.6 §4.4, TX-P8 M7).
//!
//! The step's exit names a run of `measure T1,T2,T3,T4` over the five-Bible fixture that adds
//! no unit the second time, and that is asserted here against the store itself rather than
//! against the report: the library is opened after each run and its units counted. The fixture
//! is `fixtures/library/bibles/slice/`, Ruth and Jude in all five editions, added with
//! `text add` as `bible/<edition>/<book>` so that `from collection:bible` is all ten books.
//!
//! Five languages are five analyzer chains, which a scope is refused for unless the recipe
//! names one (`SMY-E489`); the runs name the plain chain, as the M2 to M6 library tests do.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use smysl::text::library::Library;
use smysl::Record;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const PLAIN: &str = "smysl/an/1:surface";
const SCOPE: &str = "from collection:bible";

const EDITIONS: [(&str, &str, &str); 5] = [
    ("deu1912", "deu1912", "de"),
    ("eng-kjv", "eng-kjv", "en"),
    ("fraLSG", "fralsg", "fr"),
    ("russyn", "russyn", "ru"),
    ("spaRV1909", "sparv1909", "es"),
];

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-cmd-measure-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("the binary under test must run")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).to_string()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).to_string()
}

/// The five-Bible fixture as a library: ten books under `bible/`.
fn bibles(name: &str) -> (PathBuf, String) {
    let dir = scratch(name).join("lib");
    let root = dir.to_string_lossy().to_string();
    for (dir_name, alias, lang) in EDITIONS {
        for book in ["08-RUT", "65-JUD"] {
            let file = repo(&format!(
                "fixtures/library/bibles/slice/{dir_name}/{book}.usfm"
            ));
            let alias = format!("bible/{alias}/{}", book.to_lowercase());
            let o = run(&[
                "text",
                "add",
                &file.to_string_lossy(),
                "--reader",
                "usfm/1",
                "--alias",
                &alias,
                "--lang",
                lang,
                "--licence",
                "public-domain",
                "--carry",
                "text",
                "-s",
                &root,
            ]);
            assert!(o.status.success(), "text add {alias}: {}", stderr(&o));
        }
    }
    (dir, root)
}

/// Units and attestations in the library's store.
fn census(root: &Path) -> (usize, usize) {
    let library = Library::open(root).expect("a library");
    let mut units = 0;
    let mut attestations = 0;
    for r in library.store().iter() {
        match r {
            Record::Unit(_) => units += 1,
            Record::Attestation(_) => attestations += 1,
            _ => {}
        }
    }
    (units, attestations)
}

fn measure(root: &str, extra: &[&str]) -> Output {
    let mut args = vec![
        "measure",
        "T1,T2,T3,T4",
        "--scope",
        SCOPE,
        "--analyzer",
        PLAIN,
        // T3's default asks 50,000 characters of every text and a book of Ruth has 13k; set
        // for T3 alone, which leaves T1's, T2's and T4's recipes at their defaults.
        "--min-n",
        "T3=2",
    ];
    args.extend_from_slice(extra);
    args.push(root);
    run(&args)
}

/// The exit: four measures over the five Bibles, written once, and a second run that adds no
/// unit — the store holds the same units after it, and gains only the second run's
/// attestations, one per result.
#[test]
fn a_second_run_adds_no_unit() {
    let (dir, root) = bibles("twice");
    let before = census(&dir);

    let first = measure(&root, &["--at", "1000"]);
    assert!(first.status.success(), "{}", stderr(&first));
    assert!(
        stderr(&first).contains("4 unit(s): 4 new, 0 already in the store"),
        "{}",
        stderr(&first)
    );
    let after_first = census(&dir);
    // Four results and the saved scope they are grounded on.
    assert_eq!(after_first.0, before.0 + 5, "{}", stdout(&first));
    assert_eq!(after_first.1, before.1 + 4);

    let second = measure(&root, &["--at", "2000"]);
    assert!(second.status.success(), "{}", stderr(&second));
    assert!(
        stderr(&second).contains("4 unit(s): 0 new, 4 already in the store"),
        "{}",
        stderr(&second)
    );
    let after_second = census(&dir);
    assert_eq!(after_second.0, after_first.0, "a second run added a unit");
    assert_eq!(after_second.1, after_first.1 + 4, "the run's attestations");
    for id in ["T1", "T2", "T3", "T4"] {
        assert!(
            stdout(&second).contains(&format!("{id} v1  b3:")),
            "{id}: {}",
            stdout(&second)
        );
    }
}

/// `--dry-run` prints the values and writes nothing; `--seed-check` computes each result twice
/// and passes for every pure id this build has; `--json` is one object a parser reads.
#[test]
fn dry_run_seed_check_and_json() {
    let (dir, root) = bibles("dry");
    let before = census(&dir);
    let o = run(&[
        "--seed-check",
        "--json",
        "measure",
        "T1,T2,T3,T4,T5,T6,T7",
        "--scope",
        SCOPE,
        "--analyzer",
        PLAIN,
        "--min-n",
        "2",
        "--dry-run",
        &root,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    assert_eq!(census(&dir), before, "--dry-run wrote");
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).expect("one JSON object");
    assert_eq!(v["dry_run"], true);
    assert_eq!(v["new"], 7);
    let results = v["results"].as_array().unwrap();
    let ids: Vec<&str> = results.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["T1", "T2", "T3", "T4", "T5", "T6", "T7"]);
    for r in results {
        assert_eq!(r["outcome"], "value", "{r}");
        assert!(r["uid"].as_str().unwrap().starts_with("b3:"));
    }
    // T7 over five languages read as one vocabulary: Latin, Cyrillic, and the three Latin-script
    // editions' shared spellings.
    let t7 = &results[6]["values"];
    assert_eq!(t7["jsd:bible/eng-kjv/08-rut|bible/russyn/08-rut"], "1.0000");
}

/// A parameter goes to the ids that take it, or to the one it names; one that no id asked for
/// takes is refused before anything runs, as is a scoped one for an id not asked for. An id
/// the build does not have is a result, not run, beside the ones that ran.
#[test]
fn parameters_are_routed_per_id() {
    let (_dir, root) = bibles("params");
    let o = run(&[
        "--json",
        "measure",
        "T5,T7,T9",
        "--scope",
        SCOPE,
        "--analyzer",
        PLAIN,
        "--level",
        "verse",
        "--min-n",
        "2",
        "--param",
        "top:k=1",
        "--param",
        "T5.order=2",
        "--dry-run",
        &root,
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    let r = v["results"].as_array().unwrap();
    let tops = |x: &serde_json::Value, p: &str| {
        x["values"]
            .as_object()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(p))
            .count()
    };
    assert_eq!(tops(&r[0], "top:"), 1, "top:k went to T5");
    assert_eq!(tops(&r[1], "g2:"), 1, "and to T7");
    assert_eq!(r[2]["outcome"], "not-run");
    assert_eq!(r[2]["code"], "SMY-E489");

    // T5's recipe says order 2, and T7's does not mention an order at all.
    let order_2 = r[0]["recipe"].as_str().unwrap().to_string();
    let o = run(&[
        "--json",
        "measure",
        "T5",
        "--scope",
        SCOPE,
        "--analyzer",
        PLAIN,
        "--level",
        "verse",
        "--min-n",
        "2",
        "--param",
        "top:k=1",
        "--dry-run",
        &root,
    ]);
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).unwrap();
    assert_ne!(v["results"][0]["recipe"].as_str().unwrap(), order_2);

    for (args, why) in [
        (
            vec!["--param", "symbols=terms"],
            "none of the ids asked for takes `symbols`",
        ),
        (
            vec!["--param", "T1.order=2"],
            "T1 is not one of the ids asked for",
        ),
        (vec!["--param", "T5.symbols=terms"], "T5 takes no `symbols`"),
        (
            vec!["--min-n", "T3=2"],
            "T3 is not one of the ids asked for",
        ),
        (vec!["--level", "stanza"], "--level is part, chapter"),
    ] {
        let mut a = vec!["measure", "T5,T7", "--scope", SCOPE, "--dry-run"];
        a.extend(args);
        a.push(&root);
        let o = run(&a);
        assert_eq!(o.status.code(), Some(2), "{a:?}");
        assert!(stderr(&o).contains(why), "{a:?}: {}", stderr(&o));
    }
}

/// A scope whose texts choose five analyzer chains is refused unless one is named, and a path
/// that is not a library is refused before anything is read.
#[test]
fn refusals_before_anything_runs() {
    let (_dir, root) = bibles("refuse");
    let o = run(&["measure", "T1", "--scope", SCOPE, "--dry-run", &root]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("SMY-E489"), "{}", stderr(&o));

    let plain = scratch("plain").join("store.smy");
    std::fs::write(&plain, "").unwrap();
    let o = run(&["measure", "T1", "--scope", SCOPE, &plain.to_string_lossy()]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("not a library"), "{}", stderr(&o));
}

/// Three claims drawn from each book's first part, and one pair claimed the same: what the
/// unit measures (TX-P9 N1) have to count in a library that has text and units.
fn with_units(root: &Path) -> usize {
    use smysl::{canonical_uid, KernelType, Relation, Status, UnitCoreBuilder};
    use smysl_core::types::{RelKind, SourceKind, SourceRef};

    let mut library = Library::open(root).expect("a library");
    let mut parts: Vec<_> = library
        .store()
        .manifests()
        .map(|(_, m)| m.parts[0].tid)
        .collect();
    parts.sort();
    let mut records = Vec::new();
    let mut uids = Vec::new();
    for (k, tid) in parts.iter().enumerate() {
        for i in 0..3 {
            let core = UnitCoreBuilder::new(
                KernelType::Claim,
                format!("claim {i} of book {k}"),
                Status::Speculative,
            )
            .source(SourceRef::new(SourceKind::Doc, tid.canonical()))
            .build()
            .unwrap();
            uids.push(canonical_uid(&core));
            records.push(Record::Unit(core));
        }
    }
    records.push(Record::Relation(Relation::new(
        RelKind::Extension(smysl::schemas::text::SAME_AS.into()),
        uids[0],
        uids[1],
    )));
    library.append_records(&records).expect("appends");
    uids.len()
}

/// The unit measures through the command: a scope's units counted, A2 under the policy
/// `--policy` names and refused without one (`SMY-E481`), A1 refused without `--profile`,
/// a text measure and a unit measure in one run, A8 refused over five languages and run over
/// one, and the flags' own refusals.
#[test]
fn unit_measures_count_a_scopes_units() {
    let (dir, root) = bibles("units");
    let n = with_units(&dir);
    let json = |extra: &[&str]| -> serde_json::Value {
        let mut a = vec!["--json", "measure"];
        a.extend_from_slice(extra);
        a.extend(["--scope", SCOPE, "--dry-run", &root]);
        let o = run(&a);
        assert!(o.status.success(), "{a:?}: {}", stderr(&o));
        serde_json::from_str(&stdout(&o)).expect("one JSON object")
    };

    let v = json(&["A2,A4,A5,A7,A10,A9", "--policy", "strict"]);
    let r = v["results"].as_array().unwrap();
    for x in r {
        assert_eq!(x["outcome"], "value", "{x}");
        assert_eq!(x["n"]["units"], n, "{x}");
    }
    assert_eq!(r[0]["values"]["redundancy"], format!("0.0333 (1/{n})"));
    assert_eq!(r[1]["values"]["status:speculative"], n.to_string());

    let v = json(&["A2,A1"]);
    assert_eq!(v["results"][0]["code"], "SMY-E481");
    assert_eq!(v["results"][1]["code"], "SMY-E489");
    let v = json(&["A1", "--profile", "default", "--analyzer", PLAIN]);
    assert_eq!(v["results"][0]["outcome"], "value", "{v}");
    assert_eq!(v["results"][0]["n"]["placed"], n);

    // One run, a text and a unit measure: the text is read for T1 and not for A4.
    let v = json(&["T1,A4", "--analyzer", PLAIN, "--min-n", "T1=2"]);
    let ids: Vec<&str> = v["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["outcome"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["value", "value"]);

    // Units drawn from a text carry no time of their own, so a time order is refused.
    let v = json(&["A3", "--policy", "strict", "--param", "order=time"]);
    assert_eq!(v["results"][0]["code"], "SMY-E482");

    // A8 budgets by bytes, and the scope's units are drawn from five languages (`SMY-E407`);
    // one translation's units are one language, and its steps are a `--param`.
    let v = json(&["A8"]);
    assert_eq!(v["results"][0]["code"], "SMY-E407", "{v}");
    let mut a = vec!["--json", "measure", "A8", "--param", "steps=4"];
    a.extend([
        "--scope",
        "from collection:\"bible/eng-kjv\"",
        "--dry-run",
        &root,
    ]);
    let o = run(&a);
    assert!(o.status.success(), "{a:?}: {}", stderr(&o));
    let v: serde_json::Value = serde_json::from_str(&stdout(&o)).expect("one JSON object");
    assert_eq!(v["results"][0]["outcome"], "value", "{v}");
    assert_eq!(v["results"][0]["n"]["points"], 4, "{v}");

    for (bad, why) in [
        (vec!["A2", "--policy", "loose"], "--policy is strict"),
        (vec!["A2", "--policy", "attested:0"], "--policy is strict"),
        (vec!["A10", "--axis", "known"], "--axis is said"),
    ] {
        let mut a = vec!["measure"];
        a.extend(bad);
        a.extend(["--scope", SCOPE, "--dry-run", &root]);
        let o = run(&a);
        assert_eq!(o.status.code(), Some(2), "{a:?}");
        assert!(stderr(&o).contains(why), "{a:?}: {}", stderr(&o));
    }
}

/// The channel measures through the command (TX-P9 N4): `--channel` names the target, the
/// scope is the source; one KJV claim is claimed the same as one Synodal claim, so one target
/// unit of six is supported; the result is written grounded on both saved scopes and a rerun
/// finds it; C5 reads both texts; and the refusals — a target equal to the source, a channel
/// measure with no target, a target no id reads.
#[test]
fn channel_measures_compare_a_scope_with_its_target() {
    use smysl::{Relation, Status};
    use smysl_core::types::RelKind;

    let (dir, root) = bibles("channel");
    with_units(&dir);
    let kjv = "from collection:\"bible/eng-kjv\"";
    let syn = "from collection:\"bible/russyn\"";
    {
        let mut library = Library::open(&dir).expect("a library");
        let side = |alias: &str| {
            let tids: Vec<_> = library
                .store()
                .manifests()
                .filter(|(_, m)| m.alias.starts_with(alias))
                .flat_map(|(_, m)| m.parts.iter().map(|p| p.tid.canonical()))
                .collect();
            let mut u: Vec<_> = library
                .store()
                .units()
                .filter(|(_, x)| {
                    x.core.status == Status::Speculative
                        && x.core
                            .source
                            .as_ref()
                            .is_some_and(|s| tids.contains(&s.reference))
                })
                .map(|(uid, _)| *uid)
                .collect();
            u.sort();
            u[0]
        };
        let (a, b) = (side("bible/eng-kjv/"), side("bible/russyn/"));
        library
            .append_records(&[Record::Relation(Relation::new(
                RelKind::Extension(smysl::schemas::text::SAME_AS.into()),
                a,
                b,
            ))])
            .expect("appends");
    }
    let json = |extra: &[&str]| -> (Output, serde_json::Value) {
        let mut a = vec!["--json", "measure"];
        a.extend_from_slice(extra);
        a.push(&root);
        let o = run(&a);
        assert!(o.status.success(), "{a:?}: {}", stderr(&o));
        let v = serde_json::from_str(&stdout(&o)).expect("one JSON object");
        (o, v)
    };

    let args = [
        "C1,C2,C3,C4",
        "--policy",
        "strict",
        "--scope",
        kjv,
        "--channel",
        syn,
    ];
    let mut dry = args.to_vec();
    dry.push("--dry-run");
    let (_, v) = json(&dry);
    assert_eq!(v["channel"], syn);
    let r = v["results"].as_array().unwrap();
    for x in r {
        assert_eq!(x["outcome"], "value", "{x}");
    }
    let classes = r[0]["n"]["source:classes"].as_u64().unwrap();
    assert_eq!(
        r[0]["values"]["survival"]
            .as_str()
            .unwrap()
            .split_once(' ')
            .unwrap()
            .1,
        format!("(1/{classes})")
    );
    assert_eq!(r[1]["values"]["support"], "0.1667 (1/6)");
    assert_eq!(r[2]["n"]["pairs"], 1);

    let (o, v) = json(&args);
    assert!(stderr(&o).contains("4 new"), "{}", stderr(&o));
    assert!(v["channel_basis"].is_string(), "{v}");
    let (o, _) = json(&args);
    assert!(
        stderr(&o).contains("4 already in the store"),
        "{}",
        stderr(&o)
    );
    {
        let library = Library::open(&dir).expect("a library");
        let uid = smysl_core::Uid::parse(v["results"][0]["uid"].as_str().unwrap()).unwrap();
        let unit = library.store().get(&uid).expect("written");
        let grounds: Vec<String> = unit.core.grounds.iter().map(|g| g.canonical()).collect();
        let mut want = vec![
            v["basis"].as_str().unwrap().to_string(),
            v["channel_basis"].as_str().unwrap().to_string(),
        ];
        want.sort();
        let mut grounds = grounds;
        grounds.sort();
        assert_eq!(grounds, want, "a channel result rests on both scopes");
    }

    let (_, v) = json(&["C5", "--scope", kjv, "--channel", syn, "--dry-run"]);
    let x = &v["results"][0];
    assert_eq!(x["outcome"], "value", "{x}");
    assert!(x["values"]["T2:ratio"].is_string(), "{x}");
    assert_eq!(x["flags"][0], "t3-not-run", "{x}");

    let (_, v) = json(&[
        "C1",
        "--policy",
        "strict",
        "--scope",
        kjv,
        "--channel",
        kjv,
        "--dry-run",
    ]);
    assert_eq!(v["results"][0]["code"], "SMY-E489", "{v}");
    let (_, v) = json(&["C1", "--policy", "strict", "--scope", kjv, "--dry-run"]);
    assert_eq!(v["results"][0]["code"], "SMY-E489", "{v}");
    let o = run(&[
        "measure",
        "A2",
        "--policy",
        "strict",
        "--scope",
        kjv,
        "--channel",
        syn,
        "--dry-run",
        &root,
    ]);
    assert_eq!(o.status.code(), Some(2));
    assert!(stderr(&o).contains("--channel"), "{}", stderr(&o));
}

/// TX-P9 N5: the dialogue measures from the command line. D5 reads the text in windows and is
/// refused at any other level; D2 over units nobody spoke has nothing to count; D3 without a
/// policy is refused before anything runs.
#[test]
fn dialogue_measures_run_from_the_command_line() {
    let (dir, root) = bibles("dialogue");
    with_units(&dir);
    let kjv = "from collection:\"bible/eng-kjv\"";
    let json = |extra: &[&str]| -> serde_json::Value {
        let mut a = vec!["--json", "measure"];
        a.extend_from_slice(extra);
        a.push(&root);
        let o = run(&a);
        assert!(o.status.success(), "{a:?}: {}", stderr(&o));
        serde_json::from_str(&stdout(&o)).expect("one JSON object")
    };
    let v = json(&["D5", "--scope", kjv, "--level", "window:200", "--dry-run"]);
    let x = &v["results"][0];
    assert_eq!(x["outcome"], "value", "{x}");
    assert!(x["values"]["jsd:mean"].is_string(), "{x}");
    assert!(x["n"]["pairs"].as_u64().unwrap() > 0, "{x}");

    let v = json(&["D5", "--scope", kjv, "--dry-run"]);
    assert_eq!(v["results"][0]["code"], "SMY-E489", "{v}");
    let v = json(&["D2", "--scope", kjv, "--dry-run"]);
    assert_eq!(v["results"][0]["code"], "SMY-W485", "{v}");
    let v = json(&["D3", "--scope", kjv, "--dry-run"]);
    assert_eq!(v["results"][0]["code"], "SMY-E481", "{v}");
}
