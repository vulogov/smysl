//! Tokenisation for heterogeneous payloads.
//!
//! `bm25`'s own tokeniser stems, strips stop words and can detect the language. All three
//! are wrong here, and the reason is what smysl carries: a unit's payload may be a stack
//! trace, a metric series, a diff, a research abstract or a user's question. A tokeniser
//! that stems turns `latencies` into `latenc` — helpful for prose, and destructive for
//! `connection_pool_size`, which is exactly the term someone searches for.
//!
//! So this one does the minimum that is defensible across all of them:
//!
//! - split on whitespace and punctuation;
//! - split `camelCase`, `snake_case` and `kebab-case` into parts, **keeping the whole token
//!   as well**, so `poolSize` matches a query for either `poolSize` or `size`;
//! - lowercase, which is the one normalisation that helps everywhere and costs nothing;
//! - no stemming, no stop words, no language detection.
//!
//! Dropping stop words would help precision on English prose and cost nothing to add later.
//! It is left out because it is the kind of decision that should follow a measurement rather
//! than precede one, and the measurement is per `KernelType` — a stop-word list tuned on
//! prose is actively wrong on a store of `Data` units.

/// Split `text` into search terms.
///
/// Deterministic and allocation-simple: the same input yields the same vector, in the same
/// order, on any platform. Rule D reaches this crate because retrieval is a pure function of
/// the store.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-') {
        if raw.is_empty() {
            continue;
        }
        let lower = raw.to_lowercase();
        // The whole token first, so an exact identifier match still scores.
        out.push(lower.clone());

        let parts = split_identifier(raw);
        if parts.len() > 1 {
            out.extend(parts);
        }
    }
    out
}

/// Break an identifier into its parts, lowercased.
///
/// Returns one element when there is nothing to split, so the caller can test `len() > 1`
/// rather than compare against the original.
fn split_identifier(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;

    for c in s.chars() {
        let boundary = match prev {
            // `pool_size`, `pool-size`
            _ if c == '_' || c == '-' => true,
            // `poolSize` — lower to upper
            Some(p) if p.is_lowercase() && c.is_uppercase() => true,
            // `HTTPServer` — the last capital of a run belongs to the next word
            Some(p) if p.is_uppercase() && c.is_uppercase() => false,
            // `p95` — a digit starts a new part only after a letter
            Some(p) if p.is_alphabetic() && c.is_numeric() => true,
            Some(p) if p.is_numeric() && c.is_alphabetic() => true,
            _ => false,
        };

        if boundary && !cur.is_empty() {
            parts.push(std::mem::take(&mut cur).to_lowercase());
        }
        if c != '_' && c != '-' {
            cur.push(c);
        }
        prev = Some(c);
    }
    if !cur.is_empty() {
        parts.push(cur.to_lowercase());
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identifier_yields_its_parts_and_itself() {
        let t = tokenize("connection_pool_size");
        assert!(t.contains(&"connection_pool_size".to_string()), "{t:?}");
        assert!(t.contains(&"pool".to_string()), "{t:?}");
        assert!(t.contains(&"size".to_string()), "{t:?}");
    }

    #[test]
    fn camel_case_splits_and_a_capital_run_stays_together() {
        let t = tokenize("poolSize HTTPServer");
        assert!(t.contains(&"pool".to_string()), "{t:?}");
        assert!(t.contains(&"size".to_string()), "{t:?}");
        // `HTTPServer` splits at the lower-case `e`, so `httpserver` survives whole and the
        // run is not shattered into single letters.
        assert!(t.contains(&"httpserver".to_string()), "{t:?}");
        assert!(
            !t.iter().any(|s| s.len() == 1),
            "split a capital run: {t:?}"
        );
    }

    #[test]
    fn a_metric_name_keeps_its_number() {
        let t = tokenize("latency_p95");
        assert!(t.contains(&"latency_p95".to_string()), "{t:?}");
        assert!(
            t.contains(&"p".to_string()) || t.contains(&"95".to_string()),
            "{t:?}"
        );
    }

    /// No stemming, deliberately. Pinned so that adding one later is a decision somebody
    /// makes on purpose, with a measurement, rather than something a dependency bump does.
    #[test]
    fn nothing_is_stemmed() {
        assert!(tokenize("latencies").contains(&"latencies".to_string()));
        assert!(tokenize("running").contains(&"running".to_string()));
    }

    #[test]
    fn tokenisation_is_deterministic() {
        let s = "The p95 latency of connection_pool_size tripled — see poolSize.";
        assert_eq!(tokenize(s), tokenize(s));
    }
}
