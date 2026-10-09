//! `<root>/secrets/pseudonym.key`: the one file in a library that is not a corpus.
//!
//! SMYSL-2.4 §3.2 puts the pseudonym key here, thirty-two random bytes, mode 0600, never
//! logged. Everything about this module follows from the key being a *secret* rather than from
//! it being data.
//!
//! # Why the randomness is an argument
//!
//! This crate is in the pure tier, which the purity gate states as a dependency fact: no
//! runtime, no socket, and — since TX-P2 step 1 — no randomness source in the default tree,
//! because `lingua` brought `getrandom` and that was the reason it had to be a feature. A
//! library that minted its own key would need one unconditionally, and the claim would have to
//! be withdrawn for a single call.
//!
//! So [`load_or_create`] takes the thirty-two bytes as a closure, called only when the file is
//! absent. The facade's `text` feature supplies `getrandom`, and the statement stays simple:
//! the library layer is a function of its arguments, and a secret is not something a function
//! of its arguments can invent.
//!
//! # Why a missing key is not an error
//!
//! [`load`] answers `Ok(None)` for a library that has none. A library that was never
//! pseudonymised has no key, and that is its normal state — `text add` without
//! `--pseudonymise` must not create one, because a key file is a thing an operator then has to
//! keep, back up and not lose. `SMY-E450` (TX-P2 step 3) is the *other* case: a key that
//! should exist and does not, which is only knowable from a manifest that says the expression
//! was pseudonymised.

use std::path::{Path, PathBuf};

use smysl_core::error::LibError;

use crate::speaker::Key;

/// The directory secrets live in, relative to the library root.
pub const DIR: &str = "secrets";

/// The pseudonym key's path, relative to the library root.
pub const PSEUDONYM: &str = "secrets/pseudonym.key";

fn io(e: &std::io::Error) -> LibError {
    LibError::Io {
        at: PSEUDONYM.to_string(),
        message: e.to_string(),
    }
}

/// Where the key is, under this root.
pub fn path(root: &Path) -> PathBuf {
    root.join(DIR).join("pseudonym.key")
}

/// The key, if this library has one.
///
/// A file of the wrong length is a refusal and not a truncation: thirty-one bytes read as a key
/// would produce pseudonyms that look exactly like the right ones and link to nothing.
pub fn load(root: &Path) -> Result<Option<Key>, LibError> {
    let file = path(root);
    let bytes = match std::fs::read(&file) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(&e)),
    };
    let bytes: [u8; Key::BYTES] = bytes.try_into().map_err(|b: Vec<u8>| LibError::Io {
        at: PSEUDONYM.to_string(),
        message: format!(
            "a pseudonym key is {} bytes; this file is {}",
            Key::BYTES,
            b.len()
        ),
    })?;
    Ok(Some(Key::from_bytes(bytes)))
}

/// The library's key, creating one from `fresh` if there is none.
///
/// `fresh` is called at most once, and only when the file is absent. A second process racing
/// here loses: the file is created with `create_new`, and the loser re-reads what the winner
/// wrote rather than overwriting it — two keys for one library would split every person's
/// messages in two.
pub fn load_or_create(
    root: &Path,
    fresh: impl FnOnce() -> [u8; Key::BYTES],
) -> Result<Key, LibError> {
    if let Some(key) = load(root)? {
        return Ok(key);
    }
    let dir = root.join(DIR);
    std::fs::create_dir_all(&dir).map_err(|e| LibError::Io {
        at: DIR.to_string(),
        message: e.to_string(),
    })?;
    let key = Key::from_bytes(fresh());
    match create_private(&path(root), key.bytes()) {
        Ok(()) => Ok(key),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Lost the race. The winner's key is the library's key.
            load(root)?.ok_or_else(|| io(&e))
        }
        Err(e) => Err(io(&e)),
    }
}

/// Create the file readable by its owner alone, or do not create it.
///
/// The mode is set **in the same call that creates the file**, not afterwards: a `set_permissions`
/// after the write leaves a window in which the key is world-readable, and a window is all a
/// secret needs to stop being one.
#[cfg(unix)]
fn create_private(file: &Path, bytes: &[u8]) -> Result<(), std::io::Error> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;

    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(file)?;
    f.write_all(bytes)?;
    f.sync_all()
}

/// Refused where the mode cannot be set, which is the honest answer rather than the convenient
/// one: the alternative is a key file with whatever permissions the platform's default gives
/// it, and a secret stored under unknown permissions is not stored.
///
/// Not a permanent position — it is one function, and a platform's own access-control call
/// belongs in it. It is a refusal rather than a silence until somebody writes that call.
#[cfg(not(unix))]
fn create_private(_file: &Path, _bytes: &[u8]) -> Result<(), std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "a pseudonym key is stored mode 0600, and this platform's equivalent is not implemented",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "smysl-secrets-{}-{name}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).expect("a temp dir");
        dir
    }

    /// A library with no key has none, and asking does not make one.
    #[test]
    fn a_library_without_a_key_is_not_given_one_by_being_asked() {
        let root = tmp("absent");
        assert!(load(&root).expect("read").is_none());
        assert!(!path(&root).exists(), "asking created a key file");
        std::fs::remove_dir_all(&root).ok();
    }

    /// Created once, then read back: the second call returns the first key, not a new one.
    #[test]
    fn a_key_is_created_once_and_read_back_afterwards() {
        let root = tmp("create");
        let first = load_or_create(&root, || [5u8; 32]).expect("created");
        let second = load_or_create(&root, || [9u8; 32]).expect("loaded");
        assert_eq!(first, second, "the second call minted a new key");
        assert_eq!(
            first.pseudonym("alice"),
            second.pseudonym("alice"),
            "two keys would split one person in two"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// Owner-only, checked rather than asserted in a comment.
    #[cfg(unix)]
    #[test]
    fn the_key_file_is_readable_by_its_owner_alone() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = tmp("mode");
        load_or_create(&root, || [1u8; 32]).expect("created");
        let mode = std::fs::metadata(path(&root))
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "mode {:o}", mode & 0o777);
        std::fs::remove_dir_all(&root).ok();
    }

    /// A file of the wrong length is refused, not padded or truncated.
    #[test]
    fn a_key_file_of_the_wrong_length_is_refused() {
        let root = tmp("short");
        std::fs::create_dir_all(root.join(DIR)).expect("dir");
        std::fs::write(path(&root), [1u8; 31]).expect("write");
        let err = load(&root).expect_err("refused");
        match err {
            LibError::Io { at, message } => {
                assert_eq!(at, PSEUDONYM);
                assert!(message.contains("31"), "{message}");
            }
            other => panic!("{other:?}"),
        }
        std::fs::remove_dir_all(&root).ok();
    }

    /// The path is named relative to the root, so a diagnostic holds no local path.
    #[test]
    fn the_name_in_a_refusal_is_relative_to_the_library() {
        let root = tmp("relative");
        std::fs::create_dir_all(root.join(DIR)).expect("dir");
        std::fs::write(path(&root), b"nope").expect("write");
        let err = load(&root).expect_err("refused");
        let shown = err.to_string();
        assert!(shown.contains(PSEUDONYM), "{shown}");
        assert!(
            !shown.contains(root.to_str().expect("utf-8")),
            "a refusal printed the library's own path: {shown}"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
