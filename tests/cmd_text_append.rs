//! `text append`: growth (SMYSL-2.4 §5.2, TX-P2 step 3's exit).
//!
//! Three days of a Telegram export appended one day at a time, through the binary. What the
//! library's own suite asserts about identities, this asserts about the **command**: that the
//! second version supersedes the first, that the alias keeps one head, that an append restates
//! nothing the head manifest already says, and that `SMY-E450` refuses an append to a
//! pseudonymised expression with no key.
//!
//! # The one qualification §3.1 needed
//!
//! "Reuses tids for unchanged parts" is true with one word added: a part that **was the last
//! one** is rewritten exactly once. Parts partition the text — a cut may lose no byte (TX-P1
//! step 6) — and the separator between two parts belongs to the earlier of them, so the day
//! that was last gains the space that now joins it to its successor. Every part before it is
//! byte-identical. The cost of an append is therefore one rewritten part whatever the size of
//! the corpus, which this file measures rather than assumes: `objects   4 written` at every
//! step, not 2, 4, 6.

// `cli` and nothing else: `reader-telegram` is a feature of `smysl-text`, and `cli` turns it
// on. A gate naming it here would name a feature this crate does not have, and the suite would
// compile to nothing — which is how it was first written, and which `0 tests` is the only sign
// of.
#![cfg(feature = "cli")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");

/// A part per day needs a policy that does not group them: the default minimum is 64 KiB, and
/// a few hundred bytes of chat is one part under it. Stated here because tid reuse is a
/// property of the policy and not of `append`.
const PER_DAY: &str = "smysl/parts/1 level=day min=64 max=4096";

/// A Telegram export of `days` days, one message each, padded past the policy's minimum.
fn export(days: usize) -> String {
    let mut s =
        String::from(r#"{"name":"Reading group","type":"private_group","id":77,"messages":["#);
    for day in 0..days {
        if day > 0 {
            s.push(',');
        }
        // 1705314225 is 2024-01-15T10:23:45Z, then one message per day.
        let at = 1_705_314_225 + day as u64 * 86_400;
        let filler = "and then we talked about the chapter for a while. ".repeat(6);
        s.push_str(&format!(
            r#"{{"id":{},"type":"message","date_unixtime":"{at}","from":"Ada","from_id":"user111","text":"Day {}: {filler}"}}"#,
            day + 1,
            day + 1
        ));
    }
    s.push_str("]}");
    s
}

/// One directory per test: these run as threads in one binary, and a shared library root would
/// let one test append while another reads.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "smysl-append-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn write(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("the export");
    path.to_string_lossy().into_owned()
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN)
        .args(args)
        .output()
        .expect("the binary under test must run")
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// The value of a printed `key   value` line.
fn field(out: &str, key: &str) -> String {
    out.lines()
        .find_map(|l| l.trim().strip_prefix(key))
        .unwrap_or_else(|| panic!("no `{key}` line in:\n{out}"))
        .trim()
        .to_string()
}

fn add(root: &Path, file: &str, extra: &[&str]) -> Output {
    let root = root.to_string_lossy().into_owned();
    let mut args = vec![
        "text",
        "add",
        file,
        "--reader",
        "telegram/1",
        "--alias",
        "rg",
        "--lang",
        "mul",
        "--licence",
        "CC0-1.0",
        "--part-policy",
        PER_DAY,
        "-s",
        &root,
    ];
    args.extend_from_slice(extra);
    run(&args)
}

fn append(root: &Path, file: &str, extra: &[&str]) -> Output {
    let root = root.to_string_lossy().into_owned();
    let mut args = vec!["text", "append", file, "--alias", "rg", "-s", &root];
    args.extend_from_slice(extra);
    run(&args)
}

/// Three days, one at a time: a supersedes chain, one head, and one part rewritten each time.
#[test]
fn three_days_appended_one_at_a_time_grow_one_expression() {
    let dir = scratch("growth");
    let day1 = write(&dir, "d1.json", &export(1));
    let day2 = write(&dir, "d2.json", &export(2));
    let day3 = write(&dir, "d3.json", &export(3));

    let first = add(&dir, &day1, &[]);
    assert!(first.status.success(), "{}", text(&first));
    let first_out = text(&first);
    let first_mid = field(&first_out, "manifest");
    assert_eq!(field(&first_out, "parts"), "1");

    let second = append(&dir, &day2, &[]);
    assert!(second.status.success(), "{}", text(&second));
    let second_out = text(&second);
    assert_eq!(field(&second_out, "supersedes"), first_mid);
    assert_eq!(field(&second_out, "parts"), "2");
    // Four: the new day, and the day that was last, each as a part and a reading.
    assert_eq!(field(&second_out, "objects"), "4 written");
    let second_mid = field(&second_out, "manifest");
    assert_ne!(second_mid, first_mid);

    let third = append(&dir, &day3, &[]);
    assert!(third.status.success(), "{}", text(&third));
    let third_out = text(&third);
    assert_eq!(field(&third_out, "supersedes"), second_mid);
    assert_eq!(field(&third_out, "parts"), "3");
    assert_eq!(
        field(&third_out, "objects"),
        "4 written",
        "the cost of an append does not grow with the corpus"
    );

    // Part 0 is the same object in both versions: it stopped being the last part at the second
    // version, and nothing after that touches a byte of it.
    let part0 = |out: &str| field(out, "part 0");
    assert_eq!(part0(&second_out), part0(&third_out), "day one reused");
    assert_ne!(
        part0(&first_out),
        part0(&second_out),
        "it gained a separator"
    );

    // One head, three versions, and `ls` says so without reporting a fork.
    let root = dir.to_string_lossy().into_owned();
    let ls = run(&["text", "ls", "-s", &root]);
    assert!(ls.status.success(), "{}", text(&ls));
    let listed = text(&ls);
    assert!(listed.contains(&field(&third_out, "manifest")), "{listed}");
    assert!(!listed.to_lowercase().contains("fork"), "{listed}");
    assert_eq!(
        listed.matches("m3:").count(),
        1,
        "one head, not three: {listed}"
    );

    // And the newest day is reachable through the alias.
    let show = run(&["text", "show", "rg#chat.20240117.1", "--raw", "-s", &root]);
    assert!(show.status.success(), "{}", text(&show));
    assert!(text(&show).starts_with("Day 3:"), "{}", text(&show));

    let _ = std::fs::remove_dir_all(&dir);
}

/// Appending the file the corpus already holds writes no version and says so.
#[test]
fn an_unchanged_export_is_not_a_new_version() {
    let dir = scratch("noop");
    let two = write(&dir, "d2.json", &export(2));
    assert!(add(&dir, &two, &[]).status.success());

    let again = append(&dir, &two, &[]);
    assert!(again.status.success(), "{}", text(&again));
    let out = text(&again);
    assert!(out.contains("unchanged"), "{out}");
    assert!(!out.contains("supersedes"), "nothing was superseded: {out}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// An append does not restate the reader, the licence or the policy — and refuses to be given
/// them, rather than ignoring them.
///
/// Ignoring a flag is the failure mode worth a test: an operator who passed
/// `--part-policy level=chapter` and had it dropped would have a second version they believe
/// was cut one way and a corpus cut another.
#[test]
fn an_append_refuses_the_flags_it_takes_from_the_head_manifest() {
    let dir = scratch("flags");
    let one = write(&dir, "d1.json", &export(1));
    let two = write(&dir, "d2.json", &export(2));
    assert!(add(&dir, &one, &[]).status.success());

    for flag in [
        ["--reader", "txt/1"],
        ["--licence", "CC0-1.0"],
        ["--lang", "en"],
        ["--carry", "none"],
        ["--part-policy", "smysl/parts/1 level=day min=4096 max=8192"],
    ] {
        let out = append(&dir, &two, &flag);
        assert!(!out.status.success(), "{} was accepted", flag[0]);
        let shown = text(&out);
        assert!(shown.contains(flag[0]), "{shown}");
        assert!(shown.contains("head manifest"), "{shown}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--alias` is required, because an append has to name what it appends to.
#[test]
fn an_append_with_no_alias_says_what_is_missing() {
    let dir = scratch("alias");
    let one = write(&dir, "d1.json", &export(1));
    let root = dir.to_string_lossy().into_owned();
    let out = run(&["text", "append", &one, "-s", &root]);
    assert!(!out.status.success());
    let shown = text(&out);
    assert!(shown.contains("--alias"), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `SMY-E450`: an append to a pseudonymised expression without its key.
#[test]
fn appending_to_a_pseudonymised_expression_needs_its_key() {
    let dir = scratch("e450");
    let one = write(&dir, "d1.json", &export(1));
    let two = write(&dir, "d2.json", &export(2));
    let added = add(&dir, &one, &["--pseudonymise"]);
    assert!(added.status.success(), "{}", text(&added));

    let refused = append(&dir, &two, &[]);
    assert!(!refused.status.success());
    let shown = text(&refused);
    assert!(shown.contains("SMY-E450"), "{shown}");
    assert!(shown.contains("second speaker"), "{shown}");

    // With the flag the key is read back and the append goes through.
    let ok = append(&dir, &two, &["--pseudonymise"]);
    assert!(ok.status.success(), "{}", text(&ok));
    let out = text(&ok);
    assert!(out.contains("pseudonymised under"), "{out}");
    // The key's own bytes are never printed, here or anywhere.
    assert!(!out.contains("pseudonym.key\n  "), "{out}");

    // And the speaker in the corpus is a pseudonym, not a name.
    let root = dir.to_string_lossy().into_owned();
    let show = run(&[
        "text",
        "show",
        "rg#chat.20240116",
        "--segments",
        "-s",
        &root,
    ]);
    assert!(show.status.success(), "{}", text(&show));
    let shown = text(&show);
    assert!(shown.contains("spk:"), "{shown}");
    assert!(!shown.contains("user111"), "{shown}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `--pseudonymise` on an append never creates a key, and says why.
///
/// A created key would be a *different* key from the one the expression's speakers were derived
/// under, so every appended message would get a second pseudonym for somebody already in the
/// corpus — the failure `SMY-E450` exists to prevent, reached through the flag meant to prevent
/// it.
#[test]
fn an_append_will_not_mint_a_pseudonym_key() {
    let dir = scratch("nokey");
    let one = write(&dir, "d1.json", &export(1));
    let two = write(&dir, "d2.json", &export(2));
    assert!(add(&dir, &one, &[]).status.success());

    let out = append(&dir, &two, &["--pseudonymise"]);
    assert!(!out.status.success());
    let shown = text(&out);
    assert!(shown.contains("secrets/pseudonym.key"), "{shown}");
    assert!(shown.contains("cannot"), "{shown}");
    assert!(
        !dir.join("secrets").exists(),
        "the refusal created a key anyway"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two heads under one alias: `SMY-W418`, reported by `ls --forks`, and an append refused.
///
/// The other half of §5.2's growth item. A fork is what two machines appending to one
/// expression produces, and it is a *fact about a corpus* rather than an error — so `ls` reports
/// it and `append` refuses to guess which side was meant. Naming one head by mid appends to
/// that side and leaves the other a head: merging two heads is a different operation.
#[test]
fn a_fork_is_reported_and_an_append_refuses_to_pick_a_side() {
    let dir = scratch("fork");
    let one = write(&dir, "d1.json", &export(1));
    let two = write(&dir, "d2.json", &export(2));
    let three = write(&dir, "d3.json", &export(3));
    // Two adds of two different texts under one alias, neither superseding the other.
    assert!(add(&dir, &one, &[]).status.success());
    assert!(add(&dir, &two, &[]).status.success());

    let root = dir.to_string_lossy().into_owned();
    let forks = run(&["text", "ls", "--forks", "-s", &root]);
    assert!(forks.status.success(), "{}", text(&forks));
    let listed = text(&forks);
    assert!(listed.contains("SMY-W418"), "{listed}");
    assert_eq!(listed.matches("m3:").count(), 2, "two heads: {listed}");

    let refused = append(&dir, &three, &[]);
    assert!(!refused.status.success());
    let shown = text(&refused);
    assert!(shown.contains("2 heads"), "{shown}");
    assert!(shown.contains("SMY-W418"), "{shown}");

    // One of the two, by mid. The other is still a head afterwards.
    let head = listed
        .split_whitespace()
        .find(|w| w.starts_with("m3:"))
        .expect("a head in the listing")
        .to_string();
    let out = run(&["text", "append", &three, "--alias", &head, "-s", &root]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(field(&text(&out), "supersedes"), head);
    let after = run(&["text", "ls", "--forks", "-s", &root]);
    assert_eq!(
        text(&after).matches("m3:").count(),
        2,
        "appending to one side does not merge the fork: {}",
        text(&after)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// An alias nobody catalogued is a refusal that names it.
#[test]
fn appending_to_an_unknown_alias_names_it() {
    let dir = scratch("unknown");
    let one = write(&dir, "d1.json", &export(1));
    assert!(add(&dir, &one, &[]).status.success());

    let root = dir.to_string_lossy().into_owned();
    let out = run(&["text", "append", &one, "--alias", "nobody", "-s", &root]);
    assert!(!out.status.success());
    assert!(text(&out).contains("nobody"), "{}", text(&out));
    let _ = std::fs::remove_dir_all(&dir);
}
