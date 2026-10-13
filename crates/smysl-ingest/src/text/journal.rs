//! The ingest journal: what has already been extracted, and what it cost (§4.3.4, §3.5).
//!
//! A Bible is thousands of windows and a chat export is tens of thousands. A run that is killed
//! half way through — a laptop closing, a rate limit, a `^C` — must not start again from the
//! beginning, and it must not skip the window it was in the middle of either. So each window's
//! work is recorded under a key that says exactly what was done:
//!
//! ```text
//! (alias, tid, node range, recipe)
//! ```
//!
//! The recipe is in the key because the same window extracted under a different model, prompt,
//! granularity or window level is *different work* with a different answer — and a journal that
//! left it out would skip it as done.
//!
//! # Resume loses at most the batch in flight
//!
//! An entry is appended **after** its batch is staged, never before. So a process killed between
//! the call and the staging leaves no entry and the window is extracted again; a process killed
//! after staging leaves the entry and the window is skipped. The failure mode is one window
//! extracted twice, which costs one call and produces the same uids — content addressing makes
//! the repeat free — and the mode that would be unrecoverable, a window recorded as done that
//! was never staged, cannot happen.
//!
//! # Why a file and not a table
//!
//! §4.3.4 offers a redb `journal` table where the store has one and `<library>/journal.cbor`
//! where it does not. This implements the file, as the one implementation, and the reason is the
//! paragraph above: the correctness property is *append then flush*, which a length-prefixed
//! append-only file gives directly and a transactional key-value store gives by a longer route.
//! The access pattern does not argue the other way either — a journal is read once at the start
//! of a run and appended once per window, so there is nothing for an index to make faster. A
//! truncated tail, which is what a kill leaves, stops the read rather than failing it: the same
//! rule `from_cbor_seq` applies to a log.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use smysl_core::cbor::reader::Dec;
use smysl_core::cbor::writer::{Enc, MapBuilder};
use smysl_core::hash::hash_bytes;
use smysl_core::quote::Support;
use smysl_core::Tid;

/// The file a journal lives in, under the library root.
pub const FILE: &str = "journal.cbor";

/// What identifies one window's work.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub struct Key {
    /// The expression, as the catalog names it.
    pub alias: String,
    pub tid: Tid,
    /// The byte range of the part the window owned — not what it was shown. The overlap node
    /// belongs to the window before it, and counting it here would make two windows claim one
    /// node and leave the second extraction looking done when it was not.
    pub owned: (u64, u64),
    /// The recipe the window was extracted under.
    pub recipe: [u8; 32],
}

impl Key {
    pub fn new(alias: impl Into<String>, tid: Tid, owned: (u64, u64), recipe: [u8; 32]) -> Key {
        Key {
            alias: alias.into(),
            tid,
            owned,
            recipe,
        }
    }

    /// The key as stored: one hash over the four parts, length-delimited.
    ///
    /// Hashed rather than stored as a tuple because the alias is caller text of any length, and
    /// a fixed-size key keeps the journal's own encoding trivial. `0x1f` between the parts makes
    /// the concatenation injective, as `recipe.rs` does for the same reason.
    pub fn digest(&self) -> [u8; 32] {
        let mut b = Vec::with_capacity(self.alias.len() + 80);
        let mut push = |x: &[u8]| {
            b.extend_from_slice(&(x.len() as u32).to_be_bytes());
            b.extend_from_slice(x);
            b.push(0x1f);
        };
        push(self.alias.as_bytes());
        push(self.tid.canonical().as_bytes());
        push(&self.owned.0.to_be_bytes());
        push(&self.owned.1.to_be_bytes());
        push(&self.recipe);
        hash_bytes(&b)
    }
}

/// How the quotes of one window came out.
///
/// Counts and not the quotes themselves. The quote is already on every unit that kept one and in
/// the batch that was staged; what a rerun and the cost report (§6 TX-P5 step 5) need is the
/// *rate* — how often an extraction produced a quote the window supports — and three integers
/// answer that without putting a copy of the text in a second place.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Verdicts {
    pub present: u32,
    pub loose: u32,
    pub absent: u32,
}

impl Verdicts {
    pub fn count(&mut self, support: Support) {
        match support {
            Support::Present => self.present += 1,
            Support::Loose => self.loose += 1,
            Support::Absent => self.absent += 1,
            // `Support` is `#[non_exhaustive]`: a verdict this release does not know is not
            // counted as one it does, because the rate would then be a number about two things.
            _ => {}
        }
    }

    pub fn total(&self) -> u32 {
        self.present + self.loose + self.absent
    }
}

/// One window's record.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    pub key: Key,
    /// A hash over the staged batch, so a rerun can tell "already done" from "done differently".
    pub batch: [u8; 32],
    /// The ledger digest this window was sent, which the recipe deliberately does not carry: the
    /// recipe groups corroboration, and a per-window value there would put every window in a
    /// group of its own (§3.5).
    pub digest: [u8; 32],
    /// **This window's** input tokens, not the run's running total.
    ///
    /// Per window because the cost report divides by windows and multiplies by a retry rate
    /// (§3.7): a cumulative figure would make every entry a function of the ones before it in
    /// one run and incomparable across two.
    pub input_tokens: u64,
    /// This window's output tokens.
    pub output_tokens: u64,
    /// The language of the expression, as the manifest records it — `und` where it records
    /// none. In the entry and not in the key: a window is not *different work* for being in
    /// Russian, but the retry rate §3.7 wants is per language, and a journal is where it is
    /// measured rather than assumed.
    pub lang: String,
    /// Calls this window cost, repair turns included. Zero marks an entry written before this
    /// release, whose token figures are the run's running total rather than the window's.
    pub calls: u32,
    /// Of those calls, the repair turns: `calls - retries` is the number of first attempts,
    /// which is one per pass. `r = retries / first attempts` is §3.7's retry term.
    pub retries: u32,
    pub verdicts: Verdicts,
}

impl Entry {
    /// Whether this entry's cost figures can be read.
    ///
    /// An entry written before TX-P5 step 5 recorded no call count and its tokens are the
    /// *run's* running total at the moment the window was journalled. Those tokens cannot be
    /// summed with per-window ones, so the cost report counts such entries and leaves them out
    /// of the arithmetic rather than reporting a total that is two measurements added together.
    pub fn priced(&self) -> bool {
        self.calls > 0
    }

    /// First attempts: one per pass, repair turns excluded.
    pub fn attempts(&self) -> u32 {
        self.calls.saturating_sub(self.retries)
    }
}

// Map keys, small unsigned integers as every record in this format uses (§7.1 constraint 1).
mod keys {
    pub const KEY: u16 = 1;
    pub const BATCH: u16 = 2;
    pub const DIGEST: u16 = 3;
    pub const INPUT: u16 = 4;
    pub const OUTPUT: u16 = 5;
    pub const PRESENT: u16 = 6;
    pub const LOOSE: u16 = 7;
    pub const ABSENT: u16 = 8;
    pub const ALIAS: u16 = 9;
    pub const TID: u16 = 10;
    pub const FROM: u16 = 11;
    pub const TO: u16 = 12;
    pub const RECIPE: u16 = 13;
    // Added by TX-P5 step 5, for the cost report. A journal written before it decodes without
    // them — rule X in the small — and `Entry::priced` is how that is noticed.
    pub const LANG: u16 = 14;
    pub const CALLS: u16 = 15;
    pub const RETRIES: u16 = 16;
}

/// A journal, open for reading and appending.
#[derive(Debug)]
pub struct Journal {
    path: PathBuf,
    done: BTreeMap<[u8; 32], Entry>,
}

impl Journal {
    /// Open the journal under a library root, reading what is already there.
    ///
    /// A missing file is an empty journal: the first run of a corpus has nothing to resume. A
    /// **truncated tail** stops the read and keeps everything before it, which is what a kill
    /// leaves behind — and the window it was in the middle of is then extracted again, which is
    /// the losable half of the resume property.
    pub fn open(root: &Path) -> Journal {
        let path = root.join(FILE);
        let done = std::fs::read(&path)
            .map(|b| entries(&b))
            .unwrap_or_default();
        Journal { path, done }
    }

    /// A journal that is not written anywhere, for a caller with no library on disk.
    pub fn in_memory() -> Journal {
        Journal {
            path: PathBuf::new(),
            done: BTreeMap::new(),
        }
    }

    /// Whether this exact work is already recorded.
    pub fn contains(&self, key: &Key) -> bool {
        self.done.contains_key(&key.digest())
    }

    pub fn get(&self, key: &Key) -> Option<&Entry> {
        self.done.get(&key.digest())
    }

    /// Every entry, in key-digest order.
    ///
    /// The order is a hash order and therefore arbitrary — a caller that wants reading order
    /// sorts by `key.owned`. It exists for the cost report (§6 TX-P5 step 5), which sums tokens
    /// over a corpus and does not care which window came first.
    pub fn entries(&self) -> impl Iterator<Item = &Entry> {
        self.done.values()
    }

    pub fn len(&self) -> usize {
        self.done.len()
    }

    pub fn is_empty(&self) -> bool {
        self.done.is_empty()
    }

    /// Record one window, **after** its batch is staged.
    ///
    /// Appended and flushed before the call returns, because an entry buffered in a process that
    /// is about to be killed is an entry that says a window was done and cannot be read back.
    /// A journal with no path records in memory only, which is what a caller ingesting without
    /// a library on disk gets.
    pub fn record(&mut self, entry: Entry) -> std::io::Result<()> {
        let digest = entry.key.digest();
        if !self.path.as_os_str().is_empty() {
            let bytes = encode(&entry);
            let mut framed = Vec::with_capacity(bytes.len() + 4);
            framed.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            framed.extend_from_slice(&bytes);
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            f.write_all(&framed)?;
            f.flush()?;
            f.sync_data()?;
        }
        self.done.insert(digest, entry);
        Ok(())
    }

    /// Forget every entry for one expression, which is what `--reextract` asks for.
    ///
    /// In memory only: the file keeps its history, because a journal is a record of what was
    /// done and a rerun does not unmake the run before it. The entries this drops are simply
    /// not consulted, and the fresh ones are appended after them — so the newest entry for a key
    /// wins on the next open, which is how the file is decoded.
    pub fn forget(&mut self, alias: &str) {
        self.done.retain(|_, e| e.key.alias != alias);
    }
}

/// Decode a journal file, stopping at a truncated tail.
///
/// The newest entry for a key wins: the file is append-only, so a re-extraction after
/// `--reextract` appends a second entry for a key the first run wrote, and the later one is what
/// the corpus now holds.
fn entries(bytes: &[u8]) -> BTreeMap<[u8; 32], Entry> {
    let mut out = BTreeMap::new();
    let mut at = 0usize;
    while at + 4 <= bytes.len() {
        let len = u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]]);
        let from = at + 4;
        let to = from + len as usize;
        if to > bytes.len() {
            // The tail of a killed write. Everything before it is intact, and the window it
            // belonged to will be extracted again.
            break;
        }
        // A frame that does not decode is skipped rather than ending the read: its length was
        // readable, so the frames after it are still located correctly, and an entry nobody can
        // read is a window to extract again.
        if let Some(e) = decode(&bytes[from..to]) {
            out.insert(e.key.digest(), e);
        }
        at = to;
    }
    out
}

fn encode(e: &Entry) -> Vec<u8> {
    let mut key = MapBuilder::new();
    key.put(keys::ALIAS, |enc| enc.text(&e.key.alias));
    key.put(keys::TID, |enc| enc.bytes(e.key.tid.as_bytes()));
    key.put(keys::FROM, |enc| enc.uint(e.key.owned.0));
    key.put(keys::TO, |enc| enc.uint(e.key.owned.1));
    key.put(keys::RECIPE, |enc| enc.bytes(&e.key.recipe));

    let mut m = MapBuilder::new();
    m.put(keys::KEY, |enc| enc.raw(&key.into_bytes()));
    m.put(keys::BATCH, |enc| enc.bytes(&e.batch));
    m.put(keys::DIGEST, |enc| enc.bytes(&e.digest));
    m.put(keys::INPUT, |enc| enc.uint(e.input_tokens));
    m.put(keys::OUTPUT, |enc| enc.uint(e.output_tokens));
    m.put(keys::PRESENT, |enc| enc.uint(u64::from(e.verdicts.present)));
    m.put(keys::LOOSE, |enc| enc.uint(u64::from(e.verdicts.loose)));
    m.put(keys::ABSENT, |enc| enc.uint(u64::from(e.verdicts.absent)));
    m.put(keys::LANG, |enc| enc.text(&e.lang));
    m.put(keys::CALLS, |enc| enc.uint(u64::from(e.calls)));
    m.put(keys::RETRIES, |enc| enc.uint(u64::from(e.retries)));
    let mut enc = Enc::new();
    m.finish(&mut enc);
    enc.into_bytes()
}

fn decode(bytes: &[u8]) -> Option<Entry> {
    let mut d = Dec::new(bytes);
    let n = d.map_head().ok()?;
    let mut alias = String::new();
    let mut tid: Option<Tid> = None;
    let mut owned = (0u64, 0u64);
    let mut recipe = [0u8; 32];
    let mut batch = [0u8; 32];
    let mut digest = [0u8; 32];
    let mut input = 0u64;
    let mut output = 0u64;
    let mut verdicts = Verdicts::default();
    let mut lang = String::new();
    let mut calls = 0u32;
    let mut retries = 0u32;
    let mut prev = None;
    for _ in 0..n {
        let k = d.map_key(prev).ok()?;
        prev = Some(k);
        match k {
            keys::KEY => {
                let inner = d.skip_item().ok()?;
                let mut kd = Dec::new(inner);
                let kn = kd.map_head().ok()?;
                let mut kprev = None;
                for _ in 0..kn {
                    let kk = kd.map_key(kprev).ok()?;
                    kprev = Some(kk);
                    match kk {
                        keys::ALIAS => alias = kd.text().ok()?.to_string(),
                        keys::TID => tid = Some(Tid::from_bytes(thirty_two(kd.bytes().ok()?)?)),
                        keys::FROM => owned.0 = kd.uint().ok()?,
                        keys::TO => owned.1 = kd.uint().ok()?,
                        keys::RECIPE => recipe = thirty_two(kd.bytes().ok()?)?,
                        _ => {
                            kd.skip_item().ok()?;
                        }
                    }
                }
            }
            keys::BATCH => batch = thirty_two(d.bytes().ok()?)?,
            keys::DIGEST => digest = thirty_two(d.bytes().ok()?)?,
            keys::INPUT => input = d.uint().ok()?,
            keys::OUTPUT => output = d.uint().ok()?,
            keys::PRESENT => verdicts.present = d.uint().ok()? as u32,
            keys::LOOSE => verdicts.loose = d.uint().ok()? as u32,
            keys::ABSENT => verdicts.absent = d.uint().ok()? as u32,
            keys::LANG => lang = d.text().ok()?.to_string(),
            keys::CALLS => calls = d.uint().ok()? as u32,
            keys::RETRIES => retries = d.uint().ok()? as u32,
            // Rule X in the small: a key a later release added is skipped, not refused. A
            // journal is read by whatever build happens to resume a run.
            _ => {
                d.skip_item().ok()?;
            }
        }
    }
    Some(Entry {
        key: Key {
            alias,
            tid: tid?,
            owned,
            recipe,
        },
        batch,
        digest,
        input_tokens: input,
        output_tokens: output,
        lang,
        calls,
        retries,
        verdicts,
    })
}

fn thirty_two(b: &[u8]) -> Option<[u8; 32]> {
    b.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(alias: &str, at: u64, recipe: u8) -> Key {
        Key::new(alias, Tid::of(b"a part"), (at, at + 100), [recipe; 32])
    }

    fn entry(key: Key) -> Entry {
        Entry {
            key,
            batch: [7; 32],
            digest: [9; 32],
            input_tokens: 1200,
            output_tokens: 340,
            lang: "en".to_string(),
            calls: 3,
            retries: 1,
            verdicts: Verdicts {
                present: 3,
                loose: 1,
                absent: 0,
            },
        }
    }

    #[test]
    fn the_recipe_is_part_of_the_key_so_a_different_run_is_not_already_done() {
        let under_one = key("kjv", 0, 1);
        let under_two = key("kjv", 0, 2);
        assert_ne!(under_one.digest(), under_two.digest());
        let mut j = Journal::in_memory();
        j.record(entry(under_one.clone())).expect("recorded");
        assert!(j.contains(&under_one));
        assert!(
            !j.contains(&under_two),
            "a window extracted under another recipe counted as done",
        );
    }

    #[test]
    fn the_node_range_is_part_of_the_key() {
        let mut j = Journal::in_memory();
        j.record(entry(key("kjv", 0, 1))).expect("recorded");
        assert!(!j.contains(&key("kjv", 100, 1)));
    }

    #[test]
    fn an_entry_round_trips_through_the_file_and_resume_reads_it_back() {
        let dir = tempdir();
        {
            let mut j = Journal::open(&dir);
            assert!(j.is_empty());
            j.record(entry(key("kjv", 0, 1))).expect("recorded");
            j.record(entry(key("kjv", 100, 1))).expect("recorded");
        }
        let resumed = Journal::open(&dir);
        assert_eq!(resumed.len(), 2);
        let got = resumed.get(&key("kjv", 0, 1)).expect("an entry");
        assert_eq!(got.input_tokens, 1200);
        assert_eq!(got.output_tokens, 340);
        assert_eq!(got.verdicts.present, 3);
        assert_eq!(got.verdicts.loose, 1);
        assert_eq!(got.batch, [7; 32]);
        assert_eq!(got.digest, [9; 32]);
        assert_eq!(got.key.alias, "kjv");
        assert_eq!(got.key.owned, (0, 100));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_truncated_tail_keeps_every_entry_before_it() {
        let dir = tempdir();
        {
            let mut j = Journal::open(&dir);
            j.record(entry(key("kjv", 0, 1))).expect("recorded");
            j.record(entry(key("kjv", 100, 1))).expect("recorded");
        }
        // A kill in the middle of the second append.
        let path = dir.join(FILE);
        let bytes = std::fs::read(&path).expect("a journal");
        std::fs::write(&path, &bytes[..bytes.len() - 7]).expect("truncated");

        let resumed = Journal::open(&dir);
        assert_eq!(resumed.len(), 1, "the intact entry was lost with the tail");
        assert!(resumed.contains(&key("kjv", 0, 1)));
        assert!(
            !resumed.contains(&key("kjv", 100, 1)),
            "a half-written entry was believed",
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reextract_forgets_one_expression_and_leaves_the_others() {
        let mut j = Journal::in_memory();
        j.record(entry(key("kjv", 0, 1))).expect("recorded");
        j.record(entry(key("synodal", 0, 1))).expect("recorded");
        j.forget("kjv");
        assert!(!j.contains(&key("kjv", 0, 1)));
        assert!(j.contains(&key("synodal", 0, 1)));
    }

    #[test]
    fn a_later_entry_for_one_key_wins_on_the_next_open() {
        let dir = tempdir();
        {
            let mut j = Journal::open(&dir);
            j.record(entry(key("kjv", 0, 1))).expect("recorded");
            let mut second = entry(key("kjv", 0, 1));
            second.output_tokens = 999;
            j.record(second).expect("recorded");
        }
        let resumed = Journal::open(&dir);
        assert_eq!(resumed.len(), 1, "two entries for one key");
        assert_eq!(
            resumed
                .get(&key("kjv", 0, 1))
                .expect("an entry")
                .output_tokens,
            999,
            "the first run's entry outlived the re-extraction",
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_journal_with_no_path_records_nothing_to_disk_and_still_answers() {
        let mut j = Journal::in_memory();
        j.record(entry(key("kjv", 0, 1))).expect("recorded");
        assert!(j.contains(&key("kjv", 0, 1)));
    }

    #[test]
    fn the_cost_fields_round_trip_and_a_first_attempt_is_not_a_retry() {
        let dir = tempdir();
        {
            let mut j = Journal::open(&dir);
            j.record(entry(key("synodal", 0, 1))).expect("recorded");
        }
        let got = Journal::open(&dir);
        let e = got.get(&key("synodal", 0, 1)).expect("an entry");
        assert_eq!(e.lang, "en");
        assert_eq!(e.calls, 3);
        assert_eq!(e.retries, 1);
        assert_eq!(e.attempts(), 2, "two passes, one repair turn");
        assert!(e.priced());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// An entry a build before TX-P5 step 5 wrote: the three keys are simply absent, and what
    /// matters is that it still decodes — the tokens in it are a *run's* running total, and
    /// `priced` is what keeps them out of a per-window sum.
    #[test]
    fn an_entry_written_before_the_cost_fields_decodes_and_says_it_is_unpriced() {
        let k = key("kjv", 0, 1);
        let mut inner = MapBuilder::new();
        inner.put(keys::ALIAS, |enc| enc.text(&k.alias));
        inner.put(keys::TID, |enc| enc.bytes(k.tid.as_bytes()));
        inner.put(keys::FROM, |enc| enc.uint(k.owned.0));
        inner.put(keys::TO, |enc| enc.uint(k.owned.1));
        inner.put(keys::RECIPE, |enc| enc.bytes(&k.recipe));
        let mut m = MapBuilder::new();
        m.put(keys::KEY, |enc| enc.raw(&inner.into_bytes()));
        m.put(keys::BATCH, |enc| enc.bytes(&[7; 32]));
        m.put(keys::DIGEST, |enc| enc.bytes(&[9; 32]));
        m.put(keys::INPUT, |enc| enc.uint(99_000));
        m.put(keys::OUTPUT, |enc| enc.uint(9_000));
        let mut enc = Enc::new();
        m.finish(&mut enc);

        let e = decode(&enc.into_bytes()).expect("the old shape still decodes");
        assert_eq!(e.key, k);
        assert_eq!(e.input_tokens, 99_000);
        assert_eq!(e.lang, "");
        assert_eq!(e.calls, 0);
        assert!(!e.priced(), "an entry with no call count claimed a price");
    }

    /// A directory under the test temporary root, named for this test's own address.
    fn tempdir() -> PathBuf {
        let at = std::env::temp_dir().join(format!(
            "smysl-journal-{}-{:?}",
            std::process::id(),
            std::thread::current().id(),
        ));
        std::fs::create_dir_all(&at).expect("a directory");
        for f in std::fs::read_dir(&at).into_iter().flatten().flatten() {
            std::fs::remove_file(f.path()).ok();
        }
        at
    }
}
