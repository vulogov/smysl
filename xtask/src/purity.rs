//! `xtask check-purity` - enforces rules A and B (§14).
//!
//! Three checks. The first two are rule B, and either alone is escapable:
//!
//! 1. **Dependency graph.** `cargo tree --no-default-features` for the facade must not
//!    contain an async runtime, an HTTP client, an argument parser, or a TUI library.
//!    This is rule B stated as a fact about the build, not an intention. Each pure crate is
//!    checked on its own as well, and for the runtime and socket crates at `--all-features`
//!    too — see `NEVER` and `NOT_IN_THE_CORE`, which 1.10 split apart because one list was
//!    carrying two different claims and only the weaker of the two was being enforced.
//! 2. **Source grep.** The pure crates must not name a runtime or a socket, even
//!    transitively through a dependency they could add later. A dependency check alone
//!    would pass a crate that spawned threads and opened sockets by hand.
//! 3. **Rule A.** The CLI may not reach past the facade. See `RULE_A_EXEMPT_FILES` below.
//!
//! Until 0.13 this file claimed rules A and B in its first line and checked only B. Rule A
//! was the older claim of the two — `src/lib.rs` states it, and the manual restates it as a
//! fact already verified — and it was the one nothing tested. It was also false when the
//! check was finally written, which is the argument for writing it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Crates a pure crate must not link **under any feature combination**.
///
/// Rule B is "the library stays synchronous and offline", and a claim of that shape is not
/// true at default features and false behind a flag: an offline library is offline however it
/// is configured. So this set is checked at `--all-features` as well, which is what the single
/// list below did not do — a pure crate could have put `tokio` behind a non-default feature and
/// the gate would have passed it. `clap`, `ratatui` and `crossterm` are here for the adjacent
/// claim that the library is not a front end; a library that grew an argument parser behind a
/// feature would have the same problem.
const NEVER: &[&str] = &[
    "tokio",
    "ureq",
    "clap",
    "ratatui",
    "crossterm",
    "reqwest",
    "hyper",
    "async-std",
    "smol",
    "rustls",
];

/// Crates that must not be in a pure crate's **default** tree, but may be reached through a
/// named non-default feature — with the reason, and the feature that is allowed to pull them.
///
/// `serde_json` sat in the single list beside `tokio` for releases, and the two are not the
/// same claim. It links no runtime, opens no socket, reads no clock and is deterministic; its
/// `rust-version` is 1.71, below this workspace's base. What keeping it out buys is a pure core
/// with no serde stack in it, which is worth having and is not rule B.
///
/// SMYSL-2.4 OQ-37 is what forced the distinction. The alternative was a strict-JSON mode of
/// `smysl-core`'s HJSON parser, so that `json/1` and `telegram/1` could stay inside the gate;
/// measured against the inputs a JSON reader meets, that is new code rather than a narrowing,
/// and it is new code in the place where untrusted bytes arrive. The answer is `serde_json`
/// behind `reader-json`, and this list is where that answer is written down.
/// The three that arrived with TX-P1 step 3 are the readers' parsers, and each was measured
/// before it was adopted rather than after: `quick-xml` 0.41.0 is MIT and declares 1.79,
/// `pulldown-cmark` 0.13.4 is MIT and declares 1.71.1, `serde` and `serde_json` declare 1.71 —
/// all below this workspace's 1.85 base, so none of them raises a floor. (0.42 of `quick-xml`
/// declares 1.86 and is therefore **not** the pinned version: an XML parser is not a reason to
/// move the pure tier.) All four are pinned with `=` in the workspace manifest, because a
/// reader's output is a corpus's identity and a parser that changed its mind between patch
/// releases would change every tid taken with it.
const NOT_IN_THE_CORE: &[(&str, &str)] = &[
    (
        "serde_json",
        "only through a `reader-*` feature of `smysl-text` (SMYSL-2.4 OQ-37)",
    ),
    (
        "serde",
        "only through `reader-json` of `smysl-text`: `serde_json`'s own seam",
    ),
    (
        "quick-xml",
        "only through `reader-osis` and `reader-zefania` of `smysl-text`",
    ),
    ("pulldown-cmark", "only through `reader-md` of `smysl-text`"),
];

/// The pure crates. Every operation they expose is a bit-reproducible function of its
/// inputs (rule D), so none of them may reach the network or link a runtime.
const PURE_CRATES: &[&str] = &[
    "smysl-core",
    "smysl-graph",
    "smysl-check",
    "smysl-pack",
    "smysl-thread",
    "smysl-render",
    // Retrieval is pure because its default engine is: BM25 with `default-features = false`
    // brings in one transitive crate and no runtime. That is a claim worth enforcing rather
    // than asserting — a semantic backend added later would break it the moment it landed,
    // which is exactly when someone should have to think about it.
    "smysl-retrieve",
    // The library layer (SMYSL-2.4 §4.5, OQ-37). `default = []`, so its default tree has no
    // serde stack, and the readers that need one arrive each behind its own `reader-*`
    // feature — outside the default tree and inside the `--all-features` one, which is why
    // this list is now checked against both. Registered in TX-P1 step 2 rather than step 6,
    // where the plan put it: the readers land in step 3, and a gate that arrives after the
    // code it is meant to constrain is a gate that has to be argued with instead of obeyed.
    "smysl-text",
];

/// Rule A, as `src/lib.rs` states it: *"no CLI capability may be unreachable from here, and
/// no code path may be CLI-only."*
///
/// The mechanical form: nothing under `src/` except the facade itself may name a sibling
/// crate. `smysl::` is the facade and is always fine; `smysl_core::`, `smysl_ingest::` and
/// the rest are the CLI helping itself to something a library consumer cannot reach.
///
/// This is stricter than the rule needs to be — a sibling path is evidence of a bypass, not
/// proof of one, since the facade might re-export the same item under another name. Strictness
/// is the point: the cheap fix is to route through the facade, and the check is worth more
/// than the handful of paths it will ever reject.
const RULE_A_EXEMPT_FILES: &[&str] = &[
    // The facade. Naming sibling crates is its entire job.
    "lib.rs",
];

/// Deliberate exceptions, each with the reason it is allowed.
///
/// Empty, and it should be argued over before it is not. An exception here is a CLI
/// capability that a consumer of the library cannot reach, which is the thing rule A exists
/// to prevent — so the bar is not "this was inconvenient to re-export".
const RULE_A_ALLOWED: &[(&str, &str)] = &[];

/// Symbols that betray a runtime or a socket in source.
const FORBIDDEN_SYMBOLS: &[&str] = &[
    "reqwest",
    "tokio",
    "ureq",
    "std::net",
    "TcpStream",
    "UdpSocket",
    "async fn",
];

pub fn run(root: &Path) -> Result<(), String> {
    let mut failures = Vec::new();

    // --- 1. dependency graph -----------------------------------------------
    // The facade with nothing enabled is the library a consumer gets by default, so both
    // claims apply to it.
    let tree = cargo_tree(
        root,
        &["-p", "smysl", "--no-default-features", "-e", "normal"],
    )?;
    for dep in NEVER {
        if tree.iter().any(|c| c == dep) {
            failures.push(format!(
                "rule B: `{dep}` is in the --no-default-features dependency tree of `smysl`"
            ));
        }
    }
    for (dep, allowance) in NOT_IN_THE_CORE {
        if tree.iter().any(|c| c == dep) {
            failures.push(format!(
                "`{dep}` is in the --no-default-features dependency tree of `smysl`; it is \
                 permitted {allowance}, which is not a default"
            ));
        }
    }
    println!(
        "  dependency tree (--no-default-features): {} crates, none forbidden",
        tree.len()
    );

    // Each pure crate must also be clean on its own, so a future edit cannot hide a
    // runtime behind a facade feature.
    //
    // Twice over, because the two lists are checked against different trees. The default tree
    // is the pure core, and `NOT_IN_THE_CORE` is a statement about it. `NEVER` is a statement
    // about the crate, so it is checked at `--all-features` too: this is the hole the single
    // list left, and TX-P1 is about to widen it — `smysl-text` joins this list with
    // `default = []` and a feature per reader, so "clean at default features" would stop being
    // much of a claim about it.
    for krate in PURE_CRATES {
        let default_tree = cargo_tree(root, &["-p", krate, "-e", "normal"])?;
        for (dep, allowance) in NOT_IN_THE_CORE {
            if default_tree.iter().any(|c| c == dep) {
                failures.push(format!(
                    "`{dep}` is in the default dependency tree of `{krate}`; it is permitted \
                     {allowance}, which is not a default"
                ));
            }
        }
        let all_tree = cargo_tree(root, &["-p", krate, "-e", "normal", "--all-features"])?;
        for dep in NEVER {
            // Reported against whichever tree holds it, so the message says where to look.
            let where_ = if default_tree.iter().any(|c| c == dep) {
                "dependency tree"
            } else if all_tree.iter().any(|c| c == dep) {
                "--all-features dependency tree"
            } else {
                continue;
            };
            failures.push(format!("rule B: `{dep}` is in the {where_} of `{krate}`"));
        }
    }
    println!(
        "  pure crates: {} checked, at default features and at --all-features",
        PURE_CRATES.len()
    );

    // --- 2. source grep -----------------------------------------------------
    let mut scanned = 0usize;
    for krate in PURE_CRATES {
        let src = root.join("crates").join(krate).join("src");
        for file in rust_files(&src)? {
            scanned += 1;
            let text =
                std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            for (n, line) in text.lines().enumerate() {
                // Comments and doc comments may name these symbols; code may not.
                let code = line.split("//").next().unwrap_or("");
                for sym in FORBIDDEN_SYMBOLS {
                    if code.contains(sym) {
                        failures.push(format!("rule B: `{sym}` in {}:{}", file.display(), n + 1));
                    }
                }
            }
        }
    }
    println!(
        "  source scan: {scanned} files, {} symbols",
        FORBIDDEN_SYMBOLS.len()
    );

    // --- 3. rule A: the CLI goes through the facade -------------------------
    let mut checked = 0usize;
    for file in rust_files(&root.join("src"))? {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if RULE_A_EXEMPT_FILES.contains(&name.as_str()) {
            continue;
        }
        checked += 1;
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        for (n, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for path in sibling_paths(code) {
                if RULE_A_ALLOWED.iter().any(|(p, _)| *p == path) {
                    continue;
                }
                failures.push(format!(
                    "rule A: `{path}` in {}:{} — the CLI reached past the facade. Re-export it \
                     from src/lib.rs and use `smysl::`, or add it to RULE_A_ALLOWED with the \
                     reason a library consumer is not meant to have it.",
                    file.display(),
                    n + 1
                ));
            }
        }
    }
    println!("  rule A: {checked} CLI files reach only the facade");

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

/// Every `smysl_*::` path named in a line of code.
///
/// `smysl::` — the facade — is not one of these: the underscore is what distinguishes a
/// sibling crate from the crate the CLI is supposed to be built on. `smysl_core::Uid` matches;
/// `smysl::Uid` does not.
fn sibling_paths(code: &str) -> Vec<String> {
    let bytes = code.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(rel) = code[i..].find("smysl_") {
        let start = i + rel;
        // Not a match if it is the tail of a longer identifier, e.g. `my_smysl_thing`.
        let preceded = start > 0
            && (bytes[start - 1] == b'_' || (bytes[start - 1] as char).is_ascii_alphanumeric());
        let mut end = start;
        while end < bytes.len()
            && ((bytes[end] as char).is_ascii_alphanumeric() || bytes[end] == b'_')
        {
            end += 1;
        }
        // Only a path if `::` follows; a bare mention in a string or an ident is not a use.
        if !preceded && code[end..].starts_with("::") {
            out.push(code[start..end].to_string());
        }
        i = end.max(start + 1);
    }
    out
}

fn cargo_tree(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .current_dir(root)
        .arg("tree")
        .args(args)
        .args(["--prefix", "none"])
        .output()
        .map_err(|e| format!("cargo tree: {e}"))?;

    if !out.status.success() {
        return Err(format!(
            "cargo tree {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

fn rust_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if !dir.exists() {
        return Ok(out);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    // Deterministic order, so failures are reported the same way every run.
    out.sort();
    Ok(out)
}
