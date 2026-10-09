//! The library handle: a catalog and an object store, opened together (TX-P1 step 6).
//!
//! The load-bearing test here is `a_text_cut_into_several_parts_resolves_in_every_one`. Every
//! reader fixture in the tree produces exactly **one** part, so until this file nothing had
//! ever exercised a text that is cut — and a reading's segment offsets are part-local, so the
//! one-part case is precisely the case where the shift that makes that true is zero.

// Every case here reads a file, and `txt/1` is the reader they read it with. At default
// features this crate has no readers at all (`default = []`, which is what the purity gate
// checks), so without the gate the suite would fail in a configuration where there is nothing
// for it to test — and `make crate-features` builds exactly that configuration.
#![cfg(feature = "reader-txt")]

use std::path::{Path, PathBuf};

use smysl_core::ids::LangTag;
use smysl_core::types::library::{PartResolver, Resolved};
use smysl_core::types::Carry;
use smysl_text::library::{AddSpec, Library, LAYOUT, MARKER};
use smysl_text::limits::Caps;
use smysl_text::locator;
use smysl_text::part::Policy;
use smysl_text::readers::Input;
use smysl_text::reading::Level;

struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Scratch {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let p = std::env::temp_dir().join(format!(
            "smysl-library-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).expect("a scratch directory");
        Scratch(p)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn spec() -> AddSpec {
    AddSpec::new("txt/1", "notes", "CC0-1.0")
        .with_lang(LangTag::new("en").expect("a tag"))
        .with_carry(Carry::Text)
}

/// Nine lines, so a small `target_min` cuts them into several parts.
const NOTES: &str = "\
the first line of the notes
the second, a little longer than the first
the third
the fourth line mentions the engine
the fifth
the sixth line is about the pool
the seventh
the eighth line closes the morning
the ninth
";

#[test]
fn creating_a_library_writes_a_marker_and_opening_one_without_it_fails() {
    let dir = Scratch::new("marker");
    assert!(!Library::is_library(dir.path()));
    assert!(Library::open(dir.path()).is_err(), "no marker, no library");

    let _lib = Library::create(dir.path()).expect("created");
    assert!(Library::is_library(dir.path()));
    assert_eq!(
        std::fs::read_to_string(dir.path().join(MARKER))
            .expect("the marker")
            .trim(),
        LAYOUT
    );
    // Idempotent: `text add` into a fresh directory must not need two commands.
    Library::create(dir.path()).expect("created again");
}

/// A layout this release does not know is refused by name rather than half-read.
#[test]
fn a_library_from_another_layout_is_refused() {
    let dir = Scratch::new("layout");
    Library::create(dir.path()).expect("created");
    std::fs::write(dir.path().join(MARKER), "smysl/library/99\n").expect("rewritten");

    let err = Library::open(dir.path()).expect_err("refused");
    let message = err.to_string();
    assert!(message.contains("smysl/library/99"), "{message}");
    assert!(message.contains("another release"), "{message}");
}

#[test]
fn a_text_added_once_is_resolvable_by_locator() {
    let dir = Scratch::new("add");
    let mut lib = Library::create(dir.path()).expect("created");

    let added = lib
        .add(&Input::new(NOTES.as_bytes()), &spec(), &Caps::DEFAULT)
        .expect("added");
    assert_eq!(added.parts.len(), 1, "the default policy makes one part");
    assert_eq!(added.bytes, NOTES.len() as u64);
    assert_eq!(added.objects_written, 2, "a part text and its reading");

    // The manifest is in the catalog, under the alias, as the only head.
    assert_eq!(lib.store().manifest_count(), 1);
    assert_eq!(lib.store().heads("notes"), vec![added.mid]);
    assert_eq!(lib.catalog()["notes"], vec![added.mid]);

    let passage = lib
        .passage(
            "notes",
            &locator::parse("L4").expect("a locator"),
            &Caps::DEFAULT,
        )
        .expect("resolved");
    assert_eq!(passage.text, "the fourth line mentions the engine");
    assert_eq!(passage.mid, added.mid);
    assert_eq!(passage.tid, added.parts[0]);

    // And by mid, which is what `text show m3:…#L4` does.
    let by_mid = lib
        .passage(
            &added.mid.canonical(),
            &locator::parse("L6").expect("a locator"),
            &Caps::DEFAULT,
        )
        .expect("resolved");
    assert_eq!(by_mid.text, "the sixth line is about the pool");
}

/// **The step's real test.** A text cut into several parts resolves in every one of them.
///
/// A reading's `start`/`end` are offsets into the part, and a reader produces one table over
/// the whole text. A part beginning at byte 100 whose rows still said 100 would be a table
/// `Structure::build` refuses — and before it refused anything, the structure hash in the
/// manifest entry would already be a hash of the wrong table. Every reader fixture in the tree
/// produces one part, where the shift is zero, so nothing had ever run this.
#[test]
fn a_text_cut_into_several_parts_resolves_in_every_one() {
    let dir = Scratch::new("parts");
    let mut lib = Library::create(dir.path()).expect("created");

    // A policy sized for the fixture rather than for a Bible: the default `target_min` is
    // 64 KiB, which would make any test text one part.
    let policy = Policy::new(Level::new("line").expect("a level"), 40, 120);
    let added = lib
        .add(
            &Input::new(NOTES.as_bytes()),
            &spec().with_policy(policy),
            &Caps::DEFAULT,
        )
        .expect("added");
    assert!(
        added.parts.len() >= 3,
        "the policy should cut nine lines into several parts, got {}",
        added.parts.len()
    );
    assert_eq!(
        added.objects_written,
        added.parts.len() * 2,
        "a text and a reading for each part"
    );

    // Every line resolves, in whichever part holds it, to exactly its own text.
    for (n, line) in NOTES.lines().enumerate() {
        let loc = locator::parse(&format!("L{}", n + 1)).expect("a locator");
        let passage = lib
            .passage("notes", &loc, &Caps::DEFAULT)
            .unwrap_or_else(|e| panic!("L{}: {e}", n + 1));
        assert_eq!(passage.text, line, "L{}", n + 1);
        assert!(
            added.parts.contains(&passage.tid),
            "L{} resolved outside the manifest's parts",
            n + 1
        );
    }

    // Each part's structure is checkable against its own entry, which is the assertion that
    // the structure hash was computed over the part-local table.
    let manifest = lib.store().manifest(&added.mid).expect("the manifest");
    let mut total = 0u64;
    for entry in &manifest.parts {
        let structure = lib
            .structure(&added.mid, &entry.tid, &Caps::DEFAULT)
            .expect("the structure");
        assert_eq!(structure.length(), entry.length);
        assert!(!structure.is_empty());
        total += entry.length;
    }
    assert_eq!(
        total,
        NOTES.len() as u64,
        "the parts cover the text exactly"
    );
}

/// Adding the same bytes twice writes no object and names the same manifest.
#[test]
fn re_adding_an_unchanged_text_writes_nothing() {
    let dir = Scratch::new("idempotent");
    let mut lib = Library::create(dir.path()).expect("created");

    let first = lib
        .add(&Input::new(NOTES.as_bytes()), &spec(), &Caps::DEFAULT)
        .expect("added");
    let second = lib
        .add(&Input::new(NOTES.as_bytes()), &spec(), &Caps::DEFAULT)
        .expect("added again");

    assert_eq!(first.mid, second.mid, "same bytes, same manifest identity");
    assert_eq!(first.parts, second.parts);
    assert_eq!(
        second.objects_written, 0,
        "the objects were already there; an existing target is success"
    );
    // And the log holds the manifest once: `append` refuses a record it already has.
    assert_eq!(lib.store().manifest_count(), 1);
}

/// A library survives being closed and reopened, which is the only thing the log is for.
#[test]
fn a_library_reopens_with_its_catalog() {
    let dir = Scratch::new("reopen");
    let mid = {
        let mut lib = Library::create(dir.path()).expect("created");
        lib.add(&Input::new(NOTES.as_bytes()), &spec(), &Caps::DEFAULT)
            .expect("added")
            .mid
    };

    let lib = Library::open(dir.path()).expect("reopened");
    assert_eq!(lib.store().heads("notes"), vec![mid]);
    let passage = lib
        .passage(
            "notes",
            &locator::parse("L9").expect("a locator"),
            &Caps::DEFAULT,
        )
        .expect("resolved");
    assert_eq!(passage.text, "the ninth");
}

/// `SMY-E402` before a byte is written.
///
/// The licence gate runs first, so a refused add leaves no object behind. Finding out after
/// writing would mean a library holding text it may not pass on, and an operator with no way
/// to tell which objects came from the refused command.
#[test]
fn a_licence_that_refuses_carry_refuses_before_writing() {
    let dir = Scratch::new("licence");
    let mut lib = Library::create(dir.path()).expect("created");

    let spec = AddSpec::new("txt/1", "notes", "CC-BY-NC-4.0")
        .with_lang(LangTag::new("en").expect("a tag"))
        .with_carry(Carry::Text);
    let err = lib
        .add(&Input::new(NOTES.as_bytes()), &spec, &Caps::DEFAULT)
        .expect_err("refused");
    assert_eq!(err.code(), Some(smysl_core::Code::E402));

    assert_eq!(lib.store().manifest_count(), 0);
    let objects = walk(&dir.path().join("objects"));
    assert!(objects.is_empty(), "nothing written: {objects:?}");
}

/// A boundary level that names no node is a refusal, not an empty manifest.
///
/// The default policy cuts on `chapter` and `txt/1` has no chapters, so this is the case that
/// comes up without anybody doing anything unusual. An empty manifest would be valid, the
/// licence recorded, the alias taken, and not one byte of the text reachable.
#[test]
fn a_boundary_level_that_names_nothing_is_refused() {
    let dir = Scratch::new("noparts");
    let mut lib = Library::create(dir.path()).expect("created");

    let policy = Policy::new(Level::new("chapter").expect("a level"), 40, 120);
    let err = lib
        .add(
            &Input::new(NOTES.as_bytes()),
            &spec().with_policy(policy),
            &Caps::DEFAULT,
        )
        .expect_err("refused");
    let message = err.to_string();
    assert!(message.contains("chapter"), "{message}");
    assert!(
        message.contains("line"),
        "it must name the reader's own top level: {message}"
    );
    assert_eq!(
        err.code(),
        None,
        "a refusal to begin carries no corpus code"
    );
    assert_eq!(lib.store().manifest_count(), 0);
}

/// Two manifests for one alias: `text ls` shows the fork and `text show` refuses to guess.
#[test]
fn a_forked_alias_is_shown_and_not_resolved() {
    let dir = Scratch::new("fork");
    let mut lib = Library::create(dir.path()).expect("created");

    lib.add(&Input::new(NOTES.as_bytes()), &spec(), &Caps::DEFAULT)
        .expect("added");
    let other = format!("{NOTES}a tenth line somebody else had\n");
    lib.add(&Input::new(other.as_bytes()), &spec(), &Caps::DEFAULT)
        .expect("added");

    let heads = lib.catalog()["notes"].clone();
    assert_eq!(heads.len(), 2, "neither supersedes the other");

    let err = lib.head("notes").expect_err("refused");
    let message = err.to_string();
    assert!(message.contains("2 heads"), "{message}");
    assert!(message.contains("SMY-W418"), "{message}");
    // Naming one of them works, which is what the message tells the operator to do.
    assert!(lib.head(&heads[0].canonical()).is_ok());
}

/// The library is a `PartResolver`, so `check --library` can verify its objects.
///
/// It resolves through the object store *unverified* on purpose: `Library::part` verifies, and
/// a verified read can never hand back the bad object `SMY-E446` exists to report.
#[test]
fn the_library_resolves_parts_for_check() {
    let dir = Scratch::new("resolver");
    let mut lib = Library::create(dir.path()).expect("created");
    let added = lib
        .add(&Input::new(NOTES.as_bytes()), &spec(), &Caps::DEFAULT)
        .expect("added");
    let tid = added.parts[0];

    assert!(matches!(PartResolver::part(&lib, &tid), Resolved::Part(_)));
    assert!(lib.part(&tid).is_ok(), "and verified, through `part`");

    // Damage the object. The resolver still hands it over; the verified accessor refuses.
    let path = lib.objects().part_path(&tid);
    let mut bytes = std::fs::read(&path).expect("the object");
    let last = bytes.len() - 1;
    bytes[last] ^= 0x20;
    std::fs::write(&path, &bytes).expect("rewritten");

    let lib = Library::open(dir.path()).expect("reopened");
    assert!(matches!(PartResolver::part(&lib, &tid), Resolved::Part(_)));
    let err = lib.part(&tid).expect_err("the verified read refuses");
    assert_eq!(err.code(), Some(smysl_core::Code::E446));
}

/// `Library::add` and the reader fixtures agree on every identity.
///
/// `tests/readers.rs` assembles a manifest from a `ReadOutput` by hand, and so does
/// `Library::add`. Two implementations of the same arithmetic are worth having — that is how
/// the part-local offset question came up at all — but only if something makes them agree. The
/// fixture's `.expected` file is the referee: it pins the tid, the structure hash, the rdid and
/// the mid, and this adds the same bytes through the library and checks all four against it.
#[test]
fn adding_a_fixture_reproduces_the_identities_recorded_beside_it() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/library/readers");
    let bytes = std::fs::read(root.join("notes.txt")).expect("the fixture");
    let expected = std::fs::read_to_string(root.join("notes.txt.expected")).expect("expectations");

    let field = |name: &str| -> String {
        expected
            .lines()
            .find_map(|l| l.trim().strip_prefix(name))
            .unwrap_or_else(|| panic!("no `{name}` line in the expectation"))
            .trim()
            .to_string()
    };

    let dir = Scratch::new("fixture");
    let mut lib = Library::create(dir.path()).expect("created");
    let added = lib
        .add(
            &Input::new(&bytes),
            &AddSpec::new("txt/1", "notes-txt", "CC0-1.0")
                .with_lang(LangTag::new("en").expect("a tag"))
                .with_carry(Carry::Text),
            &Caps::DEFAULT,
        )
        .expect("added");

    assert_eq!(added.parts.len(), 1);
    assert_eq!(added.parts[0].canonical(), field("tid"));
    assert_eq!(added.mid.canonical(), field("mid"));
    assert_eq!(added.bytes, bytes.len() as u64);

    let manifest = lib.store().manifest(&added.mid).expect("the manifest");
    assert_eq!(manifest.parts[0].rdid.canonical(), field("rdid"));
    assert_eq!(
        manifest.parts[0]
            .structure
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        field("structure")
    );
    assert_eq!(manifest.parts[0].length, bytes.len() as u64);
}

/// Every regular file under a directory, relative to it.
fn walk(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(
                    p.strip_prefix(root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }
    out.sort();
    out
}
