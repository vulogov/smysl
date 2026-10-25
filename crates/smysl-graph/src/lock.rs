//! One writer per shard, and a lock that says who (SMYSL-2.4 §3.9.3, SMYSL-2.8 §3.8).
//!
//! `log/<shard>.lock` is created with `create_new(true)` and holds the pid, the host and the
//! command. A second writer fails with `SMY-E445` naming the holder. Nothing removes a lock by
//! timeout; `--break-lock` is how an operator says they have read it.
//!
//! Here rather than in `smysl-text`, whose library locks are where it began: the unit store's
//! writer (`redb::RedbShard`) takes the same kind of lock for the same reason, and `smysl-graph`
//! sits below `smysl-text`. One meaning, one code — the library's locks are these locks, taken
//! at the paths its layout names.
//!
//! # Why not an advisory OS lock
//!
//! OQ-36, answered in 1.10.0, and the cost is stated rather than hidden. `std::fs::File::lock`
//! is unstable at this crate's MSRV, so it would raise the floor of a pure-path crate above
//! anything its dependencies need, and `fs4` buys the same thing as a dependency. But two
//! properties decide it without reference to either cost:
//!
//! - **An advisory lock cannot name its holder**, and `SMY-E445` is specified to.
//! - **A lock the kernel releases on process death destroys the evidence** that a writer died
//!   mid-append. The stale lock *is* the crash notice; the truncated tail it warns about is
//!   already tolerated by `from_cbor_seq`. A lock that cleaned itself up would make the common
//!   case quieter and the interesting case invisible.
//!
//! [`Lock`] releases on drop, which is not the same thing: a normal return is not a crash. A
//! process killed between appends leaves its lock behind, by design.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use smysl_core::error::LibError;

/// Who holds a lock.
///
/// Read back from the file rather than remembered, because the reader of a lock is by
/// definition a different process from the writer of it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Holder {
    pub pid: u32,
    pub host: String,
    pub command: String,
}

impl std::fmt::Display for Holder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "pid {} on {} running `{}`",
            self.pid, self.host, self.command
        )
    }
}

impl Holder {
    fn here(command: &str) -> Holder {
        Holder {
            pid: std::process::id(),
            host: hostname(),
            command: command.to_string(),
        }
    }

    /// `key=value` lines, one per field.
    ///
    /// Not HJSON or CBOR: a lock file is the one file in a library that a person reads with
    /// `cat` while something is wrong, and whose parser must not be able to fail in a way that
    /// matters. An unparseable lock is still a lock (`held_by` is empty and the refusal
    /// stands), which is the property the format is chosen for.
    fn render(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(s, "pid={}", self.pid);
        let _ = writeln!(s, "host={}", self.host);
        let _ = writeln!(s, "command={}", self.command);
        s
    }

    fn parse(text: &str) -> Holder {
        let mut h = Holder::default();
        for line in text.lines() {
            match line.split_once('=') {
                Some(("pid", v)) => h.pid = v.trim().parse().unwrap_or(0),
                Some(("host", v)) => h.host = v.trim().to_string(),
                Some(("command", v)) => h.command = v.trim().to_string(),
                _ => {}
            }
        }
        h
    }
}

/// The host, as well as this process can tell.
///
/// From the environment, because asking the operating system means either a C call or a
/// dependency, and a lock file's host field is for a human deciding whether the holder is
/// their own shell or a job on another machine. `unknown` is an honest answer when nothing
/// says otherwise, and it is never the reason a refusal does or does not happen.
fn hostname() -> String {
    for key in ["HOSTNAME", "HOST"] {
        if let Ok(v) = std::env::var(key) {
            if !v.trim().is_empty() {
                return v.trim().to_string();
            }
        }
    }
    "unknown".to_string()
}

/// Where a lock is written before it is linked into place: beside it, named for this process
/// and a counter, so two writers in one process never stage over each other.
fn staged_path(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "lock".into());
    path.with_file_name(format!(
        ".{name}.{}.{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ))
}

/// A held lock. Releases on drop.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    name: String,
    released: bool,
}

impl Lock {
    /// Take the lock named `name` at `path`, or report who holds it.
    ///
    /// `name` is what `SMY-E445` prints — `library`, or a shard's name. Never a path: the
    /// message is about the library, and a path would put somebody's home directory in a
    /// diagnostic.
    pub fn acquire(path: &Path, name: &str, command: &str) -> Result<Lock, LibError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| LibError::Io {
                at: name.to_string(),
                message: e.to_string(),
            })?;
        }
        let holder = Holder::here(command);
        let io = |e: std::io::Error| LibError::Io {
            at: name.to_string(),
            message: e.to_string(),
        };
        // Written whole beside the lock, then linked into place, so the lock never exists
        // without its holder in it. Created and then written, as it first was, a second
        // writer arriving between the two read an empty file and was refused by "pid 0 on
        // running ``" — found by two processes racing for one shard (SMYSL-2.8 §5.3).
        // A hard link fails if the lock exists, which is the `create_new` this replaces.
        let staged = staged_path(path);
        std::fs::write(&staged, holder.render()).map_err(io)?;
        let linked = std::fs::hard_link(&staged, path);
        let _ = std::fs::remove_file(&staged);
        match linked {
            Ok(()) => Ok(Lock {
                path: path.to_path_buf(),
                name: name.to_string(),
                released: false,
            }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(LibError::Locked {
                what: name.to_string(),
                held_by: match std::fs::read_to_string(path) {
                    // An empty or unparseable holder is still a lock. The empty string is
                    // what `LibError::Locked`'s Display turns into "by an unreadable holder".
                    Ok(text) => match Holder::parse(&text) {
                        h if h == Holder::default() => String::new(),
                        h => h.to_string(),
                    },
                    Err(_) => String::new(),
                },
            }),
            Err(e) => Err(io(e)),
        }
    }

    /// Who holds a lock, without taking it. `None` when the lock is free.
    pub fn holder(path: &Path) -> Option<Holder> {
        std::fs::read_to_string(path)
            .ok()
            .map(|t| Holder::parse(&t))
    }

    /// Remove a lock somebody else holds: `--break-lock`.
    ///
    /// Returns the holder that was broken, so the command can print whose lock it removed.
    /// Nothing in this crate calls it — breaking a lock is an operator's decision, and a
    /// library that broke locks on its own behalf would have no reason to keep them.
    pub fn break_lock(path: &Path) -> Result<Option<Holder>, LibError> {
        let holder = Lock::holder(path);
        match std::fs::remove_file(path) {
            Ok(()) => Ok(holder),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(LibError::Io {
                at: path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "lock".to_string()),
                message: e.to_string(),
            }),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Release now rather than at the end of the scope.
    pub fn release(mut self) {
        self.remove();
    }

    fn remove(&mut self) {
        if !self.released {
            let _ = std::fs::remove_file(&self.path);
            self.released = true;
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        self.remove();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a library lock is called; the name is only ever printed.
    const LIBRARY: &str = "library";

    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> TmpDir {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!(
                "smysl-lock-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            TmpDir(p)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TmpDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_second_writer_is_e445_and_is_told_who_holds_it() {
        let d = TmpDir::new("second");
        let path = d.path().join("lock");
        let _first = Lock::acquire(&path, LIBRARY, "text add").unwrap();

        let e = Lock::acquire(&path, LIBRARY, "text append").unwrap_err();
        assert_eq!(e.code(), Some(smysl_core::Code::E445));
        let text = e.to_string();
        assert!(text.contains("library"), "{text}");
        assert!(text.contains(&std::process::id().to_string()), "{text}");
        assert!(
            text.contains("text add"),
            "names the holder's command: {text}"
        );
    }

    #[test]
    fn a_lock_is_released_on_drop_and_the_next_writer_gets_it() {
        let d = TmpDir::new("drop");
        let path = d.path().join("lock");
        {
            let _l = Lock::acquire(&path, LIBRARY, "one").unwrap();
            assert!(path.is_file());
        }
        assert!(!path.exists(), "a normal return is not a crash");
        let _second = Lock::acquire(&path, LIBRARY, "two").unwrap();
    }

    #[test]
    fn release_is_the_same_as_dropping_early() {
        let d = TmpDir::new("release");
        let path = d.path().join("lock");
        let l = Lock::acquire(&path, LIBRARY, "one").unwrap();
        l.release();
        assert!(!path.exists());
        Lock::acquire(&path, LIBRARY, "two").unwrap().release();
    }

    /// The property an advisory lock cannot have: a lock left behind by a process that died
    /// is still a lock, and it still names what died.
    #[test]
    fn a_lock_left_behind_by_a_dead_writer_still_refuses_and_still_names_it() {
        let d = TmpDir::new("stale");
        let path = d.path().join("lock");
        std::fs::write(
            &path,
            "pid=424242\nhost=some-build-box\ncommand=text add bible\n",
        )
        .unwrap();

        let e = Lock::acquire(&path, LIBRARY, "text ls").unwrap_err();
        assert!(e.to_string().contains("424242"), "{e}");
        assert!(e.to_string().contains("some-build-box"), "{e}");

        let broken = Lock::break_lock(&path).unwrap().unwrap();
        assert_eq!(broken.pid, 424242);
        assert_eq!(broken.command, "text add bible");
        assert!(!path.exists());
        assert!(Lock::break_lock(&path).unwrap().is_none(), "idempotent");
    }

    #[test]
    fn an_unreadable_lock_is_still_a_lock() {
        let d = TmpDir::new("unreadable");
        // A directory where a lock file belongs: `create_new` fails with AlreadyExists and
        // `read_to_string` fails too, which is the case the empty holder is for.
        let path = d.path().join("lock");
        std::fs::create_dir(&path).unwrap();
        let e = Lock::acquire(&path, LIBRARY, "text add").unwrap_err();
        assert_eq!(e.code(), Some(smysl_core::Code::E445));
        assert!(e.to_string().contains("unreadable holder"), "{e}");
    }

    #[test]
    fn the_file_is_readable_by_a_person() {
        let d = TmpDir::new("readable");
        let path = d.path().join("lock");
        let _l = Lock::acquire(&path, LIBRARY, "text add bible").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("pid="), "{text}");
        assert!(text.contains("command=text add bible"), "{text}");
        assert_eq!(Holder::parse(&text).command, "text add bible");
    }

    /// Eight writers at once, fifty times: one gets the lock, and every other is refused by a
    /// holder it can name — never by a lock caught between being created and being written.
    #[test]
    fn a_racing_writer_is_refused_by_a_named_holder_never_an_empty_one() {
        use std::sync::{Arc, Barrier};
        for round in 0..50 {
            let d = TmpDir::new("race");
            let path = Arc::new(d.path().join("lock"));
            let start = Arc::new(Barrier::new(8));
            let tried = Arc::new(Barrier::new(8));
            let threads: Vec<_> = (0..8)
                .map(|t| {
                    let (path, start, tried) = (path.clone(), start.clone(), tried.clone());
                    std::thread::spawn(move || {
                        start.wait();
                        let got = Lock::acquire(&path, LIBRARY, &format!("writer {t}"));
                        tried.wait();
                        got.map(|_| ()).map_err(|e| e.to_string())
                    })
                })
                .collect();
            let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
            assert_eq!(
                results.iter().filter(|r| r.is_ok()).count(),
                1,
                "round {round}"
            );
            for e in results.iter().filter_map(|r| r.as_ref().err()) {
                assert!(e.contains("running `writer "), "round {round}: {e}");
            }
            let left: Vec<_> = std::fs::read_dir(d.path()).unwrap().collect();
            assert!(
                left.is_empty(),
                "round {round}: nothing staged is left behind"
            );
        }
    }
}
