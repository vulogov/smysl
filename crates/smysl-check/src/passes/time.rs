//! Pass 13 - time (RFC SMYSL-2.4 §4.3, A-12.2 rule E).
//!
//! Five things a store can say about time that are worth telling somebody, and the division
//! between them is the point: three are about **values** and two are about **consequences**.
//!
//! The value half reads records one at a time. An EDTF string that does not parse is
//! `SMY-E410`; one that parses and names an instant no millisecond counter can hold is
//! `SMY-W449`; an `observed` instant standing outside the `published` interval beside it is
//! `SMY-W411`. None of the three needs a second record to be found.
//!
//! The consequence half runs rule E and reports what it could not do. A live dating that moved
//! no bound is `SMY-W412` — because it is held by a lock, because it is less well evidenced
//! than the value it would have overridden, or because its target selects nothing. A set of
//! records whose times cannot all be right is `SMY-W413`, and every subject involved is
//! reported *contested*: nothing is chosen for it, because picking one of two contradictory
//! dates is an adjudication and this format does not adjudicate.
//!
//! # Nothing is skipped
//!
//! Unlike [`library`](crate::passes::library), this pass raises every code it is allocated. That is worth
//! stating rather than leaving to be noticed: `SMY-E404` and `SMY-W405` are the two codes of
//! TX-P3's neighbourhood that wait for `source.span` (TX-P5), and they belong to the library
//! pass, not to this one.
//!
//! # Why a malformed value is a finding and not a refusal
//!
//! A decoder does not reject a malformed `published`. The field is inside `UnitCore` and
//! therefore inside the uid, so a value that arrived has to leave again byte for byte or the
//! unit silently changes identity — and a reader that refused it would make a whole store
//! unopenable over one field, which is F-12's lesson. So the refusal is at the *writing* end
//! (the surface parser) and this is the reading end: a store that holds one is told about it.

use smysl_core::diag::{Code, Diagnostic, Report, Subject};
use smysl_core::edtf;
use smysl_core::types::{Axis, DatingTarget, DatingValue};
use smysl_graph::Store;
use smysl_text::time::engine::{effective, Unresolved};
use smysl_text::time::{edtf as extent, Instant, Subject as TimeSubject};

/// Run the pass.
pub fn run(store: &Store, report: &mut Report) {
    values(store, report);
    observed_outside_published(store, report);
    consequences(store, report);
}

/// `SMY-E410` and `SMY-W449`: every EDTF string a store holds, read on its own.
///
/// Three carriers, and all three are checked rather than only the unit field: a manifest's
/// `published` (key 8) is what units copy under A-2.4 rule 2, so a malformed one there becomes
/// a malformed one on every unit drawn from it, and finding it once at the catalog entry is
/// the difference between one diagnostic and ten thousand.
fn values(store: &Store, report: &mut Report) {
    for (uid, unit) in store.units() {
        let Some(published) = unit
            .core
            .source
            .as_ref()
            .and_then(|s| s.published.as_deref())
        else {
            continue;
        };
        judge(published, Subject::Unit(*uid), "`source.published`", report);
    }
    for (mid, m) in store.manifests() {
        let Some(published) = m.published.as_deref() else {
            continue;
        };
        judge(
            published,
            Subject::Store,
            &format!("manifest {mid} key 8 (`published`)"),
            report,
        );
    }
    for (did, dating) in store.datings() {
        let DatingValue::Absolute(text) = &dating.value else {
            continue;
        };
        judge(
            text,
            subject_of(&dating.target),
            &format!("dating {did}"),
            report,
        );
    }
}

/// One EDTF string: malformed, out of range, or fine.
///
/// The two outcomes are deliberately exclusive. A value that does not parse has no interval, so
/// asking whether its interval fits is meaningless — and reporting both would make a reader
/// think there were two problems with one string.
fn judge(text: &str, subject: Subject, carrier: &str, report: &mut Report) {
    match edtf::parse(text) {
        Err(e) => report.push(
            Diagnostic::new(Code::E410)
                .with_subject(subject)
                .with_message(format!("{carrier}: {e}")),
        ),
        Ok(v) if !extent::fits(&v) => report.push(
            Diagnostic::new(Code::W449)
                .with_subject(subject)
                .with_message(format!(
                    "{carrier}: `{text}` is outside the millisecond instant range, so it is \
                     ordered as an open bound rather than clamped to one"
                )),
        ),
        Ok(_) => {}
    }
}

/// `SMY-W411`: an `observed` instant outside the `published` interval beside it.
///
/// A-2.4 rule 4 permits both fields on one unit, and this is the case where permitting them is
/// not the same as believing them: the two answer different questions — when the manifestation
/// was published, and when this reading was taken — and a reading taken before its own source
/// was published is a pair of records that cannot both be right. Reported rather than refused,
/// because which of the two is wrong is not a thing a checker can know.
fn observed_outside_published(store: &Store, report: &mut Report) {
    for (uid, unit) in store.units() {
        let Some(source) = unit.core.source.as_ref() else {
            continue;
        };
        let (Some(observed), Some(published)) = (source.observed, source.published.as_deref())
        else {
            continue;
        };
        let Ok(v) = edtf::parse(published) else {
            // Malformed is `SMY-E410`'s to say, and saying it twice about one string would
            // make a reader look for two defects.
            continue;
        };
        let Ok(ms) = i64::try_from(observed) else {
            continue;
        };
        let interval = extent::to_interval(&v);
        if interval.is_undated() || interval.contains(Instant(ms)) {
            continue;
        }
        report.push(
            Diagnostic::new(Code::W411)
                .with_subject(Subject::Unit(*uid))
                .with_message(format!(
                    "observed {} lies outside the published interval {interval} \
                     (`{published}`); the two answer different questions and one of them is wrong",
                    Instant(ms)
                )),
        );
    }
}

/// `SMY-W412`, `SMY-W413` and `SMY-E060`: what rule E could not do, per axis.
///
/// All three axes, because a dating on *composed* that cannot apply is as much a finding as one
/// on *said*, and the axis is in the message so a reader knows which graph to look at.
fn consequences(store: &Store, report: &mut Report) {
    for axis in Axis::ALL.iter().copied() {
        let e = effective(store, axis);
        for c in &e.inconsistent {
            let datings = if c.datings.is_empty() {
                "no dating".to_string()
            } else {
                format!(
                    "dating(s) {}",
                    c.datings
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            let free = if c.free.is_empty() {
                String::new()
            } else {
                format!(
                    " and free constraint(s) {}",
                    c.free
                        .iter()
                        .map(|f| f.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            report.push(
                Diagnostic::new(Code::W413)
                    .with_subject(subject_of_time(&c.over))
                    .with_message(format!(
                        "{axis}: the times over {} cannot all be right — {datings}{free}; \
                         nothing is chosen and it is reported contested ({})",
                        c.over, c.id
                    )),
            );
        }
        for c in &e.not_applied {
            report.push(
                Diagnostic::new(Code::W412)
                    .with_subject(subject_of_time(&c.over))
                    .with_message(format!(
                        "{axis}: a dating of {} is live and moved no bound — it is less well \
                         evidenced than the value it would have overridden ({})",
                        c.over, c.id
                    )),
            );
        }
        for did in &e.held {
            report.push(
                Diagnostic::new(Code::W412)
                    .with_subject(Subject::Store)
                    .with_message(format!(
                        "{axis}: dating {did} is held by a canonical commitment and was not \
                         applied; a resolution naming the derived contention releases it"
                    )),
            );
        }
        for (did, why) in &e.unresolved {
            match why {
                // A target that names nothing is a dangling reference, which is what
                // `SMY-E060` has always been for — and a dating is the first record type whose
                // references the integrity pass cannot see, because a dating moves no edge and
                // is therefore not in the adjacency.
                Unresolved::TargetMissing => report.push(
                    Diagnostic::new(Code::E060)
                        .with_subject(Subject::Store)
                        .with_message(format!(
                            "{axis}: dating {did} names a target this store does not hold"
                        )),
                ),
                // A window over a part whose units are not here yet, or none of whose instants
                // fall inside it. Not an error: the records it would select may arrive later,
                // and a dating written ahead of an ingest is an ordinary thing to do.
                Unresolved::SelectedNothing => report.push(
                    Diagnostic::new(Code::W412)
                        .with_subject(Subject::Store)
                        .with_message(format!(
                            "{axis}: dating {did} is live and selects no unit in this store"
                        )),
                ),
            }
        }
    }
}

/// The subject a dating's target names, where the diagnostic can carry one.
///
/// Only a unit target gives a `Subject::Unit`. A tid or a mid is not a uid — the identities are
/// domain-separated on purpose (§2.6) — so a diagnostic about one is whole-store and names the
/// identity in its message, which is what `SMY-W418` already does for a manifest fork.
fn subject_of(target: &DatingTarget) -> Subject {
    match target {
        DatingTarget::Unit(u) => Subject::Unit(*u),
        _ => Subject::Store,
    }
}

fn subject_of_time(s: &TimeSubject) -> Subject {
    match s {
        TimeSubject::Unit(u) => Subject::Unit(*u),
        TimeSubject::Manifest(_) => Subject::Store,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::ids::AgentId;
    use smysl_core::types::provenance::Hlc;
    use smysl_core::types::{
        Attestation, Commit, Commitment, Dating, Record, SourceKind, SourceRef, Status,
        UnitCoreBuilder,
    };
    use smysl_core::{canonical_uid, Did, KernelType, Op, Rung};

    fn agent() -> AgentId {
        AgentId::new("human:vu").expect("an agent")
    }

    fn store_of(records: Vec<Record>) -> Store {
        let mut s = Store::new();
        s.append(&records).expect("records a store accepts");
        s
    }

    fn run_on(records: Vec<Record>) -> Report {
        let mut r = Report::new();
        run(&store_of(records), &mut r);
        r
    }

    /// A unit with a source, built to order.
    fn unit(published: Option<&str>, observed: Option<u64>, status: Status) -> Record {
        let mut source = SourceRef::new(SourceKind::Doc, "a book");
        source.published = published.map(str::to_string);
        source.observed = observed;
        Record::Unit(
            UnitCoreBuilder::new(
                KernelType::Claim,
                "a claim long enough to clear the gist length check",
                status,
            )
            .source(source)
            .build()
            .expect("a well-formed unit"),
        )
    }

    /// `SMY-E410` cannot be reached from a surface document, and that is why it is here.
    ///
    /// The surface parser refuses a malformed EDTF value as it reads it, so no document can
    /// produce a store that holds one — which is the whole point of refusing at the writing
    /// end. The only way a store gets one is CBOR from an implementation that wrote it, and
    /// the only way to test for that is to build the store. So there is no `.smy` fixture for
    /// this code, and the absence is a property of the design rather than a gap in the suite.
    #[test]
    fn a_malformed_value_is_e410_wherever_it_is_carried() {
        // On a unit.
        let r = run_on(vec![unit(Some("1984-13"), None, Status::Cited)]);
        assert_eq!(r.count(Code::E410), 1);
        assert_eq!(
            r.count(Code::W449),
            0,
            "a value with no interval has no range"
        );

        // On a manifest, where finding it once is worth ten thousand times.
        let mut m = smysl_core::types::Manifest::new(
            "kjv/1769",
            smysl_core::LangTag::new("en").expect("a tag"),
            "osis/1",
            "public-domain",
            "top/1Ki-4Mi",
        );
        m.published = Some("1769-02-30".to_string());
        let r = run_on(vec![Record::Manifest(m)]);
        assert_eq!(r.count(Code::E410), 1);

        // And on a dating.
        let u = unit(None, None, Status::Speculative);
        let Record::Unit(core) = &u else {
            panic!("a unit")
        };
        let uid = canonical_uid(core);
        let d = Dating::new(
            smysl_core::types::DatingTarget::Unit(uid),
            Axis::Said,
            DatingValue::Absolute("not a date".to_string()),
            agent(),
            Hlc::new(1, 0, agent()),
        );
        let r = run_on(vec![u, Record::Dating(d)]);
        assert_eq!(r.count(Code::E410), 1);
    }

    /// `SMY-W449`: a value that parses and names no instant this counter can hold.
    #[test]
    fn a_value_outside_the_instant_range_is_w449_and_not_an_error() {
        let r = run_on(vec![unit(Some("Y300000000"), None, Status::Cited)]);
        assert_eq!(r.count(Code::W449), 1);
        assert_eq!(r.count(Code::E410), 0);
        // EDTF's own example of a long year is *inside* the range, so it says nothing.
        let r = run_on(vec![unit(Some("Y170000002"), None, Status::Cited)]);
        assert_eq!(r.count(Code::W449), 0);
    }

    /// `SMY-W411`: outside is reported, inside is not, and the edges are where they claim.
    #[test]
    fn an_observed_instant_is_judged_against_the_published_interval() {
        // 1984-01-01T00:00:00Z, and the first instant of 1985.
        let start_1984 = 441_763_200_000u64;
        let start_1985 = 473_385_600_000u64;
        for (observed, want, why) in [
            (
                start_1984,
                0,
                "the first instant of the interval is inside it",
            ),
            (start_1985 - 1, 0, "and so is the last"),
            (start_1985, 1, "the first instant after it is not"),
            (0, 1, "nor is one fourteen years earlier"),
        ] {
            let r = run_on(vec![unit(Some("1984"), Some(observed), Status::Cited)]);
            assert_eq!(r.count(Code::W411), want, "{why}");
        }
        // A malformed `published` is `SMY-E410`'s to report, once.
        let r = run_on(vec![unit(Some("1984-13"), Some(0), Status::Cited)]);
        assert_eq!(
            r.count(Code::W411),
            0,
            "there is no interval to be outside of"
        );
        assert_eq!(r.count(Code::E410), 1);
    }

    /// `SMY-W412`: a dating less well evidenced than the value it would have overridden.
    ///
    /// The case the rule exists for, and the one no surface document can build: `measured`
    /// needs an attestation at rung `computed`, and surface text writes no attestations.
    #[test]
    fn a_dating_that_cannot_outrank_what_it_would_override_is_w412() {
        let u = unit(None, Some(1_726_500_000_000), Status::Measured);
        let Record::Unit(core) = &u else {
            panic!("a unit")
        };
        let uid = canonical_uid(core);
        let att = Attestation::new(
            uid,
            agent(),
            Op::Imported,
            Rung::Computed,
            Hlc::new(1_726_500_001_000, 0, agent()),
        );
        // A `speculative` dating — no basis — narrowing a measured instant to 1999.
        let d = Dating::new(
            smysl_core::types::DatingTarget::Unit(uid),
            Axis::Said,
            DatingValue::Absolute("1999".to_string()),
            agent(),
            Hlc::new(2, 0, agent()),
        );
        let r = run_on(vec![u, Record::Attestation(att), Record::Dating(d)]);
        assert!(
            r.count(Code::W412) + r.count(Code::W413) >= 1,
            "a dating that cannot apply must be reported: {:?}",
            r.iter().map(|d| d.code).collect::<Vec<_>>()
        );
    }

    /// `SMY-W412`: a dating a `canonical` commitment is holding for review.
    ///
    /// The commitment names the dating by its **did**, which A-6 permits in key 0 and which
    /// the domain bytes of §2.6 keep from colliding with a uid. Built here rather than in a
    /// document because the surface grammar for `@commit` takes a uid or a label and has no
    /// spelling for a did — which is a legibility gap in the surface form, not in the wire.
    #[test]
    fn a_dating_held_by_a_canonical_commitment_is_w412() {
        let u = unit(None, Some(1_726_500_000_000), Status::Cited);
        let Record::Unit(core) = &u else {
            panic!("a unit")
        };
        let uid = canonical_uid(core);
        let d = Dating::new(
            smysl_core::types::DatingTarget::Unit(uid),
            Axis::Said,
            DatingValue::Absolute("1984".to_string()),
            agent(),
            Hlc::new(2, 0, agent()),
        );
        let did: Did = d.did();
        let lock = Commit::new(
            smysl_core::Uid::from_bytes(*did.as_bytes()),
            Commitment::Canonical,
            agent(),
            Hlc::new(3, 0, agent()),
        );
        let r = run_on(vec![u, Record::Dating(d), Record::Commit(lock)]);
        // Once, not once per axis: a dating has one axis (key 1), and the pass runs rule E
        // three times over three different graphs. A dating on *said* does not exist on the
        // other two, which is why the engine filters by axis before it asks about liveness.
        assert_eq!(r.count(Code::W412), 1);
        assert!(r
            .iter()
            .any(|x| x.code == Code::W412 && x.message.contains("held by a canonical commitment")));
    }

    /// A store with nothing wrong with its times says nothing at all.
    ///
    /// The control every pass needs: a pass that reports on a clean store is worse than one
    /// that misses a defect, because the next reader learns to ignore it.
    #[test]
    fn a_clean_store_reports_nothing() {
        let u = unit(Some("1984"), None, Status::Cited);
        let v = unit(None, Some(441_763_200_001), Status::Cited);
        let r = run_on(vec![u, v]);
        assert_eq!(r.len(), 0, "{:?}", r.iter().collect::<Vec<_>>());
    }
}
