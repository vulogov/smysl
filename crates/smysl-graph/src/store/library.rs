//! The library's view of a store: manifests, which of them are heads, and which units came
//! out of which text (RFC SMYSL-2.4 §4.3.2).
//!
//! A store holds **manifests** (record 14) and not text. Part texts and readings live in the
//! object store, and a log offered one refuses it with `SMY-E452` — see [`super::Store::append`].
//! So everything here is derived from manifests and from what a unit's `source` says about
//! where it came from.
//!
//! All of it is a map of sets rebuilt in `absorb`, which is what keeps merge order-independent:
//! two peers that receive the same manifests in opposite orders hold the same state, because
//! nothing here depends on arrival order.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::ids::{Mid, Tid};
use smysl_core::types::Manifest;
use smysl_core::Uid;

use super::Store;

/// The library state a store derives, kept in one struct so `Store`'s field list says what the
/// library part of it is rather than scattering five maps through thirty.
#[derive(Debug, Clone, Default)]
pub(super) struct Library {
    /// Every manifest, by its mid.
    pub(super) manifests: BTreeMap<Mid, Manifest>,
    /// The manifests of each expression alias.
    pub(super) by_alias: BTreeMap<String, BTreeSet<Mid>>,
    /// Mids named by some other manifest's `supersedes`.
    ///
    /// Kept as the set of *superseded* mids rather than as a chain: a head is a manifest nobody
    /// supersedes, and that is a membership question. A chain would also have to answer what to
    /// do when two manifests supersede the same one, which is a fork — a fact to report, not an
    /// error to resolve (`SMY-W418`).
    pub(super) superseded: BTreeSet<Mid>,
    /// Units whose source reference names a part, by that part's tid.
    pub(super) by_tid: BTreeMap<Tid, BTreeSet<Uid>>,
    //
    // **No `by_mid`**, though §4.3.2 lists one. It would be fed from `source.manifest`, and
    // that field arrives in TX-P5 (§4.3.1) — so a map built now could only ever be empty,
    // which is precisely what OQ-39's answer said about the `part_texts` and `readings` maps
    // that this plan already removed for the same reason. It arrives with the field that fills
    // it, in the same commit, where it can be tested.
}

impl Library {
    /// Record a manifest. Idempotent: the same manifest twice is one manifest.
    pub(super) fn absorb_manifest(&mut self, m: &Manifest) {
        let mid = m.mid();
        self.by_alias
            .entry(m.alias.clone())
            .or_default()
            .insert(mid);
        if let Some(previous) = m.supersedes {
            self.superseded.insert(previous);
        }
        self.manifests.entry(mid).or_insert_with(|| m.clone());
    }

    /// Index a unit by the part and the manifest its source names.
    ///
    /// Parsed once, here, rather than scanned for on every query: `units_with_source_prefix`
    /// exists and is public contract, but it is a scan of every unit and a *prefix* match —
    /// which answers "looks like this text" when the question is "is this text".
    pub(super) fn absorb_unit(&mut self, uid: Uid, source: Option<&smysl_core::types::SourceRef>) {
        let Some(source) = source else {
            return;
        };
        if let Some(tid) = part_of(&source.reference) {
            self.by_tid.entry(tid).or_default().insert(uid);
        }
    }
}

/// The part a reference names, if it names one.
///
/// A reference is `t3:<52 characters>` or that followed by `#<locator>`. The identity is parsed
/// in its canonical form and nothing shorter: `Tid::parse` refuses an abbreviation for the
/// reason `Uid::parse` does — an abbreviated identity in a record weakens identity silently,
/// and a 26-character prefix that happened to collide would put two texts' units in one bucket.
fn part_of(reference: &str) -> Option<Tid> {
    let rest = reference.strip_prefix(Tid::PREFIX)?;
    let body = match rest.find('#') {
        Some(i) => &rest[..i],
        None => rest,
    };
    let mut text = String::with_capacity(Tid::PREFIX.len() + body.len());
    text.push_str(Tid::PREFIX);
    text.push_str(body);
    Tid::parse(&text).ok()
}

impl Store {
    /// The manifest with this mid.
    pub fn manifest(&self, mid: &Mid) -> Option<&Manifest> {
        self.library.manifests.get(mid)
    }

    /// Every manifest the store holds, in mid order.
    pub fn manifests(&self) -> impl Iterator<Item = (&Mid, &Manifest)> + '_ {
        self.library.manifests.iter()
    }

    /// How many manifests the store holds.
    pub fn manifest_count(&self) -> usize {
        self.library.manifests.len()
    }

    /// Every expression alias the store has a manifest for, in order.
    pub fn aliases(&self) -> impl Iterator<Item = &str> + '_ {
        self.library.by_alias.keys().map(String::as_str)
    }

    /// The manifests recorded under an alias, in mid order.
    pub fn manifests_of(&self, alias: &str) -> Vec<Mid> {
        self.library
            .by_alias
            .get(alias)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Whether some other manifest names this one in `supersedes`.
    pub fn is_superseded(&self, mid: &Mid) -> bool {
        self.library.superseded.contains(mid)
    }

    /// The heads of an alias: its manifests that nothing supersedes.
    ///
    /// Usually one. **Two is a fork**, and this returns both: a fork is a fact about a corpus,
    /// and `check` reports it as `SMY-W418`. Refusing to open a forked library, or picking a
    /// winner here, would make the fact unreportable — which is the same argument that keeps a
    /// contention a record rather than an error.
    ///
    /// Empty when the alias is unknown, or when every manifest under it is superseded by
    /// another the store does not hold: a truncated chain has no head, and saying so is better
    /// than inventing one.
    pub fn heads(&self, alias: &str) -> Vec<Mid> {
        self.library
            .by_alias
            .get(alias)
            .map(|set| {
                set.iter()
                    .filter(|mid| !self.library.superseded.contains(mid))
                    .copied()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The units whose source names this part, exactly.
    ///
    /// The exact answer to the question `units_with_source_prefix` answers approximately. That
    /// method stays as it is — it has been public contract since 1.5 — but a prefix match over
    /// every unit is both a scan and a guess, and this is neither.
    pub fn units_with_tid(&self, tid: &Tid) -> Vec<Uid> {
        self.library
            .by_tid
            .get(tid)
            .map(|set| set.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Every part any manifest records, in tid order, with the length it was recorded at.
    ///
    /// The length comes from the manifest's part entry, so a caller asking whether a span fits
    /// (`SMY-E404`) needs no object store and no text. A tid recorded by two manifests with two
    /// different lengths appears once per length, because that disagreement is exactly what a
    /// check is for — collapsing it here would hide it.
    pub fn parts(&self) -> BTreeMap<Tid, BTreeSet<u64>> {
        let mut out: BTreeMap<Tid, BTreeSet<u64>> = BTreeMap::new();
        for m in self.library.manifests.values() {
            for part in &m.parts {
                out.entry(part.tid).or_default().insert(part.length);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::ids::LangTag;
    use smysl_core::types::{Carry, PartEntry};
    use smysl_core::{KernelType, Record, Status, UnitCoreBuilder};

    fn tid(byte: u8) -> Tid {
        Tid::from_normalised_bytes(&[byte])
    }

    fn manifest(alias: &str, part: u8) -> Manifest {
        let mut m = Manifest::new(
            alias,
            LangTag::new("en").expect("a tag"),
            "txt/1",
            "public-domain",
            "smysl/parts/1 level=line min=65536 max=4194304",
        );
        m.carry = Carry::Text;
        m.parts.push(PartEntry::new(
            tid(part),
            128,
            [part; 32],
            smysl_core::ids::Rdid::from_bytes([part; 32]),
        ));
        m
    }

    /// A reference names a part only in the canonical form, and a locator does not get in the way.
    #[test]
    fn a_reference_yields_its_part() {
        let t = tid(7);
        assert_eq!(part_of(&t.canonical()), Some(t));
        assert_eq!(part_of(&format!("{}#Gen.1.1", t.canonical())), Some(t));
        // The short display form is not an identity: `Tid::parse` refuses it, and so does this.
        assert_eq!(part_of(&t.short()), None);
        assert_eq!(part_of("b3:something"), None);
        assert_eq!(part_of("t3:"), None);
        assert_eq!(part_of(""), None);
    }

    /// One manifest, one head; the superseded one drops out.
    #[test]
    fn a_superseded_manifest_is_not_a_head() {
        let first = manifest("kjv", 1);
        let mut second = manifest("kjv", 2);
        second.supersedes = Some(first.mid());

        let mut library = Library::default();
        library.absorb_manifest(&first);
        library.absorb_manifest(&second);

        assert!(library.superseded.contains(&first.mid()));
        assert!(!library.superseded.contains(&second.mid()));
        assert_eq!(library.by_alias["kjv"].len(), 2);
    }

    /// Absorbing the same manifest twice changes nothing.
    #[test]
    fn absorbing_a_manifest_twice_is_absorbing_it_once() {
        let m = manifest("kjv", 1);
        let mut library = Library::default();
        library.absorb_manifest(&m);
        let after_one = library.clone();
        library.absorb_manifest(&m);
        assert_eq!(library.manifests.len(), after_one.manifests.len());
        assert_eq!(library.by_alias, after_one.by_alias);
    }

    /// The adjacency is rebuilt for everything except text, and skipped for text.
    ///
    /// The step's cost claim, asserted as a count rather than as a duration: an append used to
    /// rebuild whatever arrived, so a manifest cost the whole store.
    ///
    /// Both halves are here, and the second is the one worth having. The flag is a deny-list,
    /// so the test that a manifest does not rebuild would also pass if the flag had been
    /// written the other way round and a record type were missing from it. The view assertion
    /// is the control: a record that moves no edge *today* must still rebuild, because the
    /// only thing keeping the adjacency correct for a record type nobody has thought about
    /// yet is that the default is to rebuild.
    #[test]
    fn everything_but_text_rebuilds_the_adjacency() {
        let mut store = Store::new();
        let unit = UnitCoreBuilder::new(
            KernelType::Claim,
            "the engine is ready",
            Status::Speculative,
        )
        .build()
        .expect("a unit");
        store.append(&[Record::Unit(unit)]).expect("appended");
        let after_unit = store.adjacency_rebuilds;
        assert!(after_unit > 0, "a unit rebuilds");

        // A manifest cannot move an edge.
        store
            .append(&[Record::Manifest(manifest("kjv1769", 1))])
            .expect("appended");
        assert_eq!(
            store.adjacency_rebuilds, after_unit,
            "a manifest-only append must not rebuild the adjacency"
        );

        // A view moves no edge either, and rebuilds all the same. That is the deny-list
        // working: `View` is not text, so it is not exempt, and the cost of being wrong about
        // it is one rebuild rather than a traversal that cannot see an edge.
        store
            .append(&[Record::View(smysl_core::types::View::new(
                smysl_core::ids::ViewId::new("v/one").expect("a view id"),
                "a view that moves no edge",
            ))])
            .expect("appended");
        assert_eq!(
            store.adjacency_rebuilds,
            after_unit + 1,
            "only text is exempt from the rebuild"
        );
        let after_view = store.adjacency_rebuilds;

        // But another unit does.
        let second = UnitCoreBuilder::new(
            KernelType::Claim,
            "and the readers work",
            Status::Speculative,
        )
        .build()
        .expect("a unit");
        store.append(&[Record::Unit(second)]).expect("appended");
        assert_eq!(
            store.adjacency_rebuilds,
            after_view + 1,
            "a unit rebuilds exactly once"
        );
    }
}
