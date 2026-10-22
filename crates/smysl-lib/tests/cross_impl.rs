//! The cross-implementation harness of SMYSL-2.7 §5.4, TX-P11 step 11.
//!
//! The step's exit: all three implementations verify every export. Each of `python/`,
//! `nodejs/` and `go/` has a `record_set_digest`, an export-index reader and a `verify_export`,
//! and writes what it produces for the Rust to read. Here:
//!
//! - **The digest vectors** in `fixtures/wire/rsd/` are the Rust's. They are rebuilt and
//!   compared, and `SMYSL_BLESS=1` rewrites them, as `gen_library_fixtures` does for the wire.
//! - **Every implementation verifies every export.** F1–F13, F10 and F14 under {`single`,
//!   `by-view`} × {`full`, `ref`}, and the five Bibles with their text, are exported here and
//!   verified by each implementation, which must find every file and digest as the index says
//!   and the collection's digest equal to the Rust's.
//! - **What each implementation writes** — every wire fixture re-encoded by its own encoder,
//!   and a small store it built — is imported here, and P2 holds for each file.
//!
//! An implementation whose toolchain is absent is skipped with a message, unless
//! `SMYSL_XI_REQUIRED` is set, as the GE-T15 job sets it, when its absence fails the test.

use std::path::{Path, PathBuf};
use std::process::Command;

use smysl_core::{to_cbor, Carry, Record};
use smysl_graph::Store;
use smysl_lib::{
    export_library, export_store, import, p2, snapshot, write_collection, Closure, Collection,
    ExportOptions, ImportOptions, Input, Layout, Property, Selection, With,
};
use smysl_text::Library;

mod common;

use common::repo;

fn scratch(name: &str) -> PathBuf {
    common::dir("xi", name)
}

/// The digest vectors: an empty set, F13 once, and F13 twice in another order.
#[test]
fn the_rsd_vectors_are_the_rust_digest() {
    let dir = repo().join("fixtures/wire/rsd");
    let f13 = common::fixture("fixtures/corpus/F13-canon.smy");
    let mut repeats: Vec<Record> = f13.iter().rev().cloned().collect();
    repeats.extend(f13.iter().cloned());
    let cases: Vec<(&str, Vec<Record>)> = vec![
        ("empty.cbor", Vec::new()),
        ("f13.cbor", f13),
        ("repeats.cbor", repeats),
    ];
    let mut json = String::from("[\n");
    for (i, (file, records)) in cases.iter().enumerate() {
        let bytes: Vec<u8> = records.iter().flat_map(to_cbor).collect();
        let rsd = smysl_graph::record_set_digest(records);
        let hex: String = rsd.iter().map(|b| format!("{b:02x}")).collect();
        json.push_str(&format!(
            "  {{\"file\": \"{file}\", \"records\": {}, \"rsd\": \"{hex}\"}}{}\n",
            records.len(),
            if i + 1 < cases.len() { "," } else { "" }
        ));
        if std::env::var_os("SMYSL_BLESS").is_some() {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(file), &bytes).unwrap();
        } else {
            assert_eq!(
                std::fs::read(dir.join(file)).unwrap_or_default(),
                bytes,
                "{file}: rebuild with SMYSL_BLESS=1"
            );
        }
    }
    json.push_str("]\n");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(dir.join("cases.json"), &json).unwrap();
    } else {
        assert_eq!(
            std::fs::read_to_string(dir.join("cases.json")).unwrap_or_default(),
            json,
            "cases.json: rebuild with SMYSL_BLESS=1"
        );
    }
}

/// One implementation: how to ask it to verify a collection and to write what it produces.
struct Impl {
    name: &'static str,
    program: &'static str,
    /// The arguments before the action, and the directory the command runs in.
    prefix: Vec<String>,
    cwd: PathBuf,
}

impl Impl {
    fn run(&self, action: &str, dir: &Path) -> (bool, String) {
        let out = Command::new(self.program)
            .args(&self.prefix)
            .arg(action)
            .arg(dir)
            .current_dir(&self.cwd)
            .output()
            .unwrap_or_else(|e| panic!("{}: {e}", self.name));
        (
            out.status.success(),
            format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            ),
        )
    }
}

fn present(program: &str, arg: &str) -> bool {
    Command::new(program)
        .arg(arg)
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The implementations whose toolchain is here, or a failure for one that is not when
/// `SMYSL_XI_REQUIRED` says they all must be.
fn implementations() -> Vec<Impl> {
    let required = std::env::var_os("SMYSL_XI_REQUIRED").is_some();
    let mut out = Vec::new();
    for (name, program, probe, prefix, cwd) in [
        // `-I`, so nothing in the environment or the working directory is imported, and the
        // package directory put on the path by hand.
        (
            "python",
            "python3",
            "--version",
            vec![
                "-I".into(),
                "-c".into(),
                "import sys; sys.path.insert(0, '.'); from smysl.export import main; \
                 sys.exit(main(sys.argv[1:]))"
                    .into(),
            ],
            repo().join("python"),
        ),
        (
            "nodejs",
            "node",
            "--version",
            vec!["src/export.js".into()],
            repo().join("nodejs"),
        ),
        (
            "go",
            "go",
            "version",
            vec!["run".into(), "./cmd/xi".into()],
            repo().join("go"),
        ),
    ] {
        if present(program, probe) {
            out.push(Impl {
                name,
                program,
                prefix,
                cwd,
            });
        } else if required {
            panic!("{name}: `{program}` is absent and SMYSL_XI_REQUIRED is set");
        } else {
            eprintln!("cross_impl: skipping {name}: `{program}` is not installed");
        }
    }
    out
}

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The collections every implementation must verify.
fn exports() -> Vec<(String, Collection)> {
    let mut out = Vec::new();
    for rel in common::corpus() {
        let stem = rel
            .rsplit('/')
            .next()
            .unwrap()
            .split('.')
            .next()
            .unwrap()
            .to_lowercase();
        let store = Store::from_records(common::fixture(&rel));
        for (layout, closure) in [
            (Layout::Single, Closure::Full),
            (Layout::ByView, Closure::Ref),
        ] {
            let opts = ExportOptions::new().layout(layout).closure(closure);
            let c = export_store(&store, &Selection::all(&store), &opts).unwrap();
            out.push((
                format!("{stem}-{}-{}", layout.as_str(), closure.as_str()),
                c,
            ));
        }
    }
    let bibles = common::bibles(&scratch("bibles"), Carry::Text);
    let snap = snapshot(&bibles, &With::new()).unwrap();
    for layout in [Layout::Single, Layout::ByCollection] {
        let opts = ExportOptions::new()
            .layout(layout)
            .with(With::new().text(true));
        let c = export_library(&bibles, &snap, &Selection::all(&snap), &opts).unwrap();
        out.push((format!("bibles-{}", layout.as_str()), c));
    }
    out
}

#[test]
fn every_implementation_verifies_every_export() {
    let impls = implementations();
    if impls.is_empty() {
        return;
    }
    let root = scratch("export");
    let mut failures = Vec::new();
    for (cell, c) in exports() {
        let dir = root.join(&cell);
        write_collection(&c, &dir).unwrap();
        for i in &impls {
            let (ok, out) = i.run("verify", &dir);
            if !ok
                || !out.contains(&format!("\"rsd\":\"{}\"", hex(&c.rsd())))
                    && !out.contains(&format!("\"rsd\": \"{}\"", hex(&c.rsd())))
            {
                failures.push(format!("{} {cell}: {}", i.name, out.trim()));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn what_every_implementation_writes_imports_and_p2_holds() {
    let impls = implementations();
    let mut failures = Vec::new();
    for i in &impls {
        let dir = scratch(&format!("written-{}", i.name));
        let (ok, out) = i.run("write", &dir);
        assert!(ok, "{} write: {out}", i.name);
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        files.sort();
        assert!(files.len() > 1, "{}: wrote {files:?}", i.name);
        for f in files {
            let name = f.file_stem().unwrap().to_string_lossy().to_lowercase();
            let mut lib = Library::create(&scratch(&format!("lib-{}-{name}", i.name))).unwrap();
            let r = import(
                &mut lib,
                &[Input::File(f.clone())],
                &ImportOptions::new().accept(true),
            )
            .unwrap_or_else(|e| panic!("{} {name}: {e}", i.name));
            if r.files[0].rejected {
                failures.push(format!("{} {name}: rejected {:?}", i.name, r.report));
                continue;
            }
            match p2(&lib, &r.name).unwrap() {
                Property::Holds => {}
                other => failures.push(format!("{} {name}: P2 {other:?}", i.name)),
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
