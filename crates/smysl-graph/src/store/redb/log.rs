//! Reading the CBOR log without holding it (SMYSL-2.8 §3.5.5).
//!
//! In index mode the log is the authority and the unit store says where each core sits in it,
//! so the store reads two ways: one record at an offset (`get`), and every record in order
//! (`records`, a rebuild, the replay of a tail). Neither reads the whole file into memory,
//! which `Store::open` does and which is the point of not being `Store`.

use std::fs::File;
use std::io;

use smysl_core::{from_cbor, CodecError, Error, Record, Rolling};

/// How much of the log a sequential read asks the filesystem for at a time.
const CHUNK: usize = 1 << 20;

/// `len` bytes of `file` from `offset`, without moving a shared cursor: several readers of one
/// store may be reading at once.
pub(super) fn read_at(file: &File, offset: u64, len: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(&mut buf, offset)?;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < len {
            let n = file.seek_read(&mut buf[done..], offset + done as u64)?;
            if n == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            done += n;
        }
    }
    Ok(buf)
}

/// The complete records of a log from one offset, in order, with where each one sits.
pub(super) struct LogReader<'f> {
    /// `None` for a log that does not exist yet, which reads as empty, as `Store::open` reads it.
    file: Option<Src<'f>>,
    /// The file offset of `buf[0]`.
    base: u64,
    /// Where the next read from the file starts.
    pos: u64,
    /// Where reading stops: the file's length when the reader was made, or a length the store
    /// already describes.
    end: u64,
    buf: Vec<u8>,
    /// How much of `buf` has been decoded.
    start: usize,
    /// The log's hash so far, over every byte of every record handed out. Boxed: it is two
    /// kilobytes of BLAKE3 state, and a reader is moved into every `records()` iterator.
    hasher: Box<Rolling>,
    /// The raw bytes of the record [`LogReader::next`] last returned.
    last: std::ops::Range<usize>,
}

/// A log a reader borrows, or one it owns: a record walk over several logs opens each in turn.
/// Or bytes that would be a log's from `base` on: what a primary log's append folds, which no
/// file holds (TX-P13d).
enum Src<'f> {
    Borrowed(&'f File),
    Owned(File),
    Bytes { base: u64, bytes: Vec<u8> },
}

impl Src<'_> {
    fn read(&self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        match self {
            Src::Borrowed(f) => read_at(f, offset, len),
            Src::Owned(f) => read_at(f, offset, len),
            Src::Bytes { base, bytes } => {
                let at = offset
                    .checked_sub(*base)
                    .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?
                    as usize;
                bytes
                    .get(at..at + len)
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| io::ErrorKind::UnexpectedEof.into())
            }
        }
    }
}

impl LogReader<'static> {
    /// [`LogReader::new`] over a file it keeps.
    pub(super) fn owning(file: File, from: u64, end: u64, hasher: Rolling) -> LogReader<'static> {
        let mut r = LogReader::new(None, from, end, hasher);
        r.file = Some(Src::Owned(file));
        r
    }

    /// The records in `bytes`, read as though they were a log's from offset `base`.
    pub(super) fn over_bytes(base: u64, bytes: Vec<u8>) -> LogReader<'static> {
        let end = base + bytes.len() as u64;
        let mut r = LogReader::new(None, base, end, Rolling::new());
        r.file = Some(Src::Bytes { base, bytes });
        r
    }
}

impl<'f> LogReader<'f> {
    /// Records from `from` (a record boundary) up to `end`, with `hasher` holding the hash of
    /// everything before `from`.
    pub(super) fn new(
        file: Option<&'f File>,
        from: u64,
        end: u64,
        hasher: Rolling,
    ) -> LogReader<'f> {
        LogReader {
            file: file.map(Src::Borrowed),
            base: from,
            pos: from,
            end,
            buf: Vec::new(),
            start: 0,
            hasher: Box::new(hasher),
            last: 0..0,
        }
    }

    /// The next complete record: its offset, its length and the record. `None` at the end, or
    /// at a tail too short to be a record — what a writer mid-append leaves (`SMY-W110`).
    pub(super) fn next(&mut self) -> Result<Option<(u64, u32, Record)>, Error> {
        loop {
            if self.start < self.buf.len() {
                match from_cbor(&self.buf[self.start..]) {
                    Ok((r, n)) => {
                        let at = self.base + self.start as u64;
                        self.last = self.start..self.start + n;
                        self.hasher.update(&self.buf[self.last.clone()]);
                        self.start += n;
                        return Ok(Some((at, n as u32, r)));
                    }
                    Err(CodecError::Truncated { .. }) if self.pos < self.end => {}
                    Err(CodecError::Truncated { .. }) => return Ok(None),
                    Err(e) => return Err(e.into()),
                }
            } else if self.pos >= self.end {
                return Ok(None);
            }
            self.refill()?;
        }
    }

    /// The bytes of the record `next` last returned, as the log holds them.
    pub(super) fn raw(&self) -> &[u8] {
        &self.buf[self.last.clone()]
    }

    /// The offset just past the last complete record handed out.
    pub(super) fn consumed(&self) -> u64 {
        self.base + self.start as u64
    }

    /// Bytes after the last complete record, up to `end`.
    pub(super) fn trailing(&self) -> u64 {
        self.end - self.consumed()
    }

    /// The hash of the log up to [`LogReader::consumed`].
    pub(super) fn hash(&self) -> [u8; 32] {
        self.hasher.finish()
    }

    /// The hash of everything read so far, still open, so a writer can go on from it.
    pub(super) fn into_hasher(self) -> Rolling {
        *self.hasher
    }

    fn refill(&mut self) -> Result<(), Error> {
        self.buf.drain(..self.start);
        self.base += self.start as u64;
        self.start = 0;
        self.last = 0..0;
        let want = CHUNK.min((self.end - self.pos) as usize);
        let file = self
            .file
            .as_ref()
            .ok_or_else(|| Error::Io(io::ErrorKind::NotFound.into()))?;
        let more = file.read(self.pos, want).map_err(Error::Io)?;
        self.pos += more.len() as u64;
        self.buf.extend_from_slice(&more);
        Ok(())
    }
}

/// The hash of `file`'s first `len` bytes, read a chunk at a time: what proves a unit store
/// still describes the start of its log. `None` if the file is shorter.
pub(super) fn hash_prefix(file: &File, len: u64) -> io::Result<Option<Rolling>> {
    if file.metadata()?.len() < len {
        return Ok(None);
    }
    let mut h = Rolling::new();
    let mut at = 0u64;
    while at < len {
        let n = CHUNK.min((len - at) as usize);
        h.update(&read_at(file, at, n)?);
        at += n as u64;
    }
    Ok(Some(h))
}
