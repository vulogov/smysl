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

use std::path::{Path, PathBuf};

use smysl_core::cbor::major;
use smysl_core::cbor::writer::Enc;
use smysl_core::schemas::tel;
use smysl_core::{
    canonical_uid, from_cbor_seq, to_cbor_seq, Decimal, Record, RelKind, Relation, SchemaId,
    SourceKind, SourceRef, Status, Uid, UnitCore, UnitCoreBuilder,
};

fn fixture_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/wire/F15-telemetry.cbor")
}

/// One payload value, by its CBOR.
enum V {
    Text(&'static str),
    Int(u64),
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
        Record::SchemaDecl(tel::decl()),
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
