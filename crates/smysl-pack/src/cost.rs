//! The cost model (D-2, §18.1).
//!
//! Packing must be pure (rule D), and provider tokenisers are neither available offline nor
//! stable across versions. So the bundled estimator is deterministic and **approximate
//! against any specific model by design** - and every pack records which estimator produced
//! it, so a budget is never silently misrepresented as exact (N11).

use smysl_core::{Lod, UnitCore};

/// A token cost model.
///
/// `--tokenizer <id>` selects an alternative; whichever is used is recorded in the
/// `packinfo`, because a budget that does not say what it was counted with is a number
/// without a unit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Estimator {
    /// `ceil(utf8_len(t) / 4) + 2` (D-2). The `+ 2` is per-item framing overhead.
    #[default]
    Utf8Div4,
}

impl Estimator {
    pub const ALL: &'static [Estimator] = &[Estimator::Utf8Div4];

    /// The identifier recorded in a `packinfo`.
    pub fn id(&self) -> &'static str {
        match self {
            Estimator::Utf8Div4 => "smysl/utf8-div4",
        }
    }

    pub fn parse(s: &str) -> Option<Estimator> {
        Estimator::ALL.iter().find(|e| e.id() == s).cloned()
    }

    /// The cost of one text.
    pub fn text(&self, t: &str) -> u64 {
        match self {
            Estimator::Utf8Div4 => smysl_core::tokens(t) as u64 + 2,
        }
    }

    /// The cost of a unit at a level.
    ///
    /// Levels are cumulative: L1 includes the gist, L2 includes both. That is what makes
    /// an upgrade an incremental purchase rather than a replacement.
    pub fn unit(&self, core: &UnitCore, level: Lod) -> u64 {
        let mut total = self.text(&core.gist);
        if level >= Lod::L1 {
            total += core.body.as_deref().map(|b| self.text(b)).unwrap_or(0);
        }
        if level >= Lod::L2 {
            total += core.detail.as_deref().map(|d| self.text(d)).unwrap_or(0);
        }
        total
    }

    /// What upgrading from `from` to `to` costs.
    pub fn upgrade(&self, core: &UnitCore, from: Lod, to: Lod) -> u64 {
        self.unit(core, to).saturating_sub(self.unit(core, from))
    }
}

/// The levels a unit was actually authored at.
///
/// A unit with no body cannot be selected at L1: there is nothing to pay for. This is why
/// authored level of detail makes summarisation a lookup (rule L) rather than a generation.
pub fn available_levels(core: &UnitCore) -> Vec<Lod> {
    let mut out = vec![Lod::L0];
    if core.body.is_some() {
        out.push(Lod::L1);
    }
    if core.detail.is_some() {
        out.push(Lod::L2);
    }
    out
}

/// The value of a unit at a level: `salience · gain(level)` (§8).
///
/// Gains diminish - 1.0, 1.6, 1.85 - so at equal salience breadth beats depth. A budget
/// spent on two gists buys more than one body.
pub fn value(salience: f32, level: Lod) -> f64 {
    salience as f64 * level.gain() as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Status, UnitCoreBuilder};

    fn unit(body: Option<&str>, detail: Option<&str>) -> UnitCore {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, "a gist", Status::Speculative);
        if let Some(t) = body {
            b = b.body(t);
        }
        if let Some(t) = detail {
            b = b.detail(t);
        }
        b.build().unwrap()
    }

    #[test]
    fn the_estimator_is_the_documented_formula() {
        let e = Estimator::default();
        assert_eq!(e.text(""), 2);
        assert_eq!(e.text("a"), 3);
        assert_eq!(e.text("abcd"), 3);
        assert_eq!(e.text("abcde"), 4);
    }

    #[test]
    fn the_estimator_id_is_recorded_verbatim() {
        assert_eq!(Estimator::default().id(), "smysl/utf8-div4");
        assert_eq!(
            Estimator::parse("smysl/utf8-div4"),
            Some(Estimator::Utf8Div4)
        );
        assert_eq!(Estimator::parse("tiktoken/cl100k"), None);
    }

    /// Levels are cumulative, so an upgrade is an incremental purchase.
    #[test]
    fn levels_are_cumulative() {
        let e = Estimator::default();
        let u = unit(Some("the body text"), Some("the detail text"));
        let l0 = e.unit(&u, Lod::L0);
        let l1 = e.unit(&u, Lod::L1);
        let l2 = e.unit(&u, Lod::L2);
        assert!(l0 < l1 && l1 < l2);
        assert_eq!(l1, l0 + e.text("the body text"));
        assert_eq!(l2, l1 + e.text("the detail text"));
    }

    #[test]
    fn an_upgrade_costs_the_difference() {
        let e = Estimator::default();
        let u = unit(Some("the body text"), Some("the detail text"));
        assert_eq!(
            e.upgrade(&u, Lod::L0, Lod::L2),
            e.unit(&u, Lod::L2) - e.unit(&u, Lod::L0)
        );
        assert_eq!(e.upgrade(&u, Lod::L2, Lod::L0), 0, "downgrades are free");
    }

    /// A unit with no body cannot be bought at L1 - there is nothing to pay for.
    #[test]
    fn only_authored_levels_are_available() {
        assert_eq!(available_levels(&unit(None, None)), vec![Lod::L0]);
        assert_eq!(
            available_levels(&unit(Some("b"), None)),
            vec![Lod::L0, Lod::L1]
        );
        assert_eq!(
            available_levels(&unit(Some("b"), Some("d"))),
            vec![Lod::L0, Lod::L1, Lod::L2]
        );
    }

    #[test]
    fn asking_for_a_level_a_unit_lacks_costs_no_more_than_it_has() {
        let e = Estimator::default();
        let u = unit(None, None);
        assert_eq!(e.unit(&u, Lod::L2), e.unit(&u, Lod::L0));
    }

    /// Breadth beats depth at equal salience, which is the point of diminishing gains.
    #[test]
    fn two_gists_are_worth_more_than_one_body() {
        assert!(value(0.5, Lod::L0) * 2.0 > value(0.5, Lod::L1));
        assert!(value(0.5, Lod::L1) * 2.0 > value(0.5, Lod::L2));
    }

    #[test]
    fn value_scales_with_salience() {
        assert!(value(1.0, Lod::L0) > value(0.5, Lod::L0));
        assert_eq!(value(0.0, Lod::L2), 0.0);
    }
}
