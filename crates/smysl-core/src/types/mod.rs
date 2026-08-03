//! The kernel type system (§1, §2, §3, §4, §5).

pub mod annex;
pub mod epistemics;
pub mod provenance;
pub mod record;
pub mod relation;
pub mod thread;
pub mod unit;
pub mod view;

pub use annex::{
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

/// Escape a string as a JSON value, quotes included.
///
/// Rust's `{:?}` is close enough to JSON to be tempting and wrong: it emits `\u{1}` for a
/// control character, which no JSON parser accepts. Machine-readable output that a machine
/// cannot read is worse than none, so the escaping is spelled out here and shared by every
/// caller that emits JSON.
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod json_tests {
    use super::json_escape;

    #[test]
    fn quotes_and_backslashes_are_escaped() {
        assert_eq!(json_escape(r#"a"b\c"#), r#""a\"b\\c""#);
    }

    /// The reason this exists rather than `{:?}`: Rust emits `\u{1}`, which no JSON parser
    /// accepts.
    #[test]
    fn control_characters_use_the_four_digit_form() {
        assert_eq!(json_escape("\u{1}"), r#""\u0001""#);
        assert_eq!(json_escape("\n\t\r"), r#""\n\t\r""#);
        assert_eq!(json_escape("\u{8}\u{c}"), r#""\b\f""#);
    }

    #[test]
    fn printable_unicode_passes_through() {
        assert_eq!(json_escape("p99 \u{2192} 410ms"), "\"p99 \u{2192} 410ms\"");
    }

    #[test]
    fn an_empty_string_is_a_pair_of_quotes() {
        assert_eq!(json_escape(""), "\"\"");
    }
}

/// Quantise to 1/1024 and round-trip through `binary32` (§7.1 constraint 4).
///
/// Doing this once, at the boundary, is what makes hash stability independent of the
/// float path that derived a value: two implementations that compute a weight slightly
/// differently still encode the same bytes.
pub fn quantise(v: f32) -> f32 {
    // Total by construction, because every caller treats the result as quantised and one of
    // them asserts it. `v * 1024.0` overflows `f32` for anything past about 3.3e35, so the
    // naive form returned *infinity* — which is not a multiple of 1/1024, is not finite, and
    // tripped `debug_assert!(is_quantised(q))` in the writer. In release the assertion is
    // compiled out and the infinity was written to the store instead, which is the worse
    // half: a value the codec's own contract forbids, emitted silently.
    //
    // Found by fuzzing the surface parser, through a payload float in a `.smy` file — author
    // data, so reachable from a document another agent hands you.
    //
    // Saturating rather than refusing keeps this a pure function that the hash path can rely
    // on. A magnitude this large cannot be represented under constraint 4 at all, so there is
    // no faithful value to preserve; `MAX_QUANTISED` is the largest one there is.
    if !v.is_finite() {
        return if v.is_nan() {
            0.0
        } else if v > 0.0 {
            MAX_QUANTISED
        } else {
            -MAX_QUANTISED
        };
    }
    let scaled = v * 1024.0;
    if !scaled.is_finite() {
        return if v > 0.0 {
            MAX_QUANTISED
        } else {
            -MAX_QUANTISED
        };
    }
    scaled.round() / 1024.0
}

/// The largest magnitude that survives `* 1024.0` in `binary32`, and so the largest value
/// constraint 4 can express.
pub const MAX_QUANTISED: f32 = f32::MAX / 1024.0;

/// Whether `v` is already an exact multiple of 1/1024 representable in `binary32`.
/// The reader rejects anything else (`SMY-E081`).
pub fn is_quantised(v: f32) -> bool {
    v.is_finite() && (v * 1024.0).fract() == 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `quantise` is total, and every caller depends on that: the CBOR writer asserts its
    /// result is quantised, and in release — where the assertion is gone — it writes whatever
    /// it was given. A large payload float used to make `v * 1024.0` overflow to infinity,
    /// which is neither finite nor a multiple of 1/1024. Found by fuzzing the surface parser.
    #[test]
    fn quantise_is_total_even_where_the_scale_overflows() {
        // `1e39` cannot be written as an `f32` literal — the compiler refuses it — but the
        // surface parser produces exactly that at runtime from a payload float, which is how
        // this reached the writer in the first place. Built rather than typed, for the same
        // reason.
        let overflowing: f32 = "1e39"
            .parse()
            .expect("parses to infinity, as the parser does");
        for v in [
            overflowing,
            -overflowing,
            3.4e38,
            f32::MAX,
            f32::MIN,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NAN,
        ] {
            let q = quantise(v);
            assert!(
                is_quantised(q),
                "quantise({v}) produced {q}, which the writer would refuse to encode"
            );
        }
    }

    /// And it still leaves ordinary values exactly where they were.
    #[test]
    fn quantise_does_not_disturb_a_value_it_can_represent() {
        for v in [0.0_f32, 0.5, 614.0 / 1024.0, -0.25, 1.0, 1000.0] {
            assert_eq!(quantise(v), (v * 1024.0).round() / 1024.0);
        }
    }

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

/// Normalise to NFC.
///
/// Every text field that reaches a hash is normalised here, on construction — the unit core's
/// `gist`, `body` and `detail`, and a `SourceRef`'s `reference`, which is inside the unit core
/// and therefore inside the uid.
///
/// The encoder normalises too, and that is not redundant. 0.6 found six free-text fields
/// reaching the encoder unchecked because the invariant lived only in "the constructors that
/// happen to be remembered", and §3 constraint 6 now says outright to normalise at the
/// encoder. This is the near side of the same rule: it keeps `PartialEq` agreeing with uid
/// equality, so two values that are the same unit compare equal in memory as well as hashing
/// alike. `tests/normalisation_scope.rs` pins both halves.
pub(crate) fn normalise(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    if unicode_normalization::is_nfc(s) {
        s.to_string()
    } else {
        s.nfc().collect()
    }
}
