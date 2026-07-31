//! What `pack` actually selects, recorded, so a change to *how* it selects is visible.
//!
//! ```text
//! SMYSL_REGENERATE_GOLDEN=1 cargo test -p smysl-pack --test golden
//! ```
//!
//! The existing property tests assert that a pack is *legal*: constraints C1–C7 hold, the
//! budget is respected, value is monotone in budget, the result is deterministic and
//! independent of record order. All of that would still pass if the greedy started picking a
//! different — equally legal, differently valued — set.
//!
//! That gap matters now. The remaining performance work on `pack` is to replace the
//! per-round scan over every candidate with an ordered structure, and the whole risk of that
//! change is that the tie-break drifts. The current order is
//! `(density, salience, Reverse(uid), Reverse(level))`, and a heap that reproduces four
//! terms but not the fifth produces packs that pass every property test and are not the same
//! packs. A user would see their briefs quietly change contents.
//!
//! So this records the selection itself. It is a golden test, with everything that implies:
//! it fails on *intended* changes too, and the failure is the point — the diff says exactly
//! which unit moved, and regenerating is a deliberate act with a reviewable diff attached.
//!
//! # What it protects, measured
//!
//! Not assumed — each term of the tie-break was removed in turn to see whether this file
//! noticed:
//!
//! - **density** — protected.
//! - **`Reverse(uid)`** — protected. Flipping it fails at line 2, so density ties are common
//!   in the corpus and the uid term really does decide them.
//! - **salience** — **not protected**. Dropping the term entirely changes nothing here, so
//!   wherever two candidates tie on density they also tie on salience, and this file cannot
//!   tell the difference. The eleven property tests in `constraints.rs` pass with it removed
//!   too, which is the point: legality is not sameness.
//! - **`Reverse(level)`** — untested, for want of a fixture that reaches it.
//!
//! So the safety net for the heap rewrite is real but partial. A heap that reproduces
//! density and uid ordering and gets salience wrong would pass everything in this repository.
//! Closing that needs a fixture built to force the tie — two candidates at equal density and
//! different salience — which the corpus does not happen to contain.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use smysl_core::surface::parse_surface;
use smysl_graph::{salience, SalienceRequest, Store};
use smysl_pack::{pack, PackRequest};

const GOLDEN: &str = "tests/golden-packs.txt";

/// Every corpus fixture, at budgets that span "nothing fits" to "everything does".
///
/// The interesting behaviour is at the boundary, and 0.3.0 established that the two sides
/// are different code paths — the whole-scope fast path is taken only when the scope fits at
/// its top level, so a budget sweep that stayed on one side would exercise half the packer.
const BUDGETS: &[u64] = &[40, 120, 200, 400, 1_000, 5_000];

fn fixtures() -> Vec<(String, Store)> {
    let mut out = Vec::new();
    let dir = std::path::Path::new("../../fixtures/corpus");
    let mut paths: Vec<_> = std::fs::read_dir(dir)
        .expect("corpus directory")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "smy"))
        .collect();
    paths.sort();
    for p in paths {
        let src = std::fs::read_to_string(&p).expect("fixture reads");
        // A fixture that does not parse is not this test's problem to report.
        let Ok(parsed) = parse_surface(&src) else {
            continue;
        };
        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
        out.push((name, Store::from_records(parsed.records.clone())));
    }
    out
}

/// The record: one line per selected unit, in canonical order, with what it cost.
///
/// Uids in full rather than abbreviated. A short uid would make the file readable and would
/// also make two different units look alike in a diff, which is the one thing this file
/// exists to prevent.
fn render() -> String {
    let mut out = String::new();
    for (name, store) in fixtures() {
        let s = salience(&store, &SalienceRequest::default());
        for budget in BUDGETS {
            let req = PackRequest::budget(*budget);
            let Ok(p) = pack(&store, &s, &req) else {
                let _ = writeln!(out, "{name} budget={budget}: infeasible");
                continue;
            };
            let _ = writeln!(
                out,
                "{name} budget={budget}: {} unit(s), {} token(s), {:?}",
                p.selection.len(),
                p.used(),
                p.info.optimality.mode
            );
            let ordered: BTreeMap<_, _> = p.selection.iter().collect();
            for (uid, level) in ordered {
                let _ = writeln!(out, "    {uid} {level:?}");
            }
        }
    }
    out
}

#[test]
fn pack_selects_what_it_has_always_selected() {
    let now = render();

    if std::env::var("SMYSL_REGENERATE_GOLDEN").is_ok() {
        std::fs::write(GOLDEN, &now).expect("golden file writes");
        eprintln!("regenerated {GOLDEN}; review the diff before committing it");
        return;
    }

    let before = std::fs::read_to_string(GOLDEN).unwrap_or_else(|e| {
        panic!("{GOLDEN}: {e}\nrun with SMYSL_REGENERATE_GOLDEN=1 to create it")
    });

    if before == now {
        return;
    }

    // A useful failure: name the first line that differs rather than dumping two files and
    // leaving the reader to find it.
    let (mut b, mut n) = (before.lines(), now.lines());
    let mut line = 0usize;
    loop {
        line += 1;
        match (b.next(), n.next()) {
            (Some(x), Some(y)) if x == y => continue,
            (x, y) => panic!(
                "pack selects differently at {GOLDEN}:{line}\n  recorded: {}\n  now:      {}\n\n\
                 If this change is intended, regenerate with SMYSL_REGENERATE_GOLDEN=1 and \
                 commit the diff — it is the record of what users' packs will now contain.",
                x.unwrap_or("<end of file>"),
                y.unwrap_or("<end of file>")
            ),
        }
    }
}
