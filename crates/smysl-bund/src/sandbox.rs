//! The operating system's sandbox (RFC SMYSL-4 §5.5).
//!
//! Under [`Trust::Sandboxed`] the process cannot open a network socket, read a file outside
//! the snapshot directory, the `bund2` binary and the system libraries, write outside the
//! snapshot directory, or start another program.
//!
//! - **macOS:** `/usr/bin/sandbox-exec` with a profile written for the call. It denies
//!   everything and allows back what `bund2` needs to start and to read its snapshot. A denied
//!   operation fails with `EPERM`, which Bund2 reports as `Operation not permitted`.
//! - **Linux:** `bwrap` (bubblewrap), with every namespace unshared. The process sees the system
//!   libraries, the binary and the snapshot directory and nothing else, so there is no shell to
//!   start and no network to reach. User namespaces must be permitted, and are probed once.
//! - **Elsewhere,** or without the tool, [`BundError::SandboxUnavailable`] before any process
//!   starts.
//!
//! The probe and the paths are read in this process, before the child's environment is cleared.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::BundError;
use crate::policy::Trust;

/// The command that runs `bin` on `file` in `dir`, contained as `trust` asks. The caller adds
/// the environment, the pipes and the process group.
pub(crate) fn command(
    bin: &Path,
    file: &Path,
    dir: &Path,
    trust: Trust,
) -> Result<Command, BundError> {
    const FLAGS: [&str; 3] = ["--report", "trace", "--nocolor"];
    let mut cmd = match trust {
        Trust::Trusted => Command::new(bin),
        Trust::Sandboxed => contained(bin, dir)?,
    };
    cmd.args(FLAGS).arg(file);
    Ok(cmd)
}

/// Whether `Trust::Sandboxed` can run here; the reason when it cannot.
pub fn available() -> Result<(), BundError> {
    tool().map(|_| ())
}

fn unavailable(why: &str) -> BundError {
    BundError::SandboxUnavailable {
        platform: format!("{} ({why})", std::env::consts::OS),
    }
}

#[cfg(target_os = "macos")]
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

#[cfg(target_os = "macos")]
fn tool() -> Result<PathBuf, BundError> {
    let p = PathBuf::from(SANDBOX_EXEC);
    if p.is_file() {
        Ok(p)
    } else {
        Err(unavailable("no /usr/bin/sandbox-exec"))
    }
}

#[cfg(target_os = "macos")]
fn contained(bin: &Path, dir: &Path) -> Result<Command, BundError> {
    let tool = tool()?;
    let bin = real(bin)?;
    let mut cmd = Command::new(tool);
    cmd.arg("-p").arg(profile(&bin, &real(dir)?)?).arg(bin);
    Ok(cmd)
}

/// The macOS profile. `bund2` reads `/` itself at start (it aborts without it), which shows
/// the names at the root and nothing under them.
#[cfg(target_os = "macos")]
pub(crate) fn profile(bin: &Path, dir: &Path) -> Result<String, BundError> {
    let bin = quoted(bin)?;
    let dir = quoted(dir)?;
    Ok(format!(
        "(version 1)\n\
         (deny default)\n\
         (allow process-exec (literal {bin}))\n\
         (allow file-read* (literal {bin}) (subpath {dir})\n\
         \x20   (subpath \"/usr/lib\") (subpath \"/System/Library\")\n\
         \x20   (literal \"/dev/null\") (literal \"/dev/urandom\"))\n\
         (allow file-read-data (literal \"/\"))\n\
         (allow file-read-metadata)\n\
         (allow file-write* (subpath {dir}))\n\
         (allow sysctl-read)\n"
    ))
}

#[cfg(target_os = "macos")]
fn quoted(p: &Path) -> Result<String, BundError> {
    match p.to_str() {
        Some(s) if !s.contains(['"', '\\', '\n']) => Ok(format!("\"{s}\"")),
        _ => Err(BundError::IoError {
            message: format!("{} cannot be written in a sandbox profile", p.display()),
        }),
    }
}

#[cfg(target_os = "linux")]
fn tool() -> Result<PathBuf, BundError> {
    use std::sync::OnceLock;
    static PROBED: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    PROBED
        .get_or_init(|| {
            let bwrap = crate::probe::on_path("bwrap").ok_or("no bwrap on PATH")?;
            // The flags a call uses, around a program that is always there.
            let ok = Command::new(&bwrap)
                .args(["--unshare-all", "--die-with-parent", "--ro-bind", "/", "/"])
                .args(["--", "/bin/true"])
                .env_clear()
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success());
            if ok {
                Ok(bwrap)
            } else {
                Err("bwrap cannot unshare its namespaces here")
            }
            .map_err(str::to_string)
        })
        .clone()
        .map_err(|why| unavailable(&why))
}

#[cfg(target_os = "linux")]
fn contained(bin: &Path, dir: &Path) -> Result<Command, BundError> {
    let tool = tool()?;
    let bin = real(bin)?;
    let dir = real(dir)?;
    let mut cmd = Command::new(tool);
    cmd.args(["--unshare-all", "--die-with-parent", "--new-session"]);
    // The loader and the libraries. `/lib` and `/lib64` are symlinks into `/usr` on most
    // systems; bound as given, they resolve inside the sandbox as outside.
    for lib in [
        "/usr/lib",
        "/usr/lib64",
        "/lib",
        "/lib64",
        "/etc/ld.so.cache",
    ] {
        cmd.args(["--ro-bind-try", lib, lib]);
    }
    cmd.args(["--dev", "/dev", "--proc", "/proc"]);
    cmd.arg("--ro-bind").arg(&bin).arg(&bin);
    cmd.arg("--bind").arg(&dir).arg(&dir);
    cmd.arg("--chdir").arg(&dir);
    cmd.arg("--").arg(&bin);
    Ok(cmd)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn tool() -> Result<PathBuf, BundError> {
    Err(unavailable("no sandbox for this platform"))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn contained(_bin: &Path, _dir: &Path) -> Result<Command, BundError> {
    Err(unavailable("no sandbox for this platform"))
}

/// The path as the kernel resolves it: a sandbox rule names the file, not a link to it.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn real(p: &Path) -> Result<PathBuf, BundError> {
    p.canonicalize().map_err(|e| BundError::IoError {
        message: format!("{}: {e}", p.display()),
    })
}

/// What a denial looks like in Bund2's error text, on this platform. A script that fails this
/// way under the sandbox was stopped by it (§8).
pub(crate) fn denied(message: &str) -> bool {
    const SIGNS: [&str; 3] = [
        "Operation not permitted",
        "Permission denied",
        "Network is unreachable",
    ];
    SIGNS.iter().any(|s| message.contains(s))
}
