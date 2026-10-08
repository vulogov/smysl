//! The four library identities, and the reference form that carries one of them.
//!
//! The types are `smysl-core`'s (SMYSL-2.3 A-3, D-8: a second implementation computes them
//! from the wire, so they cannot live in the crate that reads files). What this module adds is
//! the two things that need a normalised value or a locator, neither of which `smysl-core`
//! has: [`tid`], which is the only way a tid should ever be computed, and the
//! `t3:<52 chars>[#<locator>]` reference form.

use smysl_core::error::LibError;
use smysl_core::ids::Tid;

pub use smysl_core::ids::{Did, Mid, Rdid};

use crate::locator::{self, Locator};
use crate::norm::Normalised;

/// The identity of normalised text as a part.
///
/// The one function in this workspace that should compute a tid. `Tid::from_normalised_bytes`
/// takes bytes, because the decoder has to build a part text out of whatever arrived — so the
/// guarantee that a tid is over *normalised* bytes is this signature, and
/// `tests/norm_is_the_only_gate.rs` is what keeps the guarantee from being bypassed by
/// accident.
pub fn tid(text: &Normalised) -> Tid {
    text.tid()
}

/// A reference to a part, optionally to a place in it: `t3:<52 chars>[#<locator>]` (A-5).
pub fn reference(tid: &Tid, at: Option<&Locator>) -> String {
    match at {
        Some(l) => format!("{}#{l}", tid.canonical()),
        None => tid.canonical(),
    }
}

/// Split a reference back into its part and its place.
///
/// The locator is parsed, not just carried: a reference holding an unparseable locator is a
/// reference nothing can resolve, and the place to find that out is where the string is read
/// rather than three layers later when a span check reports a disagreement (`SMY-W405`) it
/// cannot explain.
pub fn parse_reference(s: &str) -> Result<(Tid, Option<Locator>), LibError> {
    let (id, loc) = match s.split_once('#') {
        Some((id, loc)) => (id, Some(loc)),
        None => (s, None),
    };
    let tid = Tid::parse(id).map_err(|_| LibError::BadAlias {
        alias: id.to_string(),
    })?;
    match loc {
        Some(l) => {
            let parsed = locator::parse(l).map_err(|_| LibError::BadAlias {
                alias: l.to_string(),
            })?;
            Ok((tid, Some(parsed)))
        }
        None => Ok((tid, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_tid() -> Tid {
        Normalised::of("In the beginning\n").tid()
    }

    #[test]
    fn a_reference_round_trips_with_and_without_a_place() {
        let t = a_tid();
        let bare = reference(&t, None);
        assert_eq!(parse_reference(&bare).unwrap(), (t, None));

        let l = locator::parse("Gen.1.1").unwrap();
        let with = reference(&t, Some(&l));
        assert!(with.starts_with("t3:"));
        assert_eq!(parse_reference(&with).unwrap(), (t, Some(l)));
    }

    #[test]
    fn a_reference_with_an_unreadable_locator_is_refused_where_it_is_read() {
        let t = a_tid();
        let s = format!("{}#Gen.01", t.canonical());
        assert!(parse_reference(&s).is_err());
    }

    #[test]
    fn a_reference_that_is_not_a_tid_is_refused() {
        assert!(parse_reference("b3:not-a-tid").is_err());
        assert!(parse_reference("").is_err());
    }
}
