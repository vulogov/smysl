//! Lens laws L1–L7 (RFC SMYSL-2.5 §5.3, D-11), at 10,000 seeded cases each (TX-P12 step 2).
//!
//! The generator writes **random revision histories of one to four steps over the closed ops**,
//! each step one to three ops, and every history obeys rule N (A-12.1) — names are never reused,
//! a renamed or retired key is marked retired, a widening carries its paired key, a default is
//! only ever given to a key required from the same revision. Units are drawn from the declared
//! fields of a revision. The laws are statements about lenses over histories rule N admits; a
//! history that broke it would be testing something else.
//!
//! The histories are built as declarations and decoded through [`SchemaSet`], so every case also
//! exercises FC-9 and the path search. The generator is the repository's seeded xorshift, as in
//! `surface.rs` and `quote_v2.rs`: a failure names its seed.

use std::collections::{BTreeMap, BTreeSet};

use smysl_core::schema::{
    encode_lenses, encode_shape, Cond, Field, FieldType, LensChain, LensError, LensOp, LensStep,
    Mapped, SchemaSet, UnitView,
};
use smysl_core::surface::HValue;
use smysl_core::types::{SchemaDecl, Status, UnitCoreBuilder};
use smysl_core::{Code, Report, SchemaId};

const CASES: u64 = 10_000;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
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
    fn coin(&mut self) -> bool {
        self.next() & 1 == 0
    }
    fn pick<'a, T>(&mut self, v: &'a [T]) -> Option<&'a T> {
        if v.is_empty() {
            None
        } else {
            Some(&v[self.below(v.len())])
        }
    }
}

fn sid(s: &str) -> SchemaId {
    SchemaId::parse(s).unwrap()
}

fn s(v: &str) -> HValue {
    HValue::Str(v.into())
}

const ID: &str = "x.g/v1";

/// A generated history: one declaration per revision, `1..=n`.
struct History {
    decls: Vec<SchemaDecl>,
    /// The field table of each revision (index `rev - 1`), retired keys included.
    tables: Vec<BTreeMap<String, Field>>,
    types: Vec<SchemaId>,
    /// Revision `r + 1`'s step has no inverse.
    lossy: Vec<bool>,
    retires: Vec<bool>,
}

impl History {
    fn current(&self) -> u32 {
        self.decls.len() as u32
    }

    fn set(&self) -> SchemaSet {
        let mut r = Report::new();
        let set = SchemaSet::from_decls(&self.decls, &mut r);
        assert!(r.is_empty(), "a generated history decodes cleanly: {r:?}");
        set
    }

    fn invertible(&self, from: u32, to: u32) -> bool {
        (from..to).all(|r| !self.lossy[r as usize])
    }

    fn retires(&self, from: u32, to: u32) -> bool {
        (from..to).any(|r| self.retires[r as usize])
    }
}

struct Gen {
    rng: Rng,
    fresh: u32,
}

impl Gen {
    fn name(&mut self, prefix: &str) -> String {
        self.fresh += 1;
        format!("{prefix}{}", self.fresh)
    }

    /// One history of one to four steps. `allow_lossy` admits `retire` and many-to-one maps.
    fn history(&mut self, allow_lossy: bool) -> History {
        let mut table: BTreeMap<String, Field> = BTreeMap::new();
        for _ in 0..2 + self.rng.below(3) {
            let k = self.name("k");
            let f = if self.rng.coin() {
                let n = 2 + self.rng.below(3);
                let vals: Vec<String> = (0..n).map(|_| self.name("v")).collect();
                Field::enumeration(vals)
            } else {
                Field::new(FieldType::Text)
            };
            table.insert(k, f);
        }
        let mut ty = sid(&format!("x.g/{}", self.name("t")));
        let mut decl = SchemaDecl::new(sid(ID), 1);
        decl.types = vec![ty.clone()];
        decl.payload_shape = Some(encode_shape(&table));
        let mut h = History {
            decls: vec![decl],
            tables: vec![table.clone()],
            types: vec![ty.clone()],
            lossy: vec![false, false],
            retires: vec![false, false],
        };
        // Keys a later op must not rename, retire or remap: required ones and those a
        // condition names. Values a condition names are pinned the same way.
        let mut pinned: BTreeSet<String> = BTreeSet::new();
        let mut cond_values: BTreeSet<(String, String)> = BTreeSet::new();
        let steps = 1 + self.rng.below(4);
        for from in 1..=steps as u32 {
            let to = from + 1;
            let mut ops = Vec::new();
            let mut used: BTreeSet<String> = BTreeSet::new();
            let (mut lossy, mut retired) = (false, false);
            let want = 1 + self.rng.below(3);
            let mut tries = 0;
            while ops.len() < want && tries < 20 {
                tries += 1;
                let live: Vec<String> = table
                    .iter()
                    .filter(|(k, f)| !f.retired_at(to) && !used.contains(*k))
                    .map(|(k, _)| k.clone())
                    .collect();
                let free: Vec<String> = live
                    .iter()
                    .filter(|k| !pinned.contains(*k))
                    .cloned()
                    .collect();
                let enums: Vec<String> = live
                    .iter()
                    .filter(|k| table[*k].ty == FieldType::Enum && !pinned.contains(*k))
                    .cloned()
                    .collect();
                let movable = |k: &String, cv: &BTreeSet<(String, String)>| -> Vec<String> {
                    table[k]
                        .values
                        .iter()
                        .filter(|v| !cv.contains(&(k.clone(), (*v).clone())))
                        .cloned()
                        .collect()
                };
                match self.rng.below(if allow_lossy { 8 } else { 6 }) {
                    0 if !used.contains("#type") => {
                        used.insert("#type".into());
                        let next = sid(&format!("x.g/{}", self.name("t")));
                        ops.push(LensOp::RenameType {
                            from: ty.clone(),
                            to: next.clone(),
                        });
                        ty = next;
                    }
                    1 => {
                        let Some(k) = self.rng.pick(&free).cloned() else {
                            continue;
                        };
                        let k2 = self.name("k");
                        let mut f = table[&k].clone();
                        f.retired_in = None;
                        table.get_mut(&k).unwrap().retired_in = Some(to);
                        table.insert(k2.clone(), f);
                        used.insert(k.clone());
                        used.insert(k2.clone());
                        ops.push(LensOp::RenameKey { from: k, to: k2 });
                    }
                    2 => {
                        let Some(k) = self.rng.pick(&enums).cloned() else {
                            continue;
                        };
                        let vals = movable(&k, &cond_values);
                        let Some(a) = self.rng.pick(&vals).cloned() else {
                            continue;
                        };
                        let b = self.name("v");
                        let f = table.get_mut(&k).unwrap();
                        f.values.retain(|v| v != &a);
                        f.values.push(b.clone());
                        used.insert(k.clone());
                        ops.push(LensOp::MapValue {
                            key: k,
                            map: [(a, b)].into(),
                        });
                    }
                    3 => {
                        // A split that is not a widening: the value goes, a new one and a new
                        // key come.
                        let Some(k) = self.rng.pick(&enums).cloned() else {
                            continue;
                        };
                        let vals = movable(&k, &cond_values);
                        let Some(a) = self.rng.pick(&vals).cloned() else {
                            continue;
                        };
                        let b = self.name("v");
                        let k2 = self.name("k");
                        let x = self.name("x");
                        let f = table.get_mut(&k).unwrap();
                        f.values.retain(|v| v != &a);
                        f.values.push(b.clone());
                        table.insert(k2.clone(), Field::new(FieldType::Text));
                        used.insert(k.clone());
                        used.insert(k2.clone());
                        ops.push(LensOp::SplitValue {
                            key: k,
                            from: a,
                            to: b,
                            add: [(k2, s(&x))].into(),
                        });
                    }
                    4 => {
                        // A widening: `a` is read as the existing `w`, with a paired key that
                        // is required, and defaulted, when the key is `w`.
                        let Some(k) = self.rng.pick(&enums).cloned() else {
                            continue;
                        };
                        let vals = movable(&k, &cond_values);
                        if vals.len() < 2 {
                            continue;
                        }
                        let a = vals[self.rng.below(vals.len())].clone();
                        let w = table[&k].values.iter().find(|v| **v != a).cloned().unwrap();
                        let k2 = self.name("k");
                        let (d, x) = (self.name("d"), self.name("x"));
                        let when = Some(Cond::new(k.clone(), s(&w)));
                        table.get_mut(&k).unwrap().values.retain(|v| v != &a);
                        let mut f2 = Field::enumeration([d.clone(), x.clone()]);
                        f2.required_from = Some(to);
                        f2.required_when = when.clone();
                        table.insert(k2.clone(), f2);
                        pinned.insert(k.clone());
                        pinned.insert(k2.clone());
                        cond_values.insert((k.clone(), w.clone()));
                        used.insert(k.clone());
                        used.insert(k2.clone());
                        ops.push(LensOp::SplitValue {
                            key: k,
                            from: a,
                            to: w,
                            add: [(k2.clone(), s(&x))].into(),
                        });
                        ops.push(LensOp::Require {
                            key: k2.clone(),
                            when: when.clone(),
                        });
                        ops.push(LensOp::Default {
                            key: k2,
                            value: s(&d),
                            when,
                        });
                    }
                    5 => {
                        let n = self.name("k");
                        let mut f = Field::new(FieldType::Int);
                        f.required_from = Some(to);
                        table.insert(n.clone(), f);
                        pinned.insert(n.clone());
                        used.insert(n.clone());
                        ops.push(LensOp::Require {
                            key: n.clone(),
                            when: None,
                        });
                        ops.push(LensOp::Default {
                            key: n,
                            value: HValue::Int(0),
                            when: None,
                        });
                    }
                    6 => {
                        let Some(k) = self.rng.pick(&free).cloned() else {
                            continue;
                        };
                        table.get_mut(&k).unwrap().retired_in = Some(to);
                        used.insert(k.clone());
                        ops.push(LensOp::Retire { key: k });
                        lossy = true;
                        retired = true;
                    }
                    7 => {
                        // Many to one: no inverse.
                        let Some(k) = self.rng.pick(&enums).cloned() else {
                            continue;
                        };
                        let vals = movable(&k, &cond_values);
                        if vals.len() < 2 {
                            continue;
                        }
                        let (a, c) = (vals[0].clone(), vals[1].clone());
                        let b = self.name("v");
                        let f = table.get_mut(&k).unwrap();
                        f.values.retain(|v| v != &a && v != &c);
                        f.values.push(b.clone());
                        used.insert(k.clone());
                        ops.push(LensOp::MapValue {
                            key: k,
                            map: [(a, b.clone()), (c, b)].into(),
                        });
                        lossy = true;
                    }
                    _ => {}
                }
            }
            if ops.is_empty() {
                ops.push(LensOp::RenameType {
                    from: ty.clone(),
                    to: {
                        let next = sid(&format!("x.g/{}", self.name("t")));
                        ty = next.clone();
                        next
                    },
                });
            }
            let mut decl = SchemaDecl::new(sid(ID), to);
            decl.types = vec![ty.clone()];
            decl.payload_shape = Some(encode_shape(&table));
            decl.lenses = Some(encode_lenses(&[LensStep::new(from, to, ops)]));
            h.decls.push(decl);
            h.tables.push(table.clone());
            h.types.push(ty.clone());
            h.lossy.push(false);
            h.retires.push(false);
            h.lossy[from as usize] = lossy;
            h.retires[from as usize] = retired;
        }
        h
    }

    /// A unit as revision `rev` writes it: every live field it requires, the others by coin.
    fn unit(&mut self, h: &History, rev: u32) -> UnitView {
        let table = &h.tables[rev as usize - 1];
        let mut payload = BTreeMap::new();
        let required = |f: &Field| f.required_from.is_some_and(|r| r <= rev);
        let value = |g: &mut Gen, f: &Field| match f.ty {
            FieldType::Enum => s(&f.values[g.rng.below(f.values.len())]),
            FieldType::Int => HValue::Int(g.rng.below(3) as i64),
            _ => s(&format!("t{}", g.rng.below(4))),
        };
        for (k, f) in table {
            if f.retired_at(rev) || f.required_when.is_some() {
                continue;
            }
            if required(f) || self.rng.coin() {
                payload.insert(k.clone(), value(self, f));
            }
        }
        for (k, f) in table {
            if f.retired_at(rev) {
                continue;
            }
            if let Some(c) = &f.required_when {
                let must = required(f) && c.holds(&payload);
                if must || self.rng.coin() {
                    payload.insert(k.clone(), value(self, f));
                }
            }
        }
        UnitView {
            ty: h.types[rev as usize - 1].clone(),
            payload,
        }
    }
}

fn gen(seed: u64) -> Gen {
    Gen {
        rng: Rng::new(seed),
        fresh: 0,
    }
}

/// A random sub-range `from < to` of a history's revisions.
fn span(g: &mut Gen, h: &History) -> (u32, u32) {
    let n = h.current();
    let from = 1 + g.rng.below(n as usize - 1) as u32;
    let to = from + 1 + g.rng.below((n - from) as usize) as u32;
    (from, to)
}

#[test]
fn l1_round_trip() {
    let mut checked = 0;
    for seed in 0..CASES {
        let mut g = gen(seed);
        let h = g.history(false);
        let (a, b) = span(&mut g, &h);
        let c = h.set().chain(&sid(ID), a, b).unwrap();
        assert!(c.is_invertible(), "seed {seed}");
        let u = g.unit(&h, a);
        let mut v = u.clone();
        assert_eq!(
            c.forward(&mut v),
            Mapped::Whole,
            "seed {seed}: {u:?} → {v:?}"
        );
        let mut w = v.clone();
        assert_eq!(c.backward(&mut w), Ok(Mapped::Whole), "seed {seed}");
        assert_eq!(w, u, "seed {seed}: backward(forward(u)) != u");
        checked += 1;
    }
    assert_eq!(checked, CASES);
}

#[test]
fn l2_image() {
    let mut in_image = 0;
    for seed in 0..CASES {
        let mut g = gen(seed);
        let h = g.history(false);
        let (a, b) = span(&mut g, &h);
        let c = h.set().chain(&sid(ID), a, b).unwrap();
        // Half the cases draw `v` from the image directly; half draw any unit of `b` and keep
        // it when backward takes every name back, which is what being in the image means.
        let v = if g.rng.coin() {
            let mut v = g.unit(&h, a);
            c.forward(&mut v);
            v
        } else {
            g.unit(&h, b)
        };
        let mut w = v.clone();
        if c.backward(&mut w) != Ok(Mapped::Whole) {
            continue;
        }
        let mut again = w.clone();
        c.forward(&mut again);
        assert_eq!(again, v, "seed {seed}: forward(backward(v)) != v");
        in_image += 1;
    }
    assert!(in_image > CASES / 2, "{in_image} cases in the image");
}

#[test]
fn l3_associativity() {
    let mut checked = 0;
    for seed in 0..CASES {
        let mut g = gen(seed);
        let h = g.history(true);
        if h.current() < 4 {
            // Three non-empty pieces need three steps; fewer, and one piece is an identity,
            // which is L4's.
            let set = h.set();
            let n = h.current();
            let i = LensChain::identity(sid(ID), 1);
            let c = set.chain(&sid(ID), 1, n).unwrap();
            let l = i.clone().compose(c.clone()).unwrap();
            assert_eq!(l.hash, c.hash, "seed {seed}");
            checked += 1;
            continue;
        }
        let set = h.set();
        let n = h.current();
        let x = 2 + g.rng.below(n as usize - 3) as u32;
        let y = x + 1 + g.rng.below((n - x - 1) as usize) as u32;
        let a = set.chain(&sid(ID), 1, x).unwrap();
        let b = set.chain(&sid(ID), x, y).unwrap();
        let c = set.chain(&sid(ID), y, n).unwrap();
        let left = a
            .clone()
            .compose(b.clone())
            .unwrap()
            .compose(c.clone())
            .unwrap();
        let right = a.compose(b.compose(c).unwrap()).unwrap();
        let whole = set.chain(&sid(ID), 1, n).unwrap();
        assert_eq!(left.hash, right.hash, "seed {seed}");
        assert_eq!(left.hash, whole.hash, "seed {seed}");
        let u = g.unit(&h, 1);
        let (mut p, mut q) = (u.clone(), u.clone());
        assert_eq!(left.forward(&mut p), right.forward(&mut q), "seed {seed}");
        assert_eq!(p, q, "seed {seed}");
        if left.is_invertible() {
            let v = p.clone();
            let (mut p, mut q) = (v.clone(), v);
            assert_eq!(left.backward(&mut p), right.backward(&mut q), "seed {seed}");
            assert_eq!(p, q, "seed {seed}");
        }
        checked += 1;
    }
    assert_eq!(checked, CASES);
}

#[test]
fn l4_identity() {
    for seed in 0..CASES {
        let mut g = gen(seed);
        let h = g.history(true);
        let (a, b) = span(&mut g, &h);
        let set = h.set();
        let c = set.chain(&sid(ID), a, b).unwrap();
        let left = LensChain::identity(sid(ID), a).compose(c.clone()).unwrap();
        let right = c.clone().compose(LensChain::identity(sid(ID), b)).unwrap();
        let from_set = set
            .chain(&sid(ID), a, a)
            .unwrap()
            .compose(c.clone())
            .unwrap();
        for l in [&left, &right, &from_set] {
            assert_eq!(l.hash, c.hash, "seed {seed}");
            assert_eq!(l.steps(), c.steps(), "seed {seed}");
            let u = g.unit(&h, a);
            let (mut p, mut q) = (u.clone(), u);
            assert_eq!(l.forward(&mut p), c.forward(&mut q), "seed {seed}");
            assert_eq!(p, q, "seed {seed}");
        }
        // The identity changes nothing.
        let u = g.unit(&h, a);
        let mut v = u.clone();
        assert_eq!(
            LensChain::identity(sid(ID), a).forward(&mut v),
            Mapped::Whole
        );
        assert_eq!(v, u);
    }
}

#[test]
fn l5_inference_is_harmless() {
    let mut narrowed = 0;
    for seed in 0..CASES {
        let mut g = gen(seed);
        let h = g.history(true);
        let set = h.set();
        let n = h.current();
        let r = 1 + g.rng.below(n as usize) as u32;
        let view = g.unit(&h, r);
        let mut b = UnitCoreBuilder::new(view.ty.clone(), "g", Status::Speculative);
        if let Some(p) = view.payload_bytes() {
            b = b.payload(p);
        }
        let core = b.build().unwrap();
        let guess = set.infer_revision(&core).expect("one id governs the type");
        // Sound: the revision the unit was written at is consistent with its clues.
        assert!(
            guess.lo <= r && r <= guess.hi,
            "seed {seed}: written at {r}, inferred {}..={}",
            guess.lo,
            guess.hi
        );
        if guess.hi < n {
            narrowed += 1;
        }
        // Harmless: every consistent revision reads the unit the same at the current one.
        let mut want = UnitView::of(&core);
        set.chain(&sid(ID), guess.hi, n).unwrap().forward(&mut want);
        for from in guess.lo..=guess.hi {
            let mut got = UnitView::of(&core);
            set.chain(&sid(ID), from, n).unwrap().forward(&mut got);
            assert_eq!(got, want, "seed {seed}: from {from}, hi {}", guess.hi);
        }
    }
    assert!(
        narrowed > CASES / 4,
        "{narrowed} units narrowed below current"
    );
}

#[test]
fn l6_retire_refuses_backward() {
    let mut met = 0;
    for seed in 0..CASES {
        let mut g = gen(seed);
        let h = g.history(true);
        let (a, b) = span(&mut g, &h);
        let c = h.set().chain(&sid(ID), a, b).unwrap();
        let mut v = g.unit(&h, b);
        let before = v.clone();
        let got = c.backward(&mut v);
        if h.retires(a, b) {
            met += 1;
            assert!(c.has_retire(), "seed {seed}");
            let e = got.expect_err("a chain with retire has no backward reading");
            // A retire, or a many-to-one map met first, refuses: both are E422.
            assert_eq!(e.code(), Code::E422, "seed {seed}: {e}");
            assert_eq!(v, before, "seed {seed}: a refused backward changed nothing");
        } else if !h.invertible(a, b) {
            assert!(
                matches!(got, Err(LensError::NotInvertible { .. })),
                "seed {seed}"
            );
        } else {
            assert!(got.is_ok(), "seed {seed}");
        }
    }
    assert!(met > CASES / 5, "{met} chains with a retire");
}

#[test]
fn l7_never_guess() {
    let mut met = 0;
    for seed in 0..CASES {
        let mut g = gen(seed);
        let h = g.history(true);
        let (a, b) = span(&mut g, &h);
        let c = h.set().chain(&sid(ID), a, b).unwrap();
        let mut u = g.unit(&h, a);
        let enums: Vec<String> = u
            .payload
            .keys()
            .filter(|k| h.tables[a as usize - 1][*k].ty == FieldType::Enum)
            .cloned()
            .collect();
        let Some(k) = g.rng.pick(&enums).cloned() else {
            continue;
        };
        let bogus = format!("zz{seed}");
        u.payload.insert(k, s(&bogus));
        let mut v = u.clone();
        let mapped = c.forward(&mut v);
        let Some((at, _)) = v.payload.iter().find(|(_, x)| **x == s(&bogus)) else {
            // Retired on the way: absent, which is what retire reads as, not a guess.
            continue;
        };
        met += 1;
        match mapped {
            Mapped::Partial { unmappable } => {
                assert!(unmappable.contains(at), "seed {seed}: {at} not reported")
            }
            m => panic!("seed {seed}: a value outside every map read as {m:?}"),
        }
    }
    assert!(met > CASES / 2, "{met} cases carried the value through");
}

// --- the errors the engine raises, one at a time --------------------------------------------

fn two(ops: Vec<LensOp>, f1: BTreeMap<String, Field>) -> Vec<SchemaDecl> {
    let mut d1 = SchemaDecl::new(sid("x.e/v1"), 1);
    d1.types = vec![sid("x.e/t")];
    d1.payload_shape = Some(encode_shape(&f1));
    let mut d2 = SchemaDecl::new(sid("x.e/v1"), 2);
    d2.types = vec![sid("x.e/t")];
    d2.lenses = Some(encode_lenses(&[LensStep::new(1, 2, ops)]));
    vec![d1, d2]
}

fn chain_of(decls: &[SchemaDecl], from: u32, to: u32) -> Result<LensChain, LensError> {
    SchemaSet::from_decls(decls, &mut Report::new()).chain(&sid("x.e/v1"), from, to)
}

fn table(entries: &[(&str, Field)]) -> BTreeMap<String, Field> {
    entries
        .iter()
        .map(|(k, f)| (k.to_string(), f.clone()))
        .collect()
}

#[test]
fn no_path_is_e470() {
    let d = two(vec![], table(&[]));
    assert_eq!(chain_of(&d, 2, 1).unwrap_err().code(), Code::E470);
    assert_eq!(chain_of(&d, 1, 3).unwrap_err().code(), Code::E470);
    chain_of(&d, 1, 2).unwrap();
    let a = chain_of(&d, 1, 2).unwrap();
    assert_eq!(a.clone().compose(a).unwrap_err().code(), Code::E470);
}

#[test]
fn a_default_without_its_require_is_e471() {
    let ops = vec![LensOp::Default {
        key: "n".into(),
        value: HValue::Int(0),
        when: None,
    }];
    let e = chain_of(&two(ops, table(&[])), 1, 2).unwrap_err();
    assert_eq!(e.code(), Code::E471, "{e}");
}

#[test]
fn an_op_on_an_undeclared_key_is_e471() {
    let ops = vec![LensOp::RenameKey {
        from: "nobody".into(),
        to: "x".into(),
    }];
    let f = table(&[("k", Field::new(FieldType::Text))]);
    assert_eq!(chain_of(&two(ops, f), 1, 2).unwrap_err().code(), Code::E471);
}

#[test]
fn a_widening_without_its_paired_key_is_e471_and_with_it_is_listed() {
    let f = table(&[("k", Field::enumeration(["a", "w"]))]);
    let split = LensOp::SplitValue {
        key: "k".into(),
        from: "a".into(),
        to: "w".into(),
        add: [("k2".to_string(), s("x"))].into(),
    };
    let e = chain_of(&two(vec![split.clone()], f.clone()), 1, 2).unwrap_err();
    assert_eq!(e.code(), Code::E471, "{e}");

    let when = Some(Cond::new("k", s("w")));
    let ops = vec![
        split,
        LensOp::Require {
            key: "k2".into(),
            when: when.clone(),
        },
        LensOp::Default {
            key: "k2".into(),
            value: s("d"),
            when,
        },
    ];
    let c = chain_of(&two(ops, f), 1, 2).unwrap();
    let w = c.widenings();
    assert_eq!(w.len(), 1);
    assert_eq!((w[0].key.as_str(), w[0].value.as_str()), ("k", "w"));
    assert_eq!(w[0].paired, vec!["k2".to_string()]);
}

#[test]
fn an_unknown_op_leaves_units_unmappable_and_refuses_backward() {
    let ops = vec![LensOp::Unknown {
        tag: "transpose".into(),
        bytes: vec![
            0xa1, 0x00, 0x69, b't', b'r', b'a', b'n', b's', b'p', b'o', b's', b'e',
        ],
    }];
    let c = chain_of(&two(ops, table(&[])), 1, 2).unwrap();
    let mut v = UnitView {
        ty: sid("x.e/t"),
        payload: BTreeMap::new(),
    };
    assert_eq!(c.forward(&mut v), Mapped::Unusable);
    let e = c.backward(&mut v).unwrap_err();
    assert_eq!(e.code(), Code::W469);
}

#[test]
fn relation_kinds_are_renamed_where_they_are_followed() {
    use smysl_core::types::RelKind;
    let from = RelKind::parse("x.e/cites").unwrap();
    let to = RelKind::parse("x.e/quotes").unwrap();
    let ops = vec![LensOp::RenameRelation {
        from: from.clone(),
        to: to.clone(),
    }];
    let c = chain_of(&two(ops, table(&[])), 1, 2).unwrap();
    assert_eq!(c.relation_forward(&from), to);
    assert_eq!(c.relation_backward(&to), from);
    assert_eq!(c.relation_forward(&RelKind::Causes), RelKind::Causes);
}
