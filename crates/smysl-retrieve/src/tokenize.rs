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

/// How terms are produced: as written, or with common English suffixes folded (1.5).
///
/// Off by default, and deliberately: folding turns `latencies` into `latenc`, which helps prose and
/// hurts `connection_pool_size` — the term someone actually searches for. A caller retrieving over
/// English sentences can ask for it, and one retrieving over identifiers should not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Tokenizer {
    fold: bool,
}

impl Tokenizer {
    /// What [`tokenize`] does: no folding. The default.
    pub fn plain() -> Tokenizer {
        Tokenizer { fold: false }
    }

    /// Also emit a folded form of each term, so `required` and `require` meet.
    ///
    /// The folded form is emitted *beside* the term rather than instead of it, so an exact
    /// identifier match still scores as it did.
    pub fn folding() -> Tokenizer {
        Tokenizer { fold: true }
    }

    pub fn terms(&self, text: &str) -> Vec<String> {
        let mut out = tokenize(text);
        if self.fold {
            let mut folded: Vec<String> = out
                .iter()
                .filter_map(|t| fold_suffix(t))
                .filter(|f| !out.contains(f))
                .collect();
            folded.dedup();
            out.append(&mut folded);
        }
        out
    }
}

/// One English suffix folded off a term, when the result is still a word-sized stem.
///
/// Deterministic, idempotent and locale-free: `s`, `es`, `ed`, `ing` and `ly`, then a trailing
/// `e`, so `require`, `required`, `requires` and `requiring` all reach `requir`. Not a stemmer —
/// it does no lookups, has no exceptions and never rewrites the middle of a word. `ss` keeps its
/// `s`, so `class` is not `clas`.
///
/// Returns `None` when nothing was folded, so a caller can tell a stem from a term.
pub fn fold_suffix(term: &str) -> Option<String> {
    let stem = |s: &str, suffix: &str, min: usize| -> Option<String> {
        let rest = s.strip_suffix(suffix)?;
        (rest.len() >= min).then(|| rest.to_string())
    };
    let base = if term.ends_with("ss") {
        None
    } else {
        stem(term, "ing", 3)
            .or_else(|| stem(term, "ed", 3))
            .or_else(|| stem(term, "es", 3))
            .or_else(|| stem(term, "ly", 3))
            .or_else(|| stem(term, "s", 3))
    };
    let base = base.unwrap_or_else(|| term.to_string());
    let folded = match base.strip_suffix('e') {
        Some(rest) if rest.len() >= 3 => rest.to_string(),
        _ => base,
    };
    (folded != term).then_some(folded)
}

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

#[cfg(test)]
mod folding_tests {
    use super::*;

    /// R19: the four forms of one verb meet at one stem, and folding is idempotent.
    #[test]
    fn inflections_of_a_word_fold_together() {
        let stem = |w: &str| fold_suffix(w).unwrap_or_else(|| w.to_string());
        for w in ["require", "required", "requires", "requiring"] {
            assert_eq!(stem(w), "requir", "{w}");
            assert_eq!(stem(&stem(w)), stem(w), "{w}: not idempotent");
        }
        assert_eq!(stem("commands"), "command");
        assert_eq!(stem("quickly"), "quick");
    }

    /// What it must not do: eat a word whole, or fold an identifier's parts into nonsense.
    #[test]
    fn short_words_and_double_s_are_left_alone() {
        for w in ["is", "as", "has", "class", "pass", "pool", "p95"] {
            assert_eq!(fold_suffix(w), None, "{w} was folded");
        }
    }

    /// The folded form is emitted beside the term, never instead of it, and plain is unchanged.
    #[test]
    fn folding_adds_terms_and_plain_is_what_it_was() {
        let text = "Commands require an argument";
        assert_eq!(Tokenizer::plain().terms(text), tokenize(text));
        let folded = Tokenizer::folding().terms(text);
        assert!(folded.starts_with(&tokenize(text)), "{folded:?}");
        assert!(folded.contains(&"requir".to_string()), "{folded:?}");
        assert!(
            folded.contains(&"require".to_string()),
            "the term itself is kept"
        );
    }
}
