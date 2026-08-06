//! The Render IR (§20).
//!
//! Two stages, and this is the seam between them: graph + thread + profile → **IR** →
//! backend. Everything that requires knowing about the graph happens before the IR;
//! everything that requires knowing about a file format happens after it. A backend that
//! needed the store would be a backend that could disagree with another backend about what
//! the document says.
//!
//! Rule V2 is decided here rather than in each backend, for the same reason: a contention
//! that appears in markdown and not in Typst would make the suppression a property of the
//! file format rather than of the profile.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::{Contention, ContentionId, Lod, RelKind, Role, Status, Thread, Uid};
use smysl_graph::merge::contention::detect;
use smysl_graph::{DetectionContext, Store};

use crate::connective;
use crate::profile::{Connectives, Contentions, Profile, Provenance, StatusDisplay, Verbosity};

/// One rendered unit.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Block {
    pub role: Role,
    pub uid: Uid,
    pub level: Lod,
    pub text: String,
    pub status: Status,
    /// The profile's rendering of `status`. Non-empty and status-distinguishing, because
    /// rule V1 was enforced when the profile loaded.
    pub marker: String,
    pub connective: Option<&'static str>,
    pub notes: Vec<Note>,
}

/// Apparatus travelling with a block.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Note {
    pub kind: NoteKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum NoteKind {
    /// Where the claim came from: its source reference, or who attested it.
    Provenance,
    /// An open disagreement touching this unit (rule V2).
    Contention,
    /// A ground or dep the artifact does not itself render.
    Elsewhere,
}

/// What the artifact must say about itself.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct RenderMeta {
    pub profile: String,
    pub audience: Option<String>,
    /// Rule V2: set when the profile suppressed contentions that exist. It travels into
    /// every artifact, so a suppressed disagreement is always recoverable from the output.
    pub contentions_suppressed: bool,
    pub open_contentions: Vec<ContentionId>,
    pub thread: String,
    pub schema: String,
}

/// The intermediate representation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Ir {
    pub blocks: Vec<Block>,
    pub meta: RenderMeta,
    /// The thread's own gist, for a title.
    pub gist: String,
}

impl Block {
    /// The connective and the text, joined as one reads them.
    ///
    /// A connective turns the gist into a continuation, so the sentence it continues must
    /// not open with a capital: "As a result, The pool saturated" is not English. The
    /// lowering is deliberately timid - it applies only when the first word is a plain
    /// capitalised word, so `IEEE`, `SLO` and `p99` are left exactly as written. Guessing
    /// harder would mean guessing wrong on the names that matter most.
    pub fn joined(&self) -> String {
        let Some(c) = self.connective.filter(|c| !c.is_empty()) else {
            return self.text.clone();
        };
        let mut out = String::with_capacity(c.len() + self.text.len());
        out.push_str(c);
        out.push_str(&lower_lead(&self.text));
        out
    }
}

/// Lowercase the first character, but only for an ordinary capitalised word.
fn lower_lead(text: &str) -> String {
    let word = text.split_whitespace().next().unwrap_or_default();
    let ordinary = word.chars().next().is_some_and(char::is_uppercase)
        && word.chars().skip(1).all(|c| !c.is_uppercase())
        && word
            .chars()
            .all(|c| c.is_alphabetic() || c == '\'' || c == '-');
    if !ordinary {
        return text.to_string();
    }
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

impl Ir {
    /// Whether rule V2 requires the backend to emit contentions in the body.
    pub fn must_show_contentions(&self) -> bool {
        !self.meta.open_contentions.is_empty() && !self.meta.contentions_suppressed
    }

    pub fn units(&self) -> BTreeSet<Uid> {
        self.blocks.iter().map(|b| b.uid).collect()
    }
}

/// How to build the IR.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BuildOptions {
    /// Cap every block at this level, whatever the profile says. `--lod` on the CLI.
    pub lod_cap: Option<Lod>,
    /// Override the profile's contention setting. `--contentions` on the CLI.
    pub contentions: Option<Contentions>,
    /// Whether to detect contentions the store has not written down.
    ///
    /// On by default, and it has to be: merge *reports* detections rather than recording
    /// them (§5.4, so that merge stays associative), so a store almost never holds a
    /// `Contention` record. A renderer that only surfaced written-down contentions would
    /// make rule V2 vacuous in exactly the case it exists for - a live rebuttal nobody
    /// bothered to materialise.
    pub detect_contentions: bool,
}

impl Default for BuildOptions {
    fn default() -> BuildOptions {
        BuildOptions {
            lod_cap: None,
            contentions: None,
            detect_contentions: true,
        }
    }
}

/// Build the IR from a graph, a thread, and a profile (§20 stage one).
pub fn build(store: &Store, thread: &Thread, profile: &Profile, opts: &BuildOptions) -> Ir {
    let show_contentions = opts.contentions.unwrap_or(profile.show.contentions);
    let rendered: BTreeSet<Uid> = thread.units().copied().collect();

    // Rule V2. The contention set is computed before suppression, so the metadata can
    // report what was hidden rather than merely that something was.
    //
    // A fixed clock, because rendering is pure (rule D) and a contention's identity is
    // derived from its content rather than from when it was noticed.
    let mut all: Vec<Contention> = store.contentions().to_vec();
    if opts.detect_contentions {
        for c in detect(store, &DetectionContext::default()) {
            if !all.iter().any(|existing| existing.id == c.id) {
                all.push(c);
            }
        }
        all.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
    }

    let open: Vec<&Contention> = all
        .iter()
        .filter(|c| c.is_open())
        .filter(|c| match show_contentions {
            // `on-rendered` narrows *which* contentions are surfaced; it never hides one
            // that touches a unit the reader can see.
            Contentions::OnRendered => {
                rendered.contains(&c.over) || c.positions.iter().any(|p| rendered.contains(p))
            }
            _ => true,
        })
        .collect();

    let meta = RenderMeta {
        profile: profile.name.clone(),
        audience: profile.audience.clone(),
        contentions_suppressed: show_contentions == Contentions::Suppress && !open.is_empty(),
        open_contentions: open.iter().map(|c| c.id.clone()).collect(),
        thread: thread.id.to_string(),
        schema: thread.schema.to_string(),
    };

    let by_unit = contention_notes(&open, show_contentions);
    let joins = joining_kinds(store, thread);

    let mut blocks = Vec::new();
    for (i, step) in thread.steps.iter().enumerate() {
        let Some(unit) = store.get(&step.unit) else {
            // A thread naming a unit the store does not hold is a broken thread, not a
            // render failure: the other steps still say something true.
            continue;
        };

        let level = match opts.lod_cap {
            Some(cap) => profile.lod.for_role(step.role).min(cap),
            None => profile.lod.for_role(step.role),
        };
        let level = available(&unit.core, level);

        let connective = match (profile.connectives, i) {
            (Connectives::None, _) | (_, 0) => None,
            _ => joins
                .get(&step.unit)
                .and_then(|k| connective::select(k, &step.unit)),
        };

        let mut notes = Vec::new();
        if profile.show.provenance != Provenance::None {
            if let Some(s) = &unit.core.source {
                notes.push(Note {
                    kind: NoteKind::Provenance,
                    text: format!("{}: {}", s.kind, s.reference),
                });
            }
            // Who stood behind it. Agents are listed in id order, so the note is a
            // function of the store rather than of attestation arrival.
            let agents: BTreeSet<String> = unit
                .attestations
                .iter()
                .map(|a| format!("{} ({})", a.agent, a.rung))
                .collect();
            if !agents.is_empty() {
                notes.push(Note {
                    kind: NoteKind::Provenance,
                    text: agents.into_iter().collect::<Vec<_>>().join(", "),
                });
            }
        }
        notes.extend(by_unit.get(&step.unit).cloned().unwrap_or_default());

        // Grounds the reader cannot see. Rule L guarantees the *deps* are present, which
        // is what makes the text interpretable; grounds are what makes it checkable, and
        // an absent one is worth saying out loud rather than quietly omitting.
        let elsewhere: Vec<String> = unit
            .core
            .grounds
            .iter()
            .filter(|g| !rendered.contains(g))
            .map(|g| g.to_string())
            .collect();
        if !elsewhere.is_empty() && profile.verbosity == Verbosity::Full {
            notes.push(Note {
                kind: NoteKind::Elsewhere,
                text: format!("rests on {}", elsewhere.join(", ")),
            });
        }

        notes.truncate(profile.verbosity.note_budget());

        blocks.push(Block {
            role: step.role,
            uid: step.unit,
            level,
            text: text_at(&unit.core, level),
            status: unit.core.status,
            marker: profile.marker(unit.core.status).to_string(),
            connective,
            notes,
        });
    }

    Ir {
        blocks,
        meta,
        gist: thread.gist.clone(),
    }
}

/// The relation kind joining each step to the step before it.
///
/// Only edges *between consecutive steps* count. An edge to some other part of the thread
/// would produce a connective that promises a transition the text does not make.
fn joining_kinds(store: &Store, thread: &Thread) -> BTreeMap<Uid, RelKind> {
    let mut out = BTreeMap::new();
    for pair in thread.steps.windows(2) {
        let (prev, next) = (pair[0].unit, pair[1].unit);
        // Relations are scanned in a fixed order and the first match wins, so a pair of
        // units joined by two kinds always picks the same one.
        let mut found: Option<RelKind> = None;
        for r in store.relations() {
            if (r.from == next && r.to == prev) || (r.from == prev && r.to == next) {
                let better = match &found {
                    None => true,
                    Some(k) => r.kind.to_string() < k.to_string(),
                };
                if better {
                    found = Some(r.kind.clone());
                }
            }
        }
        if let Some(k) = found {
            out.insert(next, k);
        }
    }
    out
}

fn contention_notes(open: &[&Contention], show: Contentions) -> BTreeMap<Uid, Vec<Note>> {
    let mut out: BTreeMap<Uid, Vec<Note>> = BTreeMap::new();
    if show == Contentions::Suppress {
        return out;
    }
    for c in open {
        let note = Note {
            kind: NoteKind::Contention,
            text: format!(
                "{}: contested, {} position(s) on record",
                c.id,
                c.positions.len()
            ),
        };
        out.entry(c.over).or_default().push(note.clone());
        for p in &c.positions {
            out.entry(*p).or_default().push(note.clone());
        }
    }
    out
}

/// The deepest level a unit actually has at or below `want`.
///
/// Asking for L2 from a unit with only a gist must not produce an empty block; the level
/// recorded in the IR is the one the text is really at, so an artifact never claims more
/// detail than it carries.
fn available(core: &smysl_core::UnitCore, want: Lod) -> Lod {
    match want {
        Lod::L2 if core.detail.is_some() => Lod::L2,
        Lod::L2 | Lod::L1 if core.body.is_some() => Lod::L1,
        _ => Lod::L0,
    }
}

fn text_at(core: &smysl_core::UnitCore, level: Lod) -> String {
    let mut out = core.gist.clone();
    if level >= Lod::L1 {
        if let Some(b) = &core.body {
            out.push_str("\n\n");
            out.push_str(b);
        }
    }
    if level >= Lod::L2 {
        if let Some(d) = &core.detail {
            out.push_str("\n\n");
            out.push_str(d);
        }
    }
    out
}

/// Whether a status display would flatten. Kept here so a caller that builds a profile by
/// hand can ask the same question `Profile::load` asks.
pub const fn flattens(display: StatusDisplay) -> bool {
    matches!(display, StatusDisplay::None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{
        canonical_uid, AgentId, Attestation, Contention, ContentionId, Detected, DetectionKind,
        Hlc, KernelType, Op, Record, Relation, Rung, SourceKind, SourceRef, Step, ThreadId,
        ThreadSchema, UnitCore, UnitCoreBuilder,
    };

    fn core(gist: &str, status: Status) -> UnitCore {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, gist, status);
        if matches!(status, Status::Measured | Status::Cited) {
            b = b.source(SourceRef::new(SourceKind::Metric, "m"));
        }
        b.build().unwrap()
    }

    fn layered(gist: &str) -> UnitCore {
        UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative)
            .body("the body")
            .detail("the detail")
            .build()
            .unwrap()
    }

    fn thread(steps: Vec<Step>) -> Thread {
        let a = AgentId::new("tool:test").unwrap();
        Thread::new(
            ThreadId::new("t/x").unwrap(),
            ThreadSchema::Brief,
            a.clone(),
            "the gist of it",
            Hlc::zero(a),
        )
        .with_steps(steps)
    }

    fn ir(store: &Store, t: &Thread, p: &Profile) -> Ir {
        build(store, t, p, &BuildOptions::default())
    }

    #[test]
    fn a_block_carries_the_profiles_marker_for_its_status() {
        let c = core("a speculative claim", Status::Speculative);
        let u = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(c)]);
        let p = Profile::builtin("plain").unwrap();
        let out = ir(&store, &thread(vec![Step::new(Role::BottomLine, u)]), &p);

        assert_eq!(out.blocks.len(), 1);
        assert_eq!(out.blocks[0].status, Status::Speculative);
        assert_eq!(out.blocks[0].marker, p.marker(Status::Speculative));
        assert!(!out.blocks[0].marker.is_empty());
    }

    /// The IR must never claim more detail than the unit carries, or an artifact would
    /// advertise an L2 reading of a unit that only has a gist.
    #[test]
    fn the_level_is_what_the_unit_actually_has() {
        let bare = core("only a gist", Status::Speculative);
        let ub = canonical_uid(&bare);
        let full = layered("has everything");
        let uf = canonical_uid(&full);
        let store = Store::from_records(vec![Record::Unit(bare), Record::Unit(full)]);
        let p = Profile::load("profile deep { lod: { default: L2 } }").unwrap();
        let out = ir(
            &store,
            &thread(vec![
                Step::new(Role::BottomLine, ub),
                Step::new(Role::Support, uf),
            ]),
            &p,
        );

        assert_eq!(out.blocks[0].level, Lod::L0);
        assert_eq!(out.blocks[0].text, "only a gist");
        assert_eq!(out.blocks[1].level, Lod::L2);
        assert!(out.blocks[1].text.contains("the detail"));
    }

    #[test]
    fn a_role_lod_override_reaches_the_block() {
        let a = layered("the bottom line");
        let ua = canonical_uid(&a);
        let b = layered("a risk");
        let ub = canonical_uid(&b);
        let store = Store::from_records(vec![Record::Unit(a), Record::Unit(b)]);
        let p = Profile::builtin("exec").unwrap();
        let out = ir(
            &store,
            &thread(vec![
                Step::new(Role::BottomLine, ua),
                Step::new(Role::Risk, ub),
            ]),
            &p,
        );
        assert_eq!(out.blocks[0].level, Lod::L1);
        assert_eq!(out.blocks[1].level, Lod::L0, "exec renders risk at L0");
    }

    #[test]
    fn the_lod_cap_overrides_the_profile() {
        let a = layered("deep");
        let ua = canonical_uid(&a);
        let store = Store::from_records(vec![Record::Unit(a)]);
        let p = Profile::load("profile deep { lod: { default: L2 } }").unwrap();
        let out = build(
            &store,
            &thread(vec![Step::new(Role::BottomLine, ua)]),
            &p,
            &BuildOptions {
                lod_cap: Some(Lod::L0),
                ..BuildOptions::default()
            },
        );
        assert_eq!(out.blocks[0].level, Lod::L0);
    }

    // -- connectives ---------------------------------------------------------

    #[test]
    fn a_connective_comes_from_the_edge_joining_consecutive_steps() {
        let a = core("the cause", Status::Speculative);
        let ua = canonical_uid(&a);
        let b = core("the effect", Status::Speculative);
        let ub = canonical_uid(&b);
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Causes, ub, ua)),
        ]);
        let out = ir(
            &store,
            &thread(vec![
                Step::new(Role::BottomLine, ua),
                Step::new(Role::Support, ub),
            ]),
            &Profile::builtin("plain").unwrap(),
        );
        assert_eq!(out.blocks[0].connective, None, "the first block opens");
        assert!(out.blocks[1]
            .connective
            .is_some_and(|c| c.starts_with("Consequently") || c.starts_with("As a result")));
    }

    #[test]
    fn a_profile_can_turn_connectives_off() {
        let a = core("the cause", Status::Speculative);
        let ua = canonical_uid(&a);
        let b = core("the effect", Status::Speculative);
        let ub = canonical_uid(&b);
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Relation(Relation::new(RelKind::Causes, ub, ua)),
        ]);
        let p = Profile::load("profile bare { connectives: none }").unwrap();
        let out = ir(
            &store,
            &thread(vec![
                Step::new(Role::BottomLine, ua),
                Step::new(Role::Support, ub),
            ]),
            &p,
        );
        assert!(out.blocks.iter().all(|b| b.connective.is_none()));
    }

    /// An edge between two steps that are not adjacent promises a transition the text does
    /// not make.
    #[test]
    fn a_non_adjacent_edge_yields_no_connective() {
        let a = core("first", Status::Speculative);
        let ua = canonical_uid(&a);
        let b = core("second", Status::Speculative);
        let ub = canonical_uid(&b);
        let c = core("third", Status::Speculative);
        let uc = canonical_uid(&c);
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Unit(c),
            Record::Relation(Relation::new(RelKind::Causes, uc, ua)),
        ]);
        let out = ir(
            &store,
            &thread(vec![
                Step::new(Role::BottomLine, ua),
                Step::new(Role::Support, ub),
                Step::new(Role::Support, uc),
            ]),
            &Profile::builtin("plain").unwrap(),
        );
        assert!(out.blocks[2].connective.is_none());
    }

    // -- rule V2 -------------------------------------------------------------

    fn contested() -> (Store, Uid, Uid) {
        let a = core("pool saturation", Status::Speculative);
        let ua = canonical_uid(&a);
        let b = core("index regression", Status::Speculative);
        let ub = canonical_uid(&b);
        let over = core("the disputed finding", Status::Speculative);
        let uo = canonical_uid(&over);
        let c = Contention::new(
            ContentionId::new("k/pool-vs-index").unwrap(),
            uo,
            vec![ua, ub],
            Detected::new(
                DetectionKind::LiveRebuttal,
                Hlc::zero(AgentId::new("tool:t").unwrap()),
            ),
        );
        let store = Store::from_records(vec![
            Record::Unit(a),
            Record::Unit(b),
            Record::Unit(over),
            Record::Contention(c),
        ]);
        (store, uo, ua)
    }

    #[test]
    fn an_open_contention_is_surfaced_by_default() {
        let (store, uo, _) = contested();
        let out = ir(
            &store,
            &thread(vec![Step::new(Role::BottomLine, uo)]),
            &Profile::builtin("plain").unwrap(),
        );
        assert!(out.must_show_contentions());
        assert!(!out.meta.contentions_suppressed);
        assert_eq!(out.meta.open_contentions.len(), 1);
        assert!(out.blocks[0]
            .notes
            .iter()
            .any(|n| n.kind == NoteKind::Contention));
    }

    /// **The gate.** Suppression is permitted, but it is always recorded - so a reader of
    /// the artifact can tell that something was hidden even though they cannot see it.
    #[test]
    fn a_suppressed_contention_is_still_recorded_in_the_metadata() {
        let (store, uo, _) = contested();
        let p = Profile::load("profile quiet { show: { contentions: suppress } }").unwrap();
        let out = ir(&store, &thread(vec![Step::new(Role::BottomLine, uo)]), &p);

        assert!(out.meta.contentions_suppressed, "W211 must be recordable");
        assert_eq!(out.meta.open_contentions.len(), 1, "and it must say which");
        assert!(!out.must_show_contentions());
        assert!(
            out.blocks[0]
                .notes
                .iter()
                .all(|n| n.kind != NoteKind::Contention),
            "the body stays quiet, the metadata does not"
        );
    }

    /// Suppression with nothing to suppress is not suppression, so the flag stays clear
    /// and no warning is warranted.
    #[test]
    fn suppression_of_nothing_is_not_recorded_as_suppression() {
        let c = core("uncontested", Status::Speculative);
        let u = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(c)]);
        let p = Profile::load("profile quiet { show: { contentions: suppress } }").unwrap();
        let out = ir(&store, &thread(vec![Step::new(Role::BottomLine, u)]), &p);
        assert!(!out.meta.contentions_suppressed);
        assert!(out.meta.open_contentions.is_empty());
    }

    #[test]
    fn on_rendered_keeps_a_contention_that_touches_a_rendered_unit() {
        let (store, uo, ua) = contested();
        let p = Profile::load("profile r { show: { contentions: on-rendered } }").unwrap();

        let out = ir(&store, &thread(vec![Step::new(Role::BottomLine, uo)]), &p);
        assert_eq!(out.meta.open_contentions.len(), 1, "over a rendered unit");

        let out = ir(&store, &thread(vec![Step::new(Role::Support, ua)]), &p);
        assert_eq!(out.meta.open_contentions.len(), 1, "a rendered position");
    }

    /// Merge reports detections rather than recording them (§5.4), so a store almost
    /// never holds a `Contention` record. A renderer that only surfaced written-down
    /// contentions would make rule V2 vacuous in exactly the case it exists for.
    #[test]
    fn a_live_rebuttal_is_surfaced_even_though_nobody_wrote_it_down() {
        let claim = core("the fix worked", Status::Speculative);
        let uc = canonical_uid(&claim);
        let reb = core("it regressed on tuesday", Status::Speculative);
        let ur = canonical_uid(&reb);
        let t = thread(vec![
            Step::new(Role::BottomLine, uc),
            Step::new(Role::Risk, ur),
        ]);
        let store = Store::from_records(vec![
            Record::Unit(claim),
            Record::Unit(reb),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
            Record::Thread(t.clone()),
        ]);
        assert!(store.contentions().is_empty(), "nothing was written down");

        let out = ir(&store, &t, &Profile::builtin("plain").unwrap());
        assert_eq!(out.meta.open_contentions.len(), 1, "but it is still live");
        assert!(out.must_show_contentions());
    }

    /// Detection can be turned off for a caller that has already materialised its
    /// contentions and does not want them counted twice.
    #[test]
    fn detection_can_be_turned_off() {
        let claim = core("the fix worked", Status::Speculative);
        let uc = canonical_uid(&claim);
        let reb = core("it regressed", Status::Speculative);
        let ur = canonical_uid(&reb);
        let t = thread(vec![
            Step::new(Role::BottomLine, uc),
            Step::new(Role::Risk, ur),
        ]);
        let store = Store::from_records(vec![
            Record::Unit(claim),
            Record::Unit(reb),
            Record::Relation(Relation::new(RelKind::Rebuts, ur, uc)),
            Record::Thread(t.clone()),
        ]);
        let out = build(
            &store,
            &t,
            &Profile::builtin("plain").unwrap(),
            &BuildOptions {
                detect_contentions: false,
                ..BuildOptions::default()
            },
        );
        assert!(out.meta.open_contentions.is_empty());
    }

    /// A contention that was both recorded and detected is one contention, not two.
    #[test]
    fn a_recorded_contention_is_not_counted_twice() {
        let (store, uo, _) = contested();
        let out = ir(
            &store,
            &thread(vec![Step::new(Role::BottomLine, uo)]),
            &Profile::builtin("plain").unwrap(),
        );
        let mut ids = out.meta.open_contentions.clone();
        ids.dedup();
        assert_eq!(ids.len(), out.meta.open_contentions.len());
    }

    #[test]
    fn the_build_option_overrides_the_profiles_contention_setting() {
        let (store, uo, _) = contested();
        let out = build(
            &store,
            &thread(vec![Step::new(Role::BottomLine, uo)]),
            &Profile::builtin("plain").unwrap(),
            &BuildOptions {
                contentions: Some(Contentions::Suppress),
                ..BuildOptions::default()
            },
        );
        assert!(out.meta.contentions_suppressed);
    }

    // -- notes ---------------------------------------------------------------

    #[test]
    fn provenance_notes_carry_the_source_and_the_attesting_agents() {
        let c = core("measured thing", Status::Measured);
        let u = canonical_uid(&c);
        let a = AgentId::new("model:vendor/m").unwrap();
        let store = Store::from_records(vec![
            Record::Unit(c),
            Record::Attestation(Attestation::new(
                u,
                a.clone(),
                Op::Imported,
                Rung::Computed,
                Hlc::zero(a),
            )),
        ]);
        let out = ir(
            &store,
            &thread(vec![Step::new(Role::BottomLine, u)]),
            &Profile::builtin("plain").unwrap(),
        );
        let notes = &out.blocks[0].notes;
        assert!(notes.iter().any(|n| n.text.contains("metric")));
        assert!(notes.iter().any(|n| n.text.contains("model:vendor/m")));
    }

    #[test]
    fn a_profile_can_turn_provenance_off() {
        let c = core("measured thing", Status::Measured);
        let u = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(c)]);
        let p = Profile::load("profile q { show: { provenance: none } }").unwrap();
        let out = ir(&store, &thread(vec![Step::new(Role::BottomLine, u)]), &p);
        assert!(out.blocks[0]
            .notes
            .iter()
            .all(|n| n.kind != NoteKind::Provenance));
    }

    #[test]
    fn tight_verbosity_keeps_one_note() {
        let c = core("measured thing", Status::Measured);
        let u = canonical_uid(&c);
        let a = AgentId::new("model:vendor/m").unwrap();
        let store = Store::from_records(vec![
            Record::Unit(c),
            Record::Attestation(Attestation::new(
                u,
                a.clone(),
                Op::Imported,
                Rung::Computed,
                Hlc::zero(a),
            )),
        ]);
        let p = Profile::builtin("exec").unwrap();
        let out = ir(&store, &thread(vec![Step::new(Role::BottomLine, u)]), &p);
        assert_eq!(out.blocks[0].notes.len(), 1);
    }

    // -- structure -----------------------------------------------------------

    /// A thread naming a unit the store does not hold is a broken thread, not a render
    /// failure: the steps that do resolve still say something true.
    #[test]
    fn a_step_pointing_at_nothing_is_skipped_rather_than_fatal() {
        let c = core("present", Status::Speculative);
        let u = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(c)]);
        let out = ir(
            &store,
            &thread(vec![
                Step::new(Role::BottomLine, u),
                Step::new(Role::Support, Uid::from_bytes([9; 32])),
            ]),
            &Profile::builtin("plain").unwrap(),
        );
        assert_eq!(out.blocks.len(), 1);
    }

    #[test]
    fn the_meta_names_the_profile_and_the_thread() {
        let c = core("x", Status::Speculative);
        let u = canonical_uid(&c);
        let store = Store::from_records(vec![Record::Unit(c)]);
        let out = ir(
            &store,
            &thread(vec![Step::new(Role::BottomLine, u)]),
            &Profile::builtin("exec").unwrap(),
        );
        assert_eq!(out.meta.profile, "exec");
        assert_eq!(out.meta.thread, "t/x");
        assert_eq!(out.meta.schema, "brief");
        assert_eq!(out.gist, "the gist of it");
    }

    #[test]
    fn an_empty_thread_builds_an_empty_ir() {
        let store = Store::new();
        let out = ir(&store, &thread(vec![]), &Profile::builtin("plain").unwrap());
        assert!(out.blocks.is_empty());
        assert!(!out.must_show_contentions());
    }

    #[test]
    fn building_is_deterministic() {
        let (store, uo, ua) = contested();
        let t = thread(vec![
            Step::new(Role::BottomLine, uo),
            Step::new(Role::Support, ua),
        ]);
        let p = Profile::builtin("analyst").unwrap();
        assert_eq!(ir(&store, &t, &p), ir(&store, &t, &p));
    }

    // -- joining -------------------------------------------------------------

    fn block_with(connective: Option<&'static str>, text: &str) -> Block {
        Block {
            role: Role::Support,
            uid: Uid::from_bytes([1; 32]),
            level: Lod::L0,
            text: text.into(),
            status: Status::Speculative,
            marker: "?".into(),
            connective,
            notes: Vec::new(),
        }
    }

    /// "As a result, The pool saturated" is not English.
    #[test]
    fn a_connective_lowers_an_ordinary_leading_capital() {
        let b = block_with(Some("As a result, "), "The pool saturated");
        assert_eq!(b.joined(), "As a result, the pool saturated");
    }

    /// ...but guessing harder would mean guessing wrong on the names that matter most.
    #[test]
    fn a_connective_leaves_acronyms_and_identifiers_alone() {
        for text in ["IEEE says otherwise", "SLOs were met", "eu-west recovered"] {
            let b = block_with(Some("However, "), text);
            assert!(b.joined().ends_with(text), "mangled: {}", b.joined());
        }
    }

    #[test]
    fn an_absent_or_empty_connective_leaves_the_text_untouched() {
        assert_eq!(block_with(None, "The pool").joined(), "The pool");
        assert_eq!(block_with(Some(""), "The pool").joined(), "The pool");
    }

    #[test]
    fn joining_preserves_the_rest_of_a_multi_paragraph_body() {
        let b = block_with(Some("Then, "), "The first line\n\nthe second");
        assert_eq!(b.joined(), "Then, the first line\n\nthe second");
    }

    #[test]
    fn flattening_is_exactly_the_none_display() {
        assert!(flattens(StatusDisplay::None));
        assert!(!flattens(StatusDisplay::Word));
        assert!(!flattens(StatusDisplay::InlineMarker));
    }
}
