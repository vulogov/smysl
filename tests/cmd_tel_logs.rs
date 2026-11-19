//! `tel read journald/1|cloudevents/1`, `tel templates`, and events through `lib export
//! --with-substrate` (RFC SMYSL-3 §9.7, §19.5, §19.6, §21, TL-P9 step 3), against the binary.

#![cfg(all(feature = "cli", feature = "tel"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_smysl");
const AT: &str = "1768557700000";
const FROM: &str = "1768557600000";
const TO: &str = "1768557660000";
const CART: &str = r#"journald:{"host":"h1","unit":"cart.service"}"#;
const DEPLOYS: &str =
    r#"cloudevents:{"source":"/argocd/prod","type":"dev.cdevents.service.deployed.0.1.1"}"#;
const TRACE: &str = "4bf92f3577b34da6a3ce929d0e0e4736";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("smysl-tel-logs-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/tel")
        .join(rel)
        .display()
        .to_string()
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(BIN)
        .current_dir(dir)
        .args(args)
        .output()
        .expect("the binary runs")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let o = run(dir, args);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{args:?}: {}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn read(dir: &Path, reader: &str, file: &str, inst: &str) -> String {
    ok(
        dir,
        &[
            "tel",
            "read",
            reader,
            &fixture(file),
            "--instrument",
            inst,
            "--at",
            AT,
            "-s",
            "lib",
            "--yes",
        ],
    )
}

fn templates(dir: &Path, stream: &str, yes: bool) -> String {
    let mut args = vec!["tel", "templates", stream, FROM, TO, "-s", "lib"];
    if yes {
        args.extend(["--yes", "--at", AT]);
    }
    ok(dir, &args)
}

fn lines(out: &str, prefix: &str) -> Vec<String> {
    out.lines()
        .filter(|l| l.starts_with(prefix))
        .map(str::to_string)
        .collect()
}

/// A library holding the journal, the CloudEvents and the traces.
fn library(dir: &Path) {
    let o = read(dir, "journald/1", "journald/j0.json", "journald:h1");
    assert!(o.contains("9 record(s) in 2 stream(s)"), "{o}");
    read(
        dir,
        "cloudevents/1",
        "cloudevents/c0.json",
        "webhook:events",
    );
    for f in ["otlp-traces/t0.json", "otlp-traces/t1.json"] {
        read(dir, "otlp-traces-json/1", f, "otel-collector:c1");
    }
}

#[test]
fn a_journal_window_is_its_templates_and_they_commit_and_check() {
    let d = scratch("templates");
    library(&d);
    let t = templates(&d, CART, false);
    let got: Vec<String> = lines(&t, "template")
        .iter()
        .map(|l| l.split("  ").skip(3).collect::<Vec<_>>().join("  "))
        .collect();
    assert_eq!(
        got,
        vec![
            "count 4  connection from <IP> closed after <NUM> ms",
            "count 2  payment provider acme returned <NUM>",
            "count 2  retrying payment <UUID> in <NUM> ms",
        ],
        "{t}"
    );
    let e = templates(&d, DEPLOYS, false);
    assert!(
        e.contains("count 3  dev.cdevents.service.deployed.0.1.1 <*>"),
        "{e}"
    );

    templates(&d, CART, true);
    templates(&d, DEPLOYS, true);
    let c = ok(&d, &["tel", "check", "-s", "lib"]);
    assert!(
        c.contains("checked   0 window(s) and 4 template(s): 0 error(s), 0 warning(s)"),
        "{c}"
    );
    // Twice: the same units, nothing new.
    assert_eq!(
        lines(&templates(&d, CART, false), "template"),
        lines(&t, "template")
    );

    // A stream that is not a log is refused, and so is a window of nothing.
    let spans = run(
        &d,
        &[
            "tel",
            "templates",
            "b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            FROM,
            TO,
            "-s",
            "lib",
        ],
    );
    assert_ne!(spans.status.code(), Some(0));

    // Evicted: the templates stay, and are not re-derivable.
    ok(
        &d,
        &[
            "tel",
            "evict",
            "--older-than",
            "1d",
            "--at",
            "1768700000000",
            "-s",
            "lib",
            "--yes",
        ],
    );
    let c = ok(&d, &["tel", "check", "-s", "lib"]);
    assert_eq!(
        c.lines().filter(|l| l.contains("SMY-W605")).count(),
        4,
        "{c}"
    );
}

#[test]
fn events_travel_with_the_substrate_and_a_trace_is_found_again() {
    let (a, b) = (scratch("export"), scratch("import"));
    library(&a);
    templates(&a, CART, true);
    ok(
        &a,
        &["tel", "trace", TRACE, "-s", "lib", "--yes", "--at", AT],
    );
    let o = ok(
        &a,
        &[
            "lib",
            "export",
            "--scope",
            "all",
            "--with-substrate",
            "-o",
            "exp",
            "-s",
            "lib",
        ],
    );
    assert!(
        o.contains("events    6 day file(s), 18 event(s); 0 day(s) listed as evicted"),
        "{o}"
    );

    let exp = a.join("exp").display().to_string();
    let o = ok(&b, &["lib", "import", &exp, "--accept", "-s", "lib"]);
    assert!(
        o.contains("events    6 day file(s): 18 new event(s), 0 already held"),
        "{o}"
    );
    let c = ok(&b, &["tel", "check", "-s", "lib"]);
    assert!(
        c.contains("3 template(s)") && c.contains("4 span(s)"),
        "{c}"
    );
    assert!(!c.contains("SMY-W605") && !c.contains("SMY-E604"), "{c}");
    // The trace index was rebuilt on import.
    let ta = ok(&a, &["tel", "trace", TRACE, "-s", "lib"]);
    let tb = ok(&b, &["tel", "trace", TRACE, "-s", "lib"]);
    assert_eq!(lines(&ta, "span"), lines(&tb, "span"));
    assert_eq!(lines(&tb, "span").len(), 4);
    // A set: again, nothing new.
    let o = ok(&b, &["lib", "import", &exp, "--accept", "-s", "lib"]);
    assert!(o.contains("0 new event(s), 18 already held"), "{o}");
}
