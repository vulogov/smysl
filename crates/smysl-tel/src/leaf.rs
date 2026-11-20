//! Canonical leaf values (RFC SMYSL-3 §5.5): what `tel:from` and `tel:to` say, and how.
//!
//! A change unit carries a leaf's value as text read through a `vtype`, and the pair is inside
//! the unit's uid, so it has to be one pair per value:
//!
//! | `vtype` | text | when |
//! |---|---|---|
//! | `null`, `bool` | `null`, `true`, `false` | JSON literals |
//! | `int` | a decimal integer | a number the source format **types** as an integer |
//! | `dec` | the `dec` spelling | every other JSON number |
//! | `text` | the string, NFC | strings, and a number out of `dec`'s range (`SMY-W613`) |
//! | `json` | JCS with `dec` numbers | an unkeyed array or an opaque object |
//!
//! A JSON number is a `dec` unless the source *types* it as an integer, so `41` → `41.0`
//! between two snapshots is no change, and `41.2` and `41.20` from two exporters are one value.
//! Whether a field is integer-typed is the caller's to say: an OTLP `asInt`, a protobuf integer
//! field, or a Kubernetes field whose OpenAPI type is `integer`.

use core::fmt;

use smysl_core::Decimal;

use crate::jcs;
use crate::json::Json;

/// A leaf's value type (`tel:vtype`, §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VType {
    Null,
    Bool,
    Int,
    Dec,
    Text,
    Json,
}

impl VType {
    /// The spelling `x.tel/v1` gives it, one of `smysl_core::schemas::tel::VTYPES`.
    pub const fn as_str(self) -> &'static str {
        match self {
            VType::Null => "null",
            VType::Bool => "bool",
            VType::Int => "int",
            VType::Dec => "dec",
            VType::Text => "text",
            VType::Json => "json",
        }
    }
}

impl fmt::Display for VType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

/// One leaf's canonical value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Leaf {
    pub vtype: VType,
    pub text: String,
    /// A number in it was outside `dec`'s range and is kept as its token (`SMY-W613`): the
    /// value is still recorded, but two spellings of it are no longer one text.
    pub out_of_range: bool,
}

impl Leaf {
    /// The canonical value of `v`. `int_typed` says the source format types this field as an
    /// integer; it matters only for a number.
    pub fn of(v: &Json, int_typed: bool) -> Leaf {
        let plain = |vtype, text: &str| Leaf {
            vtype,
            text: text.to_string(),
            out_of_range: false,
        };
        match v {
            Json::Null => plain(VType::Null, "null"),
            Json::Bool(true) => plain(VType::Bool, "true"),
            Json::Bool(false) => plain(VType::Bool, "false"),
            Json::String(s) => plain(VType::Text, s),
            Json::Number(token) => match Decimal::parse_token(token) {
                // An integer-typed field holding an integral value: its `dec` spelling is
                // exactly the decimal integer §5.5 asks for, no exponent and no leading zero.
                Ok(d) if int_typed && d.is_integral() => plain(VType::Int, &d.to_string()),
                Ok(d) => plain(VType::Dec, &d.to_string()),
                Err(_) => Leaf {
                    vtype: VType::Text,
                    text: token.clone(),
                    out_of_range: true,
                },
            },
            Json::Array(_) | Json::Object(_) => {
                let (text, out_of_range) = jcs::canonical(v);
                Leaf {
                    vtype: VType::Json,
                    text,
                    out_of_range,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::parse;

    fn leaf(text: &str, int_typed: bool) -> (VType, String) {
        let l = Leaf::of(&parse(text).unwrap(), int_typed);
        (l.vtype, l.text)
    }

    #[test]
    fn the_table() {
        assert_eq!(leaf("null", false), (VType::Null, "null".into()));
        assert_eq!(leaf("true", false), (VType::Bool, "true".into()));
        assert_eq!(leaf("41.20", false), (VType::Dec, "41.2".into()));
        assert_eq!(leaf("3", true), (VType::Int, "3".into()));
        assert_eq!(leaf("-3", true), (VType::Int, "-3".into()));
        assert_eq!(leaf(r#""v1.2""#, false), (VType::Text, "v1.2".into()));
        assert_eq!(
            leaf(r#"["b", 1.0]"#, false),
            (VType::Json, r#"["b",1]"#.into())
        );
    }

    /// The reason the table exists: a value written two ways is one value.
    #[test]
    fn forty_one_and_forty_one_point_oh_are_no_change() {
        assert_eq!(leaf("41", false), leaf("41.0", false));
        assert_eq!(leaf("41.2", false), leaf("41.20", false));
        assert_eq!(leaf("1e3", true), leaf("1000", true));
    }

    /// An integer-typed field holding a fraction is not an integer, whatever the schema says:
    /// it is recorded as the number it is.
    #[test]
    fn a_fraction_in_an_integer_field_is_a_dec() {
        assert_eq!(leaf("2.5", true), (VType::Dec, "2.5".into()));
    }

    #[test]
    fn a_number_out_of_range_is_text_and_flagged() {
        let l = Leaf::of(&parse("1e999").unwrap(), false);
        assert_eq!(
            (l.vtype, l.text.as_str(), l.out_of_range),
            (VType::Text, "1e999", true)
        );
        assert!(Leaf::of(&parse("[1e999]").unwrap(), false).out_of_range);
    }

    #[test]
    fn every_vtype_is_one_the_schema_names() {
        for v in [
            VType::Null,
            VType::Bool,
            VType::Int,
            VType::Dec,
            VType::Text,
            VType::Json,
        ] {
            assert!(
                smysl_core::schemas::tel::VTYPES.contains(&v.as_str()),
                "{v}"
            );
        }
    }
}
