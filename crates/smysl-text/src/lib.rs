//! `smysl-text` - the library layer: normalisation, parts, readings, structure, locators, and
//! the object store the bytes come to rest in (RFC SMYSL-2.4).
//!
//! `smysl-core` holds the *records* — manifest (14), part text (15), part reading (18) — and
//! the four identities over them, because a second implementation has to be able to decode and
//! derive those without owning a reader for a single file format (D-8). Everything that
//! **interprets** them is here: normalising bytes so a tid means one thing, grouping a
//! reader's nodes into parts, reading a segment table as a tree, resolving a locator, writing
//! an object and verifying it on the way back.
//!
//! # What this crate does not do
//!
//! It does not reach the network, link a runtime or read a clock inside a reader. The purity
//! gate (`cargo xtask check-purity`) holds it to that at default features **and** at
//! `--all-features`, which is the check the single-list version of that gate did not make:
//! with a feature per reader, "clean at default features" would be barely a claim about this
//! crate. A reader's only side channel is its [`limits::Budget`], and fuel is deterministic,
//! so a refusal happens at the same byte on every machine.
//!
//! # What is here as of TX-P1 step 3
//!
//! The substrate, and the six readers that stand on it:
//!
//! | module | what it settles |
//! |---|---|
//! | [`norm`] | the four normalisations, and the newtype that makes a tid mean one thing |
//! | [`ids`] | the identities, the `t3:…#locator` reference form |
//! | [`limits`] | the caps, and fuel |
//! | [`locator`] | the locator grammar, parsed and printed one way |
//! | [`reading`] | the segment table: record 18 key 2, and the structure hash over it |
//! | [`structure`] | the table read as a tree, and locator resolution |
//! | [`part`] | the part policy, and grouping whole nodes into parts |
//! | [`manifest`] | the licence gate (`SMY-E402`), and which manifests are heads |
//! | [`objects`] | `objects/{t3,r3}/…`, staged and renamed, verified on read (`SMY-E446`) |
//! | [`lock`] | one writer per shard, named (`SMY-E445`) |
//! | [`library`] | the catalog and the objects, opened together: `add`, `resolve`, `passage` |
//! | [`readers`] | the six TX-P1 readers, one behind each `reader-*` feature |
//! | [`segment`] | sentences, where no file format marks one (TX-P2) |
//! | [`analyze`] | terms, folded and stemmed per language (TX-P2) |
//! | `lang` | which language a text is in, behind feature `detect` (TX-P2) |
//!
//! Each reader is a function from bytes to a text and a table over it — no clock, no
//! environment, no filesystem, and no name for the input — so two libraries given the same
//! file name the same part. Three of them read scripture out of three unrelated syntaxes and
//! agree to the byte, which `crates/smysl-text/tests/readers.rs` asserts with one tid over
//! three fixtures.
//!
//! [`library`] is the one module that touches a directory, added in step 6 once step 4 had
//! given a `Store` manifests to hold — a handle written earlier would have been a handle to
//! half a library. Everything else here is still a function of its arguments, which is why all
//! of it is testable without a corpus.

#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]

pub mod analyze;
pub mod ids;
#[cfg(feature = "detect")]
pub mod lang;
pub mod library;
pub mod limits;
pub mod locator;
pub mod lock;
pub mod manifest;
pub mod norm;
pub mod objects;
pub mod part;
pub mod readers;
pub mod reading;
pub mod segment;
pub mod structure;

pub use analyze::Chain;
pub use ids::{Did, LangTag, Mid, Rdid};
pub use library::{AddSpec, Added, Library, Passage};
pub use limits::{Budget, Caps, Fuel};
pub use locator::{Locator, LocatorError};
pub use lock::{Holder, Lock};
pub use manifest::{carry_allowed, heads, ManifestBuilder};
pub use norm::Normalised;
pub use objects::{ObjectKind, ObjectStore};
pub use part::{group, PartPlan, Policy};
pub use readers::{reader, Input, NoReader, Params, ReadOutput, Reader};
pub use reading::{Level, Reading, Segment};
pub use segment::Segmenter;
pub use smysl_core::error::LibError;
pub use smysl_core::ids::Tid;
pub use structure::{Defect, Node, Structure, StructureError};

/// The reader ids this crate answers to, in the phase they arrive.
///
/// The id is what a manifest records (key 3) and what `SMY-E401` compares against when a
/// corpus is re-read, so the list of names is part of the format's surface. A reader whose id
/// is not here is a reader this build did not write; which of these a given *binary* has is
/// [`readers::available`], because each sits behind its own feature.
pub const READERS: &[&str] = &[
    // TX-P1 step 3, all six built.
    "txt/1",
    "md/1",
    "usfm/1",
    "osis/1",
    "zefania/1",
    "json/1",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The ids are `name/version`, lowercase, with one slash — the shape `SMY-E401`'s message
    /// and a manifest's key 3 both assume.
    #[test]
    fn every_reader_id_has_the_shape_a_manifest_records() {
        for id in READERS {
            let (name, version) = id.split_once('/').unwrap_or_else(|| panic!("{id}"));
            assert!(!name.is_empty() && !version.is_empty(), "{id}");
            assert!(
                name.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
                "{id}"
            );
            assert!(version.parse::<u32>().is_ok(), "{id}");
        }
        let mut sorted = READERS.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), READERS.len(), "no duplicate reader ids");
    }

    /// The pieces fit: text in, structure over it, a reading, a manifest entry, an object.
    ///
    /// One test rather than a module per seam, because what it is checking is that the
    /// *identities* line up — the tid of the part, the structure hash of the table, the rdid
    /// of the record — and those only line up all together or not at all.
    #[test]
    fn the_substrate_composes_into_one_part_with_four_identities() {
        let text = Normalised::of("In the beginning\nGod created\n");
        let part = part::text_of(&text, 0..text.len() as u64).unwrap();
        assert!(part.verify());

        let rows = vec![
            Segment::new(
                0,
                17,
                Level::new("verse").unwrap(),
                locator::parse("Gen.1.1").unwrap(),
            ),
            Segment::new(
                17,
                29,
                Level::new("verse").unwrap(),
                locator::parse("Gen.1.2").unwrap(),
            ),
        ];
        let reading = Reading::new(part.tid, "osis/1", rows.clone());
        let entry = reading.entry(part.text.len() as u64);
        reading.verify_entry(&entry).unwrap();

        let mut budget = Budget::new(Caps::DEFAULT, text.len() as u64).unwrap();
        let s = Structure::build(&rows, part.text.len() as u64, &mut budget).unwrap();
        assert_eq!(s.resolve(&locator::parse("Gen.1.2").unwrap()), Some(17..29));

        let m = ManifestBuilder::new(
            "kjv1769",
            smysl_core::LangTag::new("en").unwrap(),
            "osis/1",
            manifest::licence::PUBLIC_DOMAIN,
            &Policy::default(),
        )
        .unwrap()
        .part(entry)
        .carry(smysl_core::Carry::Text)
        .build()
        .unwrap();
        assert_eq!(m.length(), part.text.len() as u64);
        assert_eq!(m.parts[0].tid, part.tid);
        assert_eq!(m.parts[0].structure, reading.structure_hash());
        assert_eq!(m.parts[0].rdid, reading.rdid());
        assert_eq!(heads(vec![(m.mid(), &m)])["kjv1769"], vec![m.mid()]);
    }
}
