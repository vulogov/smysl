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
        eprintln!("BUND2_BIN not set; skipped");
    }
    b
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

#[test]
fn sandboxed_is_refused_until_the_sandbox_exists() {
    let Some(bin) = bin() else { return };
    let policy = QueryPolicy::default().with_binary(bin);
    assert!(matches!(
        eval_store(&f1(), ":lib var? smysl.status", &policy),
        Err(BundError::SandboxUnavailable { .. })
    ));
}

/// §5.4: the loop is stopped at the wall clock, and nothing outlives the call.
#[test]
fn a_loop_is_stopped_and_leaves_nothing() {
    let Some(bin) = bin() else { return };
    let policy = trusted(&bin).with_max_wall_ms(500);
    let started = std::time::Instant::now();
    let r = eval_store(&f1(), "true { true } while", &policy);
    assert!(
        matches!(r, Err(BundError::TimeoutExceeded { max_wall_ms: 500 })),
        "{r:?}"
    );
    assert!(started.elapsed().as_millis() < 3_000);
    // Other tests run beside this one, so only this script's directory is looked for.
    let left: Vec<_> = snapshots()
        .into_iter()
        .filter(|d| {
            std::fs::read_to_string(d.join("query.bund"))
                .is_ok_and(|s| s.contains("true { true } while"))
        })
        .collect();
    assert!(
        left.is_empty(),
        "a snapshot directory outlived its call: {left:?}"
    );
    let ps = std::process::Command::new("pgrep")
        .args(["-f", "smysl-bund-.*/query.bund"])
        .output()
        .unwrap();
    assert!(
        ps.stdout.is_empty(),
        "a bund2 outlived its call: {}",
        String::from_utf8_lossy(&ps.stdout)
    );
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
    let policy = trusted(&bin);
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
        "F1 ({} records): call p50 {} µs, max {} µs; of which the process p50 {} µs",
        store.len(),
        whole[10],
        whole[19],
        process[10]
    );
}
