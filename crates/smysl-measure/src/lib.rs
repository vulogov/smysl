//! smysl measures (RFC SMYSL-2.6): a measure is a pure function of a store, a scope and a
//! recipe, and its result is a `@data` unit whose identity is that function's output.
//!
//! **TX-P8 M1 is the frame, and it holds no measure.** What is here is everything a measure
//! needs in order to have a result worth writing down: a [`Recipe`] that names every input
//! and hashes to the same 32 bytes on every machine; payload numbers that are integers
//! ([`Num`]), because a payload float is binary32 to 1/1024; transcendentals through `libm`
//! ([`num`]); a seeded generator ([`rng`]); a scope resolved by sq and saved as the result's
//! ground ([`scope`]); and the unit writer ([`to_unit`]). T1 to T7 arrive in M2 to M6, each as
//! one [`Measure`] in [`ALL`], and M7 is the `measure` command.
//!
//! # A result as a unit (SMYSL-2.6 §3.6)
//!
//! Schema `data`, status `derived`, grounded on the scope's [`scope::Basis`] and on nothing
//! else, no `deps`, no `source`. The payload is the recipe and the value; the gist is the
//! headline value rendered from its stored integer. Same recipe, same scope, same data: same
//! core, same uid, and the store deduplicates the append, so a rerun adds an attestation and
//! nothing else. A changed input is a new unit beside the old one, and an old result is never
//! overwritten.
//!
//! # Determinism (§3.5)
//!
//! Every accumulation runs in a fixed order, so this crate has no `HashMap` at all — the RFC
//! allows one for lookup and asks for a lint against iterating it, and having none is the
//! version of that lint which cannot be argued with. Floats accumulate in `f64`, `ln` and
//! `exp` come from `libm` (`clippy.toml` refuses the `f64` methods), and quantisation happens
//! once, into the payload. Nothing reads the clock, the environment or an unseeded source of
//! randomness; `xtask check-purity` greps for all three.

#![forbid(unsafe_code)]

use smysl_core::diag::{Code, Diagnostic};
use smysl_core::{KernelType, Status, UnitCore, UnitCoreBuilder};
use smysl_graph::Store;

pub mod num;
pub mod outcome;
pub mod recipe;
pub mod rng;
pub mod scope;

pub use outcome::{gist, to_payload, Ci, Num, Outcome, Refusal, Value};
pub use recipe::{Axis, ClassPolicy, Level, Param, Recipe, RecipeError};
pub use scope::{digest, resolve, Basis, Resolved};

/// Whether a measure calls a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Purity {
    Pure,
    ModelDependent,
}

/// What a measure reads, so that a run of several computes each shared input once. A set of
/// bits rather than a bitflags type, which would be a dependency for six constants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Needs(u8);

impl Needs {
    pub const NONE: Needs = Needs(0);
    pub const TEXT: Needs = Needs(1);
    pub const UNITS: Needs = Needs(1 << 1);
    pub const CLASSES: Needs = Needs(1 << 2);
    pub const TIME: Needs = Needs(1 << 3);
    pub const SALIENCE: Needs = Needs(1 << 4);
    pub const PACK: Needs = Needs(1 << 5);

    pub const fn union(self, other: Needs) -> Needs {
        Needs(self.0 | other.0)
    }

    pub const fn contains(self, other: Needs) -> bool {
        self.0 & other.0 == other.0
    }
}

/// What a measure is given. The text substrate, classes and the time index join it with the
/// measures that read them (M2 onward); constructed through [`Input::new`] so adding them
/// breaks no caller.
#[non_exhaustive]
pub struct Input<'a> {
    pub store: &'a Store,
    pub scope: &'a Resolved,
}

impl<'a> Input<'a> {
    pub fn new(store: &'a Store, scope: &'a Resolved) -> Input<'a> {
        Input { store, scope }
    }
}

/// One measure (SMYSL-2.6 §3.3).
pub trait Measure: Sync {
    /// `T2`, `A8`, `K3`.
    fn id(&self) -> &'static str;
    /// Bumped whenever the formula or a default changes. Part of the recipe, so a new version
    /// is a new unit beside the old one rather than a different value under the same uid.
    fn version(&self) -> u16;
    fn purity(&self) -> Purity;
    fn needs(&self) -> Needs;
    /// The refusals that need no computation — a lossy text, a missing policy, an undated
    /// scope. Pure and cheap; runs before [`Measure::run`].
    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal>;
    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome;
}

/// Every measure this build has, in report order. Empty in M1: the frame comes first.
pub const ALL: &[&dyn Measure] = &[];

/// A measure by id and version, against a registry.
///
/// The version matters on the way back in: a recipe read from a payload names the version it
/// was computed under, and a build whose measure has moved on cannot recompute it, so the
/// answer is `SMY-E489` rather than a value under a recipe that no longer describes it.
pub fn find<'r>(
    registry: &[&'r dyn Measure],
    id: &str,
    v: Option<u16>,
) -> Result<&'r dyn Measure, Diagnostic> {
    let Some(m) = registry.iter().find(|m| m.id() == id) else {
        let known: Vec<&str> = registry.iter().map(|m| m.id()).collect();
        return Err(
            Diagnostic::new(Code::E489).with_message(if known.is_empty() {
                format!("no measure `{id}`: this build has none yet")
            } else {
                format!("no measure `{id}`; this build has {}", known.join(", "))
            }),
        );
    };
    match v {
        Some(v) if v != m.version() => Err(Diagnostic::new(Code::E489).with_message(format!(
            "`{id}` v{v} is not this build's `{id}`, which is v{}; its result cannot be recomputed here",
            m.version()
        ))),
        _ => Ok(*m),
    }
}

/// One measure's result: the recipe it ran under, what it gave, and what was said on the way.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct MeasureResult {
    pub recipe: Recipe,
    pub outcome: Outcome,
    pub diagnostics: Vec<Diagnostic>,
}

/// Run `ids` over `input` against [`ALL`]. See [`run_with`].
pub fn run(ids: &[&str], base: &Recipe, input: &Input<'_>) -> Vec<MeasureResult> {
    run_with(ALL, ids, base, input)
}

/// Run `ids` in the order given, each under `base` with its own id and version and the
/// scope's digest. An unknown id is a result too — not run, `SMY-E489` — so the report lists
/// every id asked for and a typo is visible beside the measures that ran.
pub fn run_with(
    registry: &[&dyn Measure],
    ids: &[&str],
    base: &Recipe,
    input: &Input<'_>,
) -> Vec<MeasureResult> {
    ids.iter()
        .map(|id| match find(registry, id, None) {
            Err(d) => MeasureResult {
                recipe: base.for_measure(*id, 0),
                outcome: Outcome::NotRun {
                    why: d.message.clone(),
                    code: Some(d.code),
                },
                diagnostics: vec![d],
            },
            Ok(m) => {
                let mut recipe = base.for_measure(m.id(), m.version());
                recipe.scope_digest = input.scope.digest;
                match m.admit(&recipe, input) {
                    Err(r) => MeasureResult {
                        diagnostics: vec![r.diagnostic()],
                        outcome: Outcome::refused(r),
                        recipe,
                    },
                    Ok(()) => MeasureResult {
                        outcome: m.run(&recipe, input),
                        diagnostics: Vec::new(),
                        recipe,
                    },
                }
            }
        })
        .collect()
}

/// Why a result is not a unit.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum NotPersisted {
    /// The measure did not run; the report says why.
    NotRun,
    /// It ran, and the scope has no basis to ground it on (`SMY-W483`).
    NoBasis(Diagnostic),
    /// The value could not be written: a count past a payload integer (`SMY-E489`).
    Unwritable(Diagnostic),
}

/// The `@data` unit a result is (SMYSL-2.6 §3.6), grounded on `basis`.
pub fn to_unit(result: &MeasureResult, basis: Option<&Basis>) -> Result<UnitCore, NotPersisted> {
    let Outcome::Value(value) = &result.outcome else {
        return Err(NotPersisted::NotRun);
    };
    let Some(basis) = basis else {
        return Err(NotPersisted::NoBasis(
            Diagnostic::new(Code::W483).with_message(format!(
                "{} was computed over `{}`, which is not a saved scope, so there is no unit to \
             ground it on; the value is reported and not written",
                result.recipe.id,
                result.recipe.scope.trim()
            )),
        ));
    };
    let payload =
        to_payload(&result.recipe, value).map_err(|e| NotPersisted::Unwritable(e.diagnostic()))?;
    let full = gist(&result.recipe, value);
    UnitCoreBuilder::new(KernelType::Data, fit(&full), Status::Derived)
        .grounds([basis.uid])
        .payload(payload)
        .build()
        .map_err(|e| {
            NotPersisted::Unwritable(
                Diagnostic::new(Code::E489)
                    .with_message(format!("the unit could not be built: {e}")),
            )
        })
}

/// The gist cut to the default profile's bound, at a word, with an ellipsis — the scope is
/// last, so it is what is cut.
fn fit(full: &str) -> String {
    let g = smysl_core::GranularityProfile::default();
    if g.tokens(full).is_some_and(|n| g.gist_within_bound(n)) {
        return full.to_string();
    }
    let end = g.fit_gist_with(full, "\u{2026}").unwrap_or(0);
    let cut = &full[..end];
    let cut = cut
        .rfind(char::is_whitespace)
        .filter(|&i| i > 0)
        .map_or(cut, |i| &cut[..i]);
    format!("{}\u{2026}", cut.trim_end_matches([',', ' ']))
}
