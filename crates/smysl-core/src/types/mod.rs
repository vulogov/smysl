//! The kernel type system (§1, §2, §3, §4, §5).

pub mod aux;
pub mod epistemics;
pub mod provenance;
pub mod record;
pub mod relation;
pub mod thread;
pub mod unit;
pub mod view;

pub use aux::{
    Contention, ContentionStatus, Detected, DetectionKind, DropReason, LabelBinding, Optimality,
    PackInfo, PackMode, SchemaDecl,
};
pub use epistemics::{Date, Lod, SourceKind, SourceRef, Status};
pub use provenance::{Attestation, Hlc, Op, Rung};
pub use record::{code as record_code, Record};
pub use relation::{RelKind, Relation};
pub use thread::{Role, Step, Thread, ThreadSchema};
pub use unit::{Extra, Unit, UnitCore, UnitCoreBuilder};
pub use view::{Admission, Fidelity, GranularityProfile, View};

/// The bundled deterministic token estimator (D-2).
///
/// `tokens(t) = ceil(utf8_len(t) / 4)`. Packing must be pure (rule D), and provider
/// tokenisers are neither available offline nor stable, so budgets are approximate
/// against any specific model **by design** - and say so, via the estimator id recorded
/// in every `packinfo`.
///
/// Granularity bounds are expressed in tokens too, so this is the single place the two
/// meanings of "token" agree.
pub fn tokens(text: &str) -> u32 {
    (text.len() as u32).div_ceil(4)
}

/// Quantise to 1/1024 and round-trip through `binary32` (§7.1 constraint 4).
///
/// Doing this once, at the boundary, is what makes hash stability independent of the
/// float path that derived a value: two implementations that compute a weight slightly
/// differently still encode the same bytes.
pub fn quantise(v: f32) -> f32 {
    (v * 1024.0).round() / 1024.0
}

/// Whether `v` is already an exact multiple of 1/1024 representable in `binary32`.
/// The reader rejects anything else (`SMY-E081`).
pub fn is_quantised(v: f32) -> bool {
    v.is_finite() && (v * 1024.0).fract() == 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_estimator_is_the_documented_formula() {
        assert_eq!(tokens(""), 0);
        assert_eq!(tokens("a"), 1);
        assert_eq!(tokens("abcd"), 1);
        assert_eq!(tokens("abcde"), 2);
        // utf8_len, not char count.
        assert_eq!(tokens("\u{e9}"), 1);
        assert_eq!(tokens("\u{4f60}\u{597d}"), 2);
    }

    #[test]
    fn the_estimator_is_monotone() {
        let mut s = String::new();
        let mut prev = tokens(&s);
        for _ in 0..64 {
            s.push('x');
            let t = tokens(&s);
            assert!(t >= prev);
            prev = t;
        }
    }

    #[test]
    fn quantisation_snaps_to_1_1024() {
        assert_eq!(quantise(0.6), 614.0 / 1024.0);
        assert_eq!(quantise(0.0), 0.0);
        assert_eq!(quantise(1.0), 1.0);
        assert_eq!(quantise(0.5), 0.5);
    }

    #[test]
    fn quantisation_is_idempotent() {
        for v in [0.0f32, 0.1, 0.333, 0.6, 0.9999, 1.0] {
            let q = quantise(v);
            assert_eq!(quantise(q), q, "quantising twice must change nothing");
            assert!(is_quantised(q));
        }
    }

    /// This is the property that matters: values that differ by less than the quantum
    /// collapse to the same bytes, so a float path difference cannot change a uid.
    #[test]
    fn near_values_collapse_to_the_same_quantum() {
        let a = quantise(0.6);
        let b = quantise(0.6 + 1e-7);
        assert_eq!(a, b);
        assert_ne!(quantise(0.6), quantise(0.6 + 1.0 / 1024.0));
    }

    #[test]
    fn unquantised_values_are_detected() {
        assert!(!is_quantised(0.6));
        assert!(!is_quantised(0.1));
        assert!(is_quantised(614.0 / 1024.0));
        assert!(!is_quantised(f32::NAN));
        assert!(!is_quantised(f32::INFINITY));
    }

    #[test]
    fn every_multiple_of_the_quantum_is_quantised() {
        for i in 0..=1024u32 {
            let v = i as f32 / 1024.0;
            assert!(is_quantised(v), "{v} is a multiple of 1/1024");
            assert_eq!(quantise(v), v);
        }
    }
}
