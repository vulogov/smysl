//! RFC SMYSL-7 SG-P3: `keygen`, `sign`, `verify`, the `--sign` flags, `check --verify-sigs`, and
//! the signature files.
//!
//! The phase's gate: two agents sign overlapping attestations in two stores; merged in either
//! order, stores and signature files, both verify under `Strict`.

#![cfg(all(feature = "cli", feature = "sign", feature = "ingest"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use smysl::{AgentId, Attestation, Hlc, Op, Record, Rung, Uid};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const DOC: &str = "@doc smysl/0.1 { id: v/sg }\n\n\
    @evidence e/a { status: measured, source: { kind: metric, ref: \"p95\" } }\n~ Latency rose.\n\n\
    @claim c/b { status: speculative }\n~ The pool is small.\n\n\
    @claim c/c { status: speculative }\n~ The queue is long.\n";

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("smysl-sign-{name}-{}", std::process::id()));
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
    /// Run, expecting `code`; stdout.
    fn expect(&self, code: i32, args: &[&str]) -> String {
        let out = self.run(args);
        assert_eq!(
            out.status.code(),
            Some(code),
            "smysl {}\nstdout: {}\nstderr: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
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

fn attest(on: &Record, who: &str, ms: u64) -> Record {
    let a = agent(who);
    Record::Attestation(Attestation::new(
        uid(on),
        a.clone(),
        Op::Authored,
        Rung::Document,
        Hlc::new(ms, 0, a),
    ))
}

fn write_store(path: &str, records: &[Record]) {
    std::fs::write(path, smysl::to_cbor_seq(records)).unwrap();
}

/// Make keys for `agents`, and the key file listing them.
fn keys(d: &Dir, agents: &[&str]) {
    let mut entries = Vec::new();
    for a in agents {
        let file = format!("{}.key", a.replace(':', "-"));
        let out = d.expect(0, &["keygen", "--agent", a, "--out", &file]);
        entries.push(out.trim().to_string());
    }
    std::fs::write(
        d.0.join(".smysl/keys.hjson"),
        format!("{{ keys: [\n{}\n] }}\n", entries.join("\n")),
    )
    .unwrap();
}

#[test]
fn two_agents_two_stores_merged_either_way_verify_strict() {
    let d = Dir::new("gate");
    keys(&d, &["human:alice", "human:bob"]);
    let u = units();
    // Alice's attestation on e/a is in both stores: the overlap.
    let shared = attest(&u[0], "human:alice", 1_000);
    let a_only = attest(&u[1], "human:alice", 2_000);
    let b_only = attest(&u[2], "human:bob", 3_000);
    let b_on_a = attest(&u[0], "human:bob", 4_000);
    write_store(
        &d.p("a.cbor"),
        &[u[0].clone(), u[1].clone(), shared.clone(), a_only],
    );
    write_store(
        &d.p("b.cbor"),
        &[u[0].clone(), u[2].clone(), shared, b_only, b_on_a],
    );

    // Unsigned, strict fails and advisory warns.
    d.expect(1, &["verify", "--mode", "strict", "a.cbor"]);
    let adv = d.expect(0, &["verify", "a.cbor"]);
    assert!(
        adv.contains("2 unsigned") && adv.contains("passed"),
        "{adv}"
    );

    let out = d.expect(0, &["sign", "--key", "human-alice.key", "a.cbor"]);
    assert!(
        out.contains("signed 2 attestation(s) by human:alice"),
        "{out}"
    );
    let again = d.expect(0, &["sign", "--key", "human-alice.key", "a.cbor"]);
    assert!(
        again.contains("signed 0") && again.contains("2 already signed"),
        "{again}"
    );
    d.expect(0, &["sign", "--key", "human-alice.key", "b.cbor"]);
    d.expect(0, &["sign", "--key", "human-bob.key", "b.cbor"]);
    d.expect(0, &["verify", "--mode", "strict", "a.cbor"]);
    d.expect(0, &["verify", "--mode", "strict", "b.cbor"]);

    d.expect(0, &["merge", "a.cbor", "b.cbor", "-o", "ab.cbor"]);
    d.expect(0, &["merge", "b.cbor", "a.cbor", "-o", "ba.cbor"]);
    for s in ["ab.cbor", "ba.cbor"] {
        let out = d.expect(0, &["verify", "--mode", "strict", s]);
        assert!(
            out.contains("4 verified") && out.contains("passed"),
            "{out}"
        );
    }
    assert_eq!(
        std::fs::read(d.p("ab.cbor.sig")).unwrap(),
        std::fs::read(d.p("ba.cbor.sig")).unwrap(),
        "one signature file, whichever order"
    );
    // The shared attestation was signed twice, identically, and is kept once.
    let sigs = smysl::sign::read_signatures(&std::fs::read(d.p("ab.cbor.sig")).unwrap()).unwrap();
    assert_eq!(sigs.len(), 4);
}

#[test]
fn verify_exits_two_on_usage_and_on_the_key_file_and_one_on_a_damaged_signature() {
    let d = Dir::new("exits");
    keys(&d, &["human:alice"]);
    let u = units();
    write_store(
        &d.p("s.cbor"),
        &[u[0].clone(), attest(&u[0], "human:alice", 1)],
    );
    d.expect(0, &["sign", "--key", "human-alice.key", "s.cbor"]);
    d.expect(0, &["verify", "--mode", "strict", "s.cbor"]);

    // Usage: a selective with no scope, a scope on another mode, no key file, a bad key file.
    d.expect(2, &["verify", "--mode", "selective", "s.cbor"]);
    d.expect(2, &["verify", "--agent-prefix", "tool:", "s.cbor"]);
    d.expect(2, &["verify", "--keys", "nowhere.hjson", "s.cbor"]);
    std::fs::write(d.p("bad.hjson"), "{ keys: [ { agent: \"human:alice\" } ] }").unwrap();
    d.expect(2, &["verify", "--keys", "bad.hjson", "s.cbor"]);

    // Selective: in scope by status (the unit is measured), so signed passes...
    d.expect(
        0,
        &[
            "verify",
            "--mode",
            "selective",
            "--status-at-least",
            "cited",
            "s.cbor",
        ],
    );

    // ...and a flipped byte in the signature file fails, in every mode.
    let mut sig = std::fs::read(d.p("s.cbor.sig")).unwrap();
    let n = sig.len();
    sig[n - 1] ^= 1;
    std::fs::write(d.p("s.cbor.sig"), &sig).unwrap();
    for mode in ["advisory", "strict"] {
        let out = d.expect(1, &["verify", "--mode", mode, "s.cbor"]);
        assert!(out.contains("does not verify"), "{out}");
    }
    // A file that does not read at all is a failure too, not an empty list.
    std::fs::write(d.p("s.cbor.sig"), b"\xd2\x84").unwrap();
    d.expect(1, &["verify", "s.cbor"]);

    // JSON: one object, with its findings.
    std::fs::remove_file(d.p("s.cbor.sig")).unwrap();
    let json = d.expect(1, &["--json", "verify", "--mode", "strict", "s.cbor"]);
    assert!(
        json.contains("\"passed\":false") && json.contains("\"verdict\":\"unsigned\""),
        "{json}"
    );
}

#[test]
fn keygen_writes_an_owner_only_key_and_refuses_to_overwrite() {
    let d = Dir::new("keygen");
    let entry = d.expect(
        0,
        &["keygen", "--agent", "human:alice", "--out", "k/alice.key"],
    );
    assert!(
        entry.starts_with("{ agent: \"human:alice\", ed25519: \"hex:"),
        "{entry}"
    );
    smysl::sign::KeyFile::parse(&format!("{{ keys: [ {entry} ] }}")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(d.p("k/alice.key"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    d.expect(
        1,
        &["keygen", "--agent", "human:alice", "--out", "k/alice.key"],
    );
    d.expect(
        2,
        &["keygen", "--agent", "not an agent", "--out", "k/x.key"],
    );
    // Two keys for one agent are two keys.
    let other = d.expect(
        0,
        &["keygen", "--agent", "human:alice", "--out", "k/alice2.key"],
    );
    assert_ne!(entry, other);
}

#[test]
fn check_adds_the_signature_findings() {
    let d = Dir::new("check");
    keys(&d, &["human:alice"]);
    let u = units();
    // The speculative unit: a measured one with an authored attestation is check's own error.
    write_store(
        &d.p("s.cbor"),
        &[u[1].clone(), attest(&u[1], "human:alice", 1)],
    );
    let out = d.run(&["check", "--verify-sigs", "strict", "s.cbor"]);
    assert_eq!(out.status.code(), Some(3), "{out:?}");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("error: sign/unsigned"), "{err}");
    d.expect(0, &["check", "--verify-sigs", "advisory", "s.cbor"]);
    d.expect(0, &["sign", "--key", "human-alice.key", "s.cbor"]);
    d.expect(0, &["check", "--verify-sigs", "strict", "s.cbor"]);
    d.expect(
        2,
        &[
            "check",
            "--verify-sigs",
            "strict",
            "--keys",
            "none.hjson",
            "s.cbor",
        ],
    );
}

#[test]
fn merge_staged_sign_signs_the_batch_into_the_outputs_file() {
    let d = Dir::new("staged");
    keys(&d, &["human:alice"]);
    let u = units();
    write_store(&d.p("base.cbor"), &[u[1].clone()]);
    let alice = agent("human:alice");
    let core = u[0].as_unit().unwrap().clone();
    let staged = smysl::stage::prepare(
        &smysl::Store::new(),
        vec![core],
        Vec::new(),
        Default::default(),
        &smysl::stage::Attest::new(alice.clone(), Rung::Document, Hlc::new(5, 0, alice)),
    );
    smysl::stage::write(&d.0, &staged).unwrap();

    // --sign needs --staged, an output, and a key.
    d.expect(
        2,
        &[
            "merge",
            "--sign",
            "--key",
            "human-alice.key",
            "base.cbor",
            "-o",
            "o.cbor",
        ],
    );
    d.expect(
        2,
        &[
            "merge",
            "--staged",
            "--sign",
            "--key",
            "human-alice.key",
            "base.cbor",
        ],
    );
    d.expect(
        2,
        &["merge", "--staged", "--sign", "base.cbor", "-o", "o.cbor"],
    );

    d.expect(
        0,
        &[
            "merge",
            "--staged",
            "--sign",
            "--key",
            "human-alice.key",
            "base.cbor",
            "-o",
            "o.cbor",
        ],
    );
    let out = d.expect(0, &["verify", "--mode", "strict", "o.cbor"]);
    assert!(
        out.contains("1 verified") && out.contains("passed"),
        "{out}"
    );
}

#[test]
fn retract_sign_records_and_signs_an_attestation_on_the_edge() {
    let d = Dir::new("retract");
    keys(&d, &["human:alice"]);
    let u = units();
    write_store(
        &d.p("s.cbor"),
        &[u[1].clone(), attest(&u[1], "human:alice", 1)],
    );
    d.expect(0, &["sign", "--key", "human-alice.key", "-s", "s.cbor"]);
    let target = uid(&u[1]).to_string();
    // The key's agent must be the one retracting.
    d.expect(
        2,
        &[
            "retract",
            &target,
            "--as",
            "human:bob",
            "--sign",
            "--key",
            "human-alice.key",
            "s.cbor",
        ],
    );
    d.expect(
        0,
        &[
            "retract",
            &target,
            "--as",
            "human:alice",
            "--sign",
            "--key",
            "human-alice.key",
            "s.cbor",
        ],
    );
    let out = d.expect(0, &["verify", "--mode", "strict", "-s", "s.cbor"]);
    assert!(
        out.contains("2 verified") && out.contains("passed"),
        "{out}"
    );
    // The attestation names the edge.
    let (records, _) = smysl::from_cbor_seq(&std::fs::read(d.p("s.cbor")).unwrap()).unwrap();
    let edge = records
        .iter()
        .find_map(|r| match r {
            Record::Relation(e) => Some(e.uid()),
            _ => None,
        })
        .unwrap();
    assert!(records
        .iter()
        .any(|r| matches!(r, Record::Attestation(a) if a.uid == edge)));
}

#[test]
fn a_library_keeps_its_signatures_under_log() {
    let d = Dir::new("library");
    keys(&d, &["human:alice"]);
    smysl::text::library::Library::create(&d.0.join("lib")).unwrap();
    let out = d.expect(0, &["sign", "--key", "human-alice.key", "-s", "lib"]);
    let want = Path::new("lib").join("log").join("sigs.cbor");
    assert!(out.contains(&format!("into {}", want.display())), "{out}");
    // A file store's are beside it.
    write_store(&d.p("s.cbor"), &units()[..1]);
    let out = d.expect(0, &["sign", "--key", "human-alice.key", "s.cbor"]);
    assert!(out.contains("into s.cbor.sig"), "{out}");
}
