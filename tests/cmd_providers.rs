//! `smysl providers`, driven as a user drives it.
//!
//! Twelve mutants survived in `cmd_providers` at 1.1, and the reason none of them was reached
//! is the reason the command exists: it is the one subcommand whose job is to talk to the
//! network, so nothing tested it. That is backwards. Every decision below happens *before* any
//! request, or decides whether one is made at all — which listing to print, whether to contact
//! anything, whether `--offline` refuses a provider, and whether a probe's answer means up or
//! down. Those are exactly the ones worth pinning, and none of them needs a provider.
//!
//! **No test here sends anything off the machine.** Two techniques do it:
//!
//!   * An unreachable provider is one pointed at a closed port on loopback. The connection is
//!     refused locally and `probe()` returns `Ok(Probe::unreachable(…))`, which is the branch
//!     the `probe.reachable` guard turns on.
//!   * A reachable one is a stub server this test binds on `127.0.0.1`, answering `/api/tags`
//!     with the one JSON object `Ollama::parse_tags` needs. It is thirty lines and it is the
//!     only way to exercise the `up` branch without a running ollama.
//!
//! The hosted provider is configured at `api.example.com` and is *only* ever run under
//! `--offline`, where the refusal happens before the request is built. It is never dialled.
//! `caps().offline` is derived from the endpoint rather than the vendor, which is what makes
//! that safe to arrange — see `ProviderConfig::is_local`.
//!
//! A thirteenth mutant sits in the `#[cfg(not(feature = "providers"))]` stub of the same name.
//! `--all-features` does not compile it, so no test can kill it and none should try: it is a
//! measurement artifact of running mutants over a file with two definitions, not a gap.

// The binary only exists with `cli`, and the command only with `providers` — but `providers`
// alone compiles *no* mapper, so a configuration naming any `kind` would be skipped with
// "not compiled into this build" and every listing here would come up short. `local` is the
// narrowest feature that yields one, so it is what this file is gated on.
//
// That is the whole reason nothing below names a vendor. The first version configured the
// hosted provider as `anthropic` and passed under `--all-features` and the default build, then
// failed three tests under `--no-default-features --features local`, where that mapper does not
// exist. `caps().offline` is derived from the *endpoint* rather than the vendor — see
// `ProviderConfig::is_local` — so one mapper can supply both a local and a hosted provider, and
// the tests stop depending on which feature combination is being built.
#![cfg(all(feature = "cli", feature = "local"))]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

const SUCCESS: i32 = 0;
const OFFLINE: i32 = 7;

struct Out {
    stdout: String,
    stderr: String,
    code: i32,
}

/// Run from `dir`, because the registry is read from `./.smysl/config.hjson`.
fn run_in(dir: &Path, args: &[&str]) -> Out {
    let o = Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary under test must run");
    Out {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        code: o.status.code().unwrap_or(-1),
    }
}

/// A project of this test's own, holding the configuration it names.
///
/// Each test writes the registry it needs rather than sharing one. `cmd_fmt.rs` records why:
/// `cargo-mutants` reuses build directories, so a test that depends on a file another test
/// wrote fails for reasons that have nothing to do with the mutant, and cargo-mutants scores
/// every spurious failure as a catch.
fn project(name: &str, config: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-prov-{name}-{}", std::process::id()));
    std::fs::create_dir_all(dir.join(".smysl")).expect("project dir");
    std::fs::write(dir.join(".smysl/config.hjson"), config).expect("write config");
    dir
}

/// A port with nothing behind it.
///
/// Bound and released, rather than a fixed number: a hard-coded port is a claim about the
/// machine, and the one test in this repository that made a claim about the machine is the
/// one that failed in CI. The kernel does not hand the same port straight back, so a connect
/// here is refused rather than answered — which is what makes the provider *unreachable*
/// without any packet leaving loopback.
fn closed_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    l.local_addr().expect("local addr").port()
}

/// A stub that answers every request with ollama's `/api/tags` body.
///
/// `probe()` asks `/api/tags` and then `/api/show`; only the first has to be right, because a
/// `show` it cannot parse falls back to the configured capabilities. So one canned response
/// serves both, and the provider comes back reachable with one model installed.
///
/// The thread is detached and the process is a test binary, so it ends when the test ends.
fn stub_ollama() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    std::thread::spawn(move || {
        let body = br#"{"models":[{"name":"llama3.2"}]}"#;
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let mut buf = [0u8; 8192];
            let _ = s.read(&mut buf);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n",
                body.len()
            );
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(body);
            let _ = s.flush();
        }
    });
    port
}

/// One provider named `id`, at `endpoint`.
///
/// Every provider below is `kind: ollama`, because that is the one mapper `local` compiles.
/// What distinguishes a local provider from a hosted one is the endpoint: `is_local` matches
/// loopback and nothing else, so `127.0.0.1` gives `caps().offline == true` and any other host
/// gives `false`. That is the property `--offline` rests on, and configuring it this way is
/// closer to the rule than naming a vendor would be.
fn provider(id: &str, endpoint: &str) -> String {
    format!(
        "    {id}: {{ kind: ollama, endpoint: \"{endpoint}\", model: \"llama3.2\", \
         context_window: 8192, max_output: 2048, structured: json-schema }}\n"
    )
}

fn config(providers: &str) -> String {
    format!(
        "{{\n  providers: {{\n{providers}  }}\n  \
         routing: {{ content-ingest: nearby }}\n  fallback: [nearby]\n}}\n"
    )
}

/// One provider, at whatever loopback port is given.
fn one_local(port: u16) -> String {
    config(&provider("nearby", &format!("http://127.0.0.1:{port}")))
}

/// Two providers, both loopback, for the tests that probe.
fn two_local(a: u16, b: u16) -> String {
    config(&format!(
        "{}{}",
        provider("nearby", &format!("http://127.0.0.1:{a}")),
        provider("alsonear", &format!("http://127.0.0.1:{b}"))
    ))
}

/// One local provider and one hosted one.
///
/// `api.example.com` is reserved for documentation (RFC 2606) and is never resolved: the only
/// test using this configuration passes `--offline`, where the refusal is decided from
/// `caps()` before a request is built. Nothing here dials it.
fn local_and_hosted(port: u16) -> String {
    config(&format!(
        "{}{}",
        provider("nearby", &format!("http://127.0.0.1:{port}")),
        provider("faraway", "https://api.example.com")
    ))
}

/// The line the listing ends with, and the one thing only the listing prints.
const HINT: &str = "(--probe contacts them; --tasks reports what would egress)";

/// One provider's row, found by its id rather than by counting columns.
///
/// The rows are `{id:<14}`-padded, and matching `"nearby         refused"` as a substring
/// would tie every assertion to that width. For a *negative* assertion that is worse than
/// brittle: change the padding and the string stops matching, so the check passes without
/// checking — the failure this repository keeps rediscovering. Looking the row up by its id
/// and asserting on its content fails loudly instead, and a missing row panics rather than
/// silently satisfying a `!contains`.
fn row<'a>(out: &'a str, id: &str) -> &'a str {
    out.lines()
        .find(|l| l.starts_with(id))
        .unwrap_or_else(|| panic!("no row for `{id}` in:\n{out}"))
}

// ---------------------------------------------------------------------------------------
// Listing versus probing
// ---------------------------------------------------------------------------------------

/// `if !m.get_flag("probe") && !m.get_flag("models")` chooses the listing. Deleting either `!`
/// makes a plain `smysl providers` fall through and contact every configured provider — which
/// is the one thing the command promises not to do until asked.
///
/// The providers here are at a closed port, so a build with that defect would still exit
/// cleanly-looking; what gives it away is the hint line, which only the listing prints, and
/// the exit code, which probing a dead endpoint would not leave at 0.
#[test]
fn a_plain_listing_contacts_nothing() {
    let dir = project("listing", &two_local(closed_port(), closed_port()));
    let out = run_in(&dir, &["providers"]);

    assert_eq!(
        out.code, SUCCESS,
        "listing cannot fail; a non-zero code means it probed. stderr: {}",
        out.stderr
    );
    assert!(
        out.stdout.contains(HINT),
        "the listing ends by saying how to contact them: {}",
        out.stdout
    );
    assert!(
        out.stdout.contains("nearby") && out.stdout.contains("alsonear"),
        "and lists what is configured: {}",
        out.stdout
    );
    assert!(
        !out.stdout.contains("down") && !out.stderr.contains("probing"),
        "nothing was contacted: {} / {}",
        out.stdout,
        out.stderr
    );
}

/// The other direction: `&&` mutated to `||` makes that condition true whenever *either* flag
/// is absent, so `--probe` prints the listing and contacts nothing.
///
/// Asserting on the hint line rather than on "it worked" is the point — a probe of a closed
/// port and a listing both produce plausible output, and only the hint tells them apart.
#[test]
fn probe_contacts_rather_than_lists() {
    let dir = project("probe", &one_local(closed_port()));
    let out = run_in(&dir, &["providers", "--probe"]);

    assert!(
        !out.stdout.contains(HINT),
        "--probe is not the listing: {}",
        out.stdout
    );
    assert!(
        row(&out.stdout, "nearby").contains("down"),
        "--probe reports what it found: {}",
        out.stdout
    );
    assert_ne!(
        out.code, SUCCESS,
        "a provider that could not be reached is not a success"
    );
}

// ---------------------------------------------------------------------------------------
// The line that replaces the progress bar
// ---------------------------------------------------------------------------------------

/// `if !style.is_enabled() && ids.len() > 1` prints one line up front when there is no bar to
/// watch, so a caller knows a silent wait is the network rather than a hang. It is deliberately
/// not printed for a single provider, where the wait is short and the line is noise.
///
/// Five of the twelve mutants live in that one condition — the `!`, the `&&`, and three
/// rewrites of `>` — and every one of them changes which of these two runs prints. Neither run
/// alone would find them; the pair is the test.
///
/// Stderr is a pipe under a test harness, so progress is off and `!style.is_enabled()` holds.
///
/// Both providers are loopback, because this is one of the two tests that actually probes.
#[test]
fn the_up_front_line_appears_only_when_there_is_more_than_one_provider() {
    let two = project("upfront-two", &two_local(closed_port(), closed_port()));
    let one = project("upfront-one", &one_local(closed_port()));

    let many = run_in(&two, &["providers", "--probe"]);
    assert!(
        many.stderr.contains("probing 2 provider(s)"),
        "with a bar suppressed and more than one provider, say what is being waited on: {}",
        many.stderr
    );

    let single = run_in(&one, &["providers", "--probe"]);
    assert!(
        !single.stderr.contains("probing"),
        "one provider is a short wait and needs no announcement: {}",
        single.stderr
    );
    // The control on that silence: the run did happen and did report.
    assert!(
        single.stdout.contains("nearby"),
        "the single provider was still probed: {}",
        single.stdout
    );
}

// ---------------------------------------------------------------------------------------
// `--offline`
// ---------------------------------------------------------------------------------------

/// `if registry.is_offline() && !p.caps().offline` refuses a hosted provider under `--offline`
/// before a request exists. Both halves matter and both are mutated: `&&` to `||` refuses
/// hosted providers even when `--offline` was never given, and deleting the `!` inverts which
/// provider is refused — the local one turned away and the hosted one contacted, which is the
/// precise opposite of the flag's purpose.
///
/// One run kills both, because both make the *local* provider refused under `--offline`, and
/// it must not be: `--offline` forbids leaving the machine, not using it.
#[test]
fn offline_refuses_the_hosted_provider_and_only_it() {
    let dir = project("offline", &local_and_hosted(closed_port()));
    let out = run_in(&dir, &["providers", "--offline", "--probe"]);

    assert_eq!(
        out.code, OFFLINE,
        "refusing a hosted provider is exit 7. stdout: {}",
        out.stdout
    );
    assert!(
        row(&out.stdout, "faraway").contains("refused: --offline and this provider is hosted"),
        "the hosted provider is refused, and told it was: {}",
        out.stdout
    );
    assert!(
        !row(&out.stdout, "nearby").contains("refused"),
        "--offline forbids leaving the machine, not using it — the local provider is still \
         contacted: {}",
        out.stdout
    );
    assert!(
        row(&out.stdout, "nearby").contains("down"),
        "and it was contacted, rather than merely not refused: {}",
        out.stdout
    );
}

// ---------------------------------------------------------------------------------------
// What a probe's answer means
// ---------------------------------------------------------------------------------------

/// `Ok(probe) if probe.reachable` separates a server that answered from one that did not.
/// Forced to `true` it reports a dead endpoint as `up`; forced to `false` it reports a live one
/// as `down`. Both are the same failure — a command whose entire output is a claim about
/// reachability, making that claim without reference to the answer.
///
/// So both sides are here, which is what needs the stub server: without one, every probe is
/// unreachable and half the guard is untestable.
#[test]
fn a_probe_reports_what_it_found() {
    let up = project("reachable", &one_local(stub_ollama()));
    let down = project("unreachable", &one_local(closed_port()));

    let up = run_in(&up, &["providers", "--probe"]);
    assert_eq!(
        up.code, SUCCESS,
        "a server that answered is a clean run. stdout: {} stderr: {}",
        up.stdout, up.stderr
    );
    assert!(
        row(&up.stdout, "nearby").contains(" up "),
        "a server that answered is up: {}",
        up.stdout
    );
    assert!(
        up.stdout.contains("llama3.2 installed"),
        "and the probe reports what it learned, rather than a bare verdict: {}",
        up.stdout
    );

    let down = run_in(&down, &["providers", "--probe"]);
    assert!(
        row(&down.stdout, "nearby").contains("down"),
        "a refused connection is down: {}",
        down.stdout
    );
    assert!(
        down.stdout.contains("no server at"),
        "and says so in terms of the endpoint that was tried: {}",
        down.stdout
    );
    assert_ne!(down.code, SUCCESS, "and does not exit 0");
}
