//! T6, normalised compression distance (SMYSL-2.6 §3.7; Cilibrasi, Vitányi 2005).
//!
//! `NCD(x, y) = (min(C(xy), C(yx)) − min(C(x), C(y))) / max(C(x), C(y))`, with C T2's codec and
//! `xy` the two texts as one, joined by `\n` as a text's own segments are. The min over both
//! orders removes the compressor's asymmetry, so the matrix is symmetric by construction:
//! each pair is computed once.
//!
//! The grouping is the scope's expressions, in alias order. A library names a book as an
//! alias (`kjv/08-rut`), so a scope of one Bible's books is a matrix over books; speakers
//! wait for the measures over messages.
//!
//! A pair longer than the window is refused (`SMY-E484`) before anything is compressed: the
//! compressor would have lost x by the time it coded y, and C(xy) would be C(x) + C(y) whatever
//! the texts were. `min:n` is in texts and is 2, since a distance needs two.
//!
//! **Average-linkage clustering** follows the matrix: the closest two clusters merge, at the
//! mean distance between their members, until one is left. A cluster's label is its smallest
//! leaf, and the closest pair is found by scanning pairs in label order and keeping the first
//! strict minimum, so a tie goes to the pair whose labels come first. The merges are the
//! series `linkage`, four numbers each, as SciPy writes one: the two nodes, smaller id first,
//! the height, and the size. Leaves are `0..n` in alias order — the order of the
//! `ncd:` keys — and the k-th merge is node `n + k`.

use smysl_core::diag::Code;

use crate::num::q20;
use crate::outcome::{Num, Outcome, Refusal, Value};
use crate::recipe::Recipe;
use crate::text::compress::{codec, compress_into, refused_codec, CODEC, WINDOW};
use crate::text::entropy::corpus;
use crate::{Input, Measure, Needs, Purity};

/// T6's `min:n`, in texts.
pub const MIN_N: u64 = 2;

/// T6.
#[derive(Debug, Clone, Copy, Default)]
pub struct T6;

/// The NCD of two texts whose compressed lengths are `cx` and `cy`, given both orders' `cxy`
/// and `cyx`. Can be a little below 0 for a compressor that is not normal, and is stored as it
/// comes.
pub fn ncd(cx: u64, cy: u64, cxy: u64, cyx: u64) -> f64 {
    let num = cxy.min(cyx) as f64 - cx.min(cy) as f64;
    num / cx.max(cy) as f64
}

/// One merge: the two nodes, smaller id first, the height and the new cluster's size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Merge {
    pub a: usize,
    pub b: usize,
    pub height: f64,
    pub size: usize,
}

/// Average linkage over a symmetric matrix `d` (row-major, `n × n`), with ties to the pair
/// whose labels come first.
pub fn average_linkage(d: &[f64], n: usize) -> Vec<Merge> {
    // Active clusters, in label order: (label, node id, size). Distances between clusters by
    // label, updated by Lance–Williams for the average: d(A∪B, C) = (|A|·d(A,C) + |B|·d(B,C)) /
    // (|A| + |B|), in the same order on every machine.
    let mut dist = d.to_vec();
    let mut active: Vec<(usize, usize, usize)> = (0..n).map(|i| (i, i, 1)).collect();
    let mut merges = Vec::with_capacity(n.saturating_sub(1));
    while active.len() > 1 {
        let mut best: Option<(usize, usize, f64)> = None;
        for i in 0..active.len() {
            for j in i + 1..active.len() {
                let x = dist[active[i].0 * n + active[j].0];
                if best.is_none_or(|(_, _, b)| x < b) {
                    best = Some((i, j, x));
                }
            }
        }
        let (i, j, height) = best.expect("two clusters");
        let (la, ida, sa) = active[i];
        let (lb, idb, sb) = active[j];
        for &(lc, _, _) in &active {
            if lc == la || lc == lb {
                continue;
            }
            let v =
                (sa as f64 * dist[la * n + lc] + sb as f64 * dist[lb * n + lc]) / (sa + sb) as f64;
            dist[la * n + lc] = v;
            dist[lc * n + la] = v;
        }
        merges.push(Merge {
            a: ida.min(idb),
            b: ida.max(idb),
            height,
            size: sa + sb,
        });
        // The merged cluster keeps the smaller label, `la`, which is where `i` was.
        active[i] = (la, n + merges.len() - 1, sa + sb);
        active.remove(j);
    }
    merges
}

impl Measure for T6 {
    fn id(&self) -> &'static str {
        "T6"
    }

    fn version(&self) -> u16 {
        1
    }

    fn purity(&self) -> Purity {
        Purity::Pure
    }

    fn needs(&self) -> Needs {
        Needs::TEXT
    }

    fn complete(&self, recipe: &mut Recipe) {
        recipe.min_n.get_or_insert(MIN_N);
        recipe.codec.get_or_insert_with(|| CODEC.to_string());
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        codec(recipe)?;
        let c = corpus(input)?;
        let want = recipe.min_n.unwrap_or(MIN_N).max(MIN_N);
        let have = c.docs.len() as u64;
        if have < want {
            return Err(Refusal::new(
                Code::W485,
                format!(
                    "T6 needs at least {want} texts and `{}` has {have}",
                    recipe.scope.trim()
                ),
            ));
        }
        // The two longest texts are the longest pair.
        let mut lens: Vec<(usize, &str)> = c
            .docs
            .iter()
            .map(|d| (joined_len(&d.segments), d.alias.as_str()))
            .collect();
        lens.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        let (x, y) = (lens[0], lens[1]);
        if x.0 + 1 + y.0 > WINDOW {
            return Err(Refusal::new(
                Code::E484,
                format!(
                    "`{}` and `{}` are {} bytes together, and the compressor's window is {WINDOW}: \
                     it would have lost the first by the time it coded the second",
                    x.1,
                    y.1,
                    x.0 + 1 + y.0
                ),
            ));
        }
        Ok(())
    }

    fn run(&self, _recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let c = match corpus(input) {
            Ok(c) => c,
            Err(r) => return Outcome::refused(r),
        };
        let texts: Vec<String> = c.docs.iter().map(|d| d.text()).collect();
        let n = texts.len();
        let mut out = Vec::new();
        let mut alone = Vec::with_capacity(n);
        for t in &texts {
            if compress_into(t.as_bytes(), &mut out).is_none() {
                return refused_codec();
            }
            alone.push(out.len() as u64);
        }
        let mut d = vec![0.0; n * n];
        let mut value = Value::new()
            .with_n("texts", n as u64)
            .with_n("pairs", (n * (n - 1) / 2) as u64)
            .with_n("bytes", texts.iter().map(|t| t.len() as u64).sum());
        let mut both = String::new();
        for i in 0..n {
            for j in i + 1..n {
                let mut c2 = [0u64; 2];
                for (k, (x, y)) in [(i, j), (j, i)].into_iter().enumerate() {
                    both.clear();
                    both.push_str(&texts[x]);
                    both.push('\n');
                    both.push_str(&texts[y]);
                    if compress_into(both.as_bytes(), &mut out).is_none() {
                        return refused_codec();
                    }
                    c2[k] = out.len() as u64;
                }
                let v = ncd(alone[i], alone[j], c2[0], c2[1]);
                let Some(q) = q20(v) else {
                    return refused_codec();
                };
                d[i * n + j] = v;
                d[j * n + i] = v;
                value = value.with_value(
                    format!("ncd:{}|{}", c.docs[i].alias, c.docs[j].alias),
                    Num::Fixed(q),
                );
            }
        }
        let mut linkage = Vec::with_capacity(4 * (n - 1));
        for m in average_linkage(&d, n) {
            let Some(h) = q20(m.height) else {
                return refused_codec();
            };
            linkage.extend([
                Num::Count(m.a as u64),
                Num::Count(m.b as u64),
                Num::Fixed(h),
                Num::Count(m.size as u64),
            ]);
        }
        Outcome::Value(value.with_series("linkage", linkage))
    }
}

fn joined_len(segments: &[String]) -> usize {
    segments.iter().map(String::len).sum::<usize>() + segments.len().saturating_sub(1)
}
