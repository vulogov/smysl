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

/// A gist cut at [`GIST_CHARS`], ending in an ellipsis when it was cut: an entity-like unit's
/// key, or a change unit's `op` and pointer. Deterministic, so a cut gist is still a function of
/// what it was cut from, and the whole text is always elsewhere in the unit.
pub fn gist_of(text: &str) -> String {
    if text.chars().count() <= GIST_CHARS {
        text.to_string()
    } else {
        let mut g: String = text.chars().take(GIST_CHARS - 1).collect();
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
        gist_of(key),
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
        gist_of(key),
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

/// What an `x.tel/window` payload says (§4.1), read from its CBOR.
///
/// Read directly rather than through the surface payload model, which has no byte string: a
/// window's `digest` is `bytes32`, so a window payload has no surface form (§9, TL-P4). Fields
/// this reader does not know are skipped, as rule X asks.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowFields {
    pub series: Option<String>,
    pub instrument: Option<String>,
    pub from_ms: Option<u64>,
    pub to_ms: Option<u64>,
    pub recipe: Option<String>,
    pub basis: Option<String>,
    pub count: Option<u64>,
    pub digest: Option<[u8; 32]>,
    /// `qs`, `q` or `alpha` is present: the window carries an approximate field (§9.3).
    pub approximate: bool,
}

/// Read a window payload. `Err` names what is malformed.
pub fn window_fields(payload: &[u8]) -> Result<WindowFields, String> {
    use crate::cbor::major;
    use crate::cbor::reader::Dec;
    let mut d = Dec::new(payload);
    let n = d.map_head().map_err(|e| format!("not a map: {e}"))?;
    let mut w = WindowFields::default();
    for _ in 0..n {
        let key = d
            .text()
            .map_err(|e| format!("a key that is not text: {e}"))?;
        let what = |e: crate::error::CodecError| format!("`{key}`: {e}");
        match key {
            "tel:series" => w.series = Some(d.text().map_err(what)?.to_string()),
            "tel:instrument" => w.instrument = Some(d.text().map_err(what)?.to_string()),
            "tel:recipe" => w.recipe = Some(d.text().map_err(what)?.to_string()),
            "tel:basis" => w.basis = Some(d.text().map_err(what)?.to_string()),
            "tel:from_ms" => w.from_ms = Some(d.uint().map_err(what)?),
            "tel:to_ms" => w.to_ms = Some(d.uint().map_err(what)?),
            "tel:count" => w.count = Some(d.uint().map_err(what)?),
            "tel:digest" => {
                if d.peek_major().map_err(what)? != major::BYTES {
                    return Err("`tel:digest` is not a byte string".into());
                }
                let b = d.bytes().map_err(what)?;
                w.digest = Some(
                    b.try_into()
                        .map_err(|_| format!("`tel:digest` is {} bytes, not 32", b.len()))?,
                );
            }
            "tel:qs" | "tel:q" | "tel:alpha" => {
                w.approximate = true;
                d.skip_item().map_err(what)?;
            }
            _ => {
                d.skip_item().map_err(what)?;
            }
        }
    }
    if !d.is_done() {
        return Err("bytes after the map".into());
    }
    Ok(w)
}

/// Rule TW (§9.4): what a window's status and attestations must be, given its basis and fields.
///
/// | basis | fields | status | attestation |
/// |---|---|---|---|
/// | `substrate` | exact only | `measured` | `Op::Imported`, `Rung::Computed` |
/// | `substrate` | any approximate | `derived` | `Op::Transformed`, `Rung::Computed` |
/// | `platform` | — (no `digest`) | `cited` | `Op::Imported`, `Rung::Document` |
///
/// `None` for a payload without a known basis, which is a violation of its own.
pub fn rule_tw(w: &WindowFields) -> Option<(Status, crate::Op, crate::Rung)> {
    use crate::{Op, Rung};
    match (w.basis.as_deref()?, w.approximate) {
        ("substrate", false) => Some((Status::Measured, Op::Imported, Rung::Computed)),
        ("substrate", true) => Some((Status::Derived, Op::Transformed, Rung::Computed)),
        ("platform", _) => Some((Status::Cited, Op::Imported, Rung::Document)),
        _ => None,
    }
}

/// How a window unit, with these attestations, breaks rule TW (`SMY-E603`); empty when it does
/// not. Not a window, nothing.
pub fn rule_tw_violations(unit: &UnitCore, attestations: &[crate::Attestation]) -> Vec<String> {
    if unit.schema.as_str() != WINDOW {
        return Vec::new();
    }
    let Some(payload) = unit.payload.as_deref() else {
        return vec!["it has no payload".into()];
    };
    let w = match window_fields(payload) {
        Ok(w) => w,
        Err(e) => return vec![format!("its payload is not a window's: {e}")],
    };
    let Some((status, op, rung)) = rule_tw(&w) else {
        return vec![format!(
            "its basis is {}, not substrate or platform",
            w.basis.as_deref().unwrap_or("absent")
        )];
    };
    let mut out = Vec::new();
    let basis = w.basis.as_deref().unwrap_or("");
    let kind = if w.approximate {
        "approximate"
    } else {
        "exact"
    };
    if unit.status != status {
        out.push(format!(
            "a {kind} window of basis {basis} is {}, not {}",
            status.as_str(),
            unit.status.as_str()
        ));
    }
    match (basis, w.digest.is_some()) {
        ("substrate", false) => out.push("a substrate window has no digest".into()),
        ("platform", true) => out.push("a platform window carries a digest".into()),
        _ => {}
    }
    for a in attestations {
        if a.op != op || a.rung != rung {
            out.push(format!(
                "attested {:?} at {:?} by {}; rule TW asks {op:?} at {rung:?}",
                a.op, a.rung, a.agent
            ));
        }
    }
    out
}

/// One numeric quote of a hypothesis (§11.4): `<uid>#<field>=<value>`, or
/// `<uid>#<field>[<index>]=<value>` into a list field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Quote {
    pub unit: crate::Uid,
    /// The field without its `tel:` prefix: `max`, `q`.
    pub field: String,
    pub index: Option<usize>,
    /// As written: it must be the canonical spelling to verify.
    pub value: String,
}

/// Parse a quote. `Err` says why it is not one.
pub fn parse_quote(s: &str) -> Result<Quote, String> {
    let (uid, rest) = s
        .split_once('#')
        .ok_or_else(|| format!("`{s}` has no `#`: a quote is <uid>#<field>=<value>"))?;
    let unit: crate::Uid = uid
        .parse()
        .map_err(|_| format!("`{uid}` is not a unit uid"))?;
    let (lhs, value) = rest
        .split_once('=')
        .ok_or_else(|| format!("`{s}` has no `=`"))?;
    let (field, index) = match lhs.strip_suffix(']').and_then(|l| l.split_once('[')) {
        Some((f, i)) => (
            f,
            Some(
                i.parse::<usize>()
                    .map_err(|_| format!("`{i}` is not a 0-based index"))?,
            ),
        ),
        None => (lhs, None),
    };
    let name_ok = field.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && field
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if !name_ok {
        return Err(format!("`{field}` is not a field name"));
    }
    if value.is_empty() {
        return Err(format!("`{s}` quotes no value"));
    }
    Ok(Quote {
        unit,
        field: field.to_string(),
        index,
        value: value.to_string(),
    })
}

/// The canonical text of a payload's number at `tel:<field>` (`[index]` into a list): a `dec`
/// by its §5.3 spelling, an integer as itself. `None` when the payload, the field or the index
/// is not there, or holds no number. Read from the CBOR, so it works on a window's payload,
/// which has no surface form.
pub fn quoted_value(payload: &[u8], field: &str, index: Option<usize>) -> Option<String> {
    use crate::cbor::major;
    use crate::cbor::reader::Dec;
    fn number(d: &mut Dec<'_>) -> Option<String> {
        match d.peek_major().ok()? {
            major::UINT | major::NEGINT => Some(d.int().ok()?.to_string()),
            major::ARRAY => {
                let start = d.position();
                match crate::Decimal::decode(d) {
                    Ok(v) => Some(v.to_string()),
                    Err(_) => {
                        d.set_position(start);
                        None
                    }
                }
            }
            _ => None,
        }
    }
    let mut d = Dec::new(payload);
    let n = d.map_head().ok()?;
    let want = format!("tel:{field}");
    for _ in 0..n {
        let key = d.text().ok()?;
        if key != want {
            d.skip_item().ok()?;
            continue;
        }
        return match index {
            None => number(&mut d),
            Some(i) => {
                let len = d.array_head().ok()?;
                if i >= len {
                    return None;
                }
                for _ in 0..i {
                    d.skip_item().ok()?;
                }
                number(&mut d)
            }
        };
    }
    None
}

/// A hypothesis's quotes, as written in `tel:quotes`.
pub fn quotes_of(unit: &UnitCore) -> Vec<String> {
    let Some(o) = unit
        .payload
        .as_deref()
        .and_then(|p| crate::surface::payload::payload_to_object(p).ok())
    else {
        return Vec::new();
    };
    match o.get("tel:quotes").map(|v| &v.value) {
        Some(HValue::Array(items)) => items
            .iter()
            .filter_map(|i| match &i.value {
                HValue::Str(s) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Rule TQ (§11.4): each quote of `hypothesis` that does not verify, and why. `payload_of`
/// resolves a uid to the cited unit's payload in the record set.
///
/// A quote verifies when the cited unit is among the hypothesis's grounds, resolves, has the
/// field (and the index), and holds a value whose canonical text **equals** the quoted value:
/// `0.90` does not verify a `0.9`, because a quote is the evidence's own spelling.
pub fn rule_tq_violations(
    hypothesis: &UnitCore,
    payload_of: impl Fn(&crate::Uid) -> Option<Vec<u8>>,
) -> Vec<(String, String)> {
    if hypothesis.schema.as_str() != HYPOTHESIS {
        return Vec::new();
    }
    let mut out = Vec::new();
    for q in quotes_of(hypothesis) {
        let why = match parse_quote(&q) {
            Err(e) => Some(e),
            Ok(p) if !hypothesis.grounds.contains(&p.unit) => {
                Some("the cited unit is not among the hypothesis's grounds".to_string())
            }
            Ok(p) => match payload_of(&p.unit) {
                None => Some("the cited unit does not resolve".to_string()),
                Some(bytes) => match quoted_value(&bytes, &p.field, p.index) {
                    None => Some(format!(
                        "the cited unit has no number at `{}{}`",
                        p.field,
                        p.index.map_or(String::new(), |i| format!("[{i}]"))
                    )),
                    Some(v) if v == p.value => None,
                    Some(v) => Some(format!("the cited unit holds {v}, not {}", p.value)),
                },
            },
        };
        if let Some(w) = why {
            out.push((q, w));
        }
    }
    out
}

/// Rule TQ's cap: a hypothesis with a quote that does not verify is at most `speculative`.
pub fn tq_capped(status: Status, failures: usize) -> Status {
    if failures > 0 && status > Status::Speculative {
        Status::Speculative
    } else {
        status
    }
}

/// Detection kind 6, `RivalExplanation` (§11.5): two or more live hypotheses that each explain
/// one target, none rebutting or superseding another, each at an effective status of at least
/// `inferred` after rules M and TQ. One contention per target, `over` the target, `positions`
/// the hypotheses.
///
/// Derived-only, like kinds 4 and 5: detected from the record set, never written. A function of
/// the record set: the inputs are the store's units, its edges, the effective status a unit has
/// after retraction and rule M (`effective`), and its payload for rule TQ (`payload_of`). `now`
/// stamps the detection; it is not part of the id.
///
/// A hypothesis is **live** when nothing supersedes it.
pub fn rival_explanations<'a>(
    units: impl IntoIterator<Item = (&'a crate::Uid, &'a UnitCore)>,
    relations: impl IntoIterator<Item = &'a crate::Relation>,
    effective: impl Fn(&crate::Uid) -> Status,
    payload_of: impl Fn(&crate::Uid) -> Option<Vec<u8>>,
    now: crate::Hlc,
) -> Vec<crate::Contention> {
    use std::collections::{BTreeMap, BTreeSet};
    let explains = RelKind::Extension(EXPLAINS.to_string());
    let relations: Vec<&crate::Relation> = relations.into_iter().collect();
    let superseded: BTreeSet<crate::Uid> = relations
        .iter()
        .filter(|r| r.kind == RelKind::Supersedes)
        .map(|r| r.to)
        .collect();
    // Hypotheses that stand: live, and inferred or better once TQ has capped them.
    let mut standing: BTreeSet<crate::Uid> = BTreeSet::new();
    for (uid, u) in units {
        if u.schema.as_str() != HYPOTHESIS || superseded.contains(uid) {
            continue;
        }
        let failures = rule_tq_violations(u, &payload_of).len();
        if tq_capped(effective(uid), failures) >= Status::Inferred {
            standing.insert(*uid);
        }
    }
    let mut by_target: BTreeMap<crate::Uid, BTreeSet<crate::Uid>> = BTreeMap::new();
    let mut against: BTreeSet<(crate::Uid, crate::Uid)> = BTreeSet::new();
    for r in &relations {
        if r.kind == explains && standing.contains(&r.from) {
            by_target.entry(r.to).or_default().insert(r.from);
        }
        if r.kind == RelKind::Rebuts || r.kind == RelKind::Supersedes {
            against.insert((r.from, r.to));
            against.insert((r.to, r.from));
        }
    }
    let mut out = Vec::new();
    for (target, hyps) in by_target {
        if hyps.len() < 2 {
            continue;
        }
        let any_against = hyps
            .iter()
            .any(|a| hyps.iter().any(|b| a != b && against.contains(&(*a, *b))));
        if any_against {
            continue;
        }
        let positions: Vec<crate::Uid> = hyps.into_iter().collect();
        let kind = crate::DetectionKind::RivalExplanation;
        out.push(crate::Contention::new(
            crate::ContentionId::derive(kind, &target, &positions),
            target,
            positions,
            crate::Detected::new(kind, now.clone()),
        ));
    }
    out
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
