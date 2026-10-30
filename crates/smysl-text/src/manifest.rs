//! Building a manifest, and the two rules about one that are not about its keys.
//!
//! The record is `smysl-core`'s. What is here is what needs a decision rather than a field:
//! the licence list that decides whether text may travel (`SMY-E402`), and which manifests
//! under an alias are *heads*.

use smysl_core::error::LibError;
use smysl_core::ids::{is_alias, LangTag, Mid};
use smysl_core::types::epistemics::SourceRef;
use smysl_core::types::library::{Calendar, Carry, Manifest, ParentKind, PartEntry};

/// Licences under which this build will emit somebody else's text.
pub mod licence {
    /// The licence identifier for a text in the public domain.
    ///
    /// Not an SPDX id: SPDX has no identifier for "this is out of copyright", only for
    /// dedications like `CC0-1.0`. A 1769 Bible is not CC0 — nobody dedicated it — so the
    /// format needs a word for the common case, and this is it (A-5 key 4 names it
    /// explicitly).
    pub const PUBLIC_DOMAIN: &str = "public-domain";

    /// The licence recorded when nobody knows, which is never permissive.
    pub const UNKNOWN: &str = "unknown";

    /// Licences that permit redistributing the text itself.
    ///
    /// **Sorted, and changing it is a code change with a test.** That is the whole design: the
    /// question "may this corpus be shipped" is answered by a list somebody has to edit on
    /// purpose, not by a flag an operator can pass at three in the morning. `SMY-E402` has no
    /// override for the same reason — the one thing an override would buy is the ability to
    /// redistribute someone else's text by accident.
    ///
    /// What is *not* here is as considered as what is. The `-NC` and `-ND` Creative Commons
    /// variants permit redistribution under conditions this code cannot check (what the
    /// receiver will do with it), and a list that admitted them would be answering a question
    /// it had not been asked. A corpus under one of those is carried by reference
    /// (`Carry::Ref`), which is what that value is for.
    pub const PERMISSIVE: &[&str] = &[
        "0BSD",
        "Apache-2.0",
        "BSD-2-Clause",
        "BSD-3-Clause",
        "CC-BY-3.0",
        "CC-BY-4.0",
        "CC-BY-SA-3.0",
        "CC-BY-SA-4.0",
        "CC0-1.0",
        "ISC",
        "MIT",
        "MPL-2.0",
        "Unlicense",
        PUBLIC_DOMAIN,
    ];

    /// Whether a licence permits carrying the text.
    ///
    /// Exact match, case-sensitively. SPDX identifiers are case-insensitive by their own
    /// specification, and this is still exact: a corpus that recorded `mit` has recorded
    /// something that is not an SPDX identifier, and the refusal says so while a silent
    /// fold would make `MIT`, `mit` and `Mit` three spellings in one catalog.
    pub fn is_permissive(licence: &str) -> bool {
        PERMISSIVE.contains(&licence)
    }
}

/// Whether this licence may carry this much (`SMY-E402`).
///
/// `Carry::Ref` is always allowed: a reference is the manifest's own statement about parts the
/// receiver must already hold, and naming a text is not redistributing it.
pub fn carry_allowed(licence: &str, carry: Carry) -> Result<(), LibError> {
    match carry {
        Carry::Text if !licence::is_permissive(licence) => Err(LibError::CarryRefused {
            licence: licence.to_string(),
        }),
        _ => Ok(()),
    }
}

/// A manifest under construction.
///
/// Thin on purpose: [`Manifest`] has nineteen keys and a builder that wrapped each of them
/// would be a second place to keep the list in step. What this adds is the checks — the alias
/// grammar at the point the alias is given rather than at `check` time, and the carry gate at
/// [`ManifestBuilder::build`] — so a `Manifest` that came out of here is one the library pass
/// will not refuse.
#[derive(Debug, Clone)]
pub struct ManifestBuilder {
    m: Manifest,
}

impl ManifestBuilder {
    /// The five required keys. Refuses an alias that is not A-3's grammar.
    pub fn new(
        alias: &str,
        lang: LangTag,
        reader: impl Into<String>,
        licence: impl Into<String>,
        policy: &crate::part::Policy,
    ) -> Result<ManifestBuilder, LibError> {
        if !is_alias(alias) {
            return Err(LibError::BadAlias {
                alias: alias.to_string(),
            });
        }
        Ok(ManifestBuilder {
            m: Manifest::new(alias, lang, reader, licence, policy.id()),
        })
    }

    pub fn part(mut self, entry: PartEntry) -> ManifestBuilder {
        self.m.parts.push(entry);
        self
    }

    pub fn parts(mut self, entries: Vec<PartEntry>) -> ManifestBuilder {
        self.m.parts = entries;
        self
    }

    pub fn carry(mut self, carry: Carry) -> ManifestBuilder {
        self.m.carry = carry;
        self
    }

    pub fn title(mut self, title: impl Into<String>) -> ManifestBuilder {
        self.m.title = Some(title.into());
        self
    }

    pub fn creators(mut self, creators: Vec<String>) -> ManifestBuilder {
        self.m.creators = creators;
        self
    }

    pub fn published(mut self, edtf: impl Into<String>) -> ManifestBuilder {
        self.m.published = Some(edtf.into());
        self
    }

    pub fn calendar(mut self, calendar: Calendar) -> ManifestBuilder {
        self.m.calendar = Some(calendar);
        self
    }

    pub fn identifier(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> ManifestBuilder {
        self.m.identifiers.insert(key.into(), value.into());
        self
    }

    pub fn origin(mut self, origin: SourceRef) -> ManifestBuilder {
        self.m.origin = Some(origin);
        self
    }

    pub fn parent(mut self, parent: Mid, kind: ParentKind) -> ManifestBuilder {
        self.m.parent = Some((parent, kind));
        self
    }

    pub fn supersedes(mut self, previous: Mid) -> ManifestBuilder {
        self.m.supersedes = Some(previous);
        self
    }

    pub fn versification(mut self, scheme: impl Into<String>) -> ManifestBuilder {
        self.m.versification = Some(scheme.into());
        self
    }

    /// Mark the reading lossy: something in the source did not survive into the text.
    pub fn lossy(mut self) -> ManifestBuilder {
        self.m.lossy = true;
        self
    }

    /// The reader's manifest-level metadata, as canonical CBOR.
    pub fn raw(mut self, raw: Vec<u8>) -> ManifestBuilder {
        self.m.raw = Some(raw);
        self
    }

    /// Finish, applying the carry gate.
    pub fn build(self) -> Result<Manifest, LibError> {
        carry_allowed(&self.m.licence, self.m.carry)?;
        Ok(self.m)
    }
}

/// The head manifests under each alias.
///
/// A manifest is a head when nothing supersedes it. More than one head under an alias is a
/// **fork**, which is `SMY-W418` and not an error: a fork is a fact about a corpus, usually
/// two machines that each appended to the same expression, and refusing to open the library
/// would make the fact unreportable. This function reports it; deciding what to do about it
/// belongs to `check` and to whoever is looking at the output.
pub fn heads<'a, I>(manifests: I) -> std::collections::BTreeMap<String, Vec<Mid>>
where
    I: IntoIterator<Item = (Mid, &'a Manifest)>,
{
    let mut by_alias: std::collections::BTreeMap<String, Vec<Mid>> =
        std::collections::BTreeMap::new();
    let mut superseded = std::collections::BTreeSet::new();
    for (mid, m) in manifests {
        if let Some(prev) = m.supersedes {
            superseded.insert(prev);
        }
        by_alias.entry(m.alias.clone()).or_default().push(mid);
    }
    for mids in by_alias.values_mut() {
        mids.retain(|m| !superseded.contains(m));
        // Sorted, so two libraries listing the same fork list it in the same order. `Mid` is
        // ordered by its bytes, which is arbitrary and stable — the two properties a
        // tie-break needs.
        mids.sort();
    }
    by_alias.retain(|_, mids| !mids.is_empty());
    by_alias
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::part::Policy;

    fn en() -> LangTag {
        LangTag::new("en").unwrap()
    }

    fn builder(licence: &str) -> ManifestBuilder {
        ManifestBuilder::new("kjv1769", en(), "osis/1", licence, &Policy::default()).unwrap()
    }

    #[test]
    fn the_permissive_list_is_sorted_and_holds_no_duplicates() {
        let mut sorted = licence::PERMISSIVE.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            sorted,
            licence::PERMISSIVE.to_vec(),
            "the list is sorted so a reader can scan it"
        );
        sorted.dedup();
        assert_eq!(sorted.len(), licence::PERMISSIVE.len());
    }

    #[test]
    fn carrying_text_needs_a_permissive_licence() {
        for ok in licence::PERMISSIVE {
            carry_allowed(ok, Carry::Text).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for refused in [
            licence::UNKNOWN,
            "CC-BY-NC-4.0",
            "CC-BY-ND-4.0",
            "GPL-3.0-only",
            "all-rights-reserved",
            "mit",
        ] {
            let e = carry_allowed(refused, Carry::Text).unwrap_err();
            assert_eq!(e.code(), Some(smysl_core::Code::E402), "{refused}");
            assert!(e.to_string().contains(refused), "{e}");
        }
    }

    #[test]
    fn naming_a_text_is_not_redistributing_it() {
        for carry in [Carry::None, Carry::Ref] {
            carry_allowed(licence::UNKNOWN, carry).unwrap();
            carry_allowed("all-rights-reserved", carry).unwrap();
        }
    }

    #[test]
    fn the_carry_gate_is_applied_at_build_and_not_merely_available() {
        let e = builder("all-rights-reserved")
            .carry(Carry::Text)
            .build()
            .unwrap_err();
        assert_eq!(e.code(), Some(smysl_core::Code::E402));
        // The same manifest without the carry is fine: the gate is about what travels.
        builder("all-rights-reserved").build().unwrap();
        builder(licence::PUBLIC_DOMAIN)
            .carry(Carry::Text)
            .build()
            .unwrap();
    }

    #[test]
    fn a_bad_alias_is_refused_where_it_is_given() {
        for bad in ["KJV", "kjv 1769", "", "a//b", "bible/"] {
            assert!(
                ManifestBuilder::new(bad, en(), "osis/1", "public-domain", &Policy::default())
                    .is_err(),
                "{bad:?} should not be an alias"
            );
        }
        for good in ["kjv1769", "bible/kjv", "ru:syn1876", "a.b-c_d"] {
            ManifestBuilder::new(good, en(), "osis/1", "public-domain", &Policy::default())
                .unwrap_or_else(|e| panic!("{good}: {e}"));
        }
    }

    #[test]
    fn the_policy_is_recorded_as_the_manifests_part_policy() {
        let m = builder("public-domain").build().unwrap();
        assert_eq!(m.part_policy, Policy::default().id());
        assert!(m.alias_is_valid());
        assert!(m.parts.is_empty(), "an import manifest has no parts");
    }

    fn manifest_with(alias: &str, supersedes: Option<Mid>) -> Manifest {
        let mut m = Manifest::new(
            alias,
            en(),
            "osis/1",
            "public-domain",
            Policy::default().id(),
        );
        m.supersedes = supersedes;
        m
    }

    #[test]
    fn the_head_is_the_manifest_nothing_supersedes() {
        let first = manifest_with("kjv", None);
        let first_mid = first.mid();
        let second = manifest_with("kjv", Some(first_mid));
        let second_mid = second.mid();

        let heads = heads(vec![(first_mid, &first), (second_mid, &second)]);
        assert_eq!(heads.get("kjv"), Some(&vec![second_mid]));
    }

    #[test]
    fn two_heads_under_one_alias_are_reported_rather_than_refused() {
        let first = manifest_with("kjv", None);
        let first_mid = first.mid();
        let a = manifest_with("kjv", Some(first_mid));
        let mut b = manifest_with("kjv", Some(first_mid));
        b.title = Some("a second machine's version".to_string());
        let (a_mid, b_mid) = (a.mid(), b.mid());

        let heads = heads(vec![(first_mid, &first), (a_mid, &a), (b_mid, &b)]);
        let mut expected = vec![a_mid, b_mid];
        expected.sort();
        assert_eq!(heads.get("kjv"), Some(&expected), "the fork, in one order");
    }

    #[test]
    fn aliases_do_not_interfere_with_each_other() {
        let kjv = manifest_with("kjv", None);
        let syn = manifest_with("syn1876", None);
        let heads = heads(vec![(kjv.mid(), &kjv), (syn.mid(), &syn)]);
        assert_eq!(heads.len(), 2);
        assert_eq!(heads["kjv"], vec![kjv.mid()]);
        assert_eq!(heads["syn1876"], vec![syn.mid()]);
    }

    /// A chain leaves one head however long it is, and a manifest whose own predecessor is
    /// missing is still a head.
    ///
    /// The second half is the case a truncated log produces: `from_cbor_seq` stops at a
    /// partial tail, so a library can hold the middle of a chain and nothing before it. The
    /// manifest it names as superseded is absent, which makes no difference — being a head is
    /// a statement about what supersedes *you*.
    #[test]
    fn a_chain_leaves_one_head_however_long_it_is() {
        let a = manifest_with("kjv", None);
        let b = manifest_with("kjv", Some(a.mid()));
        let c = manifest_with("kjv", Some(b.mid()));
        let all = heads(vec![(a.mid(), &a), (b.mid(), &b), (c.mid(), &c)]);
        assert_eq!(all["kjv"], vec![c.mid()]);

        let middle_only = heads(vec![(b.mid(), &b)]);
        assert_eq!(middle_only["kjv"], vec![b.mid()]);
    }
}
