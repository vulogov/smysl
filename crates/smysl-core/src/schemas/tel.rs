//! `x.tel/v1` — the schema for telemetry study (RFC SMYSL-3 §4, TL-P1).
//!
//! Registered with the standing `x.text/v1` has: a store that does not know it preserves every
//! unit and relation under rule X and enforces none of its rules. Seventeen unit types, eight
//! relation kinds, and payload fields tabled **per type**, because one key can mean two things —
//! `tel:basis` is `start-time` or `decrease` on a reset and `substrate` or `platform` on a
//! window.
//!
//! **Field tables in code, as `x.text/v1`'s are.** The structure that puts a field map on the
//! wire is A-7, so [`decl`] leaves `payload_shape` absent and [`TYPE_DEFS`] is the table. The two
//! encodings the kernel lacks are [`FieldType::Dec`] and [`FieldType::Bytes32`].
//!
//! **Keys are `tel:`-prefixed.** The RFC names its fields bare (`scheme`, `key`, `from_ms`). The
//! repository's extension schemas prefix theirs (`text:holder`, `query:text`, `measure:v`), and
//! the prefix is not taste: a bare payload key that is a unit header word (`status`, `source`,
//! `grounds`) has no surface spelling at all. So `scheme` is `tel:scheme`, and the RFC's
//! spellings are otherwise kept, underscores included.
//!
//! **The entity-like rule (§4.1, the A-2.4 rule 7 analogue).** A series and an instrument are
//! what is measured and by what, not when, so their identity is a function of an identity
//! tuple alone and they collapse across time, readers and peers. [`series`] and [`instrument`]
//! take nothing else; [`entity_like_violations`] is what `check` reports as `SMY-E607`.

use crate::error::ShapeError;
use crate::ids::SchemaId;
use crate::schemas::{Field, FieldType};
use crate::surface::hjson::{HObject, HValue, Spanned};
use crate::types::{RelKind, SchemaDecl, SourceKind, SourceRef, Status, UnitCore, UnitCoreBuilder};

/// The schema's own id.
pub const ID: &str = "x.tel/v1";

/// The revision this build declares.
pub const VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Unit types (§4.1)
// ---------------------------------------------------------------------------

/// A series: what is measured, not when. Entity-like.
pub const SERIES: &str = "x.tel/series";
/// A series' non-identifying metadata, versioned by `supersedes`.
pub const SERIES_META: &str = "x.tel/series-meta";
/// A thing in a measurement lineage. Entity-like.
pub const INSTRUMENT: &str = "x.tel/instrument";
/// An instrument's non-identifying metadata.
pub const INSTRUMENT_META: &str = "x.tel/instrument-meta";
/// One leaf changed between two snapshots (§8).
pub const CHANGE: &str = "x.tel/change";
/// An object or series recreated: the same name, a new identity.
pub const RECREATE: &str = "x.tel/recreate";
/// A counter reset.
pub const RESET: &str = "x.tel/reset";
/// A series ended or went stale.
pub const STALE: &str = "x.tel/stale";
/// Window evidence (§9).
pub const WINDOW: &str = "x.tel/window";
/// A regime between two change points.
pub const SEGMENT: &str = "x.tel/segment";
/// A point or window judged anomalous by a recipe.
pub const ANOMALY: &str = "x.tel/anomaly";
/// An alerting system's own statement.
pub const ALERT: &str = "x.tel/alert";
/// A deterministic grouping of changes and anomalies in a window.
pub const EPISODE: &str = "x.tel/episode";
/// A proposed explanation.
pub const HYPOTHESIS: &str = "x.tel/hypothesis";
/// The anchor of one investigation.
pub const INVESTIGATION: &str = "x.tel/investigation";
/// A read-only observation an investigator made.
pub const PROBE: &str = "x.tel/probe";
/// Something done to the system.
pub const ACTION: &str = "x.tel/action";

/// The seventeen types, in §4.1's order.
pub const TYPES: &[&str] = &[
    SERIES,
    SERIES_META,
    INSTRUMENT,
    INSTRUMENT_META,
    CHANGE,
    RECREATE,
    RESET,
    STALE,
    WINDOW,
    SEGMENT,
    ANOMALY,
    ALERT,
    EPISODE,
    HYPOTHESIS,
    INVESTIGATION,
    PROBE,
    ACTION,
];

// ---------------------------------------------------------------------------
// Relation kinds (§4.2)
// ---------------------------------------------------------------------------

/// What a window, segment, anomaly, reset or stale unit is about.
pub const OF_SERIES: &str = "x.tel/of-series";
/// Lineage: the source instrument's data passed through the target.
pub const VIA: &str = "x.tel/via";
/// The instrument that measures a series at its origin.
pub const MEASURED_BY: &str = "x.tel/measured-by";
/// Two series measure one quantity, in the same unit.
pub const SAME_QUANTITY: &str = "x.tel/same-quantity";
/// Membership in an episode or an investigation.
pub const DURING: &str = "x.tel/during";
/// A hypothesis offers an explanation of its target.
pub const EXPLAINS: &str = "x.tel/explains";
/// An investigator looked at the target.
pub const CONSULTED: &str = "x.tel/consulted";
/// A causal order known from the protocol or the tool. Ordinary until GE-M5 (§10.4).
pub const PRECEDES: &str = "x.tel/precedes";

/// The eight relation kinds, in §4.2's order.
pub const RELATIONS: &[&str] = &[
    OF_SERIES,
    VIA,
    MEASURED_BY,
    SAME_QUANTITY,
    DURING,
    EXPLAINS,
    CONSULTED,
    PRECEDES,
];

// ---------------------------------------------------------------------------
// Enumerations (§4.3). All open: a value this build does not know is preserved.
// ---------------------------------------------------------------------------

pub const SCHEMES: &[&str] = &["otel", "prom", "k8s", "zabbix", "json"];
pub const CLASSES: &[&str] = &[
    "config",
    "identity",
    "membership",
    "counter",
    "gauge",
    "histogram",
    "event",
    "log",
];
pub const KINDS: &[&str] = &["counter", "gauge", "histogram", "summary", "state", "text"];
pub const ROLES: &[&str] = &[
    "exporter",
    "agent",
    "scraper",
    "collector",
    "pipeline",
    "store",
    "transform",
];
pub const VTYPES: &[&str] = &["null", "bool", "int", "dec", "text", "json"];
/// A window's basis: computed from our substrate, or transcribed from a platform's aggregate.
pub const BASES: &[&str] = &["substrate", "platform"];
/// A change's operation (§8.1 step 5).
pub const OPS: &[&str] = &["add", "remove", "replace"];
/// How a reset was known: a start-time change the source states, or a decrease we inferred.
pub const RESET_BASES: &[&str] = &["start-time", "decrease"];
/// What marked a series stale. §4.1 names the field and not its values; these are the two
/// markers §3 names, Prometheus' stale NaN and OTLP's `FLAG_NO_RECORDED_VALUE`.
pub const MARKERS: &[&str] = &["stale-nan", "no-recorded-value"];
pub const DIRECTIONS: &[&str] = &["up", "down"];
pub const STATES: &[&str] = &["firing", "resolved"];

// ---------------------------------------------------------------------------
// Fields (§4.1), per type
// ---------------------------------------------------------------------------

/// One unit type's payload: what it must carry and what it may.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TypeDef {
    pub ty: &'static str,
    pub required: &'static [Field],
    pub optional: &'static [Field],
}

impl TypeDef {
    /// The field with this key, required or optional.
    pub fn field(&self, key: &str) -> Option<&'static Field> {
        self.required
            .iter()
            .chain(self.optional.iter())
            .find(|f| f.key == key)
    }
}

const fn f(key: &'static str, ty: FieldType) -> Field {
    Field {
        key,
        ty,
        values: &[],
        literals: &[],
    }
}

const fn e(key: &'static str, values: &'static [&'static str]) -> Field {
    Field {
        key,
        ty: FieldType::Enum,
        values,
        literals: &[],
    }
}

const TEXT: FieldType = FieldType::Text;
const INT: FieldType = FieldType::Int;
const UID: FieldType = FieldType::Uid;
const DEC: FieldType = FieldType::Dec;
const DECS: FieldType = FieldType::List(&FieldType::Dec);

const FROM_MS: Field = f("tel:from_ms", INT);
const TO_MS: Field = f("tel:to_ms", INT);
const RECIPE: Field = f("tel:recipe", TEXT);
const OF_SERIES_UID: Field = f("tel:series", UID);
const AT_MS: Field = f("tel:at_ms", INT);
const OF: Field = f("tel:of", UID);

/// §4.1's table, in its order.
pub const TYPE_DEFS: &[TypeDef] = &[
    TypeDef {
        ty: SERIES,
        required: &[e("tel:scheme", SCHEMES), f("tel:key", TEXT)],
        optional: &[
            f("tel:name", TEXT),
            f("tel:unit", TEXT),
            e("tel:kind", KINDS),
        ],
    },
    TypeDef {
        ty: SERIES_META,
        required: &[OF],
        optional: &[
            f("tel:text", TEXT),
            f("tel:buckets", DECS),
            f("tel:native_id", TEXT),
        ],
    },
    TypeDef {
        ty: INSTRUMENT,
        required: &[e("tel:role", ROLES), f("tel:key", TEXT)],
        optional: &[],
    },
    TypeDef {
        ty: INSTRUMENT_META,
        required: &[OF],
        optional: &[f("tel:version", TEXT)],
    },
    TypeDef {
        ty: CHANGE,
        required: &[
            e("tel:op", OPS),
            f("tel:pointer", TEXT),
            e("tel:class", CLASSES),
            e("tel:vtype", VTYPES),
        ],
        optional: &[f("tel:from", TEXT), f("tel:to", TEXT), f("tel:object", UID)],
    },
    TypeDef {
        ty: RECREATE,
        required: &[f("tel:old", TEXT), f("tel:new", TEXT)],
        optional: &[f("tel:object", UID)],
    },
    TypeDef {
        ty: RESET,
        required: &[OF_SERIES_UID, AT_MS],
        optional: &[e("tel:basis", RESET_BASES)],
    },
    TypeDef {
        ty: STALE,
        required: &[OF_SERIES_UID, AT_MS],
        optional: &[e("tel:marker", MARKERS)],
    },
    TypeDef {
        ty: WINDOW,
        required: &[
            OF_SERIES_UID,
            f("tel:instrument", UID),
            FROM_MS,
            TO_MS,
            RECIPE,
            e("tel:basis", BASES),
            f("tel:count", INT),
        ],
        optional: &[
            // Required when `tel:basis` is `substrate` (§4.1); a condition a flat table cannot
            // state, so it is stated here and checked where windows are.
            f("tel:digest", FieldType::Bytes32),
            f("tel:min", DEC),
            f("tel:max", DEC),
            f("tel:sum", DEC),
            f("tel:mean", DEC),
            f("tel:first", DEC),
            f("tel:last", DEC),
            f("tel:qs", DECS),
            f("tel:q", DECS),
            f("tel:alpha", DEC),
        ],
    },
    TypeDef {
        ty: SEGMENT,
        required: &[OF_SERIES_UID, FROM_MS, TO_MS, RECIPE],
        optional: &[f("tel:level", DEC), f("tel:slope", DEC)],
    },
    TypeDef {
        ty: ANOMALY,
        required: &[OF_SERIES_UID, FROM_MS, TO_MS, RECIPE, f("tel:score", DEC)],
        optional: &[e("tel:direction", DIRECTIONS)],
    },
    TypeDef {
        ty: ALERT,
        required: &[
            f("tel:system", TEXT),
            f("tel:key", TEXT),
            e("tel:state", STATES),
        ],
        optional: &[f("tel:severity", TEXT)],
    },
    TypeDef {
        ty: EPISODE,
        required: &[FROM_MS, TO_MS, RECIPE],
        optional: &[f("tel:label", TEXT)],
    },
    TypeDef {
        ty: HYPOTHESIS,
        required: &[f("tel:claim", TEXT)],
        optional: &[
            f("tel:quotes", FieldType::List(&FieldType::Text)),
            f("tel:target", UID),
        ],
    },
    TypeDef {
        ty: INVESTIGATION,
        required: &[f("tel:question", TEXT), f("tel:opened_ms", INT)],
        optional: &[f("tel:rotation", TEXT), f("tel:closed_ms", INT)],
    },
    TypeDef {
        ty: PROBE,
        required: &[f("tel:what", TEXT), AT_MS],
        optional: &[f("tel:output", TEXT)],
    },
    TypeDef {
        ty: ACTION,
        required: &[f("tel:what", TEXT), AT_MS],
        optional: &[f("tel:confirmed_by", TEXT), f("tel:output", TEXT)],
    },
];

/// The table entry for a unit type, if `x.tel/v1` declares it.
pub fn type_def(ty: &str) -> Option<&'static TypeDef> {
    TYPE_DEFS.iter().find(|d| d.ty == ty)
}

/// Whether this schema id is one of the seventeen types.
pub fn declares(id: &SchemaId) -> bool {
    TYPES.contains(&id.as_str())
}

/// The declaration, as a record a store can hold. `payload_shape` is absent until A-7, as
/// `x.text/v1`'s is; infallible for the reason [`super::text::decl`] is.
pub fn decl() -> SchemaDecl {
    let mut d = SchemaDecl::new(SchemaId::Extension(ID.to_string()), VERSION);
    d.types = TYPES
        .iter()
        .map(|t| SchemaId::Extension((*t).to_string()))
        .collect();
    d.relations = RELATIONS
        .iter()
        .map(|r| RelKind::Extension((*r).to_string()))
        .collect();
    d
}

// ---------------------------------------------------------------------------
// The entity-like rule (§4.1)
// ---------------------------------------------------------------------------

/// The prefix of a series unit's `source.reference`: `series:<scheme>:<key>`.
pub const SERIES_REF: &str = "series:";
/// The prefix of an instrument unit's `source.reference`: `instrument:<key>`.
pub const INSTRUMENT_REF: &str = "instrument:";

/// The longest gist an entity-like unit is given, in characters. Past it the key is cut and
/// ends in an ellipsis; the whole key is in the payload and the source reference, so the cut
/// costs no identity, and a gist that fits a 30-token profile is what keeps `check` from
/// calling every long Prometheus label set `SMY-E022`.
pub const GIST_CHARS: usize = 64;

/// Why a series or instrument unit could not be built.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TelError {
    /// A value outside an enumeration this build writes. The enumerations are open for
    /// reading; a producer writes only what it can name.
    Unknown {
        field: &'static str,
        value: String,
    },
    /// An empty key: there is nothing to identify.
    EmptyKey,
    Shape(ShapeError),
}

impl core::fmt::Display for TelError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            TelError::Unknown { field, value } => {
                write!(f, "`{value}` is not a {field} this build writes")
            }
            TelError::EmptyKey => f.write_str("an empty key identifies nothing"),
            TelError::Shape(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for TelError {}

/// The gist of an entity-like unit: its key, cut at [`GIST_CHARS`].
fn key_gist(key: &str) -> String {
    if key.chars().count() <= GIST_CHARS {
        key.to_string()
    } else {
        let mut g: String = key.chars().take(GIST_CHARS - 1).collect();
        g.push('…');
        g
    }
}

fn payload(entries: &[(&str, &str)]) -> Vec<u8> {
    let span = crate::diag::Span::new(0, 0);
    let mut o = HObject::new();
    for (k, v) in entries {
        o.insert(
            Spanned::new((*k).to_string(), span),
            Spanned::new(HValue::Str((*v).to_string()), span),
        );
    }
    // Never `None`: both callers pass two entries.
    crate::surface::payload::object_to_payload(&o).unwrap_or_default()
}

/// A series unit (§4.1, §6): its scheme and its canonical identity key, and nothing else.
///
/// The arguments are the whole identity, as `super::text::entity`'s are: the key becomes the
/// payload's `tel:key` and the source's `series:<scheme>:<key>`, the source kind is `metric`,
/// and there is no parameter for anything that varies over time. Two readers, two peers or two
/// scrapes that see the same series therefore build the same uid. `name`, `unit` and `kind`
/// are declared fields and deliberately not written: they are inside the uid, and a reader that
/// disagreed about a unit's spelling would split one series into two. They belong in
/// `x.tel/series-meta`, which points at the series and never the other way.
///
/// **Status is `cited`, not the caller's**, for the reason an entity's is: nothing was
/// measured, and grounds are forbidden.
pub fn series(scheme: &str, key: &str) -> Result<UnitCore, TelError> {
    if !SCHEMES.contains(&scheme) {
        return Err(TelError::Unknown {
            field: "scheme",
            value: scheme.to_string(),
        });
    }
    if key.is_empty() {
        return Err(TelError::EmptyKey);
    }
    let source = SourceRef::new(SourceKind::Metric, format!("{SERIES_REF}{scheme}:{key}"));
    UnitCoreBuilder::new(
        SchemaId::Extension(SERIES.to_string()),
        key_gist(key),
        Status::Cited,
    )
    .source(source)
    .payload(payload(&[("tel:key", key), ("tel:scheme", scheme)]))
    .build()
    .map_err(TelError::Shape)
}

/// An instrument unit (§4.1, §7): its role and its own stable key, and nothing else.
///
/// The key is the instrument's identity — `prom-scrape:<job>/<instance>`,
/// `otel-collector:<service.instance.id>`, `zabbix-server:<name>` — and its version is
/// `x.tel/instrument-meta`'s, for the reason a series' metadata is not the series'.
pub fn instrument(role: &str, key: &str) -> Result<UnitCore, TelError> {
    if !ROLES.contains(&role) {
        return Err(TelError::Unknown {
            field: "role",
            value: role.to_string(),
        });
    }
    if key.is_empty() {
        return Err(TelError::EmptyKey);
    }
    let source = SourceRef::new(SourceKind::Tool, format!("{INSTRUMENT_REF}{key}"));
    UnitCoreBuilder::new(
        SchemaId::Extension(INSTRUMENT.to_string()),
        key_gist(key),
        Status::Cited,
    )
    .source(source)
    .payload(payload(&[("tel:key", key), ("tel:role", role)]))
    .build()
    .map_err(TelError::Shape)
}

/// What a series or instrument unit carries that §4.1 forbids it, by field name.
///
/// Empty for any other unit. The fields are those §4.1 names — `span`, `manifest`, `published`,
/// `observed`, `grounds` — and then the source itself, which must be exactly the kind and the
/// reference the identity tuple gives (`source.kind`, `source.reference`) with no other key
/// (`source.captured`, `source.extra`). A reference that does not agree with the payload's key is
/// `source.reference`: the two say what the series is, and only one of them can be right.
pub fn entity_like_violations(unit: &UnitCore) -> Vec<&'static str> {
    let ty = unit.schema.as_str();
    let (kind, want_ref) = match ty {
        SERIES => (SourceKind::Metric, series_reference(unit)),
        INSTRUMENT => (SourceKind::Tool, instrument_reference(unit)),
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    match unit.source.as_ref() {
        None => out.push("source.reference"),
        Some(s) => {
            if s.span().is_some() {
                out.push("span");
            }
            if s.manifest().is_some() {
                out.push("manifest");
            }
            if s.published.is_some() {
                out.push("published");
            }
            if s.observed.is_some() {
                out.push("observed");
            }
            if s.captured.is_some() {
                out.push("source.captured");
            }
            if !s.extra.is_empty() {
                out.push("source.extra");
            }
            if s.kind != kind {
                out.push("source.kind");
            }
            if want_ref.as_deref() != Some(s.reference.as_str()) {
                out.push("source.reference");
            }
        }
    }
    if !unit.grounds.is_empty() {
        out.push("grounds");
    }
    out
}

/// A payload's string field, if the payload decodes and carries it.
fn payload_str(unit: &UnitCore, key: &str) -> Option<String> {
    let o = crate::surface::payload::payload_to_object(unit.payload.as_ref()?).ok()?;
    match &o.get(key)?.value {
        HValue::Str(s) => Some(s.clone()),
        _ => None,
    }
}

/// The reference a series unit's payload says it should carry.
fn series_reference(unit: &UnitCore) -> Option<String> {
    let scheme = payload_str(unit, "tel:scheme")?;
    let key = payload_str(unit, "tel:key")?;
    Some(format!("{SERIES_REF}{scheme}:{key}"))
}

/// The reference an instrument unit's payload says it should carry.
fn instrument_reference(unit: &UnitCore) -> Option<String> {
    Some(format!("{INSTRUMENT_REF}{}", payload_str(unit, "tel:key")?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seventeen_types_and_eight_relations_all_well_formed() {
        assert_eq!(TYPES.len(), 17);
        assert_eq!(RELATIONS.len(), 8);
        assert!(SchemaId::parse(ID).is_ok());
        for t in TYPES {
            assert!(SchemaId::parse(t).is_ok(), "{t}");
            assert!(declares(&SchemaId::Extension((*t).to_string())));
        }
        for r in RELATIONS {
            let k = RelKind::parse(r).expect("a well-formed kind");
            assert!(matches!(k, RelKind::Extension(_)), "{r}");
        }
    }

    #[test]
    fn the_declaration_agrees_with_the_parser_and_redefines_nothing() {
        let d = decl();
        assert_eq!(d.id, SchemaId::parse(ID).unwrap());
        let parsed: Vec<SchemaId> = TYPES.iter().map(|t| SchemaId::parse(t).unwrap()).collect();
        assert_eq!(d.types, parsed);
        let kinds: Vec<RelKind> = RELATIONS
            .iter()
            .map(|r| RelKind::parse(r).unwrap())
            .collect();
        assert_eq!(d.relations, kinds);
        assert!(!d.redefines_kernel());
        assert!(d.payload_shape.is_none(), "the shape waits for A-7");
    }

    /// Every type has a table, in §4.1's order, and every key is `tel:`-prefixed and unique
    /// within its type.
    #[test]
    fn every_type_has_its_fields() {
        assert_eq!(TYPE_DEFS.len(), TYPES.len());
        for (d, t) in TYPE_DEFS.iter().zip(TYPES) {
            assert_eq!(d.ty, *t);
            assert!(!d.required.is_empty(), "{t} requires nothing");
            let mut keys: Vec<&str> = d
                .required
                .iter()
                .chain(d.optional.iter())
                .map(|f| f.key)
                .collect();
            assert!(keys.iter().all(|k| k.starts_with("tel:")), "{t}: {keys:?}");
            let n = keys.len();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), n, "{t} declares a key twice");
        }
        // One key, two meanings, which is why the table is per type.
        assert_eq!(
            type_def(RESET)
                .and_then(|d| d.field("tel:basis"))
                .map(|f| f.values),
            Some(RESET_BASES)
        );
        assert_eq!(
            type_def(WINDOW)
                .and_then(|d| d.field("tel:basis"))
                .map(|f| f.values),
            Some(BASES)
        );
        assert_eq!(
            type_def(WINDOW)
                .and_then(|d| d.field("tel:digest"))
                .map(|f| f.ty),
            Some(FieldType::Bytes32)
        );
        assert_eq!(
            type_def(WINDOW)
                .and_then(|d| d.field("tel:q"))
                .map(|f| f.ty),
            Some(FieldType::List(&FieldType::Dec))
        );
        assert!(type_def("x.tel/span").is_none(), "TL-P9's, not TL-P1's");
    }

    #[test]
    fn a_series_is_its_scheme_and_its_key() {
        let key = r#"http_requests_total{code="200",job="api"}"#;
        let u = series("prom", key).expect("a series");
        assert_eq!(u.schema.as_str(), SERIES);
        assert_eq!(u.status, Status::Cited);
        let s = u.source.as_ref().expect("a source");
        assert_eq!(s.kind, SourceKind::Metric);
        assert_eq!(s.reference, format!("series:prom:{key}"));
        assert!(entity_like_violations(&u).is_empty());
        assert_eq!(payload_str(&u, "tel:key").as_deref(), Some(key));
        assert_eq!(payload_str(&u, "tel:scheme").as_deref(), Some("prom"));
        // The same series from anywhere is one uid; another key or scheme is another series.
        let again = series("prom", key).unwrap();
        assert_eq!(crate::canonical_uid(&u), crate::canonical_uid(&again));
        assert_ne!(
            crate::canonical_uid(&u),
            crate::canonical_uid(&series("otel", key).unwrap())
        );
    }

    #[test]
    fn a_long_key_is_cut_in_the_gist_and_whole_everywhere_else() {
        let key = format!("node_cpu_seconds_total{{{}}}", "x=\"y\",".repeat(40));
        let u = series("prom", &key).unwrap();
        assert_eq!(u.gist.chars().count(), GIST_CHARS);
        assert!(u.gist.ends_with('…'));
        assert_eq!(payload_str(&u, "tel:key").as_deref(), Some(key.as_str()));
        // Two keys sharing the gist's prefix are still two series.
        let other = format!("{key},z=\"w\"");
        let v = series("prom", &other).unwrap();
        assert_eq!(u.gist, v.gist);
        assert_ne!(crate::canonical_uid(&u), crate::canonical_uid(&v));
    }

    #[test]
    fn an_instrument_is_its_role_and_its_key() {
        let u = instrument("scraper", "prom-scrape:node/10.0.0.1:9100").unwrap();
        assert_eq!(u.schema.as_str(), INSTRUMENT);
        let s = u.source.as_ref().unwrap();
        assert_eq!(s.kind, SourceKind::Tool);
        assert_eq!(s.reference, "instrument:prom-scrape:node/10.0.0.1:9100");
        assert!(entity_like_violations(&u).is_empty());
    }

    #[test]
    fn a_producer_writes_only_what_it_can_name() {
        assert!(matches!(
            series("graphite", "a.b"),
            Err(TelError::Unknown {
                field: "scheme",
                ..
            })
        ));
        assert!(matches!(
            instrument("oracle", "x"),
            Err(TelError::Unknown { field: "role", .. })
        ));
        assert_eq!(series("prom", ""), Err(TelError::EmptyKey));
    }

    #[test]
    fn the_rule_names_what_broke_it() {
        let good = series("prom", "up").unwrap();

        let mut timed = good.clone();
        let mut s = timed.source.clone().unwrap();
        s.observed = Some(1_705_400_000_000);
        timed.source = Some(s);
        assert_eq!(entity_like_violations(&timed), vec!["observed"]);

        let mut grounded = good.clone();
        grounded.grounds.insert(crate::Uid::from_bytes([1; 32]));
        assert_eq!(entity_like_violations(&grounded), vec!["grounds"]);

        let mut wrong_ref = good.clone();
        wrong_ref.source = Some(SourceRef::new(SourceKind::Metric, "series:prom:down"));
        assert_eq!(entity_like_violations(&wrong_ref), vec!["source.reference"]);

        let mut wrong_kind = good.clone();
        wrong_kind.source = Some(SourceRef::new(SourceKind::Doc, "series:prom:up"));
        assert_eq!(entity_like_violations(&wrong_kind), vec!["source.kind"]);

        let mut extra = good.clone();
        let mut s = extra.source.clone().unwrap();
        s.extra.insert(7, vec![0x01]);
        extra.source = Some(s);
        assert_eq!(entity_like_violations(&extra), vec!["source.extra"]);

        let mut bare = good;
        bare.source = None;
        assert_eq!(entity_like_violations(&bare), vec!["source.reference"]);

        // Not a series or an instrument: not this rule's business.
        let claim = UnitCoreBuilder::new(crate::KernelType::Claim, "c", Status::Speculative)
            .build()
            .unwrap();
        assert!(entity_like_violations(&claim).is_empty());
    }
}
