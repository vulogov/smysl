//! Episode similarity by MinHash (RFC SMYSL-3 §21, TL-P9): which past episodes look like this
//! one. It writes nothing.
//!
//! **Features** are typed strings read from an episode's units. Its members are joined to it by
//! `x.tel/during`:
//! - `series:<uid>` for each member's series: an anomaly's, reset's or stale's `tel:series`, a
//!   change's or recreate's `tel:object`;
//! - `change:<pointer>` for each change;
//! - `anomaly:<series uid>:<direction>` for each anomaly (`-` without a direction);
//! - `alert:<system>:<key>` for each alert;
//! - `template:<text>` for each `x.tel/log-template` whose window overlaps the episode's.
//!
//! **MinHash.** 128 functions `hᵢ(x) = (aᵢ·x + bᵢ) mod (2⁶¹ − 1)`, `x` a feature's BLAKE3
//! as an integer reduced mod 2⁶¹ − 1. The `(aᵢ, bᵢ)` are drawn from ChaCha20 keyed by
//! `BLAKE3("smysl/tel-minhash/1\0" ‖ recipe ‖ seed)`, the seed as eight little-endian bytes,
//! with `aᵢ` in `[1, p)` and `bᵢ` in `[0, p)` by rejection. A signature is the 128 minima, and
//! two episodes' similarity is the number of equal positions out of 128: an estimate of their
//! features' Jaccard index, which [`jaccard`] gives exactly.

use std::collections::{BTreeMap, BTreeSet};

use rand_chacha::ChaCha20Rng;
use rand_core::{RngCore, SeedableRng};
use smysl_core::schemas::tel;
use smysl_core::surface::hjson::HValue;
use smysl_core::{Uid, UnitCore};

/// The number of functions.
pub const K: usize = 128;
/// The key's domain.
pub const DOMAIN: &[u8] = b"smysl/tel-minhash/1\0";
/// 2⁶¹ − 1.
const P: u64 = (1 << 61) - 1;

/// The recipe of a run with `seed`.
pub fn recipe(seed: u64) -> String {
    format!("tel/minhash/1?k={K}&seed={seed}")
}

/// An episode's features: the union over its members and its overlapping templates.
pub fn features<'a>(
    members: impl IntoIterator<Item = &'a UnitCore>,
    templates: impl IntoIterator<Item = &'a UnitCore>,
) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for u in members {
        let Some(o) = u
            .payload
            .as_deref()
            .and_then(|p| smysl_core::surface::payload_to_object(p).ok())
        else {
            continue;
        };
        let text = |k: &str| match o.get(k).map(|e| &e.value) {
            Some(HValue::Str(s)) => Some(s.clone()),
            _ => None,
        };
        let ty = u.schema.as_str();
        let series = if ty == tel::CHANGE || ty == tel::RECREATE {
            text("tel:object")
        } else {
            text("tel:series")
        };
        if let Some(s) = &series {
            out.insert(format!("series:{s}"));
        }
        if ty == tel::CHANGE {
            if let Some(p) = text("tel:pointer") {
                out.insert(format!("change:{p}"));
            }
        } else if ty == tel::ANOMALY {
            let dir = text("tel:direction").unwrap_or_else(|| "-".into());
            out.insert(format!(
                "anomaly:{}:{dir}",
                series.as_deref().unwrap_or("-")
            ));
        } else if ty == tel::ALERT {
            if let (Some(sys), Some(key)) = (text("tel:system"), text("tel:key")) {
                out.insert(format!("alert:{sys}:{key}"));
            }
        }
    }
    for t in templates {
        if let Some(text) = t
            .payload
            .as_deref()
            .and_then(|p| field_text(p, "tel:template"))
        {
            out.insert(format!("template:{text}"));
        }
    }
    out
}

fn field_text(p: &[u8], key: &str) -> Option<String> {
    let mut d = smysl_core::cbor::reader::Dec::new(p);
    let n = d.map_head().ok()?;
    for _ in 0..n {
        if d.text().ok()? == key {
            return d.text().ok().map(str::to_string);
        }
        d.skip_item().ok()?;
    }
    None
}

fn field_uint(p: &[u8], key: &str) -> Option<u64> {
    let mut d = smysl_core::cbor::reader::Dec::new(p);
    let n = d.map_head().ok()?;
    for _ in 0..n {
        if d.text().ok()? == key {
            return d.uint().ok();
        }
        d.skip_item().ok()?;
    }
    None
}

/// The 128 functions of one seed.
#[derive(Debug, Clone)]
pub struct MinHash {
    a: [u64; K],
    b: [u64; K],
}

fn mul_mod(a: u64, b: u64) -> u64 {
    ((u128::from(a) * u128::from(b)) % u128::from(P)) as u64
}

impl MinHash {
    pub fn new(seed: u64) -> MinHash {
        let mut key = Vec::with_capacity(DOMAIN.len() + 64);
        key.extend_from_slice(DOMAIN);
        key.extend_from_slice(recipe(seed).as_bytes());
        key.extend_from_slice(&seed.to_le_bytes());
        let mut r = ChaCha20Rng::from_seed(smysl_core::hash_bytes(&key));
        // A draw below p, by rejection: the top three bits off, and 2⁶¹ − 1 itself refused.
        let mut below = |lo: u64| loop {
            let x = r.next_u64() >> 3;
            if x < P && x >= lo {
                return x;
            }
        };
        let (mut a, mut b) = ([0u64; K], [0u64; K]);
        for i in 0..K {
            a[i] = below(1);
            b[i] = below(0);
        }
        MinHash { a, b }
    }

    /// The signature of a feature set; all `u64::MAX` for the empty set.
    pub fn signature(&self, features: &BTreeSet<String>) -> [u64; K] {
        let xs: Vec<u64> = features
            .iter()
            .map(|f| {
                let h = smysl_core::hash_bytes(f.as_bytes());
                u64::from_le_bytes(h[..8].try_into().expect("eight bytes")) % P
            })
            .collect();
        let mut sig = [u64::MAX; K];
        for (i, s) in sig.iter_mut().enumerate() {
            for &x in &xs {
                let h = (mul_mod(self.a[i], x) + self.b[i]) % P;
                *s = (*s).min(h);
            }
        }
        sig
    }
}

/// Equal positions of two signatures, out of [`K`].
pub fn matches(a: &[u64; K], b: &[u64; K]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x == y).count()
}

/// The exact Jaccard index of two sets, as (intersection, union).
pub fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> (usize, usize) {
    let inter = a.intersection(b).count();
    (inter, a.len() + b.len() - inter)
}

/// Every episode of a store with its features: members by `x.tel/during`, and the templates
/// whose window overlaps the episode's.
pub fn episodes<'a>(
    units: impl IntoIterator<Item = (&'a Uid, &'a UnitCore)>,
    relations: impl IntoIterator<Item = &'a smysl_core::Relation>,
) -> BTreeMap<Uid, BTreeSet<String>> {
    let units: BTreeMap<&Uid, &UnitCore> = units.into_iter().collect();
    let window = |u: &UnitCore| {
        let p = u.payload.as_deref()?;
        Some((field_uint(p, "tel:from_ms")?, field_uint(p, "tel:to_ms")?))
    };
    let mut members: BTreeMap<Uid, Vec<&UnitCore>> = BTreeMap::new();
    for (uid, u) in &units {
        if u.schema.as_str() == tel::EPISODE {
            members.entry(**uid).or_default();
        }
    }
    for r in relations {
        if r.kind.as_str() != tel::DURING || !members.contains_key(&r.to) {
            continue;
        }
        if let Some(m) = units.get(&r.from) {
            members.get_mut(&r.to).expect("an episode").push(m);
        }
    }
    let templates: Vec<(&UnitCore, (u64, u64))> = units
        .values()
        .filter(|u| u.schema.as_str() == tel::LOG_TEMPLATE)
        .filter_map(|u| Some((*u, window(u)?)))
        .collect();
    members
        .into_iter()
        .map(|(ep, ms)| {
            let (from, to) = window(units[&ep]).unwrap_or((0, 0));
            let ts = templates
                .iter()
                .filter(|(_, (f, t))| *f < to && from < *t)
                .map(|(u, _)| *u);
            (ep, features(ms, ts))
        })
        .collect()
}

/// One neighbour of an episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neighbour {
    pub uid: Uid,
    /// Equal signature positions, out of [`K`].
    pub matches: usize,
    /// The exact Jaccard index, as (intersection, union).
    pub exact: (usize, usize),
}

/// The `k` episodes most like `of` by MinHash, most alike first, then by uid.
pub fn nearest(
    episodes: &BTreeMap<Uid, BTreeSet<String>>,
    of: &Uid,
    k: usize,
    seed: u64,
) -> Vec<Neighbour> {
    let Some(mine) = episodes.get(of) else {
        return Vec::new();
    };
    let mh = MinHash::new(seed);
    let sig = mh.signature(mine);
    let mut out: Vec<Neighbour> = episodes
        .iter()
        .filter(|(u, _)| *u != of)
        .map(|(u, f)| Neighbour {
            uid: *u,
            matches: matches(&sig, &mh.signature(f)),
            exact: jaccard(mine, f),
        })
        .collect();
    out.sort_by(|a, b| b.matches.cmp(&a.matches).then(a.uid.cmp(&b.uid)));
    out.truncate(k);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(xs: &[&str]) -> BTreeSet<String> {
        xs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn minhash_estimates_jaccard_and_is_a_function_of_its_seed() {
        let a: BTreeSet<String> = (0..60).map(|i| format!("f{i}")).collect();
        let b: BTreeSet<String> = (20..80).map(|i| format!("f{i}")).collect();
        let (i, u) = jaccard(&a, &b);
        assert_eq!((i, u), (40, 80));
        let mh = MinHash::new(7);
        let est = matches(&mh.signature(&a), &mh.signature(&b)) as f64 / K as f64;
        assert!((est - 0.5).abs() < 0.15, "{est}");
        assert_eq!(mh.signature(&a), MinHash::new(7).signature(&a));
        assert_ne!(mh.signature(&a), MinHash::new(8).signature(&a));
        assert_eq!(matches(&mh.signature(&a), &mh.signature(&a)), K);
        assert_eq!(mh.signature(&set(&[])), [u64::MAX; K]);
    }

    #[test]
    fn nearest_ranks_by_matches_then_uid() {
        let mut eps = BTreeMap::new();
        let u = |n: u8| Uid::from_bytes([n; 32]);
        eps.insert(u(1), set(&["a", "b", "c", "d"]));
        eps.insert(u(2), set(&["a", "b", "c", "e"]));
        eps.insert(u(3), set(&["x", "y"]));
        eps.insert(u(4), set(&["a", "b", "c", "d"]));
        let n = nearest(&eps, &u(1), 2, 0);
        assert_eq!(n[0].uid, u(4));
        assert_eq!(n[0].matches, K);
        assert_eq!(n[1].uid, u(2));
        assert_eq!(n[1].exact, (3, 5));
    }
}
