//! `x.tel/v1` (RFC SMYSL-3 §4, TL-P1 step 3): its entity-like units, checked and spelled.

use smysl_check::{check, CheckOptions};
use smysl_core::schemas::tel;
use smysl_core::{Code, Record};
use smysl_graph::Store;

/// A series as the producer builds it is clean, and gains `SMY-E607` the moment it carries a
/// time: one series seen at fifty instants would otherwise be fifty units.
#[test]
fn a_series_with_a_time_is_e607() {
    let good = tel::series("prom", r#"up{job="node"}"#).unwrap();
    let report = check(
        &Store::from_records(vec![Record::Unit(good.clone())]),
        CheckOptions::strict(),
    );
    assert!(!report.iter().any(|d| d.code == Code::E607), "{report:?}");

    let mut timed = good;
    let mut s = timed.source.clone().unwrap();
    s.observed = Some(1_705_400_000_000);
    timed.source = Some(s);
    let uid = smysl_core::canonical_uid(&timed);
    let report = check(
        &Store::from_records(vec![Record::Unit(timed)]),
        CheckOptions::strict(),
    );
    let e607: Vec<_> = report.iter().filter(|d| d.code == Code::E607).collect();
    assert_eq!(e607.len(), 1, "{report:?}");
    assert_eq!(e607[0].uid(), Some(&uid));
    assert!(e607[0].message.contains("observed"), "{}", e607[0].message);
}

/// A series and an instrument have a surface form, and the surface gives back the same uid:
/// `tel:`-prefixed keys are ordinary payload keys, which is why they are prefixed.
#[test]
fn entity_like_units_round_trip_through_the_surface() {
    let units = [
        tel::series("prom", r#"http_requests_total{code="200",job="api"}"#).unwrap(),
        tel::instrument("scraper", "prom-scrape:api/10.0.0.1:9100").unwrap(),
    ];
    for u in units {
        let uid = smysl_core::canonical_uid(&u);
        let records = vec![Record::Unit(u)];
        assert!(smysl_core::surface::write::unit_has_surface_form(
            match &records[0] {
                Record::Unit(u) => u,
                _ => unreachable!(),
            }
        ));
        let ctx = smysl_core::surface::WriteContext::from_labels(&Default::default());
        let text = smysl_core::surface::write_surface(None, &records, &ctx);
        let back = smysl_core::surface::parse_surface(&text).expect("it parses");
        let got: Vec<_> = back
            .records
            .iter()
            .filter_map(|r| match r {
                Record::Unit(u) => Some(smysl_core::canonical_uid(u)),
                _ => None,
            })
            .collect();
        assert_eq!(got, vec![uid], "{text}");
    }
}
