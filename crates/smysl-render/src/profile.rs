//! Profiles - the Voice plane (§10).
//!
//! A profile lives outside the wire format. It says how a graph should sound, never what
//! it says: register, person, verbosity, how much detail per role, and how status,
//! provenance and contentions are surfaced.
//!
//! **Rule V1 is enforced here, at load, and not at emit.** A profile that renders
//! `speculative` the way it renders `measured` never becomes a `Profile` value at all, so
//! there is no path from a flattening profile to an artifact. Failing at emit would mean
//! the flattening is discovered after the work is done, by whoever reads the output - which
//! is exactly the hop where it is least recoverable.

use std::collections::BTreeMap;

use smysl_core::error::RenderError;
use smysl_core::surface::hjson::{parse_object_prefix, HObject, HValue};
use smysl_core::{Lod, Role, Status};

/// How formal the prose reads. Carried into the artifact preamble; it never changes which
/// units appear, only how the surrounding text is worded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Register {
    Formal,
    #[default]
    Neutral,
    Plain,
}

/// Grammatical person for generated framing text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Person {
    First,
    Second,
    #[default]
    Third,
}

/// How much surrounding apparatus a block carries.
///
/// This caps *notes* rather than content: what a unit says is decided by the level of
/// detail, which is the graph's business. Verbosity decides how much footnoting travels
/// with it, which is the voice's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Verbosity {
    /// One note per block at most.
    Tight,
    #[default]
    Standard,
    /// Every note, plus the source reference inline.
    Full,
}

impl Verbosity {
    pub const fn note_budget(self) -> usize {
        match self {
            Verbosity::Tight => 1,
            Verbosity::Standard => 3,
            Verbosity::Full => usize::MAX,
        }
    }
}

/// How provenance is surfaced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Provenance {
    None,
    Inline,
    #[default]
    Footnote,
}

/// How epistemic status is surfaced. This is what rule V1 constrains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum StatusDisplay {
    /// A short glyph beside the text.
    #[default]
    InlineMarker,
    /// The status spelled out.
    Word,
    /// Nothing at all - which is epistemic flattening, and fails rule V1 at load.
    None,
}

/// How contentions are surfaced. This is what rule V2 constrains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Contentions {
    #[default]
    Always,
    /// Only where the contention touches a unit the artifact actually renders.
    OnRendered,
    /// Not at all - permitted, but recorded in the artifact metadata (`SMY-W211`).
    Suppress,
}

/// Whether connectives are drawn from relation kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Connectives {
    #[default]
    FromRelations,
    None,
}

/// What is surfaced, and how.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Show {
    pub provenance: Provenance,
    pub status: StatusDisplay,
    pub contentions: Contentions,
}

/// How much detail each role carries.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LodPlan {
    pub default: Lod,
    pub roles: BTreeMap<Role, Lod>,
}

impl Default for LodPlan {
    fn default() -> LodPlan {
        LodPlan {
            default: Lod::L1,
            roles: BTreeMap::new(),
        }
    }
}

impl LodPlan {
    pub fn for_role(&self, role: Role) -> Lod {
        self.roles.get(&role).copied().unwrap_or(self.default)
    }
}

/// A loaded profile. Constructing one is proof that rule V1 holds for it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Profile {
    pub name: String,
    pub register: Register,
    pub person: Person,
    pub verbosity: Verbosity,
    pub audience: Option<String>,
    pub lod: LodPlan,
    pub show: Show,
    pub connectives: Connectives,
    /// The resolved rendering of each status. Rule V1: total over [`Status::ALL`], and
    /// pairwise distinct.
    markers: BTreeMap<Status, String>,
}

impl Profile {
    /// Load a profile from its source text (§10).
    ///
    /// Accepts `profile NAME { … }` or a bare `{ … }`. Rule V1 is checked before the value
    /// is returned, so every `Profile` in existence renders each status distinguishably.
    pub fn load(src: &str) -> Result<Profile, RenderError> {
        let (name, body) = split_header(src);
        let obj = parse_object_prefix(body, 0)
            .map_err(|e| RenderError::Backend {
                target: "profile".into(),
                message: e.to_string(),
            })?
            .value;

        let mut p = Profile {
            name: name.unwrap_or_else(|| "unnamed".into()),
            ..Profile::plain()
        };

        if let Some(v) = obj.get("name").and_then(|v| v.value.as_str()) {
            p.name = v.to_string();
        }
        if let Some(v) = obj.get("register").and_then(|v| v.value.as_str()) {
            p.register = match v {
                "formal" => Register::Formal,
                "neutral" => Register::Neutral,
                "plain" => Register::Plain,
                other => return Err(unknown("register", other, &p.name)),
            };
        }
        if let Some(v) = obj.get("person").and_then(|v| v.value.as_str()) {
            p.person = match v {
                "first" => Person::First,
                "second" => Person::Second,
                "third" => Person::Third,
                other => return Err(unknown("person", other, &p.name)),
            };
        }
        if let Some(v) = obj.get("verbosity").and_then(|v| v.value.as_str()) {
            p.verbosity = match v {
                "tight" => Verbosity::Tight,
                "standard" => Verbosity::Standard,
                "full" => Verbosity::Full,
                other => return Err(unknown("verbosity", other, &p.name)),
            };
        }
        if let Some(v) = obj.get("audience").and_then(|v| v.value.as_str()) {
            p.audience = Some(v.to_string());
        }
        if let Some(v) = obj.get("connectives").and_then(|v| v.value.as_str()) {
            p.connectives = match v {
                "from-relations" => Connectives::FromRelations,
                "none" => Connectives::None,
                other => return Err(unknown("connectives", other, &p.name)),
            };
        }

        if let Some(o) = obj.get("lod").and_then(|v| v.value.as_object()) {
            if let Some(v) = o.get("default").and_then(|v| v.value.as_str()) {
                p.lod.default = lod(v).ok_or_else(|| unknown("lod.default", v, &p.name))?;
            }
            if let Some(roles) = o.get("roles").and_then(|v| v.value.as_object()) {
                for (k, v) in roles.iter() {
                    let role = Role::parse(&k.value)
                        .ok_or_else(|| unknown("lod.roles", &k.value, &p.name))?;
                    let s = v.value.as_str().unwrap_or_default();
                    p.lod.roles.insert(
                        role,
                        lod(s).ok_or_else(|| unknown("lod.roles", s, &p.name))?,
                    );
                }
            }
        }

        if let Some(o) = obj.get("show").and_then(|v| v.value.as_object()) {
            if let Some(v) = o.get("provenance").and_then(|v| v.value.as_str()) {
                p.show.provenance = match v {
                    "none" => Provenance::None,
                    "inline" => Provenance::Inline,
                    "footnote" => Provenance::Footnote,
                    other => return Err(unknown("show.provenance", other, &p.name)),
                };
            }
            if let Some(v) = o.get("status").and_then(|v| v.value.as_str()) {
                p.show.status = match v {
                    "inline-marker" => StatusDisplay::InlineMarker,
                    "word" => StatusDisplay::Word,
                    "none" => StatusDisplay::None,
                    other => return Err(unknown("show.status", other, &p.name)),
                };
            }
            if let Some(v) = o.get("contentions").and_then(|v| v.value.as_str()) {
                p.show.contentions = match v {
                    "always" => Contentions::Always,
                    "on-rendered" => Contentions::OnRendered,
                    "suppress" => Contentions::Suppress,
                    other => return Err(unknown("show.contentions", other, &p.name)),
                };
            }
        }

        p.markers = default_markers(p.show.status);
        if let Some(o) = obj.get("markers").and_then(|v| v.value.as_object()) {
            override_markers(&mut p.markers, o, &p.name)?;
        }

        p.enforce_v1()?;
        Ok(p)
    }

    /// Rule V1 (§10): every status must have a rendering, and no two may share one.
    ///
    /// A duplicate is reported against the *later* status in kernel order, so the message
    /// names the one that collided rather than the one that was there first.
    fn enforce_v1(&self) -> Result<(), RenderError> {
        let mut seen: BTreeMap<&str, Status> = BTreeMap::new();
        for &s in Status::ALL {
            let m = match self.markers.get(&s) {
                Some(m) if !m.trim().is_empty() => m.as_str(),
                _ => {
                    return Err(RenderError::ProfileFlattensStatus {
                        profile: self.name.clone(),
                        status: s.to_string(),
                    })
                }
            };
            if seen.insert(m, s).is_some() {
                return Err(RenderError::ProfileFlattensStatus {
                    profile: self.name.clone(),
                    status: s.to_string(),
                });
            }
        }
        Ok(())
    }

    /// How this profile renders a status. Total, by rule V1.
    pub fn marker(&self, status: Status) -> &str {
        self.markers
            .get(&status)
            .map(String::as_str)
            .unwrap_or_default()
    }

    /// A built-in profile by name.
    pub fn builtin(name: &str) -> Option<Profile> {
        let src = BUILTIN.iter().find(|(n, _)| *n == name)?.1;
        // A built-in that failed rule V1 would be a bug in this file, not in a user's
        // configuration, and `builtin_profiles_all_load` fails first if one ever does.
        Profile::load(src).ok()
    }

    pub fn builtin_names() -> Vec<&'static str> {
        BUILTIN.iter().map(|(n, _)| *n).collect()
    }

    /// The neutral profile everything else is a deviation from.
    pub fn plain() -> Profile {
        Profile {
            name: "plain".into(),
            register: Register::Neutral,
            person: Person::Third,
            verbosity: Verbosity::Standard,
            audience: None,
            lod: LodPlan {
                default: Lod::L1,
                roles: BTreeMap::new(),
            },
            show: Show::default(),
            connectives: Connectives::FromRelations,
            markers: default_markers(StatusDisplay::InlineMarker),
        }
    }
}

fn unknown(field: &str, value: &str, profile: &str) -> RenderError {
    RenderError::Backend {
        target: "profile".into(),
        message: format!("{profile}: `{value}` is not a valid {field}"),
    }
}

fn lod(s: &str) -> Option<Lod> {
    match s {
        "L0" | "l0" => Some(Lod::L0),
        "L1" | "l1" => Some(Lod::L1),
        "L2" | "l2" => Some(Lod::L2),
        _ => None,
    }
}

/// `profile exec { … }` - the name before the brace, and the object from the brace on.
fn split_header(src: &str) -> (Option<String>, &str) {
    let Some(brace) = src.find('{') else {
        return (None, src);
    };
    let head = src[..brace].trim();
    let head = head.strip_prefix("profile").unwrap_or(head).trim();
    let head = head.strip_suffix(':').unwrap_or(head).trim();
    let name = head.trim_matches('"').trim();
    if name.is_empty() {
        (None, &src[brace..])
    } else {
        (Some(name.to_string()), &src[brace..])
    }
}

fn default_markers(display: StatusDisplay) -> BTreeMap<Status, String> {
    let mut out = BTreeMap::new();
    for &s in Status::ALL {
        let m = match display {
            // Glyphs ordered by strength, so the artifact reads as a scale rather than as
            // a set of unrelated symbols.
            StatusDisplay::InlineMarker => match s {
                Status::Unfounded => "\u{2717}",
                Status::Speculative => "?",
                Status::Inferred => "\u{2248}",
                Status::Derived => "\u{22a2}",
                Status::Cited => "\u{2020}",
                Status::Measured => "\u{25aa}",
                // `Status` is non-exhaustive; a status with no glyph has no rendering, and
                // rule V1 refuses the profile rather than letting it flatten silently.
                _ => "",
            }
            .to_string(),
            StatusDisplay::Word => format!("[{s}]"),
            // Every status renders identically, which is precisely what rule V1 forbids.
            // The emptiness is what `enforce_v1` catches; nothing here needs to know why.
            _ => String::new(),
        };
        out.insert(s, m);
    }
    out
}

fn override_markers(
    markers: &mut BTreeMap<Status, String>,
    o: &HObject,
    profile: &str,
) -> Result<(), RenderError> {
    for (k, v) in o.iter() {
        let Some(s) = Status::parse(&k.value) else {
            return Err(unknown("markers", &k.value, profile));
        };
        let text = match &v.value {
            HValue::Str(t) => t.clone(),
            other => {
                return Err(unknown("markers", other.type_name(), profile));
            }
        };
        markers.insert(s, text);
    }
    Ok(())
}

/// The built-in profiles. Two of them are the RFC's own worked example (Appendix G): the
/// same graph read as an executive brief and as an analyst's trace.
static BUILTIN: &[(&str, &str)] = &[
    (
        "plain",
        r#"profile plain {
  register: neutral, person: third, verbosity: standard
  lod:  { default: L1 }
  show: { provenance: footnote, status: inline-marker, contentions: always }
  connectives: from-relations
}"#,
    ),
    (
        "exec",
        r#"profile exec {
  register: formal, person: third, verbosity: tight
  audience: "engineering leadership"
  lod:  { default: L1, roles: { bottom-line: L1, risk: L0, support: L0, ask: L0 } }
  show: { provenance: footnote, status: inline-marker, contentions: always }
  connectives: from-relations
}"#,
    ),
    (
        "analyst",
        r#"profile analyst {
  register: neutral, person: third, verbosity: full
  audience: "the person who has to check this"
  lod:  { default: L2 }
  show: { provenance: inline, status: word, contentions: always }
  connectives: from-relations
}"#,
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rfc_example_profile_loads() {
        let p = Profile::load(
            r#"profile exec {
  register: formal, person: third, verbosity: tight
  audience: "engineering leadership"
  lod:  { default: L1, roles: { bottom-line: L1, risk: L0 } }
  show: { provenance: footnote, status: inline-marker, contentions: always }
  connectives: from-relations
}"#,
        )
        .expect("the RFC's own example must load");

        assert_eq!(p.name, "exec");
        assert_eq!(p.register, Register::Formal);
        assert_eq!(p.person, Person::Third);
        assert_eq!(p.verbosity, Verbosity::Tight);
        assert_eq!(p.audience.as_deref(), Some("engineering leadership"));
        assert_eq!(p.lod.default, Lod::L1);
        assert_eq!(p.lod.for_role(Role::Risk), Lod::L0);
        assert_eq!(p.lod.for_role(Role::BottomLine), Lod::L1);
        assert_eq!(p.lod.for_role(Role::Ask), Lod::L1, "unlisted roles default");
        assert_eq!(p.show.provenance, Provenance::Footnote);
        assert_eq!(p.show.contentions, Contentions::Always);
        assert_eq!(p.connectives, Connectives::FromRelations);
    }

    // -- rule V1 ------------------------------------------------------------

    /// **The gate.** A profile that shows no status at all renders `speculative` exactly
    /// as it renders `measured`, and must not become a `Profile` value.
    #[test]
    fn a_profile_that_hides_status_fails_to_load() {
        let e = Profile::load("profile flat { show: { status: none } }")
            .expect_err("rule V1: this profile flattens");
        assert_eq!(e.code(), Some(smysl_core::Code::E210));
        match e {
            RenderError::ProfileFlattensStatus { profile, .. } => assert_eq!(profile, "flat"),
            other => panic!("wrong error: {other}"),
        }
    }

    #[test]
    fn a_profile_reusing_one_marker_for_two_statuses_fails_to_load() {
        let e = Profile::load(r#"profile sloppy { markers: { speculative: "!", measured: "!" } }"#)
            .expect_err("rule V1: two statuses share a rendering");
        assert_eq!(e.code(), Some(smysl_core::Code::E210));
        match e {
            // `measured` is the later status in kernel order, so it is the one that
            // collided rather than the one that was already there.
            RenderError::ProfileFlattensStatus { status, .. } => assert_eq!(status, "measured"),
            other => panic!("wrong error: {other}"),
        }
    }

    #[test]
    fn a_profile_blanking_one_status_fails_to_load() {
        let e = Profile::load(r#"profile gap { markers: { speculative: "  " } }"#)
            .expect_err("rule V1: speculative has no rendering");
        match e {
            RenderError::ProfileFlattensStatus { status, .. } => assert_eq!(status, "speculative"),
            other => panic!("wrong error: {other}"),
        }
    }

    /// The point of enforcing at load: there is no `Profile` value anywhere that flattens,
    /// so no backend has to check and none can be bypassed.
    #[test]
    fn every_loadable_profile_renders_every_status_distinguishably() {
        let sources = [
            "{ }",
            "profile a { show: { status: word } }",
            "profile b { show: { status: inline-marker } }",
            r#"profile c { markers: { unfounded: "gone" } }"#,
        ];
        for src in sources {
            let p = Profile::load(src).expect("loads");
            let mut seen = std::collections::BTreeSet::new();
            for &s in Status::ALL {
                let m = p.marker(s);
                assert!(!m.trim().is_empty(), "{}: {s} has no rendering", p.name);
                assert!(seen.insert(m), "{}: {s} reuses a rendering", p.name);
            }
        }
    }

    #[test]
    fn word_display_spells_the_status_out() {
        let p = Profile::load("profile w { show: { status: word } }").unwrap();
        assert_eq!(p.marker(Status::Speculative), "[speculative]");
        assert_eq!(p.marker(Status::Measured), "[measured]");
    }

    // -- built-ins -----------------------------------------------------------

    /// A built-in that failed rule V1 would be a bug here rather than in a user's
    /// configuration, and `Profile::builtin` swallows the error to return `Option`.
    #[test]
    fn builtin_profiles_all_load() {
        for name in Profile::builtin_names() {
            let src = BUILTIN.iter().find(|(n, _)| *n == name).unwrap().1;
            Profile::load(src).unwrap_or_else(|e| panic!("built-in {name}: {e}"));
            assert!(Profile::builtin(name).is_some());
        }
    }

    #[test]
    fn builtins_differ_in_more_than_name() {
        let exec = Profile::builtin("exec").unwrap();
        let analyst = Profile::builtin("analyst").unwrap();
        assert_ne!(exec.verbosity, analyst.verbosity);
        assert_ne!(exec.lod.default, analyst.lod.default);
        assert_ne!(exec.show.status, analyst.show.status);
    }

    #[test]
    fn an_unknown_builtin_is_none() {
        assert!(Profile::builtin("no-such-profile").is_none());
    }

    // -- parsing -------------------------------------------------------------

    #[test]
    fn a_bare_object_loads_without_a_name() {
        let p = Profile::load("{ verbosity: full }").unwrap();
        assert_eq!(p.name, "unnamed");
        assert_eq!(p.verbosity, Verbosity::Full);
    }

    #[test]
    fn an_explicit_name_key_wins_over_the_header() {
        let p = Profile::load("profile a { name: b }").unwrap();
        assert_eq!(p.name, "b");
    }

    #[test]
    fn an_unknown_enum_value_is_refused_rather_than_defaulted() {
        for src in [
            "profile x { register: shouty }",
            "profile x { person: fourth }",
            "profile x { verbosity: loud }",
            "profile x { connectives: from-vibes }",
            "profile x { lod: { default: L9 } }",
            "profile x { lod: { roles: { nonesuch: L1 } } }",
            "profile x { show: { provenance: maybe } }",
            "profile x { show: { status: hint } }",
            "profile x { show: { contentions: sometimes } }",
            r#"profile x { markers: { nonesuch: "!" } }"#,
            "profile x { markers: { measured: 4 } }",
        ] {
            assert!(Profile::load(src).is_err(), "`{src}` should not load");
        }
    }

    #[test]
    fn malformed_source_is_an_error_not_a_panic() {
        assert!(Profile::load("profile x { register: ").is_err());
        assert!(Profile::load("").is_err());
    }

    #[test]
    fn verbosity_caps_the_note_budget() {
        assert_eq!(Verbosity::Tight.note_budget(), 1);
        assert!(Verbosity::Standard.note_budget() < Verbosity::Full.note_budget());
    }

    #[test]
    fn a_role_lod_override_applies_only_to_that_role() {
        let p = Profile::load("profile x { lod: { default: L2, roles: { risk: L0 } } }").unwrap();
        assert_eq!(p.lod.for_role(Role::Risk), Lod::L0);
        assert_eq!(p.lod.for_role(Role::Support), Lod::L2);
    }
}
