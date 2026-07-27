//! Connective selection (§20).
//!
//! Deterministic template lookup keyed by the relation kind joining a block to the one
//! before it, with the variant chosen by `uid[0] % variants.len()`.
//!
//! Seeding on the uid rather than on a counter is what makes the choice stable: inserting a
//! block earlier in the thread does not reword every connective after it, and two renders
//! of the same graph produce the same prose. The alternative - asking a model for a
//! transition - would make the artifact unreproducible for the sake of variety no reader
//! asked for.

use smysl_core::{RelKind, Uid};

/// The template table of §20. An empty variant means "no connective at all", which is a
/// real choice: `elaborates` frequently reads better with nothing in front of it.
static TABLE: &[(RelKind, &[&str])] = &[
    (RelKind::Contrasts, &["However, ", "By contrast, "]),
    (RelKind::Concedes, &["Admittedly, ", "Granted, "]),
    (RelKind::Causes, &["Consequently, ", "As a result, "]),
    (RelKind::Elaborates, &["", "More precisely, "]),
    (RelKind::Exemplifies, &["For example, "]),
    (RelKind::Conditions, &["Provided that "]),
    (RelKind::Enables, &["This makes it possible that "]),
    (RelKind::Sequences, &["Then, ", "Next, "]),
    (RelKind::Answers, &["In answer, "]),
    (RelKind::Rebuts, &["Against this, ", "On the other hand, "]),
    (RelKind::Warrant, &["The warrant is that "]),
    (RelKind::Backs, &["This is backed by "]),
    (RelKind::Supersedes, &["Superseding this, "]),
    (RelKind::Retracts, &["Retracting this, "]),
];

/// The connective for an edge of `kind` leading to `uid`, or `None` when the kind has no
/// template.
///
/// The empty string is a deliberate variant, not a missing one, so it is returned as
/// `Some("")` and the caller decides whether an empty connective is worth a space.
pub fn select(kind: &RelKind, uid: &Uid) -> Option<&'static str> {
    let variants = TABLE.iter().find(|(k, _)| k == kind).map(|(_, v)| *v)?;
    let seed = uid.as_bytes()[0] as usize;
    Some(variants[seed % variants.len()])
}

/// Every kind the table knows.
pub fn kinds() -> Vec<RelKind> {
    TABLE.iter().map(|(k, _)| k.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid(first: u8) -> Uid {
        let mut b = [0u8; 32];
        b[0] = first;
        Uid::from_bytes(b)
    }

    #[test]
    fn the_rfc_table_is_present() {
        assert_eq!(
            select(&RelKind::Exemplifies, &uid(0)),
            Some("For example, ")
        );
        assert_eq!(
            select(&RelKind::Conditions, &uid(7)),
            Some("Provided that ")
        );
        assert!(select(&RelKind::Contrasts, &uid(0))
            .is_some_and(|c| c == "However, " || c == "By contrast, "));
    }

    #[test]
    fn every_kernel_kind_has_a_template() {
        for k in RelKind::KERNEL {
            assert!(select(k, &uid(0)).is_some(), "{k} has no connective");
        }
    }

    /// An extension kind is not in the table and must not be guessed at. Rule X says an
    /// unknown kind is treated as `elaborates` for *closure*; inventing prose for it is a
    /// different thing entirely.
    #[test]
    fn an_extension_kind_has_no_connective() {
        let k = RelKind::Extension("x.sre/triggers".into());
        assert_eq!(select(&k, &uid(0)), None);
    }

    /// The whole point of seeding on the uid: the same edge always reads the same way.
    #[test]
    fn selection_is_a_function_of_the_uid() {
        for byte in 0..=255u8 {
            let u = uid(byte);
            assert_eq!(select(&RelKind::Causes, &u), select(&RelKind::Causes, &u));
        }
    }

    /// ...and different uids do reach different variants, or the seeding would be
    /// decoration rather than variation.
    #[test]
    fn different_uids_reach_every_variant() {
        let seen: std::collections::BTreeSet<&str> = (0..=255u8)
            .filter_map(|b| select(&RelKind::Contrasts, &uid(b)))
            .collect();
        assert_eq!(seen.len(), 2, "both contrasts variants should be reachable");
    }

    #[test]
    fn elaborates_may_render_as_nothing_at_all() {
        let seen: std::collections::BTreeSet<&str> = (0..=255u8)
            .filter_map(|b| select(&RelKind::Elaborates, &uid(b)))
            .collect();
        assert!(seen.contains(""), "the empty variant is a real choice");
    }

    #[test]
    fn the_table_has_no_duplicate_kinds() {
        let mut seen = std::collections::BTreeSet::new();
        for k in kinds() {
            assert!(seen.insert(k.to_string()), "duplicate kind in the table");
        }
    }

    #[test]
    fn no_variant_list_is_empty() {
        for (k, v) in TABLE {
            assert!(!v.is_empty(), "{k} has an empty variant list");
        }
    }
}
