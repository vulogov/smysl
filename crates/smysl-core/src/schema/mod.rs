//! Schema evolution: FC-9 decoding, schema sets, and the lens engine (RFC SMYSL-2.5 §3.8–§3.10,
//! SMYSL-2.3 A-7, A-12.1 and Appendix B; TX-P12 steps 1 and 2).
//!
//! A schema declaration (record 8) has carried its payload shape as opaque CBOR since 0.1. A-7
//! gives key 4 a structure — a field table — and adds key 5, the **lens steps** that map one
//! revision's names to the next. This module is the one reading of both, so that `check`,
//! retrieval, `smysl-query` and `smysl-lib` cannot disagree about what a declaration says:
//!
//! - [`decode`] turns a [`SchemaDecl`](crate::types::SchemaDecl) into a [`Revision`]. A shape that does not match is kept
//!   opaque and reported (`SMY-W430`); an op tag this build does not know decodes to
//!   [`LensOp::Unknown`], whose step is unusable (`SMY-W469`). Nothing is misread.
//! - [`SchemaSet`] collects every revision of every id, finds the current one, and reports a
//!   fork (`SMY-W429`), which has no current revision until something resolves it.
//! - [`LensChain`] is a path of steps between two revisions, with its hash, composition, and
//!   forward and backward application over a [`UnitView`] (`lens.rs`); revision inference is
//!   `infer.rs`.
//!
//! **Lenses never write.** A lensed unit is a reading: its bytes, uid and attestations are
//! untouched (Appendix B). The one caller that turns a reading back into records is
//! `smysl-lib`'s `--target-revision`, and it does so as new units with their own attestations.
//!
//! **Values are [`HValue`]s**, the payload's own value model, with zero spans. A lens moves
//! payload keys and values and nothing else, so the type it moves them in is the type a payload
//! decodes to; `Cond` and `Mapped` are `PartialEq` and not `Eq` for the reason `HValue` is not
//! (a payload float).

mod check;
mod decode;
mod infer;
mod lens;
mod set;

pub use check::Totality;
pub use decode::{decode, encode_lenses, encode_shape};
pub use infer::RevisionGuess;
pub use lens::{LensChain, LensError, Mapped, UnitView, Widening};
pub use set::SchemaSet;

use std::collections::BTreeMap;

use crate::ids::SchemaId;
use crate::surface::HValue;
use crate::types::RelKind;

/// A field's declared type (A-7 field map key 0). A type name this build does not know is
/// kept as written: it is not a shape mismatch, only a type nothing here can check.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FieldType {
    Text,
    Int,
    Bool,
    Uid,
    Edtf,
    Enum,
    List(Box<FieldType>),
    Other(String),
}

impl FieldType {
    pub fn parse(s: &str) -> FieldType {
        match s {
            "text" => FieldType::Text,
            "int" => FieldType::Int,
            "bool" => FieldType::Bool,
            "uid" => FieldType::Uid,
            "edtf" => FieldType::Edtf,
            "enum" => FieldType::Enum,
            _ => match s.strip_prefix("list<").and_then(|r| r.strip_suffix('>')) {
                Some(inner) => FieldType::List(Box::new(FieldType::parse(inner))),
                None => FieldType::Other(s.to_string()),
            },
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            FieldType::Text => "text".into(),
            FieldType::Int => "int".into(),
            FieldType::Bool => "bool".into(),
            FieldType::Uid => "uid".into(),
            FieldType::Edtf => "edtf".into(),
            FieldType::Enum => "enum".into(),
            FieldType::List(t) => format!("list<{}>", t.as_string()),
            FieldType::Other(s) => s.clone(),
        }
    }
}

/// A one-key condition, `{key: value}` (A-7 `required-when`, Appendix B `when`).
#[derive(Debug, Clone, PartialEq)]
pub struct Cond {
    pub key: String,
    pub value: HValue,
}

impl Cond {
    pub fn new(key: impl Into<String>, value: HValue) -> Cond {
        Cond {
            key: key.into(),
            value,
        }
    }

    /// Whether a payload satisfies it: the key is present with exactly this value.
    pub fn holds(&self, payload: &BTreeMap<String, HValue>) -> bool {
        payload.get(&self.key) == Some(&self.value)
    }
}

/// One payload key's entry in a revision's field table (A-7).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Field {
    pub ty: FieldType,
    /// The admitted values, for `enum`.
    pub values: Vec<String>,
    pub retired_in: Option<u32>,
    pub note: Option<String>,
    pub required_from: Option<u32>,
    pub required_when: Option<Cond>,
    /// Text values admitted besides the type (`text:holder` is a `uid` that also admits `text`).
    pub literals: Vec<String>,
}

impl Field {
    pub fn new(ty: FieldType) -> Field {
        Field {
            ty,
            values: Vec::new(),
            retired_in: None,
            note: None,
            required_from: None,
            required_when: None,
            literals: Vec::new(),
        }
    }

    pub fn enumeration<I, S>(values: I) -> Field
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut f = Field::new(FieldType::Enum);
        f.values = values.into_iter().map(Into::into).collect();
        f
    }

    /// Whether `rev` is past this key's retirement.
    pub fn retired_at(&self, rev: u32) -> bool {
        self.retired_in.is_some_and(|r| rev >= r)
    }

    /// Whether a text value is one this field admits, for an `enum` (any other type admits
    /// every value as far as a lens is concerned: lenses map names, not types).
    pub fn admits(&self, v: &HValue) -> bool {
        match (&self.ty, v) {
            (FieldType::Enum, HValue::Str(s)) => {
                self.values.iter().any(|x| x == s) || self.literals.iter().any(|x| x == s)
            }
            (FieldType::Enum, _) => false,
            _ => true,
        }
    }
}

/// A revision's field table: typed when key 4 matches A-7, opaque when it does not
/// (`SMY-W430`), absent when the declaration has no key 4.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub enum Fields {
    Typed(BTreeMap<String, Field>),
    Opaque,
    #[default]
    Absent,
}

impl Fields {
    pub fn typed(&self) -> Option<&BTreeMap<String, Field>> {
        match self {
            Fields::Typed(m) => Some(m),
            _ => None,
        }
    }

    pub fn get(&self, key: &str) -> Option<&Field> {
        self.typed().and_then(|m| m.get(key))
    }
}

/// One lens operation (Appendix B). The op set is closed and the tag space open (D-5): a tag
/// this build does not know is [`LensOp::Unknown`], kept as its bytes, and makes its step
/// unusable rather than guessed at.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum LensOp {
    RenameType {
        from: SchemaId,
        to: SchemaId,
    },
    RenameRelation {
        from: RelKind,
        to: RelKind,
    },
    RenameKey {
        from: String,
        to: String,
    },
    MapValue {
        key: String,
        map: BTreeMap<String, String>,
    },
    SplitValue {
        key: String,
        from: String,
        to: String,
        add: BTreeMap<String, HValue>,
    },
    Require {
        key: String,
        when: Option<Cond>,
    },
    Default {
        key: String,
        value: HValue,
        when: Option<Cond>,
    },
    Retire {
        key: String,
    },
    Unknown {
        tag: String,
        bytes: Vec<u8>,
    },
}

impl LensOp {
    /// The op's tag, as written on the wire.
    pub fn tag(&self) -> &str {
        match self {
            LensOp::RenameType { .. } => "rename-type",
            LensOp::RenameRelation { .. } => "rename-relation",
            LensOp::RenameKey { .. } => "rename-key",
            LensOp::MapValue { .. } => "map-value",
            LensOp::SplitValue { .. } => "split-value",
            LensOp::Require { .. } => "require",
            LensOp::Default { .. } => "default",
            LensOp::Retire { .. } => "retire",
            LensOp::Unknown { tag, .. } => tag,
        }
    }

    /// Whether the op has a backward reading (Appendix B's last column). `retire` never does;
    /// a `map-value` only when it is one-to-one; an unknown op is not known to.
    pub fn is_invertible(&self) -> bool {
        match self {
            LensOp::Retire { .. } | LensOp::Unknown { .. } => false,
            LensOp::MapValue { map, .. } => {
                let mut seen = std::collections::BTreeSet::new();
                map.values().all(|v| seen.insert(v))
            }
            _ => true,
        }
    }
}

/// A lens step, `{0: from, 1: to, 2: [op…]}`, with the bytes it was received as.
///
/// The bytes are what the chain hash is over and what [`encode_lenses`] writes back, so a
/// decoded step re-encodes to exactly what arrived (law L9). A step built with
/// [`LensStep::new`] gets its canonical encoding.
#[derive(Debug, Clone, PartialEq)]
pub struct LensStep {
    pub from: u32,
    pub to: u32,
    pub ops: Vec<LensOp>,
    bytes: Vec<u8>,
}

impl LensStep {
    pub fn new(from: u32, to: u32, ops: Vec<LensOp>) -> LensStep {
        let bytes = decode::step_bytes(from, to, &ops);
        LensStep {
            from,
            to,
            ops,
            bytes,
        }
    }

    /// The step's bytes, as received or as canonically encoded.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Whether any op is one this build does not know (D-5): such a step is unusable.
    pub fn is_usable(&self) -> bool {
        !self.ops.iter().any(|o| matches!(o, LensOp::Unknown { .. }))
    }

    pub fn is_invertible(&self) -> bool {
        self.ops.iter().all(LensOp::is_invertible)
    }

    pub fn has_retire(&self) -> bool {
        self.ops.iter().any(|o| matches!(o, LensOp::Retire { .. }))
    }
}

/// One declaration of a schema id at one version, decoded (§3.8).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Revision {
    pub id: SchemaId,
    pub version: u32,
    /// BLAKE3 of the declaration record's canonical bytes: what tells two declarations of one
    /// `(id, version)` apart, and so what a fork is.
    pub decl_hash: [u8; 32],
    pub types: Vec<SchemaId>,
    pub relations: Vec<RelKind>,
    pub fields: Fields,
    pub lenses: Vec<LensStep>,
    /// Key 5 was present and did not match Appendix B's structure (`SMY-W430`): the revision
    /// has no usable steps, which is different from having none.
    pub lenses_opaque: bool,
}

impl Revision {
    /// Whether this revision declares the type.
    pub fn declares_type(&self, ty: &SchemaId) -> bool {
        self.types.iter().any(|t| t == ty)
    }
}
