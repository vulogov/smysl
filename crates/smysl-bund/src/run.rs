//! The wrapper and the process (RFC SMYSL-4 §5, §7).

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use smysl_core::ids::Uid;
use smysl_graph::Store;

use crate::decode;
use crate::error::BundError;
use crate::gate;
use crate::policy::{QueryPolicy, Trust};
use crate::sandbox;
use crate::snapshot::Snapshot;
use crate::value::QueryValue;

/// What a call answers.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct BundQueryResult {
    /// The value the script left on top of the stack.
    pub value: QueryValue,
    /// What `bund2` wrote on standard error, one line each. Not part of the answer.
    pub warnings: Vec<String>,
    /// Wall time of the process. Not part of the answer: two equal runs differ here.
    pub elapsed_ms: u64,
}

/// Evaluate `script` over a snapshot of `store`. The script reads the library as `:lib var?`.
pub fn eval_store(
    store: &Store,
    script: &str,
    policy: &QueryPolicy,
) -> Result<BundQueryResult, BundError> {
    let script = prepare(script, policy)?;
    let snap = Snapshot::of_store(store)?;
    execute(&snap, None, &script, store, policy)
}

/// Evaluate `script` over a snapshot of the library at `root`.
pub fn eval_library(
    root: &Path,
    script: &str,
    policy: &QueryPolicy,
) -> Result<BundQueryResult, BundError> {
    let script = prepare(script, policy)?;
    let (snap, store) = Snapshot::of_library(root)?;
    execute(&snap, None, &script, &store, policy)
}

/// Run `sq` over `store`, bind the uids it answers to `:seed` (a LIST of STRINGs, in the
/// answer's order), then evaluate `script` as [`eval_store`] does.
pub fn eval_over_sq(
    store: &Store,
    sq: &str,
    script: &str,
    policy: &QueryPolicy,
) -> Result<BundQueryResult, BundError> {
    let script = prepare(script, policy)?;
    let snap = Snapshot::of_store(store)?;
    let seed = run_sq(snap.root(), sq)?;
    execute(&snap, Some(&seed), &script, store, policy)
}

/// `sq` over the snapshot, read as a library, so that a query reads as it does in `smysl q` and
/// in Bund2's `smysl.query`: `from library` and not `from store:…`.
fn run_sq(root: &Path, sq: &str) -> Result<Vec<Uid>, BundError> {
    use smysl_query::{parse, plan, run, Env, PlanOptions};
    let joined = |ds: Vec<smysl_core::Diagnostic>| {
        let each: Vec<String> = ds
            .iter()
            .map(|d| format!("{} {}", d.code.as_str(), d.message))
            .collect();
        BundError::SqQueryRefusal {
            diagnostics: each.join("; "),
        }
    };
    let library = smysl_text::Library::open(root).map_err(|e| BundError::IoError {
        message: e.to_string(),
    })?;
    let store = library.store();
    let parsed = parse(sq).map_err(|d| joined(vec![d]))?;
    let scope = smysl_text::LibraryScope::new("library", &library);
    let env = Env::new(store, &scope)
        .with_times(&scope)
        .with_entities(&scope);
    let opts = PlanOptions::default();
    let planned = plan(&parsed.query, &env, &opts).map_err(joined)?;
    let answered = run(&planned, &env, &opts).map_err(joined)?;
    Ok(answered.answer.uids())
}

/// The gate, the inference cap, and the sandbox's presence: all before anything is written.
fn prepare(script: &str, policy: &QueryPolicy) -> Result<String, BundError> {
    let named = gate::check(script, policy)?;
    if policy.trust == Trust::Sandboxed {
        sandbox::available()?;
    }
    Ok(match policy.max_inferences {
        Some(cap) => gate::with_inference_cap(script, &named, cap),
        None => script.to_string(),
    })
}

/// The run's sentinel tag. It keeps the encoder's frame apart from what the script prints. It
/// is not a secret from a trusted script, which can read the file it is in.
fn tag() -> String {
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let n = COUNT.fetch_add(1, Ordering::Relaxed);
    let bytes = format!("{nanos}:{}:{n}", std::process::id());
    let hash = smysl_core::hash::hash_bytes(bytes.as_bytes());
    hash[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// §7: the preamble, the script, the encoder.
pub fn wrapper(
    root: &Path,
    seed: Option<&[Uid]>,
    script: &str,
    tag: &str,
) -> Result<String, BundError> {
    let root = root
        .to_str()
        .filter(|r| !r.contains(['"', '\\', '\n']))
        .ok_or_else(|| BundError::IoError {
            message: format!(
                "the snapshot path {} cannot be written in Bund",
                root.display()
            ),
        })?;
    let (begin, end) = decode::sentinels(tag);
    let mut out = format!(":lib \"{root}\" smysl.open var\n");
    match seed {
        None => {}
        Some([]) => out.push_str(":seed list var\n"),
        Some(uids) => {
            // The canonical form: `Display` writes the 26-character short form, which no
            // `Uid::parse` reads back.
            let items: Vec<String> = uids
                .iter()
                .map(|u| format!("\"{}\"", u.canonical()))
                .collect();
            out.push_str(&format!(":seed [ {} ] var\n", items.join(" ")));
        }
    }
    out.push_str(script);
    // A newline first: a `//` comment on the script's last line must not reach the encoder.
    out.push_str(&format!(
        "\n\"{begin}\" println\ndebug.display_stack\n\"{end}\" println\n"
    ));
    Ok(out)
}

fn execute(
    snap: &Snapshot,
    seed: Option<&[Uid]>,
    script: &str,
    source: &Store,
    policy: &QueryPolicy,
) -> Result<BundQueryResult, BundError> {
    let info = crate::probe::probe_cached(policy.bund_binary.as_deref())?;
    let tag = tag();
    let text = wrapper(snap.root(), seed, script, &tag)?;
    let file = snap.dir().join("query.bund");
    std::fs::write(&file, text).map_err(|e| BundError::IoError {
        message: format!("{}: {e}", file.display()),
    })?;

    let started = Instant::now();
    let out = spawn_bounded(&info.path, &file, snap.dir(), policy)?;
    let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);

    let stdout = String::from_utf8_lossy(&out.stdout);
    let value = decode::read_output(&stdout, &tag).map_err(|e| denial(e, policy.trust))?;
    if policy.verify_uids {
        if let Some(u) = value.uids().into_iter().find(|u| !source.contains_uid(u)) {
            return Err(BundError::NormalizationError {
                message: format!("{u} is not in the source store"),
            });
        }
    }
    let warnings = String::from_utf8_lossy(&out.stderr)
        .lines()
        .map(str::to_string)
        .collect();
    Ok(BundQueryResult {
        value,
        warnings,
        elapsed_ms,
    })
}

/// §8: under the sandbox, a failure whose text is the platform's denial was the sandbox's.
fn denial(e: BundError, trust: Trust) -> BundError {
    if trust != Trust::Sandboxed {
        return e;
    }
    let text = match &e {
        BundError::ScriptFailed { message }
        | BundError::SmyslRefusal { message, .. }
        | BundError::InvalidScriptOrArgs { message, .. } => message,
        BundError::ProcessFailed { stderr, .. } => stderr,
        _ => return e,
    };
    if sandbox::denied(text) {
        BundError::PolicyViolation {
            reason: format!("the sandbox denied it: {text}"),
            token: None,
        }
    } else {
        e
    }
}

struct Output {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// §5.1 to §5.5: the sandbox, flags before the path, the environment cleared, bounded readers, the wall
/// clock, and the process group killed with the process.
fn spawn_bounded(
    bin: &Path,
    file: &Path,
    dir: &Path,
    policy: &QueryPolicy,
) -> Result<Output, BundError> {
    let mut cmd = sandbox::command(bin, file, dir, policy.trust)?;
    cmd.env_clear()
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .env("COLUMNS", "80")
        .env("NO_COLOR", "1")
        .env("HOME", dir)
        .env("TMPDIR", dir)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|e| BundError::IoError {
        message: format!("{}: {e}", bin.display()),
    })?;

    let over = Arc::new(AtomicBool::new(false));
    let max = policy.max_output_bytes;
    let out = reader(child.stdout.take(), max, Arc::clone(&over));
    let err = reader(child.stderr.take(), max, Arc::clone(&over));

    let deadline = Instant::now() + Duration::from_millis(policy.max_wall_ms);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(e) => {
                kill(&mut child);
                return Err(BundError::IoError {
                    message: format!("waiting for bund2: {e}"),
                });
            }
        }
        if over.load(Ordering::Relaxed) || Instant::now() >= deadline {
            kill(&mut child);
            break None;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let stdout = out.join().unwrap_or_default();
    let stderr = err.join().unwrap_or_default();
    if over.load(Ordering::Relaxed) {
        return Err(BundError::OutputTooLarge {
            bytes: stdout.len().max(stderr.len()),
            max,
        });
    }
    if status.is_none() {
        return Err(BundError::TimeoutExceeded {
            max_wall_ms: policy.max_wall_ms,
        });
    }
    Ok(Output { stdout, stderr })
}

/// Read a pipe up to `max` bytes. Past it, say so and stop reading.
fn reader<R: Read + Send + 'static>(
    pipe: Option<R>,
    max: usize,
    over: Arc<AtomicBool>,
) -> JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let Some(mut pipe) = pipe else { return buf };
        let mut chunk = [0u8; 8192];
        loop {
            match pipe.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if buf.len() + n > max {
                        over.store(true, Ordering::Relaxed);
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
        }
        buf
    })
}

/// Kill the process and its group, so nothing it started outlives the call (§5.4).
fn kill(child: &mut Child) {
    #[cfg(unix)]
    {
        // The group is the child's own (`process_group(0)`), so its id is the child's.
        let _ = Command::new("/bin/kill")
            .args(["-KILL", "--", &format!("-{}", child.id())])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// The binary a call will run, for a caller that wants to say which before running it.
pub fn binary_of(policy: &QueryPolicy) -> Result<PathBuf, BundError> {
    crate::probe::probe_bund_binary(policy.bund_binary.as_deref()).map(|i| i.path)
}

/// The sandbox itself, with the gate passed by: each case runs the operation trusted and then
/// sandboxed, and asserts that the sandbox denied it, not merely that the script failed (§13).
/// Every case needs `BUND2_BIN` and passes trivially without it.
#[cfg(test)]
mod sandbox_tests {
    use super::*;

    fn bin() -> Option<PathBuf> {
        let b = std::env::var_os("BUND2_BIN").map(PathBuf::from);
        let required = std::env::var_os("SMYSL_BUND_REQUIRED").is_some();
        assert!(
            b.is_some() || !required,
            "SMYSL_BUND_REQUIRED and no BUND2_BIN"
        );
        assert!(
            sandbox::available().is_ok() || !required,
            "SMYSL_BUND_REQUIRED and no sandbox"
        );
        b
    }

    /// `script` as written, over an empty store, past the gate.
    fn raw(script: &str, trust: Trust) -> Result<BundQueryResult, BundError> {
        let policy = QueryPolicy::default()
            .with_binary(bin().unwrap())
            .with_trust(trust);
        let store = Store::from_records(Vec::new());
        let snap = Snapshot::of_store(&store)?;
        execute(&snap, None, script, &store, &policy)
    }

    fn outside() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[cfg(target_os = "macos")]
    fn denied(r: &Result<BundQueryResult, BundError>) -> bool {
        matches!(r, Err(BundError::PolicyViolation { token: None, reason }) if reason.starts_with("the sandbox denied it"))
    }

    #[test]
    fn a_shell_is_not_started() {
        if bin().is_none() || sandbox::available().is_err() {
            return;
        }
        let script = "\"echo started\" system.shell";
        let trusted = raw(script, Trust::Trusted).unwrap();
        assert_eq!(trusted.value, QueryValue::Text("started".into()));
        let r = raw(script, Trust::Sandboxed);
        assert!(r.is_err(), "{r:?}");
        #[cfg(target_os = "macos")]
        assert!(denied(&r), "{r:?}");
    }

    #[test]
    fn a_file_outside_is_not_read() {
        if bin().is_none() || sandbox::available().is_err() {
            return;
        }
        let dir = outside();
        let path = dir.path().canonicalize().unwrap().join("secret.txt");
        std::fs::write(&path, "secret").unwrap();
        let script = format!("\"{}\" file", path.display());
        let trusted = raw(&script, Trust::Trusted).unwrap();
        assert_eq!(trusted.value, QueryValue::Text("secret".into()));
        let r = raw(&script, Trust::Sandboxed);
        assert!(
            !matches!(&r, Ok(v) if v.value == QueryValue::Text("secret".into())),
            "{r:?}"
        );
    }

    #[test]
    fn a_file_outside_is_not_written() {
        if bin().is_none() || sandbox::available().is_err() {
            return;
        }
        let dir = outside();
        let path = dir.path().canonicalize().unwrap().join("written.txt");
        let script = format!("\"x\" \"{}\" file.write true", path.display());
        raw(&script, Trust::Trusted).unwrap();
        assert!(path.exists());
        std::fs::remove_file(&path).unwrap();
        let r = raw(&script, Trust::Sandboxed);
        assert!(!path.exists(), "{r:?}");
        // On Linux the path is not there to be denied: `bwrap` shows only the snapshot.
        #[cfg(target_os = "macos")]
        assert!(denied(&r), "{r:?}");
        assert!(r.is_err(), "{r:?}");
    }

    #[test]
    fn no_connection_is_made() {
        if bin().is_none() || sandbox::available().is_err() {
            return;
        }
        let reached = |trust: Trust| {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let port = listener.local_addr().unwrap().port();
            let _ = raw(&format!("\"http://127.0.0.1:{port}/\" url"), trust);
            listener.accept().is_ok()
        };
        assert!(reached(Trust::Trusted), "the case does not connect at all");
        assert!(!reached(Trust::Sandboxed));
    }

    #[test]
    fn a_query_runs_inside() {
        if bin().is_none() || sandbox::available().is_err() {
            return;
        }
        let r = raw(":lib var? smysl.status", Trust::Sandboxed).unwrap();
        assert!(matches!(r.value, QueryValue::Map(_)), "{:?}", r.value);
    }
}
