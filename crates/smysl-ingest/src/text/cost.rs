//! The cost of an extraction, measured from the journal (§3.7, §6 TX-P5 step 5).
//!
//! Draft 1 of this plan sized a phase from a planning figure: "≈ 1.6k in + ≈ 1.2k out, ×1.3 for
//! repairs" per chapter, a number only ever given for English. S0 measured 1.42x that for hosted
//! English and **3.96x for hosted Russian**, and found that the gap was not tokenization — the
//! estimator is 6% over against a model that never enters the repair loop — but the repair loop
//! itself, which the byte-based bound F-2 replaced was provoking. So the plan stopped carrying a
//! constant. §3.7's cost model is
//!
//! ```text
//! cost(window) = base(tokens) * (1 + r)
//! ```
//!
//! with `r` the **measured retry rate per language**, and this module is where it is measured.
//! Every figure here comes out of [`Journal`] entries that a real run wrote; nothing in it is a
//! coefficient somebody chose.
//!
//! # Why per language, and why that is not in the journal key
//!
//! A window extracted in Russian is not *different work* from one extracted in English — the key
//! is what makes a window resumable, and language is not part of what was done. But it is what
//! the retry rate varies with, so the language is in the **entry**, beside the tokens, and this
//! module groups by it. A report for one language is the row; the corpus total is the sum.
//!
//! # Two routes to a number, and they must not be multiplied
//!
//! [`Row::tokens_per_window`] is measured over calls that **already include** the repair turns,
//! so a projection from it is a projection with repairs in it. `(1 + r)` prices a projection made
//! the other way, from the estimator's `base` over text nobody has extracted yet. Applying both
//! would charge the repairs twice, which is how a planning figure becomes an overrun in the other
//! direction.
//!
//! # An entry that cannot be priced
//!
//! TX-P5 step 4's journal recorded the **run's running total** of tokens against each window, not
//! the window's own. Summing those across a corpus charges the first window once per window. Such
//! an entry carries no call count, which is what [`Entry::priced`] reads: it is counted in
//! [`Row::unpriced`] and left out of every total, because a figure made of two measurements added
//! together is a figure about neither.

use std::collections::BTreeMap;

use super::journal::{Entry, Journal, Verdicts};

/// The language a journal entry carries when its manifest recorded none.
pub const UNKNOWN_LANG: &str = "und";

/// What one language cost.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Row {
    pub lang: String,
    /// Entries in this group, priced and unpriced together.
    pub windows: usize,
    /// Of those, entries whose figures were left out: see the module's last paragraph.
    pub unpriced: usize,
    /// Calls, repair turns included.
    pub calls: u64,
    /// Of those calls, the repair turns.
    pub retries: u64,
    /// Of those calls, the pivot pass's: one per window that had anything to translate, under
    /// `--lang-policy pivot:<lang>` and zero otherwise. Out of [`Row::attempts`] for the reason
    /// stated there.
    pub pivots: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub verdicts: Verdicts,
}

impl Row {
    fn of(lang: &str) -> Row {
        Row {
            lang: lang.to_string(),
            ..Row::default()
        }
    }

    fn add(&mut self, e: &Entry) {
        self.windows += 1;
        if !e.priced() {
            self.unpriced += 1;
            return;
        }
        self.calls += u64::from(e.calls);
        self.retries += u64::from(e.retries);
        self.pivots += u64::from(e.pivots);
        self.input_tokens += e.input_tokens;
        self.output_tokens += e.output_tokens;
        self.verdicts.present += e.verdicts.present;
        self.verdicts.loose += e.verdicts.loose;
        self.verdicts.absent += e.verdicts.absent;
    }

    /// Windows whose figures are in the sums.
    pub fn priced(&self) -> usize {
        self.windows - self.unpriced
    }

    /// First attempts: one per pass, repair turns and the pivot pass excluded.
    ///
    /// The pivot call is a first attempt of a *different request* — translate these gists, not
    /// extract from this window — and it has no repair loop, so including it would divide the
    /// repair turns of one request by the attempts of two. The same corpus would then report a
    /// lower `r` for having been pivoted, which is a figure about the policy and not about the
    /// model. Its tokens stay in `input_tokens` and `output_tokens`, because they are what the
    /// window cost.
    pub fn attempts(&self) -> u64 {
        self.calls
            .saturating_sub(self.retries)
            .saturating_sub(self.pivots)
    }

    /// §3.7's `r`: repair turns per first attempt, or `None` where nothing was attempted.
    ///
    /// `None` rather than zero, which is the same choice [`crate::Agreement::rate`] makes: a
    /// corpus nobody has extracted has no retry rate, and reporting 0.0 for it would read as the
    /// best possible measurement of the thing that was never measured.
    pub fn retry_rate(&self) -> Option<f64> {
        let attempts = self.attempts();
        (attempts > 0).then(|| self.retries as f64 / attempts as f64)
    }

    /// Mean input and output tokens per priced window, repairs included.
    pub fn tokens_per_window(&self) -> Option<(f64, f64)> {
        let n = self.priced();
        (n > 0).then(|| {
            (
                self.input_tokens as f64 / n as f64,
                self.output_tokens as f64 / n as f64,
            )
        })
    }

    /// What `windows` more windows of this language would cost, from the measurement.
    ///
    /// Repairs are in it already — see the module doc. `None` where nothing was measured.
    pub fn project(&self, windows: u64) -> Option<(f64, f64)> {
        self.tokens_per_window()
            .map(|(i, o)| (i * windows as f64, o * windows as f64))
    }

    /// The share of extracted quotes the text supported, or `None` where no quote was checked.
    ///
    /// Reported beside the cost because it is what the cost bought: a corpus of windows whose
    /// quotes were mostly `Absent` cost the same as one whose quotes held up.
    pub fn support_rate(&self) -> Option<f64> {
        let total = self.verdicts.total();
        (total > 0).then(|| f64::from(self.verdicts.present) / f64::from(total))
    }
}

/// The cost of what a journal records, by language.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct CostReport {
    /// One row per language, in language order.
    pub by_lang: BTreeMap<String, Row>,
    /// Every row summed. Its `lang` is `*`, because a total is not a language.
    pub total: Row,
}

impl CostReport {
    /// The row for one language, if the journal has one.
    pub fn lang(&self, lang: &str) -> Option<&Row> {
        self.by_lang.get(lang)
    }
}

/// Read the cost of a journal, optionally for one expression only.
///
/// `alias` filters by the expression a window belongs to, which is what `ingest --text <alias>
/// --cost` reports; `None` is the whole journal, which is what sizing a corpus asks for.
pub fn cost(journal: &Journal, alias: Option<&str>) -> CostReport {
    let mut out = CostReport {
        by_lang: BTreeMap::new(),
        total: Row::of("*"),
    };
    for e in journal.entries() {
        if alias.is_some_and(|a| e.key.alias != a) {
            continue;
        }
        let lang = if e.lang.is_empty() {
            UNKNOWN_LANG
        } else {
            e.lang.as_str()
        };
        out.by_lang
            .entry(lang.to_string())
            .or_insert_with(|| Row::of(lang))
            .add(e);
        out.total.add(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::journal::Key;
    use super::*;
    use smysl_core::Tid;

    fn entry(alias: &str, at: u64, lang: &str, calls: u32, retries: u32) -> Entry {
        Entry {
            key: Key::new(alias, Tid::of(b"a part"), (at, at + 100), [1; 32]),
            batch: [7; 32],
            digest: [9; 32],
            input_tokens: 1000,
            output_tokens: 200,
            lang: lang.to_string(),
            calls,
            retries,
            pivots: 0,
            verdicts: Verdicts {
                present: 2,
                loose: 1,
                absent: 1,
            },
        }
    }

    fn journal(entries: Vec<Entry>) -> Journal {
        let mut j = Journal::in_memory();
        for e in entries {
            j.record(e).expect("in memory");
        }
        j
    }

    #[test]
    fn the_retry_rate_is_repairs_over_first_attempts_and_not_over_calls() {
        // Two passes and two repair turns: four calls, two of them attempts, so `r` is 1.0 —
        // every first attempt bought a second call. Over *calls* it would read 0.5, and a cost
        // model that multiplied `base` by 1.5 would under-price the window by a third.
        let j = journal(vec![entry("kjv", 0, "en", 4, 2)]);
        let r = cost(&j, None);
        let en = r.lang("en").expect("a row");
        assert_eq!((en.calls, en.retries, en.attempts()), (4, 2, 2));
        assert_eq!(en.retry_rate(), Some(1.0));
    }

    #[test]
    fn the_rows_are_per_language_and_the_total_is_their_sum() {
        let j = journal(vec![
            entry("kjv", 0, "en", 2, 0),
            entry("kjv", 100, "en", 3, 1),
            entry("synodal", 0, "ru", 6, 4),
        ]);
        let r = cost(&j, None);
        assert_eq!(r.by_lang.len(), 2);
        assert_eq!(r.lang("en").expect("en").windows, 2);
        assert_eq!(r.lang("en").expect("en").retry_rate(), Some(0.25));
        assert_eq!(r.lang("ru").expect("ru").retry_rate(), Some(2.0));
        assert_eq!(r.total.windows, 3);
        assert_eq!(r.total.input_tokens, 3000);
        // And the figure the phase exit is about: the two languages are *not* averaged into one
        // rate. S0 measured ru at 3.38x en, so a corpus sized on the total would under-price
        // every Russian window in it.
        assert_ne!(
            r.lang("en").expect("en").retry_rate(),
            r.lang("ru").expect("ru").retry_rate(),
        );
    }

    #[test]
    fn an_alias_filters_and_an_absent_one_reports_nothing_rather_than_everything() {
        let j = journal(vec![
            entry("kjv", 0, "en", 2, 0),
            entry("synodal", 0, "ru", 2, 0),
        ]);
        assert_eq!(cost(&j, Some("kjv")).total.windows, 1);
        assert_eq!(cost(&j, Some("vulgate")).total.windows, 0);
        assert_eq!(cost(&j, Some("vulgate")).total.retry_rate(), None);
    }

    #[test]
    fn an_unpriced_entry_is_counted_and_left_out_of_the_arithmetic() {
        // The step 4 shape: tokens are the run's running total and there is no call count.
        let mut old = entry("kjv", 0, "en", 0, 0);
        old.input_tokens = 99_000;
        let j = journal(vec![old, entry("kjv", 100, "en", 2, 0)]);
        let en = cost(&j, None).lang("en").expect("a row").clone();
        assert_eq!((en.windows, en.unpriced, en.priced()), (2, 1, 1));
        assert_eq!(en.input_tokens, 1000, "a cumulative figure was summed in");
        assert_eq!(en.tokens_per_window(), Some((1000.0, 200.0)));
    }

    #[test]
    fn a_manifest_with_no_language_is_a_row_and_not_a_blank_one() {
        let j = journal(vec![entry("notes", 0, "", 2, 0)]);
        let r = cost(&j, None);
        assert!(r.lang(UNKNOWN_LANG).is_some(), "{:?}", r.by_lang.keys());
    }

    #[test]
    fn a_projection_carries_the_repairs_it_measured() {
        // One window, two calls, one of them a repair: 1000 input tokens *with* the repair in
        // them. A thousand windows is a thousand times that and not `(1 + r)` times more.
        let j = journal(vec![entry("kjv", 0, "en", 2, 1)]);
        let en = cost(&j, None).lang("en").expect("a row").clone();
        assert_eq!(en.project(1000), Some((1_000_000.0, 200_000.0)));
        assert_eq!(en.retry_rate(), Some(1.0));
        assert_eq!(en.support_rate(), Some(0.5));
    }
}
