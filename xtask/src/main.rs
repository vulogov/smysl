//! Build tasks for the smysl workspace.
//!
//! ```text
//! cargo xtask check-purity   # rules A and B (§14)
//! cargo xtask determinism    # rule D (§25)
//! ```
//!
//! Both are CI gates. They exist because the properties they check - a synchronous,
//! network-free core, and bit-reproducible pure operations - are the kind that lapse
//! silently unless the build fails when they do.

mod determinism;
mod purity;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let root = workspace_root();
    let task = std::env::args().nth(1);

    let result = match task.as_deref() {
        Some("check-purity") => {
            println!("xtask check-purity (rules A, B)");
            purity::run(&root)
        }
        Some("determinism") => {
            println!("xtask determinism (rule D)");
            determinism::run(&root)
        }
        Some("all") => {
            println!("xtask check-purity (rules A, B)");
            purity::run(&root).and_then(|()| {
                println!("xtask determinism (rule D)");
                determinism::run(&root)
            })
        }
        Some(other) => Err(format!("unknown task `{other}`")),
        None => {
            usage();
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => {
            println!("ok");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "usage: cargo xtask <task>\n\n\
         tasks:\n  \
         check-purity   assert the pure crates link no runtime, HTTP client, or arg parser\n  \
         determinism    assert every pure operation is bit-reproducible\n  \
         all            both of the above"
    );
}

/// The workspace root, derived from this crate's manifest directory.
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives one level below the workspace root")
        .to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_root_contains_the_facade_manifest() {
        let root = workspace_root();
        assert!(root.join("Cargo.toml").is_file());
        assert!(root.join("crates").is_dir());
    }
}
