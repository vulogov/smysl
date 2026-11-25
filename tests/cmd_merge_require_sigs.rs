//! RFC SMYSL-8 TR-P4: `merge --require-sigs`.
//!
//! The phase's gate: merging an unsigned input under `strict` writes nothing and exits 1; merging
//! a signed and an unsigned copy of one attestation passes, in either order, since the merged
//! result is what is judged. Besides: each finding names the inputs that carried its record; a
//! staged batch is judged after `--sign` signs it; a mode or key file that cannot be applied is a
//! usage error that has read nothing.

#![cfg(all(feature = "cli", feature = "sign", feature = "ingest"))]

use std::path::PathBuf;
use std::process::{Command, Output};

use smysl::{AgentId, Attestation, Hlc, Op, Record, Rung, Uid};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const DOC: &str = "@doc smysl/0.1 { id: v/rq }\n\n\
    @evidence e/a { status: measured, source: { kind: metric, ref: \"p95\" } }\n~ Latency rose.\n\n\
    @claim c/b { status: speculative }\n~ The pool is small.\n";

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("smysl-require-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(d.join(".smysl")).unwrap();
        Dir(d)
    }
    fn has(&self, name: &str) -> bool {
        self.0.join(name).exists()
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
    /// Run, expecting `code`; (stdout, stderr).
    fn expect(&self, code: i32, args: &[&str]) -> (String, String) {
        let out = self.run(args);
        let (o, e) = (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        );
        assert_eq!(
            out.status.code(),
            Some(code),
            "smysl {}\nstdout: {o}\nstderr: {e}",
            args.join(" ")
        );
        (o, e)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn units() -> Vec<Record> {
    smysl::parse_surface(DOC)
        .unwrap()
        .records
        .into_iter()
        .filter(|r| matches!(r, Record::Unit(_)))
        .collect()
}

fn attest(on: &Record, who: &str, ms: u64) -> Record {
    let a: AgentId = who.parse().unwrap();
    let uid: Uid = smysl::canonical_uid(on.as_unit().unwrap());
    Record::Attestation(Attestation::new(
        uid,
        a.clone(),
        Op::Authored,
        Rung::Document,
        Hlc::new(ms, 0, a),
    ))
}

fn write_store(d: &Dir, name: &str, records: &[Record]) {
    std::fs::write(d.0.join(name), smysl::to_cbor_seq(records)).unwrap();
}

/// A key for human:alice, and the key file listing it.
fn enrol(d: &Dir) {
    let (entry, _) = d.expect(
        0,
        &["keygen", "--agent", "human:alice", "--out", "alice.key"],
    );
    std::fs::write(
        d.0.join(".smysl/keys.hjson"),
        format!("{{ keys: [\n{}\n] }}\n", entry.trim()),
    )
    .unwrap();
}

/// `signed.cbor` and `unsigned.cbor`: the same records, one with a signature file.
fn two_copies(d: &Dir) {
    let u = units();
    let records = [
        u[0].clone(),
        u[1].clone(),
        attest(&u[0], "human:alice", 1_000),
        attest(&u[1], "human:alice", 2_000),
    ];
    write_store(d, "signed.cbor", &records);
    write_store(d, "unsigned.cbor", &records);
    d.expect(0, &["sign", "--key", "alice.key", "signed.cbor"]);
}

#[test]
fn an_unsigned_input_under_strict_writes_nothing_and_exits_1() {
    let d = Dir::new("unsigned");
    enrol(&d);
    two_copies(&d);
    let (out, err) = d.expect(
        1,
        &[
            "merge",
            "unsigned.cbor",
            "-o",
            "out.cbor",
            "--require-sigs",
            "strict",
        ],
    );
    assert!(
        !d.has("out.cbor") && !d.has("out.cbor.sig"),
        "nothing written"
    );
    assert_eq!(
        out.matches("(in unsigned.cbor): no signature").count(),
        2,
        "{out}"
    );
    assert!(
        out.contains("strict: 0 verified, 2 unsigned") && out.contains("failed"),
        "{out}"
    );
    assert!(err.contains("nothing written"), "{err}");

    // Without the flag the same merge writes, as before.
    d.expect(0, &["merge", "unsigned.cbor", "-o", "out.cbor"]);
    assert!(d.has("out.cbor"));
}

#[test]
fn a_signed_and_an_unsigned_copy_merged_pass_in_either_order() {
    let d = Dir::new("copies");
    enrol(&d);
    two_copies(&d);
    for (i, order) in [
        ["signed.cbor", "unsigned.cbor"],
        ["unsigned.cbor", "signed.cbor"],
    ]
    .iter()
    .enumerate()
    {
        let out = format!("m{i}.cbor");
        let (_, err) = d.expect(
            0,
            &[
                "merge",
                order[0],
                order[1],
                "-o",
                &out,
                "--require-sigs",
                "strict",
            ],
        );
        assert!(
            err.contains("2 verified, 0 unsigned") && err.contains("passed"),
            "{err}"
        );
        let (v, _) = d.expect(0, &["verify", "--mode", "strict", &out]);
        assert!(v.contains("passed"), "{v}");
    }
    assert_eq!(
        std::fs::read(d.0.join("m0.cbor")).unwrap(),
        std::fs::read(d.0.join("m1.cbor")).unwrap()
    );
}

#[test]
fn findings_name_every_input_that_carried_the_record() {
    let d = Dir::new("carriers");
    enrol(&d);
    two_copies(&d);
    std::fs::copy(d.0.join("unsigned.cbor"), d.0.join("also.cbor")).unwrap();
    let (out, _) = d.expect(
        1,
        &[
            "merge",
            "unsigned.cbor",
            "also.cbor",
            "--require-sigs",
            "strict",
            "--json",
        ],
    );
    assert_eq!(
        out.matches("\"inputs\":[\"unsigned.cbor\",\"also.cbor\"]")
            .count(),
        2,
        "{out}"
    );
    assert!(out.contains("\"store\":\"merged\""), "{out}");
    assert!(out.contains("\"passed\":false"), "{out}");

    // Selective over another agent's prefix passes the same union.
    d.expect(
        0,
        &[
            "merge",
            "unsigned.cbor",
            "also.cbor",
            "-o",
            "sel.cbor",
            "--require-sigs",
            "selective",
            "--agent-prefix",
            "model:",
        ],
    );
}

#[test]
fn a_staged_batch_is_judged_after_sign_signs_it() {
    let d = Dir::new("staged");
    enrol(&d);
    two_copies(&d);
    let u = units();
    let alice: AgentId = "human:alice".parse().unwrap();
    let mut extra = u[1].as_unit().unwrap().clone();
    extra.gist = "The pool is smaller than it was.".into();
    let staged = smysl::stage::prepare(
        &smysl::Store::new(),
        vec![extra],
        Vec::new(),
        Default::default(),
        &smysl::stage::Attest::new(alice.clone(), Rung::Document, Hlc::new(5, 0, alice)),
    );
    smysl::stage::write(&d.0, &staged).unwrap();

    // Unsigned, the batch fails strict: nothing is committed.
    let (out, _) = d.expect(
        1,
        &[
            "merge",
            "--staged",
            "signed.cbor",
            "-o",
            "o.cbor",
            "--require-sigs",
            "strict",
        ],
    );
    assert!(out.contains("(in staged): no signature"), "{out}");
    assert!(!d.has("o.cbor"));

    // Signed as it is committed, it passes.
    d.expect(
        0,
        &[
            "merge",
            "--staged",
            "--sign",
            "--key",
            "alice.key",
            "signed.cbor",
            "-o",
            "o.cbor",
            "--require-sigs",
            "strict",
        ],
    );
    let (v, _) = d.expect(0, &["verify", "--mode", "strict", "o.cbor"]);
    assert!(v.contains("3 verified") && v.contains("passed"), "{v}");
}

#[test]
fn a_policy_that_cannot_be_applied_is_a_usage_error_before_any_input_is_read() {
    let d = Dir::new("usage");
    // No key file, and an input that is not there: the key file is what is reported.
    let (_, err) = d.expect(2, &["merge", "absent.cbor", "--require-sigs", "strict"]);
    assert!(err.contains("keys.hjson"), "{err}");
    enrol(&d);
    let (_, err) = d.expect(2, &["merge", "absent.cbor", "--require-sigs", "selective"]);
    assert!(err.contains("selective needs a scope"), "{err}");
    let (_, err) = d.expect(
        2,
        &[
            "merge",
            "absent.cbor",
            "--require-sigs",
            "strict",
            "--agent-prefix",
            "model:",
        ],
    );
    assert!(err.contains("scope of selective"), "{err}");
    let (_, err) = d.expect(2, &["merge", "absent.cbor", "--keys", "k.hjson"]);
    assert!(err.contains("go with --require-sigs"), "{err}");
}
