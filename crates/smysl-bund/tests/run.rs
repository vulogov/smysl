//! The runner against a real `bund2` (RFC SMYSL-4 BQ-P2). Every test here needs `BUND2_BIN`, a
//! `bund2` 0.56.0 or later built with `--features smysl`, and passes trivially without it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_bund::{
    eval_library, eval_over_sq, eval_store, BundError, QueryPolicy, QueryValue, Trust,
};
use smysl_core::surface::parse_surface;
use smysl_graph::Store;

fn bin() -> Option<PathBuf> {
    let b = std::env::var_os("BUND2_BIN").map(PathBuf::from);
    if b.is_none() {
        assert!(
            std::env::var_os("SMYSL_BUND_REQUIRED").is_none(),
            "SMYSL_BUND_REQUIRED is set and BUND2_BIN is not"
        );
        eprintln!("BUND2_BIN not set; skipped");
    }
    b
}

/// In CI (`SMYSL_BUND_REQUIRED`) the sandbox must be there, so that the suite below runs in it
/// rather than passing trusted.
#[test]
fn the_sandbox_is_here_when_required() {
    if std::env::var_os("SMYSL_BUND_REQUIRED").is_some() {
        smysl_bund::sandbox::available().expect("the sandbox");
    }
}

fn trusted(bin: &Path) -> QueryPolicy {
    QueryPolicy::default()
        .with_binary(bin)
        .with_trust(Trust::Trusted)
}

fn f1() -> Store {
    let src = std::fs::read_to_string("../../fixtures/corpus/F1-incident.smy").unwrap();
    Store::from_records(parse_surface(&src).expect("fixture parses").records)
}

fn get<'a>(v: &'a QueryValue, key: &str) -> &'a QueryValue {
    match v {
        QueryValue::Map(m) => m.get(key).unwrap_or_else(|| panic!("no `{key}` in {m:?}")),
        other => panic!("not a map: {other:?}"),
    }
}

#[test]
fn a_query_over_a_store_returns_its_uids() {
    let Some(bin) = bin() else { return };
    let store = f1();
    let r = eval_store(
        &store,
        ":lib var? \"from library\" smysl.query",
        &trusted(&bin),
    )
    .unwrap();
    let uids = r.value.uids();
    assert!(!uids.is_empty(), "{:?}", r.value);
    assert!(uids.iter().all(|u| store.contains_uid(u)));
}

#[test]
fn a_unit_is_read_by_its_uid() {
    let Some(bin) = bin() else { return };
    let store = f1();
    let unit = store.iter().find_map(|r| match r {
        smysl_core::types::Record::Unit(u) => Some(smysl_core::hash::canonical_uid(u)),
        _ => None,
    });
    let uid = unit.expect("F1 has units");
    let script = format!(":lib var? \"{}\" smysl.get", uid.canonical());
    let r = eval_store(&store, &script, &trusted(&bin)).unwrap();
    assert_eq!(get(&r.value, "uid"), &QueryValue::Uid(uid));
}

#[test]
fn sq_seeds_the_script() {
    let Some(bin) = bin() else { return };
    let store = f1();
    let r = eval_over_sq(&store, "from library", ":seed var? len", &trusted(&bin)).unwrap();
    let QueryValue::Int(n) = r.value else {
        panic!("{:?}", r.value)
    };
    assert!(n > 0);
    let empty = eval_over_sq(
        &store,
        "from library | where lang = xx",
        ":seed var? len",
        &trusted(&bin),
    );
    assert!(
        matches!(empty, Ok(ref r) if r.value == QueryValue::Int(0)),
        "{empty:?}"
    );
}

#[test]
fn equal_stores_answer_equally() {
    let Some(bin) = bin() else { return };
    let store = f1();
    let mut shuffled: Vec<_> = store.iter().cloned().collect();
    shuffled.reverse();
    let other = Store::from_records(shuffled);
    let script = ":lib var? \"from library\" smysl.query";
    let a = eval_store(&store, script, &trusted(&bin)).unwrap();
    let b = eval_store(&other, script, &trusted(&bin)).unwrap();
    assert_eq!(a.value, b.value);
}

#[test]
fn a_refusal_is_classified() {
    let Some(bin) = bin() else { return };
    let r = eval_store(&f1(), ":lib var? \"b3:x\" smysl.get", &trusted(&bin));
    assert!(
        matches!(r, Err(BundError::InvalidScriptOrArgs { ref code, .. }) if code == "SM03"),
        "{r:?}"
    );
    let r = eval_store(
        &f1(),
        ":lib var? \"from library where\" smysl.query",
        &trusted(&bin),
    );
    assert!(matches!(r, Err(BundError::SqQueryRefusal { .. })), "{r:?}");
}

/// §5.5: where the sandbox exists the default policy runs in it; elsewhere it is refused
/// before anything starts.
#[test]
fn sandboxed_runs_where_the_sandbox_exists() {
    let Some(bin) = bin() else { return };
    let policy = QueryPolicy::default().with_binary(bin);
    let r = eval_store(&f1(), ":lib var? smysl.status", &policy);
    match smysl_bund::sandbox::available() {
        Ok(()) => assert!(
            matches!(r, Ok(ref v) if matches!(v.value, QueryValue::Map(_))),
            "{r:?}"
        ),
        Err(_) => assert!(
            matches!(r, Err(BundError::SandboxUnavailable { .. })),
            "{r:?}"
        ),
    }
}

/// The policy a case runs under: the sandbox where there is one.
fn contained(bin: &Path) -> QueryPolicy {
    let p = QueryPolicy::default().with_binary(bin);
    if smysl_bund::sandbox::available().is_ok() {
        p
    } else {
        p.with_trust(Trust::Trusted)
    }
}

/// BQ-P3's adversarial suite, through the public API: each case is stopped and named. The
/// sandbox's own denials, with the gate passed by, are `run::sandbox_tests`.
#[test]
fn each_attack_is_stopped_and_named() {
    let Some(bin) = bin() else { return };
    let policy = contained(&bin);
    for (script, token) in [
        ("\"system.\" \"shell\" + call, !", "call,"),
        ("\"40 2 +\" compile", "compile"),
        ("\"http://127.0.0.1:9/\" url", "url"),
        ("\"/etc/passwd\" file", "file"),
        ("time.now", "time.now"),
        ("[ { \"ls\" sh } ] 0 at", "sh"),
    ] {
        let r = eval_store(&f1(), script, &policy);
        assert!(
            matches!(r, Err(BundError::PolicyViolation { token: Some(ref t), .. }) if t == token),
            "{script}: {r:?}"
        );
    }
    let r = eval_store(&f1(), "true { true } while", &policy.with_max_wall_ms(300));
    assert!(
        matches!(r, Err(BundError::TimeoutExceeded { max_wall_ms: 300 })),
        "{r:?}"
    );
}

const SPIN: &str = ":spinner [ [ [ spin ] ← [ spin ] ] ] kb! var :spinner var? [ spin ] kb.holds";

/// §9: `max_inferences` caps a KB the script makes, and lowers a cap it sets.
#[test]
fn the_inference_cap_stops_a_runaway_query() {
    let Some(bin) = bin() else { return };
    let policy = contained(&bin).with_max_inferences(Some(1_000));
    let r = eval_store(&f1(), SPIN, &policy);
    assert!(
        matches!(r, Err(BundError::InferenceCapReached { ref message }) if message.contains("1000")),
        "{r:?}"
    );
    let own =
        ":s [ [ [ spin ] ← [ spin ] ] ] kb! 50000000 kb.inferences var :s var? [ spin ] kb.holds";
    let r = eval_store(&f1(), own, &policy);
    assert!(
        matches!(r, Err(BundError::InferenceCapReached { ref message }) if message.contains("1000")),
        "{r:?}"
    );
    let r = eval_store(
        &f1(),
        SPIN,
        &policy.with_max_inferences(None).with_max_wall_ms(300),
    );
    assert!(matches!(r, Err(BundError::TimeoutExceeded { .. })), "{r:?}");
}

/// The determinism replay (BQ-P3): every script answers the same over equal stores written in
/// different record orders, and the same each time it runs.
const REPLAY: [&str; 24] = [
    ":lib var? \"from library\" smysl.query",
    ":lib var? \"from library | where kind = claim\" smysl.query",
    ":lib var? smysl.status",
    ":lib var? \"from library\" smysl.kb kb.clauses len",
    ":lib var? \"from library\" smysl.kb [ label L U ] kb.query",
    ":lib var? \"from library\" smysl.kb [ [ unit U claim S ] [ gist U G ] ] kb.query",
    ":lib var? \"from library\" smysl.kb [ rel R _ _ ] kb.holds",
    ":lib var? \"from library\" smysl.kb [ deps U D ] kb.query",
    ":lib var? \"from library\" smysl.kb [ rank U R ] kb.query",
    ":lib var? \"from library\" smysl.kb [ [ label c/regression C ] [ grounds C E ] ] kb.query",
    ":lib var? \"from library\" smysl.kb kb.saturate kb.clauses len",
    ":lib var? \"from library\" smysl.kb [ [ unit U K S ] [ rank S R ] ] kb.query len",
    PATHS_QUERY,
    PATHS_TABLED,
    PATHS_FINDALL,
    PATHS_PROVE,
    PATHS_DIF,
    PLAN_ONCE,
    PLAN_ALL,
    PLAN_EARLIEST,
    PLAN_RESIDUAL,
    SPIN,
    ":k kb.new [ a 1 ] kb.assert [ a 2 ] kb.asserta [ a 1 ] kb.retract [ a X ] kb.query",
    ":lib var? \"from library\" smysl.kb kb.new [ extra x ] kb.assert kb.merge kb.clauses len",
];

const PATHS: &str = "[ [ edge a b ] [ edge b c ] [ edge c d ] [ [ path X Y ] ← [ edge X Y ] ] [ [ path X Z ] ← [ edge X Y ] [ path Y Z ] ] ] kb!";
const PATHS_QUERY: &str = concat!("[ [ edge a b ] [ edge b c ] [ edge c d ] [ [ path X Y ] ← [ edge X Y ] ] [ [ path X Z ] ← [ edge X Y ] [ path Y Z ] ] ] kb!", " [ path a Z ] kb.query");
const PATHS_TABLED: &str = "[ [ edge a b ] [ edge b c ] [ [ up A B ] ← [ up A M ] [ edge M B ] ] [ [ up A B ] ← [ edge A B ] ] ] kb! :up 2 kb.table [ up a Z ] kb.query";
const PATHS_FINDALL: &str = concat!("[ [ edge a b ] [ edge b c ] [ edge c d ] [ [ path X Y ] ← [ edge X Y ] ] [ [ path X Z ] ← [ edge X Y ] [ path Y Z ] ] ] kb!", " [ findall Z [ path a Z ] L ] kb.once");
const PATHS_PROVE: &str = concat!("[ [ edge a b ] [ edge b c ] [ edge c d ] [ [ path X Y ] ← [ edge X Y ] ] [ [ path X Z ] ← [ edge X Y ] [ path Y Z ] ] ] kb!", " [ path a d ] kb.prove");
const PATHS_DIF: &str = concat!("[ [ edge a b ] [ edge b c ] [ edge c d ] [ [ path X Y ] ← [ edge X Y ] ] [ [ path X Z ] ← [ edge X Y ] [ path Y Z ] ] ] kb!", " [ [ edge A _ ] [ edge B _ ] [ dif A B ] ] kb.query");
const PLAN: &str = "[ [ [ schedule [ A B S T ] End ] ← [ ins [ A B S T ] 0 8 ] [ all_different [ A B S T ] ] [ #> B 1 ] [ #>= S [ + A 2 ] ] [ #>= T [ + S 1 ] ] [ #>= T [ + B 3 ] ] [ #>= T [ + A 2 ] ] [ #= End [ + T 1 ] ] [ #<= End 8 ] [ #≠ A 1 ] [ #< A B ] [ labeling [ :ff ] [ A B S T ] ] ] ] kb!";
const PLAN_ONCE: &str = concat!("[ [ [ schedule [ A B S T ] End ] ← [ ins [ A B S T ] 0 8 ] [ all_different [ A B S T ] ] [ #> B 1 ] [ #>= S [ + A 2 ] ] [ #>= T [ + S 1 ] ] [ #>= T [ + B 3 ] ] [ #>= T [ + A 2 ] ] [ #= End [ + T 1 ] ] [ #<= End 8 ] [ #≠ A 1 ] [ #< A B ] [ labeling [ :ff ] [ A B S T ] ] ] ] kb!", " [ schedule P E ] kb.once");
const PLAN_ALL: &str = concat!("[ [ [ schedule [ A B S T ] End ] ← [ ins [ A B S T ] 0 8 ] [ all_different [ A B S T ] ] [ #> B 1 ] [ #>= S [ + A 2 ] ] [ #>= T [ + S 1 ] ] [ #>= T [ + B 3 ] ] [ #>= T [ + A 2 ] ] [ #= End [ + T 1 ] ] [ #<= End 8 ] [ #≠ A 1 ] [ #< A B ] [ labeling [ :ff ] [ A B S T ] ] ] ] kb!", " [ schedule P E ] kb.query len");
const PLAN_EARLIEST: &str = concat!("[ [ [ schedule [ A B S T ] End ] ← [ ins [ A B S T ] 0 8 ] [ all_different [ A B S T ] ] [ #> B 1 ] [ #>= S [ + A 2 ] ] [ #>= T [ + S 1 ] ] [ #>= T [ + B 3 ] ] [ #>= T [ + A 2 ] ] [ #= End [ + T 1 ] ] [ #<= End 8 ] [ #≠ A 1 ] [ #< A B ] [ labeling [ :ff ] [ A B S T ] ] ] ] kb!", " [ [ in E 0 8 ] [ label [ E ] ] [ schedule P E ] ] kb.once");
const PLAN_RESIDUAL: &str = concat!(
    "[ [ [ schedule [ A B S T ] End ] ← [ ins [ A B S T ] 0 8 ] ] ] kb!",
    " [ [ in X 0 8 ] [ #≠ X 4 ] [ #> X 1 ] ] kb.once :_residual get"
);

#[test]
fn equal_stores_answer_equally_every_time() {
    let Some(bin) = bin() else { return };
    let _ = (PATHS, PLAN);
    let policy = contained(&bin).with_max_inferences(Some(10_000));
    let store = f1();
    let mut shuffled: Vec<_> = store.iter().cloned().collect();
    shuffled.reverse();
    shuffled.rotate_left(5);
    let other = Store::from_records(shuffled);
    let mut answered = 0;
    for script in REPLAY {
        let first = eval_store(&store, script, &policy);
        let again = eval_store(&store, script, &policy);
        let shuffled = eval_store(&other, script, &policy);
        let key = |r: &Result<smysl_bund::BundQueryResult, BundError>| match r {
            Ok(v) => Ok(v.value.clone()),
            Err(e) => Err(e.to_string()),
        };
        assert_eq!(key(&first), key(&again), "{script}");
        assert_eq!(key(&first), key(&shuffled), "{script}");
        if std::env::var_os("REPLAY_SHOW").is_some() {
            eprintln!("{script}\n  => {:?}\n", key(&first));
        }
        answered += usize::from(first.is_ok());
    }
    // The capped spinner is the one that must fail.
    assert_eq!(answered, REPLAY.len() - 1);
}

/// §5.4: the loop is stopped at the wall clock, and nothing outlives the call.
#[test]
fn a_loop_is_stopped_and_leaves_nothing() {
    let Some(bin) = bin() else { return };
    // Tests run beside one another, so this script is one no other test writes, and only what
    // runs it is looked for.
    const MARK: &str = "// a_loop_is_stopped_and_leaves_nothing";
    let script = format!("{MARK}\ntrue {{ true }} while");
    let policy = trusted(&bin).with_max_wall_ms(500);
    let started = std::time::Instant::now();
    let r = eval_store(&f1(), &script, &policy);
    assert!(
        matches!(r, Err(BundError::TimeoutExceeded { max_wall_ms: 500 })),
        "{r:?}"
    );
    assert!(started.elapsed().as_millis() < 3_000);
    let ours =
        |d: &Path| std::fs::read_to_string(d.join("query.bund")).is_ok_and(|s| s.contains(MARK));
    let left: Vec<_> = snapshots().into_iter().filter(|d| ours(d)).collect();
    assert!(
        left.is_empty(),
        "a snapshot directory outlived its call: {left:?}"
    );
    // A process still running this script would still have its file: the directory is removed
    // only after the process group is killed.
    let ps = std::process::Command::new("pgrep")
        .args(["-fl", "smysl-bund-.*/query.bund"])
        .output()
        .unwrap();
    let alive: Vec<String> = String::from_utf8_lossy(&ps.stdout)
        .lines()
        .filter(|l| {
            l.split_whitespace()
                .filter(|w| w.ends_with("/query.bund"))
                .any(|w| Path::new(w).parent().is_some_and(ours))
        })
        .map(str::to_string)
        .collect();
    assert!(alive.is_empty(), "a bund2 outlived its call: {alive:?}");
}

#[test]
fn output_past_the_bound_is_stopped() {
    let Some(bin) = bin() else { return };
    let policy = trusted(&bin).with_max_output_bytes(4_096);
    let r = eval_store(&f1(), "true { \"0123456789\" println true } while", &policy);
    assert!(
        matches!(r, Err(BundError::OutputTooLarge { max: 4_096, .. })),
        "{r:?}"
    );
}

fn snapshots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(std::env::temp_dir())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("smysl-bund-"))
        })
        .collect();
    v.sort();
    v
}

/// §4.5: the live library's bytes are what they were.
#[test]
fn a_library_is_read_through_its_snapshot_and_left_as_it_was() {
    let Some(bin) = bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("lib");
    let mut lib = smysl_text::Library::create(&root).unwrap();
    let spec =
        smysl_text::AddSpec::new("txt/1", "notes", "CC0-1.0").with_lang("en".parse().unwrap());
    lib.add(
        &smysl_text::Input::new(b"One line.\nAnother line.\n"),
        &spec,
        &smysl_text::Caps::DEFAULT,
    )
    .unwrap();
    drop(lib);
    let before = hashes(&root);

    let r = eval_library(&root, ":lib var? smysl.texts", &trusted(&bin)).unwrap();
    let texts = get(&r.value, "notes");
    assert!(
        matches!(texts, QueryValue::List(l) if l.len() == 1),
        "{texts:?}"
    );
    assert_eq!(hashes(&root), before);
}

fn hashes(root: &Path) -> BTreeMap<PathBuf, [u8; 32]> {
    let mut out = BTreeMap::new();
    let mut work = vec![root.to_path_buf()];
    while let Some(d) = work.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                work.push(p);
            } else {
                let bytes = std::fs::read(&p).unwrap();
                out.insert(
                    p.strip_prefix(root).unwrap().to_path_buf(),
                    smysl_core::hash::hash_bytes(&bytes),
                );
            }
        }
    }
    out
}

/// §4.3: a library whose shards are primary on redb, which `bund2`'s smysl 1.15 refuses with
/// `SM02`, is read through a snapshot of CBOR logs.
#[cfg(feature = "store-redb")]
#[test]
fn a_library_on_redb_is_read_through_logs() {
    let Some(bin) = bin() else { return };
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("lib");
    let mut lib = smysl_text::Library::create(&root).unwrap();
    let spec =
        smysl_text::AddSpec::new("txt/1", "notes", "CC0-1.0").with_lang("en".parse().unwrap());
    lib.add(
        &smysl_text::Input::new(b"One line.\n"),
        &spec,
        &smysl_text::Caps::DEFAULT,
    )
    .unwrap();
    let shards = lib.shards();
    lib.migrate_primary_all(&shards).unwrap();
    assert!(lib.primary_shards().len() == shards.len() && !shards.is_empty());
    drop(lib);

    let r = eval_library(&root, ":lib var? smysl.texts", &trusted(&bin)).unwrap();
    assert!(
        matches!(get(&r.value, "notes"), QueryValue::List(l) if l.len() == 1),
        "{:?}",
        r.value
    );
}

/// What a call costs, printed: `cargo test -p smysl-bund --test run cost -- --ignored --nocapture`
/// with `BUND2_BIN` set. Not a gate; RFC SMYSL-4 §13 asks for the number.
#[test]
#[ignore]
fn cost() {
    let Some(bin) = bin() else { return };
    let store = f1();
    let mut runs = vec![("trusted", trusted(&bin))];
    if smysl_bund::sandbox::available().is_ok() {
        runs.push(("sandboxed", QueryPolicy::default().with_binary(&bin)));
    }
    for (name, policy) in runs {
        let mut whole = Vec::new();
        let mut process = Vec::new();
        for _ in 0..20 {
            let t = std::time::Instant::now();
            let r = eval_store(&store, ":lib var? \"from library\" smysl.query", &policy).unwrap();
            whole.push(t.elapsed().as_micros());
            process.push(u128::from(r.elapsed_ms) * 1000);
        }
        whole.sort_unstable();
        process.sort_unstable();
        println!(
            "{name}, F1 ({} records): call p50 {} µs, max {} µs; of which the process p50 {} µs",
            store.len(),
            whole[10],
            whole[19],
            process[10]
        );
    }
}
