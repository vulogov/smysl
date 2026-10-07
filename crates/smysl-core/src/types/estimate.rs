//! Token estimators (SMYSL-2.1 F-2).
//!
//! There are two jobs here and they are not the same job.
//!
//! **Cost** is what a provider will charge for some text. That number wants to be *faithful* to
//! a real tokenizer, and [`crate::tokens`] is the bundled approximation of one: packing must be
//! pure (rule D), provider tokenisers are neither available offline nor stable, so every pack
//! records the estimator that produced its budget.
//!
//! **A granularity bound** is how much one unit may say. `l0_max` is not a prediction about a
//! provider's bill; it is an editorial limit on a gist. A count faithful to a tokenizer makes
//! that limit mean different amounts of content in different scripts, because scripts cost
//! different numbers of bytes and tokens per character. The S0 spike measured the consequence:
//! under `utf8-div4` one proposition gets 120 characters in English and 67 in Russian, and
//! 12.67% of Russian units were destroyed by the difference rather than shortened.
//!
//! So OQ-31 — faithful or content-fair? — has two answers, one per job. Cost stays faithful
//! ([`crate::tokens`], `smysl-pack`). A bound may be *content-fair*: [`TokenEstimator::Content1`]
//! counts a proposition the same whatever language states it. The default is still
//! [`TokenEstimator::Utf8Div4`], so a store written before this existed checks exactly as it did.

use core::fmt;

/// A character class, for a script-weighted count.
///
/// Fixed code-point ranges, not a Unicode-property crate: the weights are frozen against these
/// exact ranges, so the ranges are part of the frozen artefact and cannot drift under us.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Class {
    Latin,
    Cyrillic,
    Greek,
    Cjk,
    Digit,
    Space,
    Other,
}

impl Class {
    /// `smysl/content/1`'s frozen weights, in integer milli-tokens per character.
    ///
    /// Calibrated once by `scripts/calibrate_estimator.py` over seven public-domain
    /// verse-per-line editions, and pinned by `fixtures/estimator/content-1.json`. Changing any
    /// weight is a new id, `smysl/content/2`; `/1` never changes.
    ///
    /// `Digit` is not fitted. The calibration corpus holds 7 digit characters in 99,012 verses,
    /// so no weight for it is identifiable, and freezing an unidentified one would score every
    /// gist containing a number against noise. A digit is one character of content exactly as a
    /// letter is, so it takes the latin weight by construction.
    const fn content1_milli(self) -> u32 {
        match self {
            Class::Latin => 188,
            Class::Cyrillic => 260,
            Class::Greek => 229,
            Class::Cjk => 931,
            Class::Digit => 188,
            Class::Space => 456,
            Class::Other => 423,
        }
    }

    const fn of(c: char) -> Class {
        // Range patterns, not `RangeInclusive::contains`, which is not const-stable.
        match c as u32 {
            0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF => Class::Latin,
            0x400..=0x52F | 0x1C80..=0x1C8F | 0x2DE0..=0x2DFF | 0xA640..=0xA69F => Class::Cyrillic,
            0x370..=0x3FF | 0x1F00..=0x1FFF => Class::Greek,
            // CJK, plus the Japanese and Korean syllabaries.
            0x3040..=0x30FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0xAC00..=0xD7AF
            | 0x20000..=0x2FA1F => Class::Cjk,
            0x30..=0x39 => Class::Digit,
            // `char::is_whitespace` is not const, and the weights were calibrated against
            // Python's `str.isspace`. These are the code points both agree on.
            0x09..=0x0D
            | 0x20
            | 0x85
            | 0xA0
            | 0x1680
            | 0x2000..=0x200A
            | 0x2028
            | 0x2029
            | 0x202F
            | 0x205F
            | 0x3000 => Class::Space,
            _ => Class::Other,
        }
    }
}

/// How a granularity bound counts text.
///
/// Selected per profile by [`crate::GranularityProfile`], and per invocation by
/// `check --estimator`. The id travels on the wire (SMYSL-2.3 A-9, granularity key 5) so that a
/// bound is never silently compared against a count it was not written for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum TokenEstimator {
    /// `ceil(utf8_len(t) / 4)` — [`crate::tokens`], the bundled faithful approximation, and
    /// what every profile written before F-2 means.
    #[default]
    Utf8Div4,
    /// `smysl/content/1`: script-weighted so that parallel translations of one proposition get
    /// one count. Answers OQ-31 for a bound rather than for a cost.
    Content1,
}

impl TokenEstimator {
    pub const ALL: &'static [TokenEstimator] =
        &[TokenEstimator::Utf8Div4, TokenEstimator::Content1];

    /// The identifier written to granularity key 5 and accepted by `--estimator`.
    pub const fn id(self) -> &'static str {
        match self {
            TokenEstimator::Utf8Div4 => "smysl/utf8-div4",
            TokenEstimator::Content1 => "smysl/content/1",
        }
    }

    /// Parse a bundled estimator by id.
    ///
    /// `None` is not "use the default": a profile naming an estimator this build cannot parse
    /// has an **unevaluable** `l0_max`, because evaluating it under a different count would
    /// silently answer a question nobody asked. See [`crate::ProfileEstimator::Unknown`].
    pub fn parse(s: &str) -> Option<TokenEstimator> {
        TokenEstimator::ALL.iter().copied().find(|e| e.id() == s)
    }

    /// Count `t`, in whole tokens.
    ///
    /// Integer arithmetic throughout (rule D, no floats): milli-tokens are summed and the total
    /// is divided with a ceiling, so the count is the same on every target.
    pub fn count(self, t: &str) -> u32 {
        match self {
            TokenEstimator::Utf8Div4 => crate::tokens(t),
            TokenEstimator::Content1 => {
                let milli: u64 = t
                    .chars()
                    .map(|c| u64::from(Class::of(c).content1_milli()))
                    .sum();
                u32::try_from(milli.div_ceil(1000)).unwrap_or(u32::MAX)
            }
        }
    }

    /// The longest prefix of `t` that counts at or under `max`, as a byte length.
    ///
    /// Callers that have to *fit* text to a bound cannot divide the bound by four: that is the
    /// inverse of one estimator only, and under a script-weighted count it is wrong in both
    /// directions. Returns a byte index on a character boundary, so slicing at it is safe.
    pub fn fit(self, t: &str, max: u32) -> usize {
        if self.count(t) <= max {
            return t.len();
        }
        match self {
            // ceil(len/4) <= max  <=>  len <= 4 * max, and the count is per byte.
            TokenEstimator::Utf8Div4 => {
                let want = 4usize * max as usize;
                let mut end = want.min(t.len());
                while end > 0 && !t.is_char_boundary(end) {
                    end -= 1;
                }
                end
            }
            TokenEstimator::Content1 => {
                let budget = u64::from(max) * 1000;
                let mut milli = 0u64;
                let mut end = 0usize;
                for (i, c) in t.char_indices() {
                    milli += u64::from(Class::of(c).content1_milli());
                    if milli > budget {
                        return i;
                    }
                    end = i + c.len_utf8();
                }
                end
            }
        }
    }

    /// The longest prefix of `t` such that the prefix **plus `suffix`** counts at or under
    /// `max`, as a byte length on a character boundary.
    ///
    /// For a caller that shortens text and marks the cut with an ellipsis: the mark itself
    /// costs, and what it costs depends on the estimator, so it cannot be reserved in bytes.
    pub fn fit_with_suffix(self, t: &str, suffix: &str, max: u32) -> usize {
        let mut end = self.fit(t, max);
        let mut buf = String::new();
        loop {
            buf.clear();
            buf.push_str(&t[..end]);
            buf.push_str(suffix);
            if end == 0 || self.count(&buf) <= max {
                return end;
            }
            end -= 1;
            while end > 0 && !t.is_char_boundary(end) {
                end -= 1;
            }
        }
    }
}

impl fmt::Display for TokenEstimator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.id())
    }
}

/// What a granularity profile says about counting.
///
/// Three states, not two. SMYSL-2.1 §10 asked for `Option<TokenEstimator>` so that "absent" is
/// distinguishable from "the default", and for an unknown id to make `l0_max` unevaluable. Both
/// are needed at once, and an unknown id must also survive a round trip (§8.1), so the id itself
/// has to be kept. Two `Option` fields with an invariant between them is this enum.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ProfileEstimator {
    /// Granularity key 5 absent. Counts with [`TokenEstimator::Utf8Div4`], which is what every
    /// profile written before F-2 means, and encodes to the bytes it always did.
    #[default]
    Unset,
    /// Key 5 names an estimator this build has.
    Known(TokenEstimator),
    /// Key 5 names an estimator this build does not have. `l0_max` and `l1_range` are
    /// **unevaluable**: `SMY-W025`, not `SMY-E022` under a count that was never meant.
    Unknown(String),
}

impl ProfileEstimator {
    /// The estimator to count with, or `None` when the profile names one this build lacks.
    pub fn estimator(&self) -> Option<TokenEstimator> {
        match self {
            ProfileEstimator::Unset => Some(TokenEstimator::Utf8Div4),
            ProfileEstimator::Known(e) => Some(*e),
            ProfileEstimator::Unknown(_) => None,
        }
    }

    /// The id as it should appear on the wire, or `None` when key 5 is not written.
    ///
    /// `Utf8Div4` is deliberately not written: it is the meaning of an absent key, so writing it
    /// would change the bytes of every existing view (A-9).
    pub fn wire_id(&self) -> Option<&str> {
        match self {
            ProfileEstimator::Unset => None,
            ProfileEstimator::Known(TokenEstimator::Utf8Div4) => None,
            ProfileEstimator::Known(e) => Some(e.id()),
            ProfileEstimator::Unknown(s) => Some(s.as_str()),
        }
    }

    /// Read an id off the wire, keeping it whether or not this build knows it.
    pub fn from_wire(s: &str) -> ProfileEstimator {
        match TokenEstimator::parse(s) {
            Some(e) => ProfileEstimator::Known(e),
            None => ProfileEstimator::Unknown(s.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/estimator");

    /// Pull `"name": number` out of the fixture's weight object, without a JSON dependency.
    fn fixture_weights() -> Vec<(String, u32)> {
        let text = std::fs::read_to_string(format!("{FIXTURES}/content-1.json"))
            .expect("fixtures/estimator/content-1.json");
        let key = "\"weights_milli_tokens_per_char\"";
        let start = text.find(key).expect("weight object") + key.len();
        let open = start + text[start..].find('{').expect("weight object opens");
        let close = open + text[open..].find('}').expect("weight object closes");
        text[open + 1..close]
            .split(',')
            .map(|pair| {
                let (k, v) = pair.split_once(':').expect("name: value");
                (
                    k.trim().trim_matches('"').to_string(),
                    v.trim().parse::<u32>().expect("integer weight"),
                )
            })
            .collect()
    }

    /// The weights are a frozen artefact, not a constant someone may tune.
    ///
    /// `smysl/content/1` names *these* numbers; changing one is `smysl/content/2`. The
    /// calibration lives in `scripts/calibrate_estimator.py` and its output in the fixture, so
    /// the source and the artefact cannot drift apart without this failing.
    #[test]
    fn content1_weights_are_pinned_to_the_fixture() {
        let want = fixture_weights();
        assert_eq!(want.len(), 7, "every class is weighted: {want:?}");
        for (name, weight) in want {
            let class = match name.as_str() {
                "latin" => Class::Latin,
                "cyrillic" => Class::Cyrillic,
                "greek" => Class::Greek,
                "cjk" => Class::Cjk,
                "digit" => Class::Digit,
                "space" => Class::Space,
                "other" => Class::Other,
                other => panic!("the fixture names a class this build does not have: {other}"),
            };
            assert_eq!(
                class.content1_milli(),
                weight,
                "{name} disagrees with the fixture"
            );
        }
    }

    #[test]
    fn the_default_estimator_is_todays_count() {
        for t in ["", "a", "hello world", "Да будет свет", "起初神创造天地"] {
            assert_eq!(TokenEstimator::Utf8Div4.count(t), crate::tokens(t), "{t}");
        }
        assert_eq!(TokenEstimator::default(), TokenEstimator::Utf8Div4);
        assert_eq!(
            ProfileEstimator::default().estimator(),
            Some(TokenEstimator::Utf8Div4)
        );
    }

    /// Integer arithmetic only, and the two properties F-2 asks a proptest for.
    #[test]
    fn counts_are_monotone_and_nonzero() {
        let samples = [
            "a",
            "a longer piece of Latin text",
            "кириллица",
            "Ελληνικά",
            "漢字",
            "  ",
            "1234",
            "—«»…",
        ];
        for e in TokenEstimator::ALL.iter().copied() {
            for a in samples {
                assert!(e.count(a) >= 1, "{e} counted {a:?} as zero");
                for b in samples {
                    let joined = format!("{a}{b}");
                    assert!(
                        e.count(&joined) >= e.count(a).max(e.count(b)),
                        "{e} is not monotone over {a:?} + {b:?}"
                    );
                }
            }
        }
        assert_eq!(TokenEstimator::Content1.count(""), 0);
    }

    /// What the bound is worth, per script, under each estimator.
    ///
    /// The point of `content/1`: one proposition gets a comparable amount of room whatever
    /// script states it. Under `utf8-div4` a Cyrillic character costs twice a Latin one and a
    /// CJK character three times, purely because of UTF-8.
    #[test]
    fn content1_narrows_the_gap_between_scripts() {
        let l0 = 30;
        let per_script = |e: TokenEstimator, c: char| {
            let s: String = std::iter::repeat_n(c, 400).collect();
            e.fit(&s, l0) / c.len_utf8()
        };
        let div4 = |c| per_script(TokenEstimator::Utf8Div4, c);
        let content = |c| per_script(TokenEstimator::Content1, c);

        // utf8-div4: 120 Latin characters, 60 Cyrillic, 40 CJK - the inequity F-2 removes.
        assert_eq!((div4('a'), div4('б'), div4('字')), (120, 60, 40));

        // content/1: calibrated so a verse costs the same count in any of them. Russian says
        // the same thing in fewer characters than English, so a *fair* Cyrillic allowance is
        // smaller than the Latin one - just not half of it.
        let (lat, cyr, cjk) = (content('a'), content('б'), content('字'));
        assert_eq!((lat, cyr, cjk), (159, 115, 32));
        assert!(
            cyr * 100 / lat > div4('б') * 100 / div4('a'),
            "Cyrillic should keep more of the Latin allowance than under utf8-div4"
        );
    }

    #[test]
    fn fit_lands_on_character_boundaries_and_respects_the_bound() {
        let text =
            "Мы проверяем, что обрезка не рассекает символ пополам, и укладывается в предел.";
        for e in TokenEstimator::ALL.iter().copied() {
            for max in 1..12u32 {
                let end = e.fit(text, max);
                assert!(text.is_char_boundary(end), "{e} cut inside a character");
                assert!(
                    e.count(&text[..end]) <= max,
                    "{e} kept {end} bytes, over the bound {max}"
                );
            }
            // Text already inside the bound comes back whole.
            assert_eq!(e.fit("short", 99), "short".len());
            // Room is left for the mark on a shortened text.
            let end = e.fit_with_suffix(text, "\u{2026}", 5);
            assert!(text.is_char_boundary(end));
            assert!(e.count(&format!("{}\u{2026}", &text[..end])) <= 5);
        }
    }

    /// F-2's exit test (draft 3 §22), over `fixtures/estimator/parallel-en-ru.tsv`.
    ///
    /// The share of gists over `l0_max` must differ by at most 10% relative between English and
    /// Russian under `content/1`, and must reproduce the gap under `utf8-div4`. The fixture is
    /// 500 verse-aligned public-domain pairs; verses are longer than gists, so the bound is
    /// scaled to the fixture rather than the fixture to the bound - what is being measured is
    /// the *ratio* between the two languages, which is what the exit test is about.
    #[test]
    fn content1_equalises_the_over_bound_share_between_en_and_ru() {
        let text = std::fs::read_to_string(format!("{FIXTURES}/parallel-en-ru.tsv"))
            .expect("fixtures/estimator/parallel-en-ru.tsv");
        let pairs: Vec<(&str, &str)> = text
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let mut c = l.split('\t');
                let _ref = c.next()?;
                Some((c.next()?, c.next()?))
            })
            .collect();
        assert_eq!(pairs.len(), 500, "the fixture is 500 pairs");

        // A verse is about four times a gist, so the share is measured at a bound that puts
        // the English pairs near the middle of the distribution rather than all over it.
        let bound = 34;
        let ens: Vec<&str> = pairs.iter().map(|p| p.0).collect();
        let rus: Vec<&str> = pairs.iter().map(|p| p.1).collect();
        let share = |e: TokenEstimator, texts: &[&str]| {
            let over = texts.iter().filter(|t| e.count(t) > bound).count();
            over as f64 / texts.len() as f64
        };

        let (c_en, c_ru) = (
            share(TokenEstimator::Content1, &ens),
            share(TokenEstimator::Content1, &rus),
        );
        let (d_en, d_ru) = (
            share(TokenEstimator::Utf8Div4, &ens),
            share(TokenEstimator::Utf8Div4, &rus),
        );
        let gap = |a: f64, b: f64| (a - b).abs() / a;

        assert!(
            gap(d_en, d_ru) > 0.10,
            "utf8-div4 should reproduce the gap: en {d_en:.3} ru {d_ru:.3}"
        );
        assert!(
            gap(c_en, c_ru) <= 0.10,
            "content/1 should close it to 10%: en {c_en:.3} ru {c_ru:.3}, \
             gap {:.1}%",
            100.0 * gap(c_en, c_ru)
        );
    }

    #[test]
    fn an_unknown_id_is_kept_and_makes_the_bounds_unevaluable() {
        let p = ProfileEstimator::from_wire("smysl/not-a-real-estimator");
        assert_eq!(p.estimator(), None, "an unknown count must not be guessed");
        assert_eq!(
            p.wire_id(),
            Some("smysl/not-a-real-estimator"),
            "round trip"
        );
        // The default is absent on the wire, so pre-F-2 views encode to the bytes they had.
        assert_eq!(ProfileEstimator::Unset.wire_id(), None);
        assert_eq!(
            ProfileEstimator::Known(TokenEstimator::Utf8Div4).wire_id(),
            None
        );
        assert_eq!(
            ProfileEstimator::Known(TokenEstimator::Content1).wire_id(),
            Some("smysl/content/1")
        );
        assert_eq!(
            ProfileEstimator::from_wire("smysl/content/1"),
            ProfileEstimator::Known(TokenEstimator::Content1)
        );
    }

    #[test]
    fn ids_round_trip_through_parse() {
        for e in TokenEstimator::ALL.iter().copied() {
            assert_eq!(TokenEstimator::parse(e.id()), Some(e));
            assert_eq!(e.to_string(), e.id());
        }
        assert_eq!(TokenEstimator::parse("smysl/script-aware/1"), None);
    }
}
