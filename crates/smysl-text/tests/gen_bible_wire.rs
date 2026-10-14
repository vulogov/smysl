//! `fixtures/library/bibles/wire/` — a real corpus expression, for the three ports.
//!
//! GE-T1 asks that Python, JavaScript and Go recompute tid, rdid and mid from the emitted
//! records and agree. They have done that since TX-P1 step 7 against
//! `fixtures/library/wire/`, whose two parts are a sentence of English and a sentence of
//! Russian written for the purpose — enough to exercise NFC and a two-row segment table, and
//! not enough to be a corpus.
//!
//! This is the same thing over material a reader actually produced: the book of Ruth in the
//! Russian Synodal edition, four chapters, cut into parts by the part policy and read by
//! `usfm/1`, with a segment table of a hundred and some verses per part. One edition and not
//! five, because what the ports derive is language-independent — a tid is a hash of bytes —
//! while what *can* go wrong in a port is reading a **byte** offset out of a segment table over
//! multibyte text. Cyrillic is where an implementation that indexed by character instead of by
//! byte produces an identity that matches and a span that is one letter wide; the other four
//! editions would cost 300 KiB to say the same thing. The generator takes the edition as a
//! constant, so producing them is an edit and not a design.
//!
//! The bundle is the shape `fixtures/library/wire/ids.json` already has, minus the records that
//! have nothing to do with a corpus: no redaction, no datings, no domain-byte table, because
//! those are claims about the format and are checked there.
//!
//! Like `gen_library_fixtures.rs` and for the same reason, the generator **is** the test: it
//! rebuilds the bundle from the committed slice and compares, so a change to the encoder, the
//! reader or the part policy fails here instead of leaving three implementations checked
//! against bytes this build no longer produces. `SMYSL_BLESS=1` rewrites it, and that should be
//! a decision.

#![cfg(feature = "reader-usfm")]

use std::path::{Path, PathBuf};

use smysl_core::cbor::envelope::{manifest_bytes, part_reading_bytes};
use smysl_core::ids::LangTag;
use smysl_core::types::Carry;
use smysl_core::{to_cbor_seq, Record};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input;

/// The expression the bundle is built from: Ruth, Russian Synodal, four chapters.
const EDITION: &str = "russyn";
const BOOK: &str = "08-RUT.usfm";
const LANG: &str = "ru";
const VERSIFICATION: &str = "synodal";

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/library/bibles/wire")
}

/// Build the one expression and return the bundle's two files, as bytes.
fn build() -> (String, Vec<u8>) {
    // A counter as well as the pid: the two tests here both build, on two threads, and one
    // name would have them opening and deleting each other's library.
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let root = std::env::temp_dir().join(format!(
        "smysl-bible-wire-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch directory");
    let mut lib = Library::create(&root).expect("a library");

    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/library/bibles/slice")
        .join(EDITION)
        .join(BOOK);
    let raw = std::fs::read(&source).unwrap_or_else(|e| panic!("{}: {e}", source.display()));

    let alias = format!("{EDITION}/{}", BOOK.trim_end_matches(".usfm")).to_lowercase();
    let mut spec = AddSpec::new("usfm/1", &alias, "public-domain")
        .with_lang(LangTag::new(LANG).expect("a tag"))
        .with_carry(Carry::Text);
    spec.versification = Some(VERSIFICATION.to_string());
    let added = lib
        .add(&Input::new(&raw), &spec, &Caps::DEFAULT)
        .expect("added");

    let manifest = lib
        .store()
        .manifest(&added.mid)
        .expect("the manifest it wrote")
        .clone();
    let mut records = vec![Record::Manifest(manifest.clone())];
    let mut parts = String::new();
    for (i, tid) in added.parts.iter().enumerate() {
        let text = lib.part(tid).expect("the part's bytes");
        let read = lib
            .part_reading(&added.mid, tid)
            .expect("the part's reading");
        let segments = read.rows.len();
        let reading = read.to_record();
        parts.push_str(&format!(
            "    {{ \"tid_hex\": \"{}\", \"length\": {}, \"segments\": {}, \
             \"text_hex\": \"{}\", \"rdid_hex\": \"{}\", \"reading_body_hex\": \"{}\", \
             \"structure_hex\": \"{}\" }}{}\n",
            hex(text.tid.as_bytes()),
            text.text.len(),
            segments,
            hex(&text.text),
            hex(reading.rdid().as_bytes()),
            hex(&part_reading_bytes(&reading)),
            hex(&reading.structure_hash()),
            if i + 1 == added.parts.len() { "" } else { "," },
        ));
        records.push(Record::PartText(text));
        records.push(Record::PartReading(reading));
    }
    let _ = std::fs::remove_dir_all(&root);

    let ids = format!(
        "{{\n  \"purpose\": \"GE-T1's port clause over corpus material: one expression read by \
         `usfm/1` from the committed slice at ../slice/, with the identities a port must \
         recompute. Same derivations as ../../wire/ids.json — tid over the part's normalised \
         bytes, mid over the manifest body, rdid over the reading body, `structure_hex` over \
         the segment table alone with no domain byte — over a reader's own output in Cyrillic \
         rather than over a sentence written for a fixture. Written by \
         crates/smysl-text/tests/gen_bible_wire.rs.\",\n  \
         \"edition\": \"{EDITION}\",\n  \"book\": \"{BOOK}\",\n  \
         \"manifest\": {{ \"alias\": \"{}\", \"lang\": \"{}\", \"reader\": \"{}\", \
         \"versification\": \"{}\", \"mid_hex\": \"{}\", \"body_hex\": \"{}\" }},\n  \
         \"parts\": [\n{parts}  ]\n}}\n",
        manifest.alias,
        manifest.lang,
        manifest.reader,
        VERSIFICATION,
        hex(added.mid.as_bytes()),
        hex(&manifest_bytes(&manifest)),
    );
    (ids, to_cbor_seq(&records))
}

/// The bundle on disk is the bundle this build produces.
#[test]
fn the_bible_wire_bundle_is_what_this_build_emits() {
    let (ids, records) = build();
    let dir = dir();
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::create_dir_all(&dir).expect("the fixture directory");
        std::fs::write(dir.join("ids.json"), &ids).expect("ids.json");
        std::fs::write(dir.join("records.cbor"), &records).expect("records.cbor");
        println!("  blessed: {}", dir.display());
        return;
    }
    let on_disk = std::fs::read_to_string(dir.join("ids.json"))
        .unwrap_or_else(|e| panic!("{}: {e}", dir.join("ids.json").display()));
    assert_eq!(
        on_disk, ids,
        "the identity bundle three ports check against is not what this build emits"
    );
    let bytes = std::fs::read(dir.join("records.cbor")).expect("records.cbor");
    assert_eq!(
        bytes, records,
        "the record sequence three ports decode is not what this build encodes"
    );
}

/// The bundle is corpus material and not a miniature, which is the whole reason it exists.
///
/// Four parts over four chapters, a segment table of real size in each, and text that is not
/// ASCII — a bundle that lost any of those would still pass every derivation in all three
/// ports while testing what the original fixture already tested.
#[test]
fn the_bundle_is_four_parts_of_multibyte_text_with_real_segment_tables() {
    let (ids, records) = build();
    let parts = ids.matches("\"tid_hex\"").count();
    assert_eq!(parts, 4, "Ruth is four chapters and the policy merged none");
    let segments: usize = ids
        .split("\"segments\": ")
        .skip(1)
        .map(|s| {
            s.split(',')
                .next()
                .expect("a number")
                .trim()
                .parse::<usize>()
                .expect("a count")
        })
        .sum();
    // Ruth's 85 verses — 22, 23, 18 and 22 — plus the chapter row each part carries above its
    // own verses. The book row is not among them: a book spans every part, and a part-local
    // segment table holds the levels that fit inside it.
    assert_eq!(segments, 85 + 4, "Ruth's verses and a chapter row per part");
    // The text is Cyrillic, so it is longer in bytes than in characters — the property a port
    // that indexed a segment table by character would break on.
    assert!(
        records.len() > 20_000,
        "{} bytes is not a book of scripture",
        records.len()
    );
}
