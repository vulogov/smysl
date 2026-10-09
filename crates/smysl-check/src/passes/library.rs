//! Pass 12 - the library (RFC SMYSL-2.4 §4.3.3).
//!
//! Four things a store can say about texts that cannot be true at once: a reference to a part,
//! a manifest, a dating or a reading that is not a well-formed identity; one alias with two
//! heads; text in a log; and an object whose bytes are not what its name says they are.
//!
//! # Two of the plan's codes are not here
//!
//! TX-P1 step 5 names six. `SMY-E404` is a span past a part's length and `SMY-W405` is a
//! locator disagreeing with a span, and both read `SourceRef.span` — a field §4.3.1 gives to
//! TX-P5. They are not implemented *and not registered*: a code in the registry that nothing
//! can raise is a promise a reader greps for and finds nothing behind. [`SKIPPED`] is what the
//! pass says about them instead, so "this build does not check that" is a thing a caller can
//! read rather than something they have to infer from silence.
//!
//! # Why the object half is optional
//!
//! `SMY-E446` and `SMY-E401` are about bytes, and a `Store` holds no bytes: a log never holds
//! a record 15 or 18 (OQ-39), so the text behind a manifest is in an object store that `check`
//! may or may not have been handed. Without a resolver those two are skipped and the report
//! says which, exactly as `Pass::IMPLEMENTED` reports a pass this build does not run. The
//! alternative — reporting a manifest's parts as unverifiable — would make every `check` of a
//! store without a library say something is wrong when nothing is.

use smysl_core::diag::{Code, Diagnostic, Report, Subject};
use smysl_core::ids::{Did, Mid, Rdid, Tid};
use smysl_core::types::{PartResolver, Resolved, SourceRef};
use smysl_core::Record;
use smysl_graph::Store;

/// The codes this pass is allocated and does not raise, with the reason.
///
/// Public so a caller can print it rather than re-deriving the sentence, and a constant rather
/// than a doc comment so that a test can pin it.
///
/// **The codes are text, not [`Code`]**, and that is not a shortcut: they are not in the
/// registry, because nothing in this build can raise them, and a `Code` for each would be the
/// very entry the registry's own rule forbids. So this list is the only place they are named,
/// and it stops being a list of strings on the day they become raisable.
pub const SKIPPED: &[(&str, &str)] = &[
    (
        "SMY-E404",
        "a span past the part's length; `source.span` arrives in TX-P5",
    ),
    (
        "SMY-W405",
        "a locator disagreeing with a span; `source.span` arrives in TX-P5",
    ),
];

/// Run the pass. `parts` is the object store, when the caller has one.
pub fn run(
    store: &Store,
    parts: Option<&(dyn PartResolver + Send + Sync + core::panic::RefUnwindSafe)>,
    report: &mut Report,
) {
    malformed_identities(store, report);
    forks(store, report);
    text_in_the_log(store, report);
    if let Some(parts) = parts {
        objects(store, parts, report);
    }
}

/// `SMY-E403` - a reference that names an identity and is not one.
///
/// The four identities have text prefixes, so a reference either claims to be one or does not.
/// One that claims to be and is not is worse than a reference to nothing: a dangling reference
/// is `SMY-E060` and says what it is, while `t3:` followed by something shorter than 52
/// characters is an *abbreviation*, and `Tid::parse` refuses those for the reason
/// `Uid::parse` does — an abbreviated identity in a record weakens identity silently, and the
/// store's by-part index would simply not find the part.
///
/// A locator is allowed after `#` and is not checked here: it is `smysl-text`'s grammar, it
/// needs the reading to validate against, and §4.3.3 gives that to `SMY-W405`.
///
/// `source.manifest` is the other carrier §4.3.3 names. It is `Option<Mid>` from TX-P5, so it
/// cannot be malformed by then and cannot exist now; the reference is the whole of it here.
fn malformed_identities(store: &Store, report: &mut Report) {
    for (uid, unit) in store.units() {
        if let Some(source) = unit.core.source.as_ref() {
            if let Some(claimed) = ill_formed(source) {
                report.push(
                    Diagnostic::on(Code::E403, *uid)
                        .with_message(format!(
                            "`{claimed}` claims to be {} and is not one",
                            kind_of(&claimed)
                        ))
                        .with_suggestion(
                            "an identity is its prefix and 52 base32 characters; \
                             `canonical()` writes one and the short form is not one"
                                .to_string(),
                        ),
                );
            }
        }
    }
    // A manifest's `origin` is a `SourceRef` too, and a manifest that recorded where its text
    // came from with a malformed identity is the same defect with no unit to hang it on.
    for (mid, manifest) in store.manifests() {
        if let Some(claimed) = manifest.origin.as_ref().and_then(ill_formed) {
            report.push(Diagnostic::new(Code::E403).with_message(format!(
                "the origin of manifest `{}` ({}) names `{claimed}`, which claims to be {} \
                 and is not one",
                manifest.alias,
                mid.canonical(),
                kind_of(&claimed)
            )));
        }
    }
}

/// The reference, when it claims an identity it does not have.
fn ill_formed(source: &SourceRef) -> Option<String> {
    let head = match source.reference.split_once('#') {
        Some((before, _)) => before,
        None => source.reference.as_str(),
    };
    let well_formed = if head.starts_with(Tid::PREFIX) {
        Tid::parse(head).is_ok()
    } else if head.starts_with(Mid::PREFIX) {
        Mid::parse(head).is_ok()
    } else if head.starts_with(Did::PREFIX) {
        Did::parse(head).is_ok()
    } else if head.starts_with(Rdid::PREFIX) {
        Rdid::parse(head).is_ok()
    } else {
        // Claims nothing. Whether it points at anything is `SMY-E060`'s question.
        true
    };
    (!well_formed).then(|| head.to_string())
}

fn kind_of(claimed: &str) -> &'static str {
    if claimed.starts_with(Tid::PREFIX) {
        "a part identity"
    } else if claimed.starts_with(Mid::PREFIX) {
        "a manifest identity"
    } else if claimed.starts_with(Did::PREFIX) {
        "a dating identity"
    } else {
        "a reading identity"
    }
}

/// `SMY-W418` - one alias with two heads.
///
/// A warning, and reported rather than recorded. Two manifests for `kjv/1769` neither of which
/// supersedes the other is a fact about a corpus — usually two people cataloguing the same book
/// without having met — and the store returns both heads rather than picking one, because
/// picking one here would make the fact unreportable. Recording it would be worse: the next
/// manifest to arrive may be the one that supersedes one of them, and then the record would be
/// a stale finding with a hash chain behind it. The same argument keeps a contention a
/// detection rather than a record (§5.4).
fn forks(store: &Store, report: &mut Report) {
    for alias in store.aliases() {
        let heads = store.heads(alias);
        if heads.len() > 1 {
            let names: Vec<String> = heads.iter().map(Mid::canonical).collect();
            report.push(
                Diagnostic::new(Code::W418)
                    .with_message(format!(
                        "`{alias}` has {} heads: {}",
                        heads.len(),
                        names.join(", ")
                    ))
                    .with_suggestion(
                        "a manifest naming one of them in `supersedes` joins them into a \
                         chain; until then both are current"
                            .to_string(),
                    ),
            );
        }
    }
}

/// `SMY-E452` - a record 15 or 18 in a log (OQ-39).
///
/// `Store::append` refuses one, so nothing this build writes can produce this. A store built
/// from records does not go through `append` — `from_records` and `open` absorb directly, and
/// deliberately, because F-12's lesson is that one bad record must not stop `Store::open`. So
/// a log written by a producer that did not refuse it decodes, and this is what says so.
///
/// One diagnostic per record rather than one for the store: each names a tid, and the tid is
/// what an operator needs to move the bytes into the object store where they belong.
fn text_in_the_log(store: &Store, report: &mut Report) {
    for record in store.iter() {
        let (which, id) = match record {
            Record::PartText(p) => ("a part text (record 15)", p.tid.canonical()),
            Record::PartReading(r) => ("a reading (record 18)", r.rdid().canonical()),
            _ => continue,
        };
        report.push(
            Diagnostic::new(Code::E452)
                .with_message(format!("the log holds {which} for `{id}`"))
                .with_suggestion(
                    "text lives in the library's object store; a log that holds it cannot \
                     honour a redaction without rewriting its own hash chain"
                        .to_string(),
                ),
        );
    }
}

/// `SMY-E446` and `SMY-E401` - the bytes behind a manifest's parts.
///
/// Absence is not a defect: a manifest is a catalog entry and may name parts whose text this
/// library does not hold, which is what `carry: ref` is for. What is a defect is bytes that
/// are there and are not what the name says.
///
/// `SMY-E401` comes free once the bytes are verified and nothing else in `check` can raise it.
/// A part entry records the part's length, and the object is the part; if the object hashes to
/// its tid and the lengths disagree, the manifest's own catalog entry is wrong about the text
/// it names. §4.3.3's list does not mention it — the code was allocated for the structure hash
/// and the rdid, which need a reading to compare — but "does not match the part entry on
/// re-read" is exactly what this is, and C-Library forbids it, so leaving it unraisable would
/// have made that row of the conformance table decorative.
fn objects(
    store: &Store,
    parts: &(dyn PartResolver + Send + Sync + core::panic::RefUnwindSafe),
    report: &mut Report,
) {
    for (mid, manifest) in store.manifests() {
        for entry in &manifest.parts {
            let where_ = || {
                format!(
                    "part `{}` of `{}` ({})",
                    entry.tid.canonical(),
                    manifest.alias,
                    mid.canonical()
                )
            };
            match parts.part(&entry.tid) {
                Resolved::Absent => {}
                Resolved::Unreadable => {
                    report.push(Diagnostic::new(Code::E446).with_message(format!(
                        "{} is stored and is not a readable part record",
                        where_()
                    )));
                }
                Resolved::Part(stored) => {
                    if stored.tid != entry.tid {
                        report.push(Diagnostic::new(Code::E446).with_message(format!(
                            "{} is stored under that identity and claims `{}`",
                            where_(),
                            stored.tid.canonical()
                        )));
                    } else if !stored.verify() {
                        report.push(
                            Diagnostic::new(Code::E446)
                                .with_message(format!("{} does not hash to its identity", where_()))
                                .with_suggestion(
                                    "the bytes are not the ones that were stored; restore the \
                                     object or re-add the text"
                                        .to_string(),
                                ),
                        );
                    } else if stored.text.len() as u64 != entry.length {
                        report.push(
                            Diagnostic::new(Code::E401)
                                .with_subject(Subject::Store)
                                .with_message(format!(
                                    "{} records {} byte(s) and the object holds {}",
                                    where_(),
                                    entry.length,
                                    stored.text.len()
                                )),
                        );
                    }
                }
            }
        }
    }
}
