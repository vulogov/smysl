//! T3, match-length entropy rate (SMYSL-2.6 §3.7; Kontoyiannis, Algoet, Suhov, Wyner 1998).
//!
//! `Λᵢ` is 1 + the length of the longest prefix of `x[i..]` that also starts at some `j < i`,
//! overlap allowed. Two estimators over `i = 1 … n−1` are stored:
//!
//! - `H`, the increasing-window form: `(n−1) / Σ Λᵢ / log₂(i+1)`;
//! - `H:mr`, the mean-ratio form: `(1/(n−1)) Σ log₂(i+1) / Λᵢ` (Gao, Kontoyiannis,
//!   Bienenstock 2008), less biased on i.i.d. input and still low (§2.9).
//!
//! # Λ in linear time
//!
//! `Λᵢ − 1` is the longest previous factor, `LPF[i]` (Crochemore, Ilie 2008), computed from a
//! suffix array and its LCP. The suffix array is SA-IS (Nong, Zhang, Chan 2009), hand-rolled
//! over `u32` symbols so that the same code reads characters and term ids; the LCP is Kasai's
//! (Kasai et al. 2001). LPF is one pass with a stack over the suffix array: a suffix's longest
//! previous factor is shared with the nearest suffix on either side of it in suffix order that
//! starts earlier in the text, and the stack holds exactly those.
//!
//! # Symbols
//!
//! `params.symbols` is `chars` (the default) or `terms`. Characters are Unicode scalar values
//! after NFC, a text's segments joined by `\n` as T2 reads them; terms are the recipe's
//! analyzer stream, a text's segments one after another. A symbol's id is its rank among the
//! text's distinct symbols, which keeps SA-IS's buckets as small as the alphabet; Λ depends
//! only on which symbols are equal, so the ranking cannot change a value.
//!
//! # Per text, aggregated harmonically
//!
//! Matches never cross from one text into another (§3.1): each expression in scope is its own
//! `x`, and the scope's values are `H = Σ nᵢ / Σ (nᵢ / Hᵢ)` — the rate whose code length over
//! the scope is the sum of the texts' — and, for the mean-ratio form, which is a mean of
//! per-symbol rates, the mean over every symbol, `H:mr = Σ nᵢ H:mrᵢ / Σ nᵢ`. A scope of more
//! than one text stores each text's two values beside them, `H:<alias>` and `H:mr:<alias>`.
//!
//! `min:n` is in symbols and holds for **every** text (default 50,000): the estimator's bias
//! is a function of a text's own n, so a short text in a long scope is a biased term in the
//! sum whatever the sum's n. The refusal names the shortest.
//!
//! Memory is the text, the suffix array, the LCP and the LPF, four `u32` per symbol, and a
//! stack that can reach a fifth.

use std::collections::BTreeSet;

use smysl_core::diag::Code;

use crate::num::{self, q20};
use crate::outcome::{Num, Outcome, Refusal, Value};
use crate::recipe::{Param, Recipe};
use crate::text::entropy::corpus;
use crate::text::Corpus;
use crate::{Input, Measure, Needs, Purity};

/// T3's `min:n`, in symbols, for each text.
pub const MIN_N: u64 = 50_000;

/// The parameter that chooses the symbols.
pub const SYMBOLS: &str = "symbols";

/// `params.symbols`: Unicode scalar values after NFC.
pub const CHARS: &str = "chars";

/// `params.symbols`: the analyzer stream's terms.
pub const TERMS: &str = "terms";

/// A suffix array slot not yet filled.
const EMPTY: u32 = u32::MAX;

/// The suffix array of `s`, whose symbols are below `k`: the suffixes' start positions in
/// lexicographic order, a suffix before every longer one it is a prefix of.
///
/// # Panics
///
/// If a symbol is `k` or more, or `s` has `u32::MAX` symbols or more.
pub fn suffix_array(s: &[u32], k: u32) -> Vec<u32> {
    assert!(u32::try_from(s.len()).is_ok_and(|n| n < EMPTY));
    assert!(s.iter().all(|&c| c < k), "a symbol at or past the alphabet");
    match s.len() {
        0 => Vec::new(),
        _ => sa_is(s, k - 1),
    }
}

/// SA-IS over symbols in `0..=upper`. Below ten symbols a comparison sort is faster and is the
/// definition itself.
fn sa_is(s: &[u32], upper: u32) -> Vec<u32> {
    let n = s.len();
    if n < 10 {
        let mut sa: Vec<u32> = (0..n as u32).collect();
        sa.sort_by(|&a, &b| s[a as usize..].cmp(&s[b as usize..]));
        return sa;
    }
    let upper = upper as usize;
    // `ls[i]`: suffix i is S-type, smaller than suffix i + 1. The last is L-type.
    let mut ls = vec![false; n];
    for i in (0..n - 1).rev() {
        ls[i] = if s[i] == s[i + 1] {
            ls[i + 1]
        } else {
            s[i] < s[i + 1]
        };
    }
    // Bucket c holds its L-type suffixes from `sum_l[c]` and its S-type ones from `sum_s[c]`.
    let mut sum_l = vec![0u32; upper + 1];
    let mut sum_s = vec![0u32; upper + 1];
    for i in 0..n {
        if ls[i] {
            // An S-type symbol is never the largest, so `+ 1` stays in the table.
            sum_l[s[i] as usize + 1] += 1;
        } else {
            sum_s[s[i] as usize] += 1;
        }
    }
    for c in 0..=upper {
        sum_s[c] += sum_l[c];
        if c < upper {
            sum_l[c + 1] += sum_s[c];
        }
    }

    let mut lms_map = vec![EMPTY; n + 1];
    let mut lms: Vec<u32> = Vec::new();
    for i in 1..n {
        if !ls[i - 1] && ls[i] {
            lms_map[i] = lms.len() as u32;
            lms.push(i as u32);
        }
    }
    let m = lms.len();
    let mut sa = vec![EMPTY; n];
    induce(s, &ls, &sum_s, &sum_l, &lms, &mut sa);
    if m == 0 {
        return sa;
    }

    // Name the LMS substrings in sorted order, equal substrings one name, and sort them by
    // the suffix array of their names.
    let sorted: Vec<u32> = sa
        .iter()
        .copied()
        .filter(|&v| lms_map[v as usize] != EMPTY)
        .collect();
    let mut rec = vec![0u32; m];
    let mut names = 0u32;
    rec[lms_map[sorted[0] as usize] as usize] = 0;
    for i in 1..m {
        let (mut l, mut r) = (sorted[i - 1] as usize, sorted[i] as usize);
        let end = |p: usize| {
            let k = lms_map[p] as usize + 1;
            if k < m {
                lms[k] as usize
            } else {
                n
            }
        };
        let (end_l, end_r) = (end(l), end(r));
        let mut same = end_l - l == end_r - r;
        if same {
            while l < end_l && s[l] == s[r] {
                l += 1;
                r += 1;
            }
            // A substring that runs to the end of the text is unlike any that does not.
            if l == n || r == n || s[l] != s[r] {
                same = false;
            }
        }
        if !same {
            names += 1;
        }
        rec[lms_map[sorted[i] as usize] as usize] = names;
    }
    // What this level no longer needs is freed before the recursion, which is half its size.
    drop((sorted, lms_map, sa));
    let rec_sa = sa_is(&rec, names);
    drop(rec);
    let sorted: Vec<u32> = rec_sa.iter().map(|&i| lms[i as usize]).collect();
    drop((rec_sa, lms));
    let mut sa = vec![EMPTY; n];
    induce(s, &ls, &sum_s, &sum_l, &sorted, &mut sa);
    sa
}

/// Induced sorting: the LMS suffixes in the order given, then the L-type suffixes left to
/// right, then the S-type ones right to left.
fn induce(s: &[u32], ls: &[bool], sum_s: &[u32], sum_l: &[u32], lms: &[u32], sa: &mut [u32]) {
    let n = s.len();
    sa.fill(EMPTY);
    let mut buf = sum_s.to_vec();
    for &d in lms {
        let c = s[d as usize] as usize;
        sa[buf[c] as usize] = d;
        buf[c] += 1;
    }
    buf.copy_from_slice(sum_l);
    let c = s[n - 1] as usize;
    sa[buf[c] as usize] = (n - 1) as u32;
    buf[c] += 1;
    for i in 0..n {
        let v = sa[i];
        if v != EMPTY && v >= 1 && !ls[v as usize - 1] {
            let c = s[v as usize - 1] as usize;
            sa[buf[c] as usize] = v - 1;
            buf[c] += 1;
        }
    }
    buf.copy_from_slice(sum_l);
    for i in (0..n).rev() {
        let v = sa[i];
        if v != EMPTY && v >= 1 && ls[v as usize - 1] {
            let c = s[v as usize - 1] as usize + 1;
            buf[c] -= 1;
            sa[buf[c] as usize] = v - 1;
        }
    }
}

/// Kasai's LCP: `lcp[i]` is the longest common prefix of suffixes `sa[i − 1]` and `sa[i]`,
/// and `lcp[0]` is 0.
pub fn lcp(s: &[u32], sa: &[u32]) -> Vec<u32> {
    let n = s.len();
    let mut rank = vec![0u32; n];
    for (i, &p) in sa.iter().enumerate() {
        rank[p as usize] = i as u32;
    }
    let mut lcp = vec![0u32; n];
    let mut h = 0usize;
    for i in 0..n {
        let r = rank[i] as usize;
        if r == 0 {
            h = 0;
            continue;
        }
        let j = sa[r - 1] as usize;
        while i + h < n && j + h < n && s[i + h] == s[j + h] {
            h += 1;
        }
        lcp[r] = h as u32;
        h = h.saturating_sub(1);
    }
    lcp
}

/// The longest previous factor of every position of `s` (symbols below `k`): `lpf[i]` is the
/// length of the longest prefix of `s[i..]` that also starts before `i`, overlap allowed.
pub fn lpf(s: &[u32], k: u32) -> Vec<u32> {
    let sa = suffix_array(s, k);
    let mut lcp = lcp(s, &sa);
    lpf_from(&sa, &mut lcp)
}

/// LPF from a suffix array and its LCP (Crochemore, Ilie 2008). The stack holds suffix-array
/// slots whose positions increase from bottom to top; a slot below another is the nearest
/// before it that starts earlier, and `lcp[t]`, once `t` is on the stack, is rewritten to hold
/// the common prefix of `t` with the slot below it. A slot is popped by the first slot after
/// it that starts earlier, and its LPF is the longer of its prefixes in common with those two.
/// `lcp` is spent.
pub fn lpf_from(sa: &[u32], lcp: &mut [u32]) -> Vec<u32> {
    let n = sa.len();
    let mut out = vec![0u32; n];
    let mut stack: Vec<u32> = Vec::new();
    for i in 0..=n {
        // `h` is the common prefix of slot i with the slot on top of the stack.
        let mut h = if i < n { lcp[i] } else { 0 };
        while let Some(&t) = stack.last() {
            if i < n && sa[t as usize] < sa[i] {
                break;
            }
            stack.pop();
            let lt = lcp[t as usize];
            out[sa[t as usize] as usize] = lt.max(h);
            h = h.min(lt);
        }
        if i < n {
            lcp[i] = if stack.is_empty() { 0 } else { h };
            stack.push(i as u32);
        }
    }
    out
}

/// Both estimators over one text's LPF; `None` below two symbols.
pub fn estimate(lpf: &[u32]) -> Option<(f64, f64)> {
    let n = lpf.len();
    if n < 2 {
        return None;
    }
    let lg = |i: usize| num::log2((i + 1) as f64);
    let window = num::sum((1..n).map(|i| f64::from(lpf[i] + 1) / lg(i)));
    let ratio = num::sum((1..n).map(|i| lg(i) / f64::from(lpf[i] + 1)));
    let m = (n - 1) as f64;
    Some((m / window, ratio / m))
}

/// `xs` as symbol ranks, in place, and the alphabet's size. The alphabet is collected apart,
/// so ranking costs the alphabet and not a second copy of the text.
pub fn rank_in_place(xs: &mut [u32]) -> u32 {
    let alphabet: Vec<u32> = xs
        .iter()
        .copied()
        .collect::<BTreeSet<u32>>()
        .into_iter()
        .collect();
    for x in xs.iter_mut() {
        *x = alphabet.binary_search(x).expect("in the alphabet") as u32;
    }
    alphabet.len() as u32
}

/// `xs` as symbol ranks, and the alphabet's size.
pub fn ranks(xs: &[u32]) -> (Vec<u32>, u32) {
    let mut s = xs.to_vec();
    let k = rank_in_place(&mut s);
    (s, k)
}

/// T3.
#[derive(Debug, Clone, Copy, Default)]
pub struct T3;

/// Which symbols a recipe reads; `None` for a value this build does not know.
fn symbols(recipe: &Recipe) -> Option<&'static str> {
    match recipe.params.get(SYMBOLS) {
        Some(Param::Text(t)) if t == CHARS => Some(CHARS),
        Some(Param::Text(t)) if t == TERMS => Some(TERMS),
        _ => None,
    }
}

/// How many texts are in scope, by the symbols read.
fn count(c: &Corpus, which: &str) -> usize {
    if which == CHARS {
        c.docs.len()
    } else {
        c.starts.len()
    }
}

/// Text `k` in scope as its alias and its symbol count, without building its symbols.
fn len_of<'c>(c: &'c Corpus, which: &str, k: usize) -> (u64, &'c str) {
    if which == CHARS {
        let d = &c.docs[k];
        let chars: usize = d.segments.iter().map(|s| s.chars().count()).sum();
        (
            (chars + d.segments.len().saturating_sub(1)) as u64,
            d.alias.as_str(),
        )
    } else {
        let (from, to) = c.text_range(k);
        (
            c.segments[from..to].iter().map(|s| s.len() as u64).sum(),
            c.texts.get(k).map_or("", String::as_str),
        )
    }
}

/// Text `k` in scope as its symbols: a text's segments joined by `\n` for characters, one
/// after another for terms. One text at a time, so a scope costs its longest text.
fn symbols_of(c: &Corpus, which: &str, k: usize) -> Vec<u32> {
    if which == CHARS {
        let mut out = Vec::with_capacity(len_of(c, which, k).0 as usize);
        for (i, seg) in c.docs[k].segments.iter().enumerate() {
            if i > 0 {
                out.push(u32::from('\n'));
            }
            out.extend(seg.chars().map(u32::from));
        }
        out
    } else {
        let (from, to) = c.text_range(k);
        c.segments[from..to].concat()
    }
}

impl Measure for T3 {
    fn id(&self) -> &'static str {
        "T3"
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
        recipe
            .params
            .entry(SYMBOLS.to_string())
            .or_insert(Param::Text(CHARS.to_string()));
    }

    fn admit(&self, recipe: &Recipe, input: &Input<'_>) -> Result<(), Refusal> {
        let Some(which) = symbols(recipe) else {
            return Err(Refusal::new(
                Code::E489,
                format!("T3's `{SYMBOLS}` is `{CHARS}` or `{TERMS}`"),
            ));
        };
        let c = corpus(input)?;
        let want = recipe.min_n.unwrap_or(MIN_N).max(2);
        let lens: Vec<(u64, &str)> = (0..count(c, which)).map(|k| len_of(c, which, k)).collect();
        let Some((have, alias)) = lens.iter().min() else {
            return Err(Refusal::new(
                Code::W485,
                format!(
                    "T3 needs a text of at least {want} {which} and `{}` has none",
                    recipe.scope.trim()
                ),
            ));
        };
        if *have < want {
            let name = if alias.is_empty() {
                format!("`{}`", recipe.scope.trim())
            } else {
                format!("`{alias}`")
            };
            return Err(Refusal::new(
                Code::W485,
                format!("T3 needs at least {want} {which} in every text and {name} has {have}"),
            ));
        }
        if let Some((n, a)) = lens.iter().max() {
            if *n >= u64::from(EMPTY) {
                return Err(Refusal::new(
                    Code::E489,
                    format!("`{a}` has {n} symbols, past what a 32-bit suffix array indexes"),
                ));
            }
        }
        Ok(())
    }

    fn run(&self, recipe: &Recipe, input: &Input<'_>) -> Outcome {
        let c = match corpus(input) {
            Ok(c) => c,
            Err(r) => return Outcome::refused(r),
        };
        let Some(which) = symbols(recipe) else {
            return not_finite();
        };
        let mut per = Vec::new();
        for t in 0..count(c, which) {
            let mut s = symbols_of(c, which, t);
            let k = rank_in_place(&mut s);
            let n = s.len() as u64;
            let sa = suffix_array(&s, k);
            let mut lcp = lcp(&s, &sa);
            // The text is not needed once its LCP is: LPF reads positions, not symbols.
            drop(s);
            let lpf = lpf_from(&sa, &mut lcp);
            drop((sa, lcp));
            let Some((h, mr)) = estimate(&lpf) else {
                return not_finite();
            };
            per.push((len_of(c, which, t).1, n, h, mr));
        }
        let total: u64 = per.iter().map(|p| p.1).sum();
        let h = total as f64 / num::sum(per.iter().map(|p| p.1 as f64 / p.2));
        let mr = num::sum(per.iter().map(|p| p.1 as f64 * p.3)) / total as f64;
        let (Some(hq), Some(mrq)) = (q20(h), q20(mr)) else {
            return not_finite();
        };
        let mut value = Value::new()
            .with_n("symbols", total)
            .with_n("texts", per.len() as u64)
            .with_value("H", Num::Fixed(hq))
            .with_value("H:mr", Num::Fixed(mrq))
            .with_headline_in(
                "H",
                "match-length entropy rate",
                if which == CHARS {
                    "bits/char"
                } else {
                    "bits/token"
                },
            );
        if per.len() > 1 {
            for (alias, n, h, mr) in &per {
                let (Some(hq), Some(mrq)) = (q20(*h), q20(*mr)) else {
                    return not_finite();
                };
                value = value
                    .with_n(format!("symbols:{alias}"), *n)
                    .with_value(format!("H:{alias}"), Num::Fixed(hq))
                    .with_value(format!("H:mr:{alias}"), Num::Fixed(mrq));
            }
        }
        Outcome::Value(value)
    }
}

fn not_finite() -> Outcome {
    Outcome::NotRun {
        why: "the match-length rate was not a finite number".into(),
        code: None,
    }
}
