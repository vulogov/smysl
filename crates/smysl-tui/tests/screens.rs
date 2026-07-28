//! The SM-P15 TUI gate: **every pane renders the corpus without panicking, and the pack
//! simulator says which constraint is binding.**
//!
//! Rendered into a `TestBackend` and asserted as text. A terminal UI checked only by
//! looking at it is a UI whose panes rot quietly, and the pack simulator is the one pane
//! whose output is a claim about the format rather than a decoration.

use std::collections::BTreeMap;
use std::path::PathBuf;

use smysl_graph::Store;
use smysl_tui::{render_to_string, Action, App, Pane, Sim};

fn fixture(name: &str) -> (Store, BTreeMap<smysl_core::Label, smysl_core::Uid>) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/corpus")
        .join(name);
    let src = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let out = smysl_core::surface::parse_surface(&src).unwrap();
    (Store::from_records(out.records.clone()), out.labels)
}

fn app() -> App {
    let (store, labels) = fixture("F1-incident.smy");
    App::new(store, labels)
}

/// Every pane, at a normal size and at a cruelly small one. A layout that divides by a
/// zero-height rect panics, and it panics in front of the user rather than in CI.
#[test]
fn every_pane_renders_at_every_size() {
    for pane in Pane::ALL {
        let mut a = app();
        while a.pane() != *pane {
            a.update(Action::NextPane);
        }
        for (w, h) in [(120, 40), (80, 24), (40, 10), (20, 6)] {
            let screen = render_to_string(&a, w, h);
            assert_eq!(
                screen.lines().count(),
                h as usize,
                "{pane:?} at {w}x{h} drew the wrong number of rows"
            );
        }
    }
}

/// The focused pane has to be identifiable, or the tab bar is decoration.
#[test]
fn the_focused_pane_is_named_in_the_status_line() {
    for pane in Pane::ALL {
        let mut a = app();
        while a.pane() != *pane {
            a.update(Action::NextPane);
        }
        let screen = render_to_string(&a, 120, 40);
        assert!(
            screen.contains(pane.title()),
            "{:?} is not named on screen",
            pane
        );
    }
}

/// The graph pane shows the authored labels, not just uids: a label is what someone typed
/// and a uid is what the machine computed, and a browser showing only the latter is a hex
/// dump with borders.
#[test]
fn the_graph_lists_authored_labels() {
    let screen = render_to_string(&app(), 120, 40);
    for label in ["c/regression", "e/trace", "f/root-cause"] {
        assert!(screen.contains(label), "{label} is missing from the graph");
    }
}

/// **The pane that earns the TUI.** Narrowing the budget past the mandatory floor must
/// report the floor rather than an empty selection: packing refuses to ship a claim without
/// the rebuttal that answers it, and the number that would make it fit is the useful half.
#[test]
fn the_pack_simulator_names_the_floor_when_the_budget_cannot_be_met() {
    let mut a = app();
    while a.pane() != Pane::PackSimulator {
        a.update(Action::NextPane);
    }
    // Pin a unit first. With nothing forced in, a budget of zero selects nothing and is
    // perfectly feasible - the floor only exists once something must be carried, and its
    // rebuttals must be carried with it.
    a.update(Action::ToggleFocus);
    assert_eq!(a.focus().len(), 1, "pinning did not take");

    for _ in 0..12 {
        a.update(Action::BudgetDown);
    }
    assert!(
        matches!(a.sim(), Sim::Infeasible { .. }),
        "a zero budget must be infeasible, not empty"
    );

    let screen = render_to_string(&a, 120, 40);
    assert!(screen.contains("INFEASIBLE"), "{screen}");
    assert!(
        screen.contains("floor"),
        "the floor is the number worth showing:\n{screen}"
    );
}

/// At a workable budget the pane says why the selected unit survived, in the vocabulary
/// `pack --explain` already uses.
#[test]
fn the_pack_simulator_explains_why_a_unit_was_kept() {
    let mut a = app();
    while a.pane() != Pane::PackSimulator {
        a.update(Action::NextPane);
    }
    let screen = render_to_string(&a, 120, 40);
    assert!(screen.contains("selected"), "{screen}");
    assert!(
        screen.contains("kept") || screen.contains("dropped at this budget"),
        "no verdict for the cursor:\n{screen}"
    );
}

/// Help has to be reachable and reversible, or `?` is a trap.
#[test]
fn help_opens_and_closes() {
    let mut a = app();
    a.update(Action::ToggleHelp);
    let open = render_to_string(&a, 120, 40);
    assert!(open.contains("Help"), "{open}");
    assert!(open.contains("quit"));

    a.update(Action::ToggleHelp);
    let closed = render_to_string(&a, 120, 40);
    assert!(!closed.contains("close this help"));
}

/// An empty store is a real state - `smysl ui` on a fresh directory - and must draw rather
/// than divide by zero.
#[test]
fn an_empty_store_renders() {
    let a = App::new(Store::new(), BTreeMap::new());
    let screen = render_to_string(&a, 80, 24);
    assert!(screen.contains("Graph (0 units)"), "{screen}");
}

/// Rendering must not depend on anything but state, or the screenshot tests prove nothing
/// about what a user sees.
#[test]
fn the_same_state_draws_the_same_screen() {
    let a = app();
    assert_eq!(render_to_string(&a, 100, 30), render_to_string(&a, 100, 30));
}

/// Every fixture in the corpus, through every pane. The corpus is what the format is
/// measured against, so the browser has to survive all of it - including F6, which is
/// deliberately malformed.
#[test]
fn the_whole_corpus_browses() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/corpus");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "smy") {
            continue;
        }
        let src = std::fs::read_to_string(&path).unwrap();
        let out = smysl_core::surface::parse_surface(&src).unwrap();
        let mut a = App::new(Store::from_records(out.records.clone()), out.labels);
        for _ in 0..Pane::ALL.len() {
            let screen = render_to_string(&a, 100, 30);
            assert!(!screen.trim().is_empty(), "{} drew nothing", path.display());
            a.update(Action::NextPane);
        }
        seen += 1;
    }
    assert!(seen >= 8, "only {seen} fixture(s) browsed");
}
