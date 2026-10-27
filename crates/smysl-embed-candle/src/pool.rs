//! Pooling: an encoder's per-token states to one vector (SMYSL-2.5 §3.14).
//!
//! Plain loops over `f32`, in token order and then component order, rather than tensor
//! operations. The summation order is then this file's and not a kernel's, so the pooled
//! vector of one set of token states is one vector, and the tests below can state exact
//! values for it.

/// How a sequence of token states becomes one vector. Part of the model hash: one set of
/// weights pooled two ways is two models as far as a cached vector is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Pooling {
    /// The mean of the states the attention mask admits — `multilingual-e5-*`, whose model
    /// card prescribes it, with the `query: ` and `passage: ` prefixes.
    Mean,
    /// The first token's state — `bge-m3`'s dense head.
    Cls,
}

impl Pooling {
    /// The id that enters the model hash. Stable: changing one re-embeds every cache.
    pub const fn id(self) -> &'static str {
        match self {
            Pooling::Mean => "mean",
            Pooling::Cls => "cls",
        }
    }

    /// Pool `states` (one row a token) under `mask` (nonzero admits the token), then
    /// L2-normalise.
    ///
    /// A sequence the mask admits nothing of, or whose pooled vector is zero, pools to the
    /// zero vector rather than to a division by zero; cosine against it is then zero, which
    /// is no hit.
    pub fn pool(self, states: &[Vec<f32>], mask: &[u32]) -> Vec<f32> {
        let dim = states.first().map_or(0, Vec::len);
        let mut v = vec![0.0f32; dim];
        match self {
            Pooling::Mean => {
                let mut n = 0u32;
                for (row, m) in states.iter().zip(mask) {
                    if *m == 0 {
                        continue;
                    }
                    n += 1;
                    for (acc, x) in v.iter_mut().zip(row) {
                        *acc += x;
                    }
                }
                if n > 0 {
                    let n = n as f32;
                    for x in &mut v {
                        *x /= n;
                    }
                }
            }
            Pooling::Cls => {
                if let Some(first) = states.first() {
                    v.copy_from_slice(first);
                }
            }
        }
        normalise(&mut v);
        v
    }
}

/// Scale `v` to unit length; a zero vector stays zero.
pub(crate) fn normalise(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v {
            *x /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three tokens, two dimensions, the third masked out — a padded position.
    fn states() -> Vec<Vec<f32>> {
        vec![vec![3.0, 0.0], vec![1.0, 4.0], vec![100.0, -100.0]]
    }

    // Reference vectors worked by hand once and stored here. The tolerance is one ulp-scale
    // epsilon per component: the inputs are small integers, the arithmetic is a sum, a
    // division and a square root, and the claim is agreement within a build (§5.7), not
    // bit-agreement across machines.
    const EPS: f32 = 1e-6;

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= EPS)
    }

    #[test]
    fn mean_pools_what_the_mask_admits_and_normalises() {
        // mean of (3,0) and (1,4) is (2,2); unit length is (1/√2, 1/√2).
        let v = Pooling::Mean.pool(&states(), &[1, 1, 0]);
        let h = std::f32::consts::FRAC_1_SQRT_2;
        assert!(close(&v, &[h, h]), "{v:?}");
    }

    #[test]
    fn mean_without_a_mask_entry_would_take_the_padding() {
        // The same states with the third admitted: (104, -96) / 3, normalised. The mask is
        // what keeps a padded position out, and this is the vector it keeps out.
        let v = Pooling::Mean.pool(&states(), &[1, 1, 1]);
        let n = (104.0f32 * 104.0 + 96.0 * 96.0).sqrt();
        assert!(close(&v, &[104.0 / n, -96.0 / n]), "{v:?}");
    }

    #[test]
    fn cls_takes_the_first_token_whatever_the_mask_says() {
        let v = Pooling::Cls.pool(&states(), &[0, 1, 1]);
        assert!(close(&v, &[1.0, 0.0]), "{v:?}");
    }

    #[test]
    fn nothing_admitted_pools_to_zero() {
        assert_eq!(Pooling::Mean.pool(&states(), &[0, 0, 0]), vec![0.0, 0.0]);
        assert_eq!(Pooling::Mean.pool(&[], &[]), Vec::<f32>::new());
        assert_eq!(Pooling::Cls.pool(&[vec![0.0, 0.0]], &[1]), vec![0.0, 0.0]);
    }

    #[test]
    fn the_ids_are_the_ones_the_hash_takes() {
        assert_eq!(Pooling::Mean.id(), "mean");
        assert_eq!(Pooling::Cls.id(), "cls");
    }
}
