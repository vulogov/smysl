//! `xtask determinism` - enforces rule D (§25).
//!
//! Every operation except `ingest`, `attest`, and `thread --refine` MUST be a pure,
//! bit-reproducible function of its inputs. The harness runs each registered operation
//! twice under every environment permutation and asserts byte-identical stdout.
//!
//! The permutation matrix targets the four things that quietly break determinism:
//! locale-dependent collation and case folding, timezone-dependent formatting, and
//! hash-seed-dependent iteration over unordered collections.
//!
//! SM-P0 ships the harness with no operations registered. Each later phase registers its
//! operation as it lands: `pack` (SM-P9), `merge` (SM-P6), `derive_thread` (SM-P11),
//! `salience` (SM-P8), `render` (SM-P12).

use std::path::Path;
use std::process::Command;

/// One environment permutation.
#[derive(Debug, Clone, Copy)]
pub struct Env {
    pub lc_all: &'static str,
    pub tz: &'static str,
    pub hash_seed: &'static str,
}

const LOCALES: &[&str] = &["C", "ru_RU.UTF-8"];
const TIMEZONES: &[&str] = &["UTC", "Asia/Tokyo"];
const HASH_SEEDS: &[&str] = &["0", "42"];

/// The full permutation matrix, in a fixed order.
pub fn matrix() -> Vec<Env> {
    let mut out = Vec::with_capacity(LOCALES.len() * TIMEZONES.len() * HASH_SEEDS.len());
    for lc_all in LOCALES {
        for tz in TIMEZONES {
            for hash_seed in HASH_SEEDS {
                out.push(Env {
                    lc_all,
                    tz,
                    hash_seed,
                });
            }
        }
    }
    out
}

/// An operation whose output must be identical under every permutation.
pub struct Op {
    pub name: &'static str,
    /// Program and arguments, run from the workspace root.
    pub argv: &'static [&'static str],
}

/// Operations registered so far. Rule D names five; each is added by the phase that
/// implements it, so an unregistered entry here is a phase that has not landed yet.
const OPS: &[Op] = &[];

pub fn run(root: &Path) -> Result<(), String> {
    let envs = matrix();
    println!("  matrix: {} permutations", envs.len());

    if OPS.is_empty() {
        println!(
            "  no operations registered yet - pack, merge, derive_thread, salience, and \
             render register at SM-P9, P6, P11, P8, and P12"
        );
        return Ok(());
    }

    let mut failures = Vec::new();
    for op in OPS {
        let mut baseline: Option<(Env, Vec<u8>)> = None;
        for env in &envs {
            for pass in 0..2 {
                let out = capture(root, op, env).map_err(|e| format!("{}: {e}", op.name))?;
                match &baseline {
                    None => baseline = Some((*env, out)),
                    Some((first, bytes)) if *bytes != out => {
                        failures.push(format!(
                            "rule D: `{}` differs between {first:?} and {env:?} (pass {pass})",
                            op.name
                        ));
                    }
                    Some(_) => {}
                }
            }
        }
        println!("  {}: identical across {} runs", op.name, envs.len() * 2);
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n"))
    }
}

fn capture(root: &Path, op: &Op, env: &Env) -> Result<Vec<u8>, String> {
    let (prog, args) = op
        .argv
        .split_first()
        .ok_or_else(|| "empty argv".to_string())?;
    let out = Command::new(prog)
        .current_dir(root)
        .args(args)
        .env("LC_ALL", env.lc_all)
        .env("TZ", env.tz)
        .env("RUSTC_HASH_SEED", env.hash_seed)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "exited {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_covers_locale_timezone_and_hash_seed() {
        let m = matrix();
        assert_eq!(m.len(), 8);
        assert!(m.iter().any(|e| e.lc_all == "ru_RU.UTF-8"));
        assert!(m.iter().any(|e| e.tz == "Asia/Tokyo"));
        assert!(m.iter().any(|e| e.hash_seed == "42"));
    }

    #[test]
    fn matrix_order_is_fixed() {
        let a = matrix();
        let b = matrix();
        let key = |e: &Env| (e.lc_all, e.tz, e.hash_seed);
        assert_eq!(
            a.iter().map(key).collect::<Vec<_>>(),
            b.iter().map(key).collect::<Vec<_>>()
        );
    }

    /// The harness itself must detect a difference when there is one, and none when
    /// there is not. Exercised with a shell command rather than a smysl operation, since
    /// no operation is registered until SM-P6.
    #[test]
    fn harness_detects_identical_and_differing_output() {
        let root = Path::new(".");
        let stable = Op {
            name: "echo",
            argv: &["echo", "same"],
        };
        let envs = matrix();
        let first = capture(root, &stable, &envs[0]).unwrap();
        for env in &envs {
            assert_eq!(capture(root, &stable, env).unwrap(), first);
        }

        let varying = Op {
            name: "printenv",
            argv: &["printenv", "TZ"],
        };
        let a = capture(root, &varying, &envs[0]).unwrap();
        let differing = envs.iter().find(|e| e.tz != envs[0].tz).unwrap();
        assert_ne!(
            a,
            capture(root, &varying, differing).unwrap(),
            "the harness must be able to observe an environment difference"
        );
    }
}
