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

// ---------------------------------------------------------------------------
// TX-P2 step 2: the chat round trip, and what `--pseudonymise` does and does not change.
// ---------------------------------------------------------------------------

/// A chat fixture: the file, the reader that reads it, and the parameters it needs.
#[cfg(any(
    feature = "reader-telegram",
    feature = "reader-slack",
    feature = "reader-whatsapp"
))]
type Chat<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

/// The chat fixtures, with the reader each is read by and the parameters it needs.
#[cfg(any(
    feature = "reader-telegram",
    feature = "reader-slack",
    feature = "reader-whatsapp"
))]
const CHATS: &[Chat<'static>] = &[
    ("telegram.json", "telegram/1", &[]),
    ("whatsapp.txt", "whatsapp/1", &[("date-format", "dmy")]),
    ("slack.zip", "slack/1", &[]),
];

#[cfg(any(
    feature = "reader-telegram",
    feature = "reader-slack",
    feature = "reader-whatsapp"
))]
fn chat_spec(reader: &str, alias: &str, pairs: &[(&str, &str)]) -> AddSpec {
    use smysl_text::readers::Params;

    let mut params = Params::new();
    for (key, value) in pairs {
        params.set(reader, *key, *value).expect("a parameter");
    }
    let mut spec = AddSpec::new(reader, alias, "CC0-1.0")
        .with_lang(LangTag::new("mul").expect("a tag"))
        .with_carry(Carry::Text);
    spec.params = params;
    spec
}

#[cfg(any(
    feature = "reader-telegram",
    feature = "reader-slack",
    feature = "reader-whatsapp"
))]
fn chat_bytes(file: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/library/readers")
        .join(file);
    std::fs::read(&path).unwrap_or_else(|e| panic!("fixtures/library/readers/{file}: {e}"))
}

/// A chat export goes in and comes back out with its `raw` metadata byte for byte.
///
/// TX-P2 step 2's exit asks for exactly this. `raw` is the reader's own header — the chat's
/// name and kind, Slack's channel list — carried as opaque canonical CBOR in manifest key 16,
/// which means nothing downstream decodes it and therefore nothing downstream would notice it
/// being mangled. It is inside the mid, so a byte lost here renames the expression.
#[test]
#[cfg(any(feature = "reader-telegram", feature = "reader-slack"))]
fn a_chat_export_round_trips_with_its_raw_metadata_intact() {
    use smysl_text::readers::{self, Params};

    let dir = Scratch::new("chat-raw");
    let mut lib = Library::create(dir.path()).expect("created");
    let mut seen = 0usize;
    for (file, reader, pairs) in CHATS {
        if readers::reader(reader).is_err() {
            continue;
        }
        let bytes = chat_bytes(file);

        // What the reader produced, read directly, with nothing in between.
        let mut params = Params::new();
        for (key, value) in *pairs {
            params.set(reader, *key, *value).expect("a parameter");
        }
        let mut budget =
            smysl_text::limits::Budget::new(Caps::DEFAULT, bytes.len() as u64).expect("a budget");
        let out = readers::read_with(
            readers::reader(reader).expect("built"),
            &Input::new(&bytes),
            &params,
            &mut budget,
        )
        .expect("read");
        let Some(raw) = out.raw.clone() else {
            // `whatsapp/1` has no header to keep: a transcript carries no metadata at all.
            continue;
        };

        let added = lib
            .add(
                &Input::new(&bytes),
                &chat_spec(reader, &format!("chat-{seen}"), pairs),
                &Caps::DEFAULT,
            )
            .expect("added");
        let manifest = lib.store().manifest(&added.mid).expect("the manifest");
        assert_eq!(
            manifest.raw.as_ref(),
            Some(&raw),
            "{file}: the manifest's raw metadata is not what the reader produced"
        );
        // Canonical CBOR, decodable, and a map — which is what key 16 is specified to hold.
        let mut d = smysl_core::cbor::Dec::new(manifest.raw.as_ref().expect("raw"));
        let n = d.map_head().expect("a map");
        assert!(n > 0, "{file}: an empty map is not metadata");
        seen += 1;
    }
    assert!(
        seen > 0,
        "no chat reader in this build produced raw metadata"
    );
}

/// `--pseudonymise` changes the reading and not the text.
///
/// The property the whole design rests on ([`smysl_text::speaker`]): the speaker is a field of
/// a row, never a byte of the text, so a pseudonymised library and a plain one hold the **same
/// parts** — same tids, same objects — and differ only in the reading over them. Asserted
/// across two libraries rather than reasoned about, because the alternative design (rendering
/// `Alice: hello` into the text) is the one a reader would naturally write, and it would pass
/// every other test in this file.
#[test]
#[cfg(feature = "reader-telegram")]
fn pseudonymising_changes_the_reading_and_not_a_byte_of_the_text() {
    use smysl_text::speaker;

    let bytes = chat_bytes("telegram.json");
    let plain_dir = Scratch::new("chat-plain");
    let mut plain = Library::create(plain_dir.path()).expect("created");
    let plain_added = plain
        .add(
            &Input::new(&bytes),
            &chat_spec("telegram/1", "rg", &[]),
            &Caps::DEFAULT,
        )
        .expect("added");

    let key = speaker::Key::from_bytes([0x5a; 32]);
    let secret_dir = Scratch::new("chat-pseudo");
    let mut secret = Library::create(secret_dir.path()).expect("created");
    let secret_added = secret
        .add(
            &Input::new(&bytes),
            &chat_spec("telegram/1", "rg", &[]).pseudonymised(key.clone()),
            &Caps::DEFAULT,
        )
        .expect("added");

    assert_eq!(
        plain_added.parts, secret_added.parts,
        "the text is the same text, so the tids are the same tids"
    );
    assert_ne!(
        plain_added.mid, secret_added.mid,
        "the reading differs, so the manifest does"
    );

    let plain_manifest = plain.store().manifest(&plain_added.mid).expect("manifest");
    let secret_manifest = secret
        .store()
        .manifest(&secret_added.mid)
        .expect("manifest");
    assert_ne!(
        plain_manifest.parts[0].rdid, secret_manifest.parts[0].rdid,
        "the rdid is over the rows, and the rows changed"
    );
    // And the **structure hash moves too**, which this test was written expecting it not to.
    //
    // A-5 defines it as the hash of "the canonical CBOR of the structure table carried in the
    // reading's segments", and that table is one row per node with every field a reader filled
    // — including the speaker. So `structure` is a hash of the whole reading table rather than
    // of the tree, and `SMY-E401`'s `which: "structure"` does not mean "the segmentation
    // changed"; it means "the table changed", which a pseudonym does.
    //
    // Recorded rather than worked around. Narrowing A-5 to the tree alone would be an
    // amendment to the normative document and would change every structure hash ever computed,
    // so it is an open question for SMYSL-2.3 and not a decision for a reader's commit. What
    // follows from it today: pseudonymising a corpus re-identifies every *part entry* and no
    // part — the text, and therefore every tid and every object, is untouched.
    assert_ne!(
        plain_manifest.parts[0].structure, secret_manifest.parts[0].structure,
        "the table holds the speaker, so its hash moves with it"
    );

    let plain_reading = plain
        .part_reading(&plain_added.mid, &plain_added.parts[0])
        .expect("a reading");
    let secret_reading = secret
        .part_reading(&secret_added.mid, &secret_added.parts[0])
        .expect("a reading");
    let speakers: Vec<&str> = plain_reading
        .rows
        .iter()
        .filter_map(|r| r.speaker.as_deref())
        .collect();
    assert!(speakers.contains(&"user111"), "{speakers:?}");
    // Row by row, because the two readings are the same table with one field rewritten: same
    // count, same ranges, same locators, and each speaker the pseudonym of the one beside it.
    assert_eq!(plain_reading.rows.len(), secret_reading.rows.len());
    for (plain_row, secret_row) in plain_reading.rows.iter().zip(&secret_reading.rows) {
        assert_eq!(plain_row.locator, secret_row.locator);
        assert_eq!(
            (plain_row.start, plain_row.end),
            (secret_row.start, secret_row.end)
        );
        match (&plain_row.speaker, &secret_row.speaker) {
            (Some(plain_who), Some(secret_who)) => {
                assert!(speaker::is_pseudonym(secret_who), "{secret_who}");
                assert_eq!(secret_who, &key.pseudonym(plain_who));
            }
            (None, None) => {}
            (a, b) => panic!("one reading has a speaker and the other does not: {a:?} {b:?}"),
        }
    }
}

/// One person is one pseudonym, and a key read back from the library is the key.
///
/// Two adds of two different exports: the same platform id has to come out as the same
/// pseudonym, or the field buys nothing. The key is the one created on first use, read back
/// from `secrets/pseudonym.key` the way `text add --pseudonymise` reads it.
#[test]
#[cfg(feature = "reader-telegram")]
fn the_same_person_in_two_adds_is_one_pseudonym() {
    use smysl_text::{secrets, speaker};

    let dir = Scratch::new("chat-link");
    let mut lib = Library::create(dir.path()).expect("created");
    let key = secrets::load_or_create(dir.path(), || [7u8; 32]).expect("a key");
    // Read back rather than remembered: this is the call `text add` makes on the second run.
    let again = secrets::load(dir.path()).expect("read").expect("a key");
    assert_eq!(key, again);

    let bytes = chat_bytes("telegram.json");
    let first = lib
        .add(
            &Input::new(&bytes),
            &chat_spec("telegram/1", "one", &[]).pseudonymised(key.clone()),
            &Caps::DEFAULT,
        )
        .expect("added");
    // A second export of the same conversation with one message appended: a different text,
    // the same people.
    const APPENDED: &str = r#",
  {"id": 8, "type": "message", "date_unixtime": "1705392800",
   "from": "Ada", "from_id": "user111", "text": "One more thing."}
 ]
}"#;
    let more = String::from_utf8(bytes.clone())
        .expect("utf-8")
        .replace("\n ]\n}", APPENDED);
    assert_ne!(
        more.len(),
        bytes.len(),
        "the second export is a longer file"
    );
    let second = lib
        .add(
            &Input::new(more.as_bytes()),
            &chat_spec("telegram/1", "two", &[]).pseudonymised(again),
            &Caps::DEFAULT,
        )
        .expect("added");

    let who = |added: &smysl_text::Added, lib: &Library| -> Vec<String> {
        lib.part_reading(&added.mid, &added.parts[0])
            .expect("a reading")
            .rows
            .iter()
            .filter_map(|r| r.speaker.clone())
            .collect()
    };
    let first_speakers = who(&first, &lib);
    let second_speakers = who(&second, &lib);
    assert!(first_speakers.iter().all(|s| speaker::is_pseudonym(s)));
    assert_eq!(
        first_speakers[0], second_speakers[0],
        "one person, two exports, one pseudonym"
    );
    assert_eq!(first_speakers[0], key.pseudonym("user111"));
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
