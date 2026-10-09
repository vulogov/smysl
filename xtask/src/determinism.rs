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
//! `salience` (SM-P8), `render` (SM-P12), and `text add` / `text show` (TX-P1).
//!
//! # Operations that write
//!
//! Rule D's five all read a store and print. TX-P1's `text add` writes one, and a harness that
//! ran one fixed argv twice could not compare it: the second run sees what the first did, and
//! `text add` prints how many objects it wrote — 2 into an empty library, 0 into one that
//! already holds the text, both correct and different bytes. That is why `text add` was left
//! unregistered when it landed, with the exemption stated rather than argued.
//!
//! [`SCRATCH`] is the fix, and it is small: a token in `argv` that becomes a directory made
//! fresh for each one of the sixteen captures and removed afterwards. Rule D's question for a
//! writing operation is then the one it should have been all along — the same input *and the
//! same starting state* give the same output — and [`Op::setup`] builds that state for an
//! operation which needs one, like `text show`, that cannot run against a library nobody made.
//! TX-P3's `date set` has the same shape and needs nothing further here.

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

/// The token an operation's argv uses to name a scratch directory of its own.
///
/// An operation that **writes** cannot be run twice over the same directory and compared: the
/// second run sees what the first one did. `text add` prints how many objects it wrote, which
/// is 2 into an empty library and 0 into one that already holds the text — both correct, and
/// different bytes. So rule D's question for a writing operation is "the same input and the
/// same *starting state* give the same output", and the starting state has to be made.
///
/// Every occurrence of this token in `argv` or `setup` is replaced by a directory created fresh
/// for that one capture and removed afterwards. It is outside the repository, because a harness
/// that dirtied the tree would be one step from a harness that depended on it.
pub const SCRATCH: &str = "%SCRATCH%";

/// An operation whose output must be identical under every permutation.
pub struct Op {
    pub name: &'static str,
    /// Program and arguments, run from the workspace root.
    pub argv: &'static [&'static str],
    /// A command run before `argv`, in the same scratch directory, whose output is discarded.
    ///
    /// For an operation that reads what another one wrote: `text show` needs a library, and the
    /// only honest way to get one is to run `text add`. Its output is not compared — the
    /// operation under test is the second command — but its *failure* is, because a setup that
    /// silently did nothing would leave the real command reporting "no such alias" identically
    /// across all sixteen runs and passing.
    pub setup: Option<&'static [&'static str]>,
}

impl Op {
    /// Whether this operation needs a directory of its own.
    fn needs_scratch(&self) -> bool {
        self.argv
            .iter()
            .chain(self.setup.unwrap_or(&[]).iter())
            .any(|a| a.contains(SCRATCH))
    }
}

/// Operations registered so far. Rule D names five; each is added by the phase that
/// implements it, so an unregistered entry here is a phase that has not landed yet.
const OPS: &[Op] = &[
    Op {
        name: "pack",
        // The whole pipeline in one call: salience, closure expansion, greedy selection
        // and local improvement, all of which must be bit-reproducible.
        argv: &[
            "cargo",
            "run",
            "--quiet",
            "--no-default-features",
            "--features",
            "cli",
            "--",
            "--format",
            "surface",
            "pack",
            "--budget",
            "120",
            "fixtures/corpus/F1-incident.smy",
        ],
        setup: None,
    },
    Op {
        name: "salience",
        // Personalised PageRank accumulates in f64 over a fixed number of iterations in
        // dense-id order, then quantises once. Locale, timezone and hash seed must not
        // reach any of that.
        argv: &[
            "cargo",
            "run",
            "--quiet",
            "--no-default-features",
            "--features",
            "cli",
            "--",
            "salience",
            "fixtures/corpus/F1-incident.smy",
        ],
        setup: None,
    },
    Op {
        name: "merge",
        // Two corpus stores with an overlapping unit and a rebuttal between them. Merge
        // supplies its own clock rather than reading one, so the only thing that could vary
        // between runs is the implementation.
        argv: &[
            "cargo",
            "run",
            "--quiet",
            "--no-default-features",
            "--features",
            "cli",
            "--",
            "merge",
            "fixtures/corpus/F1-incident.smy",
            "fixtures/corpus/F6-adversarial.smy",
        ],
        setup: None,
    },
    Op {
        name: "derive_thread",
        // Role assignment, salience-ranked selection, Kahn ordering and coherence repair.
        // The narrative schema is the interesting one: its table is positional, so the
        // result depends on a topological order that a hash-seed change could perturb.
        argv: &[
            "cargo",
            "run",
            "--quiet",
            "--no-default-features",
            "--features",
            "cli",
            "--",
            "thread",
            "--derive",
            "narrative",
            "fixtures/corpus/F3-narrative.smy",
        ],
        setup: None,
    },
    Op {
        name: "text_add",
        // The reader, the normalisation, the part cut, and the four identities over it. The
        // locale permutation is the one that would betray a reader that case-folded or
        // collated anything: `ru_RU.UTF-8` is in the matrix for exactly this.
        //
        // What is compared is stdout, which carries the mid, every part's tid, the byte count
        // and the number of objects written. Into a directory made fresh for each capture that
        // last number is the same every time, which is what makes a *writing* operation
        // comparable at all.
        argv: &[
            "cargo",
            "run",
            "--quiet",
            "--no-default-features",
            "--features",
            "cli",
            "--",
            "text",
            "add",
            "fixtures/library/readers/gen1.usfm",
            "-s",
            SCRATCH,
            "--reader",
            "usfm/1",
            "--alias",
            "kjv/gen",
            "--licence",
            "public-domain",
            "--carry",
            "text",
            "--lang",
            "en",
        ],
        setup: None,
    },
    Op {
        name: "text_show",
        // Resolving a locator and printing the passage: the reading's segment table read back
        // as a tree, the range it names, and the bytes sliced out of the part.
        //
        // The setup is `text add` into the same fresh directory, because there is no honest way
        // to have a library without building one. Its output is discarded; its failure is not.
        argv: &[
            "cargo",
            "run",
            "--quiet",
            "--no-default-features",
            "--features",
            "cli",
            "--",
            "text",
            "show",
            "kjv/gen#Gen.1.3",
            "-s",
            SCRATCH,
            "--segments",
        ],
        setup: Some(&[
            "cargo",
            "run",
            "--quiet",
            "--no-default-features",
            "--features",
            "cli",
            "--",
            "text",
            "add",
            "fixtures/library/readers/gen1.usfm",
            "-s",
            SCRATCH,
            "--reader",
            "usfm/1",
            "--alias",
            "kjv/gen",
            "--licence",
            "public-domain",
            "--carry",
            "text",
            "--lang",
            "en",
        ]),
    },
    Op {
        name: "text_append",
        // A second version of an expression (TX-P2 step 3): the head manifest read back, the
        // reader and the policy taken from it, the new text cut, and the `supersedes` chain.
        //
        // What is compared is stdout, which carries the new mid, the mid it supersedes, every
        // part's tid and the number of objects written — so a reader or a part cut that was not
        // reproducible would move the chain, and a `supersedes` that came out differently would
        // show up as a different expression history rather than as a different byte.
        //
        // The fixture pair is one day of a chat export and two days of it. Under the default
        // part policy both are **one** part, so what this pins is the chain and the identities
        // rather than tid reuse; reuse needs a policy that cuts per day, and the growth test in
        // `tests/cmd_text_append.rs` is where that is measured.
        argv: &[
            "cargo",
            "run",
            "--quiet",
            "--no-default-features",
            "--features",
            "cli",
            "--",
            "text",
            "append",
            "fixtures/library/readers/telegram.json",
            "-s",
            SCRATCH,
            "--alias",
            "rg",
        ],
        setup: Some(&[
            "cargo",
            "run",
            "--quiet",
            "--no-default-features",
            "--features",
            "cli",
            "--",
            "text",
            "add",
            "fixtures/library/readers/telegram-first-day.json",
            "-s",
            SCRATCH,
            "--reader",
            "telegram/1",
            "--alias",
            "rg",
            "--licence",
            "CC0-1.0",
            "--carry",
            "text",
            "--lang",
            "mul",
        ]),
    },
];

/// Two-stage operations, run as a shell pipeline. `render` needs a thread, and the only
/// deterministic way to obtain one is to derive it - so the determinism claim being tested
/// is the composition, which is what a caller actually runs.
const PIPELINES: &[Op] = &[Op {
    name: "render",
    // Derivation and rendering back to back: role assignment, salience, ordering, repair,
    // IR construction, connective selection seeded by uid, and the markdown backend.
    argv: &[
        "sh",
        "-c",
        "cargo run --quiet --no-default-features --features cli -- \
             thread --derive --schema brief fixtures/corpus/F1-incident.smy \
         | cargo run --quiet --no-default-features --features cli -- \
             render --thread t/derived-brief --profile exec --target markdown -",
    ],
    setup: None,
}];

pub fn run(root: &Path) -> Result<(), String> {
    let envs = matrix();
    println!("  matrix: {} permutations", envs.len());

    if OPS.is_empty() {
        println!("  no operations registered yet");
        return Ok(());
    }
    // Rule D names five. `text add` and `text show` are TX-P1's and are not in §25's list
    // because they did not exist when it was written; they are here because they are pure
    // operations whose output must be a function of their input, which is all rule D says.
    let registered = OPS.len() + PIPELINES.len();
    println!(
        "  {registered} operations registered: rule D's five, and {} since",
        registered - 5
    );

    let mut failures = Vec::new();
    for op in OPS.iter().chain(PIPELINES) {
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
    // A scratch directory per capture, never reused, outside the repository.
    //
    // The counter is what makes it per *capture* rather than per operation: the whole point is
    // that each of the sixteen runs starts from nothing, and a directory shared between two of
    // them would make the second run's output a function of the first.
    let scratch = if op.needs_scratch() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "smysl-determinism-{}-{}-{}",
            op.name,
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        Some(dir)
    } else {
        None
    };
    let substitute = |argv: &[&str]| -> Vec<String> {
        argv.iter()
            .map(|a| match &scratch {
                Some(dir) => a.replace(SCRATCH, &dir.to_string_lossy()),
                None => (*a).to_string(),
            })
            .collect()
    };

    let result = (|| {
        if let Some(setup) = op.setup {
            let argv = substitute(setup);
            // A setup failure is an error, not a silent precondition. One that did nothing
            // would leave the real command reporting the same "no such alias" across all
            // sixteen runs — identical bytes, and a pass.
            run_argv(root, &argv, env).map_err(|e| format!("setup: {e}"))?;
        }
        let argv = substitute(op.argv);
        run_argv(root, &argv, env)
    })();

    if let Some(dir) = scratch {
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

fn run_argv(root: &Path, argv: &[String], env: &Env) -> Result<Vec<u8>, String> {
    let (prog, args) = argv.split_first().ok_or_else(|| "empty argv".to_string())?;
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
            setup: None,
        };
        let envs = matrix();
        let first = capture(root, &stable, &envs[0]).unwrap();
        for env in &envs {
            assert_eq!(capture(root, &stable, env).unwrap(), first);
        }

        let varying = Op {
            name: "printenv",
            argv: &["printenv", "TZ"],
            setup: None,
        };
        let a = capture(root, &varying, &envs[0]).unwrap();
        let differing = envs.iter().find(|e| e.tz != envs[0].tz).unwrap();
        assert_ne!(
            a,
            capture(root, &varying, differing).unwrap(),
            "the harness must be able to observe an environment difference"
        );
    }

    /// The scratch directory is fresh for each capture, which is the whole of what makes a
    /// writing operation comparable.
    ///
    /// Asserted by writing a file into it and counting: an operation that saw the previous
    /// capture's directory would count 2 on the second run. That is exactly `text add`'s
    /// `objects N written`, in a form that needs no library.
    #[test]
    fn each_capture_gets_a_directory_of_its_own() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let counting = Op {
            name: "counting",
            argv: &[
                "sh",
                "-c",
                "touch %SCRATCH%/$(ls %SCRATCH% | wc -l | tr -d ' '); ls %SCRATCH% | wc -l",
            ],
            setup: Some(&["sh", "-c", "mkdir -p %SCRATCH%"]),
        };
        assert!(counting.needs_scratch());
        let first = capture(root, &counting, &matrix()[0]).unwrap();
        let second = capture(root, &counting, &matrix()[0]).unwrap();
        assert_eq!(
            String::from_utf8_lossy(&first).trim(),
            "1",
            "the directory must start empty"
        );
        assert_eq!(first, second, "a reused directory would make this 2");
    }

    /// An operation with no token gets no directory, so nothing changes for rule D's five.
    #[test]
    fn an_operation_without_the_token_needs_no_scratch() {
        let plain = Op {
            name: "echo",
            argv: &["echo", "same"],
            setup: None,
        };
        assert!(!plain.needs_scratch());
    }

    /// A setup that fails is an error, not a silent precondition.
    ///
    /// The failure mode this guards: a setup that quietly did nothing would leave the operation
    /// under test printing the same refusal across all sixteen runs — identical bytes, and a
    /// pass that checked nothing.
    #[test]
    fn a_failing_setup_is_reported() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let op = Op {
            name: "bad-setup",
            argv: &["sh", "-c", "echo %SCRATCH% >/dev/null; echo fine"],
            setup: Some(&["sh", "-c", "exit 3"]),
        };
        let err = capture(root, &op, &matrix()[0]).expect_err("the setup exits 3");
        assert!(err.starts_with("setup: "), "{err}");
    }
}
