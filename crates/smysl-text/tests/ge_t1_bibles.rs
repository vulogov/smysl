//! **GE-T1 — determinism, over the corpus it asked for.** (SMYSL-2.4 §6, GE-T1's row at §5.5.)
//!
//! The gate asks that each library build the same way on two platforms, that mids, structure
//! hashes and rdids compare equal, and that the three ports recompute the identities from the
//! emitted records. The port half has been met since TX-P1 step 7 against
//! `fixtures/library/wire/`. What has been owed since then is the **corpus**: "the five Bibles",
//! which this repository does not hold and should not — three to six megabytes of scripture
//! per edition is not a fixture.
//!
//! So the corpus stays outside and the *measurement* comes inside. `SMYSL_GE_T1=<dir>` names a
//! directory of `<edition>/*.usfm`, one file per book; this harness builds a real library per
//! edition — `Library::create`, then `Library::add` per book with the `usfm/1` reader — and
//! compares every identity it produced against `fixtures/library/bibles/expected.tsv`. The
//! expectations are a few kilobytes because they are not the corpus: per edition, the digest of
//! the input bytes, the counts, a **roll** (BLAKE3 over the whole canonical identity listing,
//! so nothing is compared loosely), and the first and last expression's identities in full so a
//! mismatch says *where*.
//!
//! What that buys, and it is the whole point: the comparison between two platforms is no longer
//! two people agreeing over a phone call. A run on any machine with the same input digest
//! either reproduces the roll or does not.
//!
//! # What this measures, and what it cannot
//!
//! It measures reading files: line endings, NFC through whatever Unicode table the platform's
//! build used, the part policy's arithmetic over real chapter lengths, and the segment table a
//! real reader built over 1,189 chapters in five scripts. Those are the places a platform can
//! differ. It does **not** measure BLAKE3 or canonical CBOR, which are platform-independent by
//! construction — a point worth keeping in view, because it is why a single-platform run is
//! still worth having.
//!
//! The input is eBible's verse-per-line distribution of five public-domain editions, turned
//! into USFM one file per book. That conversion is mechanical and it is not this repository's
//! text — but it *is* this repository's framing, so the expectations pin the digest of the
//! converted bytes rather than of the download: a reader who reconverts differently gets told
//! that the inputs differ, not that the identities do.
//!
//! # Without the corpus
//!
//! Every test here says the clause is owed and returns. That is deliberate: the sweep must not
//! need three megabytes of scripture to pass, and a gate that quietly passed on an absent
//! corpus would be worse than one that says so.
//!
//! `SMYSL_BLESS=1` rewrites the expectations, and that should be a decision — it changes what
//! two platforms are compared against.

#![cfg(feature = "reader-usfm")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use smysl_core::hash_bytes;
use smysl_core::ids::LangTag;
use smysl_core::types::Carry;
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input;

/// The editions GE-T1 names, with the language each is in and the versification it may claim.
///
/// The scheme is stated only where it is a fact about the edition rather than a guess: the KJV
/// is the numbering `kjv` names and the Synodal is `synodal`, while Luther 1912, Segond 1910
/// and Reina-Valera 1909 are *mostly* KJV-numbered and an expression that makes no claim is not
/// aligned rather than aligned by guess (SMYSL-2.4 §3.1, manifest key 14).
const EDITIONS: &[(&str, &str, Option<&str>)] = &[
    ("deu1912", "de", None),
    ("eng-kjv", "en", Some("kjv")),
    ("fraLSG", "fr", None),
    ("russyn", "ru", Some("synodal")),
    ("spaRV1909", "es", None),
];

/// The fixture's own first lines, so it says what it is to whoever opens it.
const HEADER: &str = "\
# GE-T1's corpus clause (SMYSL-2.4 §5.5): scripture built into a library with the `usfm/1`
# reader, with every identity rolled into one digest so two platforms compare exactly.
#
# Columns: edition, key, value.
#   input_hex   BLAKE3 over the input file names and their digests. A mismatch here says the
#               corpus differs, not that the build does.
#   roll_hex    BLAKE3 over the canonical identity listing: every alias and mid, and every
#               part's tid, length, structure hash and rdid, in alias then part order.
#   first/last  the first and last book in full, so a moved roll has somewhere to start.
#
# Written by crates/smysl-text/tests/ge_t1_bibles.rs under SMYSL_BLESS=1 with SMYSL_GE_T1
# pointing at the corpus. Blessing changes what two platforms are compared against.
";

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// The expectations for the whole corpus, and for the slice that is committed beside them.
const EXPECTED: &str = "expected.tsv";
const SLICE: &str = "slice.tsv";

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/library/bibles")
        .join(name)
}

/// The corpus directory, or `None` and a line saying the clause is owed.
fn corpus() -> Option<PathBuf> {
    match std::env::var_os("SMYSL_GE_T1") {
        Some(d) => Some(PathBuf::from(d)),
        None => {
            println!(
                "  no corpus: set SMYSL_GE_T1=<dir> of <edition>/*.usfm.\n  \
                 GE-T1's corpus clause is owed, not met."
            );
            None
        }
    }
}

/// A scratch library, removed when the test ends however it ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        // A counter as well as the pid, because two tests in this file build the same edition
        // and the suite runs them on two threads: one name per edition made the second build
        // open the first one's library, which is exactly the shape of bug a determinism gate
        // must not have in its own instrument.
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "smysl-ge-t1-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("a scratch directory");
        Scratch(p)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One expression's identities, in the order the roll hashes them.
struct Expression {
    alias: String,
    mid_hex: String,
    parts: Vec<(String, u64, String, String)>,
}

/// What one edition's build produced.
struct Built {
    /// BLAKE3 over `name\0digest\0…` of every input file, in name order: which bytes were read.
    input_hex: String,
    books: usize,
    parts: usize,
    bytes: u64,
    lossy: bool,
    expressions: Vec<Expression>,
}

impl Built {
    /// BLAKE3 over the whole identity listing.
    ///
    /// A roll rather than a list of hashes in the fixture, because the list is six thousand rows
    /// per edition and the fixture is read by people. The listing is canonical — alias order,
    /// then part order, tab-separated — so two builds that produced the same identities produce
    /// the same bytes here, and the first and last expression are carried in full beside it so a
    /// mismatch has somewhere to start.
    fn roll(&self) -> String {
        let mut listing = String::new();
        for e in &self.expressions {
            listing.push_str(&format!("{}\t{}\n", e.alias, e.mid_hex));
            for (tid, len, structure, rdid) in &e.parts {
                listing.push_str(&format!("\t{tid}\t{len}\t{structure}\t{rdid}\n"));
            }
        }
        hex(&hash_bytes(listing.as_bytes()))
    }

    /// The fixture's rows for this edition: `edition<TAB>key<TAB>value`, one fact per line.
    ///
    /// A TSV and not JSON, because the only reader of this file is the test below and a
    /// hand-rolled parser for a nested object is a second thing that can be wrong about the
    /// first. Every value is a hex digest, a decimal, a bool or a short ASCII alias, so a line
    /// has no escaping and a `git diff` of a changed identity is one line long.
    fn rows(&self, edition: &str, lang: &str, versification: Option<&str>) -> String {
        let mut out = String::new();
        let mut row = |key: &str, value: &str| {
            out.push_str(&format!("{edition}\t{key}\t{value}\n"));
        };
        row("lang", lang);
        row("reader", "usfm/1");
        row("versification", versification.unwrap_or("-"));
        row("input_hex", &self.input_hex);
        row("books", &self.books.to_string());
        row("parts", &self.parts.to_string());
        row("bytes", &self.bytes.to_string());
        row("lossy", &self.lossy.to_string());
        row("roll_hex", &self.roll());
        for (label, e) in [
            ("first", self.expressions.first().expect("a first book")),
            ("last", self.expressions.last().expect("a last book")),
        ] {
            let (tid, len, structure, rdid) = &e.parts[0];
            row(&format!("{label}.alias"), &e.alias);
            row(&format!("{label}.mid_hex"), &e.mid_hex);
            row(&format!("{label}.part_count"), &e.parts.len().to_string());
            row(&format!("{label}.first_part.tid_hex"), tid);
            row(&format!("{label}.first_part.length"), &len.to_string());
            row(&format!("{label}.first_part.structure_hex"), structure);
            row(&format!("{label}.first_part.rdid_hex"), rdid);
        }
        out
    }
}

/// Build one edition into a fresh library, in book order.
///
/// One expression per book rather than one per Bible, which is how scripture is distributed and
/// also what makes the parts meaningful: `usfm/1` cuts on `chapter`, so a book is tens of parts
/// and a mid names something a person can ask for.
fn build(dir: &Path, edition: &str, lang: &str, versification: Option<&str>) -> Built {
    let root = Scratch::new(edition);
    let mut lib = Library::create(&root.0).expect("a library");

    let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join(edition))
        .unwrap_or_else(|e| panic!("{edition}: {e}"))
        .map(|e| e.expect("a directory entry").path())
        .filter(|p| p.extension().is_some_and(|x| x == "usfm"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "{edition}: no .usfm files");

    let mut inputs = Vec::new();
    let (mut parts, mut bytes, mut lossy) = (0usize, 0u64, false);
    let mut expressions = Vec::new();
    for file in &files {
        let name = file
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .to_string();
        let raw = std::fs::read(file).unwrap_or_else(|e| panic!("{name}: {e}"));
        inputs.push(format!("{name}\0{}\0", hex(&hash_bytes(&raw))));

        // The alias is the edition and the book, lowercase: `deu1912/01-gen`. The file's own
        // stem carries the canonical order, so a catalog listing reads in book order rather
        // than alphabetically — Genesis before Exodus, not 1Chr before Acts.
        let stem = name.trim_end_matches(".usfm").to_lowercase();
        let alias = format!("{edition}/{stem}").to_lowercase();
        let mut spec = AddSpec::new("usfm/1", &alias, "public-domain")
            .with_lang(LangTag::new(lang).expect("a tag"))
            .with_carry(Carry::Text);
        spec.versification = versification.map(str::to_string);

        let added = lib
            .add(&Input::new(&raw), &spec, &Caps::DEFAULT)
            .unwrap_or_else(|e| panic!("{alias}: {e}"));
        parts += added.parts.len();
        bytes += added.bytes;
        lossy |= added.lossy;

        let manifest = lib
            .store()
            .manifest(&added.mid)
            .unwrap_or_else(|| panic!("{alias}: the manifest it just wrote"));
        // The manifest is where the three identities live together, which is why they are read
        // from it rather than recomputed here: a harness that derived them a second way would
        // be comparing this file's arithmetic with itself.
        expressions.push(Expression {
            alias: manifest.alias.clone(),
            mid_hex: hex(added.mid.as_bytes()),
            parts: manifest
                .parts
                .iter()
                .map(|p| {
                    (
                        hex(p.tid.as_bytes()),
                        p.length,
                        hex(&p.structure),
                        hex(p.rdid.as_bytes()),
                    )
                })
                .collect(),
        });
    }
    Built {
        input_hex: hex(&hash_bytes(inputs.concat().as_bytes())),
        books: files.len(),
        parts,
        bytes,
        lossy,
        expressions,
    }
}

/// The fixture, as a map from edition to its `key -> value` rows.
///
/// A key that appears twice for one edition is a failure rather than a last-one-wins: a fixture
/// two platforms compare through may not hold two answers to the same question.
fn expected(name: &str) -> BTreeMap<String, BTreeMap<String, String>> {
    let path = fixture(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut out: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut col = line.split('\t');
        let (Some(edition), Some(key), Some(value)) = (col.next(), col.next(), col.next()) else {
            panic!(
                "line {}: `{line}` is not `edition<TAB>key<TAB>value`",
                n + 1
            );
        };
        assert!(
            col.next().is_none(),
            "line {}: a fourth column, and a value with a tab in it is not one of these",
            n + 1
        );
        assert!(
            out.entry(edition.to_string())
                .or_default()
                .insert(key.to_string(), value.to_string())
                .is_none(),
            "line {}: {edition} answers `{key}` twice",
            n + 1
        );
    }
    for (edition, _, _) in EDITIONS {
        assert!(
            out.contains_key(*edition),
            "the expectations hold no {edition}: {:?}",
            out.keys().collect::<Vec<_>>()
        );
    }
    out
}

/// Build every edition under `dir` and compare, or bless, the expectations in `name`.
///
/// One function for the whole Bibles and for the committed slice, because the two differ only in
/// which bytes they read: a slice measured by a second code path would be a slice that proves
/// nothing about the corpus.
fn measure(dir: &Path, name: &str, what: &str) {
    let bless = std::env::var_os("SMYSL_BLESS").is_some();
    let have = (!bless).then(|| expected(name));

    let mut rows = Vec::new();
    for (edition, lang, versification) in EDITIONS {
        let built = build(dir, edition, lang, *versification);
        println!(
            "  {edition}: {} books, {} parts, {} bytes, roll {}",
            built.books,
            built.parts,
            built.bytes,
            &built.roll()[..16]
        );
        if let Some(have) = &have {
            let want = &have[*edition];
            let check = |key: &str, got: String| {
                assert_eq!(
                    want.get(key).map(String::as_str),
                    Some(got.as_str()),
                    "{edition}: {key} moved"
                );
            };
            // The inputs first. A corpus that is not the corpus the fixture was written over
            // makes every identity below meaningless, and saying "the roll moved" about
            // different bytes would send a reader looking in the wrong place.
            check("input_hex", built.input_hex.clone());
            check("books", built.books.to_string());
            check("parts", built.parts.to_string());
            check("bytes", built.bytes.to_string());
            check("lossy", built.lossy.to_string());
            check("roll_hex", built.roll());
            for (label, e) in [
                ("first", built.expressions.first().expect("a first")),
                ("last", built.expressions.last().expect("a last")),
            ] {
                check(&format!("{label}.alias"), e.alias.clone());
                check(&format!("{label}.mid_hex"), e.mid_hex.clone());
                check(&format!("{label}.part_count"), e.parts.len().to_string());
                check(&format!("{label}.first_part.tid_hex"), e.parts[0].0.clone());
                check(
                    &format!("{label}.first_part.length"),
                    e.parts[0].1.to_string(),
                );
                check(
                    &format!("{label}.first_part.structure_hex"),
                    e.parts[0].2.clone(),
                );
                check(
                    &format!("{label}.first_part.rdid_hex"),
                    e.parts[0].3.clone(),
                );
            }
        }
        rows.push(built.rows(edition, lang, *versification));
    }

    if bless {
        let path = fixture(name);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("the fixture directory");
        std::fs::write(&path, format!("# {what}\n{HEADER}{}", rows.concat())).expect("the fixture");
        println!("  blessed: {}", path.display());
    }
}

/// Every identity of the whole corpus, against the file two platforms compare through.
///
/// One test and not five, because the five editions share one library build each and the
/// expensive part is reading twenty megabytes of scripture: splitting it per edition would
/// quintuple the setup to report the same thing.
#[test]
fn five_bibles_build_to_the_identities_the_fixture_pins() {
    let Some(dir) = corpus() else { return };
    measure(
        &dir,
        EXPECTED,
        "The five Bibles entire, read from the directory SMYSL_GE_T1 named.",
    );
}

/// The same measurement over two whole books per edition, from material that is **committed**.
///
/// This is the test that runs on two platforms without anybody arranging a corpus, which is
/// what GE-T1's platform clause actually needs: Ruth and Jude in five editions, five scripts,
/// complete books rather than excerpts, 112 KiB in the tree. Ruth is four chapters and Jude is
/// one, so a multi-part expression and a single-part one are both built — and the work a part's
/// identity depends on is *reading a file*, which is the half that can differ by platform. What
/// the slice cannot exercise is the part policy's merging: every chapter here clears the
/// minimum, so five chapters are five parts, and the merging is measured over the whole corpus.
///
/// It does not replace the clause above. A slice is not the five Bibles, and the fixture for the
/// whole corpus is what says the editions were read end to end. What the slice buys is that the
/// comparison happens on every push, on both of CI's platforms, instead of when somebody
/// remembers to mount three megabytes of scripture.
#[test]
fn the_committed_slice_builds_the_same_way_on_whatever_platform_this_is() {
    measure(
        &fixture("slice"),
        SLICE,
        "Ruth and Jude in five editions, from `slice/`: the part of the corpus small enough \
         to commit, so both of CI's platforms compare on every push.",
    );
}

/// The fixture is the five editions and not a stub, whether or not a corpus is present.
///
/// Worth its own test for the reason the generator in `smysl-core` is not `#[ignore]`: a
/// fixture that three ports and two platforms compare against must not be able to go missing
/// or empty without something failing. This one runs in every sweep, corpus or no corpus.
#[test]
fn the_expectations_name_five_editions_and_a_roll_for_each() {
    let have = expected(EXPECTED);
    assert_eq!(have.len(), EDITIONS.len(), "five editions, no more");
    for (edition, lang, versification) in EDITIONS {
        let want = &have[*edition];
        assert_eq!(want.get("lang").map(String::as_str), Some(*lang));
        assert_eq!(want.get("reader").map(String::as_str), Some("usfm/1"));
        assert_eq!(
            want.get("versification").map(String::as_str),
            Some(versification.unwrap_or("-")),
            "{edition}: the scheme is a claim and is written only where it is one"
        );
        for key in [
            "input_hex",
            "roll_hex",
            "first.mid_hex",
            "last.mid_hex",
            "first.first_part.tid_hex",
            "first.first_part.structure_hex",
            "first.first_part.rdid_hex",
        ] {
            let got = want
                .get(key)
                .unwrap_or_else(|| panic!("{edition}: no {key}"));
            assert_eq!(got.len(), 64, "{edition}: {key} is not a 256-bit digest");
            assert!(
                got.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
                "{edition}: {key} is not lowercase hex"
            );
        }
        // 66 books is the canon, and a corpus that arrived with 39 is one somebody truncated.
        assert_eq!(want.get("books").map(String::as_str), Some("66"));
        // The chapter count is 1,189 and the part count is **not**: `usfm/1` cuts on `chapter`,
        // so a part is one or more whole chapters, and the part policy's minimum merges the
        // short ones — which is why the count lands in the 1,130s rather than on the nose. What
        // the policy can never do is *split* a chapter, so more parts than chapters would mean
        // the cut level had moved under the fixture.
        let parts: usize = want
            .get("parts")
            .expect("a part count")
            .parse()
            .expect("a number");
        assert!(
            (1000..=1189).contains(&parts),
            "{edition}: {parts} parts over 1,189 chapters"
        );
    }
}

/// The slice's own expectations, and the thing that makes them a slice rather than a stub.
#[test]
fn the_slice_is_two_whole_books_in_five_editions() {
    let have = expected(SLICE);
    assert_eq!(have.len(), EDITIONS.len(), "five editions, no more");
    for (edition, _, _) in EDITIONS {
        let want = &have[*edition];
        assert_eq!(want.get("books").map(String::as_str), Some("2"));
        assert_eq!(want.get("parts").map(String::as_str), Some("5"));
        assert_eq!(
            want.get("first.alias").map(String::as_str),
            // Lowercased, and that is the format rather than this file being tidy: an alias is
            // lowercase ASCII, so eBible's `fraLSG` is the expression `fralsg`.
            Some(format!("{}/08-rut", edition.to_lowercase()).as_str()),
            "Ruth first, four chapters of it"
        );
        assert_eq!(
            want.get("last.alias").map(String::as_str),
            Some(format!("{}/65-jud", edition.to_lowercase()).as_str()),
            "and Jude, which is one"
        );
        // Not an empty read dressed up as a passing one: the Russian edition is the longest of
        // the five in bytes because Cyrillic is two bytes a letter in UTF-8, and a slice that
        // came back under a kilobyte would be a slice somebody truncated.
        let bytes: u64 = want
            .get("bytes")
            .expect("a byte count")
            .parse()
            .expect("a number");
        assert!(bytes > 10_000, "{edition}: {bytes} bytes is not two books");
    }
}

/// Two builds of the same bytes in the same process produce the same roll.
///
/// The weakest half of determinism and the cheapest to check, and it is not vacuous: the roll
/// is taken over a listing in alias and part order, so anything that reached for a `HashMap`
/// iteration order or a filesystem order on the way would show up here before a second platform
/// ever had to be found.
#[test]
fn the_same_bytes_build_the_same_roll_twice() {
    let Some(dir) = corpus() else { return };
    let (edition, lang, versification) = EDITIONS[0];
    let first = build(&dir, edition, lang, versification);
    let second = build(&dir, edition, lang, versification);
    assert_eq!(
        first.roll(),
        second.roll(),
        "{edition}: two builds disagree"
    );
    assert_eq!(first.input_hex, second.input_hex);
}
