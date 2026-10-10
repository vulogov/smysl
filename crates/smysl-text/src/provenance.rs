//! The copy set: what a unit drawn from a part carries, and nothing else (D-1, A-2.4).
//!
//! **A prohibition as much as a function.** D-1 names six fields a unit extracted from a part
//! carries — `source.reference` (the tid and locator), `span`, `manifest`, `published` *or*
//! `observed`, core `lang`, and for chat readers the payload key `text:speaker` — and then says
//! producers MUST NOT copy other manifest metadata onto units. Title, creators, page, URL: all
//! reachable through the manifest, none of them on the unit. So the way to keep the rule is to
//! have one function that writes provenance and have it write only those fields; a producer
//! that assembled a `SourceRef` by hand would be one `title` away from putting a book's name
//! inside ten thousand uids.
//!
//! **Everything here is inside identity.** `source` sits inside `UnitCore`, so each of these
//! rules decides uids, which is why A-2.4 states them normatively rather than as advice: two
//! implementations that disagreed about whether to copy `published` would give the same passage
//! two uids and neither would be wrong about anything else.
//!
//! What this release can write is the reference, `published` and `observed`. `span`, `manifest`
//! and core `lang` arrive with FC-1 and FC-3 in TX-P5, and `text:speaker` with the `x.text/v1`
//! payload; each is a field the format does not have yet rather than a rule left unapplied.

use smysl_core::types::{Manifest, SourceKind, SourceRef, UnitCoreBuilder};
use smysl_core::Tid;

use crate::time::edtf;
use crate::time::{Instant, Interval};

/// Where a unit came from, as the library knows it before the unit exists.
///
/// Not [`crate::Passage`], which is what `text show` resolves and carries the bytes. This is the
/// provenance half alone, so that the one function allowed to write a unit's `source` cannot be
/// handed the text and tempted to put some of it in.
#[derive(Debug, Clone, Copy)]
pub struct Drawn<'a> {
    /// The part the text came from.
    pub tid: Tid,
    /// The locator of the node the unit was extracted from, if the extractor knows one.
    ///
    /// **Informative, not authoritative.** When a locator and a span disagree the span wins and
    /// `SMY-W405` is reported (A-2.2) — which is why the span is the thing TX-P5 adds and this
    /// is the thing that can be absent without loss.
    pub locator: Option<&'a str>,
    /// The manifest the extraction ran under, for its `published` (key 8) alone.
    pub manifest: &'a Manifest,
    /// The segment's `observed`, when the reading gave one (record 18, key 6).
    ///
    /// A chat reader gives one for every message; a Bible reader gives none, because a verse
    /// has no send time. The two cases are what rules 2 and 3 are about.
    pub observed: Option<u64>,
}

/// Write the copy set onto a builder.
///
/// Infallible on purpose. Rule 2 says the copied value is the manifest's **byte for byte**, so
/// a manifest whose `published` is malformed EDTF is copied malformed — the alternative is a
/// unit whose provenance differs from the manifest it names, and a reader could then not tell
/// which of the two was wrong. The malformed value is reported where a store is checked
/// (`SMY-E410`), not repaired where a unit is built.
pub fn stamp(b: UnitCoreBuilder, d: &Drawn<'_>) -> UnitCoreBuilder {
    let mut source = SourceRef::new(SourceKind::Doc, reference(d));
    match (d.observed, d.manifest.published.as_deref()) {
        // Rule 2, **copy**: no `observed`, and the manifest says when it was published, so the
        // unit says so too. This is what makes a verse of the 1769 KJV dated at all: the part
        // carries no instant, and the catalog entry does.
        (None, Some(published)) => {
            source.published = Some(published.to_string());
        }
        // Rule 3, **omission**, and rule 4, both present. The unit has an instant of its own;
        // whether it also carries the manifest's publication date turns on whether the two say
        // the same thing, which `omits` answers exactly.
        (Some(observed), Some(published)) => {
            source = source.observed_at(observed);
            if !omits(published, observed) {
                source.published = Some(published.to_string());
            }
        }
        (Some(observed), None) => {
            source = source.observed_at(observed);
        }
        // Rule 5: neither present means the unit is undated on the *said* axis. Not early, not
        // late — which is a different thing from being dated to the epoch, and the reason this
        // arm writes nothing rather than writing a zero.
        (None, None) => {}
    }
    b.source(source)
}

/// `t3:<52 chars>` or that followed by `#` and a locator (A-2.2).
fn reference(d: &Drawn<'_>) -> String {
    match d.locator {
        Some(l) => format!("{}#{l}", d.tid.canonical()),
        None => d.tid.canonical(),
    }
}

/// Rule 3: whether `published` MUST be omitted beside this `observed`.
///
/// True only when the publication value's normalised interval **equals** the one millisecond
/// the instant denotes. The comparison is between intervals and is therefore exact, and the
/// consequence is that the rule almost never fires: EDTF level 1 cannot write a millisecond, so
/// the narrowest value it can express is a second — and `2026-09-30T14:05:00Z` is a one-second
/// interval that is not equal to any instant inside it.
///
/// That is the rule working rather than the rule failing. The failure it prevents is a producer
/// reasoning "these say the same thing, drop one" and dropping the one that carried a second of
/// uncertainty, which would move the unit's uid and lose the difference between "published at
/// 14:05:00" and "observed at 14:05:00.237".
///
/// A value that does not parse is never omitted: with no interval there is nothing to compare,
/// and keeping both fields loses nothing.
pub fn omits(published: &str, observed: u64) -> bool {
    let Ok(v) = edtf::parse(published) else {
        return false;
    };
    let instant = match i64::try_from(observed) {
        Ok(ms) => Instant(ms),
        Err(_) => return false,
    };
    edtf::to_interval(&v) == Interval::at(instant)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::ids::LangTag;
    use smysl_core::types::Status;
    use smysl_core::KernelType;

    fn manifest(published: Option<&str>) -> Manifest {
        let mut m = Manifest::new(
            "kjv/1769",
            LangTag::new("en").expect("a tag"),
            "osis/1",
            "public-domain",
            "smysl/parts/1 level=top min=1024 max=4194304",
        );
        m.published = published.map(str::to_string);
        m.title = Some("The Holy Bible: King James Version".to_string());
        m.creators = vec!["The Translators".to_string()];
        m
    }

    fn drawn<'a>(m: &'a Manifest, observed: Option<u64>) -> Drawn<'a> {
        Drawn {
            // Through `norm` and `ids`, as every caller in this crate must: the bytes-taking
            // constructor has exactly one call site and `tests/norm_is_the_only_gate.rs` is
            // what keeps that true. It caught this line when it was written the short way.
            tid: crate::ids::tid(&crate::norm::Normalised::of("In the beginning")),
            locator: Some("Gen.1.1"),
            manifest: m,
            observed,
        }
    }

    fn unit(d: &Drawn<'_>) -> SourceRef {
        stamp(
            UnitCoreBuilder::new(KernelType::Claim, "a claim about a verse", Status::Cited),
            d,
        )
        .build()
        .expect("a well-formed unit")
        .source
        .expect("stamp always writes a source")
    }

    #[test]
    fn the_reference_is_the_tid_and_the_locator() {
        let m = manifest(None);
        let d = drawn(&m, None);
        let s = unit(&d);
        assert_eq!(s.kind, SourceKind::Doc);
        assert_eq!(s.reference, format!("{}#Gen.1.1", d.tid.canonical()));
        // And without a locator it is the tid alone, with no trailing `#`.
        let bare = Drawn {
            locator: None,
            ..drawn(&m, None)
        };
        assert_eq!(unit(&bare).reference, d.tid.canonical());
    }

    /// Rule 2: the manifest's publication date reaches the unit byte for byte.
    #[test]
    fn a_passage_with_no_instant_copies_the_manifests_date() {
        let m = manifest(Some("1769"));
        let s = unit(&drawn(&m, None));
        assert_eq!(s.published.as_deref(), Some("1769"));
        assert_eq!(s.observed, None);
    }

    /// D-1's prohibition, asserted as behaviour: nothing else travels.
    ///
    /// The manifest here has a title and a creator, both of which a producer would be tempted
    /// to copy onto the unit "for convenience". They are reachable through the manifest, and a
    /// book's name inside ten thousand uids is not convenience.
    #[test]
    fn nothing_but_the_copy_set_travels() {
        let m = manifest(Some("1769"));
        let s = unit(&drawn(&m, None));
        assert!(!s.reference.contains("King James"));
        assert!(!s.reference.contains("Translators"));
        assert_eq!(
            s.captured, None,
            "`captured` is the operator's, not a part's"
        );
        assert!(s.extra.is_empty());
    }

    /// Rule 3 at second precision: **not** omitted, which is the whole point of the rule.
    #[test]
    fn a_second_wide_publication_date_is_not_omitted_beside_an_instant() {
        let t = 1726500000000u64;
        let m = manifest(Some("2024-09-16T15:20:00Z"));
        let s = unit(&drawn(&m, Some(t)));
        assert_eq!(s.observed, Some(t));
        assert_eq!(
            s.published.as_deref(),
            Some("2024-09-16T15:20:00Z"),
            "a one-second interval is not the millisecond inside it"
        );
        assert!(!omits("2024-09-16T15:20:00Z", t));
        // And the second holds even when the instant is the first millisecond of that second.
        assert!(!omits("2024-09-16T15:20:00Z", t));
    }

    /// Rule 3 at millisecond precision. EDTF level 1 cannot write this value, so the rule is
    /// exercised through `omits` on a one-millisecond interval directly — which is the only
    /// honest way to show that the comparison is an equality and not a tolerance.
    #[test]
    fn the_omission_rule_is_an_equality_between_intervals() {
        let t = 1726500000000i64;
        assert_eq!(
            edtf::to_interval(&edtf::parse("2024-09-16T15:20:00Z").expect("a date-time")),
            Interval::of(t, t + 1000)
        );
        assert_eq!(Interval::at(Instant(t)), Interval::of(t, t + 1));
        // The two differ by their width alone, and the rule compares widths as well as starts.
        assert_ne!(Interval::of(t, t + 1000), Interval::of(t, t + 1));
        // A year is wider still, and is never omitted either.
        assert!(!omits("2024", t as u64));
    }

    /// Rule 4: both present is permitted, and the producer's job is only not to invent one.
    #[test]
    fn both_fields_may_stand_together() {
        let m = manifest(Some("1769"));
        let s = unit(&drawn(&m, Some(1726500000000)));
        assert_eq!(s.published.as_deref(), Some("1769"));
        assert_eq!(s.observed, Some(1726500000000));
    }

    /// Rule 5: a part with no instant and a catalog entry with no date leaves the unit undated.
    #[test]
    fn neither_field_means_undated_rather_than_the_epoch() {
        let m = manifest(None);
        let s = unit(&drawn(&m, None));
        assert_eq!(s.published, None);
        assert_eq!(s.observed, None, "undated is not instant zero");
    }

    /// A malformed publication date is copied rather than repaired or dropped.
    #[test]
    fn a_malformed_date_is_copied_as_it_stands() {
        let m = manifest(Some("1769-13"));
        let s = unit(&drawn(&m, None));
        assert_eq!(s.published.as_deref(), Some("1769-13"));
        assert!(
            !omits("1769-13", 0),
            "with no interval there is nothing to compare, so nothing is omitted"
        );
    }

    /// The same passage stamped twice is the same unit, which is what makes an ingest replayable.
    #[test]
    fn stamping_is_a_function_of_its_inputs() {
        let m = manifest(Some("1769"));
        let d = drawn(&m, Some(5));
        assert_eq!(unit(&d), unit(&d));
    }
}
