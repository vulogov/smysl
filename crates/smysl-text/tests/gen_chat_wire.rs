//! `fixtures/library/chat-wire/` — **GE-T1's chat half**: the records a chat `text add` emits.
//!
//! GE-T1 asks that "Python, JavaScript and Go recompute tid/rdid/mid from the emitted records and
//! agree. They do not re-implement readers." `fixtures/library/wire/` already gives them that for
//! a scripture-shaped expression, and its records are hand-built in `smysl-core` — correctly, in
//! a crate with no reader. What it cannot give them is a **chat**, and a chat is where the three
//! identities are derived over things scripture does not have: a segment table with speakers,
//! timestamps and platform ids in it, a `mul` manifest, three readers over three unrelated
//! formats, and `raw` metadata inside the mid.
//!
//! So this file emits the records of a real `text add` — reader, normalisation, part cut, reading,
//! manifest — into a fixture the three ports read. They decode and hash; they still implement no
//! reader.
//!
//! The expectation is the ports' to check, not this file's: `ids.json` holds every identity with
//! the body bytes it was derived from, so a mismatch says whether the two sides encoded
//! differently or hashed differently. `SMYSL_BLESS=1` rewrites both files, which changes what
//! three other implementations are checked against.

#![cfg(all(
    feature = "reader-telegram",
    feature = "reader-slack",
    feature = "reader-whatsapp"
))]

use std::path::{Path, PathBuf};

use smysl_core::types::Carry;
use smysl_core::{to_cbor_seq, LangTag, Record};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::{Input, Params};

/// One export: the file, the reader that reads it, and the parameters it needs.
///
/// A named type for the reason `tests/library.rs` has one: the tuple is three wide with a slice
/// of pairs in it, and `clippy::type_complexity` is right about that.
type Chat<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

/// The three exports, the reader each is read by, and the parameters it needs.
const CHATS: &[Chat<'static>] = &[
    ("telegram.json", "telegram/1", &[]),
    ("whatsapp.txt", "whatsapp/1", &[("date-format", "dmy")]),
    ("slack.zip", "slack/1", &[]),
];

fn readers_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/library/readers")
}

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/library/chat-wire")
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Add the three exports to one library and collect what it wrote.
///
/// One library, not three: a chat corpus is several expressions in one catalog, and a fixture of
/// three separate libraries would not show that three manifests coexist in one record sequence.
fn emit() -> (Vec<Record>, String) {
    // A directory per call. The two tests here run as threads in one binary, and a shared path
    // means one test's `Drop` removes the library the other is still reading out of — which is
    // what happened the first time this file ran, as a missing object rather than as a race.
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "smysl-chat-wire-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let scratch = Scratch(dir.clone());

    let mut library = Library::create(&dir).expect("created");
    let mut records: Vec<Record> = Vec::new();
    let mut json = String::new();
    json.push_str("{\n");
    json.push_str(
        "  \"purpose\": \"The records a chat `text add` emits, for the three C-Read \
         implementations to recompute tid, rdid and mid from (GE-T1). Unlike \
         `fixtures/library/wire/`, these come from real readers over real exports: the segment \
         tables carry speakers, timestamps and platform ids, the manifests are `mul`, and two \
         of the three carry `raw` metadata inside the mid. The readers are not part of what is \
         checked — a port decodes these records and hashes them, and implements no reader.\",\n",
    );
    json.push_str("  \"expressions\": [\n");

    for (i, (file, reader, pairs)) in CHATS.iter().enumerate() {
        let bytes = std::fs::read(readers_dir().join(file))
            .unwrap_or_else(|e| panic!("fixtures/library/readers/{file}: {e}"));
        let mut params = Params::new();
        for (k, v) in *pairs {
            params.set(reader, *k, *v).expect("a parameter");
        }
        let mut spec = AddSpec::new(*reader, format!("chat-{i}"), "CC0-1.0")
            .with_lang(LangTag::new("mul").expect("a tag"))
            .with_carry(Carry::Text);
        spec.params = params;
        let added = library
            .add(&Input::new(&bytes), &spec, &Caps::DEFAULT)
            .unwrap_or_else(|e| panic!("{file}: {e}"));

        let manifest = library
            .store()
            .manifest(&added.mid)
            .expect("the manifest it just wrote")
            .clone();
        json.push_str(&format!(
            "    {{ \"file\": \"{}\", \"reader\": \"{}\", \"alias\": \"chat-{i}\", \
             \"mid_hex\": \"{}\", \"parts\": [\n",
            file,
            manifest.reader,
            hex(manifest.mid().as_bytes()),
        ));
        for (n, entry) in manifest.parts.iter().enumerate() {
            let part = library.part(&entry.tid).expect("the part it just wrote");
            let reading = library
                .part_reading(&added.mid, &entry.tid)
                .expect("the reading it just wrote");
            json.push_str(&format!(
                "      {{ \"tid_hex\": \"{}\", \"length\": {}, \"structure_hex\": \"{}\", \
                 \"rdid_hex\": \"{}\", \"segments\": {} }}{}\n",
                hex(entry.tid.as_bytes()),
                entry.length,
                hex(&entry.structure),
                hex(entry.rdid.as_bytes()),
                reading.rows.len(),
                if n + 1 == manifest.parts.len() {
                    ""
                } else {
                    ","
                },
            ));
            records.push(Record::PartText(part));
            records.push(Record::PartReading(reading.to_record()));
        }
        json.push_str(&format!(
            "    ] }}{}\n",
            if i + 1 == CHATS.len() { "" } else { "," }
        ));
        records.push(Record::Manifest(manifest));
    }
    json.push_str("  ]\n}\n");

    // Manifests first, then the text they name: the order `write_surface` emits in, and the
    // order a reader of the sequence meets the text before the claims drawn from it.
    records.sort_by_key(|r| match r {
        Record::Manifest(_) => 0,
        Record::PartText(_) => 1,
        _ => 2,
    });
    drop(scratch);
    (records, json)
}

fn check(name: &str, want: &[u8]) {
    let path = out_dir().join(name);
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::create_dir_all(out_dir()).expect("the fixture directory");
        std::fs::write(&path, want).expect("writing the fixture");
        return;
    }
    let have = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "fixtures/library/chat-wire/{name}: {e}\n  \
             SMYSL_BLESS=1 cargo test -p smysl-text --all-features --test gen_chat_wire \
             writes it, which changes what the Python, JavaScript and Go suites are checked \
             against"
        )
    });
    assert_eq!(
        have, want,
        "fixtures/library/chat-wire/{name} no longer matches what this build produces"
    );
}

#[test]
fn the_chat_wire_fixture_matches_this_build() {
    let (records, json) = emit();
    check("records.cbor", &to_cbor_seq(&records));
    check("ids.json", json.as_bytes());
}

/// The fixture is what GE-T1 needs it to be, rather than three files that happen to decode.
///
/// Three manifests, three readers, every part carrying both of its objects, and at least one
/// segment table with a speaker in it — which is the thing scripture has none of and the reason
/// this fixture exists beside `fixtures/library/wire/`.
#[test]
fn the_fixture_holds_what_ge_t1_asks_the_ports_to_check() {
    let (records, _) = emit();
    let manifests: Vec<&Record> = records
        .iter()
        .filter(|r| matches!(r, Record::Manifest(_)))
        .collect();
    assert_eq!(manifests.len(), 3, "one expression per export");

    let texts = records
        .iter()
        .filter(|r| matches!(r, Record::PartText(_)))
        .count();
    let readings = records
        .iter()
        .filter(|r| matches!(r, Record::PartReading(_)))
        .count();
    assert_eq!(texts, readings, "every part has both of its objects");
    assert!(texts >= 3, "at least one part per export");

    let mut speakers = 0usize;
    let mut raw = 0usize;
    for r in &records {
        match r {
            Record::PartReading(pr) => {
                let reading = smysl_text::reading::Reading::from_record(pr).expect("a reading");
                speakers += reading
                    .rows
                    .iter()
                    .filter(|row| row.speaker.is_some())
                    .count();
            }
            Record::Manifest(m) if m.raw.is_some() => raw += 1,
            _ => {}
        }
    }
    assert!(speakers > 0, "a chat reading names who said what");
    assert_eq!(raw, 2, "telegram and slack carry `raw`; whatsapp has none");
}
