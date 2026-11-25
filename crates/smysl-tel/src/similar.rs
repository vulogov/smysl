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
//!
//! **`tel/minhash/2`** (RFC SMYSL-9 §3, SI-P1) keeps every `/1` feature and adds two families,
//! each chosen by a [`Recipe`] key; `/1` is unchanged and stays available.
//! - `templates=novel` (default `overlap`, `/1`'s reading): an overlapping template is a feature
//!   only against its **reference**, the templates of the same stream and instrument in the
//!   latest window that ends at or before its own begins. With no template of its family there
//!   (same token count; every position equal or `<*>` on either side; at least one equal word),
//!   it is `template:<text>`. With one, it is kept as `template-rise:<text>` only when its rate
//!   is more than `rise=` times the family's (default 4), and otherwise dropped. A stream with no
//!   earlier window has no reference, and its templates are all new.
//! - `kinds=on`: `kind:<metric>:<direction>` for each anomaly, the metric being the series' name
//!   without its identifying labels ([`metric_name`]), so two faults of one kind on two services
//!   share a feature.

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

/// Which templates are features.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum Templates {
    /// Every template whose window overlaps the episode's (`/1`).
    #[default]
    Overlap,
    /// Those absent from their reference, or rising past `rise` times its rate (`/2`).
    Novel,
}

/// A similarity recipe: `tel/minhash/1?k=128&seed=S`, or `tel/minhash/2?…` with its feature keys.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Recipe {
    /// 1 or 2.
    pub version: u8,
    pub seed: u64,
    pub templates: Templates,
    /// The factor a known template's rate must exceed (`templates=novel` only).
    pub rise: u64,
    pub kinds: bool,
}

/// `rise=`'s default.
pub const RISE: u64 = 4;

impl Recipe {
    /// `tel/minhash/1` with `seed`: the features [`episodes`] reads.
    pub fn v1(seed: u64) -> Recipe {
        Recipe {
            version: 1,
            seed,
            templates: Templates::Overlap,
            rise: RISE,
            kinds: false,
        }
    }

    /// `tel/minhash/2` with `seed`, its keys at their defaults.
    pub fn v2(seed: u64) -> Recipe {
        Recipe {
            version: 2,
            ..Recipe::v1(seed)
        }
    }

    pub fn with_templates(mut self, t: Templates) -> Recipe {
        self.templates = t;
        self
    }

    pub fn with_rise(mut self, rise: u64) -> Recipe {
        self.rise = rise;
        self
    }

    pub fn with_kinds(mut self, on: bool) -> Recipe {
        self.kinds = on;
        self
    }

    /// The recipe's text, every `/2` key written: what MinHash is keyed by and what is printed.
    pub fn text(&self) -> String {
        if self.version == 1 {
            return recipe(self.seed);
        }
        let templates = match self.templates {
            Templates::Overlap => "overlap".to_string(),
            Templates::Novel => format!("novel&rise={}", self.rise),
        };
        format!(
            "tel/minhash/2?k={K}&seed={}&templates={templates}&kinds={}",
            self.seed,
            if self.kinds { "on" } else { "off" }
        )
    }

    /// Read a recipe's text. Keys may come in any order and the defaults may be left out; an
    /// unknown key, a `/1` recipe with a `/2` key, `rise=` without `templates=novel`, or a `k`
    /// other than 128 is refused.
    pub fn parse(text: &str) -> Result<Recipe, String> {
        let (head, query) = text.split_once('?').unwrap_or((text, ""));
        let mut r = match head {
            "tel/minhash/1" => Recipe::v1(0),
            "tel/minhash/2" => Recipe::v2(0),
            other => return Err(format!("`{other}` is not tel/minhash/1 or tel/minhash/2")),
        };
        let mut rise = None;
        for kv in query.split('&').filter(|kv| !kv.is_empty()) {
            let (k, v) = kv
                .split_once('=')
                .ok_or_else(|| format!("`{kv}` is not KEY=VALUE"))?;
            let number = || {
                v.parse::<u64>()
                    .map_err(|_| format!("`{k}` is a non-negative integer, not `{v}`"))
            };
            match (k, r.version) {
                ("k", _) if number()? == K as u64 => {}
                ("k", _) => return Err(format!("`k` is {K}")),
                ("seed", _) => r.seed = number()?,
                ("templates", 2) => {
                    r.templates = match v {
                        "overlap" => Templates::Overlap,
                        "novel" => Templates::Novel,
                        _ => return Err(format!("`templates` is overlap or novel, not `{v}`")),
                    }
                }
                ("rise", 2) => match number()? {
                    0 => return Err("`rise` is at least 1".into()),
                    n => rise = Some(n),
                },
                ("kinds", 2) => {
                    r.kinds = match v {
                        "on" => true,
                        "off" => false,
                        _ => return Err(format!("`kinds` is on or off, not `{v}`")),
                    }
                }
                _ => return Err(format!("`{k}` is not a key of {head}")),
            }
        }
        if let Some(n) = rise {
            if r.templates != Templates::Novel {
                return Err("`rise` goes with `templates=novel`".into());
            }
            r.rise = n;
        }
        Ok(r)
    }
}

/// A series' metric name without its identifying labels, read from its key: Prometheus' name
/// before `{`, OTLP's `name` member, a Zabbix item key before its `[`. Other schemes have none.
pub fn metric_name(scheme: &str, key: &str) -> Option<String> {
    let name = match scheme {
        "prom" => key.split('{').next()?.to_string(),
        "otel" => crate::json::parse(key)
            .ok()?
            .get("name")?
            .as_str()?
            .to_string(),
        "zabbix" => key.split_once("/item:")?.1.split('[').next()?.to_string(),
        _ => return None,
    };
    (!name.is_empty()).then_some(name)
}

/// Two templates of one family: as many tokens, no position where both are words and differ,
/// and at least one where they are the same word.
fn family(a: &str, b: &str) -> bool {
    const WILD: &str = "<*>";
    let (x, y): (Vec<&str>, Vec<&str>) = (a.split(' ').collect(), b.split(' ').collect());
    x.len() == y.len()
        && x.iter()
            .zip(&y)
            .all(|(p, q)| p == q || *p == WILD || *q == WILD)
        && x.iter().zip(&y).any(|(p, q)| p == q && *p != WILD)
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
        MinHash::of(&Recipe::v1(seed))
    }

    /// The functions of `recipe`, keyed by its text: `/1` with seed `s` is [`MinHash::new`]`(s)`.
    pub fn of(recipe: &Recipe) -> MinHash {
        let seed = recipe.seed;
        let mut key = Vec::with_capacity(DOMAIN.len() + 96);
        key.extend_from_slice(DOMAIN);
        key.extend_from_slice(recipe.text().as_bytes());
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
/// whose window overlaps the episode's. The `/1` features; [`episodes_with`] reads a recipe's.
pub fn episodes<'a>(
    units: impl IntoIterator<Item = (&'a Uid, &'a UnitCore)>,
    relations: impl IntoIterator<Item = &'a smysl_core::Relation>,
) -> BTreeMap<Uid, BTreeSet<String>> {
    episodes_with(units, relations, &Recipe::v1(0))
}

/// [`episodes`] under `recipe`'s feature keys (§3.2 of RFC SMYSL-9); the seed plays no part.
pub fn episodes_with<'a>(
    units: impl IntoIterator<Item = (&'a Uid, &'a UnitCore)>,
    relations: impl IntoIterator<Item = &'a smysl_core::Relation>,
    recipe: &Recipe,
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
    let novel = (recipe.templates == Templates::Novel).then(|| Reference::of(&units));
    members
        .into_iter()
        .map(|(ep, ms)| {
            let (from, to) = window(units[&ep]).unwrap_or((0, 0));
            let mut f = match &novel {
                None => {
                    let ts = templates
                        .iter()
                        .filter(|(_, (f, t))| *f < to && from < *t)
                        .map(|(u, _)| *u);
                    features(ms.iter().copied(), ts)
                }
                Some(reference) => {
                    let mut f = features(ms.iter().copied(), []);
                    f.extend(reference.features(from, to, recipe.rise));
                    f
                }
            };
            if recipe.kinds {
                f.extend(kinds(&ms, &units));
            }
            (ep, f)
        })
        .collect()
}

/// `kind:<metric>:<direction>` for each anomaly among `members` whose series is in `units` and
/// has a [`metric_name`].
fn kinds(members: &[&UnitCore], units: &BTreeMap<&Uid, &UnitCore>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for u in members.iter().filter(|u| u.schema.as_str() == tel::ANOMALY) {
        let Some(p) = u.payload.as_deref() else {
            continue;
        };
        let Some(series) = field_text(p, "tel:series").and_then(|s| s.parse::<Uid>().ok()) else {
            continue;
        };
        let Some(sp) = units.get(&series).and_then(|s| s.payload.as_deref()) else {
            continue;
        };
        let (Some(scheme), Some(key)) = (field_text(sp, "tel:scheme"), field_text(sp, "tel:key"))
        else {
            continue;
        };
        if let Some(m) = metric_name(&scheme, &key) {
            let dir = field_text(p, "tel:direction").unwrap_or_else(|| "-".into());
            out.insert(format!("kind:{m}:{dir}"));
        }
    }
    out
}

/// One stream's windows, keyed `(to, from)`, each with its templates and their counts.
type Windows = BTreeMap<(u64, u64), Vec<(String, u64)>>;

/// Every template of a store by stream and instrument, then by window: what `templates=novel`
/// reads its references from.
struct Reference {
    by_stream: BTreeMap<(Uid, Uid), Windows>,
}

impl Reference {
    fn of(units: &BTreeMap<&Uid, &UnitCore>) -> Reference {
        let mut by_stream: BTreeMap<(Uid, Uid), Windows> = BTreeMap::new();
        for u in units.values() {
            if u.schema.as_str() != tel::LOG_TEMPLATE {
                continue;
            }
            let Some(t) = u.payload.as_deref().and_then(crate::logs::template_fields) else {
                continue;
            };
            by_stream
                .entry((t.stream, t.inst))
                .or_default()
                .entry((t.to, t.from))
                .or_default()
                .push((t.template, t.count));
        }
        Reference { by_stream }
    }

    /// The template features of an episode over `[from, to)`.
    fn features(&self, from: u64, to: u64, rise: u64) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for windows in self.by_stream.values() {
            for (&(wt, wf), ts) in windows.iter().filter(|((t, f), _)| *f < to && from < *t) {
                // The latest window that ends at or before this one begins.
                let earlier = windows.range(..=(wf, u64::MAX)).next_back();
                for (text, count) in ts {
                    let Some((&(rt, rf), rs)) = earlier else {
                        out.insert(format!("template:{text}"));
                        continue;
                    };
                    let known: u64 = rs
                        .iter()
                        .filter(|(r, _)| family(text, r))
                        .map(|(_, n)| n)
                        .sum();
                    if known == 0 {
                        out.insert(format!("template:{text}"));
                        continue;
                    }
                    // count / (wt − wf) > rise · known / (rt − rf), in integers.
                    let mine = u128::from(*count) * u128::from(rt.saturating_sub(rf).max(1));
                    let theirs = u128::from(rise)
                        * u128::from(known)
                        * u128::from(wt.saturating_sub(wf).max(1));
                    if mine > theirs {
                        out.insert(format!("template-rise:{text}"));
                    }
                }
            }
        }
        out
    }
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
    nearest_with(episodes, of, k, &Recipe::v1(seed))
}

/// [`nearest`] by `recipe`'s functions.
pub fn nearest_with(
    episodes: &BTreeMap<Uid, BTreeSet<String>>,
    of: &Uid,
    k: usize,
    recipe: &Recipe,
) -> Vec<Neighbour> {
    let Some(mine) = episodes.get(of) else {
        return Vec::new();
    };
    let mh = MinHash::of(recipe);
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

    #[test]
    fn v1_signatures_are_the_ones_1_20_gave() {
        // Taken from the code at 9468ca7, before `/2` existed.
        let f = set(&["series:a", "anomaly:a:up", "template:x <*> y", "change:/p"]);
        let s = MinHash::new(7).signature(&f);
        assert_eq!(
            s[..4],
            [
                900864475653830758,
                1282756573638141128,
                613286010804210688,
                1108502173639764625
            ]
        );
        let s = MinHash::of(&Recipe::v1(0)).signature(&f);
        assert_eq!(s[..2], [227320333297287106, 822244436049733447]);
        assert_eq!(s[126..], [144954831627255773, 125837352952783324]);
        // `/2` with its defaults reads `/1`'s features under other functions.
        assert_ne!(MinHash::of(&Recipe::v2(0)).signature(&f), s);
    }

    #[test]
    fn a_recipe_reads_back_from_its_text_and_refuses_what_it_does_not_know() {
        for r in [
            Recipe::v1(3),
            Recipe::v2(0),
            Recipe::v2(9)
                .with_templates(Templates::Novel)
                .with_rise(6)
                .with_kinds(true),
        ] {
            assert_eq!(Recipe::parse(&r.text()).unwrap(), r, "{}", r.text());
        }
        assert_eq!(Recipe::v1(3).text(), recipe(3));
        assert_eq!(
            Recipe::v2(1).with_templates(Templates::Novel).text(),
            "tel/minhash/2?k=128&seed=1&templates=novel&rise=4&kinds=off"
        );
        assert_eq!(
            Recipe::parse("tel/minhash/2?kinds=on&templates=novel").unwrap(),
            Recipe::v2(0)
                .with_templates(Templates::Novel)
                .with_kinds(true)
        );
        for bad in [
            "tel/minhash/3",
            "tel/minhash/1?kinds=on",
            "tel/minhash/2?k=64",
            "tel/minhash/2?rise=4",
            "tel/minhash/2?templates=novel&rise=0",
            "tel/minhash/2?templates=new",
            "tel/minhash/2?weights=on",
            "tel/minhash/2?seed",
        ] {
            assert!(Recipe::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_metric_name_is_the_series_name_without_its_labels() {
        assert_eq!(
            metric_name("prom", r#"cpu{service="carts"}"#).as_deref(),
            Some("cpu")
        );
        assert_eq!(metric_name("prom", "up").as_deref(), Some("up"));
        assert_eq!(
            metric_name("zabbix", "host:web1/item:system.cpu.util[,idle]").as_deref(),
            Some("system.cpu.util")
        );
        assert_eq!(
            metric_name("otel", r#"{"attributes":{},"name":"http.server.duration"}"#).as_deref(),
            Some("http.server.duration")
        );
        assert_eq!(metric_name("json", "x"), None);
        assert_eq!(metric_name("prom", "{a=\"b\"}"), None);
    }

    use crate::detect::{self, episode, Direction, Input};
    use smysl_core::{canonical_uid, schemas::tel as schema};

    const MIN: u64 = 60_000;

    /// An anomaly on prom series `key` in `[from, from + 1m)`, and the series unit.
    fn anomaly_on(key: &str, from: u64) -> (UnitCore, UnitCore) {
        let series = schema::series("prom", key).unwrap();
        let input = Input {
            series: canonical_uid(&series),
            scheme: "prom",
            key,
            instrument: Uid::from_bytes([9; 32]),
            samples: &[],
        };
        let a = detect::anomaly(
            &input,
            from,
            from + MIN,
            &detect::Recipe::new("mad/1"),
            5.0,
            Some(Direction::Up),
        )
        .unwrap();
        (a, series)
    }

    fn template(stream: u8, from: u64, to: u64, text: &str, count: u64) -> UnitCore {
        crate::logs::Template {
            stream: Uid::from_bytes([stream; 32]),
            scheme: "journald".into(),
            key: "{}".into(),
            inst: Uid::from_bytes([7; 32]),
            from_ms: from,
            to_ms: to,
            template: text.into(),
            count,
            digest: [0; 32],
        }
        .unit()
    }

    /// One episode over `members`, with `extra` units beside it: (units, relations).
    fn store(
        members: Vec<UnitCore>,
        extra: Vec<UnitCore>,
    ) -> (BTreeMap<Uid, UnitCore>, Vec<smysl_core::Relation>) {
        let p = episode::Params {
            gap_ms: 60 * MIN,
            min_members: 1,
        };
        let g = episode::run(&members, 0, 1_000 * MIN, &p).remove(0);
        let mut units: BTreeMap<Uid, UnitCore> = members
            .into_iter()
            .chain(extra)
            .map(|u| (canonical_uid(&u), u))
            .collect();
        units.insert(canonical_uid(&g.unit), g.unit);
        (units, g.relations)
    }

    fn only(
        (units, relations): &(BTreeMap<Uid, UnitCore>, Vec<smysl_core::Relation>),
        r: &Recipe,
    ) -> BTreeSet<String> {
        let mut eps = episodes_with(units.iter(), relations.iter(), r);
        assert_eq!(eps.len(), 1);
        eps.pop_first().unwrap().1
    }

    #[test]
    fn novel_templates_drop_drift_and_keep_bursts() {
        let (a, series) = anomaly_on("cpu", 20 * MIN);
        let extra = vec![
            series,
            // The reference window, [10m, 20m).
            template(1, 10 * MIN, 20 * MIN, "GET /cart user alice", 50),
            template(1, 10 * MIN, 20 * MIN, "pool <*> of <NUM> busy", 2),
            // An older window is not the reference.
            template(1, 0, 10 * MIN, "disk full", 1),
            // The episode's window, [20m, 30m).
            // Drift: one family as the reference's, generalised the other way.
            template(1, 20 * MIN, 30 * MIN, "GET /cart user <*>", 60),
            // A burst: 2 → 40 over windows of one length.
            template(1, 20 * MIN, 30 * MIN, "pool db of <NUM> busy", 40),
            // New.
            template(1, 20 * MIN, 30 * MIN, "OOM killed <*>", 1),
            template(1, 20 * MIN, 30 * MIN, "disk full", 1),
            // A stream with no earlier window: everything in it is new.
            template(2, 20 * MIN, 30 * MIN, "started", 1),
        ];
        let st = store(vec![a], extra);
        let overlap = only(&st, &Recipe::v2(0));
        assert_eq!(overlap, only(&st, &Recipe::v1(0)));
        assert!(overlap.contains("template:GET /cart user <*>"));
        let novel = only(&st, &Recipe::v2(0).with_templates(Templates::Novel));
        let tpl: Vec<&str> = novel
            .iter()
            .filter(|f| f.starts_with("template"))
            .map(String::as_str)
            .collect();
        assert_eq!(
            tpl,
            [
                "template-rise:pool db of <NUM> busy",
                "template:OOM killed <*>",
                "template:disk full",
                "template:started"
            ]
        );
        // At rise=20 the burst is not enough; the rest is the same.
        let r = Recipe::v2(0).with_templates(Templates::Novel).with_rise(20);
        assert!(!only(&st, &r)
            .iter()
            .any(|f| f.starts_with("template-rise:")));
        // The members' features do not depend on the template reading.
        let members = |f: &BTreeSet<String>| -> Vec<String> {
            f.iter()
                .filter(|x| !x.starts_with("template"))
                .cloned()
                .collect()
        };
        assert_eq!(members(&novel), members(&overlap));
    }

    #[test]
    fn kinds_are_shared_by_one_fault_on_two_services() {
        let ep = |svc: &str| {
            let (a, s) = anomaly_on(&format!("cpu{{service=\"{svc}\"}}"), 20 * MIN);
            store(vec![a], vec![s])
        };
        let (carts, orders) = (ep("carts"), ep("orders"));
        let r = Recipe::v2(0).with_kinds(true);
        let (c, o) = (only(&carts, &r), only(&orders, &r));
        assert!(c.contains("kind:cpu:up"), "{c:?}");
        assert_eq!(c.intersection(&o).collect::<Vec<_>>(), ["kind:cpu:up"]);
        // Without `kinds`, the two share nothing.
        let r = Recipe::v2(0);
        assert_eq!(jaccard(&only(&carts, &r), &only(&orders, &r)).0, 0);
        // An anomaly whose series unit is absent has no kind.
        let (a, _) = anomaly_on("cpu", 20 * MIN);
        let lone = store(vec![a], vec![]);
        assert!(!only(&lone, &Recipe::v2(0).with_kinds(true))
            .iter()
            .any(|f| f.starts_with("kind:")));
    }
}
