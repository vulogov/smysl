//! `fixtures/wire/F15-telemetry.cbor` (RFC SMYSL-3, TL-P1 step 4): a store of `x.tel/v1`.
//!
//! C-Tel-Read asks two things of a reader: decode `dec` canonically, which
//! `fixtures/tel/dec/cases.json` tests, and preserve every `x.tel` type, relation and field
//! under rule X, which this tests. The three ports glob `fixtures/wire` and assert every record
//! re-encodes to the bytes it arrived in, so adding the file *is* the cross-implementation
//! test, as F14's was.
//!
//! What it holds is what a reader that knows nothing of telemetry has to carry: the schema's
//! declaration; a series and an instrument; a window whose payload holds `dec`s, a list of
//! `dec`s and a 32-byte digest — a CBOR byte string, which no earlier payload in the fixtures
//! has; an anomaly grounded on it; a hypothesis quoting it; and three of the new relation kinds.
//! Uid-typed fields (`tel:series`, `tel:instrument`, `tel:target`) are `b3:` text, as every
//! uid-typed payload field is until A-7; the digest is not a reference, so it stays bytes.
//!
//! `SMYSL_BLESS=1` rewrites the file. That changes what three other implementations are checked
//! against.
//!
//! F15 is a store a revision-1 build wrote, and stays one: its declaration is
//! `tel::decl_at(1)`. `fixtures/wire/F16-telemetry-info.cbor` (RFC SMYSL-5 TI-P0) is what a
//! revision-2 build adds to such a store: the revision-2 declaration beside revision 1's, and
//! two `x.tel/info` units holding between them every key the type defines, among them a list of negative
//! integers, which no earlier payload in the fixtures has. Its exit criterion is the same as
//! F15's: the three ports round-trip it byte for byte, by globbing.

use std::path::{Path, PathBuf};

use smysl_core::cbor::major;
use smysl_core::cbor::writer::Enc;
use smysl_core::schemas::tel;
use smysl_core::{
    canonical_uid, from_cbor_seq, to_cbor_seq, Decimal, Record, RelKind, Relation, SchemaId,
    SourceKind, SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};

fn fixture_path() -> PathBuf {
    wire("F15-telemetry.cbor")
}

fn wire(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/wire")
        .join(name)
}

/// One payload value, by its CBOR.
enum V {
    Text(&'static str),
    Int(u64),
    SInt(i64),
    Ints(Vec<i64>),
    Uid(Uid),
    Dec(Decimal),
    Decs(Vec<Decimal>),
    Texts(Vec<String>),
    Bytes([u8; 32]),
}

/// A payload map: text keys, sorted by their encoded bytes as §3 requires.
fn payload(entries: Vec<(&str, V)>) -> Vec<u8> {
    let mut kv: Vec<(Vec<u8>, Vec<u8>)> = entries
        .into_iter()
        .map(|(k, v)| {
            let mut ke = Enc::new();
            ke.text(k);
            let mut ve = Enc::new();
            match v {
                V::Text(s) => ve.text(s),
                V::Int(n) => ve.uint(n),
                V::SInt(n) => ve.int(n),
                V::Ints(ns) => {
                    ve.array_head(ns.len());
                    for n in ns {
                        ve.int(n);
                    }
                }
                // `b3:` text, as `text:holder` is written: until A-7 a payload uid is text, which
                // keeps its surface form (TL-P2 step 3; F15 carried byte strings until then).
                V::Uid(u) => ve.text(&u.canonical()),
                V::Dec(d) => d.encode(&mut ve),
                V::Decs(ds) => {
                    ve.array_head(ds.len());
                    for d in ds {
                        d.encode(&mut ve);
                    }
                }
                V::Texts(ts) => {
                    ve.array_head(ts.len());
                    for t in ts {
                        ve.text(&t);
                    }
                }
                V::Bytes(b) => ve.bytes(&b),
            }
            (ke.into_bytes(), ve.into_bytes())
        })
        .collect();
    kv.sort();
    let mut e = Enc::new();
    e.head(major::MAP, kv.len() as u64);
    for (k, v) in kv {
        e.raw(&k);
        e.raw(&v);
    }
    e.into_bytes()
}

fn dec(t: &str) -> Decimal {
    Decimal::parse_token(t).expect("a dec")
}

fn ty(t: &str) -> SchemaId {
    SchemaId::Extension(t.to_string())
}

fn build() -> Vec<u8> {
    let series = tel::series(
        "prom",
        r#"http_request_duration_seconds{service="checkout"}"#,
    )
    .expect("a series");
    let instrument = tel::instrument("scraper", "prom-scrape:checkout/10.0.0.7:9100").unwrap();
    let (s, i) = (canonical_uid(&series), canonical_uid(&instrument));

    let mut source = SourceRef::new(
        SourceKind::Metric,
        r#"series:prom:http_request_duration_seconds{service="checkout"}"#,
    );
    source.observed = Some(1_705_400_060_000);
    let window: UnitCore = UnitCoreBuilder::new(
        ty(tel::WINDOW),
        "checkout latency, one minute",
        Status::Derived,
    )
    .source(source.clone())
    .grounds([s])
    .payload(payload(vec![
        ("tel:series", V::Uid(s)),
        ("tel:instrument", V::Uid(i)),
        ("tel:from_ms", V::Int(1_705_400_000_000)),
        ("tel:to_ms", V::Int(1_705_400_060_000)),
        ("tel:recipe", V::Text("tel/window-ddsketch/1")),
        ("tel:basis", V::Text("substrate")),
        ("tel:count", V::Int(4)),
        ("tel:digest", V::Bytes([0xa5; 32])),
        ("tel:min", V::Dec(dec("0.0004"))),
        ("tel:max", V::Dec(dec("2.31"))),
        // Beyond i64: a mantissa no surface payload can spell, carried in CBOR.
        (
            "tel:sum",
            V::Dec(Decimal::new(18_446_744_073_709_551_615, -19).unwrap()),
        ),
        ("tel:qs", V::Decs(vec![dec("0.5"), dec("0.9"), dec("0.99")])),
        ("tel:q", V::Decs(vec![dec("0.012"), dec("0.9"), dec("2.3")])),
        ("tel:alpha", V::Dec(dec("0.01"))),
    ]))
    .build()
    .unwrap();
    let w = canonical_uid(&window);

    let anomaly = UnitCoreBuilder::new(ty(tel::ANOMALY), "checkout p99 jumped", Status::Derived)
        .source(source)
        .grounds([w])
        .payload(payload(vec![
            ("tel:series", V::Uid(s)),
            ("tel:from_ms", V::Int(1_705_400_000_000)),
            ("tel:to_ms", V::Int(1_705_400_060_000)),
            ("tel:recipe", V::Text("tel/mad/1")),
            ("tel:score", V::Dec(dec("7.25"))),
            ("tel:direction", V::Text("up")),
        ]))
        .build()
        .unwrap();
    let a = canonical_uid(&anomaly);

    let hypothesis = UnitCoreBuilder::new(
        ty(tel::HYPOTHESIS),
        "the deploy at 10:00 made checkout slow",
        Status::Inferred,
    )
    .grounds([w])
    .payload(payload(vec![
        (
            "tel:claim",
            V::Text("the deploy at 10:00 made checkout slow"),
        ),
        ("tel:quotes", V::Texts(vec![format!("{w}#tel:q[2]=2.3")])),
        ("tel:target", V::Uid(a)),
    ]))
    .build()
    .unwrap();
    let h = canonical_uid(&hypothesis);

    to_cbor_seq(&[
        Record::SchemaDecl(tel::decl_at(1).expect("revision 1")),
        Record::Unit(series),
        Record::Unit(instrument),
        Record::Unit(window),
        Record::Unit(anomaly),
        Record::Unit(hypothesis),
        Record::Relation(Relation::new(RelKind::parse(tel::OF_SERIES).unwrap(), w, s)),
        Record::Relation(Relation::new(
            RelKind::parse(tel::MEASURED_BY).unwrap(),
            s,
            i,
        )),
        Record::Relation(Relation::new(RelKind::parse(tel::EXPLAINS).unwrap(), h, a)),
    ])
}

/// The committed fixture is what this test builds, and every record in it round-trips.
#[test]
fn the_telemetry_fixture_is_current_and_round_trips() {
    let built = build();
    let path = fixture_path();
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &built).expect("writing the fixture");
    }
    let on_disk = std::fs::read(&path).expect("fixtures/wire/F15-telemetry.cbor is missing");
    assert_eq!(
        on_disk, built,
        "F15-telemetry.cbor no longer matches its source; SMYSL_BLESS=1 rewrites it, and that \
         changes what Python, JavaScript and Go are checked against"
    );
    let (records, _) = from_cbor_seq(&on_disk).expect("it decodes");
    assert_eq!(records.len(), 9);
    assert_eq!(to_cbor_seq(&records), on_disk, "it re-encodes to itself");
}

/// The window's payload is CBOR-only: the digest is a byte string and the sum's mantissa is
/// beyond `i64`, and the surface payload model holds neither. The series has a surface form.
#[test]
fn the_window_lives_in_cbor_and_the_series_does_not_have_to() {
    let (records, _) = from_cbor_seq(&build()).unwrap();
    let unit = |n: usize| match &records[n] {
        Record::Unit(u) => u.clone(),
        other => panic!("{other:?}"),
    };
    assert!(smysl_core::surface::unit_has_surface_form(&unit(1)));
    assert!(!smysl_core::surface::unit_has_surface_form(&unit(3)));
}

/// F16: what a revision-2 build writes beside F15's records. Two series, so that `mi` has its
/// `with`; a lag scan from −2 to 2 with surrogates; and an entropy under `baseline` bins, so
/// that every optional key is present in one unit or the other.
fn build_info() -> Vec<u8> {
    let a = tel::series("prom", r#"queue_depth{service="checkout"}"#).expect("a series");
    let b = tel::series("prom", r#"pool_wait_seconds{service="checkout"}"#).expect("a series");
    let instrument = tel::instrument("scraper", "prom-scrape:checkout/10.0.0.7:9100").unwrap();
    let (sa, sb, i) = (
        canonical_uid(&a),
        canonical_uid(&b),
        canonical_uid(&instrument),
    );
    let mut source = SourceRef::new(
        SourceKind::Metric,
        r#"series:prom:queue_depth{service="checkout"}"#,
    );
    source.observed = Some(1_705_403_600_000);
    let mut grounds = [sa, sb];
    grounds.sort();
    let recipe = "mi/1?bins=rank:4&cover=4/5&guard=1/10&hold=60000&lag=-2..2&min_n=80\
                  &null=200&of=value&seed=7&step=60000";
    let info: UnitCore = UnitCoreBuilder::new(
        ty(tel::INFO),
        "queue depth and pool wait, mutual information, one hour",
        Status::Derived,
    )
    .source(source)
    .grounds(grounds)
    .payload(payload(vec![
        ("tel:stat", V::Text("mi")),
        ("tel:series", V::Uid(sa)),
        ("tel:instrument", V::Uid(i)),
        ("tel:from_ms", V::Int(1_705_400_000_000)),
        ("tel:to_ms", V::Int(1_705_403_600_000)),
        ("tel:recipe", V::Text(recipe)),
        ("tel:n", V::Int(58)),
        ("tel:value", V::Dec(dec("0.412871"))),
        ("tel:digest", V::Bytes([0x5a; 32])),
        ("tel:k", V::Int(4)),
        ("tel:missing", V::Int(2)),
        ("tel:with", V::Uid(sb)),
        ("tel:with_instrument", V::Uid(i)),
        ("tel:with_digest", V::Bytes([0x3c; 32])),
        ("tel:k_with", V::Int(4)),
        ("tel:bias", V::Dec(dec("0.111924"))),
        ("tel:lags", V::Ints(vec![-2, -1, 0, 1, 2])),
        (
            "tel:values",
            V::Decs(vec![
                dec("0.061"),
                dec("0.412871"),
                dec("0.20494"),
                dec("0.0"),
                dec("0.033117"),
            ]),
        ),
        ("tel:lag", V::SInt(-1)),
        ("tel:null_b", V::Int(200)),
        ("tel:null_ge", V::Int(3)),
    ]))
    .build()
    .unwrap();
    let u = canonical_uid(&info);

    // An entropy under `baseline` bins: the keys an `mi` unit cannot hold, `mm` and the
    // reference window's three.
    let mut source = SourceRef::new(
        SourceKind::Metric,
        r#"series:prom:queue_depth{service="checkout"}"#,
    );
    source.observed = Some(1_705_403_600_000);
    let entropy: UnitCore = UnitCoreBuilder::new(
        ty(tel::INFO),
        "queue depth, entropy against the hour before, one hour",
        Status::Derived,
    )
    .source(source)
    .grounds([sa])
    .payload(payload(vec![
        ("tel:stat", V::Text("entropy")),
        ("tel:series", V::Uid(sa)),
        ("tel:instrument", V::Uid(i)),
        ("tel:from_ms", V::Int(1_705_400_000_000)),
        ("tel:to_ms", V::Int(1_705_403_600_000)),
        (
            "tel:recipe",
            V::Text(
                "ent/1?bins=baseline:4&cover=4/5&hold=60000&min_n=40&of=value\
                 &ref=1705396400000-1705400000000&step=60000",
            ),
        ),
        ("tel:n", V::Int(60)),
        ("tel:value", V::Dec(dec("1.629546"))),
        ("tel:mm", V::Dec(dec("1.665614"))),
        ("tel:digest", V::Bytes([0x5a; 32])),
        ("tel:k", V::Int(4)),
        ("tel:ref_from_ms", V::Int(1_705_396_400_000)),
        ("tel:ref_to_ms", V::Int(1_705_400_000_000)),
        ("tel:ref_digest", V::Bytes([0xc3; 32])),
    ]))
    .build()
    .unwrap();
    let h = canonical_uid(&entropy);

    to_cbor_seq(&[
        Record::SchemaDecl(tel::decl_at(1).expect("revision 1")),
        Record::SchemaDecl(tel::decl()),
        Record::Unit(a),
        Record::Unit(b),
        Record::Unit(instrument),
        Record::Unit(info),
        Record::Unit(entropy),
        Record::Relation(Relation::new(
            RelKind::parse(tel::OF_SERIES).unwrap(),
            u,
            sa,
        )),
        Record::Relation(Relation::new(
            RelKind::parse(tel::OF_SERIES).unwrap(),
            u,
            sb,
        )),
        Record::Relation(Relation::new(
            RelKind::parse(tel::OF_SERIES).unwrap(),
            h,
            sa,
        )),
    ])
}

/// F16 is current and round-trips, and it holds both revisions of the declaration.
#[test]
fn the_info_fixture_is_current_and_round_trips() {
    let built = build_info();
    let path = wire("F16-telemetry-info.cbor");
    if std::env::var_os("SMYSL_BLESS").is_some() {
        std::fs::write(&path, &built).expect("writing the fixture");
    }
    let on_disk = std::fs::read(&path).expect("fixtures/wire/F16-telemetry-info.cbor is missing");
    assert_eq!(
        on_disk, built,
        "F16-telemetry-info.cbor no longer matches its source; SMYSL_BLESS=1 rewrites it, and \
         that changes what Python, JavaScript and Go are checked against"
    );
    let (records, _) = from_cbor_seq(&on_disk).expect("it decodes");
    assert_eq!(records.len(), 10);
    assert_eq!(to_cbor_seq(&records), on_disk, "it re-encodes to itself");
    let versions: Vec<u32> = records
        .iter()
        .filter_map(|r| match r {
            Record::SchemaDecl(d) => Some(d.version),
            _ => None,
        })
        .collect();
    assert_eq!(versions, [1, 2]);
}

/// Between them the two info units hold every key the type defines, and nothing else.
#[test]
fn the_info_units_hold_every_key_the_type_defines() {
    let (records, _) = from_cbor_seq(&build_info()).unwrap();
    let mut keys = std::collections::BTreeSet::new();
    for r in &records {
        let Record::Unit(u) = r else { continue };
        if u.schema.as_str() != tel::INFO {
            continue;
        }
        let mut d = smysl_core::cbor::reader::Dec::new(u.payload.as_deref().expect("a payload"));
        for _ in 0..d.map_head().unwrap() {
            keys.insert(d.text().unwrap().to_string());
            d.skip_item().unwrap();
        }
    }
    let def = tel::type_def(tel::INFO).unwrap();
    let defined: std::collections::BTreeSet<String> = def
        .required
        .iter()
        .chain(def.optional.iter())
        .map(|f| f.key.to_string())
        .collect();
    assert_eq!(keys, defined);
}
