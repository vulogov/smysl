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
        }
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
        ] {
            assert_ne!(changed.recipe(), b.recipe());
            assert_ne!(changed.family(), b.family());
        }
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
}
