//! RFC SMYSL-8 TR-P3: `smysl keys`.
//!
//! The phase's gate for keys: `rotate` leaves the signatures made before it valid, and refuses
//! the old key's signatures on attestations after it. Besides: `add` from `keygen`'s output,
//! `revoke`, `list`, `check`, and a rewrite that would drop a comment refused.

#![cfg(all(feature = "cli", feature = "sign"))]

use std::path::PathBuf;
use std::process::{Command, Output};

use smysl::{AgentId, Attestation, Hlc, Op, Record, Rung, Uid};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const DOC: &str = "@doc smysl/0.1 { id: v/k }\n\n\
    @claim c/b { status: speculative }\n~ The pool is small.\n\n\
    @claim c/c { status: speculative }\n~ The queue is long.\n";

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("smysl-keys-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(d.join(".smysl")).unwrap();
        Dir(d)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
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
    fn keys_file(&self) -> String {
        std::fs::read_to_string(self.0.join(".smysl/keys.hjson")).unwrap()
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

fn write_store(d: &Dir, records: &[Record]) {
    std::fs::write(d.0.join("s.cbor"), smysl::to_cbor_seq(records)).unwrap();
}

/// `keygen` an agent's key and `keys add --entry` what it printed.
fn enrol(d: &Dir, agent: &str, file: &str) {
    let (entry, _) = d.expect(0, &["keygen", "--agent", agent, "--out", file]);
    std::fs::write(d.0.join("entry.txt"), entry).unwrap();
    let (out, _) = d.expect(0, &["keys", "add", "--entry", "entry.txt"]);
    assert!(out.starts_with("added "), "{out}");
}

#[test]
fn rotation_keeps_old_signatures_and_refuses_the_old_key_after_it() {
    let d = Dir::new("rotate");
    enrol(&d, "human:alice", "old.key");
    let u = units();
    // An attestation long before the rotation, signed with the old key.
    write_store(&d, &[u[0].clone(), attest(&u[0], "human:alice", 1_000)]);
    d.expect(0, &["sign", "--key", "old.key", "s.cbor"]);
    d.expect(0, &["verify", "--mode", "strict", "s.cbor"]);

    let (out, _) = d.expect(
        0,
        &[
            "keys",
            "rotate",
            "--agent",
            "human:alice",
            "--out",
            "new.key",
        ],
    );
    assert!(out.contains("revoked"), "{out}");
    let file = d.keys_file();
    assert!(
        file.contains("status: \"revoked\"") && file.contains("revoked_after_ms"),
        "{file}"
    );

    // The signature made before the rotation still verifies.
    d.expect(0, &["verify", "--mode", "strict", "s.cbor"]);

    // An attestation after the rotation, signed with the old key: refused in every mode.
    let later = u64::MAX / 4;
    write_store(
        &d,
        &[
            u[0].clone(),
            u[1].clone(),
            attest(&u[0], "human:alice", 1_000),
            attest(&u[1], "human:alice", later),
        ],
    );
    d.expect(0, &["sign", "--key", "old.key", "s.cbor"]);
    let (out, _) = d.expect(1, &["verify", "--mode", "advisory", "s.cbor"]);
    assert!(out.contains("revoked"), "{out}");

    // Signed afresh with the new key instead, it verifies.
    std::fs::remove_file(d.0.join("s.cbor.sig")).unwrap();
    d.expect(0, &["sign", "--key", "new.key", "s.cbor"]);
    d.expect(0, &["verify", "--mode", "strict", "s.cbor"]);
}

#[test]
fn add_list_revoke_and_check() {
    let d = Dir::new("edit");
    enrol(&d, "human:alice", "alice.key");
    enrol(&d, "human:bob", "bob.key");
    // Adding the same key twice is refused.
    d.expect(2, &["keys", "add", "--entry", "entry.txt"]);

    let (json, _) = d.expect(0, &["keys", "list", "--json"]);
    assert_eq!(json.matches("\"status\":\"active\"").count(), 2, "{json}");

    let u = units();
    write_store(
        &d,
        &[
            u[0].clone(),
            attest(&u[0], "human:alice", 1),
            attest(&u[0], "human:bob", 2),
        ],
    );
    d.expect(0, &["sign", "--key", "alice.key", "s.cbor"]);
    let (out, _) = d.expect(0, &["keys", "check", "s.cbor"]);
    assert!(
        out.contains("1 signature(s), 2 key(s), 1 signed nothing, 0 unknown key(s)"),
        "{out}"
    );

    // Revoking bob's key as compromised: by an eight-digit prefix of its id.
    let bob_kid = json
        .split("{\"agent\":\"human:bob\",\"kid\":\"")
        .nth(1)
        .unwrap()[..8]
        .to_string();
    d.expect(2, &["keys", "revoke", "--kid", &bob_kid]);
    let (out, _) = d.expect(
        0,
        &[
            "keys",
            "revoke",
            "--kid",
            &bob_kid,
            "--compromised",
            "--reason",
            "lost laptop",
        ],
    );
    assert!(out.contains("every signature it made is refused"), "{out}");
    let file = d.keys_file();
    assert!(file.contains("reason: \"lost laptop\""), "{file}");
    assert!(!file.contains("revoked_after_ms"), "{file}");

    // A key file that is not there is a usage error for list.
    d.expect(2, &["keys", "list", "--keys", "nowhere.hjson"]);
}

#[test]
fn a_rewrite_that_would_drop_a_comment_is_refused() {
    let d = Dir::new("comments");
    enrol(&d, "human:alice", "alice.key");
    let file = d.keys_file();
    let commented = file.replacen("keys: [", "keys: [ # the team", 1);
    std::fs::write(d.0.join(".smysl/keys.hjson"), &commented).unwrap();

    let (entry, _) = d.expect(0, &["keygen", "--agent", "human:bob", "--out", "bob.key"]);
    std::fs::write(d.0.join("entry.txt"), entry).unwrap();
    let (_, err) = d.expect(2, &["keys", "add", "--entry", "entry.txt"]);
    assert!(err.contains("line 3 has a comment"), "{err}");
    assert_eq!(
        d.keys_file(),
        commented,
        "a refused rewrite leaves the file as it was"
    );

    d.expect(0, &["keys", "add", "--entry", "entry.txt", "--rewrite"]);
    assert!(!d.keys_file().contains("the team"));
    // A rotate refused for the same reason writes no secret.
    std::fs::write(d.0.join(".smysl/keys.hjson"), &commented).unwrap();
    d.expect(
        2,
        &[
            "keys",
            "rotate",
            "--agent",
            "human:alice",
            "--out",
            "new.key",
        ],
    );
    assert!(!d.0.join("new.key").exists());
}
