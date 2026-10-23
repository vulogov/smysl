//! GE-T16 (SMYSL-2.5 §5.3 L8, §5.7; TX-P12 steps 6 and 9): the corpus, `migrate` over it,
//! and the equivalence of the lens path and the migrate path.
//!
//! **The corpus** is built here, deterministically, from two sources:
//!
//! - GE-T10's dossier corpus (`fixtures/dossier/gold/corpus.smy`), its `x.text/v1` declaration
//!   replaced by three revisions with typed fields — Appendix C's at 1, §19.5's widening of
//!   `reported` at 2 (`quoted` split into `reported` plus `text:verbatim = true`), and a key
//!   added at 3 — and a quoted twin injected beside every `reported` claim, written at 1,
//!   mentioning what the original mentions;
//! - a synthetic `x.ge16/v1` with three revisions that use every closed op but `rename-relation`
//!   — a key renamed and its old name retired, a value mapped, a value widened with its paired
//!   key, a type renamed, a key retired — and two hundred units written at random revisions,
//!   a fifth of them grounded on others, with commitments and datings on some.
//!
//! Beside the records it yields a hundred saved queries over both schemas, pinned at random
//! revisions, and the thirty English questions of GE-T10 for dossiers.

#![cfg(all(feature = "cli", feature = "stage"))]

use std::collections::{BTreeMap, BTreeSet};

use smysl::migrate::{migrate, MigrateOptions};
use smysl::schema::{
    encode_lenses, encode_shape, Cond, Field, FieldType, LensOp, LensStep, SchemaSet,
};
use smysl::surface::HValue;
use smysl::{
    canonical_uid, AgentId, Commit, Commitment, Hlc, Op, Record, RelKind, Relation, Report,
    SchemaDecl, SchemaId, SourceKind, SourceRef, Status, Store, Uid, UnitCore, UnitCoreBuilder,
};
use smysl::{Axis, Dating, DatingTarget};

fn sid(s: &str) -> SchemaId {
    SchemaId::parse(s).unwrap()
}

fn s(v: &str) -> HValue {
    HValue::Str(v.into())
}

fn agent() -> AgentId {
    AgentId::new("tool:ge-t16").unwrap()
}

/// The repository's seeded xorshift, as the lens-law tests use it.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
}

pub mod corpus {
    use super::*;

    pub const MODES: [&str; 7] = [
        "asserted",
        "reported",
        "hypothetical",
        "negated",
        "interrogative",
        "imperative",
        "quoted",
    ];

    fn text_fields(rev: u32) -> BTreeMap<String, Field> {
        let mut f = BTreeMap::new();
        let mut holder = Field::new(FieldType::Uid);
        holder.literals = vec!["text".into()];
        f.insert("text:holder".into(), holder);
        let modes: Vec<&str> = MODES
            .iter()
            .copied()
            .filter(|m| rev == 1 || *m != "quoted")
            .collect();
        f.insert("text:mode".into(), Field::enumeration(modes));
        f.insert("text:speaker".into(), Field::new(FieldType::Uid));
        f.insert("text:locator".into(), Field::new(FieldType::Text));
        f.insert("text:when".into(), Field::new(FieldType::Edtf));
        f.insert(
            "text:kind".into(),
            Field::enumeration(["person", "place", "group", "work", "object", "concept"]),
        );
        f.insert("meta:field".into(), Field::new(FieldType::Text));
        f.insert("meta:value".into(), Field::new(FieldType::Text));
        if rev >= 2 {
            let mut v = Field::new(FieldType::Bool);
            v.required_from = Some(2);
            v.required_when = Some(Cond::new("text:mode", s("reported")));
            f.insert("text:verbatim".into(), v);
        }
        if rev >= 3 {
            f.insert("text:tone".into(), Field::new(FieldType::Text));
        }
        f
    }

    /// `x.text/v1` at 1 (Appendix C), 2 (§19.5) and 3.
    pub fn text_decls() -> Vec<SchemaDecl> {
        let when = Some(Cond::new("text:mode", s("reported")));
        let step12 = LensStep::new(
            1,
            2,
            vec![
                LensOp::SplitValue {
                    key: "text:mode".into(),
                    from: "quoted".into(),
                    to: "reported".into(),
                    add: [("text:verbatim".to_string(), HValue::Bool(true))].into(),
                },
                LensOp::Require {
                    key: "text:verbatim".into(),
                    when: when.clone(),
                },
                LensOp::Default {
                    key: "text:verbatim".into(),
                    value: HValue::Bool(false),
                    when,
                },
            ],
        );
        let step23 = LensStep::new(2, 3, vec![]);
        (1..=3)
            .map(|v| {
                let mut d = smysl::schemas::text::decl();
                d.version = v;
                d.payload_shape = Some(encode_shape(&text_fields(v)));
                d.lenses = match v {
                    2 => Some(encode_lenses(std::slice::from_ref(&step12))),
                    3 => Some(encode_lenses(std::slice::from_ref(&step23))),
                    _ => None,
                };
                d
            })
            .collect()
    }

    fn ge_fields(rev: u32) -> BTreeMap<String, Field> {
        let mut f = BTreeMap::new();
        let modes: &[&str] = if rev == 1 {
            &["asserted", "reported", "quoted", "denied"]
        } else {
            &["asserted", "reported", "denied"]
        };
        f.insert("ge:mode".into(), Field::enumeration(modes.iter().copied()));
        let states: &[&str] = if rev == 1 {
            &["open", "ajar", "shut"]
        } else {
            &["open", "half", "shut"]
        };
        f.insert(
            "ge:state".into(),
            Field::enumeration(states.iter().copied()),
        );
        f.insert("ge:weight".into(), Field::new(FieldType::Int));
        let mut topic = Field::new(FieldType::Text);
        if rev >= 2 {
            topic.retired_in = Some(2);
            f.insert("ge:subject".into(), Field::new(FieldType::Text));
            let mut v = Field::new(FieldType::Bool);
            v.required_from = Some(2);
            v.required_when = Some(Cond::new("ge:mode", s("reported")));
            f.insert("ge:verbatim".into(), v);
        }
        f.insert("ge:topic".into(), topic);
        let mut old = Field::new(FieldType::Text);
        if rev >= 3 {
            old.retired_in = Some(3);
        }
        f.insert("ge:old".into(), old);
        f
    }

    /// `x.ge16/v1` at 1, 2 and 3.
    pub fn ge_decls() -> Vec<SchemaDecl> {
        let when = Some(Cond::new("ge:mode", s("reported")));
        let step12 = LensStep::new(
            1,
            2,
            vec![
                LensOp::RenameKey {
                    from: "ge:topic".into(),
                    to: "ge:subject".into(),
                },
                LensOp::MapValue {
                    key: "ge:state".into(),
                    map: [("ajar".to_string(), "half".to_string())].into(),
                },
                LensOp::SplitValue {
                    key: "ge:mode".into(),
                    from: "quoted".into(),
                    to: "reported".into(),
                    add: [("ge:verbatim".to_string(), HValue::Bool(true))].into(),
                },
                LensOp::Require {
                    key: "ge:verbatim".into(),
                    when: when.clone(),
                },
                LensOp::Default {
                    key: "ge:verbatim".into(),
                    value: HValue::Bool(false),
                    when,
                },
            ],
        );
        let step23 = LensStep::new(
            2,
            3,
            vec![
                LensOp::RenameType {
                    from: sid("x.ge16/note"),
                    to: sid("x.ge16/remark"),
                },
                LensOp::Retire {
                    key: "ge:old".into(),
                },
            ],
        );
        (1..=3)
            .map(|v| {
                let mut d = SchemaDecl::new(sid("x.ge16/v1"), v);
                d.types = if v < 3 {
                    vec![sid("x.ge16/item"), sid("x.ge16/note")]
                } else {
                    vec![sid("x.ge16/item"), sid("x.ge16/remark")]
                };
                d.payload_shape = Some(encode_shape(&ge_fields(v)));
                d.lenses = match v {
                    2 => Some(encode_lenses(std::slice::from_ref(&step12))),
                    3 => Some(encode_lenses(std::slice::from_ref(&step23))),
                    _ => None,
                };
                d
            })
            .collect()
    }

    pub const TOPICS: [&str; 6] = [
        "harbour", "ledger", "orchard", "furnace", "archive", "ferry",
    ];

    fn ge_unit(rng: &mut Rng, i: usize, grounds: &[Uid]) -> UnitCore {
        let rev = 1 + rng.below(3) as u32;
        let note = rng.below(3) == 0;
        let ty = match (note, rev) {
            (false, _) => "x.ge16/item",
            (true, 3) => "x.ge16/remark",
            (true, _) => "x.ge16/note",
        };
        let mut p: BTreeMap<String, HValue> = BTreeMap::new();
        let mode = if rev == 1 {
            *rng.pick(&["asserted", "reported", "quoted", "denied"])
        } else {
            *rng.pick(&["asserted", "reported", "denied"])
        };
        p.insert("ge:mode".into(), s(mode));
        if rev >= 2 && mode == "reported" {
            p.insert("ge:verbatim".into(), HValue::Bool(rng.below(2) == 0));
        }
        let topic = *rng.pick(&TOPICS);
        p.insert(
            if rev == 1 { "ge:topic" } else { "ge:subject" }.into(),
            s(topic),
        );
        let state = if rev == 1 {
            *rng.pick(&["open", "ajar", "shut"])
        } else {
            *rng.pick(&["open", "half", "shut"])
        };
        p.insert("ge:state".into(), s(state));
        p.insert("ge:weight".into(), HValue::Int(rng.below(10) as i64));
        if rev < 3 && rng.below(4) == 0 {
            p.insert("ge:old".into(), s("legacy"));
        }
        let view = smysl::schema::UnitView {
            ty: sid(ty),
            payload: p,
        };
        let (status, source) = if grounds.is_empty() {
            (
                Status::Cited,
                Some(SourceRef::new(SourceKind::Doc, format!("ge16:{i}"))),
            )
        } else {
            (Status::Inferred, None)
        };
        let mut b = UnitCoreBuilder::new(view.ty.clone(), format!("item {i} on {topic}"), status)
            .grounds(grounds.iter().copied());
        if let Some(src) = source {
            b = b.source(src);
        }
        if let Some(bytes) = view.payload_bytes() {
            b = b.payload(bytes);
        }
        b.build().unwrap()
    }

    /// The whole corpus: records, the hundred saved queries, and the thirty questions.
    pub struct Corpus {
        pub records: Vec<Record>,
        pub queries: Vec<String>,
        pub questions: Vec<String>,
        /// The injected quotations, by uid.
        pub quoted: BTreeSet<Uid>,
    }

    pub fn build() -> Corpus {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let src = std::fs::read_to_string(root.join("fixtures/dossier/gold/corpus.smy")).unwrap();
        let parsed = smysl::parse_surface(&src).expect("GE-T10's corpus parses");
        let mut records: Vec<Record> = parsed
            .records
            .into_iter()
            .filter(|r| !matches!(r, Record::SchemaDecl(d) if d.id.as_str() == "x.text/v1"))
            .collect();
        records.extend(text_decls().into_iter().map(Record::SchemaDecl));
        records.extend(ge_decls().into_iter().map(Record::SchemaDecl));

        // A quoted twin of every `reported` claim, written at revision 1.
        let mut twins = Vec::new();
        let mut quoted = BTreeSet::new();
        let units: Vec<UnitCore> = records
            .iter()
            .filter_map(|r| r.as_unit().cloned())
            .collect();
        for u in &units {
            let view = smysl::schema::UnitView::of(u);
            if view.payload.get("text:mode") != Some(&s("reported")) {
                continue;
            }
            let mut v = view.clone();
            v.payload.insert("text:mode".into(), s("quoted"));
            let mut twin = u.clone();
            twin.gist = format!("{} (in the speaker's words)", u.gist);
            twin.payload = v.payload_bytes();
            let t = canonical_uid(&twin);
            let original = canonical_uid(u);
            for r in &records {
                if let Record::Relation(rel) = r {
                    if rel.from == original {
                        twins.push(Record::Relation(Relation::new(rel.kind.clone(), t, rel.to)));
                    }
                }
            }
            quoted.insert(t);
            twins.push(Record::Unit(twin));
        }
        records.extend(twins);

        // Two hundred synthetic units, a fifth grounded on earlier ones.
        let mut rng = Rng(0x6e16_7e57_0000_0001);
        let mut made: Vec<Uid> = Vec::new();
        for i in 0..200 {
            let grounds: Vec<Uid> = if i >= 10 && rng.below(5) == 0 {
                (0..1 + rng.below(2))
                    .map(|_| made[rng.below(made.len())])
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect()
            } else {
                Vec::new()
            };
            let u = ge_unit(&mut rng, i, &grounds);
            let uid = canonical_uid(&u);
            made.push(uid);
            records.push(Record::Unit(u));
            if rng.below(8) == 0 {
                records.push(Record::Commit(Commit::new(
                    uid,
                    Commitment::Committed,
                    agent(),
                    Hlc::new(1_000 + i as u64, 0, agent()),
                )));
            }
            if rng.below(12) == 0 {
                let when = smysl::edtf::parse("1854").unwrap();
                records.push(Record::Dating(Dating::absolute(
                    DatingTarget::Unit(uid),
                    Axis::Said,
                    &when,
                    agent(),
                    Hlc::new(2_000 + i as u64, 0, agent()),
                )));
            }
        }

        Corpus {
            records,
            queries: queries(),
            questions: questions(root),
            quoted,
        }
    }

    /// A hundred saved queries, pinned at random revisions, over both schemas: names as the
    /// pinned revision spells them, so every one compiles forward.
    fn queries() -> Vec<String> {
        let mut rng = Rng(0x6e16_9e72_0000_0002);
        let mut out = Vec::new();
        while out.len() < 100 {
            let q = match rng.below(6) {
                0 => {
                    let r = 1 + rng.below(3);
                    let modes: &[&str] = if r == 1 {
                        &["asserted", "reported", "quoted", "denied"]
                    } else {
                        &["asserted", "reported", "denied"]
                    };
                    format!("using x.ge16/v1@{r}\nwhere mode = {}", rng.pick(modes))
                }
                1 => {
                    let r = 1 + rng.below(3);
                    let key = if r == 1 { "topic" } else { "subject" };
                    format!("using x.ge16/v1@{r}\nwhere {key} = {}", rng.pick(&TOPICS))
                }
                2 => {
                    let r = 1 + rng.below(3);
                    let states: &[&str] = if r == 1 {
                        &["open", "ajar", "shut"]
                    } else {
                        &["open", "half", "shut"]
                    };
                    let a = rng.pick(states);
                    let b = rng.pick(states);
                    format!(
                        "using x.ge16/v1@{r}\nwhere state in [{a}, {b}] and weight > {}",
                        rng.below(8)
                    )
                }
                3 => {
                    let r = 1 + rng.below(3);
                    let modes: &[&str] = if r == 1 {
                        &["reported", "quoted"]
                    } else {
                        &["reported", "asserted"]
                    };
                    let key = if r == 1 { "topic" } else { "subject" };
                    format!(
                        "using x.ge16/v1@{r}\nwhere mode = {} and {key} != {}",
                        rng.pick(modes),
                        rng.pick(&TOPICS)
                    )
                }
                4 => {
                    let r = 1 + rng.below(3);
                    let modes: &[&str] = if r == 1 {
                        &["reported", "quoted", "asserted"]
                    } else {
                        &["reported", "asserted"]
                    };
                    format!("using x.text/v1@{r}\nwhere mode = {}", rng.pick(modes))
                }
                _ => {
                    let r = 1 + rng.below(2);
                    format!("using x.ge16/v1@{r}\nwhere not (mode = quoted)\ngroup by state")
                        .replace(
                            "mode = quoted",
                            if r == 1 {
                                "mode = quoted"
                            } else {
                                "mode = denied"
                            },
                        )
                }
            };
            out.push(q);
        }
        out
    }

    fn questions(root: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(root.join("fixtures/dossier/gold/questions.tsv"))
            .unwrap()
            .lines()
            .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
            .filter_map(|l| {
                let cols: Vec<&str> = l.split('\t').collect();
                (cols.len() >= 4 && cols[2] == "en").then(|| cols[3].to_string())
            })
            .collect()
    }
}

/// The migration of one schema over the corpus, committed: the store after, and the pairs.
fn migrated(store: &Store, id: &str) -> (smysl::migrate::Migration, Store) {
    let opts = MigrateOptions::new(sid(id), 3, agent(), Hlc::new(9_000_000, 0, agent()));
    let m = migrate(store, &opts).expect("the corpus migrates");
    let mut records: Vec<Record> = store.iter().cloned().collect();
    records.extend(m.staged.records());
    (m, Store::from_records(records))
}

/// Step 6's exit (§5.7): deterministic migration of the GE-T16 corpus — attestations
/// `Transformed` with the chain hash; rule M respected; old units superseded; `relink`
/// re-points; commitments and datings only proposed (`SMY-W472`).
#[test]
fn migrate_over_the_ge_t16_corpus() {
    let c = corpus::build();
    let store = Store::from_records(c.records.clone());
    assert_eq!(c.queries.len(), 100);
    assert_eq!(c.questions.len(), 30);

    // Rule N holds over both schemas: the corpus is one `schema check` accepts.
    let mut report = Report::new();
    let set = SchemaSet::from_decls(store.schema_decls(), &mut report);
    for id in ["x.text/v1", "x.ge16/v1"] {
        let mut r = Report::new();
        assert_eq!(
            set.check_rule_n(&sid(id), &mut r),
            0,
            "{id}: {:?}",
            r.iter().collect::<Vec<_>>()
        );
    }

    for id in ["x.ge16/v1", "x.text/v1"] {
        let opts = MigrateOptions::new(sid(id), 3, agent(), Hlc::new(9_000_000, 0, agent()));
        let a = migrate(&store, &opts).unwrap();
        let b = migrate(&store, &opts).unwrap();
        assert_eq!(
            a.staged.records(),
            b.staged.records(),
            "{id}: deterministic"
        );
        assert!(!a.pairs.is_empty(), "{id}: something to migrate");
        assert!(a.unmappable.is_empty(), "{id}: {:?}", a.unmappable);
        assert!(
            !a.staged.has_errors(),
            "{id}: {:?}",
            a.staged.report.iter().collect::<Vec<_>>()
        );

        // Each new unit: `Transformed`, from the old one, the chain hash as its recipe.
        let by_new: BTreeMap<Uid, Uid> = a.pairs.iter().map(|(o, n)| (*n, *o)).collect();
        let attested: BTreeMap<Uid, &smysl::Attestation> =
            a.staged.attestations.iter().map(|x| (x.uid, x)).collect();
        for u in &a.staged.units {
            let n = canonical_uid(u);
            let old = by_new[&n];
            let at = attested[&n];
            assert_eq!(at.op, Op::Transformed);
            assert_eq!(at.parents, BTreeSet::from([old]));
            let guess = set.infer_revision(&store.get(&old).unwrap().core).unwrap();
            let chain = set.chain(&sid(id), guess.hi, 3).unwrap();
            assert_eq!(
                at.recipe,
                Some(chain.hash),
                "{id}: the chain hash is the recipe"
            );
            // Rule M: no new unit claims more than the one it replaces.
            assert!(u.status <= store.get(&old).unwrap().core.status);
        }

        // Committed: every old unit is superseded by its new one.
        let (m, after) = migrated(&store, id);
        eprintln!(
            "ge-t16 migrate {id}: {} migrated, {} already at 3, {} commitment(s) and {} \
             dating(s) proposed, {} weakened",
            m.pairs.len(),
            m.current,
            m.proposed_commitments,
            m.proposed_datings,
            m.staged.weakened.len()
        );
        let (heads, forked) = smysl::supersession_heads(&after);
        assert!(forked.is_empty());
        for (old, new) in &m.pairs {
            assert_eq!(heads.get(old), Some(new), "{id}: {old} superseded");
        }

        // Commitments and datings proposed for the new uids, not transferred.
        let commits_on_old = store
            .iter()
            .filter(|r| matches!(r, Record::Commit(c) if by_new.values().any(|o| *o == c.unit)))
            .count();
        assert_eq!(m.proposed_commitments, commits_on_old, "{id}");
        if m.proposed_commitments + m.proposed_datings > 0 {
            assert!(m
                .staged
                .report
                .iter()
                .any(|d| d.code.as_str() == "SMY-W472"));
        }
        for p in &m.staged.proposals {
            match p {
                Record::Commit(c) => assert!(by_new.contains_key(&c.unit)),
                Record::Dating(d) => {
                    assert!(matches!(d.target, DatingTarget::Unit(u) if by_new.contains_key(&u)))
                }
                other => panic!("an unexpected proposal: {other:?}"),
            }
        }

        // `relink` re-points what grounds on a migrated unit; after it, no live unit grounds
        // on a superseded one.
        let relinked = smysl::relink(&after);
        let mut records: Vec<Record> = after.iter().cloned().collect();
        records.extend(relinked.records);
        let fixed = Store::from_records(records);
        let (heads, _) = smysl::supersession_heads(&fixed);
        for (uid, unit) in fixed.units() {
            if heads.contains_key(uid) {
                continue;
            }
            for g in unit.core.grounds.iter().chain(unit.core.deps.iter()) {
                assert!(
                    !heads.contains_key(g),
                    "{id}: {uid} still grounds on superseded {g}"
                );
            }
        }
    }
}

/// The supersession edge's kind, for readers of this file: kernel `supersedes`, new → old.
#[test]
fn a_migrated_unit_supersedes_the_one_it_replaces() {
    let c = corpus::build();
    let store = Store::from_records(c.records);
    let (m, _) = migrated(&store, "x.ge16/v1");
    let (old, new) = m.pairs[0];
    assert!(m
        .staged
        .relations
        .iter()
        .any(|r| r.kind == RelKind::Supersedes && r.from == new && r.to == old));
}

/// Both migrations committed, and the map from each new uid back to the unit it replaces.
fn migrate_path(store: &Store) -> (Store, BTreeMap<Uid, Uid>) {
    let mut records: Vec<Record> = store.iter().cloned().collect();
    let mut back = BTreeMap::new();
    for id in ["x.text/v1", "x.ge16/v1"] {
        let opts = MigrateOptions::new(sid(id), 3, agent(), Hlc::new(9_000_000, 0, agent()));
        let m = migrate(store, &opts).expect("the corpus migrates");
        for (old, new) in &m.pairs {
            back.insert(*new, *old);
        }
        records.extend(m.staged.records());
    }
    (Store::from_records(records), back)
}

fn mapped(uids: &[Uid], back: &BTreeMap<Uid, Uid>) -> BTreeSet<Uid> {
    uids.iter().map(|u| *back.get(u).unwrap_or(u)).collect()
}

/// Step 9's exit (draft 3's TX-P12 exit, GE-T16): the lens path equals the migrate path on all
/// 100 saved queries and 30 dossiers (L8); the effect split reproduces the injected change;
/// the unmappable rate is reported.
#[test]
fn ge_t16_the_lens_path_is_the_migrate_path() {
    use smysl::query::{
        ask, bind, load, plan, rerun, run, to_records, AskOptions, Env, PlanOptions, StoreScope,
    };

    let c = corpus::build();
    let store = Store::from_records(c.records.clone());
    let opts = PlanOptions::default();

    // --- 100 saved queries: saved over the corpus (the lens path), rerun pinned over the
    // migrated corpus (the migrate path). Equal result sets, new uids read as the units they
    // replace.
    let mut saved_records: Vec<Record> = store.iter().cloned().collect();
    saved_records.push(Record::SchemaDecl(smysl::schemas::query::decl()));
    let mut saved: Vec<(String, Uid, BTreeSet<Uid>)> = Vec::new();
    {
        let scope = StoreScope::new("ge-t16", &store);
        let env = Env::new(&store, &scope);
        for q in &c.queries {
            let bound = bind(&smysl::query::parse(q).unwrap().query, &opts.params).unwrap();
            let p = plan(&bound, &env, &opts).unwrap_or_else(|d| panic!("{q}: {d:?}"));
            let r = run(&p, &env, &opts).unwrap_or_else(|d| panic!("{q}: {d:?}"));
            let (unit, edges) = to_records(&bound, &p, &r, &opts).unwrap();
            let uid = canonical_uid(&unit);
            saved.push((q.clone(), uid, r.answer.uids().into_iter().collect()));
            saved_records.push(Record::Unit(unit));
            saved_records.extend(edges.into_iter().map(Record::Relation));
        }
    }
    let with_saved = Store::from_records(saved_records);
    let (migrated, back) = migrate_path(&with_saved);
    // The lens path is each saved query rerun pinned over the store that holds them all, so
    // both paths read one store: lensed, and migrated.
    let lens_path: BTreeMap<Uid, BTreeSet<Uid>> = {
        let scope = StoreScope::new("ge-t16", &with_saved);
        let env = Env::new(&with_saved, &scope);
        saved
            .iter()
            .map(|(q, uid, at_save)| {
                let s = load(&with_saved, uid).unwrap();
                let again = rerun(&s, &env, &opts, false).unwrap_or_else(|d| panic!("{q}: {d:?}"));
                let now: BTreeSet<Uid> = again.run.answer.uids().into_iter().collect();
                // Saving the other ninety-nine is the only data that changed since.
                assert!(
                    now.is_superset(at_save),
                    "{q}: a rerun over more data keeps what the save found"
                );
                (*uid, now)
            })
            .collect()
    };
    let mut nonempty = 0;
    {
        let scope = StoreScope::new("ge-t16", &migrated);
        let env = Env::new(&migrated, &scope);
        for (q, uid, _) in &saved {
            let lens = &lens_path[uid];
            let s = load(&migrated, uid).unwrap();
            let again = rerun(&s, &env, &opts, false).unwrap_or_else(|d| panic!("{q}: {d:?}"));
            let got = mapped(&again.run.answer.uids(), &back);
            if &got != lens {
                let gist = |u: &Uid| {
                    migrated
                        .get(u)
                        .map(|x| format!("{} [{}]", x.core.gist, x.core.schema))
                        .unwrap_or_default()
                };
                let extra: Vec<String> = got.difference(lens).take(5).map(gist).collect();
                let missing: Vec<String> = lens.difference(&got).take(5).map(gist).collect();
                panic!(
                    "L8 on `{q}`: {} extra on the migrate path {extra:?}, {} missing {missing:?}",
                    got.difference(lens).count(),
                    lens.difference(&got).count()
                );
            }
            if !lens.is_empty() {
                nonempty += 1;
            }
        }
    }
    assert!(
        nonempty >= 80,
        "the queries ask about something: {nonempty} of 100 answered"
    );

    // --- 30 dossiers: the same question over both paths, slot by slot.
    let mut asked = 0;
    {
        let s_scope = StoreScope::new("ge-t16", &store);
        let s_env = Env::new(&store, &s_scope).with_entities(&s_scope);
        let m_scope = StoreScope::new("ge-t16", &migrated);
        let m_env = Env::new(&migrated, &m_scope).with_entities(&m_scope);
        let ask_opts = AskOptions::new(agent());
        for question in &c.questions {
            let lens = ask(question, &s_env, &ask_opts);
            let mig = ask(question, &m_env, &ask_opts);
            match (lens, mig) {
                (Ok(a), Ok(b)) => {
                    assert_eq!(a.slots.len(), b.slots.len(), "{question}");
                    for (x, y) in a.slots.iter().zip(&b.slots) {
                        assert_eq!(x.name, y.name);
                        assert_eq!(
                            x.units.iter().copied().collect::<BTreeSet<_>>(),
                            mapped(&y.units, &back),
                            "L8 on `{question}`, slot {}",
                            x.name
                        );
                    }
                    asked += 1;
                }
                (Err(a), Err(b)) => assert_eq!(format!("{a:?}"), format!("{b:?}"), "{question}"),
                (a, b) => panic!(
                    "{question}: one path answered and the other did not: {:?} / {:?}",
                    a.is_ok(),
                    b.is_ok()
                ),
            }
        }
    }
    assert_eq!(c.questions.len(), 30);

    // --- The effect split reproduces the injected change: `mode = reported` saved before
    // revisions 2 and 3 and the quotations existed, rerun `--current` after.
    let before: Vec<Record> = c
        .records
        .iter()
        .filter(|r| match r {
            Record::SchemaDecl(d) => !(d.id.as_str() == "x.text/v1" && d.version > 1),
            Record::Unit(u) => !c.quoted.contains(&canonical_uid(u)),
            Record::Relation(rel) => !c.quoted.contains(&rel.from),
            _ => true,
        })
        .cloned()
        .collect();
    let mut s0 = Store::from_records(before);
    let q = "where text:mode = reported";
    let (saved_uid, recs) = {
        let scope = StoreScope::new("ge-t16", &s0);
        let env = Env::new(&s0, &scope);
        let bound = bind(&smysl::query::parse(q).unwrap().query, &opts.params).unwrap();
        let p = plan(&bound, &env, &opts).unwrap();
        let r = run(&p, &env, &opts).unwrap();
        let (u, edges) = to_records(&bound, &p, &r, &opts).unwrap();
        let mut recs = vec![
            Record::SchemaDecl(smysl::schemas::query::decl()),
            Record::Unit(u.clone()),
        ];
        recs.extend(edges.into_iter().map(Record::Relation));
        (canonical_uid(&u), recs)
    };
    s0.append(&recs).unwrap();
    let injected: Vec<Record> = c
        .records
        .iter()
        .filter(|r| match r {
            Record::SchemaDecl(d) => d.id.as_str() == "x.text/v1" && d.version > 1,
            Record::Unit(u) => c.quoted.contains(&canonical_uid(u)),
            Record::Relation(rel) => c.quoted.contains(&rel.from),
            _ => false,
        })
        .cloned()
        .collect();
    s0.append(&injected).unwrap();
    let scope = StoreScope::new("ge-t16", &s0);
    let env = Env::new(&s0, &scope);
    let reported = load(&s0, &saved_uid).unwrap();
    let pinned = rerun(&reported, &env, &opts, false).unwrap();
    assert!(
        pinned.data_effect.is_empty(),
        "the data effect is nothing: {:?}",
        pinned.data_effect
    );
    let current = rerun(&reported, &env, &opts, true).unwrap();
    let effect = current.schema_effect.unwrap();
    assert_eq!(
        effect.added.iter().copied().collect::<BTreeSet<_>>(),
        c.quoted,
        "the schema effect is exactly the injected quotations"
    );
    assert!(effect.removed.is_empty());

    // --- The unmappable rate, over every unit either schema governs.
    let mut report = Report::new();
    let set = SchemaSet::from_decls(store.schema_decls(), &mut report);
    let mut units = 0;
    let mut lost = 0;
    for id in ["x.text/v1", "x.ge16/v1"] {
        let t = set.check_units(
            &sid(id),
            store.units().map(|(u, x)| (u, &x.core)),
            &mut report,
        );
        units += t.units;
        lost += t.unmappable.len();
    }
    eprintln!(
        "ge-t16: {} saved queries ({nonempty} answering), {asked} of 30 dossiers answered on both \
         paths, schema effect {} unit(s) = the {} injected; unmappable {lost} of {units} unit(s)",
        saved.len(),
        effect.added.len(),
        c.quoted.len()
    );
    assert_eq!(lost, 0, "the corpus is total under its lenses");
}
