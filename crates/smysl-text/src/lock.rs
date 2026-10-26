//! The library's locks (SMYSL-2.4 §3.9.3): the library lock, then shard locks in name order.
//!
//! The lock itself — [`Lock`], [`Holder`], and why it is a file rather than an advisory OS
//! lock — is `smysl_graph::lock`, which the unit store's writer takes too (SMYSL-2.8 §3.8).
//! It moved down a crate in TX-P13b step 2 and is re-exported here unchanged.

use std::path::Path;

use smysl_core::error::LibError;

pub use smysl_graph::lock::{Holder, Lock};

/// The library lock's name, as `SMY-E445` prints it.
pub const LIBRARY: &str = "library";

/// Take the library lock, then the named shard locks in name order.
///
/// One order, always, so two library-wide operations cannot deadlock: both want the library
/// lock first, so one of them fails with `SMY-E445` before it holds anything. Within a run the
/// shards are sorted, so two operations that somehow got past the library lock still cannot
/// hold each other's next lock.
///
/// Returns the locks in the order they were taken; dropping the vector releases them in that
/// order, which does not matter for correctness and does make a strace readable.
pub fn acquire_all(
    library_root: &Path,
    shards: &[String],
    command: &str,
) -> Result<Vec<Lock>, LibError> {
    let mut held = vec![Lock::acquire(&library_root.join("lock"), LIBRARY, command)?];
    let mut names: Vec<&String> = shards.iter().collect();
    names.sort();
    for shard in names {
        // By file stem: an alias may hold `/`, which must not become a directory here.
        let path = library_root
            .join("log")
            .join(format!("{}.lock", crate::shards::stem(shard)));
        // On failure the locks already held are dropped by the `?`, which releases them in
        // reverse — so a refused acquisition leaves the library exactly as it found it.
        held.push(Lock::acquire(&path, shard, command)?);
    }
    Ok(held)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

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
    fn the_library_lock_comes_first_and_the_shards_in_name_order() {
        let d = TmpDir::new("all");
        let held = acquire_all(
            d.path(),
            &["zeta".to_string(), "alpha".to_string()],
            "text add",
        )
        .unwrap();
        let names: Vec<&str> = held.iter().map(|l| l.name()).collect();
        assert_eq!(names, [LIBRARY, "alpha", "zeta"]);
        assert!(d.path().join("lock").is_file());
        assert!(d.path().join("log").join("alpha.lock").is_file());
    }

    #[test]
    fn a_refused_acquisition_leaves_nothing_held() {
        let d = TmpDir::new("partial");
        // Somebody else already holds the second shard.
        std::fs::create_dir_all(d.path().join("log")).unwrap();
        std::fs::write(
            d.path().join("log").join("zeta.lock"),
            "pid=1\nhost=h\ncommand=other\n",
        )
        .unwrap();

        let e = acquire_all(
            d.path(),
            &["alpha".to_string(), "zeta".to_string()],
            "text add",
        )
        .unwrap_err();
        assert_eq!(e.code(), Some(smysl_core::Code::E445));
        assert!(
            !d.path().join("lock").exists(),
            "the library lock was released"
        );
        assert!(
            !d.path().join("log").join("alpha.lock").exists(),
            "and so was the shard lock taken before the refusal"
        );
    }
}
