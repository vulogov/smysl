//! Application state and its transitions.
//!
//! **Nothing here touches a terminal.** State, the key map, and the transition function are
//! ordinary values and pure functions, so the interesting half of the UI is testable
//! without a tty, a screen size, or a running event loop. [`crate::draw`] renders this; it
//! never decides anything.

use std::collections::{BTreeMap, BTreeSet};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use smysl_core::{Label, Lod, Uid};
use smysl_graph::{salience, SalienceReport, SalienceRequest, Store};
use smysl_pack::{pack, Estimator, PackRequest, Reason};

use crate::Pane;

/// Everything a keypress can ask for.
///
/// A key map that returned closures, or mutated state directly, would put the decisions in
/// the event loop where no test can reach them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Action {
    Quit,
    NextPane,
    PrevPane,
    Up,
    Down,
    Top,
    Bottom,
    /// Widen the pack simulator's budget.
    BudgetUp,
    /// Narrow it - the direction that makes the constraints bind.
    BudgetDown,
    /// Back to the whole graph at full detail.
    BudgetReset,
    /// Pin the unit under the cursor into the pack, or unpin it (C5).
    ToggleFocus,
    ToggleHelp,
    /// A key with no meaning here. Explicit, so the loop never has to guess.
    Ignored,
}

impl Action {
    /// The key map, as a pure function.
    ///
    /// Both `q` and `Esc` quit, and `Ctrl-C` does too: a full-screen program that traps the
    /// one key everybody reaches for is a program people kill from another terminal.
    pub fn from_key(key: KeyEvent) -> Action {
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            return match key.code {
                KeyCode::Char('c') => Action::Quit,
                _ => Action::Ignored,
            };
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => Action::Quit,
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => Action::NextPane,
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => Action::PrevPane,
            KeyCode::Down | KeyCode::Char('j') => Action::Down,
            KeyCode::Up | KeyCode::Char('k') => Action::Up,
            KeyCode::Home | KeyCode::Char('g') => Action::Top,
            KeyCode::End | KeyCode::Char('G') => Action::Bottom,
            KeyCode::Char('+') | KeyCode::Char('=') => Action::BudgetUp,
            KeyCode::Char('-') | KeyCode::Char('_') => Action::BudgetDown,
            KeyCode::Char('0') => Action::BudgetReset,
            KeyCode::Char('f') | KeyCode::Enter => Action::ToggleFocus,
            KeyCode::Char('?') => Action::ToggleHelp,
            _ => Action::Ignored,
        }
    }
}

/// What the pack simulator found at the current budget.
///
/// `Infeasible` is a first-class outcome rather than an error to report and forget. A
/// budget too small to hold a claim *and its rebuttal* makes packing fail by design, and
/// the number that would make it feasible is the single most useful thing the pane can
/// say - so it is carried here rather than flattened into a message.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Sim {
    Packed {
        selected: BTreeMap<Uid, Lod>,
        why: BTreeMap<Uid, Reason>,
        used: u64,
        dropped: usize,
        degraded: usize,
    },
    Infeasible {
        required: u64,
    },
}

impl Sim {
    pub fn is_selected(&self, uid: &Uid) -> bool {
        match self {
            Sim::Packed { selected, .. } => selected.contains_key(uid),
            Sim::Infeasible { .. } => false,
        }
    }

    pub fn level(&self, uid: &Uid) -> Option<Lod> {
        match self {
            Sim::Packed { selected, .. } => selected.get(uid).copied(),
            Sim::Infeasible { .. } => None,
        }
    }

    pub fn reason(&self, uid: &Uid) -> Option<&Reason> {
        match self {
            Sim::Packed { why, .. } => why.get(uid),
            Sim::Infeasible { .. } => None,
        }
    }
}

/// The whole application.
#[derive(Debug)]
pub struct App {
    store: Store,
    /// uid -> the label a human wrote for it, where there is one.
    names: BTreeMap<Uid, Label>,
    /// Every unit, in a stable order, which is what the graph pane lists.
    order: Vec<Uid>,
    salience: SalienceReport,

    pane: Pane,
    cursor: usize,
    budget: u64,
    /// Units pinned into the pack (C5).
    ///
    /// The dial is far less interesting without one. With nothing forced in, a budget of
    /// zero simply selects nothing and is perfectly feasible; it is only when a unit is
    /// pinned that its rebuttals become mandatory and the floor can exceed the budget.
    /// Pinning is therefore how rule R is made visible rather than merely described.
    focus: BTreeSet<Uid>,
    /// The cost of the whole graph at full detail: the budget's ceiling and its reset.
    full: u64,
    sim: Sim,
    help: bool,
    quit: bool,
}

impl App {
    /// Build from a store and the labels its document declared.
    pub fn new(store: Store, labels: BTreeMap<Label, Uid>) -> App {
        let names = labels.into_iter().map(|(l, u)| (u, l)).collect();
        let order: Vec<Uid> = store.units().map(|(u, _)| *u).collect();
        let salience = salience(&store, &SalienceRequest::default());

        let est = Estimator::default();
        let full: u64 = order
            .iter()
            .filter_map(|u| store.get(u))
            .map(|u| {
                let level = smysl_pack::available_levels(&u.core)
                    .last()
                    .copied()
                    .unwrap_or(Lod::L0);
                est.unit(&u.core, level)
            })
            .sum();

        let mut app = App {
            store,
            names,
            order,
            salience,
            pane: Pane::Graph,
            cursor: 0,
            budget: full,
            focus: BTreeSet::new(),
            full,
            sim: Sim::Infeasible { required: 0 },
            help: false,
            quit: false,
        };
        app.resimulate();
        app
    }

    // -- reads ------------------------------------------------------------

    pub fn store(&self) -> &Store {
        &self.store
    }
    pub fn pane(&self) -> Pane {
        self.pane
    }
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn budget(&self) -> u64 {
        self.budget
    }
    pub fn full_cost(&self) -> u64 {
        self.full
    }
    pub fn sim(&self) -> &Sim {
        &self.sim
    }
    pub fn order(&self) -> &[Uid] {
        &self.order
    }
    pub fn help_open(&self) -> bool {
        self.help
    }
    pub fn should_quit(&self) -> bool {
        self.quit
    }
    pub fn salience_of(&self, uid: &Uid) -> f32 {
        self.salience.get(uid)
    }

    /// The unit under the cursor, if the graph is not empty.
    pub fn selected(&self) -> Option<&Uid> {
        self.order.get(self.cursor)
    }

    /// A short human-facing name: the authored label where there is one, and a uid prefix
    /// where there is not. A uid is the identity; a label is what someone typed.
    pub fn name(&self, uid: &Uid) -> String {
        match self.names.get(uid) {
            Some(l) => l.as_str().to_string(),
            None => {
                let s = uid.to_string();
                s.chars().take(14).collect()
            }
        }
    }

    // -- transitions ------------------------------------------------------

    /// Apply one action. The only way state changes.
    pub fn update(&mut self, action: Action) {
        match action {
            Action::Quit => self.quit = true,
            Action::ToggleHelp => self.help = !self.help,
            Action::NextPane => self.pane = step_pane(self.pane, 1),
            Action::PrevPane => self.pane = step_pane(self.pane, -1),
            Action::Down => self.move_cursor(1),
            Action::Up => self.move_cursor(-1),
            Action::Top => self.cursor = 0,
            Action::Bottom => self.cursor = self.order.len().saturating_sub(1),
            Action::BudgetUp => self.set_budget(self.budget.saturating_add(self.step())),
            Action::BudgetDown => self.set_budget(self.budget.saturating_sub(self.step())),
            Action::BudgetReset => self.set_budget(self.full),
            Action::ToggleFocus => self.toggle_focus(),
            Action::Ignored => {}
        }
    }

    /// A tenth of the whole graph, so the dial crosses the interesting range in ten presses
    /// whatever the corpus size, and never steps by zero.
    fn step(&self) -> u64 {
        (self.full / 10).max(1)
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.order.is_empty() {
            return;
        }
        let last = self.order.len() - 1;
        self.cursor = match delta {
            d if d < 0 => self.cursor.saturating_sub(d.unsigned_abs()),
            d => (self.cursor + d as usize).min(last),
        };
    }

    /// Pinned units, for the simulator and the graph's markers.
    pub fn focus(&self) -> &BTreeSet<Uid> {
        &self.focus
    }

    fn toggle_focus(&mut self) {
        let Some(uid) = self.selected().copied() else {
            return;
        };
        if !self.focus.insert(uid) {
            self.focus.remove(&uid);
        }
        self.resimulate();
    }

    fn set_budget(&mut self, budget: u64) {
        let clamped = budget.min(self.full);
        if clamped == self.budget {
            return;
        }
        self.budget = clamped;
        self.resimulate();
    }

    /// Re-run the pack at the current budget.
    ///
    /// Eagerly, on every change, rather than lazily at draw time: the pane's whole purpose
    /// is that the answer moves while you hold the key down, and a draw that computed it
    /// would recompute on every unrelated repaint too.
    fn resimulate(&mut self) {
        let mut req = PackRequest::default();
        req.budget = self.budget;
        req.estimator = Estimator::default();
        req.focus = self.focus.clone();

        self.sim = match pack(&self.store, &self.salience, &req) {
            Ok(p) => Sim::Packed {
                used: p.info.used,
                dropped: p.info.dropped.len(),
                degraded: p.info.degraded.len(),
                selected: p.selection,
                why: p.why,
            },
            Err(smysl_core::error::PackError::Infeasible { required, .. }) => {
                Sim::Infeasible { required }
            }
            // Any other refusal leaves nothing selected and no floor to report; the pane
            // says so rather than showing a stale selection from a previous budget.
            Err(_) => Sim::Infeasible { required: 0 },
        };
    }

    /// Units the selected one rests on, for the lineage pane.
    pub fn grounds_of(&self, uid: &Uid) -> BTreeSet<Uid> {
        self.store
            .get(uid)
            .map(|u| u.core.grounds.clone())
            .unwrap_or_default()
    }

    pub fn deps_of(&self, uid: &Uid) -> BTreeSet<Uid> {
        self.store
            .get(uid)
            .map(|u| u.core.deps.clone())
            .unwrap_or_default()
    }
}

fn step_pane(pane: Pane, delta: i32) -> Pane {
    let all = Pane::ALL;
    let i = all.iter().position(|p| *p == pane).unwrap_or(0) as i32;
    let n = all.len() as i32;
    all[(((i + delta) % n + n) % n) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Record, Status, UnitCoreBuilder};

    fn unit(gist: &str, body: Option<&str>) -> Record {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative);
        if let Some(t) = body {
            b = b.body(t);
        }
        Record::Unit(b.build().unwrap())
    }

    fn app() -> App {
        let records = vec![
            unit("the first claim", Some(&"word ".repeat(60))),
            unit("the second claim", Some(&"word ".repeat(60))),
            unit("the third claim", None),
        ];
        App::new(Store::from_records(records), BTreeMap::new())
    }

    #[test]
    fn panes_cycle_in_both_directions_and_wrap() {
        let mut a = app();
        assert_eq!(a.pane(), Pane::Graph);
        for _ in 0..Pane::ALL.len() {
            a.update(Action::NextPane);
        }
        assert_eq!(a.pane(), Pane::Graph, "a full cycle returns to the start");

        a.update(Action::PrevPane);
        assert_eq!(a.pane(), *Pane::ALL.last().unwrap(), "backwards wraps too");
    }

    /// A cursor that runs off either end is the commonest way a list UI panics.
    #[test]
    fn the_cursor_stays_inside_the_list() {
        let mut a = app();
        for _ in 0..50 {
            a.update(Action::Down);
        }
        assert_eq!(a.cursor(), 2, "clamped at the last unit");
        for _ in 0..50 {
            a.update(Action::Up);
        }
        assert_eq!(a.cursor(), 0, "clamped at the first");
    }

    #[test]
    fn an_empty_graph_does_not_move_or_panic() {
        let mut a = App::new(Store::new(), BTreeMap::new());
        a.update(Action::Down);
        a.update(Action::Bottom);
        assert_eq!(a.cursor(), 0);
        assert!(a.selected().is_none());
    }

    /// The dial's whole point: narrowing the budget has to change the answer.
    #[test]
    fn narrowing_the_budget_costs_something() {
        let mut a = app();
        let full = match a.sim() {
            Sim::Packed { selected, .. } => selected.len(),
            Sim::Infeasible { .. } => panic!("the full budget must be feasible"),
        };
        assert_eq!(full, 3, "everything fits at full cost");

        for _ in 0..7 {
            a.update(Action::BudgetDown);
        }
        assert!(a.budget() < a.full_cost());
        match a.sim() {
            Sim::Packed {
                selected,
                degraded,
                dropped,
                ..
            } => assert!(
                selected.len() < full || *degraded > 0 || *dropped > 0,
                "a narrower budget changed nothing"
            ),
            Sim::Infeasible { .. } => {} // also a change, and a meaningful one
        }
    }

    #[test]
    fn the_budget_never_exceeds_the_whole_graph() {
        let mut a = app();
        for _ in 0..20 {
            a.update(Action::BudgetUp);
        }
        assert_eq!(a.budget(), a.full_cost());
    }

    #[test]
    fn reset_returns_to_the_whole_graph() {
        let mut a = app();
        for _ in 0..5 {
            a.update(Action::BudgetDown);
        }
        a.update(Action::BudgetReset);
        assert_eq!(a.budget(), a.full_cost());
    }

    /// Ctrl-C must not be swallowed by a full-screen program.
    #[test]
    fn the_usual_quit_keys_all_quit() {
        for key in [
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            assert_eq!(Action::from_key(key), Action::Quit, "{key:?}");
        }
    }

    #[test]
    fn an_unmapped_key_is_ignored_rather_than_guessed() {
        let k = KeyEvent::new(KeyCode::Char('z'), KeyModifiers::NONE);
        assert_eq!(Action::from_key(k), Action::Ignored);

        let mut a = app();
        let before = (a.pane(), a.cursor(), a.budget());
        a.update(Action::Ignored);
        assert_eq!((a.pane(), a.cursor(), a.budget()), before);
    }

    #[test]
    fn quitting_is_the_only_way_to_set_the_quit_flag() {
        let mut a = app();
        for action in [
            Action::NextPane,
            Action::Down,
            Action::BudgetDown,
            Action::ToggleHelp,
            Action::Ignored,
        ] {
            a.update(action);
            assert!(!a.should_quit(), "{action:?} set the quit flag");
        }
        a.update(Action::Quit);
        assert!(a.should_quit());
    }

    #[test]
    fn a_unit_without_a_label_is_named_by_its_uid() {
        let a = app();
        let name = a.name(a.selected().unwrap());
        assert!(name.starts_with("b3:"), "{name}");
        assert!(name.len() <= 14);
    }
}
