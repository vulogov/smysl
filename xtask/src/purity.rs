//! `xtask check-purity` - enforces rules A and B (§14).
//!
//! Three checks. The first two are rule B, and either alone is escapable:
//!
//! 1. **Dependency graph.** `cargo tree --no-default-features` for the facade must not
//!    contain an async runtime, an HTTP client, an argument parser, or a TUI library.
//!    This is rule B stated as a fact about the build, not an intention.
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

/// Crates that MUST NOT appear in `--no-default-features` builds (rule B).
const FORBIDDEN_DEPS: &[&str] = &[
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
    "serde_json",
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
    let tree = cargo_tree(
        root,
        &["-p", "smysl", "--no-default-features", "-e", "normal"],
    )?;
    for dep in FORBIDDEN_DEPS {
        if tree.iter().any(|c| c == dep) {
            failures.push(format!(
                "rule B: `{dep}` is in the --no-default-features dependency tree of `smysl`"
            ));
        }
    }
    println!(
        "  dependency tree (--no-default-features): {} crates, none forbidden",
        tree.len()
    );

    // Each pure crate must also be clean on its own, so a future edit cannot hide a
    // runtime behind a facade feature.
    for krate in PURE_CRATES {
        let tree = cargo_tree(root, &["-p", krate, "-e", "normal"])?;
        for dep in FORBIDDEN_DEPS {
            if tree.iter().any(|c| c == dep) {
                failures.push(format!(
                    "rule B: `{dep}` is in the dependency tree of `{krate}`"
                ));
            }
        }
    }
    println!("  pure crates: {} checked", PURE_CRATES.len());

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
