//! TX-P9 N4's exit (SMYSL-2.6 §6): the channel measures on a source and a target with planted
//! drops (C1), additions (C2), one inflation per status step (C3) and holder rewrites (C4),
//! each ratio exact (§5.2); C5 over Ruth in two translations; and C1–C4 on the KJV and Synodal
//! Psalms fixture, the one TX-P4 aligned Psalm 14 with Psalm 13 on, reported.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use smysl_core::diag::Code;
use smysl_core::ids::{AgentId, LangTag, Uid};
use smysl_core::schemas::text::ENTITY;
use smysl_core::surface::hjson::{HObject, HValue, Spanned};
use smysl_core::surface::object_to_payload;
use smysl_core::types::{Carry, Hlc, RelKind, SourceKind, SourceRef};
use smysl_core::{
    canonical_uid, Attestation, KernelType, Op, Record, Relation, Rung, SchemaId, Span, Status,
    UnitCoreBuilder,
};
use smysl_graph::proposition::SAME_AS;
use smysl_graph::Store;
use smysl_measure::channel::{c5, Channel};
use smysl_measure::text::Corpus;
use smysl_measure::{
    run, to_unit_on, ClassPolicy, Input, MeasureResult, Num, Outcome, Recipe, Resolved, Value,
};
use smysl_text::library::{AddSpec, Library};
use smysl_text::limits::Caps;
use smysl_text::readers::Input as ReadInput;

const SOURCE: &str = "from texts where alias = source";
const TARGET: &str = "from texts where alias = target";

fn repo(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

fn same_as(a: Uid, b: Uid) -> Record {
    Record::Relation(Relation::new(RelKind::Extension(SAME_AS.into()), a, b))
}

fn payload(fields: &[(&str, String)]) -> Vec<u8> {
    let mut o = HObject::new();
    for (k, v) in fields {
        o.insert(
            Spanned::new(k.to_string(), Span::new(0, 0)),
            Spanned::new(HValue::Str(v.clone()), Span::new(0, 0)),
        );
    }
    object_to_payload(&o).unwrap()
}

/// A planted channel: the records, and each side's units in the order they were made.
#[derive(Default)]
struct Planted {
    records: Vec<Record>,
    anchor: Option<Uid>,
}

impl Planted {
    /// A claim on `side`, numbered, at `status`, with `fields` in its payload. Every unit
    /// has a source, so `cited` and `measured` are admitted, and the two that need grounds
    /// rest on one anchor outside both scopes.
    fn unit(&mut self, side: &str, i: u32, status: Status, fields: &[(&str, String)]) -> Uid {
        let anchor = *self.anchor.get_or_insert_with(|| {
            let a = UnitCoreBuilder::new(KernelType::Claim, "the anchor", Status::Speculative)
                .build()
                .unwrap();
            let uid = canonical_uid(&a);
            self.records.push(Record::Unit(a));
            uid
        });
        let mut b = UnitCoreBuilder::new(KernelType::Claim, format!("{side} unit {i}"), status)
            .source(SourceRef::new(SourceKind::Doc, format!("{side}-{i}")));
        if matches!(status, Status::Inferred | Status::Derived) {
            b = b.grounds([anchor]);
        }
        if !fields.is_empty() {
            b = b.payload(payload(fields));
        }
        let u = b.build().unwrap();
        let uid = canonical_uid(&u);
        self.records.push(Record::Unit(u));
        uid
    }

    fn entity(&mut self, name: &str, lang: &str) -> Uid {
        let u = UnitCoreBuilder::new(
            SchemaId::Extension(ENTITY.to_string()),
            name,
            Status::Speculative,
        )
        .lang(LangTag::new(lang).unwrap())
        .build()
        .unwrap();
        let uid = canonical_uid(&u);
        self.records.push(Record::Unit(u));
        uid
    }

    fn by_model(&mut self, uid: Uid) {
        let agent = AgentId::new("tool:a-model").unwrap();
        self.records.push(Record::Attestation(Attestation::new(
            uid,
            agent.clone(),
            Op::Transformed,
            Rung::Model,
            Hlc::new(1, 0, agent),
        )));
    }

    fn store(&self) -> Store {
        let mut s = Store::new();
        s.append(&self.records).expect("appends");
        s
    }
}

fn scope(text: &str, uids: &[Uid]) -> Resolved {
    Resolved::of_uids(text, uids.iter().copied().collect())
}

fn strict() -> Recipe {
    let mut r = Recipe::new("", 0, SOURCE);
    r.policy = Some(ClassPolicy::Strict);
    r
}

fn value(r: &MeasureResult) -> &Value {
    match &r.outcome {
        Outcome::Value(v) => v,
        other => panic!("{} did not run: {other:?}", r.recipe.id),
    }
}

fn refusal(r: &MeasureResult) -> Code {
    match &r.outcome {
        Outcome::NotRun { code: Some(c), .. } => *c,
        other => panic!("{} was expected to refuse: {other:?}", r.recipe.id),
    }
}

/// §5.2's channel: ten source units and eight target units.
///
/// - **Drops.** `s6` and `s7` are claimed the same as nothing in the target; `s8` and `s9`
///   are one class with each other, and `t6` joins it — claimed the same as both, since a
///   `strict` class is a clique. Nine source classes, seven survive.
/// - **Additions.** `t7` is in no aligned class, and a model attested it.
/// - **Inflation, one per status step.** `t0`…`t3` are each one rung above their partner,
///   speculative → inferred → derived → cited → measured; `t4` is level with `s4`, `t5` two
///   below `s5`, and `t6` is cited beside `s8` speculative and `s9` cited — level with the
///   stronger, so not inflated, which is the conservative partner at work.
/// - **Holders.** `t1` gains a holder the source did not name; `s2` and `t2` name David in two
///   languages, one class by a `same-as` between the entities; `s3`'s holder is moved to
///   another entity in `t3`; and `s4`'s holder is lost in `t4`. Three of seven pairs changed.
/// - **Modes.** `s0` is asserted and `t0` reported; every other unit has none.
fn planted() -> (Planted, Vec<Uid>, Vec<Uid>) {
    use Status::*;
    let mut p = Planted::default();
    let david = p.entity("David", "en");
    let davyd = p.entity("Давид", "ru");
    let nathan = p.entity("Nathan", "en");
    let saul = p.entity("Saul", "en");
    p.records.push(same_as(david, davyd));
    let h = |u: Uid| ("text:holder", u.canonical());
    let source_status = [
        Speculative,
        Inferred,
        Derived,
        Cited,
        Cited,
        Measured,
        Cited,
        Cited,
        Speculative,
        Cited,
    ];
    let source_fields: Vec<Vec<(&str, String)>> = vec![
        vec![("text:mode", "asserted".into())],
        vec![],
        vec![h(david)],
        vec![h(nathan)],
        vec![h(saul)],
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
    ];
    let s: Vec<Uid> = (0..10)
        .map(|i| {
            p.unit(
                "source",
                i,
                source_status[i as usize],
                &source_fields[i as usize],
            )
        })
        .collect();
    let target_status = [
        Inferred, Derived, Cited, Measured, Cited, Derived, Cited, Cited,
    ];
    let target_fields: Vec<Vec<(&str, String)>> = vec![
        vec![("text:mode", "reported".into())],
        vec![h(nathan)],
        vec![h(davyd)],
        vec![h(saul)],
        vec![("text:holder", "text".into())],
        vec![],
        vec![],
        vec![],
    ];
    let t: Vec<Uid> = (0..8)
        .map(|i| {
            p.unit(
                "target",
                i,
                target_status[i as usize],
                &target_fields[i as usize],
            )
        })
        .collect();
    for i in 0..6 {
        p.records.push(same_as(s[i], t[i]));
    }
    p.records.push(same_as(s[8], s[9]));
    p.records.push(same_as(s[9], t[6]));
    p.records.push(same_as(s[8], t[6]));
    p.by_model(t[7]);
    (p, s, t)
}

fn over(store: &Store, s: &Resolved, t: &Resolved, id: &str, recipe: &Recipe) -> MeasureResult {
    let input = Input::new(store, s).with_channel(t);
    run(&[id], recipe, &input).remove(0)
}

#[test]
fn c1_to_c4_give_the_planted_ratios_exactly() {
    let (p, s, t) = planted();
    let store = p.store();
    let (ss, ts) = (scope(SOURCE, &s), scope(TARGET, &t));
    let r = strict();

    let c1 = over(&store, &ss, &ts, "C1", &r);
    let v = value(&c1);
    assert_eq!(v.values["survival"], Num::Ratio(7, 9));
    assert_eq!(v.n["dropped"], 2);
    assert_eq!(v.n["source:classes"], 9);
    assert_eq!(c1.recipe.channel.as_deref(), Some(TARGET));
    assert_eq!(c1.recipe.channel_digest, Some(ts.digest));
    assert_eq!(c1.recipe.scope_digest, ss.digest);

    let c2 = over(&store, &ss, &ts, "C2", &r);
    let v = value(&c2);
    assert_eq!(v.values["support"], Num::Ratio(7, 8));
    assert_eq!(v.values["addition"], Num::Ratio(1, 8));
    assert_eq!(v.values["fabrication"], Num::Ratio(1, 8));
    assert_eq!(v.n["model"], 1);

    let c3 = over(&store, &ss, &ts, "C3", &r);
    let v = value(&c3);
    assert_eq!(v.n["pairs"], 7);
    assert_eq!(v.values["inflation"], Num::Ratio(4, 7));
    assert_eq!(v.values["deflation"], Num::Ratio(1, 7));
    assert_eq!(
        v.values["delta:1"],
        Num::Count(4),
        "one inflation per status step"
    );
    assert_eq!(v.values["delta:0"], Num::Count(2), "t4, and t6 beside s9");
    assert_eq!(v.values["delta:-2"], Num::Count(1));
    for d in -5..=5 {
        assert!(v.values.contains_key(&format!("delta:{d}")), "delta:{d}");
    }
    assert_eq!(v.values["mode:asserted>reported"], Num::Count(1));
    assert_eq!(v.values["mode:none>none"], Num::Count(6));
    assert_eq!(v.values["mode:changed"], Num::Ratio(1, 7));

    let c4 = over(&store, &ss, &ts, "C4", &r);
    let v = value(&c4);
    assert_eq!(v.values["holder:changed"], Num::Ratio(3, 7));
    assert_eq!(v.values["holder:gained"], Num::Count(1));
    assert_eq!(v.values["holder:lost"], Num::Count(1));
    assert_eq!(v.values["holder:moved"], Num::Count(1));
}

/// The partner is the strongest source member, and the uid breaks a tie.
#[test]
fn the_partner_is_the_strongest_source_member() {
    let (p, s, t) = planted();
    let store = p.store();
    let ch = Channel::bind(&store, smysl_graph::proposition::Policy::Strict, &s, &t).unwrap();
    let pairs = ch.pairs(&store);
    assert_eq!(pairs.len(), 7);
    assert!(
        pairs.contains(&(t[6], s[9])),
        "t6's partner is s9, the cited one"
    );
    let mut tied = [s[6], s[7]];
    tied.sort();
    assert_eq!(
        smysl_measure::channel::strongest(&store, &tied),
        Some(tied[0]),
        "two cited units: the smaller uid"
    );
}

/// What a channel refuses: two scopes sharing a unit (`SMY-E489`), no policy (`SMY-E481`),
/// no target (`SMY-E489`), and — for the measures over pairs — a channel with no aligned
/// pair (`SMY-W485`), where C1 and C2 still run and say none survived.
#[test]
fn a_channel_refuses_what_it_cannot_compare() {
    let (p, s, t) = planted();
    let store = p.store();
    let (ss, ts) = (scope(SOURCE, &s), scope(TARGET, &t));

    let mut overlapping = t.clone();
    overlapping.push(s[0]);
    let os = scope(TARGET, &overlapping);
    for id in ["C1", "C2", "C3", "C4"] {
        assert_eq!(refusal(&over(&store, &ss, &os, id, &strict())), Code::E489);
        assert_eq!(
            refusal(&over(&store, &ss, &ts, id, &Recipe::new("", 0, SOURCE))),
            Code::E481
        );
        let alone = Input::new(&store, &ss);
        assert_eq!(
            refusal(&run(&[id], &strict(), &alone).remove(0)),
            Code::E489
        );
    }

    // Two scopes nothing joins: only the units no edge reaches.
    let unjoined = scope(TARGET, &[t[7]]);
    let lone = scope(SOURCE, &[s[6], s[7]]);
    let c1 = over(&store, &lone, &unjoined, "C1", &strict());
    assert_eq!(value(&c1).values["survival"], Num::Ratio(0, 2));
    let c2 = over(&store, &lone, &unjoined, "C2", &strict());
    assert_eq!(value(&c2).values["addition"], Num::Ratio(1, 1));
    for id in ["C3", "C4"] {
        assert_eq!(
            refusal(&over(&store, &lone, &unjoined, id, &strict())),
            Code::W485
        );
    }
}

/// A channel result is grounded on both scopes' bases, and a recipe without a channel hashes
/// as it did before channels existed.
#[test]
fn a_channel_result_names_its_target_and_rests_on_both_scopes() {
    let (p, s, t) = planted();
    let store = p.store();
    let (ss, ts) = (scope(SOURCE, &s), scope(TARGET, &t));
    let c1 = over(&store, &ss, &ts, "C1", &strict());
    let cbor = c1.recipe.to_cbor().unwrap();
    let text = String::from_utf8_lossy(&cbor);
    assert!(
        text.contains("channel:digest"),
        "the target's digest is in the recipe"
    );

    let plain = Recipe::new("A2", 1, SOURCE);
    assert!(
        !String::from_utf8_lossy(&plain.to_cbor().unwrap()).contains("channel"),
        "a recipe with no channel writes none"
    );

    // Neither scope here is a saved query, so there is nothing to ground on.
    assert!(to_unit_on(&c1, &[ss.basis.as_ref(), ts.basis.as_ref()]).is_err());
}

// ---------------------------------------------------------------------------
// C5 over Ruth in two translations
// ---------------------------------------------------------------------------

struct Built {
    root: PathBuf,
    library: Library,
}

impl Drop for Built {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn scratch(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "smysl-measure-channel-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn ruths() -> Built {
    let root = scratch("ruth");
    let mut library = Library::create(&root).unwrap();
    for (dir, alias, lang) in [("eng-kjv", "kjv", "en"), ("russyn", "synodal", "ru")] {
        let raw = std::fs::read(repo(&format!(
            "fixtures/library/bibles/slice/{dir}/08-RUT.usfm"
        )))
        .unwrap();
        let spec = AddSpec::new("usfm/1", alias, "public-domain")
            .with_lang(LangTag::new(lang).unwrap())
            .with_carry(Carry::Text);
        library
            .add(&ReadInput::new(&raw), &spec, &Caps::DEFAULT)
            .unwrap_or_else(|e| panic!("{alias}: {e}"));
    }
    Built { root, library }
}

/// `round_half_even(num · 2²⁰ / den)`, worked here rather than borrowed from the crate.
fn ratio_q20(num: i64, den: i64) -> i64 {
    let n = i128::from(num) << 20;
    let d = i128::from(den);
    let (q, r) = (n.div_euclid(d), n.rem_euclid(d));
    let q = if 2 * r > d || (2 * r == d && q % 2 != 0) {
        q + 1
    } else {
        q
    };
    i64::try_from(q).unwrap()
}

fn corpus(lib: &Library, scope: &str) -> Corpus {
    let found = smysl_measure::text::texts(scope, lib).unwrap();
    Corpus::load(lib, &found, smysl_measure::Level::Part, None).unwrap()
}

/// C5 is the ratio of the two T2 figures a reader would see, taken from their stored
/// integers; T3, short of its 50,000 symbols in Ruth, is flagged rather than refused; and a
/// text on both sides is refused.
#[test]
fn c5_is_the_ratio_of_the_two_stored_rates() {
    let built = ruths();
    let lib = &built.library;
    let (kjv, syn) = (
        corpus(lib, "from collection:kjv"),
        corpus(lib, "from collection:synodal"),
    );
    let empty = Resolved::of_uids("from collection:kjv", BTreeSet::new());
    let target = Resolved::of_uids("from collection:synodal", BTreeSet::new());
    let t2 = |c: &Corpus| {
        let input = Input::new(lib.store(), &empty).with_text(c);
        match value(&run(&["T2"], &Recipe::new("", 0, "from collection:kjv"), &input).remove(0))
            .values["bits/char"]
        {
            Num::Fixed(q) => q,
            ref x => panic!("{x:?}"),
        }
    };
    let (s2, t2v) = (t2(&kjv), t2(&syn));

    let input = Input::new(lib.store(), &empty)
        .with_text(&kjv)
        .with_channel(&target)
        .with_target_text(&syn);
    let r = run(&["C5"], &Recipe::new("", 0, "from collection:kjv"), &input).remove(0);
    let v = value(&r);
    assert_eq!(v.values["T2:source"], Num::Fixed(s2));
    assert_eq!(v.values["T2:target"], Num::Fixed(t2v));
    assert_eq!(v.values["T2:ratio"], Num::Fixed(ratio_q20(t2v, s2)));
    assert!(v.flags.iter().any(|f| f == c5::T3_NOT_RUN), "{:?}", v.flags);
    assert_eq!(r.recipe.channel_digest, Some(syn.digest));
    assert_eq!(r.recipe.scope_digest, kjv.digest);
    eprintln!(
        "C5 Ruth KJV -> Synodal: T2 {} / {} = {}",
        Num::Fixed(t2v),
        Num::Fixed(s2),
        v.values["T2:ratio"]
    );

    let same = Input::new(lib.store(), &empty)
        .with_text(&kjv)
        .with_channel(&target)
        .with_target_text(&kjv);
    assert_eq!(
        refusal(&run(&["C5"], &Recipe::new("", 0, "from collection:kjv"), &same).remove(0)),
        Code::E489
    );
    let none = Input::new(lib.store(), &empty).with_text(&kjv);
    assert_eq!(
        refusal(&run(&["C5"], &Recipe::new("", 0, "from collection:kjv"), &none).remove(0)),
        Code::E489
    );
}

// ---------------------------------------------------------------------------
// C1–C4 on the KJV and Synodal Psalms fixture
// ---------------------------------------------------------------------------

/// The fixture of `crates/smysl-text/tests/align.rs`, as it is there: three psalms in the
/// Hebrew numbering, the same three in the Septuagint's, and the map between them. The verse
/// text is synthetic, and says which verse it stands in for; that file's header gives the
/// reason, and what is measured here is what the alignment carries, not the words.
const KJV: &str = "\\
\\id PSA The Book of Psalms
\\h Psalms
\\c 14
\\p
\\v 1 synthetic text standing in for the first verse of the fourteenth psalm
\\v 2 synthetic text standing in for the second verse of the fourteenth psalm
\\v 3 synthetic text standing in for the third verse of the fourteenth psalm
\\c 15
\\p
\\v 1 synthetic text standing in for the first verse of the fifteenth psalm
\\c 51
\\p
\\v 1 synthetic text standing in for the first verse of the fifty-first psalm
\\v 2 synthetic text standing in for the second verse of the fifty-first psalm
\\v 3 synthetic text standing in for the third verse of the fifty-first psalm
";

const SYNODAL: &str = "\\
\\id PSA The Book of Psalms
\\h Psalms
\\c 13
\\p
\\v 1 synthetic text standing in for the first verse of the thirteenth psalm
\\v 2 synthetic text standing in for the second verse of the thirteenth psalm
\\v 3 synthetic text standing in for the third verse of the thirteenth psalm
\\c 14
\\p
\\v 1 synthetic text standing in for the first verse of the fourteenth psalm
\\c 50
\\p
\\v 1 synthetic text standing in for the title, which this numbering counts as a verse
\\v 2 synthetic text standing in for the second verse of the fiftieth psalm
\\v 3 synthetic text standing in for the third verse of the fiftieth psalm
\\v 4 synthetic text standing in for the fourth verse of the fiftieth psalm
";

const MAP: &str = "smysl/versification/1
from\tkjv
to\tsynodal
Ps.14\tPs.13
Ps.51\tPs.50
Ps.51.1\tPs.50.2
Ps.51.2\tPs.50.3
Ps.51.3\tPs.50.4
";

/// A cited unit drawn from `alias` at `at`, stamped as a reader's would be.
fn verse_unit(lib: &Library, alias: &str, at: &str) -> UnitCoreBuilder {
    use smysl_text::locator;
    use smysl_text::provenance::{stamp, Drawn};
    let mid = lib.head(alias).unwrap();
    let manifest = lib.store().manifest(&mid).unwrap().clone();
    let p = lib
        .passage(alias, &locator::parse(at).unwrap(), &Caps::DEFAULT)
        .unwrap();
    let lang = manifest.lang.clone();
    stamp(
        UnitCoreBuilder::new(KernelType::Claim, format!("{alias} {at}"), Status::Cited),
        &Drawn {
            tid: p.tid,
            locator: Some(at),
            manifest: &manifest,
            observed: None,
            span: Some((p.range.start, p.range.end)),
            lang: Some(&lang),
        },
    )
}

/// §6 N4's report: a unit at every verse of each psalter, and C1–C4 from the Hebrew-numbered
/// psalter to the Septuagint-numbered one under `strict`, twice.
///
/// **Accepted as a reviewer would.** `anchored` is a candidate generator and says so (its
/// header: one engine alone is a proposal). The pairs accepted first are the ones the map
/// says are one place — each KJV verse with the Synodal verse `align::locate` gives it. Six of
/// the seven KJV verses survive (Psalm 15 is in no row of the map); six of the eight Synodal
/// verses are supported, and the two that are not are the ones the map gives no source:
/// Psalm 50's title, which the Septuagint numbering counts as a verse, and its Psalm 14, which
/// is the Hebrew 15. Nothing is inflated and no holder moves, since a reader writes neither.
///
/// **Every candidate accepted.** `anchored` also proposes the adjacent verse, and the reverse
/// of the chapter row carries Psalm 50's title onto Psalm 51:1, so it makes 16 proposals for
/// 6 places. Accepted wholesale, `strict` classes pair all three verses of Psalm 51 with the
/// wrong verse of Psalm 50 — and C1–C4 give the same numbers to the digit. That is the
/// finding: a channel measure counts classes, so it is as good as the `same-as` it is given,
/// and a wrong pairing within a psalm is invisible to it.
#[test]
fn c1_to_c4_on_the_kjv_and_synodal_psalms() {
    use smysl_text::index::align;
    use smysl_text::proposition::{Anchored, Engine};
    use smysl_text::versification;

    let root = scratch("psalms");
    let mut lib = Library::create(&root).unwrap();
    versification::install(&root, "kjv-synodal", MAP).unwrap();
    for (text, alias, lang) in [(KJV, "kjv", "en"), (SYNODAL, "synodal", "ru")] {
        let mut spec = AddSpec::new("usfm/1", alias, "public-domain")
            .with_lang(LangTag::new(lang).unwrap())
            .with_carry(Carry::Text);
        spec.versification = Some(alias.to_string());
        lib.add(&ReadInput::new(text.as_bytes()), &spec, &Caps::DEFAULT)
            .unwrap();
    }
    lib.complete_index();

    let kjv = [
        "Ps.14.1", "Ps.14.2", "Ps.14.3", "Ps.15.1", "Ps.51.1", "Ps.51.2", "Ps.51.3",
    ];
    let syn = [
        "Ps.13.1", "Ps.13.2", "Ps.13.3", "Ps.14.1", "Ps.50.1", "Ps.50.2", "Ps.50.3", "Ps.50.4",
    ];
    let (mut s, mut t, mut records) = (Vec::new(), Vec::new(), Vec::new());
    for (alias, verses, out) in [("kjv", &kjv[..], &mut s), ("synodal", &syn[..], &mut t)] {
        for at in verses {
            let u = verse_unit(&lib, alias, at).build().unwrap();
            out.push(canonical_uid(&u));
            records.push(Record::Unit(u));
        }
    }
    lib.append_records(&records).unwrap();
    let table = lib.index().align.clone();
    let (km, sm) = (lib.head("kjv").unwrap(), lib.head("synodal").unwrap());
    let exact: BTreeSet<(Uid, Uid)> = kjv
        .iter()
        .zip(&s)
        .filter_map(|(at, su)| {
            let there = align::locate(&table, &km, &sm, at)?;
            let k = syn.iter().position(|v| *v == there)?;
            let (a, b) = (*su, t[k]);
            Some(if a < b { (a, b) } else { (b, a) })
        })
        .collect();
    assert_eq!(exact.len(), 6);
    let proposed = Anchored::new(&table).propose(lib.store(), None);
    assert_eq!(proposed.len(), 16, "{proposed:#?}");
    let (accepted, rest): (Vec<_>, Vec<_>) =
        proposed.iter().partition(|p| exact.contains(&(p.a, p.b)));
    assert_eq!(
        accepted.len(),
        6,
        "every exact place is among the candidates"
    );

    let mut r = strict();
    r.scope = "from collection:kjv".into();
    let (ss, ts) = (
        scope("from collection:kjv", &s),
        scope("from collection:synodal", &t),
    );
    let measure = |store: &Store, label: &str| -> (Vec<Value>, Vec<(Uid, Uid)>) {
        let got: Vec<Value> = ["C1", "C2", "C3", "C4"]
            .iter()
            .map(|id| value(&over(store, &ss, &ts, id, &r)).clone())
            .collect();
        for (id, v) in ["C1", "C2", "C3", "C4"].iter().zip(&got) {
            let shown: Vec<String> = v
                .values
                .iter()
                .filter(|(k, x)| !k.starts_with("delta:") || **x != Num::Count(0))
                .map(|(k, x)| format!("{k} {x}"))
                .collect();
            eprintln!("{label} {id} kjv -> synodal: {}", shown.join(", "));
        }
        let ch = Channel::bind(store, smysl_graph::proposition::Policy::Strict, &s, &t).unwrap();
        (got, ch.pairs(store))
    };

    lib.append_records(
        &accepted
            .iter()
            .map(|p| same_as(p.a, p.b))
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let (got, pairs) = measure(lib.store(), "exact");
    assert_eq!(got[0].values["survival"], Num::Ratio(6, 7));
    assert_eq!(got[1].values["support"], Num::Ratio(6, 8));
    assert_eq!(got[1].values["addition"], Num::Ratio(2, 8));
    assert!(
        !got[1].values.contains_key("fabrication"),
        "no model wrote either"
    );
    assert_eq!(got[2].values["inflation"], Num::Ratio(0, 6));
    assert_eq!(got[2].values["mode:none>none"], Num::Count(6));
    assert_eq!(got[3].values["holder:changed"], Num::Ratio(0, 6));
    let right = |pairs: &[(Uid, Uid)]| {
        pairs
            .iter()
            .filter(|(a, b)| exact.contains(&if a < b { (*a, *b) } else { (*b, *a) }))
            .count()
    };
    assert_eq!(right(&pairs), 6, "every pair is the verse the map names");
    let unsupported: Vec<&str> = syn
        .iter()
        .zip(&t)
        .filter(|(_, u)| !pairs.iter().any(|(x, _)| x == *u))
        .map(|(v, _)| *v)
        .collect();
    assert_eq!(unsupported, ["Ps.14.1", "Ps.50.1"]);
    let back = value(&over(lib.store(), &ts, &ss, "C1", &r)).clone();
    assert_eq!(
        back.values["survival"],
        Num::Ratio(6, 8),
        "the other way round"
    );

    lib.append_records(&rest.iter().map(|p| same_as(p.a, p.b)).collect::<Vec<_>>())
        .unwrap();
    let (all, pairs) = measure(lib.store(), "every candidate");
    for (a, b) in got.iter().zip(&all) {
        assert_eq!(
            a.values, b.values,
            "C1–C4 cannot see which verse a class paired"
        );
    }
    assert_eq!(
        right(&pairs),
        3,
        "Psalm 51's three verses are paired one off"
    );
    drop(lib);
    let _ = std::fs::remove_dir_all(&root);
}
