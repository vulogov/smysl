//! `xtask check-native` - the no-C claims as a check rather than a sentence (SMYSL-2.5 §4).
//!
//! F-11 and draft 3 §18 say what native code a build of smysl compiles, and until TX-P10 step 1
//! nothing tested either: F-11 said "no `cc`" of a tree in which `cc` was present, as `blake3`'s
//! build-dependency, and was corrected by reading `cargo tree` by hand (SMYSL-2.5 §2.2). This is
//! that reading, for each feature set a user is offered, failing when:
//!
//! - any `-sys` crate is in the tree other than those in [`SYS_ALLOWED`]. Underscores count:
//!   `onig_sys` is how candle 0.10 would have brought Oniguruma in;
//! - `cc` is reached by any crate other than those in [`CC_ALLOWED`].
//!
//! The tree is the host's, as `cargo tree` gives it by default, with normal and build edges:
//! what compiles into the binary and what runs to compile it. Dev-dependencies are not the
//! user's build and are not read.

use std::path::Path;
use std::process::Command;

/// The feature sets of the facade a user is offered, each with defaults off so that a set is
/// exactly what it names.
pub const SETS: &[&str] = &["", "cli", "cli,semantic", "cli,semantic-candle"];

/// `-sys` crates that bind without compiling: `linux-raw-sys` is generated Rust for the
/// kernel's ABI, and has no build script.
pub const SYS_ALLOWED: &[&str] = &["linux-raw-sys"];

/// What may reach `cc`. `blake3`'s build script takes it whatever its features; with `pure`, as
/// the workspace selects, it compiles no C (SMYSL-2.5 §2.2, where that is recorded unverified
/// from a build log).
pub const CC_ALLOWED: &[&str] = &["blake3"];

pub fn run(root: &Path) -> Result<(), String> {
    let mut failures = Vec::new();
    for set in SETS {
        let label = if set.is_empty() {
            "--no-default-features".to_string()
        } else {
            format!("--no-default-features --features {set}")
        };
        let mut args = vec!["-p", "smysl", "--no-default-features", "-e", "normal,build"];
        if !set.is_empty() {
            args.extend(["--features", set]);
        }
        let tree = tree(root, &args)?;
        let sys = refused_sys(&tree);
        let cc = cc_reached_by(root, &args)?;
        let bad_cc: Vec<&String> = cc
            .iter()
            .filter(|c| !CC_ALLOWED.contains(&c.as_str()))
            .collect();
        if sys.is_empty() && bad_cc.is_empty() {
            println!("  {label}: clean ({} crates)", tree.len());
            continue;
        }
        for s in &sys {
            failures.push(format!("{label}: `{s}` is a -sys crate"));
        }
        for c in &bad_cc {
            failures.push(format!("{label}: `cc` is reached by `{c}`"));
        }
    }
    if failures.is_empty() {
        return Ok(());
    }
    Err(format!(
        "check-native: native code a build would compile\n  {}",
        failures.join("\n  ")
    ))
}

/// The `-sys` crates in a tree that are not allowed.
fn refused_sys(tree: &[String]) -> Vec<String> {
    tree.iter()
        .filter(|n| {
            (n.ends_with("-sys") || n.ends_with("_sys")) && !SYS_ALLOWED.contains(&n.as_str())
        })
        .cloned()
        .collect()
}

/// The crate names of a tree, each once, in order.
fn tree(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let mut names: Vec<String> = cargo_tree(root, args)?
        .into_iter()
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// The crates that depend on `cc` directly, or none if it is not in the tree.
fn cc_reached_by(root: &Path, args: &[&str]) -> Result<Vec<String>, String> {
    let mut with = args.to_vec();
    with.extend(["-i", "cc", "--depth", "1"]);
    let lines = match cargo_tree(root, &with) {
        Ok(l) => l,
        // `-i` names a package the tree must hold; a tree without `cc` is the clean answer.
        Err(e) if e.contains("did not match any packages") => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut names: Vec<String> = lines
        .iter()
        .skip(1)
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| !n.starts_with('['))
        .map(str::to_string)
        .collect();
    names.sort();
    names.dedup();
    Ok(names)
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
        .map(str::to_string)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sys_crate_is_refused_by_either_spelling_and_the_allowed_one_is_not() {
        let tree: Vec<String> = ["linux-raw-sys", "onig_sys", "openssl-sys", "syscall"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(refused_sys(&tree), ["onig_sys", "openssl-sys"]);
    }
}
