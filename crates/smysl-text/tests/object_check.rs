//! `SMY-E446` and `SMY-E401` against a real object store.
//!
//! The object half of `check`'s `Library` pass (pass 12, RFC SMYSL-2.4 §4.3.3). It lives here
//! rather than beside the rest of that pass's tests for two reasons, and only the second is
//! about convenience.
//!
//! A log holds no text (OQ-39), so these two codes are about bytes in an object store, and
//! `ObjectStore` is this crate's. `smysl-check` cannot depend on this crate — §4.3.3 reserves
//! that edge for the `Time` pass behind feature `text` — so the trait they meet through is
//! `smysl-core`'s `PartResolver`, and this is the side of it with an implementation.
//!
//! And the facade's dependency tree is printed in the manual. A dev-dependency on this crate
//! added `smysl-text` to the facade's `[dev-dependencies]` subtree, which `make doc-cargo`
//! reported as drift — correctly. Step 6 puts this crate in the facade's real tree; churning
//! the transcript a step early, for a test, is not worth it.

use std::path::PathBuf;
use std::sync::Arc;

use smysl_check::{check, CheckOptions};
use smysl_core::diag::Code;
use smysl_core::ids::LangTag;
use smysl_core::types::{Manifest, PartEntry, PartReading, PartResolver, PartText, Record};
use smysl_graph::Store;
use smysl_text::objects::ObjectStore;

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("smysl-library-check-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// A manifest naming one part, with the entry a correct object would satisfy.
fn manifest_for(part: &PartText) -> Manifest {
    let reading = PartReading::new(part.tid, "txt/1", PartReading::EMPTY_SEGMENTS.to_vec());
    let mut m = Manifest::new(
        "notes-txt",
        LangTag::new("en").expect("a tag"),
        "txt/1",
        "CC0-1.0",
        "smysl/parts/1 level=line min=65536 max=4194304",
    );
    m.parts.push(PartEntry::new(
        part.tid,
        part.text.len() as u64,
        [0u8; 32],
        reading.rdid(),
    ));
    m
}

/// The three states of a resolver, each against a real object store on disk.
///
/// Absence is not a defect — a manifest may name parts whose text this library does not hold,
/// which is what `carry: ref` is for — and that control is the one that keeps `SMY-E446` from
/// firing on every catalog entry.
#[test]
fn the_object_half_reports_corruption_and_not_absence() {
    let dir = scratch("objects");
    let part = PartText::new(b"the notes, normalised\n".to_vec());
    let manifest = manifest_for(&part);
    let store = Store::from_records(vec![Record::Manifest(manifest)]);

    let objects = ObjectStore::open(&dir).expect("an object store");
    let resolver: Arc<dyn PartResolver + Send + Sync + core::panic::RefUnwindSafe> =
        Arc::new(objects.clone());

    // 1. Nothing stored. Not a defect.
    let report = check(
        &store,
        CheckOptions::default().with_parts(Arc::clone(&resolver)),
    );
    assert!(
        report.is_empty(),
        "a manifest may name text it does not hold: {report}"
    );

    // 2. Stored correctly. Still nothing.
    objects.put_part(&part).expect("stored");
    let report = check(
        &store,
        CheckOptions::default().with_parts(Arc::clone(&resolver)),
    );
    assert!(report.is_empty(), "{report}");

    // 3. The bytes on disk changed under the name. `SMY-E446`.
    let path = objects.part_path(&part.tid);
    let mut bytes = std::fs::read(&path).expect("the object");
    let last = bytes.len() - 1;
    bytes[last] ^= 0x20;
    std::fs::write(&path, &bytes).expect("rewritten");
    let report = check(
        &store,
        CheckOptions::default().with_parts(Arc::clone(&resolver)),
    );
    assert_eq!(report.count(Code::E446), 1, "{report}");

    // 4. Not a part record at all. Also `SMY-E446`, and this is the case a resolver that
    //    folded "absent" together with "unreadable" would have had to report as absence.
    std::fs::write(&path, b"not cbor at all").expect("rewritten");
    let report = check(
        &store,
        CheckOptions::default().with_parts(Arc::clone(&resolver)),
    );
    assert_eq!(report.count(Code::E446), 1, "{report}");

    // And without a resolver, none of it is checked and nothing is claimed.
    let report = check(&store, CheckOptions::default());
    assert_eq!(report.count(Code::E446), 0);

    let _ = std::fs::remove_dir_all(&dir);
}

/// `SMY-E401`: the object is intact and the catalog entry is wrong about it.
///
/// This is the half §4.3.3's table does not list, and the one nothing else in `check` can
/// raise. The object hashes to its tid, so the bytes *are* the part; a part entry claiming a
/// different length is the manifest disagreeing with the text it names.
#[test]
fn a_part_entry_that_lies_about_its_length_is_reported() {
    let dir = scratch("length");
    let part = PartText::new(b"the notes, normalised\n".to_vec());
    let mut manifest = manifest_for(&part);
    manifest.parts[0].length += 1;
    let store = Store::from_records(vec![Record::Manifest(manifest)]);

    let objects = ObjectStore::open(&dir).expect("an object store");
    objects.put_part(&part).expect("stored");
    let resolver: Arc<dyn PartResolver + Send + Sync + core::panic::RefUnwindSafe> =
        Arc::new(objects);

    let report = check(&store, CheckOptions::default().with_parts(resolver));
    assert_eq!(report.count(Code::E401), 1, "{report}");
    assert_eq!(
        report.count(Code::E446),
        0,
        "the bytes are intact; only the entry is wrong: {report}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
