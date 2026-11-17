//! Episodes (RFC SMYSL-3 §21, TL-P5): changes and anomalies close in time, grouped.
//!
//! An episode is a deterministic grouping, not a judgement: the members are taken in order of
//! (time, uid), and a new episode begins wherever the gap to the previous member exceeds `gap`.
//! A group of at least `min_members` (default 2) becomes an `x.tel/episode` `[first, last + 1 ms)`,
//! `derived`, grounded on its members, each of which joins it by `x.tel/during`.
//!
//! A member's time is when it happened:
//! - an anomaly's or segment's `tel:from_ms`;
//! - a reset's or stale's `tel:at_ms`;
//! - a change's, recreate's or alert's `source.observed`, the reading that showed it.
//!
//! Units of other types, and units with no time, are not members.

use smysl_core::schemas::tel;
use smysl_core::surface::hjson::HValue;
use smysl_core::{
    canonical_uid, RelKind, Relation, SchemaId, SourceKind, SourceRef, Status, Uid, UnitCore,
    UnitCoreBuilder,
};

use super::Recipe;
use crate::scrape::{ms, payload};

/// The detector and its version.
pub const ID: &str = "episode/1";

/// The unit types an episode groups.
pub const MEMBER_TYPES: &[&str] = &[
    tel::CHANGE,
    tel::RECREATE,
    tel::RESET,
    tel::STALE,
    tel::ANOMALY,
    tel::ALERT,
];

/// Episode grouping's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// The largest gap within one episode, ms.
    pub gap_ms: u64,
    pub min_members: usize,
}

impl Default for Params {
    fn default() -> Params {
        Params {
            gap_ms: 300_000,
            min_members: 2,
        }
    }
}

impl Params {
    pub fn recipe(&self, from_ms: u64, to_ms: u64) -> Recipe {
        Recipe::new(ID)
            .with("gap", self.gap_ms)
            .with("min", self.min_members)
            .with("from", from_ms)
            .with("to", to_ms)
    }
}

/// When a member happened, or `None` if `u` is not a member.
pub fn member_time(u: &UnitCore) -> Option<u64> {
    let ty = u.schema.as_str();
    if !MEMBER_TYPES.contains(&ty) {
        return None;
    }
    let field = |k: &str| {
        let o = smysl_core::surface::payload_to_object(u.payload.as_deref()?).ok()?;
        match o.get(k)?.value {
            HValue::Int(n) => u64::try_from(n).ok(),
            _ => None,
        }
    };
    match ty {
        t if t == tel::ANOMALY => field("tel:from_ms"),
        t if t == tel::RESET || t == tel::STALE => field("tel:at_ms"),
        _ => u.source.as_ref()?.observed,
    }
}

/// One episode, its members, and their `x.tel/during` edges.
#[derive(Debug, Clone)]
pub struct Episode {
    pub unit: UnitCore,
    pub members: Vec<Uid>,
    pub relations: Vec<Relation>,
}

/// Group `units` (any order) in `[from_ms, to_ms)` into episodes.
pub fn run(units: &[UnitCore], from_ms: u64, to_ms: u64, p: &Params) -> Vec<Episode> {
    let mut members: Vec<(u64, Uid, &str)> = units
        .iter()
        .filter_map(|u| {
            let t = member_time(u)?;
            (from_ms..to_ms)
                .contains(&t)
                .then(|| (t, canonical_uid(u), u.schema.as_str()))
        })
        .collect();
    members.sort_by_key(|a| (a.0, a.1));
    members.dedup_by(|a, b| a.1 == b.1);

    let recipe = p.recipe(from_ms, to_ms);
    let mut groups: Vec<Vec<(u64, Uid, &str)>> = Vec::new();
    for m in members {
        match groups.last_mut() {
            Some(g) if m.0 - g.last().expect("a group is never empty").0 <= p.gap_ms => g.push(m),
            _ => groups.push(vec![m]),
        }
    }
    groups
        .into_iter()
        .filter(|g| g.len() >= p.min_members.max(1))
        .map(|g| {
            let (first, last) = (g[0].0, g[g.len() - 1].0);
            let count = |ty: &str| g.iter().filter(|m| m.2 == ty).count();
            let label = format!(
                "{} change(s), {} anomaly(ies), {} other",
                count(tel::CHANGE) + count(tel::RECREATE),
                count(tel::ANOMALY),
                g.len() - count(tel::CHANGE) - count(tel::RECREATE) - count(tel::ANOMALY)
            );
            let uids: Vec<Uid> = g.iter().map(|m| m.1).collect();
            let unit = UnitCoreBuilder::new(
                SchemaId::Extension(tel::EPISODE.to_string()),
                tel::gist_of(&format!("episode [{first}, {}) {label}", last + 1)),
                Status::Derived,
            )
            .source(
                SourceRef::new(SourceKind::Tool, format!("detector:{ID}")).observed_at(last + 1),
            )
            .grounds(uids.clone())
            .payload(payload(&[
                ("tel:from_ms", ms(first)),
                ("tel:to_ms", ms(last + 1)),
                ("tel:recipe", HValue::Str(recipe.text())),
                ("tel:label", HValue::Str(label)),
            ]))
            .build()
            .expect("an episode unit is well formed by construction");
            let ep = canonical_uid(&unit);
            let relations = uids
                .iter()
                .map(|m| Relation::new(RelKind::Extension(tel::DURING.to_string()), *m, ep))
                .collect();
            Episode {
                unit,
                members: uids,
                relations,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{anomaly, Direction, Input};

    fn an(t: u64, n: u8) -> UnitCore {
        let input = Input {
            series: Uid::from_bytes([n; 32]),
            scheme: "prom",
            key: "lat",
            instrument: Uid::from_bytes([2; 32]),
            samples: &[],
        };
        anomaly(
            &input,
            t,
            t + 1_000,
            &Recipe::new("t/1"),
            4.0,
            Some(Direction::Up),
        )
        .unwrap()
    }

    #[test]
    fn members_close_in_time_are_one_episode_and_order_does_not_matter() {
        let units = vec![
            an(10_000, 1),
            an(100_000, 2),
            an(5_000_000, 3),
            an(200_000, 4),
        ];
        let p = Params::default();
        let eps = run(&units, 0, 10_000_000, &p);
        assert_eq!(eps.len(), 1, "the lone one at 5 000 000 is not an episode");
        assert_eq!(eps[0].members.len(), 3);
        assert_eq!(eps[0].relations.len(), 3);
        assert_eq!(eps[0].unit.status, Status::Derived);
        assert_eq!(eps[0].unit.grounds.len(), 3);
        let mut rev = units.clone();
        rev.reverse();
        assert_eq!(
            canonical_uid(&run(&rev, 0, 10_000_000, &p)[0].unit),
            canonical_uid(&eps[0].unit)
        );
        // A smaller gap splits it.
        // A gap of 95 s keeps 10 000 and 100 000 (90 s apart) and splits off 200 000.
        let tight = Params {
            gap_ms: 95_000,
            ..p
        };
        let split = run(&units, 0, 10_000_000, &tight);
        assert_eq!(split.len(), 1);
        assert_eq!(split[0].members.len(), 2);
        assert!(run(&units, 0, 50_000, &p).is_empty(), "outside the window");
    }

    #[test]
    fn what_is_not_a_member_is_left_out() {
        let series = smysl_core::schemas::tel::series("prom", "up").unwrap();
        assert_eq!(member_time(&series), None);
        assert_eq!(member_time(&an(7, 1)), Some(7));
    }
}
