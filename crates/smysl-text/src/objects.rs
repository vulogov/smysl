//! The object store: where part texts and readings come to rest.
//!
//! `objects/t3/ab/cd…` holds exactly `to_cbor(&Record::PartText(..))`, and `objects/r3/…` the
//! same for a reading. The file is the record envelope, not the bare text — so a bundle or an
//! export can emit it verbatim, and verification is decode-then-hash rather than a format
//! conversion that might not be the inverse of the one that wrote it.
//!
//! **A log never holds 15 or 18** (OQ-39). This is where they live instead, and the reason is
//! redaction: honouring one by rewriting an append-only log resets the same hash chain that
//! would have shown the rewrite, so afterwards the log cannot distinguish the redaction from
//! an edit. Here, a redaction is an unlink, and record 19 in the log is the evidence that it
//! happened.
//!
//! # Three properties, and why each is not a convenience
//!
//! **Write to `objects/tmp/` and rename.** A reader that opened a half-written object would
//! see a truncated CBOR record, which decodes as corruption rather than as absence —
//! indistinguishable from the one thing `SMY-E446` exists to report.
//!
//! **An existing target is success.** Objects are content-addressed, so the file already
//! there has the content being written. Treating it as a conflict would make `text append`
//! fail on every unchanged part, which is most of them.
//!
//! **Verify on read.** The name is a hash of the content, so the check is free in the only
//! sense that matters: it needs nothing but the bytes already in hand. Without it a corpus
//! can serve bytes that are not what anybody stored and report nothing.

use std::path::{Path, PathBuf};

use smysl_core::error::LibError;
use smysl_core::ids::{Rdid, Tid};
use smysl_core::types::library::{PartReading, PartResolver, PartText, Resolved};
use smysl_core::types::Record;

/// The two kinds of object, and the directory each lives in.
///
/// The directory is the identity's text prefix without the colon, so the layout says what it
/// holds: `objects/t3/` is tids, `objects/r3/` is rdids. A reader who has never seen this
/// code can tell which is which from `ls`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ObjectKind {
    PartText,
    Reading,
}

impl ObjectKind {
    pub const fn dir(self) -> &'static str {
        match self {
            ObjectKind::PartText => "t3",
            ObjectKind::Reading => "r3",
        }
    }
}

/// A library's object store, rooted at `<library>/objects`.
#[derive(Debug, Clone)]
pub struct ObjectStore {
    root: PathBuf,
}

/// Name an I/O failure without naming a path outside the library.
///
/// Diagnostics get pasted into bug reports, and an absolute path is somebody's home
/// directory. The `at` here is relative to the library root — which is also the only part of
/// the path that is about the library rather than about the machine.
fn io(at: impl AsRef<str>, e: &std::io::Error) -> LibError {
    LibError::Io {
        at: at.as_ref().to_string(),
        message: e.to_string(),
    }
}

impl ObjectStore {
    /// Open a store, creating the directories it needs.
    pub fn open(root: &Path) -> Result<ObjectStore, LibError> {
        let store = ObjectStore {
            root: root.to_path_buf(),
        };
        for d in [
            store.root.join("tmp"),
            store.root.join(ObjectKind::PartText.dir()),
            store.root.join(ObjectKind::Reading.dir()),
        ] {
            std::fs::create_dir_all(&d).map_err(|e| io(rel(&store.root, &d), &e))?;
        }
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `objects/<kind>/ab/cd/<the rest>`.
    ///
    /// Two levels of two characters, which is git's layout for the same reason: a single
    /// directory with a million entries is slow on filesystems people actually use, and one
    /// level of 32 buckets is not enough fan-out for a corpus of chat messages. The name is
    /// the identity's base32 text **without** its prefix, because the prefix is already the
    /// directory.
    pub fn path_of(&self, kind: ObjectKind, canonical: &str) -> PathBuf {
        let body = canonical.split_once(':').map_or(canonical, |(_, b)| b);
        let (a, rest) = body.split_at(2.min(body.len()));
        let (b, tail) = rest.split_at(2.min(rest.len()));
        self.root.join(kind.dir()).join(a).join(b).join(tail)
    }

    pub fn part_path(&self, tid: &Tid) -> PathBuf {
        self.path_of(ObjectKind::PartText, &tid.canonical())
    }

    pub fn reading_path(&self, rdid: &Rdid) -> PathBuf {
        self.path_of(ObjectKind::Reading, &rdid.canonical())
    }

    pub fn has_part(&self, tid: &Tid) -> bool {
        self.part_path(tid).is_file()
    }

    pub fn has_reading(&self, rdid: &Rdid) -> bool {
        self.reading_path(rdid).is_file()
    }

    /// Store a part text. `Ok(true)` when it was written, `Ok(false)` when it was there
    /// already — which is the common case for `text append` and is not a conflict.
    ///
    /// The text is verified against its own tid first. A part text whose bytes do not hash to
    /// its tid is `SMY-E446` on the way *in*, because the alternative is a store that
    /// faithfully keeps a lie and reports it on every read for the rest of the corpus's life.
    pub fn put_part(&self, part: &PartText) -> Result<bool, LibError> {
        if !part.verify() {
            return Err(LibError::ObjectCorrupt {
                id: part.tid.canonical(),
            });
        }
        let path = self.part_path(&part.tid);
        self.write(&path, &smysl_core::to_cbor(&Record::PartText(part.clone())))
    }

    /// Store a reading, under the rdid it hashes to.
    ///
    /// The rdid is recomputed rather than taken from a caller: an rdid is a function of the
    /// record, so a caller with a stale one would store the record under a name that does not
    /// name it, and every later read would be `SMY-E446`.
    pub fn put_reading(&self, reading: &PartReading) -> Result<(Rdid, bool), LibError> {
        let rdid = reading.rdid();
        let path = self.reading_path(&rdid);
        let wrote = self.write(
            &path,
            &smysl_core::to_cbor(&Record::PartReading(reading.clone())),
        )?;
        Ok((rdid, wrote))
    }

    /// Read a part text back, verified (`SMY-E446`).
    pub fn get_part(&self, tid: &Tid) -> Result<PartText, LibError> {
        let path = self.part_path(tid);
        let bytes = std::fs::read(&path).map_err(|e| io(rel(&self.root, &path), &e))?;
        let corrupt = || LibError::ObjectCorrupt {
            id: tid.canonical(),
        };
        // The consumed length is checked too: an object file holds exactly one record and
        // nothing after it. Trailing bytes mean the file is not what was written, which is
        // the same statement as a flipped byte however innocent the extra bytes look.
        match smysl_core::from_cbor(&bytes) {
            Ok((Record::PartText(p), n)) if n == bytes.len() && p.tid == *tid && p.verify() => {
                Ok(p)
            }
            _ => Err(corrupt()),
        }
    }

    /// Read a reading back, verified against the rdid it is stored under.
    pub fn get_reading(&self, rdid: &Rdid) -> Result<PartReading, LibError> {
        let path = self.reading_path(rdid);
        let bytes = std::fs::read(&path).map_err(|e| io(rel(&self.root, &path), &e))?;
        let corrupt = || LibError::ObjectCorrupt {
            id: rdid.canonical(),
        };
        match smysl_core::from_cbor(&bytes) {
            Ok((Record::PartReading(r), n)) if n == bytes.len() && r.rdid() == *rdid => Ok(r),
            _ => Err(corrupt()),
        }
    }
}

/// The store as `check`'s `Library` pass sees it: what is there, unverified.
///
/// `get_part` verifies and is right to — a caller asking for text must not be handed bytes
/// that are not what was stored. That makes it useless to a checker, which needs the bad
/// object in order to report it, so this reads the same file and stops before the hash. The
/// two are deliberately not one method with a flag: the flag would be a way to ask for
/// unverified text by accident, and there is exactly one caller that wants it.
impl PartResolver for ObjectStore {
    fn part(&self, tid: &Tid) -> Resolved {
        let path = self.part_path(tid);
        let Ok(bytes) = std::fs::read(&path) else {
            // Any read failure is absence. A file that cannot be opened is not a part this
            // store can be said to hold, and reporting `SMY-E446` for a permission error
            // would name the wrong defect.
            return Resolved::Absent;
        };
        match smysl_core::from_cbor(&bytes) {
            // The consumed length is part of being readable, as it is in `get_part`: an
            // object file holds one record and nothing after it.
            Ok((Record::PartText(p), n)) if n == bytes.len() => Resolved::Part(p),
            _ => Resolved::Unreadable,
        }
    }
}

impl ObjectStore {
    /// Delete a part's bytes: rule Z, after record 19 is in the log.
    ///
    /// `Ok(false)` when there was nothing there, so a repair pass over a crash between the
    /// record and the unlink is idempotent rather than an error.
    pub fn remove_part(&self, tid: &Tid) -> Result<bool, LibError> {
        let path = self.part_path(tid);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io(rel(&self.root, &path), &e)),
        }
    }

    /// Delete a reading's bytes: rule Z, with the part's.
    ///
    /// A reading goes with its part and not after it. Record 18 holds no text, but it holds the
    /// offsets, the speakers and the ids the text was read into — and the spec's rule is about
    /// 15 **and** 18 for that reason: a reading whose part is gone is a map of something nobody
    /// may hold, and it is the half that names people.
    pub fn remove_reading(&self, rdid: &Rdid) -> Result<bool, LibError> {
        let path = self.reading_path(rdid);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(io(rel(&self.root, &path), &e)),
        }
    }

    /// Write bytes to `path` through `objects/tmp/`, atomically.
    fn write(&self, path: &Path, bytes: &[u8]) -> Result<bool, LibError> {
        if path.is_file() {
            return Ok(false);
        }
        let parent = path.parent().unwrap_or(&self.root);
        std::fs::create_dir_all(parent).map_err(|e| io(rel(&self.root, parent), &e))?;
        // The temporary name holds the process id so two writers staging the same object do
        // not stage it into the same file. They may still both rename, and that is fine: the
        // content is the same, and rename is atomic.
        let tmp = self.root.join("tmp").join(format!(
            "{}.{}.tmp",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("object"),
            std::process::id()
        ));
        std::fs::write(&tmp, bytes).map_err(|e| io(rel(&self.root, &tmp), &e))?;
        match std::fs::rename(&tmp, path) {
            Ok(()) => Ok(true),
            Err(e) => {
                // Leave nothing behind on a failure: a stale temporary file is a file whose
                // name says it is temporary and whose age says nothing.
                let _ = std::fs::remove_file(&tmp);
                Err(io(rel(&self.root, path), &e))
            }
        }
    }
}

/// A path relative to the library root, for a message.
fn rel(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::norm::Normalised;
    use crate::reading::{Reading, Segment};

    /// A directory under the system temporary directory, removed on drop.
    ///
    /// Written here rather than taken as a dependency: `tempfile` would be a new dependency
    /// in the pure core for eight lines, and the name is derived from the process id and a
    /// counter so two tests in the same binary cannot collide.
    struct TmpDir(PathBuf);

    impl TmpDir {
        fn new(tag: &str) -> TmpDir {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let p = std::env::temp_dir().join(format!(
                "smysl-text-{tag}-{}-{}",
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

    fn a_part() -> PartText {
        PartText::new(
            Normalised::of("In the beginning God created the heaven and the earth.\n")
                .as_bytes()
                .to_vec(),
        )
    }

    fn a_reading(tid: smysl_core::Tid) -> PartReading {
        let rows = vec![Segment::new(
            0,
            55,
            crate::reading::Level::new("verse").unwrap(),
            crate::locator::parse("Gen.1.1").unwrap(),
        )];
        Reading::new(tid, "osis/1", rows).to_record()
    }

    #[test]
    fn a_part_round_trips_through_the_store() {
        let d = TmpDir::new("roundtrip");
        let s = ObjectStore::open(d.path()).unwrap();
        let p = a_part();
        assert!(s.put_part(&p).unwrap(), "written");
        assert!(s.has_part(&p.tid));
        assert_eq!(s.get_part(&p.tid).unwrap(), p);
    }

    #[test]
    fn a_reading_round_trips_under_the_rdid_it_hashes_to() {
        let d = TmpDir::new("reading");
        let s = ObjectStore::open(d.path()).unwrap();
        let p = a_part();
        let r = a_reading(p.tid);
        let (rdid, wrote) = s.put_reading(&r).unwrap();
        assert!(wrote);
        assert_eq!(rdid, r.rdid());
        assert_eq!(s.get_reading(&rdid).unwrap(), r);
    }

    #[test]
    fn the_file_holds_the_record_envelope_and_nothing_else() {
        let d = TmpDir::new("envelope");
        let s = ObjectStore::open(d.path()).unwrap();
        let p = a_part();
        s.put_part(&p).unwrap();
        let bytes = std::fs::read(s.part_path(&p.tid)).unwrap();
        assert_eq!(
            bytes,
            smysl_core::to_cbor(&Record::PartText(p.clone())),
            "verbatim, so a bundle can emit it without re-encoding"
        );
    }

    #[test]
    fn writing_an_object_that_is_already_there_is_success_and_not_a_conflict() {
        let d = TmpDir::new("again");
        let s = ObjectStore::open(d.path()).unwrap();
        let p = a_part();
        assert!(s.put_part(&p).unwrap());
        assert!(!s.put_part(&p).unwrap(), "second write is a no-op");
        assert_eq!(s.get_part(&p.tid).unwrap(), p);
    }

    #[test]
    fn nothing_is_left_in_tmp_after_a_write() {
        let d = TmpDir::new("tmp");
        let s = ObjectStore::open(d.path()).unwrap();
        s.put_part(&a_part()).unwrap();
        let left: Vec<_> = std::fs::read_dir(d.path().join("tmp"))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        assert!(left.is_empty(), "{} files left staged", left.len());
    }

    #[test]
    fn a_flipped_byte_is_e446_on_read() {
        let d = TmpDir::new("flip");
        let s = ObjectStore::open(d.path()).unwrap();
        let p = a_part();
        s.put_part(&p).unwrap();

        let path = s.part_path(&p.tid);
        let mut bytes = std::fs::read(&path).unwrap();
        // The last byte of the text, which is inside the hashed content and nowhere near the
        // envelope's head.
        let last = bytes.len() - 1;
        bytes[last] ^= 0x20;
        std::fs::write(&path, &bytes).unwrap();

        let e = s.get_part(&p.tid).unwrap_err();
        assert_eq!(e.code(), Some(smysl_core::Code::E446));
        assert!(e.to_string().contains(&p.tid.canonical()), "{e}");
    }

    #[test]
    fn a_truncated_object_is_e446_and_not_a_decode_panic() {
        let d = TmpDir::new("trunc");
        let s = ObjectStore::open(d.path()).unwrap();
        let p = a_part();
        s.put_part(&p).unwrap();
        let path = s.part_path(&p.tid);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert_eq!(
            s.get_part(&p.tid).unwrap_err().code(),
            Some(smysl_core::Code::E446)
        );
    }

    #[test]
    fn a_part_whose_bytes_do_not_hash_to_its_tid_is_refused_on_the_way_in() {
        let d = TmpDir::new("lie");
        let s = ObjectStore::open(d.path()).unwrap();
        let honest = a_part();
        let liar = PartText::with_claimed_tid(honest.tid, b"different bytes\n".to_vec());
        assert!(!liar.verify());
        assert_eq!(
            s.put_part(&liar).unwrap_err().code(),
            Some(smysl_core::Code::E446)
        );
        assert!(!s.has_part(&honest.tid), "and nothing was written");
    }

    #[test]
    fn an_absent_object_is_an_io_error_and_not_a_corruption() {
        let d = TmpDir::new("absent");
        let s = ObjectStore::open(d.path()).unwrap();
        let e = s.get_part(&a_part().tid).unwrap_err();
        assert_eq!(e.code(), None, "absence is not SMY-E446");
        assert!(matches!(e, LibError::Io { .. }), "{e}");
    }

    #[test]
    fn removing_a_part_twice_is_idempotent() {
        let d = TmpDir::new("remove");
        let s = ObjectStore::open(d.path()).unwrap();
        let p = a_part();
        s.put_part(&p).unwrap();
        assert!(s.remove_part(&p.tid).unwrap());
        assert!(
            !s.remove_part(&p.tid).unwrap(),
            "a repair pass may run twice"
        );
        assert!(!s.has_part(&p.tid));
    }

    #[test]
    fn the_layout_is_two_levels_of_two_characters_under_the_prefix_directory() {
        let d = TmpDir::new("layout");
        let s = ObjectStore::open(d.path()).unwrap();
        let p = a_part();
        let canonical = p.tid.canonical();
        let body = canonical.strip_prefix("t3:").unwrap();
        let path = s.part_path(&p.tid);
        let rel = path.strip_prefix(d.path()).unwrap();
        assert_eq!(
            rel,
            Path::new("t3")
                .join(&body[..2])
                .join(&body[2..4])
                .join(&body[4..])
        );
        // An error message must never carry a path from outside the library.
        let e = s.get_part(&p.tid).unwrap_err();
        assert!(!e.to_string().contains(&*d.path().to_string_lossy()), "{e}");
    }
}
