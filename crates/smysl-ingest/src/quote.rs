//! Attributing a unit to the text it came from, and **checking the attribution**.
//!
//! A `source` names a document. It cannot name a passage, so a reviewer asking "which
//! sentence produced this claim?" has nowhere to go — and verifying a unit against its
//! source is most of what review actually is.
//!
//! A model can be asked to quote the text it drew each unit from. On its own that is just
//! another assertion, and an assertion from the thing under review is worth little. What
//! makes it worth something is that **a quote is checkable against text we already hold**:
//! if it does not occur in the source, it was invented, and the tool can say so rather than
//! the reader having to notice. That is the same move `measured` makes by requiring a
//! machine-checkable source.
//!
//! Three outcomes, because two would force a bad choice:
//!
//! - **Present.** The quote occurs in the source once whitespace and punctuation are
//!   normalised. Nothing to say.
//! - **Loose** (`SMY-W308`). Every word of the quote appears in the source, in order, but
//!   not contiguously — an elision, or a reworded join. Worth flagging and not worth
//!   refusing: models elide, and refusing would spend a repair turn on a habit.
//! - **Absent** (`SMY-E307`). The words are not there in that order. Nothing in the source
//!   supports the attribution, which is a fabrication rather than a formatting difference.

use smysl_core::{canonical_uid, Code, Diagnostic, UnitCore};

/// The payload key a quote travels under.
///
/// Payload rather than a field of `SourceRef`, which is where provenance belongs: adding a
/// key to the wire format is a change this can be promoted to once the shape has been used
/// in anger. Rule X carries it verbatim meanwhile, and `ingest:unrepaired` set the
/// precedent for ingest metadata living here.
pub const QUOTE_KEY: &str = "ingest:quote";

/// How well a quote is supported by the text it claims to come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Support {
    Present,
    Loose,
    Absent,
}

/// Collapse the differences a model will introduce without meaning anything by them.
///
/// Case, runs of whitespace, and the punctuation a model silently prettifies: curly quotes
/// for straight ones, en and em dashes for hyphens. Deliberately **not** stemming or
/// synonyms — those would make a reworded claim look attributed, which is exactly the thing
/// this exists to catch.
fn normalise(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        let c = match c {
            '\u{2018}' | '\u{2019}' | '\u{201B}' => '\'',
            '\u{201C}' | '\u{201D}' | '\u{201F}' => '"',
            '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
            '\u{00A0}' => ' ',
            other => other,
        };
        if c.is_whitespace() {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.extend(c.to_lowercase());
    }
    out
}

/// Whether a token is an elision marker rather than a word to find.
///
/// `...` and `…` are how a quotation says *something was left out here*. Matching them
/// against the source would fail every honestly elided quote, because the one thing an
/// ellipsis certainly is not is a word in the document.
fn is_elision(token: &str) -> bool {
    !token.is_empty()
        && token
            .chars()
            .all(|c| matches!(c, '.' | '\u{2026}' | '[' | ']'))
}

/// Whether `needle`'s words appear in `haystack` in order, allowing gaps.
///
/// The elision case: a model quoting "the pool saturated … which the canary contradicts"
/// has attributed honestly and dropped a clause. A subsequence check catches that while
/// still refusing a quote whose words are simply not there.
fn words_in_order(needle: &str, haystack: &str) -> bool {
    let words: Vec<&str> = needle
        .split_whitespace()
        .filter(|w| !is_elision(w))
        .collect();
    // A quote of nothing but ellipses attributes nothing. Without this the `all` below is
    // vacuously true and every such quote reads as loosely supported.
    if words.is_empty() {
        return false;
    }
    let mut hay = haystack.split_whitespace();
    words
        .into_iter()
        .all(|w| hay.any(|h| h.contains(w) || w.contains(h)))
}

/// How well the source supports a quote.
pub fn support(quote: &str, source: &str) -> Support {
    let q = normalise(quote);
    if q.is_empty() {
        return Support::Absent;
    }
    let s = normalise(source);
    if s.contains(&q) {
        return Support::Present;
    }
    if words_in_order(&q, &s) {
        return Support::Loose;
    }
    Support::Absent
}

/// The quote a unit attributes itself to, if it declared one.
pub fn quote_of(core: &UnitCore) -> Option<String> {
    let payload = core.payload.as_deref()?;
    let object = smysl_core::surface::payload::payload_to_object(payload).ok()?;
    object
        .get(QUOTE_KEY)
        .and_then(|v| v.value.as_str())
        .map(str::to_string)
}

/// Check every attributed quote in a batch against the text it was drawn from.
///
/// Units that attribute nothing are not faulted here: asking a model for a quote is a
/// request, and a missing one is a thinner record rather than a false one. What is faulted
/// is a quote the source does not support.
pub fn verify(units: &[UnitCore], source: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for u in units {
        let Some(q) = quote_of(u) else { continue };
        let uid = canonical_uid(u);
        match support(&q, source) {
            Support::Present => {}
            Support::Loose => out.push(
                Diagnostic::on(Code::W308, uid)
                    .with_message(format!(
                        "attributed quote is elided or reworded: {:?}",
                        clip(&q)
                    ))
                    .with_suggestion("quote the source verbatim, or drop the attribution"),
            ),
            Support::Absent => out.push(
                Diagnostic::on(Code::E307, uid)
                    .with_message(format!(
                        "attributed quote is not in the source: {:?}",
                        clip(&q)
                    ))
                    .with_suggestion("quote text that appears in the document, or omit `quote`"),
            ),
        }
    }
    out
}

/// A quote is untrusted input and a diagnostic reaches logs.
fn clip(s: &str) -> String {
    const LIMIT: usize = 80;
    if s.chars().count() <= LIMIT {
        return s.to_string();
    }
    let head: String = s.chars().take(LIMIT).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use smysl_core::{KernelType, Status, UnitCoreBuilder};

    const SOURCE: &str = "On Thursday the eu-west shard slowed: p95 request latency rose \
                          from 180ms to 410ms. Connection pool wait time rose alongside it.";

    #[test]
    fn a_verbatim_quote_is_present() {
        assert_eq!(
            support("p95 request latency rose", SOURCE),
            Support::Present
        );
    }

    /// The differences a model introduces without meaning anything by them.
    #[test]
    fn case_whitespace_and_smart_punctuation_do_not_matter() {
        for q in [
            "P95 REQUEST LATENCY ROSE",
            "p95   request\n  latency rose",
            "the eu\u{2011}west shard slowed",
        ] {
            assert_eq!(support(q, SOURCE), Support::Present, "{q:?}");
        }
    }

    /// An elision is honest attribution with a clause dropped. Worth flagging, not worth
    /// spending a repair turn on.
    #[test]
    fn an_elided_quote_is_loose_rather_than_absent() {
        let q = "the eu-west shard slowed ... 180ms to 410ms";
        assert_eq!(support(q, SOURCE), Support::Loose);
    }

    /// **The case this exists for.** Text the source does not contain, in an attribution
    /// that would otherwise read as authoritative.
    #[test]
    fn an_invented_quote_is_absent() {
        for q in [
            "the database was misconfigured",
            "latency fell from 410ms to 180ms",
        ] {
            assert_eq!(support(q, SOURCE), Support::Absent, "{q:?}");
        }
    }

    /// Word order carries the meaning, so a reversal is not an attribution.
    #[test]
    fn reordered_words_are_not_an_attribution() {
        assert_eq!(
            support("latency rose p95 before Thursday on", SOURCE),
            Support::Absent
        );
    }

    /// An ellipsis is the marker for "something was left out", so matching it against the
    /// source would fail every honestly elided quote.
    #[test]
    fn an_ellipsis_is_a_marker_not_a_word_to_find() {
        assert!(is_elision("..."));
        assert!(is_elision("\u{2026}"));
        assert!(is_elision("[...]"));
        assert!(!is_elision("p95"));
    }

    /// A quote of nothing but elision markers attributes nothing.
    #[test]
    fn a_quote_of_only_ellipses_is_absent() {
        assert_eq!(support("... ...", SOURCE), Support::Absent);
    }

    #[test]
    fn an_empty_quote_supports_nothing() {
        assert_eq!(support("   ", SOURCE), Support::Absent);
    }

    fn quoted(gist: &str, quote: Option<&str>) -> UnitCore {
        let mut b = UnitCoreBuilder::new(KernelType::Claim, gist, Status::Speculative);
        if let Some(q) = quote {
            let mut o = smysl_core::surface::hjson::HObject::default();
            let span = smysl_core::Span::new(0, 0);
            o.insert(
                smysl_core::surface::hjson::Spanned::new(QUOTE_KEY.to_string(), span),
                smysl_core::surface::hjson::Spanned::new(
                    smysl_core::surface::hjson::HValue::Str(q.to_string()),
                    span,
                ),
            );
            if let Some(p) = smysl_core::surface::payload::object_to_payload(&o) {
                b = b.payload(p);
            }
        }
        b.build().unwrap()
    }

    #[test]
    fn a_quote_round_trips_through_the_payload() {
        let u = quoted("a claim", Some("p95 request latency rose"));
        assert_eq!(quote_of(&u).as_deref(), Some("p95 request latency rose"));
    }

    #[test]
    fn a_unit_with_no_quote_is_not_faulted() {
        let units = vec![quoted("a claim", None)];
        assert!(verify(&units, SOURCE).is_empty());
    }

    #[test]
    fn verification_reports_the_invented_and_passes_the_real() {
        let units = vec![
            quoted("honest", Some("p95 request latency rose")),
            quoted("invented", Some("the database was misconfigured")),
        ];
        let out = verify(&units, SOURCE);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].code, Code::E307);
    }

    /// Untrusted input reaches logs.
    #[test]
    fn a_huge_quote_is_clipped_in_the_diagnostic() {
        let units = vec![quoted("x", Some(&"lorem ".repeat(200)))];
        let out = verify(&units, SOURCE);
        assert_eq!(out.len(), 1);
        assert!(out[0].to_string().len() < 300, "{}", out[0]);
    }
}
