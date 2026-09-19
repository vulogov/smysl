//! Pass 11 — commitment support (1.7).
//!
//! Rule M's shape on a second axis. Rule M says a unit may not claim more *truth* than its weakest
//! ground supports; this says a unit may not be more *settled* than the weakest thing it rests on.
//! A `Canonical` scene built on a `Floated` premise is the fiction analogue of a measured claim
//! resting on a guess — the canonical-scene-built-on-sand case, surfaced rather than hoped for.
//!
//! **A warning, not an error**, and the difference matters. Rule M is a claim about the world being
//! wrong. Outrunning your own foundations is a normal intermediate state of a draft: you commit to
//! the ending before you have settled how you get there, and a gate that refused it would make the
//! ledger unusable during the work it exists to support. A consumer that wants it fatal has
//! `--strict`.
//!
//! Silence is not `Floated`. A unit nobody has committed to carries no commitment at all, and is
//! skipped: the pass reports units whose author said something, not units whose author has not
//! got to them yet.

use smysl_core::diag::{Code, Diagnostic, Report};
use smysl_core::Commitment;
use smysl_graph::Store;

pub fn run(store: &Store, report: &mut Report) {
    for (uid, unit) in store.units() {
        let Some(level) = store.commitment_of(uid) else {
            continue;
        };
        // `Retconned` is the one level that says "this is no longer load-bearing", so resting on
        // something weaker is not a defect — it is what a retcon looks like on the way out.
        if level == Commitment::Retconned {
            continue;
        }
        let weakest = unit
            .core
            .grounds
            .iter()
            .filter_map(|g| store.commitment_of(g))
            .min();
        if let Some(floor) = weakest {
            if level > floor {
                report.push(Diagnostic::on(Code::W057, *uid).with_message(format!(
                    "committed {level} on grounds no more settled than {floor}"
                )));
            }
        }
    }
}
