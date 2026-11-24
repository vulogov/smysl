//! RFC SMYSL-8 TR-P2: the signature policy in the CLI.
//!
//! The phase's gate: the same store ranked with no policy and under `strict` differs only in
//! units whose corroboration came from unsigned attestations; `retract --as human:mallory`, with
//! origin claimed by an unsigned attestation, is refused under `strict`. Besides: an `--as` in
//! scope needs a `--key` that verifies against the key file, `trust:` in the configuration
//! applies without the flag, and `withdraw` reads the same rule.

#![cfg(all(feature = "cli", feature = "sign", feature = "ingest"))]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Output};

use smysl::{AgentId, Attestation, Hlc, Op, Record, RelKind, Relation, Rung, Uid};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const DOC: &str = "@doc smysl/0.1 { id: v/tr }\n\n\
    @evidence e/a { status: measured, source: { kind: metric, ref: \"p95\" } }\n~ Latency rose.\n\n\
    @claim c/b { status: speculative }\n~ The pool is small.\n\n\
    @claim c/c { status: speculative }\n~ The queue is long.\n";

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("smysl-trust-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(d.join(".smysl")).unwrap();
        Dir(d)
    }
    fn p(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_string()
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(BIN)
            .current_dir(&self.0)
            .args(args)
            .output()
            .unwrap()
    }
    /// Run, expecting `code`; stdout and stderr.
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

fn agent(s: &str) -> AgentId {
    s.parse().unwrap()
}

fn units() -> Vec<Record> {
    smysl::parse_surface(DOC)
        .unwrap()
        .records
        .into_iter()
        .filter(|r| matches!(r, Record::Unit(_)))
        .collect()
}

fn uid(r: &Record) -> Uid {
    smysl::canonical_uid(r.as_unit().unwrap())
}

fn attest_uid(on: Uid, who: &str, ms: u64) -> Record {
    let a = agent(who);
    Record::Attestation(Attestation::new(
        on,
        a.clone(),
        Op::Authored,
        Rung::Document,
        Hlc::new(ms, 0, a),
    ))
}

fn attest(on: &Record, who: &str, ms: u64) -> Record {
    attest_uid(uid(on), who, ms)
}

fn write_store(path: &str, records: &[Record]) {
    std::fs::write(path, smysl::to_cbor_seq(records)).unwrap();
}

/// Make keys for `agents`, and the key file listing them.
fn keys(d: &Dir, agents: &[&str]) {
    let mut entries = Vec::new();
    for a in agents {
        let file = format!("{}.key", a.replace(':', "-"));
        let (out, _) = d.expect(0, &["keygen", "--agent", a, "--out", &file]);
        entries.push(out.trim().to_string());
    }
    std::fs::write(
        d.0.join(".smysl/keys.hjson"),
        format!("{{ keys: [\n{}\n] }}\n", entries.join("\n")),
    )
    .unwrap();
}

/// `salience --json`'s ranking, by uid.
fn ranking(d: &Dir, extra: &[&str]) -> BTreeMap<String, String> {
    let mut args = vec!["salience", "--json", "s.cbor"];
    args.extend_from_slice(extra);
    let (out, _) = d.expect(0, &args);
    out.split("{\"uid\":")
        .skip(1)
        .map(|row| {
            let (u, rest) = row.split_once(",\"score\":").unwrap();
            let score = rest.trim_end_matches(['}', ']', ',', '\n']);
            (u.trim_matches('"').to_string(), score.to_string())
        })
        .collect()
}

/// e/a signed by bob; c/b by alice (signed) and two invented model agents (unsigned); c/c by
/// alice (signed).
fn mixed(d: &Dir) -> (Vec<Record>, BTreeMap<String, bool>) {
    keys(d, &["human:alice", "human:bob"]);
    let u = units();
    let records = vec![
        u[0].clone(),
        u[1].clone(),
        u[2].clone(),
        attest(&u[0], "human:bob", 1),
        attest(&u[1], "human:alice", 2),
        attest(&u[1], "model:fake/one", 3),
        attest(&u[1], "model:fake/two", 4),
        attest(&u[2], "human:alice", 5),
    ];
    write_store(&d.p("s.cbor"), &records);
    d.expect(0, &["sign", "--key", "human-alice.key", "s.cbor"]);
    d.expect(0, &["sign", "--key", "human-bob.key", "s.cbor"]);
    let unsigned: BTreeMap<String, bool> = u
        .iter()
        .enumerate()
        .map(|(i, r)| (uid(r).canonical(), i == 1))
        .collect();
    (u, unsigned)
}

#[test]
fn strict_changes_only_units_corroborated_by_unsigned_attestations() {
    let d = Dir::new("rank");
    let (u, has_unsigned) = mixed(&d);
    let plain = ranking(&d, &[]);
    let strict = ranking(&d, &["--trust-sigs", "strict"]);
    assert_eq!(plain.len(), 3);
    let changed: Vec<&String> = plain.keys().filter(|k| plain[*k] != strict[*k]).collect();
    assert!(!changed.is_empty(), "the policy changed nothing: {plain:?}");
    for k in &changed {
        assert!(
            has_unsigned[*k],
            "{k} changed but had no unsigned attestation"
        );
    }
    assert!(changed.contains(&&uid(&u[1]).canonical()));

    // Advisory admits the unsigned, as `verify` tolerates them.
    assert_eq!(ranking(&d, &["--trust-sigs", "advisory"]), plain);

    // --explain says what the policy removed.
    let target = uid(&u[1]).to_string();
    let (out, _) = d.expect(
        0,
        &[
            "salience",
            "s.cbor",
            "--explain",
            &target,
            "--trust-sigs",
            "strict",
        ],
    );
    assert!(
        out.contains("not admitted by the signature policy: model:fake/one"),
        "{out}"
    );

    // `pack` and `thread` take the policy too.
    d.expect(
        0,
        &[
            "pack",
            "--budget",
            "200",
            "s.cbor",
            "--trust-sigs",
            "strict",
        ],
    );
    d.expect(
        0,
        &[
            "thread",
            "--derive",
            "analysis",
            "s.cbor",
            "--trust-sigs",
            "strict",
        ],
    );
}

#[test]
fn a_fully_signed_store_ranks_the_same_under_every_policy() {
    let d = Dir::new("signed");
    keys(&d, &["human:alice"]);
    let u = units();
    write_store(
        &d.p("s.cbor"),
        &[
            u[0].clone(),
            u[1].clone(),
            attest(&u[0], "human:alice", 1),
            attest(&u[1], "human:alice", 2),
        ],
    );
    d.expect(0, &["sign", "--key", "human-alice.key", "s.cbor"]);
    let plain = ranking(&d, &[]);
    assert_eq!(ranking(&d, &["--trust-sigs", "strict"]), plain);
    let (p0, _) = d.expect(0, &["pack", "--budget", "200", "s.cbor"]);
    let (p1, _) = d.expect(
        0,
        &[
            "pack",
            "--budget",
            "200",
            "s.cbor",
            "--trust-sigs",
            "strict",
        ],
    );
    assert_eq!(p0, p1);
}

/// The configuration's `trust:` applies without the flag; one that does not parse is a usage
/// error, not an absent policy; a policy with no key file is a usage error too.
#[test]
fn the_configuration_sets_the_policy() {
    let d = Dir::new("config");
    mixed(&d);
    let strict = ranking(&d, &["--trust-sigs", "strict"]);
    std::fs::write(
        d.0.join(".smysl/config.hjson"),
        "{ trust: { signatures: \"strict\" } }\n",
    )
    .unwrap();
    assert_eq!(ranking(&d, &[]), strict);

    std::fs::write(
        d.0.join(".smysl/config.hjson"),
        "{ trust: { signatures: \"strong\" } }\n",
    )
    .unwrap();
    let (_, err) = d.expect(2, &["salience", "s.cbor"]);
    assert!(err.contains("no mode `strong`"), "{err}");

    std::fs::remove_file(d.0.join(".smysl/config.hjson")).unwrap();
    std::fs::remove_file(d.0.join(".smysl/keys.hjson")).unwrap();
    d.expect(2, &["salience", "s.cbor", "--trust-sigs", "strict"]);
    // A scope without a mode is a usage error, not ignored.
    d.expect(2, &["salience", "s.cbor", "--agent-prefix", "model:"]);
}

#[test]
fn origin_claimed_by_an_unsigned_attestation_does_not_authorise_under_strict() {
    let d = Dir::new("retract");
    keys(&d, &["human:alice", "human:mallory"]);
    let u = units();
    write_store(
        &d.p("s.cbor"),
        &[
            u[1].clone(),
            attest(&u[1], "human:alice", 1),
            attest(&u[1], "human:mallory", 2),
        ],
    );
    // Alice signs hers; mallory's stays unsigned.
    d.expect(0, &["sign", "--key", "human-alice.key", "s.cbor"]);
    let target = uid(&u[1]).to_string();
    let dry = |who: &str, extra: &[&str]| -> Vec<String> {
        let mut a: Vec<String> = [
            "retract",
            &target,
            "--as",
            who,
            "--dry-run",
            "--json",
            "s.cbor",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        a.extend(extra.iter().map(|s| s.to_string()));
        a
    };
    let run = |args: Vec<String>, code: i32| {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        d.expect(code, &refs)
    };

    // No policy: mallory's string is enough, as before.
    let (out, _) = run(dry("human:mallory", &[]), 0);
    assert!(out.contains("\"authorised\":true"), "{out}");

    // Strict, mallory proven by her key: her origin attestation is unsigned, so not admitted.
    let (out, _) = run(
        dry(
            "human:mallory",
            &["--trust-sigs", "strict", "--key", "human-mallory.key"],
        ),
        0,
    );
    assert!(out.contains("\"authorised\":false"), "{out}");
    assert!(out.contains("not admitted"), "{out}");

    // Strict, alice proven: her attestation is signed, so origin holds.
    let (out, _) = run(
        dry(
            "human:alice",
            &["--trust-sigs", "strict", "--key", "human-alice.key"],
        ),
        0,
    );
    assert!(out.contains("\"authorised\":true"), "{out}");

    // An --as in scope needs a key ...
    let (_, err) = run(dry("human:alice", &["--trust-sigs", "strict"]), 1);
    assert!(err.contains("needs a --key of human:alice"), "{err}");
    // ... and a key the key file does not hold proves nothing: anyone can make one.
    d.expect(
        0,
        &["keygen", "--agent", "human:alice", "--out", "rogue.key"],
    );
    let (_, err) = run(
        dry(
            "human:alice",
            &["--trust-sigs", "strict", "--key", "rogue.key"],
        ),
        1,
    );
    assert!(err.contains("does not prove it"), "{err}");

    // Advisory: unproven is a warning, and the unsigned origin is admitted.
    let (out, err) = run(dry("human:mallory", &["--trust-sigs", "advisory"]), 0);
    assert!(out.contains("\"authorised\":true"), "{out}");
    assert!(
        err.contains("warning: --as human:mallory is not proven"),
        "{err}"
    );
}

#[test]
fn a_quorum_signs_with_each_key() {
    let d = Dir::new("quorum");
    keys(&d, &["human:alice", "human:bob"]);
    let u = units();
    write_store(
        &d.p("s.cbor"),
        &[u[1].clone(), attest(&u[1], "human:alice", 1)],
    );
    let target = uid(&u[1]).to_string();
    d.expect(
        0,
        &[
            "retract",
            &target,
            "--as",
            "human:alice",
            "--as",
            "human:bob",
            "--authority",
            "quorum:2",
            "--sign",
            "--key",
            "human-alice.key",
            "--key",
            "human-bob.key",
            "--trust-sigs",
            "strict",
            "s.cbor",
        ],
    );
    let (out, _) = d.expect(0, &["verify", "--mode", "advisory", "s.cbor"]);
    assert!(out.contains("2 verified"), "{out}");
}

#[test]
fn withdraw_reads_the_same_rule() {
    let d = Dir::new("withdraw");
    keys(&d, &["human:mallory"]);
    let u = units();
    let edge = Relation::new(RelKind::Elaborates, uid(&u[1]), uid(&u[2]));
    let rid = edge.uid();
    write_store(
        &d.p("s.cbor"),
        &[
            u[1].clone(),
            u[2].clone(),
            Record::Relation(edge),
            attest_uid(rid, "human:mallory", 1),
        ],
    );
    let rid = rid.to_string();
    let base = [
        "withdraw",
        rid.as_str(),
        "--as",
        "human:mallory",
        "--dry-run",
        "--json",
    ];

    let mut plain = base.to_vec();
    plain.push("s.cbor");
    let (out, _) = d.expect(0, &plain);
    assert!(out.contains("\"authorised\":true"), "{out}");

    let mut strict = base.to_vec();
    strict.extend([
        "--trust-sigs",
        "strict",
        "--key",
        "human-mallory.key",
        "s.cbor",
    ]);
    let (out, _) = d.expect(0, &strict);
    assert!(out.contains("\"authorised\":false"), "{out}");
    assert!(out.contains("not admitted"), "{out}");
}
