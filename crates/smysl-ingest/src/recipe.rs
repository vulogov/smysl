//! Recipes (D-8, §22.5).
//!
//! ```text
//! recipe        = BLAKE3(template_id ‖ template_ver ‖ provider ‖ model ‖
//!                        granularity ‖ temperature ‖ schema_set ‖ path)
//! recipe_family = BLAKE3(template_id ‖ template_ver ‖ granularity ‖ temperature ‖
//!                        schema_set ‖ path)          # provider- and model-free
//! ```
//!
//! A model call cannot be made deterministic, but its *conditions* can be made auditable.
//! `recipe_family` drops the provider and the model precisely so E9 can aggregate the same
//! logical ingest across vendors - which is the measurement D-9 rests on.
//!
//! **Consequence for salience, and it is correctness rather than tuning:** corroboration
//! requires disjoint ancestry, and two units from the same provider under the same recipe
//! are not independent. §16.4 counts groups by `(provider, model, recipe)` for that reason.
//!
//! Fields are joined with a separator that cannot occur inside one, so
//! `("ab", "c")` and `("a", "bc")` cannot collide. Length-prefixing would do as well; a
//! forbidden byte is simpler to read in a hash input dump.

use smysl_core::hash_bytes;

use crate::IngestPath;

/// A byte that cannot appear in any field, so concatenation is injective.
const SEP: u8 = 0x1f;

/// Everything that decides what a model was asked to do.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Conditions {
    pub template_id: String,
    pub template_ver: u32,
    pub provider: String,
    pub model: String,
    pub granularity: String,
    /// Quantised before hashing, so two runs that computed 0.20000001 and 0.2 agree.
    pub temperature: f32,
    /// The schema identifiers in play, sorted - a set, not a sequence.
    pub schema_set: Vec<String>,
    pub path: IngestPath,
    /// A caller-supplied source and its policy. It decides what the output says about
    /// provenance, so two runs that differ only here are not one recipe. Absent adds nothing to
    /// the hash, so every recipe computed before this field existed is unchanged.
    pub source: Option<(smysl_core::SourceRef, smysl_core::SourcePolicy)>,
    /// The quote-check comparison form, when it is not the default (1.9).
    ///
    /// `None` and `Some(V1)` are the same run, and both add nothing to the hash: V1 is what
    /// every recipe computed before this field existed was computed under, so writing it would
    /// change every one of them for no change in what happened. Only `V2` is pushed, following
    /// the `source` precedent above.
    pub normaliser: Option<smysl_core::quote::Normaliser>,
    /// The language policy the run was asked under (1.9).
    ///
    /// `None` for a run computed before the field existed, which is every recipe up to 1.8:
    /// there was no policy then, and the templates said nothing about language. `Some` is
    /// pushed, so 1.9's runs are distinguishable from 1.8's — which they are, because the
    /// templates now carry `LANG_RULE` and the version bump says so too.
    pub lang_policy: Option<crate::LangPolicy>,
    /// The framing scheme the input was sent under (1.9).
    ///
    /// `Some("smysl/fence/1")` for every run 1.9 makes. It is recorded because a prompt
    /// override's fingerprint covers only its own text, and the framing is applied by
    /// `Template::render` rather than written in that text — so without this field an
    /// override's recipe would not change when the framing did, and the framing is part of
    /// what the model was asked under.
    pub framing: Option<&'static str>,
    /// The estimator the granularity bounds were counted with (1.9, F-2).
    ///
    /// `granularity` above is a preset *name*, and a preset says nothing about how its bounds
    /// are counted. Two runs under `default` with different estimators were asked for different
    /// gist lengths and were checked against different bounds, so they are not one recipe.
    /// `None` and `Some(Utf8Div4)` are the same run and both add nothing to the hash — the
    /// `normaliser` precedent — so every recipe computed before this field existed is unchanged.
    pub estimator: Option<smysl_core::TokenEstimator>,
    /// How many passes the window was extracted in, when it was not one (1.10, §3.4).
    ///
    /// `None` and `Some(Single)` are the same run and both add nothing to the hash — the
    /// `normaliser` precedent — so every recipe computed before consensus existed is
    /// unchanged. Only `Consensus` is pushed, and it has to be: a corpus extracted twice per
    /// window and one extracted once are not comparable, and `--single-pass` is a choice a
    /// reader of a corpus has to be able to see rather than assume.
    pub passes: Option<crate::Passes>,
    /// The structure level the windows were filled with, for a text ingest (1.10, §4.3.4).
    ///
    /// `verse` and `chapter` are different runs in the way that matters most: a window is what
    /// the model saw, so the level decides how much context every claim was extracted against.
    /// Absent for any run that is not a text ingest, which is every run before this release.
    pub window_level: Option<String>,
    /// The ledger digest **policy**: its budget and its ranking id (1.10, §3.5).
    ///
    /// The policy and never the per-window hash. The recipe groups corroboration, so a value
    /// that changed per window would put every window in a group of its own and no two
    /// extractions of one expression would ever corroborate each other. The per-window digest
    /// hash goes in the journal entry, which is what a rerun reads.
    pub digest_policy: Option<(u32, String)>,
    /// The segmenter that decided where a sentence ends (1.10).
    ///
    /// It decides where an oversized node is cut (`SMY-W447`), so two runs with different
    /// segmenters showed the model different windows.
    pub segmenter: Option<String>,
    /// The reader that produced the reading the windows were cut from (1.10).
    ///
    /// A reading is what says where the nodes are; a reader upgraded under a corpus renumbers
    /// them. Two extractions of one expression under two readers are not one recipe even though
    /// the text is identical, because the *places* are not.
    pub reader: Option<String>,
}

impl Conditions {
    pub fn new(template_id: impl Into<String>, template_ver: u32) -> Conditions {
        Conditions {
            template_id: template_id.into(),
            template_ver,
            provider: String::new(),
            model: String::new(),
            granularity: "standard".into(),
            temperature: 0.0,
            schema_set: Vec::new(),
            path: IngestPath::Surface,
            source: None,
            normaliser: None,
            lang_policy: None,
            framing: None,
            estimator: None,
            passes: None,
            window_level: None,
            digest_policy: None,
            segmenter: None,
            reader: None,
        }
    }

    pub fn with_source(
        mut self,
        s: &smysl_core::SourceRef,
        policy: smysl_core::SourcePolicy,
    ) -> Conditions {
        self.source = Some((s.clone(), policy));
        self
    }

    pub fn with_provider(mut self, p: impl Into<String>, model: impl Into<String>) -> Conditions {
        self.provider = p.into();
        self.model = model.into();
        self
    }

    pub fn with_granularity(mut self, g: impl Into<String>) -> Conditions {
        self.granularity = g.into();
        self
    }

    pub fn with_temperature(mut self, t: f32) -> Conditions {
        self.temperature = t;
        self
    }

    /// The framing scheme the input was sent under.
    pub fn with_framing(mut self, f: &'static str) -> Conditions {
        self.framing = Some(f);
        self
    }

    /// The language policy the run was asked under.
    pub fn with_lang_policy(mut self, p: crate::LangPolicy) -> Conditions {
        self.lang_policy = Some(p);
        self
    }

    /// The comparison form the quote check ran under.
    ///
    /// V1 is recorded as absent, because it is what an absent field has always meant.
    pub fn with_normaliser(mut self, n: smysl_core::quote::Normaliser) -> Conditions {
        self.normaliser = match n {
            smysl_core::quote::Normaliser::V1 => None,
            other => Some(other),
        };
        self
    }

    /// The estimator the granularity bounds were counted with.
    ///
    /// The default is recorded as absent, because it is what an absent field has always meant.
    pub fn with_estimator(mut self, e: smysl_core::TokenEstimator) -> Conditions {
        self.estimator = match e {
            smysl_core::TokenEstimator::Utf8Div4 => None,
            other => Some(other),
        };
        self
    }

    /// How many passes the window was extracted in.
    ///
    /// A single pass is recorded as absent, because it is what an absent field has always
    /// meant.
    pub fn with_passes(mut self, p: crate::Passes) -> Conditions {
        self.passes = match p {
            crate::Passes::Single => None,
            other => Some(other),
        };
        self
    }

    /// The text-ingest conditions: the window level, the digest policy, the segmenter and the
    /// reader (§4.3.4).
    ///
    /// One setter for the four, because they arrive together or not at all: they are what a
    /// *text* ingest adds, and a prose ingest has no reading to have any of them.
    pub fn for_text(
        mut self,
        window_level: impl Into<String>,
        digest_policy: Option<(u32, String)>,
        segmenter: impl Into<String>,
        reader: impl Into<String>,
    ) -> Conditions {
        self.window_level = Some(window_level.into());
        self.digest_policy = digest_policy;
        self.segmenter = Some(segmenter.into());
        self.reader = Some(reader.into());
        self
    }

    pub fn with_schemas(mut self, s: impl IntoIterator<Item = String>) -> Conditions {
        self.schema_set = s.into_iter().collect();
        self.schema_set.sort();
        self.schema_set.dedup();
        self
    }

    pub fn with_path(mut self, p: IngestPath) -> Conditions {
        self.path = p;
        self
    }

    /// The full recipe: everything, including which model ran.
    pub fn recipe(&self) -> [u8; 32] {
        let mut b = Vec::new();
        push(&mut b, self.template_id.as_bytes());
        push(&mut b, &self.template_ver.to_be_bytes());
        push(&mut b, self.provider.as_bytes());
        push(&mut b, self.model.as_bytes());
        self.push_shared(&mut b);
        hash_bytes(&b)
    }

    /// The family: the same logical ingest whoever ran it.
    pub fn family(&self) -> [u8; 32] {
        let mut b = Vec::new();
        push(&mut b, self.template_id.as_bytes());
        push(&mut b, &self.template_ver.to_be_bytes());
        self.push_shared(&mut b);
        hash_bytes(&b)
    }

    fn push_shared(&self, b: &mut Vec<u8>) {
        push(b, self.granularity.as_bytes());
        // Quantised before hashing, so two runs that computed 0.20000001 and 0.2 agree
        // about what they did - the same quantum the wire format uses everywhere else.
        push(b, &smysl_core::quantise(self.temperature).to_be_bytes());
        // Sorted and length-delimited: a set, so the order a caller happened to list the
        // schemas in cannot change the recipe.
        let mut sorted = self.schema_set.clone();
        sorted.sort();
        sorted.dedup();
        push(b, &(sorted.len() as u32).to_be_bytes());
        for s in &sorted {
            push(b, s.as_bytes());
        }
        push(b, self.path.as_str().as_bytes());
        if let Some((s, policy)) = &self.source {
            push(b, b"source");
            push(b, policy.as_str().as_bytes());
            push(b, s.kind.as_str().as_bytes());
            push(b, s.reference.as_bytes());
        }
        // Absent for V1, so every recipe computed before the field existed is unchanged. A
        // different comparison form is a different run: it decides which quotes the check
        // passes, which decides which units are staged.
        if let Some(n) = self.normaliser {
            push(b, b"normaliser");
            push(b, n.id().as_bytes());
        }
        // Absent for a recipe computed before the field existed, like `source` and
        // `normaliser` above. A policy is part of what the model was asked to do.
        if let Some(p) = self.lang_policy {
            push(b, b"lang-policy");
            push(b, p.id().as_bytes());
        }
        if let Some(f) = self.framing {
            push(b, b"framing");
            push(b, f.as_bytes());
        }
        // Absent for the default count, like `normaliser` above. It decides what the gist
        // instruction states and which gists the check accepts, so it decides what was asked
        // and what was kept.
        if let Some(e) = self.estimator {
            push(b, b"estimator");
            push(b, e.id().as_bytes());
        }
        // Absent for a single pass, like `normaliser` above. Two passes is a different run in
        // the strongest sense available: it is the policy §3.4 measures GE-T2's stability
        // under, so a figure computed across both policies would be a figure about neither.
        if let Some(p) = self.passes {
            push(b, b"passes");
            push(b, p.id().as_bytes());
        }
        // The four text-ingest fields, each absent for a run that is not one — so every recipe
        // any earlier release computed is byte for byte the recipe it was.
        if let Some(l) = &self.window_level {
            push(b, b"window-level");
            push(b, l.as_bytes());
        }
        if let Some((budget, ranking)) = &self.digest_policy {
            push(b, b"digest-policy");
            push(b, &budget.to_be_bytes());
            push(b, ranking.as_bytes());
        }
        if let Some(sg) = &self.segmenter {
            push(b, b"segmenter");
            push(b, sg.as_bytes());
        }
        if let Some(r) = &self.reader {
            push(b, b"reader");
            push(b, r.as_bytes());
        }
    }
}

fn push(out: &mut Vec<u8>, field: &[u8]) {
    // A field containing the separator would break injectivity, so it is escaped rather
    // than trusted. Nothing in practice contains 0x1f; the check costs nothing and removes
    // the need to reason about whether that stays true.
    for byte in field {
        if *byte == SEP {
            out.push(SEP);
        }
        out.push(*byte);
    }
    out.push(SEP);
}

/// The short display form of a hash, matching how uids print.
pub fn short(h: &[u8; 32]) -> String {
    smysl_core::Uid::from_bytes(*h).short()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Conditions {
        Conditions::new("ingest.surface", 1)
            .with_provider("ollama", "llama3.2")
            .with_granularity("standard")
            .with_temperature(0.0)
            .with_schemas(["smysl.kernel/0.1".to_string()])
            .with_path(IngestPath::Surface)
    }

    #[test]
    fn the_same_conditions_hash_the_same_way() {
        assert_eq!(base().recipe(), base().recipe());
        assert_eq!(base().family(), base().family());
    }

    /// The whole point of the family: E9 aggregates the same logical ingest across vendors.
    #[test]
    fn the_family_is_provider_and_model_free() {
        let a = base().with_provider("ollama", "llama3.2");
        let b = base().with_provider("deepseek", "deepseek-chat");
        assert_eq!(
            a.family(),
            b.family(),
            "the family must survive a vendor swap"
        );
        assert_ne!(a.recipe(), b.recipe(), "the recipe must not");
    }

    #[test]
    fn every_shared_field_changes_both_hashes() {
        let b = base();
        for changed in [
            b.clone().with_granularity("coarse"),
            b.clone().with_temperature(0.7),
            b.clone().with_schemas(["x.sre/incident".to_string()]),
            b.clone().with_path(IngestPath::JsonAst),
            b.clone().with_normaliser(smysl_core::quote::Normaliser::V2),
        ] {
            assert_ne!(changed.recipe(), b.recipe());
            assert_ne!(changed.family(), b.family());
        }
    }

    /// V1 adds nothing, so every recipe computed before the field existed is unchanged.
    ///
    /// The same rule as `source`: an absent field contributes no bytes. V1 is what an absent
    /// normaliser has always meant, so recording it would change every stored recipe hash for
    /// no change in what the run did — and a recipe that moves without the run moving is the
    /// one thing a recipe must not do.
    #[test]
    fn the_default_normaliser_adds_nothing() {
        let b = base();
        let explicit = b.clone().with_normaliser(smysl_core::quote::Normaliser::V1);
        assert_eq!(explicit.normaliser, None, "V1 is recorded as absent");
        assert_eq!(explicit.recipe(), b.recipe());
        assert_eq!(explicit.family(), b.family());

        // And V2 is not absent, in both hashes.
        let v2 = b.clone().with_normaliser(smysl_core::quote::Normaliser::V2);
        assert_eq!(v2.normaliser, Some(smysl_core::quote::Normaliser::V2));
        assert_ne!(v2.recipe(), b.recipe());
    }

    #[test]
    fn the_template_and_its_version_are_both_in_the_hash() {
        let b = base();
        let mut other = b.clone();
        other.template_id = "ingest.json-ast".into();
        assert_ne!(other.recipe(), b.recipe());

        let mut bumped = b.clone();
        bumped.template_ver = 2;
        assert_ne!(bumped.recipe(), b.recipe());
        assert_ne!(bumped.family(), b.family());
    }

    /// Two runs that computed 0.20000001 and 0.2 did the same thing and must say so.
    #[test]
    fn temperature_is_quantised_before_hashing() {
        let a = base().with_temperature(0.2);
        let b = base().with_temperature(0.2 + 1e-9);
        assert_eq!(a.recipe(), b.recipe());
    }

    /// The schema set is a set: the order a caller listed them in is not a condition of
    /// the run.
    #[test]
    fn the_schema_set_is_order_and_duplicate_insensitive() {
        let a = base().with_schemas(["b".to_string(), "a".to_string()]);
        let b = base().with_schemas(["a".to_string(), "b".to_string(), "a".to_string()]);
        assert_eq!(a.recipe(), b.recipe());
    }

    /// `("ab","c")` and `("a","bc")` are different conditions and must hash differently.
    #[test]
    fn field_boundaries_cannot_be_confused() {
        let a = Conditions::new("ab", 1).with_provider("c", "");
        let b = Conditions::new("a", 1).with_provider("bc", "");
        assert_ne!(a.recipe(), b.recipe());
    }

    /// A field containing the separator would break injectivity, so it is escaped rather
    /// than trusted.
    #[test]
    fn a_separator_inside_a_field_does_not_break_injectivity() {
        let a = Conditions::new("a\u{1f}b", 1);
        let b = Conditions::new("a", 1).with_provider("b", "");
        assert_ne!(a.recipe(), b.recipe());
    }

    #[test]
    fn a_recipe_is_never_its_own_family() {
        // They hash different inputs, so a caller cannot mistake one for the other.
        assert_ne!(base().recipe(), base().family());
    }

    #[test]
    fn the_short_form_reads_like_a_uid() {
        let s = short(&base().recipe());
        assert!(s.starts_with("b3:"), "{s}");
        assert_eq!(s, smysl_core::Uid::from_bytes(base().recipe()).short());
    }

    /// §16.4 counts corroboration groups by `(provider, model, recipe)`; the recipe is the
    /// part that says two units came from the same instructions.
    #[test]
    fn two_runs_of_one_pipeline_share_a_recipe() {
        let first = base();
        let second = base();
        assert_eq!(first.recipe(), second.recipe());
        assert_eq!(
            (first.provider.clone(), first.model.clone(), first.recipe()),
            (
                second.provider.clone(),
                second.model.clone(),
                second.recipe()
            )
        );
    }
    #[test]
    fn the_text_fields_are_absent_for_a_prose_ingest_and_hash_nothing() {
        let prose = base();
        let before = prose.recipe();
        let still = base();
        assert_eq!(still.recipe(), before, "a prose recipe is what it was");
        let text = base().for_text("verse", None, "smysl/sentence/1", "osis/1");
        assert_ne!(text.recipe(), before, "a text ingest is a different run");
    }

    #[test]
    fn the_window_level_the_segmenter_and_the_reader_each_change_the_recipe() {
        let at = |level: &str, seg: &str, reader: &str| {
            base().for_text(level, None, seg, reader).recipe()
        };
        let one = at("verse", "smysl/sentence/1", "osis/1");
        assert_ne!(one, at("chapter", "smysl/sentence/1", "osis/1"));
        assert_ne!(one, at("verse", "smysl/sentence/2", "osis/1"));
        assert_ne!(one, at("verse", "smysl/sentence/1", "usfm/1"));
    }

    #[test]
    fn the_digest_policy_is_in_the_recipe_and_a_per_window_hash_could_not_be() {
        let without = base().for_text("verse", None, "s/1", "osis/1").recipe();
        let small = base()
            .for_text(
                "verse",
                Some((512, "salience-then-order".to_string())),
                "s/1",
                "osis/1",
            )
            .recipe();
        let large = base()
            .for_text(
                "verse",
                Some((2048, "salience-then-order".to_string())),
                "s/1",
                "osis/1",
            )
            .recipe();
        assert_ne!(without, small);
        assert_ne!(small, large, "the budget decides what the model was shown");
        // The *policy* is stable across the windows of one run, which is what lets two
        // extractions of one expression corroborate each other at all: `family` drops the
        // provider and the model and nothing else, so a per-window value here would put every
        // window in a corroboration group of its own.
        assert_eq!(
            base()
                .for_text(
                    "verse",
                    Some((512, "salience-then-order".to_string())),
                    "s/1",
                    "osis/1"
                )
                .family(),
            base()
                .for_text(
                    "verse",
                    Some((512, "salience-then-order".to_string())),
                    "s/1",
                    "osis/1"
                )
                .with_provider("another".to_string(), "model".to_string())
                .family(),
        );
    }
}
