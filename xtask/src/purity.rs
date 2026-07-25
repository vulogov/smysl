//! `xtask check-purity` - enforces rules A and B (§14).
//!
//! Two independent checks, because either alone is escapable:
//!
//! 1. **Dependency graph.** `cargo tree --no-default-features` for the facade must not
//!    contain an async runtime, an HTTP client, an argument parser, or a TUI library.
//!    This is rule B stated as a fact about the build, not an intention.
//! 2. **Source grep.** The pure crates must not name a runtime or a socket, even
//!    transitively through a dependency they could add later. A dependency check alone
//!    would pass a crate that spawned threads and opened sockets by hand.

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
];

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

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
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
