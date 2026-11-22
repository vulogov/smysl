//! `tent/1` (RFC SMYSL-5 §6.4): the entropy of a log stream's templates over a window.
//!
//! Its input is units, not samples: the `x.tel/log-template` units of one stream, instrument,
//! window and Drain recipe that share one `digest` (RFC SMYSL-3 §9.7). The counts are their
//! `tel:count` fields in ascending order of template text, and the value, `mm` and `k` are
//! §6.1's plug-in over them. So a template-entropy unit stays recomputable after the events
//! are evicted, as long as its templates are kept.
//!
//! When the store holds no templates of the window, [`run_on`] materialises them from the
//! substrate's events, as `tel templates` does, and hands them back with the outcomes so the
//! caller can commit them beside the statistic. Only the shipped Drain recipe can be run; a
//! recipe hash naming another is read from the store or not at all.

use std::collections::BTreeMap;

use smysl_core::diag::Code;
use smysl_core::schemas::tel;
use smysl_core::{canonical_uid, Uid, UnitCore};
use smysl_series::{Backend, Substrate};

use super::{entropy, store, Computed, Fields, Outcome, Read, Refusal, Spec, Stat, Stream};
use crate::drain;
use crate::logs::{self, Template};

/// The lowercase hex of `BLAKE3` over a Drain recipe's text: how `tent/1` names it (§4, R1-4).
pub fn recipe_hash(text: &str) -> String {
    smysl_core::hash_bytes(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The hash of the Drain recipe this build runs: `templates`' default.
pub fn shipped() -> String {
    recipe_hash(drain::RECIPE)
}

/// One window's templates that share a digest: the input of one `tent/1` unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Templates {
    pub instrument: Uid,
    pub digest: [u8; 32],
    /// Count by template text, ascending.
    pub counts: BTreeMap<String, u64>,
}

/// `tent/1` over one set of templates of `stream`'s window `[from_ms, to_ms)`.
pub fn compute(
    spec: &Spec,
    stream: &Stream<'_>,
    from_ms: u64,
    to_ms: u64,
    t: &Templates,
) -> Result<Computed, Refusal> {
    if !matches!(spec.stat, Stat::Tent { .. }) {
        return Err(Refusal::recipe("info::logs::compute runs tent/1"));
    }
    // Symbols are the templates' places in text order, so the counts are summed in it.
    let counts: BTreeMap<i64, u64> = t
        .counts
        .values()
        .enumerate()
        .map(|(i, c)| (i as i64, *c))
        .collect();
    let n: u64 = counts.values().sum();
    if n < spec.min_n {
        return Err(Refusal::not_run(format!(
            "{n} events in {} template(s), under min_n {}",
            counts.len(),
            spec.min_n
        )));
    }
    let p = entropy::plugin(&counts).ok_or_else(|| Refusal::not_run("no template"))?;
    let value = store(p.value, true, "the statistic")?;
    let mm = store(p.mm, false, "mm")?;
    let read = Read {
        from_ms,
        to_ms,
        samples: &[],
        digest: t.digest,
    };
    let mut f = Fields::header(spec, stream, &read, p.n, &value);
    f.uint("tel:k", p.k);
    f.dec("tel:mm", &mm);
    let (unit, relations) = f.unit(spec, stream, &read, None);
    Ok(Computed {
        unit,
        relations,
        n: p.n,
        k: p.k,
        value,
        mm: Some(mm),
        bias: None,
        k_with: None,
        lag: None,
        values: Vec::new(),
        null: None,
        missing: 0,
    })
}

/// What [`run_on`] gives: an outcome per stream and template set, and the templates it had to
/// materialise because the store held none, for the caller to commit with the statistic.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub outcomes: Vec<Outcome>,
    pub materialised: Vec<Template>,
}

/// A stream's scheme and key, from a template unit's source (`series:<scheme>:<key>`).
fn source_of(unit: &UnitCore) -> Option<(String, String)> {
    let r = unit
        .source
        .as_ref()?
        .reference
        .strip_prefix(tel::SERIES_REF)?;
    let (scheme, key) = r.split_once(':')?;
    Some((scheme.to_string(), key.to_string()))
}

/// Run `tent/1` over each of `streams` in `[from_ms, to_ms)`: from the template units in
/// `stored` of the recipe `templates` names, one outcome per instrument and digest; when there
/// are none, from templates materialised out of the substrate's events, one per instrument (or
/// for `instrument` alone). An error is the substrate's, or a stream neither knows.
pub fn run_on<'u, B: Backend>(
    spec: &Spec,
    substrate: &Substrate<B>,
    stored: impl IntoIterator<Item = &'u UnitCore>,
    streams: &[Uid],
    instrument: Option<Uid>,
    from_ms: u64,
    to_ms: u64,
) -> Result<Run, String> {
    let Stat::Tent { templates } = &spec.stat else {
        return Err(format!("{} is not tent/1", spec.id()));
    };
    // (stream, instrument, digest) → counts by template; and each stream's scheme and key.
    let mut sets: BTreeMap<(Uid, Uid, [u8; 32]), BTreeMap<String, u64>> = BTreeMap::new();
    let mut names: BTreeMap<Uid, (String, String)> = BTreeMap::new();
    let mut seen = std::collections::BTreeSet::new();
    for unit in stored {
        if unit.schema.as_str() != tel::LOG_TEMPLATE || !seen.insert(canonical_uid(unit)) {
            continue;
        }
        let Some(f) = unit.payload.as_deref().and_then(logs::template_fields) else {
            continue;
        };
        if !streams.contains(&f.stream)
            || (f.from, f.to) != (from_ms, to_ms)
            || instrument.is_some_and(|i| i != f.inst)
            || recipe_hash(&f.recipe) != *templates
        {
            continue;
        }
        if let Some(n) = source_of(unit) {
            names.entry(f.stream).or_insert(n);
        }
        *sets
            .entry((f.stream, f.inst, f.digest))
            .or_default()
            .entry(f.template)
            .or_insert(0) += f.count;
    }
    let mut run = Run {
        outcomes: Vec::new(),
        materialised: Vec::new(),
    };
    for s in streams {
        let row = substrate.stream(s).map_err(|e| e.to_string())?;
        let (scheme, key, class) = match (&row, names.get(s)) {
            (Some(r), _) => (r.scheme.clone(), r.key.clone(), r.kind.clone()),
            (None, Some((scheme, key))) => (scheme.clone(), key.clone(), "log".to_string()),
            (None, None) => {
                if let Some(r) = substrate.row(s).map_err(|e| e.to_string())? {
                    let stream = Stream {
                        series: *s,
                        scheme: &r.scheme,
                        key: &r.key,
                        class: &r.class,
                        instrument: Uid::from_bytes([0; 32]),
                    };
                    run.outcomes.push(Outcome {
                        series: *s,
                        key: r.key.clone(),
                        instrument: stream.instrument,
                        with: None,
                        result: Err(Refusal::new(
                            Code::E701,
                            format!(
                                "tent/1 reads a log stream's templates, and this is a `{}` \
                                 series",
                                r.class
                            ),
                        )),
                    });
                    continue;
                }
                return Err(format!(
                    "neither the store nor the substrate knows stream {}",
                    s.canonical()
                ));
            }
        };
        if !matches!(scheme.as_str(), logs::JOURNALD | logs::CLOUDEVENTS) {
            return Err(format!("a `{scheme}` stream has no message to template"));
        }
        let mut found: Vec<Templates> = sets
            .range((*s, Uid::from_bytes([0; 32]), [0; 32])..)
            .take_while(|((st, _, _), _)| st == s)
            .map(|((_, inst, digest), counts)| Templates {
                instrument: *inst,
                digest: *digest,
                counts: counts.clone(),
            })
            .collect();
        if found.is_empty() {
            if *templates != shipped() {
                run.outcomes.push(Outcome {
                    series: *s,
                    key: key.clone(),
                    instrument: instrument.unwrap_or(Uid::from_bytes([0; 32])),
                    with: None,
                    result: Err(Refusal::not_run(format!(
                        "the store holds no template of recipe {templates} for this window, \
                         and this build materialises only {}",
                        drain::RECIPE
                    ))),
                });
                continue;
            }
            let insts = match instrument {
                Some(i) => vec![i],
                None => substrate.event_instruments(s).map_err(|e| e.to_string())?,
            };
            for inst in insts {
                let ts = logs::materialise(substrate, s, &inst, from_ms, to_ms)
                    .map_err(|e| e.to_string())?;
                let digest = match ts.first() {
                    Some(t) => t.digest,
                    None => {
                        substrate
                            .events(s, &inst, from_ms, to_ms)
                            .map_err(|e| e.to_string())?
                            .digest
                    }
                };
                found.push(Templates {
                    instrument: inst,
                    digest,
                    counts: ts.iter().map(|t| (t.template.clone(), t.count)).collect(),
                });
                run.materialised.extend(ts);
            }
        }
        for t in found {
            let stream = Stream {
                series: *s,
                scheme: &scheme,
                key: &key,
                class: &class,
                instrument: t.instrument,
            };
            run.outcomes.push(Outcome {
                series: *s,
                key: key.clone(),
                instrument: t.instrument,
                with: None,
                result: compute(spec, &stream, from_ms, to_ms, &t),
            });
        }
    }
    Ok(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shipped_recipe_hash_is_lowercase_hex_and_holds_no_reserved_character() {
        let h = shipped();
        assert_eq!(h.len(), 64);
        assert!(h
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
        assert_ne!(h, recipe_hash("tel/drain/1?depth=5"));
    }

    #[test]
    fn templates_count_in_text_order() {
        let spec = Spec::parse("{ stat: tent/1, min_n: 4 }").unwrap();
        let stream = Stream {
            series: Uid::from_bytes([1; 32]),
            scheme: "journald",
            key: "h1/cart.service",
            class: "log",
            instrument: Uid::from_bytes([2; 32]),
        };
        let t = |pairs: &[(&str, u64)]| Templates {
            instrument: stream.instrument,
            digest: [3; 32],
            counts: pairs.iter().map(|(k, c)| (k.to_string(), *c)).collect(),
        };
        let c = compute(
            &spec,
            &stream,
            0,
            60_000,
            &t(&[("b <*>", 2), ("a", 1), ("c", 1)]),
        )
        .unwrap();
        assert_eq!((c.n, c.k, c.value.to_string()), (4, 3, "1.5".to_string()));
        let r = compute(&spec, &stream, 0, 60_000, &t(&[("a", 3)])).unwrap_err();
        assert_eq!(r.code, Code::W703);
        let one = compute(&spec, &stream, 0, 60_000, &t(&[("a", 9)])).unwrap();
        assert_eq!(one.value.to_string(), "0");
    }
}
